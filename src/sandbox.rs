use std::time::Instant;

use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::config::Config;
use crate::constants;

#[derive(Error, Debug)]
pub enum SessionError {
    #[error("nsjail not found at {0}")]
    NsjailNotFound(String),
    #[error("sandbox requires root (nsjail needs CLONE_NEWNS/CLONE_NEWNET)")]
    NotRoot,
    #[error("sandbox is required but unavailable: {0}")]
    SandboxRequired(String),
    #[error("process error: {0}")]
    Process(#[from] std::io::Error),
    #[error("session died")]
    SessionDied,
    #[error("execution exceeded the {0}s time limit; the session was terminated")]
    Timeout(u64),
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("invalid package name {0:?}: expected a PEP 508 requirement, not an option")]
    InvalidPackage(String),
}

#[derive(Debug, PartialEq)]
pub struct ExecutionOutput {
    pub stdout: String,
    pub stderr: String,
    pub more_input_needed: bool,
    pub truncated_stdout: bool,
    pub truncated_stderr: bool,
}

#[derive(Debug)]
pub struct Session {
    pub session_id: String,
    work_dir: tempfile::TempDir,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout_reader: Option<BufReader<ChildStdout>>,
    created_at: Instant,
    last_used: Instant,
    pub sandboxed: bool,
}

fn find_nsjail(nsjail_bin: &str) -> Result<String, SessionError> {
    which::which(nsjail_bin)
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|_| SessionError::NsjailNotFound(nsjail_bin.to_string()))
}

fn check_root() -> Result<(), SessionError> {
    #[cfg(target_os = "linux")]
    {
        if unsafe { libc::geteuid() } != 0 {
            return Err(SessionError::NotRoot);
        }
    }
    Ok(())
}

/// Is a per-session nsjail available on this host? Probed once at startup so the effective
/// isolation mode is stated in the logs rather than discovered per session.
pub fn probe_sandbox(config: &Config) -> Result<String, SessionError> {
    let path = find_nsjail(&config.nsjail_path)?;
    check_root()?;
    Ok(path)
}

fn build_nsjail_cmd(nsjail_path: &str, work_path: &str, config: &Config, wrapper: &str) -> Command {
    let bindmount = format!("{}:{}", work_path, constants::SANDBOX_WORK_DIR);
    let time_limit = config.sandbox_time_limit_secs.to_string();
    let mem_limit = config.sandbox_memory_limit_bytes.to_string();

    let mut cmd = Command::new(nsjail_path);
    cmd.args([
        constants::NSJAIL_MODE_ARG,
        constants::NSJAIL_MODE_EXEC,
        constants::NSJAIL_CHROOT_ARG,
        constants::SANDBOX_CHROOT,
        constants::NSJAIL_BINDMOUNT_ARG,
        &bindmount,
        constants::NSJAIL_CWD_ARG,
        constants::SANDBOX_WORK_DIR,
        constants::NSJAIL_DISABLE_PROC,
        constants::NSJAIL_IFACE_NO_LO,
        constants::NSJAIL_REALLY_QUIET,
        constants::NSJAIL_TIME_LIMIT_ARG,
        &time_limit,
        constants::NSJAIL_MEMORY_LIMIT_ARG,
        &mem_limit,
        constants::NSJAIL_CMD_SEP,
    ]);
    cmd.arg(&config.sandbox_python)
        .arg(constants::PYTHON_UNBUFFERED)
        .arg(wrapper);
    cmd
}

fn build_direct_cmd(python: &str, wrapper: &str) -> Command {
    let mut cmd = Command::new(python);
    cmd.arg(constants::PYTHON_UNBUFFERED).arg(wrapper);
    cmd
}

/// Worker-side limits, passed through the environment so the wrapper applies them to itself.
///
/// `packages_dir` differs by mode: under nsjail the work dir is bindmounted at `/work`, but a
/// direct child sees the real host path. Passing it explicitly is what makes session-scoped
/// `install_package` work in both modes.
fn apply_worker_env(
    cmd: &mut Command,
    config: &Config,
    packages_dir: &std::path::Path,
    jailed: bool,
) {
    let visible_packages = if jailed {
        constants::PKGS_SYS_PATH_ENTRY.to_string()
    } else {
        packages_dir.to_string_lossy().to_string()
    };
    cmd.env(constants::ENV_PACKAGES_DIR, visible_packages);
    cmd.env(
        constants::ENV_LIMIT_MEMORY,
        config.worker_memory_limit_bytes.to_string(),
    );
    cmd.env(
        constants::ENV_LIMIT_CPU,
        config.worker_cpu_limit_secs.to_string(),
    );
    cmd.env(
        constants::ENV_LIMIT_FILE,
        config.worker_file_limit_bytes.to_string(),
    );
    cmd.env(
        constants::ENV_LIMIT_PROCS,
        config.worker_process_limit.to_string(),
    );
}

impl Session {
    pub fn new(
        session_id: String,
        wrapper_path: &std::path::Path,
        config: &Config,
    ) -> Result<Self, SessionError> {
        let work_dir = tempfile::tempdir()?;
        let wrapper = wrapper_path.to_string_lossy().to_string();

        let packages_dir = work_dir.path().join(constants::PKGS_DIR);
        std::fs::create_dir_all(&packages_dir)?;

        let (mut cmd, sandboxed) = if config.sandbox_enabled {
            match probe_sandbox(config) {
                Ok(nsjail_path) => {
                    let work_path = work_dir.path().to_string_lossy().to_string();
                    tracing::info!(session_id = %session_id, sandboxed = true, "Session created (nsjail)");
                    (
                        build_nsjail_cmd(&nsjail_path, &work_path, config, &wrapper),
                        true,
                    )
                }
                Err(e) if config.sandbox_required => {
                    tracing::error!(session_id = %session_id, error = %e, "Sandbox required but unavailable");
                    return Err(SessionError::SandboxRequired(e.to_string()));
                }
                Err(e) => {
                    tracing::warn!(session_id = %session_id, error = %e, "nsjail unavailable; falling back to a direct child (container is the isolation boundary)");
                    (build_direct_cmd(&config.system_python, &wrapper), false)
                }
            }
        } else {
            tracing::debug!(session_id = %session_id, sandboxed = false, "Session created (direct child)");
            (build_direct_cmd(&config.system_python, &wrapper), false)
        };

        apply_worker_env(&mut cmd, config, &packages_dir, sandboxed);
        if !sandboxed {
            cmd.current_dir(work_dir.path());
        }

        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;

        let stdin = child.stdin.take().expect("stdin not captured");
        let stdout = child.stdout.take().expect("stdout not captured");

        Ok(Self {
            session_id,
            work_dir,
            child: Some(child),
            stdin: Some(stdin),
            stdout_reader: Some(BufReader::new(stdout)),
            created_at: Instant::now(),
            last_used: Instant::now(),
            sandboxed,
        })
    }

    pub fn packages_path(&self) -> std::path::PathBuf {
        self.work_dir.path().join(constants::PKGS_DIR)
    }

    /// Run one statement batch.
    ///
    /// Any error leaves the session unusable and the caller **must** drop it: a timed-out or
    /// half-read worker still owns an unread response line, and reusing the pipe would pair that
    /// line with the *next* request's read.
    pub async fn execute(
        &mut self,
        code: &str,
        timeout_secs: u64,
    ) -> Result<ExecutionOutput, SessionError> {
        self.last_used = Instant::now();
        let preview: String = code.chars().take(80).collect();
        tracing::debug!(session_id = %self.session_id, code_len = code.len(), "Executing code: {}...", preview);

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            self.exchange(code),
        )
        .await;

        match outcome {
            Ok(result) => result,
            Err(_elapsed) => {
                tracing::warn!(session_id = %self.session_id, timeout_secs, "Execution timed out; killing worker");
                self.kill();
                Err(SessionError::Timeout(timeout_secs))
            }
        }
    }

    async fn exchange(&mut self, code: &str) -> Result<ExecutionOutput, SessionError> {
        let req = serde_json::json!({
            constants::PROTO_CMD: constants::PROTO_EXEC,
            constants::PROTO_CODE: code,
        });

        let stdin = self.stdin.as_mut().ok_or(SessionError::SessionDied)?;
        stdin
            .write_all((serde_json::to_string(&req)? + "\n").as_bytes())
            .await?;
        stdin.flush().await?;

        let reader = self
            .stdout_reader
            .as_mut()
            .ok_or(SessionError::SessionDied)?;
        let mut line = String::new();
        reader.read_line(&mut line).await?;

        if line.is_empty() {
            tracing::error!(session_id = %self.session_id, "Session process died unexpectedly");
            return Err(SessionError::SessionDied);
        }

        let resp: serde_json::Value = serde_json::from_str(&line)?;
        let stdout = str_field(&resp, constants::PROTO_STDOUT);
        let stderr = str_field(&resp, constants::PROTO_STDERR);
        let more = resp
            .get(constants::PROTO_MORE_INPUT)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let stdout_len = stdout.len();
        let stderr_len = stderr.len();

        Ok(ExecutionOutput {
            stdout: truncate_str(stdout),
            stderr: truncate_str(stderr),
            more_input_needed: more,
            truncated_stdout: stdout_len > constants::MAX_OUTPUT_BYTES,
            truncated_stderr: stderr_len > constants::MAX_OUTPUT_BYTES,
        })
    }

    fn kill(&mut self) {
        self.stdin = None;
        self.stdout_reader = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }

    pub fn idle_seconds(&self) -> u64 {
        self.last_used.elapsed().as_secs()
    }

    pub fn age_seconds(&self) -> u64 {
        self.created_at.elapsed().as_secs()
    }
}

fn str_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

impl Drop for Session {
    fn drop(&mut self) {
        tracing::debug!(session_id = %self.session_id, "Session dropped");
        self.kill();
    }
}

#[derive(Serialize)]
pub struct PipResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returncode: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

/// Reject anything pip would read as an option rather than a requirement.
///
/// `package` reaches pip as a bare argv entry, so a value like `--index-url=http://…` would be
/// parsed as a *flag* and silently repoint the installer at an attacker-chosen index. There is no
/// shell here, so this is the whole of the injection surface — but it is enough.
pub fn validate_package(package: &str) -> Result<(), SessionError> {
    let trimmed = package.trim();
    let invalid = trimmed.is_empty()
        || trimmed.starts_with('-')
        || trimmed.contains(char::is_whitespace)
        || trimmed.contains(|c: char| c.is_control());
    if invalid {
        return Err(SessionError::InvalidPackage(package.to_string()));
    }
    Ok(())
}

pub async fn run_pip_install(
    package: &str,
    system_python: &str,
) -> Result<PipResult, SessionError> {
    validate_package(package)?;
    tracing::info!(package = %package, "pip install (global)");
    Ok(pip_install_cmd(system_python, package, None).await)
}

pub async fn run_pip_install_to_target(
    package: &str,
    system_python: &str,
    target: &std::path::Path,
) -> Result<PipResult, SessionError> {
    validate_package(package)?;
    tracing::info!(package = %package, target = %target.display(), "pip install (session-scoped)");
    Ok(pip_install_cmd(system_python, package, Some(target)).await)
}

async fn pip_install_cmd(
    system_python: &str,
    package: &str,
    target: Option<&std::path::Path>,
) -> PipResult {
    let mut cmd = Command::new(system_python);
    cmd.args([
        constants::PIP_MODULE,
        constants::PIP_CMD,
        constants::PIP_INSTALL,
        constants::PIP_NO_INPUT,
    ]);
    if let Some(t) = target {
        cmd.arg(constants::PIP_TARGET_ARG).arg(t);
    }
    // Everything after `--` is a requirement, never an option.
    cmd.arg(constants::PIP_END_OF_OPTIONS).arg(package);

    match cmd.output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let code = output.status.code();
            if code != Some(0) {
                tracing::warn!(package = %package, exit_code = ?code, "pip install failed");
            }
            PipResult {
                returncode: code,
                stdout: Some(truncate_str(stdout)),
                stderr: Some(truncate_str(stderr)),
            }
        }
        Err(e) => {
            tracing::error!(package = %package, error = %e, "pip install command failed");
            PipResult {
                returncode: Some(-1),
                stdout: None,
                stderr: Some(e.to_string()),
            }
        }
    }
}

pub async fn run_pip_list(system_python: &str) -> PipResult {
    tracing::debug!("pip list");
    match Command::new(system_python)
        .args([
            constants::PIP_MODULE,
            constants::PIP_CMD,
            constants::PIP_LIST,
            constants::PIP_FORMAT_ARG,
        ])
        .output()
        .await
    {
        Ok(output) => PipResult {
            returncode: output.status.code(),
            stdout: Some(truncate_str(
                String::from_utf8_lossy(&output.stdout).to_string(),
            )),
            stderr: Some(truncate_str(
                String::from_utf8_lossy(&output.stderr).to_string(),
            )),
        },
        Err(e) => PipResult {
            returncode: Some(-1),
            stdout: None,
            stderr: Some(e.to_string()),
        },
    }
}

pub async fn get_python_info(system_python: &str) -> serde_json::Value {
    tracing::debug!("get_python_info");
    match Command::new(system_python)
        .args([constants::PYTHON_C_ARG, constants::PYTHON_INFO_CODE])
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            serde_json::from_str(&stdout).unwrap_or_else(|_| {
                serde_json::json!({
                    constants::KEY_ERROR: "Failed to parse Python info",
                    constants::KEY_RAW: stdout,
                })
            })
        }
        Err(e) => serde_json::json!({
            constants::KEY_ERROR: e.to_string(),
        }),
    }
}

/// Truncate to at most `MAX_OUTPUT_BYTES`, never splitting a UTF-8 character.
///
/// Slicing a `String` at a byte index that falls inside a multi-byte character panics, so
/// `print("🎉" * 20000)` used to take the request down with it.
pub fn truncate_str(s: String) -> String {
    truncate_to(s, constants::MAX_OUTPUT_BYTES)
}

pub fn truncate_to(s: String, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_within_limit() {
        let result = truncate_str("hello".into());
        assert_eq!(result, "hello");
    }

    #[test]
    fn truncate_exceeds_limit() {
        let big = "x".repeat(constants::MAX_OUTPUT_BYTES + 100);
        let result = truncate_str(big);
        assert_eq!(result.len(), constants::MAX_OUTPUT_BYTES);
    }

    /// Regression: slicing at a byte index inside a multi-byte character panics. Any session
    /// printing enough emoji or CJK text used to crash the request instead of truncating it.
    #[test]
    fn truncate_never_splits_a_utf8_character() {
        // 4 bytes per emoji, so the limit lands mid-character for at least one offset.
        for extra in 0..4 {
            let s = "🎉".repeat(constants::MAX_OUTPUT_BYTES / 4 + 10 + extra);
            let out = truncate_to(s, constants::MAX_OUTPUT_BYTES + extra);
            assert!(out.len() <= constants::MAX_OUTPUT_BYTES + extra);
            assert!(std::str::from_utf8(out.as_bytes()).is_ok());
        }
    }

    #[test]
    fn truncate_to_boundary_keeps_whole_characters() {
        // "é" is 2 bytes; a 3-byte budget must drop the second one entirely.
        let out = truncate_to("éé".to_string(), 3);
        assert_eq!(out, "é");
    }

    #[test]
    fn execution_output_truncation_flags() {
        let output = ExecutionOutput {
            stdout: String::new(),
            stderr: String::new(),
            more_input_needed: false,
            truncated_stdout: true,
            truncated_stderr: false,
        };
        assert!(output.truncated_stdout);
        assert!(!output.truncated_stderr);
    }

    #[test]
    fn pip_result_serialize_success() {
        let result = PipResult {
            returncode: Some(0),
            stdout: Some("requests==2.28.0".into()),
            stderr: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("returncode"));
        assert!(json.contains("requests"));
    }

    #[test]
    fn pip_result_serialize_error() {
        let result = PipResult {
            returncode: Some(-1),
            stdout: None,
            stderr: Some("pip: command not found".into()),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("returncode"));
        assert!(json.contains("pip"));
    }

    #[test]
    fn valid_package_names_accepted() {
        for ok in [
            "requests",
            "requests==2.32.4",
            "ruamel.yaml",
            "pandas>=2.0",
            "uvicorn[standard]",
            "some_pkg",
        ] {
            assert!(validate_package(ok).is_ok(), "{ok} should be accepted");
        }
    }

    /// pip parses a leading-dash argv entry as an option, so an unvalidated package name can
    /// repoint the installer at another index or enable arbitrary flags.
    #[test]
    fn option_shaped_package_names_rejected() {
        for bad in [
            "--index-url=http://example.invalid/simple",
            "-e /etc",
            "--upgrade",
            "",
            "  ",
            "requests --index-url http://example.invalid",
        ] {
            assert!(
                validate_package(bad).is_err(),
                "{bad:?} should have been rejected"
            );
        }
    }
}

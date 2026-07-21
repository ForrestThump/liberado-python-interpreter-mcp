use std::sync::Arc;
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
    #[error("process error: {0}")]
    Process(#[from] std::io::Error),
    #[error("session died")]
    SessionDied,
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct ExecutionOutput {
    pub stdout: String,
    pub stderr: String,
    pub more_input_needed: bool,
    pub truncated_stdout: bool,
    pub truncated_stderr: bool,
}

pub struct Session {
    #[allow(dead_code)]
    pub session_id: String,
    #[allow(dead_code)]
    work_dir: tempfile::TempDir,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout_reader: Option<BufReader<ChildStdout>>,
    #[allow(dead_code)]
    created_at: Instant,
    last_used: Instant,
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

fn nsjail_bindmount_arg(work_path: &str) -> String {
    format!("{}:{}", work_path, constants::SANDBOX_WORK_DIR)
}

impl Session {
    pub fn new(
        session_id: String,
        wrapper_path: &std::path::Path,
        config: &Config,
    ) -> Result<Self, SessionError> {
        let nsjail_path = find_nsjail(&config.nsjail_path)?;
        check_root()?;

        let work_dir = tempfile::tempdir()?;
        let work_path = work_dir.path().to_string_lossy().to_string();
        let bindmount = nsjail_bindmount_arg(&work_path);

        let time_limit = config.sandbox_time_limit_secs.to_string();
        let mem_limit = config.sandbox_memory_limit_bytes.to_string();

        let mut cmd = Command::new(&nsjail_path);
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
            .arg(wrapper_path);

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());
        cmd.kill_on_drop(true);

        let mut child = cmd.spawn()?;
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
        })
    }

    pub async fn execute(&mut self, code: &str) -> Result<ExecutionOutput, SessionError> {
        self.last_used = Instant::now();

        let req = serde_json::json!({
            constants::PROTO_CMD: constants::PROTO_EXEC,
            constants::PROTO_CODE: code,
        });

        let stdin = self.stdin.as_mut().ok_or(SessionError::SessionDied)?;
        stdin
            .write_all((serde_json::to_string(&req)? + "\n").as_bytes())
            .await?;
        stdin.flush().await?;

        let reader = self.stdout_reader.as_mut().ok_or(SessionError::SessionDied)?;
        let mut line = String::new();
        reader.read_line(&mut line).await?;

        if line.is_empty() {
            return Err(SessionError::SessionDied);
        }

        let resp: serde_json::Value = serde_json::from_str(&line)?;
        let stdout = resp
            .get(constants::PROTO_STDOUT)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let stderr = resp
            .get(constants::PROTO_STDERR)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
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

    pub fn idle_seconds(&self) -> u64 {
        self.last_used.elapsed().as_secs()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
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

pub async fn run_pip_install(package: &str, system_python: &str) -> PipResult {
    match Command::new(system_python)
        .args([
            constants::PIP_MODULE,
            constants::PIP_CMD,
            constants::PIP_INSTALL,
            package,
        ])
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            PipResult {
                returncode: output.status.code(),
                stdout: Some(truncate_str(stdout)),
                stderr: Some(truncate_str(stderr)),
            }
        }
        Err(e) => PipResult {
            returncode: Some(-1),
            stdout: None,
            stderr: Some(e.to_string()),
        },
    }
}

pub async fn run_pip_list(system_python: &str) -> PipResult {
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
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            PipResult {
                returncode: output.status.code(),
                stdout: Some(stdout),
                stderr: Some(String::from_utf8_lossy(&output.stderr).to_string()),
            }
        }
        Err(e) => PipResult {
            returncode: Some(-1),
            stdout: None,
            stderr: Some(e.to_string()),
        },
    }
}

pub async fn get_python_info(system_python: &str) -> serde_json::Value {
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

fn truncate_str(s: String) -> String {
    if s.len() > constants::MAX_OUTPUT_BYTES {
        s[..constants::MAX_OUTPUT_BYTES].to_string()
    } else {
        s
    }
}

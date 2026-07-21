use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

pub const MAX_OUTPUT_BYTES: usize = 50_000;
pub const SESSION_IDLE_SECONDS: u64 = 1800;

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

fn find_nsjail() -> Result<String, SessionError> {
    let nsjail_bin = std::env::var("NSJAIL_PATH").unwrap_or_else(|_| "nsjail".to_string());

    if let Ok(path) = which::which(&nsjail_bin) {
        return Ok(path.to_string_lossy().to_string());
    }

    Err(SessionError::NsjailNotFound(nsjail_bin))
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

impl Session {
    pub fn new(session_id: String, wrapper_path: Arc<PathBuf>) -> Result<Self, SessionError> {
        let nsjail_path = find_nsjail()?;
        check_root()?;

        let work_dir = tempfile::tempdir()?;
        let work_path = work_dir.path().to_string_lossy().to_string();
        let wrapper = wrapper_path.to_string_lossy().to_string();
        let python = std::env::var("SANDBOX_PYTHON").unwrap_or_else(|_| "python3".to_string());

        let mut cmd = Command::new(&nsjail_path);
        cmd.args([
            "--mode",
            "exec",
            "--chroot",
            "/",
            "--bindmount",
            &format!("{}:/work", &work_path),
            "--cwd",
            "/work",
            "--disable_proc",
            "--iface_no_lo",
            "--really_quiet",
            "--time_limit",
            "300",
            "--cgroup_mem_max",
            "536870912",
            "--",
        ]);
        cmd.arg(&python).arg("-u").arg(&wrapper);

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
            "cmd": "exec",
            "code": code,
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
            .get("stdout")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let stderr = resp
            .get("stderr")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let more = resp
            .get("more_input_needed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let stdout_len = stdout.len();
        let stderr_len = stderr.len();

        Ok(ExecutionOutput {
            stdout: if stdout_len > MAX_OUTPUT_BYTES {
                stdout[..MAX_OUTPUT_BYTES].to_string()
            } else {
                stdout
            },
            stderr: if stderr_len > MAX_OUTPUT_BYTES {
                stderr[..MAX_OUTPUT_BYTES].to_string()
            } else {
                stderr
            },
            more_input_needed: more,
            truncated_stdout: stdout_len > MAX_OUTPUT_BYTES,
            truncated_stderr: stderr_len > MAX_OUTPUT_BYTES,
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

pub async fn run_pip_install(package: &str) -> PipResult {
    let python = std::env::var("SYSTEM_PYTHON").unwrap_or_else(|_| "python3".to_string());
    match Command::new(&python)
        .args(["-m", "pip", "install", package])
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            PipResult {
                returncode: output.status.code(),
                stdout: Some(truncate(stdout)),
                stderr: Some(truncate(stderr)),
            }
        }
        Err(e) => PipResult {
            returncode: Some(-1),
            stdout: None,
            stderr: Some(e.to_string()),
        },
    }
}

pub async fn run_pip_list() -> PipResult {
    let python = std::env::var("SYSTEM_PYTHON").unwrap_or_else(|_| "python3".to_string());
    match Command::new(&python)
        .args(["-m", "pip", "list", "--format=json"])
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

pub async fn get_python_info() -> serde_json::Value {
    let python = std::env::var("SYSTEM_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let code = "import sys,json;print(json.dumps({'version':sys.version,'executable':sys.executable,'platform':sys.platform,'prefix':sys.prefix}))";

    match Command::new(&python).args(["-c", code]).output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            serde_json::from_str(&stdout).unwrap_or_else(|_| {
                serde_json::json!({
                    "error": "Failed to parse Python info",
                    "raw": stdout,
                })
            })
        }
        Err(e) => serde_json::json!({
            "error": e.to_string(),
        }),
    }
}

fn truncate(s: String) -> String {
    if s.len() > MAX_OUTPUT_BYTES {
        s[..MAX_OUTPUT_BYTES].to_string()
    } else {
        s
    }
}

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use turbomcp::prelude::*;

use crate::config::Config;
use crate::constants;
use crate::sandbox;

#[derive(Clone)]
pub struct InterpreterServer {
    config: Arc<Config>,
    sessions: Arc<Mutex<HashMap<String, sandbox::Session>>>,
    wrapper_path: Arc<PathBuf>,
}

#[turbomcp::server(
    name = "liberado-python-interpreter-mcp",
    version = "0.1.0"
)]
impl InterpreterServer {
    pub fn new(config: Config) -> Self {
        let config = Arc::new(config);
        let wrapper_path = Arc::new(config.wrapper_path.clone());
        Self {
            config,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            wrapper_path,
        }
    }

    #[tool("Execute Python code in a persistent REPL session. Variables, imports, and function definitions persist across calls within the same session. Omit session_id to create a new session; pass one from a previous response to continue.")]
    async fn execute_python(
        &self,
        code: String,
        session_id: Option<String>,
    ) -> McpResult<String> {
        self.cleanup_expired().await;

        let sid = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let mut sessions = self.sessions.lock().await;
        let created = !sessions.contains_key(&sid);

        if created {
            let session = sandbox::Session::new(
                sid.clone(),
                &self.wrapper_path,
                &self.config,
            )
            .map_err(|e| {
                McpError::internal(format!("Failed to create sandbox session: {e}"))
            })?;
            sessions.insert(sid.clone(), session);
        }

        let session = sessions
            .get_mut(&sid)
            .ok_or_else(|| McpError::internal("Session disappeared".to_string()))?;

        match session.execute(&code).await {
            Ok(output) => {
                let mut result = serde_json::json!({
                    constants::KEY_SESSION_ID: &sid,
                    constants::PROTO_STDOUT: output.stdout,
                    constants::PROTO_STDERR: output.stderr,
                    constants::PROTO_MORE_INPUT: output.more_input_needed,
                    constants::KEY_CREATED: created,
                });
                if let Some(m) = result.as_object_mut() {
                    if output.truncated_stdout {
                        m.insert(
                            constants::KEY_TRUNCATED_STDOUT.into(),
                            true.into(),
                        );
                    }
                    if output.truncated_stderr {
                        m.insert(
                            constants::KEY_TRUNCATED_STDERR.into(),
                            true.into(),
                        );
                    }
                }
                Ok(result.to_string())
            }
            Err(e) => {
                if !created {
                    sessions.remove(&sid);
                }
                Err(McpError::internal(format!("Execution error: {e}")))
            }
        }
    }

    #[tool("Destroy a Python REPL session, releasing its namespace and subprocess.")]
    async fn reset_python_session(&self, session_id: String) -> McpResult<String> {
        let mut sessions = self.sessions.lock().await;
        let existed = sessions.remove(&session_id).is_some();
        let result = serde_json::json!({
            constants::KEY_SESSION_ID: session_id,
            constants::KEY_RESET: existed,
        });
        Ok(result.to_string())
    }

    #[tool("List all active Python REPL sessions with variable counts and idle times. Sessions idle for >30 minutes are auto-cleaned.")]
    async fn list_python_sessions(&self) -> McpResult<String> {
        let sessions = self.sessions.lock().await;
        let mut list = Vec::new();
        for (sid, session) in sessions.iter() {
            list.push(serde_json::json!({
                constants::KEY_SESSION_ID: sid,
                constants::KEY_SECONDS_IDLE: session.idle_seconds(),
            }));
        }
        Ok(serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string()))
    }

    #[tool("Read a text file and return its contents.")]
    async fn read_file(&self, path: String) -> McpResult<String> {
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                let truncated =
                    content.len() > constants::MAX_OUTPUT_BYTES;
                let result = serde_json::json!({
                    constants::KEY_PATH: &path,
                    constants::KEY_CONTENT: &content
                        [..content.len().min(constants::MAX_OUTPUT_BYTES)],
                    constants::KEY_SIZE_BYTES: content.len(),
                    constants::KEY_TRUNCATED: truncated,
                });
                Ok(result.to_string())
            }
            Err(e) => {
                let result = serde_json::json!({
                    constants::KEY_PATH: &path,
                    constants::KEY_ERROR: e.to_string(),
                });
                Ok(result.to_string())
            }
        }
    }

    #[tool("Write content to a file, creating parent directories as needed. Overwrites existing files.")]
    async fn write_file(&self, path: String, content: String) -> McpResult<String> {
        let parent = PathBuf::from(&path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        if let Err(e) = tokio::fs::create_dir_all(&parent).await {
            let result = serde_json::json!({
                constants::KEY_PATH: &path,
                constants::KEY_ERROR: format!("Failed to create directory: {e}"),
            });
            return Ok(result.to_string());
        }

        match tokio::fs::write(&path, &content).await {
            Ok(_) => {
                let result = serde_json::json!({
                    constants::KEY_PATH: &path,
                    constants::KEY_WRITTEN: true,
                    constants::KEY_SIZE_BYTES: content.len(),
                });
                Ok(result.to_string())
            }
            Err(e) => {
                let result = serde_json::json!({
                    constants::KEY_PATH: &path,
                    constants::KEY_ERROR: e.to_string(),
                });
                Ok(result.to_string())
            }
        }
    }

    #[tool("Find and replace text in a file. Set count=0 to replace all occurrences.")]
    async fn edit_file(
        &self,
        path: String,
        find: String,
        replace: String,
        count: Option<usize>,
    ) -> McpResult<String> {
        let count = count.unwrap_or(1);
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                let result = serde_json::json!({
                    constants::KEY_PATH: &path,
                    constants::KEY_ERROR: e.to_string(),
                    constants::KEY_REPLACED: 0,
                });
                return Ok(result.to_string());
            }
        };

        if !content.contains(&find) {
            let result = serde_json::json!({
                constants::KEY_PATH: &path,
                constants::KEY_ERROR: constants::KEY_FIND_NOT_FOUND,
                constants::KEY_REPLACED: 0,
            });
            return Ok(result.to_string());
        }

        let (new_content, replaced) = if count == 0 {
            let r = content.matches(&find).count();
            (content.replace(&find, &replace), r)
        } else {
            (content.replacen(&find, &replace, count), count)
        };

        if let Err(e) = tokio::fs::write(&path, &new_content).await {
            let result = serde_json::json!({
                constants::KEY_PATH: &path,
                constants::KEY_ERROR: e.to_string(),
                constants::KEY_REPLACED: 0,
            });
            return Ok(result.to_string());
        }

        let result = serde_json::json!({
            constants::KEY_PATH: &path,
            constants::KEY_REPLACED: replaced,
            constants::KEY_SIZE_BYTES: new_content.len(),
        });
        Ok(result.to_string())
    }

    #[tool("Install a Python package using pip. Accepts any pip-compatible specifier (name, name==version, etc.).")]
    async fn install_package(&self, package: String) -> McpResult<String> {
        let result =
            sandbox::run_pip_install(&package, &self.config.system_python).await;
        Ok(serde_json::to_string(&result)
            .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string()))
    }

    #[tool("List all installed Python packages in JSON format.")]
    async fn list_packages(&self) -> McpResult<String> {
        let result = sandbox::run_pip_list(&self.config.system_python).await;
        Ok(serde_json::to_string(&result)
            .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string()))
    }

    #[tool("Get information about the Python runtime: version, executable path, platform.")]
    async fn get_python_info(&self) -> McpResult<String> {
        let info = sandbox::get_python_info(&self.config.system_python).await;
        Ok(info.to_string())
    }

    async fn cleanup_expired(&self) {
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_sid, session| {
            session.idle_seconds() < constants::SESSION_IDLE_SECONDS
        });
    }
}

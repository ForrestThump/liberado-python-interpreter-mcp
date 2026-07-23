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

#[turbomcp::server(name = "liberado-python-interpreter-mcp", version = "0.1.0")]
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
    async fn execute_python(&self, code: String, session_id: Option<String>) -> McpResult<String> {
        self.cleanup_expired().await;

        let sid = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let mut sessions = self.sessions.lock().await;
        let created = !sessions.contains_key(&sid);

        if created {
            tracing::info!(session_id = %sid, "Creating session");
            let session = sandbox::Session::new(sid.clone(), &self.wrapper_path, &self.config)
                .map_err(|e| {
                    tracing::error!(session_id = %sid, error = %e, "Failed to create session");
                    McpError::internal(format!("Failed to create sandbox session: {e}"))
                })?;
            sessions.insert(sid.clone(), session);
        } else {
            tracing::debug!(session_id = %sid, "Reusing existing session");
        }

        let session = sessions.get_mut(&sid).ok_or_else(|| {
            tracing::error!(session_id = %sid, "Session disappeared during execution");
            McpError::internal("Session disappeared".to_string())
        })?;

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
                        m.insert(constants::KEY_TRUNCATED_STDOUT.into(), true.into());
                    }
                    if output.truncated_stderr {
                        m.insert(constants::KEY_TRUNCATED_STDERR.into(), true.into());
                    }
                }
                Ok(result.to_string())
            }
            Err(e) => {
                tracing::warn!(session_id = %sid, error = %e, "Execution error");
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
        if existed {
            tracing::info!(session_id = %session_id, "Session reset");
        } else {
            tracing::debug!(session_id = %session_id, "Session not found for reset");
        }
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
        tracing::debug!(count = list.len(), "list_python_sessions");
        Ok(serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string()))
    }

    #[tool("Read a text file and return its contents.")]
    async fn read_file(&self, path: String) -> McpResult<String> {
        tracing::debug!(path = %path, "read_file");
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                let truncated = content.len() > constants::MAX_OUTPUT_BYTES;
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
                tracing::warn!(path = %path, error = %e, "read_file failed");
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
        tracing::debug!(path = %path, bytes = content.len(), "write_file");
        let parent = PathBuf::from(&path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        if let Err(e) = tokio::fs::create_dir_all(&parent).await {
            tracing::warn!(path = %path, error = %e, "write_file mkdir failed");
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
                tracing::warn!(path = %path, error = %e, "write_file failed");
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
        tracing::debug!(path = %path, find = %find, replace = %replace, count = count, "edit_file");
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "edit_file read failed");
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
            tracing::warn!(path = %path, error = %e, "edit_file write failed");
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

    #[tool("Install a Python package using pip. If session_id is provided, installs into that session's isolated packages directory (requires restarting the session). Without session_id, installs globally.")]
    async fn install_package(
        &self,
        package: String,
        session_id: Option<String>,
    ) -> McpResult<String> {
        let result = if let Some(ref sid) = session_id {
            let sessions = self.sessions.lock().await;
            match sessions.get(sid) {
                Some(session) => {
                    let target = session.packages_path();
                    sandbox::run_pip_install_to_target(
                        &package,
                        &self.config.system_python,
                        &target,
                    )
                    .await
                }
                None => {
                    tracing::warn!(session_id = %sid, "Session not found, falling back to global install");
                    sandbox::run_pip_install(&package, &self.config.system_python).await
                }
            }
        } else {
            sandbox::run_pip_install(&package, &self.config.system_python).await
        };
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
        let before = sessions.len();
        sessions.retain(|_sid, session| session.idle_seconds() < constants::SESSION_IDLE_SECONDS);
        let removed = before.saturating_sub(sessions.len());
        if removed > 0 {
            tracing::info!(removed = removed, "Cleaned up expired sessions");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::{Service, ServiceExt};
    use turbomcp::{JsonRpcMessage, JsonRpcRequest, VersionDispatcher};

    fn test_config() -> Config {
        Config {
            bind_addr: "127.0.0.1:0".into(),
            sandbox_enabled: false,
            nsjail_path: "nsjail".into(),
            sandbox_python: "python3".into(),
            system_python: "python3".into(),
            wrapper_path: std::env::current_dir().unwrap().join("sandbox/wrapper.py"),
            sandbox_time_limit_secs: 300,
            sandbox_memory_limit_bytes: 512 * 1024 * 1024,
            log_level: "debug".into(),
        }
    }

    fn draft_meta() -> serde_json::Value {
        serde_json::json!({ "io.modelcontextprotocol/protocolVersion": "2026-07-28" })
    }

    fn build_dispatcher() -> VersionDispatcher<InterpreterServer> {
        InterpreterServer::new(test_config()).into_server().build()
    }

    async fn call_tool(
        svc: &mut VersionDispatcher<InterpreterServer>,
        name: &str,
        arguments: serde_json::Value,
    ) -> serde_json::Value {
        let req = JsonRpcRequest::new(
            1,
            "tools/call",
            Some(serde_json::json!({
                "name": name,
                "arguments": arguments,
                "_meta": draft_meta(),
            })),
        );
        let msg = svc
            .ready()
            .await
            .unwrap()
            .call(req.into())
            .await
            .unwrap()
            .expect("response");
        let JsonRpcMessage::Response(r) = msg else {
            panic!("expected response")
        };
        r.result.expect("tool result")
    }

    async fn list_tools_impl(svc: &mut VersionDispatcher<InterpreterServer>) -> serde_json::Value {
        let req = JsonRpcRequest::new(
            1,
            "tools/list",
            Some(serde_json::json!({ "_meta": draft_meta() })),
        );
        let msg = svc
            .ready()
            .await
            .unwrap()
            .call(req.into())
            .await
            .unwrap()
            .expect("response");
        let JsonRpcMessage::Response(r) = msg else {
            panic!("expected response")
        };
        r.result.expect("tools list result")
    }

    fn tool_response_text(result: &serde_json::Value) -> &str {
        result["content"][0]["text"].as_str().unwrap()
    }

    #[tokio::test]
    async fn list_tools_returns_all_nine() {
        let result = list_tools_impl(&mut build_dispatcher()).await;
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 9);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"execute_python"));
        assert!(names.contains(&"reset_python_session"));
        assert!(names.contains(&"list_python_sessions"));
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"edit_file"));
        assert!(names.contains(&"install_package"));
        assert!(names.contains(&"list_packages"));
        assert!(names.contains(&"get_python_info"));
    }

    #[tokio::test]
    async fn read_file_success() {
        let result = call_tool(
            &mut build_dispatcher(),
            "read_file",
            serde_json::json!({"path": "Cargo.toml"}),
        )
        .await;
        let text = tool_response_text(&result);
        assert!(text.contains("liberado-python-interpreter-mcp"));
        assert!(text.contains("content"));
    }

    #[tokio::test]
    async fn read_file_missing() {
        let result = call_tool(
            &mut build_dispatcher(),
            "read_file",
            serde_json::json!({"path": "/nonexistent/path/xyz"}),
        )
        .await;
        let text = tool_response_text(&result);
        assert!(text.contains("error"));
    }

    #[tokio::test]
    async fn write_and_read_file() {
        let path = "/tmp/liberado_test_write.txt";
        let _ = std::fs::remove_file(path);

        let mut dispatcher = build_dispatcher();

        let result = call_tool(
            &mut dispatcher,
            "write_file",
            serde_json::json!({"path": path, "content": "hello from test"}),
        )
        .await;
        let text = tool_response_text(&result);
        assert!(text.contains("\"written\":true"));

        let result = call_tool(
            &mut dispatcher,
            "read_file",
            serde_json::json!({"path": path}),
        )
        .await;
        let text = tool_response_text(&result);
        assert!(text.contains("hello from test"));

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn edit_file_replaces_text() {
        let path = "/tmp/liberado_test_edit.txt";
        std::fs::write(path, "original line\n").unwrap();

        let result = call_tool(
            &mut build_dispatcher(),
            "edit_file",
            serde_json::json!({"path": path, "find": "original", "replace": "modified"}),
        )
        .await;
        let text = tool_response_text(&result);
        assert!(text.contains("\"replaced\":1"));

        let content = std::fs::read_to_string(path).unwrap();
        assert_eq!(content, "modified line\n");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn get_python_info_returns_valid_json() {
        let result = call_tool(
            &mut build_dispatcher(),
            "get_python_info",
            serde_json::json!({}),
        )
        .await;
        let text = tool_response_text(&result);
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(value.get("version").is_some());
    }

    #[tokio::test]
    async fn list_packages_returns_valid_json() {
        let result = call_tool(
            &mut build_dispatcher(),
            "list_packages",
            serde_json::json!({}),
        )
        .await;
        let text = tool_response_text(&result);
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(value.get("returncode").is_some());
    }

    #[tokio::test]
    async fn execute_python_stateful_session() {
        let mut dispatcher = build_dispatcher();

        let result = call_tool(
            &mut dispatcher,
            "execute_python",
            serde_json::json!({"code": "x = [1, 2, 3]"}),
        )
        .await;
        let text = tool_response_text(&result);
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        let sid = value[constants::KEY_SESSION_ID]
            .as_str()
            .unwrap()
            .to_string();
        assert!(value[constants::KEY_CREATED].as_bool().unwrap());

        let result = call_tool(
            &mut dispatcher,
            "execute_python",
            serde_json::json!({"code": "sum(x)", "session_id": &sid}),
        )
        .await;
        let text = tool_response_text(&result);
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(value[constants::KEY_CREATED], false);
        let stdout = value[constants::PROTO_STDOUT].as_str().unwrap();
        assert!(stdout.contains("6"));

        let _ = call_tool(
            &mut dispatcher,
            "reset_python_session",
            serde_json::json!({"session_id": &sid}),
        )
        .await;
    }

    #[tokio::test]
    async fn session_scoped_pip_install() {
        let mut dispatcher = build_dispatcher();

        let result = call_tool(
            &mut dispatcher,
            "execute_python",
            serde_json::json!({"code": "x = 42"}),
        )
        .await;
        let text = tool_response_text(&result);
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        let sid = value[constants::KEY_SESSION_ID]
            .as_str()
            .unwrap()
            .to_string();

        let result = call_tool(
            &mut dispatcher,
            "install_package",
            serde_json::json!({"package": "six", "session_id": &sid}),
        )
        .await;
        let text = tool_response_text(&result);
        let pr: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(
            pr["returncode"].is_number(),
            "pip install should return a numeric exit code"
        );

        let _ = call_tool(
            &mut dispatcher,
            "reset_python_session",
            serde_json::json!({"session_id": &sid}),
        )
        .await;
    }
}

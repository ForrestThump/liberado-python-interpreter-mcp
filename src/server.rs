use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use turbomcp::prelude::*;

use crate::config::Config;
use crate::constants;
use crate::sandbox;
use crate::workspace;

/// One live session, behind its own lock.
///
/// The map lock is only ever held long enough to look this up. Holding it across an execution —
/// as an earlier revision did — serialised the whole server behind the slowest session, so a
/// single `time.sleep(60)` stalled every other caller's tool calls too.
type SharedSession = Arc<Mutex<sandbox::Session>>;

#[derive(Clone)]
pub struct InterpreterServer {
    config: Arc<Config>,
    sessions: Arc<Mutex<HashMap<String, SharedSession>>>,
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

    #[tool(
        "Execute Python code in a persistent REPL session. Variables, imports, and function \
         definitions persist across calls within the same session, and a trailing bare expression \
         has its value echoed. Omit session_id to create a new session; pass one from a previous \
         response to continue. Long-running code is cut off at the server's execution timeout and \
         the session is terminated."
    )]
    async fn execute_python(&self, code: String, session_id: Option<String>) -> McpResult<String> {
        self.cleanup_expired().await;

        let sid = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let (session, created) = self.acquire(&sid).await?;

        let mut guard = session.lock().await;
        let sandboxed = guard.sandboxed;
        match guard.execute(&code, self.config.exec_timeout_secs).await {
            Ok(output) => {
                let mut result = serde_json::json!({
                    constants::KEY_SESSION_ID: &sid,
                    constants::PROTO_STDOUT: output.stdout,
                    constants::PROTO_STDERR: output.stderr,
                    constants::PROTO_MORE_INPUT: output.more_input_needed,
                    constants::KEY_CREATED: created,
                    constants::KEY_SANDBOXED: sandboxed,
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
                // The worker's pipe state is no longer trustworthy after a failed exchange: a
                // timed-out call may still deliver its response later, which would then be read
                // as the answer to somebody else's code. Retire the session rather than reuse it.
                drop(guard);
                tracing::warn!(session_id = %sid, error = %e, "Execution failed; retiring session");
                self.sessions.lock().await.remove(&sid);
                Err(McpError::internal(format!("Execution error: {e}")))
            }
        }
    }

    #[tool("Destroy a Python REPL session, releasing its namespace and subprocess.")]
    async fn reset_python_session(&self, session_id: String) -> McpResult<String> {
        let removed = self.sessions.lock().await.remove(&session_id);
        let existed = removed.is_some();
        drop(removed);
        if existed {
            tracing::info!(session_id = %session_id, "Session reset");
        } else {
            tracing::debug!(session_id = %session_id, "Session not found for reset");
        }
        Ok(serde_json::json!({
            constants::KEY_SESSION_ID: session_id,
            constants::KEY_RESET: existed,
        })
        .to_string())
    }

    #[tool(
        "List all active Python REPL sessions with their age, idle time, and whether they are \
         nsjail-sandboxed. Sessions idle for over 30 minutes are reaped automatically."
    )]
    async fn list_python_sessions(&self) -> McpResult<String> {
        let snapshot: Vec<(String, SharedSession)> = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .map(|(k, v)| (k.clone(), Arc::clone(v)))
                .collect()
        };

        let mut list = Vec::new();
        for (sid, session) in snapshot {
            // A busy session is mid-execution; report it without waiting for it to finish.
            let Ok(guard) = session.try_lock() else {
                list.push(serde_json::json!({
                    constants::KEY_SESSION_ID: sid,
                    "busy": true,
                }));
                continue;
            };
            list.push(serde_json::json!({
                constants::KEY_SESSION_ID: sid,
                constants::KEY_SECONDS_IDLE: guard.idle_seconds(),
                constants::KEY_SECONDS_ALIVE: guard.age_seconds(),
                constants::KEY_SANDBOXED: guard.sandboxed,
            }));
        }
        tracing::debug!(count = list.len(), "list_python_sessions");
        Ok(serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string()))
    }

    #[tool("Read a text file from the interpreter workspace and return its contents.")]
    async fn read_file(&self, path: String) -> McpResult<String> {
        tracing::debug!(path = %path, "read_file");
        let resolved = match self.resolve(&path) {
            Ok(p) => p,
            Err(response) => return Ok(response),
        };

        match tokio::fs::read_to_string(&resolved).await {
            Ok(content) => {
                let size = content.len();
                let truncated = size > constants::MAX_OUTPUT_BYTES;
                Ok(serde_json::json!({
                    constants::KEY_PATH: &path,
                    constants::KEY_CONTENT: sandbox::truncate_str(content),
                    constants::KEY_SIZE_BYTES: size,
                    constants::KEY_TRUNCATED: truncated,
                })
                .to_string())
            }
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "read_file failed");
                Ok(file_error(&path, e.to_string()))
            }
        }
    }

    #[tool(
        "Write content to a file in the interpreter workspace, creating parent directories as \
         needed. Overwrites existing files."
    )]
    async fn write_file(&self, path: String, content: String) -> McpResult<String> {
        tracing::debug!(path = %path, bytes = content.len(), "write_file");
        let resolved = match self.resolve(&path) {
            Ok(p) => p,
            Err(response) => return Ok(response),
        };

        if let Some(parent) = resolved.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                tracing::warn!(path = %path, error = %e, "write_file mkdir failed");
                return Ok(file_error(
                    &path,
                    format!("Failed to create directory: {e}"),
                ));
            }
        }

        match tokio::fs::write(&resolved, &content).await {
            Ok(_) => Ok(serde_json::json!({
                constants::KEY_PATH: &path,
                constants::KEY_WRITTEN: true,
                constants::KEY_SIZE_BYTES: content.len(),
            })
            .to_string()),
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "write_file failed");
                Ok(file_error(&path, e.to_string()))
            }
        }
    }

    #[tool(
        "Find and replace text in a file in the interpreter workspace. Set count=0 to replace all \
         occurrences. Returns the number of replacements actually made."
    )]
    async fn edit_file(
        &self,
        path: String,
        find: String,
        replace: String,
        count: Option<usize>,
    ) -> McpResult<String> {
        let count = count.unwrap_or(1);
        tracing::debug!(path = %path, count = count, "edit_file");
        let resolved = match self.resolve(&path) {
            Ok(p) => p,
            Err(response) => return Ok(response),
        };

        let content = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "edit_file read failed");
                return Ok(edit_error(&path, e.to_string()));
            }
        };

        let occurrences = content.matches(&find).count();
        if occurrences == 0 {
            return Ok(edit_error(&path, constants::KEY_FIND_NOT_FOUND.to_string()));
        }

        // `replacen` stops at whatever is there, so reporting the *requested* count (as an
        // earlier revision did) overstates the edit whenever fewer matches exist.
        let (new_content, replaced) = if count == 0 {
            (content.replace(&find, &replace), occurrences)
        } else {
            (
                content.replacen(&find, &replace, count),
                count.min(occurrences),
            )
        };

        if let Err(e) = tokio::fs::write(&resolved, &new_content).await {
            tracing::warn!(path = %path, error = %e, "edit_file write failed");
            return Ok(edit_error(&path, e.to_string()));
        }

        Ok(serde_json::json!({
            constants::KEY_PATH: &path,
            constants::KEY_REPLACED: replaced,
            constants::KEY_SIZE_BYTES: new_content.len(),
        })
        .to_string())
    }

    #[tool(
        "Install a Python package using pip. With session_id, installs into that session's \
         isolated packages directory, which is already on its sys.path — no restart needed. \
         Without session_id, installs globally for all sessions."
    )]
    async fn install_package(
        &self,
        package: String,
        session_id: Option<String>,
    ) -> McpResult<String> {
        let target = match session_id.as_ref() {
            Some(sid) => {
                let sessions = self.sessions.lock().await;
                match sessions.get(sid) {
                    Some(session) => Some(session.lock().await.packages_path()),
                    None => {
                        return Err(McpError::invalid_params(format!(
                            "no such session: {sid}. Omit session_id to install globally."
                        )));
                    }
                }
            }
            None => None,
        };

        let result = match target {
            Some(ref t) => {
                sandbox::run_pip_install_to_target(&package, &self.config.system_python, t).await
            }
            None => sandbox::run_pip_install(&package, &self.config.system_python).await,
        };

        match result {
            Ok(pip) => Ok(serde_json::to_string(&pip)
                .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())),
            Err(e) => Err(McpError::invalid_params(e.to_string())),
        }
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

    /// Look up a session, creating it if needed. Returns the shared handle and whether it is new.
    async fn acquire(&self, sid: &str) -> McpResult<(SharedSession, bool)> {
        let mut sessions = self.sessions.lock().await;
        if let Some(existing) = sessions.get(sid) {
            tracing::debug!(session_id = %sid, "Reusing existing session");
            return Ok((Arc::clone(existing), false));
        }

        if sessions.len() >= self.config.max_sessions {
            // Refuse rather than evict: someone else's live namespace is not ours to discard.
            return Err(McpError::internal(format!(
                "session limit reached ({} active). Call reset_python_session on a session you \
                 no longer need.",
                self.config.max_sessions
            )));
        }

        tracing::info!(session_id = %sid, "Creating session");
        let session = sandbox::Session::new(sid.to_string(), &self.wrapper_path, &self.config)
            .map_err(|e| {
                tracing::error!(session_id = %sid, error = %e, "Failed to create session");
                McpError::internal(format!("Failed to create sandbox session: {e}"))
            })?;
        let shared = Arc::new(Mutex::new(session));
        sessions.insert(sid.to_string(), Arc::clone(&shared));
        Ok((shared, true))
    }

    /// Resolve a caller path against the workspace allowlist, or produce the refusal payload.
    fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        workspace::resolve(path, &self.config.workspace_roots).map_err(|e| {
            tracing::warn!(path = %path, error = %e, "Refused a path outside the workspace");
            file_error(path, e.to_string())
        })
    }

    async fn cleanup_expired(&self) {
        let mut sessions = self.sessions.lock().await;
        let before = sessions.len();
        sessions.retain(|_sid, session| match session.try_lock() {
            // Busy sessions are by definition not idle.
            Err(_) => true,
            Ok(guard) => guard.idle_seconds() < constants::SESSION_IDLE_SECONDS,
        });
        let removed = before.saturating_sub(sessions.len());
        if removed > 0 {
            tracing::info!(removed = removed, "Cleaned up expired sessions");
        }
    }

    /// Reap idle sessions on a timer.
    ///
    /// Cleanup used to run only inside `execute_python`, so an abandoned session kept its
    /// subprocess and temp directory until some unrelated call happened to come in.
    pub fn spawn_reaper(&self) -> tokio::task::JoinHandle<()> {
        let server = self.clone();
        tokio::spawn(async move {
            let period = std::time::Duration::from_secs(constants::CLEANUP_INTERVAL_SECS);
            let mut ticker = tokio::time::interval(period);
            ticker.tick().await; // fires immediately; skip it
            loop {
                ticker.tick().await;
                server.cleanup_expired().await;
            }
        })
    }
}

fn file_error(path: &str, message: String) -> String {
    serde_json::json!({
        constants::KEY_PATH: path,
        constants::KEY_ERROR: message,
    })
    .to_string()
}

fn edit_error(path: &str, message: String) -> String {
    serde_json::json!({
        constants::KEY_PATH: path,
        constants::KEY_ERROR: message,
        constants::KEY_REPLACED: 0,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::{Service, ServiceExt};
    use turbomcp::{JsonRpcMessage, JsonRpcRequest, VersionDispatcher};

    /// `python3` on POSIX, `python` on the Windows dev machines. Tests that need an interpreter
    /// skip themselves if neither is present.
    fn find_python() -> Option<String> {
        ["python3", "python"]
            .into_iter()
            .find(|c| which::which(c).is_ok())
            .map(str::to_string)
    }

    struct Harness {
        _root: tempfile::TempDir,
        root: PathBuf,
        dispatcher: VersionDispatcher<InterpreterServer>,
    }

    fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let config = Config {
            bind_addr: "127.0.0.1:0".into(),
            sandbox_enabled: false,
            sandbox_required: false,
            nsjail_path: "nsjail".into(),
            sandbox_python: "python3".into(),
            system_python: find_python().unwrap_or_else(|| "python3".into()),
            wrapper_path: std::env::current_dir().unwrap().join("sandbox/wrapper.py"),
            sandbox_time_limit_secs: 300,
            sandbox_memory_limit_bytes: 512 * 1024 * 1024,
            exec_timeout_secs: 30,
            max_sessions: 4,
            workspace_roots: vec![root.clone()],
            worker_memory_limit_bytes: constants::WORKER_MEMORY_LIMIT_BYTES,
            worker_cpu_limit_secs: constants::WORKER_CPU_LIMIT_SECS,
            worker_file_limit_bytes: constants::WORKER_FILE_LIMIT_BYTES,
            worker_process_limit: constants::WORKER_PROCESS_LIMIT,
            log_level: "debug".into(),
        };
        Harness {
            _root: dir,
            root,
            dispatcher: InterpreterServer::new(config).into_server().build(),
        }
    }

    fn draft_meta() -> serde_json::Value {
        serde_json::json!({ "io.modelcontextprotocol/protocolVersion": "2026-07-28" })
    }

    async fn request(
        svc: &mut VersionDispatcher<InterpreterServer>,
        method: &str,
        params: serde_json::Value,
    ) -> turbomcp::JsonRpcResponse {
        let req = JsonRpcRequest::new(1, method, Some(params));
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
        r
    }

    async fn call_tool(
        svc: &mut VersionDispatcher<InterpreterServer>,
        name: &str,
        arguments: serde_json::Value,
    ) -> serde_json::Value {
        let r = request(
            svc,
            "tools/call",
            serde_json::json!({ "name": name, "arguments": arguments, "_meta": draft_meta() }),
        )
        .await;
        r.result.expect("tool result")
    }

    fn text(result: &serde_json::Value) -> &str {
        result["content"][0]["text"].as_str().unwrap()
    }

    fn parsed(result: &serde_json::Value) -> serde_json::Value {
        serde_json::from_str(text(result)).expect("tool payload should be JSON")
    }

    /// The message a refused tool call produced.
    ///
    /// A tool that returns `Err` does not become a JSON-RPC error: per the MCP spec turbomcp
    /// reports it as a successful response carrying `isError` content. Asserting on
    /// `response.error` alone therefore passes vacuously, so this checks both channels and
    /// insists the call actually failed.
    async fn failure_message(
        svc: &mut VersionDispatcher<InterpreterServer>,
        name: &str,
        arguments: serde_json::Value,
    ) -> String {
        let r = request(
            svc,
            "tools/call",
            serde_json::json!({ "name": name, "arguments": arguments, "_meta": draft_meta() }),
        )
        .await;

        if let Some(err) = r.error {
            return serde_json::to_string(&err).unwrap();
        }
        let result = r.result.expect("either an error or a result");
        assert_eq!(
            result["isError"], true,
            "expected the call to be refused, got {result}"
        );
        text(&result).to_string()
    }

    #[tokio::test]
    async fn list_tools_returns_all_nine() {
        let mut h = harness();
        let r = request(
            &mut h.dispatcher,
            "tools/list",
            serde_json::json!({ "_meta": draft_meta() }),
        )
        .await;
        let result = r.result.expect("tools list result");
        let tools = result["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for expected in [
            "execute_python",
            "reset_python_session",
            "list_python_sessions",
            "read_file",
            "write_file",
            "edit_file",
            "install_package",
            "list_packages",
            "get_python_info",
        ] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }
        assert_eq!(tools.len(), 9);
    }

    #[tokio::test]
    async fn write_and_read_file_inside_the_workspace() {
        let mut h = harness();
        let result = call_tool(
            &mut h.dispatcher,
            "write_file",
            serde_json::json!({"path": "notes/hello.txt", "content": "hello from test"}),
        )
        .await;
        assert_eq!(parsed(&result)[constants::KEY_WRITTEN], true);
        assert!(h.root.join("notes").join("hello.txt").exists());

        let result = call_tool(
            &mut h.dispatcher,
            "read_file",
            serde_json::json!({"path": "notes/hello.txt"}),
        )
        .await;
        assert_eq!(parsed(&result)[constants::KEY_CONTENT], "hello from test");
    }

    /// The containment guarantee, exercised through the tool surface rather than the helper:
    /// these tools run in the server process, so an unchecked path would be a way around
    /// Liberado's zone and write-class model.
    #[tokio::test]
    async fn file_tools_refuse_paths_outside_the_workspace() {
        let mut h = harness();
        let outside = if cfg!(windows) {
            "C:\\Windows\\System32\\drivers\\etc\\hosts"
        } else {
            "/etc/passwd"
        };

        for (tool, args) in [
            ("read_file", serde_json::json!({"path": outside})),
            (
                "write_file",
                serde_json::json!({"path": outside, "content": "pwned"}),
            ),
            (
                "edit_file",
                serde_json::json!({"path": outside, "find": "a", "replace": "b"}),
            ),
        ] {
            let result = call_tool(&mut h.dispatcher, tool, args).await;
            let payload = parsed(&result);
            assert!(
                payload[constants::KEY_ERROR]
                    .as_str()
                    .unwrap_or_default()
                    .contains("outside the permitted workspace"),
                "{tool} should have refused {outside}, got {payload}"
            );
        }
    }

    #[tokio::test]
    async fn write_file_refuses_dot_dot_escape() {
        let mut h = harness();
        let result = call_tool(
            &mut h.dispatcher,
            "write_file",
            serde_json::json!({"path": "../escaped.txt", "content": "nope"}),
        )
        .await;
        assert!(parsed(&result)[constants::KEY_ERROR]
            .as_str()
            .unwrap()
            .contains("outside the permitted workspace"));
        assert!(!h.root.parent().unwrap().join("escaped.txt").exists());
    }

    #[tokio::test]
    async fn read_file_missing() {
        let mut h = harness();
        let result = call_tool(
            &mut h.dispatcher,
            "read_file",
            serde_json::json!({"path": "nope.txt"}),
        )
        .await;
        assert!(parsed(&result)[constants::KEY_ERROR].is_string());
    }

    #[tokio::test]
    async fn edit_file_replaces_text() {
        let mut h = harness();
        std::fs::write(h.root.join("edit.txt"), "original line\n").unwrap();

        let result = call_tool(
            &mut h.dispatcher,
            "edit_file",
            serde_json::json!({"path": "edit.txt", "find": "original", "replace": "modified"}),
        )
        .await;
        assert_eq!(parsed(&result)[constants::KEY_REPLACED], 1);
        assert_eq!(
            std::fs::read_to_string(h.root.join("edit.txt")).unwrap(),
            "modified line\n"
        );
    }

    /// `replacen(.., 5)` on two matches replaces two. Reporting the requested count told the
    /// caller five edits had been made.
    #[tokio::test]
    async fn edit_file_reports_actual_replacement_count() {
        let mut h = harness();
        std::fs::write(h.root.join("many.txt"), "a a\n").unwrap();

        let result = call_tool(
            &mut h.dispatcher,
            "edit_file",
            serde_json::json!({"path": "many.txt", "find": "a", "replace": "b", "count": 5}),
        )
        .await;
        assert_eq!(parsed(&result)[constants::KEY_REPLACED], 2);
    }

    #[tokio::test]
    async fn edit_file_count_zero_replaces_all() {
        let mut h = harness();
        std::fs::write(h.root.join("all.txt"), "x x x\n").unwrap();

        let result = call_tool(
            &mut h.dispatcher,
            "edit_file",
            serde_json::json!({"path": "all.txt", "find": "x", "replace": "y", "count": 0}),
        )
        .await;
        assert_eq!(parsed(&result)[constants::KEY_REPLACED], 3);
        assert_eq!(
            std::fs::read_to_string(h.root.join("all.txt")).unwrap(),
            "y y y\n"
        );
    }

    #[tokio::test]
    async fn install_package_rejects_option_shaped_names() {
        let mut h = harness();
        let message = failure_message(
            &mut h.dispatcher,
            "install_package",
            serde_json::json!({"package": "--index-url=http://example.invalid/simple"}),
        )
        .await;
        assert!(
            message.contains("invalid package name"),
            "an option-shaped package name must not reach pip, got {message:?}"
        );
    }

    #[tokio::test]
    async fn install_package_rejects_unknown_session() {
        let mut h = harness();
        // Falling back to a global install (as an earlier revision did) puts the package
        // somewhere the caller did not ask for and still reports success.
        let message = failure_message(
            &mut h.dispatcher,
            "install_package",
            serde_json::json!({"package": "six", "session_id": "no-such-session"}),
        )
        .await;
        assert!(
            message.contains("no such session"),
            "unknown session should be refused, got {message:?}"
        );
    }

    #[tokio::test]
    async fn get_python_info_returns_valid_json() {
        if find_python().is_none() {
            return;
        }
        let mut h = harness();
        let result = call_tool(&mut h.dispatcher, "get_python_info", serde_json::json!({})).await;
        assert!(parsed(&result).get("version").is_some());
    }

    #[tokio::test]
    async fn list_packages_returns_valid_json() {
        if find_python().is_none() {
            return;
        }
        let mut h = harness();
        let result = call_tool(&mut h.dispatcher, "list_packages", serde_json::json!({})).await;
        assert!(parsed(&result).get("returncode").is_some());
    }

    #[tokio::test]
    async fn execute_python_stateful_session() {
        if find_python().is_none() {
            return;
        }
        let mut h = harness();
        let result = call_tool(
            &mut h.dispatcher,
            "execute_python",
            serde_json::json!({"code": "x = [1, 2, 3]"}),
        )
        .await;
        let value = parsed(&result);
        let sid = value[constants::KEY_SESSION_ID]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(value[constants::KEY_CREATED], true);
        assert_eq!(value[constants::KEY_SANDBOXED], false);

        let result = call_tool(
            &mut h.dispatcher,
            "execute_python",
            serde_json::json!({"code": "sum(x)", "session_id": &sid}),
        )
        .await;
        let value = parsed(&result);
        assert_eq!(value[constants::KEY_CREATED], false);
        assert!(value[constants::PROTO_STDOUT]
            .as_str()
            .unwrap()
            .contains("6"));
    }

    /// Multi-statement input used to fail outright: compiling in "single" mode raises
    /// "multiple statements found", and the exec fallback was gated on the incomplete-input flag
    /// so it never ran. Agents send multi-line code constantly.
    #[tokio::test]
    async fn execute_python_runs_multi_statement_code() {
        if find_python().is_none() {
            return;
        }
        let mut h = harness();
        let result = call_tool(
            &mut h.dispatcher,
            "execute_python",
            serde_json::json!({"code": "import math\nr = 2\nmath.pi * r ** 2"}),
        )
        .await;
        let value = parsed(&result);
        assert_eq!(value[constants::PROTO_STDERR], "");
        assert!(
            value[constants::PROTO_STDOUT]
                .as_str()
                .unwrap()
                .starts_with("12.56"),
            "expected the trailing expression to be echoed, got {value}"
        );
    }

    #[tokio::test]
    async fn execute_python_reports_incomplete_input() {
        if find_python().is_none() {
            return;
        }
        let mut h = harness();
        let result = call_tool(
            &mut h.dispatcher,
            "execute_python",
            serde_json::json!({"code": "if True:"}),
        )
        .await;
        assert_eq!(parsed(&result)[constants::PROTO_MORE_INPUT], true);
    }

    #[tokio::test]
    async fn execute_python_surfaces_a_traceback_without_killing_the_session() {
        if find_python().is_none() {
            return;
        }
        let mut h = harness();
        let result = call_tool(
            &mut h.dispatcher,
            "execute_python",
            serde_json::json!({"code": "1/0"}),
        )
        .await;
        let value = parsed(&result);
        let sid = value[constants::KEY_SESSION_ID]
            .as_str()
            .unwrap()
            .to_string();
        assert!(value[constants::PROTO_STDERR]
            .as_str()
            .unwrap()
            .contains("ZeroDivisionError"));

        let result = call_tool(
            &mut h.dispatcher,
            "execute_python",
            serde_json::json!({"code": "'still alive'", "session_id": &sid}),
        )
        .await;
        assert!(parsed(&result)[constants::PROTO_STDOUT]
            .as_str()
            .unwrap()
            .contains("still alive"));
    }

    /// Without a wall-clock ceiling this call never returns, and under the old global map lock
    /// it took every other session down with it.
    #[tokio::test]
    async fn execute_python_times_out_and_retires_the_session() {
        if find_python().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let config = Config {
            exec_timeout_secs: 2,
            workspace_roots: vec![root],
            system_python: find_python().unwrap(),
            wrapper_path: std::env::current_dir().unwrap().join("sandbox/wrapper.py"),
            bind_addr: "127.0.0.1:0".into(),
            sandbox_enabled: false,
            sandbox_required: false,
            nsjail_path: "nsjail".into(),
            sandbox_python: "python3".into(),
            sandbox_time_limit_secs: 300,
            sandbox_memory_limit_bytes: 512 * 1024 * 1024,
            max_sessions: 4,
            worker_memory_limit_bytes: constants::WORKER_MEMORY_LIMIT_BYTES,
            worker_cpu_limit_secs: constants::WORKER_CPU_LIMIT_SECS,
            worker_file_limit_bytes: constants::WORKER_FILE_LIMIT_BYTES,
            worker_process_limit: constants::WORKER_PROCESS_LIMIT,
            log_level: "debug".into(),
        };
        let server = InterpreterServer::new(config);
        let mut dispatcher = server.clone().into_server().build();

        let message = failure_message(
            &mut dispatcher,
            "execute_python",
            serde_json::json!({"code": "while True:\n    pass"}),
        )
        .await;
        assert!(
            message.contains("time limit"),
            "a non-terminating program must hit the timeout, got {message:?}"
        );

        // And the wedged worker must not be left in the map for the next caller to inherit.
        assert!(
            server.sessions.lock().await.is_empty(),
            "a timed-out session must be retired, not reused"
        );
    }

    #[tokio::test]
    async fn session_limit_is_enforced() {
        if find_python().is_none() {
            return;
        }
        let mut h = harness(); // max_sessions = 4
        for _ in 0..4 {
            let result = call_tool(
                &mut h.dispatcher,
                "execute_python",
                serde_json::json!({"code": "1"}),
            )
            .await;
            assert!(parsed(&result)[constants::KEY_SESSION_ID].is_string());
        }
        let message = failure_message(
            &mut h.dispatcher,
            "execute_python",
            serde_json::json!({"code": "1"}),
        )
        .await;
        assert!(
            message.contains("session limit"),
            "the 5th session should be refused, not spawned; got {message:?}"
        );
    }

    #[tokio::test]
    async fn reset_python_session_reports_whether_it_existed() {
        let mut h = harness();
        let result = call_tool(
            &mut h.dispatcher,
            "reset_python_session",
            serde_json::json!({"session_id": "never-created"}),
        )
        .await;
        assert_eq!(parsed(&result)[constants::KEY_RESET], false);
    }

    #[tokio::test]
    async fn list_python_sessions_is_empty_at_rest() {
        let mut h = harness();
        let result = call_tool(
            &mut h.dispatcher,
            "list_python_sessions",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(parsed(&result), serde_json::json!([]));
    }
}

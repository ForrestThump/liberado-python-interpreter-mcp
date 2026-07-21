import code
import io
import os
import subprocess
import sys
import threading
import time
import uuid
from contextlib import redirect_stderr, redirect_stdout

MAX_OUTPUT_BYTES = 50000
DEFAULT_TIMEOUT = 30
SESSION_IDLE_SECONDS = 1800


class PythonSession:
    def __init__(self, session_id: str):
        self.session_id = session_id
        self.namespace: dict = {}
        self.interpreter = code.InteractiveInterpreter(locals=self.namespace)
        self.output_lines: list[str] = []
        self.created_at = time.time()
        self.last_used = time.time()

    def run(self, code_str: str) -> dict:
        self.last_used = time.time()
        stdout_buf = io.StringIO()
        stderr_buf = io.StringIO()
        more = False

        try:
            with redirect_stdout(stdout_buf), redirect_stderr(stderr_buf):
                more = self.interpreter.runsource(code_str, "<mcp>", "single")
                if more:
                    stderr_buf.truncate(0)
                    stderr_buf.seek(0)
                    more = self.interpreter.runsource(code_str, "<mcp>", "exec")
        except Exception as e:
            return {
                "session_id": self.session_id,
                "stdout": stdout_buf.getvalue(),
                "stderr": stderr_buf.getvalue() + f"\n{type(e).__name__}: {e}",
                "more_input_needed": False,
            }

        stdout = stdout_buf.getvalue()
        stderr = stderr_buf.getvalue()

        return {
            "session_id": self.session_id,
            "stdout": stdout[:MAX_OUTPUT_BYTES],
            "stderr": stderr[:MAX_OUTPUT_BYTES],
            "more_input_needed": more,
            "truncated_stdout": len(stdout) > MAX_OUTPUT_BYTES,
            "truncated_stderr": len(stderr) > MAX_OUTPUT_BYTES,
        }

    def inject(self, name: str, value) -> None:
        self.namespace[name] = value
        self.last_used = time.time()


class SessionManager:
    def __init__(self):
        self._sessions: dict[str, PythonSession] = {}
        self._lock = threading.Lock()

    def get_or_create(self, session_id: str | None) -> tuple[PythonSession, bool]:
        with self._lock:
            if session_id is None:
                session_id = uuid.uuid4().hex[:12]
                created = True
            elif session_id in self._sessions:
                created = False
            else:
                created = True

            if session_id not in self._sessions:
                self._sessions[session_id] = PythonSession(session_id)

            return self._sessions[session_id], created

    def reset(self, session_id: str) -> bool:
        with self._lock:
            if session_id in self._sessions:
                del self._sessions[session_id]
                return True
            return False

    def list_sessions(self) -> list[dict]:
        with self._lock:
            now = time.time()
            return [
                {
                    "session_id": s.session_id,
                    "var_count": len(s.namespace),
                    "vars": list(s.namespace.keys())[:50],
                    "created_at": s.created_at,
                    "seconds_idle": now - s.last_used,
                }
                for s in self._sessions.values()
            ]

    def cleanup_idle(self) -> int:
        removed = 0
        with self._lock:
            now = time.time()
            stale = [
                sid
                for sid, s in self._sessions.items()
                if now - s.last_used > SESSION_IDLE_SECONDS
            ]
            for sid in stale:
                del self._sessions[sid]
                removed += 1
        return removed


_session_manager = SessionManager()


def execute_code(session_id: str | None, code_str: str) -> dict:
    session, created = _session_manager.get_or_create(session_id)
    result = session.run(code_str)
    result["session_id"] = session.session_id
    result["created"] = created
    return result


def reset_session(session_id: str) -> dict:
    ok = _session_manager.reset(session_id)
    return {"session_id": session_id, "reset": ok}


def list_sessions() -> list[dict]:
    return _session_manager.list_sessions()


def cleanup_sessions() -> int:
    return _session_manager.cleanup_idle()


def read_file(path: str) -> dict:
    try:
        path = os.path.normpath(path)
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            content = f.read()
        return {
            "path": path,
            "content": content[:MAX_OUTPUT_BYTES],
            "size_bytes": len(content),
            "truncated": len(content) > MAX_OUTPUT_BYTES,
        }
    except FileNotFoundError:
        return {"path": path, "error": "File not found"}
    except PermissionError:
        return {"path": path, "error": "Permission denied"}
    except IsADirectoryError:
        return {"path": path, "error": "Path is a directory"}
    except OSError as e:
        return {"path": path, "error": str(e)}


def write_file(path: str, content: str) -> dict:
    try:
        path = os.path.normpath(path)
        os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(content)
        return {"path": path, "written": True, "size_bytes": len(content)}
    except PermissionError:
        return {"path": path, "error": "Permission denied"}
    except IsADirectoryError:
        return {"path": path, "error": "Path is a directory"}
    except OSError as e:
        return {"path": path, "error": str(e)}


def edit_file(path: str, find: str, replace: str, count: int = 1) -> dict:
    try:
        path = os.path.normpath(path)
        with open(path, "r", encoding="utf-8") as f:
            content = f.read()

        if find not in content:
            return {"path": path, "error": "Find string not found in file", "replaced": 0}

        replaced = 0
        if count == 0:
            new_content = content.replace(find, replace)
            replaced = content.count(find)
        else:
            new_content = content
            for _ in range(count):
                if find in new_content:
                    new_content = new_content.replace(find, replace, 1)
                    replaced += 1
                else:
                    break

        with open(path, "w", encoding="utf-8") as f:
            f.write(new_content)

        return {"path": path, "replaced": replaced, "size_bytes": len(new_content)}
    except FileNotFoundError:
        return {"path": path, "error": "File not found", "replaced": 0}
    except PermissionError:
        return {"path": path, "error": "Permission denied", "replaced": 0}
    except IsADirectoryError:
        return {"path": path, "error": "Path is a directory", "replaced": 0}
    except OSError as e:
        return {"path": path, "error": str(e), "replaced": 0}


def install_pip_package(package: str) -> dict:
    result = subprocess.run(
        [sys.executable, "-m", "pip", "install", package],
        capture_output=True,
        text=True,
        timeout=120,
    )
    return {
        "returncode": result.returncode,
        "stdout": result.stdout[:MAX_OUTPUT_BYTES],
        "stderr": result.stderr[:MAX_OUTPUT_BYTES],
    }


def list_pip_packages() -> dict:
    result = subprocess.run(
        [sys.executable, "-m", "pip", "list", "--format=json"],
        capture_output=True,
        text=True,
        timeout=30,
    )
    return {
        "returncode": result.returncode,
        "stdout": result.stdout[:MAX_OUTPUT_BYTES],
        "stderr": result.stderr[:MAX_OUTPUT_BYTES],
    }

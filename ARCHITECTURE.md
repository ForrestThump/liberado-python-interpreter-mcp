# ARCHITECTURE

## Overview

`liberado-python-interpreter-mcp` exposes a persistent Python REPL and file editing tools via MCP. It uses [turbomcp](https://pypi.org/project/turbomcp/) for protocol handling.

## Components

```
src/liberado_python_interpreter/
├── __init__.py       # Package exports (mcp instance)
├── __main__.py       # CLI entry point (stdio / HTTP modes)
├── server.py         # Tool definitions via @mcp.tool() decorators
└── interpreter.py    # Session REPL, file I/O, package management
```

## Session Model

Each session wraps a `code.InteractiveInterpreter` from the Python stdlib:

1. `execute_python(code, session_id=None)` — first call creates a session (returns the ID), subsequent calls reuse it
2. Namespace persists across calls within a session: imports, variables, functions all survive
3. Multi-line code (def, class, try/except) is handled by `runsource` — incomplete blocks return `more_input_needed: true`
4. Sessions idle for >30 minutes are auto-cleaned on the next call
5. `reset_python_session(session_id)` removes a session immediately
6. `list_python_sessions()` shows active sessions, variable counts, idle times

### Why `InteractiveInterpreter` instead of subprocess

- **State**: namespace lives in-process, no serialization needed
- **Speed**: no process spawn overhead per call
- **Multi-line**: `runsource` handles compound statements correctly
- **Output capture**: `redirect_stdout` / `redirect_stderr` wraps each call
- **Downside**: scripts can affect the server process (infinite loops, memory)

## File Operations

Three tools provide scoped file access from the server's working directory:

- `read_file(path)` — read any text file
- `write_file(path, content)` — create or overwrite, auto-creates parent dirs
- `edit_file(path, find, replace, count)` — string replacement editing

Combined with sessions, the workflow is: write a `.py` file, `exec(open('script.py').read())` in the session, then incrementally refine.

## Data Flow

```
MCP client (stdio)
    │ tools/call {"name": "execute_python", "arguments": {"code": "x=1", "session_id": "abc123"}}
    ▼
turbomcp dispatch
    │
    ▼
server.execute_python(code, session_id)
    │
    ▼
interpreter.execute_code(session_id, code)
    │
    ▼
SessionManager.get_or_create("abc123")
    │
    ▼
PythonSession.run("x=1")
    │ uses code.InteractiveInterpreter.runsource()
    │ captures stdout/stderr via redirect_*
    ▼
return {"session_id": "abc123", "stdout": "", "stderr": "", "more_input_needed": false, "created": false}
```

## Dependencies

- `turbomcp>=0.2.0` — MCP server framework, zero transitive deps
- Python stdlib only for execution logic (`code`, `io`, `subprocess`, `threading`)

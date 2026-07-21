# ARCHITECTURE

## Overview

`liberado-python-interpreter-mcp` is a Rust MCP server that manages sandboxed Python REPL sessions. Each session wraps a `python3` process running inside nsjail, communicating via stdin/stdout JSON protocol over pipes.

## Components

```
src/
├── main.rs              # HTTP transport setup, tracing init
├── lib.rs               # Module exports
├── server.rs            # 9 MCP tools via #[turbomcp::server] + #[tool] macros
└── sandbox.rs           # nsjail session lifecycle, pip subprocess calls
sandbox/
└── wrapper.py           # Python in-jail script; reads JSON from stdin, execs code, writes JSON to stdout
```

## Sandbox Architecture

```
┌─────────────────────────────────────────┐
│  Rust MCP Server (runs as root)         │
│                                         │
│  execute_python("code", "session_id")    │
│    │                                     │
│    │  SandboxSession.execute(code)       │
│    │    │                                 │
│    │    │  stdin ──► {"cmd":"exec",...}  │
│    │    │                                │
│    ▼    ▼                               │
│  ┌──────────────────────────────────┐   │
│  │  nsjail (CLONE_NEWNS/NET/PID)    │   │
│  │  ┌────────────────────────────┐  │   │
│  │  │  python3 -u wrapper.py     │  │   │
│  │  │                            │  │   │
│  │  │  InteractiveInterpreter    │  │   │
│  │  │    runsource("single")     │  │   │
│  │  │    fallback: "exec"        │  │   │
│  │  │                            │  │   │
│  │  │  stdout ──► JSON response  │  │   │
│  │  └────────────────────────────┘  │   │
│  │                                  │   │
│  │  bindmount: /tmp/sess-xxx → /work│   │
│  │  no /proc, no network            │   │
│  │  mem: 512MB, time: 300s          │   │
│  └──────────────────────────────────┘   │
└─────────────────────────────────────────┘
```

## Session Protocol

The Rust server and Python wrapper communicate via newline-delimited JSON over pipes:

**Request** (Rust → Python):
```json
{"cmd": "exec", "code": "x = 1"}
```

**Response** (Python → Rust):
```json
{"ok": true, "stdout": "", "stderr": "", "more_input_needed": false}
```

The `InteractiveInterpreter` inside nsjail processes code identically to the non-sandboxed path: `"single"` compile mode first, `"exec"` fallback for compound statements.

## Per-Session Package Isolation

Each session's work directory contains a `packages/` subdirectory. When `install_package(package, session_id)` is called:

1. Server looks up the session by ID
2. Runs `pip install --target <session_work_dir>/packages/ <package>`
3. The sandbox wrapper adds `/work/packages` to `sys.path` on startup

Packages installed with `session_id` are isolated to that session only. Installing without `session_id` (legacy) installs globally into the system Python.

## Observability

All operations are traced via the `tracing` crate at appropriate levels:

| Level | Events |
|---|---|
| `INFO` | Session creation/destruction, pip installs, expired session cleanup, server startup |
| `DEBUG` | Code execution previews, file I/O operations, session reuse, sandbox process teardown |
| `WARN` | Sandbox fallback, execution errors, file I/O failures, pip failures, missing sessions |
| `ERROR` | Session process death, session creation failures, pip command spawn failures |

Set `RUST_LOG=debug` for detailed per-request traces.

## Tool Flow

```
MCP client (streamable HTTP)
    │ tools/call {"name": "execute_python", "arguments": {"code":"x=1","session_id":"abc"}}
    ▼
turbomcp #[tool] dispatch
    │
    ▼
InterpreterServer::execute_python
    │
    ├─ session exists? → reuse child process
    └─ new? → find nsjail, check root, create TempDir, spawn nsjail + python
    │
    ▼
SandboxSession::execute(code)
    │
    ├─ write JSON request to stdin
    ├─ read JSON response from stdout
    ▼
return {"session_id":"abc","stdout":"","stderr":"","created":false}
```

## Dependencies

- `turbomcp` — MCP server framework + HTTP transport (streamable HTTP)
- `tokio` — async runtime, subprocess management, I/O
- `serde` / `serde_json` — JSON serialization for the sandbox protocol
- `tempfile` — session work directories
- `uuid` — session ID generation
- `nsjail` — external binary for Linux namespace isolation

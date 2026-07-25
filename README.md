# liberado-python-interpreter-mcp

A [Model Context Protocol](https://modelcontextprotocol.io/) server offering persistent Python REPL sessions, workspace file operations, and package management. Written in Rust on [TurboMCP](https://crates.io/crates/turbomcp), served over streamable HTTP.

Each session is a long-lived `python3` subprocess with its own namespace, its own package directory, and its own resource limits. Variables, imports, and definitions survive across calls.

## Tools

### REPL & sessions

| Tool | Description |
|---|---|
| `execute_python` | Run Python in a persistent session. Omit `session_id` to start one; pass it back to continue. |
| `reset_python_session` | Destroy a session and its subprocess. |
| `list_python_sessions` | Active sessions with age, idle time, and sandbox status. |

### Files

| Tool | Description |
|---|---|
| `read_file` | Read a text file from the workspace. |
| `write_file` | Write a file, creating parent directories. |
| `edit_file` | Find-and-replace. `count=0` replaces all; the response reports replacements *actually* made. |

### Packages & environment

| Tool | Description |
|---|---|
| `install_package` | pip install. With `session_id`, into that session's private directory; without, globally. |
| `list_packages` | Installed packages as JSON. |
| `get_python_info` | Version, executable, platform. |

## Isolation model

**The container is the sandbox.** This server executes model-authored code, so the boundary that matters is the one around the whole process:

| Control | Mechanism |
|---|---|
| Privileges | Runs as uid 10001, `cap_drop: ALL`, `no-new-privileges` |
| Filesystem | Read-only rootfs; only `/workspace` and a tmpfs `/tmp` are writable |
| Memory / CPU | Container limits, plus `RLIMIT_AS` and `RLIMIT_CPU` on each worker |
| Processes | Container `pids_limit`, plus `RLIMIT_NPROC` per worker |
| Wall clock | `LIBERADO_EXEC_TIMEOUT` per call; the session is terminated when it trips |
| File size | `RLIMIT_FSIZE` per worker |
| Blast radius | Its own container — it mounts no vault and holds no credentials |

nsjail-per-session is supported but **off by default**, because it is not packaged for Debian and needs `CAP_SYS_ADMIN` under Docker — granting that to weaken the outer boundary in order to add an inner one is a poor trade for this threat model. On a host that does have nsjail, set `LIBERADO_SANDBOX_ENABLED=1`; add `LIBERADO_SANDBOX_REQUIRED=1` to refuse to start rather than fall back. Every `execute_python` response carries a `sandboxed` field stating which mode actually ran.

### Workspace containment

The file tools run in the *server* process, not inside a session. Every path they receive is resolved against `LIBERADO_WORKSPACE_ROOTS` before any I/O: `..` is collapsed lexically, symlinks are resolved, and anything landing outside the roots is refused. Relative paths resolve against the first root, so `write_file("out.csv")` means `/workspace/out.csv`.

This is what keeps the interpreter from becoming a way around a host system's own permission model — under Liberado, an agent denied a vault write must not be able to ask the interpreter to perform it instead.

## Running

### Docker

```sh
docker build -t liberado-python-interpreter-mcp .
docker run --rm -p 8004:8000 \
  --read-only --tmpfs /tmp --cap-drop ALL \
  --security-opt no-new-privileges \
  -v python-interpreter-workspace:/workspace \
  liberado-python-interpreter-mcp
```

### Local development

```sh
just run                      # or: cargo run
BIND_ADDR=127.0.0.1:9000 cargo run
just test                     # cargo test
just ci                       # lint + fmt + test + build
```

Locally, set `LIBERADO_WORKSPACE_ROOTS` to a directory you are happy for the tools to write to; it defaults to `/workspace`, which will not exist on a dev machine and is rejected at startup.

## Configuration

| Variable | Default | Description |
|---|---|---|
| `BIND_ADDR` | `0.0.0.0:8000` | HTTP listen address |
| `RUST_LOG` | `info` | Tracing filter |
| `LIBERADO_WORKSPACE_ROOTS` | `/workspace` | `:`-separated absolute roots the file tools may touch |
| `LIBERADO_EXEC_TIMEOUT` | `120` | Seconds before a single execution is killed |
| `LIBERADO_MAX_SESSIONS` | `32` | Concurrent sessions before new ones are refused |
| `LIBERADO_SANDBOX_ENABLED` | `1` (`0` in the image) | Use nsjail per session when available |
| `LIBERADO_SANDBOX_REQUIRED` | `0` | Fail instead of falling back when nsjail is unavailable |
| `NSJAIL_PATH` | `nsjail` | nsjail binary |
| `SANDBOX_PYTHON` | `python3` | Interpreter inside the jail |
| `SYSTEM_PYTHON` | `python3` | Interpreter for sessions and pip (`/opt/venv/bin/python` in the image) |
| `LIBERADO_WRAPPER_PATH` | next to the binary | Path to `sandbox/wrapper.py` |
| `LIBERADO_SANDBOX_TIME_LIMIT` | `300` | nsjail `--time_limit`, seconds |
| `LIBERADO_SANDBOX_MEMORY_LIMIT` | `536870912` | nsjail `--cgroup_mem_max`, bytes |

Malformed configuration is a **startup failure**, not a silent default: an unparseable boolean, a zero timeout, a relative workspace root, or a missing wrapper script all refuse to boot rather than surfacing as confusing tool errors later.

## Session semantics

```
> execute_python(code="x = [1, 2, 3]")
  -> {"session_id": "…", "created": true, "sandboxed": false, "stdout": ""}

> execute_python(code="sum(x)", session_id="…")
  -> {"stdout": "6\n", "created": false}
```

- **Multi-statement input works**, and a trailing bare expression has its value echoed — `import math\nr = 2\nmath.pi * r ** 2` prints the area.
- **Incomplete input** (`if True:`) returns `more_input_needed: true` rather than an error.
- **Exceptions** come back as a traceback in `stderr`; the session stays alive.
- **Timeouts** kill the worker and retire the session, because a late response would otherwise be read as the answer to somebody else's next call.
- `input()` raises `EOFError`; stdin belongs to the session protocol.
- Sessions idle for 30 minutes are reaped by a background task.

### Per-session packages

```
> install_package(package="requests", session_id="abc")
  -> {"returncode": 0}       # installed to that session's packages dir only

> execute_python(code="import requests; requests.__version__", session_id="abc")
  -> {"stdout": "'2.32.4'\n"}
```

The directory is on the worker's `sys.path` from startup, so no restart is needed. Package names that pip would read as options (`--index-url=…`) are rejected.

## MCP endpoint

```
http://<host>:8000/
```

## License

MIT

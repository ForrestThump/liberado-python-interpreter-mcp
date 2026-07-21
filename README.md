# liberado-python-interpreter-mcp

A [Model Context Protocol](https://modelcontextprotocol.io/) server providing a sandboxed persistent Python REPL with file operations and package management. Built in Rust using [TurboMCP](https://crates.io/crates/turbomcp). Each session runs inside an nsjail-isolated subprocess with no network, no `/proc`, and memory/time limits.

## Tools

### REPL & Sessions

Tool | Description
--- | ---
`execute_python` | Execute Python code in a persistent REPL session. Variables, imports, and functions survive across calls within the same session. Omit `session_id` to create a new session.
`reset_python_session` | Destroy a session, tearing down its nsjail subprocess and temp directory.
`list_python_sessions` | List all active sessions with idle times. Sessions idle for >30 minutes are auto-cleaned.

### File Operations

Tool | Description
--- | ---
`read_file` | Read a text file from the host filesystem.
`write_file` | Write content to a file, creating parent directories as needed.
`edit_file` | Find-and-replace text in a file. `count=0` replaces all occurrences.

### Package & Environment

Tool | Description
--- | ---
`install_package` | Install a Python package via pip. Pass `session_id` to install into that session's isolated packages directory (use `--target` under the session's work dir, auto-added to `sys.path`). Omit `session_id` for a global install.
`list_packages` | List all installed Python packages in JSON format.
`get_python_info` | Get Python version, executable path, and platform.

## Sandbox

Each `execute_python` session runs Python inside [nsjail](https://github.com/google/nsjail) with:

| Constraint | Value |
|---|---|
| Filesystem | Writable only in the session's temp directory (`/work`). Host rootfs is read-only via `--chroot /`. |
| Network | Disabled (`--iface_no_lo`) |
| Process info | `/proc` unavailable (`--disable_proc`) |
| Memory | 512 MB limit (`--cgroup_mem_max`) |
| Time | 300s per nsjail subprocess lifetime |

Sandbox mode requires running as root (or `CAP_SYS_ADMIN`) for Linux namespace creation. If nsjail is not found or the server is not root, session creation returns a clear error. Set `NSJAIL_PATH` to specify the nsjail binary location, or `SANDBOX_PYTHON` to use a different Python executable inside the sandbox.

## Running

### Docker (recommended, includes nsjail)

```sh
docker build -t liberado-python-interpreter-mcp .
docker run --privileged -p 8000:8000 liberado-python-interpreter-mcp
```

### Local development

```sh
cargo run
# custom bind address:
BIND_ADDR=127.0.0.1:9000 cargo run
```

### Without sandbox (skip nsjail requirement)

```sh
cargo run --no-default-features
```

### Tests

```sh
cargo test --lib
```

## MCP endpoint

```
http://<host>:8000/
```

Registered in OpenClaw and LibreChat as `liberado-python-interpreter-mcp` (streamable-http).

## Session Usage

```
> execute_python(code="x = [1, 2, 3]")
  -> {"session_id": "...", "created": true, "stdout": ""}

> execute_python(code="sum(x)", session_id="...")
  -> {"session_id": "...", "created": false, "stdout": "6\n"}
```

### Per-session Package Isolation

Pass a `session_id` to `install_package` to install into that session only. Packages go to `<session_work_dir>/packages/` and are automatically added to `sys.path` by the sandbox wrapper.

```
> execute_python(code="import requests", session_id="abc")
  -> {"stderr": "ModuleNotFoundError: ..."}

> install_package(package="requests", session_id="abc")
  -> {"returncode": 0}      # installed to session's packages dir only

> execute_python(code="import requests; print(requests.__version__)", session_id="abc")
  -> {"stdout": "2.32.4\n"}  # success — session restarted, picks up new path
```

Install without `session_id` for global packages available to all sessions.

## Configuration

| Variable | Default | Description |
|---|---|---|
| `BIND_ADDR` | `0.0.0.0:8000` | HTTP listen address |
| `RUST_LOG` | `info` | Tracing log level |
| `NSJAIL_PATH` | `nsjail` | Path to nsjail binary |
| `SANDBOX_PYTHON` | `python3` | Python executable inside sandbox |
| `SYSTEM_PYTHON` | `python3` | Python for pip operations (outside sandbox) |
| `LIBERADO_WRAPPER_PATH` | `sandbox/wrapper.py` | Path to the sandbox wrapper script |
| `LIBERADO_SANDBOX_TIME_LIMIT` | `300` | Sandbox time limit in seconds |
| `LIBERADO_SANDBOX_MEMORY_LIMIT` | `536870912` | Sandbox memory limit in bytes (512 MB) |
| `LIBERADO_SANDBOX_ENABLED` | `1` | Enable nsjail sandbox (`0`/`false`/`no` disables, runs Python directly) |

## Running Without Sandbox

Set `LIBERADO_SANDBOX_ENABLED=0` to skip nsjail and run Python directly via subprocess. Useful for local development without root. The server also auto-falls back to unsafe mode if nsjail is missing or unavailable.

```sh
LIBERADO_SANDBOX_ENABLED=0 cargo run
```

## License

MIT

# ARCHITECTURE

## Overview

`liberado-python-interpreter-mcp` manages persistent Python REPL sessions behind an MCP server. Each session is a `python3` subprocess speaking newline-delimited JSON over pipes; the Rust side owns its lifetime, its resource limits, and the wall clock.

```
src/
├── main.rs         # config load, isolation probe, HTTP transport, session reaper
├── lib.rs          # module exports
├── config.rs       # environment parsing with fail-at-boot validation
├── constants.rs    # every literal, in one place
├── server.rs       # the 9 MCP tools
├── sandbox.rs      # session lifecycle, the wire protocol, pip
└── workspace.rs    # path containment for the file tools
sandbox/
└── wrapper.py      # the in-session worker
```

## Isolation model

The container is the security boundary. See README for the control table; the reasoning is:

nsjail gives per-session namespaces, but it is not packaged for Debian and requires `CAP_SYS_ADMIN` inside Docker. Granting `CAP_SYS_ADMIN` to a container that runs model-authored code is close to granting container escape. Trading a strong outer boundary for a weaker one plus an inner boundary is a net loss here, so the deployment runs sessions as direct children of an unprivileged, capability-less, read-only-rootfs container, and reserves nsjail for hosts where it is genuinely available.

The nsjail path is kept working rather than deleted (`LIBERADO_SANDBOX_ENABLED=1`), and the choice is *observable*: the effective mode is logged at boot and returned in the `sandboxed` field of every `execute_python` response. An earlier revision fell back silently, so a caller could believe its code was jailed when it was not.

## Session protocol

```
Rust → Python   {"cmd": "exec", "code": "x = 1"}
Python → Rust   {"ok": true, "stdout": "", "stderr": "", "more_input_needed": false}
```

**Exactly one response per request.** The invariant is load-bearing: if a request ever went unanswered, or a response arrived after its request had been abandoned, every subsequent read would pair with the wrong command and the session would silently return other people's output. Three things protect it:

1. The worker replies to malformed and unknown commands rather than skipping them.
2. `input()` cannot reach the protocol stream — `sys.stdin` is swapped for an empty buffer at startup, so user code gets `EOFError` instead of consuming the next command.
3. Any failed exchange — timeout, dead worker, unparseable line — **retires the session** instead of reusing the pipe.

### Execution semantics

The worker compiles in `exec` mode after splitting off a trailing bare expression, which it evaluates and echoes. That combination is what makes both REPL ergonomics and ordinary scripts work:

```python
import math          # statement
r = 2                # statement
math.pi * r ** 2     # trailing expression -> echoed
```

A previous revision compiled in `"single"` mode and only fell back to `exec` when the input was *incomplete*. Multi-statement input is neither single nor incomplete — it raises `SyntaxError: multiple statements found`, so the fallback never fired and any multi-line program failed. `codeop.compile_command(..., "exec")` is used separately to detect genuinely incomplete input.

## Concurrency

```
sessions: Mutex<HashMap<String, Arc<Mutex<Session>>>>
```

Two locks, deliberately. The map lock is held only long enough to look up or insert an `Arc`; the per-session lock is held across the execution. A single lock over the map — as in an earlier revision — meant one session's `time.sleep(60)` blocked every other caller's tool calls, including `list_python_sessions`.

`list_python_sessions` and the reaper use `try_lock` so a busy session is reported or skipped rather than waited on.

## Workspace containment

`read_file` / `write_file` / `edit_file` execute in the server process. Without a check they reach every path the container can see, which makes the interpreter a bypass for any permission model layered above it — under Liberado, an agent refused a vault write could simply ask the interpreter to perform it.

`workspace::resolve` therefore runs before any I/O:

1. Relative paths join the first configured root.
2. `..` and `.` are collapsed **lexically first** — canonicalising first would fail on paths that do not exist yet, which is precisely the `write_file` case.
3. The longest existing prefix is canonicalised, catching symlinked directories, and the remainder re-appended.
4. The result must be under a canonicalised root, or the call is refused.

## Per-session packages

Each session's work directory holds `packages/`, which the worker puts on `sys.path` at startup. `install_package(package, session_id)` runs `pip install --target <that dir> -- <package>`.

The directory is passed in via `LIBERADO_PACKAGES_DIR` rather than hardcoded, because the worker sees it at a different path in each mode: `/work/packages` under nsjail's bindmount, and the real host path as a direct child. Hardcoding the jail path made session-scoped installs silently useless whenever nsjail was not in use.

`--` separates options from the requirement, and names that pip would parse as options are rejected before the process is spawned.

## Failure handling

| Condition | Behaviour |
|---|---|
| Malformed config | Server refuses to start |
| nsjail missing, `SANDBOX_REQUIRED=1` | Server refuses to start |
| nsjail missing, otherwise | Warn at boot, run direct, report `sandboxed: false` |
| Execution exceeds the timeout | Worker killed, session retired, error returned |
| Worker dies mid-exchange | Session retired, error returned |
| Session limit reached | New sessions refused; existing ones are never evicted |
| Output over 50 kB | Truncated on a UTF-8 character boundary, flagged in the response |
| Path outside the workspace | Refused with the roots named in the message |

## Observability

| Level | Events |
|---|---|
| `INFO` | Boot summary and isolation mode, session create/reset, pip installs, reaped sessions |
| `DEBUG` | Code previews, file operations, session reuse |
| `WARN` | Sandbox fallback, timeouts, retired sessions, refused paths, pip failures |
| `ERROR` | Worker death, session creation failure, startup failure |

## Dependencies

- `turbomcp` — MCP server framework and streamable-HTTP transport, pinned to a git rev
- `tokio` — async runtime, subprocesses, timeouts
- `serde` / `serde_json` — the session protocol
- `tempfile` — per-session work directories
- `uuid` — session ids
- `which` — nsjail discovery
- `thiserror` — error types

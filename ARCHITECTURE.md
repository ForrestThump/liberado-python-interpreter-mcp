# ARCHITECTURE

## Overview

`liberado-python-interpreter-mcp` is a Python MCP server exposing code execution and package management tools. It uses [turbomcp](https://pypi.org/project/turbomcp/) for MCP protocol handling and tool registration.

## Components

```
src/liberado_python_interpreter/
├── __init__.py       # Package exports (mcp instance)
├── __main__.py       # CLI entry point (stdio / HTTP modes)
├── server.py         # Tool definitions via @mcp.tool() decorators
└── interpreter.py    # Execution logic (subprocess isolation)
```

## Data Flow

1. MCP client sends `tools/call` over stdio
2. turbomcp dispatches to the decorated Python function
3. Tool function calls into `interpreter.py` which spawns subprocesses
4. stdout/stderr are captured, truncated to 50KB, and returned as JSON

## Execution Model

- **Python code**: written to a temp file, executed via `subprocess.run` with timeout
- **Shell commands**: `subprocess.run` with `shell=True` and timeout
- **Package management**: delegates to `python -m pip`
- All execution is capped at 300 seconds and 50KB output

## Dependencies

- `turbomcp` — MCP server framework, zero transitive dependencies
- Python stdlib only for execution logic

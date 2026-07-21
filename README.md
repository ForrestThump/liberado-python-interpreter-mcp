# liberado-python-interpreter-mcp

A [Model Context Protocol](https://modelcontextprotocol.io/) server providing a persistent Python REPL with session management, file operations, and package management. Built in Python using [TurboMCP](https://pypi.org/project/turbomcp/).

## Tools

### REPL & Sessions

Tool | Description
--- | ---
`execute_python` | Execute Python code in a persistent REPL session. Variables, imports, and functions survive across calls within the same session. Omit `session_id` to create a new session; pass one from a previous response to continue.
`reset_python_session` | Destroy a session and release its namespace
`list_python_sessions` | List all active sessions with variable counts and idle times (auto-cleans at 30 min idle)

### File Operations

Tool | Description
--- | ---
`read_file` | Read a text file from the server's working directory
`write_file` | Write content to a file, creating parent directories as needed
`edit_file` | Find-and-replace text in a file (`count=0` replaces all occurrences)

### Package Management

Tool | Description
--- | ---
`install_package` | Install a Python package via pip (accepts any pip-compatible specifier)
`list_packages` | List all installed Python packages in JSON format
`get_python_info` | Get Python runtime information (version, executable, platform)

## Running

### Docker

```sh
docker build -t liberado-python-interpreter-mcp .
docker run -v ./workdir:/workdir -w /workdir liberado-python-interpreter-mcp
```

### Local development

```sh
pip install -e .
turbomcp serve src/liberado_python_interpreter/server.py
```

### HTTP API (debug)

```sh
pip install -e .[http]
python -m liberado_python_interpreter --http 8080
```

## Claude Desktop / Cursor Configuration

```json
{
  "mcpServers": {
    "liberado-python-interpreter-mcp": {
      "command": "turbomcp",
      "args": ["serve", "/path/to/src/liberado_python_interpreter/server.py"]
    }
  }
}
```

## Session Usage

```
> execute_python(code="x = [1, 2, 3]")
  -> {"session_id": "a1b2c3d4e5f6", "stdout": "", "stderr": "", "created": true}

> execute_python(code="sum(x)", session_id="a1b2c3d4e5f6")
  -> {"session_id": "a1b2c3d4e5f6", "stdout": "6\n", "stderr": "", "created": false}
```

Multi-line blocks (def, class, try/except) are supported and return `more_input_needed: true` when incomplete.

## Security

Python code runs in-process via `code.InteractiveInterpreter` — sessions have full access to the server's Python runtime. File operations are scoped to the server's working directory. Sessions auto-expire after 30 minutes of inactivity.

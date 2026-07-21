# liberado-python-interpreter-mcp

A [Model Context Protocol](https://modelcontextprotocol.io/) server that provides Python code execution, package management, and shell command tools to AI assistants. Built in Python using [TurboMCP](https://pypi.org/project/turbomcp/).

## Tools

Tool | Description
--- | ---
`execute_python` | Execute arbitrary Python code in an isolated subprocess; returns stdout, stderr, and exit code
`execute_shell` | Execute a shell command; returns stdout, stderr, and exit code
`install_package` | Install a Python package via pip (accepts any pip-compatible specifier)
`list_packages` | List all installed Python packages in JSON format
`get_python_info` | Get Python runtime information (version, executable path, platform)

## Running

### Docker

```sh
docker build -t liberado-python-interpreter-mcp .
docker run liberado-python-interpreter-mcp
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

## Security

Tools execute in the same environment as the server process. Python code runs in an isolated subprocess with configurable timeouts. Output is capped at 50KB. No filesystem restrictions are applied beyond what the host provides.

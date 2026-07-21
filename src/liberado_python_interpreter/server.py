import json
import sys

from turbomcp import TurboMCP

from .interpreter import (
    cleanup_sessions,
    edit_file as _edit_file,
    execute_code,
    install_pip_package,
    list_pip_packages,
    list_sessions as _list_sessions,
    read_file as _read_file,
    reset_session,
    write_file as _write_file,
)

mcp = TurboMCP()


@mcp.tool()
def execute_python(code: str, session_id: str | None = None) -> str:
    """Execute Python code in a persistent REPL session. Variables,
    imports, and function definitions persist across calls within the
    same session. Use session_id to continue an existing session; omit
    it to create a new one.

    Args:
        code (str): Python source code to execute
        session_id (str | None): Session ID from a previous call, or omit for a new session
    """
    _cleanup_expired()
    result = execute_code(session_id, code)
    return json.dumps(result)


@mcp.tool()
def reset_python_session(session_id: str) -> str:
    """Destroy a Python REPL session, releasing its namespace and state.

    Args:
        session_id (str): The session ID to reset
    """
    result = reset_session(session_id)
    return json.dumps(result)


@mcp.tool()
def list_python_sessions() -> str:
    """List all active Python REPL sessions with their variable counts
    and idle times. Sessions idle for >30 minutes are auto-cleaned."""
    sessions = _list_sessions()
    return json.dumps(sessions, default=str)


@mcp.tool()
def read_file(path: str) -> str:
    """Read a text file and return its contents. Paths are relative to
    the server's working directory.

    Args:
        path (str): Path to the file to read
    """
    result = _read_file(path)
    return json.dumps(result)


@mcp.tool()
def write_file(path: str, content: str) -> str:
    """Write content to a file, creating parent directories as needed.
    Overwrites existing files. Paths are relative to the server's
    working directory.

    Args:
        path (str): Path to the output file
        content (str): Text content to write
    """
    result = _write_file(path, content)
    return json.dumps(result)


@mcp.tool()
def edit_file(path: str, find: str, replace: str, count: int = 1) -> str:
    """Find and replace text in a file. Set count=0 to replace all
    occurrences. Paths are relative to the server's working directory.

    Args:
        path (str): Path to the file to edit
        find (str): String to search for
        replace (str): Replacement string
        count (int): Number of occurrences to replace (0 = all), default 1
    """
    result = _edit_file(path, find, replace, count=count)
    return json.dumps(result)


@mcp.tool()
def install_package(package: str) -> str:
    """Install a Python package using pip. Accepts any pip-compatible
    specifier (name, name==version, name>=version, etc.).

    Args:
        package (str): Package specifier, e.g. 'requests' or 'requests==2.28.0'
    """
    result = install_pip_package(package)
    return json.dumps(result)


@mcp.tool()
def list_packages() -> str:
    """List all installed Python packages in JSON format. Returns
    package names and versions for the current Python environment."""
    result = list_pip_packages()
    return json.dumps(result)


@mcp.tool()
def get_python_info() -> str:
    """Get information about the Python runtime: version, executable
    path, platform, and architecture."""
    info = {
        "version": sys.version,
        "executable": sys.executable,
        "platform": sys.platform,
        "prefix": sys.prefix,
    }
    return json.dumps(info)


def _cleanup_expired():
    try:
        cleanup_sessions()
    except Exception:
        pass

import json
import sys

from turbomcp import TurboMCP

from .interpreter import (
    execute_code,
    install_pip_package,
    list_pip_packages,
    run_shell,
)

mcp = TurboMCP()


@mcp.tool()
def execute_python(code: str, timeout: int = 30) -> str:
    """Execute arbitrary Python code in an isolated subprocess.
    Returns stdout, stderr, and exit code. Import modules, run
    calculations, process data, or test snippets.

    Args:
        code (str): Python source code to execute
        timeout (int): Maximum execution time in seconds (default 30, max 300)
    """
    if timeout > 300:
        timeout = 300
    result = execute_code(code, timeout=timeout)
    return json.dumps(result)


@mcp.tool()
def execute_shell(command: str, timeout: int = 30) -> str:
    """Execute a shell command and return stdout, stderr, and exit code.
    Use for pip, file operations, system utilities, or any CLI tool.

    Args:
        command (str): Shell command to execute
        timeout (int): Maximum execution time in seconds (default 30, max 300)
    """
    if timeout > 300:
        timeout = 300
    result = run_shell(command, timeout=timeout)
    return json.dumps(result)


@mcp.tool()
def install_package(package: str) -> str:
    """Install a Python package using pip. Accepts any pip-compatible
    specifier (name, name==version, name>=version, etc.).

    Args:
        package (str): Package specifier to install, e.g. 'requests' or 'requests==2.28.0'
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

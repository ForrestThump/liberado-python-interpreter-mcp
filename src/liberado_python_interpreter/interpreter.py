import subprocess
import sys
import os
import tempfile


MAX_OUTPUT_BYTES = 50000
DEFAULT_TIMEOUT = 30


def execute_code(code: str, timeout: int = DEFAULT_TIMEOUT) -> dict:
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".py", delete=False, prefix="liberado_py_"
    ) as f:
        f.write(code)
        tmp_path = f.name

    try:
        result = subprocess.run(
            [sys.executable, tmp_path],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        stdout = result.stdout
        stderr = result.stderr
        return {
            "returncode": result.returncode,
            "stdout": stdout[:MAX_OUTPUT_BYTES],
            "stderr": stderr[:MAX_OUTPUT_BYTES],
            "truncated_stdout": len(result.stdout) > MAX_OUTPUT_BYTES,
            "truncated_stderr": len(result.stderr) > MAX_OUTPUT_BYTES,
        }
    except subprocess.TimeoutExpired:
        return {
            "returncode": -1,
            "stdout": "",
            "stderr": f"Execution timed out after {timeout} seconds",
            "truncated_stdout": False,
            "truncated_stderr": False,
        }
    finally:
        try:
            os.unlink(tmp_path)
        except OSError:
            pass


def install_pip_package(package: str) -> dict:
    result = subprocess.run(
        [sys.executable, "-m", "pip", "install", package],
        capture_output=True,
        text=True,
        timeout=120,
    )
    return {
        "returncode": result.returncode,
        "stdout": result.stdout[:MAX_OUTPUT_BYTES],
        "stderr": result.stderr[:MAX_OUTPUT_BYTES],
    }


def list_pip_packages() -> dict:
    result = subprocess.run(
        [sys.executable, "-m", "pip", "list", "--format=json"],
        capture_output=True,
        text=True,
        timeout=30,
    )
    return {
        "returncode": result.returncode,
        "stdout": result.stdout[:MAX_OUTPUT_BYTES],
        "stderr": result.stderr[:MAX_OUTPUT_BYTES],
    }


def run_shell(command: str, timeout: int = DEFAULT_TIMEOUT) -> dict:
    result = subprocess.run(
        command,
        shell=True,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    return {
        "returncode": result.returncode,
        "stdout": result.stdout[:MAX_OUTPUT_BYTES],
        "stderr": result.stderr[:MAX_OUTPUT_BYTES],
        "truncated_stdout": len(result.stdout) > MAX_OUTPUT_BYTES,
        "truncated_stderr": len(result.stderr) > MAX_OUTPUT_BYTES,
    }

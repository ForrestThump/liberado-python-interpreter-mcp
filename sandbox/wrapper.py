import code, io, json, sys
from contextlib import redirect_stderr, redirect_stdout

CMD_EXEC = "exec"
CMD_INFO = "info"
CMD_RESET = "reset"
CMD_ENV = "env"

KEY_CMD = "cmd"
KEY_CODE = "code"
KEY_OK = "ok"
KEY_STDOUT = "stdout"
KEY_STDERR = "stderr"
KEY_MORE_INPUT = "more_input_needed"
KEY_ERROR = "error"
KEY_VARS = "vars"
KEY_VAR_COUNT = "var_count"
KEY_VERSION = "version"
KEY_PLATFORM = "platform"

COMPILE_SINGLE = "single"
COMPILE_EXEC = "exec"
COMPILE_FILENAME = "<sandbox>"

PACKAGES_DIR = "/work/packages"

import os
if os.path.isdir(PACKAGES_DIR):
    sys.path.insert(0, PACKAGES_DIR)

_interpreter = code.InteractiveInterpreter()


def handle_exec(code_str):
    stdout_buf = io.StringIO()
    stderr_buf = io.StringIO()
    more = False
    try:
        with redirect_stdout(stdout_buf), redirect_stderr(stderr_buf):
            more = _interpreter.runsource(code_str, COMPILE_FILENAME, COMPILE_SINGLE)
            if more:
                stderr_buf.truncate(0)
                stderr_buf.seek(0)
                more = _interpreter.runsource(code_str, COMPILE_FILENAME, COMPILE_EXEC)
    except Exception as e:
        return {
            KEY_OK: True,
            KEY_STDOUT: stdout_buf.getvalue(),
            KEY_STDERR: stdout_buf.getvalue() + "\n{}: {}".format(type(e).__name__, e),
            KEY_MORE_INPUT: False,
        }
    return {
        KEY_OK: True,
        KEY_STDOUT: stdout_buf.getvalue(),
        KEY_STDERR: stderr_buf.getvalue(),
        KEY_MORE_INPUT: more,
    }


def handle_info():
    return {
        KEY_OK: True,
        KEY_VARS: list(_interpreter.locals.keys())[:100],
        KEY_VAR_COUNT: len(_interpreter.locals),
    }


def handle_reset():
    global _interpreter
    _interpreter = code.InteractiveInterpreter()
    return {KEY_OK: True}


def handle_env():
    return {
        KEY_OK: True,
        KEY_VERSION: sys.version,
        KEY_PLATFORM: sys.platform,
    }


HANDLERS = {
    CMD_EXEC: handle_exec,
    CMD_INFO: handle_info,
    CMD_RESET: handle_reset,
    CMD_ENV: handle_env,
}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            continue
        cmd = req.get(KEY_CMD, "")
        handler = HANDLERS.get(cmd)
        if handler is None:
            response = {KEY_OK: False, KEY_ERROR: "unknown command: {}".format(cmd)}
        elif cmd == CMD_EXEC:
            response = handler(req.get(KEY_CODE, ""))
        else:
            response = handler()
        sys.stdout.write(json.dumps(response) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()

"""In-sandbox Python worker.

Reads newline-delimited JSON commands on stdin, executes them in a persistent
namespace, and writes newline-delimited JSON responses on stdout. One process
per REPL session; the Rust side owns its lifetime.

The protocol channel *is* stdin, so the interpreter's own `sys.stdin` is
replaced with an empty stream at startup: a stray `input()` in user code must
raise EOFError rather than silently eat the next command and desynchronise the
protocol.
"""

import ast
import codeop
import io
import json
import os
import sys
import traceback
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

COMPILE_EXEC = "exec"
COMPILE_EVAL = "eval"
COMPILE_FILENAME = "<session>"

ENV_PACKAGES_DIR = "LIBERADO_PACKAGES_DIR"
ENV_LIMIT_MEMORY = "LIBERADO_LIMIT_MEMORY_BYTES"
ENV_LIMIT_CPU = "LIBERADO_LIMIT_CPU_SECONDS"
ENV_LIMIT_FILE = "LIBERADO_LIMIT_FILE_BYTES"
ENV_LIMIT_PROCS = "LIBERADO_LIMIT_PROCESSES"

DEFAULT_PACKAGES_DIR = "/work/packages"
MAX_REPORTED_VARS = 100

# The protocol channel. Captured before user code can reach it.
_protocol_stdin = sys.stdin
_protocol_stdout = sys.stdout
sys.stdin = io.StringIO()


def _apply_resource_limits():
    """Best-effort rlimits on this worker.

    A limit that cannot be applied is skipped rather than fatal: the container
    already carries hard memory/pids caps, and these are a second line only.
    """
    try:
        import resource
    except ImportError:  # non-POSIX
        return

    limits = (
        ("RLIMIT_AS", ENV_LIMIT_MEMORY),
        ("RLIMIT_CPU", ENV_LIMIT_CPU),
        ("RLIMIT_FSIZE", ENV_LIMIT_FILE),
        ("RLIMIT_NPROC", ENV_LIMIT_PROCS),
    )
    for attr, env_key in limits:
        raw = os.environ.get(env_key)
        if not raw:
            continue
        try:
            value = int(raw)
        except ValueError:
            continue
        if value <= 0:
            continue
        which = getattr(resource, attr, None)
        if which is None:
            continue
        try:
            soft, hard = resource.getrlimit(which)
            if hard != resource.RLIM_INFINITY:
                value = min(value, hard)
            resource.setrlimit(which, (value, hard))
        except (ValueError, OSError):
            continue


def _packages_dir():
    return os.environ.get(ENV_PACKAGES_DIR) or DEFAULT_PACKAGES_DIR


def _fresh_globals():
    return {"__name__": "__main__", "__builtins__": __builtins__}


_apply_resource_limits()

_packages = _packages_dir()
if os.path.isdir(_packages):
    sys.path.insert(0, _packages)

_globals = _fresh_globals()


def _format_exception():
    """Traceback for user code, with this wrapper's own frame removed."""
    etype, value, tb = sys.exc_info()
    if tb is not None:
        tb = tb.tb_next
    return "".join(traceback.format_exception(etype, value, tb))


def _is_incomplete(code_str):
    """True when the source is a valid prefix awaiting more lines.

    `codeop.compile_command` in *exec* mode is the right detector: unlike
    "single" mode it accepts multiple statements, so `x = 1\\ny = 2` is complete
    rather than a syntax error.
    """
    try:
        return codeop.compile_command(code_str, COMPILE_FILENAME, COMPILE_EXEC) is None
    except (SyntaxError, ValueError, OverflowError):
        # A genuine error, not an incomplete prefix. Let compilation report it.
        return False


def _split_trailing_expression(tree):
    """Return (statements_module, trailing_expression_or_None).

    REPL semantics: a trailing bare expression has its value echoed, so
    `sum(x)` prints `6` the way it would at a prompt. Everything before it runs
    as ordinary statements, which is what makes multi-statement input work.
    """
    body = list(tree.body)
    trailing = None
    if body and isinstance(body[-1], ast.Expr):
        trailing = ast.Expression(body.pop().value)
        ast.copy_location(trailing, tree.body[-1])
    module = ast.Module(body=body, type_ignores=[])
    return module, trailing


def handle_exec(code_str):
    if _is_incomplete(code_str):
        return {
            KEY_OK: True,
            KEY_STDOUT: "",
            KEY_STDERR: "",
            KEY_MORE_INPUT: True,
        }

    try:
        tree = ast.parse(code_str, COMPILE_FILENAME, COMPILE_EXEC)
        module, trailing = _split_trailing_expression(tree)
        statements = compile(module, COMPILE_FILENAME, COMPILE_EXEC)
        expression = (
            compile(trailing, COMPILE_FILENAME, COMPILE_EVAL)
            if trailing is not None
            else None
        )
    except (SyntaxError, ValueError) as exc:
        return {
            KEY_OK: True,
            KEY_STDOUT: "",
            KEY_STDERR: "".join(traceback.format_exception_only(type(exc), exc)),
            KEY_MORE_INPUT: False,
        }

    stdout_buf = io.StringIO()
    stderr_buf = io.StringIO()
    try:
        with redirect_stdout(stdout_buf), redirect_stderr(stderr_buf):
            exec(statements, _globals)
            if expression is not None:
                value = eval(expression, _globals)
                if value is not None:
                    _globals["_"] = value
                    print(repr(value))
    except SystemExit as exc:
        # User called exit(); report it without taking the worker down.
        stderr_buf.write("SystemExit: {}\n".format(exc.code))
    except BaseException:  # noqa: BLE001 - user code may raise anything
        stderr_buf.write(_format_exception())

    return {
        KEY_OK: True,
        KEY_STDOUT: stdout_buf.getvalue(),
        KEY_STDERR: stderr_buf.getvalue(),
        KEY_MORE_INPUT: False,
    }


def handle_info():
    names = [k for k in _globals if not k.startswith("__")]
    return {
        KEY_OK: True,
        KEY_VARS: sorted(names)[:MAX_REPORTED_VARS],
        KEY_VAR_COUNT: len(names),
    }


def handle_reset():
    global _globals
    _globals = _fresh_globals()
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


def _respond(payload):
    _protocol_stdout.write(json.dumps(payload) + "\n")
    _protocol_stdout.flush()


def main():
    while True:
        line = _protocol_stdin.readline()
        if not line:  # EOF: the Rust side closed the pipe
            return
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError as exc:
            # Every request must get exactly one response, or the Rust side's
            # read pairs up with the wrong command from here on.
            _respond({KEY_OK: False, KEY_ERROR: "malformed request: {}".format(exc)})
            continue

        cmd = req.get(KEY_CMD, "")
        handler = HANDLERS.get(cmd)
        if handler is None:
            _respond({KEY_OK: False, KEY_ERROR: "unknown command: {}".format(cmd)})
        elif cmd == CMD_EXEC:
            _respond(handler(req.get(KEY_CODE, "")))
        else:
            _respond(handler())


if __name__ == "__main__":
    main()

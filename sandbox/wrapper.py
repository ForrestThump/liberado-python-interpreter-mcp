import code, io, json, sys
from contextlib import redirect_stderr, redirect_stdout

_interpreter = code.InteractiveInterpreter()


def handle_exec(code_str):
    stdout_buf = io.StringIO()
    stderr_buf = io.StringIO()
    more = False
    try:
        with redirect_stdout(stdout_buf), redirect_stderr(stderr_buf):
            more = _interpreter.runsource(code_str, "<sandbox>", "single")
            if more:
                stderr_buf.truncate(0)
                stderr_buf.seek(0)
                more = _interpreter.runsource(code_str, "<sandbox>", "exec")
    except Exception as e:
        return {
            "ok": True,
            "stdout": stdout_buf.getvalue(),
            "stderr": stderr_buf.getvalue() + "\n{}: {}".format(type(e).__name__, e),
            "more_input_needed": False,
        }
    return {
        "ok": True,
        "stdout": stdout_buf.getvalue(),
        "stderr": stderr_buf.getvalue(),
        "more_input_needed": more,
    }


HANDLERS = {"exec": handle_exec}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            continue
        cmd = req.get("cmd", "")
        handler = HANDLERS.get(cmd)
        if handler is None:
            response = {"ok": False, "error": "unknown command: {}".format(cmd)}
        else:
            response = handler(req.get("code", ""))
        sys.stdout.write(json.dumps(response) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()

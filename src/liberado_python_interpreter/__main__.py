import argparse
import os
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description="liberado-python-interpreter-mcp")
    parser.add_argument(
        "--http",
        type=int,
        default=None,
        metavar="PORT",
        help="Start HTTP API on the given port",
    )
    parser.add_argument(
        "--stdio",
        action="store_true",
        default=False,
        help="Start MCP server over stdio (default)",
    )
    args = parser.parse_args()

    server_path = os.path.join(
        os.path.dirname(os.path.dirname(__file__)), "server.py"
    )

    if args.http is not None:
        os.execlp(
            sys.executable,
            sys.executable,
            "-c",
            (
                "from liberado_python_interpreter.server import mcp; "
                f"mcp.start_server(port={args.http})"
            ),
        )
    else:
        os.execlp("turbomcp", "turbomcp", "serve", server_path)


if __name__ == "__main__":
    main()

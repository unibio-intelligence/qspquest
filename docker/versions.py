"""Print the version of each QSPQuest entry point in the all-in-one image."""

from pathlib import Path
import json, subprocess, sys


def output(command):
    return subprocess.run(
        command, check=True, capture_output=True, text=True, timeout=60
    ).stdout.strip()


def mcp_server():
    """Start the stdio server once and read the version it reports."""
    message = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "qspquest-versions", "version": "1.0"},
        },
    }
    reply = subprocess.run(
        ["qspquest-mcp", "--profile", "local"],
        input=json.dumps(message) + "\n",
        check=True,
        capture_output=True,
        text=True,
        timeout=60,
    ).stdout.splitlines()[0]
    info = json.loads(reply)["result"]["serverInfo"]
    return f"{info['name']} {info['version']}"


import qspquest

rows = [
    ("CLI", output(["qspquest", "--version"])),
    ("Python", f"qspquest {qspquest.__version__} (Python {sys.version.split()[0]})"),
    (
        "R",
        output(
            [
                "Rscript",
                "-e",
                "cat(paste0('qspquest ', packageVersion('qspquest'),"
                " ' (R ', getRversion(), ')'))",
            ]
        ),
    ),
    ("JupyterLab", output(["jupyter", "lab", "--version"])),
    ("MCP server", mcp_server()),
    ("MCP build", Path("/opt/qspquest/mcp-version.txt").read_text().strip()),
]
width = max(len(name) for name, _ in rows)
for name, value in rows:
    print(f"{name:<{width}}  {value}")

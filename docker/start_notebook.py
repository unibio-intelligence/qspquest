"""Stage the teaching files without replacing users' edits, then start JupyterLab."""

from pathlib import Path
import os, shutil, sys


def initialize(work=Path("/work")):
    source = Path("/opt/qspquest/tutorial")
    target = work / "qspquest-tutorial"
    target.mkdir(parents=True, exist_ok=True)
    for path in source.rglob("*"):
        dest = target / path.relative_to(source)
        if path.is_dir():
            dest.mkdir(parents=True, exist_ok=True)
        elif not dest.exists():
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, dest)
    return target


if __name__ == "__main__":
    initialize()
    os.execvp(
        "jupyter",
        [
            "jupyter",
            "lab",
            "--ip=0.0.0.0",
            "--port=8888",
            "--no-browser",
            "--ServerApp.root_dir=/work",
            "--ServerApp.default_url=/lab/tree/qspquest-tutorial/docs/tutorials/synthetic.ipynb",
            *sys.argv[1:],
        ],
    )

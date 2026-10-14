"""Install exactly one local wheel into the invoking Python environment."""

import argparse
from pathlib import Path
import subprocess, sys

parser = argparse.ArgumentParser()
parser.add_argument("directory", type=Path)
parser.add_argument(
    "--extra",
    choices=[
        "analysis",
        "notebook",
        "docs",
        "interchange",
        "analysis,interchange",
        "notebook,interchange",
    ],
)
args = parser.parse_args()
wheels = list(args.directory.glob("*.whl"))
if len(wheels) != 1:
    raise SystemExit("expected exactly one wheel in the artifact directory")
requirement = str(wheels[0].resolve()) + (f"[{args.extra}]" if args.extra else "")
subprocess.run([sys.executable, "-m", "pip", "install", requirement], check=True)

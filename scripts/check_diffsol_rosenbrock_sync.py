#!/usr/bin/env python3
"""Verify the shared Rosenbrock sources against the included source receipt.

Optionally compare a local upstream checkout: --upstream /path/to/diffsol.
The receipt pins the numerical files shipped in this distribution.
"""
import argparse
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    vendor = root / "vendor/diffsol"
    receipt = json.loads((vendor / "ROSENBROCK_SOURCE.json").read_text())
    checked = []
    for record in receipt["shared_files"]:
        path = record["path"]
        data = (vendor / path).read_bytes()
        if hashlib.sha256(data).hexdigest() != record["sha256"]:
            raise SystemExit(
                f"Shared source changed: {path}; verify the dependency source receipt"
            )
        if args.upstream and data != (args.upstream / path).read_bytes():
            raise SystemExit(f"Upstream source differs: {path}")
        checked.append(path)
    print(
        json.dumps(
            {
                "status": "pass",
                "shared_files": checked,
                "compared_upstream": args.upstream is not None,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()

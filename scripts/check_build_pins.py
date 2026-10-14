"""Check immutable action/image references using only the Python standard library."""

import argparse
import re
from pathlib import Path

SHA = re.compile(r"[0-9a-f]{40}\Z")
IMAGE = re.compile(r"[^\s@]+@sha256:[0-9a-f]{64}\Z")


def check(root: Path) -> list[str]:
    errors = []
    workflows = sorted((root / ".github/workflows").glob("*.y*ml"))
    if not workflows:
        errors.append("No workflow files found")
    for path in workflows:
        for number, line in enumerate(path.read_text().splitlines(), 1):
            match = re.match(r"\s*(?:-\s*)?uses\s*:\s*(.*?)\s*$", line)
            if not match:
                continue
            value, _, comment = match[1].partition("#")
            value = value.strip().strip("'\"")
            if value.startswith("./"):
                if not (root / value).exists() or ".." in Path(value).parts:
                    errors.append(f"{path.relative_to(root)}:{number}: invalid local action")
                continue
            action, separator, ref = value.rpartition("@")
            if value.startswith("docker://"):
                valid = bool(IMAGE.fullmatch(value.removeprefix("docker://")))
            else:
                valid = bool(separator and "/" in action and SHA.fullmatch(ref))
            if not valid or not comment.strip():
                errors.append(f"{path.relative_to(root)}:{number}: use a full immutable pin with a version comment")
    # Check our supported build definitions, excluding dependency/archival trees.
    dockerfiles = [root / "Dockerfile", root / "bindings/mcp/Dockerfile"]
    for path in dockerfiles:
        if not path.exists():
            continue
        arguments, stages = {}, set()
        for number, line in enumerate(path.read_text().splitlines(), 1):
            stripped = line.strip()
            match = re.match(r"ARG\s+(\w+)=(\S+)\s*$", stripped, re.I)
            if match:
                arguments[match[1]] = match[2]
            match = re.match(r"#\s*syntax=(\S+)", stripped)
            if match and not IMAGE.fullmatch(match[1]):
                errors.append(f"{path.relative_to(root)}:{number}: pin Dockerfile frontend digest")
            match = re.match(r"FROM\s+(?:--platform=\S+\s+)?(\S+)(?:\s+AS\s+(\S+))?\s*$", stripped, re.I)
            if not match:
                continue
            image, stage = match.groups()
            variable = re.fullmatch(r"\$\{(\w+)\}", image)
            if variable:
                image = arguments.get(variable[1], "")
            if image.lower() != "scratch" and image.lower() not in stages and not IMAGE.fullmatch(image):
                errors.append(f"{path.relative_to(root)}:{number}: pin external base-image digest")
            if stage:
                stages.add(stage.lower())
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    errors = check(args.root.resolve())
    if errors:
        print("\n".join(errors))
        return 1
    print("Action and container source pins are immutable.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

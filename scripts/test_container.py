"""Check an exact local Docker image using bounded, offline non-root runs."""

from pathlib import Path
import argparse, json, shutil, subprocess, tempfile

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--image", default="qspquest:local")
parser.add_argument("--notebook", action="store_true")
parser.add_argument("--out", type=Path)
parser.add_argument(
    "--user", help="Optional UID:GID override for bind-mount ownership checks"
)
args = parser.parse_args()
metadata = json.loads(
    subprocess.check_output(["docker", "image", "inspect", args.image], text=True)
)[0]
assert (
    metadata["Config"]["User"] == "10001:10001"
), "Image must default to non-root execution"
with tempfile.TemporaryDirectory(prefix="qspquest-container-check-") as directory:
    stage = Path(directory) / "checks"
    stage.mkdir()
    shutil.copytree(ROOT / "examples/synthetic", stage / "examples/synthetic")
    shutil.copyfile(
        ROOT / "docs/examples/interchange_roundtrip.py",
        stage / "interchange_roundtrip.py",
    )
    shutil.copyfile(ROOT / "docker/smoke.py", stage / "smoke.py")
    work = Path(directory) / "work"
    work.mkdir()
    work.chmod(0o777)
    command = [
        "docker",
        "run",
        "--rm",
        "--network",
        "none",
        "--read-only",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "--tmpfs",
        "/tmp:rw,mode=1777,size=512m",
        "--mount",
        f"type=bind,source={stage},target=/checks,readonly",
        "--mount",
        f"type=bind,source={work},target=/work",
        metadata["Id"],
        "python",
        "/checks/smoke.py",
    ]
    if args.user:
        command[command.index(metadata["Id"]) : command.index(metadata["Id"])] = [
            "--user",
            args.user,
        ]
    if args.notebook:
        command.append("--notebook")
    try:
        subprocess.run(command, check=True, timeout=900)
        report = json.loads((work / "smoke.json").read_text())
        report["image"] = {
            "tag": args.image,
            "id": metadata["Id"],
            "architecture": metadata["Architecture"],
            "os": metadata["Os"],
            "bytes": metadata["Size"],
            "user": metadata["Config"]["User"],
            "tested_user": args.user or metadata["Config"]["User"],
        }
        if args.out:
            args.out.parent.mkdir(parents=True, exist_ok=True)
            args.out.write_text(json.dumps(report, indent=2) + "\n")
    finally:
        # Delete only this test's bind-mounted outputs as the user who created them.
        # The host cannot remove directories owned by the default container UID.
        cleanup_code = """
from pathlib import Path
import shutil
for path in Path('/work').iterdir():
    if path.is_symlink() or not path.is_dir():
        path.unlink()
    else:
        shutil.rmtree(path)
"""
        cleanup = command[:command.index('python')] + ['python', '-c', cleanup_code]
        subprocess.run(cleanup, check=True, timeout=60)

"""Record compiled artifacts and preserve dependency copyright/license notices."""

from pathlib import Path
import hashlib, json, shutil, subprocess

out = Path("/out")
(wheel,) = out.glob("*.whl")
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
# The locked aggregate includes notices missing from some registry tarballs.
# Its exact-version source evidence is reviewed with the release.
notice = Path("/build/THIRD-PARTY-NOTICES.txt")
lock_hash = sha(Path("/build/Cargo.lock"))
if f"Cargo.lock SHA-256: {lock_hash}" not in notice.read_text():
    raise SystemExit("Dependency notices do not match Cargo.lock")
licenses = out / "rust-licenses" / "licenses" / "rust"
licenses.mkdir(parents=True)
shutil.copyfile(notice, licenses / notice.name)
info = {
    "schema": "qspquest.container-build/v0.1",
    "wheel": wheel.name,
    "wheel_sha256": sha(wheel),
    "cli_sha256": sha(out / "qspquest"),
    "cargo_lock_sha256": sha(Path("/build/Cargo.lock")),
    "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
}
(out / "build-info.json").write_text(json.dumps(info, indent=2) + "\n")

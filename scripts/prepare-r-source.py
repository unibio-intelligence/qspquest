#!/usr/bin/env python3
"""Build a standalone R source archive with the locked QSPQuest workspace.

Installing the archive requires R, a compatible C compiler and Rust/Cargo.
Cargo downloads locked registry dependencies during the first installation.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def prepare(root, destination):
    destination.mkdir(parents=True, exist_ok=False)
    package = destination / "qspquest"
    package.mkdir()

    def copy(source, target):
        if source.is_symlink():
            raise ValueError(f"Symlink is not a source file: {source}")
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)

    for name in ["DESCRIPTION", "NAMESPACE", "LICENSE", "src/entrypoint.c"]:
        copy(root / "bindings/r" / name, package / name)
    copy(root / "THIRD-PARTY-NOTICES.txt", package / "inst/THIRD-PARTY-NOTICES.txt")
    for directory, suffix in [("R", ".R"), ("man", ".Rd")]:
        for source in sorted((root / "bindings/r" / directory).glob("*" + suffix)):
            copy(source, package / directory / source.name)
    engine = package / "src/engine"
    for name in ["Cargo.toml", "Cargo.lock", ".cargo/config.toml", "LICENSE"]:
        copy(root / name, engine / name)
    members = [
        "crates/qspquest-core",
        "crates/qspquest-modeling",
        "bindings/wasm",
        "bindings/cli",
        "bindings/mcp",
        "bindings/python",
        "bindings/r/src/rust",
    ]
    for member in members:
        copy(root / member / "Cargo.toml", engine / member / "Cargo.toml")
        for source in sorted((root / member / "src").rglob("*.rs")):
            copy(source, engine / source.relative_to(root))
        for source in sorted((root / member / "examples").rglob("*.rs")):
            copy(source, engine / source.relative_to(root))
        if (root / member / "build.rs").exists():
            copy(root / member / "build.rs", engine / member / "build.rs")
    copy(root / "vendor/DIFFSOL-PROVENANCE.md", engine / "vendor/DIFFSOL-PROVENANCE.md")
    for source in sorted((root / "vendor/diffsol").rglob("*")):
        if source.is_file():
            copy(source, engine / source.relative_to(root))
    (package / "src/Makevars").write_text(
        """CARGO ?= cargo
CARGO_TARGET_DIR = $(CURDIR)/rust-target
PKG_LIBS = $(CARGO_TARGET_DIR)/release/libqspquest_r.a
.PHONY: rustlib
all: $(SHLIB)
$(SHLIB): rustlib
rustlib:
\t$(CARGO) build --manifest-path engine/bindings/r/src/rust/Cargo.toml --package qspquest-r --release --locked --target-dir $(CARGO_TARGET_DIR)
"""
    )
    hashes = {
        str(path.relative_to(package)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(package.rglob("*"))
        if path.is_file()
    }
    (destination / "source-receipt.json").write_text(
        json.dumps(
            {
                "classification": "standalone source package; supported build targets are documented separately",
                "files": hashes,
            },
            indent=2,
        )
        + "\n"
    )
    return package


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--r", default="R")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    package = prepare(root, args.destination.resolve())
    subprocess.run(
        [args.r, "CMD", "build", "--no-build-vignettes", str(package)],
        cwd=package.parent,
        check=True,
    )

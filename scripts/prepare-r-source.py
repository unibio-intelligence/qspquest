#!/usr/bin/env python3
"""Build a standalone R source archive with the locked QSPQuest workspace.

Installing the archive requires R, a compatible C compiler and Rust/Cargo.
Cargo downloads locked upstream dependencies during the first installation.
"""
import argparse
import hashlib
import json
import io
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tomllib


def prepare(root, destination):
    destination.mkdir(parents=True, exist_ok=False)
    package = destination / "qspquest"
    package.mkdir()

    def copy(source, target):
        if source.is_symlink():
            raise ValueError(f"Symlink is not a source file: {source}")
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)

    for name in ["DESCRIPTION", "NAMESPACE", "src/entrypoint.c"]:
        copy(root / "bindings/r" / name, package / name)
    copy(root / "bindings/r/LICENSE", package / "inst/licenses/qspquest-LICENSE")
    copy(root / "THIRD-PARTY-NOTICES.txt", package / "inst/THIRD-PARTY-NOTICES.txt")
    for directory, suffix in [("R", ".R"), ("man", ".Rd")]:
        for source in sorted((root / "bindings/r" / directory).glob("*" + suffix)):
            copy(source, package / directory / source.name)
    if (root / "bindings/r/inst").is_dir():
        shutil.copytree(root / "bindings/r/inst", package / "inst", dirs_exist_ok=True)
    for name in ("profiled-proportional.qsp", "profiled-proportional-fit.json"):
        source = root / "docs/examples" / name
        if source.is_file():
            copy(source, package / "inst/extdata" / name)
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


def validate_lock(original, selected):
    """Allow pruning only; preserve every selected dependency's exact identity."""
    packages = tomllib.loads(original.read_text())["package"]
    allowed = {(p["name"], p["version"], p.get("source")): p.get("checksum") for p in packages}
    for item in tomllib.loads(selected.read_text())["package"]:
        key = (item["name"], item["version"], item.get("source"))
        if key not in allowed or allowed[key] != item.get("checksum"):
            raise ValueError(f"R packaging changed a locked dependency: {key}")


def dependency_notices(text, packages):
    """Select existing complete notice texts without dropping required credits."""
    required = {(p["name"], p["version"]) for p in packages}
    covered = set()
    sections = ["Third-party Rust dependency notices for the QSPQuest R package\n"]
    for block in re.split(r"(?m)^={72}\n(?=\S+ \S+ — )", text)[1:]:
        header, separator, notice = block.partition("-" * 72 + "\n")
        if not separator:
            raise ValueError("Malformed dependency notice section")
        selected = []
        for line in header.splitlines():
            match = re.match(r"^(\S+) (\S+) — ", line)
            if match and match.groups() in required:
                covered.add(match.groups())
                selected.append(line)
        if selected:
            sections.append("=" * 72 + "\n" + "\n".join(selected) + "\n" + separator + notice)
    if required - covered:
        raise ValueError(f"Missing legal notices for Rust dependencies: {sorted(required - covered)}")
    return "\n".join(sections)


def copyright_holders(folder):
    """Named holders from a crate's own legal files, excluding license templates."""
    holders = []
    for path in sorted(folder.iterdir()):
        if not (path.is_file() and re.match(r"(?i)^(license|licence|copying|copyright|notice|authors)",
                                            path.name)):
            continue
        for line in path.read_text(errors="replace").splitlines():
            match = re.match(r"^\s*Copyright\s+(?:\(c\)|\(C\)|©)?\s*(.+)$", line)
            if not match or re.search(r"[\[\]{}]", match[1]):
                continue
            holder = re.sub(r"\s*<[^>]*>", "", match[1])
            holder = re.sub(r"(?i)\s*all rights reserved\.?", "", holder)
            holder = re.sub(r"^[0-9][0-9,\-– ]*", "", holder).strip(" ,.")
            if re.search(r"[A-Za-z]{2}", holder) and holder not in holders:
                holders.append(holder)
    return holders


def prepare_cran(root, destination, cargo="cargo"):
    """Assemble an offline R package from the locked, selected engine sources."""
    package = prepare(root, destination)
    binding = root / "bindings/r"
    engine = package / "src/engine"
    for name in ("configure", "cleanup", "src/Makevars.in"):
        shutil.copyfile(binding / name, package / name)
    for name in ("configure", "cleanup"):
        (package / name).chmod(0o755)
    (package / "src/Makevars").unlink()
    shutil.copytree(binding / "inst", package / "inst", dirs_exist_ok=True)
    (package / "inst/extdata").mkdir(exist_ok=True)
    for name in ("profiled-proportional.qsp", "profiled-proportional-fit.json"):
        shutil.copyfile(root / "docs/examples" / name, package / "inst/extdata" / name)
    shutil.copytree(binding / "tests", package / "tests")

    # Retain the shared sandbox sources used by the compiler identity.
    identity_sources = {name: (engine / name).read_bytes() for name in (
        "bindings/mcp/src/core/compute.rs",
        "bindings/mcp/src/edit.rs",
    )}
    for member in ("cli", "mcp", "python"):
        shutil.rmtree(engine / "bindings" / member)
    for name, raw in identity_sources.items():
        path = engine / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(raw)
    manifest = (engine / "Cargo.toml").read_text()
    selected = '["crates/qspquest-core", "crates/qspquest-modeling", "bindings/r/src/rust"]'
    manifest = re.sub(r"(?m)^(?:members|default-members) = .*", lambda m:
                      m.group(0).split(" = ", 1)[0] + " = " + selected, manifest)
    (engine / "Cargo.toml").write_text(manifest)
    for member in ("qspquest-core", "qspquest-modeling"):
        path = engine / "crates" / member / "Cargo.toml"
        path.write_text(re.sub(r"(?ms)^\[dev-dependencies\]\n.*?(?=^\[|\Z)", "", path.read_text()))
        examples = path.parent / "examples"
        if examples.exists():
            shutil.rmtree(examples)
        # A schema-generation example is not part of the installed R interface.
        path.write_text(re.sub(r"(?ms)^\[\[example\]\]\n.*?(?=^\[|\Z)", "", path.read_text()))
    # Removing unused workspace members prunes the lockfile without updating any
    # selected dependency. Refuse any unreviewed version, source or checksum.
    result = subprocess.run([cargo, "metadata", "--offline", "--format-version", "1"],
                            cwd=engine, check=True, text=True, capture_output=True)
    validate_lock(root / "Cargo.lock", engine / "Cargo.lock")
    metadata = json.loads(result.stdout)
    packages = sorted((p for p in metadata["packages"] if p["source"]),
                      key=lambda p: (p["name"], p["version"]))
    vendor = destination / "rust-vendor"
    result = subprocess.run([cargo, "vendor", "--offline", "--locked", str(vendor)],
                            cwd=engine, check=True, text=True, capture_output=True)
    config = result.stdout.replace(str(vendor), "@VENDOR@")
    if "@VENDOR@" not in config or str(destination) in config:
        raise ValueError("Cargo did not produce a relocatable vendor configuration")
    (package / "src/cargo-config.in").write_text(config)
    supported = set()
    for target in ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu",
                   "x86_64-apple-darwin", "aarch64-apple-darwin"):
        filtered = subprocess.run([cargo, "metadata", "--frozen", "--offline",
                                   "--format-version", "1", "--filter-platform", target],
                                  cwd=engine, check=True, text=True, capture_output=True)
        supported.update(p["id"] for p in json.loads(filtered.stdout)["packages"])
    source_packages = {(p["name"], p["version"]) for p in packages if p["id"] in supported}
    omitted = []
    crate_details = {folder.name: tomllib.loads((folder / "Cargo.toml").read_text())["package"]
                     for folder in vendor.iterdir() if folder.is_dir()}
    with tarfile.open(package / "src/rust-vendor.tar.xz", "w:xz", preset=6) as archive:
        for path in sorted(vendor.rglob("*")):
            if path.is_symlink():
                raise ValueError(f"Vendored dependency contains a symlink: {path}")
            if not path.is_file():
                continue
            relative = path.relative_to(vendor)
            crate = vendor / relative.parts[0]
            details = crate_details[crate.name]
            selected = (details["name"], details["version"]) in source_packages
            legal = any(re.match(r"(?i)^(license|licence|copying|copyright|notice|authors)", part)
                        for part in relative.parts[1:])
            # Cargo resolves platform-conditional manifests even on other hosts.
            # Retain their genuine metadata and legal files, not Windows binary
            # import libraries or code unused by the supported Unix builds.
            metadata_target = relative.as_posix().split("/", 1)[1] in ("src/lib.rs", "src/main.rs", "build.rs")
            if (not selected and not legal and not metadata_target
                    and path.name not in ("Cargo.toml", ".cargo-checksum.json")):
                omitted.append(relative.as_posix())
                continue
            if path.suffix.lower() in (".a", ".lib", ".dll", ".exe", ".o", ".so", ".dylib"):
                raise ValueError(f"Binary input is not permitted in the R source package: {relative}")
            data = path.read_bytes()
            info = tarfile.TarInfo("rust-vendor/" + path.relative_to(vendor).as_posix())
            info.size = len(data)
            info.mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
            info.mtime = 0
            archive.addfile(info, io.BytesIO(data))
    credits = ["Rust dependency authors and copyright holders", "",
               "THIRD-PARTY-NOTICES.txt supplies copyright and license texts.",
               "Crate-local license files are also retained where provided.",
               "Cargo.lock records exact versions and checksums.", ""]
    folders = {(details["name"], details["version"]): vendor / folder
               for folder, details in crate_details.items()}
    inventory = []
    for item in packages:
        names = item["authors"]
        holders = copyright_holders(folders[(item["name"], item["version"])])
        if not holders:
            holders = [re.sub(r"\s*<[^>]*>", "", name).strip() for name in names]
            holders = [name for name in holders if name]
        if not holders:
            # Some crates declare neither Cargo authors nor a named holder.
            holders = [f"The {item['name']} contributors"
                       + (f" ({item['repository']})" if item.get("repository") else "")]
        credits += [f"{item['name']} {item['version']}",
                    "Authors: " + ("; ".join(names) if names else "not declared in Cargo metadata"),
                    "Copyright: " + "; ".join(holders),
                    "License: " + str(item["license"]), ""]
        inventory.append({k: item[k] for k in ("name", "version", "source", "license", "authors")})
    (package / "inst/COPYRIGHTS").write_text("\n".join(credits))
    description = (package / "DESCRIPTION").read_text()
    # R's DESCRIPTION can refer to the complete contributor/copyright inventory.
    description += ("Copyright: UniBio Intelligence; authors and copyright holders of\n"
                    "    the bundled Rust crates, listed in inst/COPYRIGHTS.\n")
    description = description.replace(
        'person("UniBio Intelligence", role = c("cph", "fnd")))',
        'person("UniBio Intelligence", role = c("cph", "fnd")),\n'
        '    person("Authors of the dependency Rust crates", role = c("ctb", "cph"),\n'
        '    comment = "see inst/COPYRIGHTS and inst/THIRD-PARTY-NOTICES.txt"))',
    )
    if "Authors of the dependency Rust crates" not in description:
        raise ValueError("DESCRIPTION must end Authors@R with the UniBio Intelligence entry")
    description += "OS_type: unix\n"
    (package / "DESCRIPTION").write_text(description)
    (package / "inst/THIRD-PARTY-NOTICES.txt").write_text(
        dependency_notices((root / "THIRD-PARTY-NOTICES.txt").read_text(), packages)
    )
    (destination / "dependency-inventory.json").write_text(json.dumps(inventory, indent=2) + "\n")
    (destination / "omitted-platform-files.json").write_text(json.dumps(omitted, indent=2) + "\n")
    # Restore Cargo's configuration path only during configure, keeping hidden
    # directories out of the submitted R source layout.
    (engine / ".cargo/config.toml").rename(engine / "cargo-config.toml")
    (engine / ".cargo").rmdir()
    shutil.rmtree(vendor)
    hashes = {path.relative_to(package).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
              for path in sorted(package.rglob("*")) if path.is_file()}
    (destination / "source-receipt.json").write_text(json.dumps({
        "classification": "offline CRAN source candidate; review and platform checks required",
        "files": hashes,
    }, indent=2) + "\n")
    return package


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--r", default="R")
    parser.add_argument("--cran", action="store_true", help="Bundle locked dependencies for offline CRAN installation")
    parser.add_argument("--cargo", default="cargo")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    package = (prepare_cran(root, args.destination.resolve(), args.cargo) if args.cran
               else prepare(root, args.destination.resolve()))
    subprocess.run(
        [args.r, "CMD", "build", "--no-build-vignettes", str(package)],
        cwd=package.parent,
        check=True,
    )

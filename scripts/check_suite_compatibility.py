#!/usr/bin/env python3
"""Verify pinned suite inputs and compare a fresh CLI run to one retained preset."""
import argparse
import csv
import gzip
import hashlib
import io
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import sys
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def relative(name):
    path = PurePosixPath(name)
    if not name or path.is_absolute() or str(path) != name or ".." in path.parts or "\\" in name:
        raise ValueError("Unsafe suite file reference")
    return path


def checked(root, name, digest):
    path = root / relative(name)
    if path.is_symlink() or not path.is_file():
        raise ValueError("Missing regular suite file: " + name)
    raw = path.read_bytes()
    if sha(raw) != digest:
        raise ValueError("Pinned checksum differs: " + name)
    return raw


def load_lock(path):
    lock = json.loads(path.read_text())
    if not isinstance(lock, dict) or lock.get("schema") != "qspquest.suite-compatibility/v1":
        raise ValueError("Unsupported suite compatibility lock")
    for component in ("models", "validation"):
        record = lock["components"][component]
        if record["repository"] != "unibio-intelligence/qspquest-" + component:
            raise ValueError("Unexpected counterpart repository: " + component)
        if not re.fullmatch(r"[0-9a-f]{40}", record["commit"]):
            raise ValueError("Counterpart commit must be a full immutable ID: " + component)
    case = lock["case"]
    for name in ("absolute_tolerance", "relative_tolerance"):
        value = case[name]
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
            raise ValueError("Invalid comparison tolerance: " + name)
    if not case["solvers"] or len(set(case["solvers"])) != len(case["solvers"]) or any(
            s not in {"diffsol_bdf", "diffsol_rodas5p"} for s in case["solvers"]):
        raise ValueError("Invalid native solver selection")
    return lock


def verify_checkout(root, expected_commit):
    """Bind a complete, clean tracked checkout to its required Git commit."""
    root = root.resolve(strict=True)
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}

    def git(*args):
        result = subprocess.run(
            ["git", "--no-replace-objects", "-c", "core.fsmonitor=false", "-C", str(root), *args],
            env=environment, capture_output=True, timeout=60,
        )
        if result.returncode:
            raise ValueError("Counterpart must be a Git checkout: " + str(root))
        return result.stdout

    if Path(os.fsdecode(git("rev-parse", "--show-toplevel")).strip()).resolve() != root:
        raise ValueError("Counterpart path must be the Git checkout root: " + str(root))
    commit = git("rev-parse", "--verify", "HEAD").decode().strip()
    if commit != expected_commit:
        raise ValueError("Counterpart commit differs from the suite lock: " + str(root))
    if git("status", "--porcelain=v1", "--untracked-files=all"):
        raise ValueError("Counterpart checkout must be clean, including untracked files: " + str(root))
    tree = git("rev-parse", "HEAD^{tree}").decode().strip()
    entries = git("ls-tree", "-rz", "--full-tree", "HEAD").split(b"\0")
    count = 0
    for entry in filter(None, entries):
        metadata, raw_name = entry.split(b"\t", 1)
        mode, kind, digest = metadata.decode().split()
        name = os.fsdecode(raw_name)
        path = root / relative(name)
        if kind != "blob" or mode not in {"100644", "100755"}:
            raise ValueError("Unsupported counterpart tracked file mode: " + name)
        # Check bytes directly: Git status can hide edits marked assume-unchanged
        # or skip-worktree, and content filters need not preserve source bytes.
        if path.resolve() != path or not stat.S_ISREG(path.lstat().st_mode):
            raise ValueError("Counterpart tracked file must be regular: " + name)
        raw = path.read_bytes()
        actual = hashlib.sha1(b"blob " + str(len(raw)).encode() + b"\0" + raw).hexdigest()
        executable = bool(path.stat().st_mode & 0o111)
        if actual != digest or (os.name != "nt" and executable != (mode == "100755")):
            raise ValueError("Counterpart tracked file differs from its commit: " + name)
        count += 1
    return {"commit": commit, "tree": tree, "tracked_files": count}


def finite_number(value):
    try:
        return type(value) in (int, float) and math.isfinite(value)
    except OverflowError:
        return False


def compare(output, native, reference, absolute, relative_tolerance, *, time_unit, model_outputs):
    try:
        table = list(csv.reader(io.StringIO(reference.decode()), strict=True))
    except (UnicodeError, csv.Error) as error:
        raise ValueError("Invalid reference CSV") from error
    if (len(table) < 2 or not isinstance(output, dict)
            or output.get("schema") != "qspquest.result/v0.1"):
        raise ValueError("Missing reference rows or incompatible result schema")
    header, records = table[0], table[1:]
    if (not all(header) or len(set(header)) != len(header)
            or not {"time", "side"} <= set(header)
            or any(len(row) != len(header) for row in records)):
        raise ValueError("Reference CSV requires unique columns and complete rows")
    rows = [dict(zip(header, row)) for row in records]
    names = set(header) - {"time", "side"}
    specs = native.get("outputs") if isinstance(native, dict) else None
    if not isinstance(specs, dict) or not names or names != set(specs):
        raise ValueError("Reference output inventory differs from the native projection")
    if any(not isinstance(spec, dict)
           or not isinstance(spec.get("native_name"), str) or not spec["native_name"]
           or not isinstance(spec.get("unit"), str) or not spec["unit"]
           for spec in specs.values()):
        raise ValueError("Native projection needs named outputs and units")
    native_names = {spec["native_name"] for spec in specs.values()}
    if len(native_names) != len(specs):
        raise ValueError("Native projection output aliases must be distinct")
    if not isinstance(model_outputs, list) or any(
            not isinstance(spec, dict) or not isinstance(spec.get("name"), str) or not spec["name"]
            or not isinstance(spec.get("unit"), str) or not spec["unit"] for spec in model_outputs):
        raise ValueError("Native model needs named output declarations and units")
    declarations = {spec["name"]: spec["unit"] for spec in model_outputs}
    if len(declarations) != len(model_outputs) or any(
            declarations.get(spec["native_name"]) != spec["unit"] for spec in specs.values()):
        raise ValueError("Native projection aliases or units differ from model declarations")
    expected_times = [float(row["time"]) for row in rows]
    expected_sides = [row["side"] for row in rows]
    times, sides = output.get("times"), output.get("sides")
    if (not isinstance(times, list) or not isinstance(sides, list)
            or not all(finite_number(t) for t in times + expected_times)
            or any(side not in ("pre", "post") for side in expected_sides)
            or any(b < a or (a == b and (expected_sides[i], expected_sides[i + 1]) != ("pre", "post"))
                   for i, (a, b) in enumerate(zip(expected_times, expected_times[1:])))):
        raise ValueError("Observation times or event sides are invalid")
    if times != expected_times or sides != expected_sides:
        raise ValueError("Observation times or event sides differ")
    if not isinstance(time_unit, str) or not time_unit or output.get("time_unit") != time_unit:
        raise ValueError("Result time unit differs from the native model")
    raw_columns = output.get("outputs")
    if not isinstance(raw_columns, list) or any(
            not isinstance(column, dict) or not isinstance(column.get("name"), str)
            for column in raw_columns):
        raise ValueError("Native outputs must be named column records")
    columns = {column["name"]: column for column in raw_columns}
    if len(columns) != len(raw_columns):
        raise ValueError("Duplicate native output name")
    if set(declarations) - set(columns):
        raise ValueError("Missing native output: " + ", ".join(sorted(set(declarations) - set(columns))))
    if set(columns) != set(declarations):
        raise ValueError("Unexpected native output columns")
    for name, column in columns.items():
        if (column.get("unit") != declarations[name] or not isinstance(column.get("values"), list)
                or len(column["values"]) != len(rows)):
            raise ValueError("Output units or observation count differ: " + name)
        if not all(finite_number(value) for value in column["values"]):
            raise ValueError("Nonfinite or nonnumeric comparison value: " + name)
    maximum = 0.0
    values = 0
    for name in sorted(names):
        spec = specs[name]
        column = columns[spec["native_name"]]
        if (column.get("unit") != spec["unit"] or not isinstance(column.get("values"), list)
                or len(column["values"]) != len(rows)):
            raise ValueError("Output units or observation count differ: " + name)
        for row, actual in zip(rows, column["values"]):
            expected = float(row[name])
            if not finite_number(actual) or not finite_number(expected):
                raise ValueError("Nonfinite comparison value: " + name)
            error = abs(actual - expected)
            tolerance = absolute + relative_tolerance * abs(expected)
            if not finite_number(error) or not finite_number(tolerance):
                raise ValueError("Comparison arithmetic overflow: " + name)
            maximum = max(maximum, error)
            values += 1
            if error > tolerance:
                raise ValueError("Retained-preset comparison failed: " + name + " at time " + row["time"])
    return {"observations": len(rows), "output_values": values, "maximum_absolute_error": maximum}


def stored_check(root, script):
    result = subprocess.run([sys.executable, "-I", str(root / script)], cwd=root,
                            capture_output=True, timeout=180)
    if result.returncode:
        raise ValueError("Stored-evidence verifier failed: " + script + ": "
                         + result.stderr.decode(errors="replace")[-2000:])
    return {"kind": "archive-verification", "status": "passed", "script": script,
            "stdout_sha256": sha(result.stdout)}


def check_bundled_models(models, catalog, engine_root):
    """Require every bundled ID/version to identify the same published bytes."""
    library = engine_root / "bindings/mcp/assets/library"
    version = json.loads((library / "version.json").read_text())
    bundled = json.loads(checked(library, "catalog.json", version["catalog_sha256"]))
    published = {}
    for row in catalog["models"]:
        identity = (row["id"], row["version"])
        if identity in published:
            raise ValueError("Duplicate published model identity/version")
        published[identity] = row
    seen = set()
    verified = []
    for row in bundled["models"]:
        identity = (row["id"], row["version"])
        if identity in seen:
            raise ValueError("Duplicate bundled model identity/version")
        seen.add(identity)
        counterpart = published.get(identity)
        if counterpart is None:
            raise ValueError("Bundled model identity/version is not published: " + "@".join(identity))
        actual = checked(library, row["archive"], row["omex_sha256"])
        expected = checked(models, counterpart["archive"], counterpart["omex_sha256"])
        if actual != expected:
            raise ValueError("Bundled archive differs from published model: " + "@".join(identity))
        verified.append({"id": row["id"], "version": row["version"], "omex_sha256": sha(actual)})
    if not verified:
        raise ValueError("Bundled model catalog is empty")
    return verified


def check_validation_engine(identity, environment):
    """Require evidence for the actual compiler, not just its package version."""
    expected = environment["engine"]["compiler_source_sha256"]
    if identity.get("compiler_source_sha256") != expected:
        raise ValueError("Validation evidence does not match the CLI compiler source")


def check(lock, models, validation, cli, report, engine_root=ROOT):
    for name in lock["required_engine_files"]:
        path = engine_root / relative(name)
        if path.is_symlink() or not path.is_file():
            raise ValueError("Required engine or tutorial input is missing: " + name)
    pins = lock["components"]
    roots = {"models": models, "validation": validation}
    verified = {role: verify_checkout(root, pins[role]["commit"]) for role, root in roots.items()}
    report["verified_counterparts"] = verified
    catalog = json.loads(checked(models, "catalog.json", pins["models"]["catalog_sha256"]))
    validation_manifest = json.loads(checked(validation, "manifest.json", pins["validation"]["manifest_sha256"]))
    version = tomllib.loads((models / "pyproject.toml").read_text())["project"]["version"]
    if version != pins["models"]["version"]:
        raise ValueError("Model-library package version differs from the suite lock")
    validation_environment = json.loads(checked(validation, "environment.json", validation_manifest["files"]["environment.json"]))
    report["bundled_models"] = check_bundled_models(models, catalog, engine_root)
    for role, root, script in (("models", models, "scripts/verify_library.py"),
                               ("validation", validation, "scripts/verify.py")):
        report["checks"].append({"component": role, **stored_check(root, script)})
    case = lock["case"]
    selected = [r for r in catalog["models"] if r["id"] == case["model_id"] and r["version"] == case["model_version"]]
    if len(selected) != 1:
        raise ValueError("Suite model identity/version is unavailable")
    row = selected[0]
    archive_bytes = checked(models, row["archive"], row["omex_sha256"])
    with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
        index = json.loads(archive.read("index.json"))
        if (index["model_id"], index["model_version"]) != (row["id"], row["version"]):
            raise ValueError("Archive model identity/version differs")
        native = index["native_execution"]
        if native["schema"] != "qspquest.models.native-execution/v1":
            raise ValueError("Unsupported native projection schema")
        model = archive.read(str(relative(native["model"])))
        model_document = json.loads(model)
        request = json.loads(archive.read(str(relative(native["scenarios"][case["scenario"]]))))
    evidence = json.loads((models / relative(row["archive"])).with_name("evidence.json").read_text())
    if evidence["omex_sha256"] != row["omex_sha256"]:
        raise ValueError("Preset evidence archive identity differs")
    cases = [c for c in evidence["cases"] if c["scenario_id"] == case["scenario"]]
    if len(cases) != 1:
        raise ValueError("Retained preset reference is unavailable")
    record = cases[0]["files"]["qspquest_csv"]
    reference_root = models / relative(row["archive"]).parent
    if record["compression"] != "gzip":
        raise ValueError("Unsupported retained reference compression")
    reference = gzip.decompress(checked(reference_root, record["path"], record["sha256"]))
    if sha(reference) != record["uncompressed_sha256"]:
        raise ValueError("Retained reference content digest differs")
    cli = cli.resolve(strict=True)
    cli_hash = sha(cli.read_bytes())
    report["artifact_sha256"] = cli_hash
    report["case"] = {"model_id": row["id"], "model_version": row["version"], "archive_sha256": row["omex_sha256"],
                      "scenario": case["scenario"], "dimensional_qualification": native.get("dimensional_qualification", False)}
    with tempfile.TemporaryDirectory(prefix="qspquest-compatibility-") as temporary:
        root = Path(temporary)
        (root / "model.json").write_bytes(model)
        for solver in case["solvers"]:
            run = {**request, "solver": solver}
            raw = json.dumps(run, allow_nan=False).encode()
            (root / "request.json").write_bytes(raw)
            result = subprocess.run([str(cli), "run", "model.json", "request.json"], cwd=root,
                                    capture_output=True, timeout=120)
            receipt = {"kind": "native-reexecution-against-retained-reference", "solver": solver,
                       "status": "failed", "request_sha256": sha(raw), "result_sha256": sha(result.stdout),
                       "exit_code": result.returncode, "absolute_tolerance": case["absolute_tolerance"],
                       "relative_tolerance": case["relative_tolerance"], "reference_sha256": sha(reference),
                       "reference_engine_identity": cases[0]["engine_identity"]}
            report["checks"].append(receipt)
            if result.returncode:
                raise ValueError("Native compatibility execution failed: " + solver + ": "
                                 + result.stderr.decode(errors="replace")[-2000:])
            output = json.loads(result.stdout)
            receipt["engine_identity"] = output["identity"]
            check_validation_engine(output["identity"], validation_environment)
            if output["identity"]["compiler_version"] != "qspquest/" + lock["engine_version"]:
                raise ValueError("CLI engine version differs from the suite lock")
            receipt.update(compare(output, native, reference, case["absolute_tolerance"], case["relative_tolerance"],
                                   time_unit=model_document["time_unit"], model_outputs=model_document["outputs"]))
            receipt["status"] = "passed"
    if sha(cli.read_bytes()) != cli_hash:
        raise ValueError("CLI artifact changed during compatibility execution")
    for role, root in roots.items():
        if verify_checkout(root, pins[role]["commit"]) != verified[role]:
            raise ValueError("Counterpart identity changed during compatibility execution: " + role)
    report["counterpart_recheck_passed"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--models", required=True, type=Path)
    parser.add_argument("--validation", required=True, type=Path)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--lock", type=Path, default=ROOT / ".github/suite-compatibility.json")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    report = {"schema": "qspquest.suite-compatibility-result/v1", "status": "failed", "checks": [],
              "scope": "All bundled model archives match the pinned models repository byte-for-byte; one library preset and stored archive integrity are verified. Independent reference regeneration, fitting, physical-unit and population qualification are not established."}
    # Refuse an existing report before executing anything; each run gets its own evidence.
    try:
        stream = args.output.open("x")
    except OSError as error:
        parser.exit(2, "suite-compatibility: cannot create a new report: " + str(error) + "\n")
    with stream:
        try:
            lock = load_lock(args.lock)
            report["lock_sha256"] = sha(args.lock.read_bytes())
            report["counterpart_pins"] = lock["components"]
            check(lock, args.models.resolve(), args.validation.resolve(), args.cli, report)
            report["status"] = "passed"
        except (ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
            report["error"] = str(error)
        json.dump(report, stream, indent=2, allow_nan=False)
        stream.write("\n")
    if report["status"] != "passed":
        parser.exit(2, "suite-compatibility: " + report["error"] + "\n")
    print("Suite compatibility passed; evidence: " + str(args.output))


if __name__ == "__main__":
    main()

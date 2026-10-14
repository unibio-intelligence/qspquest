"""Simulate a Ubi synthetic model and verify its standard OMEX round trip."""

from pathlib import Path
from hashlib import sha256
import json, math
import qspquest as pf
from qspquest.interchange import export_omex, import_omex


def run_tutorial(root, output):
    root = Path(root)
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    model = json.loads((root / "examples/synthetic/absorption.model.json").read_text())
    scenarios = {
        "bolus": pf.run_request(
            end=24,
            times=[0, 0.25, 0.5, 1, 2, 4, 8, 12, 24],
            administrations=[pf.bolus("depot", 72, "mg")],
            rtol=1e-10,
            atol=1e-12,
        )
    }
    archive = export_omex(model, scenarios)
    path = output / "absorption.omex"
    path.write_bytes(archive)
    digest = sha256(archive).hexdigest()
    loaded = import_omex(path, expected_sha256=digest)
    answer = loaded.run("bolus")
    ka = 1.2
    k = 0.8 / 6
    expected = [
        72 * ka / (6 * (ka - k)) * (math.exp(-k * t) - math.exp(-ka * t))
        for t in answer["times"]
    ]
    error = max(abs(a - b) for a, b in zip(answer["outputs"][0]["values"], expected))
    assert error < 1e-7, error
    assert export_omex(loaded.imported_model, loaded.scenarios) == archive
    report = {
        "status": "passed",
        "archive_sha256": digest,
        "max_absolute_analytic_error": error,
        "archive_bytes_preserved": True,
    }
    (output / "roundtrip.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[2]
    print(
        json.dumps(
            run_tutorial(root, root / "docs/tutorials/synthetic-output"), indent=2
        )
    )

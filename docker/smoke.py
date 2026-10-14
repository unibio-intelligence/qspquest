"""Exercise installed CLI/Python simulation and a synthetic OMEX round trip."""

from pathlib import Path
import json, subprocess, sys
import qspquest as pf
from qspquest.doctor import check_environment

out = Path("/work")
out.mkdir(exist_ok=True)
root = Path("/checks")
model = root / "examples/synthetic/absorption.qsp"
run = pf.run_request(
    end=24, times=[0, 1, 2, 4, 8, 12, 24], administrations=[pf.bolus("depot", 72, "mg")]
)
request = out / "request.json"
request.write_text(json.dumps(run))
python_result = pf.CompiledModel(model.read_text()).run(run)
cli_result = json.loads(
    subprocess.check_output(["qspquest", "run", str(model), str(request)], text=True)
)
assert (
    cli_result["times"] == python_result["times"]
    and cli_result["outputs"] == python_result["outputs"]
)
sys.path.insert(0, str(root))
from interchange_roundtrip import run_tutorial

report = {
    "status": "passed",
    "doctor": check_environment(analysis=True, interchange=True),
    "cli_python_agreement": "passed",
    "interchange": run_tutorial(root, out / "interchange"),
}
if "--notebook" in sys.argv:
    import nbformat
    from nbclient import NotebookClient

    sys.path.insert(0, "/opt/qspquest")
    from start_notebook import initialize

    tutorial = initialize(out)
    path = tutorial / "docs/tutorials/synthetic.ipynb"
    nb = nbformat.read(path, as_version=4)
    NotebookClient(
        nb,
        timeout=120,
        kernel_name="python3",
        resources={"metadata": {"path": str(path.parent)}},
    ).execute()
    assert all(c.execution_count is not None for c in nb.cells if c.cell_type == "code")
    report["synthetic_notebook"] = "passed"
(out / "smoke.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))

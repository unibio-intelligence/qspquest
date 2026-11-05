# Save your first QSPQuest result

Run a synthetic one-compartment model and save a table you can read and
rerun. Choose the Python, CLI or MCP route below. Each uses the same
6 mg bolus, clearance 0.3 L/h, volume 3 L and four post-dose observations.
The Python and CLI helpers check the result against `cp(t) = 2 * exp(-0.1 * t)`
within 1e-8 mg/L.

QSPQuest 0.1.0 publishes Python wheels on PyPI and the MCP server image on
GitHub Container Registry. The command-line tool is built from source.

## Developer: use the Python wheel

The wheel includes the model, request and executable example; no Rust compiler,
source checkout or analysis packages are needed. With CPython 3.10+ on macOS/Linux:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install qspquest
python -m qspquest.first_run --out developer-results
```

On Windows PowerShell:

```powershell
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install qspquest
python -m qspquest.first_run --out developer-results
```

To run the model from Python directly:

```python
from qspquest import CompiledModel
from qspquest.first_run import synthetic_inputs

source, request = synthetic_inputs()
result = CompiledModel(source).run(request)
print(result["identity"])
print(next(column for column in result["outputs"] if column["name"] == "cp"))
```

## Scientist: build and run the CLI

Build the CLI from a source checkout, as in the
[README](../../README.md#build-the-cli-from-source). The helper script reuses the
result writer from the Python wheel, so install the wheel as above. With that
environment active, from the source root on macOS/Linux:

```sh
cargo build --release --locked -p qspquest-cli
python docs/examples/scientist_first_run.py --binary target/release/qspquest \
  --model conformance/models/synthetic-one-compartment.qsp \
  --request docs/examples/first-run.json --out scientist-results
```

On Windows, pass `--binary target\release\qspquest.exe`. The script runs the
model, checks all four observations and saves six files. To get the JSON result
without the helper, run `target/release/qspquest run
conformance/models/synthetic-one-compartment.qsp docs/examples/first-run.json >
simulation.json`.

## Agent: use the MCP image

The published image includes the same synthetic model, so no model files or
library settings are needed. An MCP client starts it over stdio with:

```sh
docker run --rm -i --network none --read-only --cap-drop ALL \
  --security-opt no-new-privileges \
  ghcr.io/unibio-intelligence/qspquest-mcp:latest --profile local
```

Follow the [agent and Docker quickstart](agents-docker.md) to connect your MCP
client over stdio or HTTP and run the same simulation. Ask the agent to inspect
capabilities, search for `synthetic_one_compartment`, read its full card and
default scenario, inspect a simulation plan, and simulate with `detail: full`,
`outputs: ["cp"]` and the returned plan hash. The server runs locally. A chat
client, by contrast, may send the model, requests and results to its AI
provider; configure it to match your data requirements.

## Interpret and retain the output

The Python and CLI routes print this table; the MCP tools return the same values:

```text
time_h,side,cp_mg_per_L
0,post,2.000000
2,post,1.637462
8,post,0.898658
24,post,0.181436
```

`post` means after the bolus at time zero. Time is in hours and `cp` is in mg/L.
The Python and CLI helpers save six files; keep all of them:

| File | Purpose |
| --- | --- |
| `model.qsp` | Exact synthetic model source |
| `request.json` | Dosing, solver, tolerances, budgets and observations |
| `result.json` | Full result, including units and run identity |
| `first-run.csv` | Four numerical observations |
| `first-run.csv.manifest.json` | Column meanings, units and file hashes |
| `checks.json` | Comparison against the exact analytical answer |

The manifest uses `qspquest.table-manifest/v1`, with `producer` equal to `cli`
or `python`. It records `not_assessed` and `qualified: false`: passing this
check does not validate other models or clinical use. Checksums detect changed
files; they do not prove who produced them.

Choose a new output directory for each run; its parent must exist. If
installation fails, check the OS, CPU and Python version against the
[supported wheels](../installation.md); do not rename a wheel to force
installation. Do not loosen the analytical limit to make a failing check pass.

To try a named model, install the model library and follow the
[shared-model workflow](../site/shared-models.md). Check the model's units,
default scenario and limitations before running it.

# Save your first QSPQuest result

Run a synthetic one-compartment model and save a table you can read and
rerun. Choose the CLI, Python or local MCP route below. Each uses the same
6 mg bolus, clearance 0.3 L/h, volume 3 L and four post-dose observations,
and checks the result against `cp(t) = 2 * exp(-0.1 * t)` within 1e-8 mg/L.

Use source builds until matching downloads are available on the release page.
Download the release file for your machine (macOS ARM64, Linux ARM64, Linux
x86_64 or Windows x86_64) and verify it against the published checksum file.
The executables need no Rust compiler. The helper scripts need Python 3.10+
and nothing else; the Python route also installs the QSPQuest wheel.

## Scientist: use the CLI download

Extract `qspquest-cli-0.1.0-TARGET.zip` and open a terminal in its extracted
directory. It includes the executable, complete model and request, `first_run.py`,
`result_bundle.py` and legal notices. On macOS/Linux:

```sh
python3 first_run.py --out scientist-results
```

On Windows PowerShell:

```powershell
python first_run.py --out scientist-results
```

The script runs the model, checks all four observations and saves six files.
To get the JSON result without the helper, run `./qspquest run synthetic-one-compartment.qsp
first-run.json > simulation.json` on macOS/Linux, or
`.\qspquest.exe run synthetic-one-compartment.qsp first-run.json > simulation.json`
on Windows.

## Developer: use an installed wheel

The wheel includes the model, request and executable example; no source checkout
or analysis packages are needed. On macOS ARM64 with CPython 3.10+:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install --no-deps ./qspquest-0.1.0-cp310-abi3-macosx_11_0_arm64.whl
python -m qspquest.first_run --out developer-results
```

On Windows PowerShell:

```powershell
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install --no-deps .\qspquest-0.1.0-cp310-abi3-win_amd64.whl
python -m qspquest.first_run --out developer-results
```

On Linux, use your wheel's exact filename in the `pip` command. To run the
model from Python directly:

```python
from qspquest import CompiledModel
from qspquest.first_run import synthetic_inputs

source, request = synthetic_inputs()
result = CompiledModel(source).run(request)
print(result["identity"])
print(next(column for column in result["outputs"] if column["name"] == "cp"))
```

## Agent: use the local MCP download

Extract `qspquest-mcp-0.1.0-TARGET.zip` and open a terminal in its extracted
directory. It includes the server, `models/` and the helper scripts. On macOS/Linux:

```sh
python3 first_run.py --out agent-results > agent-first-run.json
```

On Windows PowerShell:

```powershell
python first_run.py --out agent-results > agent-first-run.json
```

The script starts the local server, finds the model, previews the run with
`plan_run`, runs it with `expected_plan_hash`, and downloads the full result.
`agent-first-run.json` keeps the plan, result, checks and file hashes.

For a conversational MCP client, configure absolute paths:

```json
{"mcpServers":{"qspquest":{"command":"ABSOLUTE_PATH_TO_qspquest-mcp","args":["--profile","local","--library-dir","ABSOLUTE_PATH_TO_models"]}}}
```

Use `qspquest-mcp.exe` on Windows. Ask the agent to inspect capabilities, search
for `synthetic_one_compartment`, read its full card and default scenario, inspect
a simulation plan, and simulate with `detail: full`, `outputs: ["cp"]` and the
returned plan hash. The packaged script runs entirely locally. A chat client,
by contrast, may send the model, requests and results to its AI provider;
configure it to match your data requirements.

## Interpret and retain the output

CLI and Python print this table. MCP saves the same values in its CSV and JSON:

```text
time_h,side,cp_mg_per_L
0,post,2.000000
2,post,1.637462
8,post,0.898658
24,post,0.181436
```

`post` means after the bolus at time zero. Time is in hours and `cp` is in mg/L.
Keep all six files:

| File | Purpose |
| --- | --- |
| `model.qsp` | Exact synthetic model source |
| `request.json` | Dosing, solver, tolerances, budgets and observations |
| `result.json` | Full result, including units and run identity |
| `first-run.csv` | Four numerical observations |
| `first-run.csv.manifest.json` | Column meanings, units and file hashes |
| `checks.json` | Comparison against the exact analytical answer |

The manifest uses `qspquest.table-manifest/v1`, with `producer` equal to `cli`,
`python` or `mcp`. It records `not_assessed` and `qualified: false`: passing this
check does not validate other models or clinical use. Checksums detect changed
files; they do not prove who produced them.

Choose a new output directory for each run; its parent must exist. If
installation fails, check the OS, CPU, Python version and wheel tag; do not
rename a wheel to force installation. An executable-format error means you
downloaded the wrong archive. On macOS/Linux, `chmod +x qspquest` or
`chmod +x qspquest-mcp` fixes a missing execute permission. For MCP, keep the
whole `models/` directory and both helper scripts. Do not loosen the analytical
limit to make a failing check pass.

To try a named model, install the model library and follow the
[shared-model workflow](../site/shared-models.md). Check the model's units,
default scenario and limitations before running it.

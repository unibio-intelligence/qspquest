# Save your first QSPQuest result

Run a synthetic one-compartment model and save a table you can interpret and
replay. Choose a CLI, Python or local MCP route below. Each route uses the same
6 mg bolus, clearance 0.3 L/h, volume 3 L and four post-dose observations.
It checks `cp(t) = 2 * exp(-0.1 * t)` within 1e-8 mg/L.
This check covers this synthetic case. Named models and fitting need separate
qualification.

Use source builds until matching downloads are available on the release page.
Obtain the complete matching release artifact and its trusted publisher checksum
file. Verify the archive or wheel before using it. Choose macOS ARM64, Linux
ARM64, Linux x86_64 or Windows x86_64 to match your machine; a Windows wheel or
executable cannot run on macOS or Linux. The native executables need no Rust
compiler. The guided clients need Python 3.10+ and only its standard library;
the Python route additionally installs the matching QSPQuest wheel.

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

The script runs the included `qspquest` executable with the included source and
request, checks all four observations and saves six files. For direct JSON output
without the Python helper, run `./qspquest run synthetic-one-compartment.qsp
first-run.json > simulation.json` on macOS/Linux, or
`.\qspquest.exe run synthetic-one-compartment.qsp first-run.json > simulation.json`
on Windows. Direct execution returns the complete JSON result.

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

On Linux, use the matching wheel's exact filename in the `pip` command; retain
the other macOS/Linux commands. To execute through the Python API:

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
directory. It includes the executable, `models/`, bounded stdio client and bundle
writer. On macOS/Linux:

```sh
python3 first_run.py --out agent-results > agent-first-run.json
```

On Windows PowerShell:

```powershell
python first_run.py --out agent-results > agent-first-run.json
```

The client initializes local stdio, reads capabilities and tool contracts,
discovers the model, reads its complete card and stored request, validates its
source and checks the actionable error for a missing model. It inspects
`plan_run`, then executes the same arguments with `expected_plan_hash`. It accepts
a complete inline result or retrieves every linked chunk and verifies its
checksum before interpreting the trajectory. `agent-first-run.json` retains the
plan, complete result, capabilities, checks and bundle file hashes.

For a conversational MCP client, configure absolute paths:

```json
{"mcpServers":{"qspquest":{"command":"ABSOLUTE_PATH_TO_qspquest-mcp","args":["--profile","local","--library-dir","ABSOLUTE_PATH_TO_models"]}}}
```

Use `qspquest-mcp.exe` on Windows. Ask the agent to inspect capabilities, search
for `synthetic_one_compartment`, read its full card and default scenario, inspect
a simulation plan, and simulate with `detail: full`, `outputs: ["cp"]` and the
returned plan hash. Require complete linked resources before interpreting data.
The packaged script provides the repeatable check without a provider account.
It sends only local stdio requests. A conversational client may send model
metadata, source, requests and responses to its configured model provider;
configure that client according to your data requirements.

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
The concentration falls as drug leaves the compartment. Keep all six files:

| File | Purpose |
| --- | --- |
| `model.qsp` | Exact synthetic model source |
| `request.json` | Dosing, solver, tolerances, budgets and observations |
| `result.json` | Complete native result, units, sides and engine identity |
| `first-run.csv` | Four numerical observations |
| `first-run.csv.manifest.json` | Column meanings, units, identity and input/result byte hashes |
| `checks.json` | Independent analytical comparison and its limited scope |

The manifest uses `qspquest.table-manifest/v1`, with `producer` equal to `cli`,
`python` or `mcp`. Its qualification remains `not_assessed` with `qualified: false`:
passing this numerical check does not qualify an arbitrary model or establish
clinical suitability. Checksums detect changed bytes; they do not authenticate
a producer. Retain the downloaded artifact checksum and trusted release identity
alongside the bundle.

Choose a new output directory for each run. Existing directories, files and
links are refused. Its parent must exist. If installation fails, check the OS,
CPU, Python version and wheel tag; do not rename a wheel to force installation.
An executable-format error means the wrong native archive. On macOS/Linux,
`chmod +x qspquest` or `chmod +x qspquest-mcp` resolves a missing execute bit.
For MCP, retain the entire `models/` directory and both client helpers. A numerical
or identity error should be investigated using the complete inputs and diagnostic;
do not relax the analytical limit merely to obtain a passing run.

To explore a named companion model, install its separately approved model-library
artifact and follow the [shared-model workflow](../site/shared-models.md).
Inspect its operation availability, units, stored scenario and qualification
limits before running it. The synthetic check here establishes none of that
model's scientific or fitting claims.

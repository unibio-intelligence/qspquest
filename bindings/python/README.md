# QSPQuest for Python

Compile and simulate pharmacology models locally with the QSPQuest native
engine. Models declare equations, units, parameters and dosing events. The base
package includes simulation, native individual fitting and synthetic examples.
No model library is needed for the example below.

## Install

Use CPython 3.10 or newer in a virtual environment. QSPQuest 0.1.0 is not yet
published to PyPI. From the complete project checkout or extracted Python source
archive, create an environment. On macOS or Linux:

```sh
python3 -m venv .venv
. .venv/bin/activate
```

On Windows PowerShell:

```powershell
py -3 -m venv .venv
.venv\Scripts\Activate.ps1
```

With Rust 1.98.1, Cargo and a native C/C++ compiler on `PATH`, install from
the source root:

```sh
python -m pip install .
```

The first build can take several minutes. Continue with the installation check
and synthetic example below.

### Downloaded installation files

A wheel must match the operating system and processor. For the macOS Apple
Silicon wheel, run from the directory containing that file:

```sh
python -m pip install ./qspquest-0.1.0-cp310-abi3-macosx_11_0_arm64.whl
```

For another platform, use its matching wheel filename. Wheels need no Rust
compiler. If a wheel is unavailable, install the Python source archive with
Rust 1.98.1, Cargo and a native C/C++ compiler on `PATH`:

```sh
python -m pip install ./qspquest-0.1.0.tar.gz
```

The source build installs its pinned build backend and downloads locked Cargo
dependencies. It needs network access unless those dependencies are cached and
can take several minutes. This hyphenated archive is the Python distribution;
`qspquest_0.1.0.tar.gz` is the separate R package. From an extracted Python source
archive, `python -m pip install .` is equivalent.

Check the installation:

```sh
python -c "import qspquest; print(qspquest.__version__)"
qspquest-doctor --json
```

Expect version `0.1.0`. The doctor compiles and simulates an analytical synthetic
model and reports checks, engine identity and missing optional dependencies.
An import error usually means the active interpreter differs from the one used
for installation or the native extension could not load. Use `python -m pip`
with the same interpreter and inspect the doctor's diagnostics. A wheel platform
error requires a matching wheel or the source installation route.

## Simulate and save a first result

This workflow requires only the installed base package. From any writable
working directory, choose a new output directory:

```sh
python -m qspquest.first_run --out first-result
```

The included synthetic model has a 3 L compartment and clearance 0.3 L/h. A
6 mg bolus enters `central` at time zero. The engine samples the post-event
concentration `cp` at 0, 2, 8 and 24 hours and checks it against
`cp(t) = 2 * exp(-0.1 * t)` mg/L. Expected first and last values are
`2.000000` and `0.181436` mg/L. The command refuses an existing output directory
so earlier results cannot be overwritten.

The saved files are:

| File | Purpose |
| --- | --- |
| `model.qsp` | Complete source equations, parameters and units |
| `request.json` | Solver, dosing, time units, post-event samples and budgets |
| `result.json` | Full native result, output units and engine identity |
| `first-run.csv` | Time, event side and concentration table |
| `first-run.csv.manifest.json` | Table metadata and provenance |
| `checks.json` | Analytical comparison and tolerance checks |

Replay the exact saved source and request with this complete Python command:

```sh
python -c "import json; from pathlib import Path; from qspquest import CompiledModel; p = Path('first-result'); result = CompiledModel((p / 'model.qsp').read_text()).run(json.loads((p / 'request.json').read_text())); print(json.dumps(result, indent=2))"
```

Keep input quantities, output units and event sides when interpreting results.
This illustrative model and analytical check do not establish clinical validity
or qualify a drug-specific model. Fit quality also depends on study data, model
assumptions and the observation error model.

## Optional workflows

The `analysis` extra supplies NumPy/SciPy for analysis helpers. `notebook` adds
analysis packages, pandas, matplotlib and JupyterLab. `interchange` supplies
python-libsbml for SBML/OMEX import and export. Install only the extras needed;
for example with the same local source archive:

```sh
python -m pip install './qspquest-0.1.0.tar.gz[analysis,interchange]'
qspquest-doctor --analysis --interchange --json
```

On a wheel, append the same bracketed extras to its actual filename. No extra
is needed for the first-result example.

## Security reports

Report suspected vulnerabilities privately to
helpdesk@unibiointelligence.com. Include the version, affected component, impact
and reproduction steps using synthetic inputs. Remove credentials and
confidential study data. Read the installed policy without a source checkout:

```sh
python -c "from importlib.resources import files; print(files('qspquest').joinpath('SECURITY.md').read_text(encoding='utf-8'))"
```

# QSPQuest for Python

Compile and simulate pharmacology models locally with the QSPQuest native
engine. Models declare equations, units, parameters and dosing events. The base
package includes simulation, native individual fitting and synthetic examples.
No model library is needed for the example below.

## Install

Use CPython 3.10 or newer, preferably in a virtual environment:

```sh
python -m pip install qspquest
```

Prebuilt wheels cover Linux x86_64 and ARM64 (glibc 2.28+), macOS Apple
Silicon and Intel, and Windows x86_64, so no Rust compiler is needed. Optional
extras add workflows: `qspquest[analysis]` (NumPy, SciPy), `qspquest[notebook]`
(pandas, matplotlib, JupyterLab) and `qspquest[interchange]` (SBML/OMEX).

### Build from source

On other platforms pip builds the source archive, which needs Rust 1.98.1,
Cargo and a native C/C++ compiler on `PATH`, and network access to fetch locked
dependencies (one from GitHub). From the complete project checkout or an
extracted Python source archive:

```sh
python -m pip install .
```

The first build can take several minutes. A downloaded wheel or source archive
from the GitHub release installs the same way, by passing its path to `pip`.
The hyphenated `qspquest-<version>.tar.gz` is the Python distribution;
`qspquest_<version>.tar.gz` is the separate R package.

Check the installation:

```sh
python -c "import qspquest; print(qspquest.__version__)"
qspquest-doctor --json
```

Expect version `0.1.0`. The doctor runs a small synthetic model and reports
its checks and any missing optional dependencies. An import error usually means
you installed into a different Python interpreter or the native extension could
not load; use `python -m pip` with the same interpreter. A wheel platform error
means you need a matching wheel or the source install.

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

This is an illustrative synthetic model, not a clinical or drug-specific model.

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

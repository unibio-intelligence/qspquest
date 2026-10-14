# Run a shared model

Use the [public model library](https://github.com/unibio-intelligence/qspquest-models)
for OMEX archives, named presets, model cards and stored results. With Docker,
[mount the library once](containers.md#mount-the-model-library-once),
then discover and run models through CLI/Python, R, notebooks or MCP. All use the
same catalog and selected archive versions.

The companion library archive contains `catalog.json`, `models/`, and its
`qspquest_models` Python loader. The engine image supplies the solver. Models
are mounted read-only rather than embedded in the image, so the library can be
updated independently. Keep the exact library archive and its published checksum
with the image identity when saving an analysis. A mutable directory is not an
immutable release pin; switching libraries requires checking changed model
versions and qualification limits.

The catalog contains 72 model identities, including
inspection-only versions. `--runnable` restricts CLI discovery to versions with
executable scenarios. A successful search does not mean a model supports
simulation or fitting. Read each scenario’s recorded evidence and current-engine qualification status before interpreting a run.
The loader verifies archive and indexed-member hashes and runs the archived
native QSPQuest projection. Source numeric coordinates may lack dimensional
qualification; inspect `dimensional_qualification` and the model card.
Model-specific licenses apply in addition to the engine license.

## Native Python

Install the engine using [native installation](../installation.md#python), then
install the companion model-library package from its extracted root:

```sh
python -m pip install /absolute/path/to/qspquest-models
export QSPQUEST_LIBRARY_DIR=/absolute/path/to/qspquest-models
qspquest-library --library "$QSPQUEST_LIBRARY_DIR" --search rituximab
```

The command and `python -m qspquest_models` use the companion loader's same
search, options and discovery records. Search includes the catalog summaries,
such as `B-cell recovery`. With the native engine and companion loader installed,
these commands return identical JSON records, including availability, unit status,
fitting support and current-engine qualification:

```sh
qspquest-library --search 'B-cell recovery' --format json
python -m qspquest_models --search 'B-cell recovery' --format json
```

Both commands use `QSPQUEST_LIBRARY_DIR` unless `--library` overrides it;
otherwise they look in the loader's package checkout. A package installed without
the catalog and archives requires an explicit library directory. `--model` lists
experiment IDs, `--inputs` inspects a named dynamic preset without executing an
engine, and `--all-readouts` exports all outputs and states instead of primary
readouts. Source replays retain their availability label and do not permit
exploratory dose or initial-state changes.

Pin an exact catalog version with `--model-version VERSION` when using `--model`.
An unavailable pin exits with status 2 and code `model_version_unavailable`
before execution. JSON errors include bounded migration `details` with requested
and available versions. The R helper accepts `version = "VERSION"` in both
`models$scenarios(model, version = "VERSION")` and
`models$run(model, scenario, version = "VERSION")`. Omitting the pin selects the
current catalog default; distributed alternate archives use their versioned
guide's checksum-pinned Python archive command.

Failures report the problem, cause and suggested fix to stderr. Add
`--json-errors` for a `qspquest.library-error/v1` object with `code`, `problem`,
`cause` and `fix`. Exit status is 2 for invalid arguments or unknown IDs, and 1
for library, archive, engine or export failures. Discovery JSON remains on stdout.
Without the companion loader, `qspquest-library --help` explains how to install
it; the complete option help is available after installation.

```python
import qspquest
library = qspquest.model_library()
model = library.load("qspquest_serkland_2025_rituximab_ms_pkpd")
result = model.run("population-typical-1000mg-full-ode")
assert len(result["times"]) == 201
model.write_csv(result, "rituximab.csv")
```

The result retains event sides, units, engine identity, archive checksum and
named outputs including `rituximab_concentration_mg_l` and `cd19_cells_mm3`.
No CLI path is required when the native Python engine is installed.

## R and MCP

The [R Docker example](containers.md#use-r-in-docker) uses
`library(qspquest); models <- model_library()` for discovery and named runs.
The library helper requires Python and the loader as well as an installed
QSPQuest Python engine or CLI; the Docker R image supplies them. This extra
bridge requirement applies to shared archives; native R models need neither
Python nor the shared library.

MCP uses the archived native output IDs, such as `source_output_0`; the model's
`native_execution.outputs` metadata maps them to descriptive readout names. The
Python/R library helpers return those descriptive names.

The [MCP setup](../../bindings/mcp/README.md) supports the same local catalog or
an immutable HTTPS library archive pinned by an independently trusted SHA-256.
The engine and library public repositories and assets still need publication;
a ready-to-use hosted download URL is not yet available. Local Docker workflows
use the extracted companion archive and run offline after image construction.

## Explore local versus systemic receptor occupancy

The [Target-Gated Cargo QSP tutorial](https://github.com/unibio-intelligence/qspquest-models/blob/main/tutorials/qsp-gated-cargo.md)
uses a synthetic 11-state mechanism, with [Python](https://github.com/unibio-intelligence/qspquest-models/blob/main/tutorials/qsp-gated-cargo.ipynb)
and [R](https://github.com/unibio-intelligence/qspquest-models/blob/main/tutorials/qsp-gated-cargo.Rmd)
walkthroughs. Run its existing OMEX presets, compare tumor and central receptor
occupancy, reconcile the state pools and export the analysis. It requires no
external dataset and does not add a fitting claim.

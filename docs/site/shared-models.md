# Run a shared model

Use the [public model library](https://github.com/unibio-intelligence/qspquest-models)
for OMEX archives, named presets, model cards and stored results. With Docker,
[mount the library once](containers.md#mount-the-model-library-once),
then discover and run models through CLI/Python, R, notebooks or MCP. All use the
same catalog and selected archive versions.

The companion library archive contains `catalog.json`, `models/`, and its
`qspquest_models` Python loader. The engine image supplies the solver. Models
are mounted read-only rather than embedded in the image, so the library can be
updated independently. When saving an analysis, record the library archive and
its checksum along with the image. If you switch libraries, recheck any changed
model versions.

The catalog contains 72 model entries, including some that are inspection-only.
`--runnable` limits CLI search to versions with runnable scenarios. Some models
keep their source's numeric scale without verified units; check
`dimensional_qualification` and the model card before interpreting a run.
Model-specific licenses apply in addition to the engine license.

## Native Python

Install the engine using [native installation](../installation.md#python), then
install the companion model-library package from its extracted root:

```sh
python -m pip install /absolute/path/to/qspquest-models
export QSPQUEST_LIBRARY_DIR=/absolute/path/to/qspquest-models
qspquest-models --library "$QSPQUEST_LIBRARY_DIR" --search rituximab
```

`qspquest-models` and `python -m qspquest_models` are equivalent. Search
includes the catalog summaries, such as `B-cell recovery`. With the native engine
and companion loader installed, both return the same JSON records, including
availability, unit status and fitting support:

```sh
qspquest-models --search 'B-cell recovery' --format json
python -m qspquest_models --search 'B-cell recovery' --format json
```

Both commands use `QSPQUEST_LIBRARY_DIR` unless `--library` overrides it;
otherwise they look in the loader's package checkout. A package installed without
the catalog and archives requires an explicit library directory. `--model` lists
experiment IDs, `--inputs` inspects a named dynamic preset without executing an
engine, and `--all-readouts` exports all outputs and states instead of primary
readouts. Source-replay scenarios cannot be changed (no dose or initial-state
edits).

Pin an exact catalog version with `--model-version VERSION` when using `--model`.
An unavailable pin exits with status 2 and code `model_version_unavailable`
before execution. JSON errors list the requested and available versions in
`details`. The R helper accepts `version = "VERSION"` in both
`models$scenarios(model, version = "VERSION")` and
`models$run(model, scenario, version = "VERSION")`. Omitting the pin selects the
current catalog default; distributed alternate archives use their versioned
guide's checksum-pinned Python archive command.

Failures report the problem, cause and suggested fix to stderr. Add
`--json-errors` for a `qspquest.library-error/v1` object with `code`, `problem`,
`cause` and `fix`. Exit status is 2 for invalid arguments or unknown IDs, and 1
for library, archive, engine or export failures. Discovery JSON remains on stdout.

```python
import os
from qspquest_models import Library
library = Library(os.environ["QSPQUEST_LIBRARY_DIR"])
model = library.load("qspquest_serkland_2025_rituximab_ms_pkpd")
result = model.run("population-typical-1000mg-full-ode")
assert len(result["times"]) == 201
model.write_csv(result, "rituximab.csv")
```

The result retains event sides, units, engine identity, archive checksum and
named outputs including `rituximab_concentration_mg_l` and `cd19_cells_mm3`.
No CLI path is required when the native Python engine is installed.

## R and MCP

The model library's R helper, `tutorials/qspquest-models.R`, provides
`qspquest_library(root)` for discovery and named runs; the
[R Docker example](containers.md#use-r-in-docker) uses it. The helper requires
Python and the loader as well as an installed QSPQuest Python engine or CLI;
the Docker R image supplies them. This extra
bridge requirement applies to shared archives; native R models need neither
Python nor the shared library.

MCP uses the archived native output IDs, such as `source_output_0`; the model's
`native_execution.outputs` metadata maps them to descriptive readout names. The
Python/R library helpers return those descriptive names.

The [MCP setup](../../bindings/mcp/README.md) supports the same local catalog or
a pinned HTTPS library archive with its SHA-256. A hosted download URL is not
yet available. Local Docker workflows
use the extracted companion archive and run offline after image construction.

## Explore local versus systemic receptor occupancy

The [Target-Gated Cargo QSP tutorial](https://github.com/unibio-intelligence/qspquest-models/blob/main/tutorials/qsp-gated-cargo.md)
uses a synthetic 11-state mechanism, with [Python](https://github.com/unibio-intelligence/qspquest-models/blob/main/tutorials/qsp-gated-cargo.ipynb)
and [R](https://github.com/unibio-intelligence/qspquest-models/blob/main/tutorials/qsp-gated-cargo.Rmd)
walkthroughs. Run its existing OMEX presets, compare tumor and central receptor
occupancy, reconcile the state pools and export the analysis. It needs no
external data and does not include fitting.

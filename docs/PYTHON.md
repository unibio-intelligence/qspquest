# Python binding

The implemented `qspquest-python` crate builds the Python package `qspquest`
with PyO3 0.29.2 and maturin 1.15.0. It calls the existing model compiler and
scientific execution API. The extension is configured for CPython's 3.10+
stable ABI; verification currently covers CPython 3.11 on macOS arm64 only.
Other Python versions, operating systems, free-threaded interpreters and
subinterpreters remain unqualified.

## Use

```python
import json
from pathlib import Path
from qspquest import CompiledModel, QSPQuestError

model = CompiledModel(Path("model.qsp").read_text())
request = json.loads(Path("request.json").read_text())
result = model.run(request)
# result has times, sides, named output columns with units, and execution identity.
```

`CompiledModel` accepts model text, model JSON text or a dict. Compile once and
reuse it for independent requests. `run` accepts a request dict or JSON string
and returns a fresh `qspquest.result/v0.1` dict. `run_json` returns the same
complete result as a JSON string. `model_content_hash`, `output_names` and
`output_units` expose compiled metadata. `format_model` emits canonical text.
Type hints and `py.typed` are included.

## Population data fits

`population_fit_dataset` converts a bounded subset of NONMEM-style `ID`,
`TIME`, `EVID`, `DV`, `MDV`, `AMT`, `CMT`, `RATE`, `ADDL`, `II`, and `SS` rows into
a FOCEI or SAEM request. Supply a `qspquest.fit/v0.1` template whose
`problem.kind` is `focei` or `saem`, with one subject objective containing a
versioned sensitivity request and an empty regimen schedule and observation
list. The template owns model parameters, fit bounds, solver settings and
budgets. The dataset owns dosing, sampling times and measured values.

```python
from qspquest import CompiledSensitivities, population_fit_dataset

dataset = population_fit_dataset(
    rows, fit_template,
    compartments={1: "central"},
    endpoints={2: {
        "output": "cp", "value_unit": "mg/L",
        "error": {"additive_sd": {"value": 0.05, "unit": "mg/L"},
                  "proportional_sd": 0.0},
    }},
    time_unit="h", amount_unit="mg", observation_side="post",
)
fitted = CompiledSensitivities(model, ["cl"]).fit_population_data(dataset)
population = fitted["result"]["population"]
```

`rows` may be a list of dicts or a pandas data frame; pandas is optional.
Dose `CMT` values need explicit model-target mappings, and included observation
`CMT` values need explicit output, value-unit and Gaussian-error mappings.
`RATE > 0` also requires `rate_unit`. `SS`, unsupported event types, unknown
columns (including unmapped covariates such as `WT`), and observations carrying
dose fields are rejected. `MDV = 1` omits
the row from fitting, including a missing `DV`. Same-time dose/observation
semantics come from the explicit `observation_side`, never row order.

`dataset.request` is the complete native request. `dataset.mapping` links each
zero-based source row to its subject and zero-based result row. The fit result
retains that mapping and subject IDs. For FOCEI, `fitted_observations` adds
`subject_id` and `source_row` to the engine's PRED/IPRED rows for both FOCEI
and SAEM. A one-effect SAEM request may estimate a common additive residual
SD with `additive_error_fit`; its observations must share one output, zero
proportional error, and the declared starting SD. Two-effect SAEM can also
estimate this SD. Fixed normal additive/proportional and
lognormal residual errors are available in the native observation contract;
left-censored observations use `censoring: "left"` and place the quantification
limit in `value`. A fitted additive SD can use left-censored normal rows.
One-effect native SAEM can estimate up to four bounded exponential,
subject-constant covariate effects. One- and two-effect SAEM can report local
marginal-objective uncertainty under the
limits in the [SAEM guide](SAEM.md). The dataset convenience adapter still
requires explicit native request construction for these fields.
`burn_in_iterations` can set the SAEM
exploration length independently of `max_iterations`. Inspect
`population["status"]` before treating estimates as converged. Native
scientific errors and conversion errors are `QSPQuestError` with structured
codes. This adapter does not infer covariates, censoring, units, or missing
values beyond explicit `MDV`.

`QSPQuestError` preserves `code`, `time`, `expression`, `line`, `column` and the
complete `diagnostic` where supplied by the compiler/engine. A failure returns
no partial result and does not replace the compiled model or retain request
bindings. Plain Python argument-type errors still use Python `TypeError`.
Non-finite dict values are rejected before serialization; Rust validates the
scientific request, quantities and budgets.

## Concurrency and ownership

The native compiled model is immutable and shared through `Arc`; each run
creates independent parameter/covariate bindings and solver scratch state.
Compilation, execution and result serialization release the Python GIL using
`Python::detach`. The same model can serve independent `ThreadPoolExecutor`
requests. Dict encoding and decoding run in Python; this is not a zero-copy
NumPy interface. Returned arrays are ordinary independent Python lists.
There is no implicit Rust thread pool or change to the single-threaded WASM
build. Concurrency limits belong to the caller.

Scientific callback/output/event budgets apply unchanged. Thread cancellation
does not interrupt an active native solve; use a supervising process when a hard
wall-clock deadline is required. Backend worker integration and managed
operation qualification remain outstanding.

## Build and test

Build from the full workspace:

```sh
python -m pip install maturin==1.15.0
maturin build --manifest-path bindings/python/Cargo.toml --release --locked
python -m pip install /path/to/generated/qspquest-*.whl
cargo build -p qspquest-cli --release --locked
QSPQUEST_TEST_CLI="$PWD/target/release/qspquest" python bindings/python/tests/test_binding.py
```

Wheel builds after dependency fetch can run with `--offline`. Python build
metadata lives in the workspace-root `pyproject.toml`. `maturin sdist` preserves
the workspace manifests, lockfile, compiler-fingerprint sources, and declared
example inputs. Build an extracted archive with the same `maturin build` command
from its root. Rust dependencies must already be cached for `--offline`.

An independently extracted archive passes a locked offline release build and
all six installed-wheel integration tests on CPython 3.11 macOS arm64. See
the separately retained packaging evidence.
This verifies standalone buildability and source identity; it does not establish
byte-identical rebuilds or other platforms. The package is not published.

References: [PyO3 parallelism](https://pyo3.rs/v0.29.2/parallelism) and
[maturin mixed-project configuration](https://www.maturin.rs/config).

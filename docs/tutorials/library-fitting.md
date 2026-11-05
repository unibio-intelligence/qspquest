# Discover and run an archived synthetic fitting workflow

Use local MCP to pick a model version, choose one of its fitting workflows,
preview the plan, run it and save the diagnostics. A model that can be simulated
cannot necessarily be fitted; only models with a ready fitting workflow can.

You need Python 3.10+, a matching QSPQuest MCP executable, an extracted model
library that includes a ready fitting workflow, and the client below (standard
library only). Everything runs locally. If you drive this from a chat client
instead, it may send prompts and results, including observations, to its AI
provider.

The model library includes synthetic fitting workflows for each of
`qspquest_diffdose_2026_idr_benchmark` (scalar recovery) and
`qspquest_serkland_2025_rituximab_ms_pkpd` (individual recovery), at version `0.3.0`.
Use the supported `diffsol_bdf-start-1` through `diffsol_bdf-start-3` workflows.
Each is a synthetic
recovery test: fit known simulated data and check the true values come back.

## Inspect first

Use `search_models` and `get_model` with an exact `model_id` and `version`.
Read the `fitting` object, units and limitations. For
`qspquest.library-fitting/v1`, each workflow has:

| Field | Meaning |
| --- | --- |
| `id` and `operation` | Workflow ID and the fitting tool it uses (scalar or individual) |
| `parameters` | Names, units, bounds, starting values, true values and recovery tolerances |
| `native_request` | Observations, residual errors, solver, dosing, priors and estimator settings |
| `synthetic_observations` | Observation data stored in the same archive |
| `qualification` | How the workflow was checked, and the result |
| `limitations` | What the workflow does not cover |

A draft has `qualification.status: not_assessed` and cannot be fitted.
V1 workflows cover uncensored normal errors with fixed additive/proportional
components, using scalar or individual estimation. Censoring, lognormal errors,
priors and population fitting are not supported here. These workflows do not
reproduce the original paper's population analysis. Unit `1` means no physical
unit is attached (the value is on the source's numeric scale); do not report it
as a clearance or volume in physical units just because of the parameter's name.

Read `qspquest://schema/library-fitting` for the full schema. The workflow
fixes the data, error model, bounds, starting values, solver and limits; it runs
only as stored. To fit your own variations, write your own model and use the
regular fitting tools.

## Execute and retain the result

From a source checkout, the [client](../examples/mcp_library_fit.py) and its
[stdio helper](../examples/stdio_rpc.py) are in `docs/examples/`. Build the
server, then replace the model, version and workflow placeholders with values
returned by `get_model`:

```sh
cargo build --release --locked -p qspquest-mcp
python3 docs/examples/mcp_library_fit.py target/release/qspquest-mcp \
  --library /absolute/path/to/extracted-model-library \
  --model MODEL_ID --version MODEL_VERSION --workflow WORKFLOW_ID \
  --out library-fit-results
```

On Windows PowerShell use `python`, `target\release\qspquest-mcp.exe`, and the
absolute library path on one line. Keep the library download intact; it must
contain `catalog.json` and its archives.

For the normalized scalar example included in the source checkout, from the
source root:

```sh
python3 docs/examples/mcp_library_fit.py target/release/qspquest-mcp \
  --library bindings/mcp/tests/library \
  --model qspquest_diffdose_2026_idr_benchmark --version 0.3.1 \
  --workflow diffsol_bdf-start-1 \
  --out normalized-scalar-fit
```

This selects the archive's `diffsol_bdf-start-1` workflow. It estimates
`source_symbol_5`, the normalized elimination parameter, with BDF and bounds
0.2–1.2 in unit `1`. The true value is 0.7 and the fit must recover it within
1e-5. The same request is available through
`qspquest://examples/fit/library-scalar`.

The client stops early if the model has no ready fitting workflow. Otherwise it
previews the plan, runs it with `expected_plan_hash`, downloads the result and
saves:

| File | Purpose |
| --- | --- |
| `model-card.json` | The model's fitting workflow and limitations |
| `arguments.json` | Exact inputs, including the plan hash |
| `plan.json` | The previewed plan |
| `result.json` | Full result, including estimates and run identity |
| `checks.json` | Convergence, parameter recovery and diagnostic summary |

Choose a new output directory whose parent exists. The JSON printed on stdout
reports file hashes and parameter checks.

## Interpret status and recover

A passing run has `fit_status: converged` and every parameter-recovery check
passing. Also check evaluation/iteration counts, parameters at their bounds and,
for individual fitting, the projected-gradient norm. Do not report a parameter
stuck at a bound or a non-converged estimate as a final result.

The client saves failed or non-converged results and exits with status 1.
After an input error or timeout, read the error message before retrying. A
simulation-only model returns `fitting_unsupported`. A modified request returns
`fitting_contract`: use the stored request unchanged. Do not relax recovery
tolerances to make a run pass.

These are synthetic tests. They say nothing about clinical use, real-world data
or the original study's population estimates.

## Select an exact model version

Inspect the catalog before choosing a model. In Python, call
`library.load(model_id, version=version)`; in MCP, provide both `model_id` and
`version`. The library CLI accepts `--model-version VERSION` alongside
`--model MODEL_ID`. Omitting the version selects the catalog's current version.
A missing version fails with an error; another version is never substituted.

Before changing versions, check the new model card, scenario and units. Reapply
any overrides using the new version's parameter names and units, then compare
results.

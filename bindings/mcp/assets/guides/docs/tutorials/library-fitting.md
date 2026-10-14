# Discover and run an archived synthetic fitting workflow

Use local MCP to inspect an exact model version, select its declared fitting
workflow, inspect a preparation plan, run it and retain diagnostics. Fitting
readiness is separate from simulation availability. An archive that provides
simulation scenarios does not thereby support fitting.

You need Python 3.10+, a matching QSPQuest MCP executable, an extracted approved
model-library artifact containing a ready fitting workflow, and the client below.
The client uses only the standard library. Execution is local; no model provider
account is required. A conversational agent may forward its prompts and tool
responses to its configured provider, including model metadata and observations.

The model library includes six exact synthetic workflows for each of
`qspquest_diffdose_2026_idr_benchmark` (scalar recovery) and
`qspquest_serkland_2025_rituximab_ms_pkpd` (individual recovery), at version `0.3.0`.
IDs are `diffsol_bdf-start-1` through `diffsol_bdf-start-3` and
`diffsol_rodas5p-start-1` through `diffsol_rodas5p-start-3`. The latter preserves
mass/cell units and the explicit ODE-inferred coupling annotation; fitting evidence
qualifies only the archived synthetic recovery cases.

## Inspect first

Use `search_models` and `get_model` with an exact `model_id` and `version`.
Read the complete `fitting` object, units, coordinate status, operation availability
and qualification limits. For `qspquest.library-fitting/v1`, each workflow has:

| Field | Meaning |
| --- | --- |
| `id` and `operation` | Exact workflow ID and supported scalar or individual tool |
| `parameters` | Native names, declared units, bounds, starters, synthetic truth and recovery limits |
| `native_request` | Complete observations, residual errors, solver, regimen, priors and estimator controls |
| `synthetic_observations` | Hash-indexed observation JSON in the same archive |
| `qualification` | Recorded independent method, status, evidence and diagnostic interpretation |
| `limitations` | Scope that must accompany interpretation |

A draft has `qualification.status: not_assessed` and does not enable fitting.
V1 workflows cover uncensored normal errors with fixed additive/proportional
components, using scalar or individual estimation. Censoring, lognormal errors,
priors and population estimators require a separate supported contract.
A ready declaration must retain hash-bound observations, numerical results and
evidence matching its model and request. Its scope is the exact synthetic recovery
fixture. It does not reproduce a source population analysis. Unit `1` retains
source numeric coordinates; do not label such estimates as physically qualified
clearance, volume or concentrations merely from a descriptive source name.

Read `qspquest://schema/library-fitting` for the declaration schema and the
native scalar/individual request schemas for `native_request`. The complete
request fixes observed values, residual model, bounds, starters, priors, solver,
events and work budgets. Modifying any of those fields requires separate
qualification. For exploratory fitting, use an explicitly labeled user-authored
model and its supported native fitting interface.

## Execute and retain the result

The complete source archive provides the
[client](qspquest://documentation/files/docs/examples/mcp_library_fit.py) and its
[stdio helper](qspquest://documentation/files/docs/examples/stdio_rpc.py) in `docs/examples/`. Native MCP downloads
include them as `library_fit.py` and `stdio_rpc.py`. With that download extracted,
replace the model, version and workflow placeholders with values returned by
`get_model`:

```sh
python3 library_fit.py ./qspquest-mcp \
  --library /absolute/path/to/extracted-model-library \
  --model MODEL_ID --version MODEL_VERSION --workflow WORKFLOW_ID \
  --out library-fit-results
```

On Windows PowerShell use `python`, `.\qspquest-mcp.exe`, and the actual absolute
library path on one line. The selected library must contain `catalog.json`, its
archives and required availability records; retain the complete download.

For the normalized scalar example included in the complete source archive,
from that archive's root with a matching built or downloaded native executable:

```sh
python3 docs/examples/mcp_library_fit.py /absolute/path/to/qspquest-mcp \
  --library bindings/mcp/assets/library \
  --model qspquest_diffdose_2026_idr_benchmark --version 0.3.0 \
  --workflow diffsol_bdf-start-1 \
  --out normalized-scalar-fit
```

This selects the archive's `diffsol_bdf-start-1` workflow. It estimates
`source_symbol_5`, the normalized elimination parameter, with BDF and bounds
0.2–1.2 in declared unit `1`. Its synthetic truth is 0.7 with an absolute recovery
limit of 1e-5. The archive supplies the complete observations, native request and
recovery declaration. The same request is available through
`qspquest://examples/fit/library-scalar`. It supplies no clinical observations,
physical-unit qualification or reproduction of a source population fit.

The client refuses unavailable fitting before numerical work. For a supported
workflow it obtains the complete plan, binds execution to `expected_plan_hash`,
retrieves the complete result and saves:

| File | Purpose |
| --- | --- |
| `model-card.json` | Complete fitting declaration, archive identity and scope |
| `arguments.json` | Exact execution inputs and plan guard |
| `plan.json` | Complete prospective preparation and qualification limits |
| `result.json` | Complete native result, estimates and engine identity |
| `checks.json` | Convergence, parameter recovery and diagnostic summary |

Choose a new output directory whose parent exists; existing paths and links are
refused. The JSON printed on stdout reports file hashes and parameter checks.
Keep the original library archive, its trusted checksum and executable identity
alongside these files. Byte checksums do not authenticate a producer or establish
scientific qualification.

## Interpret status and recover

A passing synthetic example has `fit_status: converged` and every declared
parameter-recovery check passing. Inspect evaluation/iteration counts, active
bounds and, for individual fitting, the projected-gradient norm. A parameter near
a bound or a nonconverged estimate is a diagnostic candidate. Do not report it as
an accepted fit or qualified uncertainty estimate.

The client retains returned failed/nonconverged results and exits with status 1.
An admission error or transport timeout cannot supply a completed numerical fit;
inspect its diagnostic before retrying. A simulation-only model returns a specific
`fitting_unsupported` reason. A changed native request returns `fitting_contract`:
restore the declared fixture or qualify a separate exploratory analysis. Never
flip readiness flags or relax recovery tolerances simply to obtain success.

Recorded archive recovery applies to its stated source, request, targets and
engine identities. Every current result retains its own identity and current-run
qualification remains unassessed unless applicable independent evidence is
separately bound to it. This workflow grants no claim about clinical suitability,
source population estimation, real-world data or other endpoints/estimators.

## Select an exact model version

Inspect the catalog before choosing a model. In Python, call
`library.load(model_id, version=version)`; in MCP, provide both `model_id` and
`version`. The library CLI accepts `--model-version VERSION` alongside
`--model MODEL_ID`. Omitting the version selects the catalog's current version.
An unavailable version fails before execution; it is never replaced automatically.

Before changing versions, check the model card, bindings, scenario, units and
operation availability. Reapply overrides using the declared names and units,
then compare results under the scientific tolerances your workflow requires.
Archive integrity and simulation availability alone do not establish fitting
support or physical-unit qualification.

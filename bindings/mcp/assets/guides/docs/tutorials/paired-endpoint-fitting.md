# Fit concentration and cell measurements together

This tutorial fits two parameters of a coupled synthetic antibody/cell model
using both concentration and cell-count measurements. It preserves the dose
history and exact pre/post observation rows in a checksummed OMEX archive.
The parameters and measurements are illustrative. This does not reconstruct a
clinical dataset or establish a recommended dose interval.

## Prerequisites and installation

Use Python 3.11 or newer and a Rust toolchain to build the Python package from
the released source checkout. Run these commands at the checkout root. The
interchange extra supplies the supported libSBML dependency; on minimal
Debian/Ubuntu systems install `libexpat1` if libSBML reports that it is missing.
No data download, vendor software, platform account or model-library package is
needed. The complete [synthetic model](qspquest://documentation/files/conformance/models/paired-antibody-cell.json)
and [runnable client](qspquest://documentation/files/docs/examples/paired_endpoint_fit.py) ship with this workflow.

```sh
python3 -m venv .venv-paired
.venv-paired/bin/python -m pip install '.[interchange]'
.venv-paired/bin/python docs/examples/paired_endpoint_fit.py prepare --output paired-input
```

`prepare` independently integrates the two equations with piecewise RK4 at 512
and 1024 steps per day. It refuses a reference whose difference exceeds
`1e-7 + 1e-7*abs(fine_value)` at any declared endpoint sample. It fixes all
40 synthetic observations, six complete requests, native admission plans and
source-row mappings before fitting. It verifies an exact archive round trip
and writes `prepared.json`, `paired-fit.omex` and `preparation.json`.

Copy `prepared_sha256` from the printed preparation receipt into this command:

```sh
.venv-paired/bin/python docs/examples/paired_endpoint_fit.py run \
  --prepared paired-input/prepared.json \
  --prepared-sha256 REPLACE_WITH_PRINTED_PREPARED_SHA256 \
  --output paired-results
```

Both output directories must be new. Files are never overwritten; use a new
directory to repeat the workflow. `run` verifies the preparation and archive
digests, checks that native admission identities have not changed, and retains
every fit and control. This is a numerical exercise with multiple fits and
independent reference calculations; it can take several minutes.

## Model and observations

The two equations in physical coordinates are

```text
dC/dt = -cl*C - cell_loss*C*B
dB/dt = baseline*turnover - drug_effect*C*B - turnover*B
```

`C` is antibody concentration in mg/L, `B` is cell density in 1/mm³, and time is
days. A bolus increases `C` by dose/volume while `B` remains continuous. The
synthetic schedule gives 1000 mg on day 0 and 500 mg on day 180, followed through
day 360. Both sides of each bolus are sampled explicitly. These are idealized
boluses; they do not represent a clinical infusion.

The fitted parameters are `cl` in 1/day and `drug_effect` in L/(mg·day). Volume,
cell-dependent concentration loss, baseline and cell turnover are fixed.
Each observation declares its own error: concentration uses additive SD
0.05 mg/L and cells use additive SD 0.2/mm³. Errors are independent normal
errors with zero proportional component. Their units cannot be pooled into one
shared SD. The native likelihood includes the `log(SD)` normalization term.

The data are long-form records with exactly `row`, `endpoint` and `value`.
`row` is the zero-based index of the explicit result grid, not a time rounded to
a nearest sample. The endpoint mapping supplies output name, value unit and
residual error. `individual_fit_dataset(records, template, endpoints=...)`
retains source order and every source-row index. It refuses duplicate
row/output keys, missing values, boolean indices, unknown endpoints and extra
columns. Empty samples and an explicit observation grid are required. No
sorting, interpolation, missing-data imputation or unit inference occurs.

`dataset.plan(model)` and `CompiledSensitivities.plan_fit(request)` check the
native parameter, bound, unit, observation and work-limit contracts without
integration. `CompiledSensitivities.objective(objective_request)` evaluates the
Gaussian likelihood and ordered sensitivity gradient without optimization.
`dataset.fit(model)` returns the full native fit result, admission plan and
source-row mapping. Native `fit.status` determines convergence; successful
admission alone does not imply convergence.

## Archive contract

`export_omex(model, scenarios, fits={name: native_request})` selects
`qspquest.omex.individual-fit/v1` when at least one fit is supplied. The archive
contains the supported SBML rate-rule representation and exact native fit JSON.
Fits preserve parameter order, starting values, bounds, units, error contracts,
optimizer limits and fixed bolus/infusion history. Import and export both invoke
native admission without fitting. Select a fit explicitly with
`archive.plan_fit(name)` or `archive.fit(name)`.

The statistical profile supports uncensored individual fitting, empty priors,
fixed endpoint-specific additive normal SDs, and the two native sensitivity
solvers. It rejects population/pooled fits, lognormal or proportional errors,
censoring, nonempty priors and additional event semantics. The native fitting
API supports broader contracts separately. The original `qspquest.omex/v1`
run-only profile remains readable; omitting `fits` retains that profile.
Existing `qspquest.*` request/schema identifiers remain the stable wire format.

This is a QSPQuest statistical extension. It does not execute arbitrary SED-ML
or import a vendor estimation project. Hashes detect changed bytes; a member
hash or an archive round trip alone does not authenticate the author or validate
the statistical model.

## Expected output and interpretation

The final `report.json` records all six fits (three starting points with BDF
and Rodas5P) and five failure controls. `passed: true` requires native
convergence, recovery within 1e-5 for `cl` and 1e-6 for `drug_effect`, complete
observations, independent likelihood/gradient/trajectory agreement, and the
expected refusals. A false result exits with status 1 and retains failed cases.
Do not remove failures or loosen acceptance limits to obtain a pass.

Each case retains its full native result, prediction grid and diagnostic rows.
Diagnostics include source row, result row, endpoint, time/side, observed and
predicted values, units, residuals, standardized residuals and endpoint
negative-log-likelihood sums. These are ordinary individual-fit residuals,
not CWRES or NPDE. Negative likelihood contributions can occur with small SD;
the normalized density can exceed one.

Review `evaluation_limit`, `iteration_limit` or `line_search_failed` as failed
convergence. Check starting points, bounds, data and residual assumptions before
increasing work limits. A changed compiled artifact requires a new preparation.
Wrong units and result rows produce admission failures; do not silently convert
them into another experiment.

Synthetic parameter recovery does not establish parameter identifiability,
confidence-interval coverage, population variability, clinical source-data
reproduction or an optimal dosing regimen. Equal calendar endpoints after
different repeat-dose times have unequal follow-up. A scientific regimen
comparison must state whether its observation window is aligned to calendar
time or to time since treatment.

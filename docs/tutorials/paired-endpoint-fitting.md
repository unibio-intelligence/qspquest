# Fit concentration and cell measurements together

This tutorial fits two parameters of a coupled synthetic antibody/cell model
using both concentration and cell-count measurements, and saves the model,
doses and observations in an OMEX archive. The parameters and measurements are
illustrative, not clinical data.

## Prerequisites and installation

Use Python 3.11 or newer and a Rust toolchain to build the Python package from
the source checkout. Run these commands at the checkout root. The interchange
extra installs libSBML; on minimal Debian/Ubuntu systems install `libexpat1` if
libSBML reports that it is missing. No downloads or accounts are needed. The
complete [synthetic model](../../conformance/models/paired-antibody-cell.json)
and [runnable client](../examples/paired_endpoint_fit.py) ship with this workflow.

```sh
python3 -m venv .venv-paired
.venv-paired/bin/python -m pip install '.[interchange]'
.venv-paired/bin/python docs/examples/paired_endpoint_fit.py prepare --output paired-input
```

`prepare` computes an independent reference solution with RK4 at 512 and 1024
steps per day, and stops if the two differ by more than
`1e-7 + 1e-7*abs(fine_value)` at any sample. It then fixes the 40 synthetic
observations and three BDF fit requests, and writes `prepared.json`, `paired-fit.omex`
and `preparation.json`.

Copy `prepared_sha256` from the printed preparation receipt into this command:

```sh
.venv-paired/bin/python docs/examples/paired_endpoint_fit.py run \
  --prepared paired-input/prepared.json \
  --prepared-sha256 REPLACE_WITH_PRINTED_PREPARED_SHA256 \
  --output paired-results
```

Both output directories must be new; files are never overwritten. `run` checks
that the prepared files are unchanged, then runs and saves every fit. It can
take several minutes.

## Model and observations

The two equations in physical coordinates are

```text
dC/dt = -cl*C - cell_loss*C*B
dB/dt = baseline*turnover - drug_effect*C*B - turnover*B
```

`C` is antibody concentration in mg/L, `B` is cell density in 1/mm³, and time is
days. A bolus increases `C` by dose/volume while `B` remains continuous. The
synthetic schedule gives 1000 mg on day 0 and 500 mg on day 180, followed through
day 360. Observations are taken just before and just after each bolus.

The fitted parameters are `cl` in 1/day and `drug_effect` in L/(mg·day). Volume,
cell-dependent concentration loss, baseline and cell turnover are fixed.
Each observation declares its own error: concentration uses additive SD
0.05 mg/L and cells use additive SD 0.2/mm³. Errors are independent normal
errors with no proportional component. Because the units differ, the two
endpoints cannot share one SD.

The data are long-form records with exactly `row`, `endpoint` and `value`.
`row` is the zero-based index into the result's time grid, not a time. The
endpoint mapping supplies output name, value unit and residual error.
`individual_fit_dataset(records, template, endpoints=...)` keeps row order and
rejects duplicate row/output pairs, missing values, unknown endpoints and extra
columns. Empty samples and an explicit observation grid are required. Data are
never sorted, interpolated or imputed, and units are never inferred.

`dataset.plan(model)` and `CompiledSensitivities.plan_fit(request)` check
parameters, bounds, units, observations and limits without simulating.
`CompiledSensitivities.objective(objective_request)` evaluates the likelihood
and its gradient without optimizing. `dataset.fit(model)` returns the full fit
result, the plan and the row mapping. Check `fit.status` for convergence.

## Archive contract

`export_omex(model, scenarios, fits={name: native_request})` selects
`qspquest.omex.individual-fit/v1` when at least one fit is supplied. The archive
contains the model as SBML and the exact fit requests as JSON, including
parameters, starting values, bounds, units, error models, optimizer limits and
doses. Import and export both validate the requests. Select a fit with
`archive.plan_fit(name)` or `archive.fit(name)`.

Archived fits support uncensored individual fitting with fixed additive normal
SDs per endpoint, no priors, and either sensitivity solver. Population fits,
lognormal or proportional errors, censoring and priors are not supported in the
archive; the regular fitting API supports more options. Archives without `fits`
use the original run-only `qspquest.omex/v1` format.

This is a QSPQuest extension to OMEX. It does not run arbitrary SED-ML or import
projects from other estimation software.

## Expected output and interpretation

The final `report.json` records three BDF fits from different starting points
and five failure controls. `passed: true` requires native
convergence, recovery within 1e-5 for `cl` and 1e-6 for `drug_effect`, complete
observations, agreement with the independent reference, and the expected
errors from the failure controls. A failed check exits with status 1 and keeps
the failed cases. Do not remove failures or loosen limits to get a pass.

Each case retains its full native result, prediction grid and diagnostic rows.
Diagnostics include source row, result row, endpoint, time/side, observed and
predicted values, units, residuals, standardized residuals and endpoint
negative-log-likelihood sums. These are ordinary individual-fit residuals,
not CWRES or NPDE. Negative likelihood contributions can occur with small SD;
the normalized density can exceed one.

Treat `evaluation_limit`, `iteration_limit` or `line_search_failed` as failed
convergence. Check starting points, bounds, data and residual assumptions before
raising limits. If the compiled model changes, run `prepare` again. Wrong units or
rows are rejected with an error.

Recovering known parameters from synthetic data does not show they can be
identified from real data or support a dosing recommendation. When comparing
regimens, state whether follow-up is measured from day 0 or from the last dose.

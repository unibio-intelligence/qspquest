# QSPQuest R binding

This workspace package uses extendr-api 0.9.0 (MIT). Compile a model once with
`compile_model(source)`, then call `simulate_model`, `fit_model`, `scan_model`
or `morris_model` with the corresponding versioned JSON string or R list.
The compiled pointer is local to an R session; recreate it in another process.
Fitting currently compiles the requested sensitivity selection per call.
Inspect `fit$fit$status`; returning a result does not imply convergence.

`result_data_frame(simulation_result)` produces long data with TIME, SIDE,
OUTPUT, VALUE and UNIT columns. It retains missing values as NA, attaches the
time unit and execution identity, and avoids collisions with model output names.
Errors use class `qspquest_error` with code, message and available scientific
location fields. JSON models permit four million bytes; model text and requests
retain a one-million-byte limit.

## Installation

Install the standalone `qspquest_0.1.0.tar.gz` archive with R, its compatible C
compiler, Rust 1.98.1 and Cargo on `PATH`:

```r
install.packages("jsonlite", repos = "https://cloud.r-project.org")
install.packages("qspquest_0.1.0.tar.gz", repos = NULL, type = "source")
library(qspquest)
packageVersion("qspquest")
```

Expected version: `0.1.0`. The archive contains the Rust workspace and builds the
native engine locally. Cargo downloads locked registry dependencies on the first
installation. An offline installation requires those dependencies already cached;
set `CARGO_NET_OFFLINE=true` to require offline operation. The package links to
the installed R runtime and does not bundle R. `jsonlite` is required; `rxode2`
is optional for exporting an rxode2 UI.

See [installation](../../docs/installation.md#r) for terminal commands, platform
limits and preparing the archive, and the [R guide](../../docs/R-USER-GUIDE.md)
for a complete simulation and fitting workflow. The archive route is checked on
macOS ARM64 and in the Linux ARM64 R image; other installations need qualification. QSPQuest
is not yet published to CRAN or R-universe.

## Count and correlated binomial fitting

`fit_gamma_poisson(request)` estimates a shared Gamma shape/rate from independent
count/exposure rows. `fit_probit_normal_binomial(request)` estimates two group
means and litter-effect SDs from independent binomial litters. Supply a complete
versioned request as an R list or JSON string; a compiled ODE model is not needed.
Inspect the returned `status` and work counts. These are bounded point estimates.

The [count tutorial](../../docs/tutorials/gamma-poisson-counts.md) and
[correlated-binomial tutorial](../../docs/tutorials/probit-normal-binomial.md)
include complete synthetic inputs, exact R commands, units, limits and expected
results. Both functions are also documented by `help("fit_gamma_poisson")`.

## Explicit NONMEM-style simulation input

For structural model translation, see the [software adapter guide](../../docs/SOFTWARE-ADAPTERS.md).
`qspquest_export_rxode2_ui(ui, path)` exports an existing rxode2-compatible UI
for the Python structural importer; rxode2 is an optional external dependency.

`nonmem_requests(data, template, compartments, time_unit, amount_unit,
rate_unit = NULL, observation_side = "post")` builds one request per subject.
`simulate_data(model, dataset)` returns per-subject native results plus a long
combined data frame with `ID` and one-based `SOURCE_ROW`. Original data remain
available as `source`; IDs in results are strings. Duplicate observations and
source observation order are retained. Subjects run sequentially, sharing one
compiled model. Budgets apply separately to each subject.

```r
# Use a versioned run request with empty sampling and event lists.
template$regimen$samples <- list()
template$regimen$administrations <- list()
dataset <- nonmem_requests(data, template, c("1" = "central"),
                           time_unit = "h", amount_unit = "mg",
                           rate_unit = "mg/h", observation_side = "post")
predictions <- simulate_data(model, dataset)
```

Required columns are ID, TIME and EVID. Supported optional columns are DV, MDV,
AMT, CMT, RATE, ADDL, II and SS. EVID 0 with MDV 0 requests an observation;
EVID 1 doses use positive AMT and an explicit CMT-to-target mapping. RATE 0
means bolus; positive RATE means finite infusion. ADDL/II expand repeat doses.
Absent dose fields default to zero and absent MDV defaults to zero. DV is
preserved without fitting or missing-value inference; set MDV explicitly when
excluding a row. A subject must retain at least one observation.

The template controls start/end, parameters, static covariates, solver and
budgets; conversion does not extend its horizon. Its administrations, resets,
covariate changes, checkpoints and sampling lists must be empty. The native
engine validates units, time bounds and model targets when executed.
Same-time observations use the explicit global pre/post setting, never source
row order. Negative RATE, nonzero SS, EVID 2/3/4, dose fields on observation
rows, and additional columns are rejected. Covariates, censoring and endpoint
mapping need explicit future adapters; this is not a general NONMEM importer.
Per-subject execution identities remain on the individual result objects.

Record terminology follows the [NONMEM Users Guide VIII](https://nonmem.iconplc.com/nonmem744/guides/VIII.pdf).
The converter is independently authored from the data contract.

## Fit observations from a dataset

`fit_data_requests(dataset, template, endpoints, mode)` prepares inspectable
versioned fit requests and source-row mappings. Mode is `individual` (one fit per
subject) or `pooled` (shared selected parameters). `fit_data(model, prepared)`
executes them and retains the requests' subject mapping and original data.

The template is an individual `qspquest.fit/v0.1` envelope containing bounds,
optimizer limits, priors and a sensitivity selection. Clear its simulation `run`
and `observations`; the dataset supplies these. Each endpoint maps an observation
CMT to an output, measurement unit and fixed Gaussian error model:

```r
template$problem$request$objective$simulation$run <- NULL
template$problem$request$objective$observations <- list()
endpoints <- list("0" = list(output = "cp", value_unit = "mg/L",
  error = list(additive_sd = list(value = 0.1, unit = "mg/L"),
               proportional_sd = 0)))
prepared <- fit_data_requests(dataset, template, endpoints, mode = "pooled")
fit <- fit_data(model, prepared)
fit$results[[1]]$fit$status
fit_parameter_data_frame(fit)
observed <- fit_observation_data_frame(model, fit)
plot(observed$PRED, observed$DV)
```

Included DV values must be finite. Exclude missing measurements explicitly with
MDV; no silent dropping occurs. Duplicate observations remain separate likelihood
terms. `SOURCE_ROW` is one-based in the original data; `RESULT_ROW` is zero-based
in that subject's simulation, matching the native request schema. Units are
validated and converted by the engine. Pooled MAP counts the template prior once;
independent MAP fits each receive the prior. Check every fit status before using
estimates. This adapter supports fixed-error Gaussian individual/pooled fits;
random effects, censoring, residual-error estimation and covariate import remain
outside this contract.

`fit_parameter_data_frame` returns one row per fitted parameter with estimate,
unit, status and objective. `fit_observation_data_frame` is an explicit replay
at final estimates. It preserves `ID`, one-based `SOURCE_ROW` and zero-based
`RESULT_ROW`, observation side, selected output, observed value and prediction.
`DV_UNIT` and `PRED_UNIT` remain separate; `RESID` is `NA` if they differ.
Replay requires converged fits and costs one additional simulation per subject.

## Population FOCEI and SAEM from rows

`population_fit_data_requests()` takes the same explicit NONMEM record subset as
`nonmem_requests()` and constructs one FOCEI or SAEM population request. The
template has exactly one empty subject objective with a versioned sensitivity
selection and an empty run regimen; its fixed effects, random effects, Omega,
seed, and optimizer settings are retained. Dose CMT values map to model targets,
and observation CMT values map separately to outputs, measurement units, and
Gaussian errors. For example:

```r
template$problem$kind <- "focei" # or "saem"
prepared <- population_fit_data_requests(
  data, template, compartments = c("1" = "central"),
  endpoints = list("2" = list(output = "cp", value_unit = "mg/L",
    error = list(additive_sd = list(value = 0.05, unit = "mg/L"),
                 proportional_sd = 0))),
  time_unit = "h", amount_unit = "mg")
fitted <- fit_population_data(model, prepared)
fitted$result$population$status
population_fit_subject_data_frame(fitted)
observed <- population_fit_observation_data_frame(fitted)
```

`prepared$mapping` preserves source rows as one-based `SOURCE_ROW` and native
subject/observation/result indices as zero-based columns. Subject order follows
first appearance of each ID. FOCEI and SAEM fitted observations are checked against
the mapping and returned with DV, PRED, IPRED, units, and residuals. A residual
is `NA` when its units differ; no implicit unit conversion occurs in the R
table. The subject table includes eta, the fitted fixed effect, and the derived
individual parameter on its declared unit scale. Subject eta estimates and
conditional likelihoods remain available. Fitted rows can accompany a
nonconverged diagnostic result; inspect status and the method-specific objective
kind before interpreting estimates. The observation helper returns `NULL` only
when the result contains no fitted-observation rows. Unsupported NONMEM event
and covariate records are rejected explicitly.

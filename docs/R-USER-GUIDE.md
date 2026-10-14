# QSPQuest quickstart and user guide for R

Build a unit-aware model, simulate doses, compare parameters, and fit synthetic
observations using QSPQuest's R package. The native engine runs locally;
these examples require no Ubi account, Julia runtime, or simulation service.
All parameter values and data in this guide are synthetic.

This guide uses the 0.1.0 R API. Install the standalone source archive;
QSPQuest is not yet published to CRAN or R-universe.
Run the R blocks in order in one session. The complete
[example script](examples/r_quickstart.R) runs the same workflow and saves its
outputs. Keep this guide with the source version you use.

## Contents

1. [Install and verify](#install-and-verify)
2. [Quickstart](#quickstart)
3. [Read and plot results](#read-and-plot-results)
4. [Change parameters and dosing](#change-parameters-and-dosing)
5. [Choose sampling and solver settings](#choose-sampling-and-solver-settings)
6. [Scan parameters and run Morris screening](#scan-parameters-and-run-morris-screening)
7. [Fit an individual model](#fit-an-individual-model)
8. [Simulate and fit NONMEM style rows](#simulate-and-fit-nonmem-style-rows)
9. [Fit population data](#fit-population-data)
10. [Save a reproducible analysis](#save-a-reproducible-analysis)
11. [Handle errors and troubleshoot](#handle-errors-and-troubleshoot)
12. [API reference and further reading](#api-reference-and-further-reading)

## Install and verify

You need R, its compatible C compiler, Rust 1.98.1 and Cargo on `PATH`,
and the standalone R archive `qspquest_0.1.0.tar.gz`. The archive contains its
Rust workspace; no separate QSPQuest source checkout is required.

In R or RStudio, from the directory containing the archive:

```r
install.packages("jsonlite", repos = "https://cloud.r-project.org")
install.packages("qspquest_0.1.0.tar.gz", repos = NULL, type = "source")
```

Then verify the installation:

```r
library(qspquest)
library(jsonlite)
packageVersion("qspquest")
```

Expected version: `0.1.0`. Cargo downloads locked dependencies on first
installation, so network access is required unless they are already cached.
The native package links to your installed R runtime; use its compatible compiler
toolchain and restart R after replacing a loaded package.

[Installation](installation.md#r) also covers terminal commands, installing from
a complete source checkout, a custom user library, and building the R archive.
Source installation is checked on macOS ARM64 and in the Linux ARM64 R image;
other platforms need separate verification. `rxode2` is optional for the UI exporter and is not required by this
guide's native simulation and fitting workflow.

## Quickstart

The model stores central drug amount in mg and reports concentration `cp` in
mg/L. Clearance is 0.3 L/h and volume is 3 L, so the elimination rate is
0.1 per hour. After a 6 mg bolus, `cp(t) = 2 exp(-0.1 t)`.

### Compile the model

```r
library(qspquest)
library(jsonlite)

source <- "model quickstart_one_compartment
time_unit = h
parameters
  cl = 0.3 [L/h] log
  v = 3 [L] log
states
  central = 0 [mg]
dynamics
  d/dt(central) = input_central - cl / v * central
outputs
  cp = central / v [mg/L]
dosing
  central [mg] = 1
end
"
model <- compile_model(source)
q <- function(value, unit) list(value = value, unit = unit)
```

The dosing declaration enables delivery to `central`; the `input_central`
term makes an infusion contribute to the dynamics. A bolus applies an amount
jump. Model text, JSON model text, and appropriately structured R lists are
accepted. A file path is not source text: read it with
`paste(readLines("model.qsp", warn = FALSE), collapse = "\n")`.

### Define the experiment and run it

Explicit observations choose exact times and sides. All six observations here
are post-event readouts, including the concentration immediately after the
time-zero bolus.

```r
times <- c(0, 1, 2, 4, 8, 12)
request <- list(
  schema = "qspquest.run/v0.1",
  solver = "diffsol_bdf",
  budgets = list(solver_callbacks = 1000000L,
                 events = 100L, output_values = 10000L),
  regimen = list(
    end = q(12, "h"), samples = list(),
    observations = lapply(times, function(t) list(time = q(t, "h"), side = "post")),
    administrations = list(list(
      target = "central", time = q(0, "h"), amount = q(6, "mg"),
      delivery = list(kind = "bolus"), bioavailability = 1
    )),
    rtol = 1e-10, atol = 1e-12
  )
)
result <- simulate_model(model, request)
frame <- result_data_frame(result)
cp <- frame[frame$OUTPUT == "cp", ]
stopifnot(
  identical(cp$TIME, as.double(times)),
  all(cp$SIDE == "post"),
  max(abs(cp$VALUE - 2 * exp(-0.1 * cp$TIME))) < 1e-8
)
print(cp)
```

Expected concentrations are 2.000000 mg/L at 0 h, 1.340640 mg/L at 4 h,
and 0.602388 mg/L at 12 h. The analytic comparison checks scientific behavior
as well as successful execution.

Compile once and reuse `model`. Each simulation starts from its own initial
conditions and request. It does not continue the previous trajectory.
`model` holds a native pointer belonging to this R session; recreate it from
source in another session or process.

### Keep JSON arrays and objects distinct

The binding encodes lists with `jsonlite::toJSON(auto_unbox = TRUE)`.
Use unnamed lists for arrays of records and named lists for objects.
For example, `administrations = list(list(...))` must remain an array even
when it contains one dose. For a one-parameter sensitivity selection, use
`list("cl")`; a length-one character vector may be encoded as a scalar.
Keep tolerances as doubles and row/budget counts as whole numbers.

Read versioned JSON without simplifying nested records:

```r
# Optional file workflow after saving a request later in this guide:
# request <- fromJSON("run.request.json", simplifyVector = FALSE)
```

Unknown fields and malformed arrays are rejected. `NA` is not an ordinary
scientific quantity; handle missing measurements explicitly through MDV in
the dataset adapter.

## Read and plot results

`result_data_frame()` returns long data with `TIME`, `SIDE`, `OUTPUT`, `VALUE`,
and `UNIT`. It attaches `time_unit` and `execution_identity` attributes and
preserves missing numeric values as `NA`. Output names cannot overwrite the
table's metadata columns because each name appears in `OUTPUT`.

```r
plot(cp$TIME, cp$VALUE, type = "b",
     xlab = paste0("Time (", attr(frame, "time_unit"), ")"),
     ylab = "cp (mg/L)")
write.csv(frame, "trajectory.csv", row.names = FALSE)
```

Filter the desired output before plotting. Retain side labels when comparing
values at an event time. States are returned only when exposed as model
outputs. CSV does not preserve execution attributes, so retain the complete
JSON result as well.

For a tidyverse workflow, the existing data frame can be passed to dplyr or
ggplot2. Those packages are optional; they are not required for the native
engine or the examples in this guide.

## Change parameters and dosing

R lists use copy-on-modify semantics, so assigning `changed <- request` and
editing a nested field creates a changed experiment without altering the
original request. The native model pointer still refers to the same compiled
model.

```r
changed <- request
changed$parameters <- list(cl = q(0.6, "L/h"), v = q(3, "L"))
faster_clearance <- simulate_model(model, changed)

repeated <- request
repeated$regimen$administrations[[1]][["repeat"]] <- list(
  interval = q(4, "h"), additional = 2L
)
repeated_result <- simulate_model(model, repeated) # Doses at 0, 4, and 8 h.

infusion <- request
infusion$regimen$administrations[[1]]$delivery <- list(
  kind = "infusion",
  span = list(kind = "duration", duration = q(2, "h"))
)
infusion_result <- simulate_model(model, infusion) # 6 mg over 2 h.
```

`additional` counts administrations after the first. Rate-based infusion uses
`span = list(kind = "rate", rate = q(3, "mg/h"))` instead of duration.
Supply exactly one span type. Bioavailability is a fraction from 0 to 1;
the delivered amount also passes through the model's amount-to-state scale.
Request lag and model-declared lag add together.

Compatible units convert explicitly: an administration in ug can target a
dosing map in mg. Amount and concentration have different dimensions; use a
declared volume or dosing scale to connect them. A positive infusion rate needs
amount/time units. Consult [the unit specification](../spec/units.md) before
using a new unit spelling.

Declare covariates in the model and supply quantity values through
`regimen$covariates`; step covariates can also use `covariate_changes`.
Derived `individual` expressions are recomputed from their inputs and cannot
be directly overridden. Constant and step numeric covariates are supported;
categorical covariates and linear interpolation are outside the current scope.

### Build an oral two compartment model

To model first-order absorption with central/peripheral distribution, add a
depot, intercompartmental clearance, and peripheral volume. Expose amounts as
outputs so the amount balance can be inspected alongside concentration.

```r
oral_source <- "model quickstart_oral_two_compartment
time_unit = h
parameters
  cl = 1 [L/h] log
  vc = 5 [L] log
  vp = 10 [L] log
  exchange = 0.5 [L/h] log
  ka = 1 [1/h] log
states
  depot = 0 [mg]
  central = 0 [mg]
  peripheral = 0 [mg]
dynamics
  d/dt(depot) = -ka * depot
  d/dt(central) = ka * depot - cl / vc * central - exchange * (central / vc - peripheral / vp)
  d/dt(peripheral) = exchange * (central / vc - peripheral / vp)
outputs
  cp = central / vc [mg/L]
  depot_amount = depot [mg]
  central_amount = central [mg]
  peripheral_amount = peripheral [mg]
dosing
  depot [mg] bolus = 1
end
"
oral_model <- compile_model(oral_source)
oral_request <- request
oral_request$regimen$administrations[[1]]$target <- "depot"
oral_request$regimen$administrations[[1]]$amount <- q(100, "mg")
oral_request$regimen$administrations[[1]]$bioavailability <- 0.8
oral_result <- simulate_model(oral_model, oral_request)
oral_frame <- result_data_frame(oral_result)
stopifnot(
  oral_frame$VALUE[oral_frame$TIME == 0 & oral_frame$OUTPUT == "cp"] == 0,
  oral_frame$VALUE[oral_frame$TIME == 0 & oral_frame$OUTPUT == "depot_amount"] == 80,
  all(oral_frame$VALUE >= -1e-8)
)
```

The available 80 mg enters the depot and reaches central through first-order
absorption. The depot permits bolus delivery only; there is no infusion
forcing in its equation. Adding the three amount derivatives cancels internal
transfers and leaves central elimination. These are simple synthetic mechanism
assumptions, not drug-specific parameter estimates. For larger PBPK/QSP
systems, inspect conservation and domains and use state-specific tolerances.

## Choose sampling and solver settings

At a bolus time, `pre` selects the amount before the jump and `post` selects
it after the jump. Dataset row order never substitutes for this choice.

```r
at_dose <- request
at_dose$regimen$observations <- list(
  list(time = q(0, "h"), side = "pre"),
  list(time = q(0, "h"), side = "post")
)
event_result <- simulate_model(model, at_dose)
stopifnot(identical(unlist(event_result$outputs[[1]]$values), c(0, 2)))
```

An explicit observation plan preserves order and duplicates. For trajectory
sampling, omit `observations` and populate `samples`; event boundaries may
add rows. Always inspect returned times and sides. Do not combine nonempty
`samples` with an observation plan, or supply an empty observation plan.

This guide uses BDF. Simulation also supports `diffsol_tsit45`,
`diffsol_esdirk34`, `diffsol_tr_bdf2`, `diffsol_rosenbrock23`, and
`diffsol_rodas5p` within their supported ODE scope. No unknown method selects
a fallback. Sensitivity-based fitting currently requires BDF. An
`analytic_linear_pk` run needs a model that declares `linear_pk`; an arbitrary
linear-looking differential equation does not select that engine automatically.

`rtol` is dimensionless, while scalar `atol` uses each state's numeric unit.
For differing state scales, remove `atol` and supply a complete quantity map:
`absolute_tolerances = list(central = q(1e-12, "mg"))` for our single state.
Use exactly one absolute-tolerance mode. Check numerical stability by comparing
tighter tolerances and an independent or analytic reference where possible.

Budgets constrain solver callbacks, events, and output values. They are work
limits rather than deadlines. More work cannot correct invalid dimensions,
equation domains, or an uninformative fitting design.

## Scan parameters and run Morris screening

Scans evaluate up to 25 explicit parameter points, each with a unique ID.
The base run has per-run budgets, and the scan also needs aggregate budgets.

```r
scan_request <- list(
  schema = "qspquest.scan/v0.1", run = request,
  points = lapply(c(0.3, 0.6, 1.2), function(value) list(
    id = paste0("cl_", value), parameters = list(cl = q(value, "L/h"))
  )),
  total_budgets = list(solver_callbacks = 3000000L,
                       events = 300L, output_values = 30000L)
)
scan <- scan_model(model, scan_request)
scan_table <- do.call(rbind, lapply(scan$results, function(point) {
  data <- result_data_frame(point$result)
  data$POINT <- point$id
  data
}))
print(scan_table[scan_table$TIME == 12, ])
```

A scan compares deterministic scenarios; it does not provide population
variability or confidence intervals. Use biologically defensible ranges.

Morris screening calculates elementary effects over chosen parameter ranges
at one output row. Two parameters and two trajectories require six runs here.

```r
morris_request <- list(
  schema = "qspquest.morris/v0.1",
  problem = list(kind = "generated", request = list(
    run = request,
    ranges = list(
      list(parameter = "cl", lower = q(0.1, "L/h"), upper = q(0.7, "L/h")),
      list(parameter = "v", lower = q(3, "L"), upper = q(6, "L"))
    ),
    trajectory_count = 2L, num_levels = 4L, seed = 42L,
    total_budgets = list(solver_callbacks = 6000000L,
                         events = 600L, output_values = 60000L),
    output = "cp", row = 3L # Zero-based: the selected 4 h observation.
  ))
)
morris <- morris_model(model, morris_request)
print(morris$result$effects)
```

`mu_star` is average absolute effect magnitude; `sigma` describes variation
across elementary effects. Their interpretation depends on the output,
sampling row, ranges, design grid and seed. The 25-run scan bound also limits
Morris designs. The R API does not expose a standalone forward-sensitivity
runner; `fit_model()` compiles the sensitivity selection needed for a fit.

## Fit an individual model

This example estimates clearance and volume from exact synthetic observations
generated with `cl = 0.6 L/h` and `v = 4.5 L`. It uses a fixed additive
normal SD of 0.1 mg/L. A real analysis needs an error model justified for its
measurement process.

```r
sensitivity_request <- list(
  schema = "qspquest.sensitivity/v0.1", run = request,
  with_respect_to = list("cl", "v"),
  derivative_absolute_tolerances = list(
    cl = list(central = q(1e-12, "mg.h/L")),
    v = list(central = q(1e-12, "mg/L"))
  )
)
observations <- lapply(seq_along(times), function(index) list(
  row = index - 1L, output = "cp",
  value = q(6 / 4.5 * exp(-0.6 * times[index] / 4.5), "mg/L"),
  error = list(additive_sd = q(0.1, "mg/L"), proportional_sd = 0)
))
fit_request <- list(
  schema = "qspquest.fit/v0.1",
  problem = list(kind = "individual", request = list(
    objective = list(simulation = sensitivity_request,
                     observations = observations, priors = list()),
    bounds = list(
      list(parameter = "cl", lower = q(0.05, "L/h"), upper = q(1.2, "L/h")),
      list(parameter = "v", lower = q(1, "L"), upper = q(8, "L"))
    ),
    max_evaluations = 400L, max_iterations = 100L, gradient_tolerance = 1e-5
  ))
)
fitted <- fit_model(model, fit_request)
print(fitted$fit$status)
print(fitted$fit$parameters)
stopifnot(
  fitted$fit$status == "converged",
  abs(fitted$fit$parameters$cl$value - 0.6) < 1e-4,
  abs(fitted$fit$parameters$v$value - 4.5) < 1e-4
)
replay <- request
replay$parameters <- fitted$fit$parameters
fit_predictions <- simulate_model(model, replay)
```

Native `row` values start at zero even in R. Our explicit observation plan
makes `row = 0` the post-dose measurement at 0 h. Derivative absolute
tolerances have state-unit divided by parameter-unit, rather than the state's
unit alone. The selected parameter order must match the fitting contract.

The engine can return a result with `evaluation_limit`; inspect the status
before using estimates. Review objective, work count, boundaries and prediction
replay. A fit to exact noiseless synthetic data verifies implementation; it
does not establish confidence-interval calibration or clinical validity.
`priors = list()` requests likelihood fitting. Supported priors produce MAP
fits; consult [Gaussian objectives](GAUSSIAN-OBJECTIVE.md) for their native
representation. The R binding does not currently expose Python's derivative-free
scalar fitting method.

## Simulate and fit NONMEM style rows

The R helpers convert a restricted record format into explicit requests.
They are useful for tabular PK workflows, but do not import a NONMEM control
stream or infer endpoint, covariate, censoring or missing-value semantics.

To use an existing CSV instead, read and validate it before calling the adapters.
Preserve string IDs such as `001`, rather than converting them to numbers:

```r
# Replace the synthetic data below with your own validated file when needed.
# data <- read.csv("observations.csv", stringsAsFactors = FALSE,
#                  colClasses = c(ID = "character"))
# Check numeric TIME/AMT/DV, explicit MDV, CMT mappings and declared units.
```

Construct dose and observation rows with separate CMT values. This example
has two subjects, both following the same synthetic concentration profile.

```r
data <- do.call(rbind, lapply(c("A", "B"), function(id) {
  data.frame(
    ID = id, TIME = c(0, 0, 1, 2, 4), EVID = c(1L, 0L, 0L, 0L, 0L),
    CMT = c(1L, 2L, 2L, 2L, 2L), AMT = c(6, 0, 0, 0, 0),
    MDV = c(1L, 0L, 0L, 0L, 0L),
    DV = c(NA_real_, 6 / 4.5 * exp(-0.6 * c(0, 1, 2, 4) / 4.5))
  )
}))
rownames(data) <- NULL

run_template <- request
run_template$regimen$samples <- list()
run_template$regimen$observations <- NULL
run_template$regimen$administrations <- list()
dataset <- nonmem_requests(
  data, run_template, compartments = c("1" = "central"),
  time_unit = "h", amount_unit = "mg", observation_side = "post"
)
predicted_data <- simulate_data(model, dataset)
print(head(predicted_data$data))
```

The dataset supplies doses and selected observations. The template supplies
parameters, static covariates, horizon, solver, and budgets. Its schedules,
including resets, covariate changes and checkpoints, must be empty. Budgets
apply per subject and subjects execute sequentially with one compiled model.

Required columns are ID, TIME, and EVID. Optional supported columns are DV,
MDV, AMT, CMT, RATE, ADDL, II, and SS. `EVID = 0`, `MDV = 0` selects a
measurement; MDV 1 excludes it. `EVID = 1` is a dose with positive AMT and a
mapped CMT. RATE 0 is bolus; positive RATE requires explicit `rate_unit` and
describes a finite infusion. ADDL/II expand repeats. Nonzero SS, EVID 2/3/4,
negative RATE, unknown columns, and dose fields on observation rows fail.
The converter does not extend the template's time horizon.

### Prepare individual or pooled fits

Clear the individual fit template's run and observations so that the prepared
dataset can supply them. Map observation CMT 2 to output `cp` and its error
model. This endpoint mapping is separate from dose CMT 1 to `central`.

```r
fit_template <- fit_request
fit_template$problem$request$objective$simulation$run <- NULL
fit_template$problem$request$objective$observations <- list()
endpoints <- list("2" = list(
  output = "cp", value_unit = "mg/L",
  error = list(additive_sd = q(0.1, "mg/L"), proportional_sd = 0)
))
prepared <- fit_data_requests(dataset, fit_template, endpoints, mode = "pooled")
data_fit <- fit_data(model, prepared)
stopifnot(data_fit$results[[1]]$fit$status == "converged")
print(fit_parameter_data_frame(data_fit))
diagnostics <- fit_observation_data_frame(model, data_fit)
print(head(diagnostics))
```

Use `mode = "individual"` for a separate fit per subject and inspect every
status. Pooled fitting shares selected parameters across subjects and has no
random effects. A pooled MAP prior is counted once; separate individual MAP
fits each receive the prior.

`SOURCE_ROW` is one-based in the original R data, while `RESULT_ROW` is
zero-based in the native result. Duplicate observations remain separate
likelihood terms. Included DV values must be finite; set MDV explicitly to
exclude missing measurements. `fit_observation_data_frame()` replays converged
fits and incurs an additional simulation per subject. It preserves observation
sides and reports separate DV/PRED units. `RESID` is `NA` when those units
differ; the table does not silently convert them for subtraction.

## Fit population data

FOCEI and SAEM use one population request with per-subject regimens and a
lognormal random-effect contract. This four-subject example estimates typical
clearance, holding volume and eta variance fixed. Fixing `omega = 0.04`
means eta SD is 0.2 on the log scale; it is not a clearance-unit variance.

```r
etas <- c(-0.22, -0.08, 0.08, 0.22)
population_rows <- do.call(rbind, lapply(seq_along(etas), function(index) {
  t <- c(1, 2, 4)
  data.frame(
    ID = paste0("S", index), TIME = c(0, t), EVID = c(1L, 0L, 0L, 0L),
    CMT = c(1L, 2L, 2L, 2L), AMT = c(6, 0, 0, 0), MDV = c(1L, 0L, 0L, 0L),
    DV = c(NA_real_, 6 / 4.5 * exp(-0.6 * exp(etas[index]) * t / 4.5))
  )
}))
rownames(population_rows) <- NULL
pop_sensitivity <- sensitivity_request
pop_sensitivity$with_respect_to <- list("cl")
pop_sensitivity$derivative_absolute_tolerances$v <- NULL
pop_sensitivity$run <- run_template
pop_sensitivity$run$parameters <- list(cl = q(0.5, "L/h"), v = q(4.5, "L"))

population_template <- list(
  schema = "qspquest.fit/v0.1",
  problem = list(kind = "focei", request = list(
    subjects = list(list(simulation = pop_sensitivity,
                         observations = list(), priors = list())),
    fixed_effects = list(list(parameter = "cl", lower = q(0.35, "L/h"),
                             upper = q(0.85, "L/h"))),
    random_effects = list(list(parameter = "cl")),
    omega = list(list(0.04)), omega_diagonal_bounds = list(list(0.04, 0.04)),
    eta_bound = 1.5, max_evaluations = 30000L,
    max_iterations = 80L, gradient_tolerance = 0.005, seed = 17L
  ))
)
population_endpoints <- list("2" = list(
  output = "cp", value_unit = "mg/L",
  error = list(additive_sd = q(0.05, "mg/L"), proportional_sd = 0)
))
population_prepared <- population_fit_data_requests(
  population_rows, population_template, compartments = c("1" = "central"),
  endpoints = population_endpoints, time_unit = "h", amount_unit = "mg",
  observation_side = "post"
)
population_fit <- fit_population_data(model, population_prepared)
population <- population_fit$result$population
print(population$status)
print(population$objective_kind)
print(population$fixed_effects)
stopifnot(
  population$status == "converged",
  abs(population$fixed_effects$cl$value - 0.6) < 0.06
)
subject_table <- population_fit_subject_data_frame(population_fit)
population_diagnostics <- population_fit_observation_data_frame(population_fit)
print(subject_table)
print(head(population_diagnostics))
```

Inspect `population_prepared$request` before fitting. Its mapping preserves
one-based SOURCE_ROW and zero-based subject, observation and result indices.
Subject order follows first appearance in the input. FOCEI diagnostics report
DV, population prediction PRED, individual prediction IPRED, and source rows.
Subject tables include eta and the corresponding individual parameter.
Residuals are missing when measurement and prediction units differ.

Allowing Omega bounds to vary changes this example into variance estimation
and requires a sufficiently informative cohort. Two random effects, error
estimation and uncertainty have method-specific limits; do not infer support
for an arbitrary external model or fitting configuration.

### Run a short SAEM example

The same adapter can construct SAEM requests. Eight iterations intentionally
illustrate a limited result; they are not a recommended estimation budget.

```r
saem_template <- population_template
saem_template$problem$kind <- "saem"
saem_template$problem$request$max_iterations <- 8L
saem_prepared <- population_fit_data_requests(
  population_rows, saem_template, compartments = c("1" = "central"),
  endpoints = population_endpoints, time_unit = "h", amount_unit = "mg",
  observation_side = "post"
)
saem_fit <- fit_population_data(model, saem_prepared)
saem <- saem_fit$result$population
print(saem$status)
print(saem$objective_kind)
saem_subjects <- population_fit_subject_data_frame(saem_fit)
saem_observations <- population_fit_observation_data_frame(saem_fit)
```

A converged result reports `saem_marginal_objective_function_value`. A limited
run may return `saem_complete_data_surrogate`; do not compare it as if it
were a final marginal objective. `saem_observations` can be `NULL` when the
native result supplies no fitted-observation rows. Inspect the returned object
and method-specific status before requesting diagnostics or interpreting fits.

The native SAEM contract includes fixed normal/lognormal errors, left censoring,
specific fitted additive-error cases, bounded one-effect fitted covariates,
and limited local uncertainty. Advanced fields require explicit native
requests; the row adapter does not infer them. See [the SAEM guide](SAEM.md)
for exact combinations and limitations. Record seeds, examine stability across
seeds, and verify recovery on a cohort representative of your intended use.

## Save a reproducible analysis

Preserve source, requests, complete results, rows, mappings, and software
versions. Do not serialize a native pointer as the portable model artifact.

```r
dir.create("qspquest-analysis", showWarnings = FALSE)
writeLines(source, "qspquest-analysis/model.qsp")
save_json <- function(object, filename) {
  write_json(object, file.path("qspquest-analysis", filename),
             auto_unbox = TRUE, pretty = TRUE, digits = NA, null = "null")
}
save_json(request, "run.request.json")
save_json(result, "run.result.json")
save_json(fit_request, "fit.request.json")
save_json(fitted, "fit.result.json")
save_json(population_prepared$request, "population.request.json")
save_json(population_fit$result, "population.result.json")
write.csv(population_fit$mapping, "qspquest-analysis/population.mapping.csv", row.names = FALSE)
saveRDS(population_fit, "qspquest-analysis/population-fit.rds")
write.csv(population_rows, "qspquest-analysis/population.rows.csv", row.names = FALSE)
saveRDS(frame, "qspquest-analysis/trajectory.rds")
writeLines(capture.output(sessionInfo()), "qspquest-analysis/session-info.txt")
print(result$identity)
```

Save the native `population_fit$result` envelope as JSON; the classed R wrapper
and its source mapping are stored separately as RDS and CSV. RDS retains
data-frame attributes in an R workflow; JSON retains the native
execution identity for interchange. That identity records model content,
resolved bindings, the run request, compiler source and backend/build details.
It identifies the execution, not the model's scientific suitability.

To reload the simulation, read its request with `simplifyVector = FALSE`,
compile the saved source, and call `simulate_model()` again. Do not share the
compiled pointer across processes or rely on it surviving `saveRDS()`.
Compile inside each parallel R worker. The tabular simulation helper itself
executes subjects sequentially.

## Handle errors and troubleshoot

Scientific compiler/engine failures use class `qspquest_error` with a code,
message, and available scientific/source-location fields. R adapter validation
can instead raise an ordinary R error. Catch the classes appropriate to the
operation you call.

```r
invalid <- request
invalid$parameters <- list(cl = q(1, "h"))
caught <- tryCatch(
  simulate_model(model, invalid),
  qspquest_error = function(error) {
    print(error$code)
    message(conditionMessage(error))
    error
  }
)
stopifnot(inherits(caught, "qspquest_error"))
stopifnot(identical(simulate_model(model, request), result))
```

Failures produce no partial scientific result. A failed request does not
replace the compiled model or retain its bindings. Some errors have fallback
line/column information rather than exact expression spans.

| Symptom | Action |
| --- | --- |
| R cannot find the package | Check `.libPaths()` and the library passed to `R CMD INSTALL`. |
| Install fails to find Cargo or R headers | Check PATH, `R_HOME`, the complete workspace, and the compiler toolchain for this R distribution. |
| Offline compilation fails | Fetch the locked Cargo dependencies before installation. |
| Missing helper after reinstall | Restart R and check `find.package("qspquest")` for an older library copy. |
| A one-element field becomes a JSON scalar | Use an unnamed list for the native array, particularly `with_respect_to`. |
| Unit or unknown-symbol error | Compare model declarations, quantity dimensions and permitted input expressions. |
| Wrong dose-time value | Inspect SIDE and request pre/post explicitly. |
| Fit uses the wrong row | Convert from one-based R source rows to zero-based native result indices; inspect the adapter mapping. |
| Data preparation rejects DV | Use explicit MDV for excluded measurements; included values must be finite. |
| Dataset rejects columns or doses | Supply explicit CMT maps and RATE units; remove unsupported columns only after deciding how their information belongs in the model. |
| Fit hits limits or parameter bounds | Check identifiability, starting values, scientific bounds and error model before extending budgets. |
| Residuals are NA | Compare DV/PRED units and missingness; no implicit table-level unit conversion is performed. |
| SAEM observation table is NULL | Inspect status and native fitted-observation availability; a limited fit need not provide a table. |

## API reference and further reading

| Task | R API | Result |
| --- | --- | --- |
| Compile | `compile_model(source)` | Session-local native model pointer |
| Simulate | `simulate_model(model, request)` | Native result as an R list |
| Tabulate a simulation | `result_data_frame(result)` | Long data and identity attributes |
| Individual, pooled or population fit | `fit_model(model, request)` | Inspect `fit` or `population` status |
| Parameter scan | `scan_model(model, request)` | Per-point simulation results |
| Morris screening | `morris_model(model, request)` | Design and elementary effects |
| Prepare dose and measurement rows | `nonmem_requests(...)` | Per-subject requests and source data |
| Simulate prepared rows | `simulate_data(model, dataset)` | Subject results and combined data |
| Prepare individual or pooled fit | `fit_data_requests(...)` | Fit requests and source mapping |
| Execute prepared fits | `fit_data(model, prepared)` | Fit results and mappings |
| Tabulate estimates | `fit_parameter_data_frame(fitted)` | Estimates, units, statuses and objective |
| Replay fitted observations | `fit_observation_data_frame(model, fitted)` | Converged-fit observation diagnostics |
| Prepare population rows | `population_fit_data_requests(...)` | One population request and mapping |
| Fit population rows | `fit_population_data(model, prepared)` | Population result and mapping |
| Tabulate population subjects | `population_fit_subject_data_frame(fitted)` | Eta and individual parameter values |
| Tabulate population observations | `population_fit_observation_data_frame(fitted)` | Fitted-observation diagnostics or NULL |

Installed help is available through `?compile_model`, `?nonmem_requests`,
`?fit_data_requests`, and `?population_fit_data_requests`. Read the
[model language](../spec/language.md), [regimen contract](../spec/regimens.md),
[run and result identity](../spec/run-results.md),
[parameter scans](PARAMETER-SCANS.md), [Morris screening](MORRIS-SENSITIVITY.md),
[Gaussian objectives](GAUSSIAN-OBJECTIVE.md), and [JSON schemas](../spec/schema/README.md)
when adapting examples. For Python colleagues, use
[the Python user guide](PYTHON-USER-GUIDE.md).

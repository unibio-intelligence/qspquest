# Purpose: compile a unit-aware model, verify its trajectory, and fit count data.
# Prerequisite: install qspquest and its required jsonlite dependency.
# Run: Rscript --vanilla -e 'source(system.file("doc", "quickstart.R", package="qspquest"))'
library(qspquest)

source_text <- paste(
  "model synthetic_decay", "time_unit = h", "parameters",
  "  k = 0.1 [1/h] log", "states", "  amount = 6 [mg]",
  "dynamics", "  d/dt(amount) = -k * amount", "outputs",
  "  remaining = amount [mg]", "end", sep = "\n")
model <- compile_model(source_text)
quantity <- function(value, unit) list(value = value, unit = unit)
request <- list(
  schema = "qspquest.run/v0.1", solver = "diffsol_bdf",
  budgets = list(solver_callbacks = 100000L, events = 100L,
                 output_values = 1000L),
  regimen = list(end = quantity(24, "h"),
    samples = lapply(c(0, 6, 12, 24), quantity, unit = "h"),
    administrations = list(), rtol = 1e-9, atol = 1e-11))
trajectory <- result_data_frame(simulate_model(model, request))
stopifnot(max(abs(trajectory$VALUE - 6 * exp(-0.1 * trajectory$TIME))) < 1e-6)
print(trajectory)
# Remaining amount at 24 h is approximately 0.5443077 mg.

count_request <- list(
  schema = "qspquest.gamma-poisson-fit/v0.1",
  observations = list(list(count = 0L, exposure = quantity(1, "h")),
                      list(count = 1L, exposure = quantity(1, "h"))),
  initial_shape = 1, shape_bounds = c(0.01, 100),
  initial_rate = quantity(1, "h"),
  rate_bounds = list(quantity(0.01, "h"), quantity(100, "h")),
  max_evaluations = 1000L, max_iterations = 200L,
  gradient_tolerance = 1e-7)
fit <- fit_gamma_poisson(count_request)
stopifnot(fit$status == "converged")
print(fit$parameters)
# The rate estimate is on its upper bound (100 h). This is a bounded point
# estimate from only two observations; it is not a confidence interval.
# Use lists for JSON arrays, including single-element arrays. Supply units
# explicitly, inspect fit status, and recreate compiled models in each session.

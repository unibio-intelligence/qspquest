\name{fit_parameter_data_frame}
\alias{fit_parameter_data_frame}
\alias{fit_observation_data_frame}
\title{Turn Dataset Fits Into Plot-Ready Tables}
\description{Extract unit-bearing parameter estimates or replay converged fits at
their final estimates to obtain source-row-aligned observations and predictions.}
\usage{
fit_parameter_data_frame(fitted)
fit_observation_data_frame(model, fitted)
}
\arguments{
\item{fitted}{A result returned by \code{fit_data}.}
\item{model}{The compiled model used by \code{fit_data}.}
}
\value{\code{fit_parameter_data_frame} returns ID, PARAMETER, ESTIMATE, UNIT,
STATUS and OBJECTIVE columns. Pooled fits have a missing ID because estimates
are shared. \code{fit_observation_data_frame} returns ID, SOURCE_ROW, RESULT_ROW,
TIME, SIDE, OUTPUT, DV, DV_UNIT, PRED, PRED_UNIT and RESID. RESID is missing when
DV and PRED use different units; the package never subtracts incompatible
values. Row order matches the source observations within each subject.}
\details{Prediction replay runs one simulation per subject on demand, using the
fitted parameters and the original request. It requires every fit to have
\code{status == "converged"}. Original subject-specific execution results remain
available in the \code{fit_data} result. These helpers cover the individual and
pooled fixed-error Gaussian dataset adapter; they do not infer random effects,
covariates, censoring, or residual-error models.}
\seealso{\code{\link{fit_data_requests}}, \code{\link{nonmem_requests}}}
\examples{
q <- function(value, unit) list(value = value, unit = unit)
model <- compile_model(paste(
  "model one_compartment", "time_unit = h", "parameters",
  "  cl = 0.6 [L/h] log", "  v = 4.5 [L] log", "states", "  central = 0 [mg]",
  "dynamics", "  d/dt(central) = input_central - cl / v * central",
  "outputs", "  cp = central / v [mg/L]", "dosing", "  central [mg] = 1",
  "end", sep = "\n"))
run_template <- list(schema = "qspquest.run/v0.1", solver = "diffsol_bdf",
  budgets = list(solver_callbacks = 1000000L, events = 100L,
                 output_values = 10000L),
  regimen = list(end = q(4, "h"), samples = list(), administrations = list(),
                 rtol = 1e-10, atol = 1e-12))
rows <- do.call(rbind, lapply(c("A", "B"), function(id) data.frame(
  ID = id, TIME = c(0, 1, 2, 4), EVID = c(1L, 0L, 0L, 0L),
  CMT = c(1L, 2L, 2L, 2L), AMT = c(6, 0, 0, 0), MDV = c(1L, 0L, 0L, 0L),
  DV = c(NA, 6 / 4.5 * exp(-0.6 * c(1, 2, 4) / 4.5)))))
dataset <- nonmem_requests(rows, run_template, c("1" = "central"), "h", "mg")
template <- list(schema = "qspquest.fit/v0.1", problem = list(
  kind = "individual", request = list(
    objective = list(simulation = list(schema = "qspquest.sensitivity/v0.1",
        with_respect_to = list("cl"),
        derivative_absolute_tolerances = list(cl = list(central = q(1e-12, "mg.h/L")))),
      observations = list(), priors = list()),
    bounds = list(list(parameter = "cl", lower = q(0.05, "L/h"),
                       upper = q(1.2, "L/h"))),
    max_evaluations = 400L, max_iterations = 100L, gradient_tolerance = 1e-5)))
endpoints <- list("2" = list(output = "cp", value_unit = "mg/L",
  error = list(additive_sd = q(0.1, "mg/L"), proportional_sd = 0)))
prepared <- fit_data_requests(dataset, template, endpoints, mode = "pooled")
fitted <- fit_data(model, prepared)
stopifnot(fitted$results[[1]]$fit$status == "converged")
print(fit_parameter_data_frame(fitted))
print(fit_observation_data_frame(model, fitted))
}

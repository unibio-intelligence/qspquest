\name{individual_fit_data_requests}
\alias{individual_fit_data_requests}
\alias{fit_individual_data}
\title{Prepare and Fit Gaussian Observation Datasets}
\description{Map measurement records to explicit model outputs, units and fixed
Gaussian error models, then fit subjects independently or pool selected parameters.}
\usage{
individual_fit_data_requests(dataset, template, endpoints, mode = c("individual", "pooled"))
fit_individual_data(model, dataset)
}
\arguments{
\item{dataset}{For preparation, a \code{qspquest_dataset} from
\code{nonmem_requests}. For execution, a \code{qspquest_fit_dataset} from
\code{individual_fit_data_requests}.}
\item{template}{An individual \code{qspquest.fit/v0.1} request list or JSON
string supplying bounds, optimizer controls, priors and a sensitivity selection.
Its objective observations and simulation run must be empty; the dataset supplies them.}
\item{endpoints}{Named list keyed by observation CMT. Each entry contains exactly
\code{output}, \code{value_unit} and \code{error}. The error contains a unit-bearing
\code{additive_sd} and dimensionless \code{proportional_sd}.}
\item{mode}{\code{"individual"} fits each subject independently;
\code{"pooled"} shares selected parameters across subjects.}
\item{model}{A compiled model from \code{compile_model}.}
}
\value{Preparation returns inspectable \code{requests}, \code{mapping},
\code{subject_ids}, \code{mode} and original \code{source}.
Execution returns the same mapping and source plus \code{results} and the
inspectable versioned \code{requests} for optional prediction replay.
Inspect every result's \code{fit$status} before using estimates.}
\details{Included DV values must be finite; use MDV to exclude missing measurements.
Observation CMT requires an explicit endpoint mapping. Duplicate observations
remain independent likelihood terms. SOURCE_ROW is one-based in the original
data; RESULT_ROW is zero-based in that subject's native simulation. The engine
converts compatible measurement/error units. A pooled MAP prior is counted once;
independent fits each receive the template prior. Subjects' requests execute
sequentially. This supports fixed-error Gaussian fits, without population random
effects, censoring, residual-error estimation or inferred covariate mappings.}
\seealso{\code{\link{nonmem_requests}}, \code{\link{fit_model}},
\code{\link{fit_parameter_data_frame}}}
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
prepared <- individual_fit_data_requests(dataset, template, endpoints, mode = "pooled")
fitted <- fit_individual_data(model, prepared)
stopifnot(fitted$results[[1]]$fit$status == "converged")
fitted$results[[1]]$fit$parameters
head(prepared$mapping)
}

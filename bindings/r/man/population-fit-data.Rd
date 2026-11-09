\name{population_fit_data_requests}
\alias{population_fit_data_requests}
\alias{fit_population_data}
\alias{population_fit_subject_data_frame}
\alias{population_fit_observation_data_frame}
\title{Prepare and Fit Population Data from NONMEM-Style Rows}
\description{Build a FOCEI or SAEM request from an explicit subset of NONMEM
records, preserving subject and source-row provenance. Execute the request and
extract subject estimates or row-aligned predictions.}
\usage{
population_fit_data_requests(data, template, compartments, endpoints,
                             time_unit, amount_unit, rate_unit = NULL,
                             observation_side = "post")
fit_population_data(model, dataset)
population_fit_subject_data_frame(fitted)
population_fit_observation_data_frame(fitted)
}
\arguments{
\item{data}{A nonempty data frame with unique columns and required ID, TIME and
EVID. Supported optional columns are DV, MDV, AMT, CMT, RATE, ADDL, II and SS.}
\item{template}{A versioned \code{qspquest.fit/v0.1} FOCEI or SAEM request,
as a list or JSON string. It must contain exactly one subject objective with
empty observations and priors, a sensitivity selection and a run whose regimen
event lists are empty. The subject serves as the common simulation template.}
\item{compartments}{Named character vector mapping dose CMT values to model
targets.}
\item{endpoints}{Named list mapping observation CMT values to lists containing
exactly \code{output}, \code{value_unit} and \code{error}. Each error contains
an explicit additive_sd quantity and finite proportional_sd.}
\item{time_unit, amount_unit, rate_unit}{Explicit unit strings. A positive RATE
requires rate_unit.}
\item{observation_side}{Either \code{"pre"} or \code{"post"}. Source-row order
does not decide dose-side semantics.}
\item{model}{A model from \code{compile_model}.}
\item{dataset}{The prepared population dataset returned by
\code{population_fit_data_requests}.}
\item{fitted}{A result returned by \code{fit_population_data}.}
}
\value{\code{population_fit_data_requests} returns a prepared request,
subject IDs, the original data frame and a mapping. SOURCE_ROW is one-based in
R; SUBJECT_INDEX, OBSERVATION_INDEX and RESULT_ROW are zero-based native indices.
\code{fit_population_data} returns the native versioned result and that mapping,
after checking subject and fitted-observation alignment.
\code{population_fit_subject_data_frame} returns one row per subject and random
effect with ID, eta, fixed effect, derived individual estimate, parameter unit,
conditional negative log likelihood and fit status.
\code{population_fit_observation_data_frame} returns FOCEI fitted observations
with ID, source and native row indices, time, side, DV, PRED, IPRED, units and
residuals. It returns \code{NULL} when SAEM has no fitted observations. Residuals
are missing when the compared units differ.}
\details{Only EVID 0 observations and EVID 1 doses are supported; MDV 1 omits an
observation. Included DV values must be finite. Nonzero SS, negative RATE,
unmapped compartments/endpoints, extra columns, covariate records and censoring
are rejected. The native engine performs model, unit, time and fitting checks.
FOCEI and SAEM objectives differ and their numerical values are not directly
interchangeable.}
\seealso{\code{\link{nonmem_requests}}, \code{\link{individual_fit_data_requests}}}
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
etas <- c(-0.22, -0.08, 0.08, 0.22)
rows <- do.call(rbind, lapply(seq_along(etas), function(i) data.frame(
  ID = paste0("S", i), TIME = c(0, 1, 2, 4), EVID = c(1L, 0L, 0L, 0L),
  CMT = c(1L, 2L, 2L, 2L), AMT = c(6, 0, 0, 0), MDV = c(1L, 0L, 0L, 0L),
  DV = c(NA, 6 / 4.5 * exp(-0.6 * exp(etas[i]) * c(1, 2, 4) / 4.5)))))
simulation <- list(schema = "qspquest.sensitivity/v0.1", run = run_template,
  with_respect_to = list("cl"),
  derivative_absolute_tolerances = list(cl = list(central = q(1e-12, "mg.h/L"))))
simulation$run$parameters <- list(cl = q(0.5, "L/h"), v = q(4.5, "L"))
template <- list(schema = "qspquest.fit/v0.1", problem = list(
  kind = "focei", request = list(
    subjects = list(list(simulation = simulation, observations = list(),
                         priors = list())),
    fixed_effects = list(list(parameter = "cl", lower = q(0.35, "L/h"),
                              upper = q(0.85, "L/h"))),
    random_effects = list(list(parameter = "cl")),
    omega = list(list(0.04)), omega_diagonal_bounds = list(list(0.04, 0.04)),
    eta_bound = 1.5, max_evaluations = 30000L, max_iterations = 80L,
    gradient_tolerance = 0.005, seed = 17L)))
endpoints <- list("2" = list(output = "cp", value_unit = "mg/L",
  error = list(additive_sd = q(0.05, "mg/L"), proportional_sd = 0)))
prepared <- population_fit_data_requests(rows, template, c("1" = "central"),
  endpoints, time_unit = "h", amount_unit = "mg")
fitted <- fit_population_data(model, prepared)
stopifnot(fitted$result$population$status == "converged")
print(population_fit_subject_data_frame(fitted))
print(head(population_fit_observation_data_frame(fitted)))
}

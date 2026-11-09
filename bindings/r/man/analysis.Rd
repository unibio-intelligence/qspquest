\name{run_analysis}
\alias{run_analysis}
\alias{fit_data}
\alias{fit_population}
\alias{fit_diagnostics}
\alias{visual_predictive_check}
\alias{profile_likelihood}
\alias{bootstrap_population}
\alias{exposure_summary}
\alias{compare_regimens}
\alias{event_sensitivities}
\title{Native Analysis Workflows}
\description{Fit bounded parameters and residual standard deviations, fit
population models by stochastic approximation expectation-maximization (SAEM),
compute likelihood profiles, subject bootstraps, visual predictive checks,
sampled-trajectory exposure metrics, paired regimen comparisons and checked
event-time derivatives. Every simulation runs in the QSPQuest engine, and R and
Python call the same native implementation.}
\usage{
run_analysis(model, operation, arguments)
fit_data(model, run, observations, parameters, error,
  max_simulations = 2000L, max_iterations = 300L, tolerance = 1e-7,
  method = c("gradient", "derivative_free"))
fit_population(model, subjects, parameters, omega, error, covariates = list(),
  covariance = c("full", "diagonal"), variance_bounds = c(1e-4, 2),
  seed = 1009L, max_iterations = 600L, burn_in = 150L, sweeps = 3L,
  tolerance = 0.01, max_simulations = 200000L)
fit_diagnostics(fit)
visual_predictive_check(model, subjects, parameters, omega, error,
  random_effects = NULL, covariates = list(), replicates = 100L, bins = NULL,
  percentiles = c(0.05, 0.5, 0.95), confidence = 0.95,
  prediction_corrected = FALSE, stratify = character(), seed = 1009L,
  max_simulations = 50000L, workers = 1L)
profile_likelihood(model, run, observations, parameters, error, parameter, grid,
  confidence = 0.95, max_simulations = 20000L, max_iterations = 300L)
bootstrap_population(model, subjects, parameters, omega, error,
  replicates = 100L, confidence = 0.95, seed = 1009L,
  minimum_success_fraction = 0.8, max_simulations = 1000000L, ...)
exposure_summary(result, output, threshold = NULL)
compare_regimens(model, regimens, parameters, omega, output,
  random_effects = NULL, subjects = 100L, seed = 1009L, threshold = NULL,
  percentiles = c(0.05, 0.5, 0.95), parameter_draws = NULL,
  max_simulations = 50000L, workers = 1L)
event_sensitivities(model, run, parameters, relative_step = 1e-3,
  relative_tolerance = 0.02, absolute_tolerance = 1e-6, max_simulations = 1000L)
}
\arguments{
\item{model}{A compiled model returned by \code{compile_model}.}
\item{operation}{Native operation name, for example \code{"fit_data"}.}
\item{arguments}{A named list of native arguments for \code{run_analysis}.}
\item{run}{A \code{qspquest.run/v0.1} request as an R list.}
\item{observations}{A list of rows, each with \code{row}, \code{output},
\code{value} (a quantity) and optional \code{censoring = "left"}.}
\item{parameters}{A named list. For fitting, each item has \code{initial},
\code{lower}, \code{upper}, \code{unit} and optional \code{path}; for
simulation-based checks, each item is a quantity with \code{value} and
\code{unit}. List order defines the order of \code{omega}.}
\item{error}{Residual error: \code{model} (\code{"normal"} or
\code{"lognormal"}), \code{additive} and \code{proportional}. A fitted SD is a
list with \code{initial}, \code{lower} and \code{upper}.}
\item{subjects}{A list of subjects, each with \code{run}, \code{observations},
optional \code{id} and optional \code{covariates}. In \code{compare_regimens},
the number of virtual subjects.}
\item{omega}{Random-effect covariance matrix, in parameter order.}
\item{covariates}{Covariate effects. For fitting, each has \code{parameter},
\code{covariate}, \code{reference}, \code{initial}, \code{lower} and
\code{upper}; for predictive checks, \code{parameter}, \code{covariate},
\code{reference} and \code{estimate}.}
\item{covariance}{Full or diagonal random-effect covariance.}
\item{variance_bounds}{Lower and upper variance bounds, shared or per effect.}
\item{seed}{Integer seed. Each work unit draws from its own substream.}
\item{max_simulations}{Aggregate limit on native simulations, including failed
simulations.}
\item{max_iterations, burn_in, sweeps, tolerance}{Optimizer or SAEM controls.}
\item{method}{\code{"gradient"} (projected BFGS with central differences) or
\code{"derivative_free"} (compass pattern search).}
\item{fit}{A fit result from \code{fit_data} or \code{fit_population}.}
\item{random_effects}{Names of parameters with random effects; all by default.}
\item{replicates}{Number of simulated replicates.}
\item{bins}{Optional increasing time-bin edges.}
\item{percentiles}{Increasing percentiles in (0, 1).}
\item{confidence}{Interval confidence level in (0, 1).}
\item{prediction_corrected}{Whether to apply prediction correction.}
\item{stratify}{Up to two subject covariate names.}
\item{workers}{Native worker threads (1 to 8). Results do not depend on it.}
\item{parameter}{Name of the profiled parameter.}
\item{grid}{Increasing values of the profiled parameter.}
\item{minimum_success_fraction}{Fraction of converged replicates required for
intervals.}
\item{...}{Population fit options passed to each bootstrap fit.}
\item{result}{A simulation result returned by \code{simulate_model}.}
\item{output}{Output name.}
\item{threshold}{Optional concentration threshold.}
\item{regimens}{A named list of run requests; the first is the reference.}
\item{parameter_draws}{Optional list of parameter quantity sets, for example
from a bootstrap, to propagate parameter uncertainty.}
\item{relative_step, relative_tolerance, absolute_tolerance}{Central-difference
step and agreement tolerances.}
}
\value{A parsed result list. Inspect \code{status} before interpreting
estimates; a returned result does not imply convergence.}
\details{\code{fit_data} is the same analysis as the Python \code{fit_data};
\code{fit_individual_data} fits the native requests that
\code{individual_fit_data_requests} prepares from data frames. The SAEM objective is a complete-data surrogate,
not a marginal likelihood. Predictive-check intervals describe Monte Carlo
variation of replicate quantiles, not parameter uncertainty. Checkpoints and
progress callbacks are available from Python only.}
\examples{
source <- paste(
  "model synthetic_decay",
  "time_unit = h",
  "parameters",
  "  k = 0.1 [1/h] log",
  "states",
  "  amount = 6 [mg]",
  "dynamics",
  "  d/dt(amount) = -k * amount",
  "outputs",
  "  remaining = amount [mg]",
  "end", sep = "\n")
model <- compile_model(source)
quantity <- function(value, unit) list(value = value, unit = unit)
run <- list(
  schema = "qspquest.run/v0.1", solver = "diffsol_bdf",
  budgets = list(solver_callbacks = 100000L, events = 100L,
                 output_values = 1000L),
  regimen = list(end = quantity(24, "h"),
    samples = lapply(c(0, 2, 4, 8, 12, 24), quantity, unit = "h"),
    administrations = list(), rtol = 1e-9, atol = 1e-11))
summary <- exposure_summary(simulate_model(model, run), "remaining")
summary$auc
observations <- lapply(seq_along(c(0, 2, 4, 8, 12, 24)) - 1L, function(row)
  list(row = row, output = "remaining",
       value = quantity(6 * exp(-0.1 * c(0, 2, 4, 8, 12, 24)[row + 1L]), "mg")))
fit <- fit_data(model, run, observations,
  list(k = list(initial = 0.3, lower = 0.01, upper = 1, unit = "1/h")),
  list(additive = 0.05))
stopifnot(identical(fit$status, "converged"),
          abs(fit$parameters$k$value - 0.1) < 1e-4)
}

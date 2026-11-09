# Native analysis workflows. Each function sends one JSON request to the
# QSPQuest engine; R and Python share the same implementation and results.
.named <- function(x, what) {
  if (!is.list(x) || is.null(names(x)) || any(!nzchar(names(x))))
    stop(what, " must be a named list of specifications", call. = FALSE)
  unname(Map(function(name, item) c(list(name = name), item), names(x), x))
}
.array <- function(x) if (is.null(x)) NULL else I(x)
.matrix <- function(x) {
  x <- as.matrix(x)
  lapply(seq_len(nrow(x)), function(i) I(as.numeric(x[i, ])))
}
.drop_null <- function(x) x[!vapply(x, is.null, logical(1))]
# Native records serialize name-keyed objects in sorted key order. Rebuild every
# list, at any depth, whose names are exactly one declared set in that order.
.declared_order <- function(x, orders) {
  orders <- Filter(length, orders)
  visit <- function(v) {
    if (!is.list(v)) return(v)
    v[] <- lapply(v, visit)
    keys <- names(v)
    if (!is.null(keys)) {
      for (wanted in orders) {
        if (length(keys) == length(wanted) && setequal(keys, wanted) && !identical(keys, wanted)) {
          v <- v[wanted]
          break
        }
      }
    }
    v
  }
  if (length(orders)) visit(x) else x
}
.analysis <- function(model, operation, arguments, orders = list()) {
  arguments <- .encode(.drop_null(arguments))
  native <- if (is.null(model)) {
    .Call(wrap__qspquest_analysis_record, operation, arguments)
  } else {
    if (!inherits(model, "qspquest_model") || !inherits(model$pointer, "externalptr"))
      stop("model must be a compiled model returned by compile_model()", call. = FALSE)
    .Call(wrap__qspquest_analysis, model$pointer, operation, arguments)
  }
  .declared_order(jsonlite::fromJSON(.native_result(native), simplifyVector = FALSE), orders)
}
run_analysis <- function(model, operation, arguments) .analysis(model, operation, arguments)
fit_data <- function(model, run, observations, parameters, error,
                     max_simulations = 2000L, max_iterations = 300L,
                     tolerance = 1e-7, method = c("gradient", "derivative_free")) {
  .analysis(model, "fit_data", list(
    run = run, observations = observations, parameters = .named(parameters, "parameters"),
    error = error, max_simulations = max_simulations, max_iterations = max_iterations,
    tolerance = tolerance, method = match.arg(method)), list(names(parameters)))
}
fit_population <- function(model, subjects, parameters, omega, error, covariates = list(),
                           covariance = c("full", "diagonal"), variance_bounds = c(1e-4, 2),
                           seed = 1009L, max_iterations = 600L, burn_in = 150L, sweeps = 3L,
                           tolerance = 0.01, max_simulations = 200000L) {
  .analysis(model, "fit_population", list(
    subjects = subjects, parameters = .named(parameters, "parameters"),
    omega = .matrix(omega), error = error, covariates = I(covariates),
    covariance = match.arg(covariance), variance_bounds = .array(variance_bounds),
    seed = seed, max_iterations = max_iterations, burn_in = burn_in, sweeps = sweeps,
    tolerance = tolerance, max_simulations = max_simulations), list(names(parameters)))
}
fit_diagnostics <- function(fit) {
  .analysis(NULL, "fit_diagnostics", list(fit = fit),
            list(if (is.list(fit$parameters)) names(fit$parameters)))
}
visual_predictive_check <- function(model, subjects, parameters, omega, error,
                                    random_effects = NULL, covariates = list(),
                                    replicates = 100L, bins = NULL,
                                    percentiles = c(0.05, 0.5, 0.95), confidence = 0.95,
                                    prediction_corrected = FALSE, stratify = character(),
                                    seed = 1009L, max_simulations = 50000L, workers = 1L) {
  .analysis(model, "visual_predictive_check", list(
    subjects = subjects, parameters = .named(parameters, "parameters"),
    omega = .matrix(omega), error = error, random_effects = .array(random_effects),
    covariates = I(covariates), replicates = replicates, bins = .array(bins),
    percentiles = .array(percentiles), confidence = confidence,
    prediction_corrected = prediction_corrected, stratify = I(stratify), seed = seed,
    max_simulations = max_simulations, workers = workers), list(names(parameters)))
}
profile_likelihood <- function(model, run, observations, parameters, error, parameter, grid,
                               confidence = 0.95, max_simulations = 20000L,
                               max_iterations = 300L) {
  .analysis(model, "profile_likelihood", list(
    run = run, observations = observations, parameters = .named(parameters, "parameters"),
    error = error, parameter = parameter, grid = .array(grid), confidence = confidence,
    max_simulations = max_simulations, max_iterations = max_iterations),
    list(names(parameters)))
}
bootstrap_population <- function(model, subjects, parameters, omega, error,
                                 replicates = 100L, confidence = 0.95, seed = 1009L,
                                 minimum_success_fraction = 0.8,
                                 max_simulations = 1000000L, ...) {
  options <- list(...)
  if (!is.null(options$covariates)) options$covariates <- I(options$covariates)
  if (!is.null(options$variance_bounds)) options$variance_bounds <- I(options$variance_bounds)
  .analysis(model, "bootstrap_population", list(
    subjects = subjects, parameters = .named(parameters, "parameters"),
    omega = .matrix(omega), error = error, replicates = replicates,
    confidence = confidence, seed = seed,
    minimum_success_fraction = minimum_success_fraction,
    max_simulations = max_simulations,
    fit_options = if (length(options)) options else structure(list(), names = character())),
    list(names(parameters)))
}
exposure_summary <- function(result, output, threshold = NULL) {
  .analysis(NULL, "exposure_summary", list(result = result, output = output, threshold = threshold))
}
compare_regimens <- function(model, regimens, parameters, omega, output, random_effects = NULL,
                             subjects = 100L, seed = 1009L, threshold = NULL,
                             percentiles = c(0.05, 0.5, 0.95), parameter_draws = NULL,
                             max_simulations = 50000L, workers = 1L) {
  if (!is.list(regimens) || is.null(names(regimens)))
    stop("regimens must be a named list of run requests", call. = FALSE)
  .analysis(model, "compare_regimens", list(
    regimens = unname(Map(function(label, run) list(label = label, run = run),
                          names(regimens), regimens)),
    parameters = .named(parameters, "parameters"), omega = .matrix(omega), output = output,
    random_effects = .array(random_effects), subjects = subjects, seed = seed,
    threshold = threshold, percentiles = .array(percentiles),
    parameter_draws = parameter_draws, max_simulations = max_simulations,
    workers = workers), list(names(parameters), names(regimens)))
}
event_sensitivities <- function(model, run, parameters, relative_step = 1e-3,
                                relative_tolerance = 0.02, absolute_tolerance = 1e-6,
                                max_simulations = 1000L) {
  .analysis(model, "event_sensitivities", list(
    run = run, parameters = .named(parameters, "parameters"),
    relative_step = relative_step, relative_tolerance = relative_tolerance,
    absolute_tolerance = absolute_tolerance, max_simulations = max_simulations),
    list(names(parameters)))
}

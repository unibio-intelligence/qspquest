# Convert an explicit NONMEM record subset to a native population fit request.
# R source rows are one-based; QSPQuest subject and result rows are zero-based.
population_fit_data_requests <- function(data, template, compartments, endpoints,
                                         time_unit, amount_unit, rate_unit = NULL,
                                         observation_side = "post") {
  fail <- function(message) stop(message, call. = FALSE)
  text <- function(x) is.character(x) && length(x) == 1L && !is.na(x) && nzchar(x)
  if (is.character(template)) template <- jsonlite::fromJSON(template, simplifyVector = FALSE)
  if (!is.list(template) || !identical(template$schema, "qspquest.fit/v0.1") ||
      !is.list(template$problem) || !text(template$problem$kind) ||
      !template$problem$kind %in% c("focei", "saem"))
    fail("template must be a FOCEI or SAEM qspquest.fit/v0.1 request")
  body <- template$problem$request
  if (!is.list(body) || !is.list(body$subjects) || length(body$subjects) != 1L ||
      !is.list(body$subjects[[1L]])) fail("template must contain exactly one empty subject objective")
  objective <- body$subjects[[1L]]
  if (!is.list(objective$simulation) ||
      !identical(objective$simulation$schema, "qspquest.sensitivity/v0.1") ||
      !is.list(objective$simulation$run) ||
      !identical(objective$simulation$run$schema, "qspquest.run/v0.1"))
    fail("template requires a versioned sensitivity selection and simulation run")
  if (length(objective$observations) || length(objective$priors))
    fail("template subject observations and priors must be empty")
  if (!is.list(endpoints) || !length(endpoints) || is.null(names(endpoints)) ||
      anyNA(names(endpoints)) || any(!nzchar(names(endpoints))) || anyDuplicated(names(endpoints)))
    fail("endpoints must be a named list mapping observation CMT values")
  for (endpoint in endpoints) {
    if (!is.list(endpoint) || anyDuplicated(names(endpoint)) ||
        !setequal(names(endpoint), c("output", "value_unit", "error")))
      fail("each endpoint requires exactly output, value_unit and error")
    if (!text(endpoint$output) || !text(endpoint$value_unit))
      fail("endpoint output and value_unit must be nonempty strings")
    error <- endpoint$error
    if (!is.list(error) || anyDuplicated(names(error)) ||
        !setequal(names(error), c("additive_sd", "proportional_sd")) ||
        !is.list(error$additive_sd) || anyDuplicated(names(error$additive_sd)) ||
        !setequal(names(error$additive_sd), c("value", "unit")) ||
        !is.numeric(error$additive_sd$value) || length(error$additive_sd$value) != 1L ||
        !is.finite(error$additive_sd$value) || !text(error$additive_sd$unit) ||
        !is.numeric(error$proportional_sd) || length(error$proportional_sd) != 1L ||
        !is.finite(error$proportional_sd))
      fail("endpoint error requires finite additive_sd quantity and proportional_sd")
  }
  dataset <- nonmem_requests(data, objective$simulation$run, compartments,
                             time_unit, amount_unit, rate_unit, observation_side)
  source <- dataset$source
  if (is.null(source$DV) || !is.numeric(source$DV) || is.null(source$CMT))
    fail("fitting requires numeric DV and observation CMT")
  mapping <- list()
  subjects <- lapply(seq_along(dataset$subjects), function(index) {
    subject <- dataset$subjects[[index]]
    rows <- subject$observation_rows
    if (any(!is.finite(source$DV[rows])))
      fail("all included DV values must be finite; use MDV to omit missing observations")
    cmts <- as.character(source$CMT[rows])
    if (anyNA(cmts) || any(!cmts %in% names(endpoints)))
      fail("every included observation CMT needs an endpoint mapping")
    prepared <- objective
    prepared$simulation$run <- subject$request
    prepared$observations <- lapply(seq_along(rows), function(i) {
      endpoint <- endpoints[[cmts[i]]]
      list(row = i - 1L, output = endpoint$output,
           value = list(value = unname(source$DV[rows[i]]), unit = endpoint$value_unit),
           error = endpoint$error)
    })
    mapping[[index]] <<- data.frame(SUBJECT_INDEX = index - 1L, ID = subject$id,
      SOURCE_ROW = rows, OBSERVATION_INDEX = seq_along(rows) - 1L,
      RESULT_ROW = seq_along(rows) - 1L,
      OUTPUT = vapply(cmts, function(cmt) endpoints[[cmt]]$output, character(1)),
      stringsAsFactors = FALSE, row.names = NULL)
    prepared
  })
  request <- template
  request$problem$request$subjects <- subjects
  rows <- do.call(rbind, mapping); rownames(rows) <- NULL
  structure(list(request = request, subject_ids = vapply(dataset$subjects, `[[`, character(1), "id"),
                 mapping = rows, source = source), class = "qspquest_population_fit_dataset")
}

fit_population_data <- function(model, dataset) {
  if (!inherits(dataset, "qspquest_population_fit_dataset"))
    stop("dataset must come from population_fit_data_requests", call. = FALSE)
  result <- fit_model(model, dataset$request)
  population <- result$population
  if (!is.list(population) || !is.list(population$subjects) ||
      length(population$subjects) != length(dataset$subject_ids))
    stop("population fit result differs from subject mapping", call. = FALSE)
  fitted <- population$fitted_observations
  if (!is.null(fitted)) {
    mapping <- dataset$mapping
    if (length(fitted) != nrow(mapping))
      stop("fitted observation count differs from source mapping", call. = FALSE)
    seen <- rep(FALSE, nrow(mapping))
    for (row in fitted) {
      index <- which(mapping$SUBJECT_INDEX == row$subject_index &
                     mapping$OBSERVATION_INDEX == row$observation_index)
      if (length(index) != 1L || seen[index] ||
          !identical(as.integer(row$row), as.integer(mapping$RESULT_ROW[index])) ||
          !identical(row$output, mapping$OUTPUT[index]))
        stop("fitted observation row differs from source mapping", call. = FALSE)
      seen[index] <- TRUE
    }
    if (!all(seen)) stop("fitted observation mapping is incomplete", call. = FALSE)
  }
  structure(list(result = result, request = dataset$request,
                 subject_ids = dataset$subject_ids, mapping = dataset$mapping,
                 source = dataset$source), class = "qspquest_population_fit_result")
}

population_fit_subject_data_frame <- function(fitted) {
  if (!inherits(fitted, "qspquest_population_fit_result"))
    stop("fitted must come from fit_population_data", call. = FALSE)
  population <- fitted$result$population
  effects <- fitted$request$problem$request$random_effects
  parameters <- vapply(effects, `[[`, character(1), "parameter")
  fixed <- lapply(parameters, function(parameter) population$fixed_effects[[parameter]])
  if (any(vapply(fixed, is.null, logical(1))))
    stop("random effect has no fitted fixed effect", call. = FALSE)
  theta <- vapply(fixed, function(value) as.double(value$value), numeric(1))
  units <- vapply(fixed, function(value) as.character(value$unit), character(1))
  rows <- lapply(seq_along(population$subjects), function(index) {
    subject <- population$subjects[[index]]
    if (length(subject$eta) != length(parameters))
      stop("subject eta count differs from random effects", call. = FALSE)
    eta <- as.double(unlist(subject$eta))
    data.frame(SUBJECT_INDEX = index - 1L, ID = fitted$subject_ids[index],
      PARAMETER = parameters, ETA = eta, FIXED_EFFECT = theta,
      INDIVIDUAL_ESTIMATE = theta * exp(eta), UNIT = units,
      NEG_LOG_LIKELIHOOD = as.double(subject$negative_log_likelihood),
      STATUS = population$status, stringsAsFactors = FALSE)
  })
  frame <- do.call(rbind, rows); rownames(frame) <- NULL; frame
}

population_fit_observation_data_frame <- function(fitted) {
  if (!inherits(fitted, "qspquest_population_fit_result"))
    stop("fitted must come from fit_population_data", call. = FALSE)
  rows <- fitted$result$population$fitted_observations
  if (is.null(rows)) return(NULL)
  mapping <- fitted$mapping
  frame <- do.call(rbind, lapply(rows, function(row) {
    index <- which(mapping$SUBJECT_INDEX == row$subject_index &
                   mapping$OBSERVATION_INDEX == row$observation_index)
    if (length(index) != 1L || row$row != mapping$RESULT_ROW[index])
      stop("fitted observation row differs from source mapping", call. = FALSE)
    value <- as.double(row$value$value); pred <- as.double(row$pred$value)
    ipred <- as.double(row$ipred$value)
    unit <- row$value$unit; pred_unit <- row$pred$unit; ipred_unit <- row$ipred$unit
    data.frame(SUBJECT_INDEX = row$subject_index, ID = mapping$ID[index],
      SOURCE_ROW = mapping$SOURCE_ROW[index], OBSERVATION_INDEX = row$observation_index,
      RESULT_ROW = row$row, TIME = as.double(row$time$value), TIME_UNIT = row$time$unit,
      SIDE = row$side, OUTPUT = row$output, DV = value, DV_UNIT = unit,
      PRED = pred, PRED_UNIT = pred_unit, IPRED = ipred, IPRED_UNIT = ipred_unit,
      RESID = if (unit == pred_unit) value - pred else NA_real_,
      IRES = if (unit == ipred_unit) value - ipred else NA_real_,
      stringsAsFactors = FALSE)
  }))
  rownames(frame) <- NULL; frame
}

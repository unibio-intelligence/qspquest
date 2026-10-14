# DV records are explicitly mapped to model readouts through observation CMT.
fit_data_requests <- function(dataset, template, endpoints, mode = c("individual", "pooled")) {
  mode <- match.arg(mode)
  fail <- function(message) stop(message, call. = FALSE)
  if (!inherits(dataset, "qspquest_dataset") || !length(dataset$subjects))
    fail("dataset must come from nonmem_requests")
  if (is.character(template)) template <- jsonlite::fromJSON(template, simplifyVector = FALSE)
  if (!is.list(template) || !identical(template$schema, "qspquest.fit/v0.1") ||
      !identical(template$problem$kind, "individual")) fail("template must be an individual qspquest.fit/v0.1 request")
  base <- template$problem$request
  if (!is.list(base$objective$simulation) ||
      !identical(base$objective$simulation$schema, "qspquest.sensitivity/v0.1"))
    fail("template requires a versioned sensitivity selection")
  if (length(base$objective$observations) || length(base$objective$simulation$run))
    fail("template observations and simulation run must be empty; dataset supplies them")
  if (!is.list(endpoints) || !length(endpoints) || is.null(names(endpoints)) ||
      anyNA(names(endpoints)) || any(!nzchar(names(endpoints))) || anyDuplicated(names(endpoints)))
    fail("endpoints must be a named list mapping observation CMT to output, value_unit and error")
  for (endpoint in endpoints) {
    if (!is.list(endpoint) || !setequal(names(endpoint), c("output", "value_unit", "error")) ||
        anyDuplicated(names(endpoint))) fail("each endpoint requires exactly output, value_unit and error")
    for (field in c("output", "value_unit"))
      if (!is.character(endpoint[[field]]) || length(endpoint[[field]]) != 1L ||
          is.na(endpoint[[field]]) || !nzchar(endpoint[[field]])) fail(paste("invalid endpoint", field))
    if (!is.list(endpoint$error)) fail("endpoint error must be an explicit Gaussian error model")
  }
  source <- dataset$source
  if (!is.numeric(source$DV) || is.null(source$CMT)) fail("fitting requires numeric DV and observation CMT")
  mappings <- list()
  objectives <- lapply(seq_along(dataset$subjects), function(index) {
    subject <- dataset$subjects[[index]]; rows <- subject$observation_rows
    if (any(!is.finite(source$DV[rows]))) fail("all included DV values must be finite; use MDV to omit missing observations")
    cmts <- as.character(source$CMT[rows])
    if (anyNA(cmts) || any(!cmts %in% names(endpoints))) fail("every observation CMT needs an endpoint mapping")
    objective <- base$objective
    objective$simulation$run <- subject$request
    objective$observations <- lapply(seq_along(rows), function(i) {
      endpoint <- endpoints[[cmts[i]]]
      list(row = i - 1L, output = endpoint$output,
        value = list(value = source$DV[rows[i]], unit = endpoint$value_unit), error = endpoint$error)
    })
    mappings[[index]] <<- data.frame(ID = subject$id, SOURCE_ROW = rows,
      RESULT_ROW = seq_along(rows) - 1L,
      OUTPUT = vapply(cmts, function(cmt) endpoints[[cmt]]$output, character(1)),
      stringsAsFactors = FALSE, row.names = NULL)
    # Pooled MAP has one shared prior, counted on the primary objective only.
    if (mode == "pooled" && index > 1L) objective$priors <- list()
    objective
  })
  if (mode == "pooled") {
    fit <- base; fit$objective <- objectives[[1]]
    requests <- list(list(schema = template$schema, problem = list(kind = "pooled",
      request = list(fit = fit, additional_subjects = objectives[-1L]))))
  } else {
    requests <- lapply(objectives, function(objective) {
      fit <- base; fit$objective <- objective
      list(schema = template$schema, problem = list(kind = "individual", request = fit))
    })
  }
  structure(list(mode = mode, requests = requests, mapping = do.call(rbind, mappings),
    subject_ids = vapply(dataset$subjects, `[[`, character(1), "id"), source = source),
    class = "qspquest_fit_dataset")
}

fit_data <- function(model, dataset) {
  if (!inherits(dataset, "qspquest_fit_dataset"))
    stop("dataset must come from fit_data_requests", call. = FALSE)
  results <- lapply(dataset$requests, function(request) fit_model(model, request))
  list(mode = dataset$mode, results = results, mapping = dataset$mapping,
       subject_ids = dataset$subject_ids, source = dataset$source,
       requests = dataset$requests)
}

# Keep estimates and their units together; never interpret a nonconverged fit
# as a successful estimate merely because the engine returned a result object.
fit_parameter_data_frame <- function(fitted) {
  if (!is.list(fitted) || is.null(fitted$mode) || is.null(fitted$results) ||
      !fitted$mode %in% c("individual", "pooled"))
    stop("fitted must be a result from fit_data", call. = FALSE)
  if (fitted$mode == "individual" &&
      length(fitted$results) != length(fitted$subject_ids))
    stop("individual result count differs from subject count", call. = FALSE)
  if (fitted$mode == "pooled" && length(fitted$results) != 1L)
    stop("pooled result must contain one fit", call. = FALSE)
  rows <- lapply(seq_along(fitted$results), function(index) {
    fit <- fitted$results[[index]]$fit
    if (!is.list(fit) || is.null(fit$status) || !is.list(fit$parameters))
      stop("fit result has no Gaussian parameter estimates", call. = FALSE)
    parameters <- fit$parameters
    if (!length(parameters) || is.null(names(parameters)) ||
        anyNA(names(parameters)) || any(!nzchar(names(parameters))))
      stop("fit result has no named parameter estimates", call. = FALSE)
    data.frame(ID = if (fitted$mode == "individual") fitted$subject_ids[index] else NA_character_,
      PARAMETER = names(parameters),
      ESTIMATE = vapply(parameters, function(p) as.double(p$value), numeric(1)),
      UNIT = vapply(parameters, function(p) as.character(p$unit), character(1)),
      STATUS = fit$status,
      OBJECTIVE = as.double(fit$objective$objective),
      stringsAsFactors = FALSE)
  })
  frame <- do.call(rbind, rows)
  rownames(frame) <- NULL
  frame
}

# Replay final parameter estimates only when requested. This avoids adding
# simulator work to every fit, while preserving row-level source provenance.
fit_observation_data_frame <- function(model, fitted) {
  if (!inherits(model, "qspquest_model") || !is.list(fitted) ||
      is.null(fitted$mode) || !fitted$mode %in% c("individual", "pooled") ||
      is.null(fitted$requests) || is.null(fitted$mapping) ||
      is.null(fitted$subject_ids) || is.null(fitted$source))
    stop("model and fitted must come from compile_model and fit_data", call. = FALSE)
  if ((fitted$mode == "individual" && length(fitted$results) != length(fitted$subject_ids)) ||
      (fitted$mode == "pooled" && length(fitted$results) != 1L))
    stop("fit result count differs from fit mode", call. = FALSE)
  if (any(vapply(fitted$results, function(x) !identical(x$fit$status, "converged"), logical(1))))
    stop("prediction replay requires converged fits", call. = FALSE)
  objectives <- if (fitted$mode == "pooled") {
    pooled <- fitted$requests[[1L]]$problem$request
    c(list(pooled$fit$objective), pooled$additional_subjects)
  } else {
    lapply(fitted$requests, function(request) request$problem$request$objective)
  }
  if (length(objectives) != length(fitted$subject_ids))
    stop("fit request subject count differs from mapping", call. = FALSE)
  rows <- lapply(seq_along(objectives), function(index) {
    objective <- objectives[[index]]
    fit <- fitted$results[[if (fitted$mode == "pooled") 1L else index]]$fit
    run <- objective$simulation$run
    for (parameter in names(fit$parameters)) run$parameters[[parameter]] <- fit$parameters[[parameter]]
    simulation <- simulate_model(model, run)
    mapping <- fitted$mapping[fitted$mapping$ID == fitted$subject_ids[index], , drop = FALSE]
    if (nrow(mapping) != length(objective$observations))
      stop("fit observation count differs from source mapping", call. = FALSE)
    result_row <- mapping$RESULT_ROW + 1L
    if (any(result_row < 1L | result_row > length(simulation$times)))
      stop("fit observation row exceeds simulation result", call. = FALSE)
    output_names <- vapply(simulation$outputs, `[[`, character(1), "name")
    output_index <- match(mapping$OUTPUT, output_names)
    if (anyNA(output_index)) stop("fit output missing from simulation result", call. = FALSE)
    pred <- vapply(seq_len(nrow(mapping)), function(i) {
      value <- simulation$outputs[[output_index[i]]]$values[[result_row[i]]]
      if (is.null(value)) NA_real_ else as.double(value)
    }, numeric(1))
    pred_unit <- vapply(output_index, function(i) simulation$outputs[[i]]$unit, character(1))
    dv_unit <- vapply(objective$observations, function(o) o$value$unit, character(1))
    dv <- as.double(fitted$source$DV[mapping$SOURCE_ROW])
    residual <- ifelse(pred_unit == dv_unit, dv - pred, NA_real_)
    data.frame(ID = mapping$ID, SOURCE_ROW = mapping$SOURCE_ROW,
      RESULT_ROW = mapping$RESULT_ROW, TIME = as.double(unlist(simulation$times))[result_row],
      SIDE = as.character(unlist(simulation$sides))[result_row],
      OUTPUT = mapping$OUTPUT, DV = dv, DV_UNIT = dv_unit,
      PRED = pred, PRED_UNIT = pred_unit, RESID = residual,
      stringsAsFactors = FALSE)
  })
  frame <- do.call(rbind, rows)
  rownames(frame) <- NULL
  frame
}

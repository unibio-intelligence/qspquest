# Explicit subset of NONMEM record semantics, with no inferred units or covariates.
nonmem_requests <- function(data, template, compartments, time_unit, amount_unit,
                            rate_unit = NULL, observation_side = "post") {
  fail <- function(message) stop(message, call. = FALSE)
  scalar_text <- function(x) is.character(x) && length(x) == 1L && !is.na(x) && nzchar(x)
  if (!is.data.frame(data) || !nrow(data) || anyDuplicated(names(data)))
    fail("data must be a nonempty data frame with unique column names")
  required <- c("ID", "TIME", "EVID")
  if (!all(required %in% names(data))) fail("ID, TIME and EVID are required")
  allowed <- c(required, "DV", "MDV", "AMT", "CMT", "RATE", "ADDL", "II", "SS")
  if (length(setdiff(names(data), allowed)))
    fail("unsupported columns: explicitly remove or map covariate/censoring columns before conversion")
  if (!scalar_text(time_unit) || !scalar_text(amount_unit) ||
      (!is.null(rate_unit) && !scalar_text(rate_unit))) fail("units must be explicit nonempty strings")
  if (!scalar_text(observation_side) || !observation_side %in% c("pre", "post"))
    fail("observation_side must be pre or post; source-row order does not determine dose-side semantics")
  if (!is.character(compartments) || !length(compartments) || is.null(names(compartments)) ||
      anyNA(compartments) || anyNA(names(compartments)) || any(!nzchar(compartments)) ||
      any(!nzchar(names(compartments))) || anyDuplicated(names(compartments)))
    fail("compartments must be a named character vector mapping CMT values to model targets")
  if (is.character(template)) template <- jsonlite::fromJSON(template, simplifyVector = FALSE)
  if (!is.list(template) || !identical(template$schema, "qspquest.run/v0.1") ||
      !is.list(template$regimen)) fail("template must be a qspquest.run/v0.1 request")
  # Dataset owns dosing and sampling. Reject other event state rather than mixing schedules.
  for (field in c("administrations", "resets", "covariate_changes", "observations", "samples", "checkpoints"))
    if (length(template$regimen[[field]])) fail(paste("template regimen must have empty", field))
  number <- function(name, default = NULL) {
    x <- data[[name]]
    if (is.null(x)) return(rep(default, nrow(data)))
    if (!is.numeric(x) || anyNA(x) || any(!is.finite(x))) fail(paste(name, "must contain finite numbers"))
    x
  }
  if (!(is.character(data$ID) || is.numeric(data$ID) || is.factor(data$ID)) ||
      anyNA(data$ID) || any(!nzchar(as.character(data$ID)))) fail("ID must contain nonmissing subject identifiers")
  if (is.numeric(data$ID) && any(!is.finite(data$ID))) fail("ID must be finite")
  ids <- as.character(data$ID)
  times <- number("TIME"); evid <- number("EVID")
  if (any(!evid %in% c(0, 1))) fail("only EVID 0 observations and EVID 1 doses are supported")
  mdv <- number("MDV", 0)
  if (any(!mdv %in% c(0, 1))) fail("MDV must be 0 or 1")
  amt <- number("AMT", 0); rate <- number("RATE", 0)
  addl <- number("ADDL", 0); ii <- number("II", 0); ss <- number("SS", 0)
  if (any(ss != 0)) fail("SS records are not supported")
  if (any(rate < 0)) fail("negative/model-defined RATE is not supported")
  if (any(addl < 0 | addl != floor(addl) | addl > 4294967295)) fail("ADDL must be a nonnegative u32 integer")
  dose <- evid == 1
  if (any(amt[dose] <= 0) || any(ii[dose & addl > 0] <= 0)) fail("doses require positive AMT and ADDL requires positive II")
  if (any(amt[!dose] != 0 | rate[!dose] != 0 | addl[!dose] != 0 | ii[!dose] != 0))
    fail("observation rows cannot carry dose fields")
  if (any(ii[dose & addl == 0] != 0)) fail("II without ADDL is unsupported")
  if (any(rate > 0) && is.null(rate_unit)) fail("positive RATE requires an explicit rate_unit")
  if (any(dose) && (is.null(data$CMT) || anyNA(data$CMT[dose]) ||
      any(!as.character(data$CMT[dose]) %in% names(compartments)))) fail("every dose CMT needs an explicit target mapping")
  if (!is.null(data$DV) && !is.numeric(data$DV)) fail("DV must be numeric when supplied")
  q <- function(value, unit) list(value = unname(value), unit = unit)
  subjects <- lapply(unique(ids), function(id) {
    rows <- which(ids == id); observed <- rows[evid[rows] == 0 & mdv[rows] == 0]
    if (!length(observed)) fail(paste("subject", id, "has no requested observation rows"))
    request <- template
    request$regimen$samples <- list()
    request$regimen$observations <- lapply(observed, function(i)
      list(time = q(times[i], time_unit), side = observation_side))
    request$regimen$administrations <- lapply(rows[dose[rows]], function(i) {
      delivery <- if (rate[i] == 0) list(kind = "bolus") else
        list(kind = "infusion", span = list(kind = "rate", rate = q(rate[i], rate_unit)))
      administration <- list(target = unname(compartments[as.character(data$CMT[i])]),
        time = q(times[i], time_unit), amount = q(amt[i], amount_unit), delivery = delivery,
        bioavailability = 1)
      if (addl[i] > 0) administration[["repeat"]] <- list(interval = q(ii[i], time_unit), additional = addl[i])
      administration
    })
    list(id = id, request = request, source_rows = rows, observation_rows = observed)
  })
  structure(list(subjects = subjects, source = data, observation_side = observation_side),
            class = "qspquest_dataset")
}

simulate_data <- function(model, dataset) {
  if (!inherits(dataset, "qspquest_dataset")) stop("dataset must come from nonmem_requests", call. = FALSE)
  results <- lapply(dataset$subjects, function(subject) {
    result <- simulate_model(model, subject$request)
    frame <- result_data_frame(result)
    n <- length(subject$observation_rows)
    if (length(result$times) != n) stop("result observation count differs from source mapping")
    frame$ID <- subject$id
    frame$SOURCE_ROW <- rep(subject$observation_rows, times = length(result$outputs))
    list(id = subject$id, result = result, data = frame)
  })
  frame <- do.call(rbind, lapply(results, `[[`, "data")); rownames(frame) <- NULL
  # Per-subject execution identities are retained in results, not inherited from the first frame.
  attr(frame, "execution_identity") <- NULL
  list(subjects = results, data = frame, source = dataset$source)
}

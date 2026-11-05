.native_result <- function(value) {
  if (inherits(value, "extendr_error")) {
    diagnostic <- jsonlite::fromJSON(value$value, simplifyVector = FALSE)
    class(diagnostic) <- c("qspquest_error", "error", "condition")
    stop(diagnostic)
  }
  value
}
.encode <- function(value) {
  if (is.character(value) && length(value) == 1L && !is.na(value)) return(value)
  jsonlite::toJSON(value, auto_unbox = TRUE, null = "null", digits = 17)
}
compile_model <- function(source) {
  ptr <- .native_result(.Call(wrap__qspquest_compile, .encode(source)))
  structure(list(pointer = ptr), class = "qspquest_model")
}
fit_gamma_poisson <- function(request) {
  result <- .native_result(.Call(wrap__qspquest_fit_gamma_poisson, .encode(request)))
  jsonlite::fromJSON(result, simplifyVector = FALSE)
}
fit_probit_normal_binomial <- function(request) {
  result <- .native_result(.Call(wrap__qspquest_fit_probit_normal_binomial, .encode(request)))
  jsonlite::fromJSON(result, simplifyVector = FALSE)
}
.execute <- function(model, operation, request) {
  if (!inherits(model, "qspquest_model") || !inherits(model$pointer, "externalptr"))
    stop("model must be a compiled model returned by compile_model()", call. = FALSE)
  native <- tryCatch(
    .Call(wrap__qspquest_execute, model$pointer, operation, .encode(request)),
    error = function(e) {
      # Saved R objects and other processes receive a null native pointer.
      if (grepl("non-null pointer", conditionMessage(e), fixed = TRUE))
        stop("this compiled model is no longer available, for example after ",
             "saveRDS()/readRDS() or in another R session; recompile it with ",
             "compile_model()", call. = FALSE)
      stop(e)
    })
  result <- .native_result(native)
  jsonlite::fromJSON(result, simplifyVector = FALSE)
}
simulate_model <- function(model, request) .execute(model, "run", request)
fit_model <- function(model, request) .execute(model, "fit", request)
scan_model <- function(model, request) .execute(model, "scan", request)
morris_model <- function(model, request) .execute(model, "morris", request)
result_data_frame <- function(result) {
  stopifnot(identical(result$schema, "qspquest.result/v0.1"))
  times <- unlist(result$times)
  sides <- unlist(result$sides)
  rows <- lapply(result$outputs, function(column) {
    values <- vapply(column$values, function(x) if (is.null(x)) NA_real_ else as.double(x), numeric(1))
    data.frame(TIME = times, SIDE = sides, OUTPUT = column$name,
               VALUE = values, UNIT = column$unit, stringsAsFactors = FALSE)
  })
  data <- if (length(rows)) do.call(rbind, rows) else
    data.frame(TIME = numeric(), SIDE = character(), OUTPUT = character(),
               VALUE = numeric(), UNIT = character(), stringsAsFactors = FALSE)
  rownames(data) <- NULL
  attr(data, "time_unit") <- result$time_unit
  attr(data, "execution_identity") <- result$identity
  data
}

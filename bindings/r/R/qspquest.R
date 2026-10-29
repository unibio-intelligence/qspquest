.native_call <- function(symbol, ...) {
  value <- .Call(symbol, ...)
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
  ptr <- .native_call(wrap__qspquest_compile, .encode(source))
  structure(list(pointer = ptr), class = "qspquest_model")
}
fit_gamma_poisson <- function(request) {
  result <- .native_call(wrap__qspquest_fit_gamma_poisson, .encode(request))
  jsonlite::fromJSON(result, simplifyVector = FALSE)
}
fit_probit_normal_binomial <- function(request) {
  result <- .native_call(wrap__qspquest_fit_probit_normal_binomial, .encode(request))
  jsonlite::fromJSON(result, simplifyVector = FALSE)
}
.execute <- function(model, operation, request) {
  stopifnot(inherits(model, "qspquest_model"))
  result <- .native_call(wrap__qspquest_execute, model$pointer, operation, .encode(request))
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
  data <- do.call(rbind, rows)
  rownames(data) <- NULL
  attr(data, "time_unit") <- result$time_unit
  attr(data, "execution_identity") <- result$identity
  data
}

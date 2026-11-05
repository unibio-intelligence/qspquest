# Optional bridge for existing rxode2/nlmixr2/nonmem2rx/monolix2rx UI objects.
# It exports syntax and bindings, without running a QSPQuest or vendor fit.

qspquest_export_rxode2_ui <- function(ui, path) {
  if (!requireNamespace("rxode2", quietly = TRUE)) {
    stop("Install rxode2 to export an rxode2 UI", call. = FALSE)
  }
  model <- ui$model
  if (!is.call(model) || !identical(model[[1L]], as.name("model")) ||
      length(model) != 2L || !is.call(model[[2L]]) ||
      !identical(model[[2L]][[1L]], as.name("{"))) {
    stop("Expected an rxode2 UI with an explicit model({}) block")
  }
  theta <- ui$theta
  if (is.null(theta)) theta <- structure(numeric(), names = character())
  if (!is.numeric(theta) || any(!is.finite(theta)) ||
      is.null(names(theta)) || anyDuplicated(names(theta))) {
    stop("Expected finite, named scalar fixed-effect initial values")
  }
  package_names <- c("rxode2", "nlmixr2est", "nonmem2rx", "monolix2rx", "babelmixr2")
  # Record installed versions without loading packages this one does not declare.
  installed <- package_names[nzchar(vapply(package_names, function(p)
    system.file(package = p), character(1)))]
  versions <- structure(lapply(installed, function(p) as.character(utils::packageVersion(p))),
                        names = installed)
  versions$R <- R.version.string
  result <- list(schema = "qspquest.rxode2-ui/v1",
                 model_source = paste(deparse(model[[2L]], width.cutoff = 500L), collapse = "\n"),
                 parameters = as.list(theta), states = as.list(ui$state),
                 source_function = paste(deparse(as.function(ui), width.cutoff = 500L), collapse = "\n"),
                 versions = versions)
  jsonlite::write_json(result, path, auto_unbox = TRUE, pretty = TRUE, digits = NA)
  invisible(result)
}

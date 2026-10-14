# Discover and execute the shared library through its verified Python loader.
model_library <- function(root = Sys.getenv("QSPQUEST_LIBRARY_DIR"),
                          python = Sys.getenv("QSPQUEST_PYTHON", "python3")) {
  if (!nzchar(root) || !file.exists(file.path(root, "catalog.json"))) {
    stop("Set QSPQUEST_LIBRARY_DIR to a directory containing catalog.json, or pass root.")
  }
  root <- normalizePath(root, mustWork = TRUE)
  if (!nzchar(Sys.which(python))) stop("Python is unavailable; set QSPQUEST_PYTHON to its executable.")
  invoke <- function(args) {
    errors <- tempfile(); on.exit(unlink(errors))
    paths <- c(root, Sys.getenv("PYTHONPATH"))
    module_path <- paste(paths[nzchar(paths)], collapse = .Platform$path.sep)
    output <- system2(python,
      c("-m", "qspquest.library", "--library", shQuote(root), "--format", "json", args),
      env = paste0("PYTHONPATH=", shQuote(module_path)), stdout = TRUE, stderr = errors)
    status <- attr(output, "status")
    if (!is.null(status) && status != 0L) {
      stop(paste(c("Model-library execution failed:", readLines(errors, warn = FALSE)), collapse = "\n"))
    }
    jsonlite::fromJSON(paste(output, collapse = "\n"), simplifyVector = FALSE)
  }
  model_args <- function(model, version) {
    args <- c("--model", shQuote(model))
    if (!is.null(version)) {
      if (!is.character(version) || length(version) != 1L || is.na(version) || !nzchar(version))
        stop("version must be NULL or one nonempty exact model-version string")
      args <- c(args, "--model-version", shQuote(version))
    }
    args
  }
  list(
    models = function() invoke(character()),
    scenarios = function(model, version = NULL) invoke(model_args(model, version)),
    run = function(model, scenario, version = NULL) invoke(c(model_args(model, version), "--scenario", shQuote(scenario)))
  )
}

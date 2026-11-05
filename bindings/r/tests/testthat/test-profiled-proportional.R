example_file <- function(name) {
  installed <- system.file("extdata", name, package = "qspquest")
  if (nzchar(installed)) {
    return(installed)
  }
  # In-tree runs read the shared example from the repository.
  path <- file.path(test_path(), "..", "..", "..", "..", "docs", "examples", name)
  skip_if_not(file.exists(path), "profiled-proportional example files are not available")
  path
}

test_that("profiled proportional fit recovers the synthetic parameters", {
  model <- compile_model(paste(readLines(example_file("profiled-proportional.qsp")), collapse = "\n"))
  request <- jsonlite::fromJSON(example_file("profiled-proportional-fit.json"), simplifyVector = FALSE)

  result <- fit_model(model, request)
  expect_equal(result$fit$status, "converged")
  expect_lt(abs(result$fit$parameters$theta$value - 2.5), 1e-5)
  expect_lt(abs(result$residual_scales$y1 - .0894427191), 1e-7)
  expect_lt(abs(result$residual_scales$y2 - .1442220510), 1e-7)

  limited <- request
  limited$problem$request$max_evaluations <- 1L
  expect_equal(fit_model(model, limited)$fit$status, "evaluation_limit")

  invalid <- request
  invalid$problem$request$objective$simulation$run$budgets$solver_callbacks <- 1L
  invalid$problem$request$objective$observations[[1]]$error$proportional_sd <- .5
  error <- tryCatch(fit_model(model, invalid), qspquest_error = identity)
  expect_s3_class(error, "qspquest_error")
  expect_identical(error$code, "invalid_input")

  known <- request
  known$problem$kind <- "individual"
  expect_null(fit_model(model, known)$residual_scales)

  expect_equal(fit_model(model, request)$fit$status, "converged")
})

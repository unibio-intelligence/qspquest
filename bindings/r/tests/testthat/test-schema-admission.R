# Current native schema tags are required at every document boundary.
bad_tags <- function(tag) {
  list(sub("qspquest", "other", tag, fixed = TRUE), sub("v0.1", "v99", tag, fixed = TRUE),
       NULL, c(tag, "other"), list(tag), 1, NA_character_)
}
records <- data.frame(ID = "A", TIME = 1, EVID = 0, CMT = 2, DV = 1)
run <- list(schema = "qspquest.run/v0.1", regimen = list())
prepare <- function(template) nonmem_requests(records, template, c(`1` = "central"), "h", "mg")
endpoint <- list(`2` = list(output = "cp", value_unit = "mg/L",
  error = list(additive_sd = list(value = .05, unit = "mg/L"), proportional_sd = 0)))

test_that("NONMEM-style records require a current run schema", {
  original <- run
  prepared <- prepare(run)
  expect_identical(run, original)
  expect_identical(prepared$subjects[[1]]$request$schema, run$schema)
  for (tag in bad_tags(run$schema)) {
    invalid <- run
    invalid$schema <- tag
    expect_error(prepare(invalid))
  }
})

test_that("individual fit data require current fit and sensitivity schemas", {
  prepared <- prepare(run)
  fit <- list(schema = "qspquest.fit/v0.1", problem = list(kind = "individual", request = list(
    objective = list(simulation = list(schema = "qspquest.sensitivity/v0.1", run = list()),
                     observations = list()))))
  original <- fit
  fit_data_requests(prepared, fit, endpoint)
  expect_identical(fit, original)
  for (level in c("fit", "sensitivity")) {
    for (tag in bad_tags(paste0("qspquest.", level, "/v0.1"))) {
      invalid <- fit
      if (level == "fit") invalid$schema <- tag
      else invalid$problem$request$objective$simulation$schema <- tag
      expect_error(fit_data_requests(prepared, invalid, endpoint))
    }
  }
})

test_that("population fit data require current schemas at every level", {
  population <- list(schema = "qspquest.fit/v0.1", problem = list(kind = "focei", request = list(
    subjects = list(list(simulation = list(schema = "qspquest.sensitivity/v0.1", run = run),
                         observations = list(), priors = list())))))
  pop_prepare <- function(template) {
    population_fit_data_requests(records, template, c(`1` = "central"), endpoint, "h", "mg")
  }
  original <- population
  pop_prepare(population)
  expect_identical(population, original)
  for (level in c("fit", "sensitivity", "run")) {
    for (tag in bad_tags(paste0("qspquest.", level, "/v0.1"))) {
      invalid <- population
      if (level == "fit") invalid$schema <- tag
      if (level == "sensitivity") invalid$problem$request$subjects[[1]]$simulation$schema <- tag
      if (level == "run") invalid$problem$request$subjects[[1]]$simulation$run$schema <- tag
      expect_error(pop_prepare(invalid))
    }
  }
})

test_that("result data frames require a current result schema", {
  result <- list(schema = "qspquest.result/v0.1", times = list(0), sides = list("post"),
    time_unit = "h", outputs = list(list(name = "cp", unit = "mg/L", values = list(1))),
    identity = list(compiler = "test-build"))
  frame <- result_data_frame(result)
  expect_equal(frame$VALUE, 1)
  expect_identical(attr(frame, "execution_identity"), result$identity)
  for (tag in bad_tags(result$schema)) {
    invalid <- result
    invalid$schema <- tag
    expect_error(result_data_frame(invalid))
  }
  empty <- result
  empty$outputs <- list()
  empty_frame <- result_data_frame(empty)
  expect_s3_class(empty_frame, "data.frame")
  expect_equal(nrow(empty_frame), 0L)
  expect_identical(names(empty_frame), c("TIME", "SIDE", "OUTPUT", "VALUE", "UNIT"))
})

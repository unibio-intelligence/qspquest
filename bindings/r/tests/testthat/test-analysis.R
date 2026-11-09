test_that("native analyses match closed-form decay and are worker independent", {
  source_text <- paste("model synthetic_decay", "time_unit = h", "parameters", "  k = 0.1 [1/h] log",
    "states", "  amount = 6 [mg]", "dynamics", "  d/dt(amount) = -k * amount", "outputs",
    "  remaining = amount [mg]", "end", sep = "\n")
  model <- compile_model(source_text)
  q <- function(value, unit) list(value = value, unit = unit)
  times <- c(0, 2, 4, 8, 12, 24)
  run <- list(schema = "qspquest.run/v0.1", solver = "diffsol_bdf",
    budgets = list(solver_callbacks = 100000L, events = 100L, output_values = 1000L),
    regimen = list(end = q(24, "h"), samples = lapply(times, q, unit = "h"),
      administrations = list(), rtol = 1e-9, atol = 1e-11))
  summary <- exposure_summary(simulate_model(model, run), "remaining")
  expect_true(all(abs(summary$auc - sum(diff(times) * (head(6 * exp(-0.1 * times), -1) + tail(6 * exp(-0.1 * times), -1)) / 2)) < 1e-6))
  obs <- lapply(seq_along(times) - 1L, function(row) list(row = row, output = "remaining", value = q(6 * exp(-0.1 * times[row + 1L]), "mg")))
  fit <- fit_data(model, run, obs, list(k = list(initial = 0.3, lower = 0.01, upper = 1, unit = "1/h")), list(additive = 0.05))
  expect_true(all(identical(fit$status, "converged"), abs(fit$parameters$k$value - 0.1) < 1e-4))
  diag <- fit_diagnostics(fit)
  expect_true(all(diag$residual_summary$count == length(times)))
  prof <- profile_likelihood(model, run, obs, list(k = list(initial = 0.3, lower = 0.01, upper = 1, unit = "1/h")), list(additive = 0.05), "k", seq(0.09, 0.11, length.out = 5))
  expect_true(all(identical(prof$status, "complete"), prof$interval[[1]] < 0.1, prof$interval[[2]] > 0.1))
  subjects <- lapply(1:5, function(i) {
    k <- 0.1 * exp((i - 3) * 0.1)
    list(id = paste0("s", i), run = run, observations = lapply(seq_along(times) - 1L, function(row)
      list(row = row, output = "remaining", value = q(6 * exp(-k * times[row + 1L]), "mg"))))
  })
  vpc1 <- visual_predictive_check(model, subjects, list(k = q(0.1, "1/h")), matrix(0.04), list(additive = 0.05), replicates = 10L, seed = 3L)
  vpc4 <- visual_predictive_check(model, subjects, list(k = q(0.1, "1/h")), matrix(0.04), list(additive = 0.05), replicates = 10L, seed = 3L, workers = 4L)
  expect_true(all(identical(vpc1$bins, vpc4$bins), identical(vpc1$status, "complete")))
  pop <- fit_population(model, subjects, list(k = list(initial = 0.2, lower = 0.01, upper = 1, unit = "1/h")), matrix(0.04), list(additive = 0.05), max_iterations = 120L, burn_in = 40L, sweeps = 2L, tolerance = 0.05, seed = 2L)
  expect_true(all(abs(pop$parameters$k$value - 0.1) / 0.1 < 0.1))
  cmp <- compare_regimens(model, list(base = run), list(k = q(0.1, "1/h")), matrix(0.04), "remaining", subjects = 5L)
  expect_true(all(identical(cmp$reference, "base")))
  zeta_first <- compare_regimens(model, list(zeta = run, alpha = run), list(k = q(0.1, "1/h")), matrix(0.04), "remaining", subjects = 5L)
  expect_identical(names(zeta_first$regimens), c("zeta", "alpha"))
  nested <- qspquest:::.declared_order(list(a = list(vc = 1, cl = 2), trace = list(list(vc = 3, cl = 4))), list(c("cl", "vc")))
  expect_identical(names(nested$a), c("cl", "vc"))
  expect_identical(names(nested$trace[[1]]), c("cl", "vc"))
  expect_error(run_analysis(model, "nope", list(a = 1)), class = "qspquest_error")

})

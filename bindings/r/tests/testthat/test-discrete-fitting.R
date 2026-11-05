q <- function(value) list(value = value, unit = "h")

count_request <- list(schema = "qspquest.gamma-poisson-fit/v0.1",
  observations = list(list(count = 0L, exposure = q(1)), list(count = 1L, exposure = q(1))),
  initial_shape = 1, shape_bounds = c(.01, 100), initial_rate = q(1),
  rate_bounds = list(q(.01), q(100)),
  max_evaluations = 1000L, max_iterations = 200L, gradient_tolerance = 1e-7)

litters <- list()
for (group in 0:1) for (successes in 0:2) {
  litters[[length(litters) + 1L]] <- list(successes = successes, total = 2L, group = group)
}
litter_request <- list(schema = "qspquest.probit-normal-binomial-fit/v0.1", observations = litters,
  initial_means = c(0, 0), mean_bounds = list(c(-4, 4), c(-4, 4)),
  initial_sds = c(0, 0), sd_bounds = list(c(0, 3), c(0, 3)),
  max_evaluations = 1000L, max_iterations = 200L, gradient_tolerance = 1e-8,
  max_quadrature_evaluations = 2000000L)

test_that("Gamma-Poisson fit reaches the analytic optimum", {
  fit <- fit_gamma_poisson(count_request)
  expect_equal(fit$status, "converged")
  expect_identical(fit$parameters$rate$value, 100)
  expect_lt(abs(fit$parameters$shape$value - 1 / (2 * log1p(.01))), 1e-5)
  # A repeated fit is unaffected by earlier calls.
  expect_equal(fit_gamma_poisson(count_request)$status, "converged")
})

test_that("probit-normal binomial fit reaches the analytic optimum", {
  fit <- fit_probit_normal_binomial(litter_request)
  expect_equal(fit$status, "converged")
  expect_lt(max(abs(unlist(fit$means))), 1e-8)
  expect_lt(max(abs(unlist(fit$sds) - 1)), 1e-7)
  expect_lt(abs(fit$negative_log_likelihood - 6 * log(3)), 1e-10)
})

test_that("invalid counts and exhausted budgets are reported", {
  invalid <- count_request
  invalid$observations[[1]]$count <- .5
  expect_error(fit_gamma_poisson(invalid), class = "qspquest_error")

  limited <- litter_request
  limited$max_quadrature_evaluations <- 1L
  error <- tryCatch(fit_probit_normal_binomial(limited), qspquest_error = identity)
  expect_s3_class(error, "qspquest_error")
  expect_identical(error$code, "work_budget")

  limited <- litter_request
  limited$max_evaluations <- 1L
  expect_equal(fit_probit_normal_binomial(limited)$status, "evaluation_limit")
})

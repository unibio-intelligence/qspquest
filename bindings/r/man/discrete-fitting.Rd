\name{fit_gamma_poisson}
\alias{fit_gamma_poisson}
\alias{fit_probit_normal_binomial}
\title{Fit Count or Correlated Binomial Data}
\description{Bounded native point estimation for independent Gamma-Poisson
count/exposure rows or two groups of independent binomial litters with normally
distributed probit effects.}
\usage{
fit_gamma_poisson(request)
fit_probit_normal_binomial(request)
}
\arguments{
\item{request}{A complete request as an R list or a JSON string, using the
\code{qspquest.gamma-poisson-fit/v0.1} or
\code{qspquest.probit-normal-binomial-fit/v0.1} schema, respectively.}
}
\value{A list with the fit status, estimates, normalized negative log likelihood,
work counts, projected gradient norm and source/request identities.
Check \code{status} before interpreting estimates. Quadrature-budget exhaustion
raises a structured \code{qspquest_error}.}
\details{Gamma rates have exposure units in the shape-rate convention.
Probit means and SDs are dimensionless, with optimization in mean and variance
coordinates. The probit-normal mixture integrates over an unbounded normal
distribution and supports zero SD. These functions calculate point estimates
without confidence intervals.}
\examples{
q <- function(value) list(value = value, unit = "h")
request <- list(
  schema = "qspquest.gamma-poisson-fit/v0.1",
  observations = list(list(count = 0L, exposure = q(1)),
                      list(count = 1L, exposure = q(1))),
  initial_shape = 1, shape_bounds = c(0.01, 100),
  initial_rate = q(1), rate_bounds = list(q(0.01), q(100)),
  max_evaluations = 1000L, max_iterations = 200L,
  gradient_tolerance = 1e-7)
fit <- fit_gamma_poisson(request)
stopifnot(fit$status == "converged")
fit$parameters
}

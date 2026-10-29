library(qspquest)
q <- function(value) list(value=value,unit="h")
count_request <- list(schema="qspquest.gamma-poisson-fit/v0.1",
  observations=list(list(count=0L,exposure=q(1)),list(count=1L,exposure=q(1))),
  initial_shape=1,shape_bounds=c(.01,100),initial_rate=q(1),rate_bounds=list(q(.01),q(100)),
  max_evaluations=1000L,max_iterations=200L,gradient_tolerance=1e-7)
count_fit <- fit_gamma_poisson(count_request)
stopifnot(count_fit$status=="converged", count_fit$parameters$rate$value==100,
          abs(count_fit$parameters$shape$value-1/(2*log1p(.01)))<1e-5)
litters <- list()
for (group in 0:1) for (successes in 0:2) litters[[length(litters)+1L]] <- list(successes=successes,total=2L,group=group)
litter_request <- list(schema="qspquest.probit-normal-binomial-fit/v0.1",observations=litters,
  initial_means=c(0,0),mean_bounds=list(c(-4,4),c(-4,4)),initial_sds=c(0,0),sd_bounds=list(c(0,3),c(0,3)),
  max_evaluations=1000L,max_iterations=200L,gradient_tolerance=1e-8,max_quadrature_evaluations=2000000L)
litter_fit <- fit_probit_normal_binomial(litter_request)
stopifnot(litter_fit$status=="converged",max(abs(unlist(litter_fit$means)))<1e-8,
          max(abs(unlist(litter_fit$sds)-1))<1e-7,abs(litter_fit$negative_log_likelihood-6*log(3))<1e-10)
invalid <- count_request;invalid$observations[[1]]$count <- .5
stopifnot(tryCatch({fit_gamma_poisson(invalid);FALSE},qspquest_error=function(e) TRUE))
limited <- litter_request;limited$max_quadrature_evaluations <- 1L
stopifnot(tryCatch({fit_probit_normal_binomial(limited);FALSE},qspquest_error=function(e) identical(e$code,"work_budget")))
limited <- litter_request;limited$max_evaluations <- 1L
stopifnot(fit_probit_normal_binomial(limited)$status=="evaluation_limit")
stopifnot(fit_gamma_poisson(count_request)$status=="converged")
cat("discrete-fitting R checks passed\n")

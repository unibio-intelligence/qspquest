library(qspquest)
args <- commandArgs(trailingOnly=TRUE)
repo <- if (length(args)) args[[1]] else "."
model_source <- paste(readLines(file.path(repo,"docs/examples/profiled-proportional.qsp")),collapse="\n")
request <- jsonlite::fromJSON(file.path(repo,"docs/examples/profiled-proportional-fit.json"),simplifyVector=FALSE)
model <- compile_model(model_source)
result <- fit_model(model,request)
stopifnot(result$fit$status=="converged",abs(result$fit$parameters$theta$value-2.5)<1e-5,
          abs(result$residual_scales$y1-.0894427191)<1e-7,abs(result$residual_scales$y2-.1442220510)<1e-7)
limited <- request;limited$problem$request$max_evaluations <- 1L
stopifnot(fit_model(model,limited)$fit$status=="evaluation_limit")
invalid <- request
invalid$problem$request$objective$simulation$run$budgets$solver_callbacks <- 1L
invalid$problem$request$objective$observations[[1]]$error$proportional_sd <- .5
stopifnot(tryCatch({fit_model(model,invalid);FALSE},qspquest_error=function(e) identical(e$code,"invalid_input")))
known <- request;known$problem$kind <- "individual"
stopifnot(is.null(fit_model(model,known)$residual_scales))
stopifnot(fit_model(model,request)$fit$status=="converged")
cat("versioned proportional-error R checks passed\n")

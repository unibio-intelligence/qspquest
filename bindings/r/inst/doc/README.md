# First QSPQuest calculation in R

This tutorial simulates first-order decay with explicit units and fits synthetic
count data. It uses no network service or model-library download.

This source package supports Linux and macOS. Install R, its compatible C
compiler, Cargo and rustc 1.89 or newer. The source
archive includes the locked Rust dependencies; installation compiles them with
at most two jobs and requires no network access once `jsonlite` 2.0.0 or newer
is installed. Older versions can round numeric request values.

```r
install.packages("qspquest")
# Or, from a downloaded source archive:
# install.packages("jsonlite")
# install.packages("qspquest_0.1.0.tar.gz", repos = NULL, type = "source")
library(qspquest)
source(system.file("doc", "quickstart.R", package = "qspquest"))
```

The complete runnable example is installed as `doc/quickstart.R`. It prints four
rows at 0, 6, 12 and 24 hours, starting at 6 mg and ending at approximately
0.5443077 mg. The analytical trajectory check must pass. Its second calculation
returns converged Gamma-Poisson shape and rate estimates, with rate at its 100 h
upper bound. Bounds matter; these are point estimates, without confidence limits.

For a bounded individual fit with profiled proportional residual error:

```r
model <- compile_model(paste(readLines(system.file("extdata",
  "profiled-proportional.qsp", package = "qspquest")), collapse = "\n"))
request <- jsonlite::fromJSON(system.file("extdata",
  "profiled-proportional-fit.json", package = "qspquest"), simplifyVector = FALSE)
fit <- fit_model(model, request)
stopifnot(fit$fit$status == "converged")
fit$fit$parameters
fit$residual_scales
```

The fitted `theta` is approximately 2.5 mg/L. The two proportional residual
scales are approximately 0.08944272 and 0.14422205. See `help("compile_model")`,
`help("fit_data_requests")` and `help("population_fit_data_requests")` for input
contracts and limitations. Use R lists for arrays and explicit value/unit pairs
for quantities. Check every fit status before interpreting estimates; a returned
result can be a nonconverged diagnostic. Compiled model pointers belong to the R
session that created them and must be recreated after loading saved work.

If installation reports a missing or old Rust toolchain, install suitable Cargo
and rustc and retry; the package does not install tools itself. Native R workflows
do not require Python. Optional model-library and rxode2 bridges require their
separate documented software and are not used by this tutorial.

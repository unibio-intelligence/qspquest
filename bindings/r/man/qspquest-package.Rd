\name{qspquest-package}
\alias{qspquest-package}
\alias{qspquest}
\docType{package}
\title{Unit-Aware Pharmacology Simulation and Bounded Estimation}
\description{Compile pharmacology models with explicit units and run simulation,
parameter scans, sensitivity analysis and bounded estimation using a native Rust
engine. Native simulation and fitting do not require Python or network services.}
\details{A complete runnable tutorial is installed at
\code{system.file("doc", "quickstart.R", package = "qspquest")}.
Run it with \code{source()} after installation. A second example uses the
synthetic model and request in the installed \code{extdata} directory.
Always inspect fitting status and declared limitations before interpreting
results.}
\seealso{\code{\link{compile_model}}, \code{\link{fit_gamma_poisson}},
\code{\link{individual_fit_data_requests}}, \code{\link{population_fit_data_requests}}}

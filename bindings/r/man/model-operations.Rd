\name{compile_model}
\alias{compile_model}
\alias{simulate_model}
\alias{fit_model}
\alias{scan_model}
\alias{morris_model}
\title{Compile and Execute Pharmacology Models}
\description{Compile a model once and execute versioned simulation, bounded fitting,
parameter scan, or Morris sensitivity requests with the Rust engine.}
\usage{
compile_model(source)
simulate_model(model, request)
fit_model(model, request)
scan_model(model, request)
morris_model(model, request)
}
\arguments{
\item{source}{A model document as an R list or JSON string, or a model text string.}
\item{model}{The session-local compiled model returned by \code{compile_model}.}
\item{request}{A versioned request as an R list or JSON string. Use the run, fit,
scan, or Morris schema corresponding to the operation. Arrays should be R lists,
including arrays containing just one item.}
}
\value{\code{compile_model} returns a \code{qspquest_model} containing an external
pointer. The execution functions return parsed, versioned result lists with
execution identity and scientific values. Inspect \code{result$fit$status}
after fitting; a returned result does not imply convergence.}
\details{Compiled pointers cannot be transferred to another process or restored
from saved R data. Recompile the source in each process. Fitting compiles its
selected sensitivities for each call. JSON models have a four-million-byte
limit; model text and requests retain a one-million-byte limit.
Units, bounds, event semantics and computational budgets are validated by
the engine. Scientific errors are R conditions of class \code{qspquest_error},
with \code{code}, \code{message} and any available location fields.
Fitting supports bounded individual and pooled Gaussian MLE/MAP with fixed event
times and fixed residual-error parameters. Population random effects and censored
likelihoods are not supported by these operations.}
\examples{
source <- paste(
  "model synthetic_decay",
  "time_unit = h",
  "parameters",
  "  k = 0.1 [1/h] log",
  "states",
  "  amount = 6 [mg]",
  "dynamics",
  "  d/dt(amount) = -k * amount",
  "outputs",
  "  remaining = amount [mg]",
  "end", sep = "\n")
model <- compile_model(source)
quantity <- function(value, unit) list(value = value, unit = unit)
request <- list(
  schema = "qspquest.run/v0.1", solver = "diffsol_bdf",
  budgets = list(solver_callbacks = 100000L, events = 100L,
                 output_values = 1000L),
  regimen = list(end = quantity(24, "h"),
    samples = lapply(c(0, 6, 12, 24), quantity, unit = "h"),
    administrations = list(), rtol = 1e-9, atol = 1e-11))
result <- simulate_model(model, request)
trajectory <- result_data_frame(result)
stopifnot(max(abs(trajectory$VALUE - 6 * exp(-0.1 * trajectory$TIME))) < 1e-6)
trajectory
}
\seealso{\code{\link{result_data_frame}}, \code{\link{nonmem_requests}},
\code{\link{fit_data_requests}}}

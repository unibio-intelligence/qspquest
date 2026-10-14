\name{nonmem_requests}
\alias{nonmem_requests}
\alias{simulate_data}
\title{Prepare and Simulate Explicit NONMEM-Style Records}
\description{Convert a supported subset of observation and dosing records into
one simulation request per subject, preserving original source-row mappings.}
\usage{
nonmem_requests(data, template, compartments, time_unit, amount_unit,
                rate_unit = NULL, observation_side = "post")
simulate_data(model, dataset)
}
\arguments{
\item{data}{A nonempty data frame with ID, TIME and EVID. Optional columns are DV,
MDV, AMT, CMT, RATE, ADDL, II and SS. Additional columns are rejected.}
\item{template}{A versioned run request as a list or JSON string. Its regimen
administrations, resets, covariate changes, checkpoints, samples and observations
must be empty. It supplies start/end, parameters, static covariates, solver,
tolerances and per-subject budgets.}
\item{compartments}{Named character vector mapping dose CMT values to model targets.}
\item{time_unit, amount_unit}{Explicit unit strings for TIME/II and AMT.}
\item{rate_unit}{Explicit RATE unit, required for positive RATE.}
\item{observation_side}{Either \code{"pre"} or \code{"post"}, applied to every
requested observation. Same-time source-row order does not choose event sides.}
\item{model}{A compiled model from \code{compile_model}.}
\item{dataset}{A \code{qspquest_dataset} from \code{nonmem_requests}.}
}
\value{\code{nonmem_requests} returns a \code{qspquest_dataset} with per-subject
requests, source rows, observation rows and original data. \code{simulate_data}
returns \code{subjects} with native results and long data, combined \code{data},
and original \code{source}. Output includes character ID and one-based SOURCE_ROW.
Execution identities remain on each subject result.}
\details{EVID 0 with MDV 0 requests an observation; EVID 1 doses require positive
AMT and a mapped CMT. RATE 0 is bolus and positive RATE is finite infusion.
ADDL/II specify repeat doses. Missing dose fields and MDV default to zero.
DV is retained without inferring missingness: set MDV explicitly. Each subject
must have an observation. Duplicate and unsorted observations retain their order.
Negative RATE, nonzero SS, EVID 2/3/4 and dose fields on observations are rejected.
The engine checks unit compatibility, model targets and the template horizon;
conversion never extends the horizon. Subjects execute sequentially.
This is not a general NONMEM importer; covariates and censoring are not inferred.}
\examples{
request <- list(schema = "qspquest.run/v0.1", solver = "diffsol_bdf",
  budgets = list(solver_callbacks = 100000, events = 100, output_values = 1000),
  regimen = list(end = list(value = 4, unit = "h"), samples = list(),
    administrations = list(), rtol = 1e-8, atol = 1e-10))
data <- data.frame(ID = c("A", "A"), TIME = c(0, 1),
  EVID = c(1, 0), AMT = c(6, 0), CMT = c(1, 0))
prepared <- nonmem_requests(data, request, c("1" = "central"), "h", "mg")
prepared$subjects[[1]]$observation_rows
}
\seealso{\code{\link{fit_data_requests}}, \code{\link{result_data_frame}}}

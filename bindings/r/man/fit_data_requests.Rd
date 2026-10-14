\name{fit_data_requests}
\alias{fit_data_requests}
\alias{fit_data}
\title{Prepare and Fit Gaussian Observation Datasets}
\description{Map measurement records to explicit model outputs, units and fixed
Gaussian error models, then fit subjects independently or pool selected parameters.}
\usage{
fit_data_requests(dataset, template, endpoints, mode = c("individual", "pooled"))
fit_data(model, dataset)
}
\arguments{
\item{dataset}{For preparation, a \code{qspquest_dataset} from
\code{nonmem_requests}. For execution, a \code{qspquest_fit_dataset} from
\code{fit_data_requests}.}
\item{template}{An individual \code{qspquest.fit/v0.1} request list or JSON
string supplying bounds, optimizer controls, priors and a sensitivity selection.
Its objective observations and simulation run must be empty; the dataset supplies them.}
\item{endpoints}{Named list keyed by observation CMT. Each entry contains exactly
\code{output}, \code{value_unit} and \code{error}. The error contains a unit-bearing
\code{additive_sd} and dimensionless \code{proportional_sd}.}
\item{mode}{\code{"individual"} fits each subject independently;
\code{"pooled"} shares selected parameters across subjects.}
\item{model}{A compiled model from \code{compile_model}.}
}
\value{Preparation returns inspectable \code{requests}, \code{mapping},
\code{subject_ids}, \code{mode} and original \code{source}.
Execution returns the same mapping and source plus \code{results} and the
inspectable versioned \code{requests} for optional prediction replay.
Inspect every result's \code{fit$status} before using estimates.}
\details{Included DV values must be finite; use MDV to exclude missing measurements.
Observation CMT requires an explicit endpoint mapping. Duplicate observations
remain independent likelihood terms. SOURCE_ROW is one-based in the original
data; RESULT_ROW is zero-based in that subject's native simulation. The engine
converts compatible measurement/error units. A pooled MAP prior is counted once;
independent fits each receive the template prior. Subjects' requests execute
sequentially. This supports fixed-error Gaussian fits, without population random
effects, censoring, residual-error estimation or inferred covariate mappings.}
\seealso{\code{\link{nonmem_requests}}, \code{\link{fit_model}},
\code{\link{fit_parameter_data_frame}}}

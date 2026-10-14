\name{population_fit_data_requests}
\alias{population_fit_data_requests}
\alias{fit_population_data}
\alias{population_fit_subject_data_frame}
\alias{population_fit_observation_data_frame}
\title{Prepare and Fit Population Data from NONMEM-Style Rows}
\description{Build a FOCEI or SAEM request from an explicit subset of NONMEM
records, preserving subject and source-row provenance. Execute the request and
extract subject estimates or row-aligned predictions.}
\usage{
population_fit_data_requests(data, template, compartments, endpoints,
                             time_unit, amount_unit, rate_unit = NULL,
                             observation_side = "post")
fit_population_data(model, dataset)
population_fit_subject_data_frame(fitted)
population_fit_observation_data_frame(fitted)
}
\arguments{
\item{data}{A nonempty data frame with unique columns and required ID, TIME and
EVID. Supported optional columns are DV, MDV, AMT, CMT, RATE, ADDL, II and SS.}
\item{template}{A versioned \code{qspquest.fit/v0.1} FOCEI or SAEM request,
as a list or JSON string. It must contain exactly one subject objective with
empty observations and priors, a sensitivity selection and a run whose regimen
event lists are empty. The subject serves as the common simulation template.}
\item{compartments}{Named character vector mapping dose CMT values to model
targets.}
\item{endpoints}{Named list mapping observation CMT values to lists containing
exactly \code{output}, \code{value_unit} and \code{error}. Each error contains
an explicit additive_sd quantity and finite proportional_sd.}
\item{time_unit, amount_unit, rate_unit}{Explicit unit strings. A positive RATE
requires rate_unit.}
\item{observation_side}{Either \code{"pre"} or \code{"post"}. Source-row order
does not decide dose-side semantics.}
\item{model}{A model from \code{compile_model}.}
\item{dataset}{The prepared population dataset returned by
\code{population_fit_data_requests}.}
\item{fitted}{A result returned by \code{fit_population_data}.}
}
\value{\code{population_fit_data_requests} returns a prepared request,
subject IDs, the original data frame and a mapping. SOURCE_ROW is one-based in
R; SUBJECT_INDEX, OBSERVATION_INDEX and RESULT_ROW are zero-based native indices.
\code{fit_population_data} returns the native versioned result and that mapping,
after checking subject and fitted-observation alignment.
\code{population_fit_subject_data_frame} returns one row per subject and random
effect with ID, eta, fixed effect, derived individual estimate, parameter unit,
conditional negative log likelihood and fit status.
\code{population_fit_observation_data_frame} returns FOCEI fitted observations
with ID, source and native row indices, time, side, DV, PRED, IPRED, units and
residuals. It returns \code{NULL} when SAEM has no fitted observations. Residuals
are missing when the compared units differ.}
\details{Only EVID 0 observations and EVID 1 doses are supported; MDV 1 omits an
observation. Included DV values must be finite. Nonzero SS, negative RATE,
unmapped compartments/endpoints, extra columns, covariate records and censoring
are rejected. The native engine performs model, unit, time and fitting checks.
FOCEI and SAEM objectives differ and their numerical values are not directly
interchangeable.}
\seealso{\code{\link{nonmem_requests}}, \code{\link{fit_data_requests}}}

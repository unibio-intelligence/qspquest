\name{fit_parameter_data_frame}
\alias{fit_parameter_data_frame}
\alias{fit_observation_data_frame}
\title{Turn Dataset Fits Into Plot-Ready Tables}
\description{Extract unit-bearing parameter estimates or replay converged fits at
their final estimates to obtain source-row-aligned observations and predictions.}
\usage{
fit_parameter_data_frame(fitted)
fit_observation_data_frame(model, fitted)
}
\arguments{
\item{fitted}{A result returned by \code{fit_data}.}
\item{model}{The compiled model used by \code{fit_data}.}
}
\value{\code{fit_parameter_data_frame} returns ID, PARAMETER, ESTIMATE, UNIT,
STATUS and OBJECTIVE columns. Pooled fits have a missing ID because estimates
are shared. \code{fit_observation_data_frame} returns ID, SOURCE_ROW, RESULT_ROW,
TIME, SIDE, OUTPUT, DV, DV_UNIT, PRED, PRED_UNIT and RESID. RESID is missing when
DV and PRED use different units; the package never subtracts incompatible
values. Row order matches the source observations within each subject.}
\details{Prediction replay runs one simulation per subject on demand, using the
fitted parameters and the original request. It requires every fit to have
\code{status == "converged"}. Original subject-specific execution results remain
available in the \code{fit_data} result. These helpers cover the individual and
pooled fixed-error Gaussian dataset adapter; they do not infer random effects,
covariates, censoring, or residual-error models.}
\seealso{\code{\link{fit_data_requests}}, \code{\link{nonmem_requests}}}

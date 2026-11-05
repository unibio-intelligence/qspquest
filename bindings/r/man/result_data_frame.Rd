\name{result_data_frame}
\alias{result_data_frame}
\title{Convert a Simulation Result to Long Data}
\description{Produce long-format output suitable for plotting or joining with
observation records. Missing values remain present as \code{NA}.}
\usage{result_data_frame(result)}
\arguments{\item{result}{A parsed \code{qspquest.result/v0.1} simulation result.}}
\value{A data frame with \code{TIME}, \code{SIDE}, \code{OUTPUT}, \code{VALUE}
and \code{UNIT} columns. Attributes \code{time_unit} and \code{execution_identity}
retain simulation metadata. Rows are grouped by output, retaining the native
observation order within each output.}
\seealso{\code{\link{simulate_model}}, \code{\link{simulate_data}}}
\examples{
# A parsed simulation result, as returned by simulate_model().
result <- list(schema = "qspquest.result/v0.1", time_unit = "h",
  times = list(0, 1), sides = list("post", "post"),
  outputs = list(list(name = "cp", unit = "mg/L", values = list(2, 1.81))),
  identity = list())
result_data_frame(result)
}

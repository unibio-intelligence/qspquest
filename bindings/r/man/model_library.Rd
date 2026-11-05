\name{model_library}
\alias{model_library}
\title{Open the Shared QSPQuest Model Library}
\usage{
model_library(root = Sys.getenv("QSPQUEST_LIBRARY_DIR"),
              python = Sys.getenv("QSPQUEST_PYTHON", "python3"))
}
\arguments{
\item{root}{Directory containing \code{catalog.json}, \code{models/} and the \code{qspquest_models} loader.}
\item{python}{Python executable with QSPQuest installed, or access to the QSPQuest CLI.}
}
\description{Browse the catalog and execute named scenarios with the model library's verified loader. The library and its Python loader are distributed separately from this package; the QSPQuest Docker Compose deployment provides both.}
\value{A list of functions: \code{models()} returns catalog rows; \code{scenarios(model, version = NULL)} returns available scenario IDs; \code{run(model, scenario, version = NULL)} returns a parsed simulation result, including units, event sides and archive checksum.}
\details{Pass a nonempty exact version string to pin the catalog selection. The default \code{NULL} selects the current catalog version. An unavailable pin fails before simulation and is never replaced automatically. Availability does not establish numerical qualification. The optional shared-library workflow requires Python and the separately distributed model-library package. Native R simulation and fitting do not require Python. Read each model's card and qualification limits before interpreting results. Inspection-only models have no executable scenarios.}
\examples{
# Requires a separately installed QSPQuest model library and Python loader.
if (nzchar(Sys.getenv("QSPQUEST_LIBRARY_DIR"))) {
  library_api <- model_library()
  catalog <- library_api$models()
  length(catalog)
}
}

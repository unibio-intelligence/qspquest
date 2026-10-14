\name{model_library}
\alias{model_library}
\title{Open the shared QSPQuest model library}
\usage{
model_library(root = Sys.getenv("QSPQUEST_LIBRARY_DIR"),
              python = Sys.getenv("QSPQUEST_PYTHON", "python3"))
}
\arguments{
\item{root}{Directory containing \code{catalog.json}, \code{models/} and the \code{qspquest_models} loader.}
\item{python}{Python executable with QSPQuest installed, or access to the QSPQuest CLI.}
}
\description{Browse the catalog and execute named scenarios with the model library's verified loader. Docker Compose configures the shared mount and engine automatically.}
\value{A list of functions: \code{models()} returns catalog rows; \code{scenarios(model, version = NULL)} returns available scenario IDs; \code{run(model, scenario, version = NULL)} returns a parsed simulation result, including units, event sides and archive checksum.}
\details{Pass a nonempty exact version string to pin the catalog selection. The default \code{NULL} selects the current catalog version. An unavailable pin fails before simulation and is never replaced automatically. Availability does not establish numerical qualification. The optional shared-library workflow requires Python and the model-library package, supplied by the Docker mount. Native R simulation and fitting do not require Python. Read each model's card and qualification limits before interpreting results. Inspection-only models have no executable scenarios.}

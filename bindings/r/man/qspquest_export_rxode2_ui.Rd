\name{qspquest_export_rxode2_ui}
\alias{qspquest_export_rxode2_ui}
\title{Export an rxode2 UI for Structural QSPQuest Import}
\usage{qspquest_export_rxode2_ui(ui, path)}
\arguments{
  \item{ui}{An existing rxode2/nlmixr2-compatible UI object with an explicit model block.}
  \item{path}{Destination JSON file.}
}
\description{
Exports model syntax, numeric fixed-effect initial values, state order and runtime
versions. Install rxode2 separately. The snapshot is read by the \code{import_rxode2_snapshot}
adapter of the separately distributed QSPQuest Python package, which checks its
own supported structural subset; this helper does not run a fit or
qualify population-estimation equivalence.
}
\value{The snapshot invisibly, after writing its JSON representation.}
\examples{
if (requireNamespace("rxode2", quietly = TRUE)) {
  ui <- rxode2::rxode2(function() {
    ini({ ka <- 1.1; cl <- 0.3; v <- 3 })
    model({
      d/dt(depot) <- -ka * depot
      d/dt(central) <- ka * depot - cl / v * central
      cp <- central / v
    })
  })
  path <- tempfile(fileext = ".json")
  snapshot <- qspquest_export_rxode2_ui(ui, path)
  names(snapshot$parameters)
  unlink(path)
}
}

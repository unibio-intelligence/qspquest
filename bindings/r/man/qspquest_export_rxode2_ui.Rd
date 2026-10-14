\name{qspquest_export_rxode2_ui}
\alias{qspquest_export_rxode2_ui}
\title{Export an rxode2 UI for structural QSPQuest import}
\usage{qspquest_export_rxode2_ui(ui, path)}
\arguments{
  \item{ui}{An existing rxode2/nlmixr2-compatible UI object with an explicit model block.}
  \item{path}{Destination JSON file.}
}
\description{
Exports model syntax, numeric fixed-effect initial values, state order and runtime
versions. Install rxode2 separately. The Python import_rxode2_snapshot adapter
checks its own supported structural subset; this helper does not run a fit or
qualify population-estimation equivalence.
}
\value{The snapshot invisibly, after writing its JSON representation.}

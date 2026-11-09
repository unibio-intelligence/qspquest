\name{qspquest-figures}
\alias{qspquest-figures}
\alias{theme_qspquest}
\alias{scale_colour_qspquest}
\alias{scale_color_qspquest}
\alias{scale_fill_qspquest}
\alias{scale_fill_qspquest_continuous}
\alias{scale_colour_qspquest_continuous}
\alias{scale_fill_qspquest_diverging}
\alias{qspquest_style_definitions}
\alias{qspquest_palette}
\alias{qspquest_journals}
\alias{qspquest_plot_profile}
\alias{qspquest_plot_observed_predicted}
\alias{qspquest_plot_residuals}
\alias{qspquest_plot_fitted_profile}
\alias{qspquest_plot_parameters}
\alias{qspquest_plot_morris}
\alias{qspquest_plot_scan}
\alias{qspquest_save_figure}
\title{Publication Figures for QSPQuest Results}
\description{Build 'ggplot2' figures directly from QSPQuest results. Axis labels
and units are read from the result. Each function returns a \code{ggplot}
object that can be extended with \code{+}. \code{qspquest_save_figure} writes
PDF, SVG and PNG files at journal column widths. The 'ggplot2' package is
optional; these functions stop with an explanatory message when it is absent.}
\usage{
theme_qspquest(base_size = NULL, small_size = NULL, journal = "cpt-psp")
scale_colour_qspquest(...)
scale_color_qspquest(...)
scale_fill_qspquest(...)
scale_fill_qspquest_continuous(...)
scale_colour_qspquest_continuous(...)
scale_fill_qspquest_diverging(...)
qspquest_palette(n = NULL)
qspquest_journals()
qspquest_style_definitions()
qspquest_plot_profile(result, output = NULL, log = FALSE, label = NULL)
qspquest_plot_observed_predicted(fitted, prediction = c("individual", "population"),
  output = NULL, log = FALSE)
qspquest_plot_residuals(fitted, x = c("time", "prediction"), output = NULL,
  time_unit = NULL)
qspquest_plot_fitted_profile(fitted, output = NULL, log = FALSE, time_unit = NULL)
qspquest_plot_parameters(fitted)
qspquest_plot_morris(result, label = NULL)
qspquest_plot_scan(result, output = NULL, metric = c("max", "min", "last", "auc"),
  label = NULL)
qspquest_save_figure(plot, path, width = NULL, height = NULL, journal = "cpt-psp",
  formats = c("pdf", "svg", "png"), dpi = 600)
}
\arguments{
\item{base_size, small_size}{Type sizes in points for titles and tick labels.
The defaults follow the journal preset.}
\item{journal}{One of \code{"cpt-psp"}, \code{"jpkpd"} or \code{"nature"}; see
\code{qspquest_journals()}.}
\item{...}{Passed to the corresponding 'ggplot2' scale.}
\item{n}{Number of categorical colours (1 to 3); all colours when \code{NULL}.}
\item{result}{For \code{qspquest_plot_profile}, a simulation result, a named
list of simulation results (one line each) or a data frame from
\code{\link{result_data_frame}}. For \code{qspquest_plot_morris} and
\code{qspquest_plot_scan}, a result from \code{\link{morris_model}} or
\code{\link{scan_model}}.}
\item{output}{Output name; required when the result has more than one output.}
\item{log}{Use a logarithmic y axis. Non-positive values are omitted with a warning.}
\item{label}{Variable name for the axis label; the unit is always taken from the result.}
\item{fitted}{A result from \code{\link{fit_population_data}}, or a data frame
from \code{\link{population_fit_observation_data_frame}} or
\code{\link{fit_observation_data_frame}}.}
\item{prediction}{Individual or population prediction on the x axis.}
\item{x}{Plot residuals against time or against the prediction.}
\item{time_unit}{Time unit for rows that do not record one, such as the output
of \code{fit_observation_data_frame}.}
\item{metric}{Summary of each scan trajectory shown in the heat map.}
\item{plot}{A \code{ggplot} object.}
\item{path}{File name. A \code{.pdf}, \code{.svg} or \code{.png} suffix selects
one format; a name without a suffix writes every format in \code{formats}.}
\item{width}{\code{"single-column"}, \code{"double-column"} or a width in mm.
By default figures designed for one column use the single-column width.}
\item{height}{Height in mm; by default the figure's own aspect ratio.}
\item{formats}{Formats written when \code{path} has no suffix.}
\item{dpi}{Resolution of the PNG file.}
}
\details{The style layer (theme, scales, palette, journal presets and
\code{qspquest_save_figure}) reads every colour, line weight and preset from
the installed style file \code{system.file("style", "figure-style.json", package =
"qspquest")}; \code{qspquest_style_definitions()} returns its contents. Colours are
the UniBio Intelligence brand hues, darkened where needed to pass contrast and
colour-vision-deficiency checks on each surface: teal, burgundy and peach in
that fixed order. Set \code{options(qspquest.style_mode = "dark")} to build
figures with the dark-surface colours. Observations are drawn as points and model predictions as
lines. Residuals are observed minus the individual (or model) prediction, in the
units of the observation; these functions do not compute CWRES or NPDE.
Parameter intervals are those reported by the native fit
(\code{fixed_effect_intervals}); none are computed here.

PDF files are written with the Cairo device when available, which embeds the
fonts; SVG files from \code{grDevices::svg} draw text as outlines. Cairo
records a creation date in PDF files.}
\value{The plotting functions return \code{ggplot} objects.
\code{qspquest_save_figure} invisibly returns the written file names.
\code{qspquest_journals} returns a data frame of presets and
\code{qspquest_style_definitions} a list of all style definitions.}
\seealso{\code{\link{simulate_model}}, \code{\link{fit_population_data}}}
\examples{
result <- list(schema = "qspquest.result/v0.1", time_unit = "h",
  times = list(0, 1, 2, 4), sides = list("post", "post", "post", "post"),
  outputs = list(list(name = "cp", unit = "mg/L", values = list(2, 1.81, 1.64, 1.34))),
  identity = list())
if (requireNamespace("ggplot2", quietly = TRUE)) {
  figure <- qspquest_plot_profile(result, label = "Plasma concentration")
  path <- tempfile("profile")
  qspquest_save_figure(figure, path, formats = "png", dpi = 300)
}
}

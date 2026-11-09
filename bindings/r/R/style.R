# Figure style: colour definitions, theme, scales and journal-size export. This layer
# does not depend on pharmacometric results. Every colour, line weight, font
# list and journal preset is read from inst/style/figure-style.json.

.style_cache <- new.env(parent = emptyenv())

qspquest_style_definitions <- function() {
  if (is.null(.style_cache$definitions)) {
    path <- system.file("style", "figure-style.json", package = "qspquest")
    if (!nzchar(path)) stop("the figure style file is not installed", call. = FALSE)
    .style_cache$definitions <- jsonlite::fromJSON(path, simplifyVector = TRUE)
  }
  .style_cache$definitions
}

.style_mode <- function() {
  mode <- getOption("qspquest.style_mode", "light")
  modes <- names(qspquest_style_definitions()$modes)
  if (!is.character(mode) || length(mode) != 1L || !mode %in% modes)
    stop("option qspquest.style_mode must be one of ", paste(modes, collapse = ", "), call. = FALSE)
  mode
}

.style_values <- function(name) qspquest_style_definitions()$modes[[.style_mode()]][[name]]

.style_color <- function(name) {
  if (!name %in% c("surface", "ink", "muted", "grid", "fill"))
    stop("unknown colour name: ", name, call. = FALSE)
  .style_values(name)
}

# ggplot2 line widths are in mm; the style file gives points.
.style_line <- function(name) qspquest_style_definitions()$lines_pt[[name]] / 72.27 * 25.4
.style_point <- function() qspquest_style_definitions()$point_size_mm

qspquest_journals <- function() {
  definitions <- qspquest_style_definitions()
  presets <- definitions$journals
  data.frame(
    JOURNAL = names(presets),
    SINGLE_MM = vapply(presets, `[[`, numeric(1), "single_mm"),
    DOUBLE_MM = vapply(presets, `[[`, numeric(1), "double_mm"),
    MAX_HEIGHT_MM = vapply(presets, `[[`, numeric(1), "max_height_mm"),
    BASE_PT = vapply(presets, `[[`, numeric(1), "base_pt"),
    SMALL_PT = vapply(presets, `[[`, numeric(1), "small_pt"),
    row.names = NULL, stringsAsFactors = FALSE)
}

.need_ggplot2 <- function() {
  if (!requireNamespace("ggplot2", quietly = TRUE))
    stop("install the ggplot2 package to make QSPQuest figures", call. = FALSE)
}

.journal <- function(journal) {
  presets <- qspquest_journals()
  if (!is.character(journal) || length(journal) != 1L || !journal %in% presets$JOURNAL)
    stop("journal must be one of ", paste(presets$JOURNAL, collapse = ", "), call. = FALSE)
  presets[presets$JOURNAL == journal, ]
}

qspquest_palette <- function(n = NULL) {
  colours <- .style_values("categorical")
  if (is.null(n)) return(colours)
  if (!is.numeric(n) || length(n) != 1L || is.na(n) || n < 1 || n > length(colours) || n != round(n))
    stop("the validated categorical palette has ", length(colours), " colours; facet the figure ",
         "when more groups are compared", call. = FALSE)
  colours[seq_len(n)]
}

theme_qspquest <- function(base_size = NULL, small_size = NULL, journal = "cpt-psp") {
  .need_ggplot2()
  preset <- .journal(journal)
  base <- if (is.null(base_size)) preset$BASE_PT else base_size
  small <- if (is.null(small_size)) preset$SMALL_PT else small_size
  if (!is.numeric(base) || !is.numeric(small) || small < 4 || small > base || base > 16)
    stop("type sizes must satisfy 4 <= small_size <= base_size <= 16 pt", call. = FALSE)
  ink <- .style_color("ink")
  surface <- .style_color("surface")
  ggplot2::theme_minimal(base_size = base, base_family = "sans") +
    ggplot2::theme(
      text = ggplot2::element_text(colour = ink),
      plot.background = ggplot2::element_rect(fill = surface, colour = surface),
      panel.background = ggplot2::element_rect(fill = surface, colour = surface),
      panel.grid.minor = ggplot2::element_blank(),
      panel.grid.major = ggplot2::element_line(colour = .style_color("grid"),
                                               linewidth = .style_line("grid")),
      axis.title = ggplot2::element_text(colour = ink, size = base),
      axis.text = ggplot2::element_text(colour = .style_color("muted"), size = small),
      strip.text = ggplot2::element_text(colour = ink, face = "bold", hjust = 0, size = base),
      strip.background = ggplot2::element_blank(),
      legend.position = "bottom", legend.title = ggplot2::element_blank(),
      legend.box = "vertical",
      legend.text = ggplot2::element_text(colour = ink, size = small),
      plot.title = ggplot2::element_text(colour = ink, face = "bold", size = base + 1))
}

scale_colour_qspquest <- function(...) {
  .need_ggplot2()
  ggplot2::scale_colour_manual(values = qspquest_palette(), ...)
}
scale_color_qspquest <- scale_colour_qspquest
scale_fill_qspquest <- function(...) {
  .need_ggplot2()
  ggplot2::scale_fill_manual(values = qspquest_palette(), ...)
}
scale_fill_qspquest_continuous <- function(...) {
  .need_ggplot2()
  ggplot2::scale_fill_gradientn(colours = .style_values("sequential"), ...)
}
scale_colour_qspquest_continuous <- function(...) {
  .need_ggplot2()
  ggplot2::scale_colour_gradientn(colours = .style_values("sequential")[-(1:2)], ...)
}
scale_fill_qspquest_diverging <- function(...) {
  .need_ggplot2()
  ggplot2::scale_fill_gradientn(colours = .style_values("diverging"), ...)
}

.size_mm <- function(plot, width, height) {
  attr(plot, "qspquest_size_mm") <- c(width, height)
  plot
}

qspquest_save_figure <- function(plot, path, width = NULL, height = NULL, journal = "cpt-psp",
                                 formats = c("pdf", "svg", "png"), dpi = 600) {
  .need_ggplot2()
  if (!inherits(plot, "ggplot")) stop("plot must be a ggplot object", call. = FALSE)
  preset <- .journal(journal)
  if (!is.numeric(dpi) || length(dpi) != 1L || dpi < 150 || dpi > 2400)
    stop("dpi must be between 150 and 2400", call. = FALSE)
  size <- attr(plot, "qspquest_size_mm")
  if (is.null(size)) size <- c(preset$SINGLE_MM, 0.75 * preset$SINGLE_MM)
  if (is.null(width)) width <- if (size[1] <= 100) "single-column" else "double-column"
  width_mm <- if (is.character(width)) {
    switch(width, "single-column" = preset$SINGLE_MM, "double-column" = preset$DOUBLE_MM,
           stop("width must be 'single-column', 'double-column' or a number of mm", call. = FALSE))
  } else width
  if (!is.numeric(width_mm) || width_mm < 20 || width_mm > preset$DOUBLE_MM)
    stop("width in mm must be between 20 and the double-column width", call. = FALSE)
  height_mm <- if (is.null(height)) min(width_mm * size[2] / size[1], preset$MAX_HEIGHT_MM) else height
  if (!is.numeric(height_mm) || height_mm < 15 || height_mm > preset$MAX_HEIGHT_MM)
    stop("height must be between 15 and ", preset$MAX_HEIGHT_MM, " mm for ", journal, call. = FALSE)
  extension <- tolower(tools::file_ext(path))
  if (nzchar(extension)) {
    if (!extension %in% c("pdf", "svg", "png"))
      stop("file suffix must be .pdf, .svg or .png", call. = FALSE)
    formats <- extension
    stem <- tools::file_path_sans_ext(path)
  } else {
    formats <- match.arg(formats, several.ok = TRUE)
    stem <- path
  }
  sized <- plot + ggplot2::theme(
    axis.title = ggplot2::element_text(size = preset$BASE_PT),
    axis.text = ggplot2::element_text(size = preset$SMALL_PT),
    strip.text = ggplot2::element_text(size = preset$BASE_PT),
    legend.text = ggplot2::element_text(size = preset$SMALL_PT),
    plot.title = ggplot2::element_text(size = preset$BASE_PT + 1))
  written <- character()
  for (format in formats) {
    target <- paste0(stem, ".", format)
    device <- switch(format,
      pdf = if (capabilities("cairo")) grDevices::cairo_pdf else grDevices::pdf,
      svg = grDevices::svg,
      png = "png")
    arguments <- list(filename = target, plot = sized, width = width_mm, height = height_mm,
                      units = "mm", dpi = dpi, device = device, bg = .style_color("surface"))
    if (format == "png" && capabilities("cairo")) arguments$type <- "cairo"
    do.call(ggplot2::ggsave, arguments)
    written <- c(written, target)
  }
  invisible(written)
}

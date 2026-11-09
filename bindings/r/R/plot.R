# Pharmacometric figures with ggplot2 (optional). Colours, line weights and
# sizes come only from the style layer in R/style.R.

.col <- function(name) as.name(name)

.label <- function(name, unit) {
  if (is.null(unit) || is.na(unit) || unit %in% c("", "1", "dimensionless")) return(name)
  paste0(name, " (", gsub(" * ", "\u00b7", unit, fixed = TRUE), ")")
}

.one <- function(values, what) {
  values <- unique(values[!is.na(values)])
  if (!length(values)) stop("the result records no ", what, call. = FALSE)
  if (length(values) > 1L)
    stop("the figure needs one ", what, "; found ", paste(values, collapse = ", "), call. = FALSE)
  values
}

.positive <- function(frame, columns, log) {
  if (!log) return(frame)
  keep <- Reduce(`&`, lapply(columns, function(column) !is.na(frame[[column]]) & frame[[column]] > 0))
  if (any(!keep)) warning(sum(!keep), " non-positive values omitted from the logarithmic axis",
                          call. = FALSE)
  if (!any(keep)) stop("no positive values remain for the logarithmic axis", call. = FALSE)
  frame[keep, , drop = FALSE]
}

.trajectory_frame <- function(result) {
  if (is.data.frame(result)) {
    needed <- c("TIME", "OUTPUT", "VALUE", "UNIT")
    if (!all(needed %in% names(result)) || is.null(attr(result, "time_unit")))
      stop("a trajectory data frame must come from result_data_frame()", call. = FALSE)
    result$GROUP <- if ("GROUP" %in% names(result)) result$GROUP else NA_character_
    return(result)
  }
  if (is.list(result) && identical(result$schema, "qspquest.result/v0.1")) {
    frame <- result_data_frame(result)
    frame$GROUP <- NA_character_
    return(frame)
  }
  if (is.list(result) && length(result) && !is.null(names(result)) && all(nzchar(names(result))) &&
      all(vapply(result, function(x) is.list(x) && identical(x$schema, "qspquest.result/v0.1"), logical(1)))) {
    frames <- lapply(names(result), function(label) {
      frame <- result_data_frame(result[[label]])
      frame$GROUP <- label
      frame
    })
    units <- unique(vapply(result, function(x) x$time_unit, character(1)))
    if (length(units) != 1L) stop("the runs use different time units", call. = FALSE)
    frame <- do.call(rbind, frames)
    frame$GROUP <- factor(frame$GROUP, levels = names(result))
    attr(frame, "time_unit") <- units
    return(frame)
  }
  if (is.list(result) && identical(result$schema, "qspquest.profile/v0.1"))
    stop("this is a likelihood profile, not a trajectory", call. = FALSE)
  stop("result must be a simulation result, a named list of simulation results ",
       "or a data frame from result_data_frame()", call. = FALSE)
}

qspquest_plot_profile <- function(result, output = NULL, log = FALSE, label = NULL) {
  .need_ggplot2()
  frame <- .trajectory_frame(result)
  time_unit <- attr(frame, "time_unit")
  outputs <- unique(frame$OUTPUT)
  if (is.null(output)) {
    if (length(outputs) != 1L)
      stop("the result has outputs ", paste(outputs, collapse = ", "), "; choose one with output",
           call. = FALSE)
    output <- outputs
  }
  if (!output %in% outputs) stop("unknown output: ", output, call. = FALSE)
  frame <- frame[frame$OUTPUT == output, , drop = FALSE]
  unit <- .one(frame$UNIT, "output unit")
  frame <- .positive(frame, "VALUE", log)
  grouped <- !all(is.na(frame$GROUP))
  plot <- ggplot2::ggplot(frame, ggplot2::aes(x = !!.col("TIME"), y = !!.col("VALUE")))
  if (!grouped) {
    plot <- plot + ggplot2::geom_path(colour = qspquest_palette()[1], linewidth = .style_line("model"))
  } else if (length(unique(frame$GROUP)) <= length(qspquest_palette())) {
    plot <- plot + ggplot2::geom_path(ggplot2::aes(colour = !!.col("GROUP")), linewidth = .style_line("model")) +
      scale_colour_qspquest()
  } else {
    plot <- plot + ggplot2::geom_path(colour = qspquest_palette()[1], linewidth = .style_line("model")) +
      ggplot2::facet_wrap(ggplot2::vars(!!.col("GROUP")))
  }
  if (log) plot <- plot + ggplot2::scale_y_log10()
  name <- if (is.null(label)) output else label
  plot <- plot + ggplot2::labs(x = .label("Time", time_unit),
    y = paste0(.label(name, unit), if (log) ", log scale" else "")) + theme_qspquest()
  .size_mm(plot, 85, 65)
}

.fit_frame <- function(fitted, time_unit = NULL) {
  frame <- if (inherits(fitted, "qspquest_population_fit_result")) {
    population_fit_observation_data_frame(fitted)
  } else fitted
  if (!is.data.frame(frame) || !all(c("ID", "TIME", "DV", "DV_UNIT", "PRED", "PRED_UNIT") %in% names(frame)))
    stop("fitted must come from fit_population_data(), population_fit_observation_data_frame() ",
         "or fit_observation_data_frame()", call. = FALSE)
  if (!nrow(frame)) stop("the fit has no fitted observation rows", call. = FALSE)
  if (any(frame$DV_UNIT != frame$PRED_UNIT) ||
      ("IPRED_UNIT" %in% names(frame) && any(frame$DV_UNIT != frame$IPRED_UNIT)))
    stop("observed and predicted values have different units", call. = FALSE)
  recorded <- if ("TIME_UNIT" %in% names(frame)) unique(frame$TIME_UNIT) else character()
  if (length(recorded) > 1L) stop("rows use more than one time unit", call. = FALSE)
  if (length(recorded) == 1L) {
    if (!is.null(time_unit) && !identical(time_unit, recorded))
      stop("time_unit differs from the recorded unit ", recorded, call. = FALSE)
    time_unit <- recorded
  }
  attr(frame, "time_unit") <- time_unit
  frame
}

.one_output <- function(frame, output) {
  outputs <- unique(frame$OUTPUT)
  if (is.null(output)) {
    if (length(outputs) != 1L)
      stop("the fit has outputs ", paste(outputs, collapse = ", "), "; choose one with output",
           call. = FALSE)
    output <- outputs
  }
  if (!output %in% outputs) stop("unknown output: ", output, call. = FALSE)
  output
}

qspquest_plot_observed_predicted <- function(fitted, prediction = c("individual", "population"),
                                             output = NULL, log = FALSE) {
  .need_ggplot2()
  frame <- .fit_frame(fitted)
  output <- .one_output(frame, output)
  frame <- frame[frame$OUTPUT == output, , drop = FALSE]
  prediction <- match.arg(prediction)
  has_individual <- "IPRED" %in% names(frame)
  column <- if (has_individual && prediction == "individual") "IPRED" else "PRED"
  name <- if (!has_individual) "Model prediction" else if (column == "IPRED")
    "Individual prediction" else "Population prediction"
  frame$X <- frame[[column]]
  frame <- .positive(frame, c("X", "DV"), log)
  unit <- .one(frame$DV_UNIT, "output unit")
  limits <- range(c(frame$X, frame$DV, if (!log) 0))
  suffix <- if (log) ", log scale" else ""
  plot <- ggplot2::ggplot(frame, ggplot2::aes(x = !!.col("X"), y = !!.col("DV"))) +
    ggplot2::geom_abline(intercept = 0, slope = 1, colour = .style_color("muted"),
                         linewidth = .style_line("reference"), linetype = "dashed") +
    ggplot2::geom_point(shape = 21, fill = .style_color("surface"), colour = .style_color("ink"), size = .style_point(), stroke = 0.4) +
    ggplot2::coord_equal(xlim = limits, ylim = limits) +
    ggplot2::labs(x = paste0(.label(paste0(name, ", ", output), unit), suffix),
                  y = paste0(.label(paste0("Observed ", output), unit), suffix)) +
    theme_qspquest()
  if (log) plot <- plot + ggplot2::scale_x_log10() + ggplot2::scale_y_log10()
  .size_mm(plot, 80, 80)
}

qspquest_plot_residuals <- function(fitted, x = c("time", "prediction"), output = NULL,
                                    time_unit = NULL) {
  .need_ggplot2()
  x <- match.arg(x)
  frame <- .fit_frame(fitted, time_unit)
  output <- .one_output(frame, output)
  frame <- frame[frame$OUTPUT == output, , drop = FALSE]
  unit <- .one(frame$DV_UNIT, "output unit")
  individual <- "IPRED" %in% names(frame)
  frame$Y <- if (individual) frame$DV - frame$IPRED else frame$DV - frame$PRED
  ylabel <- .label(if (individual) "Residual, observed \u2212 individual prediction"
                   else "Residual, observed \u2212 prediction", unit)
  if (x == "time") {
    if (is.null(attr(frame, "time_unit")))
      stop("these rows record no time unit; pass time_unit", call. = FALSE)
    frame$X <- frame$TIME
    xlabel <- .label("Time", attr(frame, "time_unit"))
  } else {
    frame$X <- if (individual) frame$IPRED else frame$PRED
    xlabel <- .label(paste0(if (individual) "Individual prediction" else "Model prediction",
                            ", ", output), unit)
  }
  plot <- ggplot2::ggplot(frame, ggplot2::aes(x = !!.col("X"), y = !!.col("Y"))) +
    ggplot2::geom_hline(yintercept = 0, colour = .style_color("muted"), linewidth = .style_line("reference")) +
    ggplot2::geom_point(shape = 21, fill = .style_color("surface"), colour = .style_color("ink"), size = .style_point(), stroke = 0.4) +
    ggplot2::labs(x = xlabel, y = ylabel) + theme_qspquest()
  .size_mm(plot, 80, 65)
}

qspquest_plot_fitted_profile <- function(fitted, output = NULL, log = FALSE, time_unit = NULL) {
  .need_ggplot2()
  frame <- .fit_frame(fitted, time_unit)
  if (is.null(attr(frame, "time_unit")))
    stop("these rows record no time unit; pass time_unit", call. = FALSE)
  output <- .one_output(frame, output)
  frame <- frame[frame$OUTPUT == output, , drop = FALSE]
  unit <- .one(frame$DV_UNIT, "output unit")
  individual <- "IPRED" %in% names(frame)
  series <- if (individual) c(IPRED = "Individual prediction", PRED = "Population prediction")
            else c(PRED = "Model prediction")
  lines <- do.call(rbind, lapply(names(series), function(column)
    data.frame(ID = frame$ID, TIME = frame$TIME, VALUE = frame[[column]],
               SERIES = series[[column]], stringsAsFactors = FALSE)))
  lines$SERIES <- factor(lines$SERIES, levels = unname(series))
  lines <- .positive(lines, "VALUE", log)
  points <- .positive(frame, "DV", log)
  plot <- ggplot2::ggplot(lines, ggplot2::aes(x = !!.col("TIME"), y = !!.col("VALUE"))) +
    ggplot2::geom_path(ggplot2::aes(colour = !!.col("SERIES"), linetype = !!.col("SERIES")),
                       linewidth = .style_line("model")) +
    ggplot2::geom_point(data = points, ggplot2::aes(x = !!.col("TIME"), y = !!.col("DV")),
                        shape = 21, fill = .style_color("surface"), colour = .style_color("ink"), size = .style_point(), stroke = 0.4) +
    ggplot2::scale_colour_manual(values = qspquest_palette(length(series))) +
    ggplot2::scale_linetype_manual(values = c("solid", "dashed")[seq_along(series)]) +
    ggplot2::labs(x = .label("Time", attr(frame, "time_unit")),
                  y = paste0(.label(output, unit), if (log) ", log scale" else "")) +
    theme_qspquest()
  subjects <- unique(frame$ID)
  if (length(subjects) > 1L) plot <- plot + ggplot2::facet_wrap(ggplot2::vars(!!.col("ID")))
  if (log) plot <- plot + ggplot2::scale_y_log10()
  .size_mm(plot, if (length(subjects) > 1L) 180 else 85,
           if (length(subjects) > 1L) 30 + 40 * ceiling(length(subjects) / 4) else 70)
}

qspquest_plot_parameters <- function(fitted) {
  .need_ggplot2()
  population <- if (inherits(fitted, "qspquest_population_fit_result")) fitted$result$population
                else fitted$population
  intervals <- population$fixed_effect_intervals
  if (is.null(intervals) || !length(intervals$intervals))
    stop("the fit has no fixed-effect confidence intervals", call. = FALSE)
  frame <- do.call(rbind, lapply(intervals$intervals, function(x)
    data.frame(PARAMETER = x$name, ESTIMATE = as.double(x$estimate), LOWER = as.double(x$lower),
               UPPER = as.double(x$upper), UNIT = x$unit, stringsAsFactors = FALSE)))
  if (any(frame$ESTIMATE <= 0 | frame$LOWER <= 0 | frame$UPPER <= 0))
    stop("relative intervals need positive estimates and bounds", call. = FALSE)
  frame$LABEL <- sprintf("%s: %.3g [%.3g, %.3g]", vapply(seq_len(nrow(frame)), function(i)
    .label(frame$PARAMETER[i], frame$UNIT[i]), character(1)), frame$ESTIMATE, frame$LOWER, frame$UPPER)
  frame$LABEL <- factor(frame$LABEL, levels = rev(frame$LABEL))
  frame$LOW <- frame$LOWER / frame$ESTIMATE
  frame$HIGH <- frame$UPPER / frame$ESTIMATE
  frame$MID <- 1
  level <- paste0(format(100 * intervals$confidence_level), "% ")
  plot <- ggplot2::ggplot(frame, ggplot2::aes(y = !!.col("LABEL"))) +
    ggplot2::geom_vline(xintercept = 1, colour = .style_color("muted"), linewidth = .style_line("reference"),
                        linetype = "dashed") +
    ggplot2::geom_segment(ggplot2::aes(x = !!.col("LOW"), xend = !!.col("HIGH"),
                                       yend = !!.col("LABEL")),
                          colour = qspquest_palette()[1], linewidth = .style_line("interval")) +
    ggplot2::geom_point(ggplot2::aes(x = !!.col("MID")), colour = .style_color("ink"), size = 1.25 * .style_point()) +
    ggplot2::scale_x_log10() +
    ggplot2::labs(x = paste0("Ratio to estimate, ", level, "confidence interval (log scale)"), y = NULL) +
    theme_qspquest() + ggplot2::theme(panel.grid.major.y = ggplot2::element_blank())
  .size_mm(plot, 85, 25 + 9 * nrow(frame))
}

qspquest_plot_morris <- function(result, label = NULL) {
  .need_ggplot2()
  if (!is.list(result) || !identical(result$schema, "qspquest.morris-result/v0.1"))
    stop("result must come from morris_model()", call. = FALSE)
  effects <- result$result$effects
  frame <- data.frame(
    PARAMETER = vapply(effects, `[[`, character(1), "parameter"),
    MU_STAR = vapply(effects, function(e) as.double(e$mu_star), numeric(1)),
    SIGMA = vapply(effects, function(e) as.double(e$sigma), numeric(1)),
    UNIT = vapply(effects, `[[`, character(1), "unit"), stringsAsFactors = FALSE)
  unit <- .one(frame$UNIT, "effect unit")
  top <- max(c(frame$MU_STAR, frame$SIGMA)) * 1.1
  if (top == 0) top <- 1
  name <- if (is.null(label)) "" else paste0(", ", label)
  plot <- ggplot2::ggplot(frame, ggplot2::aes(x = !!.col("MU_STAR"), y = !!.col("SIGMA"))) +
    ggplot2::geom_abline(intercept = 0, slope = 1, colour = .style_color("grid"), linewidth = .style_line("reference")) +
    ggplot2::geom_point(colour = qspquest_palette()[1], size = 1.6 * .style_point()) +
    ggplot2::geom_text(ggplot2::aes(label = !!.col("PARAMETER")), colour = .style_color("ink"),
                       size = .journal(qspquest_style_definitions()$default_journal)$SMALL_PT / ggplot2::.pt, vjust = -0.8) +
    ggplot2::coord_equal(xlim = c(0, top), ylim = c(0, top)) +
    ggplot2::labs(x = .label(paste0("\u03bc*, mean absolute elementary effect", name), unit),
                  y = .label(paste0("\u03c3, SD of elementary effects", name), unit)) +
    theme_qspquest()
  .size_mm(plot, 80, 80)
}

qspquest_plot_scan <- function(result, output = NULL, metric = c("max", "min", "last", "auc"),
                               label = NULL) {
  .need_ggplot2()
  if (!is.list(result) || !identical(result$schema, "qspquest.scan-result/v0.1"))
    stop("result must come from scan_model()", call. = FALSE)
  metric <- match.arg(metric)
  points <- result$results
  parameters <- do.call(rbind, lapply(points, function(point) data.frame(
    POINT = point$id, PARAMETER = names(point$parameters),
    VALUE = vapply(point$parameters, function(q) as.double(q$value), numeric(1)),
    UNIT = vapply(point$parameters, function(q) q$unit, character(1)), stringsAsFactors = FALSE)))
  varied <- unique(parameters$PARAMETER)[vapply(unique(parameters$PARAMETER), function(p)
    length(unique(parameters$VALUE[parameters$PARAMETER == p])) > 1L, logical(1))]
  if (!length(varied)) stop("no parameter varies across the scan points", call. = FALSE)
  if (length(varied) > 2L) stop("the scan varies more than two parameters", call. = FALSE)
  trajectory <- do.call(rbind, lapply(points, function(point) {
    frame <- result_data_frame(point$result)
    frame$POINT <- point$id
    frame$TIME_UNIT <- point$result$time_unit
    frame
  }))
  output <- .one_output(trajectory, output)
  trajectory <- trajectory[trajectory$OUTPUT == output, , drop = FALSE]
  unit <- .one(trajectory$UNIT, "output unit")
  time_unit <- .one(trajectory$TIME_UNIT, "time unit")
  value_of <- function(p) {
    values <- parameters[parameters$PARAMETER == p, ]
    values$VALUE[match(trajectory$POINT, values$POINT)]
  }
  unit_of <- function(p) .one(parameters$UNIT[parameters$PARAMETER == p], paste(p, "unit"))
  name <- if (is.null(label)) output else label
  if (length(varied) == 1L) {
    trajectory$PARAMETER_VALUE <- value_of(varied)
    plot <- ggplot2::ggplot(trajectory, ggplot2::aes(x = !!.col("TIME"), y = !!.col("VALUE"),
        group = !!.col("POINT"), colour = !!.col("PARAMETER_VALUE"))) +
      ggplot2::geom_path(linewidth = .style_line("model")) +
      ggplot2::scale_colour_gradientn(colours = .style_values("sequential")[-(1:2)],
                                      name = .label(varied, unit_of(varied))) +
      ggplot2::labs(x = .label("Time", time_unit), y = .label(name, unit)) +
      theme_qspquest() + ggplot2::theme(legend.position = "right",
                                        legend.title = ggplot2::element_text(colour = .style_color("ink")))
    return(.size_mm(plot, 85, 65))
  }
  summary <- vapply(split(trajectory, trajectory$POINT), function(part) switch(metric,
    max = max(part$VALUE), min = min(part$VALUE), last = part$VALUE[nrow(part)],
    auc = sum(diff(part$TIME) * (part$VALUE[-1] + part$VALUE[-nrow(part)]) / 2)), numeric(1))
  frame <- data.frame(POINT = names(summary), METRIC = unname(summary), stringsAsFactors = FALSE)
  for (p in varied) {
    values <- parameters[parameters$PARAMETER == p, ]
    frame[[p]] <- values$VALUE[match(frame$POINT, values$POINT)]
  }
  metric_names <- c(max = "Maximum", min = "Minimum", last = "Last sampled value",
                    auc = "AUC over the sampled window")
  metric_unit <- if (metric == "auc") paste(unit, "*", time_unit) else unit
  plot <- ggplot2::ggplot(frame, ggplot2::aes(x = !!.col(varied[1]), y = !!.col(varied[2]),
                                              fill = !!.col("METRIC"))) +
    ggplot2::geom_tile(colour = .style_color("surface"), linewidth = .style_line("edge")) +
    scale_fill_qspquest_continuous(name = .label(paste0(metric_names[[metric]], ", ", name), metric_unit)) +
    ggplot2::labs(x = .label(varied[1], unit_of(varied[1])), y = .label(varied[2], unit_of(varied[2]))) +
    theme_qspquest() + ggplot2::theme(legend.position = "right",
                                      legend.title = ggplot2::element_text(colour = .style_color("ink")))
  .size_mm(plot, 85, 70)
}

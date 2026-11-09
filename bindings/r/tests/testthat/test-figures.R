run_result <- function(values = list(0, 2, 1.6, 1.1), unit = "mg/L", time_unit = "h") {
  list(schema = "qspquest.result/v0.1", time_unit = time_unit,
       times = list(0, 1, 2, 4), sides = list("post", "post", "post", "post"),
       outputs = list(list(name = "cp", unit = unit, values = values)), identity = list())
}

labels_of <- function(plot) {
  if (exists("get_labs", asNamespace("ggplot2"))) ggplot2::get_labs(plot) else plot$labels
}

built <- function(plot) {
  expect_s3_class(plot, "ggplot")
  expect_no_error(ggplot2::ggplot_build(plot))
  invisible(plot)
}

test_that("profile labels come from the result units", {
  skip_if_not_installed("ggplot2")
  plot <- built(qspquest_plot_profile(run_result()))
  expect_identical(labels_of(plot)$x, "Time (h)")
  expect_identical(labels_of(plot)$y, "cp (mg/L)")
  named <- qspquest_plot_profile(run_result(), label = "Plasma concentration")
  expect_identical(labels_of(named)$y, "Plasma concentration (mg/L)")
  expect_warning(logged <- qspquest_plot_profile(run_result(), log = TRUE), "non-positive")
  built(logged)
  expect_match(labels_of(logged)$y, "log scale")
  grouped <- built(qspquest_plot_profile(list(A = run_result(), B = run_result())))
  expect_identical(labels_of(grouped)$y, "cp (mg/L)")
  frame <- result_data_frame(run_result())
  built(qspquest_plot_profile(frame))
})

test_that("profile inputs are validated", {
  skip_if_not_installed("ggplot2")
  expect_error(qspquest_plot_profile(list(schema = "qspquest.profile/v0.1")), "likelihood profile")
  expect_error(qspquest_plot_profile(run_result(), output = "missing"), "unknown output")
  expect_error(qspquest_plot_profile(list(A = run_result(), B = run_result(time_unit = "d"))),
               "different time units")
  expect_error(qspquest_plot_profile(data.frame(TIME = 1)), "result_data_frame")
})

fit_rows <- function() {
  data.frame(ID = rep(c("1", "2"), each = 3), TIME = rep(c(1, 2, 4), 2), TIME_UNIT = "h",
             OUTPUT = "cp", DV = c(2, 1.5, 1, 2.2, 1.7, 1.1), DV_UNIT = "mg/L",
             PRED = c(2.1, 1.6, 1, 2.1, 1.6, 1), PRED_UNIT = "mg/L",
             IPRED = c(2, 1.55, 0.98, 2.15, 1.68, 1.08), IPRED_UNIT = "mg/L",
             stringsAsFactors = FALSE)
}

test_that("fit diagnostics build with units and validate rows", {
  skip_if_not_installed("ggplot2")
  plot <- built(qspquest_plot_observed_predicted(fit_rows()))
  expect_identical(labels_of(plot)$x, "Individual prediction, cp (mg/L)")
  expect_identical(labels_of(plot)$y, "Observed cp (mg/L)")
  plot <- built(qspquest_plot_observed_predicted(fit_rows(), prediction = "population"))
  expect_identical(labels_of(plot)$x, "Population prediction, cp (mg/L)")
  plot <- built(qspquest_plot_residuals(fit_rows()))
  expect_identical(labels_of(plot)$x, "Time (h)")
  expect_match(labels_of(plot)$y, "^Residual, observed .+ individual prediction \\(mg/L\\)$")
  built(qspquest_plot_residuals(fit_rows(), x = "prediction"))
  built(qspquest_plot_fitted_profile(fit_rows()))
  rows <- fit_rows()
  rows$PRED_UNIT <- "ug/L"
  expect_error(qspquest_plot_observed_predicted(rows), "different units")
  rows <- fit_rows()[, setdiff(names(fit_rows()), c("TIME_UNIT", "IPRED", "IPRED_UNIT"))]
  expect_error(qspquest_plot_residuals(rows), "time unit")
  plot <- built(qspquest_plot_residuals(rows, time_unit = "h"))
  expect_match(labels_of(plot)$y, "^Residual, observed .+ prediction \\(mg/L\\)$")
  expect_false(grepl("individual", labels_of(plot)$y))
  expect_error(qspquest_plot_residuals(fit_rows(), time_unit = "d"), "differs")
  expect_error(qspquest_plot_observed_predicted(list()), "fit_population_data")
})

test_that("parameter intervals come only from the native fit", {
  skip_if_not_installed("ggplot2")
  fitted <- list(population = list(fixed_effect_intervals = list(confidence_level = 0.95,
    intervals = list(list(name = "cl", estimate = 2, lower = 1.6, upper = 2.5, unit = "L/h")))))
  plot <- built(qspquest_plot_parameters(fitted))
  expect_match(labels_of(plot)$x, "95% confidence interval")
  expect_error(qspquest_plot_parameters(list(population = list())), "no fixed-effect")
})

test_that("Morris and scan figures read their units", {
  skip_if_not_installed("ggplot2")
  morris <- list(schema = "qspquest.morris-result/v0.1", result = list(effects = list(
    list(parameter = "cl", unit = "mg/L", effects = list(-0.6, -0.7), mu = -0.65, mu_star = 0.65, sigma = 0.07),
    list(parameter = "v", unit = "mg/L", effects = list(-0.5, -0.4), mu = -0.45, mu_star = 0.45, sigma = 0.07))))
  plot <- built(qspquest_plot_morris(morris))
  expect_match(labels_of(plot)$x, "mg/L")
  expect_error(qspquest_plot_morris(list(schema = "other")), "morris_model")
  point <- function(id, cl, v) list(id = id, parameters = list(
    cl = list(value = cl, unit = "L/h"), v = list(value = v, unit = "L")), result = run_result())
  one <- list(schema = "qspquest.scan-result/v0.1",
              results = list(point("a", 1, 3), point("b", 2, 3)))
  plot <- built(qspquest_plot_scan(one))
  expect_identical(labels_of(plot)$y, "cp (mg/L)")
  two <- list(schema = "qspquest.scan-result/v0.1", results = list(point("a", 1, 3),
    point("b", 2, 3), point("c", 1, 6), point("d", 2, 6)))
  plot <- built(qspquest_plot_scan(two, metric = "auc"))
  expect_identical(labels_of(plot)$x, "cl (L/h)")
  expect_identical(labels_of(plot)$y, "v (L)")
  flat <- list(schema = "qspquest.scan-result/v0.1", results = list(point("a", 1, 3)))
  expect_error(qspquest_plot_scan(flat), "no parameter varies")
})

test_that("palette, theme and saving follow the journal presets", {
  skip_if_not_installed("ggplot2")
  definitions <- qspquest_style_definitions()
  expect_identical(qspquest_palette(), definitions$modes$light$categorical)
  expect_identical(qspquest_palette(2), definitions$modes$light$categorical[1:2])
  old <- options(qspquest.style_mode = "dark")
  expect_identical(qspquest_palette(), definitions$modes$dark$categorical)
  expect_identical(theme_qspquest()$plot.background$fill, definitions$modes$dark$surface)
  options(qspquest.style_mode = "sepia")
  expect_error(qspquest_palette(), "style_mode")
  options(old)
  expect_identical(qspquest_journals()$SINGLE_MM[qspquest_journals()$JOURNAL == "jpkpd"], 84)
  expect_error(qspquest_palette(4), "facet")
  expect_error(theme_qspquest(journal = "unknown"), "journal")
  expect_s3_class(theme_qspquest(journal = "nature"), "theme")
  plot <- qspquest_plot_profile(run_result())
  path <- tempfile("figure")
  written <- qspquest_save_figure(plot, path, formats = c("pdf", "png"), dpi = 300)
  expect_true(all(file.exists(written)))
  expect_identical(tools::file_ext(written), c("pdf", "png"))
  expect_error(qspquest_save_figure(plot, paste0(path, ".jpg")), "suffix")
  expect_error(qspquest_save_figure(plot, path, width = 500), "width")
  expect_error(qspquest_save_figure(plot, path, height = 400), "height")
  expect_error(qspquest_save_figure(list(), path), "ggplot")
})

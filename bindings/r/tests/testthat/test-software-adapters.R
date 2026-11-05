# Independent rxode2 evaluation of synthetic QSPQuest adapter exports. Generate
# the exports with bindings/python/tests/test_software_adapters.py and name
# their directory in QSPQUEST_ADAPTER_CHECKS.
test_that("rxode2 and nlmixr2 exports reproduce the analytic solution", {
  skip_if_not_installed("rxode2")
  work <- Sys.getenv("QSPQUEST_ADAPTER_CHECKS")
  skip_if(!nzchar(work), "QSPQUEST_ADAPTER_CHECKS does not name an export directory")
  work <- normalizePath(work, mustWork = TRUE)

  rxode2::setRxThreads(1L)
  ui <- rxode2::rxode2(function() {
    ini({ ka <- 1.1; cl <- 0.3; v <- 3 })
    model({
      d/dt(depot) <- -ka * depot
      d/dt(central) <- ka * depot - cl / v * central
      cp <- central / v
    })
  })
  qspquest_export_rxode2_ui(ui, file.path(work, "rxode2-snapshot.json"))
  times <- c(0, 0.25, 1, 2, 4, 8)
  for (software in c("rxode2", "nlmixr2")) {
    path <- file.path(work, paste0(software, "-export.txt"))
    model <- if (software == "rxode2") {
      rxode2::rxode2(paste(readLines(path), collapse = "\n"))
    } else {
      rxode2::rxode2(eval(parse(path), envir = new.env(parent = globalenv())))
    }
    params <- if (software == "rxode2") c(par1 = 1.1, par2 = 0.3, par3 = 3) else NULL
    events <- data.frame(time = c(0, times), evid = c(1L, rep(0L, length(times))),
                         amt = c(6, rep(0, length(times))), cmt = 1L)
    solved <- as.data.frame(rxode2::rxSolve(model, params = params, events = events,
                                            rtol = 1e-10, atol = 1e-12))
    expected <- 6 / 3 * 1.1 / (1.1 - 0.1) * (exp(-0.1 * times) - exp(-1.1 * times))
    error <- max(abs(solved$output1 - expected))
    expect_equal(nrow(solved), length(times))
    expect_lt(error, 2e-8)
    jsonlite::write_json(list(software = software, times = times,
                              predictions = solved$output1, max_absolute_error = error,
                              rxode2 = as.character(utils::packageVersion("rxode2")),
                              R = R.version.string),
                         file.path(work, paste0(software, "-check.json")),
                         auto_unbox = TRUE, pretty = TRUE, digits = NA)
  }
})

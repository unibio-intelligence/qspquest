test_that("the installed quickstart runs", {
  quickstart <- system.file("doc", "quickstart.R", package = "qspquest")
  skip_if_not(nzchar(quickstart), "the quickstart is not installed")
  expect_no_error(source(quickstart, local = new.env()))
})

# QSPQuest synthetic user-guide example. Run in a directory for analysis outputs.
library(qspquest)
library(jsonlite)

source <- "model quickstart_one_compartment
time_unit = h
parameters
  cl = 0.3 [L/h] log
  v = 3 [L] log
states
  central = 0 [mg]
dynamics
  d/dt(central) = input_central - cl / v * central
outputs
  cp = central / v [mg/L]
dosing
  central [mg] = 1
end
"
model <- compile_model(source)
q <- function(value, unit) list(value = value, unit = unit)

times <- c(0, 1, 2, 4, 8, 12)
request <- list(
  schema = "qspquest.run/v0.1",
  solver = "diffsol_bdf",
  budgets = list(solver_callbacks = 1000000L,
                 events = 100L, output_values = 10000L),
  regimen = list(
    end = q(12, "h"), samples = list(),
    observations = lapply(times, function(t) list(time = q(t, "h"), side = "post")),
    administrations = list(list(
      target = "central", time = q(0, "h"), amount = q(6, "mg"),
      delivery = list(kind = "bolus"), bioavailability = 1
    )),
    rtol = 1e-10, atol = 1e-12
  )
)
result <- simulate_model(model, request)
frame <- result_data_frame(result)
cp <- frame[frame$OUTPUT == "cp", ]
stopifnot(
  identical(cp$TIME, as.double(times)),
  all(cp$SIDE == "post"),
  max(abs(cp$VALUE - 2 * exp(-0.1 * cp$TIME))) < 1e-8
)
print(cp)

# Optional file workflow after saving a request later in this guide:
# request <- fromJSON("run.request.json", simplifyVector = FALSE)

png("trajectory.png", width = 900, height = 600)
plot(cp$TIME, cp$VALUE, type = "b",
     xlab = paste0("Time (", attr(frame, "time_unit"), ")"),
     ylab = "cp (mg/L)")
write.csv(frame, "trajectory.csv", row.names = FALSE)
dev.off()

changed <- request
changed$parameters <- list(cl = q(0.6, "L/h"), v = q(3, "L"))
faster_clearance <- simulate_model(model, changed)

repeated <- request
repeated$regimen$administrations[[1]][["repeat"]] <- list(
  interval = q(4, "h"), additional = 2L
)
repeated_result <- simulate_model(model, repeated) # Doses at 0, 4, and 8 h.

infusion <- request
infusion$regimen$administrations[[1]]$delivery <- list(
  kind = "infusion",
  span = list(kind = "duration", duration = q(2, "h"))
)
infusion_result <- simulate_model(model, infusion) # 6 mg over 2 h.

oral_source <- "model quickstart_oral_two_compartment
time_unit = h
parameters
  cl = 1 [L/h] log
  vc = 5 [L] log
  vp = 10 [L] log
  exchange = 0.5 [L/h] log
  ka = 1 [1/h] log
states
  depot = 0 [mg]
  central = 0 [mg]
  peripheral = 0 [mg]
dynamics
  d/dt(depot) = -ka * depot
  d/dt(central) = ka * depot - cl / vc * central - exchange * (central / vc - peripheral / vp)
  d/dt(peripheral) = exchange * (central / vc - peripheral / vp)
outputs
  cp = central / vc [mg/L]
  depot_amount = depot [mg]
  central_amount = central [mg]
  peripheral_amount = peripheral [mg]
dosing
  depot [mg] bolus = 1
end
"
oral_model <- compile_model(oral_source)
oral_request <- request
oral_request$regimen$administrations[[1]]$target <- "depot"
oral_request$regimen$administrations[[1]]$amount <- q(100, "mg")
oral_request$regimen$administrations[[1]]$bioavailability <- 0.8
oral_result <- simulate_model(oral_model, oral_request)
oral_frame <- result_data_frame(oral_result)
stopifnot(
  oral_frame$VALUE[oral_frame$TIME == 0 & oral_frame$OUTPUT == "cp"] == 0,
  oral_frame$VALUE[oral_frame$TIME == 0 & oral_frame$OUTPUT == "depot_amount"] == 80,
  all(oral_frame$VALUE >= -1e-8)
)

at_dose <- request
at_dose$regimen$observations <- list(
  list(time = q(0, "h"), side = "pre"),
  list(time = q(0, "h"), side = "post")
)
event_result <- simulate_model(model, at_dose)
stopifnot(identical(unlist(event_result$outputs[[1]]$values), c(0, 2)))

scan_request <- list(
  schema = "qspquest.scan/v0.1", run = request,
  points = lapply(c(0.3, 0.6, 1.2), function(value) list(
    id = paste0("cl_", value), parameters = list(cl = q(value, "L/h"))
  )),
  total_budgets = list(solver_callbacks = 3000000L,
                       events = 300L, output_values = 30000L)
)
scan <- scan_model(model, scan_request)
scan_table <- do.call(rbind, lapply(scan$results, function(point) {
  data <- result_data_frame(point$result)
  data$POINT <- point$id
  data
}))
print(scan_table[scan_table$TIME == 12, ])

morris_request <- list(
  schema = "qspquest.morris/v0.1",
  problem = list(kind = "generated", request = list(
    run = request,
    ranges = list(
      list(parameter = "cl", lower = q(0.1, "L/h"), upper = q(0.7, "L/h")),
      list(parameter = "v", lower = q(3, "L"), upper = q(6, "L"))
    ),
    trajectory_count = 2L, num_levels = 4L, seed = 42L,
    total_budgets = list(solver_callbacks = 6000000L,
                         events = 600L, output_values = 60000L),
    output = "cp", row = 3L # Zero-based: the selected 4 h observation.
  ))
)
morris <- morris_model(model, morris_request)
print(morris$result$effects)

sensitivity_request <- list(
  schema = "qspquest.sensitivity/v0.1", run = request,
  with_respect_to = list("cl", "v"),
  derivative_absolute_tolerances = list(
    cl = list(central = q(1e-12, "mg.h/L")),
    v = list(central = q(1e-12, "mg/L"))
  )
)
observations <- lapply(seq_along(times), function(index) list(
  row = index - 1L, output = "cp",
  value = q(6 / 4.5 * exp(-0.6 * times[index] / 4.5), "mg/L"),
  error = list(additive_sd = q(0.1, "mg/L"), proportional_sd = 0)
))
fit_request <- list(
  schema = "qspquest.fit/v0.1",
  problem = list(kind = "individual", request = list(
    objective = list(simulation = sensitivity_request,
                     observations = observations, priors = list()),
    bounds = list(
      list(parameter = "cl", lower = q(0.05, "L/h"), upper = q(1.2, "L/h")),
      list(parameter = "v", lower = q(1, "L"), upper = q(8, "L"))
    ),
    max_evaluations = 400L, max_iterations = 100L, gradient_tolerance = 1e-5
  ))
)
fitted <- fit_model(model, fit_request)
print(fitted$fit$status)
print(fitted$fit$parameters)
stopifnot(
  fitted$fit$status == "converged",
  abs(fitted$fit$parameters$cl$value - 0.6) < 1e-4,
  abs(fitted$fit$parameters$v$value - 4.5) < 1e-4
)
replay <- request
replay$parameters <- fitted$fit$parameters
fit_predictions <- simulate_model(model, replay)

# Replace the synthetic data below with your own validated file when needed.
# data <- read.csv("observations.csv", stringsAsFactors = FALSE,
#                  colClasses = c(ID = "character"))
# Check numeric TIME/AMT/DV, explicit MDV, CMT mappings and declared units.

data <- do.call(rbind, lapply(c("A", "B"), function(id) {
  data.frame(
    ID = id, TIME = c(0, 0, 1, 2, 4), EVID = c(1L, 0L, 0L, 0L, 0L),
    CMT = c(1L, 2L, 2L, 2L, 2L), AMT = c(6, 0, 0, 0, 0),
    MDV = c(1L, 0L, 0L, 0L, 0L),
    DV = c(NA_real_, 6 / 4.5 * exp(-0.6 * c(0, 1, 2, 4) / 4.5))
  )
}))
rownames(data) <- NULL

run_template <- request
run_template$regimen$samples <- list()
run_template$regimen$observations <- NULL
run_template$regimen$administrations <- list()
dataset <- nonmem_requests(
  data, run_template, compartments = c("1" = "central"),
  time_unit = "h", amount_unit = "mg", observation_side = "post"
)
predicted_data <- simulate_data(model, dataset)
print(head(predicted_data$data))

fit_template <- fit_request
fit_template$problem$request$objective$simulation$run <- NULL
fit_template$problem$request$objective$observations <- list()
endpoints <- list("2" = list(
  output = "cp", value_unit = "mg/L",
  error = list(additive_sd = q(0.1, "mg/L"), proportional_sd = 0)
))
prepared <- fit_data_requests(dataset, fit_template, endpoints, mode = "pooled")
data_fit <- fit_data(model, prepared)
stopifnot(data_fit$results[[1]]$fit$status == "converged")
print(fit_parameter_data_frame(data_fit))
diagnostics <- fit_observation_data_frame(model, data_fit)
print(head(diagnostics))

etas <- c(-0.22, -0.08, 0.08, 0.22)
population_rows <- do.call(rbind, lapply(seq_along(etas), function(index) {
  t <- c(1, 2, 4)
  data.frame(
    ID = paste0("S", index), TIME = c(0, t), EVID = c(1L, 0L, 0L, 0L),
    CMT = c(1L, 2L, 2L, 2L), AMT = c(6, 0, 0, 0), MDV = c(1L, 0L, 0L, 0L),
    DV = c(NA_real_, 6 / 4.5 * exp(-0.6 * exp(etas[index]) * t / 4.5))
  )
}))
rownames(population_rows) <- NULL
pop_sensitivity <- sensitivity_request
pop_sensitivity$with_respect_to <- list("cl")
pop_sensitivity$derivative_absolute_tolerances$v <- NULL
pop_sensitivity$run <- run_template
pop_sensitivity$run$parameters <- list(cl = q(0.5, "L/h"), v = q(4.5, "L"))

population_template <- list(
  schema = "qspquest.fit/v0.1",
  problem = list(kind = "focei", request = list(
    subjects = list(list(simulation = pop_sensitivity,
                         observations = list(), priors = list())),
    fixed_effects = list(list(parameter = "cl", lower = q(0.35, "L/h"),
                             upper = q(0.85, "L/h"))),
    random_effects = list(list(parameter = "cl")),
    omega = list(list(0.04)), omega_diagonal_bounds = list(list(0.04, 0.04)),
    eta_bound = 1.5, max_evaluations = 30000L,
    max_iterations = 80L, gradient_tolerance = 0.005, seed = 17L
  ))
)
population_endpoints <- list("2" = list(
  output = "cp", value_unit = "mg/L",
  error = list(additive_sd = q(0.05, "mg/L"), proportional_sd = 0)
))
population_prepared <- population_fit_data_requests(
  population_rows, population_template, compartments = c("1" = "central"),
  endpoints = population_endpoints, time_unit = "h", amount_unit = "mg",
  observation_side = "post"
)
population_fit <- fit_population_data(model, population_prepared)
population <- population_fit$result$population
print(population$status)
print(population$objective_kind)
print(population$fixed_effects)
stopifnot(
  population$status == "converged",
  abs(population$fixed_effects$cl$value - 0.6) < 0.06
)
subject_table <- population_fit_subject_data_frame(population_fit)
population_diagnostics <- population_fit_observation_data_frame(population_fit)
print(subject_table)
print(head(population_diagnostics))

saem_template <- population_template
saem_template$problem$kind <- "saem"
saem_template$problem$request$max_iterations <- 8L
saem_prepared <- population_fit_data_requests(
  population_rows, saem_template, compartments = c("1" = "central"),
  endpoints = population_endpoints, time_unit = "h", amount_unit = "mg",
  observation_side = "post"
)
saem_fit <- fit_population_data(model, saem_prepared)
saem <- saem_fit$result$population
print(saem$status)
print(saem$objective_kind)
saem_subjects <- population_fit_subject_data_frame(saem_fit)
saem_observations <- population_fit_observation_data_frame(saem_fit)

dir.create("qspquest-analysis", showWarnings = FALSE)
writeLines(source, "qspquest-analysis/model.qsp")
save_json <- function(object, filename) {
  write_json(object, file.path("qspquest-analysis", filename),
             auto_unbox = TRUE, pretty = TRUE, digits = NA, null = "null")
}
save_json(request, "run.request.json")
save_json(result, "run.result.json")
save_json(fit_request, "fit.request.json")
save_json(fitted, "fit.result.json")
save_json(population_prepared$request, "population.request.json")
save_json(population_fit$result, "population.result.json")
write.csv(population_fit$mapping, "qspquest-analysis/population.mapping.csv", row.names = FALSE)
saveRDS(population_fit, "qspquest-analysis/population-fit.rds")
write.csv(population_rows, "qspquest-analysis/population.rows.csv", row.names = FALSE)
saveRDS(frame, "qspquest-analysis/trajectory.rds")
writeLines(capture.output(sessionInfo()), "qspquest-analysis/session-info.txt")
print(result$identity)

invalid <- request
invalid$parameters <- list(cl = q(1, "h"))
caught <- tryCatch(
  simulate_model(model, invalid),
  qspquest_error = function(error) {
    print(error$code)
    message(conditionMessage(error))
    error
  }
)
stopifnot(inherits(caught, "qspquest_error"))
stopifnot(identical(simulate_model(model, request), result))

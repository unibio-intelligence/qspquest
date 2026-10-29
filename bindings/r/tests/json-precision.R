library(qspquest)
values <- c(1.2589544678037516, 32.26306704495241, 2^53-1)
encoded <- qspquest:::.encode(list(values=values))
stopifnot(identical(jsonlite::fromJSON(encoded)$values, values))
raw <- '{"value":1.2589544678037516}'
stopifnot(identical(qspquest:::.encode(raw), raw))
source <- paste(
 "model precision", "time_unit = h", "parameters",
 " theta = 1 [mg/L] estimated", " decay = 1 [1/h] fixed",
 "states", " unused = 1 [1]", "dynamics", " d/dt(unused) = -decay * unused",
 "outputs", " y = theta [mg/L]", "end", sep="\n")
request_json <- paste0('{"schema":"qspquest.run/v0.1","solver":"diffsol_bdf",',
 '"parameters":{"theta":{"value":1.2589544678037516,"unit":"mg/L"}},',
 '"budgets":{"solver_callbacks":100000,"events":100,"output_values":1000},',
 '"regimen":{"end":{"value":1,"unit":"h"},"samples":[{"value":0,"unit":"h"},{"value":1,"unit":"h"}],',
 '"administrations":[],"rtol":1e-10,"atol":1e-12}}')
request <- jsonlite::fromJSON(request_json,simplifyVector=FALSE)
model <- compile_model(source)
from_text <- simulate_model(model,request_json)
from_list <- simulate_model(model,request)
stopifnot(identical(from_text$identity,from_list$identity),
          identical(from_text$outputs,from_list$outputs))

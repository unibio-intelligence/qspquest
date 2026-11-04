# Fit proportional normal error scales

Fit bounded model parameters together with a separate residual standard deviation
for each observed output. The residual model is normal on the output scale:

`Y_i ~ Normal(mu_i, (s_output * abs(mu_i))^2)`.

For each trial model parameter vector, the exact conditional maximum-likelihood
scale is `s_output = sqrt(mean((Y_i / mu_i - 1)^2))`, so the optimizer only
searches over the model parameters.

## Prerequisites and command

Use a source checkout with Rust and Cargo installed. Run from the repository root:

The complete [model](../examples/profiled-proportional.qsp) and
[fit request](../examples/profiled-proportional-fit.json) are included.

```sh
cargo run --release -p qspquest-cli -- fit \
  docs/examples/profiled-proportional.qsp \
  docs/examples/profiled-proportional-fit.json
```

The model has two constant concentration outputs: `y1 = theta` and
`y2 = 2 * theta`. Each has four synthetic observations. The model's decay state
does not affect either output.

The result contains `fit` and `residual_scales`. Expect a `converged` status,
`theta` close to `2.5 mg/L`, and dimensionless residual scales close to
`y1 = 0.08944272` and `y2 = 0.14422205`, one per output.

## Python and R

After [installing the Python package from source](../installation.md), run from
the repository root:

```python
import json
from pathlib import Path
import qspquest

source = Path("docs/examples/profiled-proportional.qsp").read_text()
request = json.loads(Path("docs/examples/profiled-proportional-fit.json").read_text())
model = qspquest.CompiledSensitivities(source, ["theta"])
plan = model.plan_fit(request)
result = model.fit(request)
print(result["fit"]["status"], result["residual_scales"])
```

After [installing the R package from source](../installation.md#r), use the same
model and request:

```r
source <- paste(readLines("docs/examples/profiled-proportional.qsp"), collapse = "\n")
request <- jsonlite::fromJSON("docs/examples/profiled-proportional-fit.json", simplifyVector = FALSE)
model <- qspquest::compile_model(source)
result <- qspquest::fit_model(model, request)
print(result$fit$status)
print(result$residual_scales)
```

The request selects `problem.kind = "profiled_proportional"`.
`problem.kind = "individual"` uses fixed residual errors instead. `plan_fit`
checks the request without running it; zero predictions or a zero residual
scale can still make the fit fail.

## Rust API and inputs

The Rust example also reads the complete versioned request:

```sh
cargo run --release -p qspquest-modeling --example profiled_proportional_fit -- \
  docs/examples/profiled-proportional.qsp \
  docs/examples/profiled-proportional-fit.json
```

Compile a `CompiledSensitivityDocument` with the selected parameter names, then
call `execute_fit(&FitRequest)` for the versioned workflow. The lower-level
`fit_profiled_proportional_gaussian(&GaussianFitRequest)` function accepts the
nested Gaussian request directly. To evaluate the
profile likelihood and its analytic sensitivity gradient without optimizing,
call `profiled_proportional_gaussian_objective(&GaussianObjectiveRequest)`.

Supply finite bounds for every selected model parameter, in selected parameter
order, and explicit simulation and derivative tolerances. Observation `row`
indexes the returned simulation grid. For dosing experiments, explicit regimen
`observations` select exactly the desired times and pre/post sides; default
event-inclusive grids can contain additional rows.

For these profiled fits, every row must set `additive_sd = 0` and
`proportional_sd = 1`; this tells the engine to estimate the scale, which is
returned in `residual_scales`. Each
observed output needs at least two rows. Optional normal priors apply to model
parameters; scales have no priors. The methods fit one experiment, with scales
grouped by output name.

## Limits and troubleshooting

Censored rows, lognormal errors, combined additive/proportional errors and
nonzero fixed additive errors are rejected. Zero predictions, or data that fit
the model exactly (zero residuals), are also rejected.

Check `fit.status`, `fit.projected_gradient_norm`, `fit.evaluations` and `fit.iterations`.
An evaluation or iteration limit is an incomplete fit even when parameters look
plausible. Near the optimum, solver error can limit gradient accuracy; rerun
with tighter simulation and derivative tolerances before treating a small
objective change as convergence. The example uses synthetic data.

## MCP

For a user-supplied model, call `fit_individual` with the complete versioned
request in `fit_request`, including `problem.kind = "profiled_proportional"`.
`fit_population` also accepts that request. Library models can only be fitted
with their stored fitting workflow.

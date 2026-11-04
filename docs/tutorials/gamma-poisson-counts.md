# Fit event counts with measured exposure

Fit independent integer counts whose underlying event rates vary between rows.
Each count has a positive exposure:

`Y_i | lambda_i ~ Poisson(exposure_i * lambda_i)`

`lambda_i ~ Gamma(shape=alpha, rate=beta)`.

The latent rates are integrated analytically. All rows share the fitted Gamma
shape and rate. The complete [synthetic request](../examples/gamma-poisson-fit.json)
contains five counts observed over one hour each.

## Run from a source checkout

With Rust and Cargo installed, run from the repository root:

```sh
cargo run --release -p qspquest-cli -- fit-data docs/examples/gamma-poisson-fit.json
```

Expect `status = "converged"`, shape approximately `0.772885`, Gamma rate
approximately `0.128814 h`, and negative log likelihood approximately
`14.29792892`. The mean latent event intensity is `alpha / beta`, approximately
`6 /h` in this example. The likelihood is normalized and includes every count
factorial term.

## Python and Rust

After [installing the Python package from source](../installation.md), use:

```python
import json
from pathlib import Path
import qspquest

request = json.loads(Path("docs/examples/gamma-poisson-fit.json").read_text())
result = qspquest.fit_gamma_poisson(request)
print(result["status"], result["parameters"])
```

The Rust entry point is `qspquest_modeling::discrete_fit::fit_gamma_poisson`.
`gamma_poisson_objective` returns the normalized likelihood and its analytic
gradient and Hessian in log(shape), log(rate) coordinates.

## R

After [installing the R package from source](../installation.md#r), run from
the repository root:

```r
request <- jsonlite::fromJSON("docs/examples/gamma-poisson-fit.json", simplifyVector = FALSE)
result <- qspquest::fit_gamma_poisson(request)
stopifnot(result$status == "converged")
print(result$parameters)
```

## Units and controls

Counts must be nonnegative integers. Every exposure must be finite and positive.
Exposure units must be compatible with `initial_rate.unit`; compatible units are
converted before fitting. In the Gamma shape–rate convention, beta has exposure
units and lambda has reciprocal exposure units. Shape is dimensionless. Use
`unit = "1"` for explicitly normalized exposure coordinates.

Supply positive finite shape and rate bounds containing the starting values.
The optimizer works on the log scale, and `gradient_tolerance` applies there.
Check `status`, `projected_gradient_norm`, `evaluations` and `iterations`;
hitting an evaluation or iteration limit means the fit is incomplete.
`active_bounds` lists estimates that ended at a bound.

The fit supports 2–4096 rows, at most one million total observed events, and
at most twenty million operations under
`(total observed events + number of rows) * max_evaluations`.
`max_evaluations` and `max_iterations` each support 1–10000.
The result is a point estimate for one shared Gamma distribution, without
confidence intervals. Group effects and rates shared across rows are not
supported.

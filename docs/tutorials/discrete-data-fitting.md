# Fit count and binomial data

Two native workflows fit discrete observations without an ODE model. Each takes
a complete versioned request and returns bounded point estimates without
confidence intervals. Run the commands from the repository root after
[installing](../installation.md) the interface you use.

## Event counts with measured exposure

Fit independent integer counts whose underlying event rates vary between rows.
Each count has a positive exposure:

`Y_i | lambda_i ~ Poisson(exposure_i * lambda_i)`

`lambda_i ~ Gamma(shape=alpha, rate=beta)`.

The latent rates are integrated analytically. All rows share the fitted Gamma
shape and rate. The complete [synthetic request](../examples/gamma-poisson-fit.json)
contains five counts observed over one hour each.

### Run from a source checkout

With Rust and Cargo installed, run from the repository root:

```sh
cargo run --release -p qspquest-cli -- fit-data docs/examples/gamma-poisson-fit.json
```

Expect `status = "converged"`, shape approximately `0.772885`, Gamma rate
approximately `0.128814 h`, and negative log likelihood approximately
`14.29792892`. The mean latent event intensity is `alpha / beta`, approximately
`6 /h` in this example. The likelihood is normalized and includes every count
factorial term.

### Python and Rust

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

### R

After [installing the R package from source](../installation.md#r), run from
the repository root:

```r
request <- jsonlite::fromJSON("docs/examples/gamma-poisson-fit.json", simplifyVector = FALSE)
result <- qspquest::fit_gamma_poisson(request)
stopifnot(result$status == "converged")
print(result$parameters)
```

### Units and controls

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

## Correlated binomial outcomes

Fit success counts from independent litters or clusters in two groups. A
normally distributed probit effect produces correlation within each litter:

`Y_i | Z_i ~ Binomial(total_i, Phi(mean[group_i] + sd[group_i] * Z_i))`

`Z_i ~ Normal(0, 1)` independently across litters. Means and SDs use dimensionless
probit coordinates. `Phi` is the standard normal cumulative distribution.
The complete [synthetic request](../examples/probit-normal-binomial-fit.json)
has one litter with each possible outcome for two trials in each group.

### Run from a source checkout

With Rust and Cargo installed, run from the repository root:

```sh
cargo run --release -p qspquest-cli -- fit-data docs/examples/probit-normal-binomial-fit.json
```

Expect `status = "converged"`, `means` approximately `[0, 0]`, `sds`
approximately `[1, 1]`, and negative log likelihood approximately `6.59167373`.
For this synthetic example each group assigns probability one third to each
of its three possible litter outcomes. The likelihood includes the binomial
coefficients.

### Python and Rust

After [installing the Python package from source](../installation.md), run:

```python
import json
from pathlib import Path
import qspquest

request = json.loads(Path("docs/examples/probit-normal-binomial-fit.json").read_text())
result = qspquest.fit_probit_normal_binomial(request)
print(result["status"], result["means"], result["sds"])
```

The Rust function is
`qspquest_modeling::probit_normal::fit_probit_normal_binomial`;
its request type is `qspquest_core::probit_fit::ProbitNormalBinomialFitRequest`.
`probit_normal_binomial_objective` returns the normalized negative log
likelihood, gradient and Hessian in the coordinate order
`[mean_0, mean_1, variance_0, variance_1]`.

### R

After [installing the R package from source](../installation.md#r), run from
the repository root:

```r
request <- jsonlite::fromJSON("docs/examples/probit-normal-binomial-fit.json", simplifyVector = FALSE)
result <- qspquest::fit_probit_normal_binomial(request)
stopifnot(result$status == "converged")
print(result$means)
print(result$sds)
```

### Inputs and work limits

Each litter needs integer `successes` between zero and integer `total`, inclusive,
and `group` equal to zero or one. Supply at least two litters per group, at most
128 litters overall, and 1–1000 trials per litter. Means must lie within
`[-8, 8]`; SDs within `[0, 3]`. Bounds must have positive width and contain the
starts. Zero SD is supported, including as a starting value.

`gradient_tolerance` applies to the projected mean/variance gradient. `active_bounds` uses
the same four-coordinate order. Check bounds and `projected_gradient_norm`
before using the result.
`max_evaluations` and `max_iterations` each support 1–1000. Reaching either
limit reports an incomplete fit through `status`.

Set `max_quadrature_evaluations` between 1 and 20000000. It caps the numerical
integration work for the whole fit; the actual count is returned as
`quadrature_evaluations`. Each litter evaluation is also capped at 65536.
Running out returns a `work_budget` error. If the integration cannot reach the
required accuracy, the fit returns an error rather than an inaccurate result.

This workflow returns point estimates for two group means and two litter-effect
SDs, without confidence intervals.

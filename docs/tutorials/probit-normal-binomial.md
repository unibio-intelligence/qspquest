# Fit correlated binomial outcomes

Fit success counts from independent litters or clusters in two groups. A
normally distributed probit effect produces correlation within each litter:

`Y_i | Z_i ~ Binomial(total_i, Phi(mean[group_i] + sd[group_i] * Z_i))`

`Z_i ~ Normal(0, 1)` independently across litters. Means and SDs use dimensionless
probit coordinates. `Phi` is the standard normal cumulative distribution.
The complete [synthetic request](../examples/probit-normal-binomial-fit.json)
has one litter with each possible outcome for two trials in each group.

## Run from a source checkout

With Rust and Cargo installed, run from the repository root:

```sh
cargo run --release -p qspquest-cli -- fit-data docs/examples/probit-normal-binomial-fit.json
```

Expect `status = "converged"`, `means` approximately `[0, 0]`, `sds`
approximately `[1, 1]`, and negative log likelihood approximately `6.59167373`.
For this synthetic example each group assigns probability one third to each
of its three possible litter outcomes. The normalized likelihood includes
binomial coefficients and the unbounded normal mixture.

## Python and Rust

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

## R

After [installing the R package from source](../installation.md#r), run from
the repository root:

```r
request <- jsonlite::fromJSON("docs/examples/probit-normal-binomial-fit.json", simplifyVector = FALSE)
result <- qspquest::fit_probit_normal_binomial(request)
stopifnot(result$status == "converged")
print(result$means)
print(result$sds)
```

## Inputs and work limits

Each litter needs integer `successes` between zero and integer `total`, inclusive,
and `group` equal to zero or one. Supply at least two litters per group, at most
128 litters overall, and 1–1000 trials per litter. Means must lie within
`[-8, 8]`; SDs within `[0, 3]`. Bounds must have positive width and contain the
starts. Zero SD is supported. The optimizer uses variance coordinates, so
starting at zero preserves the variance score.

`gradient_tolerance` applies to the projected mean/variance score.
`active_bounds` uses the same four-coordinate order. Inspect bounds and
`projected_gradient_norm` when interpreting the result.
`max_evaluations` and `max_iterations` each support 1–1000. Reaching either
limit reports an incomplete fit through `status`.

Set `max_quadrature_evaluations` between 1 and 20000000. It bounds conditional
integrand evaluations across the entire fit, including rejected proposals;
the actual count is returned as `quadrature_evaluations`. Each litter evaluation
also has a 65536-evaluation cap. Exhausting a quadrature budget returns a
`work_budget` error. Integration must satisfy refinement checks and bound
omitted tails, including derivative moments; it refuses a result when these
checks fail.

This workflow returns bounded point estimates for two group means and two
litter-effect SDs. It does not calculate confidence intervals or establish
identifiability. Choose the grouping and observation model for the data being
analyzed.

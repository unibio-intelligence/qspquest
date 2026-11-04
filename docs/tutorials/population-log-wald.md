# Positive fixed-effect intervals from native SAEM

When you request uncertainty, a converged SAEM fit returns approximate 95%
intervals for each fixed effect. These intervals are always positive. The fixed
effects must have strictly positive bounds.

Install the Python package using [the installation guide](../installation.md).
Run the [interval example](../examples/population_log_wald.py) from the source
checkout root. It uses the included synthetic
[two-effect data generator](../examples/population_two_effects.py), creates 32
subjects with six observations each, and executes one native SAEM fit:

```sh
python docs/examples/population_log_wald.py
```

For a Rust source installation, run the same example with the compiled CLI:

```sh
cargo build --release --locked -p qspquest-cli
python3 docs/examples/population_log_wald.py --cli target/release/qspquest
```

Both commands print two positive intervals, one for each dimensionless fixed
effect, with method `positive_fixed_effect_log_wald_delta_v1`. The example uses
a temporary directory and leaves no files behind.

For a native `qspquest.fit/v0.1` request, select `kind: saem` and set
`uncertainty: true` inside its population request. Execute the same request
through `CompiledSensitivities.fit(request)` in Python, `qspquest fit` in the CLI,
or the MCP fitting tool. Over local MCP, set a result artifact directory before
fitting, because large population results are returned as downloadable chunks;
check each chunk's length and SHA-256 before parsing. Locally, two-effect SAEM
allows up to 200000 evaluations. Hosted limits are smaller; for large cohorts use
a hosted job rather than dropping subjects. For example, after the Python fit:

```python
population = result["population"]
intervals = population["fixed_effect_intervals"]
assert intervals["method"] == "positive_fixed_effect_log_wald_delta_v1"
assert intervals["confidence_level"] == 0.95
for interval in intervals["intervals"]:
    print(interval["name"], interval["lower"], interval["upper"], interval["unit"])
```

The interval is `exp(log(estimate) +/- 1.959963984540054 * SE/estimate)`.
`SE` comes from the natural-scale observed covariance. Each record names the
parameter and its physical unit and reports the estimate, source standard error,
log-coordinate standard error and interval endpoints, in the same order as the
request's fixed effects.

Check the returned status before using estimates or intervals. Total
`iterations` includes final refinement steps, reported separately as
`saem_diagnostics.marginal_refinement_iterations`; subtract them to get the
number of sampling iterations.

The `fixed_effect_intervals` field is absent if the fit did not request
covariance, did not converge, used another method, or could not compute a
covariance.

These intervals are a large-sample approximation and may not achieve 95%
coverage in small studies. They cover fixed effects only; correlations,
variances, residual errors and covariate coefficients have no intervals here.

If the intervals cannot be computed, the result still contains the fit,
estimates and covariance, plus `fixed_effect_interval_error` explaining why. A
missing interval does not mean zero uncertainty.

# Positive fixed-effect intervals from native SAEM

Native SAEM returns asymptotic 95% log-Wald intervals when a converged fit
requested observed-information uncertainty. The selected fixed effects must
have strictly positive bounds, as required by the lognormal population model.
No additional dependency is needed for interval calculation.

Install the Python package using [the installation guide](../installation.md).
Run the complete [interval example](../examples/population_log_wald.py) from the
released source checkout root. It uses the included synthetic
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
or the native MCP fitting operation. For local MCP stdio, enable its documented private result artifact directory
before fitting; large population results require complete retained resources.
Read every chunk named in the result descriptor and verify its byte length and
SHA-256 before parsing. The complete result has the same population fields.
Two-effect SAEM permits an explicit local evaluation ceiling of 200000.
Hosted interactive limits are smaller; large cohorts require an applicable
governed job route rather than removing study subjects to fit the limit.
For example, after the native Python fit:

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
log-coordinate standard error and interval endpoints. Parameter order follows
the request's fixed effects. Estimates, covariance and work limits are unchanged
by this calculation.

Native SAEM checks the final marginal objective after stochastic sampling.
Read the returned status before interpreting estimates or intervals. Total
`iterations` includes deterministic point-refinement steps. The diagnostic
`saem_diagnostics.marginal_refinement_iterations` reports those steps;
subtract it from the total to recover the sampling iteration count. The final
stochastic gain and mixing diagnostics describe sampling, and refinement uses
the same request evaluation and iteration ceilings.

The `fixed_effect_intervals` field is absent for fits that did not request
covariance, did not converge, or used another fitting method. Covariance refusals
provide no interval. Invalid covariance or an interval outside finite positive
floating-point range produces a structured error. Changing the evaluation limit
does not resolve an unsupported covariance or establish interval validity.

These intervals use a large-sample approximation. They do not guarantee 95%
finite-sample coverage, account for every source of uncertainty, or establish
biological validity. Correlation, variance, residual-error and covariate-coefficient
intervals are outside this method. Bootstrap and profile outputs use their own
named methods and assumptions.

If the optional interval transform cannot produce finite positive bounds, the
result retains the converged fit, estimates and covariance. It omits
`fixed_effect_intervals` and includes `fixed_effect_interval_error` explaining
the refusal. Do not interpret an absent interval as zero uncertainty.

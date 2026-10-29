# Population SAEM fitting

This page describes the native Rust fitting contract. The separate [Python native-simulation analysis workflows](ANALYSIS.md) support a different, explicitly bounded 1–8-effect SAEM contract; do not transfer qualification between the two implementations.

`CompiledSensitivityDocument::fit_population_saem` fits one or two positive
model parameters with lognormal subject effects. Each subject supplies its own
regimen, observations, and fixed model inputs. The request binds the subject
order, initial fixed effects, eta covariance, parameter and variance bounds,
random seed, and work budget. A successful result has `status: "converged"`
and `objective_kind: "saem_marginal_objective_function_value"`. An evaluation
limit may return a complete-data surrogate; that value is not interchangeable
with the marginal objective.

Two-effect sampling combines bounded coordinate moves with a correlated
Gaussian proposal centered at a conditional mode. Its Metropolis ratio includes
the proposal-density correction. Convergence requires stable stochastic updates,
post-burn mixing diagnostics and a bounded refinement of improving admissible
perturbations of the final marginal objective. Refinement shares the fitting work
and iteration limits. A stable update sequence alone can still return
`line_search_failed` or `evaluation_limit`; inspect those results as diagnostics.

## Observation models

The normal likelihood supports fixed additive and proportional standard
deviations, including both terms together. `error.model: "lognormal"` uses
zero additive SD and interprets `proportional_sd` as a dimensionless log-scale
SD. Predictions and values must then be positive. All coefficients are fixed
unless `additive_error_fit` explicitly estimates one common additive SD.

For a below-quantification observation, set `censoring: "left"` and put the
quantification limit in `value`. The likelihood uses the probability below
that limit. Censored observations can use fixed normal or lognormal error.
`additive_error_fit` can estimate a common additive normal SD with mixed
observed and left-censored rows. Its SAEM variance step uses the conditional
expected squared residual under the current prediction and censoring limit.
This estimate requires at least one uncensored observation and a recoverable
cohort; all-censored or weakly identified designs may converge at a bound.
Priors and correlated observation errors are outside this contract.

## Covariate effect

One-effect SAEM can estimate up to four exponential effects through
`covariate_effect`. Supply `parameter`, `covariate`, `subject_values` in the
same order as `subjects`, a `reference`, and finite `initial` and `bounds` for
each coefficient. Put effects after the first in `additional`. The subject
parameter is

`theta_subject = theta_reference * exp(sum_j coefficient_j * (value_j - reference_j) + eta)`.

Values must be finite and subject-constant. The centered design must have full
rank and more subjects than coefficients. The implementation requires fixed
residual error and does not combine fitted covariates with uncertainty.
Two-random-effect covariate inference, linear and power forms, and estimation
of coefficients on time-varying inputs remain outside this contract.

A model can already contain a stepwise time-varying covariate in its equations.
Each subject may then supply `regimen.covariates` and
`regimen.covariate_changes`; the SAEM fit estimates the selected population
parameter and eta covariance while the model-authored covariate coefficient
remains fixed. Changes take effect on the post-event side of their time point.
This path has a synthetic recovery test with different change times and values
across subjects. Linear interpolation remains unsupported by the native model.
Other population fitters reject the SAEM-specific fitted-covariate field.

## Positive fixed-effect intervals

A converged native SAEM fit with observed-information covariance returns
`fixed_effect_intervals` with method `positive_fixed_effect_log_wald_delta_v1`.
These asymptotic 95% intervals use the positive lognormal fixed-effect domain
and preserve the fitted parameters and covariance. See the complete
[log-Wald tutorial](tutorials/population-log-wald.md) for Python and CLI commands,
output fields, refusal behavior and interpretation limits.

## Uncertainty and limits

For a converged one- or two-effect fit with fixed residual error and no fitted
covariate, `uncertainty: true` estimates a local covariance for the fixed
effects and free eta covariance coordinates. It finite-differences the final
Laplace-approximated marginal objective and inverts its observed curvature.
Boundary estimates, including a two-effect correlation at its permitted limit
of −0.95 or 0.95, return an error, as do singular curvature and uncertified
perturbed objectives. The covariance is an asymptotic local approximation; it does
not include stochastic-approximation variability, a sandwich correction, or
interval calibration. The two-effect calculation includes the eta covariance
between effects. Strongly correlated cohorts can produce singular or
boundary curvature; these return an error instead of standard errors.

Two-effect covariance steps scale with the fitted natural parameter values and
stay within their bounds and positive-definite covariance region. The matrix
must also agree under step halving, with maximum scaled change at most 0.001.
Unresolved refinement returns `unsupported`. These checks share the request's
evaluation budget with fitting and the marginal convergence check.

Conditional modes use forward-sensitivity likelihood scores. Their observed
curvature differentiates those scores, avoiding subtraction of nearly equal
likelihood values. For two-effect covariance evaluation, the relative solver
error ceiling is at most `1e-10`; absolute tolerances are tightened by the same
factor, preserving their units. Tighter requested tolerances are retained.
This additional accuracy applies to the covariance calculation. It uses the
same observations, seed, parameter bounds and evaluation budget.

If covariance certification exhausts the evaluation budget, the structured
error code is `work_budget`. Increase `max_evaluations` within the supported
limits or request `uncertainty: false` to inspect diagnostic point estimates.
The latter request does not provide intervals or establish inference readiness.
An uncertified mode or curvature with budget remaining uses `unsupported`.
Nonconverged point results retain their status and objective kind; a
complete-data surrogate cannot be used as a marginal likelihood.

The native tests cover independent likelihood and gradient calculations,
synthetic parameter recovery, censored rows with fitted additive SD,
lognormal residuals, two independent exponential covariates, model-authored
step covariates, and finite symmetric one- and two-effect uncertainty.
These checks do not establish recovery for every model equation,
sampling design, dose history, or parameter boundary.

## Run a two-effect synthetic example

This walkthrough fits a random intercept and slope to 32 synthetic subjects
with six dimensionless observations each. It runs three supported native
contracts: FOCEI with a fixed correlated eta covariance, SAEM with an estimated
eta covariance and fixed additive SD, and SAEM with an estimated eta covariance
and additive SD. It uses no model library or Python packages beyond the
standard library. This is a workflow demonstration, not statistical qualification
or a pharmacological model.

Prerequisites: Python 3.10 or later, the complete matching QSPQuest source
archive, and the [CLI installation prerequisites](installation.md#cli).
Run these commands from the extracted source root. Build the CLI if needed:

```sh
cargo build --release --locked -p qspquest-cli
target/release/qspquest --version
python3 docs/examples/population_two_effects.py prepare population-demo
```

On Windows, use `target/release/qspquest.exe` and your Python executable.
Preparation creates `model.qsp`, a 192-row `observations.csv`, `design.json`
with the synthetic truth and seed, and three complete JSON requests. The
directory must be new; prepare another directory to repeat the example.
Inspect the requests to see all subject inputs, observation rows, units,
parameter bounds, covariance settings, solver tolerances and fitting budgets.

Admit each request before fitting. These commands check its supported shape
and starting inputs without executing numerical fitting:

```sh
target/release/qspquest plan population-demo/model.qsp population-demo/focei-request.json > population-demo/focei-plan.json
target/release/qspquest plan population-demo/model.qsp population-demo/saem-fixed-request.json > population-demo/saem-fixed-plan.json
target/release/qspquest plan population-demo/model.qsp population-demo/saem-fitted-request.json > population-demo/saem-fitted-plan.json
```

Run the fits and summarize their saved results:

```sh
target/release/qspquest fit population-demo/model.qsp population-demo/focei-request.json > population-demo/focei-result.json
target/release/qspquest fit population-demo/model.qsp population-demo/saem-fixed-request.json > population-demo/saem-fixed-result.json
target/release/qspquest fit population-demo/model.qsp population-demo/saem-fitted-request.json > population-demo/saem-fitted-result.json
python3 docs/examples/population_two_effects.py summarize population-demo
```

The printed table contains the two positive fixed effects, the eta variances,
their correlation and the additive SD. The generating fixed effects are
approximately 4.06 and 2.01, the eta variances are 0.16 and 0.09, their
correlation is 0.35 and the additive SD is 0.25. Sampling variation means the
fitted values differ from these inputs. FOCEI holds the supplied covariance
fixed; only the two SAEM routes estimate it. `saem-fitted` also estimates the
SD, using `estimated_residual_error.additive_sd` in its raw result.

Here `z = log(theta_intercept) + (time_in_hours - 1) * log(theta_slope)` before
subject effects and observation error. The fixed effects therefore represent
exponentiated intercept and slope values, not concentrations or a PK rate.
Both eta coordinates and their covariance are dimensionless. All observed
rows refer to the same output `z`; a fitted common SD requires that shared
output. The zero anchor state makes the time grid explicit without adding
another fitted parameter.

`summary.json` retains model/data/request/result byte hashes, native fit request
hashes and SAEM diagnostics. Keep the plans and raw results with it. The helper
refuses to summarize nonconverged results or complete-data surrogate objectives.
Convergence and a successful admission do not certify mixing, identifiability,
interval calibration or scientific suitability. All three requests disable
uncertainty; this example makes no interval-coverage claim. Two-effect FOCEI
uncertainty is unsupported, and fitted residual SD cannot be combined with
native SAEM observed covariance.

If a command fails, read the structured error on stderr before interpreting its
result file. An empty redirected result can mean the command failed. A missing
CLI requires the installation step; a missing request requires preparation.
Do not replace output units, remove bounds or loosen tolerances to hide a fit
failure. A changed design needs its own identifiability and statistical checks.

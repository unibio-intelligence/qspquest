# Population SAEM fitting

This page describes the QSPQuest stochastic approximation
expectation–maximization (SAEM) fitter [1, 2, 3], requested as a `saem` fit
request. The [analysis workflows](ANALYSIS.md) `fit_population` and
`bootstrap_population` run the same SAEM engine on native simulations. The fit
request adds a correlation limit, a lag-one mixing condition and, for one and
two effects, a marginal refinement and observed-information uncertainty.

`CompiledSensitivityDocument::fit_population_saem` fits one to eight positive
model parameters with lognormal subject effects. Each subject supplies its own
regimen, observations and fixed model inputs. The request sets the subject
order, initial fixed effects, eta covariance, parameter and variance bounds,
random seed and work budget. One-effect fits within the marginal scope admit
2 to 1,024 subjects. Two-effect fits within the marginal scope admit 2 to 128
subjects. Other fits admit up to 128 subjects and require at least the number
of effects plus two.

The sampler is componentwise random-walk Metropolis [4] on the log individual
parameters, with each proposal scale adapted towards 35% acceptance during
burn-in. With two or more effects, each sweep adds a joint random-walk move
whose covariance is the subject's own chain covariance, estimated during
burn-in and then fixed (adaptive Metropolis [7]), so correlated conditional
distributions are explored along their principal directions. The proposal
scales can shrink to one thousandth of the effect SD, so precise data, whose
conditional distributions are far narrower than Omega, still move every
chain. One effect uses three single-axis sweeps per iteration; two or more
effects default to one sweep of every axis plus one joint move. `sweeps` sets
the number of sweeps from 1 to 20.

The stochastic-approximation gain is one during burn-in and k^-0.7 after it.
The fixed-effect and covariate M-step is bounded generalized least squares
with the full eta precision. The covariance update starts from the
complete-data moment estimate. `covariance: "diagonal"` keeps only variances.
The update clamps variances to `omega_diagonal_bounds` and, when needed, shrinks
all off-diagonal entries together to cap absolute eta correlations at 0.95
while preserving positive definiteness. The stochastic convergence screen
compares estimates across a cumulative-gain window with a drift limit of 0.01.
The surrogate route additionally requires the median relative change of the
latest 20 M-step updates to be at most `gradient_tolerance`. The marginal routes
apply `gradient_tolerance` to their final refinement. Convergence also requires
post-burn-in acceptance above 5% on every axis, every subject accepted on every
axis, and a lag-one correlation of at most 0.9 for every subject's chain.

Two kinds of fit then refine the estimate on the Laplace-approximated marginal
objective. With one effect, the residual error may be fixed with up to four
covariate effects, or one common additive SD may be fitted without covariates. With
two effects, Omega must be full, there may be no covariates, and the residual
error is fixed or has one fitted additive SD. When a marginal fit produces a
usable marginal objective, the result labels it
`saem_marginal_objective_function_value`. Fits outside the marginal scope and
fits that stop before obtaining a marginal objective report
`saem_complete_data_surrogate`. Check `status` separately. A result with
`iteration_limit`, `line_search_failed` or `evaluation_limit` is diagnostic
only, regardless of its objective kind.

## Observation models

The normal likelihood supports fixed additive and proportional standard
deviations (SDs), including both terms together. `error.model: "lognormal"` uses
zero additive SD and interprets `proportional_sd` as a dimensionless log-scale
SD. Predictions and values must then be positive. All coefficients are fixed
unless `additive_error_fit` estimates a common additive SD or
`proportional_error_fit` estimates a common proportional SD. Both together fit
a combined normal residual model, whose variance is the sum of the two terms.
Every observed row must use the fitted output, the normal model and the
initial SDs. With fitted SDs the residual M-step minimizes the gain-weighted
expected complete-data objective; a constant additive SD has the closed-form
maximizer. `estimated_residual_error` reports the fitted additive SD and, when
fitted, `proportional_sd`.

For a below-quantification observation, set `censoring: "left"` and put the
quantification limit in `value`. The likelihood uses the probability below
that limit. Censored observations can use fixed normal or lognormal error.
Fitted SDs can be estimated with mixed observed and left-censored rows. The
residual step uses the conditional expected squared residual under the
current prediction and censoring limit. With a fitted proportional SD, the
number of observed rows across subjects times the maximum number of post-burn
iterations must be at most 2,000,000. This estimate needs at least one
uncensored observation and enough data to identify the SD; all-censored or
weakly identified designs may converge at a bound. Priors and correlated
observation errors are not supported.

## Covariate effects

SAEM can estimate up to eight exponential covariate effects through
`covariate_effect`. Supply `parameter`, `covariate`, `subject_values` in the
same order as `subjects`, a `reference`, and finite `initial` and `bounds` for
each coefficient. Put effects after the first in `additional`. Each effect
names one random effect, and several effects may act on the same or different
effects. The subject parameter is

`theta_subject = theta_reference * exp(sum_j coefficient_j * (value_j - reference_j) + eta)`.

Values must be finite and subject-constant. The centered design of each effect
must have full rank, and the cohort needs more subjects than effects plus
coefficients. The marginal refinement covers covariates on a one-effect fit
with fixed residual error; other covariate fits report the complete-data
surrogate. The one-effect marginal route accepts at most four covariate
effects. Fitted covariates are not combined with uncertainty. Linear and
power forms and coefficients on time-varying inputs are not supported.

A model can already contain a stepwise time-varying covariate in its equations.
Each subject may then supply `regimen.covariates` and
`regimen.covariate_changes`; the SAEM fit estimates the selected population
parameter and eta covariance while the covariate coefficient written in the
model stays fixed. Changes take effect on the post-event side of their time
point. Linear interpolation is not supported. FOCEI and Laplace reject the
SAEM-specific `covariate_effect`, `covariance`, `sweeps` and
`proportional_error_fit` fields.

## Positive fixed-effect intervals

A converged native SAEM fit with observed-information covariance returns
`fixed_effect_intervals` with method `positive_fixed_effect_log_wald_delta_v1`.
These asymptotic 95% intervals are computed on the log scale [5, 6], so they stay
positive. See the complete
[log-Wald tutorial](tutorials/population-log-wald.md) for Python and CLI commands,
output fields, refusal behavior and interpretation limits.

## Uncertainty and limits

For a converged one- or two-effect fit with fixed residual error and no fitted
covariate, `uncertainty: true` estimates a local covariance for the fixed
effects and free eta covariance coordinates. It finite-differences the final
Laplace-approximated marginal objective and inverts its observed curvature.
Boundary estimates, including a two-effect correlation at its permitted limit
of −0.95 or 0.95, return an error, as do singular curvature and objectives that
fail the accuracy check. The covariance is an asymptotic local approximation; it
does not include stochastic-approximation variability, a sandwich correction,
or interval calibration. The two-effect calculation includes the eta covariance
between effects. Strongly correlated cohorts can produce singular or
boundary curvature; these return an error instead of standard errors.

Two-effect covariance steps scale with the fitted parameter values and stay
within their bounds and the valid covariance region. The matrix must also agree
when the step is halved, with maximum scaled change at most 0.001; otherwise
the result is `unsupported`. These checks share the request's evaluation budget
with fitting.

For two-effect covariance evaluation, QSPQuest tightens the relative solver
tolerance to at most `1e-10` and scales absolute tolerances by the same factor.
Tighter requested tolerances are kept.

If the covariance calculation runs out of evaluation budget, the error code is
`work_budget`. Increase `max_evaluations` within the supported limits, or
request `uncertainty: false` to get point estimates without intervals. If the
budget remains but the calculation cannot be verified, the code is
`unsupported`. Nonconverged results keep their status and objective kind; a
complete-data surrogate cannot be used as a marginal likelihood.

Tests cover likelihood and gradient calculations, synthetic parameter recovery,
censored rows with fitted additive SD, lognormal residuals, covariates and one-
and two-effect uncertainty. They do not show recovery for every model, sampling
design, dose history or parameter boundary.

## Run a two-effect synthetic example

This walkthrough fits a random intercept and slope to 32 synthetic subjects
with six dimensionless observations each. It runs three fits: first-order conditional estimation with interaction (FOCEI) with a
fixed correlated eta covariance, SAEM with an estimated eta covariance and fixed
additive SD, and SAEM with an estimated eta covariance and additive SD. It needs
only the Python standard library. This is a workflow demonstration, not a
pharmacological model.

Prerequisites: Python 3.10 or later, the complete matching QSPQuest source
archive, and the [command-line interface (CLI) installation prerequisites](installation.md#cli).
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

Check each request before fitting. These commands validate its shape and
starting inputs without running the fit:

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
fixed; only the two SAEM fits estimate it. `saem-fitted` also estimates the
SD, using `estimated_residual_error.additive_sd` in its raw result.

Here `z = log(theta_intercept) + (time_in_hours - 1) * log(theta_slope)` before
subject effects and observation error. The fixed effects therefore represent
exponentiated intercept and slope values, not concentrations or a pharmacokinetic (PK) rate.
Both eta coordinates and their covariance are dimensionless. All observed
rows refer to the same output `z`; a fitted common SD requires that shared
output. The zero anchor state makes the time grid explicit without adding
another fitted parameter.

`summary.json` records hashes of the model, data, requests and results, plus
SAEM diagnostics. Keep the plans and raw results with it. The helper refuses to
summarize nonconverged results or complete-data surrogate objectives.
Convergence does not prove adequate mixing, identifiability or scientific
suitability. All three requests disable uncertainty. Two-effect FOCEI
uncertainty is unsupported, and fitted residual SD cannot be combined with
native SAEM observed covariance.

If a command fails, read the structured error on stderr; an empty result file
can mean the command failed. Do not change output units, remove bounds or
loosen tolerances to hide a fit failure. If the design is changed, check
identifiability again.

## References

1. Delyon B, Lavielle M, Moulines E. Convergence of a stochastic approximation
   version of the EM algorithm. *Ann Stat*. 1999;27(1):94–128.
   [doi:10.1214/aos/1018031103](https://doi.org/10.1214/aos/1018031103)
2. Kuhn E, Lavielle M. Coupling a stochastic approximation version of EM with an
   MCMC procedure. *ESAIM Probab Stat*. 2004;8:115–131.
   [doi:10.1051/ps:2004007](https://doi.org/10.1051/ps:2004007)
3. Kuhn E, Lavielle M. Maximum likelihood estimation in nonlinear mixed effects
   models. *Comput Stat Data Anal*. 2005;49(4):1020–1038.
   [doi:10.1016/j.csda.2004.07.002](https://doi.org/10.1016/j.csda.2004.07.002)
4. Metropolis N, Rosenbluth AW, Rosenbluth MN, Teller AH, Teller E. Equation of
   state calculations by fast computing machines. *J Chem Phys*.
   1953;21(6):1087–1092.
   [doi:10.1063/1.1699114](https://doi.org/10.1063/1.1699114)
5. Oehlert GW. A note on the delta method. *Am Stat*. 1992;46(1):27–29.
   [doi:10.1080/00031305.1992.10475842](https://doi.org/10.1080/00031305.1992.10475842)
6. Casella G, Berger RL. *Statistical Inference*. 2nd ed. Chapman & Hall/CRC;
   2024. Reprint of the 2002 Duxbury edition.
   [doi:10.1201/9781003456285](https://doi.org/10.1201/9781003456285)
7. Haario H, Saksman E, Tamminen J. An adaptive Metropolis algorithm.
   *Bernoulli*. 2001;7(2):223–242.
   [doi:10.2307/3318737](https://doi.org/10.2307/3318737)

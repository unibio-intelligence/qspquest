# Native-backed analysis workflows

Install `qspquest[analysis]` from this checkout. The optional NumPy/SciPy layer
adds reproducible scientific workflows using `CompiledModel.run` for every
model evaluation. It does not replace the existing Rust fitting contracts or
change the released Ubi platform routes. The numerical workflows are Python
APIs; they are not currently exposed through CLI, R or WASM bindings.

## Typed requests and result helpers

Python 3.10 or later can use `RunRequest`, `Quantity`, `ParameterSpec`,
`AnalysisObservation` and `AnalysisSubject` from `qspquest`. These are
`TypedDict` contracts: their values remain ordinary dictionaries and serialize
with `json.dumps`. Native request/result definitions in `qspquest.types` are
generated from the shipped JSON schemas. Analysis records live in
`qspquest.analysis_types`.

`CompiledModel.run`, the analysis methods on `CompiledModel`,
`CompiledSensitivities.run` and the dataset's `fit` method return
`AnalysisResult`. Call `.summary()` or `.rows()` without extra dependencies.
`.to_frame()` needs the notebook extra, and `.plot()` needs Matplotlib.
The standalone analysis functions retain their dictionary return contracts.

Run the complete [typed synthetic example](examples/python_typed.py) from the
release source directory after installing the released wheel into a fresh
environment. Replace `WHEEL` with the wheel matching your system:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install WHEEL
python docs/examples/python_typed.py
```

On Windows, activate with `.venv\Scripts\Activate.ps1` in PowerShell. The
example requires only the core package and uses a synthetic model embedded in
the file. A 6 mg bolus into a 3 L compartment with clearance 0.3 L/h produces
`cp(t) = 2 * exp(-0.1*t)` mg/L. It checks six post-dose samples from 0 to 12 h,
prints a completion summary, and reports six verified analytical samples.
This verifies the synthetic numerical workflow, without qualifying a clinical
model. To print the optional DataFrame:

```sh
python -m pip install 'qspquest[notebook]'
python docs/examples/python_typed.py --frame
```

The example calls both `.summary()` and `.to_frame()` with no type ignores.
With a Python type checker, annotate requests explicitly or use the typed
helpers `quantity`, `bolus`, `infusion` and `run_request`. For example,
`Quantity(value=6.0, unti="mg")` has a misspelled key, and
`run_request(end=12.0, times=[0.0], solver_callbacks="many")` supplies a string
where an integer is required. Both violate the declared contracts. Stable
analysis options are explicit keyword arguments, including integer
`max_simulations`, boolean `prediction_corrected`, and supported solver or
optimizer names expressed as literals.

Dynamic JSON strings and unannotated dictionaries remain accepted; they rely
on runtime validation. Static types do not verify unit dimensions, finite
values, parameter bounds, budget limits or scientific qualification. The
native binder and analysis validators enforce those input constraints. Frame
and plot return values use `Any` to keep optional pandas/Matplotlib type
packages out of the core installation.

## Run a complete Python population workflow

The [synthetic population example](examples/python_population.py) generates 24
subjects with six dimensionless observations each, fits a positive population
parameter and its log-effect variance, and checks an independent Gaussian
reference. It embeds its model and observations. Python 3.10 or later and the
released wheel with the `analysis` extra are the only prerequisites.

From the matching release source directory, replace `WHEEL` with the full path
to your wheel; keep `[analysis]` after that path:

```sh
python3 -m venv .venv-analysis
. .venv-analysis/bin/activate
python -m pip install 'WHEEL[analysis]'
python docs/examples/python_population.py
python docs/examples/python_population.py --restart-demo population-restart-demo
```

On Windows, activate with `.venv-analysis\Scripts\Activate.ps1`. The optional
restart demonstration requires a new directory. It cancels at iteration 100,
resumes the saved state, and compares estimates, the full trace and diagnostics
with the uninterrupted fit. Revalidation simulations remain in the work count.

The fixed seed gives a population parameter near 4.36 and log-effect variance
near 0.17; the generated data's Gaussian reference is printed alongside them.
The example checks numerical agreement on this synthetic dataset. It does not
establish sampling-design performance or uncertainty coverage. Iteration counts
can vary across numerical environments. Read the reported status before using
estimates; a limit or failed mixing screen leaves them diagnostic. The returned
objective is a complete-data surrogate, so it cannot supply likelihood ratios.

Use variance, rather than standard deviation, in `omega`. Observation values
and additive SD use the output's units, here `1`; observation `row` indexes the
requested time grid. Restart with the same inputs, controls, code, native build
and numerical environment. Changed inputs or older algorithm checkpoints are
rejected; start a new analysis in a new directory.

## Operations

- `CompiledModel.fit_data(run, observations, parameters, error, ...)`: bounded
  deterministic likelihood fitting with additive, proportional, combined
  normal error, lognormal error, M3 left censoring, fitted SDs, and request-time
  event controls. Combined normal errors add independent variances.
- `CompiledModel.fit_population(subjects, parameters, omega, error, ...)`:
  native-simulation SAEM for 1–8 positive lognormal random effects (one per
  selected fitted parameter), diagonal/full covariance, up to eight bounded
  exponential subject-constant covariate coefficients, and fixed/fitted
  additive/proportional/combined normal or lognormal residual error. M3 rows
  use their probability for conditional sampling and conditional latent draws
  for the residual-error M step. Each subject retains its native regimen.
- `fit_diagnostics(fit)`: plotting rows for observed versus population/individual
  predictions, conditional standardized residuals, traces, eta variance
  shrinkage, acceptance, lag-one correlations and boundary warnings. Censored
  rows retain null residuals. These residuals are not CWRES or NPDE.
- `CompiledModel.visual_predictive_check(...)`: seeded joint-IIV and residual
  simulation on the observed design, explicit/automatic time bins, one/two
  subject stratifiers, prediction correction, BLQ fractions, and replicate
  confidence intervals for quantiles. Supply `lloq` on every observation for
  complete BLQ-aware checks. Concentration quantiles use quantifiable rows in
  both real and simulated data, with BLQ fractions reported separately.
  `workers=1` is the default; 1..8 bounded workers preserve ordered random
  streams and scientific results. Every failed replicate remains in the
  denominator, and any failure suppresses confidence intervals. See the
  [parallel predictive-check tutorial](tutorials/parallel-predictive-checks.md).
- `CompiledModel.profile_likelihood(...)`: fix each grid value and refit all
  structural/error nuisance parameters. The likelihood-ratio interval uses
  a chi-square approximation and grid interpolation; unresolved endpoints are
  null rather than substituted search bounds.
- `CompiledModel.bootstrap_population(...)`: resample complete subjects,
  including event histories and covariates; give each replicate an independent
  recorded seed; retain failed, nonconverged and boundary replicates. Percentile
  intervals for structural effects, Omega, covariates and residual errors
  require the declared minimum fraction of successful fits.
- `exposure_summary(result, output, threshold=...)`: sampled-grid AUC, Cmax,
  Tmax, Cmin, endpoint value and time above a threshold. Integrates each
  piecewise-linear segment and respects pre/post dose jumps. No terminal
  extrapolation or clinical NCA claim. Refine the observation grid and check
  convergence for each intended use. Endpoint is a trough only when sampled
  at the appropriate pre-dose boundary.
- `CompiledModel.compare_regimens(...)`: pair the same virtual subjects across
  up to 25 named requests; return curve/metric percentiles and paired exposure
  differences. Optional quantity-valued `parameter_draws` propagate parameter
  uncertainty separately from IIV. No measurement error is added to latent
  concentration trajectories.
- `CompiledModel.event_sensitivities(...)`: run the complete native regimen at
  two central-difference step sizes, including parameter-dependent model lags
  and explicit request event-time controls. Reject changing result grids or
  unresolved/discontinuous derivatives. Numerical derivatives are not symbolic
  moving-event or saltation derivatives.
- `write_analysis_report(path, ...)`: standalone, offline HTML/SVG views of
  estimates, diagnostic plots, VPCs, profiles and regimen comparisons.

## Inputs and work limits

Parameter specifications contain numeric `initial`, `lower`, `upper`, and a
native unit string. Values are expressed in that unit; the native binder checks
model units. Observation records contain `row`, `output`, and a quantity-valued
`value`. The row selects the exact native result grid and side. Observation
units must equal the output units; conversion must be explicit before fitting.
A censored row adds `censoring: "left"` and uses its quantification limit as
`value`; VPC records may supply a quantity-valued `lloq` on uncensored rows too.

Each population subject supplies `id`, `run`, `observations`, and optional
numeric, subject-constant `covariates`. Covariate specifications contain
`parameter`, `covariate`, `reference`, `initial`, `lower`, `upper` and implement
`theta_subject = theta_reference * exp(sum(beta*(x-reference)) + eta)`.
Coefficients on time-varying inputs, alternative covariate forms, fixed-effect
parameters without a random effect, interoccasion effects, mixture/count/
survival likelihoods, and priors are not included in this population contract.
Native step-covariate inputs may still occur in each subject's run.

Normal error supplies numeric SDs in `additive` (observation units) and
`proportional` (dimensionless), or a fitted `{initial, lower, upper}` record.
`model: "lognormal"` uses zero additive SD and proportional SD on log output.
One analysis uses one observed output and one shared residual-error contract.

A request-time event parameter adds a `path`, for example
`["regimen", "administrations", 0, "lag"]`. Supported paths are administration
time, lag, infusion duration and repeat interval. Selected model parameters
without a path are rebound normally, including model-authored lag parameters.
Central sensitivities require interior bounds and fixed observation times away
from moving discontinuities.

`max_simulations` is an aggregate budget, including all native evaluations;
bootstrap/profile workflows share it across subfits. Every native run also
retains its own solver/event/output budget. Deterministic and population fits
report evaluation limits explicitly. VPC, comparisons and derivative operations
return no partial scientific output after exhaustion. Population expectation
and VPC storage are bounded to two million observation-draw cells.

## Scientific scope

Python SAEM compares parameter history over at least 50 post-burn samples
spanning one unit of cumulative stochastic-approximation gain. Its mixing
and drift screen is not proof of population optimality or design-specific
recovery. A limit or unresolved screen keeps a nonconverged result diagnostic.
Population checkpoints bind this convergence rule; a checkpoint from a
different rule cannot resume the current algorithm.
The first half of burn-in equilibrates individual chains under the initial
population parameters before population sufficient-statistic updates begin.

The general SAEM fitter returns a **complete-data surrogate**, not a marginal
objective or a likelihood-ratio comparison statistic. Convergence is a bounded
stationarity/mixing screen; it is not proof of a global optimum, accurate
variance recovery or calibrated uncertainty. Singular covariance fails; no
ridge is silently substituted. Bounds, residual/covariate/variance boundaries,
seeds, traces, exact native execution identity and a normalized analysis hash
are retained in results. Python/SciPy arithmetic is distinct from the existing
Rust SAEM implementation; deterministic seeds are reproducible within the
recorded environment, not a cross-platform bitwise guarantee.

Tests compare the synthetic compartment examples with an analytic oral solution,
independent matrix exponential and independent nonlinear integrator. Generated
recovery observations remain explicitly generated numerical fixtures. They
check parameter recovery, combined/M3 likelihoods, three-effect full covariance,
joint covariate/residual inference, repeated-seed determinism, event derivatives,
bootstrap failure accounting and successful intervals, paired dose scaling,
units and budgets. They do not establish broad sampling-design performance or
bootstrap/profile interval coverage. Product/image qualification and a
production capability release are separate work.

External benchmark results and source-derived examples are maintained in [QSPQuest validation](https://github.com/unibio-intelligence/qspquest-validation). The engine includes an independently authored synthetic OMEX tutorial.

## Friendly interfaces and long analyses

`bolus`, `infusion`, `quantity` and `run_request` construct explicit native
requests. New `CompiledModel` results are dict-compatible `AnalysisResult`
objects with `rows`, `to_frame`, `plot` and `summary` helpers. Dataset adapters
can call `to_subjects` or `fit` with an explicitly chosen residual contract.
`scientific_status` and `QSPQuestError.explain` state interpretation limits.

Population fitting, VPCs and subject bootstraps accept `control=AnalysisControl`
for progress, cooperative cancellation and atomic JSON restart checkpoints.
Inputs, code, model, native build and numerical environment must match on
restart. Revalidation calls count toward the work budget. No partial scientific
result is returned on cancellation. See [controls](site/controls.md) and the
[synthetic OMEX tutorial](site/interchange.md).

VPC workers may change on resume. Running work drains before a cancellation
checkpoint is sealed; charged calls from unfinished replicates remain in its
aggregate budget. Their random draws are replayed from the last complete
ordered prefix. Allocate enough budget for revalidation and discarded work.

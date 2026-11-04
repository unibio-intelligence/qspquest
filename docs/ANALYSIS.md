# Native-backed analysis workflows

Install `qspquest[analysis]` from this checkout. This optional NumPy/SciPy layer
adds analysis workflows that run every simulation through `CompiledModel.run`.
They are Python-only; the CLI, R and WASM bindings do not offer them.

## Typed requests and result helpers

Python 3.10 or later can use `RunRequest`, `Quantity`, `ParameterSpec`,
`AnalysisObservation` and `AnalysisSubject` from `qspquest`. These are
`TypedDict` types, so values are plain dictionaries that serialize with
`json.dumps`. Request and result types are in `qspquest.types` (generated from
the JSON schemas); analysis records are in `qspquest.analysis_types`.

`CompiledModel.run`, the analysis methods on `CompiledModel`,
`CompiledSensitivities.run` and the dataset's `fit` method return
`AnalysisResult`. Call `.summary()` or `.rows()` without extra dependencies.
`.to_frame()` needs the notebook extra, and `.plot()` needs Matplotlib.
Standalone analysis functions still return plain dictionaries.

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
prints a summary, and reports six samples that match the analytical solution.
It is a synthetic check, not a clinical model. To print the optional DataFrame:

```sh
python -m pip install 'qspquest[notebook]'
python docs/examples/python_typed.py --frame
```

The example calls both `.summary()` and `.to_frame()` with no type ignores.
With a Python type checker, annotate requests explicitly or use the typed
helpers `quantity`, `bolus`, `infusion` and `run_request`. For example,
`Quantity(value=6.0, unti="mg")` has a misspelled key, and
`run_request(end=12.0, times=[0.0], solver_callbacks="many")` supplies a string
where an integer is required; a type checker flags both. Analysis options are
typed keyword arguments, such as integer `max_simulations`, boolean
`prediction_corrected`, and literal solver or optimizer names.

Plain JSON strings and unannotated dictionaries also work; they are checked
when the request runs. Type checking does not cover units, finite values,
parameter bounds or budgets; QSPQuest checks those at run time. Frame and plot
results are typed `Any` so the core package does not depend on pandas or
Matplotlib.

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
restart demo needs a new directory. It cancels at iteration 100, resumes from
the saved state, and checks that the result matches an uninterrupted fit.

The fixed seed gives a population parameter near 4.36 and log-effect variance
near 0.17; the generated data's Gaussian reference is printed alongside them.
This is a numerical check on synthetic data only. Iteration counts can vary
between machines. Check the reported status before using the estimates: if a
limit was hit or the mixing check failed, treat them as diagnostic only. The
returned objective is a complete-data surrogate, not a likelihood, so do not
use it for likelihood-ratio tests.

Use variance, rather than standard deviation, in `omega`. Observation values
and additive SD use the output's units, here `1`; observation `row` indexes the
requested time grid. Resume only with the same inputs, controls, code, build and
numerical environment; otherwise the checkpoint is rejected and you should
start a new analysis in a new directory.

## Run an individual likelihood profile

Use Python 3.10 or later with the matching released wheel and its `analysis`
extra installed, as above. Run this complete example in that environment:

```sh
python - <<'PY'
import json
import math
import numpy as np
from qspquest import CompiledModel

model = CompiledModel("""model gaussian_profile
time_unit = h
parameters
 theta = 2.718281828459045 [1] estimated
 decay = 0 [1/h] fixed
states
 anchor = 0 [1]
dynamics
 d/dt(anchor) = -decay * anchor
outputs
 z = log(theta) [1]
end
""")
times = np.linspace(0, 2, 24)
y = 1.4 + np.random.default_rng(81).normal(0, 0.25, len(times))
run = {
    "schema": "qspquest.run/v0.1",
    "solver": "diffsol_bdf",
    "budgets": {"events": 100, "output_values": 1000, "solver_callbacks": 10000},
    "regimen": {
        "end": {"value": 2, "unit": "h"},
        "administrations": [], "samples": [],
        "observations": [
            {"time": {"value": float(t), "unit": "h"}, "side": "post"}
            for t in times
        ],
        "rtol": 1e-10, "atol": 1e-12,
    },
}
rows = [
    {"row": i, "output": "z", "value": {"value": float(v), "unit": "1"}}
    for i, v in enumerate(y)
]
result = model.profile_likelihood(
    run, rows,
    {"theta": {"initial": math.exp(1), "lower": math.exp(-0.2),
               "upper": math.exp(3), "unit": "1"}},
    {"additive": {"initial": 0.2, "lower": 0.05, "upper": 0.8}},
    "theta", np.exp(np.linspace(0.8, 2, 161)).tolist(),
    max_iterations=300, max_simulations=20000,
)
assert result["status"] == "complete"
assert abs(math.log(result["estimate"]) - float(y.mean())) < 1e-5
assert all(endpoint is not None for endpoint in result["interval"])
print(json.dumps({key: result[key] for key in ["status", "estimate", "interval"]}))
PY
```

The output reports a complete profile, a positive estimate near 4, and two
finite interval endpoints. The script checks the estimate against the mean of
the generated data. The 95% interval uses a chi-square likelihood-ratio
approximation with grid interpolation; it does not guarantee coverage.

The fit runs in two stages: a bounded first fit, then Powell refinement. The
result keeps both as `initial_fit` and `unrestricted_fit`; together they stay
within `max_iterations`. Each nuisance refit also uses Powell. All simulations
count toward `max_simulations`. `fit_data(method="Powell")` applies `tolerance`
to both function and parameter stopping criteria.

Check the profile status and every grid-point status. A failed nuisance fit
makes the profile incomplete even when it lies outside the interval. An
endpoint the grid cannot resolve stays null; widen the bounds or grid only if
your analysis justifies it. Never read a bound as a confidence endpoint.


## Operations

- `CompiledModel.fit_data(run, observations, parameters, error, ...)`: bounded
  deterministic likelihood fitting with additive, proportional, combined
  normal error, lognormal error, M3 left censoring, fitted SDs, and request-time
  event controls. Combined normal errors add independent variances.
- `CompiledModel.fit_population(subjects, parameters, omega, error, ...)`:
  SAEM for 1–8 positive lognormal random effects (one per fitted parameter),
  diagonal/full covariance, up to eight bounded exponential subject-constant
  covariate coefficients, and fixed/fitted additive/proportional/combined
  normal or lognormal residual error. M3 rows use their probability for
  conditional sampling and conditional latent draws for the residual-error M
  step. Each subject keeps its own regimen.
- `fit_diagnostics(fit)`: plotting rows for observed versus population/individual
  predictions, conditional standardized residuals, traces, eta variance
  shrinkage, acceptance, lag-one correlations and boundary warnings. Censored
  rows have null residuals. These residuals are not CWRES or NPDE.
- `CompiledModel.visual_predictive_check(...)`: seeded joint-IIV and residual
  simulation on the observed design, explicit/automatic time bins, one/two
  subject stratifiers, prediction correction, BLQ fractions, and replicate
  confidence intervals for quantiles. Supply `lloq` on every observation for
  complete BLQ-aware checks. Concentration quantiles use quantifiable rows in
  both real and simulated data, with BLQ fractions reported separately.
  `workers=1` is the default; any count from 1..8 gives the same results.
  Failed replicates still count in the denominator, and any failure
  suppresses confidence intervals. See the
  [parallel predictive-check tutorial](tutorials/parallel-predictive-checks.md).
- `CompiledModel.profile_likelihood(...)`: fix each grid value and refit all
  structural/error nuisance parameters. The likelihood-ratio interval uses
  a chi-square approximation and grid interpolation; unresolved endpoints are
  null rather than substituted search bounds.
- `CompiledModel.bootstrap_population(...)`: resample complete subjects,
  including event histories and covariates; give each replicate an independent
  recorded seed; keep failed, nonconverged and boundary replicates in the
  result. Percentile intervals for structural effects, Omega, covariates and
  residual errors require a minimum fraction of successful fits.
- `exposure_summary(result, output, threshold=...)`: sampled-grid AUC, Cmax,
  Tmax, Cmin, endpoint value and time above a threshold. Integrates each
  piecewise-linear segment and respects pre/post dose jumps. There is no
  terminal extrapolation, and this is not a clinical NCA. Refine the observation grid and check
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
unit string. Values are in that unit; QSPQuest checks it against the model.
Observation records contain `row`, `output`, and a quantity-valued `value`.
The row selects a point (time and pre/post side) on the result grid. Observation
units must equal the output units; conversion must be explicit before fitting.
A censored row adds `censoring: "left"` and uses its quantification limit as
`value`; VPC records may supply a quantity-valued `lloq` on uncensored rows too.

Each population subject supplies `id`, `run`, `observations`, and optional
numeric, subject-constant `covariates`. Covariate specifications contain
`parameter`, `covariate`, `reference`, `initial`, `lower`, `upper` and implement
`theta_subject = theta_reference * exp(sum(beta*(x-reference)) + eta)`.
Coefficients on time-varying inputs, alternative covariate forms, fixed-effect
parameters without a random effect, interoccasion effects, mixture/count/
survival likelihoods, and priors are not supported. Each subject's run may
still contain step-covariate inputs.

Normal error supplies numeric SDs in `additive` (observation units) and
`proportional` (dimensionless), or a fitted `{initial, lower, upper}` record.
`model: "lognormal"` uses zero additive SD and proportional SD on log output.
Each analysis fits one observed output with one residual-error model.

A request-time event parameter adds a `path`, for example
`["regimen", "administrations", 0, "lag"]`. Supported paths are administration
time, lag, infusion duration and repeat interval. Model parameters without a
path, including lag parameters defined in the model, are set as usual.
Central sensitivities require interior bounds and fixed observation times away
from moving discontinuities.

`max_simulations` caps the total number of simulations; bootstrap and profile
workflows share it across their subfits. Each simulation also has its own
solver, event and output budgets. Deterministic and population fits report
when they hit a limit. VPC, comparison and derivative operations return no
partial results once the budget runs out. Population and VPC storage is capped
at two million observation-draw cells.

## Scientific scope

Python SAEM checks convergence by comparing parameter history over at least 50
post-burn-in samples spanning one unit of cumulative stochastic-approximation
gain. The first half of burn-in lets individual chains settle under the initial
population parameters before population updates begin. Checkpoints saved under
a different convergence rule cannot be resumed.

The fitter returns a **complete-data surrogate**, not a marginal likelihood, so
it cannot be used for likelihood-ratio comparisons. Passing the convergence
check does not prove a global optimum, accurate variance recovery or calibrated
uncertainty; if a limit is hit or the check fails, treat the result as
diagnostic. A singular covariance fails rather than being silently regularized.
Results record bounds, boundary hits, seeds, traces, the native run's identity
record and an analysis hash. This Python SAEM is separate from the Rust SAEM
implementation. Seeded runs repeat exactly on the same environment, but not
necessarily bit-for-bit across platforms.

Tests compare the synthetic compartment examples with an analytic oral solution,
independent matrix exponential and independent nonlinear integrator. Recovery
tests use generated data and check parameter recovery, combined/M3
likelihoods, three-effect full covariance, joint covariate/residual inference,
repeated-seed determinism, event derivatives, bootstrap failure accounting and
successful intervals, paired dose scaling, units and budgets. They do not show
performance for other study designs or bootstrap/profile interval coverage.

External benchmark results and source-derived examples are maintained in [QSPQuest validation](https://github.com/unibio-intelligence/qspquest-validation). The engine includes an independently authored synthetic OMEX tutorial.

## Friendly interfaces and long analyses

`bolus`, `infusion`, `quantity` and `run_request` build run requests.
`CompiledModel` results are dict-compatible `AnalysisResult` objects with
`rows`, `to_frame`, `plot` and `summary` helpers. Dataset adapters can call
`to_subjects` or `fit` with a residual-error model you choose.
`scientific_status` and `QSPQuestError.explain` say what a result or error
does and does not tell you.

Population fitting, VPCs and subject bootstraps accept `control=AnalysisControl`
for progress, cancellation and JSON restart checkpoints. Restart needs the same
inputs, code, model, build and numerical environment. Simulations rerun to
check a checkpoint count toward the budget. Cancelling returns no partial
results. See [controls](site/controls.md) and the
[synthetic OMEX tutorial](site/interchange.md).

You can change the VPC worker count when resuming. On cancellation, running
work finishes before the checkpoint is saved; simulations from unfinished
replicates still count toward the budget, and those replicates rerun with the
same random draws on resume. Leave budget headroom for this extra work.

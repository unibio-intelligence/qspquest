# Likelihood profiles, bootstrap and predictive checks

The analysis workflows run in the native QSPQuest engine. Every simulation uses
the same native execution as `CompiledModel.run`, and the optimization,
sampling, quantile and linear-algebra steps are part of the engine as well.
Python, R, the command-line interface and the browser engine call one shared
implementation, so a request gives the same result from each of them. The
base installation is sufficient: no NumPy, SciPy or other numerical package
is required. NumPy arrays and pandas objects are accepted as inputs where
lists are expected.

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

## Population analysis in Python

The [synthetic population example](examples/python_population.py) generates 24
subjects with six dimensionless observations each, fits a positive population
parameter and its log-effect variance, and checks an independent Gaussian
reference. It embeds its model and observations. Python 3.10 or later and the
released wheel are the only prerequisites.

From the matching release source directory, replace `WHEEL` with the full path
to your wheel:

```sh
python3 -m venv .venv-analysis
. .venv-analysis/bin/activate
python -m pip install 'WHEEL'
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

Use variance, rather than standard deviation (SD), in `omega`. Observation values
and additive SD use the output's units, here `1`; observation `row` indexes the
requested time grid. Resume only with the same inputs, controls, code, build and
numerical environment; otherwise the checkpoint is rejected and you should
start a new analysis in a new directory.

## Likelihood profile for an individual fit

Use Python 3.10 or later with the matching released wheel installed, as above.
Run this complete example in that environment:

```sh
python - <<'PY'
import json
import math
import random
import statistics
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
times = [2 * i / 23 for i in range(24)]
generator = random.Random(81)
y = [1.4 + generator.gauss(0, 0.25) for _ in times]
run = {
    "schema": "qspquest.run/v0.1",
    "solver": "diffsol_bdf",
    "budgets": {"events": 100, "output_values": 1000, "solver_callbacks": 10000},
    "regimen": {
        "end": {"value": 2, "unit": "h"},
        "administrations": [], "samples": [],
        "observations": [
            {"time": {"value": t, "unit": "h"}, "side": "post"}
            for t in times
        ],
        "rtol": 1e-10, "atol": 1e-12,
    },
}
rows = [
    {"row": i, "output": "z", "value": {"value": v, "unit": "1"}}
    for i, v in enumerate(y)
]
result = model.profile_likelihood(
    run, rows,
    {"theta": {"initial": math.exp(1), "lower": math.exp(-0.2),
               "upper": math.exp(3), "unit": "1"}},
    {"additive": {"initial": 0.2, "lower": 0.05, "upper": 0.8}},
    "theta", [math.exp(0.8 + 1.2 * i / 160) for i in range(161)],
    max_iterations=300, max_simulations=20000,
)
assert result["status"] == "complete"
assert abs(math.log(result["estimate"]) - statistics.fmean(y)) < 1e-5
assert all(endpoint is not None for endpoint in result["interval"])
print(json.dumps({key: result[key] for key in ["status", "estimate", "interval"]}))
PY
```

The output reports a complete profile, a positive estimate near 4, and two
finite interval endpoints. The script checks the estimate against the mean of
the generated data. The 95% interval uses a chi-square likelihood-ratio
approximation with grid interpolation [1, 2]; it does not guarantee coverage.

The fit runs in two stages: a bounded gradient fit, then derivative-free
refinement by compass pattern search [3]. The result keeps both as `initial_fit` and `unrestricted_fit`;
together they stay within `max_iterations`. Each nuisance refit is also
derivative-free, and every grid point starts from the unrestricted optimum.
All simulations count toward `max_simulations`.

Check the profile status and every grid-point status. A failed nuisance fit
makes the profile incomplete even when it lies outside the interval. An
endpoint the grid cannot resolve stays null; widen the bounds or grid only if
your analysis justifies it. Never read a bound as a confidence endpoint.


## Operations

- `CompiledModel.fit_data(run, observations, parameters, error, ...)`: bounded
  deterministic likelihood fitting with additive, proportional, combined
  normal error, lognormal error, M3 left censoring [4, 5], fitted SDs, and request-time
  event controls. Combined normal errors add independent variances. Search
  coordinates are the parameter bounds rescaled to [0, 1], with fitted SDs on
  the log scale. `method="gradient"` (default) uses projected BFGS with an
  Armijo line search and central-difference gradients; it stops when the
  largest projected gradient component or the relative objective reduction
  falls below `tolerance`. `method="derivative_free"` uses a compass pattern
  search that halves its step until the step falls below `tolerance`. The
  result's `method` names the algorithm, and the reported estimate is the
  best evaluated point. A `line_search_failed` status means no sufficient
  decrease was found; treat it like an iteration limit.
- `CompiledModel.fit_population(subjects, parameters, omega, error, ...)`:
  SAEM for 1–8 positive lognormal random effects (one per fitted parameter),
  diagonal/full covariance, up to eight bounded exponential subject-constant
  covariate coefficients, and fixed/fitted additive/proportional/combined
  normal or lognormal residual error. M3 rows use their probability for
  conditional sampling, and the residual-error M step uses the analytic
  second moment of the residual truncated at the quantification limit
  (`censored_residual_step` in the result). Each subject keeps its own
  regimen.
- `fit_diagnostics(fit)`: plotting rows for observed versus population/individual
  predictions, conditional standardized residuals, traces, eta variance
  shrinkage, acceptance, lag-one correlations and boundary warnings. Censored
  rows have null residuals. These residuals are not conditional weighted residuals (CWRES) or normalized prediction distribution errors (NPDE).
- `CompiledModel.visual_predictive_check(...)` [6, 7]: seeded joint interindividual variability (IIV) and residual
  simulation on the observed design, explicit/automatic time bins, one/two
  subject stratifiers, prediction correction [8], below-limit-of-quantification (BLQ) fractions, and replicate
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
- `CompiledModel.bootstrap_population(...)`: resample complete subjects [9],
  including event histories and covariates; give each replicate an independent
  recorded seed; keep failed, nonconverged and boundary replicates in the
  result. Percentile intervals [10] for structural effects, Omega, covariates and
  residual errors require a minimum fraction of successful fits.
- `exposure_summary(result, output, threshold=...)`: sampled-grid area under the curve (AUC), maximum concentration (Cmax),
  time of Cmax (Tmax), minimum concentration (Cmin), endpoint value and time above a threshold. Integrates each
  piecewise-linear segment and respects pre/post dose jumps. There is no
  terminal extrapolation, and this is not a clinical noncompartmental analysis (NCA). Refine the observation grid and check
  convergence for each intended use. Endpoint is a trough only when sampled
  at the appropriate pre-dose boundary.
- `noncompartmental_analysis(observations, doses, units=...)`: single-dose
  NCA of observed IV bolus or extravascular profiles, grouped by subject and
  occasion. Reports Cmax, Tmax, Clast, Tlast, AUClast (linear or
  linear-up/log-down), λz with its regression diagnostics, half-life, observed
  and predicted AUCinf, the extrapolated percentage, and CL and Vz (or CL/F and
  Vz/F). Pre-dose and BLQ rows follow stated rules or stop the analysis. See the
  [noncompartmental analysis tutorial](tutorials/noncompartmental-analysis.md).
- `CompiledModel.compare_regimens(...)`: pair the same virtual subjects across
  up to 25 named requests; return curve/metric percentiles and paired exposure
  differences. Optional quantity-valued `parameter_draws` propagate parameter
  uncertainty separately from IIV. No measurement error is added to latent
  concentration trajectories. Without `omega`, `random_effects` and
  `parameter_draws` the comparison is deterministic: one subject, no random
  draws, median percentiles only and `variability: "none"`; `parameters` is
  then optional, and `subjects` above one or a `seed` is refused.
- `CompiledModel.event_sensitivities(...)`: run the complete native regimen at
  two central-difference step sizes, including parameter-dependent model lags
  and explicit request event-time controls. Reject changing result grids or
  unresolved/discontinuous derivatives. Numerical derivatives are not symbolic
  moving-event or saltation derivatives.
- `write_analysis_report(path, ...)`: standalone, offline HTML/SVG views of
  estimates, diagnostic plots, visual predictive checks (VPCs), profiles and regimen comparisons. A dict
  of comparisons keyed by model overlays the models; `runs=` overlays labelled
  runs or saved tables.
- `read_table(path)`: read a saved CSV table with its `.manifest.json` (MCP
  `save_csv`, CLI or Python first run), with units from the manifest.
- `qspquest.plot` and `to_frame(result, table)`: journal-sized figures and
  long tables with unit columns for every result above. They need the `plot`
  extra. See [publication figures](tutorials/publication-figures.md).

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

The SAEM engine checks convergence by comparing parameter history over at least 50
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
record and an analysis hash. `fit_population` and the `saem` fit request
described in [SAEM](SAEM.md) run the same SAEM engine. The fit request adds a
correlation limit and a lag-one mixing condition, and for one and two effects
refines a Laplace-approximated marginal objective; analysis results report the
complete-data surrogate. With two or more effects each sweep adds a joint
random-walk move with the subject's adapted chain covariance, and proposal
scales can shrink to one thousandth of the effect SD for precise data.

Tests compare the synthetic compartment examples with an analytic oral solution,
independent matrix exponential and independent nonlinear integrator. Recovery
tests use generated data and check parameter recovery, combined/M3
likelihoods, three-effect full covariance, joint covariate/residual inference,
repeated-seed determinism, event derivatives, bootstrap failure accounting and
successful intervals, paired dose scaling, units and budgets. They do not show
performance for other study designs or bootstrap/profile interval coverage.

External benchmark results and source-derived examples are maintained in [QSPQuest validation](https://github.com/unibio-intelligence/qspquest-validation). The engine includes an independently authored synthetic OMEX tutorial.

## Random streams and reproducibility

Stochastic analyses (population fits, VPCs, bootstraps and regimen
comparisons) draw from SplitMix64 streams with open-interval uniforms and
Box–Muller normals. Each independent unit of work, such as a VPC replicate, a
bootstrap replicate or a virtual subject, has its own substream. Its state is
the first 64 bits of the SHA-256 digest of the canonical JSON record
`{"analysis": name, "seed": seed, "unit": index}`, and the record's digest is
reported as the unit's `stream_id`. Results therefore do not depend on the
worker count or on the order in which work finishes, and adding virtual
subjects does not change the draws of existing ones. The result field
`random_stream` names this specification.

With the same seed, inputs and engine build, a result repeats exactly on the
same platform when the simulation budget is not exhausted. ODE solutions can
differ in the last bits across processor architectures, and an accepted or
rejected sampler move then changes, so agreement across platforms is
statistical rather than exact.

Results are versioned `v0.2`. Analyses run with the earlier Python
implementation used a different random generator: rerunning them with the same
seed gives results that differ within Monte Carlo error, and their saved
records remain valid descriptions of those runs. Checkpoints written by the
earlier implementation cannot be resumed; start a new analysis. Deterministic
analyses (exposure summaries, fit diagnostics and event sensitivities) match
earlier results to round-off, and deterministic fits agree within the
optimizer tolerance at well-conditioned optima. Their iteration and
simulation counts differ.

## R, the command line and the browser

R provides the same analyses, under the same names, as functions of a
compiled model: `fit_data`, `fit_population`, `fit_diagnostics`,
`visual_predictive_check`, `profile_likelihood`, `bootstrap_population`,
`exposure_summary`, `compare_regimens` and `event_sensitivities`. Fitting
specifications are named lists, in the order that defines `omega`. Data-frame
fits use `individual_fit_data_requests` and `fit_individual_data`. Checkpoints and
progress callbacks are available from Python only.

The command-line interface runs a `qspquest.analysis/v0.2` request:

```json
{"schema": "qspquest.analysis/v0.2", "operation": "fit_data",
 "arguments": {"run": {}, "observations": [], "parameters": [], "error": {}}}
```

```sh
qspquest analyze MODEL.qsp request.json > result.json
```

`exposure_summary` and `fit_diagnostics` operate on an existing result and
need no model: `qspquest analyze request.json`. In native requests, parameters
are arrays of named records, for example
`[{"name": "cl", "initial": 2, "lower": 0.1, "upper": 5, "unit": "L/h"}]`,
and regimens for `compare_regimens` are arrays of `{"label", "run"}` records;
the first is the reference. The browser engine exposes the same request as
`CompiledSimulation.analyze(json)` and `analyze_record(json)`; it runs
single-threaded and ignores `workers`.

MCP servers built with the `analysis` feature run the same request through the
`analyze` tool, after a `plan_run` preparation that runs no simulation, and
the paired regimen comparison through the `population` block of
`compare_regimens`. Hosted servers require `max_simulations` (at most 500); see
the [MCP guide](../bindings/mcp/README.md#native-analyses).

## Convenience functions and long-running analyses

`bolus`, `infusion`, `quantity` and `run_request` build run requests.
`CompiledModel` results are dict-compatible `AnalysisResult` objects with
`rows`, `to_frame`, `plot` and `summary` helpers. Dataset adapters can call
`to_subjects` or `fit` with a residual-error model you choose.
`scientific_status` and `QSPQuestError.explain` describe what a result or error
does and does not establish.

Population fitting, VPCs and subject bootstraps accept `control=AnalysisControl`
for progress, cancellation and JSON restart checkpoints. Restart needs the same
inputs, code, model, build and numerical environment. Simulations rerun to
check a checkpoint count toward the budget. Canceling returns no partial
results. See [controls](site/controls.md) and the
[synthetic OMEX tutorial](site/interchange.md).

You can change the VPC worker count when resuming. On cancellation, running
work finishes before the checkpoint is saved; simulations from unfinished
replicates still count toward the budget, and those replicates rerun with the
same random draws on resume. Allow sufficient budget for this additional work.

## References

1. Venzon DJ, Moolgavkar SH. A method for computing profile-likelihood-based
   confidence intervals. *Appl Stat*. 1988;37(1):87–94.
   [doi:10.2307/2347496](https://doi.org/10.2307/2347496)
2. Raue A, Kreutz C, Maiwald T, et al. Structural and practical identifiability
   analysis of partially observed dynamical models by exploiting the profile
   likelihood. *Bioinformatics*. 2009;25(15):1923–1929.
   [doi:10.1093/bioinformatics/btp358](https://doi.org/10.1093/bioinformatics/btp358)
3. Kolda TG, Lewis RM, Torczon V. Optimization by direct search: new
   perspectives on some classical and modern methods. *SIAM Rev*.
   2003;45(3):385–482.
   [doi:10.1137/S003614450242889](https://doi.org/10.1137/S003614450242889)
4. Beal SL. Ways to fit a PK model with some data below the quantification
   limit. *J Pharmacokinet Pharmacodyn*. 2001;28(5):481–504.
   [doi:10.1023/A:1012299115260](https://doi.org/10.1023/A:1012299115260)
5. Ahn JE, Karlsson MO, Dunne A, Ludden TM. Likelihood based approaches to
   handling data below the quantification limit using NONMEM VI. *J
   Pharmacokinet Pharmacodyn*. 2008;35(4):401–421.
   [doi:10.1007/s10928-008-9094-4](https://doi.org/10.1007/s10928-008-9094-4)
6. Holford N. The visual predictive check: superiority to standard diagnostic
   (Rorschach) plots. *PAGE*. 2005;14:Abstr 738.
7. Post TM, Freijer JI, Ploeger BA, Danhof M. Extensions to the visual
   predictive check to facilitate model performance evaluation. *J Pharmacokinet
   Pharmacodyn*. 2008;35(2):185–202.
   [doi:10.1007/s10928-007-9081-1](https://doi.org/10.1007/s10928-007-9081-1)
8. Bergstrand M, Hooker AC, Wallin JE, Karlsson MO. Prediction-corrected visual
   predictive checks for diagnosing nonlinear mixed-effects models. *AAPS J*.
   2011;13(2):143–151.
   [doi:10.1208/s12248-011-9255-z](https://doi.org/10.1208/s12248-011-9255-z)
9. Efron B. Bootstrap methods: another look at the jackknife. *Ann Stat*.
   1979;7(1):1–26.
   [doi:10.1214/aos/1176344552](https://doi.org/10.1214/aos/1176344552)
10. Efron B, Tibshirani RJ. *An Introduction to the Bootstrap*. Chapman &
    Hall/CRC; 1994.
    [doi:10.1201/9780429246593](https://doi.org/10.1201/9780429246593)

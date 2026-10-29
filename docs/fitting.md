# Parameter fitting

For count observations with measured exposure, use the native
[Gamma–Poisson fitting workflow](tutorials/gamma-poisson-counts.md).

For correlated binary outcomes within independent litters or clusters, use
[probit-normal binomial fitting](tutorials/probit-normal-binomial.md).

The [proportional-normal fitting workflow](tutorials/profiled-proportional-errors.md)
jointly estimates structural parameters and one proportional residual scale per
output through the CLI, Python, R or Rust interfaces.

QSPQuest 0.1.0 includes the following bounded fitting contracts. They use
unit-bearing observations and explicit simulation settings.

| Request kind | Supported scope |
| --- | --- |
| `individual` | Gaussian observation likelihood and optional independent normal priors; fitted parameters have finite bounds. |
| `profiled_proportional` | Uncensored normal observations with one estimated proportional SD per output, zero additive SD, and at least two observations per observed output. |
| `pooled` | Shared fitted parameters across subjects with separate regimens and observations. |
| Scalar fit | Derivative-free search for one bounded parameter with normal, uncensored observations and positive additive SD through `CompiledModel.fit_scalar()` or its scalar request. |
| `laplace` | Population fit with one lognormal random effect and a Laplace approximation. |
| `focei` | Population fit with one or two lognormal random effects and normal, uncensored observations; the two-effect correlated covariance is fixed. |
| `saem` | Population fit with one or two lognormal random effects within the supported request shapes. |

Population parameters follow `individual_parameter = fixed_effect * exp(eta)`.
The covariance matrix `omega` is in the order of `random_effects`. A bounded
common additive residual standard deviation can be estimated for one-effect
FOCEI and one- or two-effect SAEM within their supported request shapes.
The [two-effect walkthrough](SAEM.md#run-a-two-effect-synthetic-example) runs
fixed-covariance FOCEI and SAEM with fixed or estimated additive SD.
Consult the versioned types in
[`fit.rs`](../crates/qspquest-core/src/fit.rs) for exact fields and validation.

## Run an individual fit

The included synthetic request estimates clearance and volume:

```sh
cargo run --release --locked -p qspquest-cli -- fit \
  conformance/models/synthetic-one-compartment.json \
  conformance/requests/synthetic-fit.json > fit.json
```

In Python, after [installing from source](installation.md):

```python
import json
from pathlib import Path
import qspquest

source = Path("conformance/models/synthetic-one-compartment.json").read_text()
request = json.loads(Path("conformance/requests/synthetic-fit.json").read_text())
result = qspquest.CompiledSensitivities(source, ["cl", "v"]).fit(request)
print(result["fit"]["status"], result["fit"]["parameters"])
```

The request nests a `qspquest.sensitivity/v0.1` request and a
`qspquest.run/v0.1` simulation request. Observation `row` indexes that
simulation's ordered output grid; each row names an output, measured value,
and additive/proportional Gaussian error. The result's `fit` member contains
status, estimates, objective, evaluation counts, and a request hash.

Scalar fits validate parameter bounds, observation names and units, and the
starting run before searching. Native scalar preparation diagnostics identify the
relevant native request field in `expression`, such as `/lower`, `/max_iterations`
or `/observations/0/error/additive_sd`. These paths are relative to the complete
native scalar request, including when it is supplied as MCP `fit_request`.
More specific model/run expression context is retained. Inspect the declared
units, bounds and statistical contract before correcting and resubmitting; a
preparation refusal does not evaluate an objective or establish fitting convergence. With explicit `regimen.observations`, a fit
observation's `row` must index that list; invalid rows fail before simulation.
Without an explicit observation list, event timing can change the trajectory
grid during fitting, so row validity is checked at each trial. Simulation
result-storage limits are checked before integration for either grid.

Individual, pooled and population fits check all subjects' observation units,
starting schedules, row indexes and supported controls before the first
likelihood evaluation. Gaussian likelihoods accept normal or lognormal error
where the selected fitting contract supports them. Lognormal observations need
positive measured values, zero additive SD, and a positive dimensionless log SD
in `proportional_sd`. Scalar fitting and FOCEI reject lognormal observation
error. Predictions, residual variance and trial schedules still need validation
during fitting; passing the input checks does not establish convergence.

## Run a one-effect population fit

This example fits one lognormal subject effect to synthetic repeated
measurements of `log(theta)`. The fitted parameter, random effect and observations
are dimensionless; time labels are in hours. It demonstrates the fitting contract without a
drug-specific model. Use Python 3.10 or later and the Rust source-installation
prerequisites in [installation.md](installation.md), then run from the release
source directory.

Generate a complete model, 64 subjects with two measurements each, and an exact
Gaussian reference. The output directory must be new.

```sh
python - <<'PY'
import json
import math
from pathlib import Path
import random
import statistics

out = Path('population-laplace-demo')
out.mkdir()
def q(value, unit='1'):
    return {'value': value, 'unit': unit}

def save(name, value):
    with (out / name).open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')

(out / 'model.qsp').write_text('''model synthetic_random_intercept
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
''')
rng = random.Random(40211)
subjects, means = [], []
for _ in range(64):
    eta = rng.gauss(0, .4)
    values = [1.4 + eta + rng.gauss(0, .25) for _ in range(2)]
    means.append(statistics.mean(values))
    simulation = {
        'schema': 'qspquest.sensitivity/v0.1',
        'with_respect_to': ['theta'],
        'derivative_absolute_tolerances': {'theta': {'anchor': q(1e-12)}},
        'run': {'schema': 'qspquest.run/v0.1', 'solver': 'diffsol_bdf',
                'parameters': {},
                'budgets': {'solver_callbacks': 10000, 'events': 100, 'output_values': 1000},
                'regimen': {'start': q(0, 'h'), 'end': q(1, 'h'),
                            'observations': [{'time': q(t, 'h'), 'side': 'post'} for t in [0, 1]],
                            'samples': [], 'administrations': [], 'rtol': 1e-10, 'atol': 1e-12}}}
    subjects.append({'simulation': simulation, 'priors': [],
        'observations': [{'row': row, 'output': 'z', 'value': q(value),
            'error': {'model': 'normal', 'additive_sd': q(.25), 'proportional_sd': 0}}
            for row, value in enumerate(values)]})
save('request.json', {'schema': 'qspquest.fit/v0.1',
    'problem': {'kind': 'laplace', 'request': {
        'subjects': subjects, 'random_effects': [{'parameter': 'theta'}],
        'fixed_effects': [{'parameter': 'theta', 'lower': q(math.exp(-.2)), 'upper': q(math.exp(3))}],
        'omega': [[.04]], 'omega_diagonal_bounds': [[.001, .8]],
        'eta_bound': 5, 'max_evaluations': 100000, 'max_iterations': 300,
        'gradient_tolerance': .0005, 'seed': 20261011, 'uncertainty': False}}})
save('reference.json', {'theta': math.exp(statistics.mean(means)),
    'omega': max(.001, statistics.pvariance(means) - .25**2 / 2)})
PY
```

Run the bounded fit:

```sh
cargo run --release --locked -p qspquest-cli -- fit \
  population-laplace-demo/model.qsp \
  population-laplace-demo/request.json > population-laplace-demo/fit.json
```

Read the population result and compare it with the reference for these data:

```sh
python - <<'PY'
import json
from pathlib import Path

out = Path('population-laplace-demo')
result = json.loads((out / 'fit.json').read_text())['population']
reference = json.loads((out / 'reference.json').read_text())
print(result['status'], result['evaluations'])
print('theta', result['fixed_effects']['theta']['value'], 'reference', reference['theta'])
print('omega', result['omega'][0][0], 'reference', reference['omega'])
PY
```

Expect `converged`, roughly 45,000 evaluations, `theta` about 4.27 and `omega`
about 0.114. `theta` is the typical individual value; `omega` is the variance
of the dimensionless log effect. The generated population has log-effect SD
0.4 and observation SD 0.25. A finite sample can differ from those generating
values, so compare the fit with `reference.json` rather than expecting exact
recovery of the population truth.

Inspect the status before interpreting estimates. An `evaluation_limit` result
is diagnostic; inspect the bounds and consider a larger supported budget.
This Laplace contract uses one random effect and fixed residual error and does
not report uncertainty intervals. For two effects, use the
[two-effect walkthrough](SAEM.md#run-a-two-effect-synthetic-example).

## Population results and data

For `laplace`, `max_evaluations` bounds subject likelihood calculations.
`evaluations` reports the calculations performed; repeated samples may be
reused within the same fit. Budget exhaustion returns `evaluation_limit` with
the best completed population evaluation when available.

Population fit results use the `population` member. Inspect `status`,
`fixed_effects`, `omega`, subject effects, and `objective_kind` together;
objectives from different methods have different meanings and must not be
compared as interchangeable numbers. When available,
`fitted_observations` records row-aligned observed values and population and
individual predictions. Optional FOCEI uncertainty is local curvature from
the observed outer objective, not an interval calibration. SAEM's
`saem_diagnostics` reports proposal and stochastic-drift information; it is
not a convergence certificate.

Two-effect SAEM also checks measured post-burn mixing and admissible marginal
objective perturbations before reporting convergence. Its local covariance uses
bounded natural-coordinate steps and step-halving checks. Covariance evaluation
uses sensitivity scores and stricter integration error ceilings for nested
derivatives. An unresolved fit or
covariance can exhaust the same `max_evaluations` budget; inspect the returned
status and objective kind, and use `uncertainty: false` when only diagnostic
point estimates are needed. See [SAEM fitting](SAEM.md#uncertainty-and-limits).

For the one-effect FOCEI fit, `gradient_tolerance` controls the final search
step in the normalized parameter bounds. The conditional eta mode is solved
more tightly than this outer resolution, so coarse inner modes do not limit a
tight requested fit. This setting is a search resolution, not a certificate of
the final likelihood gradient. Tight ODE tolerances and sensitivity tolerances
are also necessary when comparing final parameter values at high precision.

Python's `population_fit_dataset()` converts an explicitly mapped tabular
record subset into a population request and returns source-row mappings.
Pass that object to `CompiledSensitivities.fit_population_data()`. Its API is
declared in the [Python type stub](../bindings/python/python/qspquest/__init__.pyi).
The [synthetic population tests](../crates/qspquest-modeling/tests/population_saem.rs)
show supported request shapes. Check bounds, residual-error assumptions, identifiability, and
scientific plausibility before using any fitted result.

### Clamps in constant sensitivity terms

A clamp can be used in native forward sensitivities when its complete subtree is independent of the selected parameters, states, infusion inputs and time. For example, a concentration readout can divide by `max(volume, quantity(1e-12, "L"))` when estimating clearance alone. Requesting a volume derivative for that clamp remains unsupported. State- or time-dependent nonsmooth crossings remain unqualified and are refused.

## Run a precise one-effect population fit

Purpose: estimate population clearance from a small synthetic IV cohort while
holding volume, eta variance, and residual SD fixed. Prerequisites: install the
Python package as described in [installation](installation.md), and run from
the repository root. All inputs below are included or generated with
Python's standard library. Times are hours, doses are mg, clearance is L/h,
volume is L, and concentrations are mg/L. The eta variance is dimensionless.

```sh
python - <<'PY'
import copy
import json
import math
from pathlib import Path
import qspquest

def q(value, unit):
    return {"value": value, "unit": unit}

model = json.loads(Path("conformance/models/synthetic-one-compartment.json").read_text())
model["parameters"][1]["default"] = q(4.5, "L")
simulation = json.loads(Path("conformance/requests/synthetic-sensitivity.json").read_text())
simulation["with_respect_to"] = ["cl"]
simulation["derivative_absolute_tolerances"].pop("v")
simulation["run"]["parameters"] = {"cl": q(0.5, "L/h")}
times = [0., 1., 2., 4.]
regimen = simulation["run"]["regimen"]
regimen["samples"] = []
regimen["observations"] = [{"time": q(t, "h"), "side": "post"} for t in times]
subjects = []
for i, eta in enumerate([-.45, -.32, -.21, -.12, .12, .21, .32, .45]):
    observed = [6/4.5 * math.exp(-.6 * math.exp(eta) * t/4.5)
                + (.015 if (i+j) % 2 == 0 else -.015)
                for j, t in enumerate(times)]
    subjects.append({"simulation": copy.deepcopy(simulation), "priors": [],
        "observations": [{"row": j, "output": "cp", "value": q(y, "mg/L"),
            "error": {"additive_sd": q(.03, "mg/L"), "proportional_sd": 0.}}
            for j, y in enumerate(observed)]})
request = {"schema": "qspquest.fit/v0.1", "problem": {"kind": "focei", "request": {
    "subjects": subjects,
    "fixed_effects": [{"parameter": "cl", "lower": q(.2, "L/h"), "upper": q(1., "L/h")}],
    "random_effects": [{"parameter": "cl"}], "omega": [[.1]],
    "omega_diagonal_bounds": [[.1, .1]], "eta_bound": 8.,
    "max_evaluations": 100000, "max_iterations": 1200,
    "gradient_tolerance": 1e-8, "seed": 17029}}}
compiled = qspquest.CompiledSensitivities(json.dumps(model), ["cl"])
result = compiled.fit(request)["population"]
clearance = result["fixed_effects"]["cl"]["value"]
print(result["status"], clearance, result["omega"])
assert result["status"] == "converged"
assert abs(clearance - .6011282254) < 1e-6
PY
```

Expected output is `converged`, clearance approximately `0.601128 L/h`, and
`[[0.1]]`. This estimate is for the FOCEI approximation and the supplied data;
it need not equal the data-generating clearance exactly. `row` indexes the
explicit post-dose grid. Including a pre-dose row accidentally changes which
measurement belongs to each prediction. An iteration or evaluation limit does
not establish convergence: increase the appropriate budget and inspect the
result. Keep the declared observation errors and parameter bounds fixed when
checking stability under tighter ODE and search tolerances.

For a delayed input with an exact zero-time limit, follow the complete
[inverse-Gaussian input and fraction fit](tutorials/inverse-gaussian-input.md).

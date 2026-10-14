# Scientific run and result envelopes

Draft versions: `qspquest.run/v0.1` and `qspquest.result/v0.1`. The authoritative
Rust records are in `qspquest-core::run`; they have no solver dependency.
The [run schema](schema/run.schema.json) and [result schema](schema/result.schema.json)
describe their structural contracts; see [schema limits](schema/README.md).
The complete [synthetic request](../conformance/requests/synthetic-one-compartment.json)
uses the [synthetic model](../conformance/models/synthetic-one-compartment.json).
It samples `cp(t) = 2 * exp(-0.1*t)` mg/L at 0, 1, 2, 4, 8 and 12 h after
a 6 mg bolus into a 3 L compartment with clearance 0.3 L/h. This is a numerical
example with synthetic inputs.

An installed MCP client can read these documents and example payloads through
the URIs in `qspquest://documentation`. Pass the model's JSON text as
`model.source` and the request object as `request` to `simulate`. Source
execution requires local stdio or a configured hosted sandbox; read
`qspquest://capabilities` for the active deployment. Preserve the returned
identity, units and event sides when reporting the samples.

## Static native planning

`CompiledDocument.plan_run(request)`, `plan_run_selected(request, outputs)` and
`plan_scan(request)` return typed preparation records without evaluating an ODE,
readout or objective. The [run plan](schema/run-plan.schema.json) and
[scan plan](schema/scan-plan.schema.json) schemas are closed versioned contracts.
The CLI command `qspquest plan MODEL REQUEST.json` accepts native run, scan,
sensitivity, fit and scalar-fit envelopes. Periodic planning is not exposed.

A run plan preserves the supplied request and reports the actual bound inputs,
initial states, tolerances, selected output names/units, expanded event schedule,
covariate binding phases, integration checkpoints and driver row schedule.
Named events retain driver storage order and explicit reset ordering. Event
amounts are delivered input quantities after bioavailability and before the
model's dose scale; resets use state storage units. Times and infusion durations
use `time_unit`. Requested observations preserve order, duplicates and sides;
`observation_rows` maps them to driver rows, including the shared smooth row
that can satisfy either side at a time without a discontinuity.
`selected_outputs` preserves an explicit selection versus the implicit full
set. Binding phases describe integration inputs; `readout_overrides` records
the actual pointwise predicate inputs for each affected observation before any
readout is evaluated. Derived covariates and individual parameters are reported
as derived inputs, not editable model inputs. Readout-override values are bounded
by the request's output budget; if this metadata alone exceeds the ceiling,
reduce the planned grid or predicates before requesting a complete plan.

Bound input and event units include an exact rational scale and dimension vector
(kg, m, s, mol, K, A, cd, opaque activity). Positive integer scales are decimal
strings so JSON readers do not round them. `declared` retains a declared unit's
spelling; inferred individual-parameter units use the exact representation with
no invented spelling. Opaque activity has the same assay/substance restrictions
as the ordinary [unit contract](units.md).

Workload counts describe prepared rows, states, outputs, events, binding phases
and numeric storage reservations. Budgets are ceilings, not predicted callback
counts, elapsed time or prices. A scan plan reports every point's actual equally
partitioned budget and merged explicit parameter map. It validates all points
before returning. No trajectory is included.

The `prospective_identity` is computed by execution's identity function. It does
not establish that a calculation occurred, that future domains/convergence will
pass, or that the model is scientifically qualified. `deferred_checks` states
these remaining limits. Execution validates its current inputs again. Retain
the original model, request, target and build when comparing identities.

### Plan and run the synthetic example

Use the release source, Rust 1.98.1 and Python 3.10 or later. Run from the source's
top-level directory. All model and request inputs below are shipped synthetic
fixtures; no additional repository is needed:

```sh
cargo build --release --locked -p qspquest-cli
./target/release/qspquest plan conformance/models/synthetic-one-compartment.json conformance/requests/synthetic-one-compartment.json > plan.json
./target/release/qspquest run conformance/models/synthetic-one-compartment.json conformance/requests/synthetic-one-compartment.json > result.json
python3 - <<'PY'
import json
from pathlib import Path
plan = json.loads(Path('plan.json').read_text())
result = json.loads(Path('result.json').read_text())
assert plan['prospective_identity'] == result['identity']
assert [p['time'] for p in plan['observations']] == result['times']
assert [p['side'] for p in plan['observations']] == result['sides']
assert plan['events'][0]['target_name'] == 'central'
assert plan['events'][0]['event']['amount'] == 6
assert plan['workload']['result_rows'] == 6
print('Matching identities; six requested post-dose observations, one 6 mg bolus.')
print('Still unassessed:', plan['deferred_checks'])
PY
./target/release/qspquest plan conformance/models/synthetic-one-compartment.json conformance/requests/synthetic-scan.json > scan-plan.json
```

The run plan describes clearance 0.3 L/h, volume 3 L, an initial central amount
of zero and a time-zero 6 mg bolus. Its six requested observations are post-dose
at 0, 1, 2, 4, 8 and 12 h. The driver also retains the time-zero pre-dose row.
The result contains `cp(t) = 2 * exp(-0.1*t)` mg/L. The scan plan contains the
three ordered clearance points described in the [scan tutorial](../docs/PARAMETER-SCANS.md).
Changing inputs or the model requires a new plan. Invalid units, unknown targets,
unsupported solver contracts and storage-budget failures return structured
errors; a valid plan can still fail later when its equations or readouts are
evaluated. These examples establish synthetic execution consistency, not
physical-model or clinical qualification.

### Sensitivities and Gaussian-objective preparation

`CompiledSensitivityDocument.plan_run(request)` prepares the augmented sensitivity
workflow. Its [sensitivity plan schema](schema/sensitivity-plan.schema.json)
reports the actual primal and parameter-major derivative states, exact units,
initial derivatives, absolute tolerances, covariate phases, lowerings and row
projection. Workload state/output counts include derivative coordinates/columns.
The same supported solvers, parameter order, static event constraints and budgets
apply as for `run`; unsupported moving predicates/history do not fall back to a
primal plan. The prospective identity matches the sensitivity result's identity.

With the same release source, Rust and Python prerequisites, run:

```sh
cargo build --release --locked -p qspquest-cli
./target/release/qspquest plan conformance/models/synthetic-one-compartment.json conformance/requests/synthetic-sensitivity.json > sensitivity-plan.json
./target/release/qspquest sensitivities conformance/models/synthetic-one-compartment.json conformance/requests/synthetic-sensitivity.json > sensitivity-result.json
python3 - <<'PY'
import json
from pathlib import Path
plan = json.loads(Path('sensitivity-plan.json').read_text())
result = json.loads(Path('sensitivity-result.json').read_text())
assert plan['prospective_identity'] == result['result']['identity']
assert [p['time'] for p in plan['observations']] == result['result']['times']
assert [p['side'] for p in plan['observations']] == result['result']['sides']
assert [s['parameter'] for s in plan['augmented_states']] == [None, 'cl', 'v']
assert plan['workload']['states'] == plan['workload']['outputs'] == 3
print('Matching sensitivity identity; one primal and two derivative coordinates.')
PY
```

The fixture uses fixed-time dosing and sensitivities to clearance and volume.
It retains the time-zero pre-dose row, so there are five output rows. Derivative
units are output-unit divided by parameter-unit; they are not concentrations.
Use `derivatives[*].parameter` and the exact unit record rather than inferring a
unit from an array position. Wrong derivative tolerance units and mismatched
parameter order are structured preflight errors. A valid plan does not establish
future trajectory/readout domains or numerical convergence.

`CompiledSensitivityDocument.plan_gaussian_objective(request)` uses the actual
observation/prior admission and its reduced sensitivity storage budget. The
[objective plan schema](schema/gaussian-objective-plan.schema.json) preserves the
input table and reports its converted values, error coefficients, censoring,
priors and observation/prior storage reservation. `prospective_identity` is the
full objective identity; `simulation.prospective_identity` belongs to the reduced-
budget simulation and therefore has a different request hash. No predictions,
likelihood or gradients are returned. Prediction-dependent likelihood checks are
deferred; static normal priors are checked at the bound parameters.

The shipped Rust example uses the complete synthetic individual fixture's
objective, without running its optimizer. Its default action prepares only;
`--execute` explicitly evaluates the objective at its supplied starting values:

```sh
cargo run --release --locked -p qspquest-modeling --example plan_gaussian_objective > objective-plan.json
cargo run --release --locked -p qspquest-modeling --example plan_gaussian_objective -- --execute > objective-result.json
python3 - <<'PY'
import json
from pathlib import Path
plan = json.loads(Path('objective-plan.json').read_text())
result = json.loads(Path('objective-result.json').read_text())
assert plan['prospective_identity'] == result['identity']
assert plan['prediction_dependent_checks_deferred'] is True
assert plan['reserved_objective_values'] > 0
assert 'negative_log_likelihood' not in plan
print('Matching objective identity; predictions and likelihood required execution.')
PY
```

Observation `row` indexes the ordered simulation result, including duplicate
requested observations and event sides. Value and additive SD convert to the
output's declared units. Normal proportional SD is dimensionless relative error;
lognormal proportional SD is dimensionless log error. A left-censored value is a
quantification limit. Invalid units, rows, error coefficients or priors fail
before integration. Planning an objective does not validate optimizer/population
controls, identify parameters, estimate uncertainty or qualify a fitting model.
These remain distinct fitting and scientific checks.

## Request

A request contains a schema, optional correlation `request_id`, explicit solver,
explicit budgets, parameter and initial-state overrides as quantities, and the unit-bearing
regimen. The regimen carries covariates, fixed changes, sampling sides,
administrations, resets and tolerance settings. Unknown fields are rejected.
The ID, when provided, is 1–128 bytes. Native and WASM execution share unsigned
32-bit budget limits for callback counts, output values and events.

The implemented ODE solvers are `diffsol_bdf`, `diffsol_tsit45`,
`diffsol_esdirk34`, `diffsol_tr_bdf2`, `diffsol_rosenbrock23`, and
`diffsol_rodas5p`. The last two use locally vendored Diffsol source and accept
identity-mass ODEs. Sensitivities and sensitivity-based fitting accept
`diffsol_bdf` (the default in helpers) or explicit `diffsol_rodas5p`. The selected
solver is used throughout objective evaluations, including population subjects.
Other solvers remain unsupported on this path; there is no solver fallback.
Unknown names fail decoding and do not select a fallback. The optional seed field reserves the stochastic contract,
but every non-null seed is currently rejected as unsupported. No stochastic
behavior or implicit random seed is implemented.

Set `"accuracy_control": "sampled"` on a primal numerical run to check convergence
at every requested state observation and interval endpoint. Each pass restarts
from identical interval data with both local tolerances divided by ten. The
finer result is returned only when successive trajectories change by at most
0.25 of `atol_i + rtol * max(abs(coarse_i), abs(fine_i))`; eight complete passes
are allowed. Dose events run once outside refinement. All passes consume the
same callback budget. Unresolved convergence or an exhausted budget returns a
structured error without a partial result. This is numerical convergence
checking, not a universal certificate of true global error, and can cost many
more steps. The analytic linear solver remains exact for its supported class.
Sensitivity, fitting and numerical cycle-iteration requests reject sampled
control until those contracts are qualified.

The omitted/default `"accuracy_control": "local"` uses ordinary local tolerances;
TR-BDF2 retains its existing six-pass, one-unit sampled check. Rodas5P retains
the published fourth-order continuous extension in either mode. Adaptive step
histories need not match another implementation to produce accurate samples.

`CompiledDocument.execute` returns a result only after the complete scientific
operation succeeds. Failure is the existing typed error with no partial result.
The native `execute_request` example accepts a model text/JSON file and a run
JSON file and emits the result JSON. It is an offline execution entry point,
also available through the implemented `qspquest run MODEL REQUEST.json`
command. With the release's model and request files, run:

```sh
qspquest run conformance/models/synthetic-one-compartment.json conformance/requests/synthetic-one-compartment.json
```

## Result

The native result contains schema, correlation ID, scientific identity, model
ID/description, time unit, explicit pre/post sides, times, and named output
columns with units. Display fields are separate from the identity record.

The browser worker accepts `{kind: "run", source, request}`. It transfers typed
numeric arrays and returns the same metadata and identity, with `names`,
`units` and `columns` as the columnar view. Browser sides are bytes (0 pre,
1 post); native JSON uses the names `pre`/`post`. The scientific path rejects
additional legacy protocol/regimen/override/callback fields outside the request
envelope to avoid conflicting settings.

Legacy conformance `kind: "text"` payloads remain available; they do not acquire
a scientific execution identity merely because the new envelope exists.

## Canonical hashes

Core canonicalization uses RFC 8785 JCS (`serde_jcs` 0.2.0), followed by SHA-256
(`sha2` 0.10.9). Hash strings are `sha256:` followed by 64 lowercase hex digits.
Tests cover canonical key ordering including UTF-16 order, numeric negative
zero normalization and a known SHA-256 vector. Integer JSON values outside the
exact interoperable range ±(2^53−1) are rejected; large identifiers should be
strings. Scientific binary64 values use the JCS number representation.

- `model_content_hash`: canonical serialized, validated model document. This
  includes its descriptions and declared units. Whitespace and formatter/JSON
  round trips do not change it; changing document content does. It is not a
  proof that different equations or unit spellings are mathematically equivalent.
- `bound_value_hash`: the complete numeric binding buffer, including defaults,
  converted covariates and derived individual parameters, plus the resolved initial
  state vector in declared state units and compiled order.
  Its interpretation is bound to the model content hash.
- `run_request_hash`: schema, regimen, solver, budgets and seed, plus explicit sampled accuracy control. Correlation IDs
  are excluded. Parameter and initial-state override spelling is excluded because the
  complete resolved bindings and initial vector are recorded by `bound_value_hash`. Covariate changes and their
  times remain in the regimen hash. Semantically equivalent but differently
  written regimens can have different hashes; no semantic equivalence is assumed.

## Compiler and backend provenance

Identity also records specification version, compiler package version, compiler
source SHA-256, rustc version, target, profile, encoded-rustflags SHA-256,
backend/version and algorithm. Native and WASM can share the scientific content
hashes while correctly recording different build targets.

The current workspace build script hashes ordered relative paths, lengths and
bytes for Rust core/engine/WASM sources, build script, workspace/crate manifests,
lockfile, Cargo configuration, and the complete vendored Diffsol source. This distinguishes development source changes
under the same package version. It is a source/build fingerprint, not a binary
signature, OCI attestation or proof of installed dependencies. Standalone crate
packaging and release attestation remain unfinished. The backend pins Diffsol
0.17.1 plus the local Rosenbrock methods; `backend_version` includes the exact
vendored-source SHA-256. Matching these fields identifies the numerical backend
inputs; it does not establish qualification for another model, method or target.
Required copyright and license notices accompany the distribution.

References: [RFC 8785](https://www.rfc-editor.org/rfc/rfc8785),
[serde_jcs](https://docs.rs/serde_jcs/0.2.0/serde_jcs/),
[SHA-2 implementation](https://docs.rs/sha2/0.10.9/sha2/).

## Initial-state overrides

Optional `initial_states` maps declared state names to quantities, for example
`{"central": {"value": 6, "unit": "mg"}}`. It is a run-level initial condition,
not a time-zero reset. Compatible units convert once into state storage units;
unknown names, incompatible units and non-finite values fail.

An explicit override replaces only that state's initial expression. Other initial
expressions evaluate from the current parameter/covariate binding. The overridden
expression still undergoes structural and unit compilation but is not evaluated
at binding. This allows a valid explicit initial condition even when that
expression would be undefined for the run's parameter values. Parameter bounds,
derived parameters and dose scales still undergo their ordinary validation.

Initial values precede time-zero pre-observations, covariate changes, resets and
doses. Covariate changes carry the trajectory forward without reinitialising it.
Declared invariants are checked before any event can hide an invalid initial
value. Native `bind_with_initial_states(parameters, covariates, initial_states)`
and subsequent parameter `rebind` preserve explicit initial overrides; creating a
new binding/run with an empty map restores default initial expressions.

The bound-value hash now includes resolved initial values. Initial quantities that resolve to the same numeric vector, including explicit
values equal to defaults, share the same scientific identity for otherwise
identical runs. This intentionally changes historical
bound hashes from earlier compiler versions; recorded build fingerprints remain
distinct. Rebind inputs after changing the compiler version.

## Absolute simulation start

`regimen.start` is an optional quantity. Omission means zero in the model time
unit. A raw protocol's numeric `start` similarly defaults to zero. The state
initial condition applies at start, before start-time events. All samples,
observations, reset/covariate changes and expanded administrations use absolute
model time and must lie within the inclusive window. Infusions must end within
it. No pre-start dose, active infusion, or state history is inferred.

Start and end must be finite, with end strictly greater than start. Negative
windows are supported by the engine. The public raw `regimen::expand` utility
retains its zero-start meaning; `expand_in_window` accepts an explicit start.
Time expressions are not shifted, so nonautonomous models keep their meaning.

## Fixed solver checkpoints

`regimen.checkpoints` is an optional array of time quantities (default empty).
Each must be finite, dimensionally a time, and inside the declared start/end
window. Checkpoints force integration boundaries and solver restarts without
changing state, parameters, or active inputs. Coincident checkpoints and events
share a boundary; covariate binding selection still uses covariate change times.
Each supplied checkpoint consumes the event budget, including duplicates.
Ordinary observation times remain dense-output requests and do not force restarts.
Checkpoint quantities are included in the scientific request identity.

## Piecewise smooth state Jacobians

`abs(x)` uses derivative -1 for x < 0, 0 at x = 0, and 1 for x > 0,
multiplied by the derivative of its argument. `min` and `max` use the
derivative of the selected argument; exact ties select the first argument
in source order. These are local Jacobian conventions at nonsmooth points,
not a claim of classical differentiability there. All primal arguments retain
their domain checks. Arbitrary state-dependent conditionals remain unsupported.

Native tests check multiple arguments, ties, invalid unselected primal domains,
and BDF integration across a branch boundary against analytic integrals.
Browser qualification and trajectory comparisons are recorded separately.

## Hill state Jacobian

For positive x, `hill(x,n,k)` is differentiated through all three arguments.
The implementation evaluates both H(x,n,k) and H(k,n,x), avoiding cancellation
from subtracting a rounded-to-one Hill value. At negative x all partials vanish.
At zero, the n and k partials vanish; the x partial is zero for n>1, 1/k for
n=1, and the positive-x slope at x=1e-8*k for n<1. This last convention
applies to the BDF state Jacobian and does not modify the evaluated RHS.

The expression-only parameter derivative API rejects zero-boundary x
derivatives for n<=1 because it cannot validate a model's nonnegative
invariant. End-to-end parameter sensitivity remains a separate M3 capability.

## Fitting admission plans

`CompiledSensitivityDocument.plan_fit(request)` and the CLI `plan` command
support individual, pooled, Laplace, FOCEI and SAEM requests in the
[fit request contract](schema/fit-request.schema.json). The
[fit plan contract](schema/fit-plan.schema.json) describes admission at initial
inputs, including converted bounds, units, search coordinates, subject rows and
numerical budget ceilings. It performs no objective integration, optimization,
random draws, covariance estimation or scientific qualification.

Each estimator applies its execution admission rules. Individual and pooled
plans contain augmented Gaussian-objective preparations; pooled priors appear
only in the primary objective. Laplace uses its Gaussian-objective preparation.
FOCEI plans contain normal, uncensored observations and augmented simulations.
SAEM plans contain primal likelihood preparations and do not require derivative
absolute tolerances. One-effect SAEM includes the actual centered covariate
design, initial coefficients and corresponding subject-specific fixed effects
at eta zero. The subject plan's request records these admission inputs; it is
not a future sampled or fitted parameter set. Unsupported estimator options
return their ordinary structured errors.

`fit_input_hash` binds the original input and admitted subject identities,
including model/compiler/backend context. `plan_hash` binds the complete plan
using canonical JSON with `plan_hash` set to the empty string. Neither is a
future fit-result identity: estimates, final objective identity and convergence
require execution. Changing a request ID, controls, subject order or seed changes
the input digest. Changing the descriptive metadata limit changes the plan digest.

The native default aggregate metadata ceiling is 1,000,000 conservative record
charges across subjects, rows, binding inputs, states, outputs and observations.
`plan_fit_with_limit(request, limit)` accepts an explicit alternate ceiling.
This bounds descriptive retention separately from numerical work; it is not a
byte count, predicted runtime or reservation. Exceeding it returns
`output_budget` at `plan.metadata_item_limit`, with no incomplete plan. Reduce
subject/observation grids or choose an explicit native limit. CLI uses the
default ceiling and its ordinary input-file limit.

For this workflow, build the CLI using the setup above; no additional dependency
is required. From the candidate root, prepare the complete synthetic two-parameter
fit, execute it, then inspect the plan and result:

```sh
./target/release/qspquest plan conformance/models/synthetic-one-compartment.json conformance/requests/synthetic-fit.json > fit-plan.json
./target/release/qspquest fit conformance/models/synthetic-one-compartment.json conformance/requests/synthetic-fit.json > fit.json
python3 - <<'PY'
import json
from pathlib import Path
plan = json.loads(Path('fit-plan.json').read_text())
result = json.loads(Path('fit.json').read_text())['fit']
assert plan['schema'] == 'qspquest.fit-plan/v0.1'
assert [p['parameter'] for p in plan['coordinates']] == ['cl', 'v']
assert plan['coordinates'][0]['unit']['declared'] == 'L/h'
assert plan['subjects'][0]['preparation'] == 'gaussian_objective'
assert 'objective' not in plan and 'status' not in plan
assert result['status'] == 'converged'
assert abs(result['parameters']['cl']['value'] - 0.6) < 0.001
assert abs(result['parameters']['v']['value'] - 4.5) < 0.001
admission = plan['subjects'][0]['plan']['prospective_identity']
for key in ('model_content_hash', 'compiler_source_sha256', 'backend_version'):
    assert admission[key] == result['objective']['identity'][key]
print('Admitted starts:', [(p['parameter'], p['initial']) for p in plan['coordinates']])
print('Still unassessed by planning:', plan['deferred_checks'])
PY
```

The initial values are clearance 0.3 L/h and volume 3 L. The execution fits
synthetic analytic observations generated from 0.6 L/h and 4.5 L. The plan
reports initial likelihood inputs without claiming that the optimizer will
converge or that these parameters describe a real drug.

The following complete synthetic setup creates all population, pooled and scalar
planning inputs from the shipped fixture. It needs Python 3 and the built CLI.
It prepares every supported native estimator dimension without running its
optimizer:

```sh
python3 - <<'PY'
import copy, json, math
from pathlib import Path
root = Path('fit-plan-inputs')
root.mkdir(exist_ok=True)
fit = json.loads(Path('conformance/requests/synthetic-fit.json').read_text())['problem']['request']
inputs = {'pooled': {'schema': 'qspquest.fit/v0.1', 'problem': {'kind': 'pooled',
    'request': {'fit': fit, 'additional_subjects': [copy.deepcopy(fit['objective'])]}}}}
for dimensions in (1, 2):
    subjects = [copy.deepcopy(fit['objective']) for _ in range(2)]
    names = ['cl', 'v'][:dimensions]
    for subject in subjects:
        subject['simulation']['with_respect_to'] = names
        subject['simulation']['derivative_absolute_tolerances'] = {
            name: subject['simulation']['derivative_absolute_tolerances'][name] for name in names}
    request = {'subjects': subjects, 'fixed_effects': fit['bounds'][:dimensions],
        'random_effects': [{'parameter': name} for name in names],
        'omega': [[0.1 if i == j else 0 for j in range(dimensions)] for i in range(dimensions)],
        'omega_diagonal_bounds': [[0.1, 0.1] for _ in names], 'eta_bound': 1,
        'max_evaluations': 100, 'max_iterations': 8, 'gradient_tolerance': 0.01, 'seed': 17}
    for kind in (('laplace', 'focei', 'saem') if dimensions == 1 else ('focei', 'saem')):
        inputs[f'{kind}-{dimensions}'] = {'schema': 'qspquest.fit/v0.1',
            'problem': {'kind': kind, 'request': copy.deepcopy(request)}}
scalar = {'schema': 'qspquest.scalar-fit/v0.1', 'run': copy.deepcopy(fit['objective']['simulation']['run']),
    'parameter': 'cl', 'lower': fit['bounds'][0]['lower'], 'upper': fit['bounds'][0]['upper'],
    'observations': copy.deepcopy(fit['objective']['observations']),
    'max_iterations': 100, 'absolute_tolerance': 1e-6}
for observation, time in zip(scalar['observations'], (0, 1, 2, 4)):
    observation['value'] = {'value': 2 * math.exp(-0.6 / 3 * time), 'unit': 'mg/L'}
inputs['scalar'] = scalar
for name, request in inputs.items():
    (root / f'{name}.json').write_text(json.dumps(request) + '\n')
print('Synthetic planning inputs:', ', '.join(sorted(inputs)))
PY
for name in pooled laplace-1 focei-1 focei-2 saem-1 saem-2 scalar; do
  ./target/release/qspquest plan conformance/models/synthetic-one-compartment.json "fit-plan-inputs/$name.json" > "fit-plan-inputs/$name.plan.json" || exit
 done
./target/release/qspquest fit conformance/models/synthetic-one-compartment.json fit-plan-inputs/scalar.json > scalar-fit.json
```

Each population plan contains two admitted subjects. Inspect `population.omega`,
`coordinates`, `subjects` and `deferred_checks`; SAEM also reports its effective
burn-in count. A fitted additive residual uses standard-deviation coordinates
in FOCEI and variance coordinates in SAEM, with observable units recorded.
These numerical preparations do not qualify population estimates scientifically.

`CompiledDocument.plan_scalar_fit(request)` and CLI scalar planning return the
[scalar fit plan](schema/scalar-fit-plan.schema.json). `admission` describes the
original run inputs. `first_trial` is the optimizer's initial golden-section
point in the parameter's declared units. It is not a simulated result. Explicit
observation grids are checked during admission; implicit grids retain trial-time
row checks because fitted dosing lags can change their layout.
`observation_row_checks_deferred` records that distinction. Scalar plans have a
1,000,000-record descriptive ceiling. The scalar execution above should recover
clearance 0.6 L/h while volume stays fixed at 3 L. Invalid units, incompatible
error models or invalid estimator options produce structured refusals; a valid
plan can still fail in a later equation domain or numerical solve.

## Prepare a Morris analysis

Use the same generated or explicit `qspquest.morris/v0.1` request for planning
and execution. From the installed source tree, build the CLI and run the complete
synthetic example:

```sh
cargo build --release --locked -p qspquest-cli
./target/release/qspquest plan conformance/models/synthetic-one-compartment.json conformance/requests/synthetic-morris.json > morris-plan.json
./target/release/qspquest morris conformance/models/synthetic-one-compartment.json conformance/requests/synthetic-morris.json > morris-result.json
python3 - <<'MORRIS'
import json
plan = json.load(open('morris-plan.json'))
result = json.load(open('morris-result.json'))
assert plan['schema'] == 'qspquest.morris-plan/v0.1'
assert plan['analysis_request_hash'] == result['result']['analysis_request_hash']
assert plan['scan']['scan_request_hash'] == result['result']['scan']['scan_request_hash']
assert plan['design']['seed'] == result['design']['seed'] == 42
assert plan['observation'] == {'time': 4.0, 'side': 'post'}
for point, actual in zip(plan['scan']['points'], result['result']['scan']['results']):
    assert point['run']['prospective_identity'] == actual['result']['identity']
print('Verified six point identities and the exact generated design.')
MORRIS
```

The plan records six points, two trajectories, converted range units, the selected
output row and signed normalized steps. All point units, schedules, budgets and
time/side grids must be admitted before any point executes. Invalid rows and
unequal grids fail before integration. Planning can succeed for a future numerical
domain failure; it computes no elementary effects or statistics and establishes
no scientific qualification. Generated designs retain the deterministic generator,
seed and design hash. Explicit designs retain their supplied point order and geometry.

## Periodic preparation

Use `CompiledDocument.plan_periodic_steady_state(request)` for the analytic
linear PK fixed-point operation, and `plan_iterate_periodic_state(request)` for
BDF cycle iteration. The CLI uses explicit operation names because both input
contracts wrap a run without a top-level schema discriminator:

```sh
cargo build --release --locked -p qspquest-cli
# Run from the release root, using the included synthetic models and requests.
target/release/qspquest plan-steady-state conformance/models/synthetic-linear-pk.json conformance/requests/synthetic-linear-pk-steady.json > periodic-plan.json
target/release/qspquest steady-state conformance/models/synthetic-linear-pk.json conformance/requests/synthetic-linear-pk-steady.json > periodic-result.json
target/release/qspquest plan-iterate-periodic conformance/models/synthetic-saturable-periodic.json conformance/requests/synthetic-saturable-periodic.json > iteration-plan.json
target/release/qspquest iterate-periodic conformance/models/synthetic-saturable-periodic.json conformance/requests/synthetic-saturable-periodic.json > iteration-result.json
python3 - <<'PERIODIC'
import json
plan = json.load(open('periodic-plan.json'))
result = json.load(open('periodic-result.json'))
assert plan['schema'] == 'qspquest.periodic-plan/v0.1'
assert plan['prospective_identity'] == result['identity']
assert result['maximum_scaled_residual'] <= 1
assert all(state['seed'] is None for state in plan['states'])
iteration = json.load(open('iteration-plan.json'))
iterated = json.load(open('iteration-result.json'))
assert iteration['prospective_identity'] == iterated['steady_state']['identity']
assert iterated['cycles_completed'] <= iteration['maximum_cycle_executions']
assert all(state['seed'] is not None for state in iteration['states'])
print('Verified periodic planning identities and convergence residuals.')
PERIODIC
```

Prerequisites are the documented Rust toolchain and Python 3. The
[`periodic-plan` schema](schema/periodic-plan.schema.json) is closed. Inspect
converted parameter and covariate inputs, state units, the exact normalized
phase cycle, event quantity units, convergence tolerances and the conservative
per-cycle budgets. Affine doses are already scaled into coherent state amounts;
BDF doses retain input units and the model applies its declared dose scaling.
Infusion carryover is folded into the repeated cycle before planning or execution.
A BDF seed is the actual converted pre-event state. The affine plan contains no
solved initial state. Analytic integration tolerance is null because that
propagator does not use the BDF local tolerance; convergence tolerance is separate.

Static schedule, units and partitioned event/output budgets are checked before
integration. Each affine solve reserves `state_count + 2` cycle executions;
iteration divides its total budget by `maximum_cycles`. A large iteration ceiling
can leave too little budget for each cycle. Increase total ceilings when that
partition fails, and keep events in the half-open phase interval. Requests do
not accept trajectory observations. Convergence, future trajectory domains and
fixed-point existence remain numerical execution checks. A successful plan does
not prove execution, uniqueness, stability or scientific qualification. BDF
convergence depends on the supplied seed and requires two consecutive passing
cycle residuals. Failed solves return no partial fixed point.

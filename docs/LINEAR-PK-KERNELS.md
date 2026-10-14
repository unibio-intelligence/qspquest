# Linear PK interval kernels

`qspquest_modeling::linear_pk::LinearPk` propagates amounts through a constant-rate
interval for one to three mammillary disposition compartments. The first
volume is central; each additional compartment exchanges with central through
its own clearance. An optional first-order absorption depot is the first state,
followed by central and peripheral states. Elimination occurs from central.

The constructor accepts elimination clearance, compartment volumes, exchange
clearances and an optional absorption rate. All values must use a coherent
amount/volume/time basis. This low-level API does not convert units. `advance`
accepts initial amounts, constant amount/time inputs and interval duration;
`concentrations` returns disposition concentrations and excludes the depot.
Zero clearance, zero exchange and zero absorption rates are accepted.

## Numerical method

The exact interval solution is `exp(A dt) x + dt phi_1(A dt) u`. An augmented
matrix exponential computes both functions without inverting A or dividing by
differences of rates. This supports singular systems and coincident absorption
and elimination rates. Evaluation uses scaling and squaring with a degree-20
Taylor polynomial after reducing the matrix infinity norm to at most 0.5.
Matrix products reuse scratch storage within each propagation. This does not
change the polynomial degree, scaling rule or order of the dot-product sums.
Roundoff and repeated squaring still limit accuracy. Intervals requiring more
than 60 squarings are explicitly unsupported; that limit is a work bound, not
an accuracy guarantee. No automatic solver selection or speed claim is made.

Nonfinite or negative inputs and invalid dimensions fail before execution.
Overflow and nonfinite/negative propagated results return errors, without
clamping. The immutable kernel owns no mutable trajectory state.

## Verification and remaining integration

Owned tests use 48 independently assembled SciPy matrix-exponential cases:
one, two and three compartments, with/without a depot, with/without elimination,
and four durations. Further checks use one-compartment closed forms,
coincident absorption/elimination rates, zero-rate infusion accumulation,
mass conservation and split-interval consistency. These are numerical checks
for the interval-kernel contract, without biological qualification.

`LinearPk::bind(initial_amounts)` returns a `BoundLinearPk` implementing the
common `Model` contract. `simulate_with_budgets` uses its analytic interval
propagator inside the same event loop as BDF. Boluses, overlapping infusions,
ordered resets, stopping target infusions and pre/post boundary rows therefore
use the common driver. Each interval observation is propagated from the same
initial state, so adding samples does not accumulate propagation roundoff.
Each analytic propagation consumes one solver work-budget unit; matrix size
and scaling iterations are bounded. Failed runs return no partial trajectory.

Shared-driver tests compare all compartment/depot combinations against BDF,
check reset/infusion behavior against a closed form, and verify budget failures,
recovery and unchanged results when additional samples are requested.

Typed model declarations and explicit backend selection are described below.
Binding qualification remains open. Steady state and parameter
sensitivities also remain separate requirements. Existing corpus evidence
continues to use its pinned Diffsol builds.

## Typed JSON models and explicit execution

`ModelDocument.linear_pk` declares a whole linear PK model. Its `amount_unit`
is a mass, amount-of-substance or dimensionless unit. `clearance` names a
parameter with volume/time units. `compartments` contains one to three records
with `state`, `volume` (a parameter name), and a unit-bearing `initial` value
or initial expression. The first compartment is central. `exchange_clearances`
names one volume/time parameter per peripheral compartment. Optional `depot`
contains `state`, `absorption` (a parameter with inverse-time units), and
`initial`. Coefficient names may reference parameters, constant covariates or derived
`individual` bindings. Derived chains must depend only on parameters and
constant covariates. Step-covariate, time and state dependencies are rejected;
the analytic interval is never run with a silently frozen coefficient.

Explicit `states` must be empty. The compiler constructs the equations,
amount-valued dosing targets and nonnegative-state invariants. Normal output
expressions, initial overrides and regimen handling remain available. Parameter
units are validated and converted consistently; invalid volumes, clearances or
absorption rates fail under both execution backends. Generated equations are
not accepted from the caller alongside a potentially conflicting PK declaration.

Requests explicitly select `analytic_linear_pk` or `diffsol_bdf`. The former
requires a typed declaration; ordinary ODEs are never silently classified as
linear. The result records `backend = qspquest` and
`algorithm = linear_pk_exponential_taylor20` for analytic execution. The backend
choice also enters the run-request hash. Compiler/source, model and bound-value
identities retain their ordinary meanings. Tolerances remain required by the
shared request contract, but do not tune the fixed matrix-exponential algorithm.

The owned `synthetic-linear-pk.json` model/request pair demonstrates mixed
volume and clearance units, delayed dosing and infusion. Native tests compare
both backends, equivalent unit overrides, parameter changes and error recovery.
Text parsing and formatting preserve typed PK declarations, including quantity
and expression initial conditions. Native tests verify identical JSON, content
hashes, execution identities and trajectories under both backends. See
`spec/language.md` for the block syntax. Python/WASM runtime checks for the owned fixture are recorded below.


## Python and browser evidence

The owned three-compartment/depot fixture passes six cases: both backends at
clearances of 0, 0.7 and 1.4 L/h. It includes mixed units, a delayed bolus and
an infusion. An independent SciPy event implementation gives maximum error
1.910e-14 for the analytic backend and 8.488e-9 for BDF. Chrome 154.0.8037.58
and Firefox 154.0.1 match native within 8.882e-16. Names, units, complete grids,
pre/post sides and scientific identities, including the selected backend,
are checked. Budget and kinetic-domain failures leave the worker reusable.

The installed macOS arm64 abi3 wheel passes all nine integration tests on
CPython 3.11 and 3.12, including concurrent runs with independent bindings and
backend selection. Text and JSON models preserve model identity. The build
used two compiler jobs per native/WASM build in parallel; the Python/browser
checks ran as four independent processes. No timing claim follows from them.

`conformance/evidence/linear-pk-bindings-v1.json` records wheel, WASM, CLI,
compiler, runner and fixture hashes and both Python test logs. Native and
browser per-case results remain in their referenced reports. This does not
qualify Safari, other OS/architecture wheels, all possible parameter regimes,
source-distribution rebuilds, sensitivities or product-worker integration.


## Derived coefficient checks

The compiler reuses the normal topological ordering and units of `individual`
bindings when it builds kinetic coefficient programs. Binding validates the
complete derived values before either backend runs. Tests cover chained
clearances with reversed declaration order, mL/h-to-L/h conversion, required
constant covariates, changing bindings on a shared compiled model, invalid
negative coefficients, cycle rejection and transitive step-covariate rejection.
These changes require a new binding build; earlier Python/browser receipts
remain evidence for their recorded compiler revisions only.

## Model dosing overrides

A compartment or depot can supply `dosing_override` using the ordinary `Dosing`
record: accepted modes, input amount unit, unit-checked scale expression and
optional lag. Omitting it retains the default amount-to-amount scale of one.
The override is lowered into the shared dosing target, so analytic and BDF
execution apply the same conversion, bioavailability expression and lag.
Text uses the existing `dosing` block, naming the declared PK state; formatting
preserves the override rather than folding it into request amounts.

## Periodic steady-state kernel API

`LinearPk::periodic_steady_state(cycle, budgets, tolerance)` computes amounts
immediately **before** the cycle's start-time events. The cycle spans
`[start, end)`. Boluses and resets at `start` are included; events at `end` are
rejected to avoid double counting. Infusions must start within the cycle and
finish no later than `end`. An infusion ending exactly at `end` is supported.
Cross-cycle infusions require a separate contract and are currently rejected.

The method builds the affine cycle map `x_next = P x + b` using the shared event
driver, then solves `(I - P) x = b`. Homogeneous passes zero dose/reset additions
while preserving event ordering; this avoids subtracting large forced results
to obtain transition columns. A final forced cycle checks the residual against
`absolute + relative * max(abs(start), abs(end))` for every state. The result
contains pre-cycle amounts, signed residuals and the maximum scaled residual.
Absolute tolerances use the kernel's coherent amount unit.

All `n + 2` passes receive conservative partitions of the supplied work, event
and output budgets. No partial steady-state result is returned on failure.
Non-unique, accumulating or numerically unresolved systems fail explicitly;
the method does not select an arbitrary fixed point. Resetting cycles can have
a unique fixed point even with zero clearance. Singular values are not replaced
by small positive constants, and states are not clamped.

Tests check repeated-bolus and finite-infusion closed forms, continuous-infusion
equilibrium, phase shifts, one/two/three-compartment accumulation over 250
cycles, reset-defined fixed points, and invalid/underbudgeted/unresolved cases.
The typed operation and binding evidence are described below. Cross-cycle
inputs and nonlinear periodic steady state remain separate work; no full
steady-state or performance qualification is claimed.

### Typed periodic operation

`CompiledDocument::periodic_steady_state(&SteadyStateRequest)` accepts a nested
`RunRequest`, an amount-valued `absolute_tolerance`, and `relative_tolerance`.
The nested regimen defines one complete repeating cycle. Select
`analytic_linear_pk`; omit initial overrides, samples and observations. The
result contains unit-bearing `initial_states`, signed cycle `residuals`, the
maximum scaled residual and a compiler/backend execution identity. Feed the
returned initial states into an ordinary run to simulate the cycle trajectory.

Model and request lag, input-unit conversion, bioavailability, dosing scale and
resets retain their normal execution semantics. History, switches, changing
covariates and checkpoints are currently rejected. The operation returns only
the state fixed point; it does not assert periodicity of time-dependent outputs.
JSON schemas are exported for both envelopes. Binding entry points:

- CLI: `qspquest steady-state MODEL REQUEST.json`.
- Python: `CompiledModel.periodic_steady_state(request)` returns a dict.
- WASM: `CompiledSimulation.periodic_steady_state(request_json)` returns JSON.
- Conformance worker: `{kind: 'steady-state', source, request}`.

The installed macOS arm64 wheel passes concurrent calls and failure recovery
on CPython 3.11 and 3.12. Chrome and Firefox match native amounts exactly for
three requests (two distinct clearance values plus a repeated binding).
Each native result closes one full cycle through ordinary simulation. The CLI
matches the typed library result and emits no partial output on failure.
Evidence: `conformance/evidence/steady-state-bindings-v1.json`. Safari and other
native platforms remain unqualified.

### Infusions crossing a periodic boundary

The kernel and typed operation accept finite infusions crossing one or more
cycle boundaries. The
kernel's `periodic_cycle_protocol` returns a normal amount-state protocol with
the preceding cycle's tail and the current cycle's head. Each segment retains
the infusion rate. Stop-target resets truncate delivery before wrapping;
this includes a reset exactly at cycle start. Expanded events consume the
caller’s event budget.

The ordinary finite-run API still requires every infusion to finish inside its
window. For a crossing infusion, the original regimen and the returned steady
amounts alone are insufficient to replay one finite cycle: the previous cycle's
active input must also be represented. Use an explicitly split regimen (or the
kernel's normalized amount-state protocol with a kernel-bound model).
Infusions longer than one period are supported by aggregating complete overlapping cycles into one constant input plus at most two remainder segments. This addition has native
kernel/typed evidence; the earlier binding receipt remains pinned to its prior
build and contained-infusion cases.

### Infusions spanning multiple cycles

Complete overlaps are aggregated in constant space. A periodic stop-target
reset instead limits each administration to its first cancellation time; its
input is split only across the boundary if needed. Native tests compare
24-, 30- and 121-hour infusions repeated every 12 hours against unsplit
100-cycle trajectories, with resets at three different phases. A typed
three-compartment case independently matches constant-input equilibrium.
A billion overlapping periods expand to one input event; remainder segments
are still charged to the event budget. The v2 binding receipt below covers these newly supported requests.

### Multi-cycle binding qualification

`conformance/evidence/steady-state-bindings-v2.json` pins the updated macOS
arm64 wheel and WASM. Eight cases pass on CPython 3.11/3.12 and Chrome/Firefox,
including boundary crossing, multi-period overlap and stop-target resets.
Both browsers match native amounts exactly. Five crossing/overlap cases agree
with 500 unsplit cycles to at most 1.244e-12 in model amount units. Budget,
unit and backend errors preserve subsequent Python execution; browser workers
recover after work-budget failure. The older v1 receipt remains available.

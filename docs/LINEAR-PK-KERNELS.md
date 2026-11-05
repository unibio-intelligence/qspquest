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
Roundoff and repeated squaring still limit accuracy. Intervals requiring more
than 60 squarings are rejected; that limit caps work and does not guarantee
accuracy. This method is never selected automatically.

Nonfinite or negative inputs and invalid dimensions fail before execution.
Overflow and nonfinite/negative propagated results return errors, without
clamping.

## Event handling

`LinearPk::bind(initial_amounts)` returns a `BoundLinearPk` implementing the
common `Model` contract. `simulate_with_budgets` uses its analytic interval
propagator inside the same event loop as BDF, so boluses, overlapping infusions,
ordered resets, stopping target infusions and pre/post boundary rows behave the
same way under both. Each interval observation is propagated from the same
initial state, so adding samples does not accumulate propagation roundoff.
Each analytic propagation consumes one solver work-budget unit; matrix size
and scaling iterations are bounded. Failed runs return no partial trajectory.

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
constant covariates. Step-covariate, time and state dependencies are rejected,
because coefficients must stay constant over each interval.

Explicit `states` must be empty. The compiler generates the equations,
amount-valued dosing targets and nonnegative-state invariants. Normal output
expressions, initial overrides and regimen handling remain available. Parameter
units are validated and converted; invalid volumes, clearances or absorption
rates fail under both execution backends.

Requests explicitly select `analytic_linear_pk` or `diffsol_bdf`. The former
requires a typed declaration; ordinary ODEs are never treated as linear
automatically. The result records `backend = qspquest` and
`algorithm = linear_pk_exponential_taylor20` for analytic execution.
Tolerances are still required in the request but do not affect the analytic
backend.

The `synthetic-linear-pk.json` model/request pair demonstrates mixed volume and
clearance units, delayed dosing and infusion, and tests compare both backends.
The text format supports the same PK declarations; see `spec/language.md` for
the block syntax.


## Derived coefficient checks

Derived coefficients follow the usual ordering and units of `individual`
bindings and are validated before either backend runs. Tests cover chained
clearances with reversed declaration order, mL/h-to-L/h conversion, required
constant covariates, changing bindings on a shared compiled model, invalid
negative coefficients, cycle rejection and transitive step-covariate rejection.

## Model dosing overrides

A compartment or depot can supply `dosing_override` using the ordinary `Dosing`
record: accepted modes, input amount unit, unit-checked scale expression and
optional lag. Omitting it keeps the default amount-to-amount scale of one.
Analytic and BDF execution apply the same conversion, bioavailability
expression and lag.
Text uses the existing `dosing` block, naming the declared PK state; formatting
preserves the override rather than folding it into request amounts.

## Periodic steady-state kernel API

`LinearPk::periodic_steady_state(cycle, budgets, tolerance)` computes amounts
immediately **before** the cycle's start-time events. The cycle spans
`[start, end)`. Boluses and resets at `start` are included; events at `end` are
rejected to avoid double counting. Infusions must start within the cycle; they
may run past `end` (see [Infusions crossing a periodic
boundary](#infusions-crossing-a-periodic-boundary)).

The method builds the affine cycle map `x_next = P x + b` using the shared event
driver, then solves `(I - P) x = b`. A final cycle with doses checks the
residual against `absolute + relative * max(abs(start), abs(end))` for every
state. The result contains pre-cycle amounts, signed residuals and the maximum
scaled residual. Absolute tolerances are in the kernel's amount unit.

The `n + 2` passes split the supplied work, event and output budgets between
them. No partial steady-state result is returned on failure.
Non-unique, accumulating or numerically unresolved systems fail explicitly;
the method does not select an arbitrary fixed point. Resetting cycles can have
a unique fixed point even with zero clearance. Singular values are not replaced
by small positive constants, and states are not clamped.

Tests check repeated-bolus and finite-infusion closed forms, continuous-infusion
equilibrium, phase shifts, one/two/three-compartment accumulation over 250
cycles, reset-defined fixed points, and invalid/underbudgeted/unresolved cases.

### Typed periodic operation

`CompiledDocument::periodic_steady_state(&SteadyStateRequest)` accepts a nested
`RunRequest`, an amount-valued `absolute_tolerance`, and `relative_tolerance`.
The nested regimen defines one complete repeating cycle. Select
`analytic_linear_pk`; omit initial overrides, samples and observations. The
result contains unit-bearing `initial_states`, signed cycle `residuals`, the
maximum scaled residual and an identity record. Feed the returned initial
states into an ordinary run to simulate the cycle trajectory.

Model and request lag, input-unit conversion, bioavailability, dosing scale and
resets work as in a normal run. History, switches, changing covariates and
checkpoints are currently rejected. The operation returns only the steady
states; outputs that depend directly on time need not be periodic.
JSON schemas are exported for both envelopes. Binding entry points:

- CLI: `qspquest steady-state MODEL REQUEST.json`.
- Python: `CompiledModel.periodic_steady_state(request)` returns a dict.
- WASM: `CompiledSimulation.periodic_steady_state(request_json)` returns JSON.
- Conformance worker: `{kind: 'steady-state', source, request}`.

The installed macOS arm64 wheel passes concurrent calls and failure recovery
on CPython 3.11 and 3.12. Chrome and Firefox match native amounts exactly for
three requests (two distinct clearance values plus a repeated binding).
Each native result is checked by simulating one full cycle. The CLI matches the
library result and prints no partial output on failure. Safari and other native
platforms are not yet tested.

### Infusions crossing a periodic boundary

The kernel and typed operation accept finite infusions crossing one or more
cycle boundaries. The
kernel's `periodic_cycle_protocol` returns a normal amount-state protocol with
the preceding cycle's tail and the current cycle's head. Each segment retains
the infusion rate. Stop-target resets truncate delivery before wrapping;
this includes a reset exactly at cycle start. The extra events count toward
your event budget.

The ordinary finite-run API still requires every infusion to finish inside its
window. To replay one cycle with a crossing infusion, the returned steady
amounts are not enough: you must also include the infusion still running from
the previous cycle. Use an explicitly split regimen (or the kernel's
normalized amount-state protocol with a kernel-bound model).

### Infusions spanning multiple cycles

Infusions longer than one period are supported. Complete overlaps are combined
into one constant input plus at most two remainder segments, so even a billion
overlapping periods become one input event; remainder segments still count
toward the event budget. A periodic stop-target reset instead limits each
administration to its first cancellation time; its input is split across the
boundary only if needed. Tests compare 24-, 30- and 121-hour infusions repeated
every 12 hours against unsplit 100-cycle trajectories, with resets at three
different phases. A typed three-compartment case matches constant-input
equilibrium.


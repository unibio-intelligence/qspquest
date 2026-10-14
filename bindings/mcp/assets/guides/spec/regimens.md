# Fixed administration requests

Status: implemented M1 event subset, not the complete event contract.

A text-model worker accepts exactly one `protocol` (the existing numeric form)
or `regimen`. A regimen contains unit-bearing `end`, `samples`, and
`administrations`, plus `rtol` and exactly one absolute-tolerance mode. The WASM
`CompiledSimulation.simulate_regimen` entry point uses the same native expansion
and simulation code.

Example administration:

```json
{
  "target": "central",
  "time": {"value": 0, "unit": "h"},
  "amount": {"value": 6000, "unit": "ug"},
  "delivery": {"kind": "bolus"},
  "repeat": {"interval": {"value": 120, "unit": "min"}, "additional": 2},
  "lag": {"value": 30, "unit": "min"},
  "bioavailability": 0.5
}
```

The target must be a named state with a declared dosing mapping. Times convert
to the model time unit; amounts convert to the mapping's amount unit. The
example delivers three doses at 0.5, 2.5 and 4.5 h. Each contains 3 mg before
the model's amount-to-state conversion.

## Expansion rules

- `additional` means additional administrations after the first, matching ADDL
  semantics. It is an unsigned 32-bit integer. `interval` must be positive.
- Missing/null `repeat` means one administration. Missing/null `lag` means zero.
- Time, lag and amount must be finite and nonnegative. Bioavailability is a
  required finite fraction in [0, 1]. It is not a fitted expression.
- The i-th start is `time + i * interval + lag`. Distinct representable times
  remain distinct; no epsilon clustering is used. A repeat or positive lag that
  collapses in floating-point time is rejected.
- Infusions use either `delivery.span = {kind: "duration", duration: quantity}`
  or `{kind: "rate", rate: quantity}`. The closed enum rejects both-at-once or
  unknown fields. Duration and rate must be positive.
- For rate-defined infusions, duration is `administered amount / administered
  rate` before bioavailability is applied. The delivered amount and rate are
  multiplied by F; duration does not change. Rate-defined zero-amount infusions
  are rejected because they imply zero duration. A positive-duration infusion
  with zero delivered amount is valid.
- F = 0 preserves the schedule while delivering no drug. Overlapping infusion
  rates add in the existing driver over each half-open active interval.
- The entire expanded schedule, including infusion ends, must fit inside the
  run window. The decoder rejects out-of-window administrations rather than
  truncating a regimen silently.
- Expansion preserves simultaneous records and stably sorts by exact start
  time. The driver retains its pre/post boundary semantics.

The expanded count is checked before allocating expanded events. Counting uses
u64 on native and wasm32; the default worker limit is 10,000 administrations.
Limit failures return `work_budget`, including a request for u32::MAX additional
doses. Per-administration validation errors carry `administrations[index]`.
A failure produces no partial scientific result. Unknown targets, incompatible
units, invalid fractions, invalid intervals and unrepresentable times fail.

## Remaining scope

This is a unit-bearing administration/time request, not the final full run
schema. Named administration channels
beyond the current state mappings and steady state
are not implemented. Explicit observation-side selection is described below. Dynamic event
expressions are not accepted by this schema. Solver requirements, complete
result identity and managed product integration remain separate work.

## State resets

A regimen may now contain `resets` (omitted means empty):

```json
{
  "target": "central",
  "time": {"value": 120, "unit": "min"},
  "value": {"value": 3000, "unit": "ug"},
  "order": 0,
  "active_inputs": "stop_target"
}
```

The target names a state; it does not need a dosing mapping. The value converts
to the state's unit and must be finite. Resets share the run's event budget with
expanded administrations. The raw protocol has a corresponding `kind: reset`
variant with target index, numeric time/value and the same required policy/order.
`Event::amount()` now returns an option: resets do not have a dose amount.

At a boundary the driver records the pre-state, treats ending infusions as
inactive, applies resets in ascending `order`, applies boluses, starts new
infusions, and records the post-state. Multiple resets at the same exact time
must have distinct order values, including when they name different states.
Duplicate orders fail before integration. Reordering input records does not
change the explicitly ordered reset result.

Active input policies are explicit and required:

- `continue`: retain the current active input set.
- `stop_target`: stop every previously started, still-active infusion into the
  reset state. Other targets are unaffected. Infusions starting at the reset
  time begin after the reset and remain active.

Stopped infusion records remain stopped for the rest of that run; a later
`continue` reset does not restart them. Infusions are tracked by their stable
record index within the immutable protocol. Original scheduled end times may
remain in the boundary/output timeline even when a reset stopped an infusion
early. They do not deliver additional drug. The solver is reinitialized at reset
boundaries, as at the existing dosing discontinuities.

No occasion label causes an implicit reset. No model-wide nonnegative reset
constraint is inferred: the implemented reset check enforces finite values and
units. General model invariants and reset sensitivity jump qualification remain
separate outstanding work.

## Explicit observation times and sides

A regimen can select deterministic outputs using a nonempty `observations`
list. Each point requires a unit-bearing time and a side:

```json
{
  "samples": [],
  "observations": [
    {"time": {"value": 120, "unit": "min"}, "side": "pre"},
    {"time": {"value": 2, "unit": "h"}, "side": "post"}
  ]
}
```

These are evaluation-time requests, not measured observation values or a
statistical observation model. At a declared event, `pre` selects the state
before the boundary operations and `post` selects the state after them. At a
continuous, non-event instant, both sides use the same state. The returned side
still matches the requested label. An unavailable side at a declared event is
an error, not a fallback to the other side.

The result contains exactly the requested points, in request order, including
duplicates. Times are converted to the model time unit. Consumers must not assume
that this selected result is sorted by time. The full trajectory mode remains
available by omitting/nulling `observations` and using `samples`.

Combining nonempty trajectory `samples` with an observation plan is rejected.
An explicitly empty observation plan is also rejected. Missing/unknown side
labels, incompatible units, out-of-window times and output-budget violations
fail before returning a scientific result.

Output expressions are evaluated only at selected points in observation mode.
For example, a requested post-dose `log(amount / amount_ref)` can be valid even
if its unrequested pre-dose value is undefined. Requesting that pre-dose value
returns its domain error and no partial selected result. The ODE itself still
must have valid domains throughout integration. Trajectory mode continues to
check outputs at every returned trajectory point.

Memory checks cover retained state rows plus selected output values. Event and
solver-work budgets remain in effect. The current observation result has all
declared deterministic output columns; per-endpoint selection, measured data,
censoring and likelihood evaluation remain outside this implemented slice.

## Covariate steps

A regimen accepts a `covariates` object mapping names to baseline quantities and
`covariate_changes`, each with `name`, quantity `time` and quantity `value`.
Changes require a declared `step` covariate. They must be finite, inside the run,
and unique per covariate at each exact time. Different covariates changing at
the same instant are rebound together. Constant covariates reject changes.

Order at a boundary: capture pre-event state and binding, apply covariate
changes, apply ordered state resets, apply boluses, capture post-event state and
binding. Initial conditions use the baseline binding before time-zero changes.
A covariate step does not reset the evolving state or re-evaluate its initial
condition. Explicit state resets are required for instantaneous state changes.
The changed binding recomputes derived parameters and dose conversions. Ongoing
infusion amount rates use the current amount-to-state conversion after a step;
this alone does not rescale the existing state.

The entire left interval, including solver evaluations at its ending time, uses
the left binding. Integration restarts at the exact change time. Output
projection uses the binding appropriate to each time and requested pre/post
side, including out-of-order observations and changes at zero or run end.

Changes share the event budget with administrations and resets. Numeric storage
for bound contexts is charged against the available value budget before
allocating the schedule; remaining storage is reserved for states and outputs.
One callback budget covers the whole solve, including all interval restarts.
Failures produce no partial trajectory and do not alter the baseline binding.

Use `BoundDocument.simulate_regimen` for these requests. The low-level
`regimen_protocol` converter rejects covariate-bearing requests because a raw
dosing protocol has no representation for binding changes. The WASM worker
uses the full regimen path.

## State tolerances

Each request supplies exactly one of:

- `atol`: a finite positive scalar interpreted in each state's declared numeric
  unit, retained for existing conformance clients;
- `absolute_tolerances`: an object naming every state, with a finite positive
  quantity in compatible units, for example
  `{"bulk":{"value":0.01,"unit":"mg"},"trace":{"value":1e-15,"unit":"mol"}}`.

The compiler converts quantities to state units and orders the vector by state
declaration before passing it to Diffsol. Missing/extra names, incompatible
units, zero/negative/nonfinite values, and conflicting or missing modes fail
before solving. `rtol` remains dimensionless. The same validated vector is used
across all dose, reset and covariate intervals. These are solver error-control
settings, not a guarantee that global trajectory errors equal their values.

The full regimen API handles per-state tolerances. The raw-protocol converter
rejects this mode because its legacy scalar protocol cannot represent it.

## Zero dose fractions

A dose-to-state scale must be finite and nonnegative. Zero is a valid
parameterized fraction and delivers no state increment or infusion input.
The administration still consumes the ordinary event budget and preserves
its boundaries and observation sides. Rebinding recomputes the scale; a
later positive binding restores delivery. Negative and nonfinite scales
remain errors. This applies to both typed regimen execution and raw
bolus/infusion protocols.


## Declared fixed switches

Models may declare up to 1,024 `switches`, each with a unique `name` and a
`time` expression. The expression must have time dimensions and may reference
only declared parameters. States, inputs, covariates, other switches and time
itself are excluded. Rebinding parameters recomputes the schedule; gradients
through moving switch times remain unsupported.

Each switch supplies a dimensionless numeric binding: zero before its event,
one afterward. Use it arithmetically or in a condition such as `active > 0`.
For example, the text language accepts:

```text
switches
  active = start_time
  stopped = start_time + duration
```

`start_time` and `duration` must be declared parameters with time units. JSON
uses `"switches": [{"name": "active", "time": {"kind": "symbol",
"name": "start_time"}}]`. Unknown fields are rejected. Switch names share the
model binding namespace; callers cannot override them through covariate values
or covariate changes.

Switches lower to owned step bindings. Interior boundaries retain pre values,
then update all simultaneous switches and covariates, then apply resets and
doses, and retain post values. The solver restarts at each boundary. A switch
at or before the run start is already active when initial expressions are
evaluated; this initializes the requested run and does not replay history.
Switches after the run end remain zero. A switch at the run end still has pre
and post observations. Every in-window switch change consumes an event budget
entry before simultaneous times are merged, alongside other declared events.

Use `execute` or `simulate_regimen`. Raw protocol simulation of a bound switch
model fails explicitly because it cannot carry the owned schedule. Existing
models with no switch declarations retain their serialized model identity.

Native analytic tests cover integration across switches, observation sides,
start-time initialization, parameter/unit rebinding, simultaneous switch/reset/
dose ordering, independent repeated execution, name and time restrictions,
forbidden overrides, event budgets and text round trips. Browser and Python
package qualification for this feature is pending. Plain phase declarations
retain the side-based semantics above. Explicit time predicates use the
additional pointwise-readout contract below.


### Strict and inclusive time predicates

A switch can additionally declare `comparison` as `lt`, `le`, `gt` or `ge`.
Its binding then represents the numeric truth value of `time comparison
threshold`. For example:

```text
switches
  active = time > start_time
  before_end = time <= end_time
```

JSON stores the threshold in `time` and the operator in `comparison`. The
threshold still depends only on parameters and must have time dimensions.
Equality and inequality (`eq`, `ne`) predicates are not qualified.

Integration uses the constant truth value on each open interval, with a
restart at every threshold. Outputs evaluate the predicate at the exact
sample time, preserving the difference between strict and inclusive comparisons.
Both pre- and post-event readouts use that pointwise predicate value; their
state and ordinary covariate values still follow the requested side. Initial
expressions use the exact predicate at the run start, then integration enters
the right-hand interval. No epsilon shift or smoothed approximation is used.

Predicate-dependent dose scales, including dependencies through definitions or
individual parameters, are rejected until their event-time semantics qualify.
Plain phase switch dose scales retain the earlier step-binding contract.
Parameter sensitivities through event times remain unsupported.

Native tests cover all four predicates, analytic trajectories, exact pre/post
readouts, initial values at a threshold, text round trips, and unsupported
predicate uses. The legacy adapter explicitly lowers comparisons between `t`
and parameter-only thresholds (including reversed comparisons), retains their
source trees and deduplicates identical predicates. Unsupported time expressions
and state-dependent thresholds remain compiler errors.

## Model-bound dosing lag

A state dosing declaration may include `lag`, a time-valued expression. It may
reference model parameters, constant covariates and algebraic definitions using those inputs. The
expression is evaluated for each bound run, so changing parameters changes the
schedule without recompiling the model. State, time, step-covariate and individual
parameter dependencies are currently rejected by the lag compiler. Constant
covariates use the effective bound/request values and their declared units;
missing required values fail without producing a trajectory.

The effective lag is the model lag plus the administration's explicit lag.
Both must be finite and nonnegative. The existing event expansion applies this
sum to each repeated administration and to infusion start/stop times. Model lag
does not alter bioavailability or infusion duration. Requests and model hashes
retain their original expressions and input quantities.

Use `execute` or bound `simulate_regimen`. Raw event/protocol APIs reject models
with lag because they cannot bind its expression. Text authoring uses
`lag(central) = delay` in the dosing block; formatting preserves the expression.
Duplicate declarations and targets that do not accept dosing are rejected. Browser/Python package qualification for this
new feature is pending; previous packaged builds do not include it.

Native tests cover unit conversion, additive administration lag, repeated
boluses, parameter rebinding, exact boundary sides, analytic trajectories,
negative lag, state dependencies and dimension errors.

## Administration metadata for history consumers

`qspquest_core::regimen::expand_administrations_in_window` returns one validated
`ExpandedAdministration` per occurrence. Each record preserves the original
administration index, zero-based repetition index, scheduled time before lag,
administered amount before bioavailability, and the ordinary delivered event.
The event time includes lag. Amounts use the target input unit and times use the
run time unit. Model-specific amount-to-state scaling occurs later.

The existing `expand_in_window` delegates to the same expansion and returns only
its events. Both APIs share validation, budgets, stable simultaneous-event
ordering and infusion-duration calculations. In particular, F = 0 does not erase
the original amount from the metadata record.

The model bindings below use these records. Reset history and nullable
observation representations still require separate contracts.

## Immutable history lookup

`dose_history::DoseHistory` owns normalized expanded administration records and
indexes them by target and delivery time. `at(target, time, side)` returns the
selected raw amount, delivery and scheduled times, elapsed time, and original
administration/repetition indices. Query order has no effect, including a
backwards query after an adaptive solver trial. No prior dose returns `None`.
Non-finite times, elapsed-time overflow and malformed records return errors.

At an exact boundary, `Pre` excludes all doses at that time and `Post` includes
the selected dose. `SimultaneousDose::First` and `Last` make tie selection
explicit; input order within equal times is stable. Owned native bolus
probes select `First`, including when the two original records are reversed.
Times one representable value apart remain distinct.

Infusion entries start history at delivery; stopping an infusion does not erase
that history. This concerns history readouts only and does not establish source
runtime equivalence for infusion delivery/F conventions. Reset entries are
rejected pending an explicit history policy. Model-visible bindings are described below. Dose-relative switching and
nullable result columns remain unfinished.


## Model history bindings

A model may declare a `dose_history` array:

```json
{"name":"last_dose", "target":"central", "quantity":"dose_amount", "simultaneous":"first"}
```

Every declaration requires a name, an existing dose target, a quantity and an
explicit `first` or `last` simultaneous-dose policy. Quantities are `dose_amount`
(in target input units, before F or state scaling), `dose_time` (delivered time),
`time_since_dose`, and `has_dose` (dimensionless 0 or 1). Times use model units.
The declared name is available to differential equations and readouts. Use a
lazy conditional guarded by `has_dose` when numeric history may be missing.
An evaluated numeric history read without a previous dose returns a domain
error; it never fabricates a zero previous dose.

History follows model plus request lag and creates bounded step bindings at
delivery. Pre/post observations select history on the same side as the state.
Internal bindings cannot be overridden by requests or public bind methods.
Model history cannot currently be used in initial conditions or dose scales;
reset requests are rejected. Raw protocols do not encode these bindings.

Text models use an equivalent block with an explicit simultaneous policy:

```text
dose_history
  last_dose = dose_amount(central) simultaneous first
  elapsed = time_since_dose(central) simultaneous first
  seen = has_dose(central) simultaneous first
```

All four quantities and both policies round-trip through the parser and
formatter without changing the model content identity. Units come from the
selected target and model time unit; this block does not redeclare them.
Dose-relative predicates remain to be implemented. Infusion-history readouts
do not establish matching source-runtime F/duration conventions.

## Dose-relative predicate scheduling API

`DoseHistory::predicate_schedule` plans elapsed-dose comparisons (`lt`, `le`,
`gt`, `ge`) against a fixed, nonnegative offset in normalized model time units.
A new delivery replaces the prior timer; an old threshold after that delivery
is not retained as a switch. Deliveries coinciding with a threshold supply one
stop with distinct old-history pre and new-history post values. The API returns
an initial boundary and ordered subsequent boundaries, each with exact pre/post
readouts and a separate right-hand integration phase. Missing history remains
`None`. Candidate stops are budgeted before coincident times are merged.

`predicate_at` preserves direct floating-point elapsed-time subtraction for
point readouts. This matters at decimal boundaries: native R reports
`1.1 - 1 > 0.1` as true, while an exactly representable elapsed value of 1
remains false under a strict `> 1` comparison. Integration phases use the
scheduled crossing and do not approximate one-sided behavior by nudging time.
The distinction between point values and integration phases is explicit.

Non-finite offsets, negative offsets, collapsed/overflowed crossing times,
equality/inequality predicates and invalid windows are rejected. The model declarations below use these planning APIs for execution.


## Dose-relative model switches (JSON)

A `dose_switches` declaration names a dimensionless predicate for elapsed time
since a delivery. For example:

```json
{"name":"late", "target":"depot", "offset":{"kind":"symbol","name":"delay"},
 "comparison":"gt", "simultaneous":"first"}
```

The offset is a time-valued expression of model parameters. The target must
accept dosing, the simultaneous policy is explicit, and comparisons support
`lt`, `le`, `gt`, and `ge`. Named predicates evaluate to 0 or 1; querying one
before any dose produces a domain error unless a lazy `has_dose` guard avoids
that read. Negative, non-finite and unrepresentable offsets are rejected.

The compiler creates private step bindings at deliveries and elapsed-time
crossings after model/request lag expansion. A newer dose restarts the timer.
RHS evaluation uses the right-hand phase, while every output read recomputes the
exact predicate from history and the requested pre/post side. Parameters may
be rebound without changing the compiled model. Generated changes and storage
consume existing event/output budgets; internal bindings cannot be supplied by
requests. Reset requests and predicate-dependent initial conditions or dose
scales are not qualified and fail explicitly.

Text models use a dedicated block with an explicit simultaneous-dose policy:

```text
dose_switches
  late = time_since_dose(depot) > delay simultaneous first
```

The offset accepts the same parameter expressions as JSON, including arithmetic
and parentheses. Supported operators are `<`, `<=`, `>`, and `>=`; the policy
must be `first` or `last`. Formatting preserves declarations and comparisons.
Native simulation tests qualify the implemented boundary behavior; browser,
Python-wheel and source-runtime infusion equivalence are separate checks.

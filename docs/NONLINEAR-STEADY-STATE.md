# Nonlinear periodic cycle iteration

`steady_state::iterate_periodic_cycle` finds a periodic steady state by
simulating one dosing cycle after another, starting each cycle from where the
last one ended. The caller supplies the starting guess, one periodic protocol,
per-state absolute convergence tolerances, a relative tolerance, maximum cycles
and total work budgets.

Every state must pass the tolerance check on two consecutive cycles. The result
contains the pre-dose steady state, signed residuals, maximum scaled residual
and completed cycle count. A solver error or iteration limit returns no partial
steady state. The total budgets are split equally across cycles.

The model factory must keep the supplied initial state and keep equations,
parameters and dose scaling fixed between cycles. Each cycle
uses the same phase-time window; this does not make a model with explicit
absolute-time dependence periodic. Infusions that run across cycle boundaries
are handled the same way as in analytic linear PK.

Convergence from one starting guess does not prove the steady state is unique
or stable. Slowly converging or unstable fixed points can exhaust the limit.
Integration tolerances must also be tight enough for the accuracy you need.

Tests cover saturable elimination against closed-form results, an
overlapping-infusion equilibrium, convergence from two starting guesses and
nonconvergence under overload.

## Typed operation

`CompiledDocument::iterate_periodic_state(&CycleIterationRequest)` wraps a
normal `RunRequest` with these required controls:

- `time_basis: "cycle_phase"`: equations see the same time window every cycle.
- `absolute_tolerances`: one unit-bearing convergence tolerance per state.
- `relative_tolerance` and `maximum_cycles` (at least two).

Select `diffsol_bdf`. The run's `initial_states` override the model defaults as
the starting guess. Its regimen solver tolerances control integration and are
separate from the convergence tolerances above. Per-state integration
tolerances keep their units and are converted before reaching BDF. Samples,
observations, history, switches, varying covariates and checkpoints are rejected
for this operation. Model and request lag are supported.

The result contains `steady_state` (starting states with units, signed
residuals, scaled residual and identity) plus `cycles_completed`. A different
starting guess can reach a different fixed point, so the identity includes the
starting guess as well as parameters and convergence controls. Schemas are
exported for both request and result.

JSON examples are `conformance/models/synthetic-saturable-periodic.json`
and `conformance/requests/synthetic-saturable-periodic.json`.

## Bindings and qualification

- CLI: `qspquest iterate-periodic MODEL REQUEST.json`.
- Python: `CompiledModel.iterate_periodic_state(request)` returns a dict.
- WASM: `CompiledSimulation.iterate_periodic_state(request_json)` returns JSON.
- Worker: `{kind: 'iterate-periodic', source, request}`.

The installed macOS arm64 wheel passes four cases on CPython 3.11 and 3.12:
three bolus starting guesses and an overlapping-infusion equilibrium. Results
agree with the saturable-elimination closed forms within 2e-7 mg. Concurrent
calls can share a compiled model, and an error does not affect later calls.
Chrome and Firefox agree with native states within 1.560e-10 mg. CLI results
match the library result. Test reports are in
`conformance/evidence/nonlinear-steady-bindings-v1.json`.

Only these test cases and platforms have been checked; Safari and other
platforms have not.

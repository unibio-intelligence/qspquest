# Nonlinear periodic cycle iteration

`steady_state::iterate_periodic_cycle` repeatedly executes the common event
driver with a fresh model bound to each cycle's initial state. The caller
supplies the seed, one phase-periodic protocol, per-state absolute convergence
tolerances, a relative tolerance, maximum cycles and total work budgets.

Two consecutive complete cycle residuals must pass for every state. The result
contains the verified pre-event initial state, signed residuals, maximum scaled
residual and completed cycle count. The last cycle independently checks the
candidate. A solver error or iteration limit returns no partial steady state.
All cycles receive conservative equal partitions of the total budgets.

The factory must preserve the supplied initial vector and keep equations,
parameters and dose scaling fixed. Each cycle uses the same phase-time window;
this is not an assertion that an arbitrary absolute-time model is periodic.
Infusion carryover, including multi-period overlaps and stop-target resets,
uses the same bounded normalizer as analytic linear PK.

Convergence from a seed does not prove uniqueness, global attraction, or
stability. Slowly converging or unstable fixed points can exhaust the limit.
Convergence tolerances check the numerical cycle map; integration tolerances
must also be sufficiently strict for the scientific accuracy required.

Native evidence covers saturable elimination against integrated closed-form
bolus and reset results, overlapping-infusion equilibrium, convergence from
two seeds, overload nonconvergence, invalid state binding, tolerances, budgets
and failure recovery. The typed operation is described below. CLI/Python/browser entry points and evidence are described below.

## Typed operation

`CompiledDocument::iterate_periodic_state(&CycleIterationRequest)` wraps a
normal `RunRequest` with these required controls:

- `time_basis: "cycle_phase"`: equations see the same time window every cycle.
- `absolute_tolerances`: one unit-bearing convergence tolerance per state.
- `relative_tolerance` and `maximum_cycles` (at least two).

Select `diffsol_bdf`. The run's `initial_states` override the model defaults as
the starting guess. Its regimen solver tolerances control integration and are
separate from the convergence tolerances above. Per-state integration tolerance
maps retain their units and reach BDF unchanged after conversion. Samples,
observations, history, switches, varying covariates and checkpoints are rejected
for this operation. Model and request lag remain supported.

The result contains `steady_state` (unit-bearing starting states, signed
residuals, scaled residual and scientific identity) plus `cycles_completed`.
Identity binds the starting guess, parameters, phase-time convention and
convergence controls. A different starting guess can reach a different fixed
point; it must not share the same bound-value identity. Transport request IDs
are excluded. Schemas are exported for both request and result.

Owned JSON examples are `conformance/models/synthetic-saturable-periodic.json`
and `conformance/requests/synthetic-saturable-periodic.json`. Native tests
verify the saturable-elimination closed form from two unit-bearing seeds,
conversion of gram tolerances to milligram state units, identity semantics,
nonconvergence, wrong units, unsupported controls and failure recovery.

## Bindings and qualification

- CLI: `qspquest iterate-periodic MODEL REQUEST.json`.
- Python: `CompiledModel.iterate_periodic_state(request)` returns a dict.
- WASM: `CompiledSimulation.iterate_periodic_state(request_json)` returns JSON.
- Worker: `{kind: 'iterate-periodic', source, request}`.

The installed macOS arm64 wheel passes four cases on CPython 3.11 and 3.12:
three bolus starting guesses and an overlapping-infusion equilibrium. Results
agree with the integrated saturable-elimination closed forms within 2e-7 mg.
Concurrent calls share a compiled model safely; budget, wrong-unit and
nonconvergence errors preserve later calls. Chrome and Firefox agree with
native states within 1.560e-10 mg and recover after budget rejection. CLI results
match the typed library result. Artifact/source hashes and reports are pinned
in `conformance/evidence/nonlinear-steady-bindings-v1.json`.

This qualifies the tested fixtures and platforms. Safari, other native
platforms, broader nonlinear QSP examples, product integration and performance
remain separate work.

# Extend a library model and select a regimen

This tutorial shows how an agent, or a modeller working through the MCP server,
builds a new model from a published one and uses it to compare dosing regimens.
It covers five steps:

1. copying the library model into readable, editable form;
2. adding mechanisms;
3. inspecting the result;
4. selecting a regimen against an exposure target;
5. testing that choice for sensitivity to the added assumptions.

Every step uses MCP tools only. The runnable example is
[`docs/examples/mcp_extend_model.py`](../examples/mcp_extend_model.py).

The example uses the Serkland 2025 rituximab–CD19+ B-cell model
(`qspquest_serkland_2025_rituximab_ms_pkpd`) from the
[public model library](https://github.com/unibio-intelligence/qspquest-models).
The added mechanisms use illustrative parameter values that were not estimated
from data, and the exposure target is illustrative. The regimens it selects are
not dosing recommendations.

```sh
python3 docs/examples/mcp_extend_model.py target/release/qspquest-mcp \
  --library /path/to/qspquest-models
```

## Copy the model

Library archives execute a generated native document whose symbols are often
opaque, such as `source_symbol_5`. Some readable source names, such as `V_l`, are
not legal model identifiers. `fork_model` removes both problems. It renames every
symbol to a legal readable identifier, taking the archive's names where they are
declared, and returns:

- `canonical_source`, the copy as `.qsp` text;
- `name_map`, which links each new name to its native symbol, unit and description.

The mathematics, units, default values and descriptions are unchanged.

With `scenario_id`, the stored request is translated into the new names and run
on both models. For the rituximab preset, all 2,010 output values across its 44
symbols agree exactly (`equivalence.status: identical`). Archive evidence and qualification describe
the original archive only, so the copy, and every later edit, needs its own
checks.

## Add mechanisms

`edit_model` applies typed operations to a model and recompiles the result as
`validate_model` would. An operation can add any of the following:

- a parameter or state, or a new starting value for a state;
- a term or a flux in an equation;
- a peripheral compartment;
- a covariate, or a covariate effect on a parameter;
- a dosing input;
- an output.

An operation can also rename a symbol. Each added parameter needs a description,
which should cite its source or state that it is an assumption. The response
lists every applied operation, the added declarations and the equations that
changed. An invalid operation is refused with a pointer such as
`/operations/2`, and no edit is applied.

The archived rituximab model encodes its single dose in the initial
concentration, C(0) = dose/V, and has no dosing target. The example therefore
first sets the starting concentration to zero with `set_initial`; otherwise each
regimen would start from a state that already holds the archived dose.
`add_dosing` warns when the state it makes dosable has a nonzero starting
value. The example then adds the following:

- **Dosing input.** Doses are given in mg and scaled by `1 / v_l`, because the
  central state is a concentration.
- **Peripheral tissue compartment.** Exchange is concentration-based: the
  central state loses Q/V·(C − C_t) and the tissue gains Q/V_t·(C − C_t). The
  values Q = 0.3 L/d and V_t = 3 L are assumed.
- **Weight scaling.** Volume scales with weight to the power 1 and the linear
  rate constant with weight to the power −0.25, so that linear clearance scales
  with weight to the power 0.75. `edit_model` replaces every use of `v_l` and of
  the rate constant with the new individual parameters, including the dosing
  scale.

## Inspect the result

`get_model` accepts `source` as well as `model_id` and returns the same card for
inline text as for a library model: parameters with units and descriptions,
covariates, dose targets and outputs. The card lists every parameter that can
change the model, including those that reach the equations only through
derived (`individual`) expressions. Descriptions up to 400 characters are
returned in full; a longer description is cut and marked
`description_truncated`.

## Select a regimen against a target

`simulate`, `compare_regimens` and `scan_parameters` take friendly `doses`
(amounts in their declared units, with `every`, `count` and `duration`),
`covariates` and window `readouts`. A covariate may also be given under
`parameters`, and it is routed to the regimen.

The example compares nine maintenance regimens: 250, 500 and 1000 mg every 180,
270 or 365 days, each after 1000 mg induction infusions on days 0 and 14. The
readout is the maximum CD19+ count from day 28 to day 730.
`compare_regimens` returns each member's readouts inline, even when the
trajectories are too large to return and are compacted, so a selection never
depends on cached resources. The regimens that keep CD19+ at or below the
illustrative ceiling of 10 cells/mm³ are then ranked by cumulative dose.

## Find the dose for a target

`find_dose` answers the inverse question directly: which amount of one dose meets a readout
requirement? It holds the schedule, the fixed induction doses, the parameters and the
covariates constant, and bisects the amount between a lower and an upper bound until the
readout is at most (or at least) the threshold. It returns:

- the bracketing amounts and their readout values;
- every evaluation;
- a status. `found` and `found_largest` return a bracket with one passing dose
  and one failing dose. `met_at_lower` and `not_met_at_upper` report whether the
  two endpoints and the midpoint pass. `not_monotonic` means the sampled readouts
  change direction or the midpoint disagrees with both endpoints.

Use bisection when the readout is expected to change in one direction with dose.
The trace describes sampled amounts. Use `scan_parameters` to investigate the
shape of the response when that assumption is uncertain.

The example searches for the 270-day maintenance dose that keeps the window maximum at the
target. Like `compare_regimens`, the result describes a typical subject only.

## Test the selection

A selection that depends on assumed parameters must be tested against them.
`scan_parameters` now accepts the same friendly fields and readouts as
`simulate`, so the window maximum can be scanned directly over the assumed
tissue volume. The same tool scans a covariate, such as body weight, when
`parameter` names it, and the amount of one dose when `dose_index` selects it.
Amounts may be given per body mass (for example `mg/kg`); each is resolved
against the body weight of that run, and the resolved amount is recorded with
the point. A scan of weight under a fixed dose and a scan of a per-kilogram
dose answer different questions, so state which one the selection relies on.

For maintenance regimens, `simulate_steady_state` repeats one dose every
interval until the change still to come, extrapolated from the last three
intervals, is within the tolerance, and reports the steady-state
trough, peak and AUC over the interval, each with its unit. It does not need an
analytic steady-state path, so it applies to target-mediated and other
nonlinear models.

These deterministic comparisons describe a typical subject only. Variability
between subjects and the probability of target attainment require a
population analysis. Python's `CompiledModel.compare_regimens` provides this, with
paired virtual subjects, and a population MCP tool is in development.

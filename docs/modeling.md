# Models and simulation

QSPQuest accepts a text `.qsp` model or a JSON model document. A small
synthetic one-compartment model looks like this:

```text
model synthetic_one_compartment
time_unit = h

parameters
  cl = 0.3 [L/h] log estimated
  v = 3 [L] log estimated

states
  central = 0 [mg]

dynamics
  d/dt(central) = (input_central - ((cl / v) * central))

outputs
  cp = (central / v) [mg/L]

dosing
  central [mg] = 1
end
```

Use `quantity(value, "unit")` for a physical constant in an expression, such as
`quantity(0, "mg/h")` for a zero mass rate or `quantity(500, "mL")` in a volume
expression. These constants retain their unit conversions in smooth forward
sensitivities and fitting. Plain numeric literals are dimensionless, including
zero; they cannot replace a rate with physical dimensions.

Parameters, states, outputs, and doses carry explicit units. The dosing
declaration makes `central` a valid target; `input_central` is the associated
input in the dynamics. The full [text model](../conformance/models/synthetic-one-compartment.qsp)
and equivalent [JSON model](../conformance/models/synthetic-one-compartment.json)
are included as runnable examples.

Simulation requests use the `qspquest.run/v0.1` schema. They declare a
solver, execution budgets, a regimen, sample times, tolerances, and any
parameter or initial-state overrides. A regimen can include boluses,
infusions, repeated doses, covariates and changes, and resets. Use the
[complete synthetic request](../conformance/requests/synthetic-pbpk-24.json)
as a template. `observations` can request a `pre` or `post` value at an event
time; the returned `times` and `sides` preserve that distinction.

```sh
cargo run --release --locked -p qspquest-cli -- run \
  conformance/models/synthetic-pbpk-24.qsp \
  conformance/requests/synthetic-pbpk-24.json > result.json
```

Solver names include `diffsol_bdf`, `diffsol_tsit45`,
`diffsol_esdirk34`, `diffsol_tr_bdf2`, `diffsol_rosenbrock23`,
`diffsol_rodas5p`, and `analytic_linear_pk`. Some model and operation
combinations have narrower support. Select and validate the method for the
model class; the runtime reports unsupported combinations rather than
silently switching methods. Results include named output columns, units,
sample times, event sides, and execution identity. These synthetic examples
are illustrative and make no clinical prediction.

See [fitting](fitting.md) for estimation requests and [browser integration](browser.md)
for local simulation in a Web Worker.

TR-BDF2 checks convergence of the sampled state trajectory by repeating each
interval with tighter local tolerances. It returns the finer trajectory after
agreement within the requested state-error scale. This costs additional solver
work, shares the callback budget, and fails explicitly if six passes cannot
converge. Agreement is numerical evidence rather than a universal global-error
guarantee.

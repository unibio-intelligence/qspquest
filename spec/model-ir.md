# Native model document

`qspquest.model/v0.1` is the JSON representation of the accepted QSPQuest
model language. Its schema version is independent of the package version.
See the [structural schema](schema/model.schema.json), [language](language.md)
and [run/results contract](run-results.md).

## Executable document

The records in `qspquest-core::model` accept:

- `schema`, `model_id`, `description`, `time_unit`;
- parameter name, default quantity, optional unit-bearing inclusive bounds,
  transform, fixed flag, and description;
- numeric covariate name, unit, constant/step interpolation and optional default;
- individual parameter name, expression and optional unit;
- algebraic definition name, expression and optional unit;
- state name, unit, initial quantity or expression, RHS expression, and optional
  dose-amount unit plus a closed scale expression;
- deterministic output name, unit, and closed expression;
- nonnegative-state assertions checked during execution; see [invariants](invariants.md);
- declared stiffness, sparse-Jacobian, gradient and event requirements, checked
  against engine support before compilation; see [requirements](requirements.md).

The complete [synthetic model](../conformance/models/synthetic-one-compartment.json)
has no drug-specific parameters.
The compiler validates units and defaults before returning a `CompiledDocument`.
Bindings contain parameter overrides as quantities. Missing overrides restore
model defaults; rebinding is a replacement binding, not a patch operation.
Derived parameters, initial expressions and dose conversions are recomputed for each binding.
Outputs evaluate from the same binding, state vector, and time as the model.

Identity/log/logit/probit transforms currently constrain parameter domains.
They do not imply an implemented estimation transform. Logit/probit require a
compatible dimensionless quantity strictly between zero and one after unit
conversion; 50 percent is accepted. `fixed` is retained for estimation and does
not prevent a simulation parameter edit.

The compiler reserves `time` and the `input_` prefix for expression bindings.
Identifiers are bounded ASCII snake_case. Parameters, covariates, individual
parameters, definitions and states share one unique namespace. Output labels form a separate
namespace, unique within the result: `central = central [mg]` and
`time = time [h]` are valid. Output labels do not introduce expression bindings
or shadow state/parameter/time symbols. References to another output alone
remain unsupported. Each state exposes `input_<state>` in RHS expressions, with
state/time dimensions. Input forcing must appear explicitly in its equation.
Named administration channels remain a planned extension.

The JSON decoder rejects unknown fields and unknown schema versions. The
JSON entry point has a one-million-byte limit. Current state/binding limits and
expression budgets also apply. Output count is 1–1024, with at most one million
compiled output instructions. Derived output projection checks its result
value budget and returns no partial result if any output fails.

`CompiledSimulation` is a WASM wrapper behind `m0-conformance`, not a released
browser product API. It accepts a model document once, then quantity overrides
and either a raw-number protocol or a unit-bearing regimen with scalar or
per-state absolute tolerances per simulation. It returns output columns
and exposes output names and units. Use named output columns and their declared units when reading a trajectory.

## Validation

The decoder rejects unknown fields and unsupported schema versions. Quantities
carry explicit units. Expressions use a closed node set and cannot execute host
Python, Julia, Rust or JavaScript. Parameter names and references must agree.
Compiled models check dimensions, dependencies and declared requirements.
Observation likelihoods and variability belong to explicit fitting requests;
they are not inferred from deterministic output expressions.

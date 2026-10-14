# QSPQuest model language

The accepted model fields are listed in [the native document](model-ir.md).
Models declare equations, units, inputs and outputs; unsupported blocks and
options fail explicitly. [Invariants](invariants.md) and
[requirements](requirements.md) describe the checked declarations.

## Example

```text
model synthetic_one_compartment
time_unit = h

parameters
  cl = 0.3 [L/h] log bounds(0.01 [L/h], 1 [L/h])
  v = 3 [L] log

states
  central = 0 [mg]

dynamics
  d/dt(central) = input_central - cl / v * central

outputs
  cp = central / v [mg/L]

dosing
  central [mg] = 1
end
```

The dosing line declares amount units and amount-to-state scale. A concentration
state might instead declare `central [mg] = 1 / v`. The optional `input` state
suffix is shorthand for dose amount units equal to state units and scale one;
it cannot coexist with a dosing-block mapping for that state. Infusion forcing
must appear explicitly as `input_<state>` in its RHS in this format.

## Model grammar

The grammar is line-oriented. Blank lines and `#` comments are ignored; hashes
inside double-quoted strings are preserved. Indentation is presentation only.
Block headers and declarations each occupy one line. Block order after the
header is flexible; each block can occur only once. Every state needs exactly
one derivative. Extra targets and content after `end` are errors.

```ebnf
model          = "model", identifier, newline, header*, block*, "end", newline? ;
header         = "time_unit", "=", unit_code, newline
               | "description", "=", string, newline ;
block          = parameters | covariates | individual | states | dynamics | outputs | dosing | requirements | invariants ;
parameters     = "parameters", newline, parameter* ;
parameter      = identifier, "=", quantity, parameter_option*, newline ;
parameter_option = "identity" | "log" | "logit" | "probit"
                 | "fixed" | "estimated"
                 | "bounds", "(", quantity, ",", quantity, ")"
                 | "description", string ;
covariates     = "covariates", newline, covariate* ;
covariate      = identifier, "[", unit_code, "]", ("constant" | "step"),
                 ("default", quantity)?, newline ;
individual     = "individual", newline, derived* ;
derived        = identifier, "=", expression, ("[", unit_code, "]")?, newline ;
states         = "states", newline, state* ;
state          = identifier, "=", (number | expression), "[", unit_code, "]",
                 ("unit", "[", unit_code, "]")?, "input"?, newline ;
dynamics       = "dynamics", newline, derivative* ;
derivative     = "d/dt(", identifier, ")", "=", expression, newline ;
outputs        = "outputs", newline, output* ;
output         = identifier, "=", expression, "[", unit_code, "]", newline ;
dosing         = "dosing", newline, (mapping | lag_mapping)* ;
lag_mapping    = "lag(", identifier, ")", "=", expression, newline ;
mapping        = identifier, "[", unit_code, "]", "=", expression, newline ;
requirements   = "requirements", newline, requirement* ;
requirement    = ("stiff" | "sparse_jacobian" | "gradients"), "=", ("true" | "false"), newline
               | "events", "=", "[", (event_name, (",", event_name)*)?, "]", newline ;
invariants     = "invariants", newline, invariant* ;
invariant      = "nonnegative(", identifier, (",", identifier)*, ")", newline ;
quantity       = signed_number, "[", unit_code, "]" ;
```

A model requires one `time_unit` and `end`; duplicate headers fail. Parameter
transform and estimation options each occur at most once. Bounds occur at most
once. A description option comes last. Quantities require explicit units. Individual expressions may infer their
dimensions; untyped JSON option objects are rejected.

State syntax distinguishes a literal initial quantity from an initial
expression. `x = (1) [1]` is an expression; `x = 1 [1]` is a quantity. An initial
quantity may use different compatible units from its state, for example
`x = 0.001 [g] unit [mg]`. An expression initial condition uses the state unit
and cannot use a different `unit` suffix. Initial expressions may reference
parameters and covariates, not other states or time.

Identifiers use ASCII snake_case, are at most 128 bytes, and cannot be `true`
or `false`. The compiler also reserves `time` and the `input_` prefix. Quoted
strings use JSON-compatible string escaping; this is a string syntax, not a
code or arbitrary-object escape hatch.

## Expressions

```ebnf
expression = or_expression, ("?", expression, ":", expression)? ;
or_expression = and_expression, ("||", and_expression)* ;
and_expression = comparison, ("&&", comparison)* ;
comparison = arithmetic, (("<" | "<=" | ">" | ">=" | "==" | "!="), arithmetic)? ;
```

Arithmetic precedence, from lower to higher:

1. `+`, `-` binary, left associative.
2. `*`, `/`, `%`, left associative. `%` is the specified floored modulo.
3. Unary `+`, `-`, `!`.
4. `^`, right associative.

Thus `-2^2` means `-(2^2)`, `2^3^2` means `2^(3^2)`, and `2^-2` is valid.
Comparisons do not chain: use `a < b && b < c`, not `a < b < c`. Booleans are
`true` and `false`; they are not numbers. Parentheses override precedence.
The ternary conditional and `ifelse` retain the VM's lazy branch semantics.

Function names are the closed expression enum from [expressions](expressions.md); there is
no member access, array indexing, assignment, user-defined function, host
lookup, or dynamic evaluation. Function arity, numeric/boolean types, units,
domains and allowed switching behavior are validated by the shared compiler.
The syntax parser alone does not prove a scientifically executable expression.

Numbers use finite decimal `f64`, optionally with a decimal point and exponent.
An expression may use `quantity(1, "mL")` for a finite constant with explicit
dimensions. Its first argument must be a signed numeric constant and its second
argument a quoted supported unit. The compiler converts the constant to the
surrounding expression's units; an ordinary numeric literal is dimensionless.
Nonfinite literals are rejected. An immediate minus on a numeric literal is
represented as a negative literal; an explicit unary node remains representable
with parentheses around its operand. Formatting preserves this distinction.

## Formatting and diagnostics

`language::parse` returns a document plus a source-location map.
`ParsedModel::compile` invokes `CompiledDocument::compile`; it has no alternate
simulation implementation. `language::format` validates the document first,
then emits one canonical block order with explicit parameter transform/fixed
options, parenthesized expressions, and a dosing block. Initial quantities
retain their original units. Comments and source whitespace are not semantic
model content and are not preserved.

Formatter tests cover idempotence and exact document round trips for the
supported fixtures, including initial expression kind and initial quantity
units. The formatter reparses its output, so text that would exceed parser
limits is rejected rather than emitted as an unreadable canonical document.
More exhaustive property testing is still required before a format-stability
promise.

Diagnostics contain one-based line and UTF-8 byte column, stable code, message,
and fix hint. Parser errors identify the current token or declaration.
Expression checks with a mapped model path identify the declaration's expression
start. Token-level semantic spans, imported-model source maps and runtime VM
path-to-source mapping remain outstanding. Errors without a mapped path use
line 1, column 1; callers must not present that fallback as an exact location.

Limits: model source one million bytes; expression source 2,000 bytes; expression
nodes 256; expression tree depth 32; bounded parser recursion 128. Generated
unit conversions and symbolic derivatives keep their separate compiler budget.

## WASM

The conformance-gated `CompiledSimulation.from_text(source)` compiles this same
path. `CompiledSimulation.format_text(source)` returns canonical text. Errors
are encoded with the diagnostic fields above. Simulation still uses the current
quantity overrides and raw protocol described in [the model document](model-ir.md); browser editor
integration, VM worker lifecycle qualification and the full run/result schema
remain outstanding.

## Derived individual parameters

An optional `individual` block declares deterministic derived parameters, e.g.
`cl = tv_cl * (wt / wt_ref)^0.75`. Dependencies may appear in any declaration
order. The compiler rejects cycles, unknown names and state/time dependencies.
An optional `[unit]` suffix specifies numeric storage units. Without it, the
compiler infers dimensions and uses canonical SI scale one internally. Every
consumer receives the corresponding unit conversion; no unit is guessed from a
name. Base and derived parameters share the 4,096-binding limit.

Each bind recomputes the dependency graph, then initial states, dose scales and
outputs. Derived parameters cannot be overridden directly. Failed rebinding
preserves the previous complete binding. The example expression above requires
explicit `tv_cl`, `wt` and `wt_ref` bindings with compatible units.

## Numeric covariates

`wt [kg] constant` declares a required numeric covariate.
`wt [kg] step default 70 [kg]` also allows fixed-time changes and supplies an
explicit baseline default. Values always carry units at binding; incompatible
units, missing required values and unknown names fail. Covariates have separate
binding maps from parameters and cannot be fitted through parameter overrides.
Names share the model namespace. All derived parameters are recomputed when a
covariate binding changes. `rebind(parameter_overrides)` preserves the current
covariate values while replacing parameter overrides; a new
`bind_with_covariates(parameters, covariates)` uses the supplied complete
covariate map plus declared defaults. See [covariate steps](regimens.md#covariate-steps) for timing.

Only constant and piecewise-constant numeric covariates are implemented. Linear
interpolation and categorical covariates remain unsupported. No missing-value
imputation is inferred. Models with required covariates can compile without
values, but cannot bind or simulate until those values are supplied. Initial
conditions depending on required covariates are domain-checked at binding.

Output labels have their own namespace. They may reuse a state, parameter or
`time` name while expressions still resolve the scientific symbol. Duplicate
output labels and references to output-only names fail. See model-ir.md.

## Algebraic definitions

A `definitions` block accepts `name = expression [optional_unit]`. Definitions
may refer to parameters, numeric covariates, individual parameters, states,
`time`, RHS inputs and other definitions, including forward references. Names
share the scientific namespace; cycles and unknown symbols fail compilation.

Definitions are expanded before dimensional checking and symbolic Jacobian
construction. They remain expressions evaluated at the current state and time.
An optional unit constrains dimensions; it does not insert a numeric rescaling
into the expression. Consumers convert physical values to their own declared
units. For example, `concentration = central / v [ug/L]` can feed an output in
`mg/L` without changing the underlying amount/volume calculation.

Initial and dosing-scale expressions may use definitions only when their
expanded expression uses bindings available in those contexts. Individual
parameter expressions cannot refer to definitions. Outputs cannot access RHS
inputs, even indirectly. Dynamic branches and nonsmooth crossings retain the
existing restrictions after expansion; hidden aliases do not bypass them.
Lazy branches remain lazy when evaluated.

Source expressions retain the 256-node/32-depth limit. Each expanded expression
is limited to 16,384 nodes and depth 128; aggregate expansion work is limited to
one million copied nodes, with at most 4,096 definitions. Expansion duplicates
subexpressions rather than caching their runtime values. This bounded initial
implementation does not promise common-subexpression optimization.

## Allowed delivery modes

A dosing declaration may restrict deliveries: `central [mg] bolus = 1` or
`central [mg] infusion = 1`. Omitting mode words enables both. Explicit lists
must be nonempty and unique; unknown mode words fail parsing. The canonical
formatter preserves restrictions. JSON dosing records use `modes: ["bolus"]`
or `modes: ["infusion"]`; omission defaults to both. Schema validation checks
member shape; the compiler also checks nonempty/unique semantics.

A prohibited delivery returns `unsupported` before solver dispatch, including
raw document-protocol execution. Resets are independent state events and are not
classified as administrations. Restrictions do not alter dose scale, F, lag,
repeat count or infusion duration semantics.


### Fixed switch block

The optional `switches` block declares `name = time_expression`. See
[declared fixed switches](regimens.md#declared-fixed-switches) for parameter-only
time expressions, 0/1 phase values, event ordering and work limits. The formatter
preserves declarations and the JSON schema uses the same expression tree.


A switch declaration may also use `name = time < threshold` (or `<=`, `>`,
`>=`). This declares a pointwise time predicate rather than a plain phase.
The threshold is a parameter-only expression with time dimensions. See the
[strict/inclusive predicate contract](regimens.md#strict-and-inclusive-time-predicates)
for exact readout values and one-sided integration.


## Parameter-bound model lag

Within `dosing`, write `lag(central) = delay` for a time-valued parameter or
expression of parameters and constant covariates. The target must already accept dosing, either through
its dosing declaration or the state's `input` shorthand. Each target may have
one lag declaration. The formatter preserves the expression and the parser
records its source location. See [model-bound dosing lag](regimens.md#model-bound-dosing-lag) for additive request lag,
repeated administrations and binding semantics.

```text
dosing
  central [mg] = 1
  lag(central) = delay
```


## Dose-history declarations

The `dose_history` block declares named readouts of delivered administrations:

```text
dose_history
  previous = dose_amount(central) simultaneous first
  last_time = dose_time(central) simultaneous first
  elapsed = time_since_dose(central) simultaneous first
  seen = has_dose(central) simultaneous first
```

`simultaneous first` or `simultaneous last` is required on every declaration.
The target must be a named state with dosing enabled. The parser retains a
source location under `dose_history.<name>`. Quantity names are a closed set;
unknown quantities, missing policies, additional arguments and trailing tokens
are errors. Formatting preserves the declaration order and explicit policy.

`dose_amount` uses target input units before bioavailability/state scaling;
`dose_time` and `time_since_dose` use model time units; `has_dose` is 0 or 1.
Use lazy guarded expressions when the run may include times before dosing.
An unguarded read of missing numeric history returns an error. See
[regimen history semantics](regimens.md#model-history-bindings) for event sides,
lag, resets and the remaining dose-relative switching scope.

## Linear PK declaration

A `linear_pk` block declares one to three disposition compartments and an
optional first-order absorption depot. It replaces explicit `states` and
`dynamics`; the compiler constructs the equations and amount dosing targets.

```text
linear_pk
  amount_unit = mg
  clearance = cl
  exchange_clearances = q1, q2
  compartment central volume vc = 0 [mg]
  compartment peripheral volume vp = 0 [mg]
  compartment tissue volume vt = 0 [mg]
  depot depot absorption ka = initial_depot_amount
```

The first `compartment` is central; each subsequent compartment corresponds to
one entry in `exchange_clearances`. A one-compartment model can omit that line
or supply an empty list. The depot is optional and may appear before or after
the compartment declarations. Volumes, clearances and absorption reference
named parameters, constant covariates or `individual` bindings with volume,
volume/time and inverse-time units. Derived coefficient chains must depend
only on parameters and constant covariates.

Initial values are either explicit quantities such as `10 [mg]`, or expressions
such as `initial_depot_amount` whose dimensions must equal `amount_unit`.
Dimensional constants in expressions must be named parameters. Coefficient
names and initialization expressions are validated by the normal compiler.
Duplicate scalar declarations, incomplete records, unknown options and more
than one depot are rejected. Extra ODE states or separately written dynamics
cannot override the generated mechanism.

Formatting preserves the declaration rather than expanding it into equations.
JSON and text therefore share the same scientific model content identity.
The run request selects `analytic_linear_pk` or `diffsol_bdf`; backend selection
is an execution choice, not model text. See [linear PK kernels](../docs/LINEAR-PK-KERNELS.md) for the
numerical method and validation limits.

Linear PK states also accept the ordinary `dosing` block. For example,
`depot [mg] bolus = available_fraction` plus `lag(depot) = absorption_delay`
creates the depot's `dosing_override` record. Units, expression dependencies,
mode restrictions and lag validation use the same compiler as ordinary states.

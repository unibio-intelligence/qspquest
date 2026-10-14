# Expressions

Expressions form the checked numerical language used by native models.
Code and tests live in `qspquest-core::expression`, `qspquest_modeling::compile`, and
`crates/qspquest-modeling/tests/expressions.rs`. Unit checking is implemented in the model compiler. The language parser records
declaration source locations; full token-level semantic spans remain pending. This API alone does not validate
a pharmacology model or qualify it for scientific use.

## Closed expression nodes

The JSON discriminator is `kind`. Unknown fields and unknown enum variants are
rejected. There is no arbitrary source-code field or host function lookup.

| Node | Fields | Shape |
|---|---|---|
| literal | value | finite dimensionless number or boolean |
| quantity | quantity | finite number with an explicit unit |
| symbol | name | ASCII snake_case identifier |
| unary | operator, arguments | positive/negative/not; exactly 1 child |
| binary | operator, arguments | add/subtract/multiply/divide/power/modulo; exactly 2 children |
| compare | operator, arguments | lt/le/gt/ge/eq/ne; exactly 2 numeric children |
| boolean | operator, arguments | and/or; at least 2 boolean children |
| conditional | arguments | condition, true branch, false branch |
| call | name, arguments | one closed function enum; arity checked |

Quantity constants use `{"kind":"quantity","quantity":{"value":1,"unit":"mL"}}`.
The model compiler converts them before checking dimensions, so `max(volume,
quantity(1, "mL"))` can clamp a volume declared in litres. Ordinary numeric
literals are dimensionless. The raw unitless expression API rejects physical
quantity constants; use `Program::compile_with_units` or the model compiler.
Canonical model text writes these constants as `quantity(1, "mL")`.

Literal, quantity and symbol nodes omit `arguments`. Compound nodes carry their explicit
children. Canonical text formatting uses the same nodes. See
[the model document](model-ir.md) and [expression schema](schema/expression.schema.json).

## Evaluation

- Symbols resolve once to dense runtime input indices. Values are read on each
  execution and are never folded into the instruction stream.
- Instructions write f64 registers. Numeric and boolean kinds are checked at
  compilation; boolean registers use 0/1 internally.
- Conditionals and and/or short-circuit via forward jumps. Inactive branches do
  not evaluate. No common-subexpression optimization crosses a branch guard.
- Reusable scratch buffers avoid allocating registers on every evaluation.
- libm implements transcendentals. Non-finite inputs, invalid domains, overflow,
  and singular derivatives report typed errors with expression paths.
- User expressions have at most 256 nodes and depth 32. Generated derivative
  expressions have separate bounds of 16,384 nodes and depth 128. The text
  parser additionally enforces a 2,000-byte expression limit.
- State/time/input-dependent conditions, rounding and modulo currently return
  `unsupported`; declared time-event integration is still pending. Parameter-only
  conditions are accepted. Differentiating a condition's controlling parameter
  returns `unsupported` rather than assuming a fixed boundary.

## Operators

| Operators | Evaluation | Symbolic derivative status |
|---|---|---|
| +, -, *, / | numeric; division by zero errors | product/quotient rules |
| power | negative base requires integral exponent; zero base requires positive exponent | literal exponent rule or separate base/exponent partials; finite zero-boundary limits retained |
| exp, log, log10, sqrt | usual real domains; finite outputs required | supported; sqrt derivative errors at zero |
| sin, cos, tan, tanh | libm; finite outputs required | supported |
| modulo | divisor > 0; result in [0, divisor) | da - floor(a/b) db away from wraps; wrap points error |
| abs, min, max | supported; min/max take 2+ arguments | pending nonsmooth convention qualification |
| floor, ceil | parameter-only argument | pending boundary qualification |
| hill | max(x,0)^n/(k^n+max(x,0)^n), n>0, k>0; stable logarithmic evaluation | pending boundary qualification; no approximate singular Jacobian enabled |
| comparisons / not / and / or | typed booleans, lazy and/or | not numeric derivatives |
| conditional / ifelse | selected branch only, matching branch types | selected branch when condition is independent of differentiation variable |

Derivatives retain primal domain evaluation even when algebraic simplification
produces zero. An invalid original model expression cannot become valid merely
because its derivative is zero. Conservative Jacobian sparsity and ODE runtime integration use these expressions.
The expression and model tests exercise rebinding, lazy branches, original-domain
preservation, symbolic derivatives and strict JSON decoding.

## Output-only guards

A model output may branch on its sampled state or time. The selected branch is
evaluated lazily at the requested observation; an inactive invalid logarithm or
division is not evaluated. Output guards do not create solver events or change
the differential equations. State selection still follows the observation's
pre/post event side, while time comparisons use the exact observation time.

Definitions are checked for size, shape, cycles and units before expansion.
Their consumers then determine whether dynamic switching is permitted. A guard
used only in an output is accepted; reusing that definition in dynamics still
requires a qualified event contract. Dimension checks and finite-value checks
remain active. Output gradients across guard boundaries are not qualified by
this readout feature.

The core `log` function remains strict. An importer may explicitly represent a
source runtime's alternative behavior as a visible guarded expression, retaining
that choice in its translation metadata. No logarithm flooring is automatic.

### Power derivatives at zero

Zero raised to a positive real exponent is zero. For a variable exponent, the
base partial is `p*x^(p-1)` with explicit `p=0` and `p=1` branches; this avoids
the spurious division by zero from `x^p*p/x`. At zero the base partial is zero
for `p>1`, one for `p=1`, and is rejected as nonfinite for `0<p<1`. These are
limits from the real domain on the nonnegative side. The exponent partial at
zero is zero for every positive exponent; it does not evaluate `log(0)`.
The original expression is checked before a derivative result is returned:
`0^0`, negative powers of zero and negative bases with fractional exponents
remain domain errors. Negative bases with integer exponents support the base
partial, but varying their exponent is not a real-valued neighborhood and is
rejected. This does not qualify higher derivatives or moving branch boundaries.

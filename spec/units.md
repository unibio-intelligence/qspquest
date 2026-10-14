# Units: implemented subset and lowering contract

This describes the Rust unit and expression checker. Units are carried by
the [model document](model-ir.md), [model language](language.md),
[run request](run-results.md) and [regimen](regimens.md).

## Codes and dimensions

Unit codes follow a restricted case-sensitive subset of
[UCUM](https://ucum.org/ucum). This is not a complete UCUM implementation.
Multiplication uses `.`, division uses `/`, and integer exponents attach to
symbols or groups: `mg.kg-1.d-1`, `mg/(kg.d)`, `cm2`. Multiplication and division
have equal precedence and associate left to right. Parentheses disambiguate
compound denominators. Whitespace, `^`, annotations, affine units,
non-integer numeric scale factors, and implicit multiplication are rejected.
Positive decimal integer factors are supported: `1000000000/L`, `1000.mg`.
A dot always means multiplication (`1.5` means `1` times `5`); decimal and
scientific-notation literals are not unit factors. Numeric factors must fit
`u128` and be nonzero. Bare numeric powers such as `2+10` are outside this
subset; grouped powers such as `(1000)-1` are supported.

Supported atoms:

| Family | Codes |
|---|---|
| Dimensionless | `1`, `%`, `rad`, `sr` |
| Mass | `kg`, `g`, `mg`, `ug`, `ng`, `pg`, `fg` |
| Length | `m`, `cm`, `mm`, `um`, `nm` |
| Volume | `L`, `l`, `dL`, `mL`, `uL`, `nL`, `pL` |
| Time | `s`, `ms`, `us`, `ns`, `min`, `h`, `d`, `wk`, `a` |
| Amount of substance | `mol`, `mmol`, `umol`, `nmol`, `pmol`, `fmol` |
| International activity | `[iU]`, `[IU]` |
| Opaque biological counts | `[cell]`, `[genome]` (QSPQuest extensions) |
| Other base units | `K`, `A`, `cd` |
| Derived SI units | `Hz`, `Bq`, `N`, `Pa`, `J`, `W`, `C`, `V`, `Ohm`, `S`, `F`, `Wb`, `T`, `H`, `lm`, `lx`, `Gy`, `Sv`, `kat` |

Use `nmol/L`, not the unsupported alias `nM`; use ASCII `ug`, not `µg`.
`Cel` and the bare spelling `IU` are unsupported. International activity uses
the explicit bracketed codes `[iU]` or `[IU]`; prefixed spellings are not yet
supported (use exact integer factors, such as `(1000).[iU]`). An unknown code returns
`unknown_unit`; incompatible dimensions return `unit`.

Internally the seven SI dimensions use kg, m, s, mol, K, A, cd. An eighth,
opaque international-activity exponent prevents accidental conversion of IU to
dimensionless, mass, amount, or catalytic activity quantities. IU is not an SI
dimension; this is a stricter internal compatibility rule for procedure-defined
units. Ratios of matching activity units cancel and volume/time scale changes
remain exact. The model author must ensure activity quantities refer to the same
substance and assay: the type checker does not encode assay identity or establish
cross-substance equivalence. No automatic conversion to `kat` or `umol/min` is
provided. See [UCUM biomedical units](https://ucum.org/ucum#section-Biomedical-Units).

`[cell]` and `[genome]` are QSPQuest extensions with separate opaque count
exponents. They distinguish biological counts from dimensionless normalized
coordinates and from each other, mass, molar amount and international activity.
No Avogadro, molecular-mass or genome-to-cell conversion is inferred. Matching
count units still require matching analytes and compartments; a genome count
does not establish productive nuclear entry or distinguish vector from host
genomes without that metadata. Continuous models may use expected counts and
densities with fractional values.

For example, `[cell]/mm3` and `[cell]/uL` have the same scale, while
`[cell]/L` differs by a factor of one million. The rate coefficient
`mm3/([cell].d)` multiplied by a `[cell]/mm3` density has unit `1/d`.
Bare `cell`, `genome` and `vg` spellings are unsupported. Explicitly map a
source's declared entity and spelling rather than silently treating it as `1`.

Planning records keep their eight `dimensions` coordinates for SI and activity.
When a count is involved, `entities` records nonzero `cell` or `genome`
exponents, for example `{"cell":1}`. Empty entity maps are omitted; readers of
older records default them to empty. This metadata reports dimensional
preparation and does not establish numerical or scientific qualification.

### Run a synthetic cell-density model

Use the matching QSPQuest Python build with count-unit support. In an environment
where `qspquest` is installed, run this complete example:

```python
from math import exp
from qspquest import CompiledModel

model = {
    "schema": "qspquest.model/v0.1", "model_id": "synthetic_cell_decay",
    "time_unit": "h",
    "parameters": [{"name": "k", "default": {"value": 0.25, "unit": "1/h"},
                    "bounds": None, "transform": "log", "fixed": False}],
    "states": [{"name": "cells", "unit": "[cell]/uL",
                "initial": {"kind": "quantity", "quantity": {"value": 100, "unit": "[cell]/uL"}},
                "rhs": {"kind": "unary", "operator": "negative", "arguments": [
                    {"kind": "binary", "operator": "multiply", "arguments": [
                        {"kind": "symbol", "name": "k"}, {"kind": "symbol", "name": "cells"}]}]},
                "dosing": None}],
    "outputs": [{"name": "density", "unit": "[cell]/uL",
                 "expression": {"kind": "symbol", "name": "cells"}}],
}
request = {
    "schema": "qspquest.run/v0.1", "solver": "diffsol_bdf",
    "regimen": {"end": {"value": 4, "unit": "h"}, "samples": [],
                "observations": [{"time": {"value": t, "unit": "h"}, "side": "post"} for t in [0, 4]],
                "administrations": [], "rtol": 1e-8, "atol": 1e-10},
}
result = CompiledModel(model).run(request)
density = next(column for column in result["outputs"] if column["name"] == "density")
assert density["unit"] == "[cell]/uL"
assert abs(density["values"][-1] - 100 * exp(-1)) < 1e-4
print(density["values"])
```

The analytic final density is approximately 36.787944 cells/uL. This is a
synthetic first-order decay model, not a qualified biological model. A genome
quantity is incompatible with the cell state, even when its numeric value and
volume unit match. An `unknown_unit` error for `[cell]` indicates an engine build
without count-unit support; use the matching build instead of replacing the
unit with `1`.

The normalization basis is internal; it does not claim that UCUM uses the same
base-unit convention.
Unit scales and conversion ratios are reduced positive rational numbers with
128-bit numerator and denominator. Overflow returns an error. There is no
fallback to approximate metadata. Runtime values and inserted conversion
constants use finite `f64`, so numerical results use tolerance comparisons.

Codes are bounded to 256 ASCII bytes, nesting to 16, explicit exponents to
±32, and resulting dimension exponents to ±128. Intermediate rational overflow
can reject an otherwise algebraically reducible expression. These are current
resource limits, not physical restrictions.

## Expression rules

- Bare numeric literals are dimensionless, including zero. Typed zero initial
  quantities are available through the model binding API.
- Addition, subtraction, modulo, numeric comparison, min and max require
  compatible dimensions. Conditional alternatives must have the same type and
  dimensions. Both alternatives are checked, but only the selected one runs.
- Multiplication and division combine dimensions. A dimensional base requires
  a literal integer power. A dimensionless base permits a dimensionless power.
- `sqrt` requires even dimension exponents; `abs` preserves dimensions.
- Exponential, logarithmic, trigonometric, rounding and hyperbolic functions
  require dimensionless arguments in this subset.
- `hill(x,n,k)` requires compatible x/k dimensions and dimensionless n, with a
  dimensionless result. Its derivative support remains separately restricted.
- Boolean operations require boolean values; quantities are not truthy values.
- Mass and amount of substance are separate dimensions. A conversion such as
  `mass_concentration / molar_mass` requires an explicit molar-mass binding.

Allometric clearance uses `tv_cl * (wt / wt_ref)^0.75` with `wt_ref` carrying a
weight unit. `wt / 70` cannot replace a unit-bearing weight reference.

Lowering inserts conversions at symbol loads inside the original expression
branches, performs arithmetic in the internal basis, and converts the result
to its requested unit. Source limits and dynamic-switch restrictions are
checked before lowering. Inserted conversions receive a bounded expanded
compiler budget; user expressions cannot opt into that budget. Generated
Jacobians differentiate the normalized numeric equations, retaining primal
domain checks.

## Model binding

`UnitCompiledModel` is an in-memory lowering API, not the public ModelIR.

- Time must have time dimensions. Each state declares its numeric unit.
- Initial quantities convert to the state unit; initial expressions must return
  that dimension and may use binding symbols.
- A state's RHS must have state/time dimensions. Its named infusion input has
  the same dimensions and is included explicitly in the equation.
- A dosing mapping declares its amount unit. Its scale expression must have
  state/amount dimensions, such as `1 / v` for a concentration state.
- Bindings carry declared units and optional inclusive bounds. Bounds are
  converted once at compilation; supplied quantities convert before checking
  bounds and recomputing initial values and dose scales.
- Rebinding is atomic. Failed unit conversion, bounds validation, or expression
  evaluation leaves the previous problem unchanged.

The lower-level `Protocol` contains raw numbers. Callers must use
model time units, each target's declared dose unit, and state units for absolute
error tolerances. The public run request instead carries explicit quantities
and supports [per-state absolute tolerances](regimens.md#state-tolerances).
Supply one quantity in the appropriate state unit for each state when using
that contract; a scalar tolerance is interpreted in each state's storage unit.

## Archive source coordinates

An archive may declare `unchanged_source_numeric_values` while the native model
stores a parameter or state in unit `1`. A separate archive source label such as
`1/h`, `mg/L` or `pg/mL` describes that source coordinate. It does not change the
native dimension or qualify a physical conversion. Inspect the input binding's
native unit, declared source label and accepted request coordinate before editing.
Scientific names and suffixes are not used to infer missing source units.

For example, an alias with accepted coordinate `1` and declared source label
`1/h` refuses `{"value":0.2,"unit":"1/h"}`. The unit error identifies the requested
and accepted units; its suggestion includes the declared label and the archive's
coordinate mode. Supplying `{"value":0.2,"unit":"1"}` explicitly uses the model's
numeric coordinate and preserves 0.2. This does not establish that the entire
model has been dimensionally qualified. Choose a qualified physical interface
when a physical conversion is needed.

A failed friendly-input conversion preserves the original submitted request.
Source-coordinate hints accompany that quantity refusal; unrelated errors retain
their own guidance. A missing source label remains `undeclared`, even when an
alias name resembles a physical quantity. Neither an error hint nor an accepted
numeric-coordinate edit changes an archive's scientific qualification.

Native unit refusals from planning or execution also include a declared
`unchanged_source_numeric_values` mode when applicable. Their native error code,
message and location remain intact. This context is available for canonical
native inputs as well as readable aliases; solver and unrelated errors do not
receive a source-coordinate unit hint.

## Evidence

Native tests cover exact conversion ratios, dimensional expression failures,
explicit molar-mass conversion, allometric scaling, analytic one-compartment
bolus/infusion solutions, changed time/state/dose units, bounds, and atomic
rebinding. Chrome and Firefox conformance use the same unit-aware expression
compiler in WASM. Browser expression checks alone do not qualify unit-bearing
ODE model authoring, which still needs the public IR and browser model wrapper.

`a` denotes the fixed Julian year of exactly 365.25 days (31,557,600 seconds),
following the [UCUM year definition](https://ucum.org/ucum). It is a duration
unit, not a calendar-date operation. Gregorian/tropical years remain outside
this implemented subset.

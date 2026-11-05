# Numerical qualification

43 of the 47 catalog scenarios are checked the same way: native and WASM must
agree within a scale-aware budget, and each must match an independent ODE
reference. The other four keep their original check: native and WASM must agree
within an absolute `1e-9`. These checks cover only the recorded inputs and
methods, not biological validity.

The 31 scenarios outside Liu and Rajwade used to require native and WASM to
agree within an absolute `1e-9`. That only held because an earlier patched
Diffsol produced bit-identical results on both targets. Upstream Diffsol uses
the platform `pow` in its step-size controller. Native and WASM `pow` can
differ in the last bit, so the two targets can take slightly different steps
and drift apart by about the solver tolerance. Neither result is less
accurate. 27 of these scenarios now use the same scale-aware budget and
independent reference as Liu and Rajwade. Bit identity is still reported, as a
separate diagnostic.

Four scenarios keep the absolute `1e-9` check: the three DiffDose TGI scenarios
and Yuan `mda_mb_468_high_affinity_6_86nm_et8`. Their requests declare
`rtol = 1e-8`. At that tolerance their native and WASM results are 0.29 to 0.31
of the scale-aware budget away from the independent reference, above the 0.25
limit, so they do not qualify for the scale-aware check. They are bit-identical
across targets today. If a future solver change breaks that, they fail.

Yuan `mda_mb_468_duobody_6_86nm_et8` had the same accuracy shortfall at
`rtol = 1e-8`. Model version `2.0.2` tightens only that scenario's request to
`rtol = 1e-9` and `atol = 1e-11`. Its equations, doses, grid and budgets are
unchanged.

`output-bounds.json` freezes a bound for every output, with its name, stored
unit, model hash and request hash. Changing the inputs invalidates the bounds.
The bounds were reviewed and are never regenerated to absorb a new native/WASM
mismatch.

The predeclared numerical target is:

```
abs(native - wasm) <= absolute_i + relative_i * max(abs(native), abs(wasm))
relative_i = 1e-6
absolute_i = max(original_source_atol_floor_i, 1e-9 * reference_peak_abs_i)
```

The characteristic scale comes from the independently integrated reference
trajectory. State readouts use the original declared state absolute tolerance
as their floor. Derived readouts use the minimum original state floor, a
conservative choice in these stored coordinates. The selected Liu requests
use tighter integration tolerances; the frozen acceptance budget remains the
one chosen before the new reference results. The other scenarios keep their
declared request tolerances, so their floor is the request's state absolute
tolerance, as for Rajwade. Constant readouts are also checked
against their independently evaluated values with absolute error <= 1e-9.

Input document and request hashes are also checked independently of the result
metadata. Negative controls verify that matching but wrong native/WASM results,
stale inputs and mutated dose sides are rejected.

Each implementation must consume no more than 25% of its output budget against
the reference, and refinement of the reference must consume no more than 1%.
Accuracy uses `absolute_i + relative_i * abs(reference)`. Identity, event times,
event sides, output order, names and units must match exactly. Nonfinite values,
missing scenarios and stale hashes fail qualification. Each output must contain
one value per requested observation; matching truncated results, shifted grids,
and Boolean values are rejected before native/WASM agreement is scored.

These are numerical accuracy targets for the declared archive equations. The
adapter stores these models in source coordinates with native unit `1`;
dimensional qualification remains false. Output labels referring to source
units do not establish a physical-unit qualification. Solver agreement does
not establish paper reproduction, clinical accuracy or biological validity.
Adaptive solver tolerances control local error; they do not by themselves
bound cross-implementation trajectory error. See the
[SciPy tolerance documentation](https://docs.scipy.org/doc/scipy/reference/generated/scipy.integrate.solve_ivp.html).

The reference implementation translates the archive expression graph directly
to Python and generates a separate sparse Jacobian by SymPy differentiation.
SciPy Radau and SuperLU integrate the result independently of the QSPQuest
VM, derivative implementation, BDF solver and faer LU. Integration is split
at boluses, infusion starts and ends, checkpoints and the archive's
time-predicate switches, preserving pre/post event semantics. Fixed-duration
infusions enter through the state's explicit `input_<state>` term. Hour and
day time units are supported. Unsupported reference constructs fail closed.

No archive declares a physical conservation invariant. The checker
therefore makes no mass-conservation claim. It checks finite states/readouts,
constant readouts, scaled dose jumps and exact continuity of nondosed states
using explicit pre/post-dose diagnostic requests. Pointwise checks include all
outputs. Source readouts are marked important.

Results with upstream Diffsol `883d9a0` on Linux ARM64 (native and WASM), with
the largest fraction of the output budget used in each model. All 47 scenarios
pass.

| Model | Check | Passed | Native/WASM agreement | Reference refinement | Native or WASM vs reference |
| --- | --- | ---: | ---: | ---: | ---: |
| Liu | Scale-aware | 13/13 | 0.074 | 0.0063 | 0.084 |
| Rajwade | Scale-aware | 3/3 | 0.000002 | 0.00042 | 0.14 |
| DiffDose BiTE | Scale-aware | 3/3 | 0.058 | 0.00066 | 0.18 |
| DiffDose IDR | Scale-aware | 3/3 | 0 | 0.00018 | 0.028 |
| DiffDose TGI | Absolute `1e-9` | 3/3 | 0.0000049 | — | — |
| Membrane ternary binding | Scale-aware | 5/5 | 0 | 0.00030 | 0.057 |
| Target-gated cargo | Scale-aware | 6/6 | 0.00000002 | 0.0041 | 0.23 |
| Yuan CD3x5T4 | Scale-aware | 10/10 | 0.14 | 0.00039 | 0.24 |
| Yuan CD3x5T4 high-affinity MDA-MB-468 | Absolute `1e-9` | 1/1 | 0 | — | — |
| Required maximum | | | 1 | 0.01 | 0.25 |

Bit identity holds in 23 of 47 scenarios.

Liu version `2.1.2` uses separate vector and protein BBB/BCSFB reflection
parameters. Its simulation requests declare relative tolerance `1e-12` and
per-state absolute tolerances of approximately `1e-19` in source coordinates.
The fixed acceptance bounds apply only to the model and requests whose hashes
are recorded in `output-bounds.json`. Changing the model, parameters, dosing,
observations or solver tolerances requires a new numerical check against those
bounds. Rajwade retains its declared simulation tolerances.

Arithmetic probes with identical IEEE-754 input bits show that the platform
`pow` and dense short-history products can differ between native and WASM.
Replacing `pow` with a portable implementation restores bit identity for the
31 scenarios outside Liu and Rajwade, but upstream Diffsol does not offer that
option.

Run the qualification from the repository root:

```sh
sh scripts/build-mcp.sh
cargo run -p qspquest-mcp --release --locked --features sandbox --example reference_dump -- numerical-inputs --wasm
cargo run -p qspquest-mcp --release --locked --features sandbox --example reference_dump -- event-inputs --wasm --dose-sides
python -m pip install numpy==2.3.3 scipy==1.16.2 sympy==1.14.0
OPENBLAS_NUM_THREADS=1 python bindings/mcp/scripts/independent_reference.py numerical-inputs numerical-references --analytic-jac --policy bindings/mcp/qualification/output-bounds.json
python bindings/mcp/scripts/check_numerical_qualification.py numerical-inputs numerical-references bindings/mcp/qualification/output-bounds.json numerical-qualification.json --event-inputs event-inputs
cargo run -p qspquest-mcp --release --locked --features sandbox --example parity_audit > parity.json
```

CI retains the inputs, references, errors and per-output scores. The absolute
`1e-9` bit-identity diagnostic is reported separately and is not a pass
criterion. Other architectures must run the same checks before claiming
qualification.

Large hosted models still run as managed asynchronous jobs.

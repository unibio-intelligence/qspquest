# Numerical qualification

The numerical evidence passes numerical parity for all 47 catalog scenarios. The
16 Liu and Rajwade scenarios additionally pass an independent ODE accuracy
check. The other 31 retain their existing absolute regression bounds; this
work does not add independent biological validation for them. These numerical checks apply only to the declared inputs and methods.

`output-bounds.json` freezes a bound for every output, with its name, stored
unit, model hash and request hash. Changed scientific inputs invalidate the
policy. Bounds are reviewed evidence, rather than values regenerated from a
new native/WASM mismatch.

For Liu and Rajwade, the predeclared numerical target is:

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
one chosen before the new reference results. Constant readouts are also checked
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
adapter stores these two models in source coordinates with native unit `1`;
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
at boluses, checkpoints and the archive's time-predicate switches, preserving
pre/post event semantics. Unsupported reference constructs fail closed.

Neither archive declares a physical conservation invariant. The checker
therefore makes no mass-conservation claim. It checks finite states/readouts,
constant readouts, scaled dose jumps and exact continuity of nondosed states
using explicit pre/post-dose diagnostic requests. Pointwise checks include all
outputs, including the 27 Liu and six Rajwade source readouts.

| Independent check | Liu, 13 scenarios | Rajwade, 3 scenarios | Required maximum |
| --- | ---: | ---: | ---: |
| Reference refinement / output budget | 0.006322 | 0.000424 | 0.01 |
| Native or WASM reference error / output budget | 0.155335 | 0.239175 | 0.25 |
| Source-readout reference error / output budget | 0.075680 | 0.116657 | 0.25 |

Liu version `2.1.2` uses separate vector and protein BBB/BCSFB reflection
parameters. Its simulation requests declare relative tolerance `1e-12` and
per-state absolute tolerances of approximately `1e-19` in source coordinates.
The fixed acceptance bounds apply only to the model and requests whose hashes
are recorded in `output-bounds.json`. Changing the model, parameters, dosing,
observations or solver tolerances requires a new numerical check against those
bounds. Rajwade retains its declared simulation tolerances.

Arithmetic probes first verify identical IEEE-754 input bits. Across 14 cases,
including independently generated Liu/Rajwade Jacobians, sparse LU and sparse
products matched exactly between native and WASM. Dense short-history products
showed reduction differences. The correction is confined to fixed column-order
history accumulation under DiffSol's opt-in `deterministic` feature. Sparse LU,
its pivoting and the weighted error norm remain unchanged. This is bounded
probe evidence, not a proof of identical arithmetic on every architecture.

Run the qualification from the repository root:

```sh
sh scripts/build-mcp.sh
cargo run -p qspquest-mcp --release --locked --features sandbox --example reference_dump -- numerical-inputs --wasm
cargo run -p qspquest-mcp --release --locked --features sandbox --example reference_dump -- event-inputs --large --wasm --dose-sides
python -m pip install numpy==2.3.3 scipy==1.16.2 sympy==1.14.0
OPENBLAS_NUM_THREADS=1 python bindings/mcp/scripts/independent_reference.py numerical-inputs numerical-references --analytic-jac --policy bindings/mcp/qualification/output-bounds.json
python bindings/mcp/scripts/check_numerical_qualification.py numerical-inputs numerical-references bindings/mcp/qualification/output-bounds.json numerical-qualification.json --event-inputs event-inputs
cargo run -p qspquest-mcp --release --locked --features sandbox --example parity_audit > parity.json
```

CI retains the inputs, references, errors and per-output scores. The unchanged
absolute `1e-9` determinism diagnostic is reported separately; it passes 47/47
in the local ARM64 candidate. It is not substituted for independent accuracy.
Other architectures must run the same checks before claiming qualification.

Hosted large models continue to route to managed asynchronous execution.
Interactive execution latency, heavy routing, managed job admission and job
completion remain distinct measurements. This numerical change retains the
compiler, sparsity, selected-output and async-routing performance work.

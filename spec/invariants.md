# Runtime invariants

The implemented invariant is strict state nonnegativity:

```text
invariants
  nonnegative(depot, central)
```

JSON uses `invariants: [{"kind": "nonnegative", "states": ["depot", "central"]}]`.
Only declared states may appear. Empty lists, unknown names, duplicate states
within/across declarations and unknown fields are rejected. Invariants are
part of the model content and affect its hash.

The driver checks the initial state, every accepted solver step, requested dense
output, and states before event boundaries and after each ordered reset and
bolus. Computing outputs from a supplied state checks that state too, and
runs with fixed covariates keep the check. A violation returns typed `invariant` with time, state path and value; the caller
receives no partial trajectory. Negative resets cannot be hidden by later
same-time boluses or resets.

Values must be finite and greater than or equal to zero; negative zero passes.
Initial states, reset values, analytic propagation results and states supplied
directly for output evaluation are checked exactly.

A numerical integrator controls each state only to within its absolute
tolerance. States produced by numerical integration (accepted steps and
interpolated observations), and the same states carried into later event
boundaries and boluses, therefore pass when they are at least `-atol_i`, the
absolute tolerance of state `i` requested for the run. Passes that sampled
accuracy control or TR-BDF2 refinement solve with a smaller tolerance are
checked against the same requested value. A failure message reports the
tolerance it was checked against. The reported values are not changed: there is
no clamping, projection or solver retry policy, and a value between `-atol_i`
and zero appears in the result as computed. Larger negative values fail. Fix
the model or choose suitable solver settings rather than hiding the failure.

Checks intentionally exclude internal Newton trial states and rejected solver
steps. They do not prove nonnegativity between accepted states and requested
dense-output points. Declaring this invariant does not enable unqualified
Hill derivatives, state-triggered events or positivity-preserving integration.

The `Model::check_state` hook defaults to no constraints for independently
implemented Rust models. Document-backed models implement the declared checks.
General conservation expressions, arbitrary boolean invariants and model test
blocks are not yet supported.

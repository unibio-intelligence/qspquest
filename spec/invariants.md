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
There is no implicit tolerance, clamping, projection or solver retry policy.
Tiny negative numerical values therefore fail too. Fix the model or choose
suitable solver settings rather than hiding the failure.

Checks intentionally exclude internal Newton trial states and rejected solver
steps. They do not prove nonnegativity between accepted states and requested
dense-output points. Declaring this invariant does not enable unqualified
Hill derivatives, state-triggered events or positivity-preserving integration.

The `Model::check_state` hook defaults to no constraints for independently
implemented Rust models. Document-backed models implement the declared checks.
General conservation expressions, arbitrary boolean invariants and model test
blocks are not yet supported.

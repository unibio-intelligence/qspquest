# Runtime invariants

The implemented invariant is strict state nonnegativity:

```text
invariants
  nonnegative(depot, central)
```

JSON uses `invariants: [{"kind": "nonnegative", "states": ["depot", "central"]}]`.
Only declared states may appear. Empty lists, unknown names, duplicate states
within/across declarations and unknown fields are rejected. Invariants form
part of the canonical model content and execution identity.

The driver checks the initial state, every accepted solver step, requested dense
output, and states before event boundaries and after each ordered reset and
bolus. Direct deterministic output projection checks the supplied state too.
Fixed-covariate execution preserves the check through its bound-model wrapper.
A violation returns typed `invariant` with time, state path and value; the caller
receives no partial trajectory. Negative resets cannot be hidden by later
same-time boluses or resets.

Values must be finite and greater than or equal to zero; negative zero passes.
There is no implicit tolerance, clamping, projection or solver retry policy.
Tiny negative numerical values therefore fail too. Users should improve the
model or qualify appropriate solver settings rather than silently changing
mass or hiding the failure.

Checks intentionally exclude internal Newton trial states and rejected solver
steps. They do not prove nonnegativity between accepted states and requested
dense-output points. Continuous constraint guarantees would require additional
root/extremum handling. Declaring this invariant does not enable unqualified
Hill derivatives, state-triggered events or positivity-preserving integration.

The `Model::check_state` hook defaults to no constraints for independently
implemented Rust models. Document-backed models implement the declared checks.
General conservation expressions, arbitrary boolean invariants and model test
blocks remain to be implemented.

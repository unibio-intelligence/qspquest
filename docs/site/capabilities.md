# Supported interfaces

This matrix is generated from the [capability catalog](../../spec/capabilities.json).
It lists declared interface contracts. Build features, deployment configuration,
model requirements and request admission determine whether a workflow can execute.
Source declarations do not establish scientific qualification for a release.

| Workflow | Rust | Python | R | CLI | Direct WASM | Browser Worker | Sandbox ABI | MCP |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Python likelihood fit with fitted errors and M3 | — | Requires `analysis` | — | — | — | — | — | — |
| Python SAEM with 1–8 effects | — | Requires `analysis` | — | — | — | — | — | — |
| Subject bootstrap | — | Requires `analysis` | — | — | — | — | — | — |
| Cancel offline local job | — | — | — | — | — | — | — | Declared |
| Paired regimen comparison | — | Requires `analysis` | — | — | — | — | — | Declared |
| Compile model | — | — | — | — | — | — | Requires `sandbox-abi` | — |
| Numerical moving-event sensitivities | — | Requires `analysis` | — | — | — | — | — | — |
| Bounded experiment archive export | — | Requires `interchange` | — | — | — | — | — | — |
| Bounded SBML export | — | Requires `interchange` | — | — | — | — | — | — |
| Sampled-grid exposure summaries | — | Requires `analysis` | — | — | — | — | — | — |
| Conditional fit diagnostics | — | Requires `analysis` | — | — | — | — | — | — |
| Gamma–Poisson count fitting | Declared | Declared | Declared | Declared | — | — | — | — |
| Native individual/pooled Gaussian fit | Declared | Declared | Declared | Declared | Requires `m0-conformance` | — | Requires `sandbox-abi` | Declared |
| Native bounded population fit | Declared | Declared | Declared | Declared | Requires `m0-conformance` | — | Requires `sandbox-abi` | Declared |
| Probit-normal binomial fitting | Declared | Declared | Declared | Declared | — | — | — | — |
| Native bounded scalar Gaussian fit | Declared | Declared | — | Declared | — | — | Requires `sandbox-abi` | Declared |
| Format complete decision responses | — | — | — | — | — | — | — | Declared |
| Read governed artifact | — | — | — | — | — | — | — | Requires `managed` |
| Query capability and model blockers | — | — | — | — | — | — | — | Declared |
| Read governed job | — | — | — | — | — | — | — | Requires `managed` |
| Inspect offline local job | — | — | — | — | — | — | — | Declared |
| Inspect model interface | — | — | — | — | — | — | — | Declared |
| Evaluate initial patient quantities | — | — | — | — | — | — | — | Declared |
| Retrieve stored scenario request | — | — | — | — | — | — | — | Declared |
| Bounded experiment archive import | — | Requires `interchange` | — | — | — | — | — | — |
| Bounded SBML import | — | Requires `interchange` | — | — | — | — | — | — |
| Parse model without compilation | — | — | — | — | — | — | Requires `sandbox-abi` | — |
| Bounded periodic-state iteration | Declared | Declared | — | Declared | Declared | — | — | — |
| Retrieve synthetic model template | — | — | — | — | — | — | — | Declared |
| Native Morris screening | Declared | Declared | Declared | Declared | Declared | — | Requires `sandbox-abi` | Requires `analysis` |
| Prepare native fitting workflows | Declared | — | — | Declared | — | — | — | Declared |
| Prepare Gaussian observation objective | Declared | — | — | — | — | — | — | — |
| Plan periodic BDF iteration | Declared | — | — | Declared | — | — | — | — |
| Prepare bounded Morris screening | Declared | — | — | Declared | — | — | — | — |
| Plan periodic linear steady state | Declared | — | — | Declared | — | — | — | — |
| Prepare a native run | Declared | — | — | Declared | — | — | — | Declared |
| Prepare bounded scalar fitting | Declared | — | — | Declared | — | — | — | Declared |
| Prepare a native parameter scan | Declared | — | — | Declared | — | — | — | Declared |
| Prepare augmented sensitivities | Declared | — | — | Declared | — | — | — | — |
| Preview governed job | — | — | — | — | — | — | — | Requires `managed` |
| Profile likelihood | — | Requires `analysis` | — | — | — | — | — | — |
| Progress, cancellation and restart checkpoints | — | Requires `analysis` | — | — | — | — | — | — |
| Resume offline local job | — | — | — | — | — | — | — | Declared |
| Native parameter scans | Declared | Declared | Declared | Declared | Declared | — | Requires `sandbox-abi` | Declared |
| Search model catalog | — | — | — | — | — | — | — | Declared |
| Native fixed-event sensitivities | Declared | Declared | — | Declared | Requires `m0-conformance` | — | — | — |
| Native simulation | Declared | Declared | Declared | Declared | Declared | Declared | Requires `sandbox-abi` | Declared |
| Periodic linear PK steady state | Declared | Declared | — | Declared | Declared | — | Requires `sandbox-abi` | Requires `analysis` |
| Submit offline local job | — | — | — | — | — | — | — | Declared |
| Submit governed population fit | — | — | — | — | — | — | — | Requires `managed` |
| Submit governed scan | — | — | — | — | — | — | — | Requires `managed` |
| Submit governed simulation | — | — | — | — | — | — | — | Requires `managed` |
| Summarize supplied sampled profile | — | — | — | — | — | — | — | Declared |
| Validate model | — | — | — | — | — | — | Requires `sandbox-abi` | Declared |
| VPC and prediction-corrected VPC | — | Requires `analysis` | — | — | — | — | — | — |
| Wait for governed job | — | — | — | — | — | — | — | Requires `managed` |

A dash means the declared interface does not expose that workflow.
`python_analysis` uses the optional analysis dependencies; it is distinct from
native Python sensitivity-based fitting. Native and Python SAEM have different
estimator contracts. `wasm_direct` browser classes are absent with `sandbox-abi`.
The public Worker contract declares BDF simulation; direct bindings expose their
documented additional operations and solvers.

## Solvers, estimators and execution modes

The catalog keys each expanded record by operation, interface, estimator, solver,
target and execution mode. Native simulation has seven explicit solver choices.
Sensitivities and native sensitivity-based fits accept BDF or Rodas5P.
Periodic linear steady state requires `analytic_linear_pk`; bounded cycle
iteration requires BDF. MCP withholds Rosenbrock23. Analytic execution requires
a compatible `linear_pk` declaration; availability does not validate a model.

Read the [fitting solver guide](fitting-solvers.md), [Python guide](../PYTHON-USER-GUIDE.md)
and [MCP guide](../../bindings/mcp/README.md) for their complete input contracts.
MCP hosted user-source execution requires a configured sandbox. Effective limits
and active tools are reported by `qspquest://capabilities`.

Read `qspquest://capability-registry` for the active MCP target and mode.
Use `get_capability` with the six key fields and an optional native model/version
and stored scenario to inspect blockers. `executable: null` means the exact request
still needs normal tool admission and execution. Other interfaces and targets
remain unassessed by this server. Governed records describe the transport build;
platform model readiness and execution target require an authorized preview.

## Qualification

Catalog records carry `untested` scientific status. They describe implementation
and interface exposure; they do not qualify the current artifact, clinical model
or study design. Scientific claims require matching release, operation, solver,
target, model and evidence scope. A successful synthetic example is limited to
its tested numerical contract. Model unit and fitting readiness remain separate.

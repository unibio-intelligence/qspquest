# Supported interfaces

This table is generated from the [capability catalog](../../spec/capabilities.json).
"Declared" means the interface exposes the workflow, "Requires" names the build
feature it needs, and a dash means it is not exposed. Whether a run succeeds still
depends on the build, deployment, model and request. Nothing here is a scientific
qualification.

| Workflow | Rust | Python | R | CLI | Direct WASM | Browser Worker | Sandbox ABI | MCP |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Bounded experiment archive export | — | Requires `interchange` | — | — | — | — | — | — |
| Bounded experiment archive import | — | Requires `interchange` | — | — | — | — | — | — |
| Bounded periodic-state iteration | Declared | Declared | — | Declared | Declared | — | — | — |
| Bounded SBML export | — | Requires `interchange` | — | — | — | — | — | — |
| Bounded SBML import | — | Requires `interchange` | — | — | — | — | — | — |
| Cancel offline local job | — | — | — | — | — | — | — | Declared |
| Compile model | — | — | — | — | — | — | Requires `sandbox-abi` | — |
| Conditional fit diagnostics | — | Requires `analysis` | — | — | — | — | — | — |
| Evaluate initial patient quantities | — | — | — | — | — | — | — | Declared |
| Format complete decision responses | — | — | — | — | — | — | — | Declared |
| Gamma–Poisson count fitting | Declared | Declared | Declared | Declared | — | — | — | — |
| Inspect model interface | — | — | — | — | — | — | — | Declared |
| Inspect offline local job | — | — | — | — | — | — | — | Declared |
| Native bounded population fit | Declared | Declared | Declared | Declared | Declared | — | Requires `sandbox-abi` | Declared |
| Native bounded scalar Gaussian fit | Declared | Declared | — | Declared | — | — | Requires `sandbox-abi` | Declared |
| Native fixed-event sensitivities | Declared | Declared | — | Declared | Declared | — | — | — |
| Native individual/pooled Gaussian fit | Declared | Declared | Declared | Declared | Declared | — | Requires `sandbox-abi` | Declared |
| Native Morris screening | Declared | Declared | Declared | Declared | Declared | — | Requires `sandbox-abi` | Requires `analysis` |
| Native parameter scans | Declared | Declared | Declared | Declared | Declared | — | Requires `sandbox-abi` | Declared |
| Native simulation | Declared | Declared | Declared | Declared | Declared | Declared | Requires `sandbox-abi` | Declared |
| Numerical moving-event sensitivities | — | Requires `analysis` | — | — | — | — | — | — |
| Paired regimen comparison | — | Requires `analysis` | — | — | — | — | — | Declared |
| Parse model without compilation | — | — | — | — | — | — | Requires `sandbox-abi` | — |
| Periodic linear PK steady state | Declared | Declared | — | Declared | Declared | — | Requires `sandbox-abi` | Requires `analysis` |
| Plan periodic BDF iteration | Declared | — | — | Declared | — | — | — | — |
| Plan periodic linear steady state | Declared | — | — | Declared | — | — | — | — |
| Prepare a native parameter scan | Declared | — | — | Declared | — | — | — | Declared |
| Prepare a native run | Declared | — | — | Declared | — | — | — | Declared |
| Prepare augmented sensitivities | Declared | — | — | Declared | — | — | — | — |
| Prepare bounded Morris screening | Declared | — | — | Declared | — | — | — | — |
| Prepare bounded scalar fitting | Declared | — | — | Declared | — | — | — | Declared |
| Prepare Gaussian observation objective | Declared | — | — | — | — | — | — | — |
| Prepare native fitting workflows | Declared | — | — | Declared | — | — | — | Declared |
| Preview governed job | — | — | — | — | — | — | — | Requires `managed` |
| Probit-normal binomial fitting | Declared | Declared | Declared | Declared | — | — | — | — |
| Profile likelihood | — | Requires `analysis` | — | — | — | — | — | — |
| Progress, cancellation and restart checkpoints | — | Requires `analysis` | — | — | — | — | — | — |
| Python likelihood fit with fitted errors and M3 | — | Requires `analysis` | — | — | — | — | — | — |
| Python SAEM with 1–8 effects | — | Requires `analysis` | — | — | — | — | — | — |
| Query capability and model blockers | — | — | — | — | — | — | — | Declared |
| Read governed artifact | — | — | — | — | — | — | — | Requires `managed` |
| Read governed job | — | — | — | — | — | — | — | Requires `managed` |
| Resume offline local job | — | — | — | — | — | — | — | Declared |
| Retrieve stored scenario request | — | — | — | — | — | — | — | Declared |
| Retrieve synthetic model template | — | — | — | — | — | — | — | Declared |
| Sampled-grid exposure summaries | — | Requires `analysis` | — | — | — | — | — | — |
| Search model catalog | — | — | — | — | — | — | — | Declared |
| Subject bootstrap | — | Requires `analysis` | — | — | — | — | — | — |
| Submit governed population fit | — | — | — | — | — | — | — | Requires `managed` |
| Submit governed scan | — | — | — | — | — | — | — | Requires `managed` |
| Submit governed simulation | — | — | — | — | — | — | — | Requires `managed` |
| Submit offline local job | — | — | — | — | — | — | — | Declared |
| Summarize supplied sampled profile | — | — | — | — | — | — | — | Declared |
| Validate model | — | — | — | — | — | — | Requires `sandbox-abi` | Declared |
| VPC and prediction-corrected VPC | — | Requires `analysis` | — | — | — | — | — | — |
| Wait for governed job | — | — | — | — | — | — | — | Requires `managed` |

The Python column combines the native binding and the optional analysis helpers
(`python_analysis`, installed with the `analysis` extra). Native and Python SAEM
are different estimators. Direct WASM browser classes are absent in `sandbox-abi`
builds, and the browser Worker runs BDF simulation only.

## Solvers

Native simulation offers BDF, TSIT45, ESDIRK34, TR-BDF2 and analytic linear PK.
Sensitivities and sensitivity-based fits accept BDF. Periodic linear steady state requires `analytic_linear_pk`
and a compatible `linear_pk` declaration; bounded cycle iteration requires BDF.
See the [fitting solver guide](fitting-solvers.md),
[Python guide](../PYTHON-USER-GUIDE.md) and [MCP guide](../../bindings/mcp/README.md)
for inputs.

## Check a workflow in MCP

`qspquest://capabilities` reports the active tools and limits, and
`qspquest://capability-registry` the server's target and mode. `get_capability`
takes the six key fields (operation, interface, estimator, solver, target and
execution mode), optionally with a model, version and scenario, and lists
blockers. `executable: null` means the request still needs normal admission.
Hosted MCP runs user-supplied models only with a sandbox build. Governed records
describe the transport; platform readiness needs an authorized preview.

## Qualification

Catalog records have scientific status `untested`. They describe what is
implemented and exposed, not whether a release, model or study is qualified. A
passing synthetic example covers only its tested numerical check.

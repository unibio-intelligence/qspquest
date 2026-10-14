# Bounded parameter scans

`CompiledDocument::scan` and `qspquest scan MODEL REQUEST.json` evaluate an
explicit ordered list of 1–25 parameter points. The shared run defines regimen,
solver, initial-state overrides, covariates and base parameter values. Each
point has a unique 1–128 byte ID and a parameter quantity map that overrides the
base map. Defaults still supply unspecified model parameters. There is no
implicit grid expansion. The fixture `conformance/requests/synthetic-scan.json`
shows three clearance values for the owned one-compartment model.

Requests use `qspquest.scan/v0.1`; results use `qspquest.scan-result/v0.1`.
Every point returns its ID, merged explicit parameter map and full simulation
result with execution identity. The parameter map preserves supplied units;
normalized bound values are represented by the simulation identity. The scan
hash binds the full ordered request and scan algorithm. Points with equal
scientific inputs may have equal simulation identities while retaining distinct
point IDs. Point order is preserved.

All points undergo the ordinary run's static preparation before any simulation:
parameter and initial-state binding, solver eligibility, dose lags and repeats,
switches, covariate phases, observation mapping, units, domains and storage limits.
Execution consumes these prepared regimens in point order. Trajectory-dependent
domain checks and numerical convergence still occur during execution.
Each point receives the smaller of the run budget and its equal share of
the total scan budget, independently for callbacks, events and numeric output
values. This conservative reservation bounds aggregate work and retained numeric
results. Unused budget from one point is not transferred to another. Execution
is sequential within one scan; independent scans can run concurrently.

Any point failure aborts the operation without returning partial results. The
error expression identifies its point. CLI files are never modified and retain
the one-million-byte request limit (JSON model documents permit four million
bytes). Generated request/result schemas describe
structure; runtime checks enforce point limits, units, domains and budgets.

## Run the synthetic clearance scan

This example compares clearance while holding the 3 L compartment volume and
6 mg bolus fixed. Use the release source with Rust 1.98.1 installed, and run from
its top-level directory. The build uses the release's locked dependencies:

```sh
cargo build --release --locked -p qspquest-cli
./target/release/qspquest scan conformance/models/synthetic-one-compartment.json conformance/requests/synthetic-scan.json > scan.json
```

The complete model and request are included in the release. `scan.json` contains
three ordered results, `clearance_0`, `clearance_1`, and `clearance_2`, for 0.3,
0.6, and 1.2 L/h. Each result has five rows: time-zero pre-dose and post-dose,
then 1, 2, and 4 h. The `cp` output is 0 mg/L before dosing and 2 mg/L immediately
after dosing. At 4 h it is approximately 1.34064, 0.89866, and 0.40379 mg/L,
respectively, following `cp(t) = 2 * exp(-clearance / 3 * t)`.

Use the row's side when comparing time-zero values. Supply parameter values as
quantities with compatible units and retain unique point IDs. Increasing point
count without increasing total budgets reduces each point's reserved budget;
an exhausted budget rejects the complete scan. Static preparation can reject
an invalid delayed dose in a later point before the first point integrates.
Passing preparation still leaves trajectory-domain and convergence checks for
execution. These synthetic comparisons do not establish clinical suitability.

Qualification covers independent analytic trajectories, mixed-unit overrides,
ordered results, distinct bound identities, malformed points, excess/empty point
lists, aggregate-budget exhaustion, recovery, CLI parity and independent schema
validation. Governed platform admission remains open. This scan operation does not itself estimate global sensitivity indices.

## Python and browser bindings

Python `CompiledModel.scan(request)` returns a dict and `scan_json(request)`
returns JSON. Both use the versioned scan envelope, release the GIL during
native execution and permit sharing a compiled model between independent scans.
For 100 parameter sets, submit four scans of 25 points and concatenate their
ordered `results`. Give each scan a total budget large enough to reserve the
requested per-point budget: for 25 points, multiply each run budget by 25.
Scanning does not change the chosen solver, tolerances, observations or outputs.
An isolated ABI3 macOS ARM64 wheel passes on Python 3.11.15 and 3.12.8, including
three concurrent scans and 3-point/25-point analytic cases.

WASM `CompiledSimulation.scan(request_json)` returns result JSON. The current
conformance worker accepts `kind: "scan"` with model text and the same envelope;
it rejects separate callback/protocol/regimen overrides. Cancellation terminates
the worker, and the next request rebuilds compilation state. Chrome 154 and
Firefox 154 pass the same 3-point and 25-point cases with exact trajectory parity
against native results, plus budget/malformed input failures, cancellation and
recovery. All emitted results pass independent schema validation. Safari and
cross-platform wheel qualification remain open. These are correctness checks,
not performance measurements or managed product-release qualification.

# Parameter scans

`CompiledDocument::scan` and `qspquest scan MODEL REQUEST.json` evaluate an
explicit ordered list of 1–25 parameter points. The shared run defines regimen,
solver, initial-state overrides, covariates and base parameter values. Each
point has a unique 1–128 byte ID and a parameter quantity map that overrides the
base map. Defaults still supply unspecified model parameters. Points are not
expanded into a grid. The fixture `conformance/requests/synthetic-scan.json`
shows three clearance values for the synthetic one-compartment model.

Requests use `qspquest.scan/v0.1`; results use `qspquest.scan-result/v0.1`.
Every point returns its ID, its merged parameter map (in the units
supplied) and the full simulation result. Results stay in point order.

All points are validated before any simulation starts: parameters, initial
states, solver, dose lags and repeats, switches, covariates, observations,
units, domains and storage limits. Checks that depend on the trajectory, and
solver convergence, still happen while each point runs.
Each point receives the smaller of the run budget and its equal share of
the total scan budget, independently for callbacks, events and numeric output
values. Unused budget from one point is not passed to another. Points run one
after another; separate scans can run concurrently.

If any point fails, the scan returns no partial results; the error names the
failing point. The CLI never modifies files and keeps the one-million-byte
request limit (JSON model documents permit four million bytes). The generated
schemas describe structure; point limits, units, domains and budgets are
checked at run time.

## Example: clearance scan

This example compares three clearance values while holding the 3 L compartment volume and
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
quantities with compatible units and keep point IDs unique. Adding points
without increasing the total budget reduces each point's share; if any point exhausts
its budget, the whole scan is rejected. These synthetic comparisons do not establish
clinical suitability.

Tests compare results with analytic trajectories, including mixed-unit
overrides. A scan does not compute global sensitivity indices.

## Python and browser bindings

Python `CompiledModel.scan(request)` returns a dict and `scan_json(request)`
returns JSON. Both use the versioned scan envelope, release the global interpreter lock (GIL) during
native execution and permit sharing a compiled model between independent scans.
For 100 parameter sets, submit four scans of 25 points and concatenate their
ordered `results`. Give each scan a total budget large enough to reserve the
requested per-point budget: for 25 points, multiply each run budget by 25.
Scanning does not change the chosen solver, tolerances, observations or outputs.

WebAssembly (WASM) `CompiledSimulation.scan(request_json)` returns result JSON. The current
conformance worker accepts `kind: "scan"` with model text and the same envelope;
it rejects separate callback/protocol/regimen overrides. Canceling terminates
the worker, and the next request starts a new one.

# QSPQuest MCP server

The server lets an MCP client search, inspect, simulate and fit QSPQuest models
over local stdio or stateless Streamable HTTP. Start with one of these:

- [Agent and Docker quickstart](../../docs/tutorials/agents-docker.md): a
  container with a synthetic model, connected to your own agent client.
- [First-run journeys](../../docs/tutorials/first-run-journeys.md): first runs
  with the wheel, CLI or this image, with saved CSV, replay inputs and identity.
- [Library fitting guide](../../docs/tutorials/library-fitting.md): archived
  synthetic fitting; inspect readiness before choosing an estimator.

Models load from the public model library, from a local directory or a pinned
HTTPS release. Many library models keep their source's numeric scale, so check
a model's units before interpreting results. The bundled OMEX models are
offline numerical test fixtures: their units are unverified
(`source_scale_unqualified`) and they have not been scientifically reviewed.

## Build and connect

You need Rust 1.98.1, a native compiler and the
[public model library](https://github.com/unibio-intelligence/qspquest-models).
From the QSPQuest source root:

```sh
cargo build -p qspquest-mcp --release --locked
```

This stdio build needs no WASM toolchain. The build helper selects optional
features:

```sh
sh scripts/build-mcp.sh --features ""
sh scripts/build-mcp.sh --features analysis
# Browser assets require wasm-bindgen-cli 0.2.129 on PATH.
sh scripts/build-mcp.sh --features sandbox,widget
```

`analysis` adds steady-state and Morris tools. `sandbox` builds the isolated
WASM ABI and `widget` the browser bindings; both need the wasm32 target and are
the helper's defaults. `managed` needs its deployment configuration.

The container image includes `sandbox,widget` by default. For a native image
without them, build with `docker build -f bindings/mcp/Dockerfile --target public
--build-arg MCP_FEATURES= -t qspquest-mcp:native .`

### Use a local library

Extract the model-library release so its root contains `catalog.json` and
`models/`, then pass an absolute path:

```sh
export QSPQUEST_LIBRARY_DIR="$(cd /path/to/qspquest-models && pwd)"
target/release/qspquest-mcp --library-dir "$QSPQUEST_LIBRARY_DIR"
```

The process waits for a client on stdio; exit with Ctrl-C. The library is
checksum-verified on load, and a corrupt library prevents startup. Directories
of `.qsp` source models also work.

Claude Desktop configuration (replace both absolute paths):

```json
{"mcpServers":{"qspquest":{"command":"/absolute/path/qspquest/target/release/qspquest-mcp","args":["--library-dir","/absolute/path/qspquest-models"]}}}
```

### Download a pinned library release

Use an immutable HTTPS release archive and the SHA-256 published separately by
its trusted publisher:

```sh
# Set URL and SHA256 to the release asset and its independently verified digest.
export QSPQUEST_LIBRARY_RELEASE_URL="$URL"
export QSPQUEST_LIBRARY_RELEASE_SHA256="$SHA256"
unset QSPQUEST_LIBRARY_DIR
target/release/qspquest-mcp
```

The archive must be a gzip tar with `catalog.json` at its root. The server
verifies it before serving and caches it in `~/Library/Caches/qspquest-models`
(macOS), `$XDG_CACHE_HOME/qspquest-models` or `~/.cache/qspquest-models`
(Linux), or `%LOCALAPPDATA%\qspquest-models` (Windows). Set
`QSPQUEST_LIBRARY_CACHE` to use another private directory. The cache root must
be owned by the server user (mode `0700` on Unix); shared, linked or unsafe
roots are refused.

Every start re-checks the cached archive and extracted library against the
pinned digest, so later starts work offline. A failed refresh keeps the
current library; to upgrade, change both URL and digest. Older cache entries
without a saved tarball are not trusted: choose a new cache directory. A local
`--library-dir` overrides the download.

### Search

Call `search_models` with `{"query":"rituximab"}`, then `get_model`,
`get_scenario_request` and `simulate` with the returned model and scenario.
Check the model card, units, result status and event sides. Search ranks
models by matching terms and can return models not yet cleared for simulation
or fitting. `filters` accepts exact `model_id`, `version`, `license` and
`has_output` (a native output name or alias); `tag` is a case-insensitive
substring. Results report `total` and `next_offset`.

## Tools and resources

Tools: `search_models`, `get_model`, `get_scenario_request`, `validate_model`,
`model_template`, `get_capability`, `plan_run`, `format_response`,
`simulate`, `get_patient_parameters`, `summarize_profile`,
`compare_regimens`, `scan_parameters`,
`fit_scalar`, `fit_individual`, `fit_population`. For long offline work, start
with private `--jobs-dir` and `--artifact-dir` directories and use
`submit_local_job`, `get_local_job`, `cancel_local_job` and `resume_local_job`
(see the [local job tutorial](../../docs/tutorials/mcp-deploy.md#offline-mcp-jobs)).

Read `qspquest://capabilities` for active tools, features, permissions, limits,
library identity and qualification scope. Each tool advertises an
`outputSchema`; `qspquest://schema/mcp-result` and
`qspquest://schema/capabilities` describe the shared schemas. Tool annotations
are hints; execution and export policies still apply.

`qspquest://documentation` indexes the model, request, regimen, unit and error
guides, the schemas and a synthetic model/request pair. Each entry has a MIME
type and SHA-256, and heading fragments select sections. The
[offline resource example](../../docs/tutorials/mcp-results.md#navigate-offline-documentation)
checks every indexed resource through a real client.

`get_model` returns a compact card; `detail: full` adds equations and complete
metadata, with a chunked `full_model_details_resource` when large. See the
[model-card tutorial](../../docs/tutorials/mcp-results.md#inspect-model-cards-and-retrieve-full-details). For known
output names and scenario IDs, `get_model.selection` returns declarations,
column mappings and scenario requests in one response (1 to 32 items, 40,000
bytes); see the
[selection example](../../docs/tutorials/mcp-results.md#select-exact-outputs-scenarios-and-inspection-resources).

## Results

Every tool result includes `structuredContent.contract`
(`qspquest.mcp-result/v1`) with identities, status and completeness. Status is
`success`, `scientific_failure`, `unsupported`, `async_required` or
`request_failure`; a nonconverged fit is `scientific_failure` even if `isError`
is false. Completeness is `inline_complete`, `resource_linked` or
`summary_only`. A schema-valid result still needs its units, status and
convergence checked. The runnable
[result-resource tutorial](../../docs/tutorials/mcp-results.md#read-a-complete-mcp-simulation-result)
covers each contract.

Compact simulation results are previews with event sides, sampled extrema and
a grid trapezoidal integral, not extrapolated AUC or NCA. Use `detail: full` or
read `full_result_resource` for the requested grid. Local stdio caches eight
results of up to 4 MB for 15 minutes; hosted results have no cache link. Only
selected outputs are evaluated, so domain checks on unselected readouts are
skipped.

For larger or longer-lived results, start with `--artifact-dir PATH` (an empty
private directory). Results are kept as immutable JSON for 24 hours, across
restarts, up to 128 MB each, 256 artifacts and 1 GB in total. See the
[retained-result example](../../docs/tutorials/mcp-results.md#retain-complete-results-across-local-server-restarts).

Related workflows:

- `get_patient_parameters` evaluates outputs at the starting state, before any
  dose; see the
  [initial-value example](../../docs/tutorials/mcp-results.md#inspect-initial-patient-quantities).
- `scheduled_boundary_receipt` shows forcing changes, events and observation
  sides; see the
  [scheduled-boundary tutorial](../../docs/tutorials/mcp-results.md#inspect-scheduled-boundaries-and-observation-sides).
- Numerical, conservation, source-fidelity and clinical status are reported
  separately; see the
  [scientific assessment workflow](../../docs/tutorials/mcp-results.md#inspect-separate-scientific-assessments).
- `format_response` maps computed columns into requested arrays with explicit
  units, times and `pre`/`post`/`both` sides; see the
  [response guard tutorial](../../docs/tutorials/mcp-results.md#format-complete-decision-responses).

## Requests

Quantities are `{value, unit}`. For a stored scenario use
`{"model":{"scenario":{"model_id":"qspquest_diffdose_2026_idr_benchmark",
"scenario_id":"higher_exposure"}}}`. `parameters` and `initial_states` are
quantity maps using the names and units from `get_model`. Friendly dosing
supports target, amount, at, every/count, duration, bioavailability and lag.
Observation sides are explicit. Friendly runs need a positive end; defaults are
BDF, start zero, `rtol: 1e-8`, `atol: 1e-10`, no doses and 101 evenly spaced
samples. Check `notes` and `effective_request` for defaults.

To replay a native request, pass the returned `effective_request` as `request`.
Top-level `parameters` and `initial_states` replace only their native keys;
top-level solver, end, times and doses replace those fields. An overridden
preset is no longer covered by the preset's checks. Two names for the same
input are refused.

Readout thresholds can be a number in the output's unit or a quantity such as
`{"value":5000,"unit":"ug/L"}` for an output in `mg/L`. Dose targets may
declare an input mass unit; mg, g and ug are converted before the dose scale.
For doses, thresholds, timed changes and archive aliases, explicit unit `1`
passes a value unchanged in the stored unit; canonical names and native requests
treat `1` as dimensionless (see
[units](../../spec/units.md#archive-source-coordinates)). AUC uses the sampled
trapezoidal rule with post-event start and pre-event end values; `value`
defaults to post-event and `trough` uses pre-event. Terminal half-life is null
unless the terminal points are positive and falling. Models that declare
`dose_schedule_mode` as `native_administrations` use their native schedules.
Archives may declare a treatment-cycle step input that advances on distinct
positive-dose starts on its trigger targets.

Fits assume independent Gaussian errors. `fit_scalar` estimates one parameter
with 1..500 observations and 1..1000 iterations (default 100); proportional,
lognormal and censored errors are refused, never approximated. `fit_individual`
takes `parameters_to_fit` with lower/upper quantities, or a complete native
`fit_request` (including
[profiled proportional errors](../../docs/tutorials/profiled-proportional-errors.md)).
Population fitting offers FOCEI, Laplace and SAEM; see
`qspquest://examples/fit/{scalar|individual-additive|individual-proportional|population}`
and the [population-record workflow](../../docs/tutorials/mcp-results.md#prepare-and-fit-explicit-population-records).
Missing observations are never imputed.

### Limits and repairs

| Inline limit | Local | Hosted |
| --- | --- | --- |
| Serialized request | 1000000 UTF-8 bytes | 1000000 UTF-8 bytes |
| Run solver callbacks / events / output values | 1000000 / 10000 / 25000000 | 1000000 / 10000 / 25000000 |
| Aggregate solver callbacks / events / output values | 25000000 / 250000 / 625000000 | 25000000 / 250000 / 625000000 |
| Optimizer evaluations / iterations | 100000 / 10000; native two-effect SAEM permits an explicit 200000 evaluations | 2000 / 300 |
| Population subjects / combined observations | 128 / 10000 | 16 / 256 |
| Native population random effects | At most 2 | At most 2 |

Missing run-budget fields get the run defaults above; aggregate budgets are not
defaulted. Inline scans take 1..25 values, or 2..25 evenly spaced points;
managed jobs may use the platform's 25/100/225/400 tier ceilings. For
larger work, use a local or managed job; the server never drops subjects or
observations to fit a limit.

Errors give a JSON pointer in `error.expression` (also
`contract.failure.expression`), such as `/run/budgets/events`,
`/parameters/clearance/unit`, `/doses/0/amount/unit` or `/readouts/0/side`, and
a repair hint. An
empty pointer means the whole request. Resubmit a complete corrected request.

## Save CSV files

CSV export is off until the server starts with `--output-dir PATH` (or
`QSPQUEST_MCP_OUTPUT_DIR`):

```sh
target/release/qspquest-mcp --library-dir /absolute/path/qspquest-models --output-dir /absolute/path/qspquest-results
```

Then call `simulate` with `save_csv: "study/run.csv"`. The reply's
`saved_csv.path` and `saved_csv.manifest_path` are absolute, with SHA-256
digests of both files. Keep the CSV and its `.manifest.json` together; read
`qspquest://schema/table-manifest` and `qspquest://guide/results` for the
format and reimport. Dose times can have two rows, `pre` then `post`.

The server only creates new files and refuses existing files, links and unsafe
directories (owned by the server user, mode `0700` on Unix). Comparisons write
one bundle per regimen, such as `study/compare_first.csv`; see the
[comparison example](../../docs/tutorials/mcp-results.md#inspect-and-execute-a-regimen-comparison).
Because `save_csv` can write a file, `simulate` and `compare_regimens` advertise
`readOnlyHint: false` and `idempotentHint: false`. Hosted servers refuse CSV
export.

## HTTP and hosted execution

Choose `--profile local` (stdio), `--profile public` (bounded public HTTP) or
`--profile managed` (HTTP with verified sessions). The
[deployment tutorial](../../docs/tutorials/mcp-deploy.md#choose-an-mcp-deployment-profile) covers
Host and Origin allowlists, HTTPS ingress, trusted peers and managed
configuration.

Start HTTP with `qspquest-mcp --http 127.0.0.1:8080`. Endpoints are `/mcp`,
`/healthz` and, in sandbox builds, `/assets/engine.wasm`. HTTP is stateless,
with no session ID or local run resources. User-supplied models over HTTP need
a `sandbox` build (`sh scripts/build-mcp.sh`); default builds refuse them.

Hosted user models run in Wasmtime with no host imports, 256 MiB memory, a
100-billion-instruction fuel allowance and a 45-second wall limit. Library
models run natively. Payloads are capped at 1 MB and inline responses at
256 KB. BDF is the default solver; Rosenbrock23 and Rodas5P are not
available. Hosted models with 64 or more states return `async_required`;
`submit_simulation` runs them as a managed job through `pkpd.simulate_scenario`
once the platform supports that operation.

HTTP startup precompiles up to eight library models; set
`QSPQUEST_MCP_PREWARM=0` to measure cold requests.

Set `QSPQUEST_WIDGET_WORKER_URL` to the absolute URL of
`/assets/js/engine-worker.mjs` on a `sandbox,widget` build to enable on-device
edits in the MCP App. Model and result data stay in a local Worker.

Container build: `docker build -f bindings/mcp/Dockerfile --target public -t qspquest-mcp .`
(use `--target managed` for a managed build). Configure the library URL,
SHA-256 and cache, or mount a library directory.

Other settings: `QSPQUEST_MCP_TIER`, `QSPQUEST_MCP_CONCURRENCY` and
`QSPQUEST_MCP_ALLOWED_HOSTS` (comma-separated). SDK reference:
[official Rust MCP SDK](https://rust.sdk.modelcontextprotocol.io/).

## Availability records

Libraries declaring `qspquest.model-availability/v1` supply
`qspquest_models/availability.json`. Search results and model cards include each
model's `availability`: whether it can run, its review and unit status and
fitting support. Search also returns `availability_counts`. Availability applies
only to the unchanged preset on this engine build. Libraries without the
registry load with `availability_record_status=unavailable` and null
`availability_counts`.

## Managed jobs

Managed builds add `preview_job`, `submit_simulation`, `submit_population_fit`,
`submit_scan`, `get_job`, `wait_for_job` and `get_artifact`. They need a
verified session resolver and platform support for the operation; see
[managed HTTP](../../docs/tutorials/mcp-deploy.md#managed-http)
for configuration and the synthetic tests.

1. `preview_job` sends an exact `platform_request` and returns
   `managed_preview` with `valid`, `preview_digest`, `approval_required` and
   credit `pricing`. It creates no Run. Review the price before submitting.
2. Submit with the digest in `platform_request.expected_preview_digest` and an
   `idempotency_key`. Reuse the key only to retry the identical request. A
   missing digest fails before any platform call, and invalid routing fields
   report their location, such as `/platform_request/destination_project_id`.
3. Submission returns `managed_submission.run` and `managed_submission.jobs`.
   Poll a Job's `id` with `get_job` or `wait_for_job` (`managed_job`) and check
   its state before treating work as complete.
4. `get_artifact` returns `managed_artifact` metadata. When
   `artifact_retrieval.state` is `available`, read and verify the chunks in
   `full_result_resource`; see the
   [retrieval tutorial](../../docs/tutorials/mcp-deploy.md#retrieve-a-complete-governed-result-artifact).

Only `submit_*` tools create a priced Run. Free sessions are refused before
private calls.

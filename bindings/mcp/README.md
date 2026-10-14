# QSPQuest MCP server

Start with the [agent and Docker quickstart](../../docs/tutorials/agents-docker.md).
The container includes a synthetic model and connects to your own agent client
over HTTP or stdio.

For archived synthetic fitting, use the [library fitting guide](../../docs/tutorials/library-fitting.md)
and inspect readiness before selecting an estimator.

For a complete packaged local first run with saved CSV, replay inputs and
identity, follow the [first-run journeys](../../docs/tutorials/first-run-journeys.md).

The Rust server links QSPQuest directly and provides model inspection, planning,
execution and response formatting over local stdio
or stateless Streamable HTTP. Models load from the public model library, either from a verified local
directory or a pinned HTTPS release. Native projections preserve source numeric
coordinates; inspect their dimensional-qualification status before interpreting
units. The bundled OMEX models are offline numerical qualification fixtures. Each has
an archive-bound availability record and a hashed native input inventory. All
retain `source_scale_unqualified`, publication review `pending` and current-engine
qualification `not_assessed`. Inventory verification is distinct from scientific
qualification. The legacy scalar fixture exposes `source_model_version` when its
source identity differs from its fitting-fixture archive version; both identities
and the existing archive digest are checked.

## Build and connect

Prerequisites: Rust 1.98.1, a native compiler, and the
[public model library](https://github.com/unibio-intelligence/qspquest-models).
Build from the QSPQuest source root:

```sh
cargo build -p qspquest-mcp --release --locked
```

This native stdio build needs no WASM toolchain or browser assets. The same
build helper selects optional features explicitly:

```sh
sh scripts/build-mcp.sh --features ""
sh scripts/build-mcp.sh --features analysis
# Browser assets require wasm-bindgen-cli 0.2.129 on PATH.
sh scripts/build-mcp.sh --features sandbox,widget
```

Only `sandbox` builds the isolated WASM ABI; only `widget` builds browser
bindings. Both require the wasm32 target. Default helper features are
`sandbox,widget`. `managed` requires its documented deployment configuration
before serving a managed endpoint.

For offline use, extract the model-library release archive into a directory
whose root contains `catalog.json` and `models/`. Set an absolute path:

```sh
export QSPQUEST_LIBRARY_DIR="$(cd /path/to/qspquest-models && pwd)"
target/release/qspquest-mcp --library-dir "$QSPQUEST_LIBRARY_DIR"
```

The process waits for a client on stdio. Exit with Ctrl-C. Archive hashes and
indexed members are verified on load; a corrupt or incomplete library prevents
startup. Source-only directories containing `.qsp` models are also supported.

For bounded offline background work, configure distinct private `--jobs-dir`
and `--artifact-dir` directories and use `submit_local_job`, `get_local_job`,
`cancel_local_job` and `resume_local_job`. Read the
[local job tutorial](../../docs/tutorials/mcp-local-jobs.md) for a complete
synthetic example, cancellation semantics, restart recovery and result retrieval.
Job metadata and admission do not establish scientific completion.

For a downloaded library, use an immutable HTTPS release archive and the
SHA-256 supplied separately by its trusted publisher. Set these two variables
in your shell or MCP client's environment before starting the server:

```sh
# Set URL and SHA256 to the release asset and its independently verified digest.
export QSPQUEST_LIBRARY_RELEASE_URL="$URL"
export QSPQUEST_LIBRARY_RELEASE_SHA256="$SHA256"
unset QSPQUEST_LIBRARY_DIR
target/release/qspquest-mcp
```

The archive must be gzip-compressed tar with `catalog.json` at its root, not a
repository archive with an enclosing directory. Startup downloads and verifies
the selected release before serving. The default cache is `~/Library/Caches/qspquest-models`
on macOS, `$XDG_CACHE_HOME/qspquest-models` (or `~/.cache/qspquest-models`) on
Linux, and `%LOCALAPPDATA%\qspquest-models` on Windows. Set
`QSPQUEST_LIBRARY_CACHE` to choose another private directory. Unix cache
roots must be owned by the server user with mode `0700`; Windows ownership and
ACLs must restrict modification to that user, SYSTEM and Administrators.
Shared writable cache roots, symlinks, junctions and unsafe ancestors are refused.

Each cache entry retains `release.tar.gz` and the extracted `library/` directory.
Every start verifies the tarball against the publisher's pin and compares the
complete extracted inventory with its authenticated contents. Missing, changed
or extra files prevent startup. A loaded snapshot uses verified bytes, so later
cache edits cannot change the active library. Subsequent starts work offline
when both the retained archive and extracted inventory remain intact. A missing URL or digest, invalid archive
or failed first download reports an error and prevents startup. Refresh failures
retain the active snapshot; change both the immutable URL and digest to upgrade.
Entries created without a retained tarball are not trusted. For an older cache,
choose a new private `QSPQUEST_LIBRARY_CACHE` directory and allow one verified
download. Do not move or delete the old directory while a server may still use
it. A local `--library-dir` overrides the initial download. Do not configure release
refresh variables when you want a local library to remain fixed.

Claude Desktop configuration (replace both absolute paths):

```json
{"mcpServers":{"qspquest":{"command":"/absolute/path/qspquest/target/release/qspquest-mcp","args":["--library-dir","/absolute/path/qspquest-models"]}}}
```

An MCP client can call `search_models` with `{"query":"rituximab"}`, select
the returned model ID and scenario, then call `get_model`,
`get_scenario_request` and `simulate`. Inspect the model card, declared units,
result status, event sides and engine identity. Held models remain discoverable
without acquiring simulation or fitting qualification. A model that is absent
from the selected library cannot be resolved by name.

Search matches query terms case-insensitively and ranks models by the number of
matching terms; a model matching any term may appear. The `filters` object accepts
exact `model_id`, `version`, `license` and `has_output` values. An output filter
matches a native output name or a declared output alias. The `tag` filter is a
case-insensitive substring of model metadata and its archive index. Pagination
preserves catalog order for tied scores and reports `total` and `next_offset`.
Search uses the current library after refresh.

## Client result contract

Read `qspquest://capabilities` to discover the active tools, compiled features,
deployment permissions, limits, library identity and qualification scope.
The returned tool names match `tools/list`; each tool advertises an `outputSchema`.
Read `qspquest://schema/mcp-result` and `qspquest://schema/capabilities` for the
common result and capability schemas. Tool annotations are hints; the server's
execution and export policies still apply.

Numerical schemas describe native scientific fields and the MCP readout,
summary and export descriptors. A schema-valid record still needs engine unit,
domain, convergence and qualification checks before scientific interpretation.

Every tool result includes `structuredContent.contract` with schema
`qspquest.mcp-result/v1`, a library identity, payload identity, status and
completeness. Consume these fields directly. Status is `success`,
`scientific_failure`, `unsupported`, `async_required`, or `request_failure`.
A nonconverged fit is `scientific_failure` even if `isError` is false. Async
routing does not submit a job. Completeness is `inline_complete`,
`resource_linked`, or `summary_only`; follow the listed resource URIs in the
same session before using the complete trajectory. See the runnable
[result-resource tutorial](../../docs/tutorials/mcp-result-resources.md).

If remaining inline metadata still exceeds the response limit, `delivery_receipt`
marks its omission and hashes the omitted payload. The contract preserves the
operation status and every retained resource URI. Named comparison members keep
their result links; a local job supplies its lookup ID. Retrieve those resources
for diagnostics and scientific interpretation. The omission hash describes
transported metadata and does not qualify the calculation.

## Deployment profiles

Choose `--profile local` for stdio, `--profile public` for intentionally public
bounded HTTP, or `--profile managed` for HTTP with verified sessions. Defaults
remain local stdio or public HTTP. Read the complete runnable
[deployment tutorial](../../docs/tutorials/mcp-deployment-profiles.md) for Host
and Origin allowlists, resolver readiness, HTTPS ingress, trusted peers and
where inputs, results and credentials travel.

## Inline request limits and repairs

Inline plans and calculations validate the complete submitted workload before
inserting run-budget defaults. Both submitted and defaulted requests must fit
the byte limit. A refusal leaves the submitted request unchanged.
Budget and container errors include a JSON pointer in `error.expression`, such
as `/problem/subjects/0/observations` or `/run/budgets/events`, and a repair hint
for that field. The same location is retained in `contract.failure.expression`.
Friendly-input errors also identify submitted quantity, alias, dose, time and
readout fields, for example `/parameters/clearance/unit`, `/doses/0/amount/unit`
and `/readouts/0/side`. Their suggestions describe the supported repair. A failed
normalization preserves the original arguments; resubmit a complete corrected
request rather than combining partial conversions from a refusal.

An empty pointer identifies the complete request. Scientific names inside
parameter, covariate and initial-state maps are interpreted by the native engine;
names such as `max_iterations` do not make a quantity into a workload option.

| Inline limit | Local | Hosted |
| --- | --- | --- |
| Serialized request | 1000000 UTF-8 bytes | 1000000 UTF-8 bytes |
| Run solver callbacks / events / output values | 1000000 / 10000 / 25000000 | 1000000 / 10000 / 25000000 |
| Aggregate solver callbacks / events / output values | 25000000 / 250000 / 625000000 | 25000000 / 250000 / 625000000 |
| Optimizer evaluations / iterations | 100000 / 10000 | 2000 / 300 |
| Population subjects / combined observations | 128 / 10000 | 16 / 256 |
| Native population random effects | At most 2 | At most 2 |

Provided workload counts must be positive integers within the applicable limit.
Missing run-budget fields receive the run defaults above. Aggregate budgets are
left as submitted and must satisfy the native request schema. These ceilings do
not make every estimator, solver or scientific model available; native
preparation checks the operation, units, domains and estimator constraints.
Use an explicitly supported local or governed job route when the required study
exceeds inline limits. The server never drops subjects or observations to admit
a request. See the [population-record workflow](../../docs/tutorials/mcp-population-records.md)
for complete synthetic inputs, static planning and execution.

## Hosted execution

Install the `wasm32-unknown-unknown` Rust target, then run
`sh scripts/build-mcp.sh`. `--features analysis` adds steady-state and Morris
tools. BDF is the default; Rodas5P is explicit. Rosenbrock23 is withheld.

HTTP requires a `sandbox` build for validation and user-source execution. A default
build serves local stdio and rejects hosted source execution. Build the HTTP
server with `sh scripts/build-mcp.sh`, then run the smoke script against that
sandbox-enabled binary.

Start HTTP: `qspquest-mcp --http 127.0.0.1:8080`. Endpoints are `/mcp`,
`/healthz`, and (sandbox build) `/assets/engine.wasm`. HTTP is stateless and does
not return a session ID or local run resources. An ephemeral MCP client can
discover tools and call them on each connection.

Tools: `search_models`, `get_model`, `get_scenario_request`, `validate_model`,
`model_template`, `get_capability`, `plan_run`, `format_response`,
`simulate`, `get_patient_parameters`, `summarize_profile`,
`compare_regimens`, `scan_parameters`,
`fit_scalar`, `fit_individual`, `fit_population`. Read the language and request
resources first. `qspquest://documentation` is a compact versioned index of
model, request, regimen, units and error guidance, structural schemas and a
complete synthetic model/request pair. Follow its resource URIs without a
source checkout; heading fragments select declared sections. Resource entries
include MIME type and SHA-256 so clients can verify retrieved text. Inspect
`qspquest://capabilities` for active execution features and deployment limits.
The index describes document contracts and does not establish model or release
qualification. The [offline resource example](../../docs/tutorials/mcp-model-cards.md#navigate-offline-documentation)
checks every indexed resource and its links through a real client.
Quantities require `{value, unit}`. For a scenario, use
`{"model":{"scenario":{"model_id":"qspquest_diffdose_2026_idr_benchmark",
"scenario_id":"higher_exposure"}}}`. Parameters and initials are explicit
quantity maps. Friendly dosing supports target, amount, at, every/count,
duration, bioavailability and lag. Observation sidedness is explicit.

To replay a complete native request, pass the returned `effective_request` in
`request` and preserve its native names and units. The engine validates these
values directly. Explicit top-level `parameters` and `initial_states` use the
names and units from `get_model`; after conversion, each override replaces only
its corresponding native key. Top-level solver, end, times and doses replace
those request fields. Check the returned effective request and hash before
interpreting the result; an override does not inherit preset qualification.

Friendly scalar fitting uses one estimated native parameter, lower/upper
quantities and 1..500 observations with normal additive Gaussian error. It accepts
1..1000 iterations, defaulting to 100; hosted admission may impose a lower limit.
Nonzero proportional error, lognormal error and censoring are refused with the
relevant field and a supported-workflow hint. Their scientific meaning is never
replaced with an additive or uncensored approximation. Native preparation checks
parameter bounds, initial values, units and observation rows before fitting.

Friendly individual fitting uses declared native parameter names in
`parameters_to_fit`, complete lower/upper quantities and native Gaussian
observation records. Bounds and observations retain their submitted units and
row coordinates; the engine checks their dimensions, domains, transforms and
observation grid before fitting. Malformed records and repeated fitted parameters
are refused with field locations and repair hints. A complete `fit_request`
preserves its native representation and cannot be mixed with friendly fit inputs.

Native empty grids and sided observation plans are preserved. Native samples
and observations cannot both be nonempty. Explicit top-level `times` replaces
a stored observation plan, and the returned notes identify this replacement.
Malformed or unversioned `request` objects are refused; friendly fields belong
at the top level. Two aliases for the same native input are refused.

Malformed quantity maps, dose lists and readout lists report the affected field
before normalization. Timed changes require `name` (or `parameter`), `at` and
`value`; unknown keys and conflicting names are refused. Use quantity objects
for both time and value so the selected model can check their dimensions.

Friendly runs require a positive end. To inspect starting values, choose an
explicit positive end and include time zero in `times`. Omitted friendly
settings use BDF, start zero, `rtol: 1e-8`, `atol: 1e-10`, no administrations,
and 101 evenly spaced samples when no times or readout grid is supplied.
Inspect `notes` and `effective_request` for defaults and explicit replacements.

A readout threshold can be a number in the engine output unit or a quantity
such as `{"value":5000,"unit":"ug/L"}` for an output declared in `mg/L`.
Incompatible dimensions fail before solving. Display aliases cannot give a
source-scale output physical dimensions. Unsupported unit-label conversions
are refused; use the declared label or explicit `1` for model coordinates.
Dose input units must be declared before physical amounts can be converted.
Readouts require a finite, ordered trajectory with one value per time in each
selected output. Mismatched columns, invalid observation sides and arithmetic
overflow return errors. AUC uses the sampled trapezoidal rule; window boundaries
use post-event values at the start and pre-event values at the end. Value defaults
to post-event and trough uses pre-event. Terminal half-life is null when the
terminal points are not all positive and falling; malformed data produces an
error. `summarize_profile` applies the same checks to the supplied points.
The [result-resource tutorial](../../docs/tutorials/mcp-result-resources.md)
executes these contracts with complete synthetic inputs.

`get_model` defaults to a compact interface card. Request `detail: full` for
complete metadata and equations; oversized records return a versioned
`full_model_details_resource` with a chunk manifest when needed. Read chunks in
order and verify the concatenated UTF-8 digest. Retrieve stored scenarios
separately with `get_scenario_request`. See the runnable
[model-card tutorial](../../docs/tutorials/mcp-model-cards.md).

Fits support the engine's bounded independent Gaussian observation contract.
Population fitting offers native FOCEI/Laplace/SAEM with their actual random
effect/covariate restrictions, including explicit NONMEM row mappings. See
`qspquest://examples/fit/{scalar|individual-additive|individual-proportional|population}`.
The [population-record tutorial](../../docs/tutorials/mcp-population-records.md)
plans a complete synthetic input, checks a located mapping refusal and retains
the full fit and source-row provenance. Mapping keys and containers are explicit;
malformed declarations fail before fitting and no missing observation is imputed.
Synthetic recovery and analytic likelihood checks are teaching evidence, not
clinical validation. Always inspect status, objective kind, bounds and diagnostics.

Hosted user models require Wasmtime, with no host imports, 256 MiB instance
memory, a 100-billion-instruction fuel allowance and epoch interruption. The fuel
allowance is calibrated for the small population example; the independent
45-second wall limit and 256 MiB memory limit still apply. Library models execute natively. A native
timeout retains its concurrency permit until computation exits. Hosted fits have
a 45-second deadline, at most 16 subjects, 256 observations and 300 iterations;
over-limit work is rejected. Source and tool payloads are capped at 1 MB and
inline responses at 256 KB. Inline scans accept 1..25 explicit quantity values, preserving their units and
order, or a range with 2..25 points and endpoints in the same unit. A range uses
linear endpoint interpolation in the submitted order, including descending or
constant ranges. Every quantity and generated grid value must be finite; a range
that overflows is refused before execution. Inspect the pointer and repair hint
in the refusal, or supply explicit finite values. Native preparation checks the
parameter's dimension and domain. Inline scans have at most 25 points for every tier;
managed jobs may use the platform's 25/100/225/400 tier ceilings.

Dose targets may declare an input mass unit separately from the native state
coordinate. For these targets, mg, g and ug are converted into the declared unit
before the model's dose scale is applied. Incompatible dimensions are rejected.
Inspect get_model.interface.dose_targets; unit 1 retains the native input coordinate and
does not certify physical dimensions. Native administration schedules drive
models that declare dose_schedule_mode as native_administrations. Archives may
also declare a treatment-cycle step input: distinct positive-dose starts on its
trigger targets advance the cycle, including repeats and lag; coincident starts,
zero doses and zero bioavailability do not create extra cycles.

Local CSV export is disabled until the server starts with `--output-dir PATH`
or `QSPQUEST_MCP_OUTPUT_DIR`. The directory is created when missing. Unix output
roots and nested directories must be owned by the server user with mode `0700`;
Windows ownership and ACLs must restrict modification to that user, SYSTEM and
Administrators. The server creates new files only; choose a new filename to
repeat an export. Existing files, file links and parent-directory links are
refused. Hosted servers refuse CSV exports and output-directory configuration.

For a local server, enable a dedicated output directory:

```sh
target/release/qspquest-mcp --library-dir /absolute/path/qspquest-models --output-dir /absolute/path/qspquest-results
```

Then call `simulate` with the selected model and `save_csv: "study/run.csv"`.
Nested directories are created safely. The returned `saved_csv.path` and
`saved_csv.manifest_path` are absolute. Keep the CSV and its appended
`.manifest.json` together; `saved_csv` includes both raw-byte SHA-256 digests.
The manifest records ordered units, native names, actual source, effective inputs,
engine identity, model/archive context and explicit qualification limits.
Read `qspquest://schema/table-manifest` for its structural contract and
`qspquest://guide/tables` for the runnable export and reimport workflow.
CSV contains the complete requested trajectory, with time, event side and output
columns. Dose times may appear as two rows, `pre` then `post`; the supplied reader
checks both event ordering and the table digest before analysis. The manifest
retains source-scale warnings and never asserts current-run qualification. Regimen
comparisons use the same boundary and create files such as
`study/compare_first.csv` from `save_csv: "study/compare.csv"` and regimen name
`first`. Names are reduced to letters, digits and underscores. Use distinct
names; collisions with either bundle file are refused. Handled write failures
clean newly created files; a crash can leave an incomplete pair, which the reader
rejects. All regimens and existing destinations are admitted before integration;
names must be unique and use at most 120 UTF-8 bytes. CSV export starts after all
numerical runs succeed. If a later filesystem write fails, earlier completed
bundles remain. Inspect `plan_run` with `operation: "compare_regimens"`, then
execute the same arguments with `expected_plan_hash`. The [runnable comparison
example](../../docs/tutorials/mcp-result-resources.md#inspect-and-execute-a-regimen-comparison)
checks member identities and full event-sided trajectories.

Compact simulation results are labeled previews, with event sides, sampled
extrema and a grid-based trapezoidal integral. These are not extrapolated AUC or
validated NCA. To retrieve the requested observation grid, use detail: full or
read the returned full_result_resource. A capped response preserves its cached
resource link; a chunk manifest lists the chunks to read in order and the hash
of their concatenated UTF-8 text. The complete resource includes calculation
identity and the effective request. Fetch it before the next runs evict it.
Full results may be retrieved for 15 minutes from local stdio run resources;
at most eight results of 4 MB each are cached. Hosted results have no cache link.

Configuration: `QSPQUEST_LIBRARY_DIR`, `QSPQUEST_MCP_TIER`,
`QSPQUEST_MCP_CONCURRENCY`, `QSPQUEST_MCP_ALLOWED_HOSTS` (comma-separated).
For refresh, set `QSPQUEST_LIBRARY_RELEASE_URL`, its independently pinned
`QSPQUEST_LIBRARY_RELEASE_SHA256`; optionally set `QSPQUEST_LIBRARY_CACHE` to a
private directory. The tarball
must have catalog.json at its root. Refresh is atomic and verifies all archives
and indexed members. Failed updates retain the active snapshot.

Set `QSPQUEST_WIDGET_WORKER_URL` to the absolute URL of
`/assets/js/engine-worker.mjs` on a `sandbox,widget` build to enable the
MCP App's on-device edits. The existing worker, glue and WASM are downloaded; model and
result data are passed into a local Worker, never posted to a server. Sliders
use declared bounds; unbounded parameters use numeric controls. Large models
without inline widget context should use the local engine directly.

Container: configure the model-library URL, SHA-256 and cache (or mount a
library directory) when starting the image. Build with `docker build -f bindings/mcp/Dockerfile --target public -t qspquest-mcp .`.
For a managed build use `--target managed` (or
`--build-arg MCP_FEATURES=sandbox,widget,managed` with the public target).
Public HTTP intentionally has no managed authentication. Managed non-loopback
HTTP requires an HTTPS gateway declaration and a trusted proxy IP/CIDR allowlist;
restrict the backend port to that gateway. Hosted source execution is disabled in builds without sandbox.

SDK reference: [official Rust MCP SDK](https://rust.sdk.modelcontextprotocol.io/).

Managed configuration: build with `sandbox,widget,managed`, set
`QSPQUEST_MCP_PROFILE=managed` (or `QSPQUEST_MANAGED_ENABLED=1`), `QSPQUEST_PLATFORM_API_URL` to the managed
`/api/v1` base, and `QSPQUEST_PLATFORM_SESSION_URL` to a verified platform
session resolver returning `user_id`, `tenant_id`, and `tier`. The server does
not trust client tier headers or JWT claims. Free scopes are rejected before
private model or job requests. The seven managed tools expose price preview, explicit
Run submission, Job polling and Artifact metadata; they cannot bypass release,
Project, pricing or operation-readiness checks. The legacy native research
execute route is R-only and is deliberately not used for Rust population fits.
The platform must first qualify the requested Rust operation and supply its
session resolver; an adapter alone does not qualify managed execution.

HTTP startup precompiles at most eight verified interactive library documents into a bounded
compiler cache; set `QSPQUEST_MCP_PREWARM=0` to measure cold requests. Frequently
used documents retain their place in this eight-document cache. Different models
can compile concurrently within the server's compute slots. Requests using the
simulation compiler share one compilation for each library version and model;
fitting builds its parameter-specific sensitivity model for each fit. A library
version or model-content change requires a separate cache entry. This is an engine cache,
with no request/result/session state. The bundled numerical evidence reports
47/47 frozen output-parity checks and the separate strict absolute 1e-9 diagnostic.
All 16 Liu/Rajwade scenarios also pass independent Radau accuracy and dose-side
checks; see [numerical qualification](qualification/README.md). Hosted interactive
execution and heavy routing are measured separately from completed jobs. Live job admission and other target architectures require separate validation.


Large hosted models (64 or more states) receive `async_required` before engine
compilation/integration. `submit_simulation` uses managed `pkpd.simulate_scenario`
preview and Run creation; the platform must qualify that operation first. This
response is not a completed scientific result. Interactive execution and managed
job admission have separate p95 <= 500 ms targets; heavy job completion is measured
separately. Local stdio and offline WASM qualification retain all catalog scenarios.

Selected simulation outputs are evaluated before result construction. All ODE
states, events and physical invariants remain checked. Unselected readout-only
domain checks are skipped; request all outputs when validating every readout.
Selection order is preserved and included in scientific request/cache identity.

Use `python scripts/benchmark-mcp-latency.py URL evidence.json` (requires
jsonschema) to retain both sweeps, errors/timeouts, exact nearest-rank percentiles,
interactive execution and heavy routing. It never counts a routed request as a
completed simulation or as measured managed Job admission.

## Availability records

Libraries declaring `qspquest.model-availability/v1` supply
`qspquest_models/availability.json`. Search results and compact/full model cards
expose the same archive-bound `availability` record. The loader rejects disagreement
between the selected catalog version, archive, presets, unit status, fitting flags
and stored-evidence digest. Search also returns `availability_counts` with distinct
selected identities, archived versions, preset requests and fitting denominators.

Execution class, publication review, physical units and current-engine qualification
are separate fields. Source replay is a retained check; inspection-only material
has no executable projection. A preset's availability does not qualify a changed
request or a different engine build. Legacy libraries can still load, with
`availability_record_status=unavailable` and no generated qualification claims;
`availability_counts` is null when their versioned registry is absent.

### Preview a governed job before submission

`preview_job` sends an exact `platform_request` to the configured platform and
returns `managed_preview`: `valid`, `preview_digest`, `approval_required`, fixed
credit `pricing`, and a workbench link. It creates no Run or Job. Review the price
and meet the client/platform approval requirements before submitting.

Include that digest as `platform_request.expected_preview_digest` when calling
`submit_simulation`, `submit_population_fit`, or `submit_scan`. A missing or
malformed digest fails before any platform request. The adapter forwards the
reviewed digest unchanged; it never replaces it by fetching a different price.
The platform verifies the digest against the exact request and enforces approval.

The adapter checks the exact capability, destination Project identifier, retry
key and preview-digest shape before sending a request. Invalid routing fields
carry their submitted location in `error.expression`, for example
`/platform_request/destination_project_id` or `/idempotency_key`. Platform-specific
inputs and extensions are forwarded unchanged. Passing local admission does not
establish platform approval, operation readiness or scientific qualification.

Keep the same `idempotency_key` and identical request for infrastructure retries.
Use a new key for an intentional new Run. The platform must reject reuse of a key
for a different request and atomically prevent duplicate priced Runs. A lost
submission response is retried with the original key and digest. If the platform
rejects a stale preview, obtain and review a new preview before a new submission.

Submission returns `managed_submission.run` and `managed_submission.jobs`; use a
Job's `id` with `get_job` or `wait_for_job`. Polling returns `managed_job`, including
its state, price and upstream output records. Terminal failed, timed-out, expired
and cancelled Jobs return immediately from waiting. Retrieval success means
metadata was retrieved; inspect Job state before treating work as completed.
`get_artifact` returns `managed_artifact`, including identifiers, byte size, media
type, checksum and download link. Responses preserve the original
`platform_response` when it fits; unused artifact metadata that exceeds the
budget is explicitly omitted with its digest while retaining the typed artifact
and retrieval reference. These are typed metadata projections, not native engine
results or evidence that a platform operation is scientifically qualified.

For an artifact with the versioned immutable-byte/retention contract,
`artifact_retrieval.state` is `available` and `full_result_resource` provides a
digest-bound manifest. Read its base64 byte chunks through authenticated
`resources/read` and verify the complete digest before interpreting the result.
Legacy metadata has state `unavailable`; expired retention has state `expired`.
The [complete retrieval tutorial](../../docs/tutorials/managed-result-artifacts.md)
includes a standard-library client, exact range/retention requirements and
synthetic checks. The adapter does not establish a deployed platform's storage,
authorization or scientific qualification.

#### Run the synthetic preview tutorial

Purpose: exercise explicit price review, stable retries and result retrieval
without a real account, priced operation or private service. Prerequisites: the
released source distribution, the supported Rust toolchain and its locked
build dependencies, plus permission to bind a loopback test socket. From the
project root run:

```sh
cargo test --locked -p qspquest-mcp --features managed --test managed governed_preview_digest_idempotency_session_and_free_boundary -- --nocapture
```

The complete runnable fixture is in `tests/managed.rs` within the MCP crate. It
uses a synthetic Project, empty synthetic inputs, a Pro test scope, and a 42-credit
fixture price. Expect a synthetic price line and one passing test. The test checks
that preview performs no submission, missing review digests perform no platform
call, retries forward the same key and digest, and Job/Artifact records are read
through their typed projections. It also checks exact catalog identifiers and
versions, unassessed versus unavailable execution, declared governed availability,
scenario counts and tier-locked record exclusion. See the complete
[discovery workflow](../../docs/tutorials/mcp-model-cards.md) for these fields.
These fixture credits and identifiers are not a
live offer. The test does not establish a deployed platform's atomicity or
scientific readiness.

For a real managed workflow, prerequisites additionally include a configured
verified-session resolver, a caller-owned destination Project, an exact released
operation input, and platform qualification for that operation. Inspect
`managed_preview.pricing` and `approval_required` before authorizing a Run. Free
sessions are refused before private calls. An unconfigured managed build returns
an actionable error; it cannot create a priced Run locally.

`simulate` and `compare_regimens` retain the optional `save_csv` compatibility
interface. Their discovery annotations set `readOnlyHint: false` and
`idempotentHint: false` because an explicit export can create a file. Omit
`save_csv` for calculation without persistent file writes. Export additionally
requires a configured local output directory, refuses existing files and unsafe
paths, and is unavailable in hosted mode. Job creation occurs only through an
explicit `submit_*` tool; discovery, calculation and `preview_job` do not create
a priced Run. Annotations describe possible behavior; client and platform
authorization remain authoritative.

`get_patient_parameters` prepares and evaluates selected initial outputs before administrations, retaining scenario scalar inputs and nonzero initial states. Inspect `plan_run` with the same operation and execute using `expected_plan_hash`; use the [complete initial-value example](../../docs/tutorials/mcp-result-resources.md#inspect-initial-patient-quantities). The result carries actual declared output units, all bound inputs and states, execution identity and qualification limits. It performs no integration or future schedule.


For complete local numerical results that exceed the process-local cache, start
with `--artifact-dir PATH` pointing to an empty private directory. Completed
calculations retain immutable JSON bodies for 24 hours, including across server
restarts. Responses provide a digest, byte count, expiry and chunk URI prefix in
`result_artifact`; read and verify all chunks from `full_result_resource` before
using the complete result. Storage supports up to 128 MB per body, 256 live
artifacts and 1 GB total. Capacity refusal preserves existing live results.
Only one local server may own a directory. Numerical tools advertise persistent
writes when storage is enabled; discovery and resource reads remain read-only.
Follow the [complete retained-result example](../../docs/tutorials/mcp-result-resources.md#retain-complete-results-across-local-server-restarts)
for exact setup, retrieval, restart verification and cleanup.

Prepared and executed simulations expose `scheduled_boundary_receipt` with
forcing declarations, event references and explicit observation sides. See the
[scheduled-boundary tutorial](../../docs/tutorials/mcp-result-resources.md#inspect-scheduled-boundaries-and-observation-sides).

For separate numerical, conservation, source-fidelity and clinical status, run the
[scientific assessment workflow](../../docs/tutorials/mcp-result-resources.md#inspect-separate-scientific-assessments).

Use `format_response` to map complete computed columns into requested numeric
arrays with explicit run labels, units, times and pre/post/both event sides.
It checks literal decision identifiers and optional candidate answers, refuses
missing conditions or event objects inside scalar arrays, and retains source
identities and copying receipts. See the complete
[response guard tutorial](../../docs/tutorials/mcp-result-resources.md#format-complete-decision-responses).
Archive assessments are scoped records; they never qualify a current request.


For known output names and stored scenario IDs, `get_model.selection` returns
complete selected declarations, actual column mappings and native scenario
requests in one response. It also batches advertised inspection resources for
one model-content hash. Select 1 to 32 total items within the 40,000-byte budget
and use `expected_inspection_hash` to refuse changed content. See the complete
[selection example](../../docs/tutorials/mcp-result-resources.md#select-exact-outputs-scenarios-and-inspection-resources).

# Offline MCP jobs

Use a local job when an agent needs to submit bounded independent simulations,
scans or fits, inspect progress, cancel remaining work, and retrieve every
completed result. The server offers explicit tools; it does not advertise the
MCP protocol task lifecycle. Job admission and job state are metadata. Interpret
the scientific status and qualification of each complete result separately.

## Setup and runnable synthetic example

Prerequisites: the QSPQuest source release, Rust 1.98.1, a native compiler, and
Python 3.10 or later. No additional Python packages or hosted account are needed.
From the source release root:

```sh
cargo build --locked --release -p qspquest-mcp
python3 docs/examples/mcp_local_jobs.py target/release/qspquest-mcp
python3 docs/examples/mcp_local_jobs.py target/release/qspquest-mcp --cancel
```

The [complete example](qspquest://documentation/files/docs/examples/mcp_local_jobs.py) creates a private temporary
source library using the shipped
[synthetic model](qspquest://documentation/files/conformance/models/synthetic-one-compartment.qsp).
It submits 6 mg and 12 mg boluses to separate one-compartment simulations with
clearance 0.3 L/h and volume 3 L, reads all 31 hourly concentrations from each
immutable artifact, and compares them with `dose / 3 * exp(-0.1 * time)` in mg/L.
It retries admission with the same inputs, restarts the same server binary,
and verifies that committed units and artifact digests survive without replay.
The cancellation variant requests cancellation, waits for the current native
unit to finish, and resumes any remaining units. A short job may finish before
cancellation takes effect. Both commands print:

```text
Verified two synthetic dose results, full artifacts, stable retry key and restart without replay.
```

This is a synthetic numerical check, not qualification of a clinical model.
The example removes its own temporary job and artifact directories after the
server closes. For durable work, configure persistent directories instead.

## Configure an agent client

Start local stdio with a verified library and distinct private directories:

```sh
target/release/qspquest-mcp --profile local \
  --library-dir /absolute/path/to/library \
  --jobs-dir /absolute/private/path/to/jobs \
  --artifact-dir /absolute/private/path/to/artifacts --concurrency 2
```

Directories must be empty on first use or owned by this store, private to the
server user, and have safe ancestors. Only one server may hold a job directory.
The job directory is bound to the exact executable bytes. Resume with that
binary and the original artifact directory. A different build requires a new
job directory. Unsupported files and corrupted published journals prevent
startup; the server does not overwrite unrelated data.

Inspect `qspquest://capabilities`, `get_capability`, and `tools/list` before
submitting. `deployment.local_jobs_enabled` must be true. The offline guide is
also available at `qspquest://guide/local-jobs`.
The typed job descriptor is available at `qspquest://schema/local-job`.

1. Inspect the model and units. Use `plan_run` with each intended operation and
   its arguments; inspect the full preparation and qualification limits.
2. Call `submit_local_job` with a stable ASCII `idempotency_key` and `work`, an
   array of 1 to 25 objects containing unique `label`, `operation`, and
   `arguments`. Arguments match the synchronous tool's input schema. Include
   `expected_plan_hash` from the inspected plan when binding prior inspection.
   Every unit must pass admission before any job is published.
3. Call `get_local_job` with its exact `job_id`. `completed` is the committed
   ordered prefix; `current_unit` identifies a dispatched, uncommitted unit.
   `dispatched_units` includes interrupted attempts and may exceed completed
   units. States are `queued`, `running`, `cancel_requested`, `succeeded`,
   `failed`, `cancelled`, and `expired`.
4. Read each `completed[].artifact.uri`, then every zero-based chunk under its
   `chunk_uri_prefix`. Concatenate UTF-8 text without separators, verify the
   full byte count and SHA-256, then parse the result. Inspect scientific
   status, effective request, source identity, units and qualification. Follow
   the [complete result resource workflow](qspquest://guide/tables).
5. Call `cancel_local_job` with `job_id` to stop undispatched work. A queued job
   cancels immediately. A running call drains within its existing native
   budgets and retains its result before stopping; no mid-solve interruption
   or checkpoint is offered. Cancellation cannot retract completed results.
6. Call `resume_local_job` for retryable cancelled or interrupted work. All
   committed artifacts are verified before resuming; committed units are
   skipped and pending units must still match their original preparation.
   At most three resumes are allowed.

After a process interruption, a dispatched unit may have executed without a
committed result. `retry_requires_uncommitted_acknowledgement` identifies this
case. Inspect the retained prefix, then set `retry_uncommitted: true` only if
replaying that uncommitted unit is acceptable. Units are independent numerical
calls with immutable local artifacts; exports and paid submissions are excluded.
The server makes no exactly-once claim for interrupted uncommitted work.

## Limits and common errors

Supported unit operations are `simulate`, `scan_parameters`, `fit_scalar`,
`fit_individual`, `fit_population`, and, in an analysis feature build,
`steady_state` and `morris_screening`. Existing model fitting readiness, unit,
input, solver and native work-budget checks remain mandatory. Work units do
not consume previous unit outputs. `save_csv` is refused; results use artifacts.

The queue admits at most 32 active jobs and retains at most 64 live job
descriptors. Jobs have 1 MB admission inputs, a 2 MB stored-request ceiling,
bounded journals and a 512 MB job-store budget including reserved completion
space. Artifact quotas are separate. Existing jobs/results are preserved when
capacity is full; new admission fails explicitly.

Inputs and journals expire 24 hours after admission. A running unit drains
before its input can be removed. Bounded expiration markers retain retry-key
and request digests for seven further days. Cleanup runs on startup and job
mutations. Artifact retention starts independently at each result's creation;
an expired or missing committed artifact prevents resume. Copy full verified
results before retention expires if you need a longer record.

Same-key, same-input retries return the same job within retained key history.
A changed input under the same key fails with `local_job_idempotency_conflict`;
corrected experiments need a new key. Scientific failure, including a
nonconverged fit, retains its diagnostic artifact and leaves the job failed.
It is not a retryable success. Changed source/library/preparation refuses later
dispatch and preserves the completed prefix.

Public or managed HTTP deployments cannot create local jobs or charge for
them. Their errors give the local stdio command route. A hosted governed job
has its own authorized preview and lifecycle; a local job descriptor never
substitutes for platform admission or scientific qualification.

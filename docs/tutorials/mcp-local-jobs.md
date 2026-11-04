# Offline MCP jobs

Use a local job when an agent needs to submit several independent simulations,
scans or fits, check progress, cancel remaining work, and collect every result.
Check each result's own status as well as the job's.

## Setup and runnable synthetic example

Prerequisites: the QSPQuest source release, Rust 1.98.1, a native compiler, and
Python 3.10 or later. No additional Python packages or hosted account are needed.
From the source release root:

```sh
cargo build --locked --release -p qspquest-mcp
python3 docs/examples/mcp_local_jobs.py target/release/qspquest-mcp
python3 docs/examples/mcp_local_jobs.py target/release/qspquest-mcp --cancel
```

The [complete example](../examples/mcp_local_jobs.py) creates a private temporary
source library using the shipped
[synthetic model](../../conformance/models/synthetic-one-compartment.qsp).
It submits 6 mg and 12 mg boluses to separate one-compartment simulations with
clearance 0.3 L/h and volume 3 L, reads all 31 hourly concentrations from each
result, and compares them with `dose / 3 * exp(-0.1 * time)` in mg/L. It also
resubmits the same job and restarts the server to show that finished work is
kept and not rerun. The `--cancel` variant cancels, waits for the running unit
to finish, then resumes the rest. A short job may finish before cancellation
takes effect. Both commands print:

```text
Verified two synthetic dose results, full artifacts, stable retry key and restart without replay.
```

The example deletes its temporary directories when it finishes. For real work,
use persistent directories as below.

## Configure an agent client

Start local stdio with a verified library and distinct private directories:

```sh
target/release/qspquest-mcp --profile local \
  --library-dir /absolute/path/to/library \
  --jobs-dir /absolute/private/path/to/jobs \
  --artifact-dir /absolute/private/path/to/artifacts --concurrency 2
```

Directories must be empty on first use (or already used by QSPQuest) and
readable only by the server's user, and the path must not go through symbolic
links or directories other users can write to. Only one server can use a job directory at a
time. A job directory works only with the exact executable that created it:
resume with the same binary and artifact directory, and use a new job directory
after upgrading. The server refuses to start if the directory contains unknown
or corrupted files; it never overwrites unrelated data.

Inspect `qspquest://capabilities`, `get_capability`, and `tools/list` before
submitting. `deployment.local_jobs_enabled` must be true. The offline guide is
also available at `qspquest://guide/local-jobs`.
The typed job descriptor is available at `qspquest://schema/local-job`.

1. Check the model and units. Call `plan_run` with each operation and its
   arguments to preview it.
2. Call `submit_local_job` with a stable ASCII `idempotency_key` and `work`, an
   array of 1 to 25 objects containing unique `label`, `operation`, and
   `arguments`. Arguments are the same as for the regular tool. Include
   `expected_plan_hash` from the preview to make sure the same plan runs.
   If any unit is invalid, the whole job is rejected.
3. Call `get_local_job` with its `job_id`. `completed` lists finished units in
   order; `current_unit` is the one running now. `dispatched_units` counts
   interrupted attempts too, so it may exceed the completed count. States are
   `queued`, `running`, `cancel_requested`, `succeeded`, `failed`, `cancelled`,
   and `expired`.
4. Read each `completed[].artifact.uri`, then every zero-based chunk under its
   `chunk_uri_prefix`. Concatenate UTF-8 text without separators, verify the
   full byte count and SHA-256, then parse the result. Check its status and
   units. See the [complete result resource workflow](mcp-result-resources.md).
5. Call `cancel_local_job` with `job_id` to stop work that has not started. A
   queued job cancels immediately. A running unit finishes and keeps its result;
   it is not interrupted mid-solve. Completed results are kept.
6. Call `resume_local_job` to continue cancelled or interrupted work. Finished
   units are checked and skipped; the remaining units must still match their
   original inputs. A job can be resumed at most three times.

If the server stopped while a unit was running, that unit may have run without
saving its result. `retry_requires_uncommitted_acknowledgement` flags this case.
Set `retry_uncommitted: true` only if running that unit again is acceptable.

## Limits and common errors

Supported unit operations are `simulate`, `scan_parameters`, `fit_scalar`,
`fit_individual`, `fit_population`, and, in an analysis feature build,
`steady_state` and `morris_screening`. Each unit gets the same checks as the
regular tool. Units cannot use earlier units' outputs. `save_csv` is not
available; results are saved as artifacts.

The queue accepts at most 32 active jobs and keeps at most 64 jobs in total.
Job inputs are limited to 1 MB on submission and 2 MB stored, and the job
directory to 512 MB. Artifacts have separate limits. When full, new jobs are
rejected with an error; existing jobs and results are kept.

Job inputs and logs expire 24 hours after submission. Retry keys are remembered
for seven more days. Each result artifact has its own retention period starting
when it is created; if one has expired, the job cannot be resumed. Copy results
you need to keep before they expire.

Resubmitting the same inputs with the same key returns the existing job.
Different inputs under the same key fail with `local_job_idempotency_conflict`;
use a new key. A scientific failure, such as a non-converged fit, saves its
diagnostics and marks the job failed. If the model or library changes mid-job,
the remaining units are refused and finished results are kept.

Local jobs are available only over local stdio, not public or managed HTTP;
the error message there shows the local command to use. Hosted jobs work
differently.

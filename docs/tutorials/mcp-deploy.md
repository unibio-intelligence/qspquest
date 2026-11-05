# Deploy the MCP server

Choose how the QSPQuest MCP server runs, submit offline jobs, and retrieve
complete governed result artifacts from a hosted deployment.

## Choose an MCP deployment profile

Run the MCP server locally over stdio, as an open HTTP server, or in managed
(signed-in) mode. You need the source archive, Rust 1.98.1, a native compiler,
Python 3 for the optional HTTP client below, and permission to open local
network ports. Run commands from the source root. No account is needed.

### Prepare a synthetic library

The source includes an oral-dose teaching model. Copy it to its own directory
and build the server:

```sh
mkdir -p .qspquest-tutorial-models
cp bindings/mcp/assets/templates/oral.qsp .qspquest-tutorial-models/oral.qsp
export QSPQUEST_LIBRARY_DIR="$(cd .qspquest-tutorial-models && pwd)"
cargo build --locked -p qspquest-mcp
```

For real work, use a model library and check each model's limitations.

### Local stdio

```sh
target/debug/qspquest-mcp --profile local --library-dir "$QSPQUEST_LIBRARY_DIR"
```

The process waits for an MCP client on stdio; Ctrl-C exits. This is also the
default when neither `--profile` nor `--http` is given. Configure your client
with the absolute executable path and these arguments. CSV export needs a
private `--output-dir` (not available over HTTP). Stored results are size-limited
and expire; `qspquest://capabilities` gives the limits.

### Public bounded HTTP

In the first terminal:

```sh
target/debug/qspquest-mcp --profile public --http 127.0.0.1:8080 \
  --library-dir "$QSPQUEST_LIBRARY_DIR"
```

In another terminal, run this complete stdlib client:

```sh
python3 - <<'PY'
import json
from urllib.request import Request, build_opener, ProxyHandler
opener = build_opener(ProxyHandler({}))
request = Request('http://127.0.0.1:8080/mcp',
    data=json.dumps({'jsonrpc': '2.0', 'id': 1,
                     'method': 'tools/list', 'params': {}}).encode(),
    headers={'Content-Type': 'application/json',
             'Accept': 'application/json, text/event-stream',
             'MCP-Protocol-Version': '2025-11-25'})
with opener.open(request, timeout=15) as response:
    tools = json.load(response)['result']['tools']
print('\n'.join(tool['name'] for tool in tools))
assert any(tool['name'] == 'simulate' for tool in tools)
PY
```

Expect the tool names, including `simulate`. No login is needed: this profile
is open to anyone who can reach it, with limits on computation. `--http` without
`--profile` selects this profile unless managed mode is configured. `/healthz`
reports whether the server is ready. The default build will not run
user-supplied models over HTTP; use `sh scripts/build-mcp.sh` for the sandboxed
build that can. HTTP has no CSV export or stored results. Put HTTPS in front of
the server for remote clients. Binding `0.0.0.0` or `::` makes the server
reachable from other machines; startup warns about this.

Native clients normally omit `Origin`. All requests carrying a browser Origin
are refused unless it is explicitly allowed. For a browser client, set:

```sh
export QSPQUEST_MCP_ALLOWED_ORIGINS=https://app.example.test
export QSPQUEST_MCP_ALLOWED_HOSTS=mcp.example.test
```

Replace both with your actual browser origin and MCP host. Origins match scheme, hostname and exact port; HTTPS's
default port is 443. Other ports, `null` and wildcards are refused. Host entries
are comma-separated hostnames or IP addresses with optional ports. Defaults
allow `localhost`, `127.0.0.1` and `[::1]`. Forwarded Host headers do not replace
the actual Host. Allowing an Origin does not add authentication.

### Managed HTTP

Build and execute the self-contained synthetic gateway test:

```sh
cargo test --locked -p qspquest-mcp --features managed --test deployment \
  explicit_profiles_session_origin_and_actual_peer_boundary -- --nocapture
cargo test --locked -p qspquest-mcp --features managed --test managed \
  governed_preview_digest_idempotency_session_and_free_boundary -- --nocapture
```

Expect two passing tests. The first starts the production executable and a
loopback resolver with an unissued synthetic bearer. It checks public discovery,
wrong token rejection, exact Origins, resolver readiness failure and actual peer
restrictions despite forged forwarded headers. The second uses synthetic tenants,
price preview and Job/Artifact descriptors. It rejects cross-tenant Artifact
metadata, suppresses rejected upstream bodies, and prevents Free scopes from
making private platform calls. Fixtures and complete inputs are shipped in
`bindings/mcp/tests/deployment.rs` and `bindings/mcp/tests/managed.rs`.
These tests do not check a real deployment.

A real managed deployment additionally requires an approved session resolver,
a governed API and a gateway providing HTTPS. Build the managed sandbox without the optional browser widget:

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --locked -p qspquest-wasm --target wasm32-unknown-unknown --features sandbox-abi
mkdir -p target/mcp-assets
cp target/wasm32-unknown-unknown/release/qspquest_wasm.wasm target/mcp-assets/sandbox.wasm
QSPQUEST_SANDBOX_WASM="$PWD/target/mcp-assets/sandbox.wasm" \
  cargo build --release --locked -p qspquest-mcp --features sandbox,managed
```

Then configure these actual operator-supplied values:

```sh
export QSPQUEST_PLATFORM_API_URL=https://platform.example.test/api/v1
export QSPQUEST_PLATFORM_SESSION_URL=https://platform.example.test/session
export QSPQUEST_MCP_GATEWAY_ORIGIN=https://mcp.example.test
export QSPQUEST_MCP_TRUSTED_PROXIES=10.20.30.40/32
export QSPQUEST_MCP_ALLOWED_HOSTS=mcp.example.test
export QSPQUEST_MCP_ALLOWED_ORIGINS=https://app.example.test
target/release/qspquest-mcp --profile managed --http 0.0.0.0:8080 \
  --library-dir "$QSPQUEST_LIBRARY_DIR"
```

Replace the example endpoints and proxy address before deployment. Restrict the
backend port to the HTTPS gateway on the network. The listener itself speaks
HTTP; the gateway origin is an operator declaration, not automatic TLS setup.
The MCP process checks the actual connection peer against its trusted IP/CIDR
list and ignores identity/proxy headers supplied by callers. Do not configure a
shared proxy subnet that also admits untrusted callers. Prefix `/0` is refused.
Loopback managed tests default to loopback peers and allow HTTP endpoints.

Startup fails before model loading if managed support, endpoint configuration,
HTTPS gateway declaration or required proxy restrictions are missing. Resolver
reachability uses a credential-free GET with a five-second timeout; 200, 401 or
403 confirms reachability. A redirect, unavailable endpoint or other status
prevents startup. This probe does not validate a session. Each managed request
forwards its bearer to that fixed resolver, with redirects and environment proxy
settings disabled. Its verified response supplies identity, tenant and tier;
client tier headers and unverified JWT claims never supply them. The resolver
must enforce its token audience, expiry, scopes and tenant policy. An unavailable
or rejecting resolver denies requests.

The same verified bearer accompanies governed API calls. The API must enforce
Project access, pricing, approval and operation readiness. Model inputs selected
for a managed Run go to that API, and result metadata returns through MCP.
Artifact downloads follow the platform's authorized link. Bearers are excluded
from serialized scopes and capability diagnostics; upstream rejection bodies
are suppressed. Read `qspquest://capabilities` for the active profile, limits,
resolver reachability and gateway/peer policy. `/healthz` and executable assets
remain public and carry no caller model inputs or credentials.

`QSPQUEST_MCP_PROFILE` selects the same profiles as `--profile`; the command-line
option takes precedence. Existing `QSPQUEST_MANAGED_ENABLED=1` selects managed
HTTP when no profile is supplied. Conflicting settings fail. Managed stdio and
local HTTP fail with an actionable transport error. `QSPQUEST_MCP_TIER` controls
local/public budgets; verified managed sessions supply their own tier.

## Offline MCP jobs

Use a local job when an agent needs to submit several independent simulations,
scans or fits, check progress, cancel remaining work, and collect every result.
Check each result's own status as well as the job's.

### Setup and runnable synthetic example

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

### Configure an agent client

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
also available at `qspquest://guide/deploy`.
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
   units. See the [complete result resource workflow](mcp-results.md#read-a-complete-mcp-simulation-result).
5. Call `cancel_local_job` with `job_id` to stop work that has not started. A
   queued job cancels immediately. A running unit finishes and keeps its result;
   it is not interrupted mid-solve. Completed results are kept.
6. Call `resume_local_job` to continue cancelled or interrupted work. Finished
   units are checked and skipped; the remaining units must still match their
   original inputs. A job can be resumed at most three times.

If the server stopped while a unit was running, that unit may have run without
saving its result. `retry_requires_uncommitted_acknowledgement` flags this case.
Set `retry_uncommitted: true` only if running that unit again is acceptable.

### Limits and common errors

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

## Retrieve a complete governed result artifact

Use this workflow to download a complete result file from a hosted Job,
including results larger than 4 MB. It checks that the download is complete and
intact. It does not check whether the fit converged or the science is sound;
check that in the result itself.

### Prerequisites

You need Python 3.10+, a managed QSPQuest JSON HTTP endpoint built with `managed`,
a signed-in session with access to the Project, and an artifact ID returned by
that Project's Job. The platform must support the byte-download requirements
below. Public HTTP and offline stdio have no managed sessions; for local results
use the [local result-artifact workflow](mcp-results.md#read-a-complete-mcp-simulation-result).

The [complete client](../examples/managed_artifact_download.py) uses only the
Python standard library. Run it from the source distribution. Set the endpoint
and session token in your environment; do not place a token in a URL or command
argument. HTTPS is required outside the synthetic loopback tutorial.

```sh
export QSPQUEST_MCP_URL=https://your-approved-managed-endpoint/mcp
export QSPQUEST_ARTIFACT_ID=the_artifact_id_from_your_job
# Set QSPQUEST_MCP_BEARER to your verified session through your normal secret handling.
python3 docs/examples/managed_artifact_download.py \
  --mcp-url "$QSPQUEST_MCP_URL" --artifact-id "$QSPQUEST_ARTIFACT_ID" \
  --out governed-result
```

Choose a new directory whose parent exists. The client calls `get_artifact`,
requires `artifact_retrieval.state: available`, and saves `descriptor.json`. It
downloads the file in chunks to `artifact.partial`, checks the total size and
SHA-256, then renames it to `artifact.bin`. `checks.json` records the outcome. If
a download fails, the partial file is kept and labeled; do not use it as a
result. Existing files are never replaced. The client ignores proxy environment
variables and does not follow redirects.

The command prints `status: passed`, the byte count, SHA-256 and number of
chunks. If the descriptor's media type is JSON, parse the file with:

```sh
python3 - <<'PY'
import json
from pathlib import Path
manifest = json.loads(Path('governed-result/descriptor.json').read_bytes())
assert manifest['media_type'] == 'application/json'
result = json.loads(Path('governed-result/artifact.bin').read_bytes())
print('Retained JSON type:', type(result).__name__)
print('Declared result schema:', manifest['result_schema'])
PY
```

The client does not inspect the content. Keep `descriptor.json` and
`checks.json` together with `artifact.bin`.

### Agent contract and limits

`get_artifact` returns `managed_artifact` metadata and an
`artifact_retrieval` state: `available`, `unavailable`, or `expired`. When
available, it includes the descriptor and `full_result_resource`, whose URI
contains the artifact ID and SHA-256. Read it with `resources/read` using the
same session, then read `chunk_uri_prefix + index` for every index from zero
through `chunk_count - 1`.

Each JSON chunk contains its index, byte offset, byte length, artifact URI,
artifact SHA-256, chunk SHA-256 and base64 data. Decode bytes before joining;
UTF-8 characters may span ranges. Verify every field and the final whole-file
digest. Schemas are available at `qspquest://schema/managed-result-artifact` and
`qspquest://schema/managed-artifact-chunk` in managed builds. Every read is
authorized separately; knowing a URI does not grant access.

Artifacts are limited to 128,000,000 bytes, chunks to 48,000 bytes and
retention to seven days; the descriptor gives the actual creation and expiry
times. Each read has a 15-second timeout, and each tenant can run two reads or
computations at once. After a busy or timeout response, retry the same URI
before it expires. The client does not retry or resume automatically.

`managed_artifact_expired` means the file has expired. Any mismatch in digest,
ETag, range, media type or size fails the download. Older artifacts report
`managed_artifact_bytes_unavailable`. The MCP server never follows external
download links or sends your session token to another host.

### Platform byte contract

The authorized artifact metadata may add `result_retention` with schema
`qspquest.platform-result-bytes/v1`, `immutable: true`, creation and expiration
timestamps, `retention_seconds` equal to their difference, and `result_schema`
(or null). The metadata's byte length and lowercase SHA-256 refer to the exact
downloaded representation. The download link must resolve to the configured
API base plus `/artifacts/<id>/download`, with no credentials, query, fragment,
redirect or alternate artifact ID.

That route must independently authorize the caller and Project. It must honor
`Range`, `If-Match: "sha256-<complete checksum>"` and `Accept-Encoding: identity`,
returning HTTP 206, the exact `Content-Range`, strong ETag, media type and bounded
bytes. Mutable content returns HTTP 412; expired bytes return HTTP 410. A full
body fallback is refused. The platform owns immutable storage, retention,
cleanup and expired metadata; this adapter is read only and never creates Jobs,
priced Runs, datasets or files on the server. A compiled adapter and a fixture
pass do not establish that a deployed platform fulfills these requirements.

### Run the synthetic transport checks

No real account, session token, priced operation or private platform is needed
for these source-distribution checks. Prerequisites are the supported locked
Rust toolchain/dependencies, Python 3.10+ and permission to bind loopback sockets:

```sh
cargo test --locked -p qspquest-mcp --features managed --test managed \
  governed_preview_digest_idempotency_session_and_free_boundary -- --nocapture
python3 -m unittest discover -s scripts/tests \
  -p test_managed_artifact_download.py -v
```

The Rust fixture checks authenticated first/final ranges of a 4,800,001-byte
artifact, digest-bound manifests, wrong tenants, free sessions, occupied slots
and malformed range responses. The Python checks retrieve complete Unicode
content larger than 4 MB, binary and empty content; corrupt chunks, reordered
ranges, wrong identities, expiry and a wrong complete digest fail. These are
synthetic transport checks, not evidence of scientific calculation or deployed
platform authorization, storage or retention.

# Retrieve a complete governed result artifact

Use this workflow to retrieve every byte of an artifact produced by a governed
Job, including results larger than 4 MB. It checks transport integrity and
retention. It does not establish native execution, statistical convergence or
scientific qualification; inspect those separately in the retained result.

## Prerequisites

You need Python 3.10+, a managed QSPQuest JSON HTTP endpoint built with `managed`,
a verified admitted session with access to the destination Project, and an
artifact ID returned by that Project's Job. The configured platform must support
the explicit retained-byte contract below. A successful metadata lookup or
download link alone does not promise complete MCP retrieval. Public HTTP and
offline stdio do not provide managed sessions. Offline retained results use the
[local result-artifact workflow](qspquest://guide/tables).

The [complete client](qspquest://documentation/files/docs/examples/managed_artifact_download.py) uses only the
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

Choose a new directory whose parent exists. On POSIX systems the client creates
a private directory and files. It initializes the endpoint, calls `get_artifact`
and requires `artifact_retrieval.state: available`. It saves `descriptor.json`,
streams checked chunks into `artifact.partial`, and verifies the complete byte
count and SHA-256 before renaming the payload to `artifact.bin`. `checks.json`
records the outcome. On failure it retains a labeled partial file and failure
receipt; do not interpret partial bytes as a complete result. Existing files are
never replaced. The client disables redirects and environment proxy forwarding.

The command prints `status: passed`, the exact byte count, SHA-256 and number of
chunks. `scientific_qualification` remains `not_assessed`. If the descriptor's
media type is JSON, parse the verified file with:

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

The client treats content as opaque bytes. Parsing it does not validate its
scientific schema or infer units from labels. Keep the descriptor and checks
with the payload when interpreting every row, event side, selected output and
execution identity inside that result.

## Agent contract and limits

`get_artifact` returns authorized `managed_artifact` metadata and an explicit
`artifact_retrieval` state: `available`, `unavailable`, or `expired`. An available
state includes the immutable descriptor and `full_result_resource`. Its URI
contains both the artifact ID and complete SHA-256. Read it with `resources/read`
using the same verified session, then read `chunk_uri_prefix + index` for every
index from zero through `chunk_count - 1`.

Each JSON chunk contains its index, byte offset, byte length, artifact URI,
artifact SHA-256, chunk SHA-256 and base64 data. Decode bytes before joining;
UTF-8 characters may span ranges. Verify every field and the final whole-file
digest. Schemas are available at `qspquest://schema/managed-result-artifact` and
`qspquest://schema/managed-artifact-chunk` in managed builds. Resources require
fresh authorization for every request; knowing a URI does not grant access.
Oversized unused upstream metadata is explicitly omitted with its digest under
`platform_response`; the typed artifact, retrieval state and complete-byte URI
remain available within the response budget.

The adapter limits artifacts to 128,000,000 bytes, chunks to 48,000 bytes and
declared retention to seven days. The actual descriptor supplies creation and
expiration times and the retention duration. Each resource read has a 15-second
server deadline and shares the global and two-per-tenant compute/retrieval
slots. Retry the unchanged URI after a transient busy or timeout response and
before expiration. The client makes no automatic retries or partial resumption.

`managed_artifact_expired` explicitly reports expiration. A changed digest,
ETag, range, media type, truncated response or oversized response fails. A
missing upstream record remains a metadata lookup failure; it is not fabricated
as an expired or successful artifact. Legacy descriptors report
`managed_artifact_bytes_unavailable`. Select the governed platform's supported
route or use a supported local workflow; the adapter does not fetch an arbitrary
external download link or forward session credentials to another host.

## Platform byte contract

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

## Run the synthetic transport checks

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

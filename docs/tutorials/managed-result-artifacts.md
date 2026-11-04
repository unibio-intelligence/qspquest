# Retrieve a complete governed result artifact

Use this workflow to download a complete result file from a hosted Job,
including results larger than 4 MB. It checks that the download is complete and
intact. It does not check whether the fit converged or the science is sound;
check that in the result itself.

## Prerequisites

You need Python 3.10+, a managed QSPQuest JSON HTTP endpoint built with `managed`,
a signed-in session with access to the Project, and an artifact ID returned by
that Project's Job. The platform must support the byte-download requirements
below. Public HTTP and offline stdio have no managed sessions; for local results
use the [local result-artifact workflow](mcp-result-resources.md).

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

## Agent contract and limits

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

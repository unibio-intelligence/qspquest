# Choose an MCP deployment profile

Run the MCP server locally over stdio, as an open HTTP server, or in managed
(signed-in) mode. You need the source archive, Rust 1.98.1, a native compiler,
Python 3 for the optional HTTP client below, and permission to open local
network ports. Run commands from the source root. No account is needed.

## Prepare a synthetic library

The source includes an oral-dose teaching model. Copy it to its own directory
and build the server:

```sh
mkdir -p .qspquest-tutorial-models
cp bindings/mcp/assets/templates/oral.qsp .qspquest-tutorial-models/oral.qsp
export QSPQUEST_LIBRARY_DIR="$(cd .qspquest-tutorial-models && pwd)"
cargo build --locked -p qspquest-mcp
```

For real work, use a model library and check each model's limitations.

## Local stdio

```sh
target/debug/qspquest-mcp --profile local --library-dir "$QSPQUEST_LIBRARY_DIR"
```

The process waits for an MCP client on stdio; Ctrl-C exits. This is also the
default when neither `--profile` nor `--http` is given. Configure your client
with the absolute executable path and these arguments. CSV export needs a
private `--output-dir` (not available over HTTP). Stored results are size-limited
and expire; `qspquest://capabilities` gives the limits.

## Public bounded HTTP

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

## Managed HTTP

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

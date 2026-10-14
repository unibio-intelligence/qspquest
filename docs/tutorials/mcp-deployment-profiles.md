# Choose an MCP deployment profile

Purpose: run local stdio or intentionally public HTTP, then verify the managed
boundary with a complete synthetic test. Prerequisites: the released source
archive, Rust 1.98.1, its locked build dependencies, a native compiler, Python 3
for the optional HTTP client below, and permission to bind loopback sockets.
Run commands from the source root. No account or paid operation is used here.

## Prepare a synthetic library

The released source contains a complete oral-dose teaching model. Copy it to a
separate source-only directory and build the default server:

```sh
mkdir -p .qspquest-tutorial-models
cp bindings/mcp/assets/templates/oral.qsp .qspquest-tutorial-models/oral.qsp
export QSPQUEST_LIBRARY_DIR="$(cd .qspquest-tutorial-models && pwd)"
cargo build --locked -p qspquest-mcp
```

This example uses only files shipped in the source distribution. For scientific
work, select a verified model library and inspect its qualification records.
The teaching model does not qualify a clinical prediction.

## Local stdio

```sh
target/debug/qspquest-mcp --profile local --library-dir "$QSPQUEST_LIBRARY_DIR"
```

The process prints its local profile to stderr and waits for an MCP client on
stdio. Ctrl-C exits. Omitting both `--profile` and `--http` selects this same
profile. Configure the client with the absolute executable path and these args.
Model inputs and results travel between that client and the local process.
CSV export requires an explicit private `--output-dir`; HTTP refuses this option.
Local result resources are bounded and expire according to the capabilities.

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

Expect the discovered tool names, including `simulate`. No credential is needed:
this profile intentionally exposes bounded computation. Omitting `--profile`
with `--http` selects public HTTP unless managed mode was explicitly configured.
`/healthz` reports process readiness. The default build refuses HTTP user-source
execution; use `sh scripts/build-mcp.sh` for the sandbox build. HTTP has no local
CSV exports or result-resource cache. Input and result bodies travel to this
server. Use HTTPS at the ingress for remote clients. Binding `0.0.0.0` or `::`
exposes the listener beyond loopback; startup reports that exposure.

Native clients normally omit `Origin`. All requests carrying a browser Origin
are refused unless it is explicitly allowed. For a browser client, set:

```sh
export QSPQUEST_MCP_ALLOWED_ORIGINS=https://app.example.test
export QSPQUEST_MCP_ALLOWED_HOSTS=mcp.example.test
```

These are configuration examples: replace both with the actual browser origin
and MCP request Host. Origins match scheme, hostname and exact port; HTTPS's
default port is 443. Other ports, `null` and wildcards are refused. Host entries
are comma-separated hostnames or IP addresses with optional ports. Defaults
allow `localhost`, `127.0.0.1` and `[::1]`. Forwarded Host headers do not replace
the actual Host. Allowing an Origin does not create authentication or enable
cross-origin browser access in a client that otherwise forbids it.

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
These checks do not qualify a deployed resolver's OAuth validation, platform
atomicity, prices or scientific operations.

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

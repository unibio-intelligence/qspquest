# Build only the workflow you need

Use a matching binary wheel or CLI archive when provided by a release. A Python
wheel needs CPython 3.10 or later and no Rust compiler. Core simulation and the
analytic doctor need no analysis or notebook packages:

```sh
python3 -m venv .venv
. .venv/bin/activate
# Replace this filename with your downloaded wheel for the same OS and CPU.
python -m pip install ./qspquest-0.1.0-cp310-abi3-macosx_11_0_arm64.whl
qspquest-doctor --json
```

On Windows, activate with `.venv\Scripts\Activate.ps1` and use the Windows
wheel. QSPQuest 0.1.0 installs from source or downloaded release files, not
from package or image registries. The [installation guide](installation.md) gives
the supported local-file routes and source-build prerequisites.

## Source builds

Prerequisites: Rust 1.98.1 and a native compiler. Run from the complete source
release root. The default workspace build selects shared types, the engine and
the CLI, so it does not build the Python, R, MCP or WASM interfaces:

```sh
cargo build --release --locked
target/release/qspquest run conformance/models/synthetic-one-compartment.qsp \
  docs/examples/first-run.json > simulation.json
```

The synthetic concentration is 2 mg/L at 0 h and about 0.181436 mg/L at 24 h.
The source build downloads its pinned dependencies on first use.

Select other workflows explicitly:

| Workflow | Command | Additional prerequisites |
| --- | --- | --- |
| Python core | `python -m pip install .` | CPython 3.10+, pinned build backend |
| Python analysis | `python -m pip install '.[analysis]'` | NumPy and SciPy |
| Local native MCP | `sh scripts/build-mcp.sh --features ""` | POSIX shell |
| MCP steady-state/Morris tools | `sh scripts/build-mcp.sh --features analysis` | POSIX shell |
| MCP isolated source and browser widget | `sh scripts/build-mcp.sh --features sandbox,widget` | POSIX shell, wasm-bindgen-cli 0.2.129, wasm32 Rust target |
| Standalone R source archive | `python3 scripts/prepare-r-source.py /tmp/qspquest-r-source` | Python 3.10+; R/C/Rust for installation |

The MCP helper installs the wasm32 target only for `sandbox` or `widget`.
If widget prerequisites are missing or the feature list is invalid, it stops
with a clear error before building. The native MCP executable is
still `target/release/qspquest-mcp`. Follow its
[connection guide](../bindings/mcp/README.md) to run it against a local library.

To test every interface:

```sh
cargo test --workspace --locked
```

This needs the Python and R environments set up. A core CLI build does not
test those interfaces.

## MCP container feature selection

Prerequisite: Docker with BuildKit. The default public MCP target includes
`sandbox,widget`. To build a native MCP image without those assets:

```sh
docker build -f bindings/mcp/Dockerfile --target public \
  --build-arg MCP_FEATURES= -t qspquest-mcp:native .
```

Use that image's stdio entrypoint for offline work. Native library models are
available; hosted user-source execution requires `sandbox`, and browser
bindings require `widget`. Inspect `qspquest://capabilities` for the actual
features and configuration. All targets include licenses, notices and package
inventories.

The MCP build caches locked Cargo registry dependencies by architecture.
Reusing the cache avoids repeat downloads; checksums and pinned versions are
still verified.

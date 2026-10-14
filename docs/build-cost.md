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
wheel. QSPQuest 0.1.0 uses source builds and release-file installation;
package and image registry installation is not available. The [installation guide](installation.md) gives
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
The output records event sides, units and engine identity. The source build
fetches locked dependencies on first use. It does not establish clinical or
general model qualification.

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
It builds the isolated ABI only for `sandbox` and browser bindings only for
`widget`. If widget prerequisites are absent or feature syntax is invalid, it
fails with an actionable error before building. The native MCP executable is
still `target/release/qspquest-mcp`. Follow its
[connection guide](../bindings/mcp/README.md) to run it against a local library.

Full workspace checks retain every interface:

```sh
cargo test --workspace --locked
```

These checks require the documented Python and R environments. A core CLI
build does not demonstrate that those interfaces work.

## MCP container feature selection

Prerequisite: Docker with BuildKit. The default public MCP target retains
`sandbox,widget`. To build a native MCP image without those assets:

```sh
docker build -f bindings/mcp/Dockerfile --target public \
  --build-arg MCP_FEATURES= -t qspquest-mcp:native .
```

Use that image's stdio entrypoint for offline work. Native library models are
available; hosted user-source execution requires `sandbox`, and browser
bindings require `widget`. Inspect `qspquest://capabilities` for the actual
features and configuration. The managed image retains its required managed,
sandbox and widget build independently of a custom public feature selection.
All targets retain licenses, notices and package inventories.

The MCP build caches locked Cargo registry dependencies by architecture.
Reusing a cache can avoid repeated downloads; it does not remove checksum
verification or pinning. Cold and warm build/install time, artifact and layer
sizes, and native feature smoke checks must be measured on each supported
target. No size or speed reduction is claimed without those measurements.

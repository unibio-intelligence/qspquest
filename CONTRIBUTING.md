# Contributing to QSPQuest

Report suspected vulnerabilities privately using the [security policy](SECURITY.md).

Start with an issue describing the scientific use case and a small reproducible
example. For a bug, include model text, a minimal request, expected behavior,
actual output or structured error, environment details from `qspquest-doctor
--json`, and the exact version/commit. Remove credentials and confidential data.

## Development

Use the locked Rust toolchain/dependencies declared by the checkout. Python
bindings require Python 3.10+ and maturin 1.15.0:

```sh
python -m venv .venv
# Activate .venv using your shell's normal command.
python -m pip install maturin==1.15.0
maturin develop --locked
python -m pip install '.[notebook,docs]' pytest
cargo test --workspace --locked
cargo build --locked -p qspquest-cli
# Set QSPQUEST_TEST_CLI to the built CLI for CLI-dependent Python tests.
python -m pytest bindings/python/tests
python scripts/build_docs.py
```

Add focused tests for changed behavior. Scientific changes need an independent
reference, explicit units and event sides, predeclared numerical tolerances, and
visible negative or unsupported cases. Generated recovery observations must be
labelled as generated; known model provenance must remain separate from clinical
validation. Do not report convergence, method support or model qualification
outside the evidence's declared scope.

Keep JSON serialization stable, document capability changes, and update the
unreleased changelog. Optional pandas, plotting and analysis dependencies must
not become requirements for core simulation imports. Test cancellation and
checkpoint compatibility whenever analysis state changes.

## Models, datasets and licenses

Provide the exact upstream source version, citation, license, equation mapping,
units and limitations for examples. Do not add restricted publisher assets or
unlicensed data. Model/example licenses can differ from the engine's Apache-2.0 license; retain all notices in their own directories.

## Review and release

Open a focused pull request with the problem, behavior change, and validation.
Maintainers review scientific correctness, compatibility, provenance and scope.
The paper and package release must identify the exact reviewed source. A passing
installation self-check does not qualify a model or estimation method.

## Automated checks

Code changes run formatting, core and CLI tests, and browser-client tests.
Use the manual CI workflow for the full workspace, Wasm target and native R
checks. MCP qualification and Docker builds run for relevant pull requests and changes
to `main`, including build configuration under `.cargo/`. Suite compatibility
checks exercise pinned model and validation snapshots; see
[the compatibility guide](docs/suite-compatibility.md) for a local run. Installation artifacts
and documentation/tutorials run when a maintainer starts their workflow.
These workflows build and test artifacts; publishing packages, images and
releases requires a separate maintainer action.

## Build input updates

Workflow actions use full commit IDs with readable version comments. Container
bases and the Dockerfile frontend use SHA-256 digests. Review the exact source
and execution inputs before changing a pin; preserve the selected toolchain
version. Run `python3 scripts/check_build_pins.py` and the affected installation,
container and numerical checks. Changes to workflow pins or container inputs
trigger the build and MCP numerical workflows. Retain the tested image identity
and its dated inventories under `/opt/qspquest/inventories/`, including resolved
OS packages and Python packages where applicable. These inventories describe
the tested build; package-manager updates can produce a different inventory
even when the base digest stays fixed. Consumer dependency ranges remain
independent of the container build constraints.

# Contributing to QSPQuest

Contributions are welcome: bug reports, fixes, documentation, examples and new
scientific capabilities.

## How to contribute

1. **Open an issue first.** Describe the problem or scientific use case with a
   small reproducible example. For a bug, include model text, a minimal request,
   expected behavior, actual output or structured error, `qspquest-doctor --json`
   output and the exact version or commit. Except for typo fixes, wait for a
   maintainer to agree on the approach before you open a pull request.
2. **Open a focused pull request.** Make one change per pull request, link its
   issue (`Closes #123`), and include tests and documentation for the change.
3. **Review and release.** Maintainers review scientific correctness,
   compatibility, provenance and scope. Accepted changes pass the full test
   suite before they appear in a release, and you are credited.

Contributions are licensed under the repository's [Apache-2.0 license](LICENSE).
Remove credentials and confidential data from issues. Report suspected
vulnerabilities privately using the [security policy](SECURITY.md).

## Development

Use the locked Rust toolchain/dependencies declared by the checkout. Diffsol is fetched
from its official repository at the exact revision in `Cargo.lock`. Initial
builds require network access or an already populated Cargo dependency cache. Python
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
visible negative or unsupported cases. Label generated observations as
generated, and do not present a model's source as clinical validation. Do not
claim convergence, method support or model validity beyond what the evidence shows.

Keep JSON serialization stable, document capability changes, and update the
unreleased changelog. Optional pandas, plotting and analysis dependencies must
not become requirements for core simulation imports. Test cancellation and
checkpoint compatibility whenever analysis state changes.

## Models, datasets and licenses

Provide the exact upstream source version, citation, license, equation mapping,
units and limitations for examples. Do not add restricted publisher assets or
unlicensed data. Model/example licenses can differ from the engine's Apache-2.0 license; retain all notices in their own directories.

A release identifies the exact reviewed source. A passing installation
self-check does not qualify a model or estimation method.

## Automated checks

Code changes run formatting, the Rust, Python and script tests, and the
browser-client tests. Use the manual CI workflow for the full workspace, Wasm
target and R package tests. MCP contract tests and Docker builds run for
relevant changes to `main`, including build configuration under `.cargo/`.
Documentation and tutorials run when a maintainer starts their workflow. These workflows
build and test artifacts; publishing packages, images and releases requires a
separate maintainer action.

## Validation

Numerical qualification of the engine and the compatibility check against the
model library live in the
[qspquest-validation](https://github.com/unibio-intelligence/qspquest-validation)
repository. They run against an engine checkout and record evidence for an
exact source revision. Changes to solver behaviour need a new validation run
before release.

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

## Rust registry releases

`qspquest-core` and `qspquest-modeling` are published together from a version tag
through the Release workflow. Both crates inherit the Cargo workspace version,
which must match the Python version and the `vMAJOR.MINOR.PATCH` tag. Before
creating a release tag, review the complete source and run:

```sh
cargo publish --dry-run --locked --registry crates-io -p qspquest-core -p qspquest-modeling
```

A manual Release workflow run performs this package check without uploading.
For a tag release, the `crates` job waits for the GitHub release, the package
check and approval of the `crates-io` environment. It publishes core before
modeling using a short-lived OIDC credential. No saved Cargo token or GitHub
registry secret is required. Check both crates.io version pages and run a fresh
consumer build from the registry after publication. If an upload partially
succeeds, inspect the registry before retrying; published versions cannot be
overwritten.

Each crate's crates.io Trusted Publishing configuration must name owner
`unibio-intelligence`, repository `qspquest`, workflow `release.yml` and
environment `crates-io`. Keep the GitHub environment restricted to release tags
with a required maintainer review. The authentication action's full source pin
is maintained in the workflow with the other build inputs.

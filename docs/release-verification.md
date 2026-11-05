# Verify a release artifact

Every GitHub release attaches the wheels, the source archive, an SPDX SBOM,
build-provenance and SBOM attestation bundles, and a `SHA256SUMS` file. The MCP
server image on GHCR carries the same two attestations. Verify a download with
the [GitHub CLI](https://cli.github.com/) before installing it.

## Files

Check the file against the release's checksum list, then verify that GitHub
Actions built it from this repository's release workflow:

```sh
sha256sum --check --ignore-missing SHA256SUMS
gh attestation verify qspquest-0.1.0-cp310-abi3-manylinux_2_28_x86_64.whl \
  --repo unibio-intelligence/qspquest \
  --signer-workflow unibio-intelligence/qspquest/.github/workflows/release.yml
```

On macOS use `shasum -a 256 --check --ignore-missing SHA256SUMS`. Use the same
`gh` command for the source archive. To check the SBOM attestation as well, add
`--predicate-type https://spdx.dev/Document/v2.3`.

## MCP image

Verify the image by digest, not by a mutable tag:

```sh
gh attestation verify oci://ghcr.io/unibio-intelligence/qspquest-mcp@sha256:DIGEST \
  --repo unibio-intelligence/qspquest \
  --signer-workflow unibio-intelligence/qspquest/.github/workflows/release.yml
```

A failed signature, a different repository or workflow, or a changed digest
makes `gh` exit with an error; do not install that artifact. Attestations show
who built an artifact and from which source. They do not check SBOM
completeness, licenses, security advisories or scientific validity.

## Check Python distribution contents

Before distributing built wheels or source archives, run from their matching
complete source checkout, with installation files in `wheelhouse/`:

```sh
python3 scripts/check_distribution.py wheelhouse --out distribution.json
```

Choose a new report filename. The checker requires the complete Python package,
legal notices, native extension and wheel record hashes. Source archives must
match this checkout's build inputs byte for byte; unsafe paths, links, duplicate
members and excluded development files are refused. It does not check installed
behavior, platform compatibility, dependency inventories or signatures.

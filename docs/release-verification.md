# Verify a release artifact

Verify the downloaded artifact and its SBOM before installation. You need Python
3.10 or later, GitHub CLI `gh` with the attestation verification flags shown below,
the matching complete QSPQuest project source archive or checkout, and these four
release files:

- The wheel, CLI/MCP archive, WASM archive, R archive or complete source archive.
- Its build-provenance bundle.
- Its SPDX 2.x or CycloneDX 1.x JSON SBOM.
- Its SBOM-attestation bundle.

Obtain the expected artifact/SBOM SHA-256 hashes, public repository, build workflow,
source commit and signer commit from trusted release metadata. A checksum received
only from the same untrusted download does not establish its origin. Both commits
must be full 40-character Git revisions; both checksums must be full 64-character
SHA-256 values. The supported workflow builds and attests in the same hosted job
in that repository. Use this verification workflow only when the release supplies all four files;
source-only downloads do not include signed binary artifacts.

The Python and R installation source archives do not include this verifier;
use the complete project source containing `scripts/verify_release_provenance.py`.

The verifier uses the installed GitHub CLI for signature, certificate and identity
verification. Its flag contract has been checked against `gh` 2.92.0. The CLI may
need network access to obtain trust metadata. An existing configured CLI can be
used; verification does not publish artifacts or change repository contents.

From the matching complete source release, set the values from its trusted release
metadata and the paths of your downloads. The following example uses a CLI archive:

```sh
ARTIFACT='./downloads/qspquest-cli-0.1.0-x86_64-unknown-linux-gnu.zip'
PROVENANCE_BUNDLE='./downloads/cli.provenance.jsonl'
SBOM='./downloads/cli.sbom.json'
SBOM_BUNDLE='./downloads/cli.sbom-attestation.jsonl'
REPOSITORY='unibio-intelligence/qspquest'
WORKFLOW='.github/workflows/native-installation.yml'
SOURCE_COMMIT='<full source commit from trusted release metadata>'
SIGNER_COMMIT='<full signer commit from trusted release metadata>'
ARTIFACT_SHA256='<artifact SHA-256 from trusted release metadata>'
SBOM_SHA256='<SBOM SHA-256 from trusted release metadata>'

python3 scripts/verify_release_provenance.py \
  --artifact "$ARTIFACT" --sha256 "$ARTIFACT_SHA256" \
  --provenance-bundle "$PROVENANCE_BUNDLE" \
  --sbom "$SBOM" --sbom-sha256 "$SBOM_SHA256" \
  --sbom-bundle "$SBOM_BUNDLE" \
  --repository "$REPOSITORY" --workflow "$WORKFLOW" \
  --source-commit "$SOURCE_COMMIT" --signer-commit "$SIGNER_COMMIT" \
  --out release-verification
```

Replace the four angle-bracket values before running the command. Use the actual
artifact, file names, repository and workflow from that release; the example does
not announce download availability. Select a new output directory for each run.
Wheels and other file artifacts use the same command with their own release files.

For a container, replace `--artifact "$ARTIFACT"` with
`--image "oci://registry.example/qspquest@sha256:$ARTIFACT_SHA256"`, using the actual
release registry and repository. A mutable image tag is refused. The GitHub CLI
must have access to that registry to inspect the exact image digest; retain the
same provenance, SBOM, checksum and workflow arguments.

Success prints `status: verified` and writes `release-verification/report.json`
alongside the two raw verifier stdout/stderr pairs. It checks both file hashes,
the exact artifact subject in both attestations, the expected predicate types,
source repository and commit, signer workflow and commit, and GitHub hosted
execution. It also checks the build predicate's source/workflow declarations and
compares the signed SBOM object with the supplied SBOM. Object-key order is
irrelevant; array order, field values and JSON types must agree. Inputs changing
during verification fail the check. The report includes the verifier executable,
script, input digests and exact verification commands.

Signature or identity failure, a wrong digest/commit/workflow, self-hosted execution,
an altered SBOM, malformed verifier output or a timeout exits with failure. Retain
the evidence and obtain the correct matching release files; do not drop identity
flags, substitute an unsigned SBOM or treat plausible JSON as a verified signature.
Missing `gh` or unsupported flags require a compatible CLI. A preexisting output
directory is preserved; choose another name.

Attestations establish signed artifact identity and build claims. SBOM completeness,
license acceptance, advisory status, installation behavior and scientific
qualification have separate release checks. Verification does not extend model
qualification to a changed request or establish that a deployed service is updated.

## Check Python distribution contents

Before distributing built wheels or source archives, run from their matching
complete source checkout, with installation files in `wheelhouse/`:

```sh
python3 scripts/check_distribution.py wheelhouse --out distribution.json
```

Choose a new report filename. The checker requires the complete released Python
package, legal notices, native extension and wheel record hashes. Source archives
must contain the selected workspace build inputs with identical bytes; unsafe
paths, links, duplicate members and excluded development files are refused.
It checks archive integrity against this checkout. Installed behavior, platform
compatibility, dependency inventories and release signatures require their
separate checks.

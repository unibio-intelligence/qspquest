# Run QSPQuest with Docker

To start with your agent, follow the [MCP Docker quickstart](../tutorials/agents-docker.md).
Its image includes a synthetic model and needs no library mount for the first run.

Docker supplies the engine and its language dependencies. The `runtime` target contains
CLI/Python analysis and interchange, `notebook` adds Jupyter and plotting, and
`r` adds R, jsonlite and the installed native R package. A separate MCP image
serves clients over HTTP or stdio. You need Docker with Compose on the host;
Rust, Python and R are supplied inside the images. Build from the complete public
QSPQuest source. Registry images are not yet published.

## Mount the model library once

From the engine source root, clone the companion model library beside it.
Its root contains `catalog.json`, `models/` and `qspquest_models/`:

```sh
# Run from the QSPQuest source root.
git clone https://github.com/unibio-intelligence/qspquest-models.git ../qspquest-models
export QSPQUEST_LIBRARY_DIR="$(cd ../qspquest-models && pwd)"
docker compose build models
docker compose run --rm models --search rituximab
docker compose run --rm models --model qspquest_serkland_2025_rituximab_ms_pkpd
```

If you already have the matching model-library checkout or extracted archive,
skip the clone and point `QSPQUEST_LIBRARY_DIR` at that directory.

Compose mounts the library read-only at `/models` and configures its loader.
The engine is installed in the image; no separate `pip install` of the model
library is needed. Discovery includes inspection-only models; executable
scenarios are listed separately. Read the selected version's model card before
interpreting results. The loader verifies catalog/archive and member hashes.
The [shared-model guide](shared-models.md) explains the execution contract.

Run a complete named scenario and preserve its output:

```sh
mkdir -p results
docker compose run --rm --user "$(id -u):$(id -g)"   --volume "$PWD/results:/work" models   --model qspquest_serkland_2025_rituximab_ms_pkpd   --scenario population-typical-1000mg-full-ode   --csv /work/rituximab.csv --json /work/rituximab.json
```

This produces 201 sample times from day 0 to day 200, with event sides,
`rituximab_concentration_mg_l`, `cd19_cells_mm3` and the other model outputs.
The JSON retains units, engine identity and the archive checksum. CSV contains
time, side and output columns. The run needs no network access; the build
requires network access for locked dependencies. The user override lets the
container write the host directory on Linux; on Windows, omit that option and
use an absolute writable mount path.

## Use the shared library in Python

```sh
docker compose run --rm --entrypoint python models -c '
import qspquest
library = qspquest.model_library()
print(len(library.list_models()))
model = library.load("qspquest_serkland_2025_rituximab_ms_pkpd")
result = model.run("population-typical-1000mg-full-ode")
assert len(result["times"]) == 201
print(result["scenario_id"], result["archive_sha256"])
'
```

`qspquest.model_library()` uses the configured `/models` mount. Native Python
users can install the model-library package and pass its directory explicitly.
A library load does not qualify arbitrary edits or provide a fitting contract.

## Use R in Docker

```sh
docker compose build r
docker compose run --rm r -e '
library(qspquest)
models <- model_library()
print(length(models$models()))
result <- models$run("qspquest_serkland_2025_rituximab_ms_pkpd",
                    "population-typical-1000mg-full-ode")
stopifnot(length(result$times) == 201L)
print(head(result_data_frame(result)))
'
```

The R shared-library helper uses the same verified Python loader and installed
engine inside the image. Native R modeling and fitting use the R binding
directly. Run the complete native R tutorial with
`docker compose run --rm r /opt/qspquest/r_quickstart.R`.
For local R/RStudio installations instead, see [native installation](../installation.md#r).

## Connect an MCP client to the same catalog

```sh
docker compose build mcp
docker compose up mcp
```

Connect the client to `http://127.0.0.1:8080/mcp`. `search_models`,
`get_scenario_request` and `simulate` use the same mounted catalog and presets.
Stop with Ctrl-C. This example binds only to localhost. For stdio or a pinned
HTTPS library download with a persistent verified cache, see the
[MCP setup](../../bindings/mcp/README.md). A hosted library release URL and
checksum must be published before automatic remote setup can be used.

## Engine-only image

For your own models without a shared library mount:

```sh
docker build --target runtime -t qspquest:local .
docker run --rm qspquest:local
docker run --rm qspquest:local qspquest --version
```

The default command checks the installed engine with `qspquest-doctor --analysis`.

## Use the CLI or Python

Mount inputs read-only and write results to a separate local directory. Files
must be readable by the container's user. This example validates the synthetic absorption
PK model, without needing a run request:

```sh
docker run --rm --network none --read-only --tmpfs /tmp \
  --mount "type=bind,source=$PWD/examples/synthetic,target=/inputs,readonly" \
  qspquest:local qspquest validate /inputs/absorption.qsp

# If this directory also contains your own run.json:
docker run --rm --network none --read-only --tmpfs /tmp \
  --mount "type=bind,source=$PWD/examples/synthetic,target=/inputs,readonly" \
  qspquest:local qspquest run /inputs/absorption.qsp /inputs/run.json > result.json
```

For Python scripts, mount their directory and invoke `python /inputs/script.py`.
Use a writable `/work` mount for reports and checkpoints. The default user is
UID/GID 10001. On Linux, `--user "$(id -u):$(id -g)"` allows the container to write
using your own identity. Mount paths must be absolute. Keep the same image and
scientific inputs when resuming checkpoints.

## Open the notebooks

With the library path configured above, Compose makes it available in Jupyter:

```sh
mkdir -p notebook-work
# On macOS/Linux, allow the notebook to write your mounted directory.
export QSPQUEST_UID="$(id -u)" QSPQUEST_GID="$(id -g)"
docker compose build notebook
docker compose up notebook
```

In a notebook, use `import qspquest; library = qspquest.model_library()` and
the Python example above. To run teaching notebooks without a shared library:

```sh
docker build --target notebook -t qspquest:notebook .
mkdir -p notebook-work
docker run --rm --init -p 127.0.0.1:8888:8888 \
  --user "$(id -u):$(id -g)" \
  --mount "type=bind,source=$PWD/notebook-work,target=/work" \
  qspquest:notebook
```

Open the localhost URL printed by Jupyter and use its generated token. The
image keeps Jupyter authentication enabled. Tutorial files are copied into
`notebook-work/qspquest-tutorial` on first use; later starts preserve your edits.
Reports and checkpoints remain in that local directory after the container
stops. The container listens on port 8888; the example publishes it only on the
host's loopback address. Stop with Ctrl-C.

Use **Restart Kernel and Run All Cells** to simulate the independently authored absorption model, check its analytic concentration solution, and verify a standard COMBINE/OMEX round trip. These illustrative examples use Apache-2.0.

The [oral PK notebook](../tutorials/oral-pk-analysis.md) generates its synthetic
observations on first run and demonstrates individual fitting, residual diagnostics
and a saved report.

## Verify and preserve an image

```sh
python scripts/test_container.py --image qspquest:local --out runtime-check.json
python scripts/test_container.py --image qspquest:notebook --notebook --out notebook-check.json

docker run --rm qspquest:local cat /opt/qspquest/build-info.json
docker run --rm qspquest:local cat /opt/qspquest/requirements.freeze.txt

docker save qspquest:local | gzip > qspquest-runtime.tar.gz
# Load a reviewed local or CI artifact on another matching machine:
gunzip -c qspquest-runtime.tar.gz | docker load
```

The checks run the installed image without external network access, as a
non-root user with a read-only root filesystem. They compare CLI and Python
trajectories and execute the synthetic archive round trip. For the notebook image, they execute the synthetic notebook. These checks validate this installation/workflow.

Base image versions and multi-platform digests are pinned in the Dockerfile.
Cargo uses its lockfile; numerical dependencies are constrained to reviewed
versions. `build-info.json` records the wheel/CLI/Cargo-lock hashes and Rust
version. The resolved Python environment and dependency license notices are
retained in the image. The narrow build context excludes internal documents,
credentials, test evidence and generated results. Compiler/build caches remain
outside the runtime layers.

Inspect the installed OS package and copyright records without network access:

```sh
docker run --rm --network none qspquest:local cat /opt/qspquest/inventories/runtime/os-packages.tsv
docker run --rm --network none qspquest:local cat /opt/qspquest/inventories/runtime/os-copyright.tsv
```

The copyright table lists package, version, architecture, status, original document
path, retained document path and SHA-256. Retained paths are relative to the same
`inventories/runtime` directory; builder records are under `inventories/builder`.
Multiarch package names use underscores in retained directory names. The complete
package table also retains records that are no longer installed. An installed
package with a missing or empty copyright document makes the image build fail.
These records describe package files and identities; they do not establish a
license-policy decision or cover software outside the package database.

The image targets are Linux ARM64 and Linux AMD64. Retain passing checks for
each exact image before treating its workflows as verified. These images run on Docker hosts
including macOS and Windows, but they are Linux containers. Build the images locally; registry downloads are not available yet. The Docker workflow builds, tests and
uploads image archives for the configured architectures and targets.

Both targets include `qspquest[interchange]` and libSBML. Run `qspquest-doctor --interchange` to check the installed archive round-trip path. The notebook target also includes the [oral PK archive tutorial](interchange.md).

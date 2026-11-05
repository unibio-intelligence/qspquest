# Install QSPQuest

Install the Python package from PyPI. Prebuilt wheels need no Rust toolchain or
compiler:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install qspquest
qspquest-doctor --json
python -m qspquest.first_run --out first-run
```

On Windows PowerShell, activate with `.venv\Scripts\Activate.ps1`. The doctor runs
a small analytic model and reports its checks; the first run writes six result
files to `first-run/` (use a new directory name for each run). The
[README](../README.md#install) explains the example. Add
[optional dependencies](#optional-dependencies) as extras, for example
`python -m pip install 'qspquest[notebook,interchange]'`.

For an MCP client, run the published server image; it includes a synthetic model
and needs no extra arguments:

```sh
docker run --rm --init --publish 127.0.0.1:8080:8080 --read-only \
  --cap-drop ALL --security-opt no-new-privileges \
  ghcr.io/unibio-intelligence/qspquest-mcp:latest
```

Connect your client to `http://127.0.0.1:8080/mcp`, or follow the
[agent and Docker quickstart](tutorials/agents-docker.md) for stdio.

| Interface | Prerequisites | Start here |
| --- | --- | --- |
| Python | CPython 3.10+ on a [wheel platform](#python-wheels) | `pip install qspquest` |
| MCP server | Docker Engine or Desktop | [Agent and Docker quickstart](tutorials/agents-docker.md) |
| CLI | Git, Rust 1.98.1/Cargo and a native compiler | [CLI](#cli) |
| Python from source | CPython 3.10+ and the CLI build tools | [Install from a checkout](#install-from-a-checkout) |
| R | R, its compatible C compiler and Rust 1.98.1/Cargo | [R](#r) |
| Docker (CLI, notebook, R) | Docker Engine or Desktop with Compose | [Docker setup](site/containers.md) |

QSPQuest is not yet published to CRAN or R-universe. The source routes below
need the complete source checkout. If needed, obtain it with:

```sh
git clone https://github.com/unibio-intelligence/qspquest.git
cd qspquest
```

Source builds download locked dependencies on first use and can take several
minutes. Offline builds require those dependencies to be cached already.

## CLI

The default `cargo build --release --locked` builds the engine and CLI only.
Python, R, MCP and WASM have their own install steps. `cargo test --workspace
--locked` tests the whole workspace and needs every language prerequisite.
See [building only the workflow you need](build-cost.md) for feature selection.

From the checkout root:

```sh
cargo build --release --locked -p qspquest-cli
target/release/qspquest run conformance/models/synthetic-one-compartment.qsp \
  docs/examples/first-run.json > simulation.json
```

The concentration is 2.000000 mg/L at 0 h and 0.181436 mg/L at 24 h.
The [README](../README.md#install) explains the example.

## Python

The [Python package guide](../bindings/python/README.md) provides the API's
self-contained synthetic example and also ships with the Python source archive.

### Python wheels

`pip install qspquest` selects a prebuilt `cp310-abi3` wheel, which serves every
CPython from 3.10 on, for these platforms:

| Platform | Wheel tag |
| --- | --- |
| Linux x86_64, glibc 2.28+ | `manylinux_2_28_x86_64` |
| Linux ARM64, glibc 2.28+ | `manylinux_2_28_aarch64` |
| macOS Apple Silicon | `macosx_11_0_arm64` |
| macOS Intel | `macosx_10_12_x86_64` |
| Windows x86_64 | `win_amd64` |

On other platforms (musl Linux, PyPy, 32-bit), pip falls back to the source
archive and needs Rust 1.98.1, Cargo and a native compiler; its build also
fetches one locked dependency from GitHub. Each wheel, the source archive, an
SPDX SBOM, their build-provenance and SBOM attestation bundles and a `SHA256SUMS`
file are attached to the matching
[GitHub release](https://github.com/unibio-intelligence/qspquest/releases).
Check a downloaded file's signed origin with GitHub CLI:

```sh
gh attestation verify qspquest-0.1.0-cp310-abi3-macosx_11_0_arm64.whl \
  --repo unibio-intelligence/qspquest
```

The MCP image carries the same attestations; verify it with
`gh attestation verify oci://ghcr.io/unibio-intelligence/qspquest-mcp:0.1.0 --repo unibio-intelligence/qspquest`.
[Release verification](release-verification.md) describes the complete check,
including the SBOM content.

### Install from a checkout

Use CPython 3.10+, Rust 1.98.1/Cargo and a native compiler. From the complete
QSPQuest source root on macOS or Linux:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install .
qspquest-doctor --json
python -m qspquest.first_run --out developer-results
```

On Windows PowerShell, activate with `.venv\Scripts\Activate.ps1`.
`pip` builds the native extension. The first run checks the analytic solution
and writes six result files to `developer-results/`; use a new directory name for
each run. No separate model library or analysis packages are needed.

If you already have the matching Python source archive, replace `pip install .`
with `python -m pip install ./qspquest-0.1.0.tar.gz`.

### Optional dependencies

Core simulation, native fitting and the structural software adapters are included
in the package. Choose extras for additional Python workflows:

| Extra | Dependencies and purpose |
| --- | --- |
| `analysis` | NumPy and SciPy for analysis helpers |
| `notebook` | Analysis packages, pandas, matplotlib and JupyterLab |
| `interchange` | python-libsbml for SBML/OMEX interchange |
| `docs` | Tools for building the documentation |

Install notebook and interchange support:

```sh
python -m pip install 'qspquest[notebook,interchange]'
qspquest-doctor --analysis --interchange --json
```

From a source checkout, use `'.[notebook,interchange]'` instead and open
`docs/tutorials/oral-pk-analysis.ipynb` with `python -m jupyterlab`.

The [oral PK tutorial](tutorials/oral-pk-analysis.md) includes synthetic data and
a complete fitting workflow. The [Python guide](PYTHON-USER-GUIDE.md) covers the
public API; the [synthetic notebook](tutorials/synthetic.ipynb) needs no external data.
For downloaded packages, attach the same extras to the local filename, such as
`'./qspquest-0.1.0.tar.gz[notebook,interchange]'`.

## R

Install R, a compatible C compiler and Rust 1.98.1/Cargo first. On
macOS, use a compiler and SDK compatible with your R distribution. Windows
source installation requires compatible R build tools and is not yet verified.
Use `jsonlite` 2.0.0 or newer; older releases can round numeric request values.

### Install from a checkout

From the complete source root in a macOS or Linux terminal:

```sh
Rscript -e 'install.packages("jsonlite", repos="https://cloud.r-project.org")'
R CMD INSTALL bindings/r
Rscript -e 'library(qspquest); print(packageVersion("qspquest"))'
```

Expected version: `0.1.0`. Keep the complete checkout for the source build;
copying only `bindings/r` is insufficient. The
[R quickstart](R-USER-GUIDE.md#quickstart) runs a complete model.

### Install a downloaded source archive

The standalone R archive contains its Rust workspace. If you have that matching
artifact, run from its directory:

```sh
mkdir -p "$HOME/R/qspquest-library"
export R_LIBS_USER="$HOME/R/qspquest-library"
export R_HOME="$(R RHOME)"
Rscript -e 'install.packages("jsonlite", repos="https://cloud.r-project.org")'
R CMD INSTALL --library="$R_LIBS_USER" ./qspquest_0.1.0.tar.gz
Rscript -e 'library(qspquest); print(packageVersion("qspquest"))'
```

Expected version: `0.1.0`. Cargo downloads the locked dependencies as needed;
you do not need to run `cargo fetch` separately. The package links to your R
runtime, which must remain installed.

For an offline Linux/macOS CRAN candidate, build the archive from the complete
checkout with `python3 scripts/prepare-r-source.py /tmp/qspquest-cran --cran`.
That variant includes locked Rust dependency sources and legal notices, checks
Cargo and rustc versions, and compiles with at most two jobs. Once `jsonlite`
is installed, its installation does not need network access. It also installs
the complete example at `system.file("doc", "quickstart.R", package = "qspquest")`.

Alternatively, in R or RStudio, with the archive in the working directory and
the build tools on `PATH`:

```r
install.packages("jsonlite", repos = "https://cloud.r-project.org")
install.packages("qspquest_0.1.0.tar.gz", repos = NULL, type = "source")
library(qspquest)
packageVersion("qspquest")
```

If you installed into the terminal example's custom library, start RStudio with
that `R_LIBS_USER`, or set `.libPaths(c(path.expand("~/R/qspquest-library"), .libPaths()))`
before loading the package. Restart R after replacing a loaded native package.
`rxode2` is optional and needed only for the rxode2 UI exporter; native QSPQuest
simulation and fitting do not require it. See the
[R guide](R-USER-GUIDE.md#quickstart) for a complete simulation and fitting workflow.

From the complete project source checkout, maintainers can build the standalone
R installation file:

```sh
# The destination must not already exist.
python3 scripts/prepare-r-source.py /tmp/qspquest-r-source
```

The output is `/tmp/qspquest-r-source/qspquest_0.1.0.tar.gz`. For a direct
checkout installation, use `R CMD INSTALL bindings/r` from the checkout root;
copying only `bindings/r` is insufficient.

## Docker

Docker supplies the engine and its language dependencies. Use the
[Docker setup](site/containers.md) to build the CLI/Python, notebook or R image,
mount the shared model library once, and run named scenarios. No host Rust
compiler or language package installation is needed to run these images. The
CLI/Python, notebook and R images are built locally. The MCP server image is
published as `ghcr.io/unibio-intelligence/qspquest-mcp:<version>` (and `:latest`)
for Linux AMD64 and ARM64; see the [agent quickstart](tutorials/agents-docker.md).

## Downloaded packages

`pip install qspquest` downloads the matching wheel from PyPI. To install a file
downloaded from the GitHub release instead (for example on an offline machine),
pass its path to `pip`. R users install the separate R source archive with
`R CMD INSTALL` or `install.packages`; `install.packages("qspquest")` does not
install this release.

| Installation file | Use | Build tools |
| --- | --- | --- |
| `qspquest-0.1.0-cp310-abi3-macosx_11_0_arm64.whl` | CPython 3.10+ on macOS Apple Silicon | None |
| `qspquest-0.1.0.tar.gz` | Python source installation | Rust 1.98.1, Cargo and a native compiler |
| `qspquest_0.1.0.tar.gz` | R source installation | R, its compatible C compiler, Rust 1.98.1 and Cargo |

The hyphenated `.tar.gz` is the Python package; the underscored archive is the R
package. Neither is the complete project archive `archives/qspquest.tar.gz`.
A matching Python wheel needs neither Rust nor Cargo.

### Install a Python wheel

With CPython 3.10+ on macOS Apple Silicon, run from the directory containing the
wheel:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install ./qspquest-0.1.0-cp310-abi3-macosx_11_0_arm64.whl
qspquest-doctor --json
python -c "import qspquest; print(qspquest.__version__)"
```

On Windows PowerShell, activate with `.venv\Scripts\Activate.ps1`; the macOS
wheel cannot be installed on Windows or Linux. Use the wheel matching your
platform from the [wheel table](#python-wheels), or the source route below. Expected version: `0.1.0`.
The doctor runs a small analytic model and reports its checks. To confirm the
wheel was built from the matching source, save the report and run the checker
from the extracted complete QSPQuest source archive:

```sh
qspquest-doctor --json > doctor.json
python3 scripts/check_source_identity.py --doctor doctor.json --target aarch64-apple-darwin
```

Use the actual artifact target (`aarch64-unknown-linux-gnu`,
`x86_64-unknown-linux-gnu`, `x86_64-apple-darwin` or `x86_64-pc-windows-msvc`
for those wheels).
Expect `status: verified`; changed files, a wrong target or a mismatched source
hash fail.


Run the wheel's self-contained example with `python -m qspquest.first_run --out
developer-results`; no complete source archive is needed for that workflow.


See [release verification](release-verification.md) for downloaded artifact identity,
signatures and distribution checks. Downloaded first-run examples are described
in [packaged journeys](tutorials/first-run-journeys.md).

## Browser, Docker and Rust

- [WebAssembly](browser.md): build the pinned WASM assets and run the complete browser example.
- [Docker](site/containers.md): build the CLI/Python or Jupyter image locally, or
  run the published MCP image.
- [Rust API](modeling.md): use path dependencies on `crates/qspquest-modeling` and
  `crates/qspquest-core` from this checkout.

## Platform status

| Platform | Checked route |
| --- | --- |
| macOS ARM64 | Source-built CLI, Python wheel, R binding and tutorials |
| Linux x86_64 and ARM64 with glibc 2.28+ | Released wheels installed on CPython 3.10 and 3.13; MCP image |
| Windows x86_64 | Released wheel installed on CPython 3.10 and 3.13 |
| macOS Intel | Released wheel installed on CPython 3.10 and 3.13 |
| musl and PyPy | No binary distribution; use the source archive |

## Troubleshooting

- pip starts compiling Rust: no wheel matched your platform or Python; check the
  [wheel table](#python-wheels), or install Rust 1.98.1 and native build tools.
- Missing Cargo or compiler: install Rust 1.98.1 and your platform's native build tools.
- Missing NumPy/SciPy: install the `analysis` extra in the active environment.
- Wrong notebook kernel: check `sys.executable`; install into that environment
  and restart the kernel after replacing the native extension.
- Offline build fails: fetch locked dependencies online first, then add `--offline`.
- Doctor fails: include its JSON report when [reporting a bug](../CONTRIBUTING.md).

## Build the guides

```sh
python scripts/build_user_guide_formats.py
python scripts/build_docs.py
```

Open `docs/_site/index.html` to read and search the documentation offline.

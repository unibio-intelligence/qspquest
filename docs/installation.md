# Install QSPQuest

For a first simulation, follow the [CLI quickstart](../README.md#try-the-cli).
Use the source routes below while downloadable packages and registry images
are unavailable. QSPQuest 0.1.0 is not yet published to PyPI, CRAN or R-universe.

| Interface | Prerequisites | Start here |
| --- | --- | --- |
| CLI | Git, Rust 1.98.1/Cargo and a native compiler | [CLI](#cli) |
| Python | CPython 3.10+ and the CLI build tools | [Python](#python) |
| R | R, its compatible C compiler and Rust 1.98.1/Cargo | [R](#r) |
| Docker | Docker Engine or Desktop with Compose | [Docker setup](site/containers.md) |

Commands below run from the complete source checkout. If needed, obtain it with:

```sh
git clone https://github.com/unibio-intelligence/qspquest.git
cd qspquest
```

Source builds download locked dependencies on first use and can take several
minutes. Offline builds require those dependencies to be cached already.

## CLI

The default `cargo build --release --locked` builds shared types, the engine
and CLI. Python, R, MCP and WASM remain workspace members; select their package
or documented installation command when needed. `cargo test --workspace
--locked` still covers the complete workspace and its language prerequisites.
See [building only the workflow you need](build-cost.md) for feature selection.

From the checkout root:

```sh
cargo build --release --locked -p qspquest-cli
target/release/qspquest run conformance/models/synthetic-one-compartment.qsp \
  docs/examples/first-run.json > simulation.json
```

The concentration is 2.000000 mg/L at 0 h and 0.181436 mg/L at 24 h.
The [README](../README.md#try-the-cli) explains the example.

## Python

The [Python package guide](../bindings/python/README.md) provides the API's
self-contained synthetic example and also ships with the Python source archive.

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
`pip` installs the pinned build backend and builds the native extension. The
first run checks the analytic solution and writes six replay and result files
in `developer-results/`. Keep that directory together and choose a new name
for another run. No separate model library or analysis packages are needed.

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

From the complete source checkout, install notebook and interchange support:

```sh
python -m pip install '.[notebook,interchange]'
qspquest-doctor --analysis --interchange --json
python -m jupyterlab docs/tutorials/oral-pk-analysis.ipynb
```

The [oral PK tutorial](tutorials/oral-pk-analysis.md) includes synthetic data and
a complete fitting workflow. The [Python guide](PYTHON-USER-GUIDE.md) covers the
public API; the [synthetic notebook](tutorials/synthetic.ipynb) needs no external data.
For downloaded packages, attach the same extras to the local filename, such as
`'./qspquest-0.1.0.tar.gz[notebook,interchange]'`.

## R

Install R, a compatible C compiler and Rust 1.98.1/Cargo first. On
macOS, use a compiler and SDK compatible with your R distribution. Windows
source installation requires compatible R build tools and is not yet verified.

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
mount the shared model library once, and run named scenarios. The MCP image uses
the same library. No host Rust compiler or language package installation is
needed to run these images. Images are built locally until registry publication.

## Downloaded packages

Python users install a wheel or Python source archive with `pip`. R users install
the separate R source archive with `R CMD INSTALL` or `install.packages`.
These commands use files downloaded to your machine. QSPQuest 0.1.0 is not yet
published to PyPI, CRAN or R-universe; `pip install qspquest` and
`install.packages("qspquest")` are not the installation routes for QSPQuest 0.1.0.

| Installation file | Use | Build tools |
| --- | --- | --- |
| `qspquest-0.1.0-cp310-abi3-macosx_11_0_arm64.whl` | CPython 3.10+ on macOS Apple Silicon | None |
| `qspquest-0.1.0.tar.gz` | Python source installation | Rust 1.98.1, Cargo and a native compiler |
| `qspquest_0.1.0.tar.gz` | R source installation | R, its compatible C compiler, Rust 1.98.1 and Cargo |

The hyphenated `.tar.gz` is the Python package; the underscored archive is the R
package. Neither is the complete project archive `archives/qspquest.tar.gz`.
Source builds download locked Cargo dependencies on first installation and can
take several minutes. They require network access unless those dependencies are
already cached. A matching Python wheel needs neither Rust nor Cargo.

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
wheel cannot be installed on Windows or Linux. Use a matching platform wheel
when one is available, or the source route below. Expected version: `0.1.0`.
The doctor compiles and simulates an analytic model and reports its checks. From
the extracted, matching complete QSPQuest project source archive, retain the report and verify the compiler
source and target using the included stdlib checker:

```sh
qspquest-doctor --json > doctor.json
python3 scripts/check_source_identity.py --doctor doctor.json --target aarch64-apple-darwin
```

Use the actual artifact target (`aarch64-unknown-linux-gnu`,
`x86_64-unknown-linux-gnu` or `x86_64-pc-windows-msvc` for those wheels).
Expect `status: verified`. The checker binds source bytes and the vendored backend
source to the installed artifact; changed files, a wrong target or mismatched
source hash fail. This integrity check does not extend scientific qualification.


Run the wheel's self-contained example with `python -m qspquest.first_run --out
developer-results`; no complete source archive is needed for that workflow.


See [release verification](release-verification.md) for downloaded artifact identity,
signatures and distribution checks. Downloaded first-run examples are described
in [packaged journeys](tutorials/first-run-journeys.md).

## Browser, Docker and Rust

- [WebAssembly](browser.md): build the pinned WASM assets and run the complete browser example.
- [Docker](site/containers.md): build the CLI/Python or Jupyter image locally.
- [Rust API](modeling.md): use path dependencies on `crates/qspquest-modeling` and
  `crates/qspquest-core` from this checkout.

## Platform status

| Platform | Checked route |
| --- | --- |
| macOS ARM64 | Source-built CLI, Python wheel, R binding and tutorials |
| Linux ARM64 with glibc | Docker CLI, Python, R, notebook and MCP checks |
| Linux x86_64 and Windows x86_64 | CI build/test targets; passing release CI is pending |
| macOS Intel, musl and PyPy | No tested binary distribution |

## Troubleshooting

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

# QSPQuest

QSPQuest simulates pharmacology models and fits model parameters. Run it locally
from the command line, Python or R, or connect through WebAssembly or MCP.
Models declare their equations, units, parameters and dosing events.
To use QSPQuest without installing anything, try the hosted service at
[biologics.unibiointelligence.com](https://biologics.unibiointelligence.com/).

Start with the synthetic model below. To explore a published model, use the
[model catalog](https://github.com/unibio-intelligence/qspquest-models).

## Install

QSPQuest ships prebuilt wheels for CPython 3.10 and newer on Linux (x86_64 and
ARM64, glibc 2.28+), macOS (Apple Silicon and Intel) and Windows (x86_64).
No Rust toolchain or compiler is needed:

```sh
python3 -m pip install qspquest
qspquest-doctor
python3 -m qspquest.first_run --out first-run
```

The first run simulates a 6 mg bolus into a 3 L compartment, with clearance
0.3 L/h, checks it against the analytic solution and saves the result in
`first-run/`. The `cp` output holds the concentration (mg/L) at each sample time:

| Time (h) | Concentration (mg/L) |
| ---: | ---: |
| 0 | 2.000000 |
| 2 | 1.637462 |
| 8 | 0.898658 |
| 24 | 0.181436 |

The concentration follows `cp(t) = 2 * exp(-0.1 * t)`. These synthetic parameters
illustrate a one-compartment model; they are not a clinical dosing recommendation.
Add `[analysis]`, `[notebook]` or `[interchange]` for optional workflows; see
[installation](docs/installation.md) for extras, R and other platforms.

## Start with your agent

Connect your own agent client to the QSPQuest MCP server. The published image
includes a synthetic model, so you can inspect it and run a dose simulation
without installing Rust or downloading a model library:

```sh
docker run --rm --init --publish 127.0.0.1:8080:8080 --read-only \
  --cap-drop ALL --security-opt no-new-privileges \
  ghcr.io/unibio-intelligence/qspquest-mcp:latest
```

Then add a Streamable HTTP MCP server at `http://127.0.0.1:8080/mcp` in your
client. The [agent and Docker quickstart](docs/tutorials/agents-docker.md)
covers stdio setup, a copyable agent prompt and expected results.

## Try it in your browser

Adjust dose, clearance and volume for the synthetic model, plot the concentration
and download a CSV, or load your own model. Everything runs on your device. See the
[browser quickstart](docs/browser.md).

## Build the CLI from source

The command-line tool is built from source. You need Git, Rust 1.98.1/Cargo and a
native compiler. On macOS or Linux:

```sh
git clone https://github.com/unibio-intelligence/qspquest.git
cd qspquest
cargo build --release --locked -p qspquest-cli
target/release/qspquest run \
  conformance/models/synthetic-one-compartment.qsp \
  docs/examples/first-run.json > simulation.json
```

The first build downloads locked dependencies and can take several minutes.
`simulation.json` contains the same `cp` values as the table above. For
source builds of the other interfaces, see [installation](docs/installation.md).

## Choose your next step

| Task | Guide |
| --- | --- |
| Run a model from the library | [Choose a model](https://github.com/unibio-intelligence/qspquest-models) · [Engine setup](docs/site/shared-models.md) |
| Work in Python or R | [Python](docs/installation.md#python) · [R](docs/R-USER-GUIDE.md#quickstart) |
| Write equations or fit observations | [Modeling](docs/modeling.md) · [Synthetic fitting](docs/tutorials/oral-pk-analysis.md) · [Published-data fit](docs/tutorials/published-theophylline.md) |
| Use Docker, a browser or MCP | [Docker](docs/site/containers.md) · [WebAssembly](docs/browser.md) · [MCP](bindings/mcp/README.md) |

The [guide index](docs/site/index.md) includes paired-endpoint fitting,
SBML/OMEX interchange, analysis notebooks and API references.

## Supported scope

Native models support dosing events, scans, sensitivities and bounded fitting.
The SBML importer supports rate-rule ODEs and a bounded OMEX experiment profile;
general reaction-network and SED-ML execution are outside this release.

[Independent validation](https://github.com/unibio-intelligence/qspquest-validation)
compares QSPQuest with reference engines. BDF is the supported solver for
sensitivities and fitting. The
[capability matrix](docs/site/capabilities.md) describes other methods and interfaces.
Matching those references doesn't make a model clinically suitable or mean every
model can be fitted, so read each model's scope and units before using its results.

## Contribute and get help

Contributions are welcome. Open an issue first, then follow the
[contributing guide](CONTRIBUTING.md). Report vulnerabilities privately using the
[security policy](SECURITY.md).
Ubi-owned engine code and synthetic examples use [Apache-2.0](LICENSE).
Dependency licenses are included in [third-party notices](THIRD-PARTY-NOTICES.txt).
Developed by [UniBio Intelligence](https://unibiointelligence.com).

## Acknowledgements

We gratefully acknowledge support from Anthropic's AI for Science rare disease
research grants.

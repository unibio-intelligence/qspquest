# QSPQuest

QSPQuest simulates pharmacology models and fits model parameters. Run it locally
from the command line, Python or R, or connect through WebAssembly or MCP.
Models declare their equations, units, parameters and dosing events.

Start with the synthetic model below. To explore a published model, use the
[model catalog](https://github.com/unibio-intelligence/qspquest-models).

## Start with your agent

Connect your own agent client to the QSPQuest MCP server. The Docker image
includes a synthetic model, so you can inspect it and run a dose simulation
without installing Rust or downloading a model library. Follow the complete
[agent and Docker quickstart](docs/tutorials/agents-docker.md) for HTTP or stdio
setup, a copyable agent prompt and expected results.

## Try it in your browser

Adjust dose, clearance and volume for the synthetic model, plot the concentration
and download a CSV, or load your own model. Everything runs on your device. See the
[browser quickstart](docs/browser.md).

## Try the CLI

You need Git, Rust 1.98.1/Cargo and a native compiler. On macOS or Linux:

```sh
git clone https://github.com/unibio-intelligence/qspquest.git
cd qspquest
cargo build --release --locked -p qspquest-cli
target/release/qspquest run \
  conformance/models/synthetic-one-compartment.qsp \
  docs/examples/first-run.json > simulation.json
```

The first build downloads locked dependencies and can take several minutes.
For other interfaces and platform prerequisites, see [installation](docs/installation.md).

This example gives a 6 mg bolus into a 3 L compartment, with clearance
0.3 L/h. In `simulation.json`, the `cp` output holds the concentration (mg/L)
at each sample time:

| Time (h) | Concentration (mg/L) |
| ---: | ---: |
| 0 | 2.000000 |
| 2 | 1.637462 |
| 8 | 0.898658 |
| 24 | 0.181436 |

The concentration follows `cp(t) = 2 * exp(-0.1 * t)`. These synthetic parameters
illustrate a one-compartment model; they are not a clinical dosing recommendation.

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

See [contributing](CONTRIBUTING.md) for tests, builds and bug reports, and the
[security policy](SECURITY.md) for private vulnerability reports.
Ubi-owned engine code and synthetic examples use [Apache-2.0](LICENSE).
Dependency licenses are included in [third-party notices](THIRD-PARTY-NOTICES.txt).
Developed by [UniBio Intelligence](https://unibiointelligence.com).

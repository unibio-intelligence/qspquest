# QSPQuest guides

New to QSPQuest? [Run the first simulation](../../README.md#try-the-cli), then
choose one workflow below. [Installation](../installation.md) covers prerequisites
and alternative interfaces.

| Task | Start here |
| --- | --- |
| Connect your agent | [MCP and Docker quickstart](../tutorials/agents-docker.md) |
| Run a published pharmacology model | [Model-library setup](shared-models.md) · [Catalog](https://github.com/unibio-intelligence/qspquest-models) |
| Use Python or R | [Python guide](../PYTHON-USER-GUIDE.md) · [R guide](../R-USER-GUIDE.md) |
| Write a model and fit observations | [Model equations](../modeling.md) · [Oral PK analysis](../tutorials/oral-pk-analysis.md) |
| Explore a QSP mechanism | [Target-gated cargo tutorial](https://github.com/unibio-intelligence/qspquest-models/blob/main/tutorials/qsp-gated-cargo.md) |
| Use Docker, a browser or MCP | [Docker](containers.md) · [WebAssembly](../browser.md) · [MCP](../../bindings/mcp/README.md) |

## More workflows

- [Paired concentration and cell measurements](../tutorials/paired-endpoint-fitting.md)
- [Archived synthetic library fitting](../tutorials/library-fitting.md)
- [SBML/OMEX interchange](interchange.md)
- [Software adapters](../SOFTWARE-ADAPTERS.md)
- [Packaged first runs](../tutorials/first-run-journeys.md), when matching artifacts are available

## Reference

[Supported capabilities](capabilities.md) · [Fit controls](controls.md) ·
[Fitting methods](fitting-solvers.md) · [Interfaces](interfaces.md) ·
[Python API](../api/python.md)

The Python guide is also available as a [notebook](../PYTHON-USER-GUIDE.ipynb),
and the R guide as [R Markdown](../R-USER-GUIDE.Rmd).
[Independent validation](https://github.com/unibio-intelligence/qspquest-validation)
records the tested engine, inputs, numerical limits and results. Each model card
states its own execution and interpretation limits.

# QSPQuest guides

To begin, [run the first simulation](../../README.md#install), then choose one
workflow below. [Installation](../installation.md) covers prerequisites
and alternative interfaces.

| Task | Start here |
| --- | --- |
| Connect your agent | [MCP and Docker quickstart](../tutorials/agents-docker.md) |
| Run a published pharmacology model | [Model-library setup](shared-models.md) · [Catalog](https://github.com/unibio-intelligence/qspquest-models) |
| Use Python or R | [Python guide](../PYTHON-USER-GUIDE.md) · [R guide](../R-USER-GUIDE.md) |
| Learn PK/PD analysis step by step | [PK/PD tutorials](../tutorials/pk-pd-journeys.md): single dose, multiple doses, fitting, population analysis, model evaluation |
| Write a model and fit observations | [Model equations](../modeling.md) · [Oral PK analysis](../tutorials/oral-pk-analysis.md) |
| Explore a QSP mechanism | [Target-gated cargo tutorial](https://github.com/unibio-intelligence/qspquest-models/blob/main/tutorials/qsp-gated-cargo.md) |
| Use Docker, a browser or MCP | [Docker](containers.md) · [WebAssembly](../browser.md) · [MCP](../../bindings/mcp/README.md) |

## More workflows

- [Publication figures from pharmacometric results](../tutorials/publication-figures.md)
- [Paired concentration and cell measurements](../tutorials/paired-endpoint-fitting.md)
- [Archived synthetic library fitting](../tutorials/library-fitting.md)
- [Extend a library model and select a regimen](../tutorials/extend-library-model.md)
- [SBML/OMEX interchange](interchange.md)
- [Software adapters](../SOFTWARE-ADAPTERS.md)
- [First runs](../tutorials/first-run-journeys.md) with the wheel, CLI or MCP image

## Reference

[Supported capabilities](capabilities.md) · [Fit controls](controls.md) ·
[Fitting methods](fitting-solvers.md) · [Interfaces](interfaces.md) ·
[Python API](../api/python.md)

The Python guide is also available as a [notebook](../PYTHON-USER-GUIDE.ipynb),
and the R guide as [R Markdown](../R-USER-GUIDE.Rmd).
[Independent validation](https://github.com/unibio-intelligence/qspquest-validation)
records the tested engine, inputs, numerical limits and results. Each model card
states its own execution and interpretation limits.

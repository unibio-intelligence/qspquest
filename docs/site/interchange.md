# Standardized model interchange

[Paired-endpoint fitting](../tutorials/paired-endpoint-fitting.md) adds a
QSPQuest OMEX profile for individual Gaussian fit requests that keeps endpoint
errors, dose history and exact observation rows.

[Software adapters](../SOFTWARE-ADAPTERS.md) support a defined
subset of NONMEM, nlmixr2, rxode2, Pumas, Monolix and Phoenix/WinNonlin,
with explicit event-table mappings and an optional rxode2 converter bridge.

The Ubi synthetic absorption example exports a COMBINE/OMEX archive containing SBML equations and SED-ML simulation requests. Import verifies the archive digest and preserves units, events, and sampling semantics.

```sh
python docs/examples/interchange_roundtrip.py
```

The script checks the simulated concentration against the analytic first-order solution, then verifies a deterministic archive round trip. The parameters are illustrative, not drug-specific.

[Read the executable tutorial](../examples/interchange_roundtrip.py) and [the model](../../examples/synthetic/absorption.qsp). External engine benchmarks and source-derived examples live in [QSPQuest validation](https://github.com/unibio-intelligence/qspquest-validation).

# Standardized model interchange

[Paired-endpoint fitting](../tutorials/paired-endpoint-fitting.md) adds a bounded
QSPQuest OMEX statistical profile for native individual Gaussian requests,
preserving endpoint errors, dose history and exact observation rows.

[Software adapters](../SOFTWARE-ADAPTERS.md) provide a separate deterministic
source subset for NONMEM, nlmixr2, rxode2, Pumas, Monolix and Phoenix/WinNonlin,
with explicit event-table mappings and an optional rxode2 converter bridge.

The Ubi synthetic absorption example exports a COMBINE/OMEX archive containing SBML equations and SED-ML simulation requests. Import verifies the archive digest and preserves units, events, and sampling semantics.

```sh
python docs/examples/interchange_roundtrip.py
```

The script checks the simulated concentration against the analytic first-order solution, then verifies a deterministic archive round trip. The illustrative parameters carry no drug-specific validation claim.

[Read the executable tutorial](../examples/interchange_roundtrip.py) and [the model](../../examples/synthetic/absorption.qsp). External engine benchmarks and source-derived examples live in [QSPQuest validation](https://github.com/unibio-intelligence/qspquest-validation).

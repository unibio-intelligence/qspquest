# Build inputs and inspect results

## Dosing and run helpers

```python
from qspquest import CompiledModel, bolus, infusion, run_request
from pathlib import Path
model = CompiledModel(Path('examples/synthetic/absorption.qsp').read_text())
run = run_request(end=24, times=[0, .5, 1, 2, 4, 8, 12, 24],
    administrations=[bolus('depot', 300, 'mg', time_unit='h')],
    time_unit='h', solver='analytic_linear_pk')
result = model.run(run)
print(result.summary())
frame = result.to_frame()
ax = result.plot('cp')
```

`infusion('depot', 300, 'mg', duration=1, time_unit='h')` declares a fixed-duration
infusion. The model must include the corresponding input forcing. Helpers build
native requests with explicit units, which the engine then validates.
`run_request` requires increasing sample times. Supply a native request directly
for duplicate pre/post samples, repeats, resets or step covariates.

## Result objects

`CompiledModel.run` and its analysis methods return `AnalysisResult`, a
subclass of `dict`. Dictionary indexing and JSON serialization are unchanged.
`rows()` retains event sides, units and source-row identity. `to_frame()` requires
pandas; `plot()` requires matplotlib. `summary()` explains fit status, bounds and
method limits. Wrap a plain JSON-loaded dict with `AnalysisResult(saved)` to use
these helpers after loading results.

## Prepared datasets

`population_fit_dataset` accepts explicit NONMEM-style records and
preserves their dose/observation mapping. Use `dataset.to_subjects(covariates=...)`
to prepare subjects for native-simulation fitting, or:

```python
fit = dataset.fit(model, parameters, omega, error,
    covariates_by_subject={'subject-id': {'weight_fraction': 1.0}},
    max_iterations=600, burn_in=150)
```

The `error` argument sets the residual error model and overrides any fixed
errors stored on adapter endpoints. These fits currently use one observed output
and one shared error model. Priors are unsupported.
Subject covariates are constant. Unknown subject IDs raise an error and are not silently dropped.
See the [oral pharmacokinetic (PK) notebook](../tutorials/oral-pk-analysis.md) for complete subject requests, observation rows and source-row identity.

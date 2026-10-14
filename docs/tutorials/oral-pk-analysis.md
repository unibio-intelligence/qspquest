# Oral PK: from observations to a report

Run a complete fitting workflow with **12 synthetic subjects and 96 observations**.
A seeded independent analytic generator supplies oral doses, varying clearance,
volume and absorption, and additive/proportional observation noise. No dataset
or download is needed. The equations and generated data are Apache-2.0.

Fit each subject independently, inspect predictions and residuals, and save a
standalone HTML report and a CSV of estimates. These individual fits demonstrate
the workflow; they do not estimate a population distribution or clinical effects.

## Start the notebook

Follow [Python installation](../installation.md#python) with the analysis and
notebook extras, then run from the checkout root:

```sh
python -m jupyterlab docs/tutorials/oral-pk-analysis.ipynb
```

Run the cells in order. Outputs go into `oral-pk-output` beneath the kernel's
starting directory. The model uses `dA_depot/dt = -ka*A_depot` and
`dA_central/dt = ka*A_depot - cl/vc*A_central`, with `cp = A_central/vc`.

## Generate and inspect observations

```python
from pathlib import Path
import json
import sys

import qspquest as pf

root = next(p for p in [Path.cwd(), *Path.cwd().parents]
            if (p / 'docs/examples/oral_pk_analysis.py').is_file())
sys.path.insert(0, str(root / 'docs/examples'))
from oral_pk_analysis import synthetic_subjects, parameter_specs

output = Path.cwd() / 'oral-pk-output'
output.mkdir(exist_ok=True)
model, subjects, truth = synthetic_subjects(
    (root / 'examples/synthetic/absorption.qsp').read_text())
assert len(subjects) == 12
assert sum(len(s['observations']) for s in subjects) == 96
print('12 synthetic subjects, 96 observations; seed 2026')
```

## Fit one subject

Fit clearance, volume and absorption rate with bounded likelihood optimization.
The example uses fixed additive and proportional residual standard deviations.

```python
parameters = parameter_specs()
error = {'model': 'normal', 'additive': 0.2, 'proportional': 0.1}
individual = model.fit_data(subjects[0]['run'], subjects[0]['observations'],
                            parameters, error, method='Powell',
                            max_iterations=400, max_simulations=5000)
print(individual['status'], individual['parameters'])
```

## Inspect predictions and residuals

```python
diagnostics = pf.fit_diagnostics(individual)
print(diagnostics['residual_summary'], diagnostics['warnings'])
import matplotlib.pyplot as plt
rows = individual['fitted_observations']
plt.scatter([r['time'] for r in rows], [r['observed'] for r in rows], label='Observed')
plt.plot([r['time'] for r in rows], [r['prediction'] for r in rows], label='Fitted')
plt.xlabel('Time (h)')
plt.ylabel('Concentration (mg/L)')
plt.legend()
plt.show()
```

## Fit the other subjects

Reuse the same model, parameter bounds and error model for each subject.
The seeded example returns `converged` for all 12 fits. Retain each actual status
in the exported table when using a different dataset or fitting contract.

```python
fits = [individual]
for subject in subjects[1:]:
    fits.append(model.fit_data(subject['run'], subject['observations'],
                                parameters, error, method='Powell',
                                max_iterations=400, max_simulations=5000))
print({status: sum(f['status'] == status for f in fits)
       for status in sorted({f['status'] for f in fits})})
```

## Save and inspect the report

```python
import csv

for name, value in [('synthetic-truth', truth), ('subjects', subjects), ('fits', fits)]:
    (output / (name + '.json')).write_text(json.dumps(value, indent=2, allow_nan=False))
with (output / 'estimates.csv').open('w', newline='') as stream:
    writer = csv.DictWriter(stream, fieldnames=['subject', 'status', 'cl_L_h', 'vc_L', 'ka_per_h'])
    writer.writeheader()
    for subject, fit in zip(subjects, fits):
        writer.writerow(dict(subject=subject['id'], status=fit['status'],
                             cl_L_h=fit['parameters']['cl']['value'],
                             vc_L=fit['parameters']['vc']['value'],
                             ka_per_h=fit['parameters']['ka']['value']))
pf.write_analysis_report(output / 'report.html', title='Synthetic oral PK: subject 1',
                         fit=individual)
print(output / 'report.html')
```

Open `report.html` locally to inspect the first subject's estimates, predictions,
residuals and fit status. `estimates.csv` contains all 12 subjects; the JSON files
retain the input observations, known generating parameters and engine identities.
The [Python API](../api/python.md) describes population fitting and other analyses.

# Fit published theophylline observations

Fit a one-compartment oral model to subject 1 of the `theo_sd` dataset, then
compare predictions with the observed concentration curve. This uses published
observations from the validation companion, rather than simulated recovery data.
The companion includes the dataset attribution and license notices.

You need Python 3.11 or newer, Git, a built QSPQuest CLI and the matching
[validation repository](https://github.com/unibio-intelligence/qspquest-validation).
No R installation or extra Python packages are needed. From the complete engine
checkout, first follow [source installation](../installation.md), then select the
validation revision this example reads:

```sh
git clone https://github.com/unibio-intelligence/qspquest-validation.git ../qspquest-validation
git -C ../qspquest-validation checkout 7b98a5613b39c6cfd535349b26e7740d51bf41e5
python3 -B ../qspquest-validation/scripts/verify.py
python3 -B docs/examples/published_theophylline.py \
  --validation ../qspquest-validation \
  --cli ./target/release/qspquest \
  --out ../theophylline-fit
```

If you already have the validation checkout, skip the clone but still check out
the pinned revision. Choose a new output
directory. The script checks the data files' checksums before fitting.

## Data and model

The archived `nlmixr2data` 2.0.10 `theo_sd` data contain the first-day observations
from the theophylline dataset used in nlme/NONMEM examples. This workflow selects
one subject, a dose at time zero and ten post-dose observations from 0.25 to
24.37 hours. The archived fitting contract excludes the predose observation.
It does not reproduce the original study's population analysis.

The model has a depot and a central compartment, first-order absorption `ka`,
elimination `cl / vc`, and output `cp = central / vc`. The complete `.qsp` model,
observations, bounds, initial estimates and solver settings are copied into your
output directory. The fit uses BDF and a Gaussian residual model with fixed
additive standard deviation 2.1 in source numerical coordinates, no proportional
error and no prior. It estimates `cl`, `vc` and `ka`.

The model stores amount, volume and concentration with no physical unit attached
(unit `1`, on the source's numeric scale) and time in hours, so `cl` is reported in `1/h`, `vc` in `1` and `ka` in
`1/h`. Do not report these as clearance or volume in physical units; that needs
a model with real units.

## Read the result

The command prints `status: converged`, `observations: 10` and estimates close to:

| Parameter | Estimate | Declared unit |
| --- | ---: | --- |
| `cl` | 1.58591 | `1/h` |
| `vc` | 29.39343 | `1` |
| `ka` | 1.77741 | `1/h` |

Small differences across builds are expected. The negative log-likelihood is about
17.03261. `fit.json` has convergence, bounds, the projected-gradient norm and
evaluation counts. `receipt.json` records the validation revision and inputs used.

Open `observed-predicted.csv` to inspect the ten time points, fitted concentrations
and residuals (`observed - predicted`). At 1.12 hours the observed value is 10.5 and
the prediction is about 9.0353: a converged fit does not pass through every point.
Check the residual pattern and the fixed-error assumption before interpreting the
model. This single-subject fit does not estimate population variability or prove
clinical suitability.

## Troubleshooting

- A manifest mismatch means the companion checkout differs from the engine pin.
  Select the revision shown above; do not bypass checksum verification.
- If fitting does not converge, retain `fit.json` and inspect its status and bounds.
  Increasing budgets does not establish that the model or error assumptions fit
  the observations.
- If the output directory exists, choose another name. The script preserves
  earlier results rather than overwriting them.

For constructing your own observation model, continue with the
[oral PK analysis tutorial](oral-pk-analysis.md). For the full stored comparison
matrix and data notices, see the validation companion's `METHODS.md` and
`THIRD_PARTY_NOTICES.md`.

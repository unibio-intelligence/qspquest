# Choose a solver for sensitivity-based fitting

QSPQuest uses `diffsol_bdf` for sensitivities and sensitivity-based fitting.
BDF is the default in request helpers. Select it explicitly when constructing
requests directly.

For an individual native fit, select the solver in its sensitivity request:

```python
fit_request["problem"]["request"]["objective"]["simulation"]["run"]["solver"] = "diffsol_bdf"
result = compiled_sensitivities.fit(fit_request)
```

For a native population fit, select it for each subject:

```python
for subject in fit_request["problem"]["request"]["subjects"]:
    subject["simulation"]["run"]["solver"] = "diffsol_bdf"
result = compiled_sensitivities.fit(fit_request)
```

The selection applies to states, forward sensitivities, and subsequent objective
evaluations. Other simulation solvers are not supported for sensitivity-based
fitting. Moving-event derivative limitations still apply. A population estimator's
seed controls that estimator; a run-level seed remains unsupported.

## Accuracy and interpretation

Set both integration tolerances and state/parameter derivative absolute
tolerances in their declared units. Tighten them and check that gradients,
estimates, and fit status stabilize. An accurate-looking trajectory alone
does not verify an objective gradient or convergence.

For simulation, `accuracy_control: "sampled"` performs bounded refinement with
the selected solver and rejects a trajectory that fails its convergence check.
This checks the requested samples; it does not certify global error or agreement
with an independent solver. Check declared invariants and stability under tighter
tolerances for the specific model and observations.

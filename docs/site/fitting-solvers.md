# Choose a solver for sensitivity-based fitting

The native implementation accepts `diffsol_bdf` and
`diffsol_rodas5p` for sensitivities and sensitivity-based fitting. BDF remains
the default in request helpers. Rodas5P is an explicit option; it is not an
automatic fallback. Select the method explicitly in the fitting request.

For an individual native fit, select the solver in its sensitivity request:

```python
fit_request["problem"]["request"]["objective"]["simulation"]["run"]["solver"] = "diffsol_rodas5p"
result = compiled_sensitivities.fit(fit_request)
```

For a native population fit, select it for each subject:

```python
for subject in fit_request["problem"]["request"]["subjects"]:
    subject["simulation"]["run"]["solver"] = "diffsol_rodas5p"
result = compiled_sensitivities.fit(fit_request)
```

The selection applies to states, forward sensitivities, and subsequent objective
evaluations. Individual and pooled objective identity names record BDF,
Rodas5P, or explicitly mixed subject solvers. Population request hashes bind
all subject solver selections. Other simulation solvers remain unsupported
for sensitivity-based fitting. Moving-event derivative limitations are
unchanged. A population estimator's seed controls that estimator; a run-level
seed remains unsupported.

## Accuracy and interpretation

Set both integration tolerances and state/parameter derivative absolute
tolerances in their declared units. Tighten them and check that gradients,
estimates, and fit status stabilize. An accurate-looking trajectory alone
does not verify an objective gradient or convergence.

BDF and Rodas5P use different integration algorithms and can have different
performance and positivity behavior. Check convergence and declared invariants
for the specific model and observations; a failed invariant supplies no fitted
result. Choose the solver explicitly and check stability under tighter tolerances.

## Rosenbrock23 simulation accuracy

Rosenbrock23 is available for simulation, with `accuracy_control: "sampled"`
required when qualifying sampled stiff trajectories. Its default local error
control can exceed the declared trajectory tolerance on Robertson kinetics.
Sampled control performs bounded refinement with the same solver and rejects a
trajectory that fails its convergence check. This is a numerical safeguard,
not a proof of agreement with an independent solver. Rosenbrock23 remains
unavailable for sensitivity-based fitting and MCP simulation.

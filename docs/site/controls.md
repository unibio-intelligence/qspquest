# Status, progress, cancellation and restart

## Interpret status before estimates

`scientific_status(result)` and `error.explain()` return a plain title, message,
interpretation boundary and warnings. `fit.summary()` prints them alongside
estimates. Convergence means the configured numerical checks passed, not that a
model is biologically valid or a global optimum has been found.

| Status | Interpretation |
| --- | --- |
| converged | Numerical checks passed within the stated method/design scope |
| iteration_limit | Estimates and plots are exploratory; convergence did not pass |
| evaluation_limit | Work budget ended; final prediction rows may be unavailable |
| mixing_failed | Population sampling did not pass the mixing check |
| insufficient_successful_replicates | No bootstrap interval; inspect replicate failures |
| unsupported | Requested scientific contract is outside this method's scope |
| cancelled | No complete result; resume the last verified checkpoint |

Boundary estimates remain visible even when a fitter reports convergence. The
SAEM objective is a complete-data surrogate, not a marginal likelihood.

## Control a long analysis

```python
from qspquest import AnalysisControl
control = AnalysisControl(
    progress=lambda event: print(event['completed'], '/', event['total']),
    checkpoint='population-checkpoint.json', checkpoint_every=10)
fit = model.fit_population(subjects, parameters, omega, error, control=control)
```

Population fits report complete iterations, VPCs report complete replicates, and
bootstraps report complete subject-resampling fits. `control.cancel()` may be
called by the callback or another thread. Cancellation is checked before every
native simulation and after each complete work unit. It cannot interrupt a
single native call; solver/event/output budgets bound that call. A progress
callback exception stops the workflow and propagates to its caller.

Checkpointing is optional. It uses atomic regular JSON files with checksum,
request/model/code/native-build/environment identities and the full RNG state.
No pickle or executable object is loaded. Checkpoints contain user data: keep
them in an appropriate local directory. Use a new control for each operation.

```python
control = AnalysisControl(checkpoint='population-checkpoint.json', resume=True)
fit = model.fit_population(subjects, parameters, omega, error, control=control)
```

Resume with the **same** request, seed, iteration target, simulation budget and
numerical environment. A different input or native build is rejected. Population
and VPC restarts revalidate their initial trajectories; these calls count toward
the aggregate budget. Bootstrap resumes after the last complete replicate, so
an interrupted inner fit is rerun. Work after the last saved boundary is
intentionally discarded. This is restart support, not a way to change the
scientific plan or loosen limits mid-run.

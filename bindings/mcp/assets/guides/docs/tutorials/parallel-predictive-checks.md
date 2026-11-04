# Run repeatable predictive checks with bounded workers

This tutorial simulates a synthetic one-compartment bolus cohort and saves a
visual predictive check (VPC) report. The observations are synthetic, not
patient data. You need the QSPQuest source archive, CPython 3.10+, and its
analysis dependencies. For the source installation below, Rust 1.98.1, Cargo and a native
compiler must be on `PATH`.

The [complete runnable example](qspquest://documentation/files/docs/examples/parallel_vpc.py) reads the shipped
[synthetic model](qspquest://documentation/files/conformance/models/synthetic-one-compartment.qsp).

## Install and run

From the extracted project root, in a new environment:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install '.[analysis]'
qspquest-doctor --analysis --json
OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1 python docs/examples/parallel_vpc.py \
  --workers 1 --output vpc-serial.json
OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1 python docs/examples/parallel_vpc.py \
  --workers 2 --output vpc-parallel.json
```

Set the BLAS thread counts as shown so each worker uses one thread. The
default is one worker; `workers` accepts 1..8. Start with two and measure before
adding more; speedup depends on your workload.

Both runs should print `status: complete`, 16 successful replicates, zero failed
replicates and 102 native simulations: six baselines plus 16 six-subject
replicates. The script will not overwrite an existing output file; delete it or
choose another name before rerunning. Everything runs locally; only `pip install`
downloads anything.

## Inspect and compare results

The synthetic design uses a 6 mg bolus, volume 3 L and typical clearance
0.3 L/h. Individual observed clearances range from 0.22 to 0.42 L/h. Concentration
has units mg/L and follows `2 * exp(-clearance / 3 * time)` before perturbation.
The VPC varies log clearance with variance 0.09 and adds residual SD 0.02 mg/L.
The saved `bins` contain observed and simulated concentration quantiles and
replicate intervals.

The HTML report shows these as lower and upper replicate bands. They assume the
supplied population parameters, variability and residual error are exact; if
those were fitted, the bands do not include their estimation uncertainty.

Check that worker count preserves scientific results and random streams:

```sh
python - <<'PY'
import json
from pathlib import Path
a = json.loads(Path('vpc-serial.json').read_text())
b = json.loads(Path('vpc-parallel.json').read_text())
for key in ('bins', 'replicate_records', 'identity', 'seed', 'simulations'):
    assert a[key] == b[key], key
assert a['successful_replicates'] == b['successful_replicates'] == 16
print('Scientific results, stream receipts and identity match.')
PY
```

Results do not depend on the worker count, because random draws are assigned
to replicates before work is distributed. `execution.workers` records the count
used. To reproduce results exactly, use the same NumPy, Python and QSPQuest
versions; results may differ slightly across platforms.

## Failures, cancellation and restart

A failed replicate is recorded with its error code and message, the overall
status becomes `incomplete`, and confidence intervals are null. Medians from the
successful replicates are for diagnosis only, not a final result. Running out of
budget returns an error and no report.

Use `control=qspquest.AnalysisControl(checkpoint='vpc-checkpoint.json',
checkpoint_every=1)` when calling the Python API to save completed work. A
progress callback can call `control.cancel()`; running workers stop after their
current simulation. Resume with a new control using `resume=True`; the worker
count may change, but inputs, code and software versions must match. Work redone
on resume counts against the original budget, so leave headroom; if the budget
is used up, start a new analysis with a larger one.

VPC observation-draw storage is limited to two million cells.

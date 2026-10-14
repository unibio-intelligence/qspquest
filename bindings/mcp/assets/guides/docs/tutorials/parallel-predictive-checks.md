# Run repeatable predictive checks with bounded workers

This tutorial simulates a synthetic one-compartment bolus cohort and saves a
complete VPC report. The observations come from a stated analytic model and
small deterministic perturbations; they are not patient measurements. You need
the matching complete QSPQuest source archive, CPython 3.10+, and its analysis
dependencies. For the source installation below, Rust 1.98.1, Cargo and a native
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

Set BLAS thread counts before Python starts to avoid multiplying native library
threads by analysis workers. The default is one worker; `workers` accepts 1..8.
Choose two initially and measure your own workload before increasing it.
No speedup or supported-platform scaling factor is implied by worker support.

Both runs should print `status: complete`, 16 successful replicates, zero failed
replicates and 102 native simulations: six baselines plus 16 six-subject
replicates. The script exclusively creates the requested JSON file; remove an
old output deliberately or choose another name before repeating the command.
Everything executes locally. These commands send no model or result to a job
gateway; package installation may download its declared dependencies.

## Inspect and compare results

The synthetic design uses a 6 mg bolus, volume 3 L and typical clearance
0.3 L/h. Individual observed clearances range from 0.22 to 0.42 L/h. Concentration
has units mg/L and follows `2 * exp(-clearance / 3 * time)` before perturbation.
The VPC varies log clearance with variance 0.09 and adds residual SD 0.02 mg/L.
The saved `bins` contain observed and simulated concentration quantiles and
replicate intervals. They describe this predictive experiment, not uncertainty
in fitted parameter estimates or drug-specific qualification.

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

`execution.workers` records the chosen scheduling configuration. The scientific
identity excludes worker count because the owner assigns random draws in
replicate order before dispatch. Each `replicate_records` entry retains its
index, initial RNG-state digest, status and native-call count. Reproducibility
requires the recorded NumPy, Python, engine and analysis implementations; no
cross-platform bitwise promise follows.

## Failures, cancellation and restart

A domain failure remains a failed replicate with its code and message. The
overall status becomes `incomplete`; confidence intervals are null whenever a
replicate fails. Successful-draw medians remain descriptive diagnostics and
must not be presented as a complete scientific result. All requested replicate
indices remain in the denominator. Budget exhaustion returns an error and no
partial scientific report.

Use `control=qspquest.AnalysisControl(checkpoint='vpc-checkpoint.json',
checkpoint_every=1)` when calling the Python API to retain completed work. A
progress callback can call `control.cancel()`. Running workers drain at native
call boundaries, and the checkpoint includes their charged calls even when
their unfinished replicate is discarded. Resume with a new control using
`resume=True`; the worker count may change. Inputs, code, native identity and
numerical environment must still match. Revalidation and replayed unfinished
work count against the original aggregate budget. Leave room for those calls;
an exhausted checkpoint requires a new analysis with a larger budget.

The dispatcher holds at most the selected number of submitted replicates, and
only the owner writes progress/checkpoints. VPC observation-draw storage remains
bounded to two million cells. A failed native call is charged once. This workflow
parallelizes independent predictions and does not alter population-fit updates.

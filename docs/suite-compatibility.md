# Check suite compatibility

This check tests the CLI with an exact model-library snapshot and verifies an
exact validation snapshot. It runs one bispecific-binding preset with BDF and
Rodas5P and compares both fresh outputs to the retained trajectory. Each run
records its actual engine identity, binary checksum, reference checksum,
observation counts and numerical comparison limit.

## Prerequisites

Use Git, Python 3.11 or newer and the Rust toolchain declared by the checkout.
Run these commands from the QSPQuest repository root. No additional Python
packages are required for the compatibility checker or the snapshot verifiers.
Archive checks verify the retained files and recorded fitting evidence; they do
not recompute native fitting identities. The model library documents the separate
`--native-fitting` check, which requires the matching Python binding.
The pinned counterpart commits and checksums are in
`.github/suite-compatibility.json`.
Supply each companion's Git checkout root at that exact commit. Tracked files
and the index must be unchanged, and untracked files must be removed or saved
elsewhere. Ignored caches are allowed. Extracted archives and subdirectories
are not accepted as counterpart checkouts.

```sh
cargo build -p qspquest-cli --release --locked
git clone https://github.com/unibio-intelligence/qspquest-models.git suite-models
git clone https://github.com/unibio-intelligence/qspquest-validation.git suite-validation
python3 - <<'PY'
import json
import subprocess
from pathlib import Path
lock = json.loads(Path('.github/suite-compatibility.json').read_text())
for role in ('models', 'validation'):
    subprocess.run(['git', '-C', 'suite-' + role, 'checkout', '--detach',
                    lock['components'][role]['commit']], check=True)
PY
python3 scripts/check_suite_compatibility.py \
  --models suite-models --validation suite-validation \
  --cli target/release/qspquest --output suite-compatibility.json
```

The checker uses complete model, request and reference inputs already included
in the pinned model archive. It executes in a temporary directory. It never
changes the model archive, validation evidence or CLI. The output path must be
new; choose a different report filename for each run.

Success prints `Suite compatibility passed` and writes a report with
`status: "passed"`. The report has two archive-verification records and two
native solver records. The selected preset has 564 observations and five outputs
per solver. Comparison requires exact observation times and event sides,
complete, uniquely named columns, finite numeric values, matching output and
model time units, and
`abs(actual - reference) <= 1e-8 + 1e-4 * abs(reference)` in source coordinates.
`verified_counterparts` records the actual checked commits, tree IDs and tracked
file counts. The checker verifies every tracked file against its commit before
running the counterpart verifiers and again after execution;
`counterpart_recheck_passed: true` confirms the final check succeeded.

The preset remains dimensionally unqualified. Matching its retained trajectory
does not establish physical dosing units, reproduce a full paper, qualify
fitting or population inference, or rerun an independent reference engine.
The validation verifier checks the archived evidence against its recorded
historical engine identity; it does not extend those results to the current CLI.

## Failures and updates

Wrong counterpart commits, changed or missing tracked files, staged changes,
untracked files, missing tutorial/build inputs, mismatched library versions, changed catalog or
validation-manifest hashes, corrupt archives, shifted sampling/event sides and
out-of-tolerance results cause exit 2. The report retains completed checks and
the failed solver entry when execution or comparison fails. Inspect its `error`
and `checks` fields. Restore the pinned inputs or review a deliberate pin update;
do not change tolerances just to make a different result pass.

The `Suite compatibility` workflow runs this check for relevant pull requests,
changes to `main`, and manual dispatch. MCP numerical qualification and Docker
checks also run for relevant changes, including `.cargo/config.toml`. Updating
counterpart pins requires rerunning this check and the relevant numerical,
installation and scientific qualification checks before release.

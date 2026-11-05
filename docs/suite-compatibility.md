# Check suite compatibility

This check tests the CLI against a pinned model-library snapshot and verifies a
pinned validation snapshot. It runs one bispecific-binding preset with BDF
and compares the outputs to a stored reference trajectory, then replays a few
validation presets against their retained references.

The validation evidence applies when the engine's numerical identity matches the
one it records: the solver sources and locked dependencies, including the Diffsol
revision. Version numbers, manifests, documentation and formatting outside the
solver sources do not change it, so they don't require new evidence. The CLI must
also be built from the checked-out engine.

## Prerequisites

Use Git, Python 3.11 or newer and the Rust toolchain declared by the checkout.
Run these commands from the QSPQuest repository root. No additional Python
packages are required for the compatibility checker or the snapshot verifiers.
Archive checks verify the stored files and fitting evidence without rerunning
fits. The model library documents a separate `--native-fitting` check, which
requires the matching Python binding.
The pinned counterpart commits and checksums are in
`.github/suite-compatibility.json`.
Each companion must be a Git checkout root at that exact commit, with no
modified, staged or untracked files (ignored caches are fine). Extracted
archives and subdirectories are not accepted.

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

The checker uses the inputs in the pinned model archive and runs in a temporary
directory. It never changes the model archive, validation evidence or CLI. The output path must be
new; choose a different report filename for each run.

Success prints `Suite compatibility passed` and writes a report with
`status: "passed"`. The report has two archive-verification records and two
native solver records. The selected preset has 564 observations and five outputs
per solver. Comparison requires exact observation times and event sides,
complete, uniquely named columns, finite numeric values, matching output and
model time units, and
`abs(actual - reference) <= 1e-8 + 1e-4 * abs(reference)` in source coordinates.
`verified_counterparts` records the actual checked commits, tree IDs and tracked
file counts. The checkouts are verified before and after the run;
`counterpart_recheck_passed: true` means the second check passed.

The preset's units have not been verified. Matching the stored trajectory does
not confirm dosing units, reproduce the full paper, validate fitting or
population inference, or rerun an independent reference engine. The validation
check confirms the archived results are intact; it does not rerun them with the
current CLI.

## Failures and updates

Wrong counterpart commits, changed or missing tracked files, staged changes,
untracked files, missing tutorial/build inputs, mismatched library versions, changed catalog or
validation-manifest hashes, corrupt archives, shifted sampling/event sides and
out-of-tolerance results cause exit 2. The report keeps completed checks and
the failed solver entry when execution or comparison fails. Inspect its `error`
and `checks` fields. Restore the pinned inputs or review a deliberate pin update;
do not change tolerances just to make a different result pass.

The `Suite compatibility` workflow runs this check for relevant pull requests,
changes to `main`, and manual dispatch. MCP numerical qualification and Docker
checks also run for relevant changes, including `.cargo/config.toml`. Updating
counterpart pins requires rerunning this check and the relevant numerical,
installation and scientific qualification checks before release.

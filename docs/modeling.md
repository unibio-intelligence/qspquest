# Models and simulation

QSPQuest accepts a text `.qsp` model or a JSON model document. A small
synthetic one-compartment model looks like this:

```text
model synthetic_one_compartment
time_unit = h

parameters
  cl = 0.3 [L/h] log estimated
  v = 3 [L] log estimated

states
  central = 0 [mg]

dynamics
  d/dt(central) = (input_central - ((cl / v) * central))

outputs
  cp = (central / v) [mg/L]

dosing
  central [mg] = 1
end
```

Use `quantity(value, "unit")` for a physical constant in an expression, such as
`quantity(0, "mg/h")` for a zero mass rate or `quantity(500, "mL")` in a volume
expression. Their units carry through sensitivities and fitting. Plain numeric literals are dimensionless, including
zero; they cannot replace a rate with physical dimensions.

Parameters, states, outputs, and doses carry explicit units. The dosing
declaration makes `central` a valid target; `input_central` is the associated
input in the dynamics. The full [text model](../conformance/models/synthetic-one-compartment.qsp)
and equivalent [JSON model](../conformance/models/synthetic-one-compartment.json)
are included as runnable examples.

Simulation requests use the `qspquest.run/v0.1` schema. They declare a
solver, execution budgets, a regimen, sample times, tolerances, and any
parameter or initial-state overrides. A regimen can include boluses,
infusions, repeated doses, covariates and changes, and resets. Use the
[complete synthetic request](../conformance/requests/synthetic-pbpk-24.json)
as a template. `observations` can request a `pre` or `post` value at an event
time; the returned `times` and `sides` preserve that distinction.

```sh
cargo run --release --locked -p qspquest-cli -- run \
  conformance/models/synthetic-pbpk-24.qsp \
  conformance/requests/synthetic-pbpk-24.json > result.json
```

Solver names include `diffsol_bdf`, `diffsol_tsit45`,
`diffsol_esdirk34`, `diffsol_tr_bdf2`, and `analytic_linear_pk`. Not every
solver supports every model or operation; an unsupported combination returns an error rather than
silently switching solvers. `diffsol_rosenbrock23` and `diffsol_rodas5p` are
unsupported; choose a supported method explicitly. Results include named output
columns, units,
sample times and event sides. These synthetic examples
are illustrative and make no clinical prediction.

Solver tolerances control local integration error. Check convergence under tighter
tolerances for the selected model and execution target. Native and Wasm results
are not guaranteed to be bitwise identical.

Start stiff ODE workflows with `diffsol_bdf`. For a simulation that previously
selected an unavailable method, choose the replacement explicitly and check its
outputs against the model's reference or analytic solution. `diffsol_esdirk34`
provides another implicit method for simulation; sensitivity-based fitting uses
BDF. Tighter tolerances can improve accuracy but cost more callbacks. Extra
observation times do not prescribe the solver's internal step sizes.

Compare numerical results using output-specific bounds in their declared units:
`abs(a - b) <= absolute_i + relative_i * max(abs(a), abs(b))`. Choose those bounds
from the model's accuracy requirements and independent reference before running
the comparison. Check output names, units, sample times and event sides exactly.
Report exact-bit differences separately; one fixed absolute threshold is not a
useful accuracy target for every output scale. Agreement between targets alone
does not establish accuracy.

For the included one-compartment example, compare BDF with a tenfold tighter
request and the analytic concentration `2 * exp(-0.1 * time)` mg/L:

```sh
python3 - <<'PY'
import json
from pathlib import Path
request = json.loads(Path('conformance/requests/synthetic-one-compartment.json').read_text())
request['solver'] = 'diffsol_bdf'
Path('bdf-request.json').write_text(json.dumps(request))
request['regimen']['rtol'] *= 0.1
request['regimen']['atol'] *= 0.1
Path('bdf-tighter-request.json').write_text(json.dumps(request))
PY
cargo run --release --locked -p qspquest-cli -- run conformance/models/synthetic-one-compartment.qsp bdf-request.json > bdf-result.json
cargo run --release --locked -p qspquest-cli -- run conformance/models/synthetic-one-compartment.qsp bdf-tighter-request.json > bdf-tighter-result.json
python3 - <<'PY'
import json, math
from pathlib import Path
coarse, fine = [json.loads(Path(p).read_text()) for p in ('bdf-result.json', 'bdf-tighter-result.json')]
assert coarse['times'] == fine['times'] and coarse['sides'] == fine['sides']
for result in (coarse, fine):
    cp = next(o for o in result['outputs'] if o['name'] == 'cp')
    assert cp['unit'] == 'mg/L'
    assert len(cp['values']) == len(result['times'])
    for time, side, value in zip(result['times'], result['sides'], cp['values']):
        reference = 0.0 if time == 0 and side == 'pre' else 2 * math.exp(-0.1 * time)
        assert math.isfinite(value) and abs(value - reference) <= 1e-10 + 1e-8 * abs(reference)
print('Both BDF runs satisfy the synthetic analytic concentration bounds.')
PY
```

The bounds above apply only to this synthetic example. General models need
their own output scales and reference checks. A convergence failure calls for
investigating the model, tolerances or callback budget; it does not justify
loosening biological constraints or acceptance bounds.

See [fitting](fitting.md) for estimation requests and [browser integration](browser.md)
for local simulation in a Web Worker.

TR-BDF2 checks convergence of the sampled state trajectory by repeating each
interval with tighter local tolerances. It returns the finer trajectory after
agreement within the requested state-error scale. This costs additional solver
work, shares the callback budget, and fails explicitly if six passes cannot
converge. This is a numerical check, not a guarantee of global accuracy.

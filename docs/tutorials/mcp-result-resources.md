# Read a complete MCP simulation result

This example runs a synthetic one-compartment model and retrieves every requested
observation when the inline tool response is a compact preview. It requires
Python 3.10 or later on macOS or Linux, a Rust toolchain, and the QSPQuest source distribution.
Run these commands from the distribution root:

```sh
cargo build --release -p qspquest-mcp
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp
```

The complete runnable example supplies its own model, doses and observation
grid; it requires no model-library download or assistant account. Expected output
is `Verified 301 observations; final concentration 0.09957414 mg/L`.
The [complete example](../examples/mcp_result_resources.py) and
[standalone table reader](../examples/table_bundle.py) are included in the distribution.

The model has volume 3 L and clearance 0.3 L/h. A 6 mg bolus at time zero
produces concentration `2 * exp(-0.1 * time)` mg/L. The example checks every
returned point against this independent analytic solution and verifies the full
result's calculation identity and effective request. These checks verify this
synthetic numerical experiment; they do not establish clinical validity.

The example checks `qspquest://capabilities` against `tools/list`, then reads
`structuredContent.contract` without interpreting the human-readable response.
The contract uses schema `qspquest.mcp-result/v1`. Its `status` is `success`,
`scientific_failure`, `unsupported`, `async_required`, or `request_failure`.
A nonconverged fit has `scientific_failure` even when the protocol's `isError`
flag is false; its estimates remain diagnostic. `async_required` provides a
route and does not mean that a job was submitted or a calculation completed.

`completeness` is `inline_complete`, `resource_linked`, or `summary_only`.
`resource_linked` lists the URIs needed to obtain the complete payload.
`summary_only` cannot supply a complete trajectory. The contract's
`identity.scope` distinguishes a calculation identity from metadata;
`payload_sha256` hashes the returned payload before adding the contract and is
not an engine-build identifier. Library identity and execution availability do
not establish scientific qualification for a modified request.

When `delivery_receipt.inline_fields_omitted` is true, the remaining metadata
also exceeded the response budget. Read every URI in `contract.resources`;
the operation status is preserved even when diagnostics are omitted inline.
Comparison entries retain their names and complete-result links. A local job
provides `delivery_receipt.local_job_lookup.job_id` for `get_local_job`.
The omitted-payload SHA-256 is a transport receipt, not scientific evidence.

`trajectory_kind` distinguishes `compact_preview`, `full_requested_grid`, and
`summary_only`. When `full_result_resource` is present, read that URI using
`resources/read` in the same server session before calculating trajectory-based
results. For a `utf8_chunk_manifest`, join chunks in their listed order and check
the manifest's SHA-256 before parsing the JSON. A preview's sampled rows can omit
extrema. The tool's full-grid sampled summaries remain descriptive trapezoidal
integrals and extrema; they are not certified NCA or extrapolated exposure.

Keep the server session open until resource reads finish. A missing executable
means the build step has not completed or the supplied path is wrong. Structured
simulation errors must be resolved before reading a result; never count them as
successful runs. Declared administrations outside an explicit run window are
rejected. Model-library holds and dimensional limitations still apply when using
archived models instead of this fixture.

To replay a result, call `simulate` with the same model, selected outputs and
`request: full["effective_request"]`. Complete native requests retain their
native parameter and initial-state names and units. Explicit top-level friendly
`parameters` or `initial_states` override the corresponding converted native
keys; the other native inputs remain intact. Incompatible units and unknown
native names are rejected. Compare the returned effective request and its hash
with the intended inputs. Replaying or modifying a preset does not establish
scientific qualification.

The same command checks that an explicitly empty native sampling grid is
preserved. It omits `observations`; a supplied observation plan must be nonempty.
It refuses malformed request text, an unversioned request, an explicit zero end,
and an incompatible threshold unit. Refusals have `request_failure` and no
trajectory or result resource. To observe initial values, use a positive end
and explicitly include time zero; the server does not extend a zero window.

It also starts the synthetic model at 30 mg, giving concentration 10 mg/L,
and requests time above `{"value":5000,"unit":"ug/L"}` through 10 hours.
The threshold converts to 5 mg/L. Expected duration is
`log(2) / 0.1 = 6.93147 h`, checked within `0.0001 h` against the analytic
crossing time. The readout remains a sampled-grid calculation. Its quantity
uses the actual engine output unit; a display alias cannot supply physical
dimensions for a source-scale output declared as `1`. Inspect the returned
notes for conversions, defaults and explicit replacements. Native requests
with both samples and sided observations are invalid; explicit top-level
times can replace the stored observation plan when that replacement is intended.

Readout inputs must select a known kind and output, with a positive window or
an exact point time. Malformed maps and arrays report their field instead of
being ignored. Each trajectory output must contain one finite value per time;
invalid observation sides, mismatched columns or non-finite arithmetic refuse
the summary. AUC includes the post-event start and pre-event end of a window.
`value` defaults to post-event; `trough` uses pre-event. These rules also apply
to `summarize_profile` on supplied points. Timed parameter changes accept only
`name` (or `parameter`), `at` and `value`; conflicting names and unknown keys
are errors. Use explicit quantities to make their units reviewable.


## Inspect a plan before execution

The same runnable example first calls `plan_run` with
`{"operation":"simulate","arguments":arguments}`. Its contract has
`identity.scope: "metadata"`; it contains no calculated concentration. The
complete `plan` lists bound inputs and units, the lowered event schedule,
ordered observation sides, output names/units, storage reservations, numerical
budget ceilings and checks that still require execution. It verifies all 301
requested observations, then executes the identical arguments with
`expected_plan_hash: planned["plan_hash"]`. The returned plan digest, effective
request digest and engine identity must match the inspected preparation.

For large local plans, `plan` is null and `full_plan_resource` points to the
complete plan. Retrieve it in the same session using the chunk procedure above.
Plans share the local result cache's eight entries, 4 MB per-entry ceiling and
15-minute expiry. Retention can evict an older plan or trajectory. Hosted plans
are complete inline or refuse their output budget; there is no hosted plan
resource. A refused or incomplete response does not authorize execution.

Use `required_outputs` with `simulate` or `compare_regimens` when a task needs
specific columns. Each requirement names `output` and may additionally name an
exact declared `analyte` and `compartment` from the model's output contract:

```json
{"outputs":["mab_isf_nM"],"required_outputs":[{"output":"mab_isf_nM"}]}
```

This names a required alias; it does not establish physical concentration units.
Inspect `get_model.interface.output_contract` for native units and any declared
scientific labels. Matching declarations does not establish biological qualification.
A requirement with `analyte` or `compartment` is refused when
that declaration is missing or different. Labels are not inferred from names.
Required columns must already be selected; the validator does not extend an
explicit selection. A comparison checks every member before numerical work.
Plans retain `required_output_receipts` with `selected_for_computation`; complete
results retain `computed`, the native column/unit and sample count. Receipts are
also retained in CSV manifests. A missing column, null value or incomplete grid
is a failure; the server never fills an unfulfilled requirement with zeros.
An actually computed zero remains a valid result. A refused requirement is
reported in `contract.failure.output_requirement` with `fulfilled:false` and a
specific reason. Changing requirements invalidates an inspected plan hash.

Run the complete synthetic workflow with the prerequisites above:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --required-outputs
```

The workflow retains 301 analytical concentrations and a second column computed
as `(central - central) / v`. It verifies those actual zeros, four refusals
(unknown output, absent selection, undeclared analyte and undeclared compartment)
and changed-plan refusal. Expected output is `Verified 301 analytical samples,
an explicitly computed zero column, four unfulfilled output requirements and
changed-plan refusal.` A source-only example has no declared analyte or
compartment metadata; these are synthetic protocol checks, not biological or
physical-unit qualification.

`plan.time_validation` lists each admitted run window in the model time unit,
with the number of sample and forcing times checked. Checks use compatible unit
conversion without changing supplied quantities, observation sides, empty grids
or schedules. Caller-supplied checkpoints, resets, covariate changes, repeated
doses and infusion endpoints are checked before integration. Periodic operations
retain their supported infusion tails. Native static preparation also checks
model-derived boundaries; admission is not numerical or scientific qualification.

Invalid inputs return `contract.failure.input_diagnostic`: the exact field,
supplied value, any converted value, expected constraint and a small native
request example. The example illustrates a time shape; choose bounds and events
for your scientific protocol. The validator does not clamp caller-supplied samples, extend an end
time, remove invalid events or apply the example automatically. Raw population
records follow their explicit inclusion mapping; the mapped numerical request is
then checked. Managed job previews retain their governed platform validation. Oversized field
paths or values carry explicit truncation markers and digests. JSON cannot encode
NaN or Infinity number tokens; null or string representations such as `"NaN"`
are rejected with the named value field. Malformed JSON is a transport error.

Verify this complete synthetic workflow with the same prerequisites:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --time-validation
```

It runs from 2 to 4 h with 6 mg initial central amount, 3 L volume and 0.3 L/h
clearance, retaining 121 supplied sample quantities in minutes. It checks every
concentration against `2 * exp(-0.1 * (time - 2))` mg/L, then checks seven rejected
requests: null and string nonfinite end values, an out-of-window sample, an
incompatible sample unit, conversion overflow, a late checkpoint and an infusion
that extends beyond the end. Expected output is `Verified an unchanged shifted
protocol, 121 analytical samples and seven field-level input refusals before
integration.` These are synthetic admission and numerical checks.

`plan_run` also accepts `scan_parameters`, `fit_scalar`, `fit_individual` and
`fit_population`, each with that tool's complete arguments. Fit plans describe
actual estimator-specific admission, converted observations, bounds and starts;
they contain no likelihood, fitted estimates, covariance or convergence result.
A source-scale archive remains unqualified, and a synthetic model remains
illustrative. `qualification.current_engine_and_request` is `not_assessed`.
A modified stored request cannot inherit preset evidence. Planning evaluates
neither a future trajectory/readout nor a prediction-dependent objective.

Leave `expected_plan_hash` unset during planning. When executing, use the same
arguments and add the returned digest. A changed source, library version,
request, selected outputs or deployment refuses with `plan_mismatch` before
numerical work and file export. Inspect a fresh plan before retrying. A plan
reserves no engine, execution slot, library, job, pricing or qualification;
admission and concurrency checks still apply to execution.

## Inspect initial patient quantities

Purpose: bind scalar covariates or parameter overrides and evaluate the model's
selected outputs at time zero before administrations. Use the default-feature
binary built above; this workflow needs only the included synthetic inputs and
Python's standard library.

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --patient
```

The complete runnable example calls `plan_run` with
`operation: "get_patient_parameters"`, inspects the converted inputs, initial
states, declared units, selected outputs and prospective execution identity,
then executes the same arguments with `expected_plan_hash`. A changed override
is refused. No solver, future derivative, dose, reset, scheduled parameter or
covariate change, or random draw runs during initial-value evaluation. Scalar
scenario parameters, covariates and initial-state overrides are retained.
Nonzero initial exposure stays nonzero; this is the model's initial state,
not a zero-exposure counterfactual or an equilibrium calculation.

Expected output starts with `Verified prepared initial values`. A synthetic
6 mg initial amount and a volume override of 6000 mL give 1 mg/L at time zero,
even though the example's future derivative is undefined. The separate stored
scenario starts at zero: its 6 mg administration does not apply to this tool.
The response includes `patient_quantities`, their actual `patient_units`,
`time` and `time_unit`, complete `effective_initial_values`, the effective
request, execution identity and qualification limits. `get_model` includes
declared scalar covariates and parameters. The argument `covariates` accepts
these scalar inputs with explicit units; `outputs` uses the advertised output
names, and omission evaluates every output.

An output may repeat a stored scalar or reference value; inspect its expression
before calling it patient-specific. Unit `1` preserves the model's declared
coordinates and does not establish physical units. Successful output evaluation
and a matching plan do not establish scientific qualification. Unknown inputs,
incompatible units and duplicate output names are refused. Invalid selected
output expressions can fail during evaluation after static planning succeeds.
Without retained artifacts, large initial-value results fail with
`patient_output_budget`; select fewer outputs. With `--artifact-dir PATH`, read
the linked result for complete quantities, units, inputs and states when the
inline response is capped. A successful calculation retains complete results
inline or in that resource.

## Inspect and execute a regimen comparison

Use the same default-feature binary to run the complete comparison example:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --comparison
```

It prepares named 3 mg and 6 mg boluses with `plan_run`, using
`operation: "compare_regimens"` and the complete comparison arguments. The
plan contains each member's effective request, selected outputs, exact native
preparation, prospective identity and qualification limits in execution order.
The comparison's `effective_source_sha256` hashes the ordered named member source
digests; inspect each member for its individual source digest. Large comparisons
use the same complete local plan resource and hosted output limit described above.

Execute `compare_regimens` with the same arguments and its `expected_plan_hash`.
The example reads each full result and checks all 64 event-sided observations
against `amount / 3 * exp(-0.1 * time)` mg/L, including the undosed pre-side value
at time zero. It verifies both member identities and refuses a changed later
dose. Expected output starts `Verified two prepared regimens, 64 event-sided
analytical samples`. These are synthetic numerical checks, not clinical evidence.

The same command also runs three named conditions in a synthetic irreversible
binding model: `A_bind` uses `kon = 0.5 [1/h]`, `B_zero` sets that starting
parameter to zero, and `C_double_start` doubles the free starting amount.
Both A and B start with 2 mg free and 0 mg bound; B supplies the equivalent
2,000 ug quantity to check unit conversion. C starts with 4 mg free. The example
checks all 54 computed values against `free = start * exp(-kon * time)` and
`bound = start - free`, reads each complete trajectory and rejects a changed
counterfactual with the old plan hash. This illustrates a constant pseudo-first-order
binding coefficient; it does not model a changing target pool or qualify a biological system.

Each named member accepts optional `parameters` and `initial_states` quantity maps.
They merge with the shared maps by requested name; member quantities replace
matching shared names and unspecified names retain shared quantities. An empty
map retains shared quantities. Use one alias or canonical name for each input;
two names resolving to the same input are refused. A member's `parameter_changes`
array replaces the shared timed-change array when supplied. A native base request
or selected scenario is retained and then updated by the member's friendly inputs,
including its explicit dose list. Use matching dose lists when comparing only parameters.
Native initial expressions evaluate with the selected inputs. Stored literal
initial values remain unchanged unless overridden; choose explicit states when
the experiment requires identical starting values rather than a different initialization.

Every planned and executed member has an `input_receipt`. Its `requested_inputs`
retain merged friendly quantities and explicit native request inputs before
normalization, with `requested_inputs_sha256` for their canonical JSON.
`native_parameter_contract_sha256` binds all native parameter/covariate phases,
including derived inputs, exact units, defaults and boundary times, to the selected
effective source. `native_bound_value_hash` also binds the initial state values;
compare it with the member's execution identity. `effective_request_hash` binds
the complete converted numerical request. Receipts and separate computed outputs
keep a baseline and a counterfactual traceable even when their requested labels
or quantities differ. Follow each member's full-result resource to inspect its
complete effective request, override consumers and output arrays. Distinct hashes
show distinct contracts; they do not establish a numerical effect or scientific qualification.

All 1..25 members must pass static preparation before any member integrates.
Names must be unique, nonempty, contain no control characters and use at most
120 UTF-8 bytes. With CSV export enabled, names that produce the same filename
(case-insensitively) and any occupied CSV or manifest destination are refused
before numerical work. Destination inspection creates no directories and reserves
no files. All numerical runs must succeed before CSV export starts. Each bundle
is written exclusively; a later filesystem failure can leave earlier complete
bundles. The comparison is not a filesystem transaction. Hosted comparisons share
one 45-second deadline for preparation and computation; each member retains its
own numerical budget. Without `--artifact-dir PATH`, comparisons share the eight-entry local cache; an evicted
member has no `full_result_resource` and its notice identifies the retained
summary. Use smaller comparisons or CSV export when every full grid is needed.
A plan reserves no execution capacity or qualification.

## Save and reimport an analysis bundle

The local server writes files only when started with a private `--output-dir`
(or `QSPQUEST_MCP_OUTPUT_DIR`). The example enables this explicitly:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --csv-output /tmp/qspquest-table-example
python3 docs/examples/table_bundle.py /tmp/qspquest-table-example/study/trajectory.csv
```

Choose a fresh directory or output filename for each run. Windows paths use the
same bundle format; this stdio example uses macOS/Linux subprocess readiness.
The directory contains `study/trajectory.csv` and
`study/trajectory.csv.manifest.json`. The first command verifies 301 CSV rows,
units, effective inputs, engine identity and refusal to replace either file.
The second prints the validated manifest without starting a server.

For a Python analysis, use the supplied standalone reader from the source root:

```python
import sys
sys.path.insert(0, 'docs/examples')
from table_bundle import read_table

bundle = read_table('/tmp/qspquest-table-example/study/trajectory.csv')
manifest, rows = bundle['manifest'], bundle['rows']
assert len(rows) == manifest['table']['rows'] == 301
print([(c['name'], c['unit']) for c in manifest['table']['columns']])
print(manifest['inputs']['modified'], manifest['qualification']['status'])
```

The manifest uses `qspquest.table-manifest/v1`. `saved_csv` contains both paths
and their raw-byte SHA-256 digests; `qspquest://schema/table-manifest` describes
the complete format. It records ordered column units/native names, model and
archive version/digest when available, actual engine identity, complete effective
native request, selected outputs and actual model source. A supplied source has
no archive version. Recorded archive availability and dimensional status retain
their own scope; `qualified` is false and current-run status is `not_assessed`.
Source-coordinate runs retain `source_scale_unqualified`; declared numerical
units such as `1` are not inferred physical units. A run without a stored preset,
or with changed inputs/source, is labeled modified. An output selection is
recorded separately. The MCP effective-request hash covers normalized inputs
and its internal output selection; the engine's `run_request_hash` identifies
its native request. These digests do not qualify another request or platform.

The reader verifies CSV bytes, ordered headers, row/event counts, finite cells
and nondecreasing time. Exactly two rows at one time must be `pre` then `post`;
keep both when interpreting instantaneous events. CSV aliases containing commas
or quotes are escaped normally. Limits are 256 MiB of CSV, 16 MiB of manifest
and 2,000,000 rows; admitted model/request limits still apply separately.
Keep the two files together with their original basename. Changing data, renaming
only the CSV, omitting the sidecar or reversing event sides fails validation.

Both filenames are reserved exclusively beneath the anchored local output root.
Handled write failures clean newly created entries. A process crash can leave an
incomplete pair; validation rejects it, so rerun with a fresh name. Digests detect
corruption, not malicious replacement of both files or producer authenticity.
For R or other table tools, first run the verifier, then retain the manifest with
any imported or derived data. Do not overwrite the original CSV with a spreadsheet
or `write.csv` export: changed bytes no longer match its digest.


A unit error names the affected field and the requested and declared parameter or
covariate units. A declaration of `1` uses dimensionless or source coordinates;
the server does not infer a missing physical normalization. Supplying `1/h` to
such a field is rejected before integration. Inspect the source-coordinate
contract rather than substituting a physical unit. Molar-mass advice applies only
to a mass-versus-amount dimension mismatch, such as `mg/L` versus `mol/L`;
the server does not convert mass and amount automatically. Convert explicitly
using the known substance’s molar mass, then supply the unit declared by the model.

## Plan and run synthetic Morris screening

This workflow inspects a bounded parameter-screening design before executing it.
It requires Python 3 and an MCP binary built with the optional `analysis` feature.
Run these commands from the released source tree:

```sh
cargo build --release --locked -p qspquest-mcp --features analysis
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --morris
```

The example loads the complete synthetic Morris request included in the release,
prepares six points and two trajectories at seed 42, executes with the returned
plan hash, and checks concentrations and elementary effects against the analytical
one-compartment expression. It also verifies that changing the seed refuses the
original guard. Expected output starts with `Verified six seeded Morris points`.

Call `plan_run` with `operation: "morris_screening"` and the same `model` and
native `qspquest.morris/v0.1` `request` used by `morris_screening`. Inspect the
complete `qspquest.morris-plan/v0.1` native preparation, then add the returned
`plan_hash` as `expected_plan_hash` for execution. Plans contain the exact scan,
range units, selected output/time/side, point order and step mapping. An explicit
design is supported through the same native request contract.

An absent tool means the binary lacks `analysis`; rebuild with that feature.
An out-of-range row or unequal time/side grids across points fails before any
simulation. Generated design coordinates are deterministic metadata; trajectory
domain, convergence and elementary-effect/statistic overflow remain execution
checks. These synthetic checks do not establish biological interpretation,
identifiability or scientific qualification for a library model.

## Plan and guard a periodic steady-state solve

Malformed analysis fields return an `error.expression` JSON pointer, such as
`/request/problem/request/ranges/1/upper/unit` or
`/request/relative_tolerance`, with a repair suggestion. Correct that field and
prepare the complete request again before execution. Structural admission does
not change quantities, grids, seeds, tolerances or budgets; native preparation
checks compatible dimensions and design validity.

Use the same analysis build from the Morris example above:

```sh
cargo build --release --locked -p qspquest-mcp --features analysis
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --steady-state
```

This example uses the included synthetic linear PK model and steady-state
request. It calls `plan_run` with `operation: "steady_state"`, inspects the full
`qspquest.periodic-plan/v0.1` preparation, then calls `steady_state` with the
same arguments and returned `expected_plan_hash`. It verifies the exact
prospective identity, the bounded cycle residual and rejection of a changed
convergence tolerance. Expected output starts with `Verified periodic preparation`.
The analysis feature must be present in the active server's capabilities.

Inspect the normalized infusion schedule, coherent amount units and conservative
per-cycle budget partition before execution. The plan computes no fixed point.
Read the [periodic preparation contract](../../spec/run-results.md#periodic-preparation)
for complete CLI examples and the separate BDF iteration contract. Plans and
convergence of these synthetic examples establish no scientific qualification,
uniqueness or stability for another model or request.


## Retain complete results across local server restarts

Use an explicitly selected private directory when a requested result may exceed
4 MB, or when an agent must retrieve it after the server exits. This workflow
requires the local stdio server and Python 3. It uses the synthetic model in the
included example and does not need a model-library download.

```sh
cargo build --release --locked -p qspquest-mcp
artifact_dir=$(mktemp -d)
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --artifacts "$artifact_dir"
```

The shell block uses a POSIX shell on macOS or Linux. On Windows, create an
empty private directory, use `target/release/qspquest-mcp.exe` as the binary
path, and pass the directory path after `--artifacts` in the Python command. The
script uses a portable standard-library client.

The script starts the server with `--artifact-dir PATH`, requests 6,002 sided
rows for 48 concentration outputs, and checks every value against the analytic
solution above. It verifies the complete result exceeds 4 MB, survives nine
further calculations, and remains identical after a server restart. Expected
output starts with `Verified` and reports the retained byte count, all rows,
outputs, identity, digest and unchanged expiry. Results describe this synthetic
numerical example; they do not establish scientific qualification.

When this option is enabled, completed numerical operations write immutable
JSON artifacts before sampling their inline responses. The structured response
contains `full_result_resource` and a `result_artifact` descriptor using schema
`qspquest.result-artifact/v0.1`. Read the root URI to obtain that descriptor.
Read `chunk_uri_prefix` followed by each integer from zero through
`chunk_count - 1`. Concatenate the UTF-8 text without separators, check its byte
length and SHA-256 against `bytes` and `sha256`, then parse the complete JSON.
The example implements these steps in `full_text`. Chunks are at most 48,003
bytes so multibyte characters remain intact. The complete result retains every
requested row, selected output, equal-time event side, calculation identity,
effective request and model provenance. Checksums verify bytes, not producer
identity or scientific qualification. The descriptor schema is available at
`qspquest://schema/result-artifact`.

Retention is 24 hours from creation and survives restart. Reading a result does
not extend its expiry. Each body is limited to 128 MB; a directory holds at most
256 live artifacts, 512 receipts (including expiry markers), and 1 GB including
receipts. Capacity refusal preserves
published live references. Expired bodies are removed on startup or before a
new write; their expiry receipts remain for seven additional days. Before
receipt cleanup, reads return `artifact_expired`; afterward they return
`artifact_not_found`. Ordinary resource reads and listing never delete files.
After stopping the server, delete the dedicated directory when you no longer
need its artifacts. Keep any separate scientific evidence you intend to retain.

Use an empty directory dedicated to these results. Only one server can own it
at a time. The server rejects unrelated files, links, nonregular files and
unsafe permissions. Do not edit persisted bodies or receipts; corruption is
refused. These local files are private to the operating-system account, not a
tenant authorization boundary. Local artifact storage is unavailable in HTTP
mode. Hosted governed artifact retrieval uses its separately authorized
workflow. With artifact storage enabled, numerical tool annotations advertise
persistent writes and fresh artifacts on each calculation. Metadata discovery
and resource reads remain read-only. Starting without `--artifact-dir` retains
the bounded process-local cache described above.

## Verify dose amounts before interpreting concentrations

Purpose: inspect converted dose events and bind execution to the reviewed
schedule. Use Python 3.10 or later, a Rust toolchain and the source distribution.
From the distribution root, run this complete synthetic example:

```sh
cargo build --release --locked -p qspquest-mcp
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --dose-events
```

Expect confirmation of three equivalent mass inputs, two 150 mg dose receipts
and analytical concentrations. The example submits 150 mg, 150000 ug and 0.15 g;
each produces two 150 mg boluses at 0 and 12 h and the same concentration values.
The supplied model uses a 3 L volume and elimination rate 0.1/h. Its concentration
starts at 50 mg/L after the first dose and follows the sum of the two exponential
dose contributions. Pre-side samples at a dose time exclude that dose.

`plan_run` returns `dose_event_receipts` with stage `prepared_schedule`. Each
receipt gives the canonical target, event index, time, delivery type, converted
`amount_before_model_scaling`, actual target-state unit and declared base scale.
Use the returned `plan_hash` as `expected_plan_hash` for execution. A successful
simulation records stage `executed_schedule`. Repeats, lag, infusion duration and
regimen availability follow the native plan; the adapter does not reconstruct
them from friendly names.

The event amount precedes `dosing.scale`; it is not necessarily the state
increment or a concentration. The base scale is an expression. Inspect native
binding phases and switches when that expression depends on inputs. A mass
concentration such as mg/L cannot be substituted for a dose amount in mg.
`get_model` lists the canonical target and actual state unit alongside readable
aliases. When an archive declares a source dose coordinate, the receipt includes
its coordinate name, native target, source dose unit, amount unit, state unit and
declared scaling. Inline models have no archive source declaration, so this field
is null. A declaration does not establish independent scientific qualification.
Explicit unit `1` in the convenience API means an amount already in model
coordinates; it does not identify a physical unit. Prefer the declared dose unit.

CSV sidecars preserve the complete dose-event and required-output receipts.
The [CSV reader](../examples/table_bundle.py) checks their types and consistency
with the exported columns, units and clock. Large inline replies may omit the
receipt list with `dose_event_receipt_notice`; retrieve `full_result_resource`
for the complete schedule before making calculations. Wrong dose dimensions or
contradictory archive coordinates are refused with an actionable error.

For first-order absorption or intravenous dosing in a `linear_pk` declaration,
run the included synthetic verification:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --linear-pk-doses
```

Expect four workflows, eight prepared/executed receipts and analytical
concentration checks. The complete example defines a 3 L central volume,
0.3 L/h clearance and, for oral dosing, a depot with absorption rate 1.2/h.
It gives two 6000 ug administrations with availability 0.5, a 1 h lag and a
6 h interval. Receipts report 3 mg at 1 and 7 h. The workflows dose the depot,
the central compartment with a depot present, the central compartment without
a depot, and two 2 h infusions without a depot. The infusion model declares
`central [mg] = dose_scale`, where `dose_scale` is 2. Its receipt retains the
symbol expression; each infusion delivers 6 mg after that scale is applied.
Default linear PK dosing uses scale 1 and the declared `amount_unit`.
The example checks the first-order absorption and elimination equations and
the infusion integral at every returned time, respecting pre/post event sides.
These synthetic checks establish the documented numerical workflow; they do
not qualify a drug-specific model.

## Inspect large models in bounded pages

Before choosing a library model, inspect `get_model`'s
`source_interpretation_summary`. A `declared` status gives the exact archive and
native-model hashes, the number of structural interpretations, and a
`complete_declaration_resource`. Read that resource's
`metadata.source_interpretations` for every equation/page locator, printed and
adopted coordinate, rationale, conservation implication and qualification limit.
Declarations describe selected conventions; their presence does not establish
author-code agreement or physical qualification. A `not_declared` status does
not establish literal fidelity to the source.

Purpose: inspect complete native declarations and output mappings without
assembling the whole model card. Use Python 3.10 or later, a Rust toolchain and
this source distribution. From the distribution root, run the complete synthetic
example:

```sh
cargo build --release --locked -p qspquest-mcp
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --model-pages
```

Expect confirmation of four inspection sections, complete declarations,
page/record resource equivalence and a changed-content refusal. The example
creates its own small model library, retrieves its index, traverses one record
per page and checks every corresponding resource. It needs no installed library
or scientific data. Inspection performs no simulation and grants no scientific
qualification.

For a library model, call `get_model` with `model_id` and its exact `version`.
The card returns `inspection` and `inspection_index_resource`. The index gives
record counts and first-page resources for `scalars`, `states`, `outputs` and
`equations`. Native document, record bytes, metadata and alias hashes bind the
inspection context. `archive_sha256` is present when the record declares an
archive; standalone source bytes have `record_sha256` instead. These hashes
identify retained content, not numerical accuracy or scientific qualification.

Request a tool page with arguments such as:

```json
{"model_id":"synthetic_resource_example","page":{"section":"outputs","offset":0,"limit":32,"expected_inspection_hash":"COPY_THE_INDEX_INSPECTION_HASH"}}
```

Copy the actual hash from the index. Pages accept limits from 1 to 128, defaulting
to 32. Follow `next_offset` or read `next_page_resource` until it is null. A page
can return fewer records than the requested limit to stay within the byte budget;
do not advance by adding the requested limit. Request `offset == record_count`
for an empty terminal page. Invalid limits and offsets produce actionable errors.
Use `page` by itself; `outputs` selection and `detail: "full"` request model cards.

Each entry includes its exact `native_path`, canonical name, declared aliases,
complete native record and `record_resource`. Output records include native
units, expressions and the same analyte/compartment descriptors used by named
output selection. To inspect one known output directly, use `get_model.outputs`
with its canonical name or a declared alias; no complete mapping download is
needed. Source-unit labels remain separate from the actual native units.

Scalars include parameters, covariates, individual expressions and definitions,
including unused stored parameters. States include declared state records and
any linear PK declaration; a linear PK record contains its compartment states,
initial values and coefficient names. Equations retain state initial/RHS/dosing
declarations, derived bindings, outputs, switches, dose history and linear PK.
Records follow native declaration order within those groups. The index links
complete model details for requirements, invariants and other metadata.

An oversized record or alias list is null inline and remains complete at its
`record_resource`. Read and verify its UTF-8 chunks as described above. Null
means omitted inline, not missing, zero or uncomputed. If an archive, native
document, alias or stored scenario changes, the old hash and old inspection resources are refused;
retrieve the current index before continuing. Metadata-only records have no
native sections and refuse page requests. Platform catalog descriptors likewise
require their declared governed workflow rather than native inspection.

## Bind inspected inputs to a request

Purpose: use exact native defaults and units, then refuse a request when its
inspected model or input bindings change. Prerequisites: Python 3.10 or later,
a Rust toolchain and this source distribution. From its root, run:

```sh
cargo build --release --locked -p qspquest-mcp
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --input-bindings
```

The example creates its own synthetic library. It checks clearance `cl = 0.3
L/h` and volume `v = 3 L` from scalar inspection, supplies `300 mL/h` and a
`6000 ug` central initial amount, and checks concentration against
`2 * exp(-0.1 * t)` in mg/L at zero and one hour. It then changes the synthetic
volume, restarts the library and verifies explicit migration guidance for the
previous inspection hash. Expect confirmation of defaults, unit conversion,
request binding and changed-binding refusal.

Copy `inspection.context.inspection_hash` from `get_model`. Place it in
`model.library.expected_inspection_hash` for planning and execution:

```json
{"model":{"library":{"model_id":"synthetic_resource_example","expected_inspection_hash":"COPY_THE_INDEX_INSPECTION_HASH"}},"parameters":{"cl":{"value":300,"unit":"mL/h"}},"initial_states":{"central":{"value":6000,"unit":"ug"}},"end":{"value":1,"unit":"h"},"times":[{"value":0,"unit":"h"},{"value":1,"unit":"h"}],"outputs":["cp"]}
```

For a versioned archive, also include its exact `version`. A stored scenario
uses `model.scenario` with `model_id`, `version`, `scenario_id` and
`expected_inspection_hash`. `get_model`, `get_scenario_request` and the
`get_capability.model` selection accept the same optional guard. Inline
`model.source` has no library inspection context; use the plan hash to bind
that request. Add the returned `expected_plan_hash` to the actual numerical
call, as described above, to bind its effective inputs and preparation.

Scalar and state pages expose canonical declarations and executable request
aliases. Native parameter defaults and state initial expressions remain in
the native records. State initial expressions can depend on parameters; use
`get_patient_parameters` to inspect effective starting values for the selected
request. Archive scenario overrides can likewise differ from stored defaults.
A parameter's `fixed` flag describes fitting behavior; it does not supply a
new value or establish scientific qualification.

Use actual native units for canonical input names. Declared archive input
aliases retain their explicit conversion coordinates. A state readout name
also names an initial state only when its output expression is exactly that
native state symbol, or the archive explicitly declares an input-state binding.
Computed readouts are not input aliases. Source display units remain separate
from native units: a source label such as nM does not authorize nM input into
a state declared in unit `1`. Preserve model-coordinate values unless an
explicit conversion is declared. Supply one alias or canonical name per input;
duplicate names for the same input are refused.

When the model bytes, archive, metadata or effective input bindings change,
the guarded call fails before numerical computation with code
`inspection_changed`. `contract.failure.inspection_change` includes the old
and current hashes, model/version, current `inspection_index_resource` and
`migration: "inspect_current_bindings"`. Read that index, compare the names,
native units, defaults and initial expressions, and explicitly update the
request before creating another plan. A malformed digest is refused with
`inspection_hash`. An unknown parameter, initial state, dose target, scan parameter
or patient input receives `input_alias_unknown` even without a hash guard.
`contract.failure.input_binding` identifies its group, located field, selected
model/version and selected source hash. Library selections provide the current
inspection hash and index resource with `migration: "inspect_current_bindings"`.
Read that index, inspect the actual native units and declarations, explicitly
choose the intended input, then request a new plan. Inline source selections
instead return `migration: "inspect_submitted_model"`; inspect the submitted
model's declarations. No name is guessed or replaced.

A complete `qspquest.run/v0.1` request requires canonical native names for
parameters, initial states, dose/reset targets and state tolerance keys. Supplying
a declared alias there returns `input_name_requires_native` and its declared
canonical name/unit when bounded enough to include. Use the inspected canonical
name and native unit, or put an alias in the documented top-level friendly field.
The server does not move or rewrite the refused input. Name validation precedes
numerical preparation and large-model execution routing; the diagnostic reports
`numerical_execution_started: false`. Long name and field previews are bounded
with truncation flags and SHA-256 digests of their complete text. Retain the
original arguments when reviewing a shortened diagnostic.

Missing versions remain lookup refusals. These checks establish input identity
and conversion behavior, not scientific qualification.

## Inspect pre-dose initialization and percentage denominators

This workflow shows when a changed clearance takes effect and which input
normalizes a percentage output. It uses a synthetic model supplied by the
example, Python 3.10 or later, and the MCP executable built above. From the
distribution root, run:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --initialization
```

The example initializes a target at zero at −20 h, synthesizes it at 2 mg/h,
and clears it at 1/h. A separate marker receives 1 mg at 0 h; this marks the
intervention clock without changing the target. It compares the stored baseline,
clearance 2/h active throughout the pre-dose interval, clearance changed to 2/h at −10 h during the pre-dose interval,
and clearance changed to 2/h at 0 h. For constant clearance `k`, the independent target solution is
`2/k * (1 - exp(-k * (time + 20)))` mg. The example checks 48 percentage values
against this solution and its piecewise counterpart for the boundary change.

`plan_run` supplies `plan.initialization_receipt` with stage `prepared`.
Its integration clock, initialized states and binding phases come from native
preparation. The initialized state values precede boundary updates and doses.
`predose_integration` describes the interval before the first dose, here
−20 to 0 h; it makes no equilibrium or convergence assertion. The first input
phase activates at initialization; a timed change activates `post_boundary` at
its specified time. Starting parameter overrides apply from the integration
start. Changing a parameter does not automatically recompute a separately
declared state initial value: supply the intended `initial_states` explicitly
or use a stored scenario that couples those inputs.

An archive can provide a `protocol_initialization` catalog descriptor with an
exact file path and SHA-256. Its `qspquest.protocol-initialization/v1` declaration
pins model, archive and native scenario hashes, interprets the stored clock,
and defines normalized readouts. Inspect this declaration through `get_model`.
Declarations must match the actual native readout expressions and input kinds.
Models without a declaration still receive the native initialization receipt;
their normalization policy remains undeclared.

The synthetic `fixed_pct` divides `100 * target` by the declared `reference`
input, initially 2 mg. Its basis is `declared_reference_input`: the reference
stays fixed unless explicitly changed. `condition_pct` divides by the derived
`condition_reference = synthesis / clearance` binding. Its basis is
`condition_specific_binding`: each native input phase evaluates that binding.
Neither policy automatically uses the target value at 0 h as a new baseline.
The receipt lists the exact positive denominator and unit for each phase.
A nonpositive denominator for a selected normalized output is refused during
planning; fix the input or select another output before execution.

Execute the unchanged arguments with `expected_plan_hash`. The full result
contains the receipt with stage `executed` only after its prospective identity
matches the actual native execution identity. Keep `full_result_resource` when
the inline response omits a large receipt. CSV sidecars retain the complete
receipt; the included table reader rejects changed denominator bindings, clock
windows and execution identities. Source declarations describe archived
scenarios, while the actual receipt describes the effective request. Inspect
both when changing a preset. These checks establish synthetic workflow
mechanics; source-coordinate labels and percentages do not qualify a biological
model or its physical units.

Expected output begins `Verified four protocol conditions, 48 analytical
values`. A stale inspection hash after changing a declaration is refused;
inspect the current model and plan again. Keep the server open for resource
reads and keep the CSV with its `.manifest.json` sidecar.

## Select a complete precision profile

Purpose: inspect named scalar conventions, select one complete parameter map,
and keep its receipt with the prepared request, computed result and CSV. Run the
self-contained synthetic workflow with the same installed engine and Python:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --precision-profiles
```

The script generates its complete synthetic archive and profile file in a
temporary directory. It declares baseline clearance `0.3 L/h` and alternative
clearance `0.6 L/h`, with fixed volume `3 L`. The alternative run administers
`6 mg` at time zero and returns thirteen post-event observations, zero through
twelve hours. Each concentration equals `2 * exp(-0.2 * time)` mg/L within the
script's fixed tolerance. These synthetic choices demonstrate the transport
contract; they are not reported biological rounding conventions.

Expected output is `Verified complete profile discovery/selection; 13 analytical
values; native plan/result and CSV receipts; changed plan refused; modified
profile labelled; two separately computed comparison profiles.` Temporary files
are removed after all checks finish. The
script needs no model-library download, experimental data or account. Unknown
profiles fail before numerical execution, and changing the profile invalidates
the retained plan hash.

For a catalog model that declares profiles, inspect `get_model.precision_profiles`.
It lists complete profile IDs and choices, the archive default, mutable parameter
count and the exact profile-file SHA-256. `full_model_details_resource` retains
the complete `precision_profile_manifest`, including every scenario's scalar
map and source equation/table locators. Profiles are optional; no declaration
does not imply another model's conventions.

Select an exact ID in both `get_scenario_request` and the simulation's scenario:

```json
{
  "model": {
    "scenario": {
      "model_id": "synthetic_resource_example",
      "version": "1.0.0",
      "scenario_id": "baseline",
      "precision_profile_id": "alternative"
    }
  },
  "outputs": ["cp"]
}
```

The synthetic archive above exists only inside the running example. For your own
installed catalog, use its inspected model/version/scenario/profile IDs and
native units. Retain `expected_inspection_hash` after inspection and
`expected_plan_hash` after `plan_run`, as in the preceding workflow.

Selecting a profile replaces every mutable starting scalar parameter, while
retaining archived starting states, doses and protocol. Omission selects the
declared archive default. Friendly scalar edits and timed parameter changes are
still explicit exploratory inputs; the `precision_profile_receipt` marks them
`modified` and retains the base profile hash and actual starting parameter map.
Starting-state and dose edits remain separate from this scalar-convention status.
Fixed inputs declared by the profile retain their defaults; changing one is
refused with `precision_profile_immutable`. Complete native binding phases in the
plan describe active values over time.

Receipts identify model, archive, scenario, profile, choices, exact profile-file
hash, full effective scalar map, immutable defaults and computational
transformations. They occur in the complete scenario request, native MCP plan
and result, full result resource and CSV provenance. Native receipt parameter
hashes use RFC 8785 canonical JSON with a `sha256:` prefix. Manifest-declared SDK
hashes use their separately declared encoding; they cannot replace native
identity hashes. File hashes bind the exact manifest bytes.

Named selectors are supported for simulation, comparison members and initial
value evaluation. For another exploratory operation, retrieve and inspect the
complete scenario request before applying it explicitly. Unknown profiles,
incomplete maps, stale contexts, archive/native mismatches and altered file
digests are refused. Source rounding differences alone do not establish failed
biology, physical dimensions or scientific qualification.

For `compare_regimens`, each named member may also supply its own
`precision_profile_id`. This requires a shared `model.scenario` selection.
Omission inherits the shared selector or archive default; an explicit member
selector replaces the complete scalar convention before shared and member
overrides are applied. Each member keeps its own receipt, native preparation and
computed result. The example checks both conventions separately against their
analytical trajectories; it never relabels a shared trajectory.

## Inspect scheduled boundaries and observation sides

Use this workflow to check when the driver changes its inputs, applies a dose,
stops an infusion, or visits an explicit checkpoint. It also demonstrates why
a time predicate evaluated at an observation can differ from the input used
while integrating the preceding interval.

Prerequisites are Python 3.10 or later and the local MCP executable. From the
release source root, build and run the complete synthetic example:

```sh
cargo build --release --locked -p qspquest-mcp
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --scheduled-boundaries
```

The script supplies all model text and inputs, uses a temporary local library,
and checks its saved CSV bundles before cleaning them up. Expect:

```text
Verified four predicate conventions, 144 analytical values, native schedule receipts, CSV sides and 24 changed-receipt refusals
```

The one-state model has amount in mg and clock in h. It receives a 10 mg bolus
at 0 h, a 5 mg bolus at 2 h and a 1 mg infusion from 3 to 4 h. A declared fixed
switch changes clearance at 2 h. The script exercises `<`, `<=`, `>` and `>=`
predicates and requests both `pre` and `post` at every observation, including
points immediately before and after the switch. It additionally requests a
smooth checkpoint at 4.5 h.

For the `<` example, clearance is 1/h before 2 h and 2/h afterward. Before the
second bolus, `amount(2 h, pre) = 10 * exp(-2) mg`; afterward it is
`amount(2 h, post) = 10 * exp(-2) + 5 mg`. From 2 to 3 h it decays with rate
2/h. During the infusion, the added amount after elapsed time `d` is
`(1 - exp(-2*d))/2 mg`; after 4 h that contribution decays with the same rate.
The script checks the full requested grid against these analytical curves.
The `predicate` output uses the exact observation time: `<` is false at 2 h
for either side, while `<=` is true. Observation side controls state updates;
it does not replace a declared predicate's comparison operator.

`plan_run` returns `plan.scheduled_boundary_receipt` with stage `prepared`.
Each `simulate` result and full result resource carries the same receipt with
stage `executed`, after the actual identity, times and sides are checked. It
contains:

- Fixed-switch names, original time expressions, evaluated thresholds and the
  native parameter bindings used to evaluate them. Thresholds use `time_unit`.
- Original dose-relative switch declarations, including target, offset,
  comparison and simultaneous-dose convention.
- Ordered boundary rows with explicit/native checkpoint flags, native
  post-binding phase indexes, fixed-switch names and event references. Infusion
  start and end refer to the same native event. Native event amounts and units
  remain in the native plan and dose-event receipts.
- The complete requested observations, their native driver row indexes and
  exact-time predicate input overrides. Input phase indexes refer to
  `native_preparation.plan.binding_phases` and the initialization receipt.
- The requested solver, relative tolerance and evaluated state absolute
  tolerances with their state units. `achieved_error_estimate` is `null` because
  native execution does not report an achieved-error estimate.

A boundary row can contain several origins at the same time. A requested smooth
checkpoint does not imply a forcing change. A fixed predicate outside the run
still has a declaration and initialized input value, but no in-window boundary
index. A step binding already active at the integration start can also lack a
scheduled boundary index. Dose-relative declarations retain their source form;
use the expanded native binding phases and pointwise overrides for actual
activation and observation behavior.

If the inline receipt exceeds the response budget,
`scheduled_boundary_receipt_notice` identifies its omission. Retrieve the
complete result resource, or the named comparison member's result resource.
CSV sidecars preserve the receipt; the table reader checks its native clock,
identity, phase references, predicate conventions and actual CSV times/sides.
The tutorial proves refusal of changed threshold, phase, pointwise value,
identity, observation-side and achieved-error claims.

Always state the requested side when interpreting a dose-time value. Changing
an observation's side can change its amount without changing the integrated
intervals. Requested tolerances and completed execution do not establish
physical-unit, conservation or independent scientific qualification. Inspect
the selected model's qualification and provenance before interpreting biology.

## Inspect separate scientific assessments

Use this workflow to distinguish numerical parity from source fidelity, conservation,
author-code parity, clinical validation and physical dimensions. A declaration is
bound to its exact model version, archive and native model bytes. Its evidence has
an explicit method, digest and scope. Digests verify content identity; they do not
authenticate a producer or establish the truth of a scientific claim.

Prerequisites: the source release, its existing Rust toolchain and Python 3.
From the release directory, build the local stdio server and run the complete
synthetic example:

```sh
cargo build --locked -p qspquest-mcp
python3 docs/examples/mcp_result_resources.py target/debug/qspquest-mcp --scientific-assessments
```

The example includes all required inputs and creates its own temporary library,
archive, pinned declaration and CSV output directory. It makes no network calls.
The two synthetic states start at 1 mg and 0 mg. Central loses at twice the
transfer rate while peripheral receives at only the transfer rate. At the native
default rate of 1/h, the analytical values are `central = exp(-2t)` mg and
`peripheral = (1 - exp(-2t))/2` mg. These match numerical execution, while their
sum decreases: the purported closed transfer has an explicit discrepancy.
Numerical agreement therefore coexists with a conservation finding.

The program checks six values at 0, 1 and 2 h, then another six values with the
rate changed to 2/h. It verifies sixteen changed-CSV receipt refusals and refuses
a stale inspection after the declaration changes. Expected output begins:

```text
Verified separate scientific axes, 12 analytical values, archived conservation finding, modified-request scope, 16 CSV tamper refusals and changed-declaration freshness.
```

Inspect `get_model.scientific_assessment` before choosing a model. A null value
means that no scientific assessment declaration is available. Each declared axis
has one of `not_assessed`, `passed`, `failed` or `discrepancy_observed` and a scope.
An assessed axis must include an evidence digest, method and scope. Failed or
discrepant assessments also include explicit findings with an expression, value,
declared unit and native symbol references. There is no general `validated`
status. A passed numerical record must not be interpreted as a passed clinical,
physical, conservation or source-fidelity assessment.

`plan_run.plan.qualification.scientific_assessment` retains the declaration,
whether the effective source matches the archive and whether the request matches
its stored preset. This flag compares the native input request; output selection
and reporting options are retained separately and never qualify a current result.
A null applicability value means that comparison is not
applicable or has no single source/request; inspect named comparison members.
`current_engine_and_request` remains `not_assessed`: preparation and successful
execution do not transfer an archive assessment to a current calculation.
Modified parameters, doses, outputs, times or source require evidence applicable
to the effective request. The original finding is retained as an archive record;
it is not recomputed or silently rewritten after an override.

The complete numerical result and CSV sidecar retain the same
`scientific_assessment` receipt. The CSV reader checks the closed schema,
independent axes, model identity, pinned provenance and preset applicability. It
checks transport consistency, not scientific validity. Read the complete result
resource when the response links one; a compact preview is not new qualification
evidence. For a stale inspection error, inspect the current declaration and
prepare a fresh plan instead of copying the old hash.

A library may provide `scientific_assessments: {path, sha256}` in a catalog row.
The server admits only a safe relative path to a declaration no larger than
24 KB; wrong archive/native hashes, duplicate or missing axes, unsupported
outcomes, unscoped evidence and unknown finding symbols are rejected during
library loading. Libraries without a declaration remain usable within their
existing execution contract, with scientific assessment unassessed. Hosted
execution and clinical or biological qualification are not established by this
local synthetic workflow.

## Format complete decision responses

Use `format_response` when a decision asks for specific identifier fields and
numeric arrays from several computed conditions. Each field explicitly maps an
actual returned column to the requested JSON key, with source units, requested
times and a mandatory `event_side`: `pre`, `post` or `both`. At a duplicate
dose time, `post` selects the post-dose value and `pre` the pre-dose value.
`both` includes both values with separate time/side/source-row receipts. Every requested side must exist in the source, including at smooth times.
A missing side requires a fresh simulation with that explicit observation; no
pre/post value is inferred from the other side. The tool never interpolates, converts
units, solves a model or guesses a missing output.

Prerequisites: the source release, its Rust toolchain and Python 3. From the
release directory, run this complete synthetic example:

```sh
cargo build --locked -p qspquest-mcp
python3 docs/examples/mcp_result_resources.py target/debug/qspquest-mcp --response-guard
```

The example includes its own temporary model library and artifact directory. It
computes a single 6 mg bolus, two 6 mg boluses, and two 12 mg boluses, with doses
at 0 h and, for repeated dosing, 2 h. Volume is 3 L and clearance is 0.3 L/h.
Each administered dose contributes `dose / 3 * exp(-0.1 * elapsed_h)` mg/L.
The program runs both local artifact storage and cache storage. In each mode it verifies 18 actual computed rows against these analytical values,
maps `library_state_0` into nine explicitly named concentration arrays, and
checks a literal decision identifier. It exercises retained runs, immutable
artifacts and SHA-256-pinned supplied JSON, then refuses nineteen malformed or
incomplete responses/sources. Expected output begins:

```text
Verified response guard: 18 analytical observations, three computed conditions, nine explicit column/key/side mappings
```

For a post-dose array from one computed condition, the tool arguments are:

```json
{
  "runs": [{"label": "A", "source": {"kind": "retained", "uri": "qspquest://runs/RESULT_ID"}}],
  "fields": [{"key": "A_concentration_mg_L", "run": "A", "column": "library_state_0",
              "unit": "mg/L", "time_unit": "h", "times": [0, 2, 4], "event_side": "post"}],
  "identifiers": {"decision_id": "dose-side-comparison"}
}
```

Replace `RESULT_ID` with the exact run identifier from execution. Use the
returned `full_result_resource` artifact URI directly when artifact storage is
enabled. Give each condition its separately computed source. Where available,
include `expected_plan_hash` and `expected_effective_request_hash` from that
condition's execution; a swapped source then fails instead of inheriting a new
label. Source digests and execution identities remain in the copying receipt.

The result contains `formatted_response` and `response_guard`. Only copy
`formatted_response` into the requested decision object after checking the
receipt's source identities, mappings and observation sides. To check an
existing answer, pass it as `candidate`: its exact key set, identifier strings
and finite scalar arrays must equal the requested source values. Missing
conditions or keys, extra keys, two-sided event objects inside scalar arrays,
booleans, guessed zeros and changed identifiers are failures. Rounded values
also differ: supply the original parsed numbers. `candidate_matches` describes
this copying check; it does not qualify the scientific result.

Use the actual returned column from `output_mapping`, rather than inferring a
state index from an alias. Optional `expected_analyte` and
`expected_compartment` require matching declared metadata. An absent declaration
fails that check; a chosen response label cannot establish biological identity.
Missing times require a fresh simulation with those observations. Complete
successful trajectories are required; previews, jobs and nonconverged sources
are refused. Every declared run must serve a requested field, and each requested
field must have its named run.

Local references are available only in the owning local server. A missing or
expired cache entry requires its immutable artifact or a fresh run. Arbitrary
file paths, URLs, plan references and artifact chunk references are refused.
Hosted servers require already authorized retrieval followed by
`{"kind":"supplied","json":"EXACT_COMPLETE_JSON","sha256":"LOWERCASE_HEX_DIGEST"}`.
The digest covers the exact UTF-8 JSON string; it verifies bytes and does not
authenticate a producer. Supplied result metadata remains caller-declared.
Read each source's independent scientific assessment: the guard's
`current_engine_and_request` remains `not_assessed`.

Requests allow at most 32 runs, 64 array fields, 32 identifier fields and 4096
unique times per field, within the active input/inline-output budgets. Complete source bodies have an aggregate 128 MB limit. If the
formatted answer and full copying receipts exceed the inline budget, select
fewer fields or times. This does not change the original complete result.


## Select exact outputs, scenarios and inspection resources

Use this workflow when you already know the output name and stored scenario ID.
It retrieves their complete declarations in one bounded response. Prerequisites
are Python 3, the release's Rust toolchain, and this checkout; no model download,
credentials or additional Python packages are needed.

```sh
cargo build --release --locked -p qspquest-mcp
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --inspection-selection
```

The script creates its complete synthetic one-compartment model with 100 named
concentration outputs and a 6 mg bolus scenario in a temporary library. It selects
`cp_097` and `default`, checks their equivalence to paged declarations and the
separately retrieved scenario, and compares two identical executions with the
analytical concentration `2 * exp(-0.1 * time)` in mg/L. Expected output confirms
301 concentrations, three batched inspection resources and seven refusal cases.
This synthetic example establishes neither clinical qualification nor timing
savings.

For an MCP client, request:

```json
{"model_id":"synthetic_resource_example","selection":{"outputs":["cp_097"],"scenarios":["default"]}}
```

Read `model_selection.context`, the complete native declarations in `outputs`,
`output_mapping` (including the actual returned column and unit), and complete
stored native `scenarios[].request`. The request digest binds the stored JSON;
scenario inspection applies no overrides or precision profile. Save the context's
`inspection_hash`. Later calls can supply it as top-level
`expected_inspection_hash` to refuse changed archives, documents or aliases.

`selection.resources` batches exact advertised inspection index, page and native
record URIs from that same model and hash. Each result retains its `uri` and a
typed `value` (`index`, `page` or `record`) whose `data` equals the individual
resource. Page continuation and null-record notices still apply. Other models,
URLs, files, results and chunk URIs are refused.

Select 1 to 32 total items across outputs, scenarios and resources; names within
each group must be unique. The complete selection has a 40,000-byte budget.
Unknown outputs or scenarios fail the entire batch. For
`inspection_selection_budget`, select fewer items or retrieve a large scenario
with `get_scenario_request` and large records through their ordinary chunked
resources. Do not combine `selection` with `page`, top-level `outputs` or `detail`.
Metadata-only records and governed provider descriptors have no native selection;
inspect their discovery capability and use the declared governed workflow.

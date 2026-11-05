# Work with MCP results

These workflows run synthetic models through a local QSPQuest MCP server: read
complete results, inspect model cards and offline documentation, and prepare and
fit population records. The models are synthetic, so the checks show that numbers
are computed correctly, not that any model is clinically valid.

## Read a complete MCP simulation result

These workflows run synthetic models through the local MCP server and read the
full result when a reply contains only a preview. The models are synthetic: the
checks show that numbers are computed correctly, not that any model is
clinically valid. You need Python 3.10+ on macOS or Linux, a Rust toolchain and
the QSPQuest source distribution. From the distribution root:

```sh
cargo build --release --locked -p qspquest-mcp
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp
```

Each workflow below adds one flag to the same command. No downloads or accounts
are needed. Expected output is
`Verified 301 observations; final concentration 0.09957414 mg/L`. See the
[complete example](../examples/mcp_result_resources.py) and the
[standalone table reader](../examples/table_bundle.py).

The model has volume 3 L and clearance 0.3 L/h. A 6 mg bolus at time zero gives
concentration `2 * exp(-0.1 * time)` mg/L; the example checks every returned
point against it.

### Read the result contract

Read `structuredContent.contract` (schema `qspquest.mcp-result/v1`), not the
text reply. Its `status` is `success`, `scientific_failure`, `unsupported`,
`async_required` or `request_failure`. A nonconverged fit is
`scientific_failure` even when `isError` is false; treat its estimates as
diagnostic only. `async_required` means nothing has run yet: submit a job.

`completeness` is `inline_complete`, `resource_linked` or `summary_only`.
`resource_linked` lists the URIs that hold the full result; `summary_only` has
no full trajectory. `payload_sha256` is a checksum of the returned data. When
`delivery_receipt.inline_fields_omitted` is true, read every URI in
`contract.resources`. A local job provides
`delivery_receipt.local_job_lookup.job_id` for `get_local_job`.

`trajectory_kind` is `compact_preview`, `full_requested_grid` or
`summary_only`. When `full_result_resource` is present, read it with
`resources/read` in the same session before computing anything from the
trajectory; a preview can miss peaks and troughs. For a `utf8_chunk_manifest`,
join the chunks in order and check the manifest SHA-256 before parsing. Summary
AUCs and extrema are trapezoidal values on the returned grid, not a formal NCA.
Keep the session open until all reads finish.

To replay a result, call `simulate` with the same model and outputs and
`request: full["effective_request"]`. Top-level `parameters` or
`initial_states` override matching values in that request. Compare the returned
effective request and hash with what you intended.

The default run also checks refusals: malformed or unversioned request text, a
zero end, an empty observation plan and an incompatible threshold unit all
return `request_failure` with no result. To observe initial values, use a
positive end and include time zero.

It also starts at 30 mg (10 mg/L) and requests time above
`{"value":5000,"unit":"ug/L"}` through 10 hours. The threshold converts to
5 mg/L and the duration is `log(2) / 0.1 = 6.93147 h`, checked within
`0.0001 h`. Readouts use the returned grid and the output's own unit; an output
in unit `1` stays unitless whatever it is called. Native requests cannot combine
samples and sided observations.

Readouts need a known kind and output, and a positive window or exact time.
Each output must have one finite value per time. AUC includes the post-event
start and pre-event end of a window. `value` defaults to post-event; `trough`
uses pre-event. The same rules apply to `summarize_profile`. Timed parameter
changes accept only `name` (or `parameter`), `at` and `value`.

### Inspect a plan before execution

The example first calls `plan_run` with
`{"operation":"simulate","arguments":arguments}`. The contract has
`identity.scope: "metadata"` and no concentrations. `plan` lists bound inputs
and units, the event schedule, observation sides, outputs, storage, budgets and
the checks left for execution. The example then runs the same arguments with
`expected_plan_hash: planned["plan_hash"]`; the plan digest, request digest and
engine identity must match.

Leave `expected_plan_hash` unset while planning. A changed source, library
version, request, outputs or deployment is refused with `plan_mismatch` before
any numerical work; plan again. A plan does not reserve capacity.

Large local plans return `full_plan_resource` instead of `plan`; read it with
the chunk procedure above. Plans share the local cache (eight entries, 4 MB
each, 15-minute expiry). Hosted plans are inline or refused.

`plan_run` also accepts `scan_parameters`, `fit_scalar`, `fit_individual` and
`fit_population`. Fit plans show admission, converted observations, bounds and
starts, but no likelihood, estimates or convergence.

#### Require specific outputs

Use `required_outputs` with `simulate` or `compare_regimens` when a task needs
specific columns. A requirement names `output` and may name an exact declared
`analyte` and `compartment` from `get_model.interface.output_contract`:

```json
{"outputs":["mab_isf_nM"],"required_outputs":[{"output":"mab_isf_nM"}]}
```

Required columns must already be selected, and labels are never inferred from
names. Plans carry `required_output_receipts` with `selected_for_computation`;
results add `computed`, the native column, unit and sample count; CSV manifests
keep them. A missing column or incomplete grid fails and is never filled with
zeros. A refusal appears in `contract.failure.output_requirement` with
`fulfilled:false`.

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --required-outputs
```

Expected output is `Verified 301 analytical samples,
an explicitly computed zero column, four unfulfilled output requirements and
changed-plan refusal.`

#### Check times before integration

`plan.time_validation` lists each run window in the model time unit and how
many sample and forcing times were checked. Supplied samples, checkpoints,
resets, covariate changes, repeated doses and infusion ends are checked before
integration; nothing is clamped or extended. Invalid inputs return
`contract.failure.input_diagnostic` with the field, supplied and converted
values, the constraint and a small example. JSON `NaN`, `Infinity`, null or
`"NaN"` values are rejected with the field named.

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --time-validation
```

This runs from 2 to 4 h with 121 samples in minutes, checks
`2 * exp(-0.1 * (time - 2))` mg/L and seven refusals. Expected output is
`Verified an unchanged shifted
protocol, 121 analytical samples and seven field-level input refusals before
integration.`

### Inspect initial patient quantities

`get_patient_parameters` binds scalar covariates or parameter overrides and
evaluates selected outputs at time zero, before any dose:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --patient
```

The example plans with `operation: "get_patient_parameters"`, then executes
with `expected_plan_hash`; a changed override is refused. No solver, dose,
scheduled change or random draw runs. A 6 mg initial amount with a 6000 mL
volume override gives 1 mg/L. Expected output starts with
`Verified prepared initial values`.

The response has `patient_quantities`, `patient_units`, `time`, `time_unit`,
`effective_initial_values`, the effective request, identity and qualification
limits. `covariates` takes scalar inputs with units; omitting `outputs`
evaluates every output. Unit `1` means no physical unit is attached. An output
may simply repeat a stored value; check its expression before calling it
patient-specific. Large results fail with `patient_output_budget` unless the
server has `--artifact-dir PATH`.

### Inspect and execute a regimen comparison

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --comparison
```

The example plans named 3 mg and 6 mg boluses with
`operation: "compare_regimens"`, then executes with `expected_plan_hash`. The
plan lists each member's effective request, outputs, native preparation and
identity. `effective_source_sha256` hashes the ordered member source digests.
The example checks 64 event-sided values against
`amount / 3 * exp(-0.1 * time)` mg/L and refuses a changed later dose. Expected
output starts `Verified two prepared regimens, 64 event-sided
analytical samples`.

It also runs three conditions of an irreversible binding model (`A_bind` with
`kon = 0.5 [1/h]`, `B_zero` with `kon` zero, `C_double_start` with double the
free amount) and checks 54 values against `free = start * exp(-kon * time)` and
`bound = start - free`.

Each member accepts `parameters` and `initial_states` maps, merged over the
shared maps by name; a member `parameter_changes` array replaces the shared
one. Use one name per input. Stored literal initial values do not change when
parameters change; set `initial_states` explicitly when you need identical
starts.

Every member has an `input_receipt` with `requested_inputs`,
`requested_inputs_sha256`, `native_parameter_contract_sha256`,
`native_bound_value_hash` and `effective_request_hash`. Different hashes mean
different inputs, not necessarily different results.

Comparisons take 1 to 25 members with unique names of at most 120 UTF-8 bytes.
All members are prepared before any integrates, and all runs succeed before CSV
export starts. Export is not transactional: a later file error can leave
earlier bundles. Hosted comparisons share one 45-second deadline. Without
`--artifact-dir PATH`, an evicted member has no `full_result_resource`.

### Save and reimport an analysis bundle

The local server writes files only when started with a private `--output-dir`
(or `QSPQUEST_MCP_OUTPUT_DIR`):

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --csv-output /tmp/qspquest-table-example
python3 docs/examples/table_bundle.py /tmp/qspquest-table-example/study/trajectory.csv
```

Use a fresh directory each time. The first command writes
`study/trajectory.csv` and `study/trajectory.csv.manifest.json` and checks 301
rows and refusal to overwrite. The second validates and prints the manifest. In
Python:

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

The manifest (`qspquest.table-manifest/v1`, described by
`qspquest://schema/table-manifest`) records both file digests in `saved_csv`,
column units and native names, model and archive identity, engine identity,
the effective request, selected outputs and model source. `qualified` is false
and current-run status is `not_assessed`. Source-coordinate runs keep
`source_scale_unqualified`. Changed inputs or source are labeled modified.
The engine's `run_request_hash` identifies its native request.

The reader checks CSV bytes, headers, row counts, finite cells and
nondecreasing time; two rows at one time must be `pre` then `post`. Limits are
256 MiB of CSV, 16 MiB of manifest and 2,000,000 rows. Keep both files together
under their original name, and do not re-save the CSV from a spreadsheet: its
digest will no longer match. Digests detect corruption, not tampering with both
files.

Unit errors name the field and the requested and declared units. A field
declared in `1` uses dimensionless or source coordinates; supplying `1/h` to it
is rejected (see [units](../../spec/units.md#archive-source-coordinates)). Mass
and amount are never converted automatically: use the substance's molar mass,
then supply the declared unit.

### Plan and run synthetic Morris screening

This needs the optional `analysis` feature:

```sh
cargo build --release --locked -p qspquest-mcp --features analysis
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --morris
```

The example plans six points and two trajectories at seed 42, executes with the
plan hash, checks concentrations and elementary effects analytically, and
refuses a changed seed. Expected output starts with
`Verified six seeded Morris points`.

Call `plan_run` with `operation: "morris_screening"`, the `model` and a native
`qspquest.morris/v0.1` `request`. Inspect the `qspquest.morris-plan/v0.1`
preparation (scan, range units, output/time/side, point order and step
mapping), then pass `plan_hash` as `expected_plan_hash`. If the tool is missing,
rebuild with `analysis`.

### Plan and guard a periodic steady-state solve

Use the same analysis build:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --steady-state
```

The example plans with `operation: "steady_state"`, inspects the
`qspquest.periodic-plan/v0.1` preparation, then calls `steady_state` with
`expected_plan_hash`. It checks the identity and cycle residual and refuses a
changed tolerance. Expected output starts with `Verified periodic preparation`.
The plan shows the normalized infusion schedule, amount units and per-cycle
budgets but computes no fixed point. See the
[periodic preparation contract](../../spec/run-results.md#periodic-preparation).

Malformed analysis fields return an `error.expression` JSON pointer, such as
`/request/problem/request/ranges/1/upper/unit` or
`/request/relative_tolerance`, with a suggested fix.

### Retain complete results across local server restarts

Use a private directory when a result may exceed 4 MB or must survive the
server:

```sh
artifact_dir=$(mktemp -d)
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --artifacts "$artifact_dir"
```

On Windows, create an empty private directory and use
`target/release/qspquest-mcp.exe`. The script starts the server with
`--artifact-dir PATH`, requests 6,002 rows for 48 outputs, and checks that the
result exceeds 4 MB, survives nine more calculations and a restart, and keeps
its expiry. Expected output starts with `Verified`.

With this option, numerical results are written as immutable JSON artifacts.
The response has `full_result_resource` and a `result_artifact` descriptor
(`qspquest.result-artifact/v0.1`, schema `qspquest://schema/result-artifact`).
Read `chunk_uri_prefix` followed by each integer from zero to
`chunk_count - 1`, concatenate the UTF-8 text, and check `bytes` and `sha256`
before parsing. The example's `full_text` does this. Chunks are at most 48,003
bytes.

Artifacts expire 24 hours after creation; reading does not extend this. Limits:
128 MB per body; per directory, 256 live artifacts, 512 receipts and 1 GB.
After expiry, reads return `artifact_expired` for seven days, then
`artifact_not_found`. Use an empty directory owned by one server; unrelated
files, links and unsafe permissions are rejected. Local artifacts are not
available in HTTP mode. Without `--artifact-dir`, the bounded in-memory cache
applies.

### Verify dose amounts before interpreting concentrations

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --dose-events
```

The example submits 150 mg, 150000 ug and 0.15 g; each gives two 150 mg boluses
at 0 and 12 h and the same concentrations (3 L, 0.1/h, 50 mg/L after the first
dose). Pre-side samples at a dose time exclude that dose.

`plan_run` returns `dose_event_receipts` with stage `prepared_schedule`: target,
event index, time, delivery type, `amount_before_model_scaling`, state unit and
base scale. Execution records stage `executed_schedule`. The amount is applied
before `dosing.scale`, so it is not necessarily the state increment. A mass
concentration such as mg/L cannot stand in for a dose in mg. Archives that
declare a source dose coordinate add it to the receipt. Explicit unit `1` means
an amount already in model coordinates; prefer the declared dose unit. Large
replies may set `dose_event_receipt_notice`; read `full_result_resource`.

For `linear_pk` declarations:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --linear-pk-doses
```

This runs four workflows (depot, central with depot, central without depot and
two 2 h infusions) with 3 L, 0.3 L/h and absorption 1.2/h. Two 6000 ug doses
with availability 0.5, a 1 h lag and a 6 h interval give receipts of 3 mg at 1
and 7 h. The infusion model uses `central [mg] = dose_scale` with
`dose_scale` 2, so each infusion delivers 6 mg. Default linear PK dosing uses
scale 1 and the declared `amount_unit`.

### Inspect large models in bounded pages

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --model-pages
```

The example builds a small library, reads its index, walks one record per page
and checks each record resource. No simulation runs.

For a library model, call `get_model` with `model_id` and `version`. The card
returns `inspection` and `inspection_index_resource`, which gives record counts
and first pages for `scalars`, `states`, `outputs` and `equations`.
`archive_sha256` (or `record_sha256` for standalone sources) binds the content.
Check `source_interpretation_summary` too: a `declared` status links a
`complete_declaration_resource` whose `metadata.source_interpretations` records
encoding choices. These are not a check that the model matches the paper, and
`not_declared` does not mean the model follows the source literally.

Request a page:

```json
{"model_id":"synthetic_resource_example","page":{"section":"outputs","offset":0,"limit":32,"expected_inspection_hash":"COPY_THE_INDEX_INSPECTION_HASH"}}
```

Limits run from 1 to 128 (default 32). Follow `next_offset` or
`next_page_resource` until null; a page can return fewer records than asked.
`offset == record_count` returns an empty terminal page.
Each entry has `native_path`, its canonical name, aliases, the native record and
`record_resource`. To read one known output, use `get_model.outputs` instead.
An oversized record is null inline and complete at its `record_resource`. When
the archive, document, aliases or scenarios change, old hashes and resources
are refused; read the current index.

### Bind inspected inputs to a request

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --input-bindings
```

The example reads `cl = 0.3 L/h` and `v = 3 L`, supplies `300 mL/h` and a
`6000 ug` initial amount, checks `2 * exp(-0.1 * t)` mg/L at 0 and 1 h, then
changes the volume and checks the refusal guidance.

Copy `inspection.context.inspection_hash` from `get_model` into
`model.library.expected_inspection_hash`:

```json
{"model":{"library":{"model_id":"synthetic_resource_example","expected_inspection_hash":"COPY_THE_INDEX_INSPECTION_HASH"}},"parameters":{"cl":{"value":300,"unit":"mL/h"}},"initial_states":{"central":{"value":6000,"unit":"ug"}},"end":{"value":1,"unit":"h"},"times":[{"value":0,"unit":"h"},{"value":1,"unit":"h"}],"outputs":["cp"]}
```

Add `version` for a versioned archive. A stored scenario uses `model.scenario`
with `model_id`, `version`, `scenario_id` and `expected_inspection_hash`.
`get_model`, `get_scenario_request` and `get_capability.model` accept the same
guard. Inline `model.source` has no inspection context; bind it with the plan
hash. State initial expressions can depend on parameters: use
`get_patient_parameters` to see the effective start. A source label such as nM
does not allow nM input into a state declared in unit `1`.

When the model or its bindings change, the guarded call fails with
`inspection_changed`; `contract.failure.inspection_change` gives old and current
hashes, the current `inspection_index_resource` and
`migration: "inspect_current_bindings"`. An unknown input name returns
`input_alias_unknown` with `contract.failure.input_binding` (inline sources use
`migration: "inspect_submitted_model"`). A complete `qspquest.run/v0.1` request
needs canonical native names; an alias there returns
`input_name_requires_native` with the canonical name and
`numerical_execution_started: false`. No name is guessed. A missing version
returns a lookup error.

### Inspect pre-dose initialization and percentage denominators

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --initialization
```

A target starts at zero at −20 h, is made at 2 mg/h and cleared at 1/h; a
marker dose at 0 h sets the clock. The example compares the baseline, clearance
2/h throughout, 2/h from −10 h and 2/h from 0 h against
`2/k * (1 - exp(-k * (time + 20)))` mg (48 values). Expected output begins
`Verified four protocol conditions, 48 analytical
values`.

`plan.initialization_receipt` (stage `prepared`, then `executed`) gives the
clock, initialized states and binding phases. `predose_integration` covers
−20 to 0 h and claims no equilibrium. Starting overrides apply from the
integration start, and a timed change activates `post_boundary` at its time. A
changed parameter does not recompute a separately declared initial state.

`fixed_pct` divides by a declared `reference` input
(`declared_reference_input`); `condition_pct` divides by
`condition_reference = synthesis / clearance`
(`condition_specific_binding`). Neither rebaselines at 0 h. The receipt lists
each denominator; a nonpositive one is refused at planning. Archives may
declare a `protocol_initialization` descriptor
(`qspquest.protocol-initialization/v1`), visible through `get_model`.

### Select a complete precision profile

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --precision-profiles
```

The script builds an archive with baseline clearance `0.3 L/h`, alternative
`0.6 L/h` and volume `3 L`, and checks `2 * exp(-0.2 * time)` mg/L at 13 times.
Expected output is `Verified complete profile discovery/selection; 13 analytical
values; native plan/result and CSV receipts; changed plan refused; modified
profile labelled; two separately computed comparison profiles.`

Inspect `get_model.precision_profiles` for profile IDs, the archive default and
the profile-file SHA-256; `full_model_details_resource` holds the complete
`precision_profile_manifest`. Select an ID in `get_scenario_request` and in the
simulation's scenario:

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

A profile replaces every mutable starting parameter and keeps states, doses and
protocol. Further edits mark the `precision_profile_receipt` as `modified`;
changing a fixed input is refused with `precision_profile_immutable`. Receipts
appear in the scenario request, plan, result and CSV. In `compare_regimens`,
each member may set its own `precision_profile_id` (with a shared
`model.scenario`).

### Inspect scheduled boundaries and observation sides

This shows when inputs change, doses apply, infusions stop and checkpoints are
visited, and why a predicate read at an observation can differ from the input
used over the preceding interval.

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --scheduled-boundaries
```

Expect:

```text
Verified four predicate conventions, 144 analytical values, native schedule receipts, CSV sides and 24 changed-receipt refusals
```

The model gets a 10 mg bolus at 0 h, 5 mg at 2 h and a 1 mg infusion from 3 to
4 h; a fixed switch changes clearance from 1/h to 2/h at 2 h. The script uses
`<`, `<=`, `>` and `>=` predicates, `pre` and `post` at every observation and a
checkpoint at 4.5 h. For `<`: `amount(2 h, pre) = 10 * exp(-2) mg` and
`amount(2 h, post) = 10 * exp(-2) + 5 mg`; during the infusion the added amount
after time `d` is `(1 - exp(-2*d))/2 mg`. The `predicate` output uses the exact
time: `<` is false at 2 h on both sides, `<=` is true. Side controls state
updates, not the comparison.

`plan.scheduled_boundary_receipt` (stage `prepared`, then `executed`) lists:

- Fixed switches with their time expressions, thresholds (in `time_unit`) and
  parameter bindings.
- Dose-relative switch declarations: target, offset, comparison and
  simultaneous-dose policy.
- Ordered boundary rows with checkpoint flags, phase indexes, switch names and
  event references.
- Observations with their driver rows and exact-time predicate overrides.
  Phase indexes refer to `native_preparation.plan.binding_phases`.
- Solver, tolerances and `achieved_error_estimate`, which is `null`.

If the receipt is too large, `scheduled_boundary_receipt_notice` says so; read
the full result resource. Always state the side when reporting a value at a
dose time.

### Inspect separate scientific assessments

This separates numerical parity from source fidelity, conservation, author-code
parity, clinical validation and physical dimensions.

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --scientific-assessments
```

Two states start at 1 mg and 0 mg; central loses at twice the transfer rate
while peripheral gains at the rate, so `central = exp(-2t)` mg and
`peripheral = (1 - exp(-2t))/2` mg. The numbers are right, but the total
falls: numerical agreement coexists with a conservation finding. Expected
output begins:

```text
Verified separate scientific axes, 12 analytical values, archived conservation finding, modified-request scope, 16 CSV tamper refusals and changed-declaration freshness.
```

`get_model.scientific_assessment` is null when nothing is declared. Each axis is
`not_assessed`, `passed`, `failed` or `discrepancy_observed`, with a scope;
assessed axes carry an evidence digest and method. There is no general
`validated` status. `plan_run.plan.qualification.scientific_assessment` says
whether the source and request match the archive;
`current_engine_and_request` stays `not_assessed`. Modified inputs need their
own evidence. Libraries declare assessments with
`scientific_assessments: {path, sha256}` in a catalog row (at most 24 KB).

### Format complete decision responses

`format_response` maps actual returned columns from several computed
conditions into requested JSON keys, with units, times and a required
`event_side` (`pre`, `post` or `both`). It never interpolates, converts units,
solves or guesses; a missing side or time needs a new simulation.

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --response-guard
```

The example computes one 6 mg bolus, two 6 mg boluses and two 12 mg boluses
(0 h and 2 h; 3 L; 0.3 L/h), with artifact and cache storage. It maps
`library_state_0` into nine arrays and refuses nineteen bad inputs. Expected
output begins:

```text
Verified response guard: 18 analytical observations, three computed conditions, nine explicit column/key/side mappings
```

Arguments for one post-dose array:

```json
{
  "runs": [{"label": "A", "source": {"kind": "retained", "uri": "qspquest://runs/RESULT_ID"}}],
  "fields": [{"key": "A_concentration_mg_L", "run": "A", "column": "library_state_0",
              "unit": "mg/L", "time_unit": "h", "times": [0, 2, 4], "event_side": "post"}],
  "identifiers": {"decision_id": "dose-side-comparison"}
}
```

Replace `RESULT_ID` with the run identifier, or use the artifact URI from
`full_result_resource`. Add `expected_plan_hash` and
`expected_effective_request_hash` when available so a swapped source fails.

The result has `formatted_response` and `response_guard`. Check the guard's
identities, mappings and sides before copying `formatted_response`. To check an
existing answer, pass it as `candidate`; `candidate_matches` only confirms the
values were copied exactly. Use the column from `output_mapping`.
`expected_analyte` and `expected_compartment` need declared metadata. Previews,
jobs and nonconverged sources are refused.

Local references work only in the owning server. Hosted servers take
`{"kind":"supplied","json":"EXACT_COMPLETE_JSON","sha256":"LOWERCASE_HEX_DIGEST"}`;
the digest checks bytes, not the producer. Limits: 32 runs, 64 array fields,
32 identifier fields, 4096 times per field and 128 MB of source bodies.

### Select exact outputs, scenarios and inspection resources

Use this when you already know the output name and scenario ID:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --inspection-selection
```

The script builds a model with 100 outputs and a 6 mg bolus scenario, selects
`cp_097` and `default`, and checks 301 concentrations, three batched resources
and seven refusals. Request:

```json
{"model_id":"synthetic_resource_example","selection":{"outputs":["cp_097"],"scenarios":["default"]}}
```

Read `model_selection.context`, the declarations in `outputs`, `output_mapping`
and the stored `scenarios[].request`. Save the context's `inspection_hash` and
pass it later as `expected_inspection_hash`. `selection.resources` batches
inspection index, page and record URIs from the same model; each result keeps
its `uri` and a typed `value` (`index`, `page` or `record`).

Select 1 to 32 items in total within a 40,000-byte budget. For
`inspection_selection_budget`, select fewer items or use
`get_scenario_request` and the chunked record resources. Do not combine
`selection` with `page`, top-level `outputs` or `detail`.

## Inspect model cards and retrieve full details

Use this workflow to get a short summary of a model (its card), and fetch the
full equations and metadata only when you need them. Python 3.10 or later on
macOS or Linux, the Rust toolchain and a native compiler are required. Run from
the QSPQuest source distribution root:

```sh
cargo build --release -p qspquest-mcp --locked
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --model-cards
```

The complete example creates a temporary synthetic `.qsp` model and a stored
request, starts the server with that local library, and removes them on exit.
No downloads or accounts are needed. Expected output is:

```text
Verified compact model card, complete metadata and stored scenario.
```

The example checks the card, fetches the full metadata and checks it matches
the card, and compares the stored scenario with the original request.

In an MCP client, call `get_model` with `{"model_id":"your_model_id"}`. The
default `detail: compact` response gives the model's ID and version, citations,
whether it can run, time unit, dose targets, parameter names/defaults/units,
outputs, state units, fitting support and scenario IDs. It omits equations and
source text. Lists of reported values show at most 25 names, with a count of the
full list. Check any listed blockers and unit limitations before using units.

`interface.output_contract` lists each output's ID, returned column, unit and
accepted alternative names. The unit used in the original paper is kept separate
from the unit QSPQuest returns: an output labeled `pg/mL` in the source can still
return unit `1`, and no conversion is applied. Analyte and compartment are null
unless the model states them; do not guess them from names.

Compact cards include at most 32 output descriptors. Check `output_count` and
`descriptors_complete`. To inspect particular outputs, pass `outputs` with 1 to
32 exact aliases or canonical IDs, for example
`{"model_id":"your_model_id","outputs":["your_output_name"]}`. Unknown or
ambiguous names fail with an error. The full model details always include
every output.

`plan_run` records `plan.output_mapping` for simulation and initial-value
requests and parameter scans. `simulate` and `scan_parameters` return the same
mapping beside their numerical results,
including readout selections and default outputs. Match each `requested_output`
to `returned_column` in the complete result's `outputs` array and check
`returned_unit`; you do not need to rely on column order. For initial values,
`get_patient_parameters.output_mapping[].returned_quantity` identifies the key
in `patient_quantities`. Using a different alias changes the plan hash, even
when the calculation is the same. Large mappings may be left out of the response;
`output_mapping_notice` then gives the count, and `full_result_resource` links
the full record.

Run the complete synthetic example to verify planning, mapping and every
returned concentration against its analytical value:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp
```

Expected output includes `Verified 301 observations; final concentration
0.09957414 mg/L`. The example checks column names and units before consuming
values. It needs the same prerequisites as the card example above.

To verify a parameter scan, run:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --scan
```

The complete synthetic example scans clearance at 0.3 and 0.6 L/h with a 3 L
volume and 6 mg bolus. It checks both returned columns through `output_mapping`
and all 602 concentrations against `2 * exp(-clearance / 3 * time)` mg/L.
Expected output is `Verified two parameter-scan trajectories, mapped
concentration columns and 602 analytical samples.`

Before changing a parameter, check `plan.override_receipt`. For each parameter
or covariate you set, it shows the name you used, the model's own name, the
value, and which equations and outputs use it. A parameter that appears in an
equation may still have little or no measurable effect.

`consumption` distinguishes `dynamic`, `initialization_only`,
`static_output_only` and `unused`. Initial expressions replaced by an explicit
`initial_states` quantity are listed separately in `replaced_initial_states`.
Setting an unused parameter gives a warning in the plan and result. A
static output changes a reported value without changing the simulation. Fixed
initial conditions and coefficients stay fixed unless you override them
directly; changing an upstream input does not recompute them. Initial values
written as expressions are evaluated by the engine with your inputs. Check
`source_reconstruction` before using a scaling input to represent a different
physiology or species.

Receipts are part of the plan hash and are saved with full results for
simulations, patient initial values and parameter scans. Scan receipts list the
values at each scan point. Scenario defaults appear only if you override them. A
large receipt may be left out of the response; `override_receipt_notice` gives
its entry count and `full_result_resource` links the full receipt.

Run this complete synthetic illustration with the same prerequisites:

```sh
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --overrides
```

It increases clearance from 0.3 to 0.6 L/h and also changes an unused source
scalar. It verifies 301 concentrations against `2 * exp(-0.2 * time)` mg/L,
checks the receipt and warning, confirms that another change to the unused
scalar leaves trajectories identical, and checks that a changed override is
refused when it carries the earlier plan hash. Expected output is `Verified
active and unused override receipts, unchanged inactive trajectories, 301
analytical samples and changed-input refusal.`

To inspect everything, call `get_model` with
`{"model_id":"your_model_id","version":"the_card_version","detail":"full"}`.
Small records return inline. Larger records return `detail: full_resource`, a
notice and `full_model_details_resource`. This resource link is also present on
compact cards. Read it with `resources/read`. If the returned JSON has
`kind: utf8_chunk_manifest`, read every listed chunk in order, concatenate the
UTF-8 text and verify its SHA-256 against `sha256` before parsing it. Complete
records include native parameters, states, outputs, equations, metadata and
source text or a separate source resource link.

Fetch a stored request separately with `get_scenario_request`, supplying the
same `model_id`, `version` and a `scenario_id` from the card. The returned
`request` is the stored native request; `readable_request` uses the interface
names. If you change a stored scenario, the model's original checks no longer
apply to it. For a large stored request, read
`qspquest://models/{model_id}/scenarios/{scenario_id}` and follow a chunk manifest
if returned.

A missing executable means the build failed or the path is wrong. Invalid
`detail` values are rejected; use only `compact` or `full`. A missing model or
version means it is not in the loaded library: check the current card or load a
matching library version. A checksum mismatch means a read was incomplete or the
library changed; fetch a fresh card and read the resource again.

### Inspect execution and declared source families

Run the complete synthetic family example from the same source distribution:

```sh
cargo build --release -p qspquest-mcp --locked
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --source-families
```

The example creates two native model archives and one metadata-only record for
one synthetic source. It verifies search, exact model/version selection, compact
cards and the digest of the complete family resource. Expected output starts
with `Verified full, reduced and metadata-only records`. All inputs are created
locally and removed on exit.

Every public-library search row has `execution_available`, `scenario_count` and
a `discovery` record. `discovery.executable_backend` says how the model runs,
or is null for a metadata-only record, which cannot run. A runnable model has
`execution_assessment: requires_request_validation`, meaning your specific
request is still checked when you run it; a record that cannot run has
`unavailable`. User-supplied models have `archive_sha256` null.

For exact selection, call `search_models` with `model_id` and optionally `version`.
For example, the synthetic family uses
`{"model_id":"family_full","version":"1"}`. `query` defaults to an empty string;
a supplied text query applies alongside the exact filters. Missing identifiers
or versions return an empty list. This lookup supplies the archive digest,
backend and scenario count without retrieving a full model card.

A catalog can explicitly declare `model_fidelity` as `full`, `reduced`,
`synthetic` or `unrecorded`, and a `source_family` containing `family_id`,
`paper_id` and `canonical_model: {model_id, version}`. All family members must
agree on these identifiers. The canonical model must be an executable, declared
full model at that exact version in the same loaded catalog. Invalid or ambiguous
declarations prevent catalog loading. No relationship or fidelity is inferred
from a title, model identifier or shared keyword. Catalogs without declarations
report `source_family.status: unrecorded` and `fidelity: unrecorded`.

Search and model cards give up to eight family members, the complete member
count and `members_complete`. Read `full_family_resource` for the complete list,
which has at most 32 model versions. The final URI component is the SHA-256 of
the complete UTF-8 JSON body. Follow a chunk manifest if returned, then verify
that digest. `qspquest://schema/model-discovery` and
`qspquest://schema/source-family` describe these records. A family URI stops
working if the family's membership changes. Use `get_capability` to check what a
specific model and scenario support.
Authorized platform results in `platform_models` carry the same closed
`discovery` fields. Their `representation` is `governed_catalog`. A boolean
`execution_available` preserves an explicit provider declaration; null means
unassessed. A provider `status: ready` does not establish execution availability.
An unassessed or declared available record has
`execution_assessment: requires_governed_preview`: preview the intended Job,
review its price and approval requirements, and submit only that reviewed request.
A declared false value is `unavailable`. A declared available record identifies
`governed_job` as its backend; no private native projection is inferred.

An unrecorded provider version or scenario count is null. Exact `model_id` and
`version` filters apply to both result arrays. A version filter cannot match an
unrecorded version. `get_model` checks the identifier and, if supplied, the exact
version against the authorized active record; it does not retrieve private
historical versions. Provider family relationships and archive digests remain
unrecorded in the MCP projection. Additional provider metadata retains its
provider schema and does not establish qualification. The local `total` and
pagination fields describe `models`; `platform_models` previews at most 20
matching authorized records, not a complete private inventory.
Check `platform_error` before interpreting a combined search as a successful
private-catalog lookup.

Run the complete synthetic governed discovery and preview fixture from the
released source root. It requires the supported Rust toolchain, locked build
dependencies and permission to bind a loopback test socket:

```sh
cargo test --locked -p qspquest-mcp --features managed --test managed governed_preview_digest_idempotency_session_and_free_boundary -- --nocapture
```

The fixture supplies a ready-status record without execution evidence, an
explicitly available version `1` with three declared scenarios, an unavailable
record and a tier-locked record, with a synthetic 42-credit price preview.
Expect the synthetic price line and one passing test. Everything runs locally;
no account or real credits are needed. Malformed platform records produce a
platform-response error.

### Navigate offline documentation

Use the same prerequisites as above. The documentation is built into the
server, so reading it needs no network. Run:

```sh
cargo build --release -p qspquest-mcp --locked
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --documentation
```

The example starts a local server with a temporary synthetic library. It reads
`qspquest://documentation`, checks the package version, fetches every indexed
resource, verifies each content SHA-256, and follows its embedded resource
links. The index's topics lead to model, request, unit and error guidance.
`qspquest://schema/documentation` supplies the index's structural schema.
The index also links to `qspquest://capability-catalog` and its schema at
`qspquest://schema/capability-catalog`. The catalog lists what each interface
supports (for example native fitting versus Python analysis). For what this
particular server supports, use the capability resources below.
`qspquest://capabilities` supplies the actual `target`, registry URI and
`capability_query_tool`. Read `qspquest://capability-registry` for this MCP
target and deployment. The example then calls `get_capability` with:

```json
{
  "key": {
    "operation": "simulate",
    "interface": "mcp",
    "estimator": "none",
    "solver": "diffsol_bdf",
    "target": "aarch64-apple-darwin",
    "execution_mode": "local_stdio"
  },
  "model": {"model_id": "synthetic_resource_example"},
  "scenario_id": "default"
}
```

Use the target reported by your server in place of the illustrated macOS ARM64
target. Supply the card's `version` under `model` to select an exact archive.
The synthetic workflow reports `declared: true`, `active_exposed: true`,
`assessment: requires_request_validation`, `executable: null` and
`qualified: false`. `executable: null` means the query cannot tell without
running your exact request; it does not run anything. A missing model, an
unavailable feature, an incompatible solver or a fitting workflow that is not
ready gives a list of blockers and `executable: false`. Hosted operations need a
signed-in session; preview their price before submitting.

`qspquest://schema/capability-registry` and the tool's output schema describe
the response.
For oversized resources, read the chunk manifest, concatenate chunks and
verify its digest before interpreting the document.

It then reads the complete synthetic model and request from resource URIs and
calls `simulate` with `model.source` and `request`. The 6 mg bolus, 3 L volume
and 0.3 L/h clearance imply `cp(t) = 2 * exp(-0.1*t)` mg/L. The example checks
six post-dose samples over 12 h and reports the number of resources and links
verified.

A heading not present in the installed version returns an invalid-resource
error. Read a fresh index before following links from another version. A digest
mismatch indicates incomplete retrieval or changed resource content; fetch the
complete current resource again. Running user-supplied models on a hosted
server needs a sandbox, as reported by `qspquest://capabilities`.

### Export a complete synthetic trajectory

Use the same prerequisites and source-root working directory as above. Choose a
new output directory; it is created with private permissions by the server:

```sh
cargo build --release -p qspquest-mcp --locked
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --csv-output "$PWD/qspquest-example-results"
```

The complete example supplies the one-compartment model, a 6 mg bolus, clearance
0.3 L/h, volume 3 L and 301 observations over 30 hours. No model download is
required. It starts local stdio with `--output-dir`, requests
`save_csv: "study/trajectory.csv"`, and checks every exported concentration
against `2 * exp(-0.1 * time_h)` mg/L. It also requests the same filename again
and verifies refusal without changing the first file. Expected output includes:

```text
Verified 301 CSV rows and preserved existing output: /absolute/path/qspquest-example-results/study/trajectory.csv
Verified 301 observations; final concentration 0.09957414 mg/L
```

Open `study/trajectory.csv` beneath the selected directory. Its columns are
`time_h,side,cp`; this example requests post-dose observations. Other regimens
can include both pre-dose and post-dose rows at the same time. The example
keeps the CSV and deletes its temporary library when finished.

CSV export is disabled unless a local server has `--output-dir PATH` or
`QSPQUEST_MCP_OUTPUT_DIR`. Unix output directories must be owned by that server
user with mode `0700`; Windows uses owner and ACL checks. File and directory
links, traversal and existing output files are refused. Choose another filename
or a new directory to rerun this example. Hosted servers refuse CSV export.
Regimen comparisons apply the same rules and append each sanitized regimen name
to the CSV filename; completed files remain if a later regimen fails. The CSV
holds only the trajectory, so keep the model card and request alongside it.

## Prepare and fit explicit population records

This tutorial shows how dataset records (NONMEM-style rows) become a population
fitting request, what an input error looks like, and how to run one small
synthetic FOCEI fit. You need Python 3.10 or later, a Rust toolchain and the
QSPQuest source distribution. Run these commands from the distribution root:

```sh
cargo build --release --locked -p qspquest-mcp
python3 docs/examples/mcp_population_records.py target/release/qspquest-mcp \
  --out population-record-results
```

The [complete client](../examples/mcp_population_records.py) uses the shipped
[synthetic input](../../bindings/mcp/assets/fits/population.json) and the
[standard-library transport](../examples/stdio_rpc.py). Choose a new output
directory with an existing parent. The client first sends a deliberately broken
endpoint table to `plan_run` to show the error, then previews the real records
and calls `fit_population` with that plan's hash. It saves the inputs, the error,
the plan, the full result and checks. Keep its `artifacts/` directory private.

Expected checks report four subjects, 24 included observations, an error
pointing at `mapping.endpoints`, matching plan hashes, convergence, and
clearance within 0.06 L/h of the true 0.6 L/h. A failed fit exits with a nonzero
status and keeps its diagnostics. This is a teaching example, not evidence
of clinical validity.

The fixture declares time in hours, dosing amounts in mg, and the concentration
observable `cp` in mg/L. The explicit mapping binds CMT `1` to native dosing
state `central` and CMT `2` to `cp`. Each observation declares its additive and
proportional Gaussian error. The population template supplies sensitivities,
bounds, fixed Omega, method limits and seed. Units and model structure are never
guessed from column names.

Record admission supports EVID 0/1, MDV 0/1 and SS 0. A dose needs positive AMT;
positive RATE needs an explicit `mapping.rate_unit`. ADDL is an integer from 0
through 10000 and needs a positive II when nonzero. Observation records must not
carry nonzero AMT, RATE, ADDL or II. Included observations need finite DV. An
excluded observation is ignored. Text and integer subject IDs are treated as
different. Row order is kept and recorded in `row_mapping`; rows are never
sorted. Hosted use allows at most 16 subjects and 256 included observations;
local use allows 128 subjects and 10000. Exceeding a limit gives a `budget`
error; subjects and rows are never dropped to fit.

The mapping object accepts only `time_unit`, `amount_unit`, `rate_unit`,
`observation_side`, `compartments` and `endpoints`. Each endpoint accepts only
`output`, `value_unit` and the native `error` object. Observation side defaults
to `post`; an explicit value must be `pre` or `post`. The method defaults to
`focei`; `laplace` and `saem` can be chosen explicitly. A missing or null optional
value uses its default. Misspelled fields, wrong shapes and unsupported values
are rejected with an error. Extra record columns are allowed and ignored.

Read `structuredContent.error.input_diagnostic.field`, `supplied_value` and
`expected` to find the problem. Long values are shortened. Correct the named
field, call `plan_run` again, and submit with the new `expected_plan_hash`.

The engine still checks units, output names, random effects and limits when
it fits, and FOCEI, Laplace and SAEM each have their own restrictions. Check the
objective kind, bounds, estimates, subject diagnostics and any uncertainty. Do
not compare a complete-data surrogate with a marginal likelihood, or report
non-converged estimates as final. You can also skip record mapping and pass a
complete `fit_request` directly.

The mapping compatibility fixtures can be checked separately:

```sh
cargo test --locked -p qspquest-mcp core::population_records::tests
```

These tests cover record mapping and input errors only; they do not run a fit.

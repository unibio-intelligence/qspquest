# QSPQuest MCP server

The server lets a Model Context Protocol (MCP) client search, inspect, simulate and fit QSPQuest models
over local stdio or stateless Streamable HTTP. Start with one of these:

- [Agent and Docker quickstart](../../docs/tutorials/agents-docker.md): a
  container with a synthetic model, connected to your own agent client.
- [First-run journeys](../../docs/tutorials/first-run-journeys.md): first runs
  with the wheel, command-line interface (CLI) or this image, with saved CSV, replay inputs and identity.
- [Library fitting guide](../../docs/tutorials/library-fitting.md): archived
  synthetic fitting; inspect readiness before choosing an estimator.

Models load from the public model library, from a local directory or a pinned
HTTPS release. Many library models keep their source's numeric scale, so check
a model's units before interpreting results. The bundled Open Modeling EXchange
(OMEX) models are
offline numerical test fixtures: their units are unverified
(`source_scale_unqualified`) and they have not been scientifically reviewed.

## Build and connect

You need Rust 1.98.1, a native compiler and the
[public model library](https://github.com/unibio-intelligence/qspquest-models).
From the QSPQuest source root:

```sh
cargo build -p qspquest-mcp --release --locked
```

This stdio build needs no WebAssembly (WASM) toolchain. The build helper selects optional
features:

```sh
sh scripts/build-mcp.sh --features ""
sh scripts/build-mcp.sh --features analysis
# Browser assets require wasm-bindgen-cli 0.2.129 on PATH.
sh scripts/build-mcp.sh --features sandbox,widget
```

`analysis` adds the `steady_state`, `morris_screening` and `analyze` tools and
the population block of `compare_regimens`. `sandbox` builds the isolated
WASM application binary interface (ABI) and `widget` the browser bindings; both need the wasm32 target and are
the helper's defaults. `managed` needs its deployment configuration.

The container image includes `sandbox,widget` by default. For a native image
without them, build with `docker build -f bindings/mcp/Dockerfile --target public
--build-arg MCP_FEATURES= -t qspquest-mcp:native .`

### Use a local library

Extract the model-library release so its root contains `catalog.json` and
`models/`, then pass an absolute path:

```sh
export QSPQUEST_LIBRARY_DIR="$(cd /path/to/qspquest-models && pwd)"
target/release/qspquest-mcp --library-dir "$QSPQUEST_LIBRARY_DIR"
```

The process waits for a client on stdio; exit with Ctrl-C. The library is
checksum-verified on load. A corrupt catalog or availability registry prevents
startup. An archive that fails one of its own checks is withheld and every
other model is served (see [Archive holds](#archive-holds)). Directories of
`.qsp` source models also work.

Claude Desktop configuration (replace both absolute paths):

```json
{"mcpServers":{"qspquest":{"command":"/absolute/path/qspquest/target/release/qspquest-mcp","args":["--library-dir","/absolute/path/qspquest-models"]}}}
```

### Download a pinned library release

Use an immutable HTTPS release archive and the SHA-256 published separately by
its trusted publisher:

```sh
# Set URL and SHA256 to the release asset and its independently verified digest.
export QSPQUEST_LIBRARY_RELEASE_URL="$URL"
export QSPQUEST_LIBRARY_RELEASE_SHA256="$SHA256"
unset QSPQUEST_LIBRARY_DIR
target/release/qspquest-mcp
```

The archive must be a gzip tar with `catalog.json` at its root. The server
verifies it before serving and caches it in `~/Library/Caches/qspquest-models`
(macOS), `$XDG_CACHE_HOME/qspquest-models` or `~/.cache/qspquest-models`
(Linux), or `%LOCALAPPDATA%\qspquest-models` (Windows). Set
`QSPQUEST_LIBRARY_CACHE` to use another private directory. The cache root must
be owned by the server user (mode `0700` on Unix); shared, linked or unsafe
roots are refused.

Every start re-checks the cached archive and extracted library against the
pinned digest, so later starts work offline. A failed refresh keeps the
current library; to upgrade, change both URL and digest. Older cache entries
without a saved tarball are not trusted: choose a new cache directory. A local
`--library-dir` overrides the download.

### Search

Call `search_models` with `{"query":"rituximab"}`, then `get_model`,
`get_scenario_request` and `simulate` with the returned model and scenario.
Check the model card, units, result status and event sides. Search ranks
models by matching terms and can return models not yet cleared for simulation
or fitting. `filters` accepts exact `model_id`, `version`, `license` and
`has_output` (a native output name or alias); `tag` is a case-insensitive
substring. Results report `total` and `next_offset`.

### Copy and extend a model

`fork_model` copies a library model into readable `.qsp` text with legal
identifiers, a `name_map` to the native symbols and, with `scenario_id`, a
translated request that is checked against the original
(`equivalence.status`). `edit_model` then adds mechanisms by typed operations:
parameters, states, starting values, terms, fluxes, a peripheral compartment,
covariates and covariate effects, a dosing input, outputs and renames. The result is
recompiled before it is returned. `get_model` with `source` (or the `model` reference that the other tools take) returns
the same card for inline text as for a library model. See
[Extend a library model and select a regimen](../../docs/tutorials/extend-library-model.md).

## Tools and resources

Tools: `search_models`, `get_model`, `get_scenario_request`, `validate_model`,
`model_template`, `fork_model`, `edit_model`, `find_dose`, `sensitivity`, `simulate_steady_state`, `get_capability`, `plan_run`, `format_response`,
`simulate`, `get_patient_parameters`, `summarize_profile`,
`compare_regimens`, `scan_parameters`,
`fit_scalar`, `fit_individual`, `fit_population`; with the `analysis` feature,
also `steady_state`, `morris_screening` and `analyze`. For long offline work, start
with private `--jobs-dir` and `--artifact-dir` directories and use
`submit_local_job`, `get_local_job`, `cancel_local_job` and `resume_local_job`
(see the [local job tutorial](../../docs/tutorials/mcp-deploy.md#offline-mcp-jobs)).

Read `qspquest://capabilities` for active tools, features, permissions, limits,
library identity and qualification scope. Each tool advertises an
`outputSchema`; `qspquest://schema/mcp-result` and
`qspquest://schema/capabilities` describe the shared schemas. Tool annotations
are hints; execution and export policies still apply.

`qspquest://documentation` indexes the model, request, regimen, unit and error
guides, the schemas and a synthetic model/request pair. Each entry has a MIME
type and SHA-256, and heading fragments select sections. The
[offline resource example](../../docs/tutorials/mcp-results.md#navigate-offline-documentation)
checks every indexed resource through a real client.

`get_model` returns a compact card; `detail: full` adds equations and complete
metadata, with a chunked `full_model_details_resource` when large. See the
[model-card tutorial](../../docs/tutorials/mcp-results.md#inspect-model-cards-and-retrieve-full-details). For known
output names and scenario IDs, `get_model.selection` returns declarations,
column mappings and scenario requests in one response (1 to 32 items, 40,000
bytes); see the
[selection example](../../docs/tutorials/mcp-results.md#select-exact-outputs-scenarios-and-inspection-resources).

## Results

Every tool result includes `structuredContent.contract`
(`qspquest.mcp-result/v1`) with identities, status and completeness. Status is
`success`, `scientific_failure`, `unsupported`, `async_required` or
`request_failure`; a nonconverged fit is `scientific_failure` even if `isError`
is false. Completeness is `inline_complete`, `resource_linked` or
`summary_only`. A schema-valid result still needs its units, status and
convergence checked. The runnable
[result-resource tutorial](../../docs/tutorials/mcp-results.md#read-a-complete-mcp-simulation-result)
covers each contract.

Compact simulation results are previews with event sides, sampled extrema and
a grid trapezoidal integral, not an extrapolated area under the concentration–time curve (AUC) or a
noncompartmental analysis (NCA). Single-dose NCA is
available in two places: `summarize_profile` with `route` (`iv_bolus` or
`extravascular`), `time_unit`, `concentration_unit` and optionally `dose`, and an
`nca` readout with `route` (and `dose`), which analyses the simulated window with times
measured from its start. Both report CDISC parameters (AUCLST, AUCIFO/AUCIFP,
AUCPEO/AUCPEP, LAMZ, LAMZHL, LAMZNPT, R2ADJ, CL(F)O, VZ(F)O and others), each with its
unit, using linear-up/log-down AUC by default and the adjusted-R² rule for the terminal
rate constant λz (at least three terminal points), as `qspquest.nca` does in Python.
Clearance and volume are divided by the bioavailability F for extravascular doses.
Handling of concentrations below the limit of quantification (BLQ), infusions, the area
under the first-moment curve (AUMC) and mean residence time (MRT) are not covered. Use `detail: full` or
read `full_result_resource` for the requested grid. Local stdio caches eight
results of up to 4 MB for 15 minutes; hosted results have no cache link. Only
selected outputs are evaluated, so domain checks on unselected readouts are
skipped.

For larger or longer-lived results, start with `--artifact-dir PATH` (an empty
private directory). Results are kept as immutable JSON for 24 hours, across
restarts, up to 128 MB each, 256 artifacts and 1 GB in total. See the
[retained-result example](../../docs/tutorials/mcp-results.md#retain-complete-results-across-local-server-restarts).

Related workflows:

- `get_patient_parameters` evaluates outputs at the starting state, before any
  dose; see the
  [initial-value example](../../docs/tutorials/mcp-results.md#inspect-initial-patient-quantities).
- `scheduled_boundary_receipt` shows forcing changes, events and observation
  sides; see the
  [scheduled-boundary tutorial](../../docs/tutorials/mcp-results.md#inspect-scheduled-boundaries-and-observation-sides).
- Numerical, conservation, source-fidelity and clinical status are reported
  separately; see the
  [scientific assessment workflow](../../docs/tutorials/mcp-results.md#inspect-separate-scientific-assessments).
- `format_response` maps computed columns into requested arrays with explicit
  units, times and `pre`/`post`/`both` sides; see the
  [response guard tutorial](../../docs/tutorials/mcp-results.md#format-complete-decision-responses).

## Requests

Quantities are `{value, unit}`. For a stored scenario use
`{"model":{"scenario":{"model_id":"qspquest_diffdose_2026_idr_benchmark",
"scenario_id":"higher_exposure"}}}`. `parameters` and `initial_states` are
quantity maps using the names and units from `get_model`. `covariates` is a
quantity map of declared covariates; a covariate named under `parameters` is
routed to it. Friendly dosing
supports target, amount, at, every/count, duration, bioavailability and lag.
`scan_parameters` accepts the same friendly `end`, `times`, `doses`,
`parameters`, `initial_states`, `covariates`, `solver` and `readouts` as
`simulate`, and returns each point's readouts. It varies a parameter, a covariate
(`parameter` names it) or the amount of `doses[dose_index]`; covariate and dose scans run
one `simulate` per point and add a table of scanned value against readouts. A dose amount
given per body mass (for example `mg/kg`) is resolved against body weight: a weight
covariate or parameter in the request (`wt`, `weight`, `body_weight`, ...), then the stored
scenario, then the model's default. The resolved amount is recorded in `notes` (and in
`resolved_doses` for each scan point); without a weight the dose is refused.
`simulate_steady_state` repeats one maintenance dose every `interval` until the change
still to come in each metric, extrapolated geometrically from the last three intervals, is
within `relative_tolerance` (default 1e-3). A comparison of only the last two intervals
would understate the shortfall by about the accumulation ratio minus one. It returns, per
output, the
steady-state AUC over the interval, peak, minimum, trough, average and accumulation ratio
with units. It works for any model with friendly doses, nonlinear ones included; the
native `steady_state` tool stays the exact path for `linear_pk` models. `find_dose` bisects one dose's amount
until a readout is at most or at least a threshold, for a typical subject. `sensitivity`
ranks parameters by their effect on each readout, as local elasticities or by Morris
screening over stated ranges, using the same friendly fields.
`fit_individual` takes the same friendly `end`, `doses`, `parameters` and `covariates`, and
observations given by `time` (and `side`), from which the server builds the observation
grid. Every readout result carries its unit. A readout may carry `reported`
(`{value, unit?, relative_tolerance?, absolute_tolerance?, metric?}`); its row then gains
`comparison` with the reported value in the output's unit, the relative error and
`within_tolerance`, for checking a model against published values. `search_models` matches whole words in model
names, descriptions, scenario ids and outputs, joins domain phrases (TMDD, ADC, CRS) and
refuses unknown filters. Library models whose availability record does not list `exploratory_simulation`
refuse free-form `simulate`, `scan_parameters`, `fit_scalar` and `fit_individual` calls
with error code `operation_outside_supported` and name their presets; earlier versions
returned an ordinary result for such calls (see the changelog). `dose.every` requires `dose.count`. Full output
schemas are served as `qspquest://schema/tools/<tool>`; `tools/list` advertises the
result envelope only. `compare_regimens` keeps every
member's readouts and summary inline when trajectories are compacted to fit the
response budget.
Observation sides are explicit. Friendly runs need a positive end; defaults are
the backward differentiation formula (BDF) solver, start zero, `rtol: 1e-8`, `atol: 1e-10`, no doses and 101 evenly spaced
samples. Check `notes` and `effective_request` for defaults.

To replay a native request, pass the returned `effective_request` as `request`.
Top-level `parameters` and `initial_states` replace only their native keys;
top-level solver, end, times and doses replace those fields. An overridden
preset is no longer covered by the preset's checks. Two names for the same
input are refused.

Readout thresholds can be a number in the output's unit or a quantity such as
`{"value":5000,"unit":"ug/L"}` for an output in `mg/L`. A dose is given
in the amount unit of the target's dosing mapping, which `get_model` reports as
`dose_targets[].amount_unit`; for `dosing c [mg] = 1 / v` on a concentration
state `c` this is mg, whatever the scale. Archives in source coordinates
(amount unit `1`) may declare the input mass unit in their state metadata. Mass
units such as mg, g and ug are converted before the dose scale.
For doses, thresholds, timed changes and archive aliases, explicit unit `1`
passes a value unchanged in the stored unit; canonical names and native requests
treat `1` as dimensionless (see
[units](../../spec/units.md#archive-source-coordinates)). AUC uses the sampled
trapezoidal rule with post-event start and pre-event end values; `value`
defaults to post-event and `trough` uses pre-event. Terminal half-life is null
unless the terminal points are positive and falling. Models that declare
`dose_schedule_mode` as `native_administrations` use their native schedules.
Archives may declare a treatment-cycle step input that advances on distinct
positive-dose starts on its trigger targets.

Fits assume independent Gaussian errors. `fit_scalar` estimates one parameter
with 1..500 observations and 1..1000 iterations (default 100); proportional,
lognormal and censored errors are refused, never approximated. `fit_individual`
takes `parameters_to_fit` with lower/upper quantities, or a complete native
`fit_request` (including
[profiled proportional errors](../../docs/tutorials/profiled-proportional-errors.md)).
Population fitting offers first-order conditional estimation with interaction
(FOCEI), the Laplace method and stochastic approximation
expectation-maximization (SAEM); see
`qspquest://examples/fit/{scalar|individual-additive|individual-proportional|population}`
and the [population-record workflow](../../docs/tutorials/mcp-results.md#prepare-and-fit-explicit-population-records).
Missing observations are never imputed.

### Native analyses

`analyze` runs the analysis workflows of the native engine, the same
implementation that Python, R, `qspquest analyze` and
`CompiledSimulation.analyze` call (see the
[analysis guide](../../docs/ANALYSIS.md)). Its `request` is a
`qspquest.analysis/v0.2` envelope `{schema, operation, arguments}` with operation
`fit_data`, `fit_population`, `visual_predictive_check`, `profile_likelihood`,
`bootstrap_population`, `event_sensitivities`, `exposure_summary` or
`fit_diagnostics`. Arguments use native parameter names and units, ordered arrays
of named parameter records, and complete `qspquest.run/v0.1` runs; friendly
doses and aliases are not applied inside them. For example, to fit clearance on
the synthetic model `conformance/models/synthetic-one-compartment.qsp` (a 6 mg
bolus into 3 L, so `cp(t) = 2 exp(-0.1 t)` mg/L at clearance 0.3 L/h):

```json
{"model":{"source":"<model text>"},
 "request":{"schema":"qspquest.analysis/v0.2","operation":"fit_data","arguments":{
   "run":"<conformance/requests/synthetic-one-compartment.json>",
   "observations":[{"row":1,"output":"cp","value":{"value":1.8097,"unit":"mg/L"}}],
   "parameters":[{"name":"cl","initial":0.5,"lower":0.05,"upper":2,"unit":"L/h"}],
   "error":{"additive":0.01},"max_simulations":2000}}}
```

`plan_run` with `{"operation":"analyze","arguments":{...}}` returns a
`qspquest.analysis-plan/v0.2` preparation. It checks the accepted arguments,
parameter, error, random-effect and percentile specifications and integer
controls, prepares every embedded run at the initial or typical parameter
values without integrating it, and checks observation rows and units against
the prepared grid. It runs no simulation and draws no random numbers; it reports
`max_simulations`, the exact `required_simulations` of a VPC, comparison or
event-sensitivity request, and the checks left to execution. Execute with the
returned `expected_plan_hash`. The result keeps the native record verbatim under
`analysis`, with `analysis_operation`, `analysis_status` and the analysis
`identity`; an analysis covers many simulations, so its contract identity scope
is `metadata`. Local results larger than the inline budget are linked through
`full_result_resource`. `submit_local_job` accepts `analyze` work units.

Released library models run visual predictive checks, event sensitivities and
record analyses. The estimation analyses (`fit_data`, `fit_population`,
`profile_likelihood`, `bootstrap_population`) need a qualified fitting contract
for the library version; none declares one yet, so supply the model source as an
explicitly labeled user candidate for exploratory estimation.

`compare_regimens` is one operation, the native paired comparison. Without
`population` it is the deterministic comparison of named conditions: one subject
and no random effects. Each member keeps its complete simulation record, and
with `output` the result adds the native `comparison` record of the solved
members (exposure metrics and paired differences against the first regimen,
`variability: none`, no extra simulation) and a scalar `comparison_summary`
that compaction never drops. With a `population` block
(`parameters` as ordered `{name, value, unit}` typical values, `omega`, and
optionally `random_effects`, `subjects`, `seed`, `threshold`, `percentiles`,
`parameter_draws`, `max_simulations` and `workers`) each member is prepared
exactly as a deterministic member, and the effective native runs become the
labelled regimens of the native paired comparison: the same virtual subjects
receive every regimen, and the result reports curve and exposure percentiles,
per-subject metrics and paired differences against the first regimen under
`population`. `output` is required; `save_csv`, `required_outputs`, `readouts`
and `parameter_changes` are refused, as is a shared or member override of a
parameter that varies between subjects.

Window readouts and `exposure_summary` share one trapezoid. Their threshold
durations differ on a segment lying exactly on the threshold: readouts count
strictly above or below, and `exposure_summary` counts it as above.

### Limits and repairs

| Inline limit | Local | Hosted |
| --- | --- | --- |
| Serialized request | 1000000 UTF-8 bytes | 1000000 UTF-8 bytes |
| Run solver callbacks / events / output values | 1000000 / 10000 / 25000000 | 1000000 / 10000 / 25000000 |
| Aggregate solver callbacks / events / output values | 25000000 / 250000 / 625000000 | 25000000 / 250000 / 625000000 |
| Optimizer evaluations / iterations | 100000 / 10000; native two-effect SAEM permits an explicit 200000 evaluations | 2000 / 300 |
| Population subjects / combined observations | 128 / 10000 | 16 / 256 |
| Native population random effects | At most 2 | At most 2 |
| Analysis `max_simulations` | 1..1000000; operation default when omitted | Required, 1..500 |
| Analysis workers | 1..8 | 1 |
| Analysis run budget times `max_simulations` | Not bounded beyond each run | Within the aggregate budgets above |

Missing run-budget fields get the run defaults above, including runs inside an
analysis request; aggregate budgets are not defaulted. Hosted user-source
analyses run in the WASM sandbox, whose input is 2 MB, so the model source and
request together must stay under 1.9 MB; models with 64 or more states run their
analyses through local stdio only. Inline scans take 1..25 values, or 2..25 evenly spaced points;
managed jobs may use the platform's 25/100/225/400 tier ceilings. For
larger work, use a local or managed job; the server never drops subjects or
observations to fit a limit.

Errors give a JSON pointer in `error.expression` (also
`contract.failure.expression`), such as `/run/budgets/events`,
`/parameters/clearance/unit`, `/doses/0/amount/unit` or `/readouts/0/side`, and
a repair hint. An
empty pointer means the whole request. Resubmit a complete corrected request.

## Save CSV files

CSV export is off until the server starts with `--output-dir PATH` (or
`QSPQUEST_MCP_OUTPUT_DIR`):

```sh
target/release/qspquest-mcp --library-dir /absolute/path/qspquest-models --output-dir /absolute/path/qspquest-results
```

Then call `simulate` with `save_csv: "study/run.csv"`. The reply's
`saved_csv.path` and `saved_csv.manifest_path` are absolute, with SHA-256
digests of both files. Keep the CSV and its `.manifest.json` together; read
`qspquest://schema/table-manifest` and `qspquest://guide/results` for the
format and reimport. Dose times can have two rows, `pre` then `post`.

The server only creates new files and refuses existing files, links and unsafe
directories (owned by the server user, mode `0700` on Unix). Comparisons write
one bundle per regimen, such as `study/compare_first.csv`; see the
[comparison example](../../docs/tutorials/mcp-results.md#inspect-and-execute-a-regimen-comparison).
Because `save_csv` can write a file, `simulate` and `compare_regimens` advertise
`readOnlyHint: false` and `idempotentHint: false`. Hosted servers refuse CSV
export.

## HTTP and hosted execution

Choose `--profile local` (stdio), `--profile public` (bounded public HTTP) or
`--profile managed` (HTTP with verified sessions). The
[deployment tutorial](../../docs/tutorials/mcp-deploy.md#choose-an-mcp-deployment-profile) covers
Host and Origin allowlists, HTTPS ingress, trusted peers and managed
configuration.

Start HTTP with `qspquest-mcp --http 127.0.0.1:8080`. Endpoints are `/mcp`,
`/healthz` and, in sandbox builds, `/assets/engine.wasm`. HTTP is stateless,
with no session ID or local run resources. User-supplied models over HTTP need
a `sandbox` build (`sh scripts/build-mcp.sh`); default builds refuse them.

Hosted user models run in Wasmtime with no host imports, 256 MiB memory, a
100-billion-instruction fuel allowance and a 45-second wall limit. Library
models run natively. Payloads are capped at 1 MB and inline responses at
256 KB. BDF is the default solver; Rosenbrock23 and Rodas5P are not
available. Hosted models with 64 or more states return `async_required`;
`submit_simulation` runs them as a managed job through `pkpd.simulate_scenario`
once the platform supports that operation.

HTTP startup precompiles up to eight library models; set
`QSPQUEST_MCP_PREWARM=0` to measure cold requests.

Set `QSPQUEST_WIDGET_WORKER_URL` to the absolute URL of
`/assets/js/engine-worker.mjs` on a `sandbox,widget` build to enable on-device
edits in the MCP App. Model and result data stay in a local Worker.

Container build: `docker build -f bindings/mcp/Dockerfile --target public -t qspquest-mcp .`
(use `--target managed` for a managed build). Configure the library URL,
SHA-256 and cache, or mount a library directory.

Other settings: `QSPQUEST_MCP_TIER`, `QSPQUEST_MCP_CONCURRENCY` and
`QSPQUEST_MCP_ALLOWED_HOSTS` (comma-separated). Software development kit (SDK)
reference:
[official Rust MCP SDK](https://rust.sdk.modelcontextprotocol.io/).

## Availability records

Libraries declaring `qspquest.model-availability/v1` supply
`qspquest_models/availability.json`. Search results and model cards include each
model's `availability`: whether it can run, its review and unit status and
fitting support. Search also returns `availability_counts`. Availability applies
only to the unchanged preset on this engine build. Libraries without the
registry load with `availability_record_status=unavailable` and null
`availability_counts`.

### Archive holds

Each archive is checked on load: its digests and members, its availability
record, native model, source interpretations, fitting evidence, stored
scenarios, precision profiles, scientific assessments and protocol
initialization. When one archive fails a check, the server withholds it and
continues with the rest of the library. Each withheld archive carries an
`archive_hold` record with `model_id`, `version`, `scope`, `check` and
`message`, which appears in its search row and model card. Search results and
`qspquest://capabilities` also list every hold under `archive_holds`, and the
server prints each hold to standard error at load.

The scope of a hold depends on the check that failed:

- `fitting`: the recorded fitting evidence does not match the archive or this
  engine. Fitting is withheld, `fit_ready` and `fitting_supported` become
  false, and the availability record loses `fit` from `supported_operations`.
  Inspection and simulation of the same model remain available.
- `archive`: any other archive check. The record keeps its catalog metadata
  for search, has no executable model and lists its stored scenarios under
  `unavailable_scenario_ids`. `execution_blocker` states the failed check.

Holds report a fault in the library release; aggregate `availability_counts`
still describe the release as declared. Holding applies to the local (stdio)
profile only. Public and managed deployments refuse any failing archive: the
server stops at load, and the error names the model, version and check. A local
server can do the same with `--strict-library` or `QSPQUEST_STRICT_LIBRARY=1`.
Library refreshes use the same setting.

## Managed jobs

Managed builds add `preview_job`, `submit_simulation`, `submit_population_fit`,
`submit_scan`, `get_job`, `wait_for_job` and `get_artifact`. They need a
verified session resolver and platform support for the operation; see
[managed HTTP](../../docs/tutorials/mcp-deploy.md#managed-http)
for configuration and the synthetic tests.

1. `preview_job` sends an exact `platform_request` and returns
   `managed_preview` with `valid`, `preview_digest`, `approval_required` and
   credit `pricing`. It creates no Run. Review the price before submitting.
2. Submit with the digest in `platform_request.expected_preview_digest` and an
   `idempotency_key`. Reuse the key only to retry the identical request. A
   missing digest fails before any platform call, and invalid routing fields
   report their location, such as `/platform_request/destination_project_id`.
3. Submission returns `managed_submission.run` and `managed_submission.jobs`.
   Poll a Job's `id` with `get_job` or `wait_for_job` (`managed_job`) and check
   its state before treating work as complete.
4. `get_artifact` returns `managed_artifact` metadata. When
   `artifact_retrieval.state` is `available`, read and verify the chunks in
   `full_result_resource`; see the
   [retrieval tutorial](../../docs/tutorials/mcp-deploy.md#retrieve-a-complete-governed-result-artifact).

Only `submit_*` tools create a priced Run. Free sessions are refused before
private calls.

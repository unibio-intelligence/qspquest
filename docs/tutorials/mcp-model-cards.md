# Inspect model cards and retrieve full details

Use this workflow to inspect a model's simulation interface without loading its
complete metadata and equations into the tool response. Python 3.10 or later on
macOS or Linux, the Rust toolchain and a native compiler are required. Run from
the QSPQuest source distribution root:

```sh
cargo build --release -p qspquest-mcp --locked
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --model-cards
```

The complete example creates a temporary synthetic `.qsp` model and a stored
request, starts the server with that local library, and removes them on exit.
It requires no downloaded model library, assistant account or provider credits.
Expected output is:

```text
Verified compact model card, complete metadata and stored scenario.
```

The example checks the card's time unit and simulation interface, retrieves
complete metadata, checks that its version and archive digest match the card,
and compares the stored scenario with the supplied request. These are interface
checks using a synthetic model; they do not qualify a published model or a
modified scenario for clinical use.

In an MCP client, call `get_model` with `{"model_id":"your_model_id"}`. The
default `detail: compact` response gives model identity, citations, execution
availability, time unit, dose targets, parameter names/defaults/units, modeled
outputs, state units, fitting capabilities and scenario IDs. It omits equations,
source text and extensive metadata. A reported-values name list
is a labeled preview of at most 25 names; its count describes the complete list.
Inspect execution blockers and dimensional limitations before interpreting units.

`interface.output_contract` provides the canonical output ID, returned column,
native unit and accepted request aliases. Alias records keep any declared source
unit separate from the native unit. A source label with `pg/mL` can still return
native unit `1`; this mapping applies no unit conversion. Analyte and compartment
are null unless the archive explicitly declares them. Do not infer either from
a name or state index.

Compact cards include at most 32 output descriptors. Check `output_count` and
`descriptors_complete`. To inspect particular outputs, pass `outputs` with 1 to
32 exact aliases or canonical IDs, for example
`{"model_id":"your_model_id","outputs":["your_output_name"]}`. Unknown or
ambiguous names fail before execution. The complete details resource retains
all descriptors even when a card is filtered.

`plan_run` records `plan.output_mapping` for simulation and initial-value
requests and parameter scans. `simulate` and `scan_parameters` return the same
mapping beside their numerical results,
including readout selections and default outputs. Match each `requested_output`
to `returned_column` in the complete result's `outputs` array and check
`returned_unit`; no positional decoding is required. For initial values,
`get_patient_parameters.output_mapping[].returned_quantity` identifies the key
in `patient_quantities`. Changing a request alias changes the plan hash, even
when the native calculation is equivalent. Large mappings can be omitted from
a bounded inline response; `output_mapping_notice` states the omitted mapping
count, and `full_result_resource` links the complete record. Retrieve that resource before testing
analyte completeness.

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
concentration columns and 602 analytical samples.` The mapping applies to every
scan point; it does not qualify a published model or its parameter bounds.

Before changing a parameter, inspect `plan.override_receipt`. Each explicitly
supplied parameter or covariate has its request name, canonical native name,
supplied value, effective request quantities and native consumers. State
equations are traced through algebraic definitions; output consumers also
include potential propagation through state evolution. These are structural
references across all expression branches, so they do not promise a measurable
effect or establish identifiability.

`consumption` distinguishes `dynamic`, `initialization_only`,
`static_output_only` and `unused`. Initial expressions replaced by an explicit
`initial_states` quantity are listed separately in `replaced_initial_states`.
An unused override emits a warning in the plan and result. A static output
changes a stored scalar readout without changing state dynamics. Literal initial
conditions and resolved production coefficients remain fixed unless explicitly
overridden; the server does not reconstruct source initialization or scaling
formulas. Native initial expressions and algebraic definitions are evaluated
by the engine. Inspect `source_reconstruction` before using a scaling input to
represent a different physiology or species.

Receipts are bound into the plan hash and retained with complete results for
simulations, patient initial values and parameter scans, including each regimen
in a comparison. Scan receipts list the effective point quantities. Scenario
defaults are excluded unless the caller explicitly overrides them. A large
receipt can be omitted inline; `override_receipt_notice` gives its entry count
and `full_result_resource` links the complete receipt and warnings.

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
analytical samples and changed-input refusal.` This illustrates native
consumption, without qualifying source reparameterization or physical models.

To inspect everything, call `get_model` with
`{"model_id":"your_model_id","version":"the_card_version","detail":"full"}`.
Small records return inline. Larger records return `detail: full_resource`, a
notice and `full_model_details_resource`. This resource link is also present on
compact cards. Read it with `resources/read`. If the returned JSON has
`kind: utf8_chunk_manifest`, read every listed chunk in order, concatenate the
UTF-8 text and verify its SHA-256 against `sha256` before parsing it. Complete
records include native parameters, states, outputs, equations, metadata and
source text or a separate source resource link. Model resources are served from
the loaded library rather than the temporary simulation-result cache.

Fetch a stored request separately with `get_scenario_request`, supplying the
same `model_id`, `version` and a `scenario_id` from the card. The returned
`request` is the stored native request; `readable_request` uses the interface
names. Changing a stored scenario creates a new calculation that needs its own
qualification. For a large stored request, read
`qspquest://models/{model_id}/scenarios/{scenario_id}` and follow a chunk manifest
if returned.

A missing executable means the build failed or the path is wrong. Invalid
`detail` values are rejected; use only `compact` or `full`. A missing model or
version means the selected library cannot resolve it: inspect the current card
or select a matching library version. A resource digest mismatch means retrieval
is incomplete or the library changed during the reads; obtain a fresh card and
read the complete resource again. Do not interpret a preview or an error as full
model retrieval.

## Inspect execution and declared source families

Run the complete synthetic family example from the same source distribution:

```sh
cargo build --release -p qspquest-mcp --locked
python3 docs/examples/mcp_result_resources.py target/release/qspquest-mcp --source-families
```

The example creates two native model archives and one metadata-only record for
one synthetic source. It verifies search, exact model/version selection, compact
cards and the digest of the complete family resource. Expected output starts
with `Verified full, reduced and metadata-only records`. All inputs are created
locally and removed on exit; these checks qualify the discovery protocol only.

Every public-library search row has `execution_available`, `scenario_count` and
a typed `discovery` record. `discovery.executable_backend` identifies the available
QSPQuest native projection. `contract.identity.scope` describes the search
response itself. A metadata-only or source-inspection record has execution
availability false and a null executable backend. A user-source digest appears
in the model identity; its discovery record leaves `archive_sha256` null.
Projection availability still requires validation of the intended request.
`execution_assessment` is `requires_request_validation` for a native executable
and `unavailable` when the local record has no executable.

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
`qspquest://schema/source-family` describe the closed records. A stale family URI
fails explicitly after its catalog membership changes. Family declarations and
byte digests carry no scientific qualification; use `get_capability` for the
specific model, scenario, operation and running artifact's evidence scope.
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
record and a tier-locked record. It checks null versus false, the governed route,
exact identifier/version selection and refusal for a Free scope. It uses a
synthetic 42-credit preview and verifies stable retries and typed Job/Artifact
retrieval. Expect the synthetic price line and one passing test. All services
and inputs are created locally; no account, real credits or provider access is
required. Invalid provider identifiers, scalar types or access flags produce a
platform-response error; inspect the provider contract instead of treating
malformed metadata as an executable model.

## Navigate offline documentation

Use the same Python 3.10+, macOS/Linux and source-build prerequisites as above.
The server embeds the document contracts and example payloads, so reading them
requires no network or companion library. Run:

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
`qspquest://schema/capability-catalog`. This source catalog distinguishes native
fitting from Python analysis and direct WASM from the Browser Worker. Its records
describe declared contracts and required features; they retain untested scientific
status. Use the active capability resource for this server's features and limits.
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
`qualified: false`. Null means an exact request still needs the normal tool's
admission and execution; a query does not run a solver. A missing model,
inspection-only scientific workflow, unavailable feature, incompatible analytic
solver, or unready released fitting contract gives blockers and
`executable: false`. Another interface, target or deployment remains unassessed.
Governed operations need an admitted session and configured backend; preview
their platform model readiness and price before submission.

The query reports the selected model/archive identity and optional stored request
digest. It does not assess a modified request or interpret historical archive
evidence as current qualification. `qspquest://schema/capability-registry` and
the tool's output schema describe the response contracts.
For oversized resources, read the chunk manifest, concatenate chunks and
verify its digest before interpreting the document.

It then reads the complete synthetic model and request from resource URIs and
calls `simulate` with `model.source` and `request`. The 6 mg bolus, 3 L volume
and 0.3 L/h clearance imply `cp(t) = 2 * exp(-0.1*t)` mg/L. The example checks
six post-dose samples over 12 h and reports the number of resources and links
verified. The model contains no drug-specific parameters; the calculation
does not qualify a clinical model.

A heading not present in the installed version returns an invalid-resource
error. Read a fresh index before following links from another version. A digest
mismatch indicates incomplete retrieval or changed resource content; fetch the
complete current resource again. Hosted source execution needs a configured
sandbox, as reported by `qspquest://capabilities`; reading documents alone
does not grant execution support.

## Export a complete synthetic trajectory

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
retains the CSV and cleans up its synthetic library when finished. It demonstrates
file export and numerical correctness for this synthetic model only.

CSV export is disabled unless a local server has `--output-dir PATH` or
`QSPQUEST_MCP_OUTPUT_DIR`. Unix output directories must be owned by that server
user with mode `0700`; Windows uses owner and ACL checks. File and directory
links, traversal and existing output files are refused. Choose another filename
or a new directory to rerun this example. Hosted servers refuse CSV export.
Regimen comparisons apply the same rules and append each sanitized regimen name
to the CSV filename; completed files remain if a later regimen fails. A CSV
contains the complete requested trajectory but does not itself establish model
qualification or carry the model card and effective request. Retain those with
any scientific interpretation.

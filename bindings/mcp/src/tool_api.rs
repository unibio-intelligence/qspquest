//! Typed MCP tool inputs shared by transport and admission.
use crate::contract::JsonValueSchema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Search {
    pub model_id: Option<String>,
    pub version: Option<String>,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
    #[serde(default)]
    pub query: String,
    #[schemars(with = "Option<JsonValueSchema>")]
    pub filters: Option<Value>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelId {
    /// Bind input names and units to a previously inspected native model card.
    pub expected_inspection_hash: Option<String>,
    #[serde(alias = "id")]
    pub model_id: String,
    pub version: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelDetail {
    Compact,
    Full,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelCardRequest {
    pub expected_inspection_hash: Option<String>,
    /// Inspect one content-bound page of native declarations instead of a model card.
    pub page: Option<crate::inspection::PageRequest>,
    /// Select 1 to 32 exact outputs, stored scenarios and inspection resources in one bounded response.
    pub selection: Option<crate::inspection::SelectionRequest>,
    /// Focus output descriptors on 1 to 32 exact request aliases or canonical names.
    pub outputs: Option<Vec<String>>,
    #[serde(alias = "id")]
    pub model_id: String,
    pub version: Option<String>,
    /// Compact interface by default; full metadata and equations on request.
    pub detail: Option<ModelDetail>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub expected_inspection_hash: Option<String>,
    #[serde(alias = "id")]
    pub model_id: String,
    pub scenario_id: String,
    /// Complete archive-bound scalar convention; inspect get_model.precision_profiles.
    pub precision_profile_id: Option<String>,
    pub version: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub source: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub kind: String,
    #[schemars(with = "Option<JsonValueSchema>")]
    pub options: Option<Value>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum ModelRef {
    Library { library: ModelId },
    Scenario { scenario: Scenario },
    Source { source: String },
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Quantity {
    pub value: f64,
    /// Explicit unit; "1" means a value already in the model's own coordinates.
    pub unit: String,
}
/// One readout on the solved trajectory.
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ReadoutThreshold {
    Number(f64),
    Quantity(Quantity),
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Readout {
    /// nca, auc, min, max, time_below, time_above, first_crossing, trough or value.
    pub kind: String,
    /// Output name as listed by get_model.
    pub output: String,
    /// Window start; with `to`, for nca, auc, min, max, time_below, time_above and first_crossing. Defaults to the start of the run.
    pub from: Option<Quantity>,
    /// Window end. Defaults to the end of the run.
    pub to: Option<Quantity>,
    /// Time for trough (the pre-dose value) and value.
    pub at: Option<Quantity>,
    /// Numeric threshold in the engine output's unit, or a dimensionally compatible quantity. Source-scale output units do not acquire physical dimensions from display aliases.
    pub threshold: Option<ReadoutThreshold>,
    /// first_crossing only: up, down or any.
    pub direction: Option<String>,
    /// value only: pre or post, at a dose time.
    pub side: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dose {
    pub target: String,
    pub amount: Quantity,
    pub at: Quantity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub every: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lag: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bioavailability: Option<f64>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub lower: Quantity,
    pub upper: Quantity,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScanRange {
    pub lower: Quantity,
    pub upper: Quantity,
    pub count: usize,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FitParameter {
    pub parameter: String,
    pub lower: Quantity,
    pub upper: Quantity,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamedRegimen {
    pub name: String,
    /// Complete profile for this member; requires a shared scenario selection.
    pub precision_profile_id: Option<String>,
    pub doses: Vec<Dose>,
    /// Starting parameter quantities for this member; matching names replace shared quantities.
    pub parameters: Option<std::collections::BTreeMap<String, Quantity>>,
    /// Explicit starting state quantities for this member; matching names replace shared quantities.
    pub initial_states: Option<std::collections::BTreeMap<String, Quantity>>,
    /// Parameter changes during this regimen only; same form as simulate.parameter_changes.
    #[schemars(with = "Option<Vec<JsonValueSchema>>")]
    pub parameter_changes: Option<Vec<Value>>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Simulate {
    /// Refuse before numerical work unless source, context and preparation still match plan_run.
    pub expected_plan_hash: Option<String>,
    pub model: ModelRef,
    /// Advanced and normally left unset: a complete native qspquest.run/v0.1 request object. Use doses, parameters, times and readouts instead.
    /// Versioned qspquest.run/v0.1 request; optional when a scenario is selected.
    #[schemars(with = "Option<JsonValueSchema>")]
    pub request: Option<Value>,
    /// Explicit quantities, for example {"cl":{"value":2,"unit":"L/h"}}.
    pub parameters: Option<std::collections::BTreeMap<String, Quantity>>,
    pub initial_states: Option<std::collections::BTreeMap<String, Quantity>>,
    /// Friendly doses: target, amount quantity, at quantity; optional every, count, duration, bioavailability, lag.
    /// Parameter values that change during the run: [{"name":..., "at":{"value":21,"unit":"d"}, "value":{"value":0.002,"unit":"1/d"}}]. Before the first change the parameter keeps the value from parameters, the scenario or the model default; each change holds until the next.
    #[schemars(with = "Option<Vec<JsonValueSchema>>")]
    pub parameter_changes: Option<Vec<Value>>,
    pub doses: Option<Vec<Dose>>,
    pub times: Option<Vec<Quantity>>,
    pub end: Option<Quantity>,
    pub solver: Option<String>,
    pub outputs: Option<Vec<String>>,
    /// Require named outputs and optionally their declared analyte/compartment before integration. Missing or unverified declarations refuse the request; no numerical placeholders are created.
    pub required_outputs: Option<Vec<crate::core::required_outputs::RequiredOutput>>,
    /// Readouts computed on the solved trajectory. Each entry has kind, output and either a window (from, to) or a time (at): nca (cmax, tmax, cmin, auc, cavg, terminal half-life in the window), auc, min, max, time_below (threshold), time_above (threshold), first_crossing (threshold, direction up/down/any), trough (pre-dose value at a time), value (side pre/post).
    #[schemars(with = "Option<Vec<Readout>>")]
    pub readouts: Option<Vec<Value>>,
    /// Create a full-trajectory .csv and appended .manifest.json beneath the configured output directory (local stdio only). Both names must be new. The manifest retains units, source, effective inputs, identity and qualification limits.
    pub save_csv: Option<String>,
    /// compact (default) returns a labeled preview; full returns the requested grid when it fits. Read full_result_resource for a capped result.
    pub detail: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GaussianError {
    pub additive_sd: Quantity,
    pub proportional_sd: f64,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub row: usize,
    pub output: String,
    pub value: Quantity,
    pub error: GaussianError,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Compare {
    /// Refuse before any regimen executes unless the complete comparison still matches plan_run.
    pub expected_plan_hash: Option<String>,
    pub model: ModelRef,
    #[schemars(with = "Option<JsonValueSchema>")]
    pub request: Option<Value>,
    pub regimens: Vec<NamedRegimen>,
    pub output: Option<String>,
    /// Parameter or covariate overrides applied to every regimen, by name with units.
    pub parameters: Option<std::collections::BTreeMap<String, Quantity>>,
    pub initial_states: Option<std::collections::BTreeMap<String, Quantity>>,
    /// Parameter changes during the run applied to every regimen; same form as simulate.parameter_changes.
    #[schemars(with = "Option<Vec<JsonValueSchema>>")]
    pub parameter_changes: Option<Vec<Value>>,
    pub outputs: Option<Vec<String>>,
    /// Require named outputs and optionally their declared analyte/compartment before integration. Missing or unverified declarations refuse the request; no numerical placeholders are created.
    pub required_outputs: Option<Vec<crate::core::required_outputs::RequiredOutput>>,
    pub times: Option<Vec<Quantity>>,
    pub end: Option<Quantity>,
    /// Same readouts as simulate, evaluated for every regimen.
    #[schemars(with = "Option<Vec<Readout>>")]
    pub readouts: Option<Vec<Value>>,
    /// Relative .csv path beneath the configured output directory; each regimen creates a new file with its name appended. Existing files and links are refused.
    pub save_csv: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Sample times, ascending.
    pub times: Vec<f64>,
    /// Concentrations (or any response) at those times.
    pub values: Vec<f64>,
    pub time_unit: Option<String>,
    /// Optional threshold for time below and time above.
    pub threshold: Option<f64>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Patient {
    /// Refuse before output evaluation unless the complete initial-value preparation still matches plan_run.
    pub expected_plan_hash: Option<String>,
    /// Select named outputs; omission evaluates all outputs at the bound initial state.
    pub outputs: Option<Vec<String>>,
    pub model: ModelRef,
    /// Covariates and other parameter overrides by name with units, for example {"body_weight_kg":{"value":48,"unit":"kg"}}.
    pub covariates: Option<std::collections::BTreeMap<String, Quantity>>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scan {
    /// Refuse before numerical work unless source, context and preparation still match plan_run.
    pub expected_plan_hash: Option<String>,
    pub model: ModelRef,
    #[schemars(with = "Option<JsonValueSchema>")]
    pub request: Option<Value>,
    pub parameter: String,
    pub values: Option<Vec<Quantity>>,
    pub range: Option<ScanRange>,
    pub output: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScalarFit {
    /// Refuse before numerical work unless source, context and preparation still match plan_run.
    pub expected_plan_hash: Option<String>,
    pub model: ModelRef,
    #[schemars(with = "Option<JsonValueSchema>")]
    pub request: Option<Value>,
    /// Alternatively supply the native qspquest.scalar-fit/v0.1 request.
    #[schemars(with = "Option<JsonValueSchema>")]
    pub fit_request: Option<Value>,
    pub parameter: Option<String>,
    pub bounds: Option<Bounds>,
    pub observations: Option<Vec<Observation>>,
    pub max_iterations: Option<usize>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndividualFit {
    /// Refuse before numerical work unless source, context and preparation still match plan_run.
    pub expected_plan_hash: Option<String>,
    pub model: ModelRef,
    #[schemars(with = "Option<JsonValueSchema>")]
    pub request: Option<Value>,
    /// Alternatively supply the native qspquest.fit/v0.1 individual request.
    #[schemars(with = "Option<JsonValueSchema>")]
    pub fit_request: Option<Value>,
    pub parameters_to_fit: Option<Vec<FitParameter>>,
    pub observations: Option<Vec<Observation>>,
    pub max_iterations: Option<usize>,
    pub max_evaluations: Option<usize>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PopulationFit {
    /// Refuse before numerical work unless source, context and preparation still match plan_run.
    pub expected_plan_hash: Option<String>,
    pub model: ModelRef,
    /// Native qspquest.fit/v0.1 request, or template/rows/mapping for NONMEM records.
    #[schemars(with = "Option<JsonValueSchema>")]
    pub fit_request: Option<Value>,
    #[schemars(with = "Option<JsonValueSchema>")]
    pub template: Option<Value>,
    #[schemars(with = "Option<Vec<JsonValueSchema>>")]
    pub rows: Option<Vec<Value>>,
    /// Explicit compartments/endpoints/time_unit/amount_unit/rate_unit/observation_side.
    #[schemars(with = "Option<JsonValueSchema>")]
    pub mapping: Option<Value>,
    pub method: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeAnalysis {
    /// Optional exact plan_run digest; refuses changed inputs or execution context.
    pub expected_plan_hash: Option<String>,
    pub model: ModelRef,
    #[schemars(with = "JsonValueSchema")]
    pub request: Value,
}

#[cfg(feature = "analysis")]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MorrisAnalysis {
    pub model: ModelRef,
    /// Complete qspquest.morris/v0.1 request. Native unit, geometry and grid admission applies.
    #[schemars(with = "JsonValueSchema")]
    pub request: Value,
    /// Optional exact plan_run digest; refuses changed inputs or execution context.
    pub expected_plan_hash: Option<String>,
}

#[cfg(feature = "managed")]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobInput {
    /// Exact platform Run creation input, including the reviewed expected_preview_digest, destination Project and released operation.
    #[schemars(with = "JsonValueSchema")]
    pub platform_request: Value,
    /// Stable caller key for infrastructure retries; a new user rerun requires a new key.
    pub idempotency_key: String,
}
#[cfg(feature = "managed")]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobId {
    pub job_id: String,
    pub seconds: Option<u64>,
}
#[cfg(feature = "managed")]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactId {
    pub artifact_id: String,
}
#[cfg(feature = "managed")]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobPreviewInput {
    /// Exact Run preview input. Inspect returned pricing and approval_required before submission.
    #[schemars(with = "JsonValueSchema")]
    pub platform_request: Value,
}

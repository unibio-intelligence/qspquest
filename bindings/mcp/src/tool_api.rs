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
    /// Words matched as whole words (or prefixes of 5+ letters) in model ids, names, descriptions,
    /// scenario ids and output names. Domain phrases are joined: "target-mediated" = tmdd,
    /// "antibody-drug conjugate" = adc, "cytokine release" = crs, "-mab" drugs = antibody.
    #[serde(default)]
    pub query: String,
    /// Optional exact filters: model_id, version, license, has_output (an output name), tag,
    /// availability (dynamic | source_replay | inspection_only), unit_status, fit_ready (bool),
    /// fitting_supported (bool), execution_available (bool), supports (an operation such as
    /// exploratory_simulation or fit). Unknown keys are refused.
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
    /// Library model to inspect. Omit when `source` is supplied.
    #[serde(alias = "id", default)]
    pub model_id: String,
    pub version: Option<String>,
    /// Compact interface by default; full metadata and equations on request.
    pub detail: Option<ModelDetail>,
    /// Inspect inline .qsp text or model JSON (local stdio) instead of a library model,
    /// with the same card: parameters, units, covariates, dose targets and outputs.
    pub source: Option<String>,
    /// Alternatively, the same model reference other tools take: {library:{model_id, version?}},
    /// {scenario:{model_id, ...}} or {source: text}.
    pub model: Option<ModelRef>,
}
/// The dose whose amount is searched; timing and delivery stay fixed.
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DoseSearch {
    pub target: String,
    pub at: Quantity,
    pub every: Option<Quantity>,
    pub count: Option<u64>,
    pub duration: Option<Quantity>,
    pub bioavailability: Option<f64>,
    /// Smallest and largest amount to consider, in the target's dose unit.
    pub lower: Quantity,
    pub upper: Quantity,
    /// Stop when the bracket is narrower than this fraction of its upper end (default 0.01).
    pub relative_tolerance: Option<f64>,
}
/// The requirement on one readout, in the readout output's declared unit.
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DoseConstraint {
    /// One readout in the form of simulate.readouts (kind, output, window or time, threshold).
    #[schemars(with = "Readout")]
    pub readout: Value,
    /// at_most: readout value must not exceed threshold; at_least: must not fall below it.
    pub operator: String,
    pub threshold: f64,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindDose {
    pub model: ModelRef,
    pub end: Quantity,
    pub parameters: Option<std::collections::BTreeMap<String, Quantity>>,
    pub initial_states: Option<std::collections::BTreeMap<String, Quantity>>,
    pub covariates: Option<std::collections::BTreeMap<String, Quantity>>,
    /// Fixed doses given in every evaluation, for example an induction phase.
    pub doses: Option<Vec<Dose>>,
    pub search: DoseSearch,
    pub constraint: DoseConstraint,
    /// At most this many simulations, 3..40 (default 30).
    pub max_evaluations: Option<u32>,
}
/// The repeated maintenance dose; it is given at the start of every interval.
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceDose {
    pub target: String,
    pub amount: Quantity,
    /// Infusion duration; omit for a bolus.
    pub duration: Option<Quantity>,
    pub bioavailability: Option<f64>,
    pub lag: Option<Quantity>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SimulateSteadyState {
    pub model: ModelRef,
    pub dose: MaintenanceDose,
    /// Dosing interval (tau).
    pub interval: Quantity,
    /// 1..8 outputs to summarise per interval.
    pub outputs: Vec<String>,
    pub parameters: Option<std::collections::BTreeMap<String, Quantity>>,
    pub initial_states: Option<std::collections::BTreeMap<String, Quantity>>,
    pub covariates: Option<std::collections::BTreeMap<String, Quantity>>,
    /// Steady state when the change still to come in AUC, peak, minimum and trough, estimated by
    /// geometric extrapolation of the last three intervals, is at most this fraction (default 0.001).
    pub relative_tolerance: Option<f64>,
    /// Largest number of intervals to simulate (default 400, 4..5000).
    pub max_intervals: Option<u32>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariedParameter {
    /// Parameter name as listed by get_model.
    pub name: String,
    /// Range for morris (required there); ignored by local.
    pub lower: Option<Quantity>,
    pub upper: Option<Quantity>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sensitivity {
    pub model: ModelRef,
    pub end: Quantity,
    pub doses: Option<Vec<Dose>>,
    pub parameters: Option<std::collections::BTreeMap<String, Quantity>>,
    pub initial_states: Option<std::collections::BTreeMap<String, Quantity>>,
    pub covariates: Option<std::collections::BTreeMap<String, Quantity>>,
    /// 1..8 readouts in the form of simulate.readouts; each is ranked separately.
    #[schemars(with = "Vec<Readout>")]
    pub readouts: Vec<Value>,
    /// 1..12 parameters to vary.
    pub vary: Vec<VariedParameter>,
    /// local (default): central-difference elasticities d ln(readout)/d ln(parameter) at the
    /// current values. morris: elementary-effects screening over each parameter's range.
    pub method: Option<String>,
    /// local: relative step (default 0.01).
    pub relative_step: Option<f64>,
    /// morris: trajectories (default 6), levels (default 4) and seed (default 1).
    pub trajectories: Option<u32>,
    pub levels: Option<u32>,
    pub seed: Option<u64>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditModel {
    /// The model to edit: .qsp text or model JSON (for example fork_model.canonical_source).
    pub source: String,
    /// 1..64 edits applied in order; each has an "op" and its fields:
    /// add_parameter {name, value, description, transform?, fixed?};
    /// set_parameter {name, value?, description?, fixed?};
    /// add_state {name, unit, initial? | initial_expression?};
    /// set_initial {state, initial | initial_expression} (for example zero a starting value that encoded a source dose);
    /// add_term {state, expression} (added to d/dt(state); write the sign in the expression);
    /// add_flux {from?, to?, rate} (same-unit states; leaves from, enters to);
    /// add_peripheral_compartment {central, name, coordinates: amount|concentration, central_volume, volume, clearance, volume_parameter?, clearance_parameter?, description?};
    /// add_covariate {name, unit, default: {value, unit}, interpolation?};
    /// add_covariate_effect {parameter, covariate, reference: {value, unit}, form: power|linear, coefficient (number), name?, description?};
    /// add_dosing {state, amount_unit, scale?, modes?, lag?};
    /// set_dosing {state, amount_unit?, scale?, modes?, lag? ("" removes the lag)};
    /// set_rhs {state, expression} (replaces d/dt(state); the input term is kept for dosed states);
    /// add_output {name, unit, expression};
    /// rename_symbol {from, to}.
    #[schemars(with = "Vec<JsonValueSchema>")]
    pub operations: Vec<Value>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fork {
    /// Bind to a previously inspected native model card.
    pub expected_inspection_hash: Option<String>,
    #[serde(alias = "id")]
    pub model_id: String,
    pub version: Option<String>,
    /// Stored scenario to translate into the copy's names and check against the original.
    pub scenario_id: Option<String>,
    /// Run the scenario on the original and on the copy and report their largest relative
    /// difference (default true when scenario_id is given; local stdio only).
    pub verify: Option<bool>,
    /// Drop parameters with these library roles and the outputs that use them, for example
    /// ["source_check", "information_criterion", "subject_replay", "runtime"]. Needs the
    /// library's parameter metadata; structural parameters are never dropped.
    pub exclude_roles: Option<Vec<String>>,
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
    /// Optional published or expected value to check against: {value, unit (converted to the
    /// output's unit), relative_tolerance (default 0.05), absolute_tolerance (default 0),
    /// metric (required for nca: cmax, tmax, cmin, tmin, auc, cavg or terminal_half_life)}.
    /// The result row gains comparison with the relative error and within_tolerance.
    #[schemars(with = "Option<JsonValueSchema>")]
    pub reported: Option<Value>,
    /// nca only: iv_bolus or extravascular adds single-dose NCA (lambda_z, AUC to infinity,
    /// percent extrapolated, and CL and Vz with dose) taking the window start as the dose time.
    pub route: Option<String>,
    /// nca only: the dose amount with unit, for clearance and volume.
    pub dose: Option<Quantity>,
    /// nca only: linear_up_log_down (default) or linear.
    pub auc_method: Option<String>,
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
    /// Covariate values for this member; matching names replace shared covariates.
    pub covariates: Option<std::collections::BTreeMap<String, Quantity>>,
    /// Parameter changes during this regimen only; same form as simulate.parameter_changes.
    #[schemars(with = "Option<Vec<JsonValueSchema>>")]
    pub parameter_changes: Option<Vec<Value>>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Simulate {
    /// Optional: refuse before numerical work unless source, context and preparation still match plan_run.
    pub expected_plan_hash: Option<String>,
    /// {library:{model_id}}, {scenario:{model_id, scenario_id}} or {source: .qsp text or model JSON}.
    pub model: ModelRef,
    /// Advanced and normally left unset: a complete native qspquest.run/v0.1 request object. Use doses, parameters, times and readouts instead.
    /// Versioned qspquest.run/v0.1 request; optional when a scenario is selected.
    #[schemars(with = "Option<JsonValueSchema>")]
    pub request: Option<Value>,
    /// Explicit quantities, for example {"cl":{"value":2,"unit":"L/h"}}.
    pub parameters: Option<std::collections::BTreeMap<String, Quantity>>,
    pub initial_states: Option<std::collections::BTreeMap<String, Quantity>>,
    /// Covariate values by declared name with units, for example {"wt":{"value":70,"unit":"kg"}}. Covariate names given in parameters are routed here.
    pub covariates: Option<std::collections::BTreeMap<String, Quantity>>,
    /// Parameter values that change during the run: [{"name":..., "at":{"value":21,"unit":"d"}, "value":{"value":0.002,"unit":"1/d"}}]. Before the first change the parameter keeps the value from parameters, the scenario or the model default; each change holds until the next.
    #[schemars(with = "Option<Vec<JsonValueSchema>>")]
    pub parameter_changes: Option<Vec<Value>>,
    /// Doses: target (a dose target from get_model), amount and at quantities; optional every
    /// with count (total doses including the first), duration for an infusion,
    /// bioavailability and lag. Amounts use the target's declared amount unit.
    pub doses: Option<Vec<Dose>>,
    /// Sample times (quantities); default 101 evenly spaced times from zero to end. Dose times
    /// add pre and post rows.
    pub times: Option<Vec<Quantity>>,
    /// End of the run, required unless a scenario or native request supplies it.
    pub end: Option<Quantity>,
    /// diffsol_bdf (default), diffsol_tsit45, diffsol_esdirk34, diffsol_tr_bdf2 or analytic_linear_pk.
    pub solver: Option<String>,
    /// Output names to return (default: the outputs that depend on the model).
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
    /// Observation time (preferred for fit_individual); the server builds the observation grid
    /// from these times.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<Quantity>,
    /// pre or post at a dose time (default post); only with time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    /// Alternatively, the row of an explicit native request's result grid; do not mix with time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<usize>,
    pub output: String,
    pub value: Quantity,
    /// Measurement error model: additive_sd (a quantity in the output's unit) and proportional_sd
    /// (a fraction, 0 for none). Both describe the observation's standard deviation.
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
    /// Covariate values by declared name with units, for example {"wt":{"value":70,"unit":"kg"}}. Covariate names given in parameters are routed here.
    pub covariates: Option<std::collections::BTreeMap<String, Quantity>>,
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
    /// Optional paired population comparison. The same virtual subjects, with lognormal inter-individual variability, receive every regimen, and the result reports curve and exposure percentiles and paired differences against the first regimen. Requires output; save_csv, required_outputs, readouts and parameter_changes apply only to the deterministic comparison.
    #[cfg(feature = "analysis")]
    pub population: Option<PopulationComparison>,
}
/// Typical value of a parameter that varies between virtual subjects.
#[cfg(feature = "analysis")]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamedQuantity {
    /// Native model parameter name, as in get_model detail full.
    pub name: String,
    pub value: f64,
    pub unit: String,
}
#[cfg(feature = "analysis")]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PopulationComparison {
    /// Ordered typical values of the parameters that vary between subjects; their order defines omega. These names may not also be shared or member parameter overrides.
    pub parameters: Vec<NamedQuantity>,
    /// Covariance of the log-scale random effects (variances on the diagonal), in random_effects order.
    pub omega: Vec<Vec<f64>>,
    /// Parameters with random effects, 1..8 of parameters; default all of them.
    pub random_effects: Option<Vec<String>>,
    /// Virtual subjects, 1..2000; default 100.
    pub subjects: Option<u64>,
    /// Random seed; default 1009. Each subject draws from its own recorded substream.
    pub seed: Option<u64>,
    /// Threshold in the output's unit for time above threshold.
    pub threshold: Option<f64>,
    /// 1..10 increasing values in (0, 1); default 0.05, 0.5, 0.95.
    pub percentiles: Option<Vec<f64>>,
    /// Optional 1..1000 parameter-uncertainty draws, each giving every parameter in its declared unit.
    pub parameter_draws: Option<Vec<std::collections::BTreeMap<String, Quantity>>>,
    /// Aggregate simulation ceiling; the comparison needs subjects times regimens. Required on hosted servers, at most 500.
    pub max_simulations: Option<u64>,
    /// Native worker threads, 1..8 locally and 1 when hosted; results do not depend on it.
    pub workers: Option<u64>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Sample times after the dose, ascending.
    pub times: Vec<f64>,
    /// Concentrations (or any response) at those times.
    pub values: Vec<f64>,
    /// Time unit, for example h or d (required for units on the results).
    pub time_unit: Option<String>,
    /// Concentration unit, for example mg/L or ng/mL.
    pub concentration_unit: Option<String>,
    /// Optional threshold for time below and time above.
    pub threshold: Option<f64>,
    /// Single-dose noncompartmental analysis (CDISC codes: AUCLST, AUCIFO, LAMZ, LAMZHL,
    /// AUCPEO, CL(F)O, VZ(F)O, ...) runs when route is given: iv_bolus or extravascular.
    pub route: Option<String>,
    /// Dose amount with unit, for clearance and volume (divided by F when extravascular).
    pub dose: Option<Quantity>,
    /// linear_up_log_down (default) or linear.
    pub auc_method: Option<String>,
    /// Smallest number of terminal points for lambda_z (default 3).
    pub lambda_z_min_points: Option<usize>,
    /// One flag per time: true marks a concentration below the limit of quantification (its
    /// value is ignored). BLQ rows need blq_rule.
    pub blq: Option<Vec<bool>>,
    /// What to do with BLQ rows by position, as qspquest.nca:
    /// {"leading": "zero"|"error", "embedded": "error"|"exclude"|"zero", "trailing": "exclude"|"error"}.
    #[schemars(with = "Option<JsonValueSchema>")]
    pub blq_rule: Option<Value>,
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
    /// Advanced and normally left unset: a complete native qspquest.run/v0.1 request. Use end, times, doses, parameters and covariates instead.
    #[schemars(with = "Option<JsonValueSchema>")]
    pub request: Option<Value>,
    /// The parameter or covariate to vary. Omit it and set dose_index to vary a dose amount instead.
    pub parameter: Option<String>,
    /// Vary the amount of doses[dose_index] (0-based) instead of a parameter; values are then
    /// dose amounts, for example mg or mg/kg (resolved against body weight).
    pub dose_index: Option<usize>,
    pub values: Option<Vec<Quantity>>,
    pub range: Option<ScanRange>,
    pub output: Option<String>,
    /// Shared starting parameter quantities for every scan point (the scanned parameter is set per point).
    pub parameters: Option<std::collections::BTreeMap<String, Quantity>>,
    pub initial_states: Option<std::collections::BTreeMap<String, Quantity>>,
    /// Covariate values by declared name with units, for example {"wt":{"value":70,"unit":"kg"}}. Covariate names given in parameters are routed here.
    pub covariates: Option<std::collections::BTreeMap<String, Quantity>>,
    /// Friendly doses shared by every point: target, amount, at; optional every, count, duration, bioavailability, lag.
    pub doses: Option<Vec<Dose>>,
    pub times: Option<Vec<Quantity>>,
    pub end: Option<Quantity>,
    pub solver: Option<String>,
    /// Readouts evaluated for every point, in the same form as simulate.readouts.
    #[schemars(with = "Option<Vec<Readout>>")]
    pub readouts: Option<Vec<Value>>,
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
    /// Advanced: a complete native qspquest.run/v0.1 request. Normally use end, doses,
    /// parameters and covariates with observations given by time.
    #[schemars(with = "Option<JsonValueSchema>")]
    pub request: Option<Value>,
    pub end: Option<Quantity>,
    /// Friendly doses as in simulate.
    pub doses: Option<Vec<Dose>>,
    /// Fixed parameter values for the fit (fitted parameters start from these or the defaults).
    pub parameters: Option<std::collections::BTreeMap<String, Quantity>>,
    pub initial_states: Option<std::collections::BTreeMap<String, Quantity>>,
    pub covariates: Option<std::collections::BTreeMap<String, Quantity>>,
    pub solver: Option<String>,
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

#[cfg(feature = "analysis")]
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisWorkflow {
    /// Library, scenario or source model. exposure_summary and fit_diagnostics read a supplied result record and still name the model that produced it.
    pub model: ModelRef,
    /// Complete qspquest.analysis/v0.2 envelope {schema, operation, arguments}: fit_data, fit_population, visual_predictive_check, profile_likelihood, bootstrap_population, event_sensitivities, exposure_summary or fit_diagnostics. Runs use native names and units; parameters are ordered arrays of named records. Arguments match qspquest analyze and CompiledSimulation.analyze; the MCP README has a worked example.
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

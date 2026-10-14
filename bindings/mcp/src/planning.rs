//! Static request plans. Prospective preparation is metadata, never a calculation result.
use crate::{contract::JsonValueSchema, server::*};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize, Serialize, JsonSchema)]
#[schemars(extend("type" = "object"))]
#[serde(
    tag = "operation",
    content = "arguments",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PlanRequest {
    Simulate(Simulate),
    GetPatientParameters(Patient),
    CompareRegimens(Compare),
    ScanParameters(Scan),
    FitScalar(ScalarFit),
    FitIndividual(IndividualFit),
    FitPopulation(PopulationFit),
    #[cfg(feature = "analysis")]
    MorrisScreening(MorrisAnalysis),
    #[cfg(feature = "analysis")]
    SteadyState(NativeAnalysis),
}
impl PlanRequest {
    pub fn into_parts(self) -> (String, Value) {
        let value = serde_json::to_value(self).expect("typed request serializes");
        (
            value["operation"].as_str().unwrap().into(),
            value["arguments"].clone(),
        )
    }
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub enum Version {
    #[serde(rename = "qspquest.mcp-request-plan/v0.1")]
    V01,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScientificClass {
    SourceScaleUnqualified,
    SyntheticIllustrative,
    UserCandidate,
    ArchiveUnassessed,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PresetScope {
    NoStoredPreset,
    ExactStoredRequest,
    ModifiedStoredRequest,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Assessment {
    NotAssessed,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Qualification {
    pub scientific_assessment: crate::scientific::Receipt,
    pub scientific_class: ScientificClass,
    pub current_engine_and_request: Assessment,
    pub preset_scope: PresetScope,
    pub recorded_dimensional_qualification: Option<bool>,
    pub recorded_fit_ready: Option<bool>,
    pub notice: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub operation: String,
    pub server_version: String,
    pub library_version: String,
    pub model_id: Option<String>,
    pub model_version: Option<String>,
    pub archive_sha256: Option<String>,
    pub scenario_id: Option<String>,
    pub original_source_sha256: String,
    pub effective_source_sha256: String,
    pub model_metadata_sha256: String,
    pub adapter_arguments_sha256: String,
    pub effective_request_hash: String,
    pub selected_outputs: Option<Vec<String>>,
    pub deployment: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRoute {
    pub kind: String,
    pub governed: Option<crate::contract::AsyncRoute>,
    /// Planning reserves no execution slot, library version, pricing or job authorization.
    pub notice: String,
}
#[derive(Deserialize, Serialize)]
#[serde(
    tag = "kind",
    content = "plan",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum NativePreparation {
    Simulate(qspquest_core::planning::RunPlan),
    GetPatientParameters(qspquest_core::initial_values::InitialValuesPlan),
    CompareRegimens(ComparisonPreparation),
    ScanParameters(qspquest_core::planning::ScanPlan),
    FitScalar(qspquest_core::planning::ScalarFitPlan),
    FitIndividual(qspquest_core::planning::FitPlan),
    FitPopulation(qspquest_core::planning::FitPlan),
    #[cfg(feature = "analysis")]
    MorrisScreening(qspquest_core::planning::MorrisPlan),
    #[cfg(feature = "analysis")]
    SteadyState(qspquest_core::planning::PeriodicPlan),
}
impl NativePreparation {
    pub fn parse(operation: &str, value: Value) -> Result<Self, Value> {
        serde_json::from_value(json!({"kind":operation,"plan":value}))
            .map_err(|e| crate::policy::error("plan_contract", e.to_string()))
    }
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestPlan {
    pub scheduled_boundary_receipt: Option<crate::boundaries::Receipt>,
    pub initialization_receipt: Option<crate::initialization::Receipt>,
    pub precision_profile_receipt: Option<crate::precision::Receipt>,
    pub dose_event_receipts: Option<Vec<DoseEventReceipt>>,
    pub time_validation: Option<Vec<crate::core::time_validation::TimeWindow>>,
    pub required_output_receipts: Option<Vec<crate::core::required_outputs::RequiredOutputReceipt>>,
    pub override_receipt: Option<crate::core::overrides::OverrideReceipt>,
    pub output_mapping: Option<Vec<crate::contract::OutputMapping>>,
    pub schema: Version,
    /// SHA-256 of canonical JSON with this field empty. It is not authorization.
    pub plan_hash: String,
    pub context: Context,
    #[schemars(with = "JsonValueSchema")]
    pub effective_request: Value,
    #[schemars(with = "Vec<JsonValueSchema>")]
    pub row_mapping: Vec<Value>,
    pub assumptions: Vec<String>,
    pub qualification: Qualification,
    pub execution_route: ExecutionRoute,
    #[schemars(with = "JsonValueSchema")]
    pub native_preparation: NativePreparation,
}

/// A declared source coordinate is metadata, not a molecular-mass conversion.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceDoseCoordinate {
    pub native_target: String,
    pub amount_unit: String,
    pub state_unit: String,
    pub source_dose_unit: String,
    pub source_coordinate: String,
    pub scaling: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoseEventStage {
    PreparedSchedule,
    ExecutedSchedule,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoseDelivery {
    Bolus,
    Infusion,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DoseQuantity {
    pub value: f64,
    pub unit: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DoseEventReceipt {
    pub stage: DoseEventStage,
    /// Index in the native plan's complete event storage order, including resets.
    pub event_index: usize,
    pub canonical_target: String,
    pub delivery: DoseDelivery,
    pub time: DoseQuantity,
    /// Expanded amount after regimen availability, before model dosing.scale.
    pub amount_before_model_scaling: DoseQuantity,
    pub duration: Option<DoseQuantity>,
    pub target_state_unit: String,
    #[schemars(with = "JsonValueSchema")]
    pub declared_base_scale: qspquest_core::expression::Expr,
    pub archive_source_coordinate: Option<SourceDoseCoordinate>,
    pub notice: String,
}

pub fn source_dose_coordinates(
    native: &Value,
    model: &Value,
) -> Result<std::collections::BTreeMap<String, SourceDoseCoordinate>, Value> {
    let value = &native["dose_contracts"];
    if value.is_null() {
        return Ok(Default::default());
    }
    let contracts: std::collections::BTreeMap<String, SourceDoseCoordinate> =
        serde_json::from_value(value.clone()).map_err(|e| {
            crate::policy::error(
                "dose_contract",
                format!("Invalid archive dose_contracts: {e}"),
            )
        })?;
    let mut targets = std::collections::BTreeSet::new();
    let states = crate::library::display_states(model);
    for (alias, coordinate) in &contracts {
        if [
            alias,
            &coordinate.native_target,
            &coordinate.amount_unit,
            &coordinate.state_unit,
            &coordinate.source_dose_unit,
            &coordinate.source_coordinate,
            &coordinate.scaling,
        ]
        .into_iter()
        .any(|s| s.trim().is_empty() || s.len() > 512 || s.chars().any(char::is_control))
            || !targets.insert(&coordinate.native_target)
        {
            return Err(crate::policy::error(
                "dose_contract",
                "Archive dose coordinates must be bounded, nonempty and unambiguous for each native target.",
            ));
        }
        let state = states
            .iter()
            .find(|s| s["name"] == coordinate.native_target)
            .ok_or_else(|| {
                crate::policy::error(
                    "dose_contract",
                    "Archive dose coordinate refers to an absent native state.",
                )
            })?;
        if state["dosing"].is_null()
            || state["unit"] != coordinate.state_unit
            || state["dosing"]["amount_unit"] != coordinate.amount_unit
        {
            return Err(crate::policy::error(
                "dose_contract",
                "Archive dose coordinate differs from the native state's declared dosing or state units.",
            ));
        }
    }
    Ok(contracts)
}

/// Use the core's actual expanded events; never reconstruct repeats or conversions.
pub fn dose_event_receipts(
    plan: &qspquest_core::planning::RunPlan,
    model: &Value,
    native: &Value,
) -> Result<Vec<DoseEventReceipt>, Value> {
    let contracts = source_dose_coordinates(native, model)?;
    let states = crate::library::display_states(model);
    let mut receipts = Vec::new();
    for (event_index, event) in plan.events.iter().enumerate() {
        let (amount, delivery, duration) = match event.event {
            qspquest_core::Event::Bolus { amount, .. } => (amount, DoseDelivery::Bolus, None),
            qspquest_core::Event::Infusion {
                amount, duration, ..
            } => (amount, DoseDelivery::Infusion, Some(duration)),
            qspquest_core::Event::Reset { .. } => continue,
        };
        let unit = event.quantity_unit.declared.clone().ok_or_else(|| {
            crate::policy::error(
                "dose_contract",
                "Native dose input unit is not declared; inspect the complete native plan.",
            )
        })?;
        let target = event.event.target();
        let state = states.get(target).ok_or_else(|| {
            crate::policy::error(
                "dose_contract",
                "Native event target is absent from the resolved model.",
            )
        })?;
        let planned_state = plan.states.get(target).ok_or_else(|| {
            crate::policy::error(
                "dose_contract",
                "Native event target is absent from the prepared state list.",
            )
        })?;
        if state["name"] != event.target_name
            || planned_state.name != event.target_name
            || state["unit"] != planned_state.unit
        {
            return Err(crate::policy::error(
                "dose_contract",
                "Native event target differs from the resolved model.",
            ));
        }
        let target_state_unit = state["unit"]
            .as_str()
            .ok_or_else(|| {
                crate::policy::error("dose_contract", "Native target state unit is missing.")
            })?
            .to_owned();
        let declared_base_scale = serde_json::from_value(state["dosing"]["scale"].clone())
            .map_err(|e| {
                crate::policy::error(
                    "dose_contract",
                    format!("Native dosing scale is invalid: {e}"),
                )
            })?;
        let source = contracts
            .values()
            .find(|c| c.native_target == event.target_name)
            .cloned();
        if source.as_ref().is_some_and(|c| c.amount_unit != unit) {
            return Err(crate::policy::error(
                "dose_contract",
                "Expanded dose input unit differs from the archive dose coordinate.",
            ));
        }
        let quantity = |value| DoseQuantity {
            value,
            unit: plan.time_unit.clone(),
        };
        receipts.push(DoseEventReceipt {
            stage: DoseEventStage::PreparedSchedule, event_index,
            canonical_target: event.target_name.clone(), delivery,
            time: quantity(event.event.time()), amount_before_model_scaling: DoseQuantity { value: amount, unit },
            duration: duration.map(quantity), target_state_unit, declared_base_scale,
            archive_source_coordinate: source,
            notice: "The effective event amount includes regimen availability and precedes model dosing.scale. The declared base scale is an expression, not an evaluated state increment; inspect native binding phases and switches. Source labels are archive declarations and do not grant independent scientific qualification.".into(),
        });
    }
    Ok(receipts)
}
impl RequestPlan {
    pub fn seal(&mut self) -> Result<(), Value> {
        self.plan_hash.clear();
        self.plan_hash = qspquest_core::identity::canonical_hash(
            &serde_json::to_value(&*self)
                .map_err(|e| crate::policy::error("plan_contract", e.to_string()))?,
        )
        .map_err(|e| crate::policy::error("plan_contract", e.to_string()))?;
        Ok(())
    }
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanPayload {
    pub plan_hash: String,
    pub effective_request_hash: String,
    pub plan_bytes: usize,
    pub plan: Option<RequestPlan>,
    /// Local, bounded 15-minute metadata retention; read chunks and verify their digest.
    pub full_plan_resource: Option<String>,
}

/// Every member has a complete simulation plan in execution order.
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComparisonInputReceipt {
    /// Explicit native request inputs and merged friendly starting inputs before normalization.
    #[schemars(with = "JsonValueSchema")]
    pub requested_inputs: Value,
    pub requested_inputs_sha256: String,
    /// Every native parameter/covariate binding phase, including derived values, defaults, units and step times.
    pub native_parameter_contract_sha256: String,
    pub native_bound_value_hash: String,
    pub effective_request_hash: String,
    pub effective_source_sha256: String,
    pub scope: String,
}
impl ComparisonInputReceipt {
    pub(crate) fn from_plan(arguments: &Value, plan: &RequestPlan) -> Result<Self, Value> {
        let requested_inputs = json!({
            "parameters":arguments["parameters"], "initial_states":arguments["initial_states"],
            "parameter_changes":arguments["parameter_changes"],
            "native_request_inputs":{"parameters":arguments["request"]["parameters"],
                "covariates":arguments["request"]["covariates"],
                "initial_states":arguments["request"]["initial_states"],
                "covariate_changes":arguments["request"]["regimen"]["covariate_changes"]}});
        let native = serde_json::to_value(&plan.native_preparation)
            .map_err(|e| crate::policy::error("comparison_inputs", e.to_string()))?;
        let phases = native["plan"]["binding_phases"].as_array().ok_or_else(|| {
            crate::policy::error(
                "comparison_inputs",
                "A comparison member requires native simulation binding phases",
            )
        })?;
        let digest = |value: &Value| {
            qspquest_core::identity::canonical_hash(value)
                .map_err(|e| crate::policy::error("comparison_inputs", e.to_string()))
        };
        let native_bound_value_hash = native["plan"]["prospective_identity"]["bound_value_hash"]
            .as_str()
            .ok_or_else(|| {
                crate::policy::error("comparison_inputs", "A native binding identity is required")
            })?
            .to_owned();
        Ok(Self {
            requested_inputs_sha256: digest(&requested_inputs)?,
            native_parameter_contract_sha256: digest(&json!({"effective_source_sha256":plan.context.effective_source_sha256,"binding_phases":phases}))?,
            native_bound_value_hash,
            effective_request_hash: plan.context.effective_request_hash.clone(),
            effective_source_sha256: plan.context.effective_source_sha256.clone(),
            requested_inputs,
            scope: "Requested inputs precede normalization. The native parameter contract hashes every parameter/covariate binding phase with its selected source. The native bound-value identity also binds initial states. Stored scenario inputs and resolved literals remain as declared unless explicitly overridden; scientific reinitialization and qualification are not inferred.".into(),
        })
    }
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlannedRegimen {
    pub name: String,
    pub input_receipt: ComparisonInputReceipt,
    pub preparation: RequestPlan,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComparisonPreparation {
    pub regimens: Vec<PlannedRegimen>,
    pub deferred_checks: Vec<String>,
}

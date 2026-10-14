//! Versioned transport status alongside the engine's unchanged scientific payload.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

pub const RESULT_SCHEMA: &str = "qspquest.mcp-result/v1";
pub const CAPABILITY_SCHEMA: &str = "qspquest.mcp-capabilities/v1";

pub(crate) struct JsonValueSchema;
impl JsonSchema for JsonValueSchema {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "NativeJson".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::Schema::from(serde_json::Map::new())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EngineIdentity {
    pub model_content_hash: String,
    pub spec_version: String,
    pub compiler_version: String,
    pub compiler_source_sha256: String,
    pub rustc_version: String,
    pub target: String,
    pub build_profile: String,
    pub rustflags_sha256: String,
    pub bound_value_hash: String,
    pub run_request_hash: String,
    pub backend: String,
    pub backend_version: String,
    pub algorithm: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IdentityScope {
    Execution,
    Metadata,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResultIdentity {
    pub scope: IdentityScope,
    pub server_version: String,
    /// SHA-256 of the payload before adding the contract, not an engine build hash.
    pub payload_sha256: String,
    pub execution: Option<EngineIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    InlineComplete,
    ResourceLinked,
    SummaryOnly,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_binding: Option<crate::core::input_names::InputBindingFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inspection_change: Option<crate::inspection::InspectionChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_diagnostic: Option<crate::core::time_validation::InputDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_requirement: Option<crate::core::required_outputs::OutputRequirementFailure>,
    pub code: String,
    pub message: String,
    pub suggestion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AsyncRoute {
    pub kind: RouteKind,
    pub tool: String,
    pub capability_id: String,
    pub states: usize,
    pub requires_verified_session: bool,
    pub requires_platform_qualification: bool,
    pub local_execution_supported: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub enum RouteKind {
    #[serde(rename = "governed_job")]
    GovernedJob,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Success,
    ScientificFailure { failure: Failure },
    Unsupported { failure: Failure },
    AsyncRequired { failure: Failure, route: AsyncRoute },
    RequestFailure { failure: Failure },
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub enum ResultVersion {
    #[serde(rename = "qspquest.mcp-result/v1")]
    V1,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Contract {
    pub schema: ResultVersion,
    pub library_version: String,
    pub completeness: Completeness,
    pub identity: ResultIdentity,
    pub resources: Vec<String>,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ToolOutput {
    pub contract: Contract,
    pub library_version: String,
    /// The existing engine or metadata identity; contract.identity describes its scope.
    #[schemars(with = "JsonValueSchema")]
    pub identity: Value,
    #[serde(flatten)]
    pub payload: BTreeMap<String, Value>,
}

fn bounded(text: &str) -> String {
    if text.len() <= 4096 {
        return text.to_owned();
    }
    let mut end = 4096;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [diagnostic shortened]", &text[..end])
}

fn failure(error: &Value) -> Failure {
    Failure {
        input_binding: serde_json::from_value(error["input_binding"].clone()).ok(),
        inspection_change: serde_json::from_value(error["inspection_change"].clone()).ok(),
        input_diagnostic: serde_json::from_value(error["input_diagnostic"].clone()).ok(),
        output_requirement: serde_json::from_value(error["output_requirement"].clone()).ok(),
        code: bounded(error["code"].as_str().unwrap_or("request_failed")),
        message: bounded(
            error["message"]
                .as_str()
                .unwrap_or("The operation failed; inspect the error payload for details."),
        ),
        suggestion: error["suggestion"].as_str().map(bounded),
        time: error["time"].as_f64(),
        expression: error["expression"].as_str().map(bounded),
    }
}

fn outcome(value: &Value) -> Outcome {
    if let Some(error) = value.get("error") {
        let f = failure(error);
        let code = f.code.replace('_', "").to_ascii_lowercase();
        if code == "asyncrequired" {
            return match serde_json::from_value(value["execution_route"].clone()) {
                Ok(route) => Outcome::AsyncRequired { failure: f, route },
                Err(_) => Outcome::RequestFailure {
                    failure: Failure {
                        input_binding: None,
                        inspection_change: None,
                        input_diagnostic: None,
                        output_requirement: None,
                        code: "invalid_async_route".into(),
                        message: "Async routing metadata is incomplete; no job was submitted."
                            .into(),
                        suggestion: None,
                        time: None,
                        expression: None,
                    },
                },
            };
        }
        if code.starts_with("unsupported")
            || code.ends_with("unsupported")
            || code == "sandboxunavailable"
            || code == "modelunavailable"
        {
            return Outcome::Unsupported { failure: f };
        }
        if matches!(
            code.as_str(),
            "domain" | "invariant" | "solver" | "workbudget" | "outputbudget"
        ) {
            return Outcome::ScientificFailure { failure: f };
        }
        return Outcome::RequestFailure { failure: f };
    }
    if value["scientific_status"]["converged"] == false
        || value["schema"] == "qspquest.scalar-fit-result/v0.1"
            && value["status"]
                .as_str()
                .is_some_and(|status| status != "converged")
    {
        return Outcome::ScientificFailure { failure: Failure {
            input_binding: None,
            inspection_change: None,
            input_diagnostic: None,
            output_requirement: None,
            code: "nonconverged".into(),
            message: "The fit did not converge. Returned estimates are diagnostic candidates.".into(),
            suggestion: Some("Inspect convergence, diagnostics and active bounds before interpreting estimates or uncertainty.".into()),
            time: None,
            expression: None,
        } };
    }
    Outcome::Success
}

pub fn attach(value: &mut Value, library_version: &str, capped: bool) {
    // A compact response may omit the rows that originally carried these links.
    let retained_resources: Vec<String> = if capped {
        value["contract"]["resources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|uri| uri.as_str().map(str::to_owned))
            .collect()
    } else {
        Vec::new()
    };
    value
        .as_object_mut()
        .expect("tool payload is an object")
        .remove("contract");
    value["library_version"] = json!(library_version);
    let execution = serde_json::from_value::<EngineIdentity>(value["identity"].clone()).ok();
    let mut resources = Vec::new();
    for key in [
        "full_result_resource",
        "full_model_details_resource",
        "inspection_index_resource",
        "full_plan_resource",
    ] {
        if let Some(uri) = value[key].as_str() {
            resources.push(uri.to_owned());
        }
    }
    for row in value["regimens"].as_array().into_iter().flatten() {
        if let Some(uri) = row["result"]["full_result_resource"].as_str() {
            resources.push(uri.to_owned());
        }
    }
    for row in value["local_job"]["completed"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let Some(uri) = row["artifact"]["uri"].as_str() {
            resources.push(uri.to_owned());
        }
    }
    for uri in retained_resources {
        if !resources.contains(&uri) {
            resources.push(uri);
        }
    }
    let completeness = if !resources.is_empty() {
        Completeness::ResourceLinked
    } else if capped
        || matches!(
            value["trajectory_kind"].as_str(),
            Some("summary_only" | "compact_preview")
        )
        || value["results"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|row| {
                matches!(
                    row["result"]["trajectory_kind"].as_str(),
                    Some("summary_only" | "compact_preview")
                )
            })
        || value["regimens"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|row| {
                matches!(
                    row["result"]["trajectory_kind"].as_str(),
                    Some("summary_only" | "compact_preview")
                )
            })
    {
        Completeness::SummaryOnly
    } else {
        Completeness::InlineComplete
    };
    let contract = Contract {
        schema: ResultVersion::V1,
        library_version: library_version.into(),
        completeness,
        identity: ResultIdentity {
            scope: if execution.is_some() {
                IdentityScope::Execution
            } else {
                IdentityScope::Metadata
            },
            server_version: env!("CARGO_PKG_VERSION").into(),
            payload_sha256: crate::library::hash(value.to_string().as_bytes()),
            execution,
        },
        resources,
        outcome: outcome(value),
    };
    value["contract"] = serde_json::to_value(contract).expect("typed contract serializes");
}

pub fn output_schema() -> Arc<serde_json::Map<String, Value>> {
    schema::<ToolOutput>().as_object().unwrap().clone().into()
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub enum CapabilityVersion {
    #[serde(rename = "qspquest.mcp-capabilities/v1")]
    V1,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildFeatures {
    pub analysis: bool,
    pub managed: bool,
    pub sandbox: bool,
    pub widget: bool,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    pub mode: String,
    pub profile: crate::deployment::Profile,
    pub verified_session_resolver_ready: bool,
    pub browser_origin_validation: bool,
    pub gateway_tls_declared: bool,
    pub proxy_peer_restriction: bool,
    pub csv_export_enabled: bool,
    pub local_result_artifacts_enabled: bool,
    pub local_jobs_enabled: bool,
    pub sandbox_configured: bool,
    pub user_source_execution: bool,
    pub governed_jobs_require_verified_session: bool,
    pub governed_backend_configured: bool,
    /// Compiled adapter only; each upstream artifact must declare its own retained-byte contract.
    pub governed_artifact_bytes_adapter: bool,
    pub widget_resource_available: bool,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub input_bytes: usize,
    pub encoded_response_bytes: usize,
    pub compute_slots: usize,
    pub hosted_deadline_ms: Option<u64>,
    pub local_result_retention_seconds: Option<u64>,
    pub local_result_entries: Option<usize>,
    pub local_result_bytes_per_entry: Option<usize>,
    pub local_artifact_retention_seconds: Option<u64>,
    pub local_artifact_entries: Option<usize>,
    pub local_artifact_bytes_per_entry: Option<u64>,
    pub local_artifact_total_bytes: Option<u64>,
    pub local_artifact_chunk_bytes: Option<usize>,
    pub local_artifact_receipt_entries: Option<usize>,
    pub local_artifact_expiration_marker_seconds: Option<u64>,
    pub local_job_active_entries: Option<usize>,
    pub local_job_retained_entries: Option<usize>,
    pub local_job_work_units: Option<usize>,
    pub local_job_retention_seconds: Option<u64>,
    pub local_job_expiration_marker_seconds: Option<u64>,
    pub managed_artifact_chunk_bytes: Option<u64>,
    pub managed_artifact_bytes_per_entry: Option<u64>,
    pub managed_artifact_max_retention_seconds: Option<u64>,
    pub managed_artifact_retrieval_deadline_ms: Option<u64>,
    pub inline_regimens: usize,
    pub inline_scan_points: usize,
    pub population_subjects: usize,
    pub population_observations: usize,
}
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDocument {
    pub schema: CapabilityVersion,
    pub result_schema: ResultVersion,
    pub server_version: String,
    pub tools: Vec<String>,
    pub target: String,
    pub capability_catalog_uri: String,
    pub capability_registry_uri: String,
    pub capability_query_tool: String,
    pub features: BuildFeatures,
    pub deployment: Deployment,
    pub limits: Limits,
    pub library_version: String,
    pub model_records: usize,
    pub availability_counts: Option<AvailabilityCounts>,
    pub current_engine_qualification: ArchiveQualification,
    pub qualification_notice: String,
}

pub fn capability_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(CapabilityDocument)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn core_diagnostics_survive_typed_and_bounded_failure_records() {
        let diagnostic = crate::core::compute::core_error(qspquest_modeling::Error {
            code: qspquest_modeling::ErrorCode::Unit,
            message: "Observation unit differs".into(),
            time: Some(2.5),
            expression: Some("subjects[1].observations[0].value".into()),
        });
        assert_eq!(diagnostic["code"], "Unit");
        let mut result = json!({"error":diagnostic});
        attach(&mut result, "fixture", false);
        assert_eq!(result["contract"]["failure"]["time"], 2.5);
        assert_eq!(
            result["contract"]["failure"]["expression"],
            "subjects[1].observations[0].value"
        );
        assert!(result["contract"]["failure"]["suggestion"]
            .as_str()
            .unwrap()
            .contains("declared by the model"));
        let _: Failure = serde_json::from_value(result["contract"]["failure"].clone()).unwrap();
        let mut large = json!({"error":{"code":"Domain","message":"domain","expression":"é".repeat(10000),"time":2.5}});
        attach(&mut large, "fixture", true);
        assert!(
            large["contract"]["failure"]["expression"]
                .as_str()
                .unwrap()
                .len()
                < 4200
        );
    }
    #[test]
    fn previews_without_a_retrievable_grid_are_summary_only() {
        for mut value in [
            json!({"trajectory_kind":"compact_preview"}),
            json!({"regimens":[{"result":{"trajectory_kind":"summary_only"}}]}),
        ] {
            attach(&mut value, "library-pin", false);
            assert_eq!(value["contract"]["completeness"], "summary_only");
            assert_eq!(value["contract"]["status"], "success");
        }
    }
    #[test]
    fn local_job_schemas_keep_metadata_false_and_do_not_replace_error_contracts() {
        let base = output_schema();
        for tool in [
            "submit_local_job",
            "get_local_job",
            "cancel_local_job",
            "resume_local_job",
        ] {
            let schema = output_schema_for(tool);
            assert_eq!(schema["$defs"]["Failure"], base["$defs"]["Failure"]);
            assert_eq!(
                schema["$defs"]["LocalJobDescriptor"]["properties"]["scientific_result"]["const"],
                false
            );
            assert_eq!(
                schema["$defs"]["LocalJobDescriptor"]["properties"]["schema"]["const"],
                "qspquest.local-job/v1"
            );
            assert_eq!(
                schema["$defs"]["LocalJobFailure"]["additionalProperties"],
                false
            );
        }
    }
    #[test]
    fn failure_routing_and_convergence_are_distinct_from_success() {
        for (value, expected) in [
            (
                json!({"error":{"code":"Domain","message":"invalid model domain"}}),
                "scientific_failure",
            ),
            (
                json!({"error":{"code":"unsupported_solver","message":"solver withheld"}}),
                "unsupported",
            ),
            (
                json!({"error":{"code":"invalid_input","message":"invalid input"}}),
                "request_failure",
            ),
            (
                json!({"scientific_status":{"converged":false},"fit":{"estimates":[1]}}),
                "scientific_failure",
            ),
            (json!({"scientific_status":{"converged":true}}), "success"),
        ] {
            let mut value = value;
            value["identity"] = json!({"backend":"metadata"});
            attach(&mut value, "library-pin", false);
            assert_eq!(value["contract"]["status"], expected);
            let _: ToolOutput = serde_json::from_value(value).unwrap_or_else(|_| panic!("base output requires legacy identity and library fields supplied by Server::response"));
        }
    }
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionClass {
    Dynamic,
    SourceReplay,
    InspectionOnly,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PublicationReview {
    Pending,
    Unrecorded,
    Approved,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnitStatus {
    PhysicallyQualified,
    SourceScaleUnqualified,
    Unqualified,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub enum ArchiveQualification {
    #[serde(rename = "not_assessed")]
    NotAssessed,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct AvailabilityDescription {
    pub schema: String,
    pub model_id: String,
    pub archive_version: String,
    pub selected_version: String,
    pub archive_sha256: String,
    pub availability: ExecutionClass,
    pub publication_review: PublicationReview,
    pub preset_ids: Vec<String>,
    pub unit_status: UnitStatus,
    pub fitting_supported: bool,
    pub fit_ready: bool,
    pub evidence_id: String,
    pub current_engine_qualification: ArchiveQualification,
    pub selected: bool,
    pub supported_operations: Vec<String>,
    pub blockers: Vec<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct AvailabilityCounts {
    pub selected_model_identities: usize,
    pub versioned_archives: usize,
    pub selected_preset_requests: usize,
    pub all_archive_preset_requests: usize,
    pub selected_availability: BTreeMap<String, usize>,
    pub selected_fitting_supported: usize,
    pub selected_fit_ready: usize,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ModelMetadata {
    pub id: String,
    pub version: String,
    pub fitting_supported: Option<bool>,
    pub fit_ready: Option<bool>,
    pub scenario_ids: Option<Vec<String>>,
    pub availability: Option<AvailabilityDescription>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct DiscoveryPayload {
    pub models: Vec<DiscoveryModel>,
    pub total: usize,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub availability_counts: Option<AvailabilityCounts>,
    /// Authorized provider metadata with a closed MCP discovery projection.
    pub platform_models: Option<Vec<crate::discovery::PlatformDiscoveryModel>>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct DiscoveryModel {
    #[serde(flatten)]
    pub metadata: ModelMetadata,
    pub execution_available: bool,
    pub scenario_count: usize,
    pub discovery: crate::discovery::ModelDiscovery,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ScenarioPayload {
    pub precision_profile_receipt: Option<crate::precision::Receipt>,
    /// Deliberate native engine-request escape hatch; read qspquest://schema/run.
    pub request: BTreeMap<String, Value>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ValidationPayload {
    pub valid: bool,
    pub model_content_hash: String,
    /// Native model object; its full engine schema is qspquest://schema/model.
    pub model: BTreeMap<String, Value>,
    pub canonical_source: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ParameterValue {
    pub name: String,
    pub value: f64,
    pub unit: String,
    pub source: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct CountNotice {
    pub count: usize,
    pub note: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ParametersUsed {
    Values(Vec<ParameterValue>),
    Notice(CountNotice),
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct EventValues {
    pub pre_dose: f64,
    pub post_dose: f64,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum PointValue {
    Number(f64),
    Event(EventValues),
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct PatientPayload {
    pub scientific_assessment: Option<crate::scientific::Receipt>,
    pub precision_profile_receipt: Option<crate::precision::Receipt>,
    pub time_validation: Option<Vec<crate::core::time_validation::TimeWindow>>,
    pub override_receipt: Option<crate::core::overrides::OverrideReceipt>,
    pub override_receipt_notice: Option<String>,
    pub output_mapping_notice: Option<String>,
    pub output_mapping: Option<Vec<OutputMapping>>,
    pub full_result_resource: Option<String>,
    pub result_artifact: Option<crate::artifacts::Descriptor>,
    pub patient_quantities: BTreeMap<String, PointValue>,
    pub patient_units: BTreeMap<String, String>,
    pub time: f64,
    pub time_unit: String,
    pub plan_hash: Option<String>,
    pub effective_request_hash: String,
    #[schemars(with = "JsonValueSchema")]
    pub effective_initial_values: Value,
    pub qualification: crate::planning::Qualification,
    pub parameters_used: ParametersUsed,
    pub notes: Option<Vec<String>>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct NcaSummary {
    pub cmax: f64,
    pub tmax: f64,
    pub cmin: f64,
    pub tmin: f64,
    pub auc: f64,
    pub cavg: f64,
    pub terminal_half_life: Option<f64>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct Crossing {
    pub time: f64,
    pub direction: String,
    pub at_dose_event: bool,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ReadoutWindow {
    pub output: String,
    pub from: f64,
    pub to: f64,
    pub time_unit: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadoutResult {
    Nca {
        #[serde(flatten)]
        window: ReadoutWindow,
        value: NcaSummary,
        note: String,
    },
    Auc {
        #[serde(flatten)]
        window: ReadoutWindow,
        value: f64,
        unit: String,
    },
    Min {
        #[serde(flatten)]
        window: ReadoutWindow,
        value: f64,
        time: f64,
    },
    Max {
        #[serde(flatten)]
        window: ReadoutWindow,
        value: f64,
        time: f64,
    },
    TimeBelow {
        #[serde(flatten)]
        window: ReadoutWindow,
        threshold: f64,
        value: f64,
        unit: String,
    },
    TimeAbove {
        #[serde(flatten)]
        window: ReadoutWindow,
        threshold: f64,
        value: f64,
        unit: String,
    },
    FirstCrossing {
        #[serde(flatten)]
        window: ReadoutWindow,
        threshold: f64,
        value: Option<Crossing>,
    },
    Trough {
        output: String,
        at: f64,
        value: f64,
        side: String,
        pre: Option<f64>,
        post: Option<f64>,
    },
    Value {
        output: String,
        at: f64,
        value: f64,
        side: String,
        pre: Option<f64>,
        post: Option<f64>,
    },
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ProfilePayload {
    pub results: Vec<ReadoutResult>,
    pub method: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct TimeValue {
    pub time: f64,
    pub value: f64,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct SampledSummary {
    pub output: String,
    pub unit: String,
    pub sampled_max: f64,
    pub sampled_tmax: f64,
    pub sampled_min: f64,
    pub trapezoidal_auc: f64,
    pub auc_unit: String,
    pub pre_event_samples: Option<Vec<TimeValue>>,
    pub interpretation: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct Sampling {
    pub original_points: usize,
    pub returned_points: usize,
    pub event_sides_preserved: bool,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryKind {
    CompactPreview,
    FullRequestedGrid,
    SummaryOnly,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ScientificStatus {
    pub converged: bool,
    pub interpretation: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SavedCsv {
    pub path: String,
    pub relative_path: String,
    pub table_sha256: String,
    pub manifest_path: String,
    pub relative_manifest_path: String,
    pub manifest_sha256: String,
    pub manifest_schema: crate::table::Version,
    pub rows: usize,
    pub columns: Vec<String>,
    pub overwrite: bool,
    pub note: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct SimulationFields {
    pub scientific_assessment_notice: Option<String>,
    pub scientific_assessment: Option<crate::scientific::Receipt>,
    pub scheduled_boundary_receipt_notice: Option<String>,
    pub scheduled_boundary_receipt: Option<crate::boundaries::Receipt>,
    pub initialization_receipt: Option<crate::initialization::Receipt>,
    pub initialization_receipt_notice: Option<String>,
    pub precision_profile_receipt: Option<crate::precision::Receipt>,
    pub dose_event_receipts: Option<Vec<crate::planning::DoseEventReceipt>>,
    pub dose_event_receipt_notice: Option<String>,
    pub required_output_receipts: Option<Vec<crate::core::required_outputs::RequiredOutputReceipt>>,
    pub required_output_notice: Option<String>,
    pub time_validation: Option<Vec<crate::core::time_validation::TimeWindow>>,
    pub override_receipt: Option<crate::core::overrides::OverrideReceipt>,
    pub override_receipt_notice: Option<String>,
    pub output_mapping_notice: Option<String>,
    pub output_mapping: Option<Vec<OutputMapping>>,
    pub plan_hash: Option<String>,
    pub summary: Option<Vec<SampledSummary>>,
    pub sampling: Option<Sampling>,
    pub trajectory_kind: Option<TrajectoryKind>,
    pub trajectory_notice: Option<String>,
    pub full_result_resource: Option<String>,
    pub result_artifact: Option<crate::artifacts::Descriptor>,
    pub run_id: Option<String>,
    pub effective_request_hash: Option<String>,
    pub readouts: Option<ProfilePayload>,
    pub values_at_requested_times: Option<Vec<BTreeMap<String, PointValue>>>,
    pub saved_csv: Option<SavedCsv>,
    pub parameters_used: Option<ParametersUsed>,
    pub notes: Option<Vec<String>>,
    pub scientific_status: Option<ScientificStatus>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct RegimenFields {
    pub identity: EngineIdentity,
    #[serde(flatten)]
    pub fields: SimulationFields,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct RegimenRecord {
    pub name: String,
    pub input_receipt: crate::planning::ComparisonInputReceipt,
    pub result: RegimenFields,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ComparisonPayload {
    pub full_result_resource: Option<String>,
    pub result_artifact: Option<crate::artifacts::Descriptor>,
    pub plan_hash: String,
    pub effective_request_hash: String,
    pub regimens: Vec<RegimenRecord>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct InterfaceParameter {
    pub name: String,
    pub unit: Option<String>,
    pub default: Option<f64>,
    pub description: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct InterfaceName {
    pub name: String,
    pub unit: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct InterfaceDose {
    pub target: String,
    pub amount_unit: String,
    pub modes: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
pub struct OutputAlias {
    pub name: String,
    pub declared_source_unit: Option<String>,
    pub analyte: Option<String>,
    pub compartment: Option<String>,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
pub struct OutputDescriptor {
    pub canonical_output: String,
    pub returned_column: String,
    pub returned_unit: String,
    pub request_aliases: Vec<OutputAlias>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct OutputMapping {
    pub requested_output: String,
    pub canonical_output: String,
    pub returned_column: String,
    pub returned_unit: String,
    pub declared_source_unit: Option<String>,
    pub analyte: Option<String>,
    pub compartment: Option<String>,
    /// Key in patient_quantities for initial-value evaluation; null for trajectories.
    pub returned_quantity: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct OutputContract {
    pub output_count: usize,
    pub descriptors_complete: bool,
    pub descriptors: Vec<OutputDescriptor>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ModelInterface {
    pub output_contract: OutputContract,
    pub parameters: Vec<InterfaceParameter>,
    pub covariates: Vec<InterfaceParameter>,
    pub states: Vec<InterfaceName>,
    pub outputs: Vec<String>,
    pub dose_targets: Vec<InterfaceDose>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CardDetail {
    Compact,
    Full,
    FullResource,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ModelCardPayload {
    pub scientific_assessment: Option<crate::scientific::Declaration>,
    pub protocol_initialization: Option<crate::initialization::Declaration>,
    pub precision_profiles: Option<crate::precision::Summary>,
    pub precision_profile_manifest: Option<crate::precision::Manifest>,
    pub inspection: Option<crate::inspection::Index>,
    pub inspection_index_resource: Option<String>,
    pub model_page: Option<crate::inspection::Page>,
    pub model_selection: Option<crate::inspection::Selection>,
    pub discovery: Option<crate::discovery::ModelDiscovery>,
    pub metadata: Option<ModelMetadata>,
    pub detail: Option<CardDetail>,
    pub execution_available: Option<bool>,
    pub scenario_ids: Option<Vec<String>>,
    pub full_model_details_resource: Option<String>,
    pub interface: Option<ModelInterface>,
    /// Authorized platform metadata retains its own schema and qualification.
    pub platform_model: Option<BTreeMap<String, Value>>,
    pub platform_descriptor: Option<PlatformCatalogDescriptor>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct PlatformCatalogDescriptor {
    /// Identifier requested from the authorized catalog; aliases are not normalized here.
    pub model_id: String,
    pub native_projection: bool,
    pub execution_route: PlatformExecutionRoute,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlatformExecutionRoute {
    GovernedJob,
}

pub(crate) fn schema<T: JsonSchema>() -> Value {
    let mut value = serde_json::to_value(schemars::schema_for!(T)).unwrap();
    // The core uses another schema-generator version; retain its exact closed
    // expression schema instead of widening a typed dosing expression to JSON.
    if value["$defs"]["DoseEventReceipt"].is_object() {
        let mut expression = native_schema(
            include_str!("../assets/expression.schema.json"),
            "DoseScale",
        );
        let definitions = expression.as_object_mut().unwrap().remove("$defs").unwrap();
        value["$defs"]
            .as_object_mut()
            .unwrap()
            .extend(definitions.as_object().unwrap().clone());
        value["$defs"]["DoseEventReceipt"]["properties"]["declared_base_scale"] = expression;
    }
    if value["$defs"]["NativeRecord"].is_object() {
        let mut native =
            native_schema(include_str!("../assets/model.schema.json"), "InspectionDoc");
        let definitions = native.as_object_mut().unwrap().remove("$defs").unwrap();
        value["$defs"]
            .as_object_mut()
            .unwrap()
            .extend(definitions.as_object().unwrap().clone());
        for variant in value["$defs"]["NativeRecord"]["oneOf"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            let kind = variant["properties"]["kind"]["const"].as_str().or_else(|| {
                variant["properties"]["kind"]["enum"]
                    .as_array()
                    .and_then(|values| values.first())
                    .and_then(Value::as_str)
            });
            let name = match kind {
                Some("parameter") => "Parameter",
                Some("covariate") => "Covariate",
                Some("individual") => "IndividualParameter",
                Some("definition") => "Definition",
                Some("state") => "State",
                Some("fixed_switch") => "FixedSwitch",
                Some("dose_history") => "DoseHistoryBinding",
                Some("dose_switch") => "DoseSwitch",
                Some("linear_pk") => "LinearPkDefinition",
                _ => continue,
            };
            variant["properties"]["data"] = json!({"$ref":format!("#/$defs/InspectionDoc_{name}")});
        }
        value["$defs"]["OutputData"]["properties"]["native"] =
            json!({"$ref":"#/$defs/InspectionDoc_Output"});
    }
    value
}

/// Namespace generated engine and adapter schemas in the MCP schema's $defs
/// namespace. Native nested records remain strict; only adapter-extended roots
/// and scan trajectories permit the additional MCP display fields.
fn native_schema(text: &str, prefix: &str) -> Value {
    fn rewrite(value: &mut Value, prefix: &str) {
        match value {
            Value::Object(map) => {
                if let Some(Value::String(reference)) = map.get_mut("$ref") {
                    if let Some(name) = reference
                        .strip_prefix("#/definitions/")
                        .or_else(|| reference.strip_prefix("#/$defs/"))
                    {
                        *reference = format!("#/$defs/{prefix}_{name}");
                    }
                }
                if let Some(Value::Array(items)) = map.get("items").cloned() {
                    let count = items.len();
                    map.insert("prefixItems".into(), Value::Array(items));
                    map.insert("items".into(), json!({}));
                    if map.remove("additionalItems") == Some(json!(false)) {
                        map.insert("maxItems".into(), json!(count));
                    }
                }
                for child in map.values_mut() {
                    rewrite(child, prefix);
                }
            }
            Value::Array(values) => values.iter_mut().for_each(|v| rewrite(v, prefix)),
            _ => {}
        }
    }
    let mut value: Value = serde_json::from_str(text).expect("generated native schema");
    let object = value.as_object_mut().unwrap();
    for key in [
        "$schema",
        "additionalProperties",
        "x-qspquest-spec-version",
        "x-qspquest-validation",
    ] {
        object.remove(key);
    }
    let mut definitions = object
        .remove("definitions")
        .or_else(|| object.remove("$defs"))
        .unwrap_or(json!({}));
    if let Some(run) = definitions
        .get_mut("RunResult")
        .and_then(Value::as_object_mut)
    {
        run.remove("additionalProperties");
    }
    let definitions = definitions
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, child)| (format!("{prefix}_{name}"), child.clone()))
        .collect();
    object.insert("$defs".into(), Value::Object(definitions));
    rewrite(&mut value, prefix);
    value
}
fn merge_payload(target: &mut Value, mut supplement: Value) {
    let supplement = supplement.as_object_mut().unwrap();
    if let Some(Value::Object(definitions)) = supplement.remove("$defs") {
        target
            .as_object_mut()
            .unwrap()
            .entry("$defs")
            .or_insert(json!({}))
            .as_object_mut()
            .unwrap()
            .extend(definitions);
    }
    if let Some(Value::Object(properties)) = supplement.remove("properties") {
        target["properties"]
            .as_object_mut()
            .unwrap()
            .extend(properties);
    }
}

fn plan_payload_schema() -> Value {
    let mut payload = schema::<crate::planning::PlanPayload>();
    // The adapter envelope adds contract, library_version and metadata identity.
    payload
        .as_object_mut()
        .unwrap()
        .remove("additionalProperties");
    let mut alternatives = Vec::new();
    for (kind, text, prefix) in [
        (
            "simulate",
            include_str!("../assets/run-plan.schema.json"),
            "PlannedRun",
        ),
        (
            "get_patient_parameters",
            include_str!("../assets/initial-values-plan.schema.json"),
            "PlannedInitialValues",
        ),
        (
            "scan_parameters",
            include_str!("../assets/scan-plan.schema.json"),
            "PlannedScan",
        ),
        #[cfg(feature = "analysis")]
        (
            "morris_screening",
            include_str!("../assets/morris-plan.schema.json"),
            "PlannedMorris",
        ),
        #[cfg(feature = "analysis")]
        (
            "steady_state",
            include_str!("../assets/periodic-plan.schema.json"),
            "PlannedPeriodic",
        ),
        (
            "fit_scalar",
            include_str!("../assets/scalar-fit-plan.schema.json"),
            "PlannedScalarFit",
        ),
        (
            "fit_individual",
            include_str!("../assets/fit-plan.schema.json"),
            "PlannedIndividualFit",
        ),
        (
            "fit_population",
            include_str!("../assets/fit-plan.schema.json"),
            "PlannedPopulationFit",
        ),
    ] {
        let mut native = native_schema(text, prefix);
        let definitions = native.as_object_mut().unwrap().remove("$defs").unwrap();
        payload["$defs"]
            .as_object_mut()
            .unwrap()
            .extend(definitions.as_object().unwrap().clone());
        native["additionalProperties"] = json!(false);
        alternatives.push(json!({"type":"object","additionalProperties":false,
            "required":["kind","plan"],"properties":{"kind":{"const":kind},"plan":native}}));
    }
    let mut comparison = native_schema(
        &schema::<crate::planning::ComparisonPreparation>().to_string(),
        "Compared",
    );
    let simulation = alternatives
        .iter()
        .find(|value| value["properties"]["kind"]["const"] == "simulate")
        .unwrap()
        .clone();
    comparison["$defs"]["Compared_RequestPlan"]["properties"]["native_preparation"] = simulation;
    let definitions = comparison.as_object_mut().unwrap().remove("$defs").unwrap();
    payload["$defs"]
        .as_object_mut()
        .unwrap()
        .extend(definitions.as_object().unwrap().clone());
    comparison["additionalProperties"] = json!(false);
    alternatives.push(json!({"type":"object","additionalProperties":false,
        "required":["kind","plan"],"properties":{"kind":{"const":"compare_regimens"},"plan":comparison}}));
    payload["$defs"]["RequestPlan"]["properties"]["native_preparation"] =
        json!({"oneOf":alternatives});
    payload
}

pub fn output_schema_for(tool: &str) -> Arc<serde_json::Map<String, Value>> {
    let mut base = output_schema().as_ref().clone();
    let native = match tool {
        "simulate" => Some((include_str!("../assets/result.schema.json"), "Run")),
        "fit_scalar" => Some((
            include_str!("../assets/scalar-fit-result.schema.json"),
            "Scalar",
        )),
        "fit_individual" | "fit_population" => {
            Some((include_str!("../assets/fit-result.schema.json"), "Fit"))
        }
        "scan_parameters" => Some((include_str!("../assets/scan-result.schema.json"), "Scan")),
        "steady_state" => Some((
            include_str!("../assets/steady-state-result.schema.json"),
            "SteadyState",
        )),
        "morris_screening" => Some((
            include_str!("../assets/morris-result.schema.json"),
            "Morris",
        )),
        _ => None,
    };
    let mut payload = if let Some((text, prefix)) = native {
        let mut value = native_schema(text, prefix);
        merge_payload(&mut value, schema::<SimulationFields>());
        Some(value)
    } else {
        match tool {
            "submit_local_job" | "get_local_job" | "cancel_local_job" | "resume_local_job" => {
                Some(schema::<crate::local_jobs::Payload>())
            }
            "plan_run" => Some(plan_payload_schema()),
            "get_capability" => Some(schema::<crate::capability::Answer>()),
            "search_models" => Some(schema::<DiscoveryPayload>()),
            "get_model" => Some(schema::<ModelCardPayload>()),
            "get_scenario_request" => Some(schema::<ScenarioPayload>()),
            "validate_model" | "model_template" => Some(schema::<ValidationPayload>()),
            "summarize_profile" => Some(schema::<ProfilePayload>()),
            "format_response" => Some(schema::<crate::response_format::FormatPayload>()),
            "get_patient_parameters" => Some(schema::<PatientPayload>()),
            "compare_regimens" => Some(schema::<ComparisonPayload>()),
            #[cfg(feature = "managed")]
            "preview_job" => Some(schema::<crate::jobs::PreviewPayload>()),
            #[cfg(feature = "managed")]
            "submit_simulation" | "submit_population_fit" | "submit_scan" => {
                Some(schema::<crate::jobs::SubmissionPayload>())
            }
            #[cfg(feature = "managed")]
            "get_job" | "wait_for_job" => Some(schema::<crate::jobs::JobPayload>()),
            #[cfg(feature = "managed")]
            "get_artifact" => Some(schema::<crate::jobs::ArtifactPayload>()),
            _ => None,
        }
    };
    if let Some(value) = payload.as_mut() {
        if matches!(
            tool,
            "submit_local_job" | "get_local_job" | "cancel_local_job" | "resume_local_job"
        ) {
            value["$defs"]["LocalJobDescriptor"]["properties"]["schema"]["const"] =
                json!("qspquest.local-job/v1");
            value["$defs"]["LocalJobDescriptor"]["properties"]["scientific_result"]["const"] =
                json!(false);
        }
        if tool == "get_capability" {
            // The adapter adds the MCP envelope; nested capability records stay closed.
            value
                .as_object_mut()
                .unwrap()
                .remove("additionalProperties");
        }
        if tool == "get_model" {
            value["$defs"]["PlatformCatalogDescriptor"]["properties"]["native_projection"]
                ["const"] = json!(false);
            let mut native =
                native_schema(include_str!("../assets/run.schema.json"), "SelectedRequest");
            if let Some(definitions) = native.as_object_mut().unwrap().remove("$defs") {
                value["$defs"]
                    .as_object_mut()
                    .unwrap()
                    .extend(definitions.as_object().unwrap().clone());
            }
            native["additionalProperties"] = json!(false);
            value["$defs"]["SelectedScenario"]["properties"]["request"] = native;
        }
        let nested = match tool {
            "get_patient_parameters" => Some((
                "effective_initial_values",
                include_str!("../assets/initial-values-result.schema.json"),
                "InitialValuesResult",
            )),
            "get_scenario_request" => Some((
                "request",
                include_str!("../assets/run.schema.json"),
                "Request",
            )),
            "validate_model" | "model_template" => Some((
                "model",
                include_str!("../assets/model.schema.json"),
                "Model",
            )),
            _ => None,
        };
        if let Some((field, text, prefix)) = nested {
            let mut native = native_schema(text, prefix);
            if let Some(definitions) = native.as_object_mut().unwrap().remove("$defs") {
                value
                    .as_object_mut()
                    .unwrap()
                    .entry("$defs")
                    .or_insert(json!({}))
                    .as_object_mut()
                    .unwrap()
                    .extend(definitions.as_object().unwrap().clone());
            }
            native["additionalProperties"] = json!(false);
            value["properties"][field] = native;
        }
    }
    if let Some(Value::Object(ref mut payload)) = payload {
        payload.remove("$schema");
        if let Some(Value::Object(definitions)) = payload.remove("$defs") {
            base.entry("$defs")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .unwrap()
                .extend(definitions);
        }
        // Errors have no scientific payload. Capping may omit fields, but any
        // fields retained must still have their declared type.
        let required = payload.remove("required").unwrap_or(json!([]));
        let mut complete_fields = json!({"required":required});
        if tool == "get_model" {
            // A page has its own content-bound context. Requiring a model card
            // on this branch would repeat the potentially large discovery data.
            payload.insert("anyOf".into(), json!([
                {"required":["model_page","inspection_index_resource"],
                 "properties":{"model_page":{"not":{"type":"null"}},
                               "inspection_index_resource":{"type":"string"}},
                 "not":{"anyOf":[{"required":["discovery"]},{"required":["metadata"]},{"required":["platform_model"]},{"required":["model_selection"]}]}},
                {"required":["model_selection","inspection_index_resource"],
                 "properties":{"model_selection":{"not":{"type":"null"}},
                               "inspection_index_resource":{"type":"string"}},
                 "not":{"anyOf":[{"required":["discovery"]},{"required":["metadata"]},{"required":["platform_model"]},{"required":["model_page"]}]}},
                {"required":["discovery"],
                 "properties":{"discovery":{"not":{"type":"null"}}},
                 "not":{"anyOf":[{"required":["model_page"]},{"required":["model_selection"]}]}}
            ]));
            complete_fields["if"] = json!({"not":{"anyOf":[{"required":["model_page"]},{"required":["model_selection"]}]}});
            complete_fields["then"] = json!({
                "anyOf":[{"required":["metadata"]},{"required":["platform_model"]}],
                "allOf":[{"if":{"required":["platform_model"]},"then":{"required":["platform_descriptor"]}}]
            });
        }
        let complete = json!({"if":{"properties":{"contract":{"properties":{"completeness":{"const":"inline_complete"}}}}},"then":complete_fields});
        payload.insert("allOf".into(), json!([complete]));
        base.insert("allOf".into(), json!([{"if":{"allOf":[
            {"properties":{"contract":{"properties":{"status":{"enum":["success","scientific_failure"]}}}}},
            {"not":{"required":["error"]}}
        ]},"then":payload}]));
    }
    base.into()
}

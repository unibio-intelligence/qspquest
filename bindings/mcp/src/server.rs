pub use crate::tool_api::*;
mod execution;
mod local_jobs;
#[cfg(feature = "managed")]
mod managed;
use crate::model_cards::{compact_model_card, model_details, model_details_resource};
use crate::{
    cache::Cache,
    core::{compute, requests, summary, usability},
    library::{hash, Library, ModelSource},
    policy::{self, Tier},
};
use rmcp::{
    handler::server::wrapper::Parameters, model::*, service::RequestContext, tool, tool_handler,
    tool_router, ErrorData, RoleServer, ServerHandler,
};
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct Server {
    pub library: Arc<RwLock<Library>>,
    pub hosted: bool,
    deployment_profile: crate::deployment::Profile,
    resolver_ready: bool,
    gateway_tls_declared: bool,
    csv_output: Option<Arc<crate::export::OutputDirectory>>,
    artifact_store: Option<Arc<crate::artifacts::Store>>,
    local_jobs: Option<Arc<crate::local_jobs::Store>>,
    local_job_slots: Arc<tokio::sync::Semaphore>,
    compute_slots: usize,
    pub tier: Tier,
    pub semaphore: Arc<tokio::sync::Semaphore>,
    pub cache: Arc<Cache>,
    pub compiled: Arc<
        std::sync::Mutex<
            std::collections::VecDeque<(
                String,
                Arc<qspquest_modeling::document::CompiledDocument>,
            )>,
        >,
    >,
    compilation_flights: Arc<crate::cache::CompilationFlights>,
    search_index: Arc<RwLock<crate::library::SearchIndex>>,
    pub tenant_semaphore: Option<Arc<tokio::sync::Semaphore>>,
    pub submitter_provenance: Option<Value>,
    #[cfg(feature = "sandbox")]
    pub sandbox: Option<Arc<crate::sandbox::Sandbox>>,
}

/// Resolved adapter inputs owned by both request inspection and execution.
struct PreparedInvocation {
    scheduled_boundary_receipt: Option<crate::boundaries::Receipt>,
    initialization_receipt: Option<crate::initialization::Receipt>,
    dose_event_receipts: Option<Vec<crate::planning::DoseEventReceipt>>,
    precision_profile_receipt: Option<crate::precision::Receipt>,
    required_output_receipts: Vec<crate::core::required_outputs::RequiredOutputReceipt>,
    time_validation: Vec<crate::core::time_validation::TimeWindow>,
    override_receipt: crate::core::overrides::OverrideReceipt,
    output_mapping: Option<Vec<crate::contract::OutputMapping>>,
    base: Option<Value>,
    args: Value,
    source: String,
    metadata: Value,
    model_json: Value,
    native: Value,
    notes: Vec<String>,
    row_mapping: Value,
    request: Value,
    compute_request: Value,
    effective_hash: String,
    trusted: bool,
    original_source_hash: String,
    library_version: String,
}
struct PreparedComparison {
    members: Vec<(
        String,
        PreparedInvocation,
        String,
        crate::planning::ComparisonInputReceipt,
    )>,
    plan: crate::planning::RequestPlan,
}
impl Server {
    pub fn new(library: Library, hosted: bool, tier: Tier, concurrency: usize) -> Self {
        let search_index = crate::library::SearchIndex::new(&library);
        Self {
            library: Arc::new(RwLock::new(library)),
            search_index: Arc::new(RwLock::new(search_index)),
            hosted,
            deployment_profile: if hosted {
                crate::deployment::Profile::Public
            } else {
                crate::deployment::Profile::Local
            },
            resolver_ready: false,
            gateway_tls_declared: false,
            csv_output: None,
            artifact_store: None,
            local_jobs: None,
            local_job_slots: Arc::new(tokio::sync::Semaphore::new(concurrency)),
            compute_slots: concurrency,
            tier,
            semaphore: Arc::new(tokio::sync::Semaphore::new(concurrency)),
            cache: Arc::new(Cache::default()),
            compiled: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
            compilation_flights: Arc::new(crate::cache::CompilationFlights::default()),
            tenant_semaphore: None,
            submitter_provenance: None,
            #[cfg(feature = "sandbox")]
            sandbox: None,
        }
    }
    pub(crate) fn library_search(&self, query: &str, filters: &Value) -> Value {
        let library = self.library.read().unwrap();
        if let Some(value) = self
            .search_index
            .read()
            .unwrap()
            .search(&library, query, filters)
        {
            return value;
        }
        let mut index = self.search_index.write().unwrap();
        if !index.current(&library) {
            *index = crate::library::SearchIndex::new(&library);
        }
        index.search(&library, query, filters).unwrap()
    }
    pub fn with_deployment(
        mut self,
        settings: &crate::deployment::Settings,
        resolver_ready: bool,
    ) -> Self {
        self.deployment_profile = settings.profile;
        self.resolver_ready = resolver_ready;
        self.gateway_tls_declared = settings.external_tls;
        self
    }
    /// Enable exclusive local exports under an explicitly selected private root.
    pub fn with_csv_output(mut self, path: &std::path::Path) -> Result<Self, String> {
        if self.hosted {
            return Err("CSV export directories are available only for local stdio servers".into());
        }
        self.csv_output = Some(Arc::new(crate::export::OutputDirectory::open(path)?));
        Ok(self)
    }
    /// Enable retained local results under an explicitly selected private root.
    pub fn with_artifact_storage(mut self, path: &std::path::Path) -> Result<Self, String> {
        if self.hosted {
            return Err("Result artifacts are available only for local stdio servers".into());
        }
        self.artifact_store = Some(Arc::new(
            crate::artifacts::Store::open(path).map_err(|e| e.message)?,
        ));
        Ok(self)
    }
    fn ensure_identity(value: &mut Value) {
        if value.get("identity").is_none() && value["schema"] == "qspquest.scalar-fit-result/v0.1" {
            if let Ok(identity) = serde_json::from_value::<crate::contract::EngineIdentity>(
                value["objective"]["identity"].clone(),
            ) {
                value["identity"] = serde_json::to_value(identity).unwrap();
            }
        }
        if value.get("identity").is_none() {
            value["identity"] = json!({"engine":concat!("qspquest/",env!("CARGO_PKG_VERSION")),"source_hash":hash(value.to_string().as_bytes()),"backend":"metadata","backend_version":env!("CARGO_PKG_VERSION")});
        }
    }
    fn persist_result(&self, operation: &str, result: &mut Value) -> Result<bool, Value> {
        let Some(store) = &self.artifact_store else {
            return Ok(false);
        };
        let text = result.as_object_mut().unwrap().remove("_text");
        Self::ensure_identity(result);
        let library_version = result["library_version"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| self.library.read().unwrap().version.clone());
        result["library_version"] = json!(library_version);
        crate::contract::attach(result, &library_version, false);
        let stored = store.put(operation, result);
        if let Some(text) = text {
            result.as_object_mut().unwrap().insert("_text".into(), text);
        }
        let descriptor = stored.map_err(|e| policy::error(e.code, e.message))?;
        result["full_result_resource"] = json!(descriptor.uri);
        result["result_artifact"] = serde_json::to_value(descriptor).unwrap();
        Ok(true)
    }
    pub async fn prewarm(&self) -> Result<(), String> {
        let library = self.library.read().unwrap();
        let version = library.version.clone();
        let docs: Vec<_> = library
            .records
            .iter()
            .filter(|r| {
                !r.model.is_null()
                    && r.metadata["candidate"] != true
                    && (!self.hosted || policy::async_route(&r.model, "simulate").is_none())
            })
            .map(|r| r.model.to_string())
            .take(8)
            .collect();
        drop(library);
        let cache = self.compiled.clone();
        let flights = self.compilation_flights.clone();
        tokio::task::spawn_blocking(move || {
            for source in docs {
                let key = format!("{version}:{}", hash(source.as_bytes()));
                let document = serde_json::from_str(&source).map_err(|e| e.to_string())?;
                crate::cache::compiled(&cache, &flights, key, || {
                    qspquest_modeling::document::CompiledDocument::compile(&document)
                        .map_err(compute::core_error)
                })
                .map_err(|e| e.to_string())?;
            }
            Ok::<_, String>(())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    fn scoped(&self, ctx: &RequestContext<RoleServer>) -> Self {
        let server = self.clone();
        #[cfg(feature = "managed")]
        let mut server = server;
        #[cfg(feature = "managed")]
        if let Ok(scope) = crate::jobs::Scope::from_context(ctx) {
            server.tier = Tier::parse(&scope.tier).unwrap_or(Tier::Free);
            server.tenant_semaphore = Some(crate::jobs::tenant_slots(&scope.tenant_id));
            server.submitter_provenance =
                Some(json!({"principal_id":scope.user_id,"tenant_id":scope.tenant_id}));
        }
        let _ = ctx;
        server
    }
    pub fn capabilities(&self) -> Value {
        use crate::contract::{BuildFeatures, CapabilityDocument, Deployment, Limits};
        let library = self.library.read().unwrap();
        #[cfg(feature = "sandbox")]
        let sandbox_configured = self.sandbox.is_some();
        #[cfg(not(feature = "sandbox"))]
        let sandbox_configured = false;
        #[cfg(feature = "managed")]
        let governed_backend_configured = self.deployment_profile
            == crate::deployment::Profile::Managed
            && crate::jobs::HttpJobBackend::configured().is_ok();
        #[cfg(not(feature = "managed"))]
        let governed_backend_configured = false;
        serde_json::to_value(CapabilityDocument {
            schema: crate::contract::CapabilityVersion::V1, result_schema: crate::contract::ResultVersion::V1,
            server_version: env!("CARGO_PKG_VERSION").into(),
            tools: Self::tool_router().list_all().into_iter().map(|t| t.name.to_string()).collect(),
            target: crate::capability::TARGET.into(), capability_catalog_uri: crate::capability::CATALOG_URI.into(),
            capability_registry_uri: crate::capability::REGISTRY_URI.into(), capability_query_tool: "get_capability".into(),
            features: BuildFeatures { analysis: cfg!(feature = "analysis"), managed: cfg!(feature = "managed"), sandbox: cfg!(feature = "sandbox"), widget: cfg!(feature = "widget") },
            deployment: Deployment { mode: if self.hosted { "hosted" } else { "local_stdio" }.into(),
                profile: self.deployment_profile, verified_session_resolver_ready: self.resolver_ready,
                browser_origin_validation: self.hosted, gateway_tls_declared: self.gateway_tls_declared,
                proxy_peer_restriction: self.deployment_profile == crate::deployment::Profile::Managed,
                csv_export_enabled: !self.hosted && self.csv_output.is_some(), local_result_artifacts_enabled: self.artifact_store.is_some(), local_jobs_enabled: self.local_jobs.is_some(), sandbox_configured,
                user_source_execution: !self.hosted || sandbox_configured,
                governed_jobs_require_verified_session: cfg!(feature = "managed"), governed_backend_configured,
                governed_artifact_bytes_adapter: cfg!(feature = "managed"), widget_resource_available: true },
            limits: Limits { input_bytes: policy::INPUT_LIMIT, encoded_response_bytes: policy::RESPONSE_LIMIT,
                compute_slots: self.compute_slots, hosted_deadline_ms: self.hosted.then_some(45_000),
                local_result_retention_seconds: (!self.hosted).then_some(900), local_result_entries: (!self.hosted).then_some(8),
                local_result_bytes_per_entry: (!self.hosted).then_some(4_000_000),
                local_artifact_retention_seconds: self.artifact_store.as_ref().map(|_| crate::artifacts::RETENTION_SECONDS),
                local_artifact_entries: self.artifact_store.as_ref().map(|_| crate::artifacts::MAX_LIVE_ARTIFACTS),
                local_artifact_bytes_per_entry: self.artifact_store.as_ref().map(|_| crate::artifacts::MAX_BYTES),
                local_artifact_total_bytes: self.artifact_store.as_ref().map(|_| crate::artifacts::MAX_TOTAL_BYTES),
                local_artifact_chunk_bytes: self.artifact_store.as_ref().map(|_| crate::artifacts::CHUNK_BYTES),
                local_artifact_receipt_entries: self.artifact_store.as_ref().map(|_| crate::artifacts::MAX_RECORDS),
                local_artifact_expiration_marker_seconds: self.artifact_store.as_ref().map(|_| crate::artifacts::EXPIRATION_MARKER_SECONDS),
                local_job_active_entries: self.local_jobs.as_ref().map(|_| crate::local_jobs::MAX_ACTIVE),
                local_job_retained_entries: self.local_jobs.as_ref().map(|_| crate::local_jobs::MAX_JOBS),
                local_job_work_units: self.local_jobs.as_ref().map(|_| crate::local_jobs::MAX_UNITS),
                local_job_retention_seconds: self.local_jobs.as_ref().map(|_| crate::local_jobs::RETENTION_SECONDS),
                local_job_expiration_marker_seconds: self.local_jobs.as_ref().map(|_| crate::local_jobs::TOMBSTONE_SECONDS), inline_regimens: 25, inline_scan_points: 25,
                managed_artifact_chunk_bytes: (cfg!(feature = "managed") && self.hosted).then_some(48_000),
                managed_artifact_bytes_per_entry: (cfg!(feature = "managed") && self.hosted).then_some(128_000_000),
                managed_artifact_max_retention_seconds: (cfg!(feature = "managed") && self.hosted).then_some(604_800),
                managed_artifact_retrieval_deadline_ms: (cfg!(feature = "managed") && self.hosted).then_some(15_000),
                population_subjects: if self.hosted {16} else {128}, population_observations: if self.hosted {256} else {10_000} },
            library_version: library.version.clone(), model_records: library.records.len(),
            availability_counts: serde_json::from_value(library.availability_counts.clone()).ok(), current_engine_qualification: crate::contract::ArchiveQualification::NotAssessed,
            qualification_notice: "Execution availability and stored evidence do not establish qualification for this engine build, target, operation or modified request. Inspect model availability and exact release evidence before interpreting results.".into(),
        }).unwrap()
    }
    pub fn capability_registry(&self) -> Value {
        let capabilities = self.capabilities();
        let tools: Vec<String> = serde_json::from_value(capabilities["tools"].clone()).unwrap();
        let features = serde_json::from_value(capabilities["features"].clone()).unwrap();
        let runtime = crate::capability::Runtime {
            target: crate::capability::TARGET,
            mode: self.capability_mode(),
            tools: &tools,
            features: &features,
            governed_ready: false,
            local_jobs_ready: self.local_jobs.is_some(),
            user_source_execution: capabilities["deployment"]["user_source_execution"] == true,
        };
        serde_json::to_value(crate::capability::registry(
            &runtime,
            serde_json::from_value(capabilities["limits"].clone()).unwrap(),
        ))
        .unwrap()
    }
    fn capability_mode(&self) -> &'static str {
        if !self.hosted {
            "local_stdio"
        } else if self.deployment_profile == crate::deployment::Profile::Managed {
            "managed_http"
        } else {
            "public_http"
        }
    }
    fn response(&self, mut value: Value) -> CallToolResult {
        if !value.is_object() {
            value = json!({"result":value})
        }
        let text = value
            .as_object_mut()
            .and_then(|o| o.remove("_text"))
            .and_then(|t| t.as_str().map(str::to_owned));
        let library_version = value["library_version"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| self.library.read().unwrap().version.clone());
        value["library_version"] = json!(library_version);
        Self::ensure_identity(&mut value);
        crate::contract::attach(&mut value, &library_version, false);
        let mut result = if value.get("error").is_some() {
            CallToolResult::structured_error(value)
        } else {
            CallToolResult::structured(value)
        };
        let lead = text.filter(|_| result.is_error != Some(true));
        if let Some(text) = lead.clone() {
            // Lead with the fields a caller needs; the complete record stays in structuredContent.
            result.content = vec![ContentBlock::text(text)];
        }
        if serde_json::to_vec(&result).unwrap().len() + 128 > policy::RESPONSE_LIMIT {
            if let Some(v) = &result.structured_content {
                if v.get("error").is_none()
                    && v["effective_request_hash"].is_string()
                    && v["full_result_resource"].is_null()
                    && v["full_plan_resource"].is_null()
                    && v["saved_csv"].is_null()
                    && v["contract"]["resources"]
                        .as_array()
                        .is_none_or(Vec::is_empty)
                {
                    return self.response(policy::error("result_output_budget", "The complete numerical result exceeds the inline budget and has no retained resource. Request fewer outputs or a smaller grid; for local stdio calculations, enable --artifact-dir PATH before running the request."));
                }
                let mut compact = serde_json::Map::new();
                for key in [
                    "contract",
                    "execution_route",
                    "latency_contract",
                    "readouts",
                    "output_mapping",
                    "override_receipt",
                    "precision_profile_receipt",
                    "time_validation",
                    "required_output_receipts",
                    "dose_event_receipts",
                    "scheduled_boundary_receipt",
                    "scheduled_boundary_receipt_notice",
                    "initialization_receipt",
                    "dose_event_receipt_notice",
                    "initialization_receipt_notice",
                    "values_at_requested_times",
                    "values_note",
                    "saved_csv",
                    "notes",
                    "parameters_used",
                    "summary",
                    "pharmacology_summary",
                    "trajectory_kind",
                    "trajectory_notice",
                    "sampling",
                    "scientific_status",
                    "scientific_assessment",
                    "source_interpretation_summary",
                    "identity",
                    "run_id",
                    "full_result_resource",
                    "result_artifact",
                    "managed_artifact",
                    "artifact_retrieval",
                    "local_job",
                    "full_model_details_resource",
                    "effective_request_hash",
                    "plan_hash",
                    "full_plan_resource",
                    "plan_bytes",
                    "library_version",
                    "error",
                ] {
                    if let Some(field) = v.get(key) {
                        compact.insert(key.into(), field.clone());
                    }
                }
                if let Some(regimens) = v["regimens"].as_array() {
                    let members: Vec<Value> = regimens.iter().map(|record| {
                        let mut member = record.clone();
                        if let Some(result) = member["result"].as_object_mut() {
                            let omitted_boundaries = result.remove("scheduled_boundary_receipt").is_some_and(|v| !v.is_null());
                            let omitted_initialization = result.remove("initialization_receipt").is_some_and(|v| !v.is_null());
                            let omitted_scientific = result.remove("scientific_assessment").is_some_and(|v| !v.is_null());
                            result.retain(|key, _| matches!(key.as_str(), "identity" | "run_id" | "full_result_resource" | "result_artifact" | "plan_hash" | "effective_request_hash" | "saved_csv" | "trajectory_kind" | "trajectory_notice"));
                            if omitted_scientific { result.insert("scientific_assessment_notice".into(), json!("Scoped archive assessment omitted inline. The current engine and request remain not_assessed. Read this named member's full_result_resource or saved CSV sidecar for the declaration and applicability; numerical execution does not qualify any scientific axis.")); }
                            if omitted_boundaries { result.insert("scheduled_boundary_receipt_notice".into(), json!("Scheduled boundary receipt omitted inline. Read this named member's full_result_resource or saved CSV sidecar for forcing origins and observation conventions.")); }
                            if omitted_initialization {
                                result.insert("initialization_receipt_notice".into(), json!(if result.get("full_result_resource").is_some_and(Value::is_string) {
                                    "Initialization receipt omitted inline. Read this named member's full_result_resource for its actual clock, initialized states, binding phases and denominator policies."
                                } else {
                                    "Initialization receipt omitted inline. This member has no retained result resource; use its saved CSV sidecar, retained artifacts or a smaller comparison for complete receipt inspection."
                                }));
                            }
                        }
                        member
                    }).collect();
                    compact.insert("regimens".into(), json!(members));
                }
                let mut bounded = Value::Object(compact);
                if bounded.get("error").is_some() {
                    // Retain the bounded typed diagnostic instead of a potentially huge raw error.
                    bounded["error"] = bounded["contract"]["failure"].clone();
                }
                if bounded.get("trajectory_kind").is_some() {
                    bounded["trajectory_kind"] = json!("summary_only");
                    bounded["trajectory_notice"] = json!(
                        "The inline trajectory exceeds the response budget and is omitted. Read full_result_resource for the complete grid, or select fewer outputs when no cached resource is available."
                    );
                    bounded["sampling"]["returned_points"] = json!(0);
                }
                if bounded.get("full_result_resource").is_some()
                    && bounded.to_string().len() > policy::RESPONSE_LIMIT / 2
                {
                    bounded.as_object_mut().unwrap().remove("summary");
                    bounded["summary_notice"] = json!(
                        "Inline summary exceeds the response budget. Retrieve the complete result resource for numerical calculations."
                    );
                }
                for key in [
                    "summary",
                    "pharmacology_summary",
                    "parameters_used",
                    "output_mapping",
                    "override_receipt",
                    "required_output_receipts",
                    "dose_event_receipts",
                    "scheduled_boundary_receipt",
                    "initialization_receipt",
                    "values_at_requested_times",
                ] {
                    if bounded.to_string().len() > policy::RESPONSE_LIMIT / 3 {
                        bounded.as_object_mut().unwrap().remove(key);
                        if key == "override_receipt" {
                            bounded["override_receipt_notice"] = json!(format!("All {} override receipt entries are omitted inline. Read full_result_resource for complete native consumers and warnings.", v["override_receipt"]["entries"].as_array().map_or(0, Vec::len)));
                        }
                        if key == "required_output_receipts" {
                            bounded["required_output_notice"] = json!("Required-output receipts are omitted inline. Read full_result_resource for the complete fulfilled requirements and numerical columns.");
                        }
                        if key == "dose_event_receipts" {
                            bounded["dose_event_receipt_notice"] = json!("Dose-event receipts are omitted inline. Read full_result_resource for the complete effective schedule, source coordinates and input units.");
                        }
                        if key == "scheduled_boundary_receipt" {
                            bounded["scheduled_boundary_receipt_notice"] = json!("Scheduled boundary receipt omitted inline. Read full_result_resource for forcing origins, event references and explicit observation sides.");
                        }
                        if key == "initialization_receipt" {
                            bounded["initialization_receipt_notice"] = json!("Initialization receipt omitted inline. Read full_result_resource for the actual clock, initialized states, binding phases and denominator policies.");
                        }
                        if key == "output_mapping" {
                            bounded["output_mapping_notice"] = json!(format!("All {} mapping entries are omitted inline. Read full_result_resource for the complete request-to-column mapping.", v["output_mapping"].as_array().map_or(0, Vec::len)));
                        }
                        bounded["summary_notice"] = json!(
                            "Part of the result exceeds the response budget and was left out. Name the outputs you need, use readouts, or save_csv for the full trajectory."
                        );
                    }
                }
                let lead = lead.filter(|t| t.len() < policy::RESPONSE_LIMIT / 3);
                crate::contract::attach(&mut bounded, &library_version, true);
                let text = bounded.to_string();
                // Completeness explicitly records omitted fields; status and links survive.
                result.structured_content = Some(bounded);
                result.content = vec![ContentBlock::text(
                    if let Some(lead) = lead.clone().filter(|t| t.len() < 60_000) {
                        lead
                    } else if text.len() < 16_000 {
                        text
                    } else {
                        "Full fields are attached in structuredContent; inspect status and identity before interpreting the result.".into()
                    },
                )];
            }
        }
        if let Some(uri) = result
            .structured_content
            .as_ref()
            .and_then(|v| v["full_model_details_resource"].as_str())
        {
            result.content.push(ContentBlock::resource_link(Resource::new(uri, "Complete model details")
                .with_description("Versioned metadata, native names and equations. Read chunks in order and verify their digest when a manifest is returned.")
                .with_mime_type("application/json")));
        }
        let mut linked = Vec::new();
        if let Some(value) = result.structured_content.as_ref() {
            for key in ["full_result_resource", "full_plan_resource"] {
                if let Some(uri) = value[key].as_str() {
                    linked.push(uri.to_owned());
                }
            }
            for row in value["regimens"].as_array().into_iter().flatten() {
                if let Some(uri) = row["result"]["full_result_resource"].as_str() {
                    linked.push(uri.to_owned());
                }
            }
        }
        for uri in linked {
            let (name, description) = if uri.starts_with("qspquest://plans/") {
                ("Complete static request plan", "Scientific preparation metadata with units, schedule, workload and deferred checks; no calculated trajectory or qualification. Read and verify all chunks.")
            } else if uri.starts_with("qspquest://managed-artifacts/") {
                ("Retained governed artifact descriptor", "Authenticated immutable-byte manifest. Read and verify every base64 chunk and the complete SHA-256 before interpreting its opaque content; retrieval does not qualify science.")
            } else {
                ("Complete simulation result", "Complete requested grid, event sides, outputs and calculation identity. Read this resource before claiming full-result retrieval.")
            };
            result.content.push(ContentBlock::resource_link(
                Resource::new(uri, name)
                    .with_description(description)
                    .with_mime_type("application/json"),
            ));
        }
        if serde_json::to_vec(&result).unwrap().len() + 128 > policy::RESPONSE_LIMIT {
            // A last delivery reduction must not turn a completed operation into an
            // apparent execution failure or discard its already-retained results.
            let original = result.structured_content.as_ref().unwrap();
            let mut original_contract: crate::contract::Contract =
                serde_json::from_value(original["contract"].clone()).unwrap();
            match &mut original_contract.outcome {
                crate::contract::Outcome::Success => {}
                crate::contract::Outcome::ScientificFailure { failure }
                | crate::contract::Outcome::Unsupported { failure }
                | crate::contract::Outcome::AsyncRequired { failure, .. }
                | crate::contract::Outcome::RequestFailure { failure } => {
                    failure.input_binding = None;
                    failure.inspection_change = None;
                    failure.input_diagnostic = None;
                    failure.output_requirement = None;
                }
            }
            let mut minimal = json!({"contract":{"resources":original_contract.resources},
                "delivery_receipt":{"inline_fields_omitted":true,
                    "omitted_payload_sha256":hash(original.to_string().as_bytes()),
                    "scientific_qualification":"not_assessed",
                    "notice":"Inline details exceed the response budget. The operation status and complete retained-resource references are preserved. Retrieve and verify those resources before interpreting results; this delivery receipt does not qualify science."}});
            for key in [
                "identity",
                "run_id",
                "effective_request_hash",
                "plan_hash",
                "full_result_resource",
                "full_plan_resource",
                "full_model_details_resource",
                "inspection_index_resource",
                "saved_csv",
            ] {
                if let Some(field) = original.get(key).filter(|v| v.to_string().len() <= 16_000) {
                    minimal[key] = field.clone();
                }
            }
            if let Some(rows) = original["regimens"].as_array() {
                minimal["regimens"] = Value::Array(
                    rows.iter()
                        .map(|row| {
                            json!({"name":row["name"],"result":{
                        "full_result_resource":row["result"]["full_result_resource"],
                        "effective_request_hash":row["result"]["effective_request_hash"]}})
                        })
                        .collect(),
                );
            }
            if original["local_job"]["job_id"].is_string() {
                minimal["delivery_receipt"]["local_job_lookup"] = json!({
                    "job_id":original["local_job"]["job_id"],
                    "state":original["local_job"]["state"],
                    "notice":"Use get_local_job with this job_id for the complete retained job descriptor; completed resource URIs remain in contract.resources."});
            }
            let original_outcome = serde_json::to_value(&original_contract.outcome).unwrap();
            if !original_outcome["failure"].is_null() {
                minimal["error"] = original_outcome["failure"].clone();
            }
            crate::contract::attach(&mut minimal, &library_version, true);
            let mut contract: crate::contract::Contract =
                serde_json::from_value(minimal["contract"].clone()).unwrap();
            contract.outcome = original_contract.outcome;
            minimal["contract"] = serde_json::to_value(contract).unwrap();
            let mut reduced = if result.is_error == Some(true) {
                CallToolResult::structured_error(minimal)
            } else {
                CallToolResult::structured(minimal)
            };
            reduced.content = vec![ContentBlock::text(
                "Inline details are omitted. Inspect structuredContent.contract for the unchanged operation status and every retained resource URI before interpreting the result."),
            ];
            for uri in reduced.structured_content.as_ref().unwrap()["contract"]["resources"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                reduced.content.push(ContentBlock::resource_link(
                    Resource::new(uri, "Retained complete resource")
                        .with_description("Read and verify the retained resource. Its content and scientific evidence must be inspected separately from this bounded delivery receipt.")
                        .with_mime_type("application/json")));
            }
            if serde_json::to_vec(&reduced).unwrap().len() + 128 <= policy::RESPONSE_LIMIT {
                return reduced;
            }
            let mut failure = json!({"error":{"code":"response_limit","message":"Encoded response exceeds 256 KB. Select fewer outputs, shorten the grid, or retrieve a chunked stdio run resource."},"identity":{"engine":concat!("qspquest/",env!("CARGO_PKG_VERSION")),"backend":"metadata"}});
            crate::contract::attach(&mut failure, &self.library.read().unwrap().version, false);
            return CallToolResult::structured_error(failure);
        }
        result
    }

    /// When a library model cannot be evaluated from its bare defaults, say which scenarios carry a working set-up.
    fn domain_hint(&self, mut value: Value, model: &Value) -> Value {
        if value["error"]["code"] == "Domain" && model.get("library").is_some() {
            let id = model["library"]["model_id"].as_str().unwrap_or("");
            let scenarios: Vec<String> = self
                .library
                .read()
                .unwrap()
                .find(id, model["library"]["version"].as_str())
                .map(|r| r.scenarios.keys().take(8).cloned().collect())
                .unwrap_or_default();
            if !scenarios.is_empty() {
                value["error"]["suggestion"] = json!(format!(
                    "This model's bare defaults do not give a valid starting state. Start from one of its scenarios, model {{\"scenario\":{{\"model_id\":\"{id}\",\"scenario_id\":...}}}}, and override doses and parameters: {}",
                    scenarios.join(", ")
                ));
            }
        }
        value
    }
    fn resolve(
        &self,
        m: &Value,
    ) -> Result<
        (
            String,
            Value,
            Option<Value>,
            bool,
            String,
            Option<Arc<crate::precision::Profiles>>,
        ),
        Value,
    > {
        if let Some(s) = m["source"].as_str() {
            return Ok((
                s.into(),
                json!({"candidate":true,"license":"user supplied","citation":null}),
                None,
                false,
                self.library.read().unwrap().version.clone(),
                None,
            ));
        }
        let (r, sid) = if m.get("scenario").is_some() {
            (&m["scenario"], m["scenario"]["scenario_id"].as_str())
        } else {
            (&m["library"], None)
        };
        let id = r["model_id"]
            .as_str()
            .or_else(|| r["id"].as_str())
            .ok_or_else(|| {
                policy::error(
                    "invalid_model",
                    "model requires library.model_id, scenario.model_id or source",
                )
            })?;
        let lib = self.library.read().unwrap();
        let record = lib
            .find(id, r["version"].as_str())
            .ok_or_else(|| policy::error("not_found", "Model/version not found"))?;
        let expected = match r.get("expected_inspection_hash") {
            None | Some(Value::Null) => None,
            Some(Value::String(digest)) => Some(digest.as_str()),
            Some(_) => {
                return Err(policy::error(
                    "inspection_hash",
                    "expected_inspection_hash must be the exact string digest from the inspection index.",
                ));
            }
        };
        crate::inspection::check_context(record, expected)?;
        if record.model.is_null() {
            return Err(policy::error(
                "model_unavailable",
                "This model is metadata only. Inspect get_model.metadata for its source-rights or execution blocker; no substitute model is implied.",
            ));
        }
        let mut base = if let Some(sid) = sid {
            Some(
                record
                    .scenarios
                    .get(sid)
                    .ok_or_else(|| policy::error("not_found", "Scenario not found"))?
                    .clone(),
            )
        } else {
            None
        };
        if let Some(sid) = sid {
            if let Some(profiles) = &record.precision_profiles {
                profiles.apply(
                    sid,
                    r["precision_profile_id"].as_str(),
                    base.as_mut().unwrap(),
                )?;
            } else if r["precision_profile_id"].is_string() {
                return Err(policy::error(
                    "precision_profiles_unavailable",
                    "No precision profiles declared for this exact archive. Inspect get_model before selecting a profile.",
                ));
            }
        }
        let mut metadata = record.metadata.clone();
        metadata["archive_sha256"] = json!(record.archive_hash);
        metadata["source_citation"] = record.index["source_citation"].clone();
        metadata["native_execution"] = record.index["native_execution"].clone();
        // A fixed input-rate schedule only matters where a stored scenario actually sets the rate.
        if metadata["native_execution"].is_object() {
            metadata["native_execution"]["presets_set_input_rate"] =
                json!(record.scenarios.values().any(|s| {
                    s["regimen"]["covariate_changes"]
                        .as_array()
                        .is_some_and(|a| !a.is_empty())
                }));
            metadata["native_execution"]["preset_ids"] =
                json!(record.scenarios.keys().collect::<Vec<_>>());
        }
        metadata["fitting"] = record.fitting.clone();
        Ok((
            record.source_text(),
            metadata,
            base,
            record.metadata["candidate"] != true,
            lib.version.clone(),
            record.precision_profiles.clone(),
        ))
    }
    async fn prepare_invocation(
        &self,
        operation: &str,
        args: Value,
    ) -> Result<PreparedInvocation, Value> {
        self.prepare_invocation_for(operation, args, false).await
    }
    async fn prepare_invocation_for(
        &self,
        operation: &str,
        mut args: Value,
        planning: bool,
    ) -> Result<PreparedInvocation, Value> {
        let object = args
            .as_object_mut()
            .ok_or_else(|| policy::error("request", "tool arguments must be an object"))?;
        object.remove("expected_plan_hash");
        object.retain(|_, value| !value.is_null());
        let original_arguments = args.clone();
        if args.get("fit_request").is_some() {
            let conflicting: &[&str] = match operation {
                "fit_scalar" => &[
                    "request",
                    "parameter",
                    "bounds",
                    "observations",
                    "max_iterations",
                ],
                "fit_individual" => &[
                    "request",
                    "parameters_to_fit",
                    "observations",
                    "max_iterations",
                    "max_evaluations",
                ],
                "fit_population" => &["template", "rows", "mapping", "method"],
                _ => &[],
            };
            if let Some(field) = conflicting.iter().find(|field| args.get(**field).is_some()) {
                return Err(
                    json!({"error":{"code":"fit_request_conflict","message":format!("fit_request conflicts with {field}; supply one complete native fit request or the friendly inputs."),"expression":field,"suggestion":"Remove the conflicting friendly input or put the intended value in the complete fit_request; no field was ignored."}}),
                );
            }
        }
        if args.to_string().len() > policy::INPUT_LIMIT {
            return Err(policy::error("input_limit", "Tool arguments exceed 1 MB"));
        }
        crate::core::time_validation::finite_inputs(&args, "arguments")?;
        if args
            .get("detail")
            .is_some_and(|v| v != "compact" && v != "full")
        {
            return Err(policy::error("detail", "detail must be compact or full"));
        }
        if let Some(path) = args["save_csv"].as_str() {
            if self.hosted {
                return Err(policy::error(
                    "save_csv",
                    "Files are written only by a local stdio server.",
                ));
            }
            if self.csv_output.is_none() {
                return Err(policy::error(
                        "save_csv",
                        "CSV export is disabled. Configure --output-dir PATH or QSPQUEST_MCP_OUTPUT_DIR.",
                    ));
            }
            usability::safe_csv_path(path).map_err(|e| policy::error("save_csv", e))?;
        }
        let (mut source, metadata, base, trusted, library_version, precision_profiles) =
            self.resolve(&args["model"])?;
        if args["model"]["scenario"]["precision_profile_id"].is_string()
            && !matches!(operation, "simulate" | "get_patient_parameters")
        {
            return Err(policy::error(
                "precision_profile_operation",
                "Named precision selectors are supported for simulation, comparison members and initial-value evaluation. Retrieve the complete scenario request for another exploratory operation.",
            ));
        }
        let original_source_hash = hash(source.as_bytes());
        if trusted
            && args.get("doses").is_some_and(|v| !v.is_null())
            && metadata["native_execution"]["dose_schedule_mode"] == "preset_rate_schedule"
            && metadata["native_execution"]["presets_set_input_rate"] == true
        {
            let presets = metadata["native_execution"]["preset_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .take(12)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(
                json!({"error":{"code":"dose_schedule_unsupported","message":format!("This model delivers its dose through a stored input-rate schedule, so a custom dose list cannot be applied. Run one of its scenarios instead, model {{\"scenario\":{{\"model_id\":...,\"scenario_id\":...}}}}, where parameters, end, times, outputs and readouts can still be changed: {presets}"),"suggestion":"For a dose the scenarios do not cover, use the model equations from get_model in your own calculation and say so."}}),
            );
        }
        let model_json = if self.hosted && !trusted {
            // Parsing author-controlled DSL must be isolated too: malformed
            // expression trees cannot exhaust the native server's stack.
            self.compute(source.clone(), "inspect_model", json!({}), false)
                .await?
        } else {
            let model = if trusted {
                serde_json::from_str(&source).map_err(|e| json!({"message":e.to_string()}))
            } else {
                compute::model(&source)
            }
            .map_err(|e| json!({"error":e}))?;
            serde_json::to_value(model).unwrap()
        };
        if trusted && operation.starts_with("fit_") {
            let contract = &metadata["fitting"];
            if metadata["fit_ready"] != true
                || metadata["fitting_supported"] != true
                || !crate::fitting::supports_operation(contract, operation)
            {
                return Err(policy::error(
                        "fitting_unsupported",
                        "This released library version is not qualified for the requested fitting operation. Read get_model.fitting; use an explicitly labeled user candidate for exploratory fitting.",
                    ));
            }
        }
        let mut metadata = metadata;
        if metadata["simulation_evidence"] == "fixed_presets" {
            metadata["request_evidence_scope"] = json!(
                    "Preset evidence applies only to the exact stored request. Modified simulations and scans are exploratory; validate against the scientific question."
                );
        }
        let mut native = usability::augment(&metadata["native_execution"], &model_json);
        usability::validate_input_aliases(&model_json, &native)
            .map_err(|e| policy::error("input_alias_metadata", e))?;
        let mut notes = Vec::new();
        if operation == "get_patient_parameters" {
            if !native["parameters"].is_object() {
                native["parameters"] = json!({});
            }
            for (field, quantity_field) in [("parameters", "default"), ("covariates", "")] {
                for input in model_json[field].as_array().into_iter().flatten() {
                    let name = input["name"]
                        .as_str()
                        .ok_or_else(|| policy::error("model", "Model input needs a name"))?;
                    let unit = if quantity_field.is_empty() {
                        &input["unit"]
                    } else {
                        &input[quantity_field]["unit"]
                    };
                    if let Some(entry) = native["parameters"].get(name) {
                        if entry["native_name"] != name {
                            return Err(policy::error("patient_input_alias", format!("Input '{name}' conflicts with a different metadata alias; correct the model interface before initial-value evaluation")));
                        }
                    } else {
                        native["parameters"][name] =
                            json!({"native_name":name,"source_unit":unit,"request_unit":unit});
                    }
                }
            }
        }
        if matches!(operation, "simulate" | "scan_parameters") {
            crate::core::time_validation::friendly_times(
                &original_arguments,
                base.as_ref(),
                &model_json,
            )?;
        }
        if let Some(issue) = crate::core::input_names::check(operation, &args, &native) {
            let library_selection = args["model"]["source"].as_str().is_none();
            let index = if library_selection {
                self.library
                    .read()
                    .unwrap()
                    .find(
                        metadata["id"].as_str().unwrap_or(""),
                        metadata["version"].as_str(),
                    )
                    .and_then(|record| crate::inspection::index(record).ok())
            } else {
                None
            };
            return Err(crate::core::input_names::error(
                issue,
                metadata["id"]
                    .as_str()
                    .or_else(|| model_json["model_id"].as_str())
                    .unwrap_or("selected_model"),
                metadata["version"].as_str(),
                &format!("sha256:{original_source_hash}"),
                library_selection,
                index,
            ));
        }
        if self.hosted && !planning {
            if let Some(route) = policy::async_route(&model_json, operation) {
                return Err(route);
            }
        }
        if operation == "get_patient_parameters" {
            args["parameters"] = args.get("covariates").cloned().unwrap_or(json!({}));
            notes.extend(
                usability::normalize_located(&mut args, &native, &model_json, None)
                    .map_err(|e| e.into_value("covariates"))?,
            );
            notes.push("Outputs are evaluated at time zero, before administrations. Model or selected scenario parameter values, scalar covariates and initial states are retained; administrations, resets, future changes, observation grids and solver settings are not applied.".into());
        }
        if matches!(operation, "simulate" | "scan_parameters") {
            let supplied_args = args.clone();
            if let Some(name) = args["parameter"].as_str() {
                if let Some(n) = native["parameters"][name]["native_name"].as_str() {
                    args["parameter"] = json!(n);
                }
            }
            if args
                .get("request")
                .is_some_and(|r| !r.is_object() || r["schema"] != "qspquest.run/v0.1")
            {
                return Err(policy::error("request", "request must be a complete qspquest.run/v0.1 object. Put friendly doses, parameters, end, times and readouts at the top level; no request fields were ignored or moved."));
            }
            if operation == "simulate" && trusted && !args["outputs"].is_array() {
                // Default to the outputs that depend on the model; stored published numbers are returned only when named.
                let modelled = usability::model_outputs(&model_json);
                if !modelled.is_empty()
                    && modelled.len() < model_json["outputs"].as_array().map(Vec::len).unwrap_or(0)
                {
                    args["outputs"] = json!(modelled);
                    notes.push("outputs omitted: model-dependent outputs are selected; stored reported-value outputs require explicit selection".into());
                }
            }
            notes.extend(
                usability::normalize_located(&mut args, &native, &model_json, base.as_ref())
                    .map_err(|e| e.into_value("request"))?,
            );

            if let (Some(outputs), Some(specs)) = (
                args["outputs"].as_array().cloned(),
                args["readouts"].as_array(),
            ) {
                let mut outputs = outputs;
                for spec in specs {
                    if !spec["output"].is_null() && !outputs.contains(&spec["output"]) {
                        outputs.push(spec["output"].clone());
                    }
                }
                args["outputs"] = json!(outputs);
            }
            notes.extend(requests::run_notes(&supplied_args, &args, base.as_ref()));
        }
        let mut row_mapping = json!([]);
        let mut request = match operation {
            "fit_population" => {
                let (r, m) = requests::population(&args, self.hosted)?;
                row_mapping = m;
                r
            }
            "get_patient_parameters" => {
                let mut request = json!({"schema":"qspquest.initial-values/v0.1", "parameters":{}, "covariates":{}, "initial_states":{}, "max_output_values":25_000_000});
                if let Some(base) = base.as_ref() {
                    for field in ["parameters", "initial_states"] {
                        if let Some(values) = base.get(field) {
                            if !values.is_object() {
                                return Err(policy::error(
                                    "scenario",
                                    format!("Scenario {field} must map names to quantities"),
                                ));
                            }
                            request[field] = values.clone();
                        }
                    }
                    if let Some(values) = base.get("regimen").and_then(|r| r.get("covariates")) {
                        if !values.is_object() {
                            return Err(policy::error(
                                "scenario",
                                "Scenario covariates must map names to scalar quantities",
                            ));
                        }
                        request["covariates"] = values.clone();
                    }
                }
                for (name, value) in args["parameters"].as_object().into_iter().flatten() {
                    let covariate = model_json["covariates"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|c| c["name"] == name.as_str());
                    request[if covariate {
                        "covariates"
                    } else {
                        "parameters"
                    }][name] = value.clone();
                }
                request
            }
            "steady_state" | "morris_screening" => {
                crate::core::analysis_request::admit(operation, &args["request"])
                    .map_err(|error| error.into_value("request"))?;
                args["request"].clone()
            }
            _ if args.get("fit_request").is_some() => args["fit_request"].clone(),
            _ => requests::run(&args, base.as_ref(), &model_json)
                .map_err(|e| policy::error("request", e))?,
        };
        if operation == "simulate" {
            notes.extend(
                usability::treatment_cycles(&mut request, &native, &model_json)
                    .map_err(|e| policy::error("treatment_cycle", e))?,
            );
            if args.get("request").is_none() && base.is_some() && args.get("end").is_some() {
                notes.extend(
                    usability::trim_events_after_end(&mut request)
                        .map_err(|e| policy::error("request", e))?,
                );
            }
        }
        if let Some(changes) = args["parameter_changes"]
            .as_array()
            .filter(|a| !a.is_empty())
        {
            if operation != "simulate" || !trusted {
                return Err(policy::error(
                    "request",
                    "parameter_changes applies to simulate and compare_regimens on library models",
                ));
            }
            let (changed, more) =
                usability::parameter_changes(&source, &mut request, changes, &native, &model_json)
                    .map_err(|e| policy::error("request", e))?;
            source = changed;
            notes.extend(more);
        }
        if operation == "fit_individual" {
            request = crate::core::individual_request::prepare(&args, request, &model_json)
                .map_err(|e| e.into_value("fit_request"))?
        }
        if operation == "fit_scalar" {
            request = crate::core::scalar_request::prepare(&args, request)
                .map_err(|e| e.into_value("fit_request"))?;
        }
        if operation == "scan_parameters" {
            let points = crate::core::scan_request::prepare(&args)?;
            policy::admit(&mut request, self.hosted)?;
            request = points.into_request(request)?;
        }
        if trusted && operation.starts_with("fit_") {
            let c = &metadata["fitting"];
            let workflow = crate::fitting::enforce(c, operation, &request)
                .map_err(|message| policy::error("fitting_contract", message))?;
            if let Some(id) = workflow {
                notes.push(format!("Library fitting workflow {id}: exact synthetic recovery fixture; recorded evidence does not qualify the active engine or a clinical/source population analysis."));
            }
        }
        if let Some(selected) = args["outputs"].as_array_mut() {
            let aliases = &native["outputs"];
            for name in selected {
                if let Some(native) = name.as_str().and_then(|n| {
                    aliases[n]
                        .as_str()
                        .or_else(|| aliases[n]["native_name"].as_str())
                }) {
                    *name = json!(native)
                }
            }
        }
        policy::admit(&mut request, self.hosted)?;
        if let Some(output) = args["output"].as_str() {
            if let Some(native) = native["outputs"][output]["native_name"].as_str() {
                let native = native.to_owned();
                args["output"] = json!(native);
            }
        }
        if let Some(selected) = args["outputs"].as_array() {
            if selected.is_empty()
                || selected.iter().any(|name| {
                    !model_json["outputs"]
                        .as_array()
                        .is_some_and(|os| os.iter().any(|o| o["name"] == *name))
                })
            {
                let names: Vec<String> = native["outputs"]
                    .as_object()
                    .filter(|m| !m.is_empty())
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_else(|| {
                        model_json["outputs"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|o| o["name"].as_str().map(str::to_owned))
                            .collect()
                    });
                return Err(policy::error(
                    "outputs",
                    format!(
                        "Unknown output name. This model's outputs are: {}",
                        names.join(", ")
                    ),
                ));
            }
        }
        let output_mapping = if matches!(
            operation,
            "simulate" | "get_patient_parameters" | "scan_parameters"
        ) {
            let mut requested = usability::requested_output_names(&original_arguments);
            if operation == "scan_parameters" {
                if let Some(name) = original_arguments["output"].as_str() {
                    requested.push(name.into());
                }
            }
            let mut mapping = usability::output_mapping(&model_json, &native, &requested)
                .map_err(|e| policy::error("outputs", e))?;
            let selection = if operation == "scan_parameters" {
                args["output"]
                    .as_str()
                    .map(|name| json!([name]))
                    .unwrap_or(Value::Null)
            } else {
                args["outputs"].clone()
            };
            let effective: Vec<String> = selection
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_else(|| {
                    model_json["outputs"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|o| o["name"].as_str().map(str::to_owned))
                        .collect()
                });
            for name in effective {
                if !mapping.iter().any(|m| m.canonical_output == name) {
                    mapping.extend(
                        usability::output_mapping(&model_json, &native, &[name])
                            .map_err(|e| policy::error("outputs", e))?,
                    );
                }
            }
            Some(mapping)
        } else {
            None
        };
        if original_arguments.get("required_outputs").is_some() && operation != "simulate" {
            return Err(policy::error("required_output", "required_outputs is supported by simulate and compare_regimens; comparison members are checked as simulations"));
        }
        let selected_outputs: Option<Vec<String>> = args["outputs"].as_array().map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        });
        let required_output_receipts = crate::core::required_outputs::prepare(
            &model_json,
            &native,
            &original_arguments["required_outputs"],
            selected_outputs.as_deref(),
        )?;
        crate::core::time_validation::finite_inputs(&request, "effective_request")?;
        let time_validation =
            crate::core::time_validation::windows(&request, &model_json, operation)?;
        let receipt_model =
            serde_json::from_str::<Value>(&source).unwrap_or_else(|_| model_json.clone());
        let override_receipt =
            crate::core::overrides::receipt(&receipt_model, &native, &original_arguments, &request);
        notes.extend(
            override_receipt
                .entries
                .iter()
                .filter_map(|entry| entry.warning.clone()),
        );
        let mut compute_request = request.clone();
        if operation == "simulate" && args["outputs"].is_array() {
            compute_request["_selected_outputs"] = args["outputs"].clone();
        }
        if operation == "get_patient_parameters" && args["outputs"].is_array() {
            request["outputs"] = args["outputs"].clone();
            compute_request["outputs"] = args["outputs"].clone();
        }
        let effective_hash = hash(compute_request.to_string().as_bytes());
        if operation == "simulate" {
            crate::planning::source_dose_coordinates(&native, &model_json)?;
        }
        let precision_profile_receipt =
            if matches!(operation, "simulate" | "get_patient_parameters") {
                if let (Some(profiles), Some(sid)) = (
                    &precision_profiles,
                    args["model"]["scenario"]["scenario_id"].as_str(),
                ) {
                    Some(
                        profiles.receipt(
                            sid,
                            args["model"]["scenario"]["precision_profile_id"].as_str(),
                            &request,
                            &model_json,
                            args["parameter_changes"]
                                .as_array()
                                .is_some_and(|v| !v.is_empty()),
                        )?,
                    )
                } else {
                    None
                }
            } else {
                None
            };
        Ok(PreparedInvocation {
            scheduled_boundary_receipt: None,
            initialization_receipt: None,
            dose_event_receipts: None,
            precision_profile_receipt,
            required_output_receipts,
            time_validation,
            override_receipt,
            output_mapping,
            base,
            args,
            source,
            metadata,
            model_json,
            native,
            notes,
            row_mapping,
            request,
            compute_request,
            effective_hash,
            trusted,
            original_source_hash,
            library_version,
        })
    }
    fn invocation_qualification(
        &self,
        prepared: &PreparedInvocation,
    ) -> crate::planning::Qualification {
        use crate::planning::*;
        let modified = prepared
            .base
            .as_ref()
            .is_some_and(|base| base != &prepared.request)
            || prepared.original_source_hash != hash(prepared.source.as_bytes());
        let metadata = &prepared.metadata;
        let illustrative = !prepared.trusted
            && [
                "iv_one_compartment",
                "oral",
                "two_compartment",
                "tmdd_qss",
                "idr_inhibition",
                "idr_stimulation",
                "transit",
            ]
            .into_iter()
            .filter_map(templates)
            .filter_map(|source| compute::model(source).ok())
            .any(|model| serde_json::to_value(model).ok().as_ref() == Some(&prepared.model_json));
        let scientific_class = if metadata["native_execution"]["coordinates"]
            == "unchanged_source_numeric_values"
            || metadata["native_execution"]["dimensional_qualification"] == false
        {
            ScientificClass::SourceScaleUnqualified
        } else if illustrative {
            ScientificClass::SyntheticIllustrative
        } else if !prepared.trusted {
            ScientificClass::UserCandidate
        } else {
            ScientificClass::ArchiveUnassessed
        };
        Qualification {
                scientific_assessment: crate::scientific::Receipt::new(metadata, prepared.original_source_hash == hash(prepared.source.as_bytes()), prepared.base.as_ref().map(|base| base == &prepared.request && !modified)),
                scientific_class, current_engine_and_request: Assessment::NotAssessed,
                preset_scope: if prepared.base.is_none() { PresetScope::NoStoredPreset }
                    else if modified { PresetScope::ModifiedStoredRequest }
                    else { PresetScope::ExactStoredRequest },
                recorded_dimensional_qualification: metadata["native_execution"]["dimensional_qualification"].as_bool(),
                recorded_fit_ready: metadata["fit_ready"].as_bool(),
                notice: "Static preparation does not establish independent scientific qualification, successful output evaluation, integration or fitting convergence. Stored preset evidence does not qualify modified requests or this engine build. Source-scale units retain their declared coordinates. Inspect all deferred checks before interpreting execution.".into(),
            }
    }
    async fn describe_invocation(
        &self,
        operation: &str,
        prepared: &PreparedInvocation,
    ) -> Result<crate::planning::RequestPlan, Value> {
        use crate::planning::*;
        let native = self
            .compute_versioned(
                prepared.source.clone(),
                &format!("plan_{operation}"),
                prepared.compute_request.clone(),
                prepared.trusted,
                &prepared.library_version,
            )
            .await
            .map_err(|error| usability::source_coordinate_refusal(error, &prepared.native))?;
        let native_preparation = NativePreparation::parse(operation, native)?;
        let dose_event_receipts = if let NativePreparation::Simulate(plan) = &native_preparation {
            Some(crate::planning::dose_event_receipts(
                plan,
                &prepared.model_json,
                &prepared.native,
            )?)
        } else {
            None
        };
        let scheduled_boundary_receipt =
            if let NativePreparation::Simulate(plan) = &native_preparation {
                Some(crate::boundaries::prepare(plan, &prepared.model_json)?)
            } else {
                None
            };
        let initialization_receipt = if let NativePreparation::Simulate(plan) = &native_preparation
        {
            Some(crate::initialization::prepare(
                plan,
                &prepared.metadata,
                prepared.args["model"]["scenario"]["scenario_id"].as_str(),
            )?)
        } else {
            None
        };
        let qualification = self.invocation_qualification(prepared);
        let metadata = &prepared.metadata;
        let governed = if self.hosted {
            policy::async_route(&prepared.model_json, operation)
                .map(|route| {
                    serde_json::from_value(route["execution_route"].clone())
                        .map_err(|e| policy::error("plan_contract", e.to_string()))
                })
                .transpose()?
        } else {
            None
        };
        let text = |value: &Value| value.as_str().map(str::to_owned);
        let mut plan = RequestPlan {
            scheduled_boundary_receipt,
            initialization_receipt,
            dose_event_receipts,
            precision_profile_receipt: prepared.precision_profile_receipt.clone(),
            schema: Version::V01, plan_hash: String::new(),
            context: Context {
                operation: operation.into(), server_version: env!("CARGO_PKG_VERSION").into(),
                library_version: prepared.library_version.clone(),
                model_id: text(&metadata["id"]), model_version: text(&metadata["version"]),
                archive_sha256: text(&metadata["archive_sha256"]),
                scenario_id: text(&prepared.args["model"]["scenario"]["scenario_id"]),
                original_source_sha256: prepared.original_source_hash.clone(),
                effective_source_sha256: hash(prepared.source.as_bytes()),
                model_metadata_sha256: hash(metadata.to_string().as_bytes()),
                adapter_arguments_sha256: hash(prepared.args.to_string().as_bytes()),
                effective_request_hash: prepared.effective_hash.clone(),
                selected_outputs: prepared.args["outputs"].as_array()
                    .map(|outputs| outputs.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()),
                deployment: self.capability_mode().into(),
            },
            output_mapping: prepared.output_mapping.clone(),
            override_receipt: Some(prepared.override_receipt.clone()),
            time_validation: Some(prepared.time_validation.clone()),
            required_output_receipts: Some(prepared.required_output_receipts.clone()),
            effective_request: prepared.request.clone(),
            row_mapping: prepared.row_mapping.as_array().cloned().unwrap_or_default(),
            assumptions: prepared.notes.clone(),
            qualification,
            execution_route: ExecutionRoute {
                kind: if governed.is_some() { "governed_job" } else { "inline" }.into(), governed,
                notice: "The plan reserves no engine, library snapshot, execution slot, pricing, destination or job authorization. Execution rechecks admission; use expected_plan_hash to refuse a changed context.".into(),
            },
            native_preparation,
        };
        plan.seal()?;
        Ok(plan)
    }
    fn complete_plan_payload(
        &self,
        plan: crate::planning::RequestPlan,
        effective_hash: &str,
        library_version: &str,
    ) -> Result<Value, Value> {
        let value = serde_json::to_value(&plan)
            .map_err(|e| policy::error("plan_contract", e.to_string()))?;
        let bytes = value.to_string().len();
        let mut result = serde_json::to_value(crate::planning::PlanPayload {
            plan_hash: plan.plan_hash.clone(),
            effective_request_hash: effective_hash.into(),
            plan_bytes: bytes,
            plan: Some(plan),
            full_plan_resource: None,
        })
        .map_err(|e| policy::error("plan_contract", e.to_string()))?;
        result["library_version"] = json!(library_version);
        // Measure the actual text/structured envelope, including JSON escaping and contracts.
        // A successful plan must retain every field rather than become a bounded summary.
        let fits_inline = bytes <= 90_000
            && self
                .response(result.clone())
                .structured_content
                .as_ref()
                .is_some_and(|reply| reply["plan"] == value);
        if !fits_inline {
            if self.hosted {
                return Err(policy::error("plan_output_budget", "The complete hosted plan exceeds the inline budget. Reduce its grid, events or subjects, or inspect it using local stdio; no partial plan or execution was produced."));
            }
            let id = result["plan_hash"].as_str().unwrap();
            let uri = format!("qspquest://plans/{id}");
            if !self.cache.put(format!("plan:{id}"), value) {
                return Err(policy::error("plan_output_budget", "The complete plan exceeds 4 MB. Reduce its grid, events or subjects; no partial plan or execution was produced."));
            }
            result["plan"] = Value::Null;
            result["full_plan_resource"] = json!(uri);
        }
        Ok(result)
    }
    async fn prepare_comparison(&self, mut args: Value) -> Result<PreparedComparison, Value> {
        use crate::planning::*;
        let object = args
            .as_object_mut()
            .ok_or_else(|| policy::error("request", "Comparison arguments must be an object"))?;
        object.remove("expected_plan_hash");
        object.retain(|_, value| !value.is_null());
        if args.to_string().len() > policy::INPUT_LIMIT {
            return Err(policy::error("input_limit", "Tool arguments exceed 1 MB"));
        }
        let original_args = args.clone();
        if let Some(output) = args["output"].as_str() {
            if args["outputs"].is_array() && args["outputs"] != json!([output]) {
                return Err(policy::error(
                    "outputs",
                    "output and outputs conflict; use one selection",
                ));
            }
            args["outputs"] = json!([output]);
        }
        let rows = args["regimens"].as_array().cloned().unwrap_or_default();
        if rows.is_empty() || rows.len() > 25 {
            return Err(policy::error(
                "budget",
                "regimens must contain 1..25 named entries",
            ));
        }
        let csv_base = args["save_csv"].as_str().map(str::to_owned);
        if let Some(base) = csv_base.as_ref() {
            crate::export::csv_path(base).map_err(|e| policy::error("save_csv", e))?;
        }
        let mut names = std::collections::BTreeSet::new();
        let mut destinations = std::collections::BTreeSet::new();
        let mut members = Vec::new();
        let mut plans = Vec::new();
        let mut group: Option<Value> = None;
        let mut effective = Vec::new();
        let mut source_hashes = Vec::new();
        for (member_index, row) in rows.into_iter().enumerate() {
            let name = row["name"]
                .as_str()
                .ok_or_else(|| policy::error("regimens", "Each regimen needs a name"))?;
            if name.trim().is_empty()
                || name.len() > 120
                || name.chars().any(char::is_control)
                || !names.insert(name.to_owned())
            {
                return Err(policy::error("regimens", "Regimen names must be unique, nonempty, at most 120 UTF-8 bytes and contain no control characters"));
            }
            let mut member_args = args.clone();
            for field in ["regimens", "output"] {
                member_args.as_object_mut().unwrap().remove(field);
            }
            if row["precision_profile_id"].is_string() {
                if !member_args["model"]["scenario"].is_object() {
                    return Err(policy::error(
                        "precision_profile_scenario",
                        format!(
                            "Comparison member {member_index} ({name}): a precision_profile_id requires model.scenario with an exact archive and scenario."
                        ),
                    ));
                }
                member_args["model"]["scenario"]["precision_profile_id"] =
                    row["precision_profile_id"].clone();
            }
            member_args["doses"] = row["doses"].clone();
            for field in ["parameters", "initial_states"] {
                if let Some(overrides) = row[field].as_object() {
                    let mut quantities =
                        member_args[field].as_object().cloned().unwrap_or_default();
                    quantities.extend(overrides.clone());
                    member_args[field] = json!(quantities);
                }
            }
            if row["parameter_changes"].is_array() {
                member_args["parameter_changes"] = row["parameter_changes"].clone();
            }
            if let Some(base) = csv_base.as_ref() {
                let suffix: String = name
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                    .collect();
                let destination = format!("{}_{suffix}.csv", base.strip_suffix(".csv").unwrap());
                // Refuse case-folded collisions on every platform, including case-insensitive filesystems.
                if !destinations.insert(destination.to_ascii_lowercase()) {
                    return Err(policy::error(
                        "save_csv",
                        "Regimen names produce colliding CSV filenames; choose distinct names",
                    ));
                }
                member_args["save_csv"] = json!(destination);
            }
            let requested_member = member_args.clone();
            let contextualize = |mut error: Value| {
                if let Some(message) = error["error"]["message"].as_str().map(str::to_owned) {
                    error["error"]["message"] = json!(format!(
                        "Comparison member {member_index} ({name}): {message}"
                    ));
                }
                error
            };
            let mut prepared = self
                .prepare_invocation_for("simulate", member_args, true)
                .await
                .map_err(contextualize)?;
            if let Some(destination) = prepared.args["save_csv"].as_str() {
                self.csv_output
                    .as_ref()
                    .ok_or_else(|| policy::error("save_csv", "CSV export is disabled"))?
                    .preflight_bundle(destination)
                    .map_err(|e| policy::error("save_csv", e))?;
            }
            let plan = self
                .describe_invocation("simulate", &prepared)
                .await
                .map_err(contextualize)?;
            prepared.dose_event_receipts = plan.dose_event_receipts.clone();
            prepared.scheduled_boundary_receipt = plan.scheduled_boundary_receipt.clone();
            prepared.initialization_receipt = plan.initialization_receipt.clone();
            let value = serde_json::to_value(&plan)
                .map_err(|e| policy::error("plan_contract", e.to_string()))?;
            let input_receipt =
                crate::planning::ComparisonInputReceipt::from_plan(&requested_member, &plan)?;
            if let Some(group) = group.as_mut() {
                for field in [
                    "library_version",
                    "model_id",
                    "model_version",
                    "archive_sha256",
                    "scenario_id",
                    "original_source_sha256",
                    "model_metadata_sha256",
                    "deployment",
                ] {
                    if group["context"][field] != value["context"][field] {
                        return Err(policy::error("comparison_context", "The model or library changed during preparation; prepare the complete comparison again"));
                    }
                }
                if value["qualification"]["preset_scope"] == "modified_stored_request" {
                    group["qualification"]["preset_scope"] = json!("modified_stored_request");
                }
                if value["qualification"]["scientific_class"] == "user_candidate" {
                    group["qualification"]["scientific_class"] = json!("user_candidate");
                }
            } else {
                group = Some(value);
            }
            source_hashes.push(
                json!({"name":name,"effective_source_sha256":plan.context.effective_source_sha256}),
            );
            effective.push(json!({"name":name,"request":plan.effective_request,"selected_outputs":plan.context.selected_outputs}));
            members.push((
                name.to_owned(),
                prepared,
                plan.plan_hash.clone(),
                input_receipt.clone(),
            ));
            plans.push(PlannedRegimen {
                name: name.to_owned(),
                input_receipt,
                preparation: plan,
            });
        }
        let mut group = group.unwrap();
        group["context"]["operation"] = json!("compare_regimens");
        group["context"]["adapter_arguments_sha256"] =
            json!(hash(original_args.to_string().as_bytes()));
        group["context"]["effective_source_sha256"] =
            json!(hash(json!(source_hashes).to_string().as_bytes()));
        group["effective_request"] = json!({"regimens":effective});
        group["context"]["effective_request_hash"] =
            json!(hash(group["effective_request"].to_string().as_bytes()));
        group["row_mapping"] = json!([]);
        group["output_mapping"] = Value::Null;
        group["override_receipt"] = Value::Null;
        group["precision_profile_receipt"] = Value::Null;
        group["time_validation"] = Value::Null;
        group["required_output_receipts"] = Value::Null;
        group["dose_event_receipts"] = Value::Null;
        group["scheduled_boundary_receipt"] = Value::Null;
        group["initialization_receipt"] = Value::Null;
        group["qualification"]["scientific_assessment"]["request_matches_stored_preset"] =
            Value::Null;
        group["qualification"]["scientific_assessment"]["effective_source_matches_archive"] =
            Value::Null;
        group["assumptions"] = json!(["Member plans preserve the complete per-regimen inputs, source, selected outputs, units, identities, workload ceilings and qualification limits in execution order.", "The comparison effective_source_sha256 hashes the ordered named member source digests. It is not a single model source digest.", "All members are admitted before numerical work. CSV destination inspection reserves no names; exclusive writes recheck after every numerical run succeeds. CSV bundles are individually completed, not a filesystem transaction across regimens."]);
        group["native_preparation"] = serde_json::to_value(NativePreparation::CompareRegimens(ComparisonPreparation {
            regimens: plans,
            deferred_checks: vec!["Successful integration and requested trajectory readouts for every member".into(), "Independent scientific qualification for each effective request and engine".into(), "Exclusive CSV bundle creation after all numerical runs succeed; filesystem state can change after inspection".into()],
        })).map_err(|e| policy::error("plan_contract", e.to_string()))?;
        let mut plan: RequestPlan = serde_json::from_value(group)
            .map_err(|e| policy::error("plan_contract", e.to_string()))?;
        plan.seal()?;
        Ok(PreparedComparison { members, plan })
    }
    async fn plan_invocation(&self, operation: &str, args: Value) -> Value {
        let prepare = async {
            if args.get("expected_plan_hash").is_some_and(|v| !v.is_null()) {
                return Err(policy::error("expected_plan_hash", "Leave expected_plan_hash unset when planning; add the returned plan_hash only to execution."));
            }
            if operation == "compare_regimens" {
                let prepared = self.prepare_comparison(args).await?;
                let effective_hash = prepared.plan.context.effective_request_hash.clone();
                let library_version = prepared.plan.context.library_version.clone();
                return self.complete_plan_payload(
                    prepared.plan,
                    &effective_hash,
                    &library_version,
                );
            }
            let prepared = self.prepare_invocation_for(operation, args, true).await?;
            let plan = self.describe_invocation(operation, &prepared).await?;
            self.complete_plan_payload(plan, &prepared.effective_hash, &prepared.library_version)
        };
        if self.hosted {
            match tokio::time::timeout(std::time::Duration::from_secs(45), prepare).await {
                Ok(result) => result.unwrap_or_else(|e| e),
                Err(_) => policy::error("deadline", "Hosted request planning exceeded its aggregate 45-second limit; active isolated/native work retains its slot until it exits."),
            }
        } else {
            prepare.await.unwrap_or_else(|e| e)
        }
    }
    async fn execute_prepared_invocation(
        &self,
        operation: &str,
        mut prepared: PreparedInvocation,
        plan_hash: Option<String>,
    ) -> Result<Value, Value> {
        if operation == "simulate" && prepared.dose_event_receipts.is_none() {
            let plan = self.describe_invocation(operation, &prepared).await?;
            prepared.dose_event_receipts = plan.dose_event_receipts;
            prepared.scheduled_boundary_receipt = plan.scheduled_boundary_receipt;
            prepared.initialization_receipt = plan.initialization_receipt;
        }
        let result = self
            .compute_versioned(
                prepared.source.clone(),
                operation,
                prepared.compute_request.clone(),
                prepared.trusted,
                &prepared.library_version,
            )
            .await
            .map_err(|error| usability::source_coordinate_refusal(error, &prepared.native))?;
        self.finish_prepared_invocation(operation, prepared, plan_hash, result)
    }
    fn finish_prepared_invocation(
        &self,
        operation: &str,
        prepared: PreparedInvocation,
        plan_hash: Option<String>,
        mut result: Value,
    ) -> Result<Value, Value> {
        let scientific_assessment = self
            .invocation_qualification(&prepared)
            .scientific_assessment;
        if operation != "get_patient_parameters" {
            result["scientific_assessment"] = json!(scientific_assessment);
        }
        let qualification = (operation == "get_patient_parameters")
            .then(|| self.invocation_qualification(&prepared));
        let PreparedInvocation {
            mut scheduled_boundary_receipt,
            mut initialization_receipt,
            mut dose_event_receipts,
            precision_profile_receipt,
            mut required_output_receipts,
            time_validation,
            override_receipt,
            mut output_mapping,
            base,
            args,
            source,
            mut metadata,
            model_json,
            native,
            notes,
            row_mapping,
            request,
            compute_request,
            effective_hash,
            trusted: _,
            original_source_hash,
            library_version,
        } = prepared;
        if let Some(receipt) = &precision_profile_receipt {
            metadata["precision_profile_receipt"] = json!(receipt);
        }
        crate::core::required_outputs::complete(&mut required_output_receipts, &result)?;
        if operation == "simulate" {
            if let Some(receipt) = &mut scheduled_boundary_receipt {
                crate::boundaries::complete(receipt, &result)?;
            }
            result["scheduled_boundary_receipt"] = json!(scheduled_boundary_receipt);
            if let Some(receipt) = &mut initialization_receipt {
                crate::initialization::complete(receipt, &result)?;
            }
            result["initialization_receipt"] = json!(initialization_receipt);
            result["required_output_receipts"] = json!(required_output_receipts);
            if let Some(receipts) = &mut dose_event_receipts {
                for receipt in receipts {
                    receipt.stage = crate::planning::DoseEventStage::ExecutedSchedule;
                }
            }
            result["dose_event_receipts"] = json!(dose_event_receipts);
        }
        if let Some(mapping) = &output_mapping {
            for entry in mapping {
                let records: Vec<&Value> = if operation == "scan_parameters" {
                    result["results"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|point| &point["result"])
                        .collect()
                } else {
                    vec![&result]
                };
                let matched = !records.is_empty()
                    && records.iter().all(|record| {
                        record["outputs"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|o| {
                                o["name"] == entry.returned_column
                                    && o["unit"] == entry.returned_unit
                            })
                            .count()
                            == 1
                    });
                if !matched {
                    return Err(policy::error("output_mapping", format!("Requested output '{}' did not return exactly one '{}' column with native unit '{}'", entry.requested_output, entry.returned_column, entry.returned_unit)));
                }
            }
            if matches!(operation, "simulate" | "scan_parameters") {
                result["output_mapping"] = serde_json::to_value(mapping).unwrap();
            }
        }
        let override_summary = override_receipt.summary();
        if operation == "get_patient_parameters" {
            let mut quantities = serde_json::Map::new();
            let mut units = serde_json::Map::new();
            for output in result["outputs"].as_array().into_iter().flatten() {
                let name = native["outputs"]
                    .as_object()
                    .and_then(|m| {
                        m.iter().find(|(_, e)| {
                            e["native_name"] == output["name"] || **e == output["name"]
                        })
                    })
                    .map(|(k, _)| k.clone())
                    .unwrap_or_else(|| output["name"].as_str().unwrap().into());
                if let Some(mapping) = output_mapping.as_mut() {
                    for entry in mapping
                        .iter_mut()
                        .filter(|m| m.canonical_output == output["name"])
                    {
                        entry.returned_quantity = Some(name.clone());
                    }
                }
                quantities.insert(name.clone(), output["value"].clone());
                units.insert(name, output["unit"].clone());
            }
            let mut used = Vec::new();
            for input in result["inputs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|i| i["kind"] == "parameter" || i["kind"] == "covariate")
            {
                let name = input["name"].as_str().unwrap();
                let field = if input["kind"] == "covariate" {
                    "covariates"
                } else {
                    "parameters"
                };
                let source = if !args["parameters"][name].is_null() {
                    "given"
                } else if !request[field][name].is_null() {
                    "scenario"
                } else {
                    "model default"
                };
                used.push(json!({"name":name,"unit":input["unit"]["declared"],"value":input["value"],"source":source}));
            }
            let used = if used.len() <= 60 {
                json!(used)
            } else {
                json!({"count":used.len(),"note":"Complete converted inputs are retained in effective_initial_values.inputs and the inspected preparation."})
            };
            let model = json!({"id":metadata["id"],"version":metadata["version"],"dois":metadata["dois"],"archive_sha256":metadata["archive_sha256"]});
            let mut value = json!({"precision_profile_receipt":precision_profile_receipt,"time_validation":time_validation,"override_receipt":override_receipt,"output_mapping":output_mapping,"scientific_assessment":scientific_assessment,"patient_quantities":quantities,"patient_units":units,
                "parameters_used":used,"time":0.0,"time_unit":result["time_unit"],
                "identity":result["identity"],"effective_request":request,"effective_request_hash":effective_hash,
                "effective_initial_values":result,"model":model,"notes":notes,"library_version":library_version,"qualification":qualification,
                "hint":"Scalar overrides replace their corresponding model/scenario values. Outputs describe the retained initial state before administrations, including any nonzero initial exposure. Declared unit 1 retains source coordinates; successful evaluation does not establish physical or scientific qualification. Some outputs may repeat stored scalar or reference values; inspect their expressions."});
            if let Some(hash) = plan_hash {
                value["plan_hash"] = json!(hash);
            }
            value["_text"] = json!(usability::ordered(&[
                ("patient_quantities", &value["patient_quantities"]),
                ("scientific_assessment", &value["scientific_assessment"]),
                ("patient_units", &value["patient_units"]),
                ("override_receipt", &override_summary),
                (
                    "precision_profile_receipt",
                    &value["precision_profile_receipt"]
                ),
                ("parameters_used", &value["parameters_used"]),
                ("time", &value["time"]),
                ("time_unit", &value["time_unit"]),
                ("notes", &value["notes"]),
                ("hint", &value["hint"])
            ]));
            if self.persist_result(operation, &mut value)? {
                return Ok(value);
            }
            let encoded = self.response(value.clone()).structured_content;
            if encoded.as_ref().is_none_or(|v| {
                v["patient_quantities"] != value["patient_quantities"]
                    || v["patient_units"] != value["patient_units"]
                    || v["effective_initial_values"] != value["effective_initial_values"]
            }) {
                return Err(policy::error("patient_output_budget", "The complete initial-value result exceeds the inline budget. Select fewer outputs; no partial quantities or result resource were returned."));
            }
            return Ok(value);
        }
        if let Some(plan_hash) = plan_hash {
            result["plan_hash"] = json!(plan_hash);
        }
        result["library_version"] = json!(library_version);
        let (mut readouts, mut requested, mut saved, mut used, mut values_note) = (
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
        );
        if operation == "simulate" {
            if args["readouts"].is_array() {
                readouts = usability::readouts(&result, &args["readouts"], &native)
                    .map_err(|e| policy::error("readouts", e))?;
            }
            if let Some(times) = args["_requested_times"].as_array() {
                if times.len() <= 60 {
                    requested =
                        usability::requested_values(&result, &args["_requested_times"], &native);
                } else {
                    values_note = json!(format!(
                    "{} times were requested; the table is not returned inline. Use readouts for the quantities you need, or save_csv for the full trajectory.",
                    times.len()
                ));
                }
            }
            if let Some(path) = args["save_csv"].as_str() {
                if self.hosted {
                    return Err(policy::error(
                        "save_csv",
                        "Files are written only by a local server; read the run resource instead.",
                    ));
                }
                let output = self.csv_output.as_ref().ok_or_else(|| policy::error("save_csv", "CSV export is disabled. Start the local server with --output-dir PATH or QSPQUEST_MCP_OUTPUT_DIR set to a private output directory."))?;
                let table =
                    usability::csv(&result, &native).map_err(|e| policy::error("save_csv", e))?;
                let manifest = crate::table::manifest(
                    path,
                    &result,
                    &table,
                    crate::table::Context {
                        metadata: &metadata,
                        request: &request,
                        source: &source,
                        scenario_id: args["model"]["scenario"]["scenario_id"].as_str(),
                        modified: base.as_ref().is_none_or(|b| b != &request)
                            || original_source_hash != hash(source.as_bytes()),
                        effective_hash: &effective_hash,
                        selected_outputs: &args["outputs"],
                    },
                )
                .map_err(|e| policy::error("save_csv", e))?;
                let (target, manifest_path) = output
                    .write_bundle(path, table.csv.as_bytes(), &manifest)
                    .map_err(|e| policy::error("save_csv", e))?;
                saved = json!({"path":target,"relative_path":path,"rows":table.rows,"columns":table.columns.iter().skip(2).map(|c|&c.name).collect::<Vec<_>>(),"overwrite":false,
                "table_sha256":hash(table.csv.as_bytes()),"manifest_path":manifest_path,"relative_manifest_path":format!("{path}.manifest.json"),"manifest_sha256":hash(&manifest),"manifest_schema":crate::table::SCHEMA,
                "note":"Keep the CSV and manifest together. Equal-time event rows are pre then post. Manifest digests verify bytes, not producer identity or scientific qualification."});
            }
            used = usability::parameters_used(&model_json, &native, &request, &args["parameters"]);
        }
        if operation == "simulate" {
            result["summary"] = summary::sampled_summary(&result, &args["outputs"]);
            result["trajectory_kind"] = json!("full_requested_grid");
            result["sampling"] = json!({"original_points":result["times"].as_array().map_or(0, Vec::len),"returned_points":result["times"].as_array().map_or(0, Vec::len),"event_sides_preserved":true});
        }
        if !row_mapping.as_array().unwrap().is_empty() {
            result["row_mapping"] = row_mapping
        }
        if operation == "simulate" && args["outputs"].is_array() {
            result["effective_outputs"] = args["outputs"].clone();
        }
        result["time_validation"] = json!(time_validation);
        result["override_receipt"] = serde_json::to_value(&override_receipt).unwrap();
        result["precision_profile_receipt"] = json!(precision_profile_receipt);
        result["effective_request_hash"] = json!(effective_hash);
        if !readouts.is_null() {
            result["readouts"] = readouts.clone();
        }
        if !requested.is_null() {
            result["values_at_requested_times"] = requested.clone();
        }
        if !notes.is_empty() {
            result["notes"] = json!(notes);
        }
        if !saved.is_null() {
            result["saved_csv"] = saved.clone();
        }
        if !values_note.is_null() {
            result["values_note"] = values_note.clone();
        }
        if !used.is_null() {
            result["parameters_used"] = used.clone();
        }
        if operation == "simulate" {
            // Outputs are named readably in the summary for every caller (single runs and regimen comparisons).
            if let Some(rows) = result["summary"].as_array_mut() {
                for row in rows {
                    if let Some(alias) = native["outputs"].as_object().and_then(|m| {
                        m.iter()
                            .find(|(_, e)| e["native_name"] == row["output"])
                            .map(|(k, _)| k.clone())
                    }) {
                        row["output"] = json!(alias);
                    }
                }
            }
            let threshold_results: Vec<_> = readouts["results"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| r["kind"] == "time_above")
                .cloned()
                .collect();
            result["pharmacology_summary"] = json!({
                "method":"Sampled grid, not NCA: sampled extrema, trapezoidal AUC and explicitly requested threshold readouts; no extrapolation.",
                "outputs":result["summary"],"time_above_threshold":threshold_results,
                "trough_note":"Pre-event samples are not necessarily dosing troughs. Request a trough readout at the dose time when needed."
            });
        }
        if operation == "simulate" && args["detail"] != "full" {
            // The text block leads with the answer and names outputs readably; structuredContent keeps
            // the complete record (effective request, provenance, identity).
            let summary = result["summary"].clone();
            let model =
                json!({"id":metadata["id"],"version":metadata["version"],"dois":metadata["dois"]});
            result["_text"] = json!(usability::ordered(&[
                ("readouts", &readouts),
                ("output_mapping", &result["output_mapping"]),
                ("override_receipt", &override_summary),
                (
                    "precision_profile_receipt",
                    &result["precision_profile_receipt"]
                ),
                ("values_at_requested_times", &requested),
                ("values_note", &values_note),
                ("saved_csv", &saved),
                ("summary", &summary),
                ("pharmacology_summary", &result["pharmacology_summary"]),
                ("scientific_assessment", &result["scientific_assessment"]),
                ("trajectory_kind", &result["trajectory_kind"]),
                ("trajectory_notice", &result["trajectory_notice"]),
                ("sampling", &result["sampling"]),
                ("time_unit", &result["time_unit"]),
                ("parameters_used", &used),
                ("notes", &result["notes"]),
                (
                    "hint",
                    &json!(
                        "Add readouts for window AUC, minimum, time below a threshold or a pre-dose trough; save_csv writes the full CSV trajectory and a .manifest.json with units, effective inputs, identity and qualification limits; keep both files together."
                    )
                ),
                ("model", &model),
                ("run_id", &result["run_id"]),
                ("full_result_resource", &result["full_result_resource"]),
                ("effective_request_hash", &result["effective_request_hash"]),
                (
                    "more",
                    &json!(
                        "structuredContent carries the sampled trajectory, the effective request and provenance."
                    )
                ),
            ]));
        }
        result["effective_request"] = request;
        result["model_provenance"] = metadata;
        if result["precision_profile_receipt"].is_object() {
            result["model_provenance"]["precision_profile_receipt"] =
                result["precision_profile_receipt"].clone();
        }
        if let Some(submitter) = &self.submitter_provenance {
            result["model_provenance"]["submitter"] = submitter.clone()
        }
        let fit_status = result
            .get("fit")
            .or_else(|| result.get("population"))
            .and_then(|f| f["status"].as_str())
            .or_else(|| {
                (operation == "fit_scalar")
                    .then(|| result["status"].as_str())
                    .flatten()
            });
        if let Some(status) = fit_status {
            result["scientific_status"] = json!({"converged":status=="converged","interpretation":if status=="converged"{"Check objective kind, diagnostics and active bounds before interpreting estimates or uncertainty."}else{"Nonconverged estimates and objective are diagnostic candidates. Do not treat them as final estimates or reliable uncertainty."}})
        }
        let run_id = (operation == "simulate").then(|| {
            hash(
                json!({"source":source,"request":compute_request,
                "model_metadata_sha256":hash(result["model_provenance"].to_string().as_bytes()),
                "override_receipt":result["override_receipt"],"required_output_receipts":result["required_output_receipts"],"dose_event_receipts":result["dose_event_receipts"],"output_mapping":result["output_mapping"],"plan_hash":result["plan_hash"]})
                .to_string()
                .as_bytes(),
            )
        });
        if let Some(run_id) = &run_id {
            result["run_id"] = json!(run_id);
        }
        // Retain the complete numerical result and context before any inline sampling.
        let retained = self.persist_result(operation, &mut result)?;
        if operation == "simulate" {
            let run_id = run_id.unwrap();
            let cached =
                !retained && !self.hosted && self.cache.put(run_id.clone(), result.clone());
            result["run_id"] = json!(run_id);
            if cached {
                result["full_result_resource"] = json!(format!("qspquest://runs/{run_id}"));
            }
            if args["detail"] != "full"
                || (retained || cached) && result.to_string().len() > 100_000
            {
                if !self.hosted
                    && !retained
                    && !cached
                    && result.to_string().len() > policy::RESPONSE_LIMIT / 2
                {
                    return Err(policy::error("result_artifact_required", "The complete trajectory exceeds the local cache budget. Start the local server with --artifact-dir PATH, use detail full for an inline result that fits, or request a smaller grid."));
                }
                result = summary::compact(result, &args["outputs"]);
            }
            if source.len() < 32_000 {
                result["widget_context"] = json!({"source":source,"model":model_json,"worker_url":std::env::var("QSPQUEST_WIDGET_WORKER_URL").ok()});
            }
        }
        if operation == "scan_parameters" {
            if let Some(results) = result["results"].as_array_mut() {
                for r in results {
                    // Without retained artifacts, preserve every point's requested grid inline.
                    if retained {
                        r["result"] = summary::compact(
                            r["result"].take(),
                            &args
                                .get("output")
                                .and_then(Value::as_str)
                                .map(|s| json!([s]))
                                .unwrap_or(Value::Null),
                        );
                    }
                }
            }
        }
        if operation == "simulate" {
            result["_text"] = json!(usability::ordered(&[
                ("readouts", &result["readouts"]),
                ("output_mapping", &result["output_mapping"]),
                ("override_receipt", &override_summary),
                ("summary", &result["summary"]),
                ("scientific_assessment", &result["scientific_assessment"]),
                ("trajectory_kind", &result["trajectory_kind"]),
                ("sampling", &result["sampling"]),
                ("saved_csv", &result["saved_csv"]),
                ("full_result_resource", &result["full_result_resource"]),
                ("notes", &result["notes"])
            ]));
        }
        Ok(result)
    }
    fn format_response_value(&self, request: crate::response_format::FormatRequest) -> Value {
        use crate::response_format::SourceOrigin;
        let result = crate::response_format::format(request, |uri, remaining| {
            if self.hosted {
                return Err(policy::error("response_source_unavailable_profile", "Hosted servers do not read local retained result references. Retrieve the complete result through the authorized artifact interface and supply its exact JSON and digest."));
            }
            if let Some(id) = uri.strip_prefix("qspquest://runs/") {
                if !crate::response_format::digest(id) {
                    return Err(policy::error("response_source_uri", "Use the exact complete simulation result URI; plan and chunk references are not trajectories."));
                }
                return self.cache.get(id).map(|value| (value.to_string(), SourceOrigin::RetainedRun))
                    .ok_or_else(|| policy::error("response_source_expired", "The complete run is absent or expired from the local cache. Use its immutable artifact, or rerun and retrieve the complete result."));
            }
            if let Some(id) = uri.strip_prefix(crate::artifacts::URI_PREFIX) {
                if !crate::response_format::digest(id) {
                    return Err(policy::error(
                        "response_source_uri",
                        "Use the exact artifact descriptor URI, without a chunk suffix.",
                    ));
                }
                let store = self.artifact_store.as_ref().ok_or_else(|| policy::error("artifact_storage_disabled", "Local artifact storage is disabled; use a currently retained run or supplied complete JSON."))?;
                let descriptor = store
                    .descriptor(id)
                    .map_err(|e| policy::error(e.code, e.message))?;
                if descriptor.bytes > remaining as u64 {
                    return Err(policy::error("response_source_limit", "Complete result sources exceed the aggregate 128 MB formatting limit. Select fewer sources; no values were formatted."));
                }
                let mut body = String::with_capacity(descriptor.bytes as usize);
                for index in 0..descriptor.chunk_count {
                    body.push_str(
                        &store
                            .chunk(id, index)
                            .map_err(|e| policy::error(e.code, e.message))?,
                    );
                }
                if body.len() as u64 != descriptor.bytes
                    || hash(body.as_bytes()) != descriptor.sha256
                {
                    return Err(policy::error("artifact_corrupt", "Complete retained artifact bytes differ from the descriptor. No response was formatted."));
                }
                return Ok((body, SourceOrigin::RetainedArtifact));
            }
            Err(policy::error("response_source_uri", "Only exact local run/artifact result references are read. Supply already retrieved JSON for other authorized result sources; arbitrary URLs and file paths are not read."))
        });
        result
            .map(|payload| serde_json::to_value(payload).unwrap())
            .unwrap_or_else(|e| e)
    }
    async fn execute_comparison(&self, args: Value) -> Value {
        let execute = async {
            let expected = args
                .get("expected_plan_hash")
                .filter(|v| !v.is_null())
                .cloned();
            if expected.as_ref().is_some_and(|v| {
                !v.as_str()
                    .and_then(|s| s.strip_prefix("sha256:"))
                    .is_some_and(|s| {
                        s.len() == 64
                            && s.bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    })
            }) {
                return Err(policy::error(
                    "expected_plan_hash",
                    "expected_plan_hash must be the exact sha256: digest returned by plan_run",
                ));
            }
            let prepared = self.prepare_comparison(args).await?;
            let plan_hash = prepared.plan.plan_hash.clone();
            let effective_request_hash = prepared.plan.context.effective_request_hash.clone();
            if expected
                .as_ref()
                .is_some_and(|v| v.as_str() != Some(plan_hash.as_str()))
            {
                return Err(
                    json!({"error":{"code":"plan_mismatch","message":"A regimen, source, library, preparation or deployment changed since planning.","expression":"expected_plan_hash","suggestion":"Call plan_run again and inspect the complete comparison plan."},"actual_plan_hash":plan_hash,"effective_request_hash":effective_request_hash}),
                );
            }
            let mut computed = Vec::new();
            // Keep captured inputs; no library/source lookup occurs between admission and execution.
            for (name, member, hash, input_receipt) in prepared.members {
                let value = self
                    .compute_versioned(
                        member.source.clone(),
                        "simulate",
                        member.compute_request.clone(),
                        member.trusted,
                        &member.library_version,
                    )
                    .await
                    .map_err(|error| usability::source_coordinate_refusal(error, &member.native))?;
                computed.push((name, member, hash, input_receipt, value));
            }
            let (mut results, mut brief) = (Vec::new(), Vec::new());
            for (name, member, hash, input_receipt, value) in computed {
                let value =
                    self.finish_prepared_invocation("simulate", member, Some(hash), value)?;
                let mut value = value;
                if let Some(o) = value.as_object_mut() {
                    // One compact record per regimen; the full run stays retrievable by run_id.
                    for key in [
                        "_text",
                        "effective_request",
                        "model_provenance",
                        "widget_context",
                        "outputs",
                        "times",
                        "sides",
                        "description",
                        "contract",
                        "schema",
                        "request_id",
                    ] {
                        o.remove(key);
                    }
                    if let Some(sampling) = o.get_mut("sampling") {
                        sampling["returned_points"] = json!(0);
                    }
                    o.insert("trajectory_kind".into(), json!("summary_only"));
                    o.insert("trajectory_notice".into(), json!("This regimen record contains sampled summaries. Read its full_result_resource for the complete grid, event sides, identity and effective request."));
                    let asked =
                        o.get("readouts").is_some() || o.get("values_at_requested_times").is_some();
                    if let Some(rows) = o.get_mut("summary").and_then(Value::as_array_mut) {
                        for row in rows {
                            if let Some(r) = row.as_object_mut() {
                                r.remove("interpretation");
                                if asked {
                                    r.remove("pre_event_samples");
                                }
                            }
                        }
                    }
                }
                brief.push(json!({"name":name,"requested_inputs_sha256":input_receipt.requested_inputs_sha256,
                    "native_parameter_contract_sha256":input_receipt.native_parameter_contract_sha256,
                    "effective_request_hash":input_receipt.effective_request_hash,
                    "full_result_resource":value["full_result_resource"],
                    "precision_profile_receipt":value["precision_profile_receipt"],
                    "scientific_assessment":value["scientific_assessment"],
                    "readouts":value["readouts"]["results"],"values_at_requested_times":value["values_at_requested_times"],"values_note":value["values_note"],"saved_csv":value["saved_csv"],"summary":value["summary"]}));
                results.push(json!({"name":name,"input_receipt":input_receipt,"result":value}));
            }
            // A comparison can exceed the cache's eight entries. Advertise only retained trajectories.
            for record in &mut results {
                let member = &mut record["result"];
                if member["full_result_resource"]
                    .as_str()
                    .is_some_and(|u| u.starts_with("qspquest://runs/"))
                    && member["run_id"]
                        .as_str()
                        .is_none_or(|id| self.cache.get(id).is_none())
                {
                    member
                        .as_object_mut()
                        .unwrap()
                        .remove("full_result_resource");
                    member["trajectory_notice"] = json!("Only sampled summaries remain inline; this trajectory was evicted from the bounded local cache. Run a smaller comparison or use save_csv for the full requested grids.");
                }
            }
            for (preview, record) in brief.iter_mut().zip(&results) {
                preview["full_result_resource"] = record["result"]["full_result_resource"].clone();
            }
            let text = usability::ordered(&[("regimens", &json!(brief))]);
            let mut result = json!({"regimens":results,"plan_hash":plan_hash,"effective_request_hash":effective_request_hash,"library_version":prepared.plan.context.library_version,"_text":text});
            self.persist_result("compare_regimens", &mut result)?;
            Ok(result)
        };
        if self.hosted {
            match tokio::time::timeout(std::time::Duration::from_secs(45), execute).await {
                Ok(result) => result.unwrap_or_else(|e| e),
                Err(_) => policy::error("deadline", "Regimen comparison preparation plus computation exceeded its aggregate 45-second budget; active isolated/native work retains its slot until it exits"),
            }
        } else {
            execute.await.unwrap_or_else(|e| e)
        }
    }
    async fn invoke(&self, operation: &str, args: Value) -> Value {
        let raw = args.get("request").is_some_and(|r| !r.is_null());
        let execute = async {
            let expected = args
                .get("expected_plan_hash")
                .filter(|v| !v.is_null())
                .cloned();
            if let Some(expected) = expected.as_ref() {
                let valid = expected
                    .as_str()
                    .and_then(|s| s.strip_prefix("sha256:"))
                    .is_some_and(|s| {
                        s.len() == 64
                            && s.bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    });
                if !valid {
                    return Err(policy::error(
                        "expected_plan_hash",
                        "expected_plan_hash must be the exact sha256: digest returned by plan_run",
                    ));
                }
            }
            let mut prepared = self.prepare_invocation(operation, args).await?;
            let plan_hash = if let Some(expected) = expected {
                let expected = expected.as_str().ok_or_else(|| {
                    policy::error(
                        "expected_plan_hash",
                        "expected_plan_hash must be the string returned by plan_run",
                    )
                })?;
                let plan = self.describe_invocation(operation, &prepared).await?;
                if expected != plan.plan_hash {
                    return Err(
                        json!({"error":{"code":"plan_mismatch","message":"Resolved source, library, inputs, preparation or deployment changed since planning.","expression":"expected_plan_hash","suggestion":"Call plan_run again, inspect the complete plan and use its new plan_hash with the same arguments."},"actual_plan_hash":plan.plan_hash,"effective_request_hash":prepared.effective_hash}),
                    );
                }
                prepared.dose_event_receipts = plan.dose_event_receipts.clone();
                prepared.scheduled_boundary_receipt = plan.scheduled_boundary_receipt.clone();
                prepared.initialization_receipt = plan.initialization_receipt.clone();
                Some(plan.plan_hash)
            } else {
                None
            };
            self.execute_prepared_invocation(operation, prepared, plan_hash)
                .await
        };
        let mut value = if self.hosted {
            match tokio::time::timeout(std::time::Duration::from_secs(45), execute).await {
                Ok(result) => result.unwrap_or_else(|e| e),
                Err(_) => policy::error(
                    "deadline",
                    "Hosted inspection plus computation exceeded its aggregate 45-second limit; active isolated/native work retains its slot until it exits.",
                ),
            }
        } else {
            execute.await.unwrap_or_else(|e| e)
        };
        if raw && value["error"].is_object() {
            let specific = value["error"]["suggestion"].as_str().unwrap_or("");
            value["error"]["suggestion"] = json!(format!("{specific} `request` uses native names and units. Friendly overrides belong at the top level and use the interface from get_model."));
        }
        value
    }
}
#[tool_router(router=basic_tools)]
impl Server {
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Search the pharmacology library, including unavailable source identities. Returns licenses, execution status, outputs and scenario IDs. Paginate with offset/limit and next_offset."
    )]
    async fn search_models(
        &self,
        Parameters(p): Parameters<Search>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let mut filters = p.filters.clone().unwrap_or_else(|| json!({}));
        if !filters.is_object() {
            filters = json!({});
        }
        if let Some(offset) = p.offset {
            filters["offset"] = json!(offset);
        }
        if let Some(limit) = p.limit {
            filters["limit"] = json!(limit.min(25));
        }
        if let Some(id) = p.model_id {
            if id.trim().is_empty() {
                return self.response(policy::error(
                    "model_id",
                    "model_id must be a nonempty exact identifier",
                ));
            }
            filters["model_id"] = json!(id);
        }
        if let Some(version) = p.version {
            if version.trim().is_empty() {
                return self.response(policy::error(
                    "version",
                    "version must be a nonempty exact version",
                ));
            }
            filters["version"] = json!(version);
        }
        let mut value = self.library_search(&p.query, &filters);
        #[cfg(feature = "managed")]
        if let Ok(scope) = crate::jobs::Scope::from_context(&ctx) {
            if scope.admitted().is_ok() {
                let extra = async {
                    crate::jobs::PlatformModelSource::configured()?
                        .search_discovery(&scope, &p.query, &filters)
                        .await
                };
                match extra.await {
                    Ok(rows) => {
                        value["platform_models"] = json!(rows);
                        value["platform_notice"] = json!(
                            "Platform records expose closed discovery fields. Null execution availability is unassessed, not unavailable; preview the intended governed Job before submission. Provider metadata and family declarations do not establish current qualification."
                        );
                    }
                    Err(e) => value["platform_error"] = e,
                }
            }
        }
        let _ = ctx;
        if let Some(rows) = value.get_mut("models").and_then(Value::as_array_mut) {
            for row in rows {
                for key in ["output_aliases", "outputs"] {
                    let n = row[key]
                        .as_object()
                        .map(|m| m.len())
                        .or(row[key].as_array().map(Vec::len))
                        .unwrap_or(0);
                    if n > 12 {
                        row[key] = json!({"count":n,"note":"get_model lists them"});
                    }
                }
            }
        }
        value["hint"] = json!(
            "Call get_model with the id for its compact interface; request detail full for equations and complete metadata. Retrieve a stored scenario with get_scenario_request."
        );
        self.response(value)
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Inspect a compact model interface with units, dose targets, citations and scenario ids. Use selection {outputs?, scenarios?, resources?} for 1 to 32 exact names or advertised inspection URIs in one complete response (40000-byte budget); bind it with expected_inspection_hash. Use page {section, offset?, limit?, expected_inspection_hash?} for bounded scalars, states, outputs or equations. Request outputs by name for card descriptors, or detail full for complete metadata."
    )]
    async fn get_model(
        &self,
        Parameters(p): Parameters<ModelCardRequest>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let value = {
            let l = self.library.read().unwrap();
            if let Some(r) = l.find(&p.model_id, p.version.as_deref()) {
                if let Err(error) =
                    crate::inspection::check_context(r, p.expected_inspection_hash.as_deref())
                {
                    drop(l);
                    return self.response(error);
                }
                if let Some(selection) = &p.selection {
                    if p.page.is_some() || p.outputs.is_some() || p.detail.is_some() {
                        return self.response(policy::error("inspection_arguments", "Use selection by itself with model_id, optional version and expected_inspection_hash; page, outputs and detail select other response modes."));
                    }
                    return self.response(match crate::inspection::select(r, selection) {
                        Ok(selected) => json!({"inspection_index_resource":selected.index_resource,"model_selection":selected}),
                        Err(error) => error,
                    });
                }
                if let Some(page) = &p.page {
                    if p.outputs.is_some() || matches!(p.detail, Some(ModelDetail::Full)) {
                        return self.response(policy::error("inspection_arguments", "Use page by itself; output-name selection and detail full return model cards."));
                    }
                    return self.response(match crate::inspection::page(r, page) {
                        Ok(page) => json!({"inspection_index_resource":page.index_resource,"model_page":page}),
                        Err(error) => error,
                    });
                }
                let mut details = model_details(r, &l);
                if let Some(names) = &p.outputs {
                    if names.is_empty() || names.len() > 32 {
                        return self.response(policy::error(
                            "outputs",
                            "get_model.outputs needs 1 to 32 canonical names or request aliases",
                        ));
                    }
                    let native = usability::augment(&r.index["native_execution"], &r.model);
                    let mapping = match usability::output_mapping(&r.model, &native, names) {
                        Ok(m) => m,
                        Err(e) => return self.response(policy::error("outputs", e)),
                    };
                    let selected: Vec<_> = usability::output_descriptors(&r.model, &native)
                        .into_iter()
                        .filter(|d| {
                            mapping
                                .iter()
                                .any(|m| m.canonical_output == d.canonical_output)
                        })
                        .collect();
                    details["interface"]["output_contract"]["descriptors"] =
                        serde_json::to_value(selected).unwrap();
                    details["interface"]["output_contract"]["descriptors_complete"] = json!(
                        details["interface"]["output_contract"]["descriptors"]
                            .as_array()
                            .map_or(0, Vec::len)
                            == r.model["outputs"].as_array().map_or(0, Vec::len)
                    );
                }
                if !details["error"].is_null() {
                    return self.response(details);
                }
                let full_requested = matches!(p.detail, Some(ModelDetail::Full));
                let value = if full_requested
                    && details.to_string().len() < policy::RESPONSE_LIMIT / 4
                {
                    let mut full = details;
                    full["detail"] = json!("full");
                    full["full_model_details_resource"] = json!(model_details_resource(r));
                    full
                } else {
                    let mut compact = compact_model_card(r, &details);
                    if full_requested {
                        compact["detail"] = json!("full_resource");
                        compact["full_details_notice"] = json!(
                            "Full model details exceed the inline response budget. Read full_model_details_resource; join and verify its chunks when a manifest is returned."
                        );
                    }
                    compact
                };
                let mut value = value;
                if value["detail"] != "full" && p.outputs.is_none() {
                    if let Some(descriptors) =
                        value["interface"]["output_contract"]["descriptors"].as_array_mut()
                    {
                        if descriptors.len() > 32 {
                            descriptors.truncate(32);
                            value["interface"]["output_contract"]["descriptors_complete"] =
                                json!(false);
                        }
                    }
                }
                match crate::inspection::index(r) {
                    Ok(index) => {
                        value["inspection_index_resource"] = json!(index.index_resource);
                        value["inspection"] = json!(index);
                    }
                    Err(error) => return self.response(error),
                }
                Some(value)
            } else {
                None
            }
        };
        if let Some(value) = value {
            return self.response(value);
        }
        if p.page.is_some() || p.selection.is_some() || p.expected_inspection_hash.is_some() {
            return self.response(policy::error("inspection_unavailable", "Paged native inspection requires a local native record. Authorized platform descriptors do not expose a native document."));
        }
        if p.outputs.is_some() {
            return self.response(policy::error("output_contract_unavailable", "Output descriptor selection requires a local native model record. Platform catalog records do not declare native output mappings; inspect their governed execution route."));
        }
        #[cfg(feature = "managed")]
        if let Ok(scope) = crate::jobs::Scope::from_context(&ctx) {
            if scope.admitted().is_ok() {
                let result = async {
                    crate::jobs::PlatformModelSource::configured()?
                        .model(&scope, &p.model_id)
                        .await
                };
                return self.response(match result.await {
                    Ok(model) => {
                        let data = model.get("data").unwrap_or(&model);
                        match crate::discovery::platform_model(data) {
                            Ok(projected) if projected.id != p.model_id => policy::error("platform_response", "The platform returned a different model identifier"),
                            Ok(projected) if p.version.as_deref().is_some_and(|wanted| projected.version.as_deref() != Some(wanted)) => policy::error("unsupported_version", "The authorized active platform record does not resolve this exact version; select its reported version or omit version when unrecorded"),
                            Ok(projected) => json!({"platform_model":model,"discovery":projected.discovery,"execution_available":projected.execution_available,"platform_descriptor":{"model_id":p.model_id,"native_projection":false,"execution_route":"governed_job"},"model_source":"authorized_platform_catalog","scientific_status":"Preview the intended governed Job before submission. Execution availability and provider readiness are not current scientific qualification."}),
                            Err(message) => policy::error("platform_response", message),
                        }
                    },
                    Err(e) => e,
                });
            }
        }
        let _ = ctx;
        let wanted = p.model_id.to_lowercase();
        let near: Vec<String> = self
            .library
            .read()
            .unwrap()
            .records
            .iter()
            .filter_map(|r| r.metadata["id"].as_str())
            .filter(|id| {
                wanted
                    .split(|c: char| !c.is_ascii_alphanumeric())
                    .any(|part| part.len() > 3 && id.to_lowercase().contains(part))
            })
            .take(8)
            .map(str::to_owned)
            .collect();
        self.response(policy::error(
            "not_found",
            if near.is_empty() {
                "Model not found. Use search_models to find the exact model_id.".to_owned()
            } else {
                format!(
                    "Model not found. Closest ids: {}. Use search_models for others.",
                    near.join(", ")
                )
            },
        ))
    }
    #[tool(output_schema = crate::contract::output_schema(), description = "Get the complete versioned request for a named library scenario.")]
    async fn get_scenario_request(&self, Parameters(p): Parameters<Scenario>) -> CallToolResult {
        let l = self.library.read().unwrap();
        let v = l
            .find(&p.model_id, p.version.as_deref())
            .and_then(|r| r.scenarios.get(&p.scenario_id).map(|request| (r, request)))
            .map(|(r, request)| {
                if let Err(error) = crate::inspection::check_context(r, p.expected_inspection_hash.as_deref()) { return error; }
                let mut request = request.clone();
                let precision_profile_receipt = if let Some(profiles) = &r.precision_profiles {
                    if let Err(error) = profiles.apply(&p.scenario_id, p.precision_profile_id.as_deref(), &mut request) {return error;}
                    match profiles.receipt(&p.scenario_id, p.precision_profile_id.as_deref(), &request, &r.model, false) {Ok(receipt)=>Some(receipt), Err(error)=>return error}
                } else {
                    if p.precision_profile_id.is_some() {return policy::error("precision_profiles_unavailable", "No precision profiles declared for this exact archive; inspect get_model.");}
                    None
                };
                let readable = usability::readable_request(&request, &r.index["native_execution"], &r.model);
                let text = usability::ordered(&[
                    ("scenario", &json!({"model_id":p.model_id,"scenario_id":p.scenario_id})),
                    ("readable_request", &readable),
                    ("precision_profile_receipt", &json!(precision_profile_receipt)),
                    ("hint", &json!("To run or vary it, call simulate with model {scenario:{model_id, scenario_id}} and override doses, parameters, times or readouts by name. structuredContent holds the selected complete native request. A precision_profile_id selects every mutable scalar parameter; later scalar or timed overrides are labelled modified.")),
                ]);
                json!({"request":request,"precision_profile_receipt":precision_profile_receipt,"readable_request":readable,"_text":text})
            })
            .unwrap_or_else(|| policy::error("not_found", "Scenario not found"));
        drop(l);
        self.response(v)
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Validate .qsp text or model JSON, returning canonical .qsp, model hash or a located diagnostic."
    )]
    async fn validate_model(
        &self,
        Parameters(p): Parameters<Source>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let scoped = self.scoped(&ctx);
        let _ = self;
        let server = &scoped;
        let r = server
            .compute(p.source, "validate_model", json!({}), false)
            .await
            .unwrap_or_else(|e| e);
        server.response(r)
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Copy exact computed simulation columns into requested response keys, with explicit pre/post/both event-side selection and complete per-run mapping receipts. Optionally reject a proposed final response with missing conditions/keys, wrong identifiers, event objects, nonnumeric arrays or changed values. Reads retained local results or pinned supplied complete JSON; performs no solve, interpolation, unit conversion, file write or job submission. Supplied data is not producer authentication or scientific qualification.")]
    async fn format_response(
        &self,
        Parameters(p): Parameters<crate::response_format::FormatRequest>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let scoped = self.scoped(&ctx);
        scoped.response(scoped.format_response_value(p))
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Query a six-dimensional capability key and optional native model/version/scenario. Reports source implementation, active exposure, model blockers and exact-evidence qualification status. Null executable means normal request admission and execution are still required. Performs no solve, fit, job submission, file export or private platform lookup.")]
    async fn get_capability(
        &self,
        Parameters(p): Parameters<crate::capability::Query>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        if serde_json::to_vec(&p).unwrap().len() > 16_384 {
            return self.response(policy::error(
                "input_limit",
                "Capability query exceeds 16 KiB; supply a capability key and model identity only.",
            ));
        }
        let scoped = self.scoped(&ctx);
        let caps = scoped.capabilities();
        let tools: Vec<String> = serde_json::from_value(caps["tools"].clone()).unwrap();
        let features = serde_json::from_value(caps["features"].clone()).unwrap();
        #[cfg(feature = "managed")]
        let verified =
            crate::jobs::Scope::from_context(&ctx).is_ok_and(|scope| scope.admitted().is_ok());
        #[cfg(not(feature = "managed"))]
        let verified = false;
        let runtime = crate::capability::Runtime {
            target: crate::capability::TARGET,
            mode: scoped.capability_mode(),
            tools: &tools,
            features: &features,
            governed_ready: verified && caps["deployment"]["governed_backend_configured"] == true,
            local_jobs_ready: scoped.local_jobs.is_some(),
            user_source_execution: caps["deployment"]["user_source_execution"] == true,
        };
        let library = scoped.library.read().unwrap();
        let record = p
            .model
            .as_ref()
            .and_then(|model| library.find(&model.model_id, model.version.as_deref()));
        if let Some(expected) = p
            .model
            .as_ref()
            .and_then(|m| m.expected_inspection_hash.as_deref())
        {
            let checked = record.ok_or_else(|| policy::error("inspection_unavailable", "The inspected native model/version is unavailable; inspect the current model before migrating input names."))
                .and_then(|r| crate::inspection::check_context(r, Some(expected)));
            if let Err(error) = checked {
                drop(library);
                return scoped.response(error);
            }
        }
        let result = crate::capability::answer(&p, &runtime, record);
        drop(library);
        scoped.response(serde_json::to_value(result).unwrap())
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Get a validated unit-bearing illustrative model template: iv_one_compartment, oral, two_compartment, tmdd_qss, idr_inhibition, idr_stimulation, transit."
    )]
    async fn model_template(&self, Parameters(p): Parameters<Template>) -> CallToolResult {
        if p.options
            .as_ref()
            .is_some_and(|o| o.as_object().is_none_or(|o| !o.is_empty()))
        {
            return self.response(policy::error("unsupported_options","Template options are reserved. Edit the returned canonical_source and validate_model instead."));
        }
        let value = match templates(&p.kind) {
            Some(source) => match compute::execute(source, "validate_model", json!({})) {
                Ok(mut v) => {
                    v["scientific_status"] =
                        json!("Synthetic/illustrative template; no drug-specific validation");
                    v
                }
                Err(e) => json!({"error":e}),
            },
            None => policy::error("template", "Unknown template kind"),
        };
        self.response(value)
    }
    #[tool(output_schema = crate::contract::output_schema(), description = "Submit 1..25 ordered independent local work units with a stable retry key. Requires local stdio --jobs-dir and --artifact-dir in distinct private directories. Every unit passes the normal typed admission and no-solve preparation before a job is created. Same-key same-input retries return the same job. Admission is metadata; inspect get_local_job and every complete result artifact for scientific status. No hosted job or charge is created.")]
    async fn submit_local_job(
        &self,
        Parameters(p): Parameters<crate::local_jobs::Submit>,
    ) -> CallToolResult {
        self.response(
            self.submit_local_work(p)
                .await
                .unwrap_or_else(|error| error),
        )
    }
    #[tool(output_schema = crate::contract::output_schema(), description = "Inspect a local job's state, ordered committed units, progress, retryability and retained result artifacts. Metadata success does not mean scientific completion; read each complete artifact and its scientific status.")]
    async fn get_local_job(
        &self,
        Parameters(p): Parameters<crate::local_jobs::JobId>,
    ) -> CallToolResult {
        self.response(
            self.local_job_store()
                .and_then(|store| store.get(&p.job_id))
                .map(|job| json!({"local_job":job}))
                .unwrap_or_else(|error| error),
        )
    }
    #[tool(output_schema = crate::contract::output_schema(), description = "Cancel queued local work immediately, or request cancellation after the current bounded native unit drains and its result is retained. Completed units are never discarded. Repeated cancellation returns the current state.")]
    async fn cancel_local_job(
        &self,
        Parameters(p): Parameters<crate::local_jobs::JobId>,
    ) -> CallToolResult {
        self.response(
            self.local_job_store()
                .and_then(|store| store.cancel(&p.job_id))
                .map(|job| json!({"local_job":job}))
                .unwrap_or_else(|error| error),
        )
    }
    #[tool(output_schema = crate::contract::output_schema(), description = "Resume cancelled or process-interrupted local work using the original executable, job directory and verified result artifacts. Skip committed units; revalidate each uncommitted unit against its admission plan. If an interrupted unit may already have run, retry_uncommitted:true explicitly permits its replay. At most three resumes; scientific/request failures require corrected inputs and a new key. No mid-solve checkpoint.")]
    async fn resume_local_job(
        &self,
        Parameters(p): Parameters<crate::local_jobs::Resume>,
    ) -> CallToolResult {
        self.response(self.resume_local_work(p).unwrap_or_else(|error| error))
    }
    #[tool(output_schema = crate::contract::output_schema(), description = "Prepare initial patient quantities, a simulation, named regimen comparison, scan, fit or available periodic steady-state or Morris request without integration, likelihood evaluation, optimization, simulation random draws, file export or job submission. Inspect units, event schedule, workload ceilings, assumptions, deferred checks and qualification limits. Execute the same arguments with expected_plan_hash. Large local plans have a complete chunked resource; a plan reserves no execution or qualification.")]
    async fn plan_run(
        &self,
        Parameters(p): Parameters<crate::planning::PlanRequest>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let scoped = self.scoped(&ctx);
        let (operation, args) = p.into_parts();
        scoped.response(scoped.plan_invocation(&operation, args).await)
    }
    #[tool(output_schema = crate::contract::output_schema(), description="Simulate a library/scenario/source model with unit-bearing overrides and doses. Returns sampled-grid exposure summaries (not NCA), identity and a labeled compact preview by default. Use required_outputs to require named columns and any declared analyte/compartment before integration; unfulfilled requirements return errors without numeric placeholders. For the complete requested trajectory use detail full, or read the linked full_result_resource when capped.",meta=serde_json::from_value::<MetaObject>(json!({"ui":{"resourceUri":"ui://qspquest/simulation"}})).unwrap())]
    async fn simulate(
        &self,
        Parameters(p): Parameters<Simulate>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let scoped = self.scoped(&ctx);
        let _ = self;
        let server = &scoped;
        let args = serde_json::to_value(p).unwrap();
        let model = args["model"].clone();
        server.response(server.domain_hint(server.invoke("simulate", args).await, &model))
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Model quantities for one patient. Give covariates by name with units (for example body weight, albumin); evaluates model outputs at time zero before administrations, with declared engine units, bound inputs and identity. Model or scenario initial states are retained; no future interval is integrated. Select outputs when needed. Inspect plan_run and execute with expected_plan_hash."
    )]
    async fn get_patient_parameters(
        &self,
        Parameters(p): Parameters<Patient>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let scoped = self.scoped(&ctx);
        let _ = self;
        let server = &scoped;
        let args = serde_json::to_value(p).unwrap();
        let model = args["model"].clone();
        server.response(
            server.domain_hint(server.invoke("get_patient_parameters", args).await, &model),
        )
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Summarize a concentration-time profile you already have (from a file, a paper or your own run): cmax, tmax, cmin, AUC, average and terminal half-life, plus time below and above an optional threshold."
    )]
    async fn summarize_profile(&self, Parameters(p): Parameters<Profile>) -> CallToolResult {
        if p.times.len() < 2
            || p.times.len() != p.values.len()
            || p.times.windows(2).any(|w| w[1] < w[0])
        {
            return self.response(policy::error(
                "profile",
                "times and values must have the same length (at least 2) and times must ascend",
            ));
        }
        let (from, to) = (p.times[0], *p.times.last().unwrap());
        let result = json!({"time_unit":p.time_unit.clone().unwrap_or_default(),"times":p.times,"outputs":[{"name":"profile","values":p.values}]});
        let window = json!({"output":"profile","from":{"value":from},"to":{"value":to}});
        let mut specs =
            vec![json!({"kind":"nca","output":"profile","from":window["from"],"to":window["to"]})];
        if let Some(c) = p.threshold {
            for kind in ["time_below", "time_above"] {
                specs.push(json!({"kind":kind,"output":"profile","threshold":c,"from":window["from"],"to":window["to"]}));
            }
        }
        match usability::readouts(&result, &json!(specs), &Value::Null) {
            Ok(mut r) => {
                r["method"] = json!(
                    "Trapezoidal AUC on the supplied points; crossings interpolated linearly between points."
                );
                let text =
                    usability::ordered(&[("summary", &r["results"]), ("method", &r["method"])]);
                r["_text"] = json!(text);
                self.response(r)
            }
            Err(e) => self.response(policy::error("profile", e)),
        }
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Compare 1..25 uniquely named conditions using the same model and base request. Each entry has name, doses and optional starting parameters, initial_states and timed parameter_changes. Starting member quantities replace matching shared names. Every member is prepared before any solve; input receipts bind requested and native parameter contracts to separate computed results. Inspect plan_run and bind execution with expected_plan_hash. CSV export begins after all numerical runs succeed; required_outputs is checked for every member."
    )]
    async fn compare_regimens(
        &self,
        Parameters(p): Parameters<Compare>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let scoped = self.scoped(&ctx);
        let _ = self;
        let server = &scoped;
        server.response(
            server
                .execute_comparison(serde_json::to_value(p).unwrap())
                .await,
        )
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Scan one parameter over 1..25 explicit quantities or a unit-bearing range. Returns compact trajectories and summaries."
    )]
    async fn scan_parameters(
        &self,
        Parameters(p): Parameters<Scan>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let scoped = self.scoped(&ctx);
        let _ = self;
        let server = &scoped;
        server.response(
            server
                .invoke("scan_parameters", serde_json::to_value(p).unwrap())
                .await,
        )
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Fit one estimated parameter with explicit bounds and Gaussian observations; supports a native scalar fit_request."
    )]
    async fn fit_scalar(
        &self,
        Parameters(p): Parameters<ScalarFit>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let scoped = self.scoped(&ctx);
        let _ = self;
        let server = &scoped;
        server.response(
            server
                .invoke("fit_scalar", serde_json::to_value(p).unwrap())
                .await,
        )
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Sensitivity-based bounded Gaussian individual fitting. Provide parameters_to_fit, observations or a versioned native fit_request."
    )]
    async fn fit_individual(
        &self,
        Parameters(p): Parameters<IndividualFit>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let scoped = self.scoped(&ctx);
        let _ = self;
        let server = &scoped;
        server.response(
            server
                .invoke("fit_individual", serde_json::to_value(p).unwrap())
                .await,
        )
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Native bounded FOCEI, Laplace or SAEM population fit. Explicit NONMEM row mappings; hosted limits 16 subjects/256 observations/300 iterations and 45 seconds."
    )]
    async fn fit_population(
        &self,
        Parameters(p): Parameters<PopulationFit>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let scoped = self.scoped(&ctx);
        let _ = self;
        let server = &scoped;
        server.response(
            server
                .invoke("fit_population", serde_json::to_value(p).unwrap())
                .await,
        )
    }
}
pub fn templates(kind: &str) -> Option<&'static str> {
    match kind {
        "iv_one_compartment" => Some(include_str!("../assets/templates/iv.qsp")),
        "oral" => Some(include_str!("../assets/templates/oral.qsp")),
        "two_compartment" => Some(include_str!("../assets/templates/two.qsp")),
        "tmdd_qss" => Some(include_str!("../assets/templates/tmdd.qsp")),
        "idr_inhibition" => Some(include_str!("../assets/templates/idr-inhibition.qsp")),
        "idr_stimulation" => Some(include_str!("../assets/templates/idr-stimulation.qsp")),
        "transit" => Some(include_str!("../assets/templates/transit.qsp")),
        _ => None,
    }
}
#[tool_handler]
impl ServerHandler for Server {
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut tools = Self::tool_router().list_all();
        for tool in &mut tools {
            if tool.name.as_ref() == "get_artifact" {
                tool.annotations = Some(ToolAnnotations::from_raw(
                    None,
                    Some(true),
                    Some(false),
                    Some(true),
                    Some(true),
                ));
            }
        }
        if self.artifact_store.is_some() {
            for tool in &mut tools {
                if matches!(
                    tool.name.as_ref(),
                    "simulate"
                        | "compare_regimens"
                        | "get_patient_parameters"
                        | "scan_parameters"
                        | "fit_scalar"
                        | "fit_individual"
                        | "fit_population"
                        | "steady_state"
                        | "morris_screening"
                ) {
                    tool.annotations = Some(ToolAnnotations::from_raw(
                        None,
                        Some(false),
                        Some(false),
                        Some(false),
                        Some(false),
                    ));
                }
            }
        }
        Ok(ListToolsResult {
            tools,
            next_cursor: None,
            meta: None,
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if matches!(
            request.name.as_ref(),
            "plan_run"
                | "format_response"
                | "simulate"
                | "scan_parameters"
                | "compare_regimens"
                | "get_patient_parameters"
                | "fit_scalar"
                | "fit_individual"
                | "fit_population"
                | "steady_state"
                | "morris_screening"
                | "submit_local_job"
                | "get_local_job"
                | "cancel_local_job"
                | "resume_local_job"
        ) {
            if let Some(arguments) = &request.arguments {
                let arguments = Value::Object(arguments.clone());
                if arguments.to_string().len() > policy::INPUT_LIMIT {
                    return Ok(self
                        .response(policy::error("input_limit", "Tool arguments exceed 1 MB"))
                        .into());
                }
                if let Err(error) =
                    crate::core::time_validation::finite_inputs(&arguments, "arguments")
                {
                    return Ok(self.response(error).into());
                }
            }
        }
        let response = Self::tool_router()
            .call(rmcp::handler::server::tool::ToolCallContext::new(
                self, request, context,
            ))
            .await?;
        match response {
            CallToolResponse::Complete(result) if result.structured_content.is_none() => {
                // Framework argument errors must obey the advertised result contract too.
                let diagnostic = result
                    .content
                    .iter()
                    .filter_map(|block| block.as_text())
                    .map(|text| text.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let code = if diagnostic.starts_with("failed to deserialize parameters:") {
                    "invalid_tool_arguments"
                } else {
                    "invalid_tool_result"
                };
                Ok(self
                    .response(json!({"error": {
                        "code": code,
                        "message": diagnostic.chars().take(4096).collect::<String>(),
                        "suggestion": "Use the inputSchema returned by tools/list for this tool."
                    }}))
                    .into())
            }
            response => Ok(response),
        }
    }
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(serde_json::from_value(json!({"tools":{},"resources":{},"prompts":{},"extensions":{"io.modelcontextprotocol/ui":{"mimeTypes":["text/html;profile=mcp-app"]}}})).unwrap()).with_server_info(Implementation::new("qspquest-mcp", env!("CARGO_PKG_VERSION"))).with_instructions("Workflow: search_models to find a published model; get_model for its compact interface (dose targets, parameters with units, outputs, scenario ids; detail full for equations and complete metadata); get_model.selection for known output names, stored scenarios and batched advertised inspection resources, binding later calls with expected_inspection_hash; get_patient_parameters for the clearance, volume and other quantities the model derives for given covariates; simulate or compare_regimens with doses, parameters, parameter_changes (a parameter that changes at a given time), times and readouts (window AUC, minimum, time below a threshold, pre-dose trough, nca) for the numbers. Use the names and units shown by get_model; unit \"1\" means the model's own number. Results lead with the requested values; save_csv writes the full CSV trajectory and a .manifest.json with units, effective inputs, identity and qualification limits; keep both files together. Read full_result_resource when the contract links a resource and you need the complete numerical payload. Use format_response for requested scalar arrays and decision identifiers: declare every condition, actual returned column, unit, time and explicit pre/post/both event side; inspect the source and copying receipts. Missing conditions and event objects inside scalar arrays fail; formatting does not qualify science. Preset scenarios are declared input sets; recorded evidence covers only its named scope. A modified run needs applicable evidence. Label synthetic examples and nonconverged fits as such.")
    }
    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let mut uris = vec![
            "qspquest://capabilities".to_owned(),
            "qspquest://schema/mcp-result".to_owned(),
            "qspquest://schema/capabilities".to_owned(),
            "qspquest://schema/result-artifact".to_owned(),
            "qspquest://schema/local-job".to_owned(),
            "qspquest://schema/library-fitting".to_owned(),
            "qspquest://schema/model-discovery".to_owned(),
            "qspquest://schema/source-family".to_owned(),
            "qspquest://schema/model-inspection".to_owned(),
            "qspquest://schema/model-inspection-page".to_owned(),
            crate::capability::REGISTRY_URI.to_owned(),
            "qspquest://schema/capability-registry".to_owned(),
            crate::table::SCHEMA_URI.to_owned(),
        ];
        #[cfg(feature = "managed")]
        uris.extend([
            "qspquest://schema/managed-result-artifact".into(),
            "qspquest://schema/managed-artifact-chunk".into(),
        ]);
        if let Some(store) = &self.artifact_store {
            uris.extend(
                store.uris().map_err(|e| {
                    ErrorData::invalid_params(e.message, Some(json!({"code":e.code})))
                })?,
            );
        }
        let documentation = crate::documentation::resources();
        uris.extend(documentation.iter().map(|(uri, _)| uri.clone()));
        uris.extend(
            crate::discovery::family_records(&self.library.read().unwrap().records)
                .map_err(|message| ErrorData::invalid_params(message, None))?
                .iter()
                .map(crate::discovery::family_uri),
        );
        for r in &self.library.read().unwrap().records {
            let index = crate::inspection::index(r).map_err(|error| {
                ErrorData::invalid_params("Invalid inspection metadata", Some(error))
            })?;
            uris.push(index.index_resource);

            uris.push(format!(
                "qspquest://models/{}",
                r.metadata["id"].as_str().unwrap()
            ));
            for s in r.scenarios.keys() {
                uris.push(format!(
                    "qspquest://models/{}/scenarios/{s}",
                    r.metadata["id"].as_str().unwrap()
                ))
            }
        }
        for name in [
            "scalar",
            "individual-additive",
            "individual-proportional",
            "population",
            "library-scalar",
        ] {
            uris.push(format!("qspquest://examples/fit/{name}"))
        }
        uris.push("ui://qspquest/simulation".into());
        serde_json::from_value(json!({"resources":uris.into_iter().map(|u|json!({"uri":u,"name":u,"mimeType":documentation.iter().find(|(uri,_)|uri==&u).map(|(_,mime)|mime.as_str()).unwrap_or(if u.starts_with("ui:"){"text/html;profile=mcp-app"}else{"application/json"})})).collect::<Vec<_>>()})).map_err(|e|ErrorData::internal_error(e.to_string(),None))
    }
    async fn read_resource(
        &self,
        p: ReadResourceRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        #[cfg(feature = "managed")]
        if p.uri.starts_with(crate::managed_artifacts::URI_PREFIX) {
            let result = async {
                let scope = crate::jobs::Scope::from_context(&ctx)?;
                let backend = crate::jobs::HttpJobBackend::configured()?;
                scope.admitted()?;
                let _permit = self.semaphore.clone().try_acquire_owned()
                    .map_err(|_| policy::error("resource_busy", "Server compute/retrieval slots are occupied; retry this unchanged resource URI"))?;
                tokio::time::timeout(std::time::Duration::from_secs(15), backend.artifact_resource(&scope, &p.uri)).await
                    .map_err(|_| policy::error("managed_artifact_timeout", "Authenticated artifact retrieval exceeded 15 seconds; retry the unchanged URI before expiration"))?
            }.await.map_err(|failure: Value| ErrorData::invalid_params(
                failure["error"]["message"].as_str().unwrap_or("Managed artifact retrieval failed").to_owned(),
                Some(failure)))?;
            return serde_json::from_value::<ReadResourceResult>(json!({"contents":[{"uri":p.uri,"mimeType":"application/json","text":result.to_string()}]}))
                .map(Into::into).map_err(|e| ErrorData::internal_error(e.to_string(), None));
        }
        let _ = ctx;
        if let Some(path) = p.uri.strip_prefix(crate::artifacts::URI_PREFIX) {
            let store = self.artifact_store.as_ref().ok_or_else(|| {
                ErrorData::invalid_params(
                    "Local result artifact storage is disabled",
                    Some(json!({"code":"artifact_storage_disabled"})),
                )
            })?;
            let (id, chunk) = path
                .split_once("/chunks/")
                .map_or((path, None), |(id, index)| (id, Some(index)));
            let (mime, text) = if let Some(index) = chunk {
                let index = index.parse::<usize>().map_err(|_| {
                    ErrorData::invalid_params(
                        "Invalid artifact chunk index",
                        Some(json!({"code":"artifact_chunk"})),
                    )
                })?;
                ("text/plain; charset=utf-8", store.chunk(id, index))
            } else {
                (
                    "application/json",
                    store
                        .descriptor(id)
                        .map(|d| serde_json::to_string(&d).unwrap()),
                )
            };
            let text = text
                .map_err(|e| ErrorData::invalid_params(e.message, Some(json!({"code":e.code}))))?;
            return serde_json::from_value::<ReadResourceResult>(
                json!({"contents":[{"uri":p.uri,"mimeType":mime,"text":text}]}),
            )
            .map(Into::into)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None));
        }
        let (base_uri, chunk) = p
            .uri
            .split_once("/chunks/")
            .map_or((p.uri.as_str(), None), |(a, b)| {
                (a, b.parse::<usize>().ok())
            });
        let uri = base_uri;
        let mut mime = "application/json";
        let text = if uri == "qspquest://capabilities" {
            self.capabilities().to_string()
        } else if uri == crate::capability::REGISTRY_URI {
            self.capability_registry().to_string()
        } else if uri == "qspquest://schema/capability-registry" {
            crate::capability::schema().to_string()
        } else if uri == "qspquest://schema/result-artifact" {
            serde_json::to_string(&schemars::schema_for!(crate::artifacts::Descriptor)).unwrap()
        } else if uri == "qspquest://schema/managed-result-artifact" {
            #[cfg(feature = "managed")]
            {
                crate::contract::schema::<crate::managed_artifacts::Descriptor>().to_string()
            }
            #[cfg(not(feature = "managed"))]
            {
                return Err(ErrorData::invalid_params(
                    "Managed artifact retrieval is not compiled",
                    Some(json!({"code":"managed_unavailable"})),
                ));
            }
        } else if uri == "qspquest://schema/managed-artifact-chunk" {
            #[cfg(feature = "managed")]
            {
                crate::contract::schema::<crate::managed_artifacts::Chunk>().to_string()
            }
            #[cfg(not(feature = "managed"))]
            {
                return Err(ErrorData::invalid_params(
                    "Managed artifact retrieval is not compiled",
                    Some(json!({"code":"managed_unavailable"})),
                ));
            }
        } else if uri == "qspquest://schema/local-job" {
            crate::local_jobs::schema().to_string()
        } else if uri == "qspquest://schema/library-fitting" {
            crate::contract::schema::<crate::fitting::Declaration>().to_string()
        } else if uri == "qspquest://schema/model-discovery" {
            serde_json::to_string(&schemars::schema_for!(crate::discovery::ModelDiscovery)).unwrap()
        } else if uri == "qspquest://schema/model-inspection" {
            crate::contract::schema::<crate::inspection::Index>().to_string()
        } else if uri == "qspquest://schema/model-inspection-page" {
            crate::contract::schema::<crate::inspection::Page>().to_string()
        } else if uri == "qspquest://schema/source-family" {
            serde_json::to_string(&schemars::schema_for!(crate::discovery::FullSourceFamily))
                .unwrap()
        } else if let Some(id) = uri.strip_prefix("qspquest://source-families/") {
            if id.len() != 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(ErrorData::invalid_params(
                    "Use the exact source-family URI returned by discovery",
                    None,
                ));
            }
            let family = crate::discovery::family_records(&self.library.read().unwrap().records)
                .map_err(|message| ErrorData::invalid_params(message, None))?
                .into_iter()
                .find(|family| crate::discovery::family_uri(family) == uri)
                .ok_or_else(|| {
                    ErrorData::invalid_params(
                        "Source family not found in this library snapshot",
                        None,
                    )
                })?;
            serde_json::to_string(&family).unwrap()
        } else if uri == crate::table::SCHEMA_URI {
            crate::table::schema().to_string()
        } else if uri == "qspquest://schema/mcp-result" {
            serde_json::to_value(crate::contract::output_schema())
                .unwrap()
                .to_string()
        } else if uri == "qspquest://schema/capabilities" {
            crate::contract::capability_schema().to_string()
        } else if let Some(document) = crate::documentation::read(uri) {
            let (document_mime, text) =
                document.map_err(|message| ErrorData::invalid_params(message, None))?;
            let text = resource_part(&text, base_uri, chunk)?;
            return serde_json::from_value::<ReadResourceResult>(
                json!({"contents":[{"uri":p.uri,"mimeType":document_mime,"text":text}]}),
            )
            .map(Into::into)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None));
        } else if uri == "ui://qspquest/simulation" {
            mime = "text/html;profile=mcp-app";
            include_str!("../assets/widget.html").into()
        } else if let Some(id) = uri.strip_prefix("qspquest://plans/") {
            if self.hosted {
                return Err(ErrorData::invalid_params(
                    "Plan resources exist only in local stdio mode",
                    None,
                ));
            }
            self.cache
                .get(&format!("plan:{id}"))
                .ok_or_else(|| {
                    ErrorData::invalid_params(
                        "Plan expired or not found; call plan_run again",
                        None,
                    )
                })?
                .to_string()
        } else if let Some(id) = uri.strip_prefix("qspquest://runs/") {
            if self.hosted {
                return Err(ErrorData::invalid_params(
                    "Run resources exist only in local stdio mode",
                    None,
                ));
            }
            self.cache
                .get(id)
                .ok_or_else(|| ErrorData::invalid_params("Run expired or not found", None))?
                .to_string()
        } else if let Some(name) = uri.strip_prefix("qspquest://examples/fit/") {
            match name {
                "scalar" => include_str!("../assets/fits/scalar.json"),
                "individual-additive" => include_str!("../assets/fits/individual-additive.json"),
                "individual-proportional" => {
                    include_str!("../assets/fits/individual-proportional.json")
                }
                "population" => include_str!("../assets/fits/population.json"),
                "library-scalar" => include_str!("../assets/fits/library-scalar.json"),
                _ => return Err(ErrorData::invalid_params("Unknown example", None)),
            }
            .into()
        } else if let Some(path) = uri.strip_prefix("qspquest://models/") {
            if let Some((id, inspection)) = path.split_once("/inspection/") {
                let digest = inspection.split('/').next().unwrap_or("");
                let l = self.library.read().unwrap();
                let r = l.records.iter().filter(|r| r.metadata["id"] == id).find(|r| crate::inspection::index(r).is_ok_and(|index| index.context.inspection_hash.trim_start_matches("sha256:") == digest)).ok_or_else(|| ErrorData::invalid_params("Inspection content changed or was not found; read the current model card", Some(json!({"code":"inspection_changed"}))))?;
                let value = crate::inspection::resource(r, inspection).map_err(|error| {
                    ErrorData::invalid_params("Inspection resource refused", Some(error))
                })?;
                let text = resource_part(&value.to_string(), base_uri, chunk)?;
                return serde_json::from_value::<ReadResourceResult>(
                    json!({"contents":[{"uri":p.uri,"mimeType":"application/json","text":text}]}),
                )
                .map(Into::into)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None));
            }
            if let Some((id, version)) = path.split_once("/details/") {
                let l = self.library.read().unwrap();
                let r = l.find(id, Some(version)).ok_or_else(|| {
                    ErrorData::invalid_params(
                        "Model version not found; inspect the current model card",
                        None,
                    )
                })?;
                let mut value = model_details(r, &l);
                value.as_object_mut().unwrap().remove("_text");
                value["detail"] = json!("full");
                value["library_version"] = json!(l.version);
                let text = resource_part(&value.to_string(), base_uri, chunk)?;
                return serde_json::from_value::<ReadResourceResult>(
                    json!({"contents":[{"uri":p.uri,"mimeType":"application/json","text":text}]}),
                )
                .map(Into::into)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None));
            }
            let (id, sid) = path
                .split_once("/scenarios/")
                .map_or((path, None), |(a, b)| (a, Some(b)));
            let l = self.library.read().unwrap();
            let r = l
                .find(id, None)
                .ok_or_else(|| ErrorData::invalid_params("Model not found", None))?;
            if let Some(s) = sid {
                r.scenarios
                    .get(s)
                    .ok_or_else(|| ErrorData::invalid_params("Scenario not found", None))?
                    .to_string()
            } else {
                r.source_text()
            }
        } else {
            return Err(ErrorData::invalid_params("Unknown resource", None));
        };
        let text = resource_part(&text, base_uri, chunk)?;
        serde_json::from_value::<ReadResourceResult>(json!({"contents":[{"uri":p.uri,"mimeType":mime,"text":text,"_meta":if base_uri.starts_with("ui:"){widget_meta()}else{json!({})}}]})).map(Into::into).map_err(|e|ErrorData::internal_error(e.to_string(),None))
    }
    async fn list_prompts(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        serde_json::from_value(json!({"prompts":[{"name":"build-pk-model","description":"Build a unit-bearing model"},{"name":"simulate-regimen","description":"Simulate a library regimen"},{"name":"fit-observations","description":"Prepare and assess a bounded fit"},{"name":"compare-dosing","description":"Compare named regimens"}]})).map_err(|e|ErrorData::internal_error(e.to_string(),None))
    }
    async fn get_prompt(
        &self,
        p: GetPromptRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        if ![
            "build-pk-model",
            "simulate-regimen",
            "fit-observations",
            "compare-dosing",
        ]
        .contains(&p.name.as_str())
        {
            return Err(ErrorData::invalid_params("Unknown prompt", None));
        }
        serde_json::from_value::<GetPromptResult>(json!({"messages":[{"role":"user","content":{"type":"text","text":format!("For {}, first read qspquest://guide/language and qspquest://guide/requests. Inspect model units and fitting capabilities, use explicit quantities, validate source edits, and report exact identity, synthetic provenance, convergence and limitations. {}",p.name,serde_json::to_string(&p.arguments).unwrap_or_default())}}]})).map(Into::into).map_err(|e|ErrorData::internal_error(e.to_string(),None))
    }
}

#[cfg(feature = "analysis")]
#[tool_router(router=analysis_tools)]
impl Server {
    #[tool(output_schema = crate::contract::output_schema(), description = "Periodic steady state; supply a native steady-state request.")]
    async fn steady_state(
        &self,
        Parameters(p): Parameters<NativeAnalysis>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let scoped = self.scoped(&ctx);
        let _ = self;
        let server = &scoped;
        server.response(
            server
                .invoke("steady_state", serde_json::to_value(p).unwrap())
                .await,
        )
    }
    #[tool(output_schema = crate::contract::output_schema(), description = "Bounded Morris screening; supply a native Morris request.")]
    async fn morris_screening(
        &self,
        Parameters(p): Parameters<MorrisAnalysis>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let scoped = self.scoped(&ctx);
        let _ = self;
        let server = &scoped;
        server.response(
            server
                .invoke("morris_screening", serde_json::to_value(p).unwrap())
                .await,
        )
    }
}
impl Server {
    fn tool_router() -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        let mut r = Self::basic_tools();
        #[cfg(feature = "analysis")]
        r.merge(Self::analysis_tools());
        #[cfg(feature = "managed")]
        r.merge(Self::managed_tools());
        for route in r.map.values_mut() {
            let name = route.attr.name.as_ref();
            let csv = matches!(name, "simulate" | "compare_regimens");
            let local_mutation = matches!(
                name,
                "submit_local_job" | "cancel_local_job" | "resume_local_job"
            );
            let submit = name.starts_with("submit_") && name != "submit_local_job";
            let platform = submit
                || name == "preview_job"
                || matches!(name, "get_job" | "wait_for_job" | "get_artifact")
                || cfg!(feature = "managed") && matches!(name, "search_models" | "get_model");
            route.attr.output_schema = Some(crate::contract::output_schema_for(name));
            route.attr.annotations = Some(ToolAnnotations::from_raw(
                None,
                Some(!csv && !submit && !local_mutation),
                Some(false),
                Some(!csv),
                Some(platform),
            ));
        }
        r
    }
}

#[cfg(feature = "managed")]
#[tool_router(router=managed_tools)]
impl Server {
    #[tool(output_schema = crate::contract::output_schema(), description = "Preview a governed simulation, population fit or scan: return fixed credit pricing, validation and approval requirements without creating a Run or Job. Requires a verified managed session. Submit only after reviewing the returned price, using its preview_digest and a stable retry key.")]
    async fn preview_job(
        &self,
        Parameters(p): Parameters<JobPreviewInput>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let result = async {
            let scope = crate::jobs::Scope::from_context(&ctx)?;
            let backend = crate::jobs::HttpJobBackend::configured()?;
            backend.preview(&scope, p.platform_request).await
        };
        self.response(result.await.unwrap_or_else(|e| e))
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Submit a heavy simulation through governed Run preview, fixed pricing and a durable Job. Requires a verified managed session and a platform-qualified simulation operation."
    )]
    async fn submit_simulation(
        &self,
        Parameters(p): Parameters<JobInput>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        if p.platform_request["capability_id"] != "pkpd.simulate_scenario" {
            return self.response(policy::error(
                "operation",
                "submit_simulation requires the governed pkpd.simulate_scenario capability",
            ));
        }
        self.response(self.submit_job(p, ctx).await)
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Submit a population fit through the platform's governed Run preview, pricing and durable Job contract. Requires a verified managed session and qualified operation."
    )]
    async fn submit_population_fit(
        &self,
        Parameters(p): Parameters<JobInput>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        if p.platform_request["capability_id"] != "pkpd.fit_data" {
            return self.response(policy::error(
                "operation",
                "submit_population_fit requires the governed pkpd.fit_data capability",
            ));
        }
        self.response(self.submit_job(p, ctx).await)
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Submit a larger scan through the platform's governed Run/Job contract; scientific tier limits remain server-authoritative."
    )]
    async fn submit_scan(
        &self,
        Parameters(p): Parameters<JobInput>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        if p.platform_request["capability_id"] != "pkpd.parameter_scan" {
            return self.response(policy::error(
                "operation",
                "submit_scan requires the governed pkpd.parameter_scan capability",
            ));
        }
        self.response(self.submit_job(p, ctx).await)
    }
    #[tool(output_schema = crate::contract::output_schema(), description = "Retrieve a governed Job accessible to the verified caller.")]
    async fn get_job(
        &self,
        Parameters(p): Parameters<JobId>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.response(self.poll_job(p, ctx, false).await)
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Wait up to 40 seconds for a caller-owned governed Job; returns current status if still running."
    )]
    async fn wait_for_job(
        &self,
        Parameters(p): Parameters<JobId>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.response(self.poll_job(p, ctx, true).await)
    }
    #[tool(output_schema = crate::contract::output_schema(),
        description = "Retrieve authorized governed Artifact metadata and its explicit immutable-byte retention state. Available artifacts provide a digest-bound full_result_resource for authenticated chunk retrieval; legacy metadata does not promise bytes."
    )]
    async fn get_artifact(
        &self,
        Parameters(p): Parameters<ArtifactId>,
        ctx: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let r = async {
            let scope = crate::jobs::Scope::from_context(&ctx)?;
            let backend = crate::jobs::HttpJobBackend::configured()?;
            backend.artifact(&scope, &p.artifact_id).await
        };
        self.response(r.await.unwrap_or_else(|e| e))
    }
}

fn resource_part(text: &str, uri: &str, chunk: Option<usize>) -> Result<String, ErrorData> {
    let mut chunks = Vec::new();
    let mut from = 0;
    while from < text.len() {
        let mut end = (from + 48_000).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1
        }
        chunks.push(&text[from..end]);
        from = end
    }
    if let Some(index) = chunk {
        return chunks
            .get(index)
            .map(|s| (*s).to_owned())
            .ok_or_else(|| ErrorData::invalid_params("Chunk not found", None));
    }
    if text.len() <= 48_000 {
        return Ok(text.to_owned());
    }
    Ok(json!({"kind":"utf8_chunk_manifest","sha256":hash(text.as_bytes()),"instructions":"Read each URI in order, concatenate content text with no separators, then verify SHA-256 and parse the original JSON/document.","chunks":(0..chunks.len()).map(|i|format!("{uri}/chunks/{i}")).collect::<Vec<_>>(),"bytes":text.len()}).to_string())
}
fn widget_meta() -> Value {
    let origins: Vec<String> = std::env::var("QSPQUEST_WIDGET_WORKER_URL")
        .ok()
        .and_then(|s| reqwest::Url::parse(&s).ok())
        .map(|u| vec![u.origin().ascii_serialization()])
        .unwrap_or_default();
    json!({"ui":{"prefersBorder":true,"csp":{"connectDomains":origins,"resourceDomains":origins.iter().cloned().chain(std::iter::once("blob:".to_owned())).collect::<Vec<_>>()}}})
}

#[cfg(test)]
mod core_library_response_tests {
    use super::*;
    fn scalar_identity_fixture() -> Value {
        json!({"model_content_hash":"model", "spec_version":"spec",
            "compiler_version":"compiler", "compiler_source_sha256":"source",
            "rustc_version":"rustc", "target":"target", "build_profile":"release",
            "rustflags_sha256":"flags", "bound_value_hash":"bound",
            "run_request_hash":"request", "backend":"diffsol",
            "backend_version":"backend", "algorithm":"bdf"})
    }
    #[test]
    fn scalar_identity_response_preserves_execution_and_convergence_status() {
        let server = Server::new(Library::embedded().unwrap(), false, Tier::Free, 2);
        let identity = scalar_identity_fixture();
        for (status, expected) in [
            ("converged", "success"),
            ("iteration_limit", "scientific_failure"),
        ] {
            let payload = json!({"schema":"qspquest.scalar-fit-result/v0.1", "status":status,
                "parameters":{"cl":{"value":0.6,"unit":"L/h"}},
                "objective":{"identity":identity,"negative_log_likelihood":-8.3}});
            let value = server.response(payload.clone()).structured_content.unwrap();
            assert_eq!(value["objective"], payload["objective"]);
            assert_eq!(value["parameters"], payload["parameters"]);
            assert_eq!(value["identity"], identity);
            assert_eq!(value["contract"]["identity"]["execution"], identity);
            assert_eq!(value["contract"]["identity"]["scope"], "execution");
            assert_eq!(value["contract"]["status"], expected);
        }
    }
    #[test]
    fn scalar_identity_persistence_preserves_execution_in_stored_result() {
        let directory = std::env::temp_dir().join(format!(
            "qspquest-scalar-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let server = Server::new(Library::embedded().unwrap(), false, Tier::Free, 2)
            .with_artifact_storage(&directory)
            .unwrap();
        let identity = scalar_identity_fixture();
        let mut payload = json!({"schema":"qspquest.scalar-fit-result/v0.1", "status":"converged",
            "objective":{"identity":identity,"negative_log_likelihood":-8.3}});
        assert!(server.persist_result("fit_scalar", &mut payload).unwrap());
        let descriptor: crate::artifacts::Descriptor =
            serde_json::from_value(payload["result_artifact"].clone()).unwrap();
        let id = descriptor
            .uri
            .strip_prefix(crate::artifacts::URI_PREFIX)
            .unwrap();
        let store = server.artifact_store.as_ref().unwrap();
        let text: String = (0..descriptor.chunk_count)
            .map(|i| store.chunk(id, i).unwrap())
            .collect();
        let stored: Value = serde_json::from_str(&text).unwrap();
        drop(server);
        std::fs::remove_dir_all(directory).unwrap();
        assert_eq!(stored["identity"], identity);
        assert_eq!(stored["objective"]["identity"], identity);
        assert_eq!(stored["contract"]["identity"]["execution"], identity);
        assert_eq!(stored["contract"]["identity"]["scope"], "execution");
    }
    #[test]
    fn scalar_identity_incomplete_or_unrelated_payload_stays_metadata() {
        let server = Server::new(Library::embedded().unwrap(), false, Tier::Free, 2);
        for payload in [
            json!({"schema":"qspquest.scalar-fit-result/v0.1",
                "objective":{"identity":{"backend":"diffsol","model_content_hash":"model"}}}),
            json!({"schema":"qspquest.metadata/v1","objective":{"identity":scalar_identity_fixture()}}),
        ] {
            let value = server.response(payload).structured_content.unwrap();
            assert_eq!(value["identity"]["backend"], "metadata");
            assert_eq!(value["contract"]["identity"]["scope"], "metadata");
            assert!(value["contract"]["identity"]["execution"].is_null());
        }
    }
    #[test]
    fn scalar_identity_explicit_top_level_identity_is_preserved() {
        let server = Server::new(Library::embedded().unwrap(), false, Tier::Free, 2);
        let mut explicit = scalar_identity_fixture();
        explicit["run_request_hash"] = json!("explicit");
        let value = server
            .response(json!({"schema":"qspquest.scalar-fit-result/v0.1",
            "status":"converged", "identity":explicit,
            "objective":{"identity":scalar_identity_fixture()}}))
            .structured_content
            .unwrap();
        assert_eq!(value["identity"], explicit);
        assert_eq!(value["contract"]["identity"]["execution"], explicit);
    }
    #[tokio::test]
    async fn compiled_cache_binds_captured_versions_and_preserves_trajectories() {
        let server = Server::new(Library::embedded().unwrap(), false, Tier::Free, 4);
        server.library.write().unwrap().version = "current-v2".into();
        let source = |rate: usize| {
            let text = format!(
                "model synthetic_decay\ntime_unit = h\nparameters\n k = {rate} [1/h] fixed\nstates\n amount = 1 [mg]\ndynamics\n d/dt(amount) = -k * amount\noutputs\n amount_out = amount [mg]\n twice_amount = 2 * amount [mg]\nend\n"
            );
            serde_json::to_string(&compute::model(&text).unwrap()).unwrap()
        };
        let request = json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf",
            "parameters":{},"budgets":{"solver_callbacks":10000,"events":100,"output_values":1000},
            "regimen":{"start":{"value":0,"unit":"h"},"end":{"value":2,"unit":"h"},
                "rtol":1e-10,"atol":1e-12,"samples":[{"value":0,"unit":"h"},{"value":1,"unit":"h"},{"value":2,"unit":"h"}],"administrations":[]}});
        for (version, rate) in [
            ("captured-v1", 1),
            ("captured-v1", 1),
            ("current-v2", 1),
            ("current-v2", 2),
        ] {
            let mut selected = request.clone();
            selected["_selected_outputs"] = json!(["amount_out"]);
            let full = server
                .compute_versioned(source(rate), "simulate", request.clone(), true, version)
                .await
                .unwrap();
            let subset = server
                .compute_versioned(source(rate), "simulate", selected, true, version)
                .await
                .unwrap();
            let full_column = full["outputs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == "amount_out")
                .unwrap();
            assert_eq!(subset["outputs"].as_array().unwrap().len(), 1);
            assert_eq!(&subset["outputs"][0], full_column);
            for (time, value) in full["times"]
                .as_array()
                .unwrap()
                .iter()
                .zip(full_column["values"].as_array().unwrap())
            {
                assert!(
                    (value.as_f64().unwrap() - (-(rate as f64) * time.as_f64().unwrap()).exp())
                        .abs()
                        < 1e-8
                );
            }
        }
        let entries = server.compiled.lock().unwrap();
        assert_eq!(
            entries.len(),
            3,
            "Same source in two versions and changed source require distinct entries"
        );
        assert!(entries
            .iter()
            .any(|(key, _)| key.starts_with("captured-v1:")));
        assert_eq!(
            entries
                .iter()
                .filter(|(key, _)| key.starts_with("current-v2:"))
                .count(),
            2
        );
    }
    #[tokio::test]
    async fn initialization_receipts_bind_predose_and_boundary_perturbations() {
        let server = Server::new(
            crate::initialization::synthetic_library(),
            false,
            policy::Tier::Free,
            1,
        );
        for (name, change) in [
            ("baseline", json!({})),
            (
                "predose",
                json!({"parameters":{"clearance":{"value":2,"unit":"1/h"}}}),
            ),
            (
                "during-predose",
                json!({"parameter_changes":[{"name":"clearance","at":{"value":-10,"unit":"h"},"value":{"value":2,"unit":"1/h"}}]}),
            ),
            (
                "boundary",
                json!({"parameter_changes":[{"name":"clearance","at":{"value":0,"unit":"h"},"value":{"value":2,"unit":"1/h"}}]}),
            ),
        ] {
            let mut args = json!({"model":{"scenario":{"model_id":"synthetic_protocol_initialization","scenario_id":"baseline"}},"outputs":["fixed_pct","condition_pct"],"detail":"full"});
            args.as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
            let planned = server.plan_invocation("simulate", args.clone()).await;
            assert!(planned["error"].is_null(), "{planned}");
            let prepared = planned["plan"]["initialization_receipt"].clone();
            assert_eq!(prepared["stage"], "prepared");
            assert_eq!(
                prepared["predose_integration"],
                json!({"start":-20.,"end":0.})
            );
            let phases = prepared["binding_phases"].as_array().unwrap();
            let timed = name == "boundary" || name == "during-predose";
            let boundary = if name == "during-predose" { -10. } else { 0. };
            assert_eq!(phases.len(), if timed { 2 } else { 1 });
            let clearance = |p: &Value| {
                p["inputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|i| i["name"] == "clearance")
                    .unwrap()["value"]
                    .as_f64()
                    .unwrap()
            };
            assert_eq!(
                clearance(&phases[0]),
                if name == "predose" { 2. } else { 1. }
            );
            if timed {
                assert_eq!(phases[1]["time"], boundary);
                assert_eq!(phases[1]["activation"], "post_boundary");
                assert_eq!(clearance(&phases[1]), 2.);
            }
            args["expected_plan_hash"] = planned["plan_hash"].clone();
            let result = server.invoke("simulate", args).await;
            assert!(result["error"].is_null(), "{result}");
            let mut executed = prepared;
            executed["stage"] = json!("executed");
            assert_eq!(result["initialization_receipt"], executed);
            assert_eq!(executed["planned_execution_identity"], result["identity"]);
            for column in result["outputs"].as_array().unwrap() {
                for (time, value) in result["times"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(column["values"].as_array().unwrap())
                {
                    let t = time.as_f64().unwrap();
                    let rate: f64 = if name == "predose" { 2. } else { 1. };
                    let mut target = 2. / rate * (-(-rate * (t + 20.)).exp_m1());
                    if timed && t >= boundary {
                        target = 1.
                            + (2. * (-(-(boundary + 20.)).exp_m1()) - 1.)
                                * (-2. * (t - boundary)).exp();
                    }
                    let denominator = if column["name"] == "fixed_pct" {
                        2.
                    } else if name == "predose" || (timed && t >= boundary) {
                        1.
                    } else {
                        2.
                    };
                    assert!(
                        (value.as_f64().unwrap() - 100. * target / denominator).abs() < 2e-6,
                        "{name} {t} {column}"
                    );
                }
            }
        }
        let args = json!({"model":{"scenario":{"model_id":"synthetic_protocol_initialization","scenario_id":"baseline"}},"parameters":{"reference":{"value":0,"unit":"mg"}},"outputs":["fixed_pct"]});
        let refused = server.plan_invocation("simulate", args.clone()).await;
        assert_eq!(refused["error"]["code"], "initialization_contract");
        let mut selected = args;
        selected["outputs"] = json!(["condition_pct"]);
        let accepted = server.plan_invocation("simulate", selected).await;
        assert!(accepted["error"].is_null(), "{accepted}");
    }
    #[tokio::test]
    async fn precision_profile_plan_execution_and_modification_are_bound() {
        let server = Server::new(
            crate::precision::synthetic_library(),
            false,
            policy::Tier::Free,
            1,
        );
        let mut args = json!({"model":{"scenario":{"model_id":"synthetic_one_compartment","scenario_id":"baseline","precision_profile_id":"alternative"}},"outputs":["cp"],"detail":"full"});
        let plan = server.plan_invocation("simulate", args.clone()).await;
        assert!(plan["error"].is_null(), "{plan}");
        assert_eq!(
            plan["plan"]["precision_profile_receipt"]["parameters_status"],
            "matches_profile"
        );
        args["expected_plan_hash"] = plan["plan_hash"].clone();
        let result = server.invoke("simulate", args.clone()).await;
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(
            result["precision_profile_receipt"],
            plan["plan"]["precision_profile_receipt"]
        );
        assert_eq!(
            result["model_provenance"]["precision_profile_receipt"],
            result["precision_profile_receipt"]
        );
        assert_eq!(
            result["identity"],
            plan["plan"]["native_preparation"]["plan"]["prospective_identity"]
        );
        let cp = result["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["name"] == "cp")
            .unwrap();
        for (i, (time, value)) in result["times"]
            .as_array()
            .unwrap()
            .iter()
            .zip(cp["values"].as_array().unwrap())
            .enumerate()
        {
            let t = time.as_f64().unwrap();
            let expected = if t == 0.0 && result["sides"][i] == "pre" {
                0.0
            } else {
                12.0 / 3.0 * (-0.6 / 3.0 * t).exp()
            };
            assert!(
                (value.as_f64().unwrap() - expected).abs() < 1e-8,
                "t={t}, side={}, actual={value}, expected={expected}",
                result["sides"][i]
            );
        }
        args["model"]["scenario"]["precision_profile_id"] = json!("baseline");
        assert_eq!(
            server.invoke("simulate", args.clone()).await["error"]["code"],
            "plan_mismatch"
        );
        args.as_object_mut().unwrap().remove("expected_plan_hash");
        args["parameters"] = json!({"cl":{"value":1.9,"unit":"L/h"}});
        let modified = server.plan_invocation("simulate", args.clone()).await;
        assert_eq!(
            modified["plan"]["precision_profile_receipt"]["parameters_status"],
            "modified"
        );
        assert_eq!(
            modified["plan"]["precision_profile_receipt"]["changed_parameter_names"],
            json!(["cl"])
        );
        args.as_object_mut().unwrap().remove("parameters");
        args["parameter_changes"] =
            json!([{"name":"cl","at":{"value":6,"unit":"h"},"value":{"value":0.9,"unit":"L/h"}}]);
        let timed = server.plan_invocation("simulate", args.clone()).await;
        assert!(timed["error"].is_null(), "{timed}");
        assert_eq!(
            timed["plan"]["precision_profile_receipt"]["parameters_status"],
            "modified"
        );
        assert_eq!(
            timed["plan"]["precision_profile_receipt"]["timed_parameter_changes_present"],
            true
        );
        args["model"]["scenario"]["precision_profile_id"] = json!("invented");
        assert_eq!(
            server.invoke("simulate", args).await["error"]["code"],
            "precision_profile_unknown"
        );
    }

    #[test]
    fn planning_input_schema_satisfies_mcp_root_object_contract() {
        let schema =
            serde_json::to_value(schemars::schema_for!(crate::planning::PlanRequest)).unwrap();
        assert_eq!(schema["type"], "object");
        assert!(schema["oneOf"].as_array().is_some());
        assert!(Server::tool_router()
            .list_all()
            .iter()
            .any(|tool| tool.name == "plan_run"));
    }
    #[tokio::test]
    async fn output_alias_mapping_binds_plans_and_matches_returned_native_columns() {
        let source = "model synthetic_source_labels\ntime_unit = h\nparameters\n k = 1 [1/h]\nstates\n x = 1 [1]\ndynamics\n d/dt(x) = -k * x\noutputs\n library_state_7 = x [1]\nend\n";
        let mut library = Library::embedded().unwrap();
        let record = &mut library.records[0];
        let id = record.metadata["id"].clone();
        record.model =
            compute::execute(source, "validate_model", json!({})).unwrap()["model"].clone();
        record.source = None;
        record.scenarios.clear();
        record.index["native_execution"] = json!({"states":{"IL6_pg_ml":{"native_name":"library_state_7","unit":"pg/mL"}},"outputs":{},"coordinates":"unchanged_source_numeric_values","dimensional_qualification":false});
        let server = Server::new(library, false, policy::Tier::Free, 1);
        let mut args = json!({"model":{"library":{"model_id":id}},"outputs":["IL6_pg_ml"],"end":{"value":1,"unit":"h"},"times":[{"value":0,"unit":"h"},{"value":1,"unit":"h"}]});
        let planned = server.plan_invocation("simulate", args.clone()).await;
        let plan = planned["plan"].clone();
        assert!(plan["error"].is_null(), "{plan}");
        assert_eq!(plan["output_mapping"][0]["requested_output"], "IL6_pg_ml");
        args["expected_plan_hash"] = plan["plan_hash"].clone();
        let result = server.invoke("simulate", args.clone()).await;
        assert!(result["error"].is_null(), "{result}");
        let retained = server
            .cache
            .get(result["run_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(retained["output_mapping"], plan["output_mapping"]);
        assert_eq!(retained["output_mapping"][0]["returned_unit"], "1");
        assert_eq!(
            retained["outputs"][0]["name"],
            retained["output_mapping"][0]["returned_column"]
        );
        assert!(
            (retained["outputs"][0]["values"][1].as_f64().unwrap() - (-1.0_f64).exp()).abs() < 1e-4
        );
        args["outputs"] = json!(["library_state_7"]);
        assert_eq!(
            server.invoke("simulate", args.clone()).await["error"]["code"],
            "plan_mismatch"
        );
        args.as_object_mut().unwrap().remove("expected_plan_hash");
        let native_planned = server.plan_invocation("simulate", args).await;
        let native_plan = native_planned["plan"].clone();
        assert_eq!(
            native_plan["effective_request_hash"],
            plan["effective_request_hash"]
        );
        assert_ne!(native_plan["plan_hash"], plan["plan_hash"]);
        let mut native_args = json!({"model":{"library":{"model_id":id}},"outputs":["library_state_7"],"end":{"value":1,"unit":"h"},"times":[{"value":0,"unit":"h"},{"value":1,"unit":"h"}]});
        native_args["expected_plan_hash"] = native_plan["plan_hash"].clone();
        let native_result = server.invoke("simulate", native_args).await;
        assert!(native_result["error"].is_null(), "{native_result}");
        assert_ne!(native_result["run_id"], result["run_id"]);
        let original = server
            .cache
            .get(result["run_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(original["output_mapping"], plan["output_mapping"]);
        let native_record = server
            .cache
            .get(native_result["run_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(original["outputs"], native_record["outputs"]);
        assert_eq!(original["identity"], native_record["identity"]);

        let patient = server
            .invoke(
                "get_patient_parameters",
                json!({"model":{"library":{"model_id":id}},"outputs":["IL6_pg_ml"]}),
            )
            .await;
        assert!(patient["error"].is_null(), "{patient}");
        assert_eq!(
            patient["output_mapping"][0]["returned_quantity"],
            "IL6_pg_ml"
        );
        assert_eq!(patient["patient_quantities"]["IL6_pg_ml"], 1.0);
        let scan = json!({"model":{"library":{"model_id":id}},"parameter":"k","values":[{"value":1,"unit":"1/h"},{"value":2,"unit":"1/h"}],"output":"IL6_pg_ml", "request":{"schema":"qspquest.run/v0.1","solver":"diffsol_bdf","budgets":{"solver_callbacks":1000000,"events":100,"output_values":10000},"regimen":{"end":{"value":1,"unit":"h"},"samples":[],"administrations":[],"rtol":1e-8,"atol":1e-10}}});
        let planned_scan = server
            .plan_invocation("scan_parameters", scan.clone())
            .await;
        assert!(planned_scan["error"].is_null(), "{planned_scan}");
        let mut scan = scan;
        scan["expected_plan_hash"] = planned_scan["plan_hash"].clone();
        let result_scan = server.invoke("scan_parameters", scan).await;
        assert!(result_scan["error"].is_null(), "{result_scan}");
        assert_eq!(
            result_scan["output_mapping"],
            planned_scan["plan"]["output_mapping"]
        );
        assert_eq!(
            result_scan["output_mapping"][0]["requested_output"],
            "IL6_pg_ml"
        );
        assert_eq!(result_scan["results"].as_array().unwrap().len(), 2);
    }
    #[tokio::test]
    async fn inactive_overrides_are_visible_in_plans_patient_scan_and_cached_results() {
        let source = "model synthetic_overrides\ntime_unit = h\nparameters\n k = 1 [1/h]\n source_scale = 1 [1]\nstates\n x = 1 [1]\ndynamics\n d/dt(x) = -k * x\noutputs\n response = x [1]\nend\n";
        let mut library = Library::embedded().unwrap();
        let record = &mut library.records[0];
        let id = record.metadata["id"].clone();
        record.model =
            compute::execute(source, "validate_model", json!({})).unwrap()["model"].clone();
        record.source = None;
        record.scenarios.clear();
        record.index["native_execution"] = json!({"parameters":{"scale":{"native_name":"source_scale","source_unit":"1","request_unit":"1"}},"outputs":{},"states":{}});
        let server = Server::new(library, false, policy::Tier::Free, 1);
        let mut args = json!({"model":{"library":{"model_id":id}},"parameters":{"scale":{"value":9,"unit":"1"}},"outputs":["response"],"end":{"value":1,"unit":"h"}});
        let planned = server.plan_invocation("simulate", args.clone()).await;
        assert!(planned["error"].is_null(), "{planned}");
        let entry = &planned["plan"]["override_receipt"]["entries"][0];
        assert_eq!(entry["requested_parameter"], "scale");
        assert_eq!(entry["canonical_parameter"], "source_scale");
        assert_eq!(entry["consumption"], "unused");
        assert!(entry["warning"].as_str().unwrap().contains("cannot change"));
        args["expected_plan_hash"] = planned["plan_hash"].clone();
        let result = server.invoke("simulate", args.clone()).await;
        assert!(result["error"].is_null(), "{result}");
        let full = server
            .cache
            .get(result["run_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            full["override_receipt"],
            planned["plan"]["override_receipt"]
        );
        args["parameters"]["scale"]["value"] = json!(99);
        assert_eq!(
            server.invoke("simulate", args.clone()).await["error"]["code"],
            "plan_mismatch"
        );
        args.as_object_mut().unwrap().remove("expected_plan_hash");
        let changed = server.invoke("simulate", args).await;
        let changed = server
            .cache
            .get(changed["run_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(changed["outputs"], full["outputs"]);
        let patient = server.invoke("get_patient_parameters", json!({"model":{"library":{"model_id":id}},"covariates":{"scale":{"value":9,"unit":"1"}},"outputs":["response"]})).await;
        assert!(patient["error"].is_null(), "{patient}");
        assert_eq!(
            patient["override_receipt"]["entries"][0]["consumption"],
            "unused"
        );
        let scan = server.plan_invocation("scan_parameters", json!({"model":{"library":{"model_id":id}},"parameter":"source_scale","values":[{"value":1,"unit":"1"},{"value":9,"unit":"1"}],"request":{"schema":"qspquest.run/v0.1","solver":"diffsol_bdf","regimen":{"end":{"value":1,"unit":"h"},"samples":[],"administrations":[],"rtol":1e-8,"atol":1e-10}},"output":"response"})).await;
        assert!(scan["error"].is_null(), "{scan}");
        assert_eq!(
            scan["plan"]["override_receipt"]["entries"][0]["consumption"],
            "unused"
        );
        assert_eq!(
            scan["plan"]["override_receipt"]["entries"][0]["effective_values"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn bounded_output_mapping_names_complete_resource_and_preserves_entry_count() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let mapping: Vec<_> = (0..1000).map(|i| json!({"requested_output":format!("alias_{i}_{}", "x".repeat(300)),"canonical_output":"cp","returned_column":"cp","returned_unit":"mg/L","declared_source_unit":null,"analyte":null,"compartment":null,"returned_quantity":null})).collect();
        let complete = json!({"effective_request_hash":"mapping-test", "output_mapping":mapping, "library_version":"test"});
        assert!(server.cache.put("mapping-test".into(), complete.clone()));
        let mut value = complete;
        value["full_result_resource"] = json!("qspquest://runs/mapping-test");
        let bounded = server.response(value).structured_content.unwrap();
        assert!(bounded["output_mapping"].is_null());
        assert!(bounded["output_mapping_notice"]
            .as_str()
            .unwrap()
            .contains("1000 mapping entries"));
        assert_eq!(
            bounded["full_result_resource"],
            "qspquest://runs/mapping-test"
        );
        assert_eq!(
            server.cache.get("mapping-test").unwrap()["output_mapping"]
                .as_array()
                .unwrap()
                .len(),
            1000
        );
    }
    fn plan_args() -> Value {
        json!({"model":{"source":include_str!("../assets/templates/iv.qsp")},
            "request":{"schema":"qspquest.run/v0.1","solver":"diffsol_bdf",
            "budgets":{"solver_callbacks":1000000,"events":100,"output_values":10000},
            "parameters":{},"initial_states":{},"regimen":{"end":{"value":2,"unit":"h"},
            "samples":[],"administrations":[],"rtol":1e-8,"atol":1e-10}},"outputs":["cp"]})
    }
    #[tokio::test]
    async fn patient_preparation_preserves_initial_exposure_without_future_rhs() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let mut args = json!({"model":{"source":include_str!("../assets/templates/iv.qsp").replace("(cl / v)","(cl / (v - v))").replace("central = 0 [mg]","central = 6 [mg]")},"covariates":{"v":{"value":6000,"unit":"mL"}},"outputs":["cp"]});
        let plan = server
            .plan_invocation("get_patient_parameters", args.clone())
            .await;
        assert!(plan["error"].is_null(), "{plan}");
        assert_eq!(
            plan["plan"]["native_preparation"]["kind"],
            "get_patient_parameters"
        );
        args["expected_plan_hash"] = plan["plan_hash"].clone();
        let result = server.invoke("get_patient_parameters", args.clone()).await;
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(result["patient_quantities"]["cp"], 1.0);
        assert_eq!(result["patient_units"]["cp"], "mg/L");
        assert_eq!(
            result["identity"],
            plan["plan"]["native_preparation"]["plan"]["prospective_identity"]
        );
        assert_eq!(result["plan_hash"], plan["plan_hash"]);
        assert_eq!(
            result["qualification"]["current_engine_and_request"],
            "not_assessed"
        );
        assert!(result["effective_initial_values"]["plan_hash"].is_null());
        assert!(result["effective_initial_values"]["library_version"].is_null());
        serde_json::from_value::<qspquest_core::initial_values::InitialValuesResult>(
            result["effective_initial_values"].clone(),
        )
        .expect("nested initial values retain the closed native contract");
        assert!(result["scientific_assessment"].is_object());
        assert_eq!(
            result["effective_initial_values"]["states"][0]["value"],
            6.0
        );
        args["covariates"]["v"]["value"] = json!(3000);
        assert_eq!(
            server.invoke("get_patient_parameters", args).await["error"]["code"],
            "plan_mismatch"
        );
    }
    #[tokio::test]
    async fn patient_plan_does_not_evaluate_selected_outputs() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let mut args = json!({"model":{"source":include_str!("../assets/templates/iv.qsp").replace("cp = (central / v)","cp = (central / (v - v))")},"covariates":{},"outputs":["cp"]});
        assert!(server
            .plan_invocation("get_patient_parameters", args.clone())
            .await["error"]
            .is_null());
        assert_eq!(
            server.invoke("get_patient_parameters", args.clone()).await["error"]["code"],
            "Domain"
        );
        args["outputs"] = json!(["missing"]);
        assert!(!server.plan_invocation("get_patient_parameters", args).await["error"].is_null());
    }
    #[tokio::test]
    async fn patient_true_covariates_and_complete_inline_budget_are_enforced() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let mut model: Value = serde_json::from_str(include_str!(
            "../../../conformance/models/synthetic-one-compartment.json"
        ))
        .unwrap();
        model["covariates"] = json!([{"name":"weight","unit":"kg","interpolation":"constant","default":{"value":70,"unit":"kg"}}]);
        model["outputs"].as_array_mut().unwrap().push(
            json!({"name":"weight","unit":"kg","expression":{"kind":"symbol","name":"weight"}}),
        );
        let mut args = json!({"model":{"source":model.to_string()},"covariates":{"weight":{"value":80000,"unit":"g"}},"outputs":["weight"]});
        let result = server.invoke("get_patient_parameters", args.clone()).await;
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(result["patient_quantities"]["weight"], 80.0);
        assert_eq!(
            result["effective_request"]["covariates"]["weight"]["unit"],
            "kg"
        );
        assert_eq!(
            result["parameters_used"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["name"] == "weight")
                .unwrap()["source"],
            "given"
        );
        let output = model["outputs"][1].clone();
        for i in 0..900 {
            let mut o = output.clone();
            o["name"] = json!(format!("initial_weight_{}_{i}", "x".repeat(90)));
            model["outputs"].as_array_mut().unwrap().push(o);
        }
        args["model"]["source"] = json!(model.to_string());
        args.as_object_mut().unwrap().remove("outputs");
        assert_eq!(
            server.invoke("get_patient_parameters", args.clone()).await["error"]["code"],
            "patient_output_budget"
        );
        let directory = std::env::temp_dir().join(format!(
            "qspquest-patient-artifacts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let retaining = server.clone().with_artifact_storage(&directory).unwrap();
        let complete = retaining
            .invoke("get_patient_parameters", args.clone())
            .await;
        assert!(complete["error"].is_null(), "{complete}");
        let encoded = retaining.response(complete.clone());
        assert!(serde_json::to_vec(&encoded).unwrap().len() <= policy::RESPONSE_LIMIT);
        let bounded = encoded.structured_content.unwrap();
        assert_eq!(bounded["contract"]["completeness"], "resource_linked");
        assert_eq!(bounded["result_artifact"], complete["result_artifact"]);
        let store = retaining.artifact_store.as_ref().unwrap();
        let descriptor: crate::artifacts::Descriptor =
            serde_json::from_value(complete["result_artifact"].clone()).unwrap();
        let id = descriptor
            .uri
            .strip_prefix(crate::artifacts::URI_PREFIX)
            .unwrap();
        let text: String = (0..descriptor.chunk_count)
            .map(|index| store.chunk(id, index).unwrap())
            .collect();
        let full: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(full["patient_quantities"].as_object().unwrap().len(), 902);
        assert_eq!(
            full["effective_initial_values"],
            complete["effective_initial_values"]
        );
        assert_eq!(full["identity"], complete["identity"]);
        drop(retaining);
        std::fs::remove_dir_all(directory).unwrap();
        args["outputs"] = json!(["weight"]);
        assert_eq!(
            server.invoke("get_patient_parameters", args).await["patient_quantities"]["weight"],
            80.0
        );
    }
    fn comparison_args() -> Value {
        let mut args = plan_args();
        args["regimens"] = json!([
            {"name":"low","doses":[{"target":"central","at":{"value":0,"unit":"h"},"amount":{"value":3,"unit":"mg"}}]},
            {"name":"high","doses":[{"target":"central","at":{"value":0,"unit":"h"},"amount":{"value":6,"unit":"mg"}}]}
        ]);
        args
    }
    #[tokio::test]
    async fn comparison_plans_bind_every_member_and_retain_each_execution_identity() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let mut args = comparison_args();
        let plan = server
            .plan_invocation("compare_regimens", args.clone())
            .await;
        assert!(plan["error"].is_null(), "{plan}");
        assert_eq!(
            plan,
            server
                .plan_invocation("compare_regimens", args.clone())
                .await
        );
        let members = &plan["plan"]["native_preparation"]["plan"]["regimens"];
        assert_eq!(members.as_array().unwrap().len(), 2);
        assert_eq!(members[1]["name"], "high");
        assert_eq!(
            members[0]["preparation"]["native_preparation"]["kind"],
            "simulate"
        );
        args["expected_plan_hash"] = plan["plan_hash"].clone();
        let result = server.execute_comparison(args.clone()).await;
        assert!(result["error"].is_null(), "{result}");
        assert!(result["_text"]
            .as_str()
            .unwrap()
            .contains("scientific_assessment"));
        assert_eq!(result["plan_hash"], plan["plan_hash"]);
        assert_eq!(
            result["effective_request_hash"],
            plan["effective_request_hash"]
        );
        for index in 0..2 {
            let member = &result["regimens"][index]["result"];
            let stored = server
                .cache
                .get(member["run_id"].as_str().unwrap())
                .unwrap();
            assert_eq!(
                stored["identity"],
                members[index]["preparation"]["native_preparation"]["plan"]["prospective_identity"]
            );
            assert_eq!(
                member["plan_hash"],
                members[index]["preparation"]["plan_hash"]
            );
            assert_eq!(
                member["effective_request_hash"],
                members[index]["preparation"]["context"]["effective_request_hash"]
            );
            let prepared_receipts = &members[index]["preparation"]["dose_event_receipts"];
            assert_eq!(prepared_receipts[0]["stage"], "prepared_schedule");
            assert_eq!(
                prepared_receipts[0]["amount_before_model_scaling"]["value"],
                json!(3.0 * (index + 1) as f64)
            );
            let mut executed_receipts = prepared_receipts.clone();
            executed_receipts[0]["stage"] = json!("executed_schedule");
            assert_eq!(stored["dose_event_receipts"], executed_receipts);
        }
        let low = result["regimens"][0]["result"]["summary"][0]["sampled_max"].as_f64();
        let high = result["regimens"][1]["result"]["summary"][0]["sampled_max"].as_f64();
        assert!(low.is_some() && high.is_some(), "{result}");
        assert!((high.unwrap() - 2.0 * low.unwrap()).abs() < 1e-8);
        args["regimens"][1]["doses"][0]["amount"]["value"] = json!(7);
        assert_eq!(
            server.execute_comparison(args.clone()).await["error"]["code"],
            "plan_mismatch"
        );
        args["regimens"][1]["doses"][0]["amount"]["value"] = json!(6);
        server.library.write().unwrap().version.push_str("-changed");
        assert_eq!(
            server.execute_comparison(args).await["error"]["code"],
            "plan_mismatch"
        );
    }
    #[tokio::test]
    async fn comparison_counterfactuals_preserve_shared_inputs_and_bind_separate_results() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let source = "model synthetic_binding_conditions\ntime_unit = h\nparameters\n kon = 0.5 [1/h] fixed\n start_amount = 6 [mg] fixed\nstates\n free = start_amount [mg]\n bound = 0 [mg]\ndynamics\n d/dt(free) = -kon * free\n d/dt(bound) = kon * free\noutputs\n free_amount = free [mg]\n bound_amount = bound [mg]\nend\n";
        let mut args = json!({"model":{"source":source},"end":{"value":2,"unit":"h"},
            "times":[{"value":0,"unit":"h"},{"value":1,"unit":"h"},{"value":2,"unit":"h"}],
            "parameters":{"kon":{"value":0.5,"unit":"1/h"},"start_amount":{"value":6,"unit":"mg"}},
            "initial_states":{"free":{"value":2,"unit":"mg"},"bound":{"value":0,"unit":"mg"}},
            "outputs":["free_amount","bound_amount"],"regimens":[
                {"name":"A_bind","doses":[]},
                {"name":"B_zero","doses":[],"parameters":{"kon":{"value":0,"unit":"1/h"}},"initial_states":{"free":{"value":2000,"unit":"ug"}}},
                {"name":"C_double_start","doses":[],"initial_states":{"free":{"value":4,"unit":"mg"}}}]});
        let original = args.clone();
        let planned = server
            .plan_invocation("compare_regimens", args.clone())
            .await;
        assert!(planned["error"].is_null(), "{planned}");
        assert_eq!(args, original);
        let members = &planned["plan"]["native_preparation"]["plan"]["regimens"];
        assert_ne!(
            members[0]["input_receipt"]["requested_inputs_sha256"],
            members[1]["input_receipt"]["requested_inputs_sha256"]
        );
        assert_ne!(
            members[0]["input_receipt"]["native_parameter_contract_sha256"],
            members[1]["input_receipt"]["native_parameter_contract_sha256"]
        );
        assert_eq!(
            members[0]["input_receipt"]["native_parameter_contract_sha256"],
            members[2]["input_receipt"]["native_parameter_contract_sha256"]
        );
        args["expected_plan_hash"] = planned["plan_hash"].clone();
        let result = server.execute_comparison(args.clone()).await;
        assert!(result["error"].is_null(), "{result}");
        let envelope = server.response(result.clone()).structured_content.unwrap();
        assert_eq!(envelope["contract"]["status"], "success");
        for index in 0..3 {
            let row = &result["regimens"][index];
            let receipt = &row["input_receipt"];
            assert_eq!(row["name"], members[index]["name"]);
            assert_eq!(receipt, &members[index]["input_receipt"]);
            assert_eq!(
                receipt["native_bound_value_hash"],
                row["result"]["identity"]["bound_value_hash"]
            );
            assert!(row["result"]["override_receipt"]["entries"].is_array());
            let full = server
                .cache
                .get(row["result"]["run_id"].as_str().unwrap())
                .unwrap();
            assert_eq!(
                receipt["effective_request_hash"],
                full["effective_request_hash"]
            );
            assert_eq!(
                full["effective_request"]["parameters"]["start_amount"],
                json!({"value":6.0,"unit":"mg"})
            );
            assert_eq!(
                full["effective_request"]["initial_states"]["bound"],
                json!({"value":0.0,"unit":"mg"})
            );
            let start = if index == 2 { 4.0 } else { 2.0 };
            let k = if index == 1 { 0.0 } else { 0.5 };
            for (time, free) in full["times"]
                .as_array()
                .unwrap()
                .iter()
                .zip(full["outputs"][0]["values"].as_array().unwrap())
            {
                assert!(
                    (free.as_f64().unwrap() - start * (-k * time.as_f64().unwrap()).exp()).abs()
                        < 1e-6
                );
            }
            for (free, bound) in full["outputs"][0]["values"]
                .as_array()
                .unwrap()
                .iter()
                .zip(full["outputs"][1]["values"].as_array().unwrap())
            {
                assert!((free.as_f64().unwrap() + bound.as_f64().unwrap() - start).abs() < 1e-7);
            }
        }
        args["regimens"][1]["parameters"]["kon"]["value"] = json!(0.1);
        assert_eq!(
            server.execute_comparison(args).await["error"]["code"],
            "plan_mismatch"
        );
    }

    #[tokio::test]
    async fn comparison_refuses_invalid_later_member_inputs_before_any_integration() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let mut args = comparison_args();
        args["model"]["source"] =
            json!(include_str!("../assets/templates/iv.qsp").replace("(cl / v)", "(cl / (v - v))"));
        for (field, name, quantity, code) in [
            (
                "parameters",
                "retired_k",
                json!({"value":1,"unit":"1"}),
                "input_alias_unknown",
            ),
            ("parameters", "cl", json!({"value":1,"unit":"mg"}), "Unit"),
            (
                "initial_states",
                "retired_state",
                json!({"value":1,"unit":"mg"}),
                "input_alias_unknown",
            ),
            (
                "initial_states",
                "central",
                json!({"value":1,"unit":"L"}),
                "Unit",
            ),
        ] {
            let mut invalid = args.clone();
            invalid["regimens"][1][field] = json!({name:quantity});
            let error = server.execute_comparison(invalid).await;
            assert!(error.get("regimens").is_none(), "{error}");
            assert_eq!(error["error"]["code"], code, "{error}");
            assert!(
                error["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("member 1 (high)"),
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn comparison_admits_all_members_and_destinations_before_poisoned_rhs() {
        let root = std::env::temp_dir().join(format!(
            "qspquest-comparison-admission-{}",
            std::process::id()
        ));
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1)
            .with_csv_output(&root)
            .unwrap();
        let mut args = comparison_args();
        args["model"]["source"] =
            json!(include_str!("../assets/templates/iv.qsp").replace("(cl / v)", "(cl / (v - v))"));
        args["save_csv"] = json!("new/comparison.csv");
        let plan = server
            .plan_invocation("compare_regimens", args.clone())
            .await;
        assert!(plan["error"].is_null(), "{plan}");
        assert!(!root.join("new").exists());
        args["regimens"][1]["doses"][0]["amount"]["unit"] = json!("h");
        let refused = server.execute_comparison(args.clone()).await;
        assert!(!refused["error"].is_null(), "{refused}");
        assert_ne!(refused["error"]["code"], "Domain", "{refused}");
        assert!(!root.join("new").exists());
        args["regimens"][1]["doses"][0]["amount"]["unit"] = json!("mg");
        args["save_csv"] = json!("comparison.csv");
        std::fs::write(root.join("comparison_high.csv.manifest.json"), "existing").unwrap();
        assert_eq!(
            server.execute_comparison(args.clone()).await["error"]["code"],
            "save_csv"
        );
        assert!(!root.join("comparison_low.csv").exists());
        assert_eq!(
            std::fs::read(root.join("comparison_high.csv.manifest.json")).unwrap(),
            b"existing"
        );
        args.as_object_mut().unwrap().remove("save_csv");
        assert_eq!(
            server.execute_comparison(args).await["error"]["code"],
            "Domain"
        );
        drop(server);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn comparison_never_advertises_already_evicted_member_trajectories() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let mut args = comparison_args();
        args["regimens"] = json!((0..10)
            .map(|i| json!({"name":format!("regimen_{i}"),"doses":[{
            "target":"central","at":{"value":0,"unit":"h"},"amount":{"value":i + 1,"unit":"mg"}}]}))
            .collect::<Vec<_>>());
        let result = server.execute_comparison(args).await;
        assert!(result["error"].is_null(), "{result}");
        let records = result["regimens"].as_array().unwrap();
        assert_eq!(records.len(), 10);
        assert_eq!(
            records
                .iter()
                .filter(|r| r["result"]["full_result_resource"].is_string())
                .count(),
            8
        );
        for record in records {
            let member = &record["result"];
            if member["full_result_resource"].is_string() {
                assert!(server
                    .cache
                    .get(member["run_id"].as_str().unwrap())
                    .is_some());
            } else {
                assert!(member["trajectory_notice"]
                    .as_str()
                    .unwrap()
                    .contains("evicted"));
            }
        }
    }
    #[tokio::test]
    async fn comparison_refuses_ambiguous_names_and_csv_collisions_without_exports() {
        let root =
            std::env::temp_dir().join(format!("qspquest-comparison-names-{}", std::process::id()));
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1)
            .with_csv_output(&root)
            .unwrap();
        let mut args = comparison_args();
        args["regimens"][1]["name"] = json!("low");
        assert_eq!(
            server.execute_comparison(args.clone()).await["error"]["code"],
            "regimens"
        );
        args["regimens"][0]["name"] = json!("a-b");
        args["regimens"][1]["name"] = json!("a b");
        args["save_csv"] = json!("comparison.csv");
        assert_eq!(
            server
                .plan_invocation("compare_regimens", args.clone())
                .await["error"]["code"],
            "save_csv"
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        args["regimens"][1]["name"] = json!("different");
        args["outputs"] = json!(["cp"]);
        args["output"] = json!("missing");
        assert_eq!(
            server.execute_comparison(args).await["error"]["code"],
            "outputs"
        );
        drop(server);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn plans_bind_context_and_guard_execution_without_promoting_qualification() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let mut args = plan_args();
        let plan = server.plan_invocation("simulate", args.clone()).await;
        assert!(plan["error"].is_null(), "{plan}");
        assert_eq!(
            plan["plan"]["qualification"]["scientific_class"],
            "synthetic_illustrative"
        );
        assert_eq!(
            plan["plan"]["qualification"]["current_engine_and_request"],
            "not_assessed"
        );
        assert_eq!(
            plan["plan"]["native_preparation"]["plan"]["request"]["regimen"]["samples"],
            json!([])
        );
        assert_eq!(plan, server.plan_invocation("simulate", args.clone()).await);
        assert!(server
            .cache
            .get(&format!("plan:{}", plan["plan_hash"].as_str().unwrap()))
            .is_none());
        args["expected_plan_hash"] = plan["plan_hash"].clone();
        let result = server.invoke("simulate", args.clone()).await;
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(result["plan_hash"], plan["plan_hash"]);
        assert_eq!(
            result["effective_request_hash"],
            plan["effective_request_hash"]
        );
        assert_eq!(
            result["identity"],
            plan["plan"]["native_preparation"]["plan"]["prospective_identity"]
        );
        let invalid = server.plan_invocation("simulate", args.clone()).await;
        assert_eq!(invalid["error"]["code"], "expected_plan_hash");
        args["request"]["regimen"]["end"]["value"] = json!(3);
        assert_eq!(
            server.invoke("simulate", args.clone()).await["error"]["code"],
            "plan_mismatch"
        );
        args["request"]["regimen"]["end"]["value"] = json!(2);
        server
            .library
            .write()
            .unwrap()
            .version
            .push_str("-refreshed");
        assert_eq!(
            server.invoke("simulate", args.clone()).await["error"]["code"],
            "plan_mismatch"
        );
        args.as_object_mut().unwrap().remove("expected_plan_hash");
        let refreshed = server.plan_invocation("simulate", args.clone()).await;
        args["expected_plan_hash"] = refreshed["plan_hash"].clone();
        let updated = server.invoke("simulate", args).await;
        assert!(updated["error"].is_null(), "{updated}");
        let retained = server
            .cache
            .get(updated["run_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(retained["plan_hash"], refreshed["plan_hash"]);
        assert_eq!(retained["library_version"], refreshed["library_version"]);
        let mut value = plan;
        let captured = value["library_version"].clone();
        value["_text"] = json!("Static plan");
        let response = server.response(value).structured_content.unwrap();
        assert_eq!(response["library_version"], captured);
        assert_eq!(response["contract"]["library_version"], captured);
        assert_eq!(response["contract"]["identity"]["scope"], "metadata");
    }
    #[tokio::test]
    async fn plans_defer_readout_domains_but_reject_static_units_and_fit_fields() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let mut args = plan_args();
        args["model"]["source"] = json!(include_str!("../assets/templates/iv.qsp")
            .replace("(central / v)", "(central / (v - v))"));
        let plan = server.plan_invocation("simulate", args.clone()).await;
        assert!(plan["error"].is_null(), "{plan}");
        args["expected_plan_hash"] = plan["plan_hash"].clone();
        assert_eq!(
            server.invoke("simulate", args.clone()).await["error"]["code"],
            "Domain"
        );
        args["request"]["parameters"] = json!({"v":{"value":4,"unit":"mg"}});
        let refusal = server
            .plan_invocation("simulate", {
                let mut a = args.clone();
                a.as_object_mut().unwrap().remove("expected_plan_hash");
                a
            })
            .await;
        assert_eq!(refusal["error"]["code"], "Unit");
        assert_eq!(refusal["error"]["expression"], "parameters.v", "{refusal}");
        for (name, text) in [
            ("fit_scalar", include_str!("../assets/fits/scalar.json")),
            (
                "fit_individual",
                include_str!("../assets/fits/individual-additive.json"),
            ),
            (
                "fit_population",
                include_str!("../assets/fits/population.json"),
            ),
        ] {
            let example: Value = serde_json::from_str(text).unwrap();
            let mut arguments = example["arguments"].clone();
            arguments["model"]["source"] = json!(arguments["model"]["source"]
                .as_str()
                .unwrap()
                .replace("(central / v)", "(central / (v - v))"));
            let plan = server.plan_invocation(name, arguments.clone()).await;
            assert!(plan["error"].is_null(), "{name}: {plan}");
            assert_eq!(plan["plan"]["native_preparation"]["kind"], name);
            arguments["expected_plan_hash"] = plan["plan_hash"].clone();
            let execution = server.invoke(name, arguments).await;
            assert_eq!(execution["error"]["code"], "Domain", "{name}: {execution}");
        }
    }
    #[cfg(feature = "analysis")]
    #[tokio::test]
    async fn malformed_analysis_fields_refuse_before_compute_permits_for_plans_and_execution() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let _occupied = server.semaphore.clone().try_acquire_owned().unwrap();
        let source = include_str!("../../../conformance/models/synthetic-one-compartment.json");
        let mut morris: Value = serde_json::from_str(include_str!(
            "../../../conformance/requests/synthetic-morris.json"
        ))
        .unwrap();
        morris["problem"]["request"]["ranges"][1]["upper"]["unit"] = json!(false);
        let mut periodic: Value = serde_json::from_str(include_str!(
            "../../../conformance/requests/synthetic-linear-pk-steady.json"
        ))
        .unwrap();
        periodic["relative_tolerance"] = json!("tight");
        for (operation, request, code, pointer) in [
            (
                "morris_screening",
                morris,
                "request",
                "/request/problem/request/ranges/1/upper/unit",
            ),
            (
                "steady_state",
                periodic,
                "request_input_validation",
                "arguments.request.relative_tolerance",
            ),
        ] {
            let arguments = json!({"model":{"source":source},"request":request});
            for outcome in [
                server.plan_invocation(operation, arguments.clone()).await,
                server.invoke(operation, arguments.clone()).await,
            ] {
                assert_eq!(outcome["error"]["code"], code, "{outcome}");
                assert_eq!(outcome["error"]["expression"], pointer, "{outcome}");
                assert!(outcome["error"]["suggestion"].is_string());
                assert!(outcome.get("result").is_none());
            }
            assert_eq!(server.semaphore.available_permits(), 0);
        }
    }

    #[cfg(feature = "analysis")]
    #[tokio::test]
    async fn morris_planning_and_guards_use_the_complete_analysis_request() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let source = include_str!("../../../conformance/models/synthetic-one-compartment.json");
        let request: Value = serde_json::from_str(include_str!(
            "../../../conformance/requests/synthetic-morris.json"
        ))
        .unwrap();
        let mut args = json!({"model":{"source":source},"request":request});
        let plan = server
            .plan_invocation("morris_screening", args.clone())
            .await;
        assert!(plan["error"].is_null(), "{plan}");
        assert_eq!(
            plan["plan"]["native_preparation"]["kind"],
            "morris_screening"
        );
        args["expected_plan_hash"] = plan["plan_hash"].clone();
        let result = server.invoke("morris_screening", args.clone()).await;
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(result["plan_hash"], plan["plan_hash"]);
        assert_eq!(
            result["result"]["analysis_request_hash"],
            plan["plan"]["native_preparation"]["plan"]["analysis_request_hash"]
        );
        args["request"]["problem"]["request"]["seed"] = json!(43);
        let refused = server.invoke("morris_screening", args).await;
        assert_eq!(refused["error"]["code"], "plan_mismatch");
        assert!(refused.get("result").is_none());
    }

    #[cfg(feature = "analysis")]
    #[tokio::test]
    async fn periodic_planning_guards_cover_tolerances_and_request_identity() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let source = include_str!("../../../conformance/models/synthetic-linear-pk.json");
        let request: Value = serde_json::from_str(include_str!(
            "../../../conformance/requests/synthetic-linear-pk-steady.json"
        ))
        .unwrap();
        let mut args = json!({"model":{"source":source},"request":request});
        let plan = server.plan_invocation("steady_state", args.clone()).await;
        assert!(plan["error"].is_null(), "{plan}");
        assert_eq!(plan["plan"]["native_preparation"]["kind"], "steady_state");
        args["expected_plan_hash"] = plan["plan_hash"].clone();
        let result = server.invoke("steady_state", args.clone()).await;
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(result["plan_hash"], plan["plan_hash"]);
        assert_eq!(
            result["identity"],
            plan["plan"]["native_preparation"]["plan"]["prospective_identity"]
        );
        args["request"]["relative_tolerance"] = json!(1e-9);
        assert_eq!(
            server.invoke("steady_state", args.clone()).await["error"]["code"],
            "plan_mismatch"
        );
        args.as_object_mut().unwrap().remove("expected_plan_hash");
        args["request"]["run"]["budgets"]["events"] = json!(1);
        assert_eq!(
            server.plan_invocation("steady_state", args).await["error"]["code"],
            "WorkBudget"
        );
    }

    #[tokio::test]
    async fn input_name_refusals_are_located_versioned_and_precede_execution() {
        let mut library = Library::embedded().unwrap();
        library
            .records
            .retain(|r| r.metadata["id"] == "qspquest_diffdose_2026_idr_benchmark");
        let record = &mut library.records[0];
        let id = record.metadata["id"].clone();
        let version = record.metadata["version"].clone();
        let parameter = record.model["parameters"][0].clone();
        record.index["native_execution"]["parameters"]["friendly_input"] = json!({"native_name":parameter["name"],"source_unit":parameter["default"]["unit"],"request_unit":parameter["default"]["unit"]});
        let digest = crate::inspection::index(record)
            .unwrap()
            .context
            .inspection_hash;
        let server = Server::new(library, false, policy::Tier::Free, 1);
        let _permit = server.semaphore.clone().try_acquire_owned().unwrap();
        for (operation, patch, group, field, code) in [
            (
                "simulate",
                json!({"parameters":{"retired_input":{"value":1,"unit":"1"}}}),
                "parameter",
                "parameters.retired_input",
                "input_alias_unknown",
            ),
            (
                "simulate",
                json!({"initial_states":{"retired_input":{"value":1,"unit":"1"}}}),
                "initial_state",
                "initial_states.retired_input",
                "input_alias_unknown",
            ),
            (
                "simulate",
                json!({"doses":[{"target":"retired_input","at":{"value":0,"unit":"d"},"amount":{"value":1,"unit":"1"}}]}),
                "dose_target",
                "doses[0].target",
                "input_alias_unknown",
            ),
            (
                "scan_parameters",
                json!({"parameter":"retired_input"}),
                "scan_parameter",
                "parameter",
                "input_alias_unknown",
            ),
            (
                "get_patient_parameters",
                json!({"covariates":{"retired_input":{"value":1,"unit":"1"}}}),
                "patient_input",
                "covariates.retired_input",
                "input_alias_unknown",
            ),
            (
                "simulate",
                json!({"request":{"schema":"qspquest.run/v0.1","parameters":{"retired_input":{"value":1,"unit":"1"}}}}),
                "parameter",
                "request.parameters.retired_input",
                "input_alias_unknown",
            ),
            (
                "simulate",
                json!({"request":{"schema":"qspquest.run/v0.1","parameters":{"friendly_input":parameter["default"]}}}),
                "parameter",
                "request.parameters.friendly_input",
                "input_name_requires_native",
            ),
            (
                "simulate",
                json!({"request":{"schema":"qspquest.run/v0.1","regimen":{"resets":[{"target":"retired_input"}]}}}),
                "state_reset",
                "request.regimen.resets[0].target",
                "input_alias_unknown",
            ),
            (
                "simulate",
                json!({"request":{"schema":"qspquest.run/v0.1","regimen":{"administrations":[{"target":"retired_input"}]}}}),
                "dose_target",
                "request.regimen.administrations[0].target",
                "input_alias_unknown",
            ),
            (
                "simulate",
                json!({"request":{"schema":"qspquest.run/v0.1","regimen":{"absolute_tolerances":{"retired_input":{"value":1,"unit":"1"}}}}}),
                "state_tolerance",
                "request.regimen.absolute_tolerances.retired_input",
                "input_alias_unknown",
            ),
        ] {
            let mut args = json!({"model":{"library":{"model_id":id,"version":version}},"end":{"value":1,"unit":"d"}});
            if operation != "simulate" {
                args.as_object_mut().unwrap().remove("end");
            }
            args.as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            let original = args.clone();
            for value in [
                server.plan_invocation(operation, args.clone()).await,
                server.invoke(operation, args.clone()).await,
            ] {
                assert_eq!(value["error"]["code"], code, "{value}");
                assert!(value.get("result").is_none() && value.get("plan").is_none());
                let reply = server.response(value).structured_content.unwrap();
                let binding = &reply["contract"]["failure"]["input_binding"];
                assert_eq!(binding["group"], group);
                assert_eq!(binding["field"], field);
                assert_eq!(binding["model_id"], id);
                assert_eq!(binding["model_version"], version);
                assert_eq!(binding["current_inspection_hash"], digest);
                assert_eq!(binding["migration"], "inspect_current_bindings");
                assert_eq!(binding["numerical_execution_started"], false);
                assert!(binding["inspection_index_resource"]
                    .as_str()
                    .unwrap()
                    .ends_with("/index"));
                if code == "input_name_requires_native" {
                    assert_eq!(binding["canonical_name"], parameter["name"]);
                    assert_eq!(binding["native_unit"], parameter["default"]["unit"]);
                }
            }
            assert_eq!(args, original);
        }
    }

    #[tokio::test]
    async fn input_name_refusals_cover_large_hosted_routing_and_bounded_source_names() {
        let mut library = Library::embedded().unwrap();
        library
            .records
            .retain(|r| r.metadata["id"] == "qspquest_diffdose_2026_idr_benchmark");
        let record = &mut library.records[0];
        let template = record.model["states"][0].clone();
        for index in 0..64 {
            let mut state = template.clone();
            state["name"] = json!(format!("extra_{index}"));
            record.model["states"].as_array_mut().unwrap().push(state);
        }
        let id = record.metadata["id"].clone();
        let server = Server::new(library, true, policy::Tier::Free, 1);
        let args = json!({"model":{"library":{"model_id":id}},"end":{"value":1,"unit":"d"}});
        assert_eq!(
            server.invoke("simulate", args.clone()).await["error"]["code"],
            "async_required"
        );
        let mut invalid = args;
        invalid["parameters"] = json!({"retired_input":{"value":1,"unit":"1"}});
        assert_eq!(
            server.invoke("simulate", invalid).await["error"]["code"],
            "input_alias_unknown"
        );
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let name = "🧪".repeat(4000);
        let args = json!({"model":{"source":"model source_input_guard\ntime_unit = h\nparameters\n k = 1 [1/h] fixed\nstates\n x = 1 [mg]\ndynamics\n d/dt(x) = -k * x\noutputs\n y = (1 / (x-x)) [1]\nend\n"},"parameters":{name.clone():{"value":1,"unit":"1"}},"end":{"value":1,"unit":"h"}});
        let value = server.plan_invocation("simulate", args).await;
        assert_eq!(value["error"]["code"], "input_alias_unknown", "{value}");
        let reply = server.response(value).structured_content.unwrap();
        let failure = &reply["contract"]["failure"]["input_binding"];
        assert_eq!(failure["migration"], "inspect_submitted_model");
        assert!(
            failure["current_inspection_hash"].is_null()
                && failure["inspection_index_resource"].is_null()
        );
        assert_eq!(failure["name_truncated"], true);
        assert_eq!(
            failure["complete_name_sha256"],
            format!("sha256:{}", hash(name.as_bytes()))
        );
        assert_eq!(failure["numerical_execution_started"], false);
        assert!(serde_json::to_vec(failure).unwrap().len() < 16_000);
    }

    #[tokio::test]
    async fn source_coordinate_unit_errors_name_units_without_molar_mass_advice() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let source = "model synthetic_source_coordinates\ntime_unit = h\nparameters\n rate = 1 [1] fixed\n clock = 1 [1/h] fixed\nstates\n x = 1 [1]\ndynamics\n d/dt(x) = (-clock * rate * x)\noutputs\n y = (1 / (x - x)) [1]\nend";
        let mut args = json!({"model":{"source":source},"parameters":{"rate":{"value":1,"unit":"1/h"}},"end":{"value":2,"unit":"h"}});
        let value = server.plan_invocation("simulate", args.clone()).await;
        assert_eq!(value["error"]["code"], "Unit");
        assert_eq!(value["error"]["expression"], "parameters.rate");
        let message = value["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("requested unit \"1/h\"; declared unit \"1\""),
            "{message}"
        );
        assert!(
            message.contains("normalization is not inferred"),
            "{message}"
        );
        assert!(!message.contains("molar mass"), "{message}");
        let reply = server.response(value).structured_content.unwrap();
        assert_eq!(
            reply["contract"]["failure"]["expression"],
            "parameters.rate"
        );
        assert!(reply["contract"]["failure"]["message"]
            .as_str()
            .unwrap()
            .contains("declared unit \"1\""));
        args["parameters"]["rate"]["unit"] = json!("1");
        let plan = server.plan_invocation("simulate", args.clone()).await;
        assert!(plan["error"].is_null(), "{plan}");
        args["expected_plan_hash"] = plan["plan_hash"].clone();
        assert_eq!(
            server.invoke("simulate", args).await["error"]["code"],
            "Domain"
        );
    }

    #[tokio::test]
    async fn inspected_bindings_guard_library_and_scenario_requests_before_compute() {
        let mut library = Library::embedded().unwrap();
        library
            .records
            .retain(|r| r.metadata["id"] == "qspquest_diffdose_2026_idr_benchmark");
        let record = &library.records[0];
        let id = record.metadata["id"].clone();
        let version = record.metadata["version"].clone();
        let digest = crate::inspection::index(record)
            .unwrap()
            .context
            .inspection_hash;
        let scenario = record.scenarios.keys().next().unwrap().clone();
        let server = Server::new(library, false, policy::Tier::Free, 1);
        let _permit = server.semaphore.clone().try_acquire_owned().unwrap();
        for reference in [
            json!({"library":{"model_id":id,"version":version,"expected_inspection_hash":digest}}),
            json!({"scenario":{"model_id":id,"version":version,"scenario_id":scenario,"expected_inspection_hash":digest}}),
        ] {
            let args =
                json!({"model":reference,"end":{"value":1.0,"unit":"d"},"outputs":["biomarker"]});
            let resolved = server.resolve(&args["model"]).unwrap();
            assert!(!resolved.0.is_empty());
            server.library.write().unwrap().records[0].index["native_execution"]
                ["alias_revision"] = json!(1);
            let refused = server.plan_invocation("simulate", args).await;
            assert_eq!(refused["error"]["code"], "inspection_changed");
            assert_eq!(
                refused["error"]["inspection_change"]["migration"],
                "inspect_current_bindings"
            );
            let reply = server.response(refused).structured_content.unwrap();
            assert_eq!(
                reply["contract"]["failure"]["inspection_change"]["expected_inspection_hash"],
                digest
            );
            server.library.write().unwrap().records[0].index["native_execution"]
                .as_object_mut()
                .unwrap()
                .remove("alias_revision");
        }
        for invalid in [json!("invalid"), json!(42), json!(true), json!({})] {
            let malformed = server
                .resolve(&json!({"library":{"model_id":id,"expected_inspection_hash":invalid}}))
                .unwrap_err();
            assert_eq!(malformed["error"]["code"], "inspection_hash");
        }
    }
    #[tokio::test]
    async fn escaped_plan_text_cannot_silently_lose_complete_preparation() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let payload = server.plan_invocation("simulate", plan_args()).await;
        let mut plan: crate::planning::RequestPlan =
            serde_json::from_value(payload["plan"].clone()).unwrap();
        let base = serde_json::to_string(&plan).unwrap().len();
        plan.assumptions.push("\\".repeat((89_000 - base) / 2));
        plan.seal().unwrap();
        assert!(serde_json::to_string(&plan).unwrap().len() < 90_000);
        let complete = serde_json::to_value(&plan).unwrap();
        let raw = json!({"plan":complete,"plan_hash":plan.plan_hash,
            "effective_request_hash":payload["effective_request_hash"],
            "plan_bytes":serde_json::to_string(&plan).unwrap().len(),"full_plan_resource":null});
        assert!(
            server.response(raw).structured_content.unwrap()["plan"].is_null(),
            "Escaped text must exercise the actual MCP envelope limit"
        );
        let response = server
            .complete_plan_payload(
                plan,
                payload["effective_request_hash"].as_str().unwrap(),
                payload["library_version"].as_str().unwrap(),
            )
            .unwrap();
        let uri = response["full_plan_resource"].as_str().unwrap();
        let id = uri.strip_prefix("qspquest://plans/").unwrap();
        assert_eq!(server.cache.get(&format!("plan:{id}")).unwrap(), complete);
        let reply = server.response(response);
        assert_eq!(
            reply.structured_content.unwrap()["contract"]["completeness"],
            "resource_linked"
        );
    }
    #[tokio::test]
    async fn complete_large_local_plans_use_bounded_metadata_and_utf8_chunks() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let mut args = plan_args();
        args["request"]["regimen"]["samples"] = json!((0..3000)
            .map(|i| json!({"value":i as f64 / 1500.,"unit":"h"}))
            .collect::<Vec<_>>());
        let plan = server.plan_invocation("simulate", args.clone()).await;
        assert!(plan["error"].is_null(), "{plan}");
        assert!(plan["plan"].is_null());
        let uri = plan["full_plan_resource"].as_str().unwrap().to_owned();
        let id = uri.strip_prefix("qspquest://plans/").unwrap();
        let retained = server.cache.get(&format!("plan:{id}")).unwrap();
        assert!(retained.get("identity").is_none());
        let text = retained.to_string();
        let manifest: Value =
            serde_json::from_str(&resource_part(&text, &uri, None).unwrap()).unwrap();
        let mut recovered = String::new();
        for index in 0..manifest["chunks"].as_array().unwrap().len() {
            recovered.push_str(&resource_part(&text, &uri, Some(index)).unwrap());
        }
        assert_eq!(hash(recovered.as_bytes()), manifest["sha256"]);
        assert_eq!(serde_json::from_str::<Value>(&recovered).unwrap(), retained);
        assert_eq!(retained["plan_hash"], plan["plan_hash"]);
        let response = server.response(plan).structured_content.unwrap();
        assert_eq!(response["contract"]["completeness"], "resource_linked");
        assert_eq!(response["contract"]["resources"], json!([uri]));
        // A supplied hostile source never falls back to native hosted planning.
        let hosted = Server::new(Library::embedded().unwrap(), true, policy::Tier::Free, 1);
        assert_eq!(
            hosted.plan_invocation("simulate", plan_args()).await["error"]["code"],
            "sandbox_unavailable"
        );
    }
    #[tokio::test]
    async fn adapter_preparation_preserves_requests_without_acquiring_compute_slots() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let permit = server.semaphore.clone().try_acquire_owned().unwrap();
        let source = include_str!("../assets/templates/iv.qsp");
        let request = json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf","budgets":{"solver_callbacks":1000000,"events":10000,"output_values":25000000},"parameters":{},"initial_states":{},"regimen":{"end":{"value":2,"unit":"h"},"samples":[],"administrations":[],"rtol":1e-8,"atol":1e-10}});
        let args = json!({"model":{"source":source},"request":request,"outputs":["cp"]});
        let before = server.compiled.lock().unwrap().len();
        let prepared = server
            .prepare_invocation("simulate", args.clone())
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(prepared.request, request);
        assert_eq!(
            prepared.effective_hash,
            hash(prepared.compute_request.to_string().as_bytes())
        );
        assert_eq!(server.compiled.lock().unwrap().len(), before);
        let unavailable = server.invoke("simulate", args.clone()).await;
        assert_eq!(unavailable["error"]["code"], "busy");
        drop(permit);
        let result = server.invoke("simulate", args).await;
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(result["effective_request"], prepared.request);
        assert_eq!(result["effective_request_hash"], prepared.effective_hash);
        assert!(server
            .cache
            .get(result["run_id"].as_str().unwrap())
            .is_some());
    }
    #[tokio::test]
    async fn malformed_requests_and_readouts_fail_before_compute_admission() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let _permit = server.semaphore.clone().try_acquire_owned().unwrap();
        let model = json!({"source":include_str!("../assets/templates/iv.qsp")});
        for (arguments, field) in [
            (json!({"request":"simulate for one day"}), "request must be"),
            (
                json!({"request":{"end":{"value":2,"unit":"h"}}}),
                "request must be",
            ),
            (json!({"end":{"value":0,"unit":"h"}}), "end.value"),
            (
                json!({"end":{"value":2,"unit":"h"},"readouts":[{"kind":"time_above","output":"cp","threshold":{"value":5,"unit":"kg"}}]}),
                "readouts[0].threshold",
            ),
        ] {
            let mut args = arguments;
            args["model"] = model.clone();
            let result = server.invoke("simulate", args).await;
            let code = if field == "end.value" {
                "request_input_validation"
            } else {
                "request"
            };
            assert_eq!(result["error"]["code"], code, "{result}");
            assert!(
                result["error"]["message"].as_str().unwrap().contains(field),
                "{result}"
            );
        }
    }
    #[tokio::test]
    async fn required_outputs_refuse_before_compute_and_bind_complete_results() {
        let source = "model synthetic_analytes\ntime_unit = h\nparameters\n k = 1 [1/h]\n v = 3 [L]\nstates\n vector = 0 [1]\n antibody = 6 [mg]\ndynamics\n d/dt(vector) = -k * vector\n d/dt(antibody) = -k * antibody\noutputs\n vector_readout = vector [1]\n antibody_readout = (antibody / v) [mg/L]\nend\n";
        let mut library = Library::embedded().unwrap();
        let record = &mut library.records[0];
        let id = record.metadata["id"].clone();
        record.model =
            compute::execute(source, "validate_model", json!({})).unwrap()["model"].clone();
        record.source = None;
        record.scenarios.clear();
        record.index["native_execution"] = json!({"outputs":{"aav":{"native_name":"vector_readout","unit":"1","analyte":"aav-vector","compartment":"brain"},"mab":{"native_name":"antibody_readout","unit":"mg/L","analyte":"antibody","compartment":"plasma"}},"states":{}});
        let server = Server::new(library, false, policy::Tier::Free, 1);
        let mut args = json!({"model":{"library":{"model_id":id}},"outputs":["aav"],"end":{"value":2,"unit":"h"},"times":[{"value":0,"unit":"h"},{"value":1,"unit":"h"},{"value":2,"unit":"h"}],"required_outputs":[{"output":"mab"}]});
        let permit = server.semaphore.clone().try_acquire_owned().unwrap();
        let failure = server.invoke("simulate", args.clone()).await;
        assert_eq!(
            failure["error"]["output_requirement"]["reason"],
            "not_selected"
        );
        let encoded = server.response(failure).structured_content.unwrap();
        assert_eq!(
            encoded["contract"]["failure"]["output_requirement"]["fulfilled"],
            false
        );
        assert!(encoded["outputs"].is_null());
        assert!(encoded["run_id"].is_null());
        args["required_outputs"] = json!([{"output":"vector_readout","analyte":"antibody"}]);
        assert_eq!(
            server.invoke("simulate", args.clone()).await["error"]["output_requirement"]["reason"],
            "analyte_mismatch"
        );
        drop(permit);
        args["outputs"] = json!(["aav", "mab"]);
        args["required_outputs"] = json!([{"output":"mab","analyte":"antibody","compartment":"plasma"},{"output":"aav","analyte":"aav-vector","compartment":"brain"}]);
        let plan = server.plan_invocation("simulate", args.clone()).await;
        assert!(plan["error"].is_null(), "{plan}");
        assert_eq!(
            plan["plan"]["required_output_receipts"][0]["status"],
            "selected_for_computation"
        );
        args["expected_plan_hash"] = plan["plan_hash"].clone();
        let result = server.invoke("simulate", args.clone()).await;
        assert!(result["error"].is_null(), "{result}");
        let full = server
            .cache
            .get(result["run_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(full["required_output_receipts"][0]["status"], "computed");
        assert_eq!(full["required_output_receipts"][0]["sample_count"], 3);
        for output in full["outputs"].as_array().unwrap() {
            if output["name"] == "vector_readout" {
                assert_eq!(output["values"], json!([0.0, 0.0, 0.0]));
            } else {
                for (t, x) in full["times"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(output["values"].as_array().unwrap())
                {
                    assert!(
                        (x.as_f64().unwrap() - 2.0 * (-t.as_f64().unwrap()).exp()).abs() < 1e-5
                    );
                }
            }
        }
        let mut changed = args.clone();
        changed["required_outputs"][0]
            .as_object_mut()
            .unwrap()
            .remove("analyte");
        assert_eq!(
            server.invoke("simulate", changed.clone()).await["error"]["code"],
            "plan_mismatch"
        );
        changed
            .as_object_mut()
            .unwrap()
            .remove("expected_plan_hash");
        changed.as_object_mut().unwrap().remove("required_outputs");
        let without = server.invoke("simulate", changed).await;
        let without = server
            .cache
            .get(without["run_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(without["outputs"], full["outputs"]);
        assert_ne!(without["run_id"], full["run_id"]);
        let table = usability::csv(&full, &full["model_provenance"]["native_execution"]).unwrap();
        let manifest = crate::table::manifest(
            "required.csv",
            &full,
            &table,
            crate::table::Context {
                metadata: &full["model_provenance"],
                request: &full["effective_request"],
                source,
                scenario_id: None,
                modified: true,
                effective_hash: full["effective_request_hash"].as_str().unwrap(),
                selected_outputs: &json!(["vector_readout", "antibody_readout"]),
            },
        )
        .unwrap();
        let manifest: Value = serde_json::from_slice(&manifest).unwrap();
        assert_eq!(
            manifest["required_output_receipts"],
            full["required_output_receipts"]
        );
        let mut comparison = args.clone();
        comparison
            .as_object_mut()
            .unwrap()
            .remove("expected_plan_hash");
        comparison["regimens"] = json!([{"name":"first","doses":[]},{"name":"second","doses":[]}]);
        let compared = server.execute_comparison(comparison).await;
        assert!(compared["error"].is_null(), "{compared}");
        for member in compared["regimens"].as_array().unwrap() {
            assert_eq!(
                member["result"]["required_output_receipts"][0]["status"],
                "computed"
            );
        }
        let prepared = server.prepare_invocation("simulate", args).await.unwrap();
        let missing = server
            .finish_prepared_invocation(
                "simulate",
                prepared,
                None,
                json!({"times":[0,1,2],"outputs":[]}),
            )
            .unwrap_err();
        assert_eq!(
            missing["error"]["output_requirement"]["reason"],
            "incomplete_column"
        );
    }
    #[tokio::test]
    async fn dose_receipts_follow_native_repeats_lag_availability_and_csv() {
        let source = include_str!("../assets/templates/iv.qsp");
        let mut library = Library::embedded().unwrap();
        let record = &mut library.records[0];
        let id = record.metadata["id"].clone();
        record.model =
            compute::execute(source, "validate_model", json!({})).unwrap()["model"].clone();
        record.source = None;
        record.scenarios.clear();
        record.index["native_execution"] = json!({
            "states":{"central":{"native_name":"central","source_unit":"mg","request_unit":"mg","dose_amount_unit":"mg"}},
            "dose_contracts":{"source_depot":{"native_target":"central","amount_unit":"mg","state_unit":"mg","source_dose_unit":"mg","source_coordinate":"source_depot","scaling":"One mg adds one mg before elimination."}}});
        let server = Server::new(library, false, policy::Tier::Free, 1);
        let mut args = json!({"model":{"library":{"model_id":id}},"outputs":["cp"],
            "end":{"value":20,"unit":"h"},"times":[{"value":0,"unit":"h"},{"value":20,"unit":"h"}],
            "doses":[{"target":"central","amount":{"value":150000,"unit":"ug"},"at":{"value":0,"unit":"h"},"lag":{"value":2,"unit":"h"},"every":{"value":12,"unit":"h"},"count":2,"duration":{"value":3,"unit":"h"},"bioavailability":0.5}]});
        let plan = server.plan_invocation("simulate", args.clone()).await;
        assert!(plan["error"].is_null(), "{plan}");
        let receipts = plan["plan"]["dose_event_receipts"].as_array().unwrap();
        assert_eq!(receipts.len(), 2);
        for (i, receipt) in receipts.iter().enumerate() {
            assert_eq!(receipt["stage"], "prepared_schedule");
            assert_eq!(receipt["canonical_target"], "central");
            assert_eq!(receipt["delivery"], "infusion");
            assert_eq!(
                receipt["time"],
                json!({"value":2.0+12.0*i as f64,"unit":"h"})
            );
            assert_eq!(receipt["duration"], json!({"value":3.0,"unit":"h"}));
            assert_eq!(
                receipt["amount_before_model_scaling"],
                json!({"value":75.0,"unit":"mg"})
            );
            assert_eq!(
                receipt["archive_source_coordinate"]["source_coordinate"],
                "source_depot"
            );
            assert_eq!(
                receipt["archive_source_coordinate"]["source_dose_unit"],
                "mg"
            );
        }
        args["expected_plan_hash"] = plan["plan_hash"].clone();
        let result = server.invoke("simulate", args).await;
        assert!(result["error"].is_null(), "{result}");
        let full = server
            .cache
            .get(result["run_id"].as_str().unwrap())
            .unwrap();
        let mut expected = plan["plan"]["dose_event_receipts"].clone();
        for receipt in expected.as_array_mut().unwrap() {
            receipt["stage"] = json!("executed_schedule");
        }
        assert_eq!(full["dose_event_receipts"], expected);
        let table = usability::csv(&full, &full["model_provenance"]["native_execution"]).unwrap();
        let manifest = crate::table::manifest(
            "dose.csv",
            &full,
            &table,
            crate::table::Context {
                metadata: &full["model_provenance"],
                request: &full["effective_request"],
                source,
                scenario_id: None,
                modified: true,
                effective_hash: full["effective_request_hash"].as_str().unwrap(),
                selected_outputs: &json!(["cp"]),
            },
        )
        .unwrap();
        let manifest: Value = serde_json::from_slice(&manifest).unwrap();
        assert_eq!(manifest["dose_event_receipts"], expected);
        // Exercise transport omission using an actual completed receipt, retaining
        // the full record in the same cache used by resource reads.
        let mut large = full.clone();
        large["dose_event_receipts"] = json!(vec![expected[0].clone(); 1000]);
        assert!(server
            .cache
            .put("large-dose-receipts".into(), large.clone()));
        large["full_result_resource"] = json!("qspquest://runs/large-dose-receipts");
        let response = server.response(large);
        assert!(serde_json::to_vec(&response).unwrap().len() <= policy::RESPONSE_LIMIT);
        let bounded = response.structured_content.unwrap();
        assert!(bounded["dose_event_receipts"].is_null());
        assert!(bounded["dose_event_receipt_notice"]
            .as_str()
            .unwrap()
            .contains("omitted inline"));
        assert_eq!(
            bounded["full_result_resource"],
            "qspquest://runs/large-dose-receipts"
        );
        assert_eq!(
            server.cache.get("large-dose-receipts").unwrap()["dose_event_receipts"]
                .as_array()
                .unwrap()
                .len(),
            1000
        );
    }
    #[tokio::test]
    async fn linear_pk_dose_receipts_preserve_depot_order_overrides_and_archive_units() {
        let oral = include_str!("../assets/templates/oral.qsp");
        let no_depot = oral.replace(" depot depot absorption ka = 0 [mg]\n", "");
        let scaled = no_depot.replace("outputs\n", "dosing\n central [mg] = 2\noutputs\n");
        let grams = scaled.replace("central [mg] = 2", "central [g] = 2");
        for (source, target, target_index, scale, amount_unit, event_amount) in [
            (oral, "depot", 0, 1., "mg", 3.),
            (oral, "central", 1, 1., "mg", 3.),
            (no_depot.as_str(), "central", 0, 1., "mg", 3.),
            (scaled.as_str(), "central", 0, 2., "mg", 3.),
            (grams.as_str(), "central", 0, 2., "g", 0.003),
        ] {
            let mut library = Library::embedded().unwrap();
            let record = &mut library.records[0];
            let id = record.metadata["id"].clone();
            record.model =
                compute::execute(source, "validate_model", json!({})).unwrap()["model"].clone();
            record.source = None;
            record.scenarios.clear();
            record.index["native_execution"] = json!({"states":{"archived_dose":{"native_name":target,
                "source_unit":"mg","request_unit":"mg","dose_amount_unit":amount_unit}},"dose_contracts":{"archived_dose":{
                "native_target":target,"amount_unit":amount_unit,"state_unit":"mg","source_dose_unit":"mg",
                "source_coordinate":"archived_dose","scaling":"Delivered mass precedes the model dosing scale."}}});
            let server = Server::new(library, false, policy::Tier::Free, 1);
            let mut args = json!({"model":{"library":{"model_id":id}},"outputs":["cp"],
                "end":{"value":2,"unit":"h"},"times":[{"value":0,"unit":"h"},{"value":2,"unit":"h"}],
                "doses":[{"target":"archived_dose","amount":{"value":6000,"unit":"ug"},"at":{"value":0,"unit":"h"},"bioavailability":0.5}]});
            let plan = server.plan_invocation("simulate", args.clone()).await;
            assert!(plan["error"].is_null(), "{source}: {plan}");
            let receipt = &plan["plan"]["dose_event_receipts"][0];
            assert_eq!(receipt["canonical_target"], target);
            assert_eq!(
                receipt["declared_base_scale"],
                json!(qspquest_core::expression::Expr::number(scale))
            );
            assert_eq!(
                receipt["amount_before_model_scaling"],
                json!({"value":event_amount,"unit":amount_unit})
            );
            assert_eq!(receipt["target_state_unit"], "mg");
            assert_eq!(
                receipt["archive_source_coordinate"]["source_coordinate"],
                "archived_dose"
            );
            args["expected_plan_hash"] = plan["plan_hash"].clone();
            let result = server.invoke("simulate", args).await;
            assert!(result["error"].is_null(), "{result}");
            let full = server
                .cache
                .get(result["run_id"].as_str().unwrap())
                .unwrap();
            let mut expected = plan["plan"]["dose_event_receipts"].clone();
            expected[0]["stage"] = json!("executed_schedule");
            assert_eq!(full["dose_event_receipts"], expected);
            let times = full["times"].as_array().unwrap();
            let values = full["outputs"][0]["values"].as_array().unwrap();
            for (i, (time, value)) in times.iter().zip(values).enumerate() {
                let t = time.as_f64().unwrap();
                let k = 0.8 / 6.;
                let expected = if t == 0. && full["sides"][i] == "pre" {
                    0.
                } else if target == "depot" {
                    0.5 * 1.2 / (1.2 - k) * ((-k * t).exp() - (-1.2 * t).exp())
                } else {
                    0.5 * scale * (-k * t).exp()
                };
                assert!((value.as_f64().unwrap() - expected).abs() < 1e-6, "{full}");
            }
            // A receipt must refuse a plan from a different state layout.
            let mut native: qspquest_core::planning::RunPlan =
                serde_json::from_value(plan["plan"]["native_preparation"]["plan"].clone()).unwrap();
            assert_eq!(native.events[0].event.target(), target_index);
            native.states[target_index].unit = "g".into();
            let library = server.library.read().unwrap();
            let resolved = &library.records[0].model;
            assert_eq!(
                crate::planning::dose_event_receipts(
                    &native,
                    resolved,
                    &library.records[0].index["native_execution"]
                )
                .unwrap_err()["error"]["code"],
                "dose_contract"
            );
        }
    }
    #[tokio::test]
    async fn contradictory_source_dose_coordinates_refuse_without_compute_slots() {
        let source = include_str!("../assets/templates/iv.qsp");
        for field in [
            "amount_unit",
            "state_unit",
            "native_target",
            "source_coordinate",
        ] {
            let mut library = Library::embedded().unwrap();
            let record = &mut library.records[0];
            let id = record.metadata["id"].clone();
            record.model =
                compute::execute(source, "validate_model", json!({})).unwrap()["model"].clone();
            record.source = None;
            record.scenarios.clear();
            let mut coordinate = json!({"native_target":"central","amount_unit":"mg","state_unit":"mg","source_dose_unit":"mg","source_coordinate":"source_depot","scaling":"One mg adds one mg."});
            coordinate[field] = json!(if field == "source_coordinate" {
                ""
            } else {
                "invalid"
            });
            record.index["native_execution"] = json!({"dose_contracts":{"depot":coordinate}});
            let server = Server::new(library, false, policy::Tier::Free, 1);
            let _permit = server.semaphore.clone().try_acquire_owned().unwrap();
            let failure = server
                .invoke(
                    "simulate",
                    json!({"model":{"library":{"model_id":id}},"end":{"value":2,"unit":"h"}}),
                )
                .await;
            assert_eq!(
                failure["error"]["code"], "dose_contract",
                "{field}: {failure}"
            );
            assert!(failure["outputs"].is_null() && failure["run_id"].is_null());
        }
    }
    #[tokio::test]
    async fn time_preflight_names_fields_before_compute_and_preserves_valid_protocols() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let request = json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf","budgets":{"solver_callbacks":1000000,"events":100,"output_values":10000},"initial_states":{"central":{"value":6,"unit":"mg"}},"regimen":{"start":{"value":2,"unit":"h"},"end":{"value":4,"unit":"h"},"samples":[{"value":120,"unit":"min"},{"value":180,"unit":"min"},{"value":240,"unit":"min"}],"administrations":[],"rtol":1e-10,"atol":1e-12}});
        let args = json!({"model":{"source":include_str!("../assets/templates/iv.qsp")},"request":request,"outputs":["cp"]});
        let permit = server.semaphore.clone().try_acquire_owned().unwrap();
        let mut bad = args.clone();
        bad["request"]["regimen"]["samples"][1]["value"] = json!(241);
        let rejected = server.invoke("simulate", bad).await;
        assert_eq!(
            rejected["error"]["input_diagnostic"]["field"],
            "effective_request.regimen.samples[1]"
        );
        assert_eq!(
            rejected["error"]["input_diagnostic"]["supplied_value"],
            json!({"value":241,"unit":"min"})
        );
        let mut bad = args.clone();
        bad["request"]["regimen"]["end"]["value"] = json!("NaN");
        let rejected = server.invoke("simulate", bad).await;
        assert_eq!(
            rejected["error"]["input_diagnostic"]["field"],
            "arguments.request.regimen.end.value"
        );
        assert_eq!(
            rejected["error"]["input_diagnostic"]["supplied_value"],
            "NaN"
        );
        drop(permit);
        let planned = server.plan_invocation("simulate", args.clone()).await;
        assert!(planned["error"].is_null(), "{planned}");
        assert_eq!(planned["plan"]["effective_request"], request);
        assert_eq!(planned["plan"]["time_validation"][0]["start"], 2.0);
        assert_eq!(planned["plan"]["time_validation"][0]["end"], 4.0);
        let mut args = args;
        args["expected_plan_hash"] = planned["plan_hash"].clone();
        let result = server.invoke("simulate", args).await;
        assert!(result["error"].is_null(), "{result}");
        let full = server
            .cache
            .get(result["run_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(full["effective_request"], request);
        assert_eq!(full["time_validation"], planned["plan"]["time_validation"]);
        for (time, value) in full["times"]
            .as_array()
            .unwrap()
            .iter()
            .zip(full["outputs"][0]["values"].as_array().unwrap())
        {
            assert!(
                (value.as_f64().unwrap() - 2.0 * (-0.1 * (time.as_f64().unwrap() - 2.0)).exp())
                    .abs()
                    < 1e-7
            );
        }
    }
    #[test]
    fn oversized_numerical_result_without_retention_is_an_explicit_failure() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1);
        let response = server.response(json!({"times":vec![0.0;100_000],"effective_request_hash":"synthetic request","identity":{"backend":"qspquest","algorithm":"synthetic fixture"}}));
        assert_eq!(response.is_error, Some(true));
        let value = response.structured_content.unwrap();
        assert_eq!(value["error"]["code"], "result_output_budget");
        assert!(value["contract"]["resources"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(value.get("times").is_none());
    }
    #[tokio::test]
    async fn retained_comparison_preserves_all_member_trajectories_beyond_cache_capacity() {
        let directory = std::env::temp_dir().join(format!(
            "qspquest-comparison-artifacts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 1)
            .with_artifact_storage(&directory)
            .unwrap();
        let mut args = comparison_args();
        args["regimens"] = json!((0..10).map(|i| json!({"name":format!("member{i}"),"doses":[{"target":"central","at":{"value":0,"unit":"h"},"amount":{"value":i+1,"unit":"mg"}}]})).collect::<Vec<_>>());
        let result = server.execute_comparison(args).await;
        assert!(result["error"].is_null(), "{result}");
        let store = server.artifact_store.as_ref().unwrap();
        assert_eq!(store.uris().unwrap().len(), 11);
        for member in result["regimens"].as_array().unwrap() {
            let inline = &member["result"];
            let descriptor: crate::artifacts::Descriptor =
                serde_json::from_value(inline["result_artifact"].clone()).unwrap();
            let id = descriptor
                .uri
                .strip_prefix(crate::artifacts::URI_PREFIX)
                .unwrap();
            assert_eq!(store.descriptor(id).unwrap().sha256, descriptor.sha256);
            let text: String = (0..descriptor.chunk_count)
                .map(|index| store.chunk(id, index).unwrap())
                .collect();
            let full: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(full["identity"], inline["identity"]);
            assert_eq!(full["plan_hash"], inline["plan_hash"]);
            assert_eq!(
                full["times"].as_array().unwrap().len(),
                inline["sampling"]["original_points"].as_u64().unwrap() as usize
            );
            assert_eq!(
                full["times"].as_array().unwrap().len(),
                full["sides"].as_array().unwrap().len()
            );
        }
        drop(server);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn oversized_comparison_preserves_named_members_and_complete_receipt_links() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 2);
        let records: Vec<Value> = (0..4).map(|i| json!({"name":format!("condition-{i}"),
            "input_receipt":{"requested_inputs_sha256":format!("inputs-{i}")},
            "result":{"identity":{"backend":"test"},"full_result_resource":format!("qspquest://runs/condition-{i}"),
                "scientific_assessment":crate::scientific::Receipt::new(&json!({}), true, None),
                "scheduled_boundary_receipt":{"boundaries":vec![0.;100_000]},
                "initialization_receipt":{"initialized_states":vec![0.;100_000]}}})).collect();
        let response = server.response(
            json!({"regimens":records,"plan_hash":"plan","effective_request_hash":"request"}),
        );
        assert_ne!(response.is_error, Some(true));
        assert!(serde_json::to_vec(&response).unwrap().len() < policy::RESPONSE_LIMIT);
        let value = response.structured_content.unwrap();
        for (i, member) in value["regimens"].as_array().unwrap().iter().enumerate() {
            assert_eq!(member["name"], format!("condition-{i}"));
            assert_eq!(
                member["input_receipt"]["requested_inputs_sha256"],
                format!("inputs-{i}")
            );
            assert_eq!(
                member["result"]["full_result_resource"],
                format!("qspquest://runs/condition-{i}")
            );
            assert!(member["result"]["scheduled_boundary_receipt"].is_null());
            assert!(member["result"]["scheduled_boundary_receipt_notice"]
                .as_str()
                .unwrap()
                .contains("named member"));
            assert!(member["result"]["initialization_receipt"].is_null());
            assert!(member["result"]["scientific_assessment"].is_null());
            assert!(member["result"]["scientific_assessment_notice"]
                .as_str()
                .unwrap()
                .contains("not_assessed"));
            assert!(member["result"]["initialization_receipt_notice"]
                .as_str()
                .unwrap()
                .contains("named member"));
        }
        assert_eq!(value["contract"]["resources"].as_array().unwrap().len(), 4);
    }
    #[test]
    fn oversized_inline_result_preserves_cached_resource() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 2);
        let result = server.response(json!({
            "times": vec![0.; 100_000],
            "full_result_resource": "qspquest://runs/cached-scientific-result",
            "effective_request_hash": "request-hash",
            "summary": {"values": vec![0.; 100_000]}
        }));
        assert_ne!(result.is_error, Some(true));
        let value = result.structured_content.as_ref().unwrap();
        assert_eq!(
            value["full_result_resource"],
            "qspquest://runs/cached-scientific-result"
        );
        assert_eq!(value["effective_request_hash"], "request-hash");
        assert!(serde_json::to_vec(&result).unwrap().len() < policy::RESPONSE_LIMIT);
        assert!(value.get("times").is_none());
    }
    #[test]
    fn oversized_pharmacology_summary_preserves_cached_resource() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 2);
        let result = server.response(json!({
            "times": vec![0.; 100_000],
            "full_result_resource": "qspquest://runs/large-model-result",
            "effective_request_hash": "large-request-hash",
            "summary": {"values": vec![0.; 100_000]},
            "pharmacology_summary": {"values": vec![0.; 100_000]}
        }));
        assert_ne!(result.is_error, Some(true));
        let value = result.structured_content.as_ref().unwrap();
        assert_eq!(
            value["full_result_resource"],
            "qspquest://runs/large-model-result"
        );
        assert_eq!(value["effective_request_hash"], "large-request-hash");
        assert!(serde_json::to_vec(&result).unwrap().len() < policy::RESPONSE_LIMIT);
        assert!(value.get("pharmacology_summary").is_none());
    }
    #[test]
    fn final_delivery_receipt_preserves_operation_outcomes_and_complete_links() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 2);
        let uri = format!("qspquest://managed-artifacts/fixture/{}", "a".repeat(64));
        for (fields, status, is_error) in [
            (json!({}), "success", false),
            (
                json!({"scientific_status":{"converged":false}}),
                "scientific_failure",
                false,
            ),
            (
                json!({"error":{"code":"Domain","message":"Synthetic domain failure"}}),
                "scientific_failure",
                true,
            ),
            (
                json!({"error":{"code":"unsupported_fit","message":"Synthetic unsupported contract"}}),
                "unsupported",
                true,
            ),
            (
                json!({"error":{"code":"invalid_input","message":"Synthetic request failure"}}),
                "request_failure",
                true,
            ),
            (
                json!({"error":{"code":"async_required","message":"Synthetic route"},
                "execution_route":{"kind":"governed_job","tool":"submit_simulation","capability_id":"pkpd.simulate",
                    "states":725,"requires_verified_session":true,"requires_platform_qualification":true,"local_execution_supported":true}}),
                "async_required",
                true,
            ),
        ] {
            let mut payload = fields;
            payload["notes"] = json!(["λ".repeat(200_000)]);
            payload["full_result_resource"] = json!(uri);
            payload["effective_request_hash"] = json!("synthetic-request");
            let result = server.response(payload);
            assert!(serde_json::to_vec(&result).unwrap().len() + 128 <= policy::RESPONSE_LIMIT);
            assert_eq!(result.is_error == Some(true), is_error);
            let value = result.structured_content.as_ref().unwrap();
            assert_eq!(value["contract"]["status"], status);
            assert_eq!(value["contract"]["completeness"], "resource_linked");
            assert_eq!(value["contract"]["resources"], json!([uri]));
            assert_eq!(value["full_result_resource"], uri);
            assert_eq!(value["delivery_receipt"]["inline_fields_omitted"], true);
            assert_eq!(
                value["delivery_receipt"]["scientific_qualification"],
                "not_assessed"
            );
            let _: crate::contract::ToolOutput = serde_json::from_value(value.clone()).unwrap();
            let mut payload = value.clone();
            payload.as_object_mut().unwrap().remove("contract");
            assert_eq!(
                value["contract"]["identity"]["payload_sha256"],
                hash(payload.to_string().as_bytes())
            );
            let encoded_content = serde_json::to_value(&result.content).unwrap();
            assert!(encoded_content
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["type"] == "resource_link" && v["uri"] == uri));
            if status == "async_required" {
                assert_eq!(value["contract"]["route"]["tool"], "submit_simulation");
            }
        }
        let result = server.response(json!({"notes":["x".repeat(400_000)],
            "error":{"code":"Domain","message":"Synthetic failure without a retained result"}}));
        let value = result.structured_content.as_ref().unwrap();
        assert!(serde_json::to_vec(&result).unwrap().len() + 128 <= policy::RESPONSE_LIMIT);
        assert_eq!(value["contract"]["status"], "scientific_failure");
        assert_eq!(value["contract"]["completeness"], "summary_only");
        assert_eq!(value["contract"]["resources"], json!([]));
        assert_eq!(value["error"]["code"], "Domain");
        assert_eq!(value["delivery_receipt"]["inline_fields_omitted"], true);
    }
    #[test]
    fn final_delivery_receipt_preserves_every_named_comparison_member() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 2);
        let rows: Vec<Value> = (0..25)
            .map(|i| {
                json!({"name":format!("condition-{i}"),
            "input_receipt":{"unused_detail":"x".repeat(20_000)},
            "result":{"full_result_resource":format!("qspquest://runs/delivery-{i}"),
                "effective_request_hash":format!("request-{i}")}})
            })
            .collect();
        let result =
            server.response(json!({"regimens":rows,"effective_request_hash":"group-request"}));
        assert!(serde_json::to_vec(&result).unwrap().len() + 128 <= policy::RESPONSE_LIMIT);
        let value = result.structured_content.as_ref().unwrap();
        assert_eq!(value["contract"]["status"], "success");
        assert_eq!(value["delivery_receipt"]["inline_fields_omitted"], true);
        assert_eq!(value["regimens"].as_array().unwrap().len(), 25);
        assert_eq!(value["contract"]["resources"].as_array().unwrap().len(), 25);
        for (i, row) in value["regimens"].as_array().unwrap().iter().enumerate() {
            assert_eq!(row["name"], format!("condition-{i}"));
            assert_eq!(
                row["result"]["full_result_resource"],
                format!("qspquest://runs/delivery-{i}")
            );
            assert_eq!(
                row["result"]["effective_request_hash"],
                format!("request-{i}")
            );
        }
    }
    #[test]
    fn final_delivery_receipt_preserves_local_job_lookup_and_completed_resources() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 2);
        let completed: Vec<Value> = (0..25)
            .map(|i| {
                json!({"index":i,
            "artifact":{"uri":format!("qspquest://runs/job-delivery-{i}")}})
            })
            .collect();
        let result = server.response(
            json!({"local_job":{"job_id":"synthetic-job","state":"succeeded",
            "instructions":"x".repeat(400_000),"completed":completed}}),
        );
        assert!(serde_json::to_vec(&result).unwrap().len() + 128 <= policy::RESPONSE_LIMIT);
        let value = result.structured_content.as_ref().unwrap();
        assert_eq!(value["contract"]["status"], "success");
        assert_eq!(
            value["delivery_receipt"]["local_job_lookup"]["job_id"],
            "synthetic-job"
        );
        assert_eq!(
            value["delivery_receipt"]["local_job_lookup"]["state"],
            "succeeded"
        );
        assert_eq!(value["contract"]["resources"].as_array().unwrap().len(), 25);
        let _: crate::contract::ToolOutput = serde_json::from_value(value.clone()).unwrap();
    }
    #[test]
    fn capped_failure_and_routing_preserve_status_and_resource_links() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 2);
        for (payload, status) in [
            (
                json!({"error":{"code":"Domain","message":"é".repeat(200_000)}}),
                "scientific_failure",
            ),
            (
                json!({"error":{"code":"async_required","message":"route"},
                "execution_route":{"kind":"governed_job","tool":"submit_simulation","capability_id":"pkpd.simulate","states":725,
                    "requires_verified_session":true,"requires_platform_qualification":true,"local_execution_supported":true},
                "unused":vec![0;200_000]}),
                "async_required",
            ),
            (
                json!({"regimens":[{"name":"fixture","result":{"full_result_resource":"qspquest://runs/fixture","times":vec![0;200_000]}}]}),
                "success",
            ),
        ] {
            let result = server.response(payload);
            assert!(serde_json::to_vec(&result).unwrap().len() < policy::RESPONSE_LIMIT);
            let value = result.structured_content.unwrap();
            assert_eq!(value["contract"]["status"], status);
            let _: crate::contract::ToolOutput = serde_json::from_value(value.clone()).unwrap();
            if status == "success" {
                assert_eq!(value["contract"]["completeness"], "resource_linked");
                assert_eq!(
                    value["contract"]["resources"],
                    json!(["qspquest://runs/fixture"])
                );
            } else {
                assert_eq!(value["contract"]["completeness"], "summary_only");
                assert!(value["error"]["message"].as_str().unwrap().len() < 4200);
            }
        }
    }

    #[test]
    fn capped_model_card_retains_structural_declaration_identity_and_complete_resource() {
        let server = Server::new(Library::embedded().unwrap(), false, policy::Tier::Free, 2);
        let uri = "qspquest://models/synthetic/details/1";
        let summary = json!({"status":"declared","entry_count":1,"native_model_sha256":"b".repeat(64),
            "author_code_qualification":"not_assessed","dimensional_qualification":false,"complete_declaration_resource":uri});
        let output = server.response(json!({"source_interpretation_summary":summary,"full_model_details_resource":uri,"unused":"x".repeat(200_000)}));
        assert!(serde_json::to_vec(&output).unwrap().len() < policy::RESPONSE_LIMIT);
        let value = output.structured_content.unwrap();
        assert_eq!(value["source_interpretation_summary"], summary);
        assert_eq!(value["full_model_details_resource"], uri);
        assert_eq!(value["contract"]["completeness"], "resource_linked");
    }
}

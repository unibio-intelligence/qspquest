//! Source declarations projected onto the active MCP transport and model archive.
use crate::{contract::JsonValueSchema, library::Record};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::OnceLock};

pub const REGISTRY_URI: &str = "qspquest://capability-registry";
pub const CATALOG_URI: &str = "qspquest://capability-catalog";
pub const TARGET: &str = env!("QSPQUEST_MCP_TARGET");

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub operation: String,
    pub interface: String,
    pub estimator: String,
    pub solver: String,
    pub target: String,
    pub execution_mode: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub key: Key,
    /// A local selected/versioned model archive; private platform models are not projected.
    pub model: Option<crate::server::ModelId>,
    /// Optional stored scenario. A modified request still needs normal tool admission.
    pub scenario_id: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub operation: String,
    pub interface: String,
    pub api: String,
    pub implemented: bool,
    pub exposed: bool,
    pub estimators: Vec<String>,
    pub solvers: Vec<String>,
    pub targets: Vec<String>,
    pub modes: Vec<String>,
    pub required_features: Vec<String>,
    pub requirements: String,
    #[schemars(with = "JsonValueSchema")]
    pub limits: Value,
    pub example_ids: Vec<String>,
    pub source: String,
    pub symbol: String,
    pub evidence_status: String,
    pub evidence_ids: Vec<String>,
}

struct Catalog {
    entries: Vec<Declaration>,
    compiler: String,
    digest: String,
}
fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let text = include_str!("../assets/guides/spec/capabilities.json");
        let value: Value = serde_json::from_str(text).expect("Validated capability catalog");
        assert_eq!(value["schema"], "qspquest.capability-catalog/v1");
        let entries: Vec<Declaration> =
            serde_json::from_value(value["entries"].clone()).expect("Capability declarations");
        assert!(entries
            .iter()
            .all(|e| e.evidence_status == "untested" && e.evidence_ids.is_empty()));
        Catalog {
            entries,
            compiler: value["compiler_source_sha256"].as_str().unwrap().into(),
            digest: crate::library::hash(text.as_bytes()),
        }
    })
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Assessment {
    Blocked,
    RequiresRequestValidation,
    NotAssessed,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelAssessment {
    pub model_id: String,
    pub version: Option<String>,
    pub archive_sha256: Option<String>,
    pub scenario_id: Option<String>,
    pub stored_request_sha256: Option<String>,
    pub execution_available: bool,
    pub fit_ready: bool,
    #[schemars(with = "JsonValueSchema")]
    pub availability: Value,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub schema: String,
    pub key: Key,
    pub declared: bool,
    pub declaration: Option<Declaration>,
    /// Null for another interface, target or deployment; false for a withheld active tool.
    pub active_exposed: Option<bool>,
    /// False when blocked; null until an exact request is admitted and executed.
    pub executable: Option<bool>,
    pub assessment: Assessment,
    pub blockers: Vec<String>,
    pub model: Option<ModelAssessment>,
    pub qualified: bool,
    pub evidence_status: String,
    pub evidence_ids: Vec<String>,
    pub catalog_uri: String,
    pub catalog_sha256: String,
    pub compiler_source_sha256: String,
    pub scope: String,
}

pub struct Runtime<'a> {
    pub target: &'a str,
    pub mode: &'a str,
    pub tools: &'a [String],
    pub features: &'a BTreeMap<String, bool>,
    pub governed_ready: bool,
    pub local_jobs_ready: bool,
    pub user_source_execution: bool,
}

pub fn answer(query: &Query, runtime: &Runtime<'_>, record: Option<&Record>) -> Answer {
    let key = &query.key;
    let declaration = catalog()
        .entries
        .iter()
        .find(|entry| {
            entry.operation == key.operation
                && entry.interface == key.interface
                && entry.estimators.contains(&key.estimator)
                && entry.solvers.contains(&key.solver)
                && entry.targets.contains(&key.target)
                && entry.modes.contains(&key.execution_mode)
        })
        .cloned();
    let active = key.interface == "mcp"
        && key.target == runtime.target
        && key.execution_mode == runtime.mode;
    let mut blockers = Vec::new();
    let exposed = declaration.as_ref().is_some_and(|entry| {
        entry.exposed
            && runtime.tools.contains(&entry.api)
            && entry
                .required_features
                .iter()
                .all(|f| runtime.features.get(f) == Some(&true))
    });
    if declaration.is_none() {
        blockers.push("Combination is absent from the supported source catalog.".into());
    } else if !active {
        blockers
            .push("This server does not assess another interface, target or deployment.".into());
    } else if !exposed {
        blockers.push("The active build does not expose this tool or required feature.".into());
    }
    let governed = declaration
        .as_ref()
        .is_some_and(|entry| entry.required_features.iter().any(|f| f == "managed"));
    if active && governed && !runtime.governed_ready {
        blockers.push("Governed operations require a configured managed backend and verified session; platform readiness is assessed by its preview.".into());
    }
    if active
        && matches!(
            key.operation.as_str(),
            "submit_local_job" | "get_local_job" | "cancel_local_job" | "resume_local_job"
        )
        && !runtime.local_jobs_ready
    {
        blockers.push("Local jobs require local stdio with --jobs-dir PATH and --artifact-dir PATH in distinct dedicated private directories.".into());
    }
    let mut model = None;
    if let Some(requested) = &query.model {
        if key.interface != "mcp" || governed {
            blockers.push("Model feasibility is assessed only for native MCP operations; platform models require an authorized governed preview.".into());
        } else if let Some(record) = record {
            let scenario = query
                .scenario_id
                .as_ref()
                .and_then(|id| record.scenarios.get(id));
            let scientific = key.solver != "none";
            if scientific && record.metadata["candidate"] == true && !runtime.user_source_execution
            {
                blockers.push("Hosted user candidates require a configured sandbox.".into());
            }
            if scientific && record.model.is_null() {
                blockers
                    .push("This archive is inspection only and has no executable model.".into());
            }
            if query.scenario_id.is_some() && scenario.is_none() {
                blockers.push("Stored scenario is absent from this model/version.".into());
            }
            if scientific
                && key.solver == "analytic_linear_pk"
                && record.model["linear_pk"].is_null()
            {
                blockers
                    .push("The analytic solver requires this model to declare linear_pk.".into());
            }
            if scientific
                && key.operation.starts_with("fit_")
                && record.metadata["candidate"] != true
                && (record.metadata["fit_ready"] != true
                    || record.metadata["fitting_supported"] != true
                    || !crate::fitting::supports_operation(&record.fitting, &key.operation))
            {
                blockers.push(
                    "The released archive has no ready fitting contract for this operation.".into(),
                );
            }
            if runtime.mode != "local_stdio" && scientific {
                if let Some(route) = crate::policy::async_route(&record.model, &key.operation) {
                    blockers.push(
                        route["error"]["message"]
                            .as_str()
                            .unwrap_or("This model requires a governed job.")
                            .into(),
                    );
                }
            }
            model = Some(ModelAssessment {
                model_id: requested.model_id.clone(),
                version: record.metadata["version"].as_str().map(str::to_owned),
                archive_sha256: Some(record.archive_hash.clone()),
                scenario_id: query.scenario_id.clone(),
                stored_request_sha256: scenario
                    .map(|s| crate::library::hash(s.to_string().as_bytes())),
                execution_available: !record.model.is_null(),
                fit_ready: record.metadata["fit_ready"] == true,
                availability: record.metadata["availability"].clone(),
            });
        } else {
            blockers.push("Model/version is absent from the server's native catalog. Use authorized platform discovery for private models.".into());
        }
    } else if query.scenario_id.is_some() {
        blockers.push("scenario_id requires a model identity.".into());
    }
    let blocked = declaration.is_none() || active && !blockers.is_empty();
    Answer {
        schema: "qspquest.capability-answer/v1".into(), key: key.clone(),
        declared: declaration.is_some(), declaration,
        active_exposed: active.then_some(exposed), executable: blocked.then_some(false),
        assessment: if blocked { Assessment::Blocked } else if active && !governed { Assessment::RequiresRequestValidation } else { Assessment::NotAssessed },
        blockers, model, qualified: false, evidence_status: "not_assessed".into(), evidence_ids: vec![],
        catalog_uri: CATALOG_URI.into(), catalog_sha256: catalog().digest.clone(), compiler_source_sha256: catalog().compiler.clone(),
        scope: "Source declarations and active tool/model blockers only. Null executable means an exact request must pass the normal tool admission and execution. No solve, fit, job, file export or private platform lookup occurs. Archive evidence does not qualify this build, target or modified request.".into(),
    }
}

#[derive(Serialize, JsonSchema)]
pub struct Registry {
    pub schema: String,
    pub catalog_uri: String,
    pub catalog_sha256: String,
    pub compiler_source_sha256: String,
    pub target: String,
    pub execution_mode: String,
    pub entries: Vec<Answer>,
    pub limits: crate::contract::Limits,
}

pub fn registry(runtime: &Runtime<'_>, limits: crate::contract::Limits) -> Registry {
    let mut entries = Vec::new();
    for e in &catalog().entries {
        if e.interface != "mcp"
            || !e.targets.iter().any(|t| t == runtime.target)
            || !e.modes.iter().any(|m| m == runtime.mode)
        {
            continue;
        }
        for estimator in &e.estimators {
            for solver in &e.solvers {
                let query = Query {
                    key: Key {
                        operation: e.operation.clone(),
                        interface: e.interface.clone(),
                        estimator: estimator.clone(),
                        solver: solver.clone(),
                        target: runtime.target.into(),
                        execution_mode: runtime.mode.into(),
                    },
                    model: None,
                    scenario_id: None,
                };
                entries.push(answer(&query, runtime, None));
            }
        }
    }
    Registry {
        schema: "qspquest.capability-registry/v1".into(),
        catalog_uri: CATALOG_URI.into(),
        catalog_sha256: catalog().digest.clone(),
        compiler_source_sha256: catalog().compiler.clone(),
        target: runtime.target.into(),
        execution_mode: runtime.mode.into(),
        entries,
        limits,
    }
}

pub fn schema() -> Value {
    serde_json::to_value(schemars::schema_for!(Registry)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn declarations_do_not_override_active_tools_models_or_evidence() {
        let tools = vec![
            "simulate".into(),
            "fit_individual".into(),
            "get_model".into(),
        ];
        let features = BTreeMap::new();
        let runtime = Runtime {
            target: TARGET,
            mode: "local_stdio",
            tools: &tools,
            features: &features,
            governed_ready: false,
            local_jobs_ready: false,
            user_source_execution: true,
        };
        let library = crate::library::Library::embedded().unwrap();
        let mut record = library.records[0].clone();
        let mut query = Query {
            key: Key {
                operation: "simulate".into(),
                interface: "mcp".into(),
                estimator: "none".into(),
                solver: "diffsol_bdf".into(),
                target: TARGET.into(),
                execution_mode: "local_stdio".into(),
            },
            model: Some(crate::server::ModelId {
                expected_inspection_hash: None,
                model_id: record.metadata["id"].as_str().unwrap().into(),
                version: None,
            }),
            scenario_id: None,
        };
        let result = answer(&query, &runtime, Some(&record));
        assert!(matches!(
            result.assessment,
            Assessment::RequiresRequestValidation
        ));
        assert_eq!(result.active_exposed, Some(true));
        assert_eq!(result.executable, None);
        assert!(!result.qualified);
        assert!(result.evidence_ids.is_empty());
        query.scenario_id = Some("absent-scenario".into());
        assert!(matches!(
            answer(&query, &runtime, Some(&record)).assessment,
            Assessment::Blocked
        ));
        query.scenario_id = None;
        assert_eq!(answer(&query, &runtime, None).executable, Some(false));
        record.model = Value::Null;
        assert_eq!(
            answer(&query, &runtime, Some(&record)).executable,
            Some(false)
        );
        query.key.operation = "get_model".into();
        query.key.solver = "none".into();
        assert!(matches!(
            answer(&query, &runtime, Some(&record)).assessment,
            Assessment::RequiresRequestValidation
        ));
        query.key.operation = "fit_individual".into();
        query.key.solver = "diffsol_bdf".into();
        query.key.estimator = "individual".into();
        record = library.records[0].clone();
        record.metadata["candidate"] = json!(false);
        record.metadata["fit_ready"] = json!(false);
        assert_eq!(
            answer(&query, &runtime, Some(&record)).executable,
            Some(false)
        );
        query.key.operation = "simulate".into();
        query.key.estimator = "none".into();
        query.key.solver = "analytic_linear_pk".into();
        record.model["linear_pk"] = Value::Null;
        assert_eq!(
            answer(&query, &runtime, Some(&record)).executable,
            Some(false)
        );
        query.key.solver = "diffsol_rosenbrock23".into();
        assert!(!answer(&query, &runtime, Some(&record)).declared);
        query.key.solver = "diffsol_bdf".into();
        query.key.interface = "python".into();
        query.key.execution_mode = "in_process".into();
        query.model = None;
        let other = answer(&query, &runtime, None);
        assert!(other.declared && other.active_exposed.is_none() && other.executable.is_none());
        assert!(matches!(other.assessment, Assessment::NotAssessed));
        query.key.interface = "mcp".into();
        query.key.operation = "steady_state".into();
        query.key.solver = "analytic_linear_pk".into();
        query.key.execution_mode = "local_stdio".into();
        assert_eq!(answer(&query, &runtime, None).active_exposed, Some(false));
    }

    #[test]
    fn hosted_candidates_and_governed_routes_require_configured_boundaries() {
        let tools = vec!["simulate".into(), "submit_simulation".into()];
        let features = BTreeMap::from([("managed".into(), true)]);
        let runtime = Runtime {
            target: TARGET,
            mode: "managed_http",
            tools: &tools,
            features: &features,
            governed_ready: false,
            local_jobs_ready: false,
            user_source_execution: false,
        };
        let mut record = crate::library::Library::embedded().unwrap().records[0].clone();
        record.metadata["candidate"] = json!(true);
        let mut query = Query {
            key: Key {
                operation: "simulate".into(),
                interface: "mcp".into(),
                estimator: "none".into(),
                solver: "diffsol_bdf".into(),
                target: TARGET.into(),
                execution_mode: "managed_http".into(),
            },
            model: Some(crate::server::ModelId {
                expected_inspection_hash: None,
                model_id: "candidate".into(),
                version: None,
            }),
            scenario_id: None,
        };
        assert_eq!(
            answer(&query, &runtime, Some(&record)).executable,
            Some(false)
        );
        query.key.operation = "submit_simulation".into();
        query.key.solver = "none".into();
        query.model = None;
        let result = answer(&query, &runtime, None);
        assert_eq!(result.active_exposed, Some(true));
        assert_eq!(result.executable, Some(false));
        assert!(!result.qualified);
        let configured = Runtime {
            governed_ready: true,
            ..runtime
        };
        let result = answer(&query, &configured, None);
        assert!(matches!(result.assessment, Assessment::NotAssessed));
        assert!(result.executable.is_none());
    }
}

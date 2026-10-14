//! Located input-name refusals without guessing a scientific replacement.
use crate::{core::usability, inspection};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InputGroup {
    Parameter,
    InitialState,
    DoseTarget,
    ScanParameter,
    PatientInput,
    StateReset,
    StateTolerance,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    UnknownName,
    CanonicalNameRequired,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Migration {
    InspectCurrentBindings,
    InspectSubmittedModel,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputBindingFailure {
    pub group: InputGroup,
    pub reason: RefusalReason,
    pub field: String,
    pub field_truncated: bool,
    pub complete_field_sha256: Option<String>,
    pub supplied_name: String,
    pub name_truncated: bool,
    pub complete_name_sha256: Option<String>,
    pub native_request: bool,
    pub canonical_name: Option<String>,
    pub native_unit: Option<String>,
    pub model_id: String,
    pub model_id_truncated: bool,
    pub complete_model_id_sha256: Option<String>,
    pub model_version: Option<String>,
    pub model_version_truncated: bool,
    /// Exact source text selected for this refused preparation, not an engine build hash.
    pub selected_source_sha256: String,
    /// Current library inspection context, when available. No replacement is applied.
    pub current_inspection_hash: Option<String>,
    pub inspection_index_resource: Option<String>,
    pub migration: Migration,
    #[schemars(schema_with = "not_started_schema")]
    pub numerical_execution_started: bool,
}
fn not_started_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::Schema::from(serde_json::Map::from_iter([("const".into(), json!(false))]))
}

pub(crate) struct NameIssue {
    group: InputGroup,
    reason: RefusalReason,
    field: String,
    name: String,
    native_request: bool,
    canonical_name: Option<String>,
    native_unit: Option<String>,
}
fn issue(
    group: InputGroup,
    field: String,
    name: &str,
    table: &Value,
    native_request: bool,
) -> Option<NameIssue> {
    let entry = usability::lookup(name, table);
    let reason = match &entry {
        None => RefusalReason::UnknownName,
        Some((target, _)) if native_request && target != name => {
            RefusalReason::CanonicalNameRequired
        }
        Some(_) => return None,
    };
    Some(NameIssue {
        group,
        reason,
        field,
        name: name.into(),
        native_request,
        canonical_name: entry.as_ref().map(|(target, _)| target.clone()),
        native_unit: entry.and_then(|(_, row)| row["request_unit"].as_str().map(str::to_owned)),
    })
}
/// Check only documented friendly names and explicit native run inputs. Never rewrite inputs.
pub(crate) fn check(operation: &str, args: &Value, native: &Value) -> Option<NameIssue> {
    if operation == "get_patient_parameters" {
        for name in args["covariates"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, _)| name)
        {
            if let Some(error) = issue(
                InputGroup::PatientInput,
                format!("covariates.{name}"),
                name,
                &native["parameters"],
                false,
            ) {
                return Some(error);
            }
        }
        return None;
    }
    if !matches!(operation, "simulate" | "scan_parameters") {
        return None;
    }
    for (field, table, group) in [
        ("parameters", "parameters", InputGroup::Parameter),
        ("initial_states", "states", InputGroup::InitialState),
    ] {
        for name in args[field]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, _)| name)
        {
            if let Some(error) = issue(
                group,
                format!("{field}.{name}"),
                name,
                &native[table],
                false,
            ) {
                return Some(error);
            }
        }
        if args["request"]["schema"] == "qspquest.run/v0.1" {
            for name in args["request"][field]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(name, _)| name)
            {
                if let Some(error) = issue(
                    group,
                    format!("request.{field}.{name}"),
                    name,
                    &native[table],
                    true,
                ) {
                    return Some(error);
                }
            }
        }
    }
    if operation == "scan_parameters" {
        if let Some(name) = args["parameter"].as_str() {
            if let Some(error) = issue(
                InputGroup::ScanParameter,
                "parameter".into(),
                name,
                &native["parameters"],
                false,
            ) {
                return Some(error);
            }
        }
    }
    for (index, dose) in args["doses"].as_array().into_iter().flatten().enumerate() {
        if let Some(name) = dose["target"].as_str() {
            if let Some(error) = issue(
                InputGroup::DoseTarget,
                format!("doses[{index}].target"),
                name,
                &native["states"],
                false,
            ) {
                return Some(error);
            }
        }
    }
    if args["request"]["schema"] == "qspquest.run/v0.1" {
        for (field, group) in [
            ("administrations", InputGroup::DoseTarget),
            ("resets", InputGroup::StateReset),
        ] {
            for (index, event) in args["request"]["regimen"][field]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                if let Some(name) = event["target"].as_str() {
                    if let Some(error) = issue(
                        group,
                        format!("request.regimen.{field}[{index}].target"),
                        name,
                        &native["states"],
                        true,
                    ) {
                        return Some(error);
                    }
                }
            }
        }
        for name in args["request"]["regimen"]["absolute_tolerances"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, _)| name)
        {
            if let Some(error) = issue(
                InputGroup::StateTolerance,
                format!("request.regimen.absolute_tolerances.{name}"),
                name,
                &native["states"],
                true,
            ) {
                return Some(error);
            }
        }
    }
    None
}
fn bounded(text: &str, limit: usize) -> (String, bool, Option<String>) {
    let value: String = text.chars().take(limit).collect();
    let truncated = value.len() < text.len();
    let digest = truncated.then(|| format!("sha256:{}", crate::library::hash(text.as_bytes())));
    (value, truncated, digest)
}
pub(crate) fn error(
    issue: NameIssue,
    model_id: &str,
    version: Option<&str>,
    source_sha256: &str,
    library_selection: bool,
    index: Option<inspection::Index>,
) -> Value {
    let (field, field_truncated, complete_field_sha256) = bounded(&issue.field, 256);
    let (name, name_truncated, complete_name_sha256) = bounded(&issue.name, 128);
    let (model_id, model_id_truncated, complete_model_id_sha256) = bounded(model_id, 256);
    let (model_version, model_version_truncated) = version
        .map(|v| {
            let (text, shortened, _) = bounded(v, 128);
            (Some(text), shortened)
        })
        .unwrap_or((None, false));
    let code = match issue.reason {
        RefusalReason::UnknownName => "input_alias_unknown",
        RefusalReason::CanonicalNameRequired => "input_name_requires_native",
    };
    let migration = if library_selection {
        Migration::InspectCurrentBindings
    } else {
        Migration::InspectSubmittedModel
    };
    let message = match issue.reason {
        RefusalReason::UnknownName => "Input name is unavailable in the selected model's input table. Inspect its current native names, units and declarations, then explicitly select the intended input before creating another plan.",
        RefusalReason::CanonicalNameRequired => "A complete native request requires canonical input names and native units. Inspect the declared binding, or supply the alias through its documented top-level friendly input field; no input was moved or rewritten.",
    };
    let failure = InputBindingFailure {
        group: issue.group,
        reason: issue.reason,
        field: field.clone(),
        field_truncated,
        complete_field_sha256,
        supplied_name: name,
        name_truncated,
        complete_name_sha256,
        native_request: issue.native_request,
        canonical_name: issue.canonical_name.filter(|n| n.len() <= 512),
        native_unit: issue.native_unit.filter(|u| u.len() <= 128),
        model_id,
        model_id_truncated,
        complete_model_id_sha256,
        model_version,
        model_version_truncated,
        selected_source_sha256: source_sha256.into(),
        current_inspection_hash: index.as_ref().map(|i| i.context.inspection_hash.clone()),
        inspection_index_resource: index
            .map(|i| i.index_resource)
            .filter(|uri| uri.len() <= 2048),
        migration,
        numerical_execution_started: false,
    };
    json!({"error":{"code":code,"message":message,"expression":field,"input_binding":failure,
        "suggestion":"Inspect the selected model and its binding context. Preserve the intended experiment, choose names and units explicitly, and request a new plan."}})
}

//! Model execution and explicitly declared source relationships for discovery.
use crate::library::Record;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelReference {
    #[schemars(length(min = 1, max = 128))]
    pub model_id: String,
    #[schemars(length(min = 1, max = 128))]
    pub version: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FamilyDeclaration {
    #[schemars(length(min = 1, max = 128))]
    pub family_id: String,
    #[schemars(length(min = 1, max = 512))]
    pub paper_id: String,
    pub canonical_model: ModelReference,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    Full,
    Reduced,
    Synthetic,
    Unrecorded,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Representation {
    NativeProjection,
    UserSource,
    SourceInspection,
    MetadataOnly,
    GovernedCatalog,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    #[serde(rename = "qspquest_native")]
    QSPQuestNative,
    GovernedJob,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ExecutionAvailability {
    Known(bool),
    Unassessed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ScenarioCount {
    Known(usize),
    Unrecorded,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAssessment {
    RequiresRequestValidation,
    RequiresGovernedPreview,
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FamilyStatus {
    Declared,
    Unrecorded,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FamilyMember {
    pub model: ModelReference,
    pub representation: Representation,
    pub fidelity: Fidelity,
    pub execution_available: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FullSourceFamily {
    pub declaration: FamilyDeclaration,
    #[schemars(length(max = 32))]
    pub members: Vec<FamilyMember>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceFamily {
    pub status: FamilyStatus,
    pub declaration: Option<FamilyDeclaration>,
    pub member_count: usize,
    pub members_complete: bool,
    #[schemars(length(max = 8))]
    pub members: Vec<FamilyMember>,
    pub full_family_resource: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelDiscovery {
    pub execution_available: ExecutionAvailability,
    pub execution_assessment: ExecutionAssessment,
    pub executable_backend: Option<Backend>,
    pub representation: Representation,
    pub fidelity: Fidelity,
    pub archive_sha256: Option<String>,
    pub scenario_count: ScenarioCount,
    /// Projection availability still requires validation of the intended request.
    pub request_validation_required: bool,
    /// Relationships and fidelity are catalog declarations, not scientific qualification.
    pub source_family: SourceFamily,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct PlatformDiscoveryModel {
    pub id: String,
    pub version: Option<String>,
    pub execution_available: ExecutionAvailability,
    pub scenario_count: ScenarioCount,
    pub discovery: ModelDiscovery,
    /// Authorized provider metadata; the closed discovery record is the MCP projection.
    #[serde(flatten)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[cfg(any(feature = "managed", test))]
pub(crate) fn platform_model(record: &serde_json::Value) -> Result<PlatformDiscoveryModel, String> {
    use serde_json::json;
    if record["access"]["tier_locked"] == true {
        return Err("Platform model is tier locked; select an authorized model".into());
    }
    if !record["access"]["tier_locked"].is_null() && !record["access"]["tier_locked"].is_boolean() {
        return Err("Platform access tier_locked must be a boolean".into());
    }
    if !record.is_object() || !record["id"].as_str().is_some_and(|id| bounded(id, 128)) {
        return Err("Platform model needs a bounded nonempty id".into());
    }
    if !record["version"].is_null() && !record["version"].as_str().is_some_and(|v| bounded(v, 128))
    {
        return Err(
            "Platform model version must be a bounded nonempty string when supplied".into(),
        );
    }
    let availability: ExecutionAvailability =
        serde_json::from_value(record["execution_available"].clone())
            .map_err(|_| "Platform execution_available must be boolean or null")?;
    let count: ScenarioCount = serde_json::from_value(record["scenario_count"].clone())
        .map_err(|_| "Platform scenario_count must be a nonnegative integer or null")?;
    let description = ModelDiscovery {
        execution_available: availability,
        execution_assessment: if availability == ExecutionAvailability::Known(false) {
            ExecutionAssessment::Unavailable
        } else {
            ExecutionAssessment::RequiresGovernedPreview
        },
        executable_backend: (availability == ExecutionAvailability::Known(true))
            .then_some(Backend::GovernedJob),
        representation: Representation::GovernedCatalog,
        fidelity: fidelity(record)?,
        archive_sha256: None,
        scenario_count: count,
        request_validation_required: true,
        source_family: SourceFamily {
            status: FamilyStatus::Unrecorded,
            declaration: None,
            member_count: 0,
            members_complete: true,
            members: Vec::new(),
            full_family_resource: None,
        },
    };
    let mut row = record.clone();
    row["execution_available"] = json!(availability);
    row["scenario_count"] = json!(count);
    row["discovery"] = json!(description);
    serde_json::from_value(row).map_err(|_| "Invalid platform discovery record".into())
}
#[cfg(any(feature = "managed", test))]
pub(crate) fn platform_models(
    data: &serde_json::Value,
    query: &str,
    filters: &serde_json::Value,
) -> Result<Vec<PlatformDiscoveryModel>, String> {
    let records = data["models"]
        .as_array()
        .ok_or("Platform catalog needs a models array")?;
    let tokens: Vec<_> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut selected = Vec::new();
    for record in records {
        if record["access"]["tier_locked"] == true {
            continue;
        }
        let row = platform_model(record)?;
        if filters["model_id"]
            .as_str()
            .is_some_and(|wanted| row.id != wanted)
            || filters["version"]
                .as_str()
                .is_some_and(|wanted| row.version.as_deref() != Some(wanted))
            || !tokens
                .iter()
                .all(|token| record.to_string().to_lowercase().contains(token))
        {
            continue;
        }
        selected.push(row);
        if selected.len() == 20 {
            break;
        }
    }
    Ok(selected)
}
fn bounded(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}
fn reference(record: &Record) -> ModelReference {
    ModelReference {
        model_id: record.metadata["id"].as_str().unwrap_or("").into(),
        version: record.metadata["version"].as_str().unwrap_or("").into(),
    }
}
fn fidelity(metadata: &serde_json::Value) -> Result<Fidelity, String> {
    match metadata.get("model_fidelity") {
        None => Ok(Fidelity::Unrecorded),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|_| "model_fidelity must be full, reduced, synthetic or unrecorded".into()),
    }
}
fn declaration(record: &Record) -> Result<Option<FamilyDeclaration>, String> {
    let Some(value) = record.metadata.get("source_family") else {
        return Ok(None);
    };
    let declaration: FamilyDeclaration = serde_json::from_value(value.clone())
        .map_err(|_| "source_family needs family_id, paper_id and canonical_model {model_id, version}; unknown fields are forbidden".to_owned())?;
    if !bounded(&declaration.family_id, 128)
        || !bounded(&declaration.paper_id, 512)
        || !bounded(&declaration.canonical_model.model_id, 128)
        || !bounded(&declaration.canonical_model.version, 128)
    {
        return Err(
            "source_family identifiers must be nonempty, bounded and contain no control characters"
                .into(),
        );
    }
    Ok(Some(declaration))
}
fn representation(record: &Record) -> Representation {
    if record.source.is_some() {
        Representation::UserSource
    } else if !record.model.is_null() {
        Representation::NativeProjection
    } else if record.archive_hash.is_empty() {
        Representation::MetadataOnly
    } else {
        Representation::SourceInspection
    }
}
fn families(records: &[Record]) -> Result<BTreeMap<String, FullSourceFamily>, String> {
    let mut groups: BTreeMap<String, FullSourceFamily> = BTreeMap::new();
    for record in records {
        let fidelity = fidelity(&record.metadata)?;
        let Some(declaration) = declaration(record)? else {
            continue;
        };
        let group = groups
            .entry(declaration.family_id.clone())
            .or_insert_with(|| FullSourceFamily {
                declaration: declaration.clone(),
                members: Vec::new(),
            });
        if group.declaration != declaration {
            return Err("Members of a source family must declare the same paper_id and canonical model/version".into());
        }
        if group.members.len() >= 32 {
            return Err("A source family may contain at most 32 model versions".into());
        }
        let model = reference(record);
        if !bounded(&model.model_id, 128) || !bounded(&model.version, 128) {
            return Err(
                "source_family member model_id and version must be nonempty and at most 128 bytes"
                    .into(),
            );
        }
        if group.members.iter().any(|member| member.model == model) {
            return Err("source_family member model/version references must be unique".into());
        }
        group.members.push(FamilyMember {
            model,
            representation: representation(record),
            fidelity,
            execution_available: !record.model.is_null(),
        });
    }
    for group in groups.values_mut() {
        let canonical = &group.declaration.canonical_model;
        let matches: Vec<_> = group
            .members
            .iter()
            .filter(|member| &member.model == canonical)
            .collect();
        if matches.len() != 1
            || !matches[0].execution_available
            || matches[0].fidelity != Fidelity::Full
        {
            return Err("source_family canonical_model must resolve to exactly one executable full model in the same family and version".into());
        }
        group.members.sort_by(|a, b| {
            (&a.model.model_id, &a.model.version).cmp(&(&b.model.model_id, &b.model.version))
        });
    }
    Ok(groups)
}
pub(crate) fn validate(records: &[Record]) -> Result<(), String> {
    families(records).map(|_| ())
}
pub(crate) fn family_records(records: &[Record]) -> Result<Vec<FullSourceFamily>, String> {
    families(records).map(|groups| groups.into_values().collect())
}
pub(crate) fn family_uri(family: &FullSourceFamily) -> String {
    format!(
        "qspquest://source-families/{}",
        crate::library::hash(serde_json::to_string(family).unwrap().as_bytes())
    )
}
pub(crate) fn describe_all(records: &[Record]) -> Result<Vec<ModelDiscovery>, String> {
    let groups = families(records)?;
    records
        .iter()
        .map(|record| {
            let source_family = declaration(record)?
                .and_then(|d| groups.get(&d.family_id))
                .map(|group| SourceFamily {
                    status: FamilyStatus::Declared,
                    declaration: Some(group.declaration.clone()),
                    member_count: group.members.len(),
                    members_complete: group.members.len() <= 8,
                    members: group.members.iter().take(8).cloned().collect(),
                    full_family_resource: Some(family_uri(group)),
                })
                .unwrap_or(SourceFamily {
                    status: FamilyStatus::Unrecorded,
                    declaration: None,
                    member_count: 0,
                    members_complete: true,
                    members: Vec::new(),
                    full_family_resource: None,
                });
            let execution_available = !record.model.is_null();
            Ok(ModelDiscovery {
                execution_available: ExecutionAvailability::Known(execution_available),
                execution_assessment: if execution_available {
                    ExecutionAssessment::RequiresRequestValidation
                } else {
                    ExecutionAssessment::Unavailable
                },
                executable_backend: execution_available.then_some(Backend::QSPQuestNative),
                representation: representation(record),
                fidelity: fidelity(&record.metadata)?,
                archive_sha256: (record.source.is_none() && !record.archive_hash.is_empty())
                    .then(|| record.archive_hash.clone()),
                scenario_count: ScenarioCount::Known(record.scenarios.len()),
                request_validation_required: execution_available,
                source_family,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn governed_records_distinguish_unassessed_unavailable_and_declared_execution() {
        let data = json!({"models":[
            {"id":"unknown","status":"ready","access":{"tier_locked":false}},
            {"id":"available","version":"2","execution_available":true,"scenario_count":3},
            {"id":"unavailable","execution_available":false,"scenario_count":0},
            {"id":"hidden","access":{"tier_locked":true},"private_canary":"hidden"}
        ]});
        let rows = platform_models(&data, "", &json!({})).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0].execution_available,
            ExecutionAvailability::Unassessed
        );
        assert_eq!(
            rows[0].discovery.execution_assessment,
            ExecutionAssessment::RequiresGovernedPreview
        );
        assert_eq!(rows[0].scenario_count, ScenarioCount::Unrecorded);
        assert_eq!(rows[0].discovery.executable_backend, None);
        assert_eq!(
            rows[1].discovery.executable_backend,
            Some(Backend::GovernedJob)
        );
        assert_eq!(
            rows[1].discovery.execution_assessment,
            ExecutionAssessment::RequiresGovernedPreview
        );
        assert_eq!(
            rows[2].discovery.execution_assessment,
            ExecutionAssessment::Unavailable
        );
        assert_eq!(rows[2].discovery.executable_backend, None);
        let serialized = serde_json::to_value(&rows).unwrap();
        assert!(serialized[0]["execution_available"].is_null());
        assert!(!serialized.to_string().contains("private_canary"));
        assert_eq!(
            rows[0].discovery.source_family.status,
            FamilyStatus::Unrecorded
        );
        assert_eq!(rows[0].discovery.archive_sha256, None);
    }
    #[test]
    fn governed_exact_filters_respect_version_and_do_not_infer_active_identity() {
        let data = json!({"models":[{"id":"one","version":"1"},{"id":"two","version":"2"},{"id":"active"}]});
        assert_eq!(
            platform_models(&data, "", &json!({"model_id":"two","version":"2"}))
                .unwrap()
                .len(),
            1
        );
        assert!(
            platform_models(&data, "", &json!({"model_id":"two","version":"1"}))
                .unwrap()
                .is_empty()
        );
        assert!(
            platform_models(&data, "", &json!({"model_id":"active","version":"1"}))
                .unwrap()
                .is_empty()
        );
        assert!(platform_models(&data, "one", &json!({"model_id":"two"}))
            .unwrap()
            .is_empty());
    }
    #[test]
    fn malformed_governed_discovery_is_refused_without_projecting_provider_claims() {
        for row in [
            json!({"id":""}),
            json!({"id":"one","version":3}),
            json!({"id":"one","execution_available":"true"}),
            json!({"id":"one","scenario_count":-1}),
            json!({"id":"one","model_fidelity":"qualified"}),
            json!({"id":"one","access":{"tier_locked":"false"}}),
        ] {
            assert!(platform_models(&json!({"models":[row]}), "", &json!({})).is_err());
        }
        assert!(platform_model(&json!({"id":"locked","access":{"tier_locked":true}})).is_err());
        let mut data = json!({"models":[]});
        data["models"] = json!((0..30)
            .map(|i| json!({"id":format!("model-{i}")}))
            .collect::<Vec<_>>());
        assert_eq!(platform_models(&data, "", &json!({})).unwrap().len(), 20);
    }

    fn record(id: &str, executable: bool, fidelity: &str) -> Record {
        Record {
            metadata: json!({"id":id,"version":"1","model_fidelity":fidelity,
                "source_family":{"family_id":"synthetic-family","paper_id":"synthetic-paper",
                "canonical_model":{"model_id":"full","version":"1"}}}),
            model: if executable {
                json!({"states":[]})
            } else {
                Value::Null
            },
            index: json!({}),
            scenarios: if executable {
                BTreeMap::from([("default".into(), json!({}))])
            } else {
                BTreeMap::new()
            },
            archive_hash: if executable {
                "a".repeat(64)
            } else {
                String::new()
            },
            fitting: json!({}),
            source: None,
            precision_profiles: None,
        }
    }
    #[test]
    fn related_full_reduced_and_metadata_records_have_explicit_execution_contracts() {
        let records = vec![
            record("full", true, "full"),
            record("metadata", false, "unrecorded"),
            record("reduced", true, "reduced"),
        ];
        let described = describe_all(&records).unwrap();
        assert_eq!(
            described[0].execution_available,
            ExecutionAvailability::Known(true)
        );
        assert_eq!(
            described[2].execution_available,
            ExecutionAvailability::Known(true)
        );
        assert_eq!(
            described[0].executable_backend,
            Some(Backend::QSPQuestNative)
        );
        assert_eq!(
            described[1].execution_available,
            ExecutionAvailability::Known(false)
        );
        assert_eq!(described[1].executable_backend, None);
        assert_eq!(described[1].representation, Representation::MetadataOnly);
        assert_eq!(described[1].scenario_count, ScenarioCount::Known(0));
        for description in &described {
            assert_eq!(description.source_family.member_count, 3);
            assert!(description.source_family.members_complete);
            assert_eq!(
                description
                    .source_family
                    .declaration
                    .as_ref()
                    .unwrap()
                    .canonical_model
                    .model_id,
                "full"
            );
            assert_eq!(
                description.source_family.full_family_resource,
                described[0].source_family.full_family_resource
            );
        }
        let library = crate::library::Library {
            version: "fixture".into(),
            records,
            availability_counts: Value::Null,
        };
        let search = library.search("", &json!({"model_id":"full","version":"1"}));
        assert_eq!(search["total"], 1);
        assert_eq!(search["models"][0]["discovery"]["fidelity"], "full");
        assert_eq!(
            search["models"][0]["discovery"]["archive_sha256"],
            "a".repeat(64)
        );
        let _: crate::contract::DiscoveryPayload = serde_json::from_value(search).unwrap();
    }
    #[test]
    fn canonical_relationships_and_closed_declarations_are_admitted_atomically() {
        let valid = vec![
            record("full", true, "full"),
            record("metadata", false, "unrecorded"),
        ];
        for change in 0..7 {
            let mut records = valid.clone();
            match change {
                0 => records[0].metadata["version"] = json!("2"),
                1 => records[0].model = Value::Null,
                2 => records[0].metadata["model_fidelity"] = json!("reduced"),
                3 => records[1].metadata["source_family"]["paper_id"] = json!("different-paper"),
                4 => records[1].metadata["source_family"]["extra"] = json!(true),
                5 => records[1].metadata["source_family"]["family_id"] = json!("a".repeat(129)),
                _ => records[1].metadata["model_fidelity"] = json!("unknown"),
            }
            assert!(validate(&records).is_err(), "case {change}");
            assert!(describe_all(&records).is_err(), "case {change}");
        }
        let mut duplicate = valid;
        duplicate.push(duplicate[0].clone());
        assert!(validate(&duplicate).is_err());
    }
    #[test]
    fn bounded_family_previews_keep_a_digest_addressed_complete_inventory() {
        let mut records = vec![record("full", true, "full")];
        records.extend((0..11).map(|i| record(&format!("reduced-{i:02}"), true, "reduced")));
        let description = describe_all(&records).unwrap().remove(0);
        assert_eq!(description.source_family.member_count, 12);
        assert_eq!(description.source_family.members.len(), 8);
        assert!(!description.source_family.members_complete);
        let complete = family_records(&records).unwrap().remove(0);
        assert_eq!(complete.members.len(), 12);
        assert_eq!(
            description.source_family.full_family_resource,
            Some(family_uri(&complete))
        );
        records.reverse();
        assert_eq!(
            family_uri(&family_records(&records).unwrap()[0]),
            family_uri(&complete)
        );
        records.extend((12..33).map(|i| record(&format!("reduced-{i:02}"), true, "reduced")));
        assert!(validate(&records).is_err());
    }
    #[test]
    fn missing_relationships_and_user_source_digests_are_not_fabricated() {
        let mut user = record("user", true, "unrecorded");
        user.metadata
            .as_object_mut()
            .unwrap()
            .remove("source_family");
        user.source = Some("user source".into());
        let mut inspection = record("inspection", false, "unrecorded");
        inspection
            .metadata
            .as_object_mut()
            .unwrap()
            .remove("source_family");
        inspection.archive_hash = "b".repeat(64);
        let described = describe_all(&[user, inspection]).unwrap();
        assert_eq!(described[0].representation, Representation::UserSource);
        assert_eq!(described[0].archive_sha256, None);
        assert_eq!(described[0].source_family.status, FamilyStatus::Unrecorded);
        assert_eq!(described[0].source_family.declaration, None);
        assert_eq!(described[0].source_family.full_family_resource, None);
        assert_eq!(
            described[1].representation,
            Representation::SourceInspection
        );
        assert_eq!(described[1].executable_backend, None);
    }
}

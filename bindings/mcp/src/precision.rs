//! Complete, file-pinned source conventions; native identities remain separate.
use crate::{library::hash, planning::DoseQuantity, policy};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub path: String,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Choices {
    pub cell_line_parameters: String,
    pub molecule_conversion: String,
    pub half_life_constant: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub doi: String,
    pub locators: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub activation_cell_line_group: usize,
    pub killing_cell_line_group: usize,
    pub donor_group: usize,
    pub parameters: BTreeMap<String, DoseQuantity>,
    /// Declared SDK hash, in the manifest's hash_encoding; not a native identity.
    pub parameters_sha256: String,
    pub transformations: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub choices: Choices,
    pub scenarios: BTreeMap<String, Scenario>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub enum ManifestSchema {
    #[serde(rename = "qspquest.precision-profiles/v1")]
    V1,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub enum ReceiptSchema {
    #[serde(rename = "qspquest.mcp-precision-profile-receipt/v1")]
    V1,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: ManifestSchema,
    pub model_id: String,
    pub model_version: String,
    pub archive_sha256: String,
    pub native_model_sha256: String,
    pub default_profile_id: String,
    pub source: Source,
    pub immutable_parameters: BTreeMap<String, DoseQuantity>,
    pub parameter_count: usize,
    pub mutable_parameter_count: usize,
    pub native_half_life_definition_names: Vec<String>,
    pub profiles: Vec<Profile>,
    pub hash_encoding: String,
    pub scope: String,
}
#[derive(Clone, Debug)]
pub struct Profiles {
    pub manifest: Manifest,
    pub file_sha256: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamedChoice {
    pub id: String,
    pub choices: Choices,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub profile_file_sha256: String,
    pub default_profile_id: String,
    pub profiles: Vec<NamedChoice>,
    pub mutable_parameter_count: usize,
    pub scenario_count: usize,
    pub notice: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParameterStatus {
    MatchesProfile,
    Modified,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema: ReceiptSchema,
    pub model_id: String,
    pub model_version: String,
    pub archive_sha256: String,
    pub profile_file_sha256: String,
    pub profile_id: String,
    pub scenario_id: String,
    pub choices: Choices,
    pub parameters_status: ParameterStatus,
    pub changed_parameter_names: Vec<String>,
    pub timed_parameter_changes_present: bool,
    pub profile_parameters_sha256: String,
    pub effective_parameters_sha256: String,
    pub effective_parameters: BTreeMap<String, DoseQuantity>,
    pub immutable_parameters: BTreeMap<String, DoseQuantity>,
    pub transformations: Vec<String>,
    pub source: Source,
    pub hash_encoding: String,
    pub scope: String,
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn same_quantity(a: &DoseQuantity, b: &Value) -> bool {
    b["value"].as_f64() == Some(a.value) && b["unit"].as_str() == Some(a.unit.as_str())
}
impl Profiles {
    pub fn load(
        raw: &[u8],
        descriptor: &Descriptor,
        metadata: &Value,
        model: &Value,
        native_model_hash: &str,
        scenarios: &BTreeMap<String, Value>,
    ) -> Result<Self, String> {
        if raw.len() > 4_000_000
            || !valid_hash(&descriptor.sha256)
            || hash(raw) != descriptor.sha256
        {
            return Err(
                "Precision profile file exceeds 4 MB or differs from catalog SHA-256".into(),
            );
        }
        let manifest: Manifest =
            serde_json::from_slice(raw).map_err(|e| format!("Precision profiles: {e}"))?;
        if manifest.model_id != metadata["id"]
            || manifest.model_version != metadata["version"]
            || manifest.archive_sha256 != metadata["omex_sha256"]
            || manifest.native_model_sha256 != native_model_hash
        {
            return Err("Precision profile model/version/archive/native identity mismatch".into());
        }
        let parameters: BTreeMap<_, _> = model["parameters"]
            .as_array()
            .ok_or("Precision profiles require native parameters")?
            .iter()
            .map(|p| {
                Ok((
                    p["name"]
                        .as_str()
                        .ok_or("Native parameter name missing")?
                        .to_owned(),
                    p,
                ))
            })
            .collect::<Result<_, String>>()?;
        let mutable: BTreeSet<_> = parameters
            .iter()
            .filter(|(_, p)| p["fixed"] != true)
            .map(|(n, _)| n.clone())
            .collect();
        let fixed: BTreeSet<_> = parameters
            .keys()
            .filter(|n| !mutable.contains(*n))
            .cloned()
            .collect();
        if manifest.parameter_count != parameters.len()
            || manifest.mutable_parameter_count != mutable.len()
            || manifest
                .immutable_parameters
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>()
                != fixed
            || manifest
                .immutable_parameters
                .iter()
                .any(|(n, q)| !same_quantity(q, &parameters[n]["default"]))
        {
            return Err("Precision profile native parameter inventory mismatch".into());
        }
        if manifest.profiles.is_empty() || manifest.profiles.len() > 64 {
            return Err("Precision profiles require 1-64 complete named conventions".into());
        }
        let mut names = BTreeSet::new();
        for profile in &manifest.profiles {
            if profile.id.is_empty() || profile.id.len() > 128 || !names.insert(profile.id.clone())
            {
                return Err(
                    "Precision profile identifiers must be unique nonempty strings under 129 bytes"
                        .into(),
                );
            }
            if profile.scenarios.keys().collect::<Vec<_>>() != scenarios.keys().collect::<Vec<_>>()
            {
                return Err("Precision profile must cover every archived scenario".into());
            }
            for (sid, entry) in &profile.scenarios {
                if entry.parameters.keys().cloned().collect::<BTreeSet<_>>() != mutable
                    || !valid_hash(&entry.parameters_sha256)
                {
                    return Err("Precision profile requires every mutable parameter and its declared SDK digest".into());
                }
                for (name, q) in &entry.parameters {
                    let p = parameters[name];
                    if !q.value.is_finite() || p["default"]["unit"] != q.unit {
                        return Err(format!(
                            "Precision profile quantity differs from native unit: {name}"
                        ));
                    }
                    if let Some(bounds) = p["bounds"].as_array() {
                        if bounds.len() != 2
                            || bounds.iter().any(|b| b["unit"] != q.unit)
                            || bounds[0]["value"].as_f64().is_none_or(|v| q.value < v)
                            || bounds[1]["value"].as_f64().is_none_or(|v| q.value > v)
                        {
                            return Err(format!(
                                "Precision profile parameter outside native bounds: {name}"
                            ));
                        }
                    }
                    if profile.id == manifest.default_profile_id {
                        let base = scenarios[sid]["parameters"]
                            .get(name)
                            .unwrap_or(&p["default"]);
                        if !same_quantity(q, base) {
                            return Err(
                                "Precision profile default differs from archived scenario".into()
                            );
                        }
                    }
                }
            }
        }
        if !names.contains(&manifest.default_profile_id) {
            return Err("Precision profile default is unavailable".into());
        }
        Ok(Self {
            manifest,
            file_sha256: descriptor.sha256.clone(),
        })
    }
    pub fn summary(&self) -> Summary {
        Summary {profile_file_sha256:self.file_sha256.clone(),default_profile_id:self.manifest.default_profile_id.clone(),
            profiles:self.manifest.profiles.iter().map(|p| NamedChoice{id:p.id.clone(),choices:p.choices.clone()}).collect(),
            mutable_parameter_count:self.manifest.mutable_parameter_count,
            scenario_count:self.manifest.profiles[0].scenarios.len(),
            notice:"Choose scenario.precision_profile_id to replace every mutable scalar input with one complete source convention. Later scalar or timed overrides are labelled modified. Initial states, doses and protocol remain separate; rounding differences do not establish failed biology or scientific qualification.".into()}
    }
    fn select(&self, sid: &str, id: Option<&str>) -> Result<(&Profile, &Scenario), Value> {
        let id = id.unwrap_or(&self.manifest.default_profile_id);
        let p=self.manifest.profiles.iter().find(|p|p.id==id).ok_or_else(||policy::error("precision_profile_unknown","Unknown precision profile. Inspect get_model.precision_profiles and select an exact profile_id."))?;
        Ok((
            p,
            p.scenarios.get(sid).ok_or_else(|| {
                policy::error(
                    "precision_profile_scenario",
                    "The selected precision profile has no such scenario.",
                )
            })?,
        ))
    }
    pub fn apply(&self, sid: &str, id: Option<&str>, request: &mut Value) -> Result<(), Value> {
        let (_, entry) = self.select(sid, id)?;
        request["parameters"] = json!(entry.parameters);
        Ok(())
    }
    pub fn receipt(
        &self,
        sid: &str,
        id: Option<&str>,
        request: &Value,
        model: &Value,
        timed_overrides_present: bool,
    ) -> Result<Receipt, Value> {
        let (p, entry) = self.select(sid, id)?;
        for (name, declared) in &self.manifest.immutable_parameters {
            if request["parameters"]
                .get(name)
                .is_some_and(|q| !same_quantity(declared, q))
            {
                return Err(policy::error("precision_profile_immutable", format!(
                    "Precision profile fixed input {name} must retain its declared default. Select a model without this profile contract for a different fixed convention; inspect the complete native inputs before execution."
                )));
            }
        }
        let mut effective = BTreeMap::new();
        for row in model["parameters"].as_array().ok_or_else(|| {
            policy::error("precision_profile_contract", "Native parameters missing")
        })? {
            if row["fixed"] == true {
                continue;
            }
            let name = row["name"].as_str().ok_or_else(|| {
                policy::error("precision_profile_contract", "Native name missing")
            })?;
            let q = request["parameters"].get(name).unwrap_or(&row["default"]);
            effective.insert(
                name.to_owned(),
                serde_json::from_value::<DoseQuantity>(q.clone())
                    .map_err(|e| policy::error("precision_profile_contract", e.to_string()))?,
            );
        }
        let changed: Vec<_> = entry
            .parameters
            .iter()
            .filter(|(name, q)| {
                let actual = &effective[*name];
                actual.value != q.value || actual.unit != q.unit
            })
            .map(|(n, _)| n.clone())
            .collect();
        let timed = timed_overrides_present
            || request["parameter_changes"]
                .as_array()
                .is_some_and(|v| !v.is_empty());
        let digest = |values: &BTreeMap<String, DoseQuantity>| {
            qspquest_core::identity::canonical_hash(&json!(values))
                .map_err(|e| policy::error("precision_profile_contract", e.to_string()))
        };
        Ok(Receipt {schema:ReceiptSchema::V1,model_id:self.manifest.model_id.clone(),
            model_version:self.manifest.model_version.clone(),archive_sha256:self.manifest.archive_sha256.clone(),
            profile_file_sha256:self.file_sha256.clone(),profile_id:p.id.clone(),scenario_id:sid.into(),choices:p.choices.clone(),
            parameters_status:if changed.is_empty() && !timed {ParameterStatus::MatchesProfile}else{ParameterStatus::Modified},
            changed_parameter_names:changed,timed_parameter_changes_present:timed,profile_parameters_sha256:digest(&entry.parameters)?,
            effective_parameters_sha256:digest(&effective)?,effective_parameters:effective,immutable_parameters:self.manifest.immutable_parameters.clone(),
            transformations:entry.transformations.clone(),source:self.manifest.source.clone(),
            hash_encoding:"Parameter hashes use native RFC 8785 canonical JSON (sha256: prefix). Profile file hash pins exact bytes. Manifest-declared SDK hashes use their separately declared encoding and are not native identities.".into(),
            scope:"Complete starting scalar parameter convention only. States, doses, covariates, solver protocol and active native binding phases are separate. Modified inputs do not claim the named whole profile. Rounding differences do not establish failed biology or scientific qualification.".into()})
    }
}

#[cfg(test)]
pub(crate) fn synthetic_library() -> crate::library::Library {
    let source = include_str!("../assets/templates/iv.qsp");
    let model = crate::core::compute::execute(source, "inspect_model", json!({})).unwrap();
    let mut base = crate::core::requests::run(&json!({"end":{"value":12,"unit":"h"},
        "times":[{"value":1,"unit":"h"},{"value":12,"unit":"h"}],
        "doses":[{"target":"central","amount":{"value":12,"unit":"mg"},"at":{"value":0,"unit":"h"}}]}),None,&model).unwrap();
    base["regimen"]["rtol"] = json!(1e-10);
    base["regimen"]["atol"] = json!(1e-12);
    let values: serde_json::Map<String, Value> = model["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["name"].as_str().unwrap().into(), p["default"].clone()))
        .collect();
    base["parameters"] = json!(values);
    let metadata = json!({"id":model["model_id"],"version":"1.0.0","omex_sha256":"a".repeat(64),
        "name":"Synthetic complete convention fixture","candidate":false,"execution_available":true,
        "fit_ready":false,"fitting_supported":false,"scenario_ids":["baseline"],"license":"user supplied"});
    let mut alternate = values.clone();
    alternate["cl"]["value"] = json!(0.6);
    let entry = |values| {
        json!({"activation_cell_line_group":1,"killing_cell_line_group":1,"donor_group":1,
        "parameters":values,"parameters_sha256":"b".repeat(64),"transformations":[]})
    };
    let choice = json!({"cell_line_parameters":"synthetic","molecule_conversion":"synthetic","half_life_constant":"synthetic"});
    let native_hash = hash(model.to_string().as_bytes());
    let manifest = json!({"schema":"qspquest.precision-profiles/v1","model_id":model["model_id"],"model_version":"1.0.0",
        "archive_sha256":"a".repeat(64),"native_model_sha256":native_hash,"default_profile_id":"baseline",
        "source":{"doi":"synthetic contract fixture","locators":[]},"immutable_parameters":{},
        "parameter_count":2,"mutable_parameter_count":2,"native_half_life_definition_names":[],
        "profiles":[{"id":"baseline","choices":choice,"scenarios":{"baseline":entry(values)}},
                    {"id":"alternative","choices":choice,"scenarios":{"baseline":entry(alternate)}}],
        "hash_encoding":"Declared synthetic fixture digest; native receipt identity is independently computed.","scope":"Synthetic contract fixture only"});
    let raw = serde_json::to_vec(&manifest).unwrap();
    let descriptor = Descriptor {
        path: "profiles.json".into(),
        sha256: hash(&raw),
    };
    let scenarios = BTreeMap::from([("baseline".into(), base)]);
    let profiles = Profiles::load(
        &raw,
        &descriptor,
        &metadata,
        &model,
        &native_hash,
        &scenarios,
    )
    .unwrap();
    let record = crate::library::Record {
        metadata,
        model,
        index: json!({"native_execution":{}}),
        scenarios,
        archive_hash: "a".repeat(64),
        fitting: json!({"supported":false}),
        source: None,
        precision_profiles: Some(std::sync::Arc::new(profiles)),
    };
    crate::library::Library {
        version: "synthetic_precision_library".into(),
        records: vec![record],
        availability_counts: Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_default_and_alternative_contracts_and_invalid_files() {
        let library = synthetic_library();
        let r = &library.records[0];
        let profiles = r.precision_profiles.as_ref().unwrap();
        let mut request = r.scenarios["baseline"].clone();
        profiles
            .apply("baseline", Some("alternative"), &mut request)
            .unwrap();
        assert_eq!(request["parameters"]["cl"]["value"], 0.6);
        assert_eq!(request["regimen"], r.scenarios["baseline"]["regimen"]);
        let receipt = profiles
            .receipt("baseline", Some("alternative"), &request, &r.model, false)
            .unwrap();
        assert!(matches!(
            receipt.parameters_status,
            ParameterStatus::MatchesProfile
        ));
        assert_eq!(
            receipt.profile_parameters_sha256,
            receipt.effective_parameters_sha256
        );
        assert_eq!(receipt.effective_parameters.len(), 2);
        request["parameters"]["cl"]["value"] = json!(1.9);
        let changed = profiles
            .receipt("baseline", Some("alternative"), &request, &r.model, false)
            .unwrap();
        assert!(matches!(
            changed.parameters_status,
            ParameterStatus::Modified
        ));
        assert_eq!(changed.changed_parameter_names, vec!["cl"]);
        for error in [
            "digest",
            "version",
            "unit",
            "partial",
            "default",
            "duplicate",
            "unknown",
        ] {
            let mut manifest = json!(profiles.manifest);
            match error {
                "version" => manifest["model_version"] = json!("2.0.0"),
                "unit" => {
                    manifest["profiles"][1]["scenarios"]["baseline"]["parameters"]["cl"]["unit"] =
                        json!("1")
                }
                "partial" => {
                    manifest["profiles"][1]["scenarios"]["baseline"]["parameters"]
                        .as_object_mut()
                        .unwrap()
                        .remove("cl");
                }
                "default" => {
                    manifest["profiles"][0]["scenarios"]["baseline"]["parameters"]["cl"]["value"] =
                        json!(7)
                }
                "duplicate" => {
                    let row = manifest["profiles"][0].clone();
                    manifest["profiles"].as_array_mut().unwrap().push(row);
                }
                "unknown" => manifest["ignored_field"] = json!(true),
                _ => {}
            }
            let raw = serde_json::to_vec(&manifest).unwrap();
            let descriptor = Descriptor {
                path: "profiles.json".into(),
                sha256: if error == "digest" {
                    "0".repeat(64)
                } else {
                    hash(&raw)
                },
            };
            assert!(
                Profiles::load(
                    &raw,
                    &descriptor,
                    &r.metadata,
                    &r.model,
                    &profiles.manifest.native_model_sha256,
                    &r.scenarios
                )
                .is_err(),
                "{error}"
            );
        }
    }
}

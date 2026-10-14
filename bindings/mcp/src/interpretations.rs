//! Identity-bound declarations of selected source conventions, without scientific approval.
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Declaration {
    schema: String,
    model_id: String,
    model_version: String,
    archive_sha256: String,
    native_model_sha256: String,
    coverage: String,
    author_code_qualification: String,
    dimensional_qualification: bool,
    entries: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    source_locator: String,
    rationale: String,
    conservation_implications: String,
    qualification_limits: String,
    printed_coordinates: Vec<String>,
    adopted_coordinates: Vec<String>,
    source_definitions: Vec<String>,
    native_definitions: Vec<String>,
}
fn text(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= 2000 && !value.chars().any(|c| (c as u32) < 32)
}
fn list(values: &[String]) -> bool {
    (1..=32).contains(&values.len())
        && values.iter().all(|value| text(value))
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn state_dependencies(value: &Value, states: &BTreeSet<&str>, found: &mut BTreeSet<String>) {
    match value {
        Value::Object(fields) => {
            if value["kind"] == "symbol" {
                if let Some(name) = value["name"].as_str().filter(|name| states.contains(name)) {
                    found.insert(name.to_owned());
                }
            }
            for value in fields.values() {
                state_dependencies(value, states, found);
            }
        }
        Value::Array(values) => {
            for value in values {
                state_dependencies(value, states, found);
            }
        }
        _ => {}
    }
}
/// Match declared identity and native references to the already verified archive members.
/// Source-page accuracy and author-code fidelity remain separate scientific review work.
pub(crate) fn validate(
    metadata: &Value,
    model: &Value,
    index: &Value,
    archive_hash: &str,
) -> Result<(), String> {
    let Some(raw) = metadata.get("source_interpretations") else {
        return Ok(());
    };
    if raw.to_string().len() > 16000 {
        return Err("Source interpretation declaration exceeds 16 KB".into());
    }
    let declaration: Declaration = serde_json::from_value(raw.clone())
        .map_err(|error| format!("Source interpretation declaration: {error}"))?;
    let native_path = index["native_execution"]["model"]
        .as_str()
        .ok_or("Source interpretations require an executable native model")?;
    if declaration.schema != "qspquest.source-interpretations/v1"
        || Some(declaration.model_id.as_str()) != metadata["id"].as_str()
        || Some(declaration.model_version.as_str()) != metadata["version"].as_str()
        || declaration.archive_sha256 != archive_hash
        || !hash(&declaration.archive_sha256)
        || !hash(&declaration.native_model_sha256)
        || Some(declaration.native_model_sha256.as_str()) != index["members"][native_path].as_str()
    {
        return Err("Source interpretation model/version/archive/native identity mismatch".into());
    }
    if declaration.coverage != "selected_structural_interpretations"
        || declaration.author_code_qualification != "not_assessed"
        || declaration.dimensional_qualification
        || !(1..=32).contains(&declaration.entries.len())
    {
        return Err("Source interpretations must retain selected coverage and unassessed author-code/dimensional qualification".into());
    }
    let definitions: BTreeSet<_> = model["definitions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["name"].as_str())
        .collect();
    let states: BTreeSet<_> = model["states"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["name"].as_str())
        .collect();
    let mut ids = BTreeSet::new();
    for entry in &declaration.entries {
        if ![
            &entry.id,
            &entry.source_locator,
            &entry.rationale,
            &entry.conservation_implications,
            &entry.qualification_limits,
        ]
        .iter()
        .all(|value| text(value))
            || !ids.insert(&entry.id)
            || ![
                &entry.printed_coordinates,
                &entry.adopted_coordinates,
                &entry.source_definitions,
                &entry.native_definitions,
            ]
            .iter()
            .all(|values| list(values))
            || entry.source_definitions.len() != entry.native_definitions.len()
            || entry
                .native_definitions
                .iter()
                .any(|name| !definitions.contains(name.as_str()))
            || entry
                .adopted_coordinates
                .iter()
                .any(|name| index["native_execution"]["states"].get(name).is_none())
        {
            return Err("Source interpretation entry has invalid text, duplicate identifiers or unresolved native definitions/coordinates".into());
        }
        let mut adopted = BTreeSet::new();
        for coordinate in &entry.adopted_coordinates {
            let output = &index["native_execution"]["states"][coordinate]["native_name"];
            let expression = model["outputs"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|row| row["name"] == *output)
                .map(|row| &row["expression"])
                .ok_or(
                    "Source interpretation coordinate must name an actual native state readout",
                )?;
            let name = expression["name"]
                .as_str()
                .filter(|name| states.contains(name))
                .filter(|_| expression["kind"] == "symbol")
                .ok_or("Source interpretation coordinate must resolve to one native state")?;
            adopted.insert(name.to_owned());
        }
        for name in &entry.native_definitions {
            let expression = model["definitions"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|row| row["name"] == name.as_str())
                .map(|row| &row["expression"])
                .ok_or("Source interpretation native definition missing")?;
            let mut used = BTreeSet::new();
            state_dependencies(expression, &states, &mut used);
            if used != adopted {
                return Err("Source interpretation adopted coordinates differ from native equation state dependencies".into());
            }
        }
    }
    Ok(())
}

/// Bounded discovery directs the reader to every complete declared entry, without truncating it.
pub(crate) fn summary(metadata: &Value, details_resource: &str) -> Value {
    let Some(raw) = metadata.get("source_interpretations") else {
        return json!({"status":"not_declared","notice":"No structural interpretation declaration is attached. This does not establish literal source fidelity or absence of reconstruction choices."});
    };
    json!({"status":"declared","coverage":raw["coverage"],"model_id":raw["model_id"],
        "model_version":raw["model_version"],"archive_sha256":raw["archive_sha256"],
        "native_model_sha256":raw["native_model_sha256"],"entry_count":raw["entries"].as_array().map(Vec::len),
        "author_code_qualification":raw["author_code_qualification"],"dimensional_qualification":raw["dimensional_qualification"],
        "complete_declaration_resource":details_resource,"complete_declaration_field":"metadata.source_interpretations",
        "notice":"Inspect all complete entries before choosing a source convention: equation/page locators, printed and adopted coordinates, source/native definitions, rationale, conservation implications and qualification limits. Identity and native-reference admission do not qualify source-page accuracy, author-code agreement, conservation or physical dimensions."})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Value, Value, Value) {
        let metadata = json!({"id":"synthetic","version":"1","source_interpretations":{
            "schema":"qspquest.source-interpretations/v1","model_id":"synthetic","model_version":"1",
            "archive_sha256":"a".repeat(64),"native_model_sha256":"b".repeat(64),
            "coverage":"selected_structural_interpretations","author_code_qualification":"not_assessed","dimensional_qualification":false,
            "entries":[{"id":"locality","source_locator":"Synthetic equation 1, page 1","rationale":"Selected coordinate convention",
                "conservation_implications":"Conservation is unassessed","qualification_limits":"Synthetic declaration only",
                "printed_coordinates":["plasma"],"adopted_coordinates":["lymph"],"source_definitions":["source_binding"],"native_definitions":["binding"]}]}});
        (
            metadata,
            json!({"definitions":[{"name":"binding","expression":{"kind":"symbol","name":"lymph_state"}}],"states":[{"name":"lymph_state"},{"name":"plasma_state"}],"outputs":[{"name":"lymph_readout","expression":{"kind":"symbol","name":"lymph_state"}}]}),
            json!({"native_execution":{"model":"native/model.json","states":{"lymph":{"native_name":"lymph_readout"}}},"members":{"native/model.json":"b".repeat(64)}}),
        )
    }
    #[test]
    fn exact_identity_and_native_references_admit_without_rewriting_metadata() {
        let (metadata, model, index) = fixture();
        let before = metadata.clone();
        validate(&metadata, &model, &index, &"a".repeat(64)).unwrap();
        assert_eq!(metadata, before);
    }
    #[test]
    fn stale_versions_hashes_and_absent_native_references_refuse() {
        for (path, value) in [
            ("/source_interpretations/model_version", json!("2")),
            (
                "/source_interpretations/archive_sha256",
                json!("c".repeat(64)),
            ),
            (
                "/source_interpretations/native_model_sha256",
                json!("c".repeat(64)),
            ),
            (
                "/source_interpretations/entries/0/native_definitions/0",
                json!("missing"),
            ),
            (
                "/source_interpretations/entries/0/adopted_coordinates/0",
                json!("missing"),
            ),
        ] {
            let (mut metadata, model, index) = fixture();
            *metadata.pointer_mut(path).unwrap() = value;
            assert!(validate(&metadata, &model, &index, &"a".repeat(64)).is_err());
        }
    }
    #[test]
    fn declaration_cannot_promote_qualification_or_omit_conservation_limits() {
        for (path, value) in [
            (
                "/source_interpretations/dimensional_qualification",
                json!(true),
            ),
            (
                "/source_interpretations/author_code_qualification",
                json!("passed"),
            ),
            ("/source_interpretations/coverage", json!("complete")),
            (
                "/source_interpretations/entries/0/conservation_implications",
                json!(""),
            ),
        ] {
            let (mut metadata, model, index) = fixture();
            *metadata.pointer_mut(path).unwrap() = value;
            assert!(validate(&metadata, &model, &index, &"a".repeat(64)).is_err());
        }
    }
    #[test]
    fn adopted_coordinates_must_match_the_actual_equation_and_readout() {
        let (metadata, mut model, index) = fixture();
        model["definitions"][0]["expression"]["name"] = json!("plasma_state");
        assert!(validate(&metadata, &model, &index, &"a".repeat(64)).is_err());
        model["definitions"][0]["expression"]["name"] = json!("lymph_state");
        model["outputs"][0]["expression"]["name"] = json!("plasma_state");
        assert!(validate(&metadata, &model, &index, &"a".repeat(64)).is_err());
    }
    #[test]
    fn summary_is_bounded_and_points_to_complete_entries_without_changing_them() {
        let (metadata, _, _) = fixture();
        let result = summary(&metadata, "qspquest://models/synthetic/details/1");
        assert_eq!(result["entry_count"], 1);
        assert_eq!(result["native_model_sha256"], "b".repeat(64));
        assert_eq!(
            result["complete_declaration_field"],
            "metadata.source_interpretations"
        );
        assert!(!result
            .to_string()
            .contains("Selected coordinate convention"));
        assert_eq!(summary(&json!({}), "unused")["status"], "not_declared");
    }
}

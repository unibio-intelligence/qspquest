//! Model inspection responses built from verified library records.
use crate::{core::usability, policy};
use serde_json::{json, Value};

pub(crate) fn model_details(
    r: &crate::library::Record,
    library: &crate::library::Library,
) -> Value {
    let discovery = match crate::discovery::describe_all(&library.records) {
        Ok(records) => records
            .into_iter()
            .zip(&library.records)
            .find(|(_, candidate)| std::ptr::eq(*candidate, r))
            .map(|(discovery, _)| discovery),
        Err(message) => return policy::error("discovery_metadata", message),
    };
    if r.model.is_null() {
        return json!({"metadata":r.metadata,"execution_available":false,"scenario_ids":[],"discovery":discovery});
    }
    let source = r.source_text();
    let states = crate::library::display_states(&r.model);
    let mut v = json!({"metadata":r.metadata,"parameters":r.model["parameters"],"covariates":r.model["covariates"],"states":states,"outputs":r.model["outputs"],"time_unit":r.model["time_unit"],"dose_targets":states.iter().filter(|st|!st["dosing"].is_null()).map(|st|json!({"target":st["name"],"amount_unit":st["dosing"]["amount_unit"],"modes":st["dosing"]["modes"]})).collect::<Vec<_>>(),"requirements":r.model["requirements"],"scenario_ids":r.scenarios.keys().collect::<Vec<_>>(),"archive_sha256":r.archive_hash,"native_execution":r.index["native_execution"],"source_citation":r.index["source_citation"],"fitting":r.fitting});
    v["discovery"] = json!(discovery);
    v["scientific_assessment"] = r.metadata["scientific_assessment_contract"].clone();
    v["source_interpretation_summary"] =
        crate::interpretations::summary(&r.metadata, &model_details_resource(r));
    v["protocol_initialization"] = r.metadata["protocol_initialization_contract"].clone();
    if let Some(profiles) = &r.precision_profiles {
        v["precision_profiles"] = json!(profiles.summary());
        v["precision_profile_manifest"] = json!(profiles.manifest);
    }
    let named = usability::augment(&r.index["native_execution"], &r.model);
    // One stored dose per target, so the caller sees the magnitude the model is used with.
    let mut examples = serde_json::Map::new();
    for (sid, request) in r.scenarios.iter() {
        for a in request["regimen"]["administrations"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(target) = a["target"].as_str() {
                examples.entry(target.to_owned()).or_insert_with(|| json!({"scenario":sid,"amount":a["amount"]["value"],"note":"the number a stored scenario doses, in the unit the model expects"}));
            }
        }
    }
    let card = usability::interface(
        &r.model,
        &named,
        &v["scenario_ids"],
        &Value::Object(examples),
    );
    v["_text"] = json!(usability::ordered(&[
        ("model_id", &r.metadata["id"]),
        ("name", &r.metadata["name"]),
        ("description", &r.model["description"]),
        ("time_unit", &card["time_unit"]),
        ("how_to_simulate", &card["how_to_simulate"]),
        ("covariates", &card["covariates"]),
        ("dose_targets", &card["dose_targets"]),
        ("parameters", &card["parameters"]),
        ("outputs", &card["outputs"]),
        ("states", &card["states"]),
        ("scenario_ids", &card["scenario_ids"]),
        ("precision_profiles", &v["precision_profiles"]),
        ("protocol_initialization", &v["protocol_initialization"]),
        ("scientific_assessment", &v["scientific_assessment"]),
        ("source_interpretation_summary", &v["source_interpretation_summary"]),
        ("equations", &usability::equations(&r.model, &named)),
        (
            "hint",
            &json!(
                "Next: get_patient_parameters for derived clearance and volume, or simulate with doses, parameters, times and readouts. Names and units are the ones listed here."
            )
        ),
        (
            "reported_values",
            &json!({"count":card["reported_values"]["count"],"note":card["reported_values"]["note"],"first_names":card["reported_values"]["names"].as_array().map(|a| a.iter().take(25).cloned().collect::<Vec<_>>())})
        ),
        ("dois", &r.metadata["dois"]),
        ("version", &r.metadata["version"]),
        ("execution_available", &r.metadata["execution_available"]),
        (
            "more",
            &json!("structuredContent carries the full record, including native names.")
        ),
    ]));
    v["equations"] = usability::equations(&r.model, &named);
    v["interface"] = card;
    if source.len() < 64_000 {
        v["source"] = json!(source)
    } else {
        v["source_resource"] = json!(format!(
            "qspquest://models/{}",
            r.metadata["id"].as_str().unwrap_or("")
        ))
    }
    v
}

pub(crate) fn model_details_resource(r: &crate::library::Record) -> String {
    format!(
        "qspquest://models/{}/details/{}",
        r.metadata["id"].as_str().unwrap_or(""),
        r.metadata["version"].as_str().unwrap_or("")
    )
}

pub(crate) fn compact_model_card(r: &crate::library::Record, details: &Value) -> Value {
    let mut metadata = serde_json::Map::new();
    for key in [
        "id",
        "name",
        "version",
        "dois",
        "execution_available",
        "license",
        "candidate",
        "source_rights",
        "execution_blocker",
        "limitations",
        "availability",
        "availability_record_status",
    ] {
        if let Some(value) = r.metadata.get(key) {
            metadata.insert(key.into(), value.clone());
        }
    }
    let mut interface = details["interface"].clone();
    if let Some(names) = interface["reported_values"]["names"].as_array() {
        let preview: Vec<Value> = names.iter().take(25).cloned().collect();
        interface["reported_values"]["names"] = json!(preview);
        interface["reported_values"]["names_notice"] =
            json!("First 25 names only; full details contain the complete list.");
    }
    let mut card = json!({"metadata":metadata,"detail":"compact","scientific_assessment":details["scientific_assessment"],"precision_profiles":details["precision_profiles"],"protocol_initialization":details["protocol_initialization"],"execution_available":!r.model.is_null(),"discovery":details["discovery"],"time_unit":details["time_unit"],"interface":interface,"requirements":details["requirements"],"source_citation":details["source_citation"],"archive_sha256":r.archive_hash,"fitting":details["fitting"],"scenario_ids":r.scenarios.keys().collect::<Vec<_>>(),"descriptive_tags":details["descriptive_tags"],"what_you_can_ask":details["what_you_can_ask"],"full_model_details_resource":model_details_resource(r),"hint":"Use interface names and units for simulation. Request detail full for equations, native names and complete metadata, or read full_model_details_resource. Retrieve a stored scenario separately with get_scenario_request."});
    card["source_interpretation_summary"] =
        crate::interpretations::summary(&r.metadata, &model_details_resource(r));
    card
}

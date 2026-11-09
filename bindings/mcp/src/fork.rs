//! Readable, editable copies of library models.
//!
//! Library archives execute a generated native projection whose symbols are
//! often opaque (`source_symbol_N`). Their readable names live in the archive's
//! `native_execution` tables and are not always legal model identifiers. A fork
//! renames every symbol to a legal readable identifier, leaves the mathematics
//! unchanged, and translates a stored scenario into the same names so that the
//! copy can be checked against the original before it is edited.
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub use crate::edit::{apply_renames, legal_identifier};
use crate::edit::{rename_string, MAX_IDENTIFIER, SYMBOL_SECTIONS};

/// One renamed symbol, reported so that callers can map archive evidence onto the copy.
fn mapping_row(kind: &str, native: &str, readable: &str, reason: &str, model: &Value) -> Value {
    let declaration = model[match kind {
        "state" => "states",
        "output" => "outputs",
        "covariate" => "covariates",
        "individual" => "individual",
        "definition" => "definitions",
        _ => "parameters",
    }]
    .as_array()
    .into_iter()
    .flatten()
    .find(|d| d["name"] == native);
    let unit = declaration
        .map(|d| {
            d.get("default")
                .and_then(|q| q.get("unit"))
                .or_else(|| d.get("unit"))
                .cloned()
                .unwrap_or(Value::Null)
        })
        .unwrap_or(Value::Null);
    json!({"kind":kind,"native":native,"readable":readable,"unit":unit,"reason":reason,
        "description":declaration.and_then(|d| d.get("description")).cloned().unwrap_or(Value::Null)})
}

/// Generated archive symbols that carry no meaning of their own.
fn is_opaque(name: &str) -> bool {
    [
        "source_symbol_",
        "source_output_",
        "library_state_",
        "legacy_unit_basis_",
    ]
    .iter()
    .any(|prefix| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| rest.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// A short identifier from the first words of a declaration's description, for archives that
/// declare no readable name: "Typical systemic clearance (L/h)" becomes typical_systemic_clearance.
fn description_name(declaration: &Value) -> Option<String> {
    const STOP: [&str; 12] = [
        "the", "a", "an", "of", "for", "in", "on", "to", "and", "used", "with", "by",
    ];
    let text = declaration["description"].as_str()?;
    let head = text
        .split(['(', '[', ';', ':', '.', ','])
        .next()
        .unwrap_or("");
    let words: Vec<String> = head
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|w| !w.is_empty() && !STOP.contains(&w.as_str()))
        .take(5)
        .collect();
    if words.is_empty() {
        return None;
    }
    let mut name = legal_identifier(&words.join("_"));
    name.truncate(48);
    let name = name.trim_end_matches('_').to_owned();
    (!name.is_empty() && !is_opaque(&name)).then_some(name)
}

fn unique(candidate: String, taken: &mut BTreeSet<String>) -> String {
    if taken.insert(candidate.clone()) {
        return candidate;
    }
    for n in 2.. {
        let suffix = format!("_{n}");
        let mut base = candidate.clone();
        base.truncate(MAX_IDENTIFIER - suffix.len());
        let name = format!("{base}{suffix}");
        if taken.insert(name.clone()) {
            return name;
        }
    }
    unreachable!()
}

/// Library parameter metadata keyed by native name (`parameters.json`, when present).
fn sidecar_entries(sidecar: &Value) -> BTreeMap<String, Value> {
    let mut entries = BTreeMap::new();
    // Every section that shares the model's symbol namespace.
    for section in [
        "parameters",
        "states",
        "covariates",
        "individual",
        "definitions",
        "switches",
        "dose_switches",
        "dose_history",
    ] {
        for entry in sidecar[section].as_array().into_iter().flatten() {
            if let Some(native) = entry["native_name"].as_str() {
                entries.insert(native.to_owned(), entry.clone());
            }
        }
    }
    entries
}

/// Readable names declared by the archive for each native symbol.
fn declared_names(native: &Value) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    for section in ["parameters", "input_states", "covariates"] {
        for (readable, entry) in native[section].as_object().into_iter().flatten() {
            let target = entry["native_name"].as_str().or_else(|| entry.as_str());
            if let Some(target) = target {
                names.entry(target.to_owned()).or_insert(readable.clone());
            }
        }
    }
    names
}

fn declared_output_names(native: &Value) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    for (readable, entry) in native["outputs"].as_object().into_iter().flatten() {
        if let Some(target) = entry["native_name"].as_str().or_else(|| entry.as_str()) {
            names.entry(target.to_owned()).or_insert(readable.clone());
        }
    }
    // Archives also label state columns, for example `C -> library_state_0`.
    for (readable, entry) in native["states"].as_object().into_iter().flatten() {
        if let Some(target) = entry["native_name"].as_str().or_else(|| entry.as_str()) {
            names
                .entry(target.to_owned())
                .or_insert(format!("state_{readable}"));
        }
    }
    names
}

/// Rename every symbol in a native model document. Returns the copy and the map rows.
pub fn readable_model(model: &Value, native: &Value, model_id: &str) -> (Value, Vec<Value>) {
    readable_model_with(model, native, &Value::Null, model_id)
}

/// As `readable_model`, preferring a library sidecar's `readable_name` and annotating each
/// row with its declared role, variability and provenance.
pub fn readable_model_with(
    model: &Value,
    native: &Value,
    sidecar: &Value,
    model_id: &str,
) -> (Value, Vec<Value>) {
    let curated = sidecar_entries(sidecar);
    let mut declared = declared_names(native);
    for (native_name, entry) in &curated {
        if let Some(readable) = entry["readable_name"].as_str() {
            declared.insert(native_name.clone(), readable.to_owned());
        }
    }
    let mut taken = BTreeSet::new();
    let mut symbols = BTreeMap::new();
    let mut rows = Vec::new();
    for section in SYMBOL_SECTIONS {
        let kind = match section {
            "parameters" => "parameter",
            "covariates" => "covariate",
            "states" => "state",
            "individual" => "individual",
            "definitions" => "definition",
            _ => "switch",
        };
        for declaration in model[section].as_array().into_iter().flatten() {
            let Some(native_name) = declaration["name"].as_str() else {
                continue;
            };
            let (preferred, reason) = match declared.get(native_name) {
                Some(readable) if readable == &legal_identifier(readable) => {
                    (readable.clone(), "archive readable name")
                }
                Some(readable) => (
                    legal_identifier(readable),
                    "archive readable name made a legal identifier",
                ),
                None if is_opaque(native_name) => match description_name(declaration) {
                    Some(name) => (name, "derived from the declaration's description"),
                    None => (
                        native_name.to_owned(),
                        "unchanged native name (no archive name or description)",
                    ),
                },
                None if native_name == legal_identifier(native_name) => {
                    (native_name.to_owned(), "unchanged native name")
                }
                None => (legal_identifier(native_name), "native name made legal"),
            };
            let readable = unique(preferred, &mut taken);
            let mut row = mapping_row(kind, native_name, &readable, reason, model);
            if let Some(entry) = curated.get(native_name) {
                row["reason"] = json!("library parameter metadata");
                for key in [
                    "source_name",
                    "role",
                    "variability",
                    "provenance",
                    "classification",
                ] {
                    if let Some(value) = entry.get(key) {
                        row[key] = value.clone();
                    }
                }
            }
            rows.push(row);
            symbols.insert(native_name.to_owned(), readable);
        }
    }
    let mut declared_outputs = declared_output_names(native);
    for entry in sidecar["outputs"].as_array().into_iter().flatten() {
        if let (Some(native_name), Some(readable)) = (
            entry["native_name"].as_str(),
            entry["readable_name"].as_str(),
        ) {
            declared_outputs.insert(native_name.to_owned(), readable.to_owned());
        }
    }
    let mut output_taken = BTreeSet::new();
    let mut outputs = BTreeMap::new();
    for output in model["outputs"].as_array().into_iter().flatten() {
        let Some(native_name) = output["name"].as_str() else {
            continue;
        };
        let preferred = declared_outputs
            .get(native_name)
            .map(|r| legal_identifier(r))
            .unwrap_or_else(|| legal_identifier(native_name));
        let readable = unique(preferred, &mut output_taken);
        rows.push(mapping_row(
            "output",
            native_name,
            &readable,
            if declared_outputs.contains_key(native_name) {
                "archive output label"
            } else {
                "native output name"
            },
            model,
        ));
        outputs.insert(native_name.to_owned(), readable);
    }

    let mut copy = model.clone();
    apply_renames(&mut copy, &symbols, &outputs);
    copy["model_id"] = json!(legal_identifier(&format!("{model_id}_fork")));
    (copy, rows)
}

fn rename_keys(value: &mut Value, symbols: &BTreeMap<String, String>) {
    if let Some(object) = value.as_object_mut() {
        let renamed: Map<String, Value> = std::mem::take(object)
            .into_iter()
            .map(|(k, v)| (symbols.get(&k).cloned().unwrap_or(k), v))
            .collect();
        *object = renamed;
    }
}

/// Translate a stored native request into the fork's names.
pub fn readable_request(request: &Value, rows: &[Value]) -> Value {
    let symbols: BTreeMap<String, String> = rows
        .iter()
        .filter(|r| r["kind"] != "output")
        .filter_map(|r| {
            Some((
                r["native"].as_str()?.to_owned(),
                r["readable"].as_str()?.to_owned(),
            ))
        })
        .collect();
    let mut copy = request.clone();
    for key in ["parameters", "initial_states"] {
        if let Some(map) = copy.get_mut(key) {
            rename_keys(map, &symbols);
        }
    }
    let Some(regimen) = copy.get_mut("regimen") else {
        return copy;
    };
    if let Some(map) = regimen.get_mut("covariates") {
        rename_keys(map, &symbols);
    }
    for key in [
        "administrations",
        "resets",
        "state_changes",
        "covariate_changes",
    ] {
        for event in regimen
            .get_mut(key)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            for field in ["target", "name", "covariate"] {
                if let Some(v) = event.get_mut(field) {
                    rename_string(v, &symbols);
                }
            }
        }
    }
    if let Some(tolerances) = regimen.get_mut("absolute_tolerances") {
        rename_keys(tolerances, &symbols);
    }
    copy
}

fn references(value: &Value, names: &BTreeSet<String>) -> bool {
    match value {
        Value::Object(o) => {
            (o.get("kind").and_then(Value::as_str) == Some("symbol")
                && o.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| names.contains(n)))
                || o.values().any(|v| references(v, names))
        }
        Value::Array(a) => a.iter().any(|v| references(v, names)),
        _ => false,
    }
}

/// Drop parameters whose library role is listed (for example source_check bookkeeping or
/// information criteria) and the outputs that use them. Refuses when a dynamic part of the
/// model still references one, because then the role does not describe its use.
pub fn drop_roles(
    copy: &mut Value,
    rows: &mut Vec<Value>,
    roles: &[String],
) -> Result<Vec<String>, String> {
    let dropped: BTreeSet<String> = rows
        .iter()
        .filter(|r| {
            r["kind"] == "parameter"
                && r["role"]
                    .as_str()
                    .is_some_and(|role| roles.iter().any(|x| x == role))
        })
        .filter_map(|r| r["readable"].as_str().map(str::to_owned))
        .collect();
    if dropped.is_empty() {
        return Ok(Vec::new());
    }
    for section in [
        "states",
        "individual",
        "definitions",
        "switches",
        "dose_switches",
        "linear_pk",
    ] {
        if references(&copy[section], &dropped) {
            return Err(format!("a parameter with an excluded role is used in {section}; the library role does not describe its use, so nothing was dropped"));
        }
    }
    let mut dropped_outputs = Vec::new();
    if let Some(outputs) = copy.get_mut("outputs").and_then(Value::as_array_mut) {
        outputs.retain(|o| {
            let keep = !references(&o["expression"], &dropped);
            if !keep {
                dropped_outputs.push(o["name"].as_str().unwrap_or("").to_owned());
            }
            keep
        });
        if outputs.is_empty() {
            return Err("excluding these roles would remove every output".into());
        }
    }
    if let Some(parameters) = copy.get_mut("parameters").and_then(Value::as_array_mut) {
        parameters.retain(|p| !p["name"].as_str().is_some_and(|n| dropped.contains(n)));
    }
    for row in rows.iter_mut() {
        let name = row["readable"].as_str().unwrap_or("").to_owned();
        if (row["kind"] == "parameter" && dropped.contains(&name))
            || (row["kind"] == "output" && dropped_outputs.contains(&name))
        {
            row["dropped"] = json!(true);
        }
    }
    Ok(dropped_outputs)
}

/// Largest relative difference between two runs, matched by output and row.
pub fn compare_runs(original: &Value, fork: &Value, rows: &[Value]) -> Result<Value, String> {
    let outputs: BTreeMap<&str, &str> = rows
        .iter()
        .filter(|r| r["kind"] == "output")
        .filter_map(|r| Some((r["native"].as_str()?, r["readable"].as_str()?)))
        .collect();
    if original["times"] != fork["times"] || original["sides"] != fork["sides"] {
        return Err("The fork returned a different time grid or event sides".into());
    }
    let mut worst = 0.0f64;
    let mut compared = 0usize;
    for column in original["outputs"].as_array().into_iter().flatten() {
        let Some(native_name) = column["name"].as_str() else {
            continue;
        };
        let readable = outputs.get(native_name).copied().unwrap_or(native_name);
        if rows
            .iter()
            .any(|r| r["kind"] == "output" && r["readable"] == readable && r["dropped"] == true)
        {
            continue;
        }
        let Some(other) = fork["outputs"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| c["name"] == readable)
        else {
            return Err(format!(
                "The fork has no output {readable} for {native_name}"
            ));
        };
        for (a, b) in column["values"]
            .as_array()
            .into_iter()
            .flatten()
            .zip(other["values"].as_array().into_iter().flatten())
        {
            let (Some(a), Some(b)) = (a.as_f64(), b.as_f64()) else {
                continue;
            };
            worst = worst.max((a - b).abs() / a.abs().max(b.abs()).max(1e-300));
            compared += 1;
        }
    }
    Ok(json!({"compared_values":compared,"max_relative_difference":worst}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_names_without_archive_names_come_from_descriptions() {
        let model = json!({"parameters":[{"name":"source_symbol_5","description":"Typical systemic clearance (L/h) of the antibody"},
            {"name":"source_symbol_6","description":""}]});
        let (_, rows) = readable_model(&model, &json!({}), "m");
        assert_eq!(rows[0]["readable"], "typical_systemic_clearance");
        assert_eq!(rows[1]["readable"], "source_symbol_6");
    }

    #[test]
    fn identifiers_are_made_legal_and_unique() {
        assert_eq!(legal_identifier("V_l"), "v_l");
        assert_eq!(legal_identifier("C"), "c");
        assert_eq!(legal_identifier("L0_cells_mm3"), "l0_cells_mm3");
        assert_eq!(
            legal_identifier("kdeg1 (mm3/cell/day)"),
            "kdeg1_mm3_cell_day"
        );
        assert_eq!(legal_identifier("bodyWeight"), "body_weight");
        assert_eq!(legal_identifier("2nd"), "p_2nd");
        assert_eq!(legal_identifier("time"), "time_value");
        assert_eq!(legal_identifier("input_dose"), "input_dose_value");
        let mut taken = BTreeSet::new();
        assert_eq!(unique("c".into(), &mut taken), "c");
        assert_eq!(unique("c".into(), &mut taken), "c_2");
    }

    #[test]
    fn library_metadata_supplies_names_roles_and_provenance() {
        let model = json!({"schema":"qspquest.model/v0.1","model_id":"m","time_unit":"d",
            "parameters":[{"name":"source_symbol_1","default":{"value":1.0,"unit":"1/d"},"description":"k","fixed":false,"transform":"identity"},
                          {"name":"source_symbol_2","default":{"value":0.5,"unit":"1"},"description":"sd","fixed":false,"transform":"identity"}],
            "states":[{"name":"source_symbol_0","unit":"mg","initial":{"kind":"quantity","quantity":{"value":0.0,"unit":"mg"}},
                "rhs":{"kind":"binary","operator":"multiply","arguments":[{"kind":"unary","operator":"negative","arguments":[{"kind":"symbol","name":"source_symbol_1"}]},{"kind":"symbol","name":"source_symbol_0"}]},"dosing":null}],
            "outputs":[{"name":"source_output_0","unit":"mg","expression":{"kind":"symbol","name":"source_symbol_0"}}]});
        let sidecar = json!({"parameters":[
                {"native_name":"source_symbol_1","readable_name":"kel_day","role":"structural","provenance":{"status":"estimated"}},
                {"native_name":"source_symbol_2","readable_name":"omega_kel","role":"iiv_sd","variability":{"distribution":"log_normal","target":"kel_day","scale":"sd"}}],
            "states":[{"native_name":"source_symbol_0","readable_name":"c"}],
            "outputs":[{"native_name":"source_output_0","readable_name":"amount_mg"}]});
        let (copy, rows) = readable_model_with(&model, &json!({}), &sidecar, "m");
        assert_eq!(copy["parameters"][0]["name"], "kel_day");
        assert_eq!(copy["states"][0]["name"], "c");
        assert_eq!(copy["outputs"][0]["name"], "amount_mg");
        let sd = rows.iter().find(|r| r["readable"] == "omega_kel").unwrap();
        assert_eq!(sd["role"], "iiv_sd");
        assert_eq!(sd["variability"]["target"], "kel_day");
    }

    #[test]
    fn renaming_preserves_inputs_functions_and_string_references() {
        let model = json!({
            "schema":"qspquest.model/v0.1","model_id":"m","time_unit":"d",
            "parameters":[{"name":"source_symbol_1","default":{"value":1.0,"unit":"1/d"},"description":"k","fixed":false,"transform":"identity"}],
            "states":[{"name":"source_symbol_0","unit":"mg","initial":{"kind":"quantity","quantity":{"value":0.0,"unit":"mg"}},
                "rhs":{"kind":"binary","operator":"subtract","arguments":[{"kind":"symbol","name":"input_source_symbol_0"},
                    {"kind":"call","name":"max","arguments":[{"kind":"symbol","name":"source_symbol_1"},{"kind":"symbol","name":"source_symbol_0"}]}]},
                "dosing":null}],
            "outputs":[{"name":"source_output_0","unit":"mg","expression":{"kind":"symbol","name":"source_symbol_0"}}],
            "invariants":[{"kind":"nonnegative","states":["source_symbol_0"]}]
        });
        let native = json!({"parameters":{"K_el":{"native_name":"source_symbol_1"}},
            "input_states":{"A":{"native_name":"source_symbol_0"}},
            "outputs":{"amount_mg":{"native_name":"source_output_0"}}});
        let (copy, rows) = readable_model(&model, &native, "m");
        assert_eq!(copy["parameters"][0]["name"], "k_el");
        assert_eq!(copy["states"][0]["name"], "a");
        assert_eq!(copy["states"][0]["rhs"]["arguments"][0]["name"], "input_a");
        assert_eq!(copy["states"][0]["rhs"]["arguments"][1]["name"], "max");
        assert_eq!(
            copy["states"][0]["rhs"]["arguments"][1]["arguments"][0]["name"],
            "k_el"
        );
        assert_eq!(copy["outputs"][0]["name"], "amount_mg");
        assert_eq!(copy["invariants"][0]["states"][0], "a");
        assert_eq!(copy["model_id"], "m_fork");
        assert_eq!(rows.len(), 3);
        let request = json!({"parameters":{"source_symbol_1":{"value":2.0,"unit":"1/d"}},
            "initial_states":{"source_symbol_0":{"value":1.0,"unit":"mg"}},
            "regimen":{"administrations":[{"target":"source_symbol_0"}]}});
        let translated = readable_request(&request, &rows);
        assert!(translated["parameters"]["k_el"].is_object());
        assert!(translated["initial_states"]["a"].is_object());
        assert_eq!(translated["regimen"]["administrations"][0]["target"], "a");
    }
}

//! Exact, event-aware copies of complete simulation columns into requested answer keys.
use crate::{
    contract::{EngineIdentity, OutputMapping},
    library::hash,
    planning::Assessment,
    policy,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SOURCE_BYTES: usize = 128_000_000;

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResultSource {
    Retained { uri: String },
    Supplied { json: String, sha256: String },
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamedResult {
    pub label: String,
    pub source: ResultSource,
    pub expected_plan_hash: Option<String>,
    pub expected_effective_request_hash: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResponseEventSide {
    Pre,
    Post,
    Both,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordedSide {
    Pre,
    Post,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResponseField {
    pub key: String,
    pub run: String,
    /// Exact returned column; use the simulation's output_mapping for request aliases.
    pub column: String,
    pub unit: String,
    pub time_unit: String,
    pub times: Vec<f64>,
    pub event_side: ResponseEventSide,
    pub expected_analyte: Option<String>,
    pub expected_compartment: Option<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormatRequest {
    pub runs: Vec<NamedResult>,
    pub fields: Vec<ResponseField>,
    /// Exact requested identifier fields, such as case_id or decision_id.
    #[serde(default)]
    pub identifiers: BTreeMap<String, String>,
    /// Optional proposed final object. Every requested key and value must match.
    pub candidate: Option<Value>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum AnswerValue {
    Numbers(Vec<f64>),
    Identifier(String),
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceOrigin {
    RetainedRun,
    RetainedArtifact,
    SuppliedPayload,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResponseCheck {
    Constructed,
    CandidateMatches,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub enum GuardVersion {
    #[serde(rename = "qspquest.response-format/v1")]
    V1,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceReceipt {
    pub run: String,
    pub origin: SourceOrigin,
    pub source_uri: Option<String>,
    pub source_json_sha256: String,
    pub source_rows: usize,
    pub execution_identity: Option<EngineIdentity>,
    pub effective_request_hash: Option<String>,
    pub plan_hash: Option<String>,
    pub scientific_assessment: Option<crate::scientific::Receipt>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectedObservation {
    pub time: f64,
    pub side: RecordedSide,
    pub source_row: usize,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldReceipt {
    pub key: String,
    pub run: String,
    pub returned_column: String,
    pub unit: String,
    pub time_unit: String,
    pub requested_event_side: ResponseEventSide,
    pub observations: Vec<SelectedObservation>,
    pub output_mapping: Option<OutputMapping>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResponseGuard {
    pub schema: GuardVersion,
    pub check: ResponseCheck,
    pub identifiers: BTreeMap<String, String>,
    pub sources: Vec<SourceReceipt>,
    pub fields: Vec<FieldReceipt>,
    pub current_engine_and_request: Assessment,
    pub notices: Vec<String>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct FormatPayload {
    pub formatted_response: BTreeMap<String, AnswerValue>,
    pub response_guard: ResponseGuard,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Column {
    name: String,
    unit: String,
    values: Vec<f64>,
}
// Native payloads contain additional provenance and diagnostics. These remain in
// the source digest and URI; only known trajectory fields participate in copying.
#[derive(Deserialize)]
struct Frame {
    schema: String,
    time_unit: String,
    times: Vec<f64>,
    sides: Vec<RecordedSide>,
    outputs: Vec<Column>,
    identity: Option<EngineIdentity>,
    effective_request_hash: Option<String>,
    plan_hash: Option<String>,
    scientific_assessment: Option<crate::scientific::Receipt>,
    #[serde(default)]
    output_mapping: Vec<OutputMapping>,
    trajectory_kind: Option<String>,
    sampling: Option<Value>,
    contract: Option<Value>,
    error: Option<Value>,
    converged: Option<bool>,
}
struct Loaded {
    frame: Frame,
    sha256: String,
    origin: SourceOrigin,
    uri: Option<String>,
}
fn text(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
pub fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn failure(code: &str, at: impl Into<String>, message: impl Into<String>) -> Value {
    let mut e = policy::error(code, message.into());
    e["error"]["expression"] = json!(at.into());
    e
}
fn validate_frame(frame: &Frame, at: &str) -> Result<(), Value> {
    if !matches!(
        frame.schema.as_str(),
        "qspquest.result/v0.1" | "qspquest.mcp-result/v1"
    ) {
        return Err(failure("response_source_schema",at,"Supply a complete native simulation result, or each comparison member's complete result. Plans, fit summaries and model cards are not trajectories."));
    }
    if frame.error.is_some()
        || frame.converged == Some(false)
        || frame
            .contract
            .as_ref()
            .is_some_and(|c| c["status"] != "success" || c["completeness"] != "inline_complete")
        || frame
            .trajectory_kind
            .as_deref()
            .is_some_and(|k| k != "full_requested_grid")
        || frame
            .sampling
            .as_ref()
            .is_some_and(|s| s["original_points"] != s["returned_points"])
    {
        return Err(failure("response_source_incomplete",at,"Use the complete successful result resource. A compact preview, failed/nonconverged result or routed job is not a complete simulation source."));
    }
    if !text(&frame.time_unit) || frame.times.is_empty() || frame.times.len() != frame.sides.len() {
        return Err(failure(
            "response_source_shape",
            at,
            "Source requires a declared clock and equally sized nonempty time/side arrays.",
        ));
    }
    for (i, t) in frame.times.iter().enumerate() {
        if !t.is_finite()
            || i > 0
                && (*t < frame.times[i - 1]
                    || *t == frame.times[i - 1]
                        && (frame.sides[i - 1] != RecordedSide::Pre
                            || frame.sides[i] != RecordedSide::Post
                            || i > 1 && frame.times[i - 2] == *t))
        {
            return Err(failure("response_source_observations",format!("{at}.times[{i}]"),"Times must be finite and ordered; equal-time observations must be exactly one pre/post pair."));
        }
    }
    let mut names = BTreeSet::new();
    for (i, c) in frame.outputs.iter().enumerate() {
        if !text(&c.name)
            || !text(&c.unit)
            || !names.insert(&c.name)
            || c.values.len() != frame.times.len()
            || c.values.iter().any(|v| !v.is_finite())
        {
            return Err(failure("response_source_column",format!("{at}.outputs[{i}]"),"Each source column requires a unique name, declared unit and one finite scalar number for every observation. Event objects and missing values are not scalar arrays."));
        }
    }
    if names.is_empty() {
        return Err(failure(
            "response_source_column",
            at,
            "Source has no computed columns; missing outputs cannot be replaced by numeric zeros.",
        ));
    }
    Ok(())
}
/// `load` reads only a server-retained result URI. Supplied JSON is checked here.
/// The operation copies source scalars; it never solves, interpolates or converts.
pub fn format(
    request: FormatRequest,
    mut load: impl FnMut(&str, usize) -> Result<(String, SourceOrigin), Value>,
) -> Result<FormatPayload, Value> {
    if request.runs.is_empty()
        || request.runs.len() > 32
        || request.fields.is_empty()
        || request.fields.len() > 64
        || request.identifiers.len() > 32
    {
        return Err(failure(
            "response_contract_limit",
            "arguments",
            "Supply 1–32 named results, 1–64 requested fields and at most 32 identifier fields.",
        ));
    }
    let mut keys = BTreeSet::new();
    for (key, value) in &request.identifiers {
        if !text(key) || !text(value) || !keys.insert(key.clone()) {
            return Err(failure(
                "response_identifier",
                "identifiers",
                "Identifier keys and values must be bounded, nonempty text.",
            ));
        }
    }
    let mut referenced = BTreeSet::new();
    for (i, f) in request.fields.iter().enumerate() {
        let mut times = BTreeSet::new();
        if !text(&f.key)
            || !text(&f.run)
            || !text(&f.column)
            || !text(&f.unit)
            || !text(&f.time_unit)
            || !keys.insert(f.key.clone())
            || f.times.is_empty()
            || f.times.len() > 4096
            || f.times.iter().any(|t| !t.is_finite())
            || f.times
                .iter()
                .any(|t| !times.insert(if *t == 0.0 { 0 } else { t.to_bits() }))
            || [&f.expected_analyte, &f.expected_compartment]
                .into_iter()
                .flatten()
                .any(|v| !text(v))
        {
            return Err(failure("response_field_contract",format!("fields[{i}]"),"Provide a unique response key, named run, exact returned column and declared units, an explicit event side, and 1–4096 unique finite requested times. Identifier and array keys must be distinct."));
        }
        referenced.insert(f.run.clone());
    }
    let mut loaded = BTreeMap::new();
    let mut remaining = MAX_SOURCE_BYTES;
    for (i, run) in request.runs.into_iter().enumerate() {
        let at = format!("runs[{i}].source");
        if !text(&run.label) || loaded.contains_key(&run.label) {
            return Err(failure(
                "response_run_label",
                format!("runs[{i}].label"),
                "Supply one source per unique requested run label.",
            ));
        }
        let (raw, origin, uri) = match run.source {
            ResultSource::Retained { uri } => {
                let (raw, origin) = load(&uri, remaining)?;
                (raw, origin, Some(uri))
            }
            ResultSource::Supplied { json, sha256 } => {
                if !digest(&sha256) || hash(json.as_bytes()) != sha256 {
                    return Err(failure("response_source_digest",&at,"Supplied complete JSON must match its exact UTF-8 SHA-256 pin. Digests identify bytes, not their producer."));
                }
                (json, SourceOrigin::SuppliedPayload, None)
            }
        };
        if raw.len() > remaining {
            return Err(failure(
                "response_source_limit",
                &at,
                "Complete result sources exceed the aggregate 128 MB formatting limit. Select fewer sources; no values were formatted.",
            ));
        }
        remaining -= raw.len();
        let frame: Frame = serde_json::from_str(&raw).map_err(|e| {
            failure(
                "response_source_shape",
                &at,
                format!("Complete result contains invalid trajectory fields: {e}"),
            )
        })?;
        validate_frame(&frame, &at)?;
        for (expected, actual, name) in [
            (&run.expected_plan_hash, &frame.plan_hash, "plan_hash"),
            (
                &run.expected_effective_request_hash,
                &frame.effective_request_hash,
                "effective_request_hash",
            ),
        ] {
            if expected
                .as_ref()
                .is_some_and(|v| actual.as_ref() != Some(v))
            {
                return Err(failure("response_source_identity",format!("runs[{i}].expected_{name}"),"The retained/supplied source does not match this named condition's expected request identity. Retrieve its separately computed result."));
            }
        }
        loaded.insert(
            run.label,
            Loaded {
                frame,
                sha256: hash(raw.as_bytes()),
                origin,
                uri,
            },
        );
    }
    if loaded.keys().cloned().collect::<BTreeSet<_>>() != referenced {
        return Err(failure("response_run_completeness","runs","Every requested field needs its named computed result, and every supplied run needs at least one requested field. Missing conditions must remain explicit failures."));
    }
    let mut response: BTreeMap<String, AnswerValue> = request
        .identifiers
        .iter()
        .map(|(k, v)| (k.clone(), AnswerValue::Identifier(v.clone())))
        .collect();
    let mut receipts = Vec::new();
    for (i, f) in request.fields.into_iter().enumerate() {
        let frame = &loaded[&f.run].frame;
        let column=frame.outputs.iter().find(|c| c.name==f.column).ok_or_else(|| failure("response_output_missing",format!("fields[{i}].column"),format!("Column '{}' was not computed for run '{}'. Use an actual returned column; rerun with the missing output rather than supplying zero.",f.column,f.run)))?;
        if f.unit != column.unit || f.time_unit != frame.time_unit {
            return Err(failure("response_declared_units",format!("fields[{i}]"),format!("Source column uses '{}' and clock '{}'; request declared '{}' and '{}'. Formatting copies source coordinates without unit conversion.",column.unit,frame.time_unit,f.unit,f.time_unit)));
        }
        let mapping = frame
            .output_mapping
            .iter()
            .find(|m| m.returned_column == column.name);
        for (wanted, actual, name) in [
            (
                &f.expected_analyte,
                mapping.and_then(|m| m.analyte.as_ref()),
                "analyte",
            ),
            (
                &f.expected_compartment,
                mapping.and_then(|m| m.compartment.as_ref()),
                "compartment",
            ),
        ] {
            if wanted.as_ref().is_some_and(|v| actual != Some(v)) {
                return Err(failure("response_output_semantics",format!("fields[{i}].expected_{name}"),format!("Source does not declare the requested {name} for this returned column. Inspect output_mapping or select a declared output; labels do not establish analyte identity.")));
            }
        }
        let mut observations = Vec::new();
        let mut values = Vec::new();
        for t in &f.times {
            let first = frame.times.partition_point(|v| v < t);
            let end = frame.times.partition_point(|v| v <= t);
            if first == end {
                return Err(failure("response_time_not_computed",format!("fields[{i}].times"),format!("Run '{}' has no observation at {t} {}. Supply an actually computed time or rerun with that observation; no interpolation is performed.",f.run,frame.time_unit)));
            }
            let selected: Vec<usize> = match f.event_side {
                ResponseEventSide::Pre if frame.sides[first] == RecordedSide::Pre => vec![first],
                ResponseEventSide::Post if frame.sides[end - 1] == RecordedSide::Post => vec![end - 1],
                ResponseEventSide::Both if end - first == 2 => (first..end).collect(),
                _ => return Err(failure("response_event_side_missing",format!("fields[{i}].event_side"),format!("Run '{}' has no requested {:?} observation at time {t}. Rerun with the required native observation sides; formatting does not derive one side from the other.",f.run,f.event_side))),
            };
            for row in selected {
                values.push(column.values[row]);
                observations.push(SelectedObservation {
                    time: frame.times[row],
                    side: frame.sides[row],
                    source_row: row,
                });
            }
        }
        response.insert(f.key.clone(), AnswerValue::Numbers(values));
        receipts.push(FieldReceipt {
            key: f.key,
            run: f.run,
            returned_column: column.name.clone(),
            unit: column.unit.clone(),
            time_unit: frame.time_unit.clone(),
            requested_event_side: f.event_side,
            observations,
            output_mapping: mapping.cloned(),
        });
    }
    let expected = serde_json::to_value(&response).unwrap();
    let candidate_provided = request.candidate.is_some();
    if let Some(candidate) = request.candidate {
        let object = candidate.as_object().ok_or_else(|| {
            failure(
                "response_candidate_shape",
                "candidate",
                "Proposed response must be one object with every requested key.",
            )
        })?;
        if object.keys().cloned().collect::<BTreeSet<_>>() != keys {
            return Err(failure("response_candidate_keys","candidate","Proposed response has missing or extra output/identifier keys. Copy every requested key exactly, including all conditions and decision identifiers."));
        }
        for (key, value) in &response {
            let actual = &object[key];
            let matches = match value {
                AnswerValue::Identifier(v) => actual.as_str() == Some(v),
                AnswerValue::Numbers(v) => actual.as_array().is_some_and(|a| {
                    a.len() == v.len()
                        && a.iter()
                            .zip(v)
                            .all(|(x, y)| x.as_f64().is_some_and(|n| n.is_finite() && n == *y))
                }),
            };
            if !matches {
                return Err(failure("response_candidate_value",format!("candidate.{key}"),"Requested identifiers must match exactly; arrays must contain only the exact selected source numbers in the requested order. Event objects, nulls, booleans, guessed zeros, missing rows and values from another condition are refused."));
            }
        }
    }
    let mut notices=vec!["Formatting checks explicit response keys, source columns and requested event sides. It copies reported values without interpolation or unit conversion and does not qualify numerical, physical, biological or clinical conclusions. Every requested side must exist in the source, including at smooth times. No side is inferred from another side. Both sides are included only when event_side is explicitly both and both were computed.".into(),"Supplied JSON is caller-declared data, not proof that this server produced the result. SHA-256 verifies byte identity, not producer authenticity. Inspect every source receipt and its independent assessment scope.".into()];
    let mut digests = BTreeSet::new();
    for data in loaded.values() {
        if !digests.insert(&data.sha256) {
            notices.push("Multiple run labels share identical source bytes. Verify the requested conditions were independently applied; distinct labels alone do not establish a counterfactual.".into());
            break;
        }
    }
    let sources = loaded
        .into_iter()
        .map(|(run, data)| SourceReceipt {
            run,
            origin: data.origin,
            source_uri: data.uri,
            source_json_sha256: data.sha256,
            source_rows: data.frame.times.len(),
            execution_identity: data.frame.identity,
            effective_request_hash: data.frame.effective_request_hash,
            plan_hash: data.frame.plan_hash,
            scientific_assessment: data.frame.scientific_assessment,
        })
        .collect();
    let payload = FormatPayload {
        formatted_response: response,
        response_guard: ResponseGuard {
            schema: GuardVersion::V1,
            check: if candidate_provided {
                ResponseCheck::CandidateMatches
            } else {
                ResponseCheck::Constructed
            },
            identifiers: request.identifiers,
            sources,
            fields: receipts,
            current_engine_and_request: Assessment::NotAssessed,
            notices,
        },
    };
    if serde_json::to_vec(&payload).unwrap().len() > policy::RESPONSE_LIMIT / 3
        || expected.to_string().len() > policy::RESPONSE_LIMIT / 3
    {
        return Err(failure("response_format_output_budget","fields","Formatted response and complete copying receipts exceed the inline budget. Request fewer fields or sample times; the original complete results remain available through their supplied references."));
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Value {
        json!({"schema":"qspquest.result/v0.1","time_unit":"h","times":[0.,0.,2.,2.,4.,4.],"sides":["pre","post","pre","post","pre","post"],"outputs":[{"name":"library_state_0","unit":"mg/L","values":[0.,2.,1.5,3.5,2.5,2.5]}],"trajectory_kind":"full_requested_grid","effective_request_hash":"condition-a"})
    }
    fn request(source: Value, side: &str) -> Value {
        let raw = source.to_string();
        json!({"runs":[{"label":"A","source":{"kind":"supplied","json":raw,"sha256":hash(raw.as_bytes())},"expected_effective_request_hash":"condition-a"}],"fields":[{"key":"A_concentration","run":"A","column":"library_state_0","unit":"mg/L","time_unit":"h","times":[0.,2.,4.],"event_side":side}],"identifiers":{"decision_id":"dose-side"}})
    }
    fn run(value: Value) -> Result<Value, Value> {
        let request: FormatRequest = serde_json::from_value(value).unwrap();
        format(request, |_, _| {
            panic!("Supplied data must not read a resource")
        })
        .map(|r| serde_json::to_value(r).unwrap())
    }
    #[test]
    fn explicit_event_sides_preserve_source_numbers_and_record_exact_rows() {
        for (side, values, rows) in [
            ("pre", json!([0., 1.5, 2.5]), json!([0, 2, 4])),
            ("post", json!([2., 3.5, 2.5]), json!([1, 3, 5])),
            (
                "both",
                json!([0., 2., 1.5, 3.5, 2.5, 2.5]),
                json!([0, 1, 2, 3, 4, 5]),
            ),
        ] {
            let result = run(request(source(), side)).unwrap();
            assert_eq!(result["formatted_response"]["A_concentration"], values);
            let observed: Vec<Value> = result["response_guard"]["fields"][0]["observations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["source_row"].clone())
                .collect();
            assert_eq!(json!(observed), rows);
            assert_eq!(
                result["response_guard"]["current_engine_and_request"],
                "not_assessed"
            );
            assert_eq!(
                result["response_guard"]["sources"][0]["origin"],
                "supplied_payload"
            );
            let mut proposed = request(source(), side);
            proposed["candidate"] = result["formatted_response"].clone();
            assert_eq!(
                run(proposed).unwrap()["response_guard"]["check"],
                "candidate_matches"
            );
        }
    }
    #[test]
    fn a_missing_recorded_side_is_not_inferred_from_the_other_side() {
        for (removed, present, missing) in [(0, "post", "pre"), (1, "pre", "post")] {
            let mut frame = source();
            for key in ["times", "sides"] {
                frame[key].as_array_mut().unwrap().remove(removed);
            }
            frame["outputs"][0]["values"]
                .as_array_mut()
                .unwrap()
                .remove(removed);
            assert!(run(request(frame.clone(), present)).is_ok());
            for side in [missing, "both"] {
                assert_eq!(
                    run(request(frame.clone(), side)).unwrap_err()["error"]["code"],
                    "response_event_side_missing"
                );
            }
        }
    }
    #[test]
    fn missing_conditions_keys_identifiers_and_nonnumeric_event_arrays_are_refused() {
        let base = request(source(), "post");
        let expected = run(base.clone()).unwrap()["formatted_response"].clone();
        let mut bad = base.clone();
        bad["fields"][0]["run"] = json!("B");
        assert_eq!(
            run(bad).unwrap_err()["error"]["code"],
            "response_run_completeness"
        );
        let mut bad = base.clone();
        bad["fields"][0]["column"] = json!("uncomputed_antibody");
        assert_eq!(
            run(bad).unwrap_err()["error"]["code"],
            "response_output_missing"
        );
        for candidate in [
            json!({"A_concentration":[2.,3.5,2.5]}),
            json!({"decision_id":"wrong","A_concentration":[2.,3.5,2.5]}),
            json!({"decision_id":"dose-side","A_concentration":[{"pre_dose":0.,"post_dose":2.},3.5,2.5]}),
            json!({"decision_id":"dose-side","A_concentration":[true,3.5,2.5]}),
            json!({"decision_id":"dose-side","A_concentration":[0.,0.,0.]}),
        ] {
            let mut bad = base.clone();
            bad["candidate"] = candidate;
            assert!(run(bad).is_err());
        }
        let mut good = base;
        good["candidate"] = expected;
        assert!(run(good).is_ok());
    }
    #[test]
    fn source_pins_completeness_units_and_sampling_are_checked_without_repair() {
        let base = request(source(), "post");
        for (field, value, code) in [
            ("unit", json!("ug/mL"), "response_declared_units"),
            ("times", json!([1.]), "response_time_not_computed"),
            (
                "expected_analyte",
                json!("antibody"),
                "response_output_semantics",
            ),
        ] {
            let mut bad = base.clone();
            bad["fields"][0][field] = value;
            assert_eq!(run(bad).unwrap_err()["error"]["code"], code);
        }
        let mut bad = base;
        bad["runs"][0]["source"]["sha256"] = json!("0".repeat(64));
        assert_eq!(
            run(bad).unwrap_err()["error"]["code"],
            "response_source_digest"
        );
        for mutate in ["preview", "failed", "nonconverged", "sides", "nonnumeric"] {
            let mut data = source();
            match mutate {
                "preview" => data["trajectory_kind"] = json!("compact_preview"),
                "failed" => data["error"] = json!({"code":"Domain"}),
                "nonconverged" => data["converged"] = json!(false),
                "sides" => data["sides"][0] = json!("post"),
                _ => data["outputs"][0]["values"][0] = json!({"pre_dose":0.,"post_dose":2.}),
            }
            assert!(run(request(data, "post")).is_err(), "{mutate}");
        }
    }
}

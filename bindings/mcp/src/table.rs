//! CSV analysis bundles with replay inputs and explicit evidence scope.
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{json, Map, Value};

pub const SCHEMA: &str = "qspquest.table-manifest/v1";
pub const SCHEMA_URI: &str = "qspquest://schema/table-manifest";
pub const MAX_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_ROWS: usize = 2_000_000;

#[derive(Deserialize, Serialize, JsonSchema)]
pub enum Version {
    #[serde(rename = "qspquest.table-manifest/v1")]
    V1,
}
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Producer {
    Mcp,
    ModelLibrary,
    Cli,
    Python,
}
#[derive(Clone, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ColumnKind {
    Time,
    EventSide,
    Output,
    State,
    SourceParameter,
}
#[derive(Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub name: String,
    pub unit: Option<String>,
    pub kind: ColumnKind,
    pub native_name: Option<String>,
    pub meaning: Option<String>,
}
#[derive(Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Table {
    pub filename: String,
    pub sha256: String,
    pub bytes: usize,
    pub rows: usize,
    pub columns: Vec<Column>,
}
#[derive(Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Events {
    pub rule: String,
    pub pre: usize,
    pub post: usize,
    pub event_pairs: usize,
}
#[derive(Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub id: Option<String>,
    pub version: Option<String>,
    pub archive_sha256: Option<String>,
    pub provenance: Option<Map<String, Value>>,
}
#[derive(Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Inputs {
    pub effective_request: Option<Map<String, Value>>,
    pub selected_outputs: Option<Vec<String>>,
    pub effective_request_hash: Option<String>,
    pub scenario_id: Option<String>,
    pub modified: Option<bool>,
}
/// This format never transfers recorded archive evidence to a current run.
pub struct NotQualified;
impl Serialize for NotQualified {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(false)
    }
}
impl JsonSchema for NotQualified {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "NotQualified".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        schemars::json_schema!({"type":"boolean", "const":false})
    }
}
#[derive(Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Qualification {
    pub qualified: NotQualified,
    pub status: String,
    pub scope: String,
    pub unit_status: String,
    pub coordinates: Option<String>,
    pub recorded_dimensional_qualification: Option<bool>,
    pub recorded_availability: Option<Map<String, Value>>,
}
#[derive(Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scientific_assessment: Option<crate::scientific::Receipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_boundary_receipt: Option<crate::boundaries::Receipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initialization_receipt: Option<crate::initialization::Receipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose_event_receipts: Option<Vec<crate::planning::DoseEventReceipt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_output_receipts: Option<Vec<crate::core::required_outputs::RequiredOutputReceipt>>,
    pub schema: Version,
    pub producer: Producer,
    pub table: Table,
    pub events: Events,
    pub model: Model,
    pub engine_identity: Option<Map<String, Value>>,
    pub inputs: Inputs,
    pub qualification: Qualification,
}

pub struct Trajectory {
    pub csv: String,
    pub columns: Vec<Column>,
    pub rows: usize,
    pub events: Events,
}
pub struct Context<'a> {
    pub metadata: &'a Value,
    pub request: &'a Value,
    pub source: &'a str,
    pub scenario_id: Option<&'a str>,
    pub modified: bool,
    pub effective_hash: &'a str,
    pub selected_outputs: &'a Value,
}

fn text(value: &Value) -> Result<Option<String>, String> {
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .filter(|s| !s.is_empty() && !s.contains('\0'))
        .map(|s| Some(s.to_owned()))
        .ok_or_else(|| "Table units and names must be nonempty text or null".into())
}
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}
fn finite(value: &Value) -> Result<f64, String> {
    value
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| "CSV requires complete finite numerical rows".into())
}

pub fn trajectory(result: &Value, native: &Value) -> Result<Trajectory, String> {
    let times = result["times"].as_array().ok_or("CSV times are missing")?;
    let sides = result["sides"]
        .as_array()
        .ok_or("CSV event sides are missing")?;
    let outputs = result["outputs"]
        .as_array()
        .ok_or("CSV outputs are missing")?;
    if times.len() != sides.len() || times.len() > MAX_ROWS {
        return Err("CSV sides must match the time rows, within the 2,000,000-row limit".into());
    }
    let unit = text(&result["time_unit"])?;
    let mut columns = vec![
        Column {
            name: format!("time_{}", unit.as_deref().unwrap_or("unit")),
            unit,
            kind: ColumnKind::Time,
            native_name: None,
            meaning: None,
        },
        Column {
            name: "side".into(),
            unit: None,
            kind: ColumnKind::EventSide,
            native_name: None,
            meaning: None,
        },
    ];
    for output in outputs {
        let name = text(&output["name"])?.ok_or("CSV output name is missing")?;
        let readable = crate::core::usability::readable(&name, native);
        if readable.is_empty()
            || readable.contains('\0')
            || columns.iter().any(|c| c.name == readable)
        {
            return Err(
                "CSV readable column names must be nonempty and unique, including time and side"
                    .into(),
            );
        }
        if output["values"]
            .as_array()
            .is_none_or(|v| v.len() != times.len())
        {
            return Err("CSV output values must match the time row count".into());
        }
        columns.push(Column {
            name: readable,
            unit: text(&output["unit"])?,
            kind: ColumnKind::Output,
            native_name: Some(name),
            meaning: None,
        });
    }
    let mut csv = columns
        .iter()
        .map(|c| csv_field(&c.name))
        .collect::<Vec<_>>()
        .join(",");
    csv.push('\n');
    let mut events = Events {
        rule: "nondecreasing_time;equal_time_requires_pre_then_post".into(),
        pre: 0,
        post: 0,
        event_pairs: 0,
    };
    let mut previous: Option<(f64, &str)> = None;
    let mut paired = false;
    for (i, time) in times.iter().enumerate() {
        let value = finite(time)?;
        let side = sides[i]
            .as_str()
            .filter(|s| matches!(*s, "pre" | "post"))
            .ok_or("CSV event side must be pre or post")?;
        if let Some((before, before_side)) = previous {
            if value < before {
                return Err("CSV times must be nondecreasing".into());
            }
            if value == before {
                if paired || (before_side, side) != ("pre", "post") {
                    return Err("Equal-time CSV rows must be exactly pre then post".into());
                }
                paired = true;
                events.event_pairs += 1;
            } else {
                paired = false;
            }
        }
        if side == "pre" {
            events.pre += 1;
        } else {
            events.post += 1;
        }
        previous = Some((value, side));
        csv.push_str(&format!("{time},{side}"));
        for output in outputs {
            finite(&output["values"][i])?;
            csv.push_str(&format!(",{}", output["values"][i]));
        }
        csv.push('\n');
        if csv.len() > MAX_BYTES {
            return Err("CSV exceeds the 256 MiB table limit".into());
        }
    }
    if csv.len() > MAX_BYTES {
        return Err("CSV exceeds the 256 MiB table limit".into());
    }
    Ok(Trajectory {
        csv,
        columns,
        rows: times.len(),
        events,
    })
}

pub fn manifest(
    path: &str,
    result: &Value,
    trajectory: &Trajectory,
    context: Context<'_>,
) -> Result<Vec<u8>, String> {
    let path = crate::export::csv_path(path)?;
    let metadata = context.metadata;
    let coordinates = text(&metadata["native_execution"]["coordinates"])?;
    let selected_outputs = if context.selected_outputs.is_null() {
        None
    } else {
        let names = context
            .selected_outputs
            .as_array()
            .ok_or("Selected outputs must be an array")?;
        let mut selected = Vec::new();
        for name in names {
            let name = text(name)?.ok_or("Selected output name is missing")?;
            if selected.contains(&name) {
                return Err("Selected output names must be unique".into());
            }
            selected.push(name);
        }
        Some(selected)
    };
    let dimensional = match metadata["native_execution"].get("dimensional_qualification") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_bool()
                .ok_or("Recorded dimensional status must be boolean")?,
        ),
    };
    let manifest = Manifest {
        scientific_assessment: serde_json::from_value(result["scientific_assessment"].clone()).ok(),
        scheduled_boundary_receipt: serde_json::from_value(
            result["scheduled_boundary_receipt"].clone(),
        )
        .ok(),
        initialization_receipt: serde_json::from_value(result["initialization_receipt"].clone())
            .ok(),
        dose_event_receipts: serde_json::from_value(result["dose_event_receipts"].clone()).ok(),
        required_output_receipts: serde_json::from_value(
            result["required_output_receipts"].clone(),
        )
        .ok(),
        schema: Version::V1,
        producer: Producer::Mcp,
        table: Table {
            filename: path.file_name().unwrap().to_string_lossy().into_owned(),
            sha256: crate::library::hash(trajectory.csv.as_bytes()),
            bytes: trajectory.csv.len(),
            rows: trajectory.rows,
            columns: trajectory.columns.clone(),
        },
        events: trajectory.events.clone(),
        model: Model {
            id: text(&result["model_id"])?,
            version: text(&metadata["version"])?,
            archive_sha256: text(&metadata["archive_sha256"])?,
            provenance: Some(
                json!({"metadata":metadata,"source":context.source})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        },
        engine_identity: result["identity"].as_object().cloned(),
        inputs: Inputs {
            effective_request: Some(
                context
                    .request
                    .as_object()
                    .ok_or("Effective request must be an object")?
                    .clone(),
            ),
            selected_outputs,
            effective_request_hash: Some(context.effective_hash.into()),
            scenario_id: context.scenario_id.map(str::to_owned),
            modified: Some(context.modified),
        },
        qualification: Qualification {
            qualified: NotQualified,
            status: "not_assessed".into(),
            scope: "current_engine_and_effective_request".into(),
            unit_status: if coordinates.as_deref() == Some("unchanged_source_numeric_values") {
                "source_scale_unqualified"
            } else {
                "unqualified"
            }
            .into(),
            coordinates,
            recorded_dimensional_qualification: dimensional,
            recorded_availability: metadata["availability"].as_object().cloned(),
        },
    };
    let mut bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err("Table manifest exceeds the 16 MiB limit".into());
    }
    Ok(bytes)
}

pub fn schema() -> Value {
    let mut schema = crate::contract::schema::<Manifest>();
    // Nullable replay context is mandatory. Optional receipts retain their
    // generated presence rules, including optional nested receipt fields.
    schema["required"] = json!([
        "schema",
        "producer",
        "table",
        "events",
        "model",
        "engine_identity",
        "inputs",
        "qualification"
    ]);
    for name in [
        "Table",
        "Column",
        "Events",
        "Model",
        "Inputs",
        "Qualification",
    ] {
        let fields = schema["$defs"][name]["properties"]
            .as_object()
            .expect("table context has generated properties")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        schema["$defs"][name]["required"] = json!(fields);
    }
    let defs = &mut schema["$defs"];
    defs["Table"]["properties"]["bytes"]["maximum"] = json!(MAX_BYTES);
    defs["Table"]["properties"]["rows"]["maximum"] = json!(MAX_ROWS);
    defs["Events"]["properties"]["rule"]["const"] =
        json!("nondecreasing_time;equal_time_requires_pre_then_post");
    defs["Qualification"]["properties"]["status"]["const"] = json!("not_assessed");
    defs["Qualification"]["properties"]["scope"]["const"] =
        json!("current_engine_and_effective_request");
    defs["Qualification"]["properties"]["unit_status"]["enum"] =
        json!(["source_scale_unqualified", "unqualified"]);
    schema
}

#[cfg(test)]
mod tests {
    use super::*;
    fn result() -> Value {
        json!({"model_id":"synthetic","identity":{"backend":"fixture"},"time_unit":"h","times":[0.0,1.0,1.0],"sides":["post","pre","post"],"outputs":[{"name":"cp","unit":"mg/L","values":[0.0,1.0,2.0]}]})
    }
    #[test]
    fn quoted_columns_and_event_pairs_keep_units_native_names_and_inputs() {
        let result = result();
        let native = json!({"outputs":{"free,\"drug\"":{"native_name":"cp"}}});
        let table = trajectory(&result, &native).unwrap();
        assert!(table.csv.starts_with("time_h,side,\"free,\"\"drug\"\"\"\n"));
        assert_eq!(table.events.event_pairs, 1);
        let metadata = json!({"version":"0.1.0","archive_sha256":"a".repeat(64),"native_execution":{"coordinates":"unchanged_source_numeric_values","dimensional_qualification":false},"availability":{"current_engine_qualification":"not_assessed"}});
        let request = json!({"regimen":{"end":{"value":1,"unit":"h"}}});
        let bytes = manifest(
            "study/run.csv",
            &result,
            &table,
            Context {
                metadata: &metadata,
                request: &request,
                source: "actual modified model",
                scenario_id: Some("preset"),
                modified: true,
                effective_hash: "opaque-request-hash",
                selected_outputs: &json!(["cp"]),
            },
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["table"]["sha256"],
            crate::library::hash(table.csv.as_bytes())
        );
        assert_eq!(value["table"]["filename"], "run.csv");
        assert_eq!(value["table"]["columns"][2]["unit"], "mg/L");
        assert_eq!(value["table"]["columns"][2]["native_name"], "cp");
        assert_eq!(value["inputs"]["effective_request"], request);
        assert_eq!(value["inputs"]["modified"], true);
        assert_eq!(
            value["model"]["provenance"]["source"],
            "actual modified model"
        );
        assert_eq!(value["qualification"]["qualified"], false);
        assert_eq!(
            value["qualification"]["unit_status"],
            "source_scale_unqualified"
        );
    }
    #[test]
    fn malformed_trajectories_and_alias_collisions_are_refused() {
        for case in [
            "length",
            "null",
            "side",
            "reverse",
            "triple",
            "decreasing",
            "missing_name",
        ] {
            let mut value = result();
            match case {
                "length" => {
                    value["outputs"][0]["values"].as_array_mut().unwrap().pop();
                }
                "null" => value["outputs"][0]["values"][0] = Value::Null,
                "side" => value["sides"][0] = json!("unknown"),
                "reverse" => {
                    value["sides"][1] = json!("post");
                    value["sides"][2] = json!("pre");
                }
                "triple" => value["times"][0] = json!(1.0),
                "decreasing" => value["times"][2] = json!(-1.0),
                _ => value["outputs"][0]["name"] = Value::Null,
            }
            assert!(trajectory(&value, &Value::Null).is_err(), "{case}");
        }
        for alias in ["side", "time_h", "", "bad\0name"] {
            let native = json!({"outputs":{alias:{"native_name":"cp"}}});
            assert!(trajectory(&result(), &native).is_err(), "{alias:?}");
        }
        let mut value = result();
        let duplicate = value["outputs"][0].clone();
        value["outputs"].as_array_mut().unwrap().push(duplicate);
        assert!(trajectory(&value, &Value::Null).is_err());
    }
    #[test]
    fn schema_requires_nullable_context_and_cannot_assert_qualification() {
        let value = schema();
        assert_eq!(value["$defs"]["NotQualified"]["const"], false);
        assert!(value["required"]
            .as_array()
            .unwrap()
            .contains(&json!("engine_identity")));
        assert_eq!(
            value["$defs"]["Table"]["properties"]["rows"]["maximum"],
            MAX_ROWS
        );
        assert_eq!(
            value["$defs"]["Manifest"]["additionalProperties"],
            Value::Null
        );
        assert_eq!(value["additionalProperties"], false);
        for name in [
            "scientific_assessment",
            "scheduled_boundary_receipt",
            "initialization_receipt",
            "dose_event_receipts",
            "required_output_receipts",
        ] {
            assert!(!value["required"].as_array().unwrap().contains(&json!(name)));
        }
        let empty = json!({"times":[],"sides":[],"outputs":[]});
        let table = trajectory(&empty, &Value::Null).unwrap();
        assert_eq!(table.csv, "time_unit,side\n");
        assert_eq!(table.rows, 0);
    }
}

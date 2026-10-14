//! Current request and result formats have strict schema and field admission.
use qspquest_core::run::{AccuracyControl, RunRequest};
use serde_json::{json, Value};

const CURRENT_REQUEST: &str = include_str!("../../../docs/examples/first-run.json");

fn same_value(encoded: &Value, original: &Value) -> bool {
    match original {
        Value::Number(_) => encoded.as_f64() == original.as_f64(),
        Value::Array(values) => encoded.as_array().is_some_and(|rows| {
            rows.len() == values.len() && rows.iter().zip(values).all(|(a, b)| same_value(a, b))
        }),
        Value::Object(fields) => encoded.as_object().is_some_and(|object| {
            fields
                .iter()
                .all(|(key, value)| object.get(key).is_some_and(|v| same_value(v, value)))
        }),
        _ => encoded == original,
    }
}

#[test]
fn current_request_roundtrip_preserves_scientific_values() {
    let original: Value = serde_json::from_str(CURRENT_REQUEST).unwrap();
    let parsed: RunRequest = serde_json::from_value(original.clone()).unwrap();
    assert_eq!(parsed.accuracy_control, AccuracyControl::Local);
    let encoded = serde_json::to_value(parsed).unwrap();
    assert_eq!(encoded["schema"], "qspquest.run/v0.1");
    for key in ["solver", "budgets"] {
        assert_eq!(encoded[key], original[key], "{key}");
    }
    for key in ["end", "observations", "administrations", "rtol", "atol"] {
        // Optional fields are emitted as null but must not change original values.
        for (index, record) in original["regimen"][key]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            for (field, value) in record.as_object().unwrap() {
                assert!(
                    same_value(&encoded["regimen"][key][index][field], value),
                    "{key}.{field}"
                );
            }
        }
        if !original["regimen"][key].is_array() {
            assert!(
                same_value(&encoded["regimen"][key], &original["regimen"][key]),
                "{key}"
            );
        }
    }
}

#[test]
fn unsupported_formats_and_fields_cannot_be_silently_discarded() {
    let original: Value = serde_json::from_str(CURRENT_REQUEST).unwrap();
    for schema in ["qspquest.run/v99", "other.run/v0.1"] {
        let mut changed = original.clone();
        changed["schema"] = json!(schema);
        assert!(
            serde_json::from_value::<RunRequest>(changed).is_err(),
            "{schema}"
        );
    }
    for pointer in [
        "/unexpected",
        "/regimen/unexpected",
        "/regimen/observations/0/unexpected",
        "/regimen/administrations/0/unexpected",
        "/regimen/administrations/0/amount/unexpected",
    ] {
        let mut changed = original.clone();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        changed
            .pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(field.into(), json!(true));
        assert!(
            serde_json::from_value::<RunRequest>(changed).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn only_current_schema_identifiers_are_admitted() {
    fn schema<T: serde::de::DeserializeOwned + serde::Serialize>(name: &str) {
        let current = format!("qspquest.{name}/v0.1");
        let value: T = serde_json::from_value(json!(current)).unwrap();
        assert_eq!(serde_json::to_value(value).unwrap(), json!(current));
        for invalid in [
            json!(format!("other.{name}/v0.1")),
            json!(format!("qspquest.{name}/v99")),
            Value::Null,
            json!([current]),
            json!(1),
        ] {
            assert!(
                serde_json::from_value::<T>(invalid.clone()).is_err(),
                "{invalid}"
            );
        }
    }
    use qspquest_core::{fit, model, morris, run, scan};
    schema::<model::Schema>("model");
    schema::<run::RunSchema>("run");
    schema::<run::ResultSchema>("result");
    schema::<run::SensitivitySchema>("sensitivity");
    schema::<run::SensitivityResultSchema>("sensitivity-result");
    schema::<scan::ScanSchema>("scan");
    schema::<scan::ScanResultSchema>("scan-result");
    schema::<morris::MorrisSchema>("morris");
    schema::<morris::MorrisResultSchema>("morris-result");
    schema::<fit::ScalarFitSchema>("scalar-fit");
    schema::<fit::ScalarFitResultSchema>("scalar-fit-result");
    schema::<fit::FitSchema>("fit");
    schema::<fit::FitResultSchema>("fit-result");
}

#[test]
fn nested_fit_schemas_are_required_at_every_level() {
    use qspquest_core::fit::FitRequest;
    let current: Value = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-fit.json"
    ))
    .unwrap();
    let parsed: FitRequest = serde_json::from_value(current.clone()).unwrap();
    let encoded = serde_json::to_value(parsed).unwrap();
    let reparsed: FitRequest = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(serde_json::to_value(reparsed).unwrap(), encoded);
    for pointer in [
        "",
        "/problem/request/objective/simulation",
        "/problem/request/objective/simulation/run",
    ] {
        let tag = current.pointer(pointer).unwrap()["schema"]
            .as_str()
            .unwrap();
        for invalid in [
            json!(tag.replace("qspquest.", "other.")),
            json!(tag.replace("v0.1", "v99")),
            Value::Null,
            json!([tag]),
            json!(false),
        ] {
            let mut changed = current.clone();
            changed.pointer_mut(pointer).unwrap()["schema"] = invalid;
            assert!(
                serde_json::from_value::<FitRequest>(changed).is_err(),
                "{pointer}"
            );
        }
        let mut missing = current.clone();
        missing
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("schema");
        assert!(
            serde_json::from_value::<FitRequest>(missing).is_err(),
            "{pointer}"
        );
    }
}

use qspquest_core::{initial_values::InitialValuesRequest, model::Quantity};
use qspquest_modeling::document::CompiledDocument;
use serde_json::json;
const FIXTURE: &str = include_str!("../../../conformance/models/synthetic-one-compartment.json");
fn request() -> InitialValuesRequest {
    serde_json::from_value(json!({"schema":"qspquest.initial-values/v0.1"})).unwrap()
}
#[test]
fn initial_values_bind_converted_inputs_and_states_without_future_rhs() {
    let mut model: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    let ratio = &mut model["states"][0]["rhs"]["arguments"][1]["arguments"][0]["arguments"][1];
    *ratio = json!({"kind":"binary","operator":"subtract","arguments":[{"kind":"symbol","name":"v"},{"kind":"symbol","name":"v"}]});
    model["covariates"] = json!([{"name":"weight","unit":"kg","interpolation":"constant","default":{"value":70,"unit":"kg"}}]);
    model["outputs"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"weight","unit":"kg","expression":{"kind":"symbol","name":"weight"}}));
    let compiled = CompiledDocument::from_json(&model.to_string()).unwrap();
    let mut r = request();
    r.parameters.insert(
        "v".into(),
        Quantity {
            value: 6000.0,
            unit: "mL".into(),
        },
    );
    r.initial_states.insert(
        "central".into(),
        Quantity {
            value: 6000.0,
            unit: "ug".into(),
        },
    );
    r.covariates.insert(
        "weight".into(),
        Quantity {
            value: 80000.0,
            unit: "g".into(),
        },
    );
    let plan = compiled.plan_initial_values(&r).unwrap();
    let result = compiled.initial_values(&r).unwrap();
    assert_eq!(result.identity, plan.prospective_identity);
    assert_eq!(result.identity.algorithm, "initial_output_evaluation");
    assert_eq!(result.states[0].value, 6.0);
    assert_eq!(result.outputs[0].value, 1.0);
    assert_eq!(result.outputs[0].unit, "mg/L");
    assert_eq!(result.outputs[1].value, 80.0);
    let first = result.identity;
    r.parameters.get_mut("v").unwrap().value = 3000.0;
    assert_ne!(first, compiled.initial_values(&r).unwrap().identity);
}
#[test]
fn initial_plan_defers_output_domain_but_admits_names_units_and_budgets() {
    let mut model: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    model["outputs"][0]["expression"]["arguments"][1] = json!({"kind":"binary","operator":"subtract","arguments":[{"kind":"symbol","name":"v"},{"kind":"symbol","name":"v"}]});
    let compiled = CompiledDocument::from_json(&model.to_string()).unwrap();
    let mut r = request();
    assert!(compiled.plan_initial_values(&r).is_ok());
    assert_eq!(
        compiled.initial_values(&r).unwrap_err().code,
        qspquest_core::ErrorCode::Domain
    );
    for names in [
        vec![],
        vec!["cp".into(), "cp".into()],
        vec!["missing".into()],
    ] {
        r.outputs = Some(names);
        assert!(compiled.plan_initial_values(&r).is_err());
    }
    r.outputs = None;
    r.max_output_values = 1;
    assert_eq!(
        compiled.plan_initial_values(&r).unwrap_err().code,
        qspquest_core::ErrorCode::OutputBudget
    );
    r.max_output_values = 10;
    r.parameters.insert(
        "v".into(),
        Quantity {
            value: 3.0,
            unit: "h".into(),
        },
    );
    assert!(compiled.plan_initial_values(&r).is_err());
}

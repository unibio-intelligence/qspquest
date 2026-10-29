use qspquest_core::{model::ModelDocument, run::RunRequest};
use qspquest_modeling::{document::CompiledDocument, language};
use serde_json::{json, Value};
fn document() -> ModelDocument {
    let mut doc: Value = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    doc["parameters"].as_array_mut().unwrap().push(json!({"name":"delay","default":{"value":30,"unit":"min"},"bounds":null,"transform":"identity","fixed":false,"description":""}));
    doc["states"][0]["dosing"]["lag"] = json!({"kind":"symbol","name":"delay"});
    serde_json::from_value(doc).unwrap()
}
fn request() -> RunRequest {
    serde_json::from_value(json!({
        "schema":"qspquest.run/v0.1","solver":"diffsol_bdf",
        "budgets":{"solver_callbacks":1000000,"output_values":10000,"events":100},
        "regimen":{"end":{"value":8,"unit":"h"},"samples":[],
        "observations":(0..=8).flat_map(|t| ["pre","post"].map(move |side| json!({"time":{"value":t,"unit":"h"},"side":side}))).collect::<Vec<_>>(),
        "administrations":[{"target":"central","time":{"value":0,"unit":"h"},"amount":{"value":6000,"unit":"ug"},"delivery":{"kind":"bolus"},"repeat":{"interval":{"value":2,"unit":"h"},"additional":2},"lag":{"value":30,"unit":"min"},"bioavailability":0.5}],
        "rtol":1e-10,"atol":1e-12}
    })).unwrap()
}
#[test]
fn parameter_lag_rebinds_repeated_events_and_preserves_sides() {
    let model = CompiledDocument::compile(&document()).unwrap();
    for delay in [30., 90., 30.] {
        let mut req = request();
        req.parameters.insert(
            "delay".into(),
            qspquest_core::model::Quantity {
                value: delay,
                unit: "min".into(),
            },
        );
        let result = model.execute(&req).unwrap();
        let start = delay / 60. + 0.5;
        for (i, &t) in result.times.iter().enumerate() {
            let post = matches!(
                result.sides[i],
                qspquest_core::regimen::ObservationSide::Post
            );
            let expected = (0..3)
                .map(|j| start + 2. * j as f64)
                .filter(|d| *d < t || (*d == t && post))
                .map(|d| (-0.1 * (t - d)).exp())
                .sum::<f64>();
            assert!((result.outputs[0].values[i] - expected).abs() < 1e-7);
        }
        let rows = model
            .bind(&req.parameters)
            .unwrap()
            .simulate_regimen(&req.regimen, req.budgets)
            .unwrap();
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(row.values[0], result.outputs[0].values[i]);
        }
    }
}
#[test]
fn invalid_lags_and_unbound_protocols_fail_closed() {
    let doc = document();
    let model = CompiledDocument::compile(&doc).unwrap();
    let text = language::format(&doc).unwrap();
    let parsed = language::parse(&text).unwrap();
    assert_eq!(
        serde_json::to_value(&doc).unwrap(),
        serde_json::to_value(&parsed.document).unwrap()
    );
    assert!(text.contains("lag(central) = delay"));
    assert!(parsed.locations.contains_key("states.central.dosing.lag"));
    let repeated = text.replace(
        "  lag(central) = delay",
        "  lag(central) = delay\n  lag(central) = delay",
    );
    assert!(language::parse(&repeated).is_err());
    assert!(language::parse(&text.replace("lag(central)", "lag(missing)")).is_err());
    assert!(model
        .expand_regimen(&request().regimen.administrations, 8., 100)
        .is_err());
    assert!(model
        .regimen_protocol(&request().regimen, Default::default())
        .is_err());
    let mut req = request();
    req.parameters.insert(
        "delay".into(),
        qspquest_core::model::Quantity {
            value: -1.,
            unit: "h".into(),
        },
    );
    assert!(model
        .execute(&req)
        .unwrap_err()
        .message
        .contains("nonnegative"));
    let mut bad = doc.clone();
    bad.states[0].dosing.as_mut().unwrap().lag =
        Some(qspquest_core::expression::Expr::symbol("central"));
    assert!(CompiledDocument::compile(&bad).is_err());
    bad.states[0].dosing.as_mut().unwrap().lag =
        Some(qspquest_core::expression::Expr::symbol("cl"));
    assert!(CompiledDocument::compile(&bad).is_err());
}

#[test]
fn constant_covariate_lag_uses_request_values_and_rejects_steps() {
    use qspquest_core::{
        expression::{Binary, Expr},
        model::{Covariate, Interpolation, Quantity},
    };
    let mut doc = document();
    doc.covariates.push(Covariate {
        name: "factor".into(),
        unit: "1".into(),
        interpolation: Interpolation::Constant,
        default: None,
    });
    doc.states[0].dosing.as_mut().unwrap().lag = Some(Expr::binary(
        Binary::Multiply,
        Expr::symbol("delay"),
        Expr::symbol("factor"),
    ));
    let model = CompiledDocument::compile(&doc).unwrap();
    assert!(model.execute(&request()).is_err());
    for factor in [1., 3., 1.] {
        let mut req = request();
        req.regimen.covariates.insert(
            "factor".into(),
            Quantity {
                value: factor,
                unit: "1".into(),
            },
        );
        let result = model.execute(&req).unwrap();
        let start = 0.5 * factor + 0.5;
        for (i, &t) in result.times.iter().enumerate() {
            let post = matches!(
                result.sides[i],
                qspquest_core::regimen::ObservationSide::Post
            );
            let expected = (0..3)
                .map(|j| start + 2. * j as f64)
                .filter(|d| *d < t || (*d == t && post))
                .map(|d| (-0.1 * (t - d)).exp())
                .sum::<f64>();
            assert!((result.outputs[0].values[i] - expected).abs() < 1e-7);
        }
    }
    doc.covariates[0].interpolation = Interpolation::Step;
    assert!(CompiledDocument::compile(&doc).is_err());
}

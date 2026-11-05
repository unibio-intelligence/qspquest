use qspquest_core::{model::ModelDocument, run::RunRequest};
use qspquest_modeling::{document::CompiledDocument, language};
use serde_json::{json, Value};
fn symbol(name: &str) -> Value {
    json!({"kind":"symbol","name":name})
}
fn number(value: f64) -> Value {
    json!({"kind":"literal","value":value})
}
fn binary(op: &str, a: Value, b: Value) -> Value {
    json!({"kind":"binary","operator":op,"arguments":[a,b]})
}
fn guarded(value: Value, zero: Value) -> Value {
    json!({"kind":"conditional","arguments":[{"kind":"compare","operator":"gt","arguments":[symbol("seen"),number(0.)]},value,zero]})
}
fn document() -> ModelDocument {
    let amount = guarded(
        symbol("previous"),
        binary("multiply", symbol("depot"), number(0.)),
    );
    let elapsed = guarded(
        symbol("elapsed"),
        binary("multiply", symbol("time"), number(0.)),
    );
    serde_json::from_value(json!({"schema":"qspquest.model/v0.1","model_id":"owned_history","description":"Owned history integration","time_unit":"h",
 "parameters":[{"name":"rate","default":{"value":1,"unit":"1/h"},"bounds":null,"transform":"identity","fixed":false,"description":""}],
 "dose_history":[{"name":"previous","target":"depot","quantity":"dose_amount","simultaneous":"first"},{"name":"elapsed","target":"depot","quantity":"time_since_dose","simultaneous":"first"},{"name":"seen","target":"depot","quantity":"has_dose","simultaneous":"first"}],
 "states":[{"name":"depot","unit":"mg","initial":{"kind":"quantity","quantity":{"value":0,"unit":"mg"}},"rhs":binary("multiply",binary("multiply",symbol("depot"),symbol("rate")),number(0.)),"dosing":{"amount_unit":"mg","scale":number(1.)}},
 {"name":"integral","unit":"mg","initial":{"kind":"quantity","quantity":{"value":0,"unit":"mg"}},"rhs":binary("multiply",amount.clone(),symbol("rate")),"dosing":null}],
 "outputs":[{"name":"last_amount","unit":"mg","expression":amount},{"name":"elapsed_time","unit":"h","expression":elapsed},{"name":"available","unit":"1","expression":symbol("seen")},{"name":"integrated","unit":"mg","expression":symbol("integral")},{"name":"delivered","unit":"mg","expression":symbol("depot")}]
 })).unwrap()
}
fn request() -> RunRequest {
    serde_json::from_value(json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf","budgets":{"solver_callbacks":1000000,"output_values":10000,"events":100},
 "regimen":{"end":{"value":4,"unit":"h"},"samples":[],"observations":([0.,1.,1.5,2.,3.,3.5,4.].into_iter().flat_map(|t|["pre","post"].map(move |side|json!({"time":{"value":t,"unit":"h"},"side":side}))).collect::<Vec<_>>()),
 "administrations":[{"target":"depot","time":{"value":1,"unit":"h"},"amount":{"value":8000,"unit":"ug"},"delivery":{"kind":"bolus"},"lag":{"value":30,"unit":"min"},"bioavailability":0.25},{"target":"depot","time":{"value":3,"unit":"h"},"amount":{"value":4,"unit":"mg"},"delivery":{"kind":"bolus"},"lag":{"value":30,"unit":"min"},"bioavailability":0.25}],"rtol":1e-10,"atol":1e-12}})).unwrap()
}
#[test]
fn model_history_drives_rhs_and_readouts_with_exact_sides() {
    let compiled = CompiledDocument::compile(&document()).unwrap();
    let req = request();
    let result = compiled.execute(&req).unwrap();
    for (i, &t) in result.times.iter().enumerate() {
        let post = matches!(
            result.sides[i],
            qspquest_core::regimen::ObservationSide::Post
        );
        let first = t > 1.5 || (t == 1.5 && post);
        let second = t > 3.5 || (t == 3.5 && post);
        let expected = if second {
            4.
        } else if first {
            8.
        } else {
            0.
        };
        assert_eq!(result.outputs[0].values[i], expected);
        assert_eq!(
            result.outputs[1].values[i],
            if second {
                t - 3.5
            } else if first {
                t - 1.5
            } else {
                0.
            }
        );
        assert_eq!(result.outputs[2].values[i], if first { 1. } else { 0. });
        let integral = 8. * (t - 1.5).max(0.).min(2.) + 4. * (t - 3.5).max(0.);
        assert!((result.outputs[3].values[i] - integral).abs() < 1e-7);
        assert_eq!(
            result.outputs[4].values[i],
            if second {
                3.
            } else if first {
                2.
            } else {
                0.
            }
        );
    }
    let repeat = compiled.execute(&req).unwrap();
    assert_eq!(result.times, repeat.times);
    let rows = compiled
        .bind(&req.parameters)
        .unwrap()
        .simulate_regimen(&req.regimen, req.budgets)
        .unwrap();
    for (i, row) in rows.iter().enumerate() {
        for (j, output) in result.outputs.iter().enumerate() {
            assert_eq!(row.values[j], output.values[i]);
        }
    }

    let text = language::format(&document()).unwrap();
    let parsed = language::parse(&text).unwrap();
    assert_eq!(
        serde_json::to_value(document()).unwrap(),
        serde_json::to_value(&parsed.document).unwrap()
    );
    assert!(parsed.locations.contains_key("dose_history.previous"));
    let from_text = CompiledDocument::compile(&parsed.document)
        .unwrap()
        .execute(&req)
        .unwrap();
    assert_eq!(
        from_text.identity.model_content_hash,
        result.identity.model_content_hash
    );
    for (a, b) in from_text.outputs.iter().zip(&result.outputs) {
        assert_eq!(a.values, b.values);
    }
}
#[test]
fn missing_history_and_external_overrides_fail_without_poisoning_later_runs() {
    let mut doc = document();
    doc.outputs[0].expression = qspquest_core::expression::Expr::symbol("previous");
    let compiled = CompiledDocument::compile(&doc).unwrap();
    assert!(compiled.execute(&request()).is_err());
    let good = CompiledDocument::compile(&document()).unwrap();
    let mut req = request();
    req.regimen.covariates.insert(
        "history_internal_0_value".into(),
        qspquest_core::model::Quantity {
            value: 999.,
            unit: "mg".into(),
        },
    );
    assert!(good.execute(&req).is_err());
    assert!(good
        .bind_with_covariates(&req.parameters, &req.regimen.covariates)
        .is_err());
    assert!(good.execute(&request()).is_ok());
    let mut req = request();
    req.budgets.events = 2;
    assert!(good.execute(&req).is_err());
}

#[test]
fn initial_dose_post_side_exposes_history_but_pre_side_is_missing() {
    let mut doc = document();
    doc.outputs[0].expression = qspquest_core::expression::Expr::symbol("previous");
    let model = CompiledDocument::compile(&doc).unwrap();
    let mut req = request();
    req.regimen.administrations.truncate(1);
    req.regimen.administrations[0].time.value = 0.;
    req.regimen.administrations[0].lag = None;
    req.regimen
        .observations
        .as_mut()
        .unwrap()
        .retain(|o| matches!(o.side, qspquest_core::regimen::ObservationSide::Post));
    let result = model.execute(&req).unwrap();
    assert_eq!(result.outputs[0].values[0], 8.);
    req.regimen.observations.as_mut().unwrap()[0].side =
        qspquest_core::regimen::ObservationSide::Pre;
    assert!(model.execute(&req).is_err());
}
#[test]
fn model_lag_is_applied_before_history_and_unsafe_consumers_are_rejected() {
    let mut value = serde_json::to_value(document()).unwrap();
    value["parameters"].as_array_mut().unwrap().push(json!({"name":"delay","default":{"value":30,"unit":"min"},"bounds":null,"transform":"identity","fixed":false,"description":""}));
    value["states"][0]["dosing"]["lag"] = symbol("delay");
    let doc: ModelDocument = serde_json::from_value(value).unwrap();
    let model = CompiledDocument::compile(&doc).unwrap();
    let result = model.execute(&request()).unwrap();
    for (i, &time) in result.times.iter().enumerate() {
        let post = matches!(
            result.sides[i],
            qspquest_core::regimen::ObservationSide::Post
        );
        let expected = if time == 4. && post {
            4.
        } else if time > 2. || (time == 2. && post) {
            8.
        } else {
            0.
        };
        assert_eq!(result.outputs[0].values[i], expected);
    }
    let mut bad = document();
    bad.states[0].initial = qspquest_core::model::Initial::Expression {
        expression: qspquest_core::expression::Expr::symbol("previous"),
    };
    assert!(CompiledDocument::compile(&bad).is_err());
    let mut bad = document();
    bad.states[0].dosing.as_mut().unwrap().scale = qspquest_core::expression::Expr::symbol("seen");
    assert!(CompiledDocument::compile(&bad).is_err());
}

#[test]
fn history_language_roundtrips_all_quantities_and_policies() {
    use qspquest_core::{
        dose_history::SimultaneousDose,
        model::{DoseHistoryBinding, HistoryQuantity},
    };
    for quantity in [
        HistoryQuantity::DoseAmount,
        HistoryQuantity::DoseTime,
        HistoryQuantity::TimeSinceDose,
        HistoryQuantity::HasDose,
    ] {
        for simultaneous in [SimultaneousDose::First, SimultaneousDose::Last] {
            let mut doc = document();
            doc.dose_history.push(DoseHistoryBinding {
                name: "extra_history".into(),
                target: "depot".into(),
                quantity,
                simultaneous,
            });
            let text = language::format(&doc).unwrap();
            let parsed = language::parse(&text).unwrap();
            assert_eq!(
                serde_json::to_value(&doc).unwrap(),
                serde_json::to_value(&parsed.document).unwrap()
            );
            assert_eq!(language::format(&parsed.document).unwrap(), text);
        }
    }
}
#[test]
fn history_language_rejects_ambiguous_or_malformed_declarations() {
    let text = language::format(&document()).unwrap();
    let valid = "previous = dose_amount(depot) simultaneous first";
    assert!(text.contains(valid));
    for invalid in [
        "previous = dose_amount(depot)",
        "previous = dose_amount(depot) simultaneous sum",
        "previous = guess(depot) simultaneous first",
        "previous = dose_amount(depot, integral) simultaneous first",
        "previous = dose_amount(depot) simultaneous first extra",
        "previous = dose_amount(depot simultaneous first",
    ] {
        let source = text.replace(valid, invalid);
        let error = language::parse(&source).unwrap_err();
        assert!(error.line > 1);
        assert!(error.column > 1);
    }
}

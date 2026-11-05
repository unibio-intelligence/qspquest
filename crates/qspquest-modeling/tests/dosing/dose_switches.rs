use qspquest_core::{model::ModelDocument, regimen::ObservationSide, run::RunRequest};
use qspquest_modeling::document::CompiledDocument;
use serde_json::{json, Value};
fn s(name: &str) -> Value {
    json!({"kind":"symbol","name":name})
}
fn n(value: f64) -> Value {
    json!({"kind":"literal","value":value})
}
fn model(op: &str, offset: f64) -> ModelDocument {
    let phase = json!({"kind":"conditional","arguments":[{"kind":"compare","operator":"gt","arguments":[s("seen"),n(0.)]},s("phase"),n(0.)]});
    serde_json::from_value(json!({"schema":"qspquest.model/v0.1","model_id":"owned_dose_switch","description":"Owned dose-relative phase","time_unit":"h",
 "parameters":[{"name":"delay","default":{"value":offset,"unit":"h"},"bounds":null,"transform":"identity","fixed":false,"description":""},{"name":"rate","default":{"value":1,"unit":"mg/h"},"bounds":null,"transform":"identity","fixed":false,"description":""}],
 "dose_history":[{"name":"seen","target":"depot","quantity":"has_dose","simultaneous":"first"}],
 "dose_switches":[{"name":"phase","target":"depot","offset":s("delay"),"comparison":op,"simultaneous":"first"}],
 "states":[{"name":"depot","unit":"mg","initial":{"kind":"quantity","quantity":{"value":0,"unit":"mg"}},"rhs":{"kind":"binary","operator":"multiply","arguments":[s("rate"),n(0.)]},"dosing":{"amount_unit":"mg","scale":n(1.)}},
 {"name":"accumulated","unit":"mg","initial":{"kind":"quantity","quantity":{"value":0,"unit":"mg"}},"rhs":{"kind":"binary","operator":"multiply","arguments":[s("rate"),phase.clone()]},"dosing":null}],
 "outputs":[{"name":"active","unit":"1","expression":phase},{"name":"integral","unit":"mg","expression":s("accumulated")}]})).unwrap()
}
fn request(doses: &[f64], times: &[f64], end: f64) -> RunRequest {
    let observations = times
        .iter()
        .flat_map(|&t| {
            ["pre", "post"].map(move |side| json!({"time":{"value":t,"unit":"h"},"side":side}))
        })
        .collect::<Vec<_>>();
    let administrations=doses.iter().map(|&t|json!({"target":"depot","time":{"value":t,"unit":"h"},"amount":{"value":8,"unit":"mg"},"delivery":{"kind":"bolus"},"bioavailability":0.25})).collect::<Vec<_>>();
    serde_json::from_value(json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf","budgets":{"solver_callbacks":1000000,"output_values":10000,"events":100},"regimen":{"end":{"value":end,"unit":"h"},"samples":[],"observations":observations,"administrations":administrations,"rtol":1e-10,"atol":1e-12}})).unwrap()
}
#[test]
fn relative_switches_restart_and_keep_pointwise_pre_post_values() {
    for op in ["lt", "le", "gt", "ge"] {
        let m = CompiledDocument::compile(&model(op, 2.)).unwrap();
        let req = request(&[0., 1., 4.], &[0., 1., 2., 3., 4., 6., 7.], 7.);
        let out = m.execute(&req).unwrap();
        for (i, &time) in out.times.iter().enumerate() {
            let post = matches!(out.sides[i], ObservationSide::Post);
            let last = [0., 1., 4.]
                .into_iter()
                .filter(|&d| d < time || (d == time && post))
                .last();
            let expected = last.is_some_and(|d| match op {
                "lt" => time - d < 2.,
                "le" => time - d <= 2.,
                "gt" => time - d > 2.,
                _ => time - d >= 2.,
            });
            assert_eq!(out.outputs[0].values[i], if expected { 1. } else { 0. });
            let late = (time - 3.).max(0.).min(1.) + (time - 6.).max(0.);
            let area = if op == "lt" || op == "le" {
                time - late
            } else {
                late
            };
            assert!(
                (out.outputs[1].values[i] - area).abs() < 1e-7,
                "{op} {time}: {} vs {area}",
                out.outputs[1].values[i]
            );
        }
        let bound = m.bind(&req.parameters).unwrap();
        let rows = bound.simulate_regimen(&req.regimen, req.budgets).unwrap();
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(row.values[0], out.outputs[0].values[i]);
            assert_eq!(row.values[1], out.outputs[1].values[i]);
        }
    }
}
#[test]
fn decimal_readouts_and_integration_phases_remain_distinct() {
    let m = CompiledDocument::compile(&model("lt", 0.6)).unwrap();
    let boundary = 0.3 + 0.6;
    let req = request(&[0.3], &[0., 0.3, boundary, 1.], 1.);
    let out = m.execute(&req).unwrap();
    for (i, &time) in out.times.iter().enumerate() {
        if time == boundary {
            assert_eq!(out.outputs[0].values[i], 1.);
        }
        assert!((out.outputs[1].values[i] - (time - 0.3).max(0.).min(0.6)).abs() < 1e-7);
    }
    let m = CompiledDocument::compile(&model("gt", 0.1)).unwrap();
    let out = m.execute(&request(&[1.], &[1., 1.1, 1.2], 2.)).unwrap();
    for (i, &time) in out.times.iter().enumerate() {
        if time == 1.1 {
            assert_eq!(out.outputs[0].values[i], 1.);
        }
    }
}
#[test]
fn offsets_rebind_and_invalid_requests_do_not_corrupt_models() {
    let m = CompiledDocument::compile(&model("gt", 2.)).unwrap();
    for offset in [1., 2., 1.] {
        let mut req = request(&[0.], &[0., 1., 2., 3.], 3.);
        req.parameters.insert(
            "delay".into(),
            qspquest_core::model::Quantity {
                value: offset,
                unit: "h".into(),
            },
        );
        let out = m.execute(&req).unwrap();
        assert!((out.outputs[1].values.last().unwrap() - (3. - offset)).abs() < 1e-7);
    }
    let mut req = request(&[0.], &[0., 3.], 3.);
    req.parameters.insert(
        "delay".into(),
        qspquest_core::model::Quantity {
            value: -1.,
            unit: "h".into(),
        },
    );
    assert!(m.execute(&req).is_err());
    let mut req = request(&[0.], &[0., 3.], 3.);
    req.regimen.covariates.insert(
        "dose_switch_internal_0_value".into(),
        qspquest_core::model::Quantity {
            value: 1.,
            unit: "1".into(),
        },
    );
    assert!(m.execute(&req).is_err());
    let mut req = request(&[0.], &[0., 3.], 3.);
    req.budgets.events = 2;
    assert!(m.execute(&req).is_err());
    assert!(m.execute(&request(&[0.], &[0., 3.], 3.)).is_ok());
    assert!(CompiledDocument::compile(&model("eq", 2.)).is_err());
}

#[test]
fn language_preserves_predicates_identity_and_execution() {
    use qspquest_core::{
        dose_history::SimultaneousDose,
        expression::{Binary, Expr},
    };
    use qspquest_modeling::{document::CompiledDocument, language};
    for op in ["lt", "le", "gt", "ge"] {
        for policy in [SimultaneousDose::First, SimultaneousDose::Last] {
            let mut doc = model(op, 2.);
            doc.dose_switches[0].simultaneous = policy;
            doc.dose_switches[0].offset = Expr::Binary {
                operator: Binary::Multiply,
                arguments: Box::new([
                    Expr::symbol("delay"),
                    serde_json::from_value(json!({"kind":"literal","value":1.25})).unwrap(),
                ]),
            };
            let text = language::format(&doc).unwrap();
            let parsed = language::parse(&text).unwrap();
            assert_eq!(
                serde_json::to_value(&doc).unwrap(),
                serde_json::to_value(&parsed.document).unwrap()
            );
            assert_eq!(language::format(&parsed.document).unwrap(), text);
            let location = &parsed.locations["dose_switches.phase"];
            assert!(location.line > 1 && location.column > 1);
            let req = request(&[0., 1., 4.], &[0., 1., 2., 3.5, 4., 6.5, 7.], 7.);
            assert_eq!(
                CompiledDocument::compile(&doc)
                    .unwrap()
                    .model_content_hash(),
                parsed.compile().unwrap().model_content_hash()
            );
            let json_result = CompiledDocument::compile(&doc)
                .unwrap()
                .execute(&req)
                .unwrap();
            let text_result = parsed.compile().unwrap().execute(&req).unwrap();
            assert_eq!(json_result.times, text_result.times);
            for (a, b) in json_result.outputs.iter().zip(text_result.outputs.iter()) {
                assert_eq!(a.values, b.values);
            }
        }
    }
}
#[test]
fn language_rejects_malformed_dose_predicates() {
    use qspquest_modeling::language;
    let text = language::format(&model("gt", 2.)).unwrap();
    let valid = "phase = time_since_dose(depot) > delay simultaneous first";
    assert!(text.contains(valid));
    for invalid in [
        "phase = time_since_dose(depot) > delay",
        "phase = time_since_dose(depot) > delay simultaneous sum",
        "phase = time_since_dose(depot) > delay simultaneous first extra",
        "phase = time_since_dose(depot, other) > delay simultaneous first",
        "phase = time_since_dose(depot) == delay simultaneous first",
        "phase = time_since_dose(depot) != delay simultaneous first",
        "phase = time_since_dose(depot) > simultaneous first",
        "phase = dose_time(depot) > delay simultaneous first",
        "phase = time_since_dose(depot > delay simultaneous first",
    ] {
        let err = language::parse(&text.replace(valid, invalid)).unwrap_err();
        assert!(err.line > 1 && err.column > 1, "{invalid}");
    }
}

use qspquest_core::{model::*, run::*, ErrorCode};
use qspquest_modeling::{document::CompiledDocument, language, Model};
use serde_json::json;
use std::collections::BTreeMap;
const TEXT: &str = "model fixed_switch_test
time_unit = h
parameters
  on = 1 [h] identity fixed
  off = 2 [h] identity fixed
  rate = 3 [1/h] identity fixed
switches
  active = on
  stopped = off
definitions
  running = active * (1 - stopped)
states
  x = 0 [1]
dynamics
  d/dt(x) = rate * running
outputs
  x = x [1]
  phase = running [1]
end
";
fn source() -> ModelDocument {
    language::parse(TEXT).unwrap().document
}
fn request() -> RunRequest {
    serde_json::from_value(json!({
        "schema":"qspquest.run/v0.1", "solver":"diffsol_bdf",
        "budgets":{"solver_callbacks":1000000,"output_values":1000000,"events":100},
        "regimen":{"end":{"value":3,"unit":"h"},"samples":[],
            "observations":[
                {"time":{"value":0,"unit":"h"},"side":"post"},
                {"time":{"value":1,"unit":"h"},"side":"pre"},
                {"time":{"value":1,"unit":"h"},"side":"post"},
                {"time":{"value":1.5,"unit":"h"},"side":"post"},
                {"time":{"value":2,"unit":"h"},"side":"pre"},
                {"time":{"value":2,"unit":"h"},"side":"post"},
                {"time":{"value":3,"unit":"h"},"side":"post"}],
            "administrations":[],"rtol":1e-10,"atol":1e-12}
    }))
    .unwrap()
}
#[test]
fn declared_switches_match_analytic_integrals_and_sides() {
    let model = CompiledDocument::compile(&source()).unwrap();
    let result = model.execute(&request()).unwrap();
    for (actual, expected) in result.outputs[0]
        .values
        .iter()
        .zip([0., 0., 0., 1.5, 3., 3., 3.])
    {
        assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
    }
    assert_eq!(result.outputs[1].values, vec![0., 0., 1., 1., 1., 0., 0.]);
    let formatted = language::format(&source()).unwrap();
    assert_eq!(language::parse(&formatted).unwrap().document, source());
    assert_eq!(
        language::format(&language::parse(&formatted).unwrap().document).unwrap(),
        formatted
    );
    let bound = model.bind(&BTreeMap::new()).unwrap();
    assert_eq!(bound.validate().unwrap_err().code, ErrorCode::Unsupported);
    assert!(model
        .regimen_protocol(&request().regimen, request().budgets)
        .is_err());
    assert_eq!(
        bound
            .simulate_regimen(&request().regimen, request().budgets)
            .unwrap()
            .len(),
        7
    );
}
#[test]
fn rebinding_changes_schedule_and_initial_boundary_is_post_switch() {
    let model = CompiledDocument::compile(&source()).unwrap();
    let original = model.execute(&request()).unwrap();
    let mut req = request();
    req.parameters.insert(
        "on".into(),
        Quantity {
            value: 30.,
            unit: "min".into(),
        },
    );
    let changed = model.execute(&req).unwrap();
    assert!((changed.outputs[0].values.last().unwrap() - 4.5).abs() < 1e-8);
    assert_ne!(
        changed.identity.bound_value_hash,
        original.identity.bound_value_hash
    );
    assert_eq!(
        serde_json::to_value(&original.outputs).unwrap(),
        serde_json::to_value(model.execute(&request()).unwrap().outputs).unwrap()
    );
    req.regimen.start = Some(Quantity {
        value: 1.,
        unit: "h".into(),
    });
    req.regimen.observations = Some(vec![qspquest_core::regimen::ObservationPoint {
        time: Quantity {
            value: 1.,
            unit: "h".into(),
        },
        side: qspquest_core::regimen::ObservationSide::Post,
    }]);
    req.parameters.insert(
        "on".into(),
        Quantity {
            value: 1.,
            unit: "h".into(),
        },
    );
    let start = model.execute(&req).unwrap();
    assert_eq!(start.outputs[1].values, vec![1.]);
}
#[test]
fn invalid_times_overrides_and_budgets_fail_explicitly() {
    let model = CompiledDocument::compile(&source()).unwrap();
    let mut req = request();
    req.budgets.events = 1;
    assert_eq!(model.execute(&req).unwrap_err().code, ErrorCode::WorkBudget);
    let mut req = request();
    req.regimen.covariates.insert(
        "active".into(),
        Quantity {
            value: 0.,
            unit: "1".into(),
        },
    );
    assert!(model
        .execute(&req)
        .unwrap_err()
        .message
        .contains("cannot be overridden"));
    assert!(model
        .bind_with_covariates(&BTreeMap::new(), &req.regimen.covariates)
        .is_err());
    let mut bad = source();
    bad.switches[0].time = qspquest_core::expression::Expr::symbol("x");
    assert!(CompiledDocument::compile(&bad).is_err());
    bad.switches[0].time = qspquest_core::expression::Expr::symbol("time");
    assert!(CompiledDocument::compile(&bad).is_err());
    bad.switches[0].time = qspquest_core::expression::Expr::symbol("rate");
    assert_eq!(
        CompiledDocument::compile(&bad).unwrap_err().code,
        ErrorCode::Unit
    );
    let mut bad = source();
    bad.switches[0].name = "x".into();
    assert!(CompiledDocument::compile(&bad).is_err());
}

#[test]
fn switches_apply_before_simultaneous_reset_and_bolus() {
    let text = TEXT.replace("end\n", "dosing\n  x [1] = running\nend\n");
    let model = CompiledDocument::compile(&language::parse(&text).unwrap().document).unwrap();
    let mut raw = serde_json::to_value(request()).unwrap();
    raw["regimen"]["administrations"] = json!([
        {"target":"x","time":{"value":1,"unit":"h"},"amount":{"value":5,"unit":"1"},"delivery":{"kind":"bolus"},"bioavailability":1},
        {"target":"x","time":{"value":2,"unit":"h"},"amount":{"value":5,"unit":"1"},"delivery":{"kind":"bolus"},"bioavailability":1}
    ]);
    raw["regimen"]["resets"] = json!([
        {"target":"x","time":{"value":2,"unit":"h"},"value":{"value":2,"unit":"1"},"order":0,"active_inputs":"continue"}
    ]);
    let result = model
        .execute(&serde_json::from_value(raw).unwrap())
        .unwrap();
    for (actual, expected) in result.outputs[0]
        .values
        .iter()
        .zip([0., 0., 5., 6.5, 8., 2., 2.])
    {
        assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
    }
}
#[test]
fn coincident_switches_are_charged_and_initials_use_start_phase() {
    let model = CompiledDocument::compile(&source()).unwrap();
    let mut req = request();
    req.parameters.insert(
        "off".into(),
        Quantity {
            value: 1.,
            unit: "h".into(),
        },
    );
    let result = model.execute(&req).unwrap();
    assert!(result
        .outputs
        .iter()
        .all(|column| column.values.iter().all(|v| *v == 0.)));
    req.budgets.events = 1;
    assert_eq!(model.execute(&req).unwrap_err().code, ErrorCode::WorkBudget);
    let mut doc = source();
    doc.states[0].initial = Initial::Expression {
        expression: qspquest_core::expression::Expr::symbol("active"),
    };
    let model = CompiledDocument::compile(&doc).unwrap();
    let mut req = request();
    req.regimen.start = Some(Quantity {
        value: 1.,
        unit: "h".into(),
    });
    req.regimen.observations = None;
    req.regimen.samples = vec![
        Quantity {
            value: 1.,
            unit: "h".into(),
        },
        Quantity {
            value: 3.,
            unit: "h".into(),
        },
    ];
    let result = model.execute(&req).unwrap();
    assert_eq!(result.outputs[0].values[0], 1.);
    assert!((result.outputs[0].values.last().unwrap() - 4.).abs() < 1e-8);
}

#[test]
fn strict_and_inclusive_predicates_keep_exact_readout_values() {
    for (op, rising, at) in [
        ("<", false, 0.),
        ("<=", false, 1.),
        (">", true, 0.),
        (">=", true, 1.),
    ] {
        let text = TEXT
            .replace("active = on", &format!("active = time {op} on"))
            .replace("  stopped = off\n", "")
            .replace("active * (1 - stopped)", "active");
        let doc = language::parse(&text).unwrap().document;
        let formatted = language::format(&doc).unwrap();
        assert_eq!(language::parse(&formatted).unwrap().document, doc);
        let model = CompiledDocument::compile(&doc).unwrap();
        let result = model.execute(&request()).unwrap();
        for (i, &time) in result.times.iter().enumerate() {
            let expected = 3.
                * if rising {
                    (time - 1.).max(0.)
                } else {
                    time.min(1.)
                };
            assert!(
                (result.outputs[0].values[i] - expected).abs() < 1e-8,
                "{op} at {time}"
            );
            let phase = if time == 1. {
                at
            } else if (time > 1.) == rising {
                1.
            } else {
                0.
            };
            assert_eq!(result.outputs[1].values[i], phase, "{op} at {time}");
        }
        let mut start_doc = doc.clone();
        start_doc.states[0].initial = Initial::Expression {
            expression: qspquest_core::expression::Expr::symbol("active"),
        };
        let mut req = request();
        req.regimen.start = Some(Quantity {
            value: 1.,
            unit: "h".into(),
        });
        req.regimen.observations = None;
        req.regimen.samples = vec![
            Quantity {
                value: 1.,
                unit: "h".into(),
            },
            Quantity {
                value: 3.,
                unit: "h".into(),
            },
        ];
        let result = CompiledDocument::compile(&start_doc)
            .unwrap()
            .execute(&req)
            .unwrap();
        assert_eq!(result.outputs[0].values[0], at);
        assert_eq!(result.outputs[1].values[0], at);
        let expected = at + if rising { 6. } else { 0. };
        assert!((result.outputs[0].values.last().unwrap() - expected).abs() < 1e-8);
    }
}
#[test]
fn unsupported_predicate_uses_fail_closed() {
    use qspquest_core::expression::{Compare, Expr};
    let mut doc = source();
    doc.switches[0].comparison = Some(Compare::Eq);
    assert_eq!(
        CompiledDocument::compile(&doc).unwrap_err().code,
        ErrorCode::Unsupported
    );
    doc.switches[0].comparison = Some(Compare::Ge);
    doc.states[0].dosing = Some(Dosing {
        lag: None,
        modes: vec![DeliveryMode::Bolus],
        amount_unit: "1".into(),
        scale: Expr::symbol("running"),
    });
    assert!(CompiledDocument::compile(&doc)
        .unwrap_err()
        .message
        .contains("dose scales"));
}

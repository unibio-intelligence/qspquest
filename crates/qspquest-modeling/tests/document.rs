use qspquest_core::model::*;
use qspquest_modeling::{document::CompiledDocument, simulate, Event, Model, Protocol};
use std::collections::BTreeMap;
const FIXTURE: &str = include_str!("../../../conformance/models/synthetic-one-compartment.json");
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
#[test]
fn document_roundtrip_simulates_and_outputs_rebind_without_recompiling() {
    let source: ModelDocument = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(
        source,
        serde_json::from_str::<ModelDocument>(&serde_json::to_string(&source).unwrap()).unwrap()
    );
    let compiled = CompiledDocument::from_json(FIXTURE).unwrap();
    assert_eq!(compiled.output_names(), ["cp"]);
    assert_eq!(compiled.output_units(), ["mg/L"]);
    let mut bound = compiled.bind(&BTreeMap::new()).unwrap();
    let p = Protocol {
        start: 0.0,
        end: 10.0,
        samples: vec![0.0, 1.0, 5.0, 10.0],
        events: vec![Event::Bolus {
            time: 0.0,
            target: 0,
            amount: 6.0,
        }],
        rtol: 1e-9,
        atol: 1e-11,
    };
    for (v, cl) in [(3.0, 0.3), (6.0, 0.3), (6.0, 0.6), (3.0, 0.3)] {
        bound
            .rebind(&BTreeMap::from([
                ("v".into(), q(v * 1000.0, "mL")),
                ("cl".into(), q(cl, "L/h")),
            ]))
            .unwrap();
        for row in simulate(&bound, &p).unwrap() {
            let expected = if row.time == 0.0 && row.side == "pre" {
                0.0
            } else {
                6.0 / v * (-cl / v * row.time).exp()
            };
            assert!((bound.outputs(row.time, &row.values).unwrap()[0] - expected).abs() < 2e-8);
        }
    }
    assert!(bound
        .rebind(&BTreeMap::from([("v".into(), q(-1.0, "L"))]))
        .is_err());
    assert_eq!(bound.outputs(0.0, &[6.0]).unwrap(), vec![2.0]);
}
#[test]
fn schema_unknown_fields_symbols_dimensions_and_defaults_are_rejected() {
    let original: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    for field in [
        "tenant_id",
        "registry",
        "solver",
        "random_effects",
        "individual",
    ] {
        let mut value = original.clone();
        value[field] = serde_json::json!({});
        assert!(
            CompiledDocument::from_json(&value.to_string()).is_err(),
            "{field}"
        );
    }
    let mut d: ModelDocument = serde_json::from_str(FIXTURE).unwrap();
    d.outputs[0].unit = "h".into();
    assert!(CompiledDocument::compile(&d).is_err());
    d.outputs[0].unit = "mg/L".into();
    d.parameters[0].name = "time".into();
    assert!(CompiledDocument::compile(&d).is_err());
    let mut d: ModelDocument = serde_json::from_str(FIXTURE).unwrap();
    d.parameters[0].default.value = 0.0;
    assert!(CompiledDocument::compile(&d).is_err());
    assert!(CompiledDocument::from_json(&FIXTURE.replace("v0.1", "v99")).is_err());
    let c = CompiledDocument::from_json(FIXTURE).unwrap();
    assert!(c
        .bind(&BTreeMap::from([("typo".into(), q(1.0, "L"))]))
        .is_err());
    let mut b = c.bind(&BTreeMap::new()).unwrap();
    assert!(b
        .rebind(&BTreeMap::from([("v".into(), q(1.0, "h"))]))
        .is_err());
    assert_eq!(b.initial(), vec![0.0]);
    assert!(b.outputs(f64::NAN, &[0.0]).is_err());
}
#[test]
fn dimensionless_transform_domains_account_for_unit_scale() {
    let mut d: ModelDocument = serde_json::from_str(FIXTURE).unwrap();
    d.parameters.push(Parameter {
        name: "fraction".into(),
        default: q(50.0, "%"),
        bounds: None,
        transform: Transform::Logit,
        fixed: true,
        description: "".into(),
    });
    let compiled = CompiledDocument::compile(&d).unwrap();
    assert!(compiled
        .bind(&BTreeMap::from([("fraction".into(), q(0.5, "1"))]))
        .is_ok());
    assert!(compiled
        .bind(&BTreeMap::from([("fraction".into(), q(100.0, "%"))]))
        .is_err());
    d.parameters.last_mut().unwrap().default = q(0.5, "mg");
    assert!(CompiledDocument::compile(&d).is_err());
}

#[test]
fn output_projection_enforces_budget_and_preserves_domain_diagnostics() {
    let mut d: ModelDocument = serde_json::from_str(FIXTURE).unwrap();
    for name in ["cp_alt", "cp_third"] {
        let mut output = d.outputs[0].clone();
        output.name = name.into();
        d.outputs.push(output);
    }
    let compiled = CompiledDocument::compile(&d).unwrap();
    let bound = compiled.bind(&BTreeMap::new()).unwrap();
    let protocol = Protocol {
        start: 0.0,
        end: 1.0,
        samples: vec![0.0, 0.5, 1.0],
        events: vec![Event::Bolus {
            time: 0.0,
            target: 0,
            amount: 6.0,
        }],
        rtol: 1e-8,
        atol: 1e-10,
    };
    let limit = protocol.events.len() * 4 + protocol.samples.len() + 2;
    let budgets = qspquest_modeling::Budgets {
        output_values: limit,
        ..Default::default()
    };
    assert!(qspquest_modeling::simulate_with_budgets(&bound, &protocol, budgets).is_ok());
    let error = bound
        .simulate_outputs(
            &protocol,
            qspquest_modeling::Budgets {
                output_values: limit,
                ..Default::default()
            },
        )
        .unwrap_err();
    assert_eq!(error.code, qspquest_core::ErrorCode::OutputBudget);
    assert!(error.message.contains("derived outputs"));
    d.parameters.push(Parameter {
        name: "mass_ref".into(),
        default: q(1.0, "mg"),
        bounds: None,
        transform: Transform::Log,
        fixed: true,
        description: "".into(),
    });
    d.outputs[0].unit = "1".into();
    d.outputs[0].expression = qspquest_core::expression::Expr::call(
        qspquest_core::expression::Function::Log,
        vec![qspquest_core::expression::Expr::binary(
            qspquest_core::expression::Binary::Divide,
            qspquest_core::expression::Expr::symbol("central"),
            qspquest_core::expression::Expr::symbol("mass_ref"),
        )],
    );
    let bound = CompiledDocument::compile(&d)
        .unwrap()
        .bind(&BTreeMap::new())
        .unwrap();
    let error = bound
        .simulate_outputs(&protocol, Default::default())
        .unwrap_err();
    assert_eq!(error.code, qspquest_core::ErrorCode::Domain);
    assert_eq!(error.time, Some(0.0));
    assert!(error.expression.unwrap().starts_with("outputs.cp."));
}

#[test]
fn bdf_dense_observations_match_analytic_decay_near_stop() {
    let model: ModelDocument = serde_json::from_str(r#"{"schema":"qspquest.model/v0.1","model_id":"synthetic_migration","description":"","time_unit":"h","parameters":[{"name":"cl","default":{"value":0.3,"unit":"L/h"},"bounds":null,"transform":"identity","fixed":false,"description":""},{"name":"v","default":{"value":3.0,"unit":"L"},"bounds":null,"transform":"identity","fixed":false,"description":""}],"states":[{"name":"a","unit":"mg","initial":{"kind":"quantity","quantity":{"value":6.0,"unit":"mg"}},"rhs":{"kind":"binary","operator":"subtract","arguments":[{"kind":"symbol","name":"input_a"},{"kind":"binary","operator":"multiply","arguments":[{"kind":"symbol","name":"cl"},{"kind":"symbol","name":"c"}]}]},"dosing":{"modes":["bolus","infusion"],"amount_unit":"mg","scale":{"kind":"literal","value":1.0}}}],"covariates":[],"definitions":[{"name":"c","expression":{"kind":"binary","operator":"divide","arguments":[{"kind":"symbol","name":"a"},{"kind":"symbol","name":"v"}]},"unit":null}],"outputs":[{"name":"c","unit":"mg/L","expression":{"kind":"symbol","name":"c"}}],"invariants":[]}"#).unwrap();
    let compiled = CompiledDocument::compile(&model).unwrap();
    let mut request: qspquest_core::run::RunRequest = serde_json::from_str(r#"{"schema":"qspquest.run/v0.1","solver":"diffsol_bdf","budgets":{"solver_callbacks":1000000,"output_values":10000,"events":100},"parameters":{"cl":{"value":0.5,"unit":"L/h"},"v":{"value":4.0,"unit":"L"}},"initial_states":{"a":{"value":4.0,"unit":"mg"}},"regimen":{"start":{"value":0.0,"unit":"h"},"end":{"value":2.0,"unit":"h"},"samples":[],"observations":[{"time":{"value":0.0,"unit":"h"},"side":"post"},{"time":{"value":0.25,"unit":"h"},"side":"post"},{"time":{"value":0.5,"unit":"h"},"side":"post"},{"time":{"value":0.75,"unit":"h"},"side":"post"},{"time":{"value":1.0,"unit":"h"},"side":"post"},{"time":{"value":1.25,"unit":"h"},"side":"post"},{"time":{"value":1.5,"unit":"h"},"side":"post"},{"time":{"value":1.75,"unit":"h"},"side":"post"},{"time":{"value":2.0,"unit":"h"},"side":"pre"}],"checkpoints":[],"administrations":[{"target":"a","time":{"value":0.0,"unit":"h"},"amount":{"value":2.0,"unit":"mg"},"delivery":{"kind":"bolus"},"lag":{"value":0.0,"unit":"h"},"bioavailability":1.0,"repeat":null}],"covariates":{},"covariate_changes":[],"rtol":1e-06,"atol":1e-08}}"#).unwrap();
    for (cl, v) in [
        (0.4909339300778936, 3.978133574581507),
        (0.48848538688661186, 3.958292550603993),
    ] {
        request.parameters.insert("cl".into(), q(cl, "L/h"));
        request.parameters.insert("v".into(), q(v, "L"));
        let result = compiled.execute(&request).unwrap();
        for (time, concentration) in result.times.iter().zip(&result.outputs[0].values) {
            let expected = 6. / v * libm::exp(-cl * time / v);
            assert!(
                (concentration - expected).abs() < 2e-6,
                "dense concentration at {time}: {concentration} vs {expected}"
            );
        }
    }
}

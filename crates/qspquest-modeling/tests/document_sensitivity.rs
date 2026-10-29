use qspquest_core::{
    expression::{Binary, Expr},
    model::*,
};
use qspquest_modeling::{
    document::{sensitivity::CompiledSensitivityDocument, CompiledDocument},
    simulate, Event, Model, Protocol,
};
use std::collections::BTreeMap;
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
fn source() -> ModelDocument {
    let mut d: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    d.parameters[0].name = "theta".into();
    d.covariates = vec![Covariate {
        name: "weight_ratio".into(),
        unit: "1".into(),
        interpolation: Interpolation::Constant,
        default: Some(q(1.2, "1")),
    }];
    d.individual = vec![IndividualParameter {
        name: "cl".into(),
        expression: Expr::binary(
            Binary::Multiply,
            Expr::symbol("theta"),
            Expr::symbol("weight_ratio"),
        ),
        unit: Some("mL/min".into()),
    }];
    d.definitions = vec![Definition {
        name: "concentration".into(),
        expression: d.outputs[0].expression.clone(),
        unit: Some("mg/L".into()),
    }];
    d.outputs[0].expression = Expr::symbol("concentration");
    d
}
fn protocol() -> Protocol {
    Protocol {
        start: 0.,
        end: 8.,
        samples: vec![0.5, 2., 4., 8.],
        events: vec![
            Event::Bolus {
                time: 0.,
                target: 0,
                amount: 6.,
            },
            Event::Infusion {
                time: 1.,
                target: 0,
                amount: 4.,
                duration: 2.,
            },
        ],
        rtol: 1e-10,
        atol: 1e-12,
    }
}
#[test]
fn typed_document_derivatives_match_ordinary_document_perturbations() {
    let d = source();
    let names = vec!["theta".into(), "v".into()];
    let compiled = CompiledSensitivityDocument::compile(&d, &names).unwrap();
    let base = CompiledDocument::compile(&d).unwrap();
    assert_eq!(compiled.output_names(), ["cp"]);
    assert_eq!(compiled.output_units(), ["mg/L"]);
    assert_eq!(compiled.parameters(), names);
    let mut bound = compiled.bind(&BTreeMap::new(), &BTreeMap::new()).unwrap();
    let rows = simulate(&bound, &protocol()).unwrap();
    for (p, name, value, unit) in [(0, "theta", 0.3, "L/h"), (1, "v", 3., "L")] {
        let h = 1e-4;
        let hi = base
            .bind(&BTreeMap::from([(name.into(), q(value + h, unit))]))
            .unwrap();
        let lo = base
            .bind(&BTreeMap::from([(name.into(), q(value - h, unit))]))
            .unwrap();
        let high = simulate(&hi, &protocol()).unwrap();
        let low = simulate(&lo, &protocol()).unwrap();
        for ((row, high), low) in rows.iter().zip(high).zip(low) {
            let fd = (high.values[0] - low.values[0]) / (2. * h);
            assert!((row.values[p + 1] - fd).abs() < 1e-5);
            let actual = bound.outputs(row.time, &row.values, &[0.]).unwrap();
            let fd = (hi.outputs(high.time, &high.values).unwrap()[0]
                - lo.outputs(low.time, &low.values).unwrap()[0])
                / (2. * h);
            assert!((actual.derivatives[p][0] - fd).abs() < 1e-5);
        }
    }
    let before = bound.initial();
    assert!(bound
        .rebind(
            &BTreeMap::from([("v".into(), q(-1., "L"))]),
            &BTreeMap::new()
        )
        .is_err());
    assert_eq!(bound.initial(), before);
    bound
        .rebind(
            &BTreeMap::from([("v".into(), q(3000., "mL"))]),
            &BTreeMap::from([("weight_ratio".into(), q(1.2, "1"))]),
        )
        .unwrap();
    let again = simulate(&bound, &protocol()).unwrap();
    for (a, b) in rows.iter().zip(again) {
        for (x, y) in a.values.iter().zip(b.values) {
            assert!((x - y).abs() < 1e-10);
        }
    }
}
#[test]
fn document_sensitivity_preserves_delivery_rules_and_rejects_unqualified_features() {
    let mut d = source();
    d.states[0].dosing.as_mut().unwrap().modes = vec![DeliveryMode::Bolus];
    let c = CompiledSensitivityDocument::compile(&d, &["theta".into()]).unwrap();
    let bound = c.bind(&BTreeMap::new(), &BTreeMap::new()).unwrap();
    assert!(simulate(&bound, &protocol()).is_err());
    assert!(CompiledSensitivityDocument::compile(&d, &["cl".into()]).is_err());
    d.states[0].dosing.as_mut().unwrap().lag = Some(Expr::symbol("lag"));
    d.parameters.push(Parameter {
        name: "lag".into(),
        default: q(1., "h"),
        bounds: None,
        transform: Transform::Identity,
        fixed: false,
        description: String::new(),
    });
    assert!(CompiledSensitivityDocument::compile(&d, &["theta".into()]).is_ok());
    let error = CompiledSensitivityDocument::compile(&d, &["lag".into()]).unwrap_err();
    assert_eq!(error.code, qspquest_core::ErrorCode::Unsupported);
}

#[test]
fn nonnegative_invariants_apply_to_primal_states_only() {
    let mut d = source();
    d.invariants.push(Invariant::Nonnegative {
        states: vec!["central".into()],
    });
    let c = CompiledSensitivityDocument::compile(&d, &["theta".into()]).unwrap();
    let bound = c.bind(&BTreeMap::new(), &BTreeMap::new()).unwrap();
    assert!(bound.check_state(0., &[1., -5.]).is_ok());
    assert!(bound.check_state(0., &[-1., 5.]).is_err());
    assert!(bound.check_state(0., &[1.]).is_err());
}

#[test]
fn projected_sensitivities_match_eventful_full_simulations_and_enforce_budget() {
    let mut d = source();
    d.parameters.push(Parameter {
        name: "f".into(),
        default: q(0.6, "1"),
        bounds: None,
        transform: Transform::Identity,
        fixed: false,
        description: String::new(),
    });
    d.states[0].dosing.as_mut().unwrap().scale = Expr::symbol("f");
    let compiled = CompiledSensitivityDocument::compile(&d, &["f".into()]).unwrap();
    let base = CompiledDocument::compile(&d).unwrap();
    let mut p = protocol();
    p.events.push(Event::Reset {
        time: 2.,
        target: 0,
        value: 2.,
        order: 0,
        active_inputs: qspquest_core::ActiveInputs::StopTarget,
    });
    p.events.push(Event::Infusion {
        time: 2.,
        target: 0,
        amount: 5.,
        duration: 3.,
    });
    for value in [0., 0.6] {
        let overrides = BTreeMap::from([("f".into(), q(value, "1"))]);
        let bound = compiled.bind(&overrides, &BTreeMap::new()).unwrap();
        let rows = bound
            .simulate_outputs(&p, qspquest_modeling::Budgets::default())
            .unwrap();
        let h = 1e-4;
        let high = base
            .bind(&BTreeMap::from([("f".into(), q(value + h, "1"))]))
            .unwrap();
        let low = base.bind(&overrides).unwrap();
        let hi = simulate(&high, &p).unwrap();
        let lo = simulate(&low, &p).unwrap();
        for ((row, hi), lo) in rows.iter().zip(hi).zip(lo) {
            let high = high.outputs(hi.time, &hi.values).unwrap()[0];
            let low = low.outputs(lo.time, &lo.values).unwrap()[0];
            assert_eq!((row.time, row.side.as_str()), (hi.time, hi.side.as_str()));
            assert!((row.values[0] - low).abs() < 1e-7);
            assert!((row.derivatives[0][0] - (high - low) / h).abs() < 1e-5);
        }
        let budgets = qspquest_modeling::Budgets {
            output_values: rows.len() * 2 + 1,
            ..qspquest_modeling::Budgets::default()
        };
        assert!(bound.simulate_outputs(&p, budgets).is_err());
        assert!(bound
            .simulate_outputs(&p, qspquest_modeling::Budgets::default())
            .is_ok());
    }
}

use qspquest_core::{regimen::*, ActiveInputs};
fn point(value: f64, unit: &str, side: ObservationSide) -> ObservationPoint {
    ObservationPoint {
        time: q(value, unit),
        side,
    }
}
fn sensitivity_regimen() -> Request {
    Request {
        checkpoints: vec![],
        start: None,
        covariates: Default::default(),
        covariate_changes: vec![],
        end: q(4.0, "h"),
        samples: vec![],
        observations: Some(vec![
            point(120.0, "min", ObservationSide::Pre),
            point(2.0, "h", ObservationSide::Post),
            point(1.0, "h", ObservationSide::Pre),
            point(1.0, "h", ObservationSide::Post),
            point(120.0, "min", ObservationSide::Post),
        ]),
        administrations: vec![
            Administration {
                target: "central".into(),
                time: q(0.0, "h"),
                amount: q(6.0, "mg"),
                delivery: Delivery::Bolus,
                repeat: None,
                lag: None,
                bioavailability: 1.0,
            },
            Administration {
                target: "central".into(),
                time: q(2.0, "h"),
                amount: q(3.0, "mg"),
                delivery: Delivery::Bolus,
                repeat: None,
                lag: None,
                bioavailability: 1.0,
            },
        ],
        resets: vec![StateReset {
            target: "central".into(),
            time: q(2.0, "h"),
            value: q(3000.0, "ug"),
            order: 0,
            active_inputs: ActiveInputs::Continue,
        }],
        rtol: 1e-10,
        atol: Some(1e-12),
        absolute_tolerances: None,
    }
}

#[test]
fn typed_regimen_preserves_observation_order_and_quantity_tolerances() {
    let d = source();
    let c = CompiledSensitivityDocument::compile(&d, &["theta".into(), "v".into()]).unwrap();
    let base = CompiledDocument::compile(&d).unwrap();
    let mut request = sensitivity_regimen();
    request.checkpoints = vec![q(1., "h")];
    request.atol = None;
    request.absolute_tolerances = Some(BTreeMap::from([("central".into(), q(1e-9, "ug"))]));
    request.administrations.push(Administration {
        target: "central".into(),
        time: q(0.5, "h"),
        amount: q(6., "mg"),
        delivery: Delivery::Infusion {
            span: InfusionSpan::Duration {
                duration: q(3., "h"),
            },
        },
        repeat: None,
        lag: None,
        bioavailability: 0.7,
    });
    request
        .observations
        .as_mut()
        .unwrap()
        .push(point(3., "h", ObservationSide::Pre));
    let tolerances = BTreeMap::from([
        (
            "theta".into(),
            BTreeMap::from([("central".into(), q(1e-9, "ug.h/L"))]),
        ),
        (
            "v".into(),
            BTreeMap::from([("central".into(), q(1e-12, "mg/L"))]),
        ),
    ]);
    let rows = c
        .simulate_regimen(&BTreeMap::new(), &request, &tolerances, Default::default())
        .unwrap();
    assert_eq!(rows.len(), 6);
    assert_eq!(
        rows.iter()
            .map(|r| (r.time, r.side.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (2., "pre"),
            (2., "post"),
            (1., "pre"),
            (1., "post"),
            (2., "post"),
            (3., "pre")
        ]
    );
    assert_eq!(rows[1].derivatives, rows[4].derivatives);
    for (p, name, value, unit) in [(0, "theta", 0.3, "L/h"), (1, "v", 3., "L")] {
        let h = 1e-4;
        let high = base
            .bind(&BTreeMap::from([(name.into(), q(value + h, unit))]))
            .unwrap()
            .simulate_regimen(&request, Default::default())
            .unwrap();
        let low = base
            .bind(&BTreeMap::from([(name.into(), q(value - h, unit))]))
            .unwrap()
            .simulate_regimen(&request, Default::default())
            .unwrap();
        for ((r, hi), lo) in rows.iter().zip(high).zip(low) {
            assert!((r.derivatives[p][0] - (hi.values[0] - lo.values[0]) / (2. * h)).abs() < 1e-5);
        }
    }
    let mut bad = tolerances.clone();
    bad.get_mut("theta")
        .unwrap()
        .get_mut("central")
        .unwrap()
        .unit = "mg".into();
    assert!(c
        .simulate_regimen(&BTreeMap::new(), &request, &bad, Default::default())
        .is_err());
    let mut bad = tolerances.clone();
    bad.remove("v");
    assert!(c
        .simulate_regimen(&BTreeMap::new(), &request, &bad, Default::default())
        .is_err());
    let mut bad = tolerances.clone();
    bad.get_mut("v").unwrap().get_mut("central").unwrap().value = 0.;
    assert!(c
        .simulate_regimen(&BTreeMap::new(), &request, &bad, Default::default())
        .is_err());
    assert!(c
        .simulate_regimen(&BTreeMap::new(), &request, &tolerances, Default::default())
        .is_ok());
}

#[test]
fn sensitivity_observations_do_not_evaluate_unrequested_output_domains() {
    let mut d = source();
    d.parameters.push(Parameter {
        name: "reference_amount".into(),
        default: q(1., "mg"),
        bounds: None,
        transform: Transform::Log,
        fixed: true,
        description: String::new(),
    });
    d.outputs[0].unit = "1".into();
    d.outputs[0].expression = Expr::call(
        qspquest_core::expression::Function::Log,
        vec![Expr::binary(
            Binary::Divide,
            Expr::symbol("central"),
            Expr::symbol("reference_amount"),
        )],
    );
    let c = CompiledSensitivityDocument::compile(&d, &["theta".into()]).unwrap();
    let mut request = sensitivity_regimen();
    request.observations = Some(vec![point(1., "h", ObservationSide::Post)]);
    let tolerances = BTreeMap::from([(
        "theta".into(),
        BTreeMap::from([("central".into(), q(1e-12, "mg.h/L"))]),
    )]);
    assert_eq!(
        c.simulate_regimen(&BTreeMap::new(), &request, &tolerances, Default::default())
            .unwrap()
            .len(),
        1
    );
    request.observations = None;
    assert!(c
        .simulate_regimen(&BTreeMap::new(), &request, &tolerances, Default::default())
        .is_err());
}

fn sensitivity_request() -> qspquest_core::run::SensitivityRequest {
    use qspquest_core::run::*;
    SensitivityRequest {
        schema: SensitivitySchema::V01,
        run: RunRequest {
            schema: RunSchema::V01,
            request_id: Some("sensitivity-fixture".into()),
            seed: None,
            solver: Solver::DiffsolBdf,
            accuracy_control: Default::default(),
            budgets: Default::default(),
            parameters: Default::default(),
            initial_states: Default::default(),
            regimen: sensitivity_regimen(),
        },
        with_respect_to: vec!["theta".into(), "v".into()],
        derivative_absolute_tolerances: BTreeMap::from([
            (
                "theta".into(),
                BTreeMap::from([("central".into(), q(1e-12, "mg.h/L"))]),
            ),
            (
                "v".into(),
                BTreeMap::from([("central".into(), q(1e-12, "mg/L"))]),
            ),
        ]),
    }
}
#[test]
fn sensitivity_envelopes_bind_identity_units_order_and_controls() {
    use qspquest_core::run::*;
    let d = source();
    let mut request = sensitivity_request();
    let c = CompiledSensitivityDocument::compile(&d, &request.with_respect_to).unwrap();
    let result = c.run(&request).unwrap();
    assert_eq!(result.result.times, vec![2., 2., 1., 1., 2.]);
    assert_eq!(result.derivatives.len(), 2);
    assert_eq!(result.derivatives[0].parameter, "theta");
    assert_eq!(result.derivatives[0].output, "cp");
    assert_eq!(result.derivatives[0].unit, "(mg/L)/(L/h)");
    assert_eq!(
        result.result.identity.algorithm,
        "bdf_augmented_forward_sensitivity"
    );
    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["schema"], "qspquest.sensitivity-result/v0.1");
    let decoded: SensitivityResult = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.result.identity, result.result.identity);
    request.run.request_id = Some("another-correlation".into());
    request.run.parameters.insert("v".into(), q(3000., "mL"));
    let equivalent = c.run(&request).unwrap();
    assert_eq!(equivalent.result.identity, result.result.identity);
    assert_eq!(equivalent.result.request_id, request.run.request_id);
    request.run.parameters.insert("v".into(), q(6., "L"));
    let changed = c.run(&request).unwrap();
    assert_ne!(
        changed.result.identity.bound_value_hash,
        result.result.identity.bound_value_hash
    );
    assert_eq!(
        changed.result.identity.run_request_hash,
        result.result.identity.run_request_hash
    );
    request
        .derivative_absolute_tolerances
        .get_mut("v")
        .unwrap()
        .get_mut("central")
        .unwrap()
        .value = 1e-10;
    let changed_tolerance = c.run(&request).unwrap();
    assert_ne!(
        changed_tolerance.result.identity.run_request_hash,
        changed.result.identity.run_request_hash
    );
    request.with_respect_to.reverse();
    assert!(c.run(&request).is_err());
    let reordered = CompiledSensitivityDocument::compile(&d, &request.with_respect_to)
        .unwrap()
        .run(&request)
        .unwrap();
    assert_eq!(reordered.derivatives[0].parameter, "v");
    assert_ne!(
        reordered.result.identity.run_request_hash,
        changed_tolerance.result.identity.run_request_hash
    );
}
#[test]
fn sensitivity_envelopes_reject_unknown_fields_and_unsupported_execution() {
    use qspquest_core::run::*;
    let request = sensitivity_request();
    let c = CompiledSensitivityDocument::compile(&source(), &request.with_respect_to).unwrap();
    let mut json = serde_json::to_value(&request).unwrap();
    json["random_effects"] = serde_json::json!({});
    assert!(serde_json::from_value::<SensitivityRequest>(json).is_err());
    for mode in 0..4 {
        let mut bad = request.clone();
        match mode {
            0 => bad.run.seed = Some(1),
            1 => bad.run.solver = Solver::AnalyticLinearPk,
            2 => {
                bad.run
                    .initial_states
                    .insert("unknown_state".into(), q(1., "mg"));
            }
            _ => bad.run.request_id = Some(String::new()),
        };
        assert!(c.run(&bad).is_err());
    }
    assert!(c.run(&request).is_ok());
}

#[test]
fn serialized_sensitivity_fixture_matches_closed_form() {
    use qspquest_core::run::*;
    let d: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let request: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    let result = CompiledSensitivityDocument::compile(&d, &request.with_respect_to)
        .unwrap()
        .run(&request)
        .unwrap();
    for (i, t) in result.result.times.iter().enumerate() {
        let c = if *t == 0. && matches!(result.result.sides[i], ObservationSide::Pre) {
            0.
        } else {
            2. * (-0.1 * t).exp()
        };
        assert!((result.result.outputs[0].values[i] - c).abs() < 1e-8);
        assert!((result.derivatives[0].values[i] + t / 3. * c).abs() < 1e-8);
        assert!((result.derivatives[1].values[i] - c * (-1. / 3. + 0.3 * t / 9.)).abs() < 1e-8);
    }
    if let Ok(path) = std::env::var("QSPQUEST_SENSITIVITY_RESULT") {
        std::fs::write(path, serde_json::to_string_pretty(&result).unwrap()).unwrap();
    }
}

fn initial_expression_source() -> ModelDocument {
    let mut d = source();
    d.parameters[0].transform = Transform::Identity;
    for (name, value, unit) in [("a0", 2., "mg"), ("theta_ref", 0.3, "L/h")] {
        d.parameters.push(Parameter {
            name: name.into(),
            default: q(value, unit),
            bounds: None,
            transform: Transform::Identity,
            fixed: true,
            description: String::new(),
        });
    }
    let ratio = Expr::binary(
        Binary::Divide,
        Expr::symbol("theta"),
        Expr::symbol("theta_ref"),
    );
    d.states[0].initial = Initial::Expression {
        expression: Expr::binary(
            Binary::Multiply,
            Expr::symbol("a0"),
            Expr::call(
                qspquest_core::expression::Function::Sqrt,
                vec![ratio.clone()],
            ),
        ),
    };
    let mut second = d.states[0].clone();
    second.name = "second".into();
    second.dosing = None;
    second.initial = Initial::Expression {
        expression: Expr::binary(Binary::Multiply, Expr::symbol("a0"), ratio),
    };
    second.rhs = Expr::binary(
        Binary::Multiply,
        Expr::number(-1.),
        Expr::binary(
            Binary::Multiply,
            Expr::binary(Binary::Divide, Expr::symbol("cl"), Expr::symbol("v")),
            Expr::symbol("second"),
        ),
    );
    d.states.push(second);
    d
}
#[test]
fn partial_initial_overrides_zero_only_replaced_initial_derivatives() {
    let d = initial_expression_source();
    let c = CompiledSensitivityDocument::compile(&d, &["theta".into(), "v".into()]).unwrap();
    let initial = BTreeMap::from([("central".into(), q(3000., "ug"))]);
    let bound = c
        .bind_with_initial_states(&BTreeMap::new(), &BTreeMap::new(), &initial)
        .unwrap();
    let mut bound = bound;
    let original = bound.initial();
    assert!(bound
        .rebind(
            &BTreeMap::from([("v".into(), q(-1., "L"))]),
            &BTreeMap::new()
        )
        .is_err());
    assert_eq!(bound.initial(), original);
    bound.rebind(&BTreeMap::new(), &BTreeMap::new()).unwrap();
    let values = bound.initial();
    assert_eq!(&values[..3], &[3., 2., 0.]);
    assert!((values[3] - 2. / 0.3).abs() < 1e-12);
    assert_eq!(&values[4..], &[0., 0.]);
    let zero = BTreeMap::from([("theta".into(), q(0., "L/h"))]);
    assert!(c.bind(&zero, &BTreeMap::new()).is_err());
    let bound = c
        .bind_with_initial_states(&zero, &BTreeMap::new(), &initial)
        .unwrap();
    assert_eq!(bound.initial()[2], 0.);
    assert!((bound.initial()[3] - 2. / 0.3).abs() < 1e-12);
    let bad = BTreeMap::from([("central".into(), q(3., "L"))]);
    assert!(c
        .bind_with_initial_states(&zero, &BTreeMap::new(), &bad)
        .is_err());
}
#[test]
fn initial_override_runs_match_perturbed_primal_and_bind_identity() {
    let d = initial_expression_source();
    let mut request = sensitivity_request();
    request.run.initial_states = BTreeMap::from([("central".into(), q(3., "mg"))]);
    request.run.parameters = BTreeMap::from([("theta".into(), q(0., "L/h"))]);
    for states in request.derivative_absolute_tolerances.values_mut() {
        let q = states["central"].clone();
        states.insert("second".into(), q);
    }
    let c = CompiledSensitivityDocument::compile(&d, &request.with_respect_to).unwrap();
    let base = CompiledDocument::compile(&d).unwrap();
    let result = c.run(&request).unwrap();
    for (p, name, value, unit) in [(0, "theta", 0., "L/h"), (1, "v", 3., "L")] {
        let h = 1e-5;
        let mut hi = request.run.parameters.clone();
        let mut lo = hi.clone();
        hi.insert(name.into(), q(value + h, unit));
        lo.insert(name.into(), q(value - h, unit));
        let high = base
            .bind_with_initial_states(&hi, &BTreeMap::new(), &request.run.initial_states)
            .unwrap()
            .simulate_regimen(&request.run.regimen, Default::default())
            .unwrap();
        let low = base
            .bind_with_initial_states(&lo, &BTreeMap::new(), &request.run.initial_states)
            .unwrap()
            .simulate_regimen(&request.run.regimen, Default::default())
            .unwrap();
        for (i, (hi, lo)) in high.iter().zip(low).enumerate() {
            assert!(
                (result.derivatives[p].values[i] - (hi.values[0] - lo.values[0]) / (2. * h)).abs()
                    < 1e-5
            );
        }
    }
    request
        .run
        .initial_states
        .insert("central".into(), q(3000., "ug"));
    assert_eq!(
        c.run(&request).unwrap().result.identity,
        result.result.identity
    );
    request
        .run
        .initial_states
        .insert("central".into(), q(4., "mg"));
    assert_ne!(
        c.run(&request).unwrap().result.identity.bound_value_hash,
        result.result.identity.bound_value_hash
    );
}

#[test]
fn equal_primal_initials_with_different_derivatives_have_distinct_identity() {
    let d = initial_expression_source();
    let mut request = sensitivity_request();
    for states in request.derivative_absolute_tolerances.values_mut() {
        states.insert("second".into(), states["central"].clone());
    }
    let c = CompiledSensitivityDocument::compile(&d, &request.with_respect_to).unwrap();
    let derived = c.run(&request).unwrap();
    request
        .run
        .initial_states
        .insert("central".into(), q(2., "mg"));
    let fixed = c.run(&request).unwrap();
    assert_ne!(
        derived.result.identity.bound_value_hash,
        fixed.result.identity.bound_value_hash
    );
    assert_eq!(
        derived.result.identity.run_request_hash,
        fixed.result.identity.run_request_hash
    );
    for (a, b) in derived.result.outputs[0]
        .values
        .iter()
        .zip(&fixed.result.outputs[0].values)
    {
        assert!((a - b).abs() < 1e-7);
    }
    assert!((derived.derivatives[0].values[0] - fixed.derivatives[0].values[0]).abs() > 0.1);
}

#[test]
fn scheduled_covariates_preserve_sensitivities_and_event_side_outputs() {
    let mut d = source();
    d.covariates[0].interpolation = Interpolation::Step;
    for (name, value, unit) in [("f", 0.6, "1"), ("a0", 2., "mg")] {
        d.parameters.push(Parameter {
            name: name.into(),
            default: q(value, unit),
            bounds: None,
            transform: Transform::Identity,
            fixed: false,
            description: String::new(),
        });
    }
    d.states[0].initial = Initial::Expression {
        expression: Expr::binary(
            Binary::Multiply,
            Expr::symbol("a0"),
            Expr::symbol("weight_ratio"),
        ),
    };
    d.states[0].dosing.as_mut().unwrap().scale = Expr::binary(
        Binary::Multiply,
        Expr::symbol("f"),
        Expr::symbol("weight_ratio"),
    );
    d.outputs.push(Output {
        name: "scaled_cp".into(),
        unit: "mg/L".into(),
        expression: Expr::binary(
            Binary::Multiply,
            Expr::symbol("concentration"),
            Expr::symbol("weight_ratio"),
        ),
    });
    let mut request = sensitivity_request();
    request.run.budgets.output_values = 100000;
    request.with_respect_to.extend(["f".into(), "a0".into()]);
    request.derivative_absolute_tolerances.insert(
        "f".into(),
        BTreeMap::from([("central".into(), q(1e-12, "mg"))]),
    );
    request.derivative_absolute_tolerances.insert(
        "a0".into(),
        BTreeMap::from([("central".into(), q(1e-12, "1"))]),
    );
    request.run.regimen.covariate_changes = vec![(2., 0.), (0., 0.6), (1., 2.)]
        .into_iter()
        .map(|(t, v)| CovariateChange {
            name: "weight_ratio".into(),
            time: q(t, "h"),
            value: q(v, "1"),
        })
        .collect();
    request.run.regimen.administrations.push(Administration {
        target: "central".into(),
        time: q(0.5, "h"),
        amount: q(6., "mg"),
        delivery: Delivery::Infusion {
            span: InfusionSpan::Duration {
                duration: q(3., "h"),
            },
        },
        repeat: None,
        lag: None,
        bioavailability: 1.,
    });
    request.run.regimen.observations.as_mut().unwrap().extend([
        point(0., "h", ObservationSide::Pre),
        point(0., "h", ObservationSide::Post),
        point(3., "h", ObservationSide::Pre),
    ]);
    let c = CompiledSensitivityDocument::compile(&d, &request.with_respect_to).unwrap();
    let primal = CompiledDocument::compile(&d).unwrap();
    let result = c.run(&request).unwrap();
    let reference = primal.execute(&request.run).unwrap();
    for (o, r) in result.result.outputs.iter().zip(&reference.outputs) {
        for (a, b) in o.values.iter().zip(&r.values) {
            assert!((a - b).abs() < 1e-7);
        }
    }
    for (p, parameter) in d.parameters.iter().enumerate() {
        let h = 1e-4;
        let mut hi = request.run.clone();
        let mut lo = hi.clone();
        hi.parameters.insert(
            parameter.name.clone(),
            q(parameter.default.value + h, &parameter.default.unit),
        );
        lo.parameters.insert(
            parameter.name.clone(),
            q(parameter.default.value - h, &parameter.default.unit),
        );
        let high = primal.execute(&hi).unwrap();
        let low = primal.execute(&lo).unwrap();
        for o in 0..d.outputs.len() {
            for (i, (high, low)) in high.outputs[o]
                .values
                .iter()
                .zip(&low.outputs[o].values)
                .enumerate()
            {
                assert!(
                    (result.derivatives[p * d.outputs.len() + o].values[i]
                        - (high - low) / (2. * h))
                        .abs()
                        < 2e-5,
                    "parameter {} output {o} observation {i}",
                    parameter.name
                );
            }
        }
    }
    // At t=1 only the output's covariate factor jumps; the state sensitivity stays continuous.
    assert!((result.derivatives[0].values[2] - result.derivatives[0].values[3]).abs() < 1e-12);
    assert!(
        (result.derivatives[1].values[3] - result.derivatives[1].values[2] * 2. / 0.6).abs()
            < 1e-10
    );
    for mode in 0..4 {
        let mut bad = request.clone();
        match mode {
            0 => bad
                .run
                .regimen
                .covariate_changes
                .push(bad.run.regimen.covariate_changes[0].clone()),
            1 => bad.run.regimen.covariate_changes[0].time = q(20., "h"),
            2 => bad.run.regimen.covariate_changes[0].value = q(1., "mg"),
            _ => bad.run.budgets.output_values = 100,
        };
        assert!(c.run(&bad).is_err());
    }
    let mut constant = d.clone();
    constant.covariates[0].interpolation = Interpolation::Constant;
    assert!(
        CompiledSensitivityDocument::compile(&constant, &request.with_respect_to)
            .unwrap()
            .run(&request)
            .is_err()
    );
    assert!(c.run(&request).is_ok());
}

#[test]
fn later_covariates_do_not_reevaluate_initial_derivatives() {
    let mut d = initial_expression_source();
    d.covariates[0].interpolation = Interpolation::Step;
    d.states[0].initial = Initial::Expression {
        expression: Expr::binary(
            Binary::Multiply,
            Expr::symbol("a0"),
            Expr::call(
                qspquest_core::expression::Function::Sqrt,
                vec![Expr::binary(
                    Binary::Multiply,
                    Expr::symbol("weight_ratio"),
                    Expr::binary(
                        Binary::Divide,
                        Expr::symbol("theta"),
                        Expr::symbol("theta_ref"),
                    ),
                )],
            ),
        ),
    };
    let mut request = sensitivity_request();
    request.run.budgets.output_values = 100000;
    request.run.regimen.covariate_changes = vec![CovariateChange {
        name: "weight_ratio".into(),
        time: q(1., "h"),
        value: q(0., "1"),
    }];
    for states in request.derivative_absolute_tolerances.values_mut() {
        states.insert("second".into(), states["central"].clone());
    }
    let compiled = CompiledSensitivityDocument::compile(&d, &request.with_respect_to).unwrap();
    let result = compiled.run(&request).unwrap();
    for column in &result.derivatives {
        assert!(column.values.iter().all(|x| x.is_finite()));
        assert!((column.values[2] - column.values[3]).abs() < 1e-12);
    }
    let before = compiled.bind(
        &BTreeMap::new(),
        &BTreeMap::from([("weight_ratio".into(), q(0., "1"))]),
    );
    assert!(
        before.is_err(),
        "the unused initial derivative is singular at this later covariate value"
    );
}

#[test]
fn unselected_parameter_lags_and_constant_covariates_match_perturbed_runs() {
    let mut d = source();
    d.parameters.push(Parameter {
        name: "delay".into(),
        default: q(0.5, "h"),
        bounds: None,
        transform: Transform::Identity,
        fixed: false,
        description: String::new(),
    });
    d.covariates.push(Covariate {
        name: "lag_offset".into(),
        unit: "min".into(),
        interpolation: Interpolation::Constant,
        default: Some(q(15., "min")),
    });
    d.definitions.push(Definition {
        name: "lag_base".into(),
        expression: Expr::symbol("delay"),
        unit: Some("h".into()),
    });
    d.definitions.push(Definition {
        name: "lag_total".into(),
        expression: Expr::binary(
            Binary::Add,
            Expr::symbol("lag_base"),
            Expr::symbol("lag_offset"),
        ),
        unit: Some("h".into()),
    });
    d.states[0].dosing.as_mut().unwrap().lag = Some(Expr::symbol("lag_total"));
    let error = CompiledSensitivityDocument::compile(&d, &["delay".into()]).unwrap_err();
    assert_eq!(error.code, qspquest_core::ErrorCode::Unsupported);
    assert_eq!(
        error.expression.as_deref(),
        Some("states.central.dosing.lag")
    );
    let mut request = sensitivity_request();
    request.run.regimen.administrations[0].lag = Some(q(15., "min"));
    request.run.regimen.administrations[0].repeat = Some(Repeat {
        interval: q(1., "h"),
        additional: 1,
    });
    request.run.regimen.administrations.push(Administration {
        target: "central".into(),
        time: q(1., "h"),
        amount: q(2., "mg"),
        delivery: Delivery::Infusion {
            span: InfusionSpan::Duration {
                duration: q(1., "h"),
            },
        },
        repeat: None,
        lag: None,
        bioavailability: 1.,
    });
    request.run.regimen.observations = Some(
        [0., 0.75, 1., 1.5, 1.75, 2., 2.5, 2.75, 3., 4.]
            .into_iter()
            .flat_map(|t| {
                [
                    point(t, "h", ObservationSide::Pre),
                    point(t, "h", ObservationSide::Post),
                ]
            })
            .collect(),
    );
    let c = CompiledSensitivityDocument::compile(&d, &request.with_respect_to).unwrap();
    let primal = CompiledDocument::compile(&d).unwrap();
    assert!(
        simulate(
            &c.bind(&BTreeMap::new(), &BTreeMap::new()).unwrap(),
            &protocol()
        )
        .is_err(),
        "low-level protocol must not silently ignore model lag"
    );
    for delay in [0.5, 0.25] {
        request.run.parameters = BTreeMap::from([("delay".into(), q(delay, "h"))]);
        let result = c.run(&request).unwrap();
        let reference = primal.execute(&request.run).unwrap();
        assert_eq!(result.result.times, reference.times);
        for (a, b) in result.result.outputs[0]
            .values
            .iter()
            .zip(&reference.outputs[0].values)
        {
            assert!((a - b).abs() < 1e-7);
        }
        for (p, name, value, unit) in [(0, "theta", 0.3, "L/h"), (1, "v", 3., "L")] {
            let h = 1e-4;
            let mut hi = request.run.clone();
            let mut lo = hi.clone();
            hi.parameters.insert(name.into(), q(value + h, unit));
            lo.parameters.insert(name.into(), q(value - h, unit));
            let high = primal.execute(&hi).unwrap();
            let low = primal.execute(&lo).unwrap();
            for (i, (a, b)) in high.outputs[0]
                .values
                .iter()
                .zip(&low.outputs[0].values)
                .enumerate()
            {
                assert!((result.derivatives[p].values[i] - (a - b) / (2. * h)).abs() < 1e-5);
            }
        }
    }
    request.run.parameters.insert("delay".into(), q(-1., "h"));
    assert!(c.run(&request).is_err());
    request.run.parameters.clear();
    assert!(c.run(&request).is_ok());
}

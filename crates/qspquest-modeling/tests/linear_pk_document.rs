use qspquest_core::{
    model::ModelDocument,
    run::{RunRequest, Solver},
};
use qspquest_modeling::document::CompiledDocument;
use serde_json::json;
fn model() -> ModelDocument {
    serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-linear-pk.json"
    ))
    .unwrap()
}
fn request() -> RunRequest {
    serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-linear-pk.json"
    ))
    .unwrap()
}
#[test]
fn typed_pk_backends_agree_and_identity_records_choice() {
    let m = CompiledDocument::compile(&model()).unwrap();
    let mut q = request();
    let analytic = m.execute(&q).unwrap();
    q.solver = Solver::DiffsolBdf;
    let ode = m.execute(&q).unwrap();
    assert_eq!(analytic.times, ode.times);
    assert_eq!(analytic.sides, ode.sides);
    assert_eq!(
        analytic.identity.model_content_hash,
        ode.identity.model_content_hash
    );
    assert_eq!(
        analytic.identity.bound_value_hash,
        ode.identity.bound_value_hash
    );
    assert_ne!(
        analytic.identity.run_request_hash,
        ode.identity.run_request_hash
    );
    assert_eq!(analytic.identity.backend, "qspquest");
    assert_eq!(ode.identity.backend, "diffsol");
    assert_eq!(
        analytic.identity.algorithm,
        "linear_pk_exponential_taylor20"
    );
    for (a, b) in analytic.outputs.iter().zip(ode.outputs) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.unit, b.unit);
        for (a, b) in a.values.iter().zip(b.values) {
            assert!((a - b).abs() < 1e-7, "{a} vs {b}");
        }
    }
    let mut q = request();
    q.parameters.insert(
        "cl".into(),
        serde_json::from_value(json!({"value":0.7,"unit":"L/h"})).unwrap(),
    );
    let equivalent = m.execute(&q).unwrap();
    assert_eq!(
        equivalent.identity.bound_value_hash,
        analytic.identity.bound_value_hash
    );
    for (a, b) in equivalent.outputs.iter().zip(&analytic.outputs) {
        assert_eq!(a.values, b.values);
    }
    q.parameters.insert(
        "cl".into(),
        serde_json::from_value(json!({"value":1.4,"unit":"L/h"})).unwrap(),
    );
    let changed = m.execute(&q).unwrap();
    assert_ne!(
        changed.identity.bound_value_hash,
        analytic.identity.bound_value_hash
    );
    assert_ne!(changed.outputs[1].values, analytic.outputs[1].values);
}
#[test]
fn linear_pk_rejects_invalid_declarations_and_parameter_rebindings() {
    let mut m = model();
    m.linear_pk.as_mut().unwrap().exchange_clearances.clear();
    assert!(CompiledDocument::compile(&m).is_err());
    let mut m = model();
    m.parameters[0].default.unit = "mg".into();
    assert!(CompiledDocument::compile(&m).is_err());
    let mut m = model();
    m.linear_pk.as_mut().unwrap().amount_unit = "L".into();
    assert!(CompiledDocument::compile(&m).is_err());
    let mut m = model();
    m.linear_pk.as_mut().unwrap().clearance = "time".into();
    assert!(CompiledDocument::compile(&m).is_err());
    let m = CompiledDocument::compile(&model()).unwrap();
    for (name, value) in [("cl", -1.), ("vc", 0.), ("ka", -1.)] {
        let mut q = request();
        let unit = if name == "cl" {
            "L/h"
        } else if name == "vc" {
            "L"
        } else {
            "1/h"
        };
        q.parameters.insert(
            name.into(),
            serde_json::from_value(json!({"value":value,"unit":unit})).unwrap(),
        );
        for solver in [Solver::DiffsolBdf, Solver::AnalyticLinearPk] {
            q.solver = solver;
            assert!(m.execute(&q).is_err());
        }
    }
    let mut q = request();
    q.budgets.solver_callbacks = 0;
    assert!(m.execute(&q).is_err());
    assert!(m.execute(&request()).is_ok());
    assert!(qspquest_modeling::language::format(&model()).is_ok());
}
#[test]
fn analytic_solver_never_implicitly_selects_an_ordinary_ode() {
    let m = CompiledDocument::from_json(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    assert_eq!(
        m.execute(&request()).unwrap_err().code,
        qspquest_modeling::ErrorCode::Unsupported
    );
}

#[test]
fn linear_pk_language_preserves_json_identity_and_both_backends() {
    use qspquest_modeling::language;
    for expression_initial in [false, true] {
        let mut doc = model();
        if expression_initial {
            doc.parameters.push(serde_json::from_value(json!({"name":"initial_amount","default":{"value":1,"unit":"mg"},"bounds":null,"transform":"identity","fixed":false,"description":""})).unwrap());
            doc.linear_pk
                .as_mut()
                .unwrap()
                .depot
                .as_mut()
                .unwrap()
                .initial = serde_json::from_value(
                json!({"kind":"expression","expression":{"kind":"symbol","name":"initial_amount"}}),
            )
            .unwrap();
        }
        let text = language::format(&doc).unwrap();
        let parsed = language::parse(&text).unwrap();
        assert_eq!(
            serde_json::to_value(&doc).unwrap(),
            serde_json::to_value(&parsed.document).unwrap()
        );
        assert_eq!(language::format(&parsed.document).unwrap(), text);
        assert!(parsed.locations["linear_pk.clearance"].line > 1);
        let a = CompiledDocument::compile(&doc).unwrap();
        let b = parsed.compile().unwrap();
        assert_eq!(a.model_content_hash(), b.model_content_hash());
        for solver in [Solver::AnalyticLinearPk, Solver::DiffsolBdf] {
            let mut q = request();
            q.solver = solver;
            let x = a.execute(&q).unwrap();
            let y = b.execute(&q).unwrap();
            assert_eq!(x.identity, y.identity);
            assert_eq!(x.times, y.times);
            assert_eq!(x.sides, y.sides);
            for (x, y) in x.outputs.iter().zip(y.outputs) {
                assert_eq!(x.values, y.values);
            }
        }
    }
}
#[test]
fn linear_pk_language_rejects_incomplete_or_ambiguous_declarations() {
    use qspquest_modeling::language;
    let text = language::format(&model()).unwrap();
    for (old, new) in [
        ("clearance = cl", "clearance = cl + q1"),
        ("clearance = cl", "clearance = cl\n  clearance = cl"),
        ("exchange_clearances = q1, q2", "exchange_clearances = q1,"),
        (
            "compartment central volume vc",
            "compartment central volume",
        ),
        ("amount_unit = mg", "amount_unit = unknown"),
        ("clearance = cl", "unknown = cl"),
        (
            "compartment central volume vc = 1 [mg]",
            "compartment central volume vc = 1 [mg] extra",
        ),
    ] {
        assert!(text.contains(old));
        assert!(language::parse(&text.replace(old, new)).is_err(), "{new}");
    }
    assert!(language::parse("model empty\ntime_unit = h\nlinear_pk\nend\n").is_err());
}

#[test]
fn one_compartment_text_without_depot_or_exchange_runs_closed_form() {
    use qspquest_modeling::language;
    let text = "model one\ntime_unit = h\nparameters\n  cl = 1 [L/h]\n  vc = 2 [L]\nlinear_pk\n  amount_unit = mg\n  clearance = cl\n  compartment central volume vc = 10 [mg]\noutputs\n  amount = central [mg]\nend\n";
    let parsed = language::parse(text).unwrap();
    let canonical = language::format(&parsed.document).unwrap();
    assert!(canonical.contains("exchange_clearances = \n"));
    let m = language::parse(&canonical).unwrap().compile().unwrap();
    let mut q = request();
    q.regimen.administrations.clear();
    let result = m.execute(&q).unwrap();
    for (t, a) in result.times.iter().zip(&result.outputs[0].values) {
        assert!((a - 10. * libm::exp(-0.5 * t)).abs() < 1e-10);
    }
}

#[test]
fn derived_pk_coefficients_rebind_constant_covariates_and_match_bdf() {
    let mut doc = model();
    doc.covariates.push(
        serde_json::from_value(
            json!({"name":"size_factor","unit":"1","interpolation":"constant","default":null}),
        )
        .unwrap(),
    );
    // Reverse declaration order tests use of the compiler's topological ordering.
    doc.individual=serde_json::from_value(json!([
        {"name":"effective_clearance","unit":"L/h","expression":{"kind":"symbol","name":"scaled_clearance"}},
        {"name":"scaled_clearance","unit":"mL/h","expression":{"kind":"binary","operator":"multiply","arguments":[{"kind":"symbol","name":"cl"},{"kind":"symbol","name":"size_factor"}]}}
    ])).unwrap();
    doc.linear_pk.as_mut().unwrap().clearance = "effective_clearance".into();
    let model = CompiledDocument::compile(&doc).unwrap();
    assert!(model.execute(&request()).is_err());
    let mut first = None;
    for factor in [1., 2., 1.] {
        let mut q = request();
        q.regimen.covariates.insert(
            "size_factor".into(),
            serde_json::from_value(json!({"value":factor,"unit":"1"})).unwrap(),
        );
        let a = model.execute(&q).unwrap();
        q.solver = Solver::DiffsolBdf;
        let b = model.execute(&q).unwrap();
        for (a, b) in a.outputs.iter().zip(&b.outputs) {
            for (x, y) in a.values.iter().zip(&b.values) {
                assert!((x - y).abs() < 1e-7);
            }
        }
        if factor == 1. {
            if let Some(ref values) = first {
                assert_eq!(values, &a.outputs[1].values);
            } else {
                first = Some(a.outputs[1].values.clone());
            }
        } else {
            assert_ne!(first.as_ref().unwrap(), &a.outputs[1].values);
        }
    }
    let mut bad = request();
    bad.regimen.covariates.insert(
        "size_factor".into(),
        serde_json::from_value(json!({"value":-1.,"unit":"1"})).unwrap(),
    );
    for solver in [Solver::AnalyticLinearPk, Solver::DiffsolBdf] {
        bad.solver = solver;
        assert!(model.execute(&bad).is_err());
    }
    let text = qspquest_modeling::language::format(&doc).unwrap();
    assert_eq!(
        serde_json::to_value(&doc).unwrap(),
        serde_json::to_value(qspquest_modeling::language::parse(&text).unwrap().document).unwrap()
    );
}
#[test]
fn derived_pk_coefficients_reject_transitive_time_varying_dependencies() {
    let mut doc = model();
    doc.covariates.push(serde_json::from_value(json!({"name":"changing","unit":"1","interpolation":"step","default":{"value":1,"unit":"1"}})).unwrap());
    doc.individual=serde_json::from_value(json!([
        {"name":"effective_clearance","unit":"L/h","expression":{"kind":"symbol","name":"scaled_clearance"}},
        {"name":"scaled_clearance","unit":"mL/h","expression":{"kind":"binary","operator":"multiply","arguments":[{"kind":"symbol","name":"cl"},{"kind":"symbol","name":"changing"}]}}
    ])).unwrap();
    doc.linear_pk.as_mut().unwrap().clearance = "effective_clearance".into();
    let error = CompiledDocument::compile(&doc).unwrap_err();
    assert_eq!(error.code, qspquest_modeling::ErrorCode::Unsupported);
    doc.covariates[0].interpolation = qspquest_core::model::Interpolation::Constant;
    assert!(CompiledDocument::compile(&doc).is_ok());
    doc.individual[1].expression = qspquest_core::expression::Expr::symbol("effective_clearance");
    assert!(CompiledDocument::compile(&doc).is_err());
}

#[test]
fn pk_dosing_overrides_preserve_lag_scale_modes_and_text_roundtrip() {
    let mut doc = model();
    let dose:qspquest_core::model::Dosing=serde_json::from_value(json!({"amount_unit":"mg","scale":{"kind":"literal","value":0.25},"lag":{"kind":"symbol","name":"lagtime"},"modes":["bolus"]})).unwrap();
    doc.linear_pk
        .as_mut()
        .unwrap()
        .depot
        .as_mut()
        .unwrap()
        .dosing_override = Some(dose);
    doc.parameters.push(serde_json::from_value(json!({"name":"lagtime","default":{"value":0.5,"unit":"h"},"bounds":null,"transform":"identity","fixed":false,"description":""})).unwrap());
    let m = CompiledDocument::compile(&doc).unwrap();
    let mut q = request();
    let a = m.execute(&q).unwrap();
    q.solver = Solver::DiffsolBdf;
    let b = m.execute(&q).unwrap();
    for (a, b) in a.outputs.iter().zip(b.outputs) {
        for (x, y) in a.values.iter().zip(b.values) {
            assert!((x - y).abs() < 1e-7);
        }
    }
    let text = qspquest_modeling::language::format(&doc).unwrap();
    let parsed = qspquest_modeling::language::parse(&text).unwrap();
    assert_eq!(
        serde_json::to_value(&doc).unwrap(),
        serde_json::to_value(&parsed.document).unwrap()
    );
    assert_eq!(
        m.model_content_hash(),
        parsed.compile().unwrap().model_content_hash()
    );
    let mut invalid = request();
    invalid.regimen.administrations[0].delivery = qspquest_core::regimen::Delivery::Infusion {
        span: qspquest_core::regimen::InfusionSpan::Duration {
            duration: qspquest_core::model::Quantity {
                value: 1.,
                unit: "h".into(),
            },
        },
    };
    assert!(m.execute(&invalid).is_err());
    for (i, &time) in a.times.iter().enumerate() {
        if time == 1. && matches!(a.sides[i], qspquest_core::regimen::ObservationSide::Post) {
            assert!((a.outputs[0].values[i] - (libm::exp(-0.4) + 5.)).abs() < 1e-10);
        }
    }
}

#[test]
fn typed_periodic_steady_state_preserves_units_lag_and_dose_scaling() {
    use qspquest_core::{model::Quantity, run::SteadyStateRequest};
    let mut source = model();
    let pk = source.linear_pk.as_mut().unwrap();
    pk.depot.as_mut().unwrap().dosing_override = Some(
        serde_json::from_value(json!({
            "modes":["bolus"],"amount_unit":"g","scale":{"kind":"literal","value":0.5}
        }))
        .unwrap(),
    );
    let compiled = CompiledDocument::compile(&source).unwrap();
    let mut run = request();
    run.regimen.observations = None;
    let q = SteadyStateRequest {
        run: run.clone(),
        absolute_tolerance: Quantity {
            value: 1e-12,
            unit: "g".into(),
        },
        relative_tolerance: 1e-10,
    };
    let steady = compiled.periodic_steady_state(&q).unwrap();
    assert!(steady.maximum_scaled_residual <= 1.);
    assert_eq!(
        steady.identity.algorithm,
        "linear_pk_periodic_affine_taylor20"
    );
    assert!(steady.initial_states.values().all(|q| q.unit == "mg"));
    run.initial_states = steady.initial_states.clone();
    run.regimen.samples = vec![Quantity {
        value: 8.,
        unit: "h".into(),
    }];
    let end = compiled.execute(&run).unwrap();
    for (name, initial) in &steady.initial_states {
        let output = end.outputs.iter().find(|o| o.name == *name).unwrap();
        assert!((output.values.last().unwrap() - initial.value).abs() < 1e-8);
    }
    let mut bad = q.clone();
    bad.run.initial_states = steady.initial_states.clone();
    assert!(compiled.periodic_steady_state(&bad).is_err());
    bad = q.clone();
    bad.absolute_tolerance.unit = "L".into();
    assert!(compiled.periodic_steady_state(&bad).is_err());
    bad = q.clone();
    bad.run.solver = Solver::DiffsolBdf;
    assert!(compiled.periodic_steady_state(&bad).is_err());
    let mut renamed = q.clone();
    renamed.run.request_id = Some("correlation".into());
    assert_eq!(
        compiled
            .periodic_steady_state(&renamed)
            .unwrap()
            .identity
            .run_request_hash,
        steady.identity.run_request_hash
    );
}

#[test]
fn typed_steady_state_rejects_nonperiodic_inputs_and_recovers() {
    use qspquest_core::{model::Quantity, run::SteadyStateRequest};
    let m = CompiledDocument::compile(&model()).unwrap();
    let mut run = request();
    run.regimen.observations = None;
    let q = SteadyStateRequest {
        run,
        absolute_tolerance: Quantity {
            value: 1e-9,
            unit: "mg".into(),
        },
        relative_tolerance: 1e-10,
    };
    let reference = m.periodic_steady_state(&q).unwrap();
    for index in 0..5 {
        let mut bad = q.clone();
        match index {
            0 => bad.run.budgets.solver_callbacks = 0,
            1 => bad.run.regimen.administrations[0].time.value = 8.,
            2 => bad.run.regimen.checkpoints.push(Quantity {
                value: 2.,
                unit: "h".into(),
            }),
            3 => bad.relative_tolerance = -1.,
            _ => bad.run.regimen.samples.push(Quantity {
                value: 1.,
                unit: "h".into(),
            }),
        }
        assert!(m.periodic_steady_state(&bad).is_err(), "case {index}");
    }
    let recovered = m.periodic_steady_state(&q).unwrap();
    assert_eq!(
        serde_json::to_value(reference).unwrap(),
        serde_json::to_value(recovered).unwrap()
    );
}

#[test]
fn typed_periodic_infusion_wraps_without_weakening_finite_run_validation() {
    use qspquest_core::{model::Quantity, run::SteadyStateRequest};
    let compiled = CompiledDocument::compile(&model()).unwrap();
    let mut run = request();
    run.regimen.observations = None;
    run.regimen.administrations.remove(0);
    run.regimen.administrations[0].time = Quantity {
        value: 7.,
        unit: "h".into(),
    };
    // Two-hour infusion starts one hour before the eight-hour cycle boundary.
    let q = SteadyStateRequest {
        run: run.clone(),
        absolute_tolerance: Quantity {
            value: 1e-9,
            unit: "mg".into(),
        },
        relative_tolerance: 1e-10,
    };
    let result = compiled.periodic_steady_state(&q).unwrap();
    assert!(compiled.execute(&run).is_err());
    // Independent explicit regimen contains the previous tail and current head.
    let mut explicit = run.clone();
    explicit.initial_states = result.initial_states.clone();
    let mut tail = run.regimen.administrations[0].clone();
    tail.time.value = 0.;
    tail.amount.value = 6.;
    tail.delivery = serde_json::from_value(
        json!({"kind":"infusion","span":{"kind":"duration","duration":{"value":1.,"unit":"h"}}}),
    )
    .unwrap();
    let mut head = tail.clone();
    head.time.value = 7.;
    explicit.regimen.administrations = vec![tail, head];
    explicit.regimen.samples = vec![Quantity {
        value: 8.,
        unit: "h".into(),
    }];
    let trajectory = compiled.execute(&explicit).unwrap();
    for output in trajectory.outputs {
        assert!(
            (output.values.last().unwrap() - result.initial_states[&output.name].value).abs()
                < 1e-8
        );
    }
}

#[test]
fn typed_multi_period_infusion_matches_constant_input_equilibrium() {
    use qspquest_core::{model::Quantity, run::SteadyStateRequest};
    let model = CompiledDocument::compile(&model()).unwrap();
    let mut run = request();
    run.regimen.observations = None;
    run.regimen.administrations.remove(0);
    let input = &mut run.regimen.administrations[0];
    input.time.value = 7.;
    input.amount = Quantity {
        value: 32.,
        unit: "mg".into(),
    };
    input.delivery = serde_json::from_value(
        json!({"kind":"infusion","span":{"kind":"duration","duration":{"value":16.,"unit":"h"}}}),
    )
    .unwrap();
    let q = SteadyStateRequest {
        run,
        absolute_tolerance: Quantity {
            value: 1e-9,
            unit: "mg".into(),
        },
        relative_tolerance: 1e-10,
    };
    let result = model.periodic_steady_state(&q).unwrap();
    for (name, volume) in [("central", 3.), ("peripheral", 9.), ("tissue", 18.)] {
        assert!((result.initial_states[name].value - 4. / 0.7 * volume).abs() < 1e-8);
    }
    assert!(result.initial_states["depot"].value.abs() < 1e-12);
}

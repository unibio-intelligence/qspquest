use qspquest_core::expression::{Binary, Expr, Function};
use qspquest_modeling::{
    runtime::{
        sensitivity::CompiledForwardSensitivity, CompiledModel, ModelDefinition, StateDefinition,
    },
    simulate_with_budgets, Budgets, Event, Protocol,
};
fn s(name: &str) -> Expr {
    Expr::symbol(name)
}
fn b(op: Binary, a: Expr, c: Expr) -> Expr {
    Expr::binary(op, a, c)
}
fn definition(nonlinear: bool) -> ModelDefinition {
    let loss = if nonlinear {
        b(
            Binary::Divide,
            b(Binary::Multiply, s("cl"), s("c")),
            b(Binary::Add, s("v"), s("c")),
        )
    } else {
        b(Binary::Multiply, b(Binary::Divide, s("cl"), s("v")), s("c"))
    };
    ModelDefinition {
        bindings: vec![
            "cl".into(),
            "v".into(),
            "f".into(),
            "a0".into(),
            "k2".into(),
        ],
        states: vec![
            StateDefinition {
                name: "c".into(),
                initial: b(Binary::Divide, s("a0"), s("v")),
                rhs: b(Binary::Subtract, s("input_c"), loss),
                dose_scale: Some(b(Binary::Divide, s("f"), s("v"))),
            },
            StateDefinition {
                name: "response".into(),
                initial: Expr::number(0.),
                rhs: b(
                    Binary::Subtract,
                    s("c"),
                    b(Binary::Multiply, s("k2"), s("response")),
                ),
                dose_scale: None,
            },
        ],
    }
}
fn protocol() -> Protocol {
    Protocol {
        start: 0.,
        end: 8.,
        samples: vec![0.5, 1.5, 2.5, 4.5, 7.],
        events: vec![
            Event::Bolus {
                time: 0.,
                target: 0,
                amount: 5.,
            },
            Event::Bolus {
                time: 2.,
                target: 0,
                amount: 7.,
            },
            Event::Bolus {
                time: 4.,
                target: 0,
                amount: 3.,
            },
            Event::Infusion {
                time: 1.,
                target: 0,
                amount: 12.,
                duration: 4.,
            },
            Event::Reset {
                time: 3.,
                target: 0,
                value: 2.,
                order: 0,
                active_inputs: qspquest_core::ActiveInputs::StopTarget,
            },
        ],
        rtol: 1e-10,
        atol: 1e-12,
    }
}
#[test]
fn generated_linear_and_nonlinear_trajectories_match_full_regimen_differences() {
    for nonlinear in [false, true] {
        let def = definition(nonlinear);
        let compiled = CompiledForwardSensitivity::compile(&def, &def.bindings).unwrap();
        let primal = CompiledModel::compile(&def).unwrap();
        let values = [0.7, 3., 0.6, 9., 0.5];
        let model = compiled.bind(&values).unwrap();
        let p = protocol();
        let rows = simulate_with_budgets(&model, &p, Budgets::default()).unwrap();
        for parameter in 0..values.len() {
            let h = 1e-4;
            let mut plus = values;
            let mut minus = values;
            plus[parameter] += h;
            minus[parameter] -= h;
            let high = simulate_with_budgets(&primal.bind(&plus).unwrap(), &p, Budgets::default())
                .unwrap();
            let low = simulate_with_budgets(&primal.bind(&minus).unwrap(), &p, Budgets::default())
                .unwrap();
            for ((row, high), low) in rows.iter().zip(high).zip(low) {
                assert_eq!((row.time, &row.side), (high.time, &high.side));
                for state in 0..2 {
                    let expected = (high.values[state] - low.values[state]) / (2. * h);
                    let actual = row.values[2 + parameter * 2 + state];
                    assert!((actual-expected).abs()<2e-5,"nonlinear {nonlinear}, parameter {parameter}, state {state}, time {}: {actual} vs {expected}",row.time);
                }
            }
        }
        let reset = rows
            .iter()
            .find(|r| r.time == 3. && r.side == "post")
            .unwrap();
        for p in 0..5 {
            assert_eq!(reset.values[2 + p * 2], 0.);
        }
    }
}
#[test]
fn generated_initial_and_bolus_derivatives_remain_nonzero_at_zero_delivery() {
    let def = definition(false);
    let compiled = CompiledForwardSensitivity::compile(&def, &["v".into(), "f".into()]).unwrap();
    let mut bound = compiled.bind(&[0.7, 3., 0.6, 9., 0.5]).unwrap();
    let rows = simulate_with_budgets(&bound, &protocol(), Budgets::default()).unwrap();
    assert!((rows[0].values[2] + 1.).abs() < 1e-12);
    assert!((rows[1].values[2] + 12. / 9.).abs() < 1e-12);
    assert!((rows[1].values[4] - 5. / 3.).abs() < 1e-12);
    bound.rebind(&[0.7, 3., 0., 0., 0.5]).unwrap();
    let mut p = protocol();
    p.events = vec![
        Event::Bolus {
            time: 0.,
            target: 0,
            amount: 5.,
        },
        Event::Infusion {
            time: 0.,
            target: 0,
            amount: 8.,
            duration: 8.,
        },
    ];
    for row in simulate_with_budgets(&bound, &p, Budgets::default()).unwrap() {
        let e = libm::exp(-0.7 * row.time / 3.);
        let expected = if row.time == 0. && row.side == "pre" {
            0.
        } else {
            5. / 3. * e + (1. - e) / 0.7
        };
        assert!(row.values[0].abs() < 1e-12);
        assert!((row.values[4] - expected).abs() < 1e-8);
    }
}
#[test]
fn unqualified_nonsmooth_models_and_sensitivity_reset_targets_reject() {
    let mut def = definition(false);
    def.states[0].rhs = Expr::call(Function::Abs, vec![s("c")]);
    assert!(CompiledForwardSensitivity::compile(&def, &["v".into()]).is_err());
    let def = definition(false);
    let compiled = CompiledForwardSensitivity::compile(&def, &["v".into()]).unwrap();
    let bound = compiled.bind(&[0.7, 3., 0.6, 9., 0.5]).unwrap();
    let mut p = protocol();
    p.events = vec![Event::Reset {
        time: 0.,
        target: 2,
        value: 0.,
        order: 0,
        active_inputs: qspquest_core::ActiveInputs::Continue,
    }];
    assert!(simulate_with_budgets(&bound, &p, Budgets::default()).is_err());
    assert!(simulate_with_budgets(&bound, &protocol(), Budgets::default()).is_ok());
}

#[test]
fn observable_chain_rules_match_perturbed_full_trajectories() {
    use qspquest_modeling::runtime::sensitivity::CompiledSensitivityReadouts;
    let def = definition(true);
    let names = def.bindings.clone();
    let augmented = CompiledForwardSensitivity::compile(&def, &names).unwrap();
    let primal = CompiledModel::compile(&def).unwrap();
    let expressions = vec![
        b(Binary::Multiply, s("c"), s("v")),
        b(Binary::Divide, s("c"), b(Binary::Add, s("v"), s("c"))),
        b(Binary::Add, s("response"), s("f")),
    ];
    let readouts = CompiledSensitivityReadouts::compile(&def, &names, &expressions).unwrap();
    let values = [0.7, 3., 0.6, 9., 0.5];
    let bound = readouts.bind(&values).unwrap();
    let p = protocol();
    let rows =
        simulate_with_budgets(&augmented.bind(&values).unwrap(), &p, Budgets::default()).unwrap();
    for parameter in 0..5 {
        let h = 1e-4;
        let mut high = values;
        let mut low = values;
        high[parameter] += h;
        low[parameter] -= h;
        let hi =
            simulate_with_budgets(&primal.bind(&high).unwrap(), &p, Budgets::default()).unwrap();
        let lo =
            simulate_with_budgets(&primal.bind(&low).unwrap(), &p, Budgets::default()).unwrap();
        for ((row, hi), lo) in rows.iter().zip(hi).zip(lo) {
            let result = bound.evaluate(row.time, &row.values, &[0., 0.]).unwrap();
            let observe = |x: &[f64], v: &[f64]| [x[0] * v[1], x[0] / (v[1] + x[0]), x[1] + v[2]];
            for (o, (a, b)) in observe(&hi.values, &high)
                .iter()
                .zip(observe(&lo.values, &low))
                .enumerate()
            {
                let fd = (a - b) / (2. * h);
                assert!(
                    (result.derivatives[parameter][o] - fd).abs() < 5e-5,
                    "parameter {parameter}, output {o}, time {}",
                    row.time
                );
            }
        }
    }
}
#[test]
fn input_readout_chain_rule_keeps_zero_bioavailability_derivative() {
    use qspquest_modeling::runtime::sensitivity::CompiledSensitivityReadouts;
    let def = definition(false);
    let selected = vec!["v".into(), "f".into()];
    let compiled = CompiledSensitivityReadouts::compile(
        &def,
        &selected,
        &[b(Binary::Multiply, s("input_c"), s("v"))],
    )
    .unwrap();
    let mut bound = compiled.bind(&[0.7, 3., 0.6, 0., 0.5]).unwrap();
    for f in [0.6, 0.] {
        bound.rebind(&[0.7, 3., f, 0., 0.5]).unwrap();
        let result = bound.evaluate(1., &[0.; 6], &[6., 0.]).unwrap();
        assert!((result.values[0] - 6. * f).abs() < 1e-12);
        assert!(result.derivatives[0][0].abs() < 1e-12);
        assert!((result.derivatives[1][0] - 6.).abs() < 1e-12);
    }
    assert!(bound.evaluate(0., &[0.; 5], &[0., 0.]).is_err());
}

#[test]
fn readout_domain_failure_and_failed_rebind_preserve_valid_evaluation() {
    use qspquest_modeling::runtime::sensitivity::CompiledSensitivityReadouts;
    let def = definition(false);
    let selected = vec!["v".into(), "f".into()];
    let compiled = CompiledSensitivityReadouts::compile(
        &def,
        &selected,
        &[Expr::call(Function::Log, vec![s("c")])],
    )
    .unwrap();
    let mut bound = compiled.bind(&[0.7, 3., 0.6, 0., 0.5]).unwrap();
    assert!(bound
        .evaluate(0., &[-1., 0., 1., 0., 0., 0.], &[0., 0.])
        .is_err());
    let reference = bound
        .evaluate(0., &[2., 0., 1., 0., 0., 0.], &[0., 0.])
        .unwrap();
    assert!((reference.values[0] - libm::log(2.)).abs() < 1e-12);
    assert_eq!(reference.derivatives[0][0], 0.5);
    assert!(bound.rebind(&[0.7, 0., 0.6, 0., 0.5]).is_err());
    let recovered = bound
        .evaluate(0., &[2., 0., 1., 0., 0., 0.], &[0., 0.])
        .unwrap();
    assert_eq!(recovered.values, reference.values);
    assert_eq!(recovered.derivatives, reference.derivatives);
    assert!(CompiledSensitivityReadouts::compile(
        &def,
        &selected,
        &[Expr::call(Function::Abs, vec![s("c")])]
    )
    .is_err());
}

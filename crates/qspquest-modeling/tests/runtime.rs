use qspquest_core::expression::{Binary, Expr};
use qspquest_modeling::runtime::{CompiledModel, ModelDefinition, StateDefinition};
use qspquest_modeling::{simulate, Event, Model, Protocol};
#[path = "../../../conformance/tmdd.rs"]
mod tmdd;
#[path = "../../../conformance/tmdd_vm.rs"]
mod tmdd_vm;
fn b(op: Binary, a: Expr, b: Expr) -> Expr {
    Expr::binary(op, a, b)
}
fn s(name: &str) -> Expr {
    Expr::symbol(name)
}
fn n(value: f64) -> Expr {
    Expr::number(value)
}
#[test]
fn jacobian_cache_refresh_failure_does_not_corrupt_previous_state_or_rebinding() {
    use qspquest_core::expression::Function;
    let compiled = CompiledModel::compile(&ModelDefinition {
        bindings: vec!["scale".into()],
        states: vec![
            StateDefinition {
                name: "x".into(),
                initial: n(1.0),
                dose_scale: None,
                rhs: b(
                    Binary::Multiply,
                    s("scale"),
                    b(Binary::Multiply, s("x"), s("x")),
                ),
            },
            StateDefinition {
                name: "y".into(),
                initial: n(0.0),
                dose_scale: None,
                rhs: Expr::call(Function::Log, vec![s("x")]),
            },
        ],
    })
    .unwrap();
    let mut bound = compiled.bind(&[1.0]).unwrap();
    let mut out = [0.0; 2];
    bound
        .jac_mul(0.0, &[1.0, 0.0], &[0.0; 2], &[1.0, 0.0], &mut out)
        .unwrap();
    assert_eq!(out, [2.0, 1.0]);
    // The first row can be refreshed before a later row fails its primal domain.
    assert!(bound
        .jac_mul(0.0, &[0.0, 0.0], &[0.0; 2], &[1.0, 0.0], &mut out)
        .is_err());
    bound
        .jac_mul(0.0, &[1.0, 0.0], &[0.0; 2], &[1.0, 0.0], &mut out)
        .unwrap();
    assert_eq!(out, [2.0, 1.0]);
    bound
        .jac_mul(1.0, &[2.0, 0.0], &[1.0; 2], &[1.0, 0.0], &mut out)
        .unwrap();
    assert_eq!(out, [4.0, 0.5]);
    bound.rebind(&[3.0]).unwrap();
    bound
        .jac_mul(1.0, &[2.0, 0.0], &[1.0; 2], &[1.0, 0.0], &mut out)
        .unwrap();
    assert_eq!(out, [12.0, 0.5]);
}
#[test]
fn all_eight_tmdd_vm_trajectories_match_the_handwritten_model() {
    let cases: Vec<tmdd::Case> =
        serde_json::from_str(include_str!("../../../conformance/tmdd-cases.json")).unwrap();
    let compiled = CompiledModel::compile(&tmdd_vm::definition()).unwrap();
    assert!(compiled.jacobian_sparsity().len() < 49);
    for case in cases {
        let bound = compiled.bind(&tmdd_vm::values(&case.model)).unwrap();
        let actual = simulate(&bound, &case.protocol).unwrap();
        let expected = simulate(&case.model, &case.protocol).unwrap();
        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(&expected) {
            assert_eq!((a.time, &a.side), (e.time, &e.side));
            for (x, y) in a.values.iter().zip(&e.values) {
                assert!(
                    (x - y).abs() <= 2e-6 + 2e-5 * y.abs(),
                    "{} at {}: {x} vs {y}",
                    case.id,
                    a.time
                );
            }
        }
        let x = [1.2, 0.7, 0.3, 0.9, 2.0, 1.0, 3.0];
        for column in 0..7 {
            let mut direction = [0.0; 7];
            direction[column] = 1.0;
            let mut vm = [0.0; 7];
            let mut reference = [0.0; 7];
            bound
                .jac_mul(1.0, &x, &[0.0; 7], &direction, &mut vm)
                .unwrap();
            case.model
                .jac_mul(1.0, &x, &[0.0; 7], &direction, &mut reference)
                .unwrap();
            for (a, e) in vm.iter().zip(reference) {
                assert!((a - e).abs() < 1e-10 * (1.0 + e.abs()));
            }
        }
    }
}
fn pk() -> ModelDefinition {
    let factor = b(Binary::Divide, s("wt"), n(70.0));
    ModelDefinition {
        bindings: ["v", "cl", "wt", "initial", "f"].map(String::from).to_vec(),
        states: vec![StateDefinition {
            name: "c".into(),
            initial: b(Binary::Multiply, s("initial"), factor.clone()),
            dose_scale: Some(b(Binary::Divide, s("f"), s("v"))),
            rhs: b(
                Binary::Subtract,
                s("input_c"),
                b(
                    Binary::Multiply,
                    b(
                        Binary::Divide,
                        b(Binary::Multiply, s("cl"), b(Binary::Power, factor, n(0.75))),
                        s("v"),
                    ),
                    s("c"),
                ),
            ),
        }],
    }
}
#[test]
fn rebinding_recomputes_initials_covariates_and_bolus_conversions() {
    let definition = pk();
    let compiled = CompiledModel::compile(&definition).unwrap();
    let original = [3.0, 0.3, 70.0, 2.0, 0.8];
    let mut bound = compiled.bind(&original).unwrap();
    let protocol = Protocol {
        start: 0.0,
        end: 10.0,
        samples: vec![0.1, 1.0, 3.0, 10.0],
        events: vec![
            Event::Bolus {
                time: 0.0,
                target: 0,
                amount: 12.0,
            },
            Event::Bolus {
                time: 2.0,
                target: 0,
                amount: 6.0,
            },
        ],
        rtol: 1e-8,
        atol: 1e-10,
    };
    for (index, value) in [(0, 5.0), (1, 0.8), (2, 100.0), (3, 4.0), (4, 0.5), (0, 3.0)] {
        let mut changed = original;
        changed[index] = value;
        bound.rebind(&changed).unwrap();
        let fresh = CompiledModel::compile(&definition)
            .unwrap()
            .bind(&changed)
            .unwrap();
        assert_eq!(
            simulate(&bound, &protocol).unwrap(),
            simulate(&fresh, &protocol).unwrap()
        );
        bound.rebind(&original).unwrap();
        assert_eq!(bound.initial(), vec![2.0]);
        assert_eq!(bound.dose_scale(0), Some(0.8 / 3.0));
    }
    let previous = simulate(&bound, &protocol).unwrap();
    assert!(bound.rebind(&[0.0, 0.3, 70.0, 2.0, 0.8]).is_err());
    assert_eq!(simulate(&bound, &protocol).unwrap(), previous);
}
#[test]
fn jacobian_uses_current_named_input_rates() {
    let definition = ModelDefinition {
        bindings: vec![],
        states: vec![StateDefinition {
            name: "x".into(),
            initial: n(1.0),
            dose_scale: None,
            rhs: b(Binary::Multiply, s("input_x"), s("x")),
        }],
    };
    let bound = CompiledModel::compile(&definition)
        .unwrap()
        .bind(&[])
        .unwrap();
    let mut result = [0.0];
    bound
        .jac_mul(2.0, &[3.0], &[7.0], &[2.0], &mut result)
        .unwrap();
    assert_eq!(result, [14.0]);
}

#[test]
fn jacobian_cache_tracks_direction_state_rate_time_and_rebind() {
    let definition = ModelDefinition {
        bindings: vec!["coefficient".into()],
        states: vec![StateDefinition {
            name: "x".into(),
            initial: n(1.0),
            dose_scale: None,
            rhs: b(
                Binary::Add,
                b(
                    Binary::Multiply,
                    s("coefficient"),
                    b(Binary::Multiply, s("x"), s("x")),
                ),
                b(
                    Binary::Multiply,
                    b(Binary::Add, s("input_x"), s("time")),
                    s("x"),
                ),
            ),
        }],
    };
    let compiled = CompiledModel::compile(&definition).unwrap();
    let mut bound = compiled.bind(&[2.0]).unwrap();
    let mut result = [0.0];
    for (time, state, rate, direction, expected) in [
        (1.0, 3.0, 5.0, 1.0, 18.0),
        (1.0, 3.0, 5.0, 2.0, 36.0),
        (1.0, 4.0, 5.0, 1.0, 22.0),
        (1.0, 4.0, 6.0, 1.0, 23.0),
        (2.0, 4.0, 6.0, 1.0, 24.0),
    ] {
        bound
            .jac_mul(time, &[state], &[rate], &[direction], &mut result)
            .unwrap();
        assert_eq!(result, [expected]);
    }
    bound.rebind(&[3.0]).unwrap();
    bound
        .jac_mul(2.0, &[4.0], &[6.0], &[1.0], &mut result)
        .unwrap();
    assert_eq!(result, [32.0]);
}

#[test]
fn bolus_volume_derivative_matches_the_analytic_jump() {
    let compiled = CompiledModel::compile(&pk()).unwrap();
    let protocol = Protocol {
        start: 0.0,
        end: 1.0,
        samples: vec![],
        events: vec![Event::Bolus {
            time: 0.0,
            target: 0,
            amount: 12.0,
        }],
        rtol: 1e-9,
        atol: 1e-11,
    };
    let jump = |v: f64| {
        let bound = compiled.bind(&[v, 0.3, 70.0, 2.0, 0.8]).unwrap();
        let output = simulate(&bound, &protocol).unwrap();
        output[1].values[0] - output[0].values[0]
    };
    let derivative = (jump(3.0 + 1e-4) - jump(3.0 - 1e-4)) / 2e-4;
    assert!((derivative - (-12.0 * 0.8 / 9.0)).abs() < 2e-8);
}
#[test]
fn vm_domain_and_budget_errors_survive_the_solver_adapter() {
    use qspquest_core::expression::Function;
    use qspquest_modeling::{simulate_with_budgets, Budgets, ErrorCode};
    let definition = ModelDefinition {
        bindings: vec![],
        states: vec![StateDefinition {
            name: "x".into(),
            initial: n(1.0),
            dose_scale: None,
            rhs: Expr::call(Function::Log, vec![b(Binary::Subtract, n(1.0), s("time"))]),
        }],
    };
    let bound = CompiledModel::compile(&definition)
        .unwrap()
        .bind(&[])
        .unwrap();
    let protocol = Protocol {
        start: 0.0,
        end: 2.0,
        samples: vec![0.1, 0.2],
        events: vec![],
        rtol: 1e-8,
        atol: 1e-10,
    };
    let error = simulate(&bound, &protocol).unwrap_err();
    assert_eq!(error.code, ErrorCode::Domain);
    assert!(error.expression.unwrap().contains("dynamics[0]"));
    assert!(error.time.unwrap() >= 1.0);
    let error = simulate_with_budgets(
        &bound,
        &protocol,
        Budgets {
            solver_callbacks: 10,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::WorkBudget);
}

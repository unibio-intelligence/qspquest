use qspquest_core::expression::{Binary, Expr, Function, Unary};
use qspquest_modeling::{
    runtime::{
        sensitivity::{
            CompiledForwardSensitivity, CompiledSensitivityReadouts, DerivedBinding,
            ResolvedBindings,
        },
        CompiledModel, ModelDefinition, StateDefinition,
    },
    simulate_with_budgets, Budgets, Event, Protocol,
};
fn s(n: &str) -> Expr {
    Expr::symbol(n)
}
fn b(op: Binary, a: Expr, c: Expr) -> Expr {
    Expr::binary(op, a, c)
}
fn bindings() -> Vec<String> {
    ["theta", "wt", "vref", "eta", "a0"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}
fn derived() -> Vec<DerivedBinding> {
    vec![
        DerivedBinding {
            name: "cl".into(),
            expression: b(
                Binary::Multiply,
                s("theta"),
                b(Binary::Power, s("factor"), Expr::number(0.75)),
            ),
        },
        DerivedBinding {
            name: "v".into(),
            expression: b(Binary::Multiply, s("vref"), s("factor")),
        },
        DerivedBinding {
            name: "f".into(),
            expression: b(
                Binary::Divide,
                Expr::number(1.),
                b(
                    Binary::Add,
                    Expr::number(1.),
                    Expr::call(
                        Function::Exp,
                        vec![Expr::Unary {
                            operator: Unary::Negative,
                            arguments: Box::new([s("eta")]),
                        }],
                    ),
                ),
            ),
        },
        DerivedBinding {
            name: "factor".into(),
            expression: b(Binary::Divide, s("wt"), Expr::number(70.)),
        },
    ]
}
fn definition() -> ModelDefinition {
    ModelDefinition {
        bindings: bindings(),
        states: vec![StateDefinition {
            name: "c".into(),
            initial: b(Binary::Divide, s("a0"), s("v")),
            rhs: b(
                Binary::Subtract,
                s("input_c"),
                b(Binary::Multiply, b(Binary::Divide, s("cl"), s("v")), s("c")),
            ),
            dose_scale: Some(b(Binary::Divide, s("f"), s("v"))),
        }],
    }
}
fn protocol() -> Protocol {
    Protocol {
        start: 0.,
        end: 8.,
        samples: vec![0.5, 1.5, 3., 5., 7.],
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
            Event::Infusion {
                time: 1.,
                target: 0,
                amount: 12.,
                duration: 4.,
            },
        ],
        rtol: 1e-10,
        atol: 1e-12,
    }
}
#[test]
fn allometric_and_bioavailability_chains_match_independently_rebound_trajectories() {
    let source = definition();
    let graph = ResolvedBindings::new(&source.bindings, &derived()).unwrap();
    let expanded = graph.expand_model(&source).unwrap();
    let sensitivity = CompiledForwardSensitivity::compile(&expanded, &source.bindings).unwrap();
    let output = graph
        .expand_readout(&b(Binary::Multiply, s("c"), s("v")))
        .unwrap();
    let readouts =
        CompiledSensitivityReadouts::compile(&expanded, &source.bindings, &[output]).unwrap();
    let values = [0.7, 82., 3., 0.4, 9.];
    let rows = simulate_with_budgets(
        &sensitivity.bind(&values).unwrap(),
        &protocol(),
        Budgets::default(),
    )
    .unwrap();
    let outputs = readouts.bind(&values).unwrap();
    let mut independent = source.clone();
    independent.bindings = ["cl", "v", "f", "a0"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let primal = CompiledModel::compile(&independent).unwrap();
    let bind = |v: &[f64]| {
        [
            v[0] * (v[1] / 70.).powf(0.75),
            v[2] * v[1] / 70.,
            1. / (1. + libm::exp(-v[3])),
            v[4],
        ]
    };
    for parameter in 0..5 {
        let h = if parameter == 1 { 1e-3 } else { 1e-4 };
        let mut hi = values;
        let mut lo = values;
        hi[parameter] += h;
        lo[parameter] -= h;
        let high = bind(&hi);
        let low = bind(&lo);
        let a = simulate_with_budgets(
            &primal.bind(&high).unwrap(),
            &protocol(),
            Budgets::default(),
        )
        .unwrap();
        let b = simulate_with_budgets(&primal.bind(&low).unwrap(), &protocol(), Budgets::default())
            .unwrap();
        for ((row, a), b) in rows.iter().zip(a).zip(b) {
            let dc = (a.values[0] - b.values[0]) / (2. * h);
            assert!(
                (dc - row.values[1 + parameter]).abs() < 2e-5,
                "parameter {parameter}, time {}",
                row.time
            );
            let result = outputs.evaluate(row.time, &row.values, &[0.]).unwrap();
            let da = (a.values[0] * high[1] - b.values[0] * low[1]) / (2. * h);
            assert!((da - result.derivatives[parameter][0]).abs() < 5e-5);
        }
    }
}
#[test]
fn derived_graph_rejects_cycles_unknown_dynamic_and_colliding_names() {
    let base = bindings();
    for defs in [
        vec![
            DerivedBinding {
                name: "a".into(),
                expression: s("b"),
            },
            DerivedBinding {
                name: "b".into(),
                expression: s("a"),
            },
        ],
        vec![DerivedBinding {
            name: "cl".into(),
            expression: s("time"),
        }],
        vec![DerivedBinding {
            name: "wt".into(),
            expression: Expr::number(1.),
        }],
    ] {
        assert!(ResolvedBindings::new(&base, &defs).is_err());
    }
    let graph = ResolvedBindings::new(&base, &derived()).unwrap();
    let mut wrong = definition();
    wrong.bindings.reverse();
    assert!(graph.expand_model(&wrong).is_err());
    let mut explosive = vec![DerivedBinding {
        name: "d0".into(),
        expression: s("theta"),
    }];
    for i in 1..25 {
        let prior = s(&format!("d{}", i - 1));
        explosive.push(DerivedBinding {
            name: format!("d{i}"),
            expression: b(Binary::Add, prior.clone(), prior),
        });
    }
    assert!(ResolvedBindings::new(&base, &explosive).is_err());
}

#[test]
fn repeated_readouts_share_one_expansion_budget() {
    let base = bindings();
    let mut derived = vec![DerivedBinding {
        name: "d0".into(),
        expression: s("theta"),
    }];
    for i in 1..16 {
        let prior = s(&format!("d{}", i - 1));
        derived.push(DerivedBinding {
            name: format!("d{i}"),
            expression: b(Binary::Add, prior.clone(), prior),
        });
    }
    let graph = ResolvedBindings::new(&base, &derived).unwrap();
    assert!(graph.expand_readouts(&[s("d15")]).is_ok());
    assert!(graph.expand_readouts(&vec![s("d15"); 32]).is_err());
}

use qspquest_core::{
    expression::{Binary, Expr},
    units::Unit,
};
use qspquest_modeling::{
    runtime::{sensitivity::UnitCompiledForwardSensitivity, units::*},
    simulate_with_budgets, Budgets, Event, Model, Protocol,
};
fn u(s: &str) -> Unit {
    Unit::parse(s).unwrap()
}
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: u(unit),
    }
}
fn s(n: &str) -> Expr {
    Expr::symbol(n)
}
fn b(op: Binary, a: Expr, c: Expr) -> Expr {
    Expr::binary(op, a, c)
}
fn definition() -> UnitModelDefinition {
    UnitModelDefinition {
        time_unit: u("h"),
        bindings: [("cl", "mL/min"), ("v", "L"), ("a0", "g"), ("f", "1")]
            .iter()
            .map(|(name, unit)| BindingDefinition {
                name: (*name).into(),
                unit: u(unit),
                bounds: Some((q(0., unit), q(10000., unit))),
            })
            .collect(),
        states: vec![UnitStateDefinition {
            name: "c".into(),
            unit: u("mg/L"),
            initial: InitialCondition::Expression(b(Binary::Divide, s("a0"), s("v"))),
            rhs: b(
                Binary::Subtract,
                s("input_c"),
                b(Binary::Multiply, b(Binary::Divide, s("cl"), s("v")), s("c")),
            ),
            dosing: Some((u("g"), b(Binary::Divide, s("f"), s("v")))),
        }],
    }
}
fn protocol() -> Protocol {
    Protocol {
        start: 0.,
        end: 3.,
        samples: vec![0.5, 1.5, 2.5],
        events: vec![
            Event::Bolus {
                time: 0.,
                target: 0,
                amount: 0.005,
            },
            Event::Infusion {
                time: 1.,
                target: 0,
                amount: 0.006,
                duration: 2.,
            },
        ],
        rtol: 1e-10,
        atol: 1e-12,
    }
}
#[test]
fn converted_bindings_preserve_declared_derivative_basis() {
    let d = definition();
    let names = d
        .bindings
        .iter()
        .map(|x| x.name.clone())
        .collect::<Vec<_>>();
    let compiled = UnitCompiledForwardSensitivity::compile(&d, &names).unwrap();
    let values = [q(50., "mL/min"), q(3., "L"), q(0.009, "g"), q(0.6, "1")];
    let equivalents = [q(3., "L/h"), q(3000., "mL"), q(9., "mg"), q(0.6, "1")];
    let rows = simulate_with_budgets(
        &compiled.bind(&values).unwrap(),
        &protocol(),
        Budgets::default(),
    )
    .unwrap();
    let converted = simulate_with_budgets(
        &compiled.bind(&equivalents).unwrap(),
        &protocol(),
        Budgets::default(),
    )
    .unwrap();
    for (a, c) in rows.iter().zip(converted) {
        for (x, y) in a.values.iter().zip(c.values) {
            assert!((x - y).abs() < 1e-8);
        }
    }
    let primal = UnitCompiledModel::compile(&d).unwrap();
    for p in 0..4 {
        let h = if p == 2 { 1e-6 } else { 1e-4 };
        let mut hi = values;
        let mut lo = values;
        hi[p].value += h;
        lo[p].value -= h;
        let high =
            simulate_with_budgets(&primal.bind(&hi).unwrap(), &protocol(), Budgets::default())
                .unwrap();
        let low =
            simulate_with_budgets(&primal.bind(&lo).unwrap(), &protocol(), Budgets::default())
                .unwrap();
        for ((row, high), low) in rows.iter().zip(high).zip(low) {
            let expected = (high.values[0] - low.values[0]) / (2. * h);
            assert!(
                (row.values[p + 1] - expected).abs() < 3e-4,
                "parameter {p}: {} vs {expected}",
                row.values[p + 1]
            );
        }
    }
    let initial = compiled.bind(&values).unwrap().initial();
    assert!((initial[2] + 1.).abs() < 1e-12);
    assert!((initial[3] - 1000. / 3.).abs() < 1e-10);
    let derivative_unit = u("mg/L").divide(u("L/h")).unwrap();
    let conversion = compiled.augmented_state_units[1]
        .convert(1., derivative_unit)
        .unwrap();
    assert!((conversion - 1. / 0.06).abs() < 1e-10);
}
#[test]
fn invalid_quantity_rebind_is_atomic() {
    let compiled = UnitCompiledForwardSensitivity::compile(&definition(), &["v".into()]).unwrap();
    let good = [q(50., "mL/min"), q(3., "L"), q(0.009, "g"), q(0.6, "1")];
    let mut bound = compiled.bind(&good).unwrap();
    let original = bound.initial();
    let mut bad = good;
    bad[1] = q(3., "mg");
    assert!(bound.rebind(&bad).is_err());
    assert_eq!(bound.initial(), original);
    bad = good;
    bad[1] = q(-3., "L");
    assert!(bound.rebind(&bad).is_err());
    assert_eq!(bound.initial(), original);
    assert!(compiled.bind(&good[..3]).is_err());
}

#[test]
fn unit_observables_include_direct_state_and_input_derivatives() {
    use qspquest_modeling::runtime::sensitivity::{
        UnitCompiledSensitivityReadouts, UnitSensitivityReadout,
    };
    let d = definition();
    let names = d
        .bindings
        .iter()
        .map(|b| b.name.clone())
        .collect::<Vec<_>>();
    let readouts = vec![
        UnitSensitivityReadout {
            expression: b(Binary::Multiply, s("c"), s("v")),
            unit: u("g"),
        },
        UnitSensitivityReadout {
            expression: b(
                Binary::Divide,
                s("c"),
                b(Binary::Add, s("c"), b(Binary::Divide, s("a0"), s("v"))),
            ),
            unit: u("1"),
        },
        UnitSensitivityReadout {
            expression: b(Binary::Multiply, s("input_c"), s("v")),
            unit: u("g/min"),
        },
    ];
    let obs = UnitCompiledSensitivityReadouts::compile(&d, &names, &readouts).unwrap();
    let compiled = UnitCompiledForwardSensitivity::compile(&d, &names).unwrap();
    let values = [q(50., "mL/min"), q(3., "L"), q(0.009, "g"), q(0.6, "1")];
    let bound = obs.bind(&values).unwrap();
    let rows = simulate_with_budgets(
        &compiled.bind(&values).unwrap(),
        &protocol(),
        Budgets::default(),
    )
    .unwrap();
    let primal = UnitCompiledModel::compile(&d).unwrap();
    for p in 0..4 {
        let h = if p == 2 { 1e-6 } else { 1e-4 };
        let mut hi = values;
        let mut lo = values;
        hi[p].value += h;
        lo[p].value -= h;
        let high =
            simulate_with_budgets(&primal.bind(&hi).unwrap(), &protocol(), Budgets::default())
                .unwrap();
        let low =
            simulate_with_budgets(&primal.bind(&lo).unwrap(), &protocol(), Budgets::default())
                .unwrap();
        for ((row, high), low) in rows.iter().zip(high).zip(low) {
            let active = (row.time > 1. && row.time < 3.)
                || (row.time == 1. && row.side == "post")
                || (row.time == 3. && row.side == "pre");
            let raw = if active { 0.003 } else { 0. };
            let output = bound.evaluate(row.time, &row.values, &[raw]).unwrap();
            let oracle = |c: f64, v: &[Quantity; 4]| {
                [
                    c * v[1].value / 1000.,
                    c / (c + v[2].value * 1000. / v[1].value),
                    raw * v[3].value / 60.,
                ]
            };
            let expected = oracle(row.values[0], &values);
            let high = oracle(high.values[0], &hi);
            let low = oracle(low.values[0], &lo);
            for o in 0..3 {
                assert!((output.values[o] - expected[o]).abs() < 1e-12);
                let fd = (high[o] - low[o]) / (2. * h);
                assert!(
                    (output.derivatives[p][o] - fd).abs() < 2e-5,
                    "p {p} o {o}: {} vs {fd}",
                    output.derivatives[p][o]
                );
            }
        }
    }
    assert!((obs.derivative_units[2][0].convert(1., u("1")).unwrap() - 1.).abs() < 1e-12);
    let mut bad = readouts.clone();
    bad[0].unit = u("h");
    assert!(UnitCompiledSensitivityReadouts::compile(&d, &names, &bad).is_err());
    let mut bound = bound;
    let initial = compiled.bind(&values).unwrap().initial();
    let before = bound.evaluate(0., &initial, &[0.]).unwrap();
    let mut bad_values = values;
    bad_values[1] = q(3., "mg");
    assert!(bound.rebind(&bad_values).is_err());
    assert_eq!(
        before.values,
        bound.evaluate(0., &initial, &[0.]).unwrap().values
    );
}

#[test]
fn derived_quantities_preserve_units_and_root_parameter_chains() {
    use qspquest_modeling::runtime::sensitivity::{
        resolve_unit_bindings, UnitCompiledSensitivityReadouts, UnitDerivedBinding,
        UnitSensitivityReadout,
    };
    let original = definition();
    let oracle = UnitCompiledModel::compile(&original).unwrap();
    let mut d = original.clone();
    d.bindings = [
        ("theta", "L/h"),
        ("weight", "kg"),
        ("reference", "kg"),
        ("vref", "mL"),
        ("a0", "g"),
        ("f", "1"),
    ]
    .iter()
    .map(|(name, unit)| BindingDefinition {
        name: (*name).into(),
        unit: u(unit),
        bounds: None,
    })
    .collect();
    let ratio = b(Binary::Divide, s("weight"), s("reference"));
    let derived = vec![
        UnitDerivedBinding {
            name: "v".into(),
            unit: u("L"),
            expression: b(Binary::Multiply, s("vref"), ratio.clone()),
        },
        UnitDerivedBinding {
            name: "cl".into(),
            unit: u("mL/min"),
            expression: b(
                Binary::Multiply,
                s("theta"),
                b(Binary::Power, ratio, Expr::number(0.75)),
            ),
        },
    ];
    let readouts = vec![UnitSensitivityReadout {
        expression: b(Binary::Multiply, s("c"), s("v")),
        unit: u("g"),
    }];
    let (expanded, outputs) = resolve_unit_bindings(&d, &derived, &readouts).unwrap();
    let names = d
        .bindings
        .iter()
        .map(|b| b.name.clone())
        .collect::<Vec<_>>();
    let compiled = UnitCompiledForwardSensitivity::compile(&expanded, &names).unwrap();
    let outputs = UnitCompiledSensitivityReadouts::compile(&expanded, &names, &outputs).unwrap();
    let values = [
        q(3., "L/h"),
        q(80., "kg"),
        q(70., "kg"),
        q(3000., "mL"),
        q(0.009, "g"),
        q(0.6, "1"),
    ];
    let reference = |v: &[Quantity; 6]| {
        let ratio = v[1].value / v[2].value;
        [
            q(v[0].value * ratio.powf(0.75) / 0.06, "mL/min"),
            q(v[3].value * ratio / 1000., "L"),
            v[4],
            v[5],
        ]
    };
    let rows = simulate_with_budgets(
        &compiled.bind(&values).unwrap(),
        &protocol(),
        Budgets::default(),
    )
    .unwrap();
    let outputs = outputs.bind(&values).unwrap();
    let reference_rows = simulate_with_budgets(
        &oracle.bind(&reference(&values)).unwrap(),
        &protocol(),
        Budgets::default(),
    )
    .unwrap();
    for (a, r) in rows.iter().zip(reference_rows) {
        assert!((a.values[0] - r.values[0]).abs() < 1e-7);
    }
    for p in 0..6 {
        let h = if p == 4 { 1e-6 } else { 1e-3 };
        let mut hi = values;
        let mut lo = values;
        hi[p].value += h;
        lo[p].value -= h;
        let hv = reference(&hi);
        let lv = reference(&lo);
        let high =
            simulate_with_budgets(&oracle.bind(&hv).unwrap(), &protocol(), Budgets::default())
                .unwrap();
        let low =
            simulate_with_budgets(&oracle.bind(&lv).unwrap(), &protocol(), Budgets::default())
                .unwrap();
        for ((row, high), low) in rows.iter().zip(high).zip(low) {
            let fd = (high.values[0] - low.values[0]) / (2. * h);
            assert!((row.values[p + 1] - fd).abs() < 4e-4);
            let output = outputs.evaluate(row.time, &row.values, &[0.]).unwrap();
            let fd = (high.values[0] * hv[1].value - low.values[0] * lv[1].value) / (2000. * h);
            assert!((output.derivatives[p][0] - fd).abs() < 2e-6);
        }
    }
    let mut bad = derived.clone();
    bad[0].unit = u("mg");
    assert!(resolve_unit_bindings(&d, &bad, &readouts).is_err());
    let mut bad = derived.clone();
    bad[0].expression = s("time");
    assert!(resolve_unit_bindings(&d, &bad, &readouts).is_err());
    let mut bad = derived.clone();
    bad[0].expression = s("v");
    assert!(resolve_unit_bindings(&d, &bad, &readouts).is_err());
    assert!(resolve_unit_bindings(&d, &derived, &vec![readouts[0].clone(); 1025]).is_err());
}

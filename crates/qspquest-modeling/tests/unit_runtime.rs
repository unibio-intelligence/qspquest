use qspquest_core::{
    expression::{Binary, Expr},
    units::Unit,
    ErrorCode,
};
use qspquest_modeling::{runtime::units::*, simulate, Event, Model, Protocol};
fn u(code: &str) -> Unit {
    Unit::parse(code).unwrap()
}
fn q(value: f64, code: &str) -> Quantity {
    Quantity {
        value,
        unit: u(code),
    }
}
fn s(name: &str) -> Expr {
    Expr::symbol(name)
}
fn b(op: Binary, a: Expr, b: Expr) -> Expr {
    Expr::binary(op, a, b)
}
fn definition() -> UnitModelDefinition {
    UnitModelDefinition {
        time_unit: u("h"),
        bindings: vec![
            BindingDefinition {
                name: "cl".into(),
                unit: u("L/h"),
                bounds: Some((q(0.0, "L/h"), q(2.0, "L/h"))),
            },
            BindingDefinition {
                name: "v".into(),
                unit: u("L"),
                bounds: Some((q(1.0, "L"), q(10000.0, "mL"))),
            },
            BindingDefinition {
                name: "initial".into(),
                unit: u("mg/L"),
                bounds: None,
            },
        ],
        states: vec![UnitStateDefinition {
            name: "c".into(),
            unit: u("mg/L"),
            initial: InitialCondition::Expression(s("initial")),
            rhs: b(
                Binary::Subtract,
                s("input_c"),
                b(Binary::Multiply, b(Binary::Divide, s("cl"), s("v")), s("c")),
            ),
            dosing: Some((u("mg"), b(Binary::Divide, Expr::number(1.0), s("v")))),
        }],
    }
}
fn protocol() -> Protocol {
    Protocol {
        start: 0.0,
        end: 4.0,
        samples: vec![0.0, 0.5, 1.0, 2.0, 3.0, 4.0],
        events: vec![
            Event::Bolus {
                time: 0.0,
                target: 0,
                amount: 6.0,
            },
            Event::Infusion {
                time: 1.0,
                target: 0,
                amount: 3.0,
                duration: 1.0,
            },
        ],
        rtol: 1e-9,
        atol: 1e-11,
    }
}
#[test]
fn equivalent_bindings_match_analytic_bolus_and_infusion_trajectories() {
    let compiled = UnitCompiledModel::compile(&definition()).unwrap();
    let a = compiled
        .bind(&[q(0.3, "L/h"), q(3.0, "L"), q(2.0, "mg/L")])
        .unwrap();
    let b = compiled
        .bind(&[q(5.0, "mL/min"), q(3000.0, "mL"), q(2.0, "ug/mL")])
        .unwrap();
    let ta = simulate(&a, &protocol()).unwrap();
    let tb = simulate(&b, &protocol()).unwrap();
    for (a, b) in ta.iter().zip(&tb) {
        assert_eq!((a.time, &a.side), (b.time, &b.side));
        assert!((a.values[0] - b.values[0]).abs() < 1e-8);
        let t = a.time;
        let initial = if t == 0.0 && a.side == "pre" {
            2.0
        } else {
            4.0
        };
        let infusion = if t <= 1.0 {
            0.0
        } else {
            10.0 * (1.0 - (-0.1 * (t.min(2.0) - 1.0)).exp()) * (-0.1 * (t - 2.0).max(0.0)).exp()
        };
        let expected = initial * (-0.1 * t).exp() + infusion;
        assert!(
            (a.values[0] - expected).abs() < 2e-7,
            "{t} {} {} vs {expected}",
            a.side,
            a.values[0]
        );
    }
    let mut jac = [0.0];
    b.jac_mul(1.0, &[2.0], &[1.0], &[1.0], &mut jac).unwrap();
    assert!((jac[0] + 0.1).abs() < 1e-14);
}
#[test]
fn changing_declared_time_state_and_dose_units_preserves_physics() {
    let mut d = definition();
    d.time_unit = u("min");
    d.states[0].unit = u("g/L");
    d.states[0].dosing.as_mut().unwrap().0 = u("g");
    let model = UnitCompiledModel::compile(&d)
        .unwrap()
        .bind(&[q(0.3, "L/h"), q(3.0, "L"), q(2.0, "mg/L")])
        .unwrap();
    let mut p = protocol();
    p.end *= 60.0;
    p.samples.iter_mut().for_each(|t| *t *= 60.0);
    p.atol /= 1000.0;
    for e in &mut p.events {
        match e {
            Event::Reset { .. } => unreachable!("fixture has only doses"),
            Event::Bolus { time, amount, .. } => {
                *time *= 60.0;
                *amount /= 1000.0;
            }
            Event::Infusion {
                time,
                amount,
                duration,
                ..
            } => {
                *time *= 60.0;
                *amount /= 1000.0;
                *duration *= 60.0;
            }
        }
    }
    let reference = UnitCompiledModel::compile(&definition())
        .unwrap()
        .bind(&[q(0.3, "L/h"), q(3.0, "L"), q(2.0, "mg/L")])
        .unwrap();
    for (a, b) in simulate(&model, &p)
        .unwrap()
        .iter()
        .zip(simulate(&reference, &protocol()).unwrap())
    {
        assert_eq!(a.side, b.side);
        assert!((a.time / 60.0 - b.time).abs() < 1e-14);
        assert!((1000.0 * a.values[0] - b.values[0]).abs() < 2e-7);
    }
}
#[test]
fn invalid_units_and_bounds_fail_before_solving_and_failed_rebind_is_atomic() {
    let compiled = UnitCompiledModel::compile(&definition()).unwrap();
    let original = [q(0.3, "L/h"), q(3.0, "L"), q(2.0, "mg/L")];
    let mut model = compiled.bind(&original).unwrap();
    let before = simulate(&model, &protocol()).unwrap();
    for bad in [q(30000.0, "mL"), q(3.0, "mg")] {
        assert!(model.rebind(&[original[0], bad, original[2]]).is_err());
        assert_eq!(before, simulate(&model, &protocol()).unwrap());
    }
    model
        .rebind(&[original[0], q(6.0, "L"), q(4.0, "ug/mL")])
        .unwrap();
    assert_eq!(model.initial(), vec![4.0]);
    assert!((model.dose_scale(0).unwrap() - 1.0 / 6.0).abs() < 1e-14);
    let mut d = definition();
    d.states[0].rhs = s("c");
    let error = UnitCompiledModel::compile(&d).unwrap_err();
    assert_eq!(error.code, ErrorCode::Unit);
    assert!(error.expression.unwrap().starts_with("states.c.rhs"));
    let mut d = definition();
    d.states[0].initial = InitialCondition::Quantity(q(2.0, "h"));
    assert_eq!(
        UnitCompiledModel::compile(&d).unwrap_err().code,
        ErrorCode::Unit
    );
    let mut d = definition();
    d.states[0].dosing.as_mut().unwrap().1 = s("v");
    assert_eq!(
        UnitCompiledModel::compile(&d).unwrap_err().code,
        ErrorCode::Unit
    );
    let mut d = definition();
    d.bindings[0].bounds = Some((q(1.0, "h"), q(2.0, "h")));
    assert_eq!(
        UnitCompiledModel::compile(&d).unwrap_err().code,
        ErrorCode::Unit
    );
}

#[test]
fn inserted_conversions_have_a_separate_budget_without_relaxing_source_rules() {
    fn sum(mut terms: Vec<Expr>) -> Expr {
        while terms.len() > 1 {
            terms = terms
                .chunks(2)
                .map(|pair| {
                    if pair.len() == 2 {
                        b(Binary::Add, pair[0].clone(), pair[1].clone())
                    } else {
                        pair[0].clone()
                    }
                })
                .collect();
        }
        terms.pop().unwrap()
    }
    let mut d = definition();
    // 32 copies of cl*c/v have 191 source nodes. Conversion insertion expands
    // beyond the source cap, while the balanced source remains shallow.
    d.states[0].rhs = sum((0..32)
        .map(|_| b(Binary::Divide, b(Binary::Multiply, s("cl"), s("c")), s("v")))
        .collect());
    let model = UnitCompiledModel::compile(&d)
        .unwrap()
        .bind(&[q(0.3, "L/h"), q(3.0, "L"), q(2.0, "mg/L")])
        .unwrap();
    let mut out = [0.0];
    model.rhs(0.0, &[2.0], &[0.0], &mut out).unwrap();
    assert!((out[0] - 6.4).abs() < 1e-12);
    model
        .jac_mul(0.0, &[2.0], &[0.0], &[1.0], &mut out)
        .unwrap();
    assert!((out[0] - 3.2).abs() < 1e-12);
    let rhs = d.states[0].rhs.clone();
    d.states[0].rhs = Expr::conditional(
        Expr::Compare {
            operator: qspquest_core::expression::Compare::Gt,
            arguments: Box::new([s("c"), s("initial")]),
        },
        b(Binary::Divide, s("c"), s("v")),
        b(Binary::Divide, s("c"), s("v")),
    );
    assert_eq!(
        UnitCompiledModel::compile(&d).unwrap_err().code,
        ErrorCode::Unsupported
    );
    d.states[0].rhs = b(Binary::Add, rhs.clone(), rhs);
    assert_eq!(
        UnitCompiledModel::compile(&d).unwrap_err().code,
        ErrorCode::ExpressionLimit
    );
}

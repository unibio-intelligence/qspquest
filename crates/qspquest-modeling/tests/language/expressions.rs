use qspquest_core::expression::*;
use qspquest_core::ErrorCode;
use qspquest_modeling::compile::{Program, Symbol, SymbolKind};
fn symbols() -> Vec<Symbol> {
    vec![
        Symbol {
            name: "x".into(),
            kind: SymbolKind::Parameter,
        },
        Symbol {
            name: "p".into(),
            kind: SymbolKind::Parameter,
        },
    ]
}
fn x() -> Expr {
    Expr::symbol("x")
}
fn n(v: f64) -> Expr {
    Expr::number(v)
}
fn b(op: Binary, a: Expr, b: Expr) -> Expr {
    Expr::binary(op, a, b)
}
fn f(op: Function, a: Expr) -> Expr {
    Expr::call(op, vec![a])
}
fn gt(a: Expr, b: Expr) -> Expr {
    Expr::Compare {
        operator: Compare::Gt,
        arguments: Box::new([a, b]),
    }
}
#[test]
fn lazy_guards_preserve_invalid_inactive_branch_domains() {
    let expr = Expr::conditional(gt(x(), n(0.0)), f(Function::Log, x()), n(7.0));
    let program = Program::compile(&expr, &symbols()).unwrap();
    assert_eq!(program.evaluate(&[-1.0, 0.0]).unwrap(), 7.0);
    assert_eq!(program.evaluate(&[0.0, 0.0]).unwrap(), 7.0);
    assert!((program.evaluate(&[2.0, 0.0]).unwrap() - 2f64.ln()).abs() < 1e-15);
    let both = Expr::Boolean {
        operator: Boolean::And,
        arguments: vec![gt(x(), n(0.0)), gt(f(Function::Log, x()), n(0.0))],
    };
    assert_eq!(
        Program::compile(&both, &symbols())
            .unwrap()
            .evaluate(&[-2.0, 0.0])
            .unwrap(),
        0.0
    );
}
#[test]
fn symbolic_smooth_derivatives_match_finite_differences() {
    let expressions = vec![
        b(Binary::Multiply, x(), Expr::symbol("p")),
        b(Binary::Divide, x(), b(Binary::Add, x(), n(2.0))),
        b(Binary::Power, x(), n(2.7)),
        b(Binary::Power, x(), Expr::symbol("p")),
        f(Function::Exp, x()),
        f(Function::Log, x()),
        f(Function::Log10, x()),
        f(Function::Sqrt, x()),
        f(Function::Sin, x()),
        f(Function::Cos, x()),
        f(Function::Tan, x()),
        f(Function::Tanh, x()),
    ];
    for expr in expressions {
        let value = Program::compile(&expr, &symbols()).unwrap();
        for (index, name) in ["x", "p"].iter().enumerate() {
            let derivative = Program::derivative(&expr, &symbols(), name).unwrap();
            for point in [0.2, 0.7, 1.2] {
                let mut plus = [point, 1.7];
                let mut minus = plus;
                plus[index] += 1e-6;
                minus[index] -= 1e-6;
                let expected =
                    (value.evaluate(&plus).unwrap() - value.evaluate(&minus).unwrap()) / 2e-6;
                let actual = derivative.evaluate(&[point, 1.7]).unwrap();
                assert!(
                    (actual - expected).abs() < 2e-7 * (1.0 + expected.abs()),
                    "{expr:?}, {name}: {actual} vs {expected}"
                );
            }
        }
    }
}
#[test]
fn derivative_retains_original_domain_and_zero_boundary_rules() {
    let invalid = b(Binary::Divide, n(1.0), n(0.0));
    assert_eq!(
        Program::derivative(&invalid, &symbols(), "x")
            .unwrap()
            .evaluate(&[2.0, 1.0])
            .unwrap_err()
            .code,
        ErrorCode::Domain
    );
    let root = f(Function::Sqrt, x());
    assert_eq!(
        Program::compile(&root, &symbols())
            .unwrap()
            .evaluate(&[0.0, 0.0])
            .unwrap(),
        0.0
    );
    assert_eq!(
        Program::derivative(&root, &symbols(), "x")
            .unwrap()
            .evaluate(&[0.0, 0.0])
            .unwrap_err()
            .code,
        ErrorCode::Domain
    );
    let square = b(Binary::Power, x(), n(2.0));
    assert_eq!(
        Program::derivative(&square, &symbols(), "x")
            .unwrap()
            .evaluate(&[-3.0, 0.0])
            .unwrap(),
        -6.0
    );
}
#[test]
fn rebinding_updates_every_load_without_recompilation() {
    let expr = b(
        Binary::Add,
        b(Binary::Divide, x(), Expr::symbol("p")),
        b(Binary::Multiply, x(), Expr::symbol("p")),
    );
    let program = Program::compile(&expr, &symbols()).unwrap();
    let mut scratch = program.workspace();
    for values in [[5.0, 2.0], [5.0, 4.0], [10.0, 4.0], [5.0, 2.0]] {
        assert_eq!(
            program.evaluate_into(&values, &mut scratch).unwrap(),
            values[0] / values[1] + values[0] * values[1]
        );
    }
}
#[test]
fn diagnostics_cover_limits_types_symbols_and_dynamic_switches() {
    assert_eq!(
        Program::compile(&Expr::symbol("missing"), &symbols())
            .unwrap_err()
            .code,
        ErrorCode::UnknownSymbol
    );
    assert_eq!(
        Program::compile(&Expr::conditional(x(), n(1.0), n(0.0)), &symbols())
            .unwrap_err()
            .code,
        ErrorCode::Type
    );
    let mut nested = x();
    for _ in 0..33 {
        nested = f(Function::Exp, nested);
    }
    assert_eq!(
        Program::compile(&nested, &symbols()).unwrap_err().code,
        ErrorCode::ExpressionLimit
    );
    let mut states = symbols();
    states[0].kind = SymbolKind::State;
    assert_eq!(
        Program::compile(&Expr::conditional(gt(x(), n(0.0)), x(), n(0.0)), &states)
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    let error = Program::compile(&f(Function::Log, x()), &symbols())
        .unwrap()
        .evaluate(&[-1.0, 0.0])
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Domain);
    assert_eq!(error.expression.as_deref(), Some("root"));
}
#[test]
fn modulo_and_hill_values_follow_the_declared_conventions() {
    let modulo = Program::compile(&b(Binary::Modulo, x(), n(3.0)), &symbols()).unwrap();
    for (input, expected) in [
        (-4.0, 2.0),
        (-3.0, 0.0),
        (-1.0, 2.0),
        (0.0, 0.0),
        (4.0, 1.0),
    ] {
        assert_eq!(modulo.evaluate(&[input, 0.0]).unwrap(), expected);
    }
    let hill = Program::compile(
        &Expr::call(Function::Hill, vec![x(), n(2.0), n(3.0)]),
        &symbols(),
    )
    .unwrap();
    for (input, expected) in [(-1.0, 0.0), (0.0, 0.0), (3.0, 0.5)] {
        assert_eq!(hill.evaluate(&[input, 0.0]).unwrap(), expected);
    }
    assert_eq!(hill.evaluate(&[1e300, 0.0]).unwrap(), 1.0);
}
#[test]
fn closed_ir_round_trips_and_rejects_unknown_operators() {
    let expr = Expr::conditional(gt(x(), n(0.0)), f(Function::Log, x()), n(0.0));
    let json = serde_json::to_string(&expr).unwrap();
    assert_eq!(serde_json::from_str::<Expr>(&json).unwrap(), expr);
    assert!(
        serde_json::from_str::<Expr>(r#"{"kind":"call","name":"system","arguments":[]}"#).is_err()
    );
    assert!(
        serde_json::from_str::<Expr>(r#"{"kind":"literal","value":1,"source":"escape"}"#).is_err()
    );
}

#[test]
fn modulo_derivatives_are_signed_and_reject_wrap_points() {
    let expr = b(Binary::Modulo, x(), Expr::symbol("p"));
    let dx = Program::derivative(&expr, &symbols(), "x").unwrap();
    let dp = Program::derivative(&expr, &symbols(), "p").unwrap();
    for (point, expected) in [(-4.0, 2.0), (-1.0, 1.0), (1.0, 0.0), (4.0, -1.0)] {
        assert_eq!(dx.evaluate(&[point, 3.0]).unwrap(), 1.0);
        assert_eq!(dp.evaluate(&[point, 3.0]).unwrap(), expected);
    }
    for point in [-3.0, 0.0, 3.0] {
        assert_eq!(
            dx.evaluate(&[point, 3.0]).unwrap_err().code,
            ErrorCode::Domain
        );
        assert_eq!(
            dp.evaluate(&[point, 3.0]).unwrap_err().code,
            ErrorCode::Domain
        );
    }
}

#[test]
fn piecewise_derivatives_select_first_tie_and_preserve_domains() {
    let mut states = symbols();
    states[0].kind = SymbolKind::State;
    for function in [Function::Min, Function::Max] {
        let expr = Expr::call(function, vec![x(), Expr::symbol("p"), n(2.0)]);
        let dx = Program::derivative(&expr, &states, "x").unwrap();
        let dp = Program::derivative(&expr, &states, "p").unwrap();
        for values in [[1.0, 1.0], [2.0, 2.0], [3.0, 3.0], [0.0, 3.0], [3.0, 0.0]] {
            let entries = [values[0], values[1], 2.0];
            let mut selected = 0;
            for index in 1..3 {
                if (function == Function::Min && entries[index] < entries[selected])
                    || (function == Function::Max && entries[index] > entries[selected])
                {
                    selected = index;
                }
            }
            assert_eq!(
                dx.evaluate(&values).unwrap(),
                if selected == 0 { 1.0 } else { 0.0 }
            );
            assert_eq!(
                dp.evaluate(&values).unwrap(),
                if selected == 1 { 1.0 } else { 0.0 }
            );
        }
        // Invalid unselected primal arguments must not disappear in the derivative.
        let invalid = Expr::call(function, vec![x(), f(Function::Log, Expr::symbol("p"))]);
        assert_eq!(
            Program::derivative(&invalid, &states, "x")
                .unwrap()
                .evaluate(&[1.0, -1.0])
                .unwrap_err()
                .code,
            ErrorCode::Domain
        );
    }
    let abs = Program::derivative(&f(Function::Abs, x()), &states, "x").unwrap();
    for (value, expected) in [(-1.0, -1.0), (-0.0, 0.0), (0.0, 0.0), (1.0, 1.0)] {
        assert_eq!(abs.evaluate(&[value, 0.0]).unwrap(), expected);
    }
}

#[test]
fn bdf_crosses_piecewise_boundaries_against_analytic_integrals() {
    use qspquest_modeling::{language, simulate, Protocol};
    use std::collections::BTreeMap;
    for function in ["min", "max", "abs"] {
        let argument = if function == "abs" { "ramp" } else { "ramp, 0" };
        let source = format!("model piecewise\ntime_unit = h\nparameters\n  rate = 1 [1/h]\nstates\n  ramp = -1 [1]\n  integral = 0 [1]\ndynamics\n  d/dt(ramp) = rate\n  d/dt(integral) = rate * {function}({argument})\noutputs\n  integral = integral [1]\nend\n");
        let model = language::parse(&source).unwrap().compile().unwrap();
        let bound = model.bind(&BTreeMap::new()).unwrap();
        let protocol = Protocol {
            start: 0.,
            end: 2.,
            samples: (0..=40).map(|i| i as f64 / 20.).collect(),
            events: vec![],
            rtol: 1e-10,
            atol: 1e-12,
        };
        for row in simulate(&bound, &protocol).unwrap() {
            let t = row.time;
            let expected = match function {
                "min" => {
                    if t <= 1. {
                        -t + t * t / 2.
                    } else {
                        -0.5
                    }
                }
                "max" => {
                    if t <= 1. {
                        0.
                    } else {
                        (t - 1.).powi(2) / 2.
                    }
                }
                _ => {
                    if t <= 1. {
                        t - t * t / 2.
                    } else {
                        0.5 + (t - 1.).powi(2) / 2.
                    }
                }
            };
            assert!(
                (row.values[1] - expected).abs() < 1e-7,
                "{function} at {t}: {} vs {expected}",
                row.values[1]
            );
        }
    }
}

#[test]
fn hill_jacobian_matches_finite_differences_and_boundary_contract() {
    let symbols = vec![
        Symbol {
            name: "x".into(),
            kind: SymbolKind::State,
        },
        Symbol {
            name: "n".into(),
            kind: SymbolKind::Parameter,
        },
        Symbol {
            name: "k".into(),
            kind: SymbolKind::Parameter,
        },
    ];
    let expr = Expr::call(
        Function::Hill,
        vec![x(), Expr::symbol("n"), Expr::symbol("k")],
    );
    let value = Program::compile(&expr, &symbols).unwrap();
    for (index, name) in ["x", "n", "k"].iter().enumerate() {
        let derivative = Program::derivative(&expr, &symbols, name).unwrap();
        for exponent in [0.5, 1.0, 2.0, 5.0] {
            for point in [0.1, 1.0, 10.0] {
                let base = [point, exponent, 2.0];
                let mut plus = base;
                let mut minus = base;
                plus[index] += 1e-6;
                minus[index] -= 1e-6;
                let expected =
                    (value.evaluate(&plus).unwrap() - value.evaluate(&minus).unwrap()) / 2e-6;
                let actual = derivative.evaluate(&base).unwrap();
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "{name}: {base:?}: {actual} vs {expected}"
                );
            }
            assert_eq!(derivative.evaluate(&[-1.0, exponent, 2.0]).unwrap(), 0.0);
            let expected = if index != 0 || exponent > 1.0 {
                0.0
            } else if exponent == 1.0 {
                0.5
            } else {
                let z = 1e-8_f64.powf(exponent);
                exponent * z / (2e-8 * (1.0 + z).powi(2))
            };
            let actual = derivative.evaluate(&[0.0, exponent, 2.0]).unwrap();
            assert!(
                (actual - expected).abs() <= 1e-12 * (1.0 + expected.abs()),
                "boundary {name}: {actual} vs {expected}"
            );
        }
    }
    let derivative = Program::derivative(&expr, &symbols, "x").unwrap();
    // A rounded-to-one Hill value must retain its nonzero local derivative.
    let actual = derivative.evaluate(&[1e150, 1.0, 1.0]).unwrap();
    assert!((actual / 1e-300 - 1.0).abs() < 1e-12);
    for values in [[1.0, 0.0, 2.0], [0.0, 1.0, 0.0], [-1.0, -1.0, 1.0]] {
        assert_eq!(
            derivative.evaluate(&values).unwrap_err().code,
            ErrorCode::Domain
        );
    }
    let mut parameters = symbols.clone();
    parameters[0].kind = SymbolKind::Parameter;
    let diagnostic = Program::derivative(&expr, &parameters, "x").unwrap();
    for exponent in [0.5, 1.0] {
        assert_eq!(
            diagnostic.evaluate(&[0.0, exponent, 2.0]).unwrap_err().code,
            ErrorCode::Domain
        );
    }
}

#[test]
fn hill_with_zero_initial_drug_integrates_against_closed_form() {
    use qspquest_modeling::{language, simulate, Protocol};
    use std::collections::BTreeMap;
    for exponent in [0.5, 1.0, 2.0] {
        let source = format!("model hill_zero\ntime_unit = h\nparameters\n  rate = 1 [1/h]\n  n = {exponent} [1]\n  k = 1 [1]\nstates\n  drug = 0 [1]\n  integral = 0 [1]\ndynamics\n  d/dt(drug) = rate\n  d/dt(integral) = rate * hill(drug, n, k)\noutputs\n  integral = integral [1]\nend\n");
        let model = language::parse(&source).unwrap().compile().unwrap();
        let bound = model.bind(&BTreeMap::new()).unwrap();
        let protocol = Protocol {
            start: 0.,
            end: 4.,
            samples: (0..=40).map(|i| i as f64 / 10.).collect(),
            events: vec![],
            rtol: 1e-9,
            atol: 1e-11,
        };
        for row in simulate(&bound, &protocol).unwrap() {
            let t = row.time;
            let expected = if exponent == 0.5 {
                t - 2. * t.sqrt() + 2. * t.sqrt().ln_1p()
            } else if exponent == 1.0 {
                t - t.ln_1p()
            } else {
                t - t.atan()
            };
            assert!(
                (row.values[1] - expected).abs() < 2e-7,
                "n={exponent}, t={t}: {} vs {expected}",
                row.values[1]
            );
        }
    }
}

#[test]
fn zero_base_positive_powers_and_their_finite_partials_are_supported() {
    let expr = b(Binary::Power, x(), Expr::symbol("p"));
    let value = Program::compile(&expr, &symbols()).unwrap();
    let dx = Program::derivative(&expr, &symbols(), "x").unwrap();
    let dp = Program::derivative(&expr, &symbols(), "p").unwrap();
    for exponent in [0.5, 1.0, 2.0, 2.79, 3.0] {
        assert_eq!(value.evaluate(&[0.0, exponent]).unwrap(), 0.0);
        assert_eq!(dp.evaluate(&[0.0, exponent]).unwrap(), 0.0);
        if exponent >= 1.0 {
            assert_eq!(
                dx.evaluate(&[0.0, exponent]).unwrap(),
                if exponent == 1.0 { 1.0 } else { 0.0 }
            );
        } else {
            assert_eq!(
                dx.evaluate(&[0.0, exponent]).unwrap_err().code,
                ErrorCode::Domain
            );
        }
    }
    for input in [[0.0, 0.0], [0.0, -1.0], [-1.0, 2.79]] {
        assert_eq!(value.evaluate(&input).unwrap_err().code, ErrorCode::Domain);
        assert_eq!(dx.evaluate(&input).unwrap_err().code, ErrorCode::Domain);
        assert_eq!(dp.evaluate(&input).unwrap_err().code, ErrorCode::Domain);
    }
    assert_eq!(dx.evaluate(&[-2.0, 3.0]).unwrap(), 12.0);
    assert_eq!(dx.evaluate(&[-2.0, 0.0]).unwrap(), 0.0);
    assert_eq!(
        dp.evaluate(&[-2.0, 3.0]).unwrap_err().code,
        ErrorCode::Domain
    );
}

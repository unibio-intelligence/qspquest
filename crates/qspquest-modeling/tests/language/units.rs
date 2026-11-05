use qspquest_core::{
    expression::{Binary, Expr, Function},
    units::{Rational, Unit},
    ErrorCode,
};
use qspquest_modeling::compile::{Program, Symbol, SymbolKind};
fn u(s: &str) -> Unit {
    Unit::parse(s).unwrap()
}
fn symbol(name: &str, unit: &str) -> (Symbol, Unit) {
    (
        Symbol {
            name: name.into(),
            kind: SymbolKind::Parameter,
        },
        u(unit),
    )
}
fn s(name: &str) -> Expr {
    Expr::symbol(name)
}
fn b(op: Binary, a: Expr, b: Expr) -> Expr {
    Expr::binary(op, a, b)
}
#[test]
fn pharmaceutical_conversions_use_exact_scale_ratios() {
    assert_eq!(
        u("g").conversion_to(u("kg")).unwrap(),
        Rational::new(1, 1000).unwrap()
    );
    assert_eq!(
        u("h").conversion_to(u("min")).unwrap(),
        Rational::new(60, 1).unwrap()
    );
    assert_eq!(u("ug/mL").conversion_to(u("mg/L")).unwrap(), Rational::ONE);
    assert_eq!(
        u("nmol/L").conversion_to(u("mol/m3")).unwrap(),
        Rational::new(1, 1000000).unwrap()
    );
    assert_eq!(
        u("(mol/L)").conversion_to(u("nmol/L")).unwrap(),
        Rational::new(1000000000, 1).unwrap()
    );
    assert_eq!(
        u("L/h").conversion_to(u("mL/min")).unwrap(),
        Rational::new(50, 3).unwrap()
    );
    assert_eq!(u("mg.kg-1.d-1"), u("mg/(kg.d)"));
    assert_eq!(u("Pa"), u("kg/(m.s2)"));
    assert_eq!(
        u("a").conversion_to(u("d")).unwrap(),
        Rational::new(1461, 4).unwrap()
    );
    assert_eq!(u("a").convert(1.0, u("s")).unwrap(), 31557600.0);
    assert_eq!(
        u("1/a").conversion_to(u("1/d")).unwrap(),
        Rational::new(4, 1461).unwrap()
    );
}
#[test]
fn incompatible_units_and_unsupported_syntax_fail_closed() {
    assert_eq!(
        u("mg/L").conversion_to(u("nmol/L")).unwrap_err().code,
        ErrorCode::Unit
    );
    for code in [
        "0", "1-2", "2+10", "1000mg", "12a", "1e9", "1e-3", "nM", "µg", "Cel", "IU", "unknown",
        "mg//L", "mg / L", "(mg/L", "s-", "mg^2",
    ] {
        assert!(Unit::parse(code).is_err(), "{code}");
    }
    assert!(Unit::parse("nm32").is_err());
    assert!(Unit::parse(&"(".repeat(40)).is_err());
    assert!(u("kg").convert(f64::INFINITY, u("g")).is_err());
}
#[test]
fn equivalent_weight_units_give_identical_allometric_clearance() {
    let expr = b(
        Binary::Multiply,
        s("tv_cl"),
        b(
            Binary::Power,
            b(Binary::Divide, s("wt"), s("wt_ref")),
            Expr::number(0.75),
        ),
    );
    let kg = Program::compile_with_units(
        &expr,
        &[
            symbol("tv_cl", "L/h"),
            symbol("wt", "kg"),
            symbol("wt_ref", "kg"),
        ],
        u("L/h"),
    )
    .unwrap();
    let g = Program::compile_with_units(
        &expr,
        &[
            symbol("tv_cl", "L/h"),
            symbol("wt", "g"),
            symbol("wt_ref", "kg"),
        ],
        u("L/h"),
    )
    .unwrap();
    for weight in [35.0, 70.0, 140.0] {
        assert_eq!(
            kg.evaluate(&[0.134, weight, 70.0]).unwrap(),
            g.evaluate(&[0.134, weight * 1000.0, 70.0]).unwrap()
        );
    }
    let bad = b(
        Binary::Multiply,
        s("tv_cl"),
        b(
            Binary::Power,
            b(Binary::Divide, s("wt"), Expr::number(70.0)),
            Expr::number(0.75),
        ),
    );
    assert_eq!(
        Program::compile_with_units(
            &bad,
            &[symbol("tv_cl", "L/h"), symbol("wt", "kg")],
            u("L/h")
        )
        .unwrap_err()
        .code,
        ErrorCode::Unit
    );
}
#[test]
fn dimensional_arithmetic_and_transcendental_rules_are_enforced() {
    let add = b(Binary::Add, s("a"), s("b"));
    let program =
        Program::compile_with_units(&add, &[symbol("a", "mg"), symbol("b", "g")], u("mg")).unwrap();
    assert!((program.evaluate(&[1.0, 0.001]).unwrap() - 2.0).abs() < 1e-14);
    assert!(
        Program::compile_with_units(&add, &[symbol("a", "mg"), symbol("b", "h")], u("mg")).is_err()
    );
    assert!(Program::compile_with_units(
        &Expr::call(Function::Exp, vec![s("a")]),
        &[symbol("a", "kg")],
        u("1")
    )
    .is_err());
    assert!(Program::compile_with_units(
        &Expr::call(Function::Sqrt, vec![s("a")]),
        &[symbol("a", "m")],
        u("m")
    )
    .is_err());
    let root = Program::compile_with_units(
        &Expr::call(Function::Sqrt, vec![s("a")]),
        &[symbol("a", "cm2")],
        u("m"),
    )
    .unwrap();
    assert!((root.evaluate(&[4.0]).unwrap() - 0.02).abs() < 1e-15);
}
#[test]
fn explicit_molar_mass_enables_mass_amount_conversion() {
    let expression = b(Binary::Divide, s("mass_concentration"), s("molar_mass"));
    let program = Program::compile_with_units(
        &expression,
        &[
            symbol("mass_concentration", "mg/L"),
            symbol("molar_mass", "g/mol"),
        ],
        u("nmol/L"),
    )
    .unwrap();
    assert!((program.evaluate(&[1.0, 1000.0]).unwrap() - 1000.0).abs() < 1e-9);
}

#[test]
fn integer_unit_factors_preserve_exact_scales_and_bounds() {
    assert_eq!(u("1.5"), u("5")); // dot is multiplication, not a decimal point
    assert_eq!(u("11").scale, Rational::new(11, 1).unwrap());
    assert_eq!(u("1000.mg"), u("g"));
    assert_eq!(
        u("1000000000/L").conversion_to(u("1/mL")).unwrap(),
        Rational::new(1000000, 1).unwrap()
    );
    assert_eq!(u("(1000)-1"), u("1/1000"));
    assert_eq!(u("1000/1000"), Unit::ONE);
    assert_eq!(
        u("mg/(1000.L)").conversion_to(u("mg/L")).unwrap(),
        Rational::new(1, 1000).unwrap()
    );
    assert_eq!(u(&u128::MAX.to_string()).scale.numerator(), u128::MAX);
    for code in [
        "340282366920938463463374607431768211456",
        "0/L",
        "340282366920938463463374607431768211455.2",
        "(1000000000)32",
    ] {
        assert_eq!(
            Unit::parse(code).unwrap_err().code,
            ErrorCode::Unit,
            "{code}"
        );
    }
    let program =
        Program::compile_with_units(&s("cells"), &[symbol("cells", "1000000000/L")], u("1/mL"))
            .unwrap();
    assert_eq!(program.evaluate(&[2.5]).unwrap(), 2500000.0);
}

#[test]
fn activity_covariate_ratio_is_dimensionless_but_activity_alone_is_not() {
    let expr = b(Binary::Divide, s("activity"), s("reference"));
    let program = Program::compile_with_units(
        &expr,
        &[symbol("activity", "[iU]/L"), symbol("reference", "[iU]/mL")],
        u("1"),
    )
    .unwrap();
    assert!((program.evaluate(&[21.0, 0.021]).unwrap() - 1.0).abs() < 1e-14);
    assert!(
        Program::compile_with_units(&s("activity"), &[symbol("activity", "[iU]/L")], u("1/L"),)
            .is_err()
    );
}

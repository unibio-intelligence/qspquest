use qspquest_core::expression::{Binary, Expr};
use qspquest_modeling::{
    runtime::{sensitivity::CompiledSensitivity, ModelDefinition, StateDefinition},
    Model,
};
fn s(name: &str) -> Expr {
    Expr::symbol(name)
}
fn b(op: Binary, a: Expr, c: Expr) -> Expr {
    Expr::binary(op, a, c)
}
fn definition() -> ModelDefinition {
    ModelDefinition {
        bindings: vec!["cl".into(), "v".into(), "f".into(), "a0".into()],
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
#[test]
fn symbolic_initial_dose_and_forcing_derivatives_rebind_without_recompilation() {
    let program = CompiledSensitivity::compile(
        &definition(),
        &["v".into(), "cl".into(), "f".into(), "a0".into()],
    )
    .unwrap();
    let mut bound = program.bind(&[0.7, 3., 0.6, 9.]).unwrap();
    for values in [[0.7, 3., 0.6, 9.], [1.2, 5., 0., 12.], [0.7, 3., 0.6, 9.]] {
        bound.rebind(&values).unwrap();
        let [cl, v, f, a0] = values;
        let expected = [-a0 / v.powi(2), 0., 0., 1. / v];
        for (row, value) in bound.initial_derivatives().iter().zip(expected) {
            assert!((row[0] - value).abs() < 1e-12);
        }
        for (row, value) in
            bound
                .dose_scale_derivatives()
                .iter()
                .zip([-f / v.powi(2), 0., 1. / v, 0.])
        {
            assert!((row[0] - value).abs() < 1e-12);
        }
        let forcing = bound.rhs_forcing(2., &[4.], &[6.]).unwrap();
        for (row, value) in forcing.iter().zip([
            cl / v.powi(2) * 4. - f / v.powi(2) * 6.,
            -4. / v,
            6. / v,
            0.,
        ]) {
            assert!((row[0] - value).abs() < 1e-12);
        }
        assert_eq!(bound.primal().initial(), vec![a0 / v]);
    }
    let initial = bound.primal().initial();
    assert!(bound.rebind(&[0.7, 0., 0.6, 9.]).is_err());
    assert_eq!(bound.primal().initial(), initial);
}
#[test]
fn derivative_selection_and_dimensions_reject_invalid_inputs() {
    for names in [vec![], vec!["missing".into()], vec!["v".into(), "v".into()]] {
        assert!(CompiledSensitivity::compile(&definition(), &names).is_err());
    }
    let program = CompiledSensitivity::compile(&definition(), &["v".into()]).unwrap();
    let bound = program.bind(&[0.7, 3., 0.6, 9.]).unwrap();
    assert!(bound.rhs_forcing(0., &[], &[1.]).is_err());
    assert!(bound.rhs_forcing(0., &[1.], &[-1.]).is_err());
}

#[test]
fn nonlinear_rhs_and_input_chain_rule_match_independent_central_differences() {
    let mut def = definition();
    def.states[0].rhs = b(
        Binary::Subtract,
        b(Binary::Multiply, s("input_c"), s("input_c")),
        b(
            Binary::Divide,
            b(Binary::Multiply, s("cl"), s("c")),
            b(Binary::Add, s("v"), s("c")),
        ),
    );
    let names = def.bindings.clone();
    let program = CompiledSensitivity::compile(&def, &names).unwrap();
    let values = [0.7, 3., 0.6, 9.];
    let base = program.bind(&values).unwrap();
    let forcing = base.rhs_forcing(2., &[4.], &[6.]).unwrap();
    for parameter in 0..4 {
        let mut plus = values;
        let mut minus = values;
        let h = 1e-5;
        plus[parameter] += h;
        minus[parameter] -= h;
        let evaluate = |values: &[f64]| {
            let bound = program.bind(values).unwrap();
            let mut out = [0.];
            bound
                .primal()
                .rhs(
                    2.,
                    &[4.],
                    &[6. * bound.primal().dose_scale(0).unwrap()],
                    &mut out,
                )
                .unwrap();
            out[0]
        };
        let expected = (evaluate(&plus) - evaluate(&minus)) / (2. * h);
        assert!(
            (forcing[parameter][0] - expected).abs() < 1e-7,
            "parameter {parameter}"
        );
    }
}

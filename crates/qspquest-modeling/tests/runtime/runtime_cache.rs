use qspquest_core::expression::{Binary, Expr};
use qspquest_modeling::runtime::{CompiledModel, ModelDefinition, StateDefinition};
use qspquest_modeling::Model;
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

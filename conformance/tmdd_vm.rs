//! Same independently authored synthetic equations, expressed through the public expression nodes.
use super::tmdd::Tmdd;
use qspquest_core::expression::{Binary, Expr};
use qspquest_modeling::runtime::{ModelDefinition, StateDefinition};
fn s(name: &str) -> Expr {
    Expr::symbol(name)
}
fn n(x: f64) -> Expr {
    Expr::number(x)
}
fn add(a: Expr, b: Expr) -> Expr {
    Expr::binary(Binary::Add, a, b)
}
fn sub(a: Expr, b: Expr) -> Expr {
    Expr::binary(Binary::Subtract, a, b)
}
fn mul(a: Expr, b: Expr) -> Expr {
    Expr::binary(Binary::Multiply, a, b)
}
fn div(a: Expr, b: Expr) -> Expr {
    Expr::binary(Binary::Divide, a, b)
}
pub fn definition() -> ModelDefinition {
    let binding = sub(mul(mul(s("kon"), s("c")), s("r")), mul(s("koff"), s("x")));
    let exchange = mul(s("q"), sub(s("c"), s("p")));
    let rhs = vec![
        sub(
            sub(
                s("input_c"),
                div(add(mul(s("cl"), s("c")), exchange.clone()), s("vc")),
            ),
            binding.clone(),
        ),
        add(s("input_p"), div(exchange, s("vp"))),
        sub(mul(s("kdeg"), sub(s("r0"), s("r"))), binding.clone()),
        sub(binding, mul(s("kint"), s("x"))),
        add(mul(s("cl"), s("c")), mul(mul(s("vc"), s("kint")), s("x"))),
        add(mul(s("kdeg"), s("r")), mul(s("kint"), s("x"))),
        mul(s("kdeg"), s("r0")),
    ];
    ModelDefinition {
        bindings: ["vc", "vp", "cl", "q", "kon", "koff", "kint", "kdeg", "r0"]
            .map(String::from)
            .to_vec(),
        states: ["c", "p", "r", "x", "ed", "et", "it"]
            .into_iter()
            .zip(rhs)
            .enumerate()
            .map(|(i, (name, rhs))| StateDefinition {
                name: name.into(),
                rhs,
                initial: if i == 2 { s("r0") } else { n(0.0) },
                dose_scale: match i {
                    0 => Some(div(n(1.0), s("vc"))),
                    1 => Some(div(n(1.0), s("vp"))),
                    _ => None,
                },
            })
            .collect(),
    }
}
pub fn values(m: &Tmdd) -> Vec<f64> {
    vec![m.vc, m.vp, m.cl, m.q, m.kon, m.koff, m.kint, m.kdeg, m.r0]
}

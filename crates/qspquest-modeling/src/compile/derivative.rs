use super::*;
fn binary(op: Binary, a: Expr, b: Expr) -> Expr {
    Expr::binary(op, a, b)
}
fn add(a: Expr, b: Expr) -> Expr {
    if zero(&a) {
        b
    } else if zero(&b) {
        a
    } else {
        binary(Binary::Add, a, b)
    }
}
fn mul(a: Expr, b: Expr) -> Expr {
    if zero(&a) || zero(&b) {
        Expr::number(0.0)
    } else if one(&a) {
        b
    } else if one(&b) {
        a
    } else {
        binary(Binary::Multiply, a, b)
    }
}
fn zero(e: &Expr) -> bool {
    matches!(e,Expr::Literal{value:Literal::Number(x)} if *x==0.0)
}
fn one(e: &Expr) -> bool {
    matches!(e,Expr::Literal{value:Literal::Number(x)} if *x==1.0)
}
fn neg(e: Expr) -> Expr {
    mul(Expr::number(-1.0), e)
}
fn sub(a: Expr, b: Expr) -> Expr {
    add(a, neg(b))
}
fn div(a: Expr, b: Expr) -> Expr {
    binary(Binary::Divide, a, b)
}
fn power(a: Expr, b: f64) -> Expr {
    if b == 1.0 {
        a
    } else if b == 0.0 {
        Expr::number(1.0)
    } else {
        binary(Binary::Power, a, Expr::number(b))
    }
}
fn call(f: Function, x: Expr) -> Expr {
    Expr::call(f, vec![x])
}
fn depends(expr: &Expr, name: &str) -> bool {
    matches!(expr,Expr::Symbol{name:n} if n==name)
        || expr.children().iter().any(|e| depends(e, name))
}
pub(super) fn differentiate(expr: &Expr, name: &str, state_jacobian: bool) -> Result<Expr, Error> {
    if !depends(expr, name) {
        return Ok(Expr::number(0.0));
    }
    let d = |e: &Expr| differentiate(e, name, state_jacobian);
    Ok(match expr {
        Expr::Symbol { .. } => Expr::number(1.0),
        Expr::Unary {
            operator,
            arguments,
        } => match operator {
            Unary::Positive => d(&arguments[0])?,
            Unary::Negative => neg(d(&arguments[0])?),
            Unary::Not => {
                return Err(error(
                    ErrorCode::Type,
                    "boolean derivative is undefined",
                    "root",
                ))
            }
        },
        Expr::Binary {
            operator,
            arguments,
        } => {
            let [a, b] = arguments.as_ref();
            let (da, db) = (d(a)?, d(b)?);
            match operator {
                Binary::Add => add(da, db),
                Binary::Subtract => sub(da, db),
                Binary::Multiply => add(mul(da, b.clone()), mul(a.clone(), db)),
                Binary::Divide => div(
                    sub(mul(da, b.clone()), mul(a.clone(), db)),
                    power(b.clone(), 2.0),
                ),
                Binary::Power => {
                    if let Expr::Literal {
                        value: Literal::Number(n),
                    } = b
                    {
                        mul(mul(Expr::number(*n), power(a.clone(), n - 1.0)), da)
                    } else {
                        let equal = |left, right| Expr::Compare {
                            operator: Compare::Eq,
                            arguments: Box::new([left, right]),
                        };
                        // Separate partials avoid 0/0 in the base derivative
                        // and 0*log(0) in the exponent derivative. The compiled
                        // derivative still checks the original power domain.
                        let base_partial = if zero(&da) {
                            Expr::number(0.0)
                        } else {
                            Expr::conditional(
                                equal(b.clone(), Expr::number(0.0)),
                                Expr::number(0.0),
                                Expr::conditional(
                                    equal(b.clone(), Expr::number(1.0)),
                                    da.clone(),
                                    mul(
                                        mul(
                                            b.clone(),
                                            binary(
                                                Binary::Power,
                                                a.clone(),
                                                sub(b.clone(), Expr::number(1.0)),
                                            ),
                                        ),
                                        da,
                                    ),
                                ),
                            )
                        };
                        let exponent_partial = if zero(&db) {
                            Expr::number(0.0)
                        } else {
                            Expr::conditional(
                                equal(a.clone(), Expr::number(0.0)),
                                Expr::number(0.0),
                                mul(mul(expr.clone(), call(Function::Log, a.clone())), db),
                            )
                        };
                        add(base_partial, exponent_partial)
                    }
                }
                Binary::Modulo => add(
                    // A zero remainder is a discontinuity: force a domain error
                    // rather than reporting an away-from-boundary derivative.
                    div(Expr::number(0.0), expr.clone()),
                    sub(
                        da,
                        mul(call(Function::Floor, div(a.clone(), b.clone())), db),
                    ),
                ),
            }
        }
        Expr::Conditional { arguments } => {
            if depends(&arguments[0], name) {
                return Err(error(
                    ErrorCode::Unsupported,
                    "differentiation through moving branch boundaries is not qualified",
                    "root",
                ));
            }
            Expr::conditional(arguments[0].clone(), d(&arguments[1])?, d(&arguments[2])?)
        }
        Expr::Call {
            name: Function::Ifelse,
            arguments,
        } => {
            return differentiate(
                &Expr::conditional(
                    arguments[0].clone(),
                    arguments[1].clone(),
                    arguments[2].clone(),
                ),
                name,
                state_jacobian,
            )
        }
        Expr::Call {
            name: f @ (Function::Min | Function::Max),
            arguments,
        } => {
            // Compare against the primal extremum in source order. Equality
            // deliberately selects the first argument at a tie.
            let mut selected = d(arguments.last().expect("validated call arity"))?;
            for argument in arguments[..arguments.len() - 1].iter().rev() {
                selected = Expr::conditional(
                    Expr::Compare {
                        operator: Compare::Eq,
                        arguments: Box::new([argument.clone(), Expr::call(*f, arguments.clone())]),
                    },
                    d(argument)?,
                    selected,
                );
            }
            selected
        }
        Expr::Call {
            name: Function::Abs,
            arguments,
        } => {
            let x = arguments[0].clone();
            Expr::conditional(
                Expr::Compare {
                    operator: Compare::Gt,
                    arguments: Box::new([x.clone(), Expr::number(0.0)]),
                },
                d(&x)?,
                Expr::conditional(
                    Expr::Compare {
                        operator: Compare::Lt,
                        arguments: Box::new([x.clone(), Expr::number(0.0)]),
                    },
                    neg(d(&x)?),
                    Expr::number(0.0),
                ),
            )
        }
        Expr::Call {
            name: Function::Hill,
            arguments,
        } => {
            let (x, n, k) = (
                arguments[0].clone(),
                arguments[1].clone(),
                arguments[2].clone(),
            );
            let (dx, dn, dk) = (d(&x)?, d(&n)?, d(&k)?);
            let compare = |operator, a, b| Expr::Compare {
                operator,
                arguments: Box::new([a, b]),
            };
            // H(k,n,x) is the stable complement of H(x,n,k): avoid 1-H,
            // which loses the derivative when a saturated H rounds to one.
            let slope = |a: Expr| {
                let h = Expr::call(Function::Hill, vec![a.clone(), n.clone(), k.clone()]);
                let complement = Expr::call(Function::Hill, vec![k.clone(), n.clone(), a.clone()]);
                mul(mul(div(h, a), complement), n.clone())
            };
            let h = expr.clone();
            let complement = Expr::call(Function::Hill, vec![k.clone(), n.clone(), x.clone()]);
            let positive = add(
                mul(slope(x.clone()), dx.clone()),
                add(
                    mul(
                        mul(
                            mul(h.clone(), complement.clone()),
                            sub(
                                call(Function::Log, x.clone()),
                                call(Function::Log, k.clone()),
                            ),
                        ),
                        dn,
                    ),
                    neg(mul(mul(mul(div(h, k.clone()), complement), n.clone()), dk)),
                ),
            );
            let boundary = if zero(&dx) {
                Expr::number(0.0)
            } else if state_jacobian {
                Expr::conditional(
                    compare(Compare::Gt, n.clone(), Expr::number(1.0)),
                    Expr::number(0.0),
                    Expr::conditional(
                        compare(Compare::Eq, n.clone(), Expr::number(1.0)),
                        div(dx.clone(), k.clone()),
                        mul(slope(mul(Expr::number(1e-8), k.clone())), dx),
                    ),
                )
            } else {
                // Parameter-sensitivity invariant assertions are not available
                // through this expression-only diagnostic API.
                Expr::conditional(
                    compare(Compare::Gt, n, Expr::number(1.0)),
                    Expr::number(0.0),
                    div(Expr::number(1.0), Expr::number(0.0)),
                )
            };
            Expr::conditional(
                compare(Compare::Gt, x.clone(), Expr::number(0.0)),
                positive,
                Expr::conditional(
                    compare(Compare::Lt, x, Expr::number(0.0)),
                    Expr::number(0.0),
                    boundary,
                ),
            )
        }
        Expr::Call { name: f, arguments } => {
            let x = arguments[0].clone();
            let dx = d(&x)?;
            let factor = match f {
                Function::Exp => expr.clone(),
                Function::Log => div(Expr::number(1.0), x),
                Function::Log10 => div(
                    Expr::number(1.0),
                    mul(x, Expr::number(std::f64::consts::LN_10)),
                ),
                Function::Sqrt => div(Expr::number(1.0), mul(Expr::number(2.0), expr.clone())),
                Function::Sin => call(Function::Cos, x),
                Function::Cos => neg(call(Function::Sin, x)),
                Function::Tan => div(Expr::number(1.0), power(call(Function::Cos, x), 2.0)),
                Function::Tanh => sub(Expr::number(1.0), power(expr.clone(), 2.0)),
                // Hill boundaries and discontinuous operators require separate
                // capability tests before their derivatives are enabled.
                _ => {
                    return Err(error(
                        ErrorCode::Unsupported,
                        format!("symbolic derivative for {f:?} is not qualified"),
                        "root",
                    ))
                }
            };
            mul(factor, dx)
        }
        _ => {
            return Err(error(
                ErrorCode::Type,
                "expression is not differentiable numeric output",
                "root",
            ))
        }
    })
}

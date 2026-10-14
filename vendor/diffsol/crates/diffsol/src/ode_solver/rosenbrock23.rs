//! Rosenbrock23 compatibility name for the shared tableau-driven Rosenbrock implementation.
use crate::{DefaultDenseMatrix, Op};
pub type Rosenbrock23<'a, Eqn, LS, M = <<Eqn as Op>::V as DefaultDenseMatrix>::M> =
    crate::Rosenbrock<'a, Eqn, LS, M>;

//! Rodas5P compatibility name for the shared tableau-driven Rosenbrock implementation.
use crate::{DefaultDenseMatrix, Op};
pub type Rodas5P<'a, Eqn, LS, M = <<Eqn as Op>::V as DefaultDenseMatrix>::M> =
    crate::Rosenbrock<'a, Eqn, LS, M>;

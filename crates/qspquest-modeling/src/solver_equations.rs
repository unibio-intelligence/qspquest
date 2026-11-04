//! Equation adapter supplying complete structural Jacobian dependencies.
use diffsol::matrix::sparsity::MatrixSparsity;
use diffsol::{
    ConstantOp, JacobianColoring, Matrix, NonLinearOp, NonLinearOpJacobian, OdeEquations,
    OdeEquationsRef, Op, UnitCallable, Vector,
};

pub(crate) struct Equations<M: Matrix<T = f64>, F, G> {
    rhs: Rhs<M, F, G>,
    initial: Initial<M>,
}

pub(crate) struct Rhs<M: Matrix<T = f64>, F, G> {
    evaluate: F,
    jacobian_action: G,
    n: usize,
    context: M::C,
    sparsity: Option<M::Sparsity>,
    coloring: Option<JacobianColoring<M>>,
}

pub(crate) struct Initial<M: Matrix<T = f64>> {
    values: Vec<f64>,
    context: M::C,
}

impl<M: Matrix<T = f64>, F, G> Equations<M, F, G> {
    pub(crate) fn new(
        initial: Vec<f64>,
        pattern: Option<Vec<(usize, usize)>>,
        evaluate: F,
        jacobian_action: G,
    ) -> Result<Self, diffsol::DiffsolError> {
        let n = initial.len();
        let context = M::C::default();
        let (sparsity, coloring) = if let Some(mut indices) = pattern {
            if indices.iter().any(|&(row, col)| row >= n || col >= n) {
                return Err(diffsol::DiffsolError::Other(
                    "invalid RHS structural sparsity indices".into(),
                ));
            }
            indices.sort_unstable();
            indices.dedup();
            let sparsity = M::Sparsity::try_from_indices(n, n, indices.clone())
                .map_err(|error| diffsol::DiffsolError::Other(error.to_string()))?;
            let coloring = JacobianColoring::new(&sparsity, &indices, context.clone());
            (Some(sparsity), Some(coloring))
        } else {
            (None, None)
        };
        Ok(Self {
            rhs: Rhs {
                evaluate,
                jacobian_action,
                n,
                context: context.clone(),
                sparsity,
                coloring,
            },
            initial: Initial {
                values: initial,
                context,
            },
        })
    }
}

impl<M: Matrix<T = f64>, F, G> Op for Equations<M, F, G> {
    type T = f64;
    type V = M::V;
    type M = M;
    type C = M::C;
    fn context(&self) -> &Self::C {
        &self.rhs.context
    }
    fn nstates(&self) -> usize {
        self.rhs.n
    }
    fn nout(&self) -> usize {
        self.rhs.n
    }
    fn nparams(&self) -> usize {
        0
    }
}

impl<M: Matrix<T = f64>, F, G> Op for Rhs<M, F, G> {
    type T = f64;
    type V = M::V;
    type M = M;
    type C = M::C;
    fn context(&self) -> &Self::C {
        &self.context
    }
    fn nstates(&self) -> usize {
        self.n
    }
    fn nout(&self) -> usize {
        self.n
    }
    fn nparams(&self) -> usize {
        0
    }
}

impl<M: Matrix<T = f64>> Op for Initial<M> {
    type T = f64;
    type V = M::V;
    type M = M;
    type C = M::C;
    fn context(&self) -> &Self::C {
        &self.context
    }
    fn nstates(&self) -> usize {
        self.values.len()
    }
    fn nout(&self) -> usize {
        self.values.len()
    }
    fn nparams(&self) -> usize {
        0
    }
}

impl<M: Matrix<T = f64>, F, G> NonLinearOp for Rhs<M, F, G>
where
    F: Fn(&[f64], f64, &mut [f64]),
{
    fn call_inplace(&self, x: &M::V, t: f64, y: &mut M::V) {
        y.for_each_batch_host([x], |out, [x], _| (self.evaluate)(x, t, out));
    }
}

impl<M: Matrix<T = f64>, F, G> NonLinearOpJacobian for Rhs<M, F, G>
where
    F: Fn(&[f64], f64, &mut [f64]),
    G: Fn(&[f64], f64, &[f64], &mut [f64]),
{
    fn jac_mul_inplace(&self, x: &M::V, t: f64, v: &M::V, y: &mut M::V) {
        y.for_each_batch_host([x, v], |out, [x, v], _| {
            (self.jacobian_action)(x, t, v, out)
        });
    }
    fn jacobian_sparsity(&self) -> Option<M::Sparsity> {
        self.sparsity.clone()
    }
    fn jacobian_inplace(&self, x: &M::V, t: f64, y: &mut M) {
        if let Some(coloring) = &self.coloring {
            coloring.jacobian_inplace(self, x, t, y);
        } else {
            self._default_jacobian_inplace(x, t, y);
        }
    }
}

impl<M: Matrix<T = f64>> ConstantOp for Initial<M> {
    fn call_inplace(&self, _t: f64, y: &mut M::V) {
        y.for_each_batch_host([], |out, [], _| out.copy_from_slice(&self.values));
    }
}

impl<'a, M: Matrix<T = f64>, F, G> OdeEquationsRef<'a> for Equations<M, F, G>
where
    F: Fn(&[f64], f64, &mut [f64]),
    G: Fn(&[f64], f64, &[f64], &mut [f64]),
{
    type Rhs = &'a Rhs<M, F, G>;
    type Init = &'a Initial<M>;
    type Mass = UnitCallable<M>;
    type Root = UnitCallable<M>;
    type Out = UnitCallable<M>;
    type Reset = UnitCallable<M>;
}

impl<M: Matrix<T = f64>, F, G> OdeEquations for Equations<M, F, G>
where
    F: Fn(&[f64], f64, &mut [f64]),
    G: Fn(&[f64], f64, &[f64], &mut [f64]),
{
    fn rhs(&self) -> &Rhs<M, F, G> {
        &self.rhs
    }
    fn init(&self) -> &Initial<M> {
        &self.initial
    }
    fn mass(&self) -> Option<UnitCallable<M>> {
        None
    }
    fn set_params(&mut self, _p: &M::V) {}
    fn get_params(&self, _p: &mut M::V) {}
}

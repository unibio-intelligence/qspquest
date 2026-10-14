use super::{jacobian_update::SolverState, runge_kutta::Rk, OdeSolverStatistics};
use crate::{
    error::DiffsolError, error::OdeSolverError, ode_solver_error, op::sdirk::SdirkCallable,
    DefaultDenseMatrix, DenseMatrix, ExplicitRkConfig, LinearSolver, Matrix, MatrixOp, NoAug,
    NonLinearOp, NonLinearOpJacobian, NonLinearOpTimePartial, OdeEquationsImplicit,
    OdeSolverMethod, OdeSolverProblem, OdeSolverState, OdeSolverStopReason, Op, RkState, Scalar,
    StateRef, StateRefMut, Tableau, Vector,
};
use num_traits::{FromPrimitive, One, Signed, ToPrimitive, Zero};

// Reorthogonalized, pivoted Gram-Schmidt on a scaled constant mass matrix.
// Cache the row space (differential state increments) and the orthogonal
// complement of the column space (algebraic equations), independently of backend.
fn orthonormal_basis<T: Scalar>(mut candidates: Vec<Vec<T>>, n: usize) -> Vec<Vec<T>> {
    let mut basis: Vec<Vec<T>> = Vec::new();
    let threshold = T::EPSILON * T::from_f64((8 * n) as f64).unwrap();
    while !candidates.is_empty() {
        for v in &mut candidates {
            for _ in 0..2 {
                for q in &basis {
                    let dot = v.iter().zip(q).fold(T::zero(), |sum, (a, b)| sum + *a * *b);
                    for (a, b) in v.iter_mut().zip(q) {
                        *a -= dot * *b;
                    }
                }
            }
        }
        let norms: Vec<T> = candidates
            .iter()
            .map(|v| v.iter().fold(T::zero(), |sum, x| sum + *x * *x).sqrt())
            .collect();
        let pivot = (0..norms.len())
            .max_by(|&a, &b| {
                norms[a]
                    .partial_cmp(&norms[b])
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();
        let norm = norms[pivot];
        if norm <= threshold {
            break;
        }
        let mut q = candidates.swap_remove(pivot);
        for x in &mut q {
            *x /= norm;
        }
        basis.push(q);
    }
    basis
}
fn mass_error_subspaces<T: Scalar>(mut rows: Vec<Vec<T>>) -> (Vec<Vec<T>>, Vec<Vec<T>>) {
    let n = rows.len();
    let largest = rows
        .iter()
        .flatten()
        .fold(T::zero(), |a, b| if a > b.abs() { a } else { b.abs() });
    for x in rows.iter_mut().flatten() {
        *x /= largest;
    }
    let columns = (0..n)
        .map(|j| rows.iter().map(|r| r[j]).collect())
        .collect();
    let differential = orthonormal_basis(rows, n);
    let column_space = orthonormal_basis(columns, n);
    let mut complement = Vec::new();
    for i in 0..n {
        let mut q = vec![T::zero(); n];
        q[i] = T::one();
        for _ in 0..2 {
            for v in &column_space {
                let dot = q.iter().zip(v).fold(T::zero(), |sum, (a, b)| sum + *a * *b);
                for (a, b) in q.iter_mut().zip(v) {
                    *a -= dot * *b;
                }
            }
        }
        complement.push(q);
    }
    let algebraic = orthonormal_basis(complement, n);
    (differential, algebraic)
}

/// Result of bounded convergence checking at caller-selected observation times.
#[derive(Clone, Debug)]
pub struct RefinedTrajectory<T> {
    /// The finer sampled trajectory that satisfied the convergence criterion.
    pub samples: Vec<Vec<T>>,
    /// Number of complete solves, including the first unrefined solve.
    pub passes: usize,
    /// Maximum componentwise change in units of the original tolerance scale.
    pub maximum_scaled_change: f64,
    /// Multiplier applied to both original local tolerances in the final solve.
    pub local_tolerance_factor: T,
}

/// Refine complete sampled trajectories with the same method and initial data.
///
/// `solve(factor)` must restart the same interval and return the same observation
/// times/components with local tolerances multiplied by `factor`. Factors are
/// 1, 0.1, 0.01, ...; events must not be replayed outside that interval. The
/// callback owns work limits and must share its budget across all passes.
/// The finer trajectory is returned when successive samples agree within
/// agreement_fraction * (atol_i + rtol * max(abs(coarse_i), abs(fine_i))).
/// A fraction of 0.25 leaves room for the unresolved error of the finer solve. This is sampled
/// convergence evidence, not a certified bound on the true global error.
/// Failed solves, non-finite data, shape changes and exhausted passes fail closed.
/// Refinement also fails closed after three changes improve by less than 5%,
/// or after a change below one tolerance unit grows more than eightfold (pass >=4).
/// Diagnostics report the pass, sample change and local tolerance factor; these
/// guards identify stalled/deteriorating convergence, not its physical cause.
/// Local solver/controller defaults and the method's dense extension are unchanged.
pub fn refine_sampled_trajectory<T, E>(
    rtol: T,
    atol: &[T],
    maximum_passes: usize,
    agreement_fraction: f64,
    mut solve: impl FnMut(T) -> Result<Vec<Vec<T>>, E>,
    map_error: impl Fn(DiffsolError) -> E,
) -> Result<RefinedTrajectory<T>, E>
where
    T: Scalar,
{
    let invalid = |message: &str| map_error(ode_solver_error!(Other, message));
    let relative = rtol.to_f64().unwrap_or(f64::NAN);
    let absolute: Vec<_> = atol
        .iter()
        .map(|v| v.to_f64().unwrap_or(f64::NAN))
        .collect();
    if !agreement_fraction.is_finite()
        || agreement_fraction <= 0.0
        || agreement_fraction > 1.0
        || !(2..=16).contains(&maximum_passes)
        || !relative.is_finite()
        || relative <= 0.0
        || absolute.is_empty()
        || absolute.iter().any(|v| !v.is_finite() || *v <= 0.0)
    {
        return Err(invalid(
            "Invalid sampled refinement tolerances or pass limit",
        ));
    }
    let mut previous: Option<Vec<Vec<T>>> = None;
    let mut previous_change: Option<f64> = None;
    let mut stalled_changes = 0;
    let mut factor = T::one();
    for pass in 1..=maximum_passes {
        let current = solve(factor)?;
        let width = current.first().map_or(0, Vec::len);
        if width == 0
            || (absolute.len() != 1 && absolute.len() != width)
            || current.iter().any(|row| {
                row.len() != width || row.iter().any(|v| !v.to_f64().is_some_and(f64::is_finite))
            })
        {
            return Err(invalid("Invalid sampled refinement trajectory"));
        }
        if let Some(coarse) = &previous {
            if coarse.len() != current.len() || coarse[0].len() != width {
                return Err(invalid("Sampled refinement trajectory shape changed"));
            }
            let mut maximum = 0.0_f64;
            for (a, b) in coarse.iter().zip(&current) {
                for (i, (a, b)) in a.iter().zip(b).enumerate() {
                    let a = a.to_f64().unwrap();
                    let b = b.to_f64().unwrap();
                    let scale = absolute[if absolute.len() == 1 { 0 } else { i }]
                        + relative * a.abs().max(b.abs());
                    let change = (a - b).abs() / scale;
                    if !scale.is_finite() || !change.is_finite() {
                        return Err(invalid("Non-finite sampled refinement error scale"));
                    }
                    maximum = maximum.max(change);
                }
            }
            if maximum <= agreement_fraction {
                return Ok(RefinedTrajectory {
                    samples: current,
                    passes: pass,
                    maximum_scaled_change: maximum,
                    local_tolerance_factor: factor,
                });
            }
            if let Some(before) = previous_change {
                if pass >= 4 && before <= 1.0 && maximum > 8.0 * before {
                    return Err(invalid(&format!("Sampled refinement worsened at pass {pass}: change {maximum:e} tolerance units (previous {before:e}); local tolerance factor {:e}",factor.to_f64().unwrap())));
                }
                stalled_changes = if maximum >= 0.95 * before {
                    stalled_changes + 1
                } else {
                    0
                };
                if stalled_changes >= 3 {
                    return Err(invalid(&format!("Sampled refinement stalled at pass {pass}: change {maximum:e} tolerance units; local tolerance factor {:e}",factor.to_f64().unwrap())));
                }
            }
            previous_change = Some(maximum);
        }
        previous = Some(current);
        factor *= T::from_f64(0.1).unwrap();
    }
    Err(invalid(
        &format!("Sampled trajectory did not converge within {maximum_passes} refinement passes; last change {:e} tolerance units",previous_change.unwrap_or(f64::NAN)),
    ))
}

/// A tableau-driven class of linearly implicit Rosenbrock-Wanner methods.
/// Each attempted step freezes the Jacobian and reuses one linear factorization.
/// A tableau's continuous extension supplies dense output; otherwise Hermite interpolation is used.
/// For ODEs, `state().dy` is the RHS at the accepted endpoint. For mass-matrix
/// DAEs it is the continuous extension's endpoint derivative.
/// Constant mass matrices are supported for index-1 DAEs with a continuous extension.
/// Rosenbrock23 controls differential embedded errors and
/// algebraic endpoint residuals separately. Algebraic RHS rows must be scaled so
/// their residuals have the meaning of the corresponding absolute tolerances.
/// Non-diagonal constant mass uses cached orthogonal subspaces with numerical
/// rank threshold 8*n*epsilon after scaling by the largest matrix entry.
/// This does not project dense output or provide a global-error bound.
/// Integrated outputs use quadrature along the required state continuous extension;
/// Rodas5P outputs have global order four.
/// After mutating the state, keep `dy` consistent: Hermite interpolation uses it
/// as the left-endpoint derivative for tableaus without a continuous extension.
/// Forward and adjoint sensitivities are not yet implemented for this class.
pub struct Rosenbrock<'a, Eqn, LS, M = <<Eqn as Op>::V as DefaultDenseMatrix>::M>
where
    Eqn: OdeEquationsImplicit,
    Eqn::V: DefaultDenseMatrix<T = Eqn::T, C = Eqn::C>,
    M: DenseMatrix<T = Eqn::T, V = Eqn::V, C = Eqn::C>,
    LS: LinearSolver<Eqn::M>,
{
    rk: Rk<'a, Eqn, M>,
    linear_solver: LS,
    op: SdirkCallable<&'a Eqn>,
    ft: Eqn::V,
    scratch: Eqn::V,
    config: ExplicitRkConfig<Eqn::T>,
    discontinuity_stop: Option<Eqn::T>,
    time_derivative_within_step: bool,
    maximum_step: Option<Eqn::T>,
    state_compensation: Eqn::V,
    trial_compensation: Eqn::V,
    time_compensation: Eqn::T,
    algebraic_indices: Vec<usize>,
    differential_basis: Vec<Vec<Eqn::T>>,
    algebraic_basis: Vec<Vec<Eqn::T>>,
}
impl<Eqn, LS, M> Clone for Rosenbrock<'_, Eqn, LS, M>
where
    Eqn: OdeEquationsImplicit,
    Eqn::V: DefaultDenseMatrix<T = Eqn::T, C = Eqn::C>,
    M: DenseMatrix<T = Eqn::T, V = Eqn::V, C = Eqn::C>,
    LS: LinearSolver<Eqn::M>,
{
    fn clone(&self) -> Self {
        let op = self.op.clone_state(self.rk.problem().eqn());
        let mut linear_solver = LS::default();
        linear_solver.set_problem(&op);
        Self {
            rk: self.rk.clone(),
            linear_solver,
            op,
            ft: self.ft.clone(),
            scratch: self.scratch.clone(),
            config: self.config.clone(),
            discontinuity_stop: self.discontinuity_stop,
            time_derivative_within_step: self.time_derivative_within_step,
            maximum_step: self.maximum_step,
            state_compensation: self.state_compensation.clone(),
            trial_compensation: self.trial_compensation.clone(),
            time_compensation: self.time_compensation,
            algebraic_indices: self.algebraic_indices.clone(),
            differential_basis: self.differential_basis.clone(),
            algebraic_basis: self.algebraic_basis.clone(),
        }
    }
}
impl<'a, Eqn, LS, M> Rosenbrock<'a, Eqn, LS, M>
where
    Eqn: OdeEquationsImplicit,
    Eqn::V: DefaultDenseMatrix<T = Eqn::T, C = Eqn::C>,
    M: DenseMatrix<T = Eqn::T, V = Eqn::V, C = Eqn::C>,
    LS: LinearSolver<Eqn::M>,
{
    pub fn new(
        problem: &'a OdeSolverProblem<Eqn>,
        state: RkState<Eqn::V>,
        tableau: Tableau<Eqn::T>,
        mut linear_solver: LS,
    ) -> Result<Self, DiffsolError> {
        let invalid = || {
            ode_solver_error!(InvalidTableau, "Expected finite, strictly lower triangular Rosenbrock coefficients with positive gamma and orders")
        };
        let row = tableau.rosenbrock().ok_or_else(invalid)?;
        let n = tableau.s();
        let finite = |x: Eqn::T| x.to_f64().is_some_and(f64::is_finite);
        if n == 0
            || tableau.order() == 0
            || row.error_order == 0
            || !finite(row.gamma)
            || row.gamma <= Eqn::T::zero()
            || row.time.len() != n
            || row.coupling.nrows() != n
            || row.coupling.ncols() != n
        {
            return Err(invalid());
        }
        for i in 0..n {
            if ![tableau.b()[i], tableau.c()[i], tableau.d()[i], row.time[i]]
                .into_iter()
                .all(finite)
            {
                return Err(invalid());
            }
            for j in 0..n {
                let a = tableau.a(i, j);
                let c = row.coupling[(i, j)];
                if !finite(a)
                    || !finite(c)
                    || (j >= i && (a != Eqn::T::zero() || c != Eqn::T::zero()))
                {
                    return Err(invalid());
                }
            }
        }
        if let Some(beta) = tableau.beta_t() {
            if beta.nrows() == 0 || beta.ncols() != n {
                return Err(invalid());
            }
            for i in 0..n {
                if !beta.as_col_slice(i).iter().copied().all(finite) {
                    return Err(invalid());
                }
            }
        } else if problem.eqn.mass().is_some() {
            return Err(ode_solver_error!(
                InvalidTableau,
                "Mass-matrix Rosenbrock methods require a continuous extension"
            ));
        }
        if !state.s.is_empty() {
            return Err(OdeSolverError::SensitivityNotSupported.into());
        }
        if problem.integrate_out && tableau.beta_t().is_none() {
            return Err(ode_solver_error!(
                InvalidTableau,
                "Integrated Rosenbrock outputs require a continuous extension"
            ));
        }
        let ft = Eqn::V::zeros(state.y.len(), problem.context().clone());
        let op = SdirkCallable::new(&problem.eqn, Eqn::T::one(), problem.context().clone());
        // Rosenbrock23's embedded third-order ODE formula is not an
        // algebraic-state error estimate. Control endpoint constraints separately.
        // Diagonal mass retains its inexpensive exact-coordinate specialization.
        let mut algebraic_indices = Vec::new();
        let mut differential_basis = Vec::new();
        let mut algebraic_basis = Vec::new();
        if row.algebraic_error_control && problem.eqn.mass().is_some() {
            let mass = op.mass(state.t);
            let mut algebraic = vec![true; state.y.len()];
            let mut diagonal = true;
            let (positions, values) = mass.triplet_iter();
            for ((i, j), value) in positions.zip(values) {
                if value != Eqn::T::zero() {
                    diagonal &= i == j;
                    algebraic[i] = false;
                }
            }
            if diagonal {
                algebraic_indices = algebraic
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, zero)| zero.then_some(i))
                    .collect();
            } else {
                let mut rows = vec![vec![Eqn::T::zero(); state.y.len()]; state.y.len()];
                let (positions, values) = mass.triplet_iter();
                for ((i, j), value) in positions.zip(values) {
                    rows[i][j] = value;
                }
                (differential_basis, algebraic_basis) = mass_error_subspaces(rows);
            }
        }
        linear_solver.set_problem(&op);
        Ok(Self {
            rk: Rk::new(problem, state, tableau)?,
            linear_solver,
            op,
            scratch: ft.clone(),
            config: ExplicitRkConfig::new(&problem.ode_options),
            discontinuity_stop: None,
            time_derivative_within_step: false,
            maximum_step: None,
            state_compensation: ft.clone(),
            trial_compensation: ft.clone(),
            time_compensation: Eqn::T::zero(),
            ft,
            algebraic_indices,
            differential_basis,
            algebraic_basis,
        })
    }
    /// Bound the absolute step size, including when adaptive endpoint estimates
    /// permit larger steps. This can improve dense-output and derivative accuracy.
    /// A step bound supplements tolerances; it is not a global-error guarantee.
    pub fn set_maximum_step(&mut self, maximum: Eqn::T) -> Result<(), DiffsolError> {
        if maximum <= Eqn::T::zero() || !maximum.to_f64().is_some_and(f64::is_finite) {
            return Err(ode_solver_error!(
                Other,
                "Maximum Rosenbrock step must be finite and positive"
            ));
        }
        self.maximum_step = Some(maximum);
        Ok(())
    }

    /// Restrict numerical time-derivative probes to the attempted step.
    /// Use this for piecewise forcing; analytic overrides still take precedence.
    pub fn set_time_derivative_within_step(&mut self, enabled: bool) {
        self.time_derivative_within_step = enabled;
    }

    /// Stop at a known forcing jump, evaluating endpoint stages on its incoming side.
    /// Numerical time probes remain inside the step until this stop is reached.
    pub fn set_discontinuity_stop_time(&mut self, tstop: Eqn::T) -> Result<(), DiffsolError> {
        self.rk.set_stop_time(tstop)?;
        self.discontinuity_stop = Some(tstop);
        Ok(())
    }

    fn endpoint_eval_time(&self, h: Eqn::T) -> Result<Eqn::T, DiffsolError> {
        let start = self.rk.state().t;
        let end = start + h;
        let Some(stop) = self.discontinuity_stop else {
            return Ok(end);
        };
        let roundoff = Eqn::T::from_f64(100.0).unwrap() * Eqn::T::EPSILON * (end.abs() + h.abs());
        if (end - stop).abs() > roundoff {
            return Ok(end);
        }
        let nominal = Eqn::T::EPSILON * (Eqn::T::one() + end.abs());
        let quarter = h.abs() / Eqn::T::from_f64(4.0).unwrap();
        let delta = if nominal < quarter { nominal } else { quarter };
        let shifted = if h >= Eqn::T::zero() {
            end - delta
        } else {
            end + delta
        };
        if (h >= Eqn::T::zero() && start < shifted && shifted < end)
            || (h < Eqn::T::zero() && end < shifted && shifted < start)
        {
            Ok(shifted)
        } else {
            Err(ode_solver_error!(
                Other,
                "Rosenbrock discontinuity has no representable incoming endpoint"
            ))
        }
    }

    fn update_time_partial(&mut self, h: Eqn::T) -> Result<(), DiffsolError> {
        let state = self.rk.state();
        let rhs = self.rk.problem().eqn.rhs();
        if rhs.time_partial_inplace(&state.y, state.t, &mut self.ft) {
            return Ok(());
        }
        if !self.time_derivative_within_step && self.discontinuity_stop.is_none() {
            rhs.time_derive_inplace(&state.y, state.t, &mut self.ft);
            return Ok(());
        }
        let t = state.t;
        let third = Eqn::T::from_f64(1.0 / 3.0).unwrap();
        let nominal = Eqn::T::EPSILON.cbrt() * (Eqn::T::one() + t.abs());
        let bounded = if nominal < h.abs() * third {
            nominal
        } else {
            h.abs() * third
        };
        let delta = if h >= Eqn::T::zero() {
            bounded
        } else {
            -bounded
        };
        let p1 = t + delta;
        let p2 = t + delta + delta;
        let end = t + h;
        if !((h >= Eqn::T::zero() && t < p1 && p1 < p2 && p2 < end)
            || (h < Eqn::T::zero() && end < p2 && p2 < p1 && p1 < t))
        {
            return Err(ode_solver_error!(
                Other,
                "Rosenbrock time-derivative probes cannot advance within this step"
            ));
        }
        let f0 = rhs.call(&state.y, t);
        rhs.call_inplace(&state.y, p1, &mut self.ft);
        rhs.call_inplace(&state.y, p2, &mut self.scratch);
        self.ft.axpy(-Eqn::T::one(), &f0, Eqn::T::one());
        self.scratch.axpy(-Eqn::T::one(), &f0, Eqn::T::one());
        // Account for the actual representable probe spacings; subtract the
        // baseline first so an autonomous RHS produces exactly zero.
        let a = p1 - t;
        let b = p2 - t;
        self.ft *= crate::scale(b / (a * (b - a)));
        self.ft
            .axpy(-a / (b * (b - a)), &self.scratch, Eqn::T::one());
        Ok(())
    }

    /// Sample the existing extension, then enforce index-1 algebraic consistency.
    ///
    /// This opt-in observation correction leaves stages, the accepted trajectory,
    /// and the published dense extension unchanged. Corrections lie in ker(M),
    /// so M*y is preserved. It is useful when an approximate Jacobian degrades
    /// algebraic dense output. It does not restore the extension's order or
    /// certify differential-state accuracy. Use only within a smooth step.
    /// The supplied Jacobian must permit convergence of the reduced constraint
    /// Newton solve. Residuals are scaled by equation absolute tolerances;
    /// singular/non-finite corrections and exhausted iterations fail closed.
    pub fn interpolate_consistent(
        &self,
        t: Eqn::T,
        maximum_iterations: usize,
    ) -> Result<Eqn::V, DiffsolError> {
        if !(1..=16).contains(&maximum_iterations) || !t.to_f64().is_some_and(f64::is_finite) {
            return Err(ode_solver_error!(
                Other,
                "Invalid algebraic interpolation iteration limit"
            ));
        }
        let mut y = self.interpolate(t)?;
        if (0..y.len()).any(|i| !y.get_index(i).to_f64().is_some_and(f64::is_finite)) {
            return Err(ode_solver_error!(Other, "Non-finite interpolation state"));
        }
        if self.problem().eqn.mass().is_none() {
            return Ok(y);
        }
        let n = y.len();
        let mass = self.op.mass(self.rk.state().t);
        let mut rows = vec![vec![Eqn::T::zero(); n]; n];
        let (positions, values) = mass.triplet_iter();
        for ((i, j), v) in positions.zip(values) {
            rows[i][j] = v;
        }
        drop(mass);
        let diagonal = (0..n).all(|i| (0..n).all(|j| i == j || rows[i][j] == Eqn::T::zero()));
        let (left, right) = if diagonal {
            let q: Vec<Vec<_>> = (0..n)
                .filter(|&i| rows[i][i] == Eqn::T::zero())
                .map(|i| {
                    let mut q = vec![Eqn::T::zero(); n];
                    q[i] = Eqn::T::one();
                    q
                })
                .collect();
            (q.clone(), q)
        } else {
            let (differential, left) = mass_error_subspaces(rows);
            let mut candidates = Vec::new();
            for i in 0..n {
                let mut q = vec![Eqn::T::zero(); n];
                q[i] = Eqn::T::one();
                for _ in 0..2 {
                    for v in &differential {
                        let dot = q
                            .iter()
                            .zip(v)
                            .fold(Eqn::T::zero(), |a, (b, c)| a + *b * *c);
                        for (a, b) in q.iter_mut().zip(v) {
                            *a -= dot * *b;
                        }
                    }
                }
                candidates.push(q);
            }
            (left, orthonormal_basis(candidates, n))
        };
        let k = left.len();
        if k == 0 {
            return Ok(y);
        }
        if right.len() != k {
            return Err(ode_solver_error!(Other, "Inconsistent mass null spaces"));
        }
        let ctx = self.problem().context().clone();
        let rhs = self.problem().eqn.rhs();
        let tolerances: Vec<_> = left
            .iter()
            .map(|q| {
                q.iter()
                    .enumerate()
                    .fold(Eqn::T::zero(), |a, (i, v)| {
                        let z = *v * self.problem().atol.get_index(i);
                        a + z * z
                    })
                    .sqrt()
            })
            .collect();
        let mut f = Eqn::V::zeros(n, ctx.clone());
        let mut direction = f.clone();
        let mut jac = f.clone();
        for iteration in 0..=maximum_iterations {
            rhs.call_inplace(&y, t, &mut f);
            let residuals: Vec<_> = left
                .iter()
                .map(|q| {
                    q.iter()
                        .enumerate()
                        .fold(Eqn::T::zero(), |a, (i, v)| a + *v * f.get_index(i))
                })
                .collect();
            if residuals.iter().zip(&tolerances).all(|(r, a)| {
                (*r / *a)
                    .to_f64()
                    .is_some_and(|v| v.is_finite() && v.abs() <= 0.01)
            }) {
                return Ok(y);
            }
            if iteration == maximum_iterations {
                break;
            }
            let mut positions = Vec::with_capacity(k * k);
            let mut values = Vec::with_capacity(k * k);
            for (j, q) in right.iter().enumerate() {
                for (i, v) in q.iter().enumerate() {
                    direction.set_index(i, *v);
                }
                rhs.jac_mul_inplace(&y, t, &direction, &mut jac);
                for (i, q) in left.iter().enumerate() {
                    positions.push((i, j));
                    values.push(
                        q.iter()
                            .enumerate()
                            .fold(Eqn::T::zero(), |a, (r, v)| a + *v * jac.get_index(r)),
                    );
                }
            }
            if values
                .iter()
                .chain(&residuals)
                .any(|x| !x.to_f64().is_some_and(f64::is_finite))
            {
                return Err(ode_solver_error!(
                    Other,
                    "Non-finite algebraic interpolation correction"
                ));
            }
            let matrix = Eqn::M::try_from_triplets(k, k, positions, values, ctx.clone())?;
            let op = MatrixOp::new(matrix);
            let mut solver = LS::default();
            solver.set_problem(&op);
            let mut correction =
                Eqn::V::from_vec(residuals.into_iter().map(|x| -x).collect(), ctx.clone());
            LinearSolver::set_linearisation(&mut solver, &op, &correction, t);
            solver.solve_in_place(&mut correction)?;
            for (j, q) in right.iter().enumerate() {
                for (i, v) in q.iter().enumerate() {
                    y.set_index(i, y.get_index(i) + *v * correction.get_index(j));
                }
            }
            if (0..n).any(|i| !y.get_index(i).to_f64().is_some_and(f64::is_finite)) {
                return Err(ode_solver_error!(
                    Other,
                    "Non-finite algebraic interpolation state"
                ));
            }
        }
        Err(ode_solver_error!(
            Other,
            "Algebraic interpolation did not converge within the iteration limit"
        ))
    }

    pub fn get_statistics(&self) -> &OdeSolverStatistics {
        self.rk.get_statistics()
    }
}
impl<'a, Eqn, LS, M> OdeSolverMethod<'a, Eqn> for Rosenbrock<'a, Eqn, LS, M>
where
    Eqn: OdeEquationsImplicit,
    Eqn::V: DefaultDenseMatrix<T = Eqn::T, C = Eqn::C>,
    M: DenseMatrix<T = Eqn::T, V = Eqn::V, C = Eqn::C>,
    LS: LinearSolver<Eqn::M>,
{
    type State = RkState<Eqn::V>;
    type Config = ExplicitRkConfig<Eqn::T>;
    fn config(&self) -> &Self::Config {
        &self.config
    }
    fn config_mut(&mut self) -> &mut Self::Config {
        &mut self.config
    }
    fn problem(&self) -> &'a OdeSolverProblem<Eqn> {
        self.rk.problem()
    }
    fn state(&self) -> StateRef<'_, Eqn::V> {
        self.rk.state().as_ref()
    }
    fn state_mut(&mut self) -> StateRefMut<'_, Eqn::V> {
        self.state_compensation.fill(Eqn::T::zero());
        self.time_compensation = Eqn::T::zero();
        self.rk.state_mut().as_mut()
    }
    fn state_clone(&self) -> Self::State {
        self.rk.state().clone()
    }
    fn checkpoint(&mut self) -> Self::State {
        self.rk.state().clone()
    }
    fn into_state(self) -> Self::State {
        self.rk.into_state()
    }
    fn set_state(&mut self, state: Self::State) {
        self.state_compensation.fill(Eqn::T::zero());
        self.time_compensation = Eqn::T::zero();
        self.rk.set_state(state);
    }
    fn order(&self) -> usize {
        self.rk.order()
    }
    fn jacobian(&self) -> Option<std::cell::Ref<'_, Eqn::M>> {
        // Evaluate the RHS Jacobian at y, rather than the SDIRK stage phi + c*y.
        self.op.zero_phi();
        Some(self.op.rhs_jac(&self.rk.state().y, self.rk.state().t))
    }
    fn mass(&self) -> Option<std::cell::Ref<'_, Eqn::M>> {
        self.problem()
            .eqn
            .mass()
            .map(|_| self.op.mass(self.rk.state().t))
    }
    fn apply_reset(&mut self) -> Result<(), DiffsolError> {
        let problem = self.problem();
        self.rk
            .state_mut()
            .as_mut()
            .apply_reset_with_mass::<LS, _>(problem)
    }
    fn step(&mut self) -> Result<OdeSolverStopReason<Eqn::T>, DiffsolError> {
        let mut h = self.rk.start_step()?;
        if let Some(maximum) = self.maximum_step {
            if h.abs() > maximum {
                h = h.signum() * maximum;
            }
        }
        if h.abs() < self.config.minimum_timestep {
            return Err(OdeSolverError::StepSizeTooSmall {
                time: self.rk.state().t.to_f64().unwrap(),
            }
            .into());
        }
        let gamma = self.rk.tableau().rosenbrock().unwrap().gamma;
        let mut attempts = 0;
        let (factor, error) = loop {
            let state = self.rk.state();
            self.op.zero_phi();
            if state.t + h == state.t || state.t + (h - self.time_compensation) == state.t {
                return Err(OdeSolverError::StepSizeTooSmall {
                    time: self.rk.state().t.to_f64().unwrap(),
                }
                .into());
            }
            self.op.set_h(gamma * h);
            self.op.set_jacobian_is_stale();
            LinearSolver::set_linearisation(&mut self.linear_solver, &self.op, &state.y, state.t);
            self.update_time_partial(h)?;
            let endpoint_time = self.endpoint_eval_time(h)?;
            self.rk
                .statistics_mut()
                .record_linear_solver_setup(if attempts == 0 {
                    SolverState::StepSuccess
                } else {
                    SolverState::ErrorTestFail
                });
            self.rk.start_step_attempt(h, None::<&mut NoAug<Eqn>>);
            for i in 0..self.rk.tableau().s() {
                self.rk.do_stage_rosenbrock(
                    i,
                    h,
                    &self.op,
                    &mut self.linear_solver,
                    &self.ft,
                    endpoint_time,
                )?;
            }
            self.rk.finish_step_rosenbrock_compensated(
                h,
                endpoint_time,
                Some((&self.state_compensation, &mut self.trial_compensation)),
            );
            if self.problem().integrate_out {
                self.rk.integrate_rosenbrock_outputs(h, &mut self.scratch);
            }
            // Use diffsol's shared RK error scaling at the starting state, not the endpoint.
            let error = if self.rk.rosenbrock_trial_is_finite() {
                let embedded = self.rk.error_norm(h, None::<&mut NoAug<Eqn>>, |error| {
                    for &i in &self.algebraic_indices {
                        error.set_index(i, Eqn::T::zero());
                    }
                    if !self.algebraic_basis.is_empty() {
                        let original: Vec<_> =
                            (0..error.len()).map(|i| error.get_index(i)).collect();
                        error.fill(Eqn::T::zero());
                        for q in &self.differential_basis {
                            let dot = q
                                .iter()
                                .zip(&original)
                                .fold(Eqn::T::zero(), |sum, (a, b)| sum + *a * *b);
                            for (i, x) in q.iter().enumerate() {
                                error.set_index(i, error.get_index(i) + dot * *x);
                            }
                        }
                    }
                    Ok(())
                })?;
                if self.algebraic_indices.is_empty() && self.algebraic_basis.is_empty() {
                    embedded
                } else {
                    self.op.eqn().rhs().call_inplace(
                        self.rk.rosenbrock_trial_y(),
                        endpoint_time,
                        &mut self.scratch,
                    );
                    let mut constraint = Eqn::T::zero();
                    for &i in &self.algebraic_indices {
                        let value = self.scratch.get_index(i) / self.problem().atol.get_index(i);
                        if !value.to_f64().is_some_and(f64::is_finite) {
                            constraint = Eqn::T::from_f64(f64::INFINITY).unwrap();
                            break;
                        }
                        constraint += value * value;
                    }
                    for q in &self.algebraic_basis {
                        let residual = q.iter().enumerate().fold(Eqn::T::zero(), |sum, (i, x)| {
                            sum + *x * self.scratch.get_index(i)
                        });
                        let tolerance = q
                            .iter()
                            .enumerate()
                            .fold(Eqn::T::zero(), |sum, (i, x)| {
                                let a = *x * self.problem().atol.get_index(i);
                                sum + a * a
                            })
                            .sqrt();
                        let value = residual / tolerance;
                        if !value.to_f64().is_some_and(f64::is_finite) {
                            constraint = Eqn::T::from_f64(f64::INFINITY).unwrap();
                            break;
                        }
                        constraint += value * value;
                    }
                    constraint /= Eqn::T::from_f64(self.scratch.len() as f64).unwrap();
                    let norm = embedded.sqrt() + constraint.sqrt();
                    norm * norm
                }
            } else {
                Eqn::T::from_f64(f64::INFINITY).unwrap()
            };
            let factor = if error.to_f64().is_some_and(f64::is_finite) {
                self.rk.factor(
                    error,
                    1.0,
                    self.config.minimum_timestep_shrink,
                    self.config.maximum_timestep_shrink,
                    self.config.minimum_timestep_growth,
                    self.config.maximum_timestep_growth,
                )
            } else {
                // Non-finite stage evaluations are recoverable trials. Shrink
                // deterministically, bounded by the usual failure/minimum-step guards.
                self.config.minimum_timestep_shrink
            };
            if error < Eqn::T::one() {
                break (factor, error);
            }
            h *= factor;
            attempts += 1;
            self.rk.reset_prev_error();
            self.rk.error_test_fail(
                h,
                attempts,
                self.config.maximum_error_test_failures,
                self.config.minimum_timestep,
            )?;
        };
        self.rk.store_rosenbrock_hermite_derivatives(h);
        self.rk.set_prev_error(error);
        let start = self.rk.state().t;
        let increment = h - self.time_compensation;
        let end = start + increment;
        self.time_compensation = (end - start) - increment;
        let reason = self.rk.step_accepted_at_time(h, h * factor, false, end)?;
        self.state_compensation.copy_from(&self.trial_compensation);
        if matches!(reason, OdeSolverStopReason::TstopReached) {
            self.discontinuity_stop = None;
            self.time_compensation = Eqn::T::zero();
        }
        Ok(reason)
    }
    fn set_stop_time(&mut self, t: Eqn::T) -> Result<(), DiffsolError> {
        self.rk.set_stop_time(t)?;
        self.discontinuity_stop = None;
        Ok(())
    }
    fn interpolate_inplace(&self, t: Eqn::T, out: &mut Eqn::V) -> Result<(), DiffsolError> {
        self.rk.interpolate_inplace(t, out)
    }
    fn interpolate_dy_inplace(&self, t: Eqn::T, out: &mut Eqn::V) -> Result<(), DiffsolError> {
        self.rk.interpolate_dy_inplace(t, out)
    }
    fn interpolate_out_inplace(&self, t: Eqn::T, out: &mut Eqn::V) -> Result<(), DiffsolError> {
        self.rk.interpolate_out_inplace(t, out)
    }
    fn interpolate_sens_inplace(&self, t: Eqn::T, out: &mut Eqn::V) -> Result<(), DiffsolError> {
        self.rk.interpolate_sens_inplace(t, out)
    }
    fn state_mut_back(&mut self, t: Eqn::T) -> Result<(), DiffsolError> {
        self.state_compensation.fill(Eqn::T::zero());
        self.time_compensation = Eqn::T::zero();
        self.rk.state_mut_back(t, self.rk.problem().integrate_out)
    }
}
#[cfg(test)]
#[path = "rosenbrock_harness_tests.rs"]
mod harness_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ode_equations::test_models::{
            exponential_decay::{exponential_decay_problem, exponential_decay_problem_with_root},
            exponential_decay_with_algebraic::{
                exponential_decay_with_algebraic_adjoint_problem,
                exponential_decay_with_algebraic_problem,
            },
            robertson_ode::robertson_ode,
        },
        ode_solver::tests::{
            test_checkpointing, test_config, test_interpolate, test_interpolate_dy,
            test_ode_solver, test_problem, test_state_mut,
        },
        FaerLU, FaerMat, NalgebraLU, NalgebraMat, OdeBuilder, TableauMat, TableauVec, VectorView,
    };
    type Mat = NalgebraMat<f64>;
    type LS = NalgebraLU<f64>;
    fn advance<'a, E: OdeEquationsImplicit<T = f64> + 'a, S: OdeSolverMethod<'a, E>>(
        s: &mut S,
        t: f64,
    ) {
        s.set_stop_time(t).unwrap();
        while s.step().unwrap() != OdeSolverStopReason::TstopReached {}
    }
    macro_rules! contract {
        ($mat:ty, $ls:ty) => {{
            test_state_mut(test_problem::<$mat>(false).rodas5p::<$ls>().unwrap());
            test_interpolate(test_problem::<$mat>(false).rodas5p::<$ls>().unwrap());
            test_interpolate(test_problem::<$mat>(true).rodas5p::<$ls>().unwrap());
            test_interpolate_dy(test_problem::<$mat>(false).rodas5p::<$ls>().unwrap());
            test_config(robertson_ode::<$mat>(false, 1).0.rodas5p::<$ls>().unwrap());
            let (p, sol) = exponential_decay_problem::<$mat>(false);
            test_checkpointing(
                sol,
                p.rodas5p::<$ls>().unwrap(),
                p.rodas5p::<$ls>().unwrap(),
            );
            for use_tstop in [false, true] {
                let (p, sol) = exponential_decay_problem::<$mat>(false);
                let mut s = p.rodas5p::<$ls>().unwrap();
                test_ode_solver(&mut s, sol, None, use_tstop, false);
                let stats = s.get_statistics();
                assert_eq!(
                    stats.number_of_linear_solver_setups,
                    stats.number_of_steps + stats.number_of_error_test_failures
                );
                assert_eq!(stats.number_of_nonlinear_solver_iterations, 0);
                let (p, sol) = exponential_decay_problem_with_root::<$mat>(false, false);
                test_ode_solver(
                    &mut p.rodas5p::<$ls>().unwrap(),
                    sol,
                    None,
                    use_tstop,
                    false,
                );
            }
            let (p, sol) = robertson_ode::<$mat>(false, 1);
            test_ode_solver(&mut p.rodas5p::<$ls>().unwrap(), sol, None, false, false);
        }};
    }
    #[test]
    fn nalgebra_shared_contract() {
        contract!(Mat, LS);
    }
    #[test]
    fn faer_shared_contract() {
        contract!(FaerMat<f64>, FaerLU<f64>);
    }
    #[test]
    fn minimum_timestep_is_checked_before_any_attempt() {
        for tableau in [Tableau::rodas5p(), Tableau::rosenbrock23()] {
            for h in [1e-6, -1e-6] {
                let (p, _) = exponential_decay_problem::<Mat>(false);
                let mut s = p
                    .rosenbrock_solver::<LS, Mat>(p.rodas5p_state::<LS>().unwrap(), tableau)
                    .unwrap();
                *s.state_mut().h = h;
                s.config_mut().minimum_timestep = 1e-5;
                let before = (s.state().t, s.state().y.clone());
                assert!(matches!(
                    s.step(),
                    Err(DiffsolError::OdeSolverError(
                        OdeSolverError::StepSizeTooSmall { .. }
                    ))
                ));
                assert_eq!(s.state().t, before.0);
                s.state().y.assert_eq_st(&before.1, 0.0);
                assert_eq!(s.get_statistics().number_of_linear_solver_setups, 0);
            }
        }
    }
    #[test]
    fn continuous_extension_avoids_endpoint_rhs_evaluation() {
        use std::{cell::Cell, rc::Rc};
        for tableau in [Tableau::rodas5p(), Tableau::rosenbrock23()] {
            let calls = Rc::new(Cell::new(0));
            let observed = calls.clone();
            let p = OdeBuilder::<Mat>::new()
                .rtol(1e-5)
                .atol([1e-7])
                .rhs_implicit(
                    move |x, _, _, f| {
                        observed.set(observed.get() + 1);
                        f[0] = -x[0];
                    },
                    |_, _, _, v, jv| jv[0] = -v[0],
                )
                .init(|_, _, y| y[0] = 1.0, 1)
                .build()
                .unwrap();
            let stages = tableau.s();
            let mut s = p
                .rosenbrock_solver::<LS, Mat>(p.rodas5p_state::<LS>().unwrap(), tableau)
                .unwrap();
            *s.state_mut().h = 0.001;
            calls.set(0); // Exclude constructor/initial-timestep work.
            s.step().unwrap();
            let attempts = 1 + s.get_statistics().number_of_error_test_failures;
            // Each stage plus two probes from the shared central f_t implementation.
            assert_eq!(calls.get(), attempts * (stages + 3));
            s.interpolate_dy(s.state().t)
                .unwrap()
                .assert_eq_st(s.state().dy, 1e-12);
        }
    }
    #[test]
    fn rosenbrock23_sparse_robertson_interpolation() {
        macro_rules! check {
            ($mat:ty, $ls:ty) => {{
                let times: Vec<_> = (0..61).map(|i| 10f64.powf(-6.0 + i as f64 / 6.0)).collect();
                let (mut reference_problem, _) = robertson_ode::<$mat>(false, 1);
                reference_problem.rtol = 1e-11;
                reference_problem.atol.fill(1e-14);
                let reference = reference_problem
                    .bdf::<$ls>()
                    .unwrap()
                    .solve_dense(&times)
                    .unwrap()
                    .0;
                let (p, _) = robertson_ode::<$mat>(false, 1);
                let mut s = p.rosenbrock23::<$ls>().unwrap();
                let mut max_error: f64 = 0.0;
                for (i, &t) in times.iter().enumerate() {
                    // Never stop at an observation: exercise interior samples in large stiff steps.
                    while s.state().t < t {
                        s.step().unwrap();
                    }
                    let expected = reference.column(i).into_owned();
                    let mut error = s.interpolate(t).unwrap();
                    error.axpy(-1.0, &expected, 1.0);
                    let scaled = error.squared_norm(&expected, &p.atol, p.rtol).sqrt();
                    max_error = max_error.max(scaled);
                }
                assert!(
                    max_error < 15.0,
                    "sparse Robertson error: {max_error} tolerance units"
                );
            }};
        }
        check!(Mat, LS);
        check!(FaerMat<f64>, FaerLU<f64>);
    }
    #[test]
    fn rosenbrock23_controls_nonlinear_algebraic_endpoint() {
        macro_rules! check {
            ($mat:ty, $ls:ty) => {{
                // y' = z^2, 0 = z^2 - 2 - sin(t). Both states have analytic solutions.
                let p = OdeBuilder::<$mat>::new()
                    .rtol(1e-5)
                    .atol([1e-7])
                    .rhs_implicit(
                        |x, _, t, f| {
                            f[0] = x[1] * x[1];
                            f[1] = x[1] * x[1] - 2.0 - t.sin();
                        },
                        |x, _, _, v, f| {
                            f[0] = 2.0 * x[1] * v[1];
                            f[1] = f[0];
                        },
                    )
                    .mass(|v, _, _, beta, y| {
                        y[0] = v[0] + beta * y[0];
                        y[1] *= beta;
                    })
                    .init(
                        |_, _, y| {
                            y[0] = 0.0;
                            y[1] = 2.0f64.sqrt();
                        },
                        2,
                    )
                    .build()
                    .unwrap();
                let mut s = p.rosenbrock23::<$ls>().unwrap();
                s.set_stop_time(2.0).unwrap();
                while s.state().t < 2.0 {
                    s.step().unwrap();
                    let t = s.state().t;
                    let y = s.state().y;
                    let residual = (y[1] * y[1] - 2.0 - t.sin()).abs();
                    assert!(residual < 2e-7, "constraint residual {residual} at {t}");
                    assert!(s.get_statistics().number_of_steps < 100_000);
                }
                assert!((s.state().y[0] - (5.0 - 2.0f64.cos())).abs() < 2e-5);
                assert!((s.state().y[1] - (2.0 + 2.0f64.sin()).sqrt()).abs() < 2e-7);
            }};
        }
        check!(Mat, LS);
        check!(FaerMat<f64>, FaerLU<f64>);
    }
    #[test]
    fn rosenbrock23_controls_rotated_algebraic_endpoint() {
        macro_rules! check {
            ($mat:ty, $ls:ty) => {{
                let c = std::f64::consts::FRAC_1_SQRT_2;
                // Rotate both equations and coordinates of y'=z², 0=z²-2-sin(t).
                // Every mass row/column is nonzero, but the matrix has rank one.
                let p = OdeBuilder::<$mat>::new()
                    .rtol(1e-4)
                    .atol([1e-7])
                    .rhs_implicit(
                        move |u, _, t, f| {
                            let z = c * (u[1] - u[0]);
                            let differential = z * z;
                            let algebraic = differential - 2.0 - t.sin();
                            f[0] = c * (differential - algebraic);
                            f[1] = c * (differential + algebraic);
                        },
                        move |u, _, _, v, f| {
                            let dz2 = 2.0 * c * (u[1] - u[0]) * c * (v[1] - v[0]);
                            f[0] = 0.0;
                            f[1] = 2.0 * c * dz2;
                        },
                    )
                    .mass(|v, _, _, beta, y| {
                        let d = 0.5 * (v[0] + v[1]);
                        y[0] = d + beta * y[0];
                        y[1] = d + beta * y[1];
                    })
                    .init(
                        |_, _, y| {
                            y[0] = -1.0;
                            y[1] = 1.0;
                        },
                        2,
                    )
                    .build()
                    .unwrap();
                let mut solver = p.rosenbrock23::<$ls>().unwrap();
                solver.set_stop_time(2.0).unwrap();
                while solver.state().t < 2.0 {
                    solver.step().unwrap();
                    let u = solver.state().y;
                    let z = c * (u[1] - u[0]);
                    let residual = (z * z - 2.0 - solver.state().t.sin()).abs();
                    assert!(residual < 2e-7, "rotated constraint residual {residual}");
                    if solver.get_statistics().number_of_steps == 1 {
                        let t = solver.state().t / 2.0;
                        let raw = solver.interpolate(t).unwrap();
                        let projected = solver.interpolate_consistent(t, 8).unwrap();
                        assert!(
                            (c * (raw[0] + raw[1] - projected[0] - projected[1])).abs() < 1e-14
                        );
                        let z = c * (projected[1] - projected[0]);
                        assert!((z * z - 2.0 - t.sin()).abs() < 2e-9);
                    }
                    assert!(solver.get_statistics().number_of_steps < 100_000);
                }
                let u = solver.state().y;
                assert!((c * (u[0] + u[1]) - (5.0 - 2.0f64.cos())).abs() < 2e-5);
            }};
        }
        check!(Mat, LS);
        check!(FaerMat<f64>, FaerLU<f64>);
    }
    #[test]
    fn small_increments_accumulate_without_roundoff_drift() {
        macro_rules! check {
            ($mat:ty, $ls:ty) => {{
                for tableau in [Tableau::rodas5p(), Tableau::rosenbrock23()] {
                    let p = OdeBuilder::<$mat>::new()
                        .rtol(1e-3)
                        .atol([1.0])
                        .rhs_implicit(|_, _, _, f| f[0] = 1.0, |_, _, _, _, f| f[0] = 0.0)
                        .init(|_, _, y| y[0] = 1e8, 1)
                        .build()
                        .unwrap();
                    let mut s = p
                        .rosenbrock_solver::<$ls, $mat>(p.rodas5p_state::<$ls>().unwrap(), tableau)
                        .unwrap();
                    s.set_maximum_step(1e-8).unwrap();
                    *s.state_mut().h = 1e-8;
                    s.config_mut().minimum_timestep = 0.0;
                    s.set_stop_time(1e-4).unwrap();
                    while s.step().unwrap() != OdeSolverStopReason::TstopReached {}
                    let error = (s.state().y[0] - 1e8 - 1e-4).abs();
                    assert!(error < 2e-8, "accumulation error {error}");
                }
            }};
        }
        check!(Mat, LS);
        check!(FaerMat<f64>, FaerLU<f64>);
    }
    #[test]
    fn consistent_interpolation_preserves_differential_state() {
        macro_rules! check {
            ($mat:ty,$ls:ty) => {{
                let p = OdeBuilder::<$mat>::new()
                    .rtol(1e-2)
                    .atol([1e-7])
                    .rhs_implicit(
                        |y, _, _, f| {
                            f[0] = y[1];
                            f[1] = y[0] * y[0] + y[1] * y[1] - 1.0;
                        },
                        |y, _, _, v, f| {
                            f[0] = 0.0;
                            f[1] = 2.0 * y[1] * v[1];
                        },
                    )
                    .mass(|v, _, _, beta, y| {
                        y[0] = v[0] + beta * y[0];
                        y[1] *= beta;
                    })
                    .init(
                        |_, _, y| {
                            y[0] = 0.3;
                            y[1] = 0.91f64.sqrt();
                        },
                        2,
                    )
                    .build()
                    .unwrap();
                let mut solver = p.rodas5p::<$ls>().unwrap();
                solver.set_maximum_step(0.05).unwrap();
                *solver.state_mut().h = 0.05;
                solver.step().unwrap();
                let t = solver.state().t / 2.0;
                let raw = solver.interpolate(t).unwrap();
                let before = solver.state().y.clone();
                let steps = solver.get_statistics().number_of_steps;
                assert!((raw[0] * raw[0] + raw[1] * raw[1] - 1.0).abs() > 1e-7);
                let corrected = solver.interpolate_consistent(t, 8).unwrap();
                assert_eq!(raw[0], corrected[0]);
                assert!(
                    (corrected[0] * corrected[0] + corrected[1] * corrected[1] - 1.0).abs() < 1e-9
                );
                solver.state().y.assert_eq_st(&before, 0.0);
                assert_eq!(steps, solver.get_statistics().number_of_steps);
                assert!(solver.interpolate_consistent(t, 0).is_err());
                assert!(solver.interpolate_consistent(f64::NAN, 8).is_err());
            }};
        }
        check!(Mat, LS);
        check!(FaerMat<f64>, FaerLU<f64>);
    }
    #[test]
    fn rosenbrock_rejects_steps_without_representable_progress() {
        for tableau in [Tableau::rodas5p(), Tableau::rosenbrock23()] {
            let p = OdeBuilder::<Mat>::new()
                .rhs_implicit(|_, _, _, f| f[0] = 0.0, |_, _, _, _, f| f[0] = 0.0)
                .init(|_, _, y| y[0] = 0.0, 1)
                .build()
                .unwrap();
            let mut solver = p
                .rosenbrock_solver::<LS, Mat>(p.rodas5p_state::<LS>().unwrap(), tableau)
                .unwrap();
            {
                let state = solver.state_mut();
                *state.t = 1.0;
                *state.h = 1e-20;
            }
            solver.config_mut().minimum_timestep = 0.0;
            assert!(matches!(
                solver.step(),
                Err(DiffsolError::OdeSolverError(
                    OdeSolverError::StepSizeTooSmall { .. }
                ))
            ));
            assert_eq!(solver.get_statistics().number_of_steps, 0);
        }
    }
    #[test]
    fn constant_mass_index_one_dae() {
        macro_rules! check {
            ($mat:ty, $ls:ty) => {{
                let (p, sol) = exponential_decay_with_algebraic_problem::<$mat>(false);
                test_ode_solver(&mut p.rodas5p::<$ls>().unwrap(), sol, None, false, false);
                let (p, sol) = exponential_decay_with_algebraic_adjoint_problem::<$mat>(true);
                let mut s = p.rodas5p::<$ls>().unwrap();
                // The shared state harness applies out(y); this fixture instead expects integral(out).
                for point in sol.solution_points {
                    while s.state().t < point.t { s.step().unwrap(); }
                    s.interpolate_out(point.t).unwrap().assert_eq_st(&point.state, 2e-5);
                }
            }};
        }
        check!(Mat, LS);
        check!(FaerMat<f64>, FaerLU<f64>);
    }
    fn sine_problem(
        lambda: f64,
        integrate: bool,
    ) -> OdeSolverProblem<
        impl OdeEquationsImplicit<
            M = Mat,
            V = crate::NalgebraVec<f64>,
            T = f64,
            C = crate::NalgebraContext,
        >,
    > {
        OdeBuilder::<Mat>::new()
            .rtol(10.0)
            .atol([10.0])
            .rhs_implicit(
                move |x, _, t, f| f[0] = lambda * (x[0] - t.sin()) + t.cos(),
                move |_, _, _, v, jv| jv[0] = lambda * v[0],
            )
            .init(|_, _, y| y[0] = 0.0, 1)
            .integrate_out(integrate)
            .out_implicit(
                |x, _, t, g| g[0] = x[0] * x[0] + t.powi(4),
                |_, _, _, _, _| unreachable!("quadrature does not use output Jacobians"),
                1,
            )
            .build()
            .unwrap()
    }
    fn fixed_error(h: f64, lambda: f64) -> f64 {
        let p = sine_problem(lambda, false);
        let mut s = p.rodas5p::<LS>().unwrap();
        *s.state_mut().h = h;
        s.config_mut().maximum_timestep_growth = 1.0;
        s.config_mut().minimum_timestep_growth = 1.0;
        advance(&mut s, 1.0);
        (s.state().y[0] - 1.0f64.sin()).abs()
    }
    #[test]
    fn fifth_order_and_stiff_order_reduction() {
        let coarse = fixed_error(0.2, -2.0);
        let fine = fixed_error(0.1, -2.0);
        assert!(coarse > 20.0 * fine, "{coarse} {fine}");
        assert!(fixed_error(0.2, -1000.0) > fixed_error(0.1, -1000.0));
    }
    #[test]
    fn continuous_extension_midpoint_convergence() {
        let error = |h| {
            let p = sine_problem(0.0, false);
            let mut s = p.rodas5p::<LS>().unwrap();
            *s.state_mut().h = h;
            s.step().unwrap();
            (s.interpolate(h / 2.0).unwrap()[0] - (h / 2.0).sin()).abs()
        };
        assert!(error(0.4) > 12.0 * error(0.2));
    }
    #[test]
    fn embedded_difference_has_local_order_five() {
        let error = |h| {
            let p = sine_problem(-2.0, false);
            let mut s = p.rodas5p::<LS>().unwrap();
            *s.state_mut().h = h;
            s.step().unwrap();
            s.rk.error_norm(h, None::<&mut NoAug<_>>, |_| Ok(()))
                .unwrap()
                .sqrt()
        };
        let ratio = error(0.2) / error(0.1);
        assert!((20.0..45.0).contains(&ratio), "{ratio}");
    }
    #[test]
    fn nonlinear_jacobian_refreshes_at_the_accepted_state() {
        use std::{cell::RefCell, rc::Rc};
        let points = Rc::new(RefCell::new(Vec::new()));
        let observed = points.clone();
        let p = OdeBuilder::<Mat>::new()
            .rtol(1e-7)
            .atol([1e-9])
            .rhs_implicit(
                |x, _, _, f| f[0] = -x[0] * x[0],
                move |x, _, t, v, jv| {
                    observed.borrow_mut().push((t, x[0]));
                    jv[0] = -2.0 * x[0] * v[0];
                },
            )
            .init(|_, _, y| y[0] = 2.0, 1)
            .build()
            .unwrap();
        let mut s = p.rodas5p::<LS>().unwrap();
        s.step().unwrap();
        let (t, y) = (s.state().t, s.state().y[0]);
        s.step().unwrap();
        assert!(points
            .borrow()
            .iter()
            .any(|&(at, x)| at == t && (x - y).abs() < 1e-12));
    }
    #[test]
    fn rejection_preserves_the_initial_solution() {
        let p = OdeBuilder::<Mat>::new()
            .rtol(1e-8)
            .atol([1e-10])
            .rhs_implicit(
                |x, _, _, f| f[0] = -100.0 * x[0],
                |_, _, _, v, jv| jv[0] = -100.0 * v[0],
            )
            .init(|_, _, y| y[0] = 1.0, 1)
            .build()
            .unwrap();
        let mut s = p.rodas5p::<LS>().unwrap();
        *s.state_mut().h = 0.1;
        s.step().unwrap();
        assert!(s.get_statistics().number_of_error_test_failures > 0);
        assert!(s.state().t < 0.1);
        assert!((s.state().y[0] - (-100.0 * s.state().t).exp()).abs() < 2e-5);
        advance(&mut s, 0.1);
        assert!((s.state().y[0] - (-10.0f64).exp()).abs() < 2e-5);
    }
    fn euler(gamma: f64, diagonal: f64, coupling: f64) -> Tableau<f64> {
        Tableau::new_rosenbrock(
            TableauMat::from_slice(1, 1, &[diagonal]),
            TableauVec::from_slice(&[1.0]),
            TableauVec::from_slice(&[0.0]),
            TableauVec::from_slice(&[0.0]),
            1,
            None,
            TableauMat::from_slice(1, 1, &[coupling]),
            gamma,
            TableauVec::from_slice(&[0.0]),
            1,
        )
    }
    #[test]
    fn custom_tableau_and_single_stage_hermite_interpolation() {
        let (p, _) = exponential_decay_problem::<Mat>(false);
        let mut state = p.rodas5p_state::<LS>().unwrap();
        state.h = 0.1;
        let mut s = p
            .rosenbrock_solver::<LS, Mat>(state, euler(1.0, 0.0, 0.0))
            .unwrap();
        let y0 = s.state().y[0];
        let dy0 = s.state().dy[0];
        s.step().unwrap();
        assert!((s.state().y[0] - y0 / 1.01).abs() < 1e-12);
        assert!((s.interpolate_dy(0.0).unwrap()[0] - dy0).abs() < 1e-12);
        assert!((s.interpolate_dy(0.1).unwrap()[0] - s.state().dy[0]).abs() < 1e-12);
    }
    #[test]
    fn invalid_tableaus_are_typed_errors() {
        let (p, _) = exponential_decay_problem::<Mat>(false);
        for tableau in [
            euler(0.0, 0.0, 0.0),
            euler(-1.0, 0.0, 0.0),
            euler(f64::NAN, 0.0, 0.0),
            euler(1.0, 1.0, 0.0),
            euler(1.0, 0.0, 1.0),
            Tableau::esdirk34(),
        ] {
            let error = p
                .rosenbrock_solver::<LS, Mat>(p.rodas5p_state::<LS>().unwrap(), tableau)
                .err()
                .unwrap();
            assert!(matches!(
                error,
                DiffsolError::OdeSolverError(OdeSolverError::InvalidTableau(_))
            ));
        }
        let (p, _) = exponential_decay_with_algebraic_problem::<Mat>(false);
        assert!(p
            .rosenbrock_solver::<LS, Mat>(p.rodas5p_state::<LS>().unwrap(), euler(1.0, 0.0, 0.0))
            .is_err());
    }
    #[test]
    fn constant_integrated_output_is_consistent_and_dense() {
        macro_rules! check {
            ($mat:ty, $ls:ty) => {{
                let p = test_problem::<$mat>(true);
                let mut s = p.rodas5p::<$ls>().unwrap();
                *s.state_mut().h = 0.1;
                s.step().unwrap();
                assert!((s.state().g[0] - 0.1 * s.state().y[0]).abs() < 1e-12);
                assert!(
                    (s.interpolate_out(0.05).unwrap()[0] - 0.05 * s.state().y[0]).abs() < 1e-12
                );
            }};
        }
        check!(Mat, LS);
        check!(FaerMat<f64>, FaerLU<f64>);
    }
    #[test]
    fn nonlinear_integrated_output_converges_and_restarts() {
        let error = |h| {
            let p = sine_problem(-2.0, true);
            let mut s = p.rodas5p::<LS>().unwrap();
            *s.state_mut().h = h;
            s.config_mut().minimum_timestep_growth = 1.0;
            s.config_mut().maximum_timestep_growth = 1.0;
            advance(&mut s, 1.0);

            let state = s.checkpoint();
            let mut restarted = p.rodas5p_solver::<LS>(state).unwrap();
            *restarted.config_mut() = s.config().clone();
            advance(&mut s, 2.0);
            advance(&mut restarted, 2.0);
            assert!((s.state().g[0] - restarted.state().g[0]).abs() < 1e-12);
            (s.interpolate_out(s.state().t).unwrap()[0] - (1.0 - (4.0f64).sin() / 4.0 + 32.0 / 5.0))
                .abs()
        };
        let coarse = error(0.2);
        let fine = error(0.1);
        assert!(coarse > 20.0 * fine, "{coarse} {fine}");
    }
    #[test]
    fn integrated_output_has_its_own_error_control() {
        let p = OdeBuilder::<Mat>::new()
            .rtol(1e-6)
            .atol([1e-8])
            .integrate_out(true)
            .out_rtol(1e-8)
            .out_atol([1e-10])
            .rhs_implicit(|_, _, _, f| f[0] = 0.0, |_, _, _, _, jv| jv[0] = 0.0)
            .init(|_, _, y| y[0] = 1.0, 1)
            .out_implicit(
                |_, _, t, g| g[0] = t.exp(),
                |_, _, _, _, _| unreachable!("quadrature does not use output Jacobians"),
                1,
            )
            .build()
            .unwrap();
        let mut s = p.rodas5p::<LS>().unwrap();
        *s.state_mut().h = 1.0;
        advance(&mut s, 1.0);
        assert!(s.get_statistics().number_of_error_test_failures > 0);
        assert!((s.state().g[0] - (1.0f64.exp() - 1.0)).abs() < 1e-8);
    }

    #[test]
    fn paper_prothero_robinson_problem_two() {
        const LAMBDA: f64 = 1e5;
        let g = |t: f64| 10.0 - (10.0 + t) * (-t).exp();
        let problem = OdeBuilder::<Mat>::new()
            .rtol(1e-8)
            .atol([1e-10])
            .rhs_implicit(
                |x, _, t, f| {
                    let g = 10.0 - (10.0 + t) * (-t).exp();
                    let dg = (9.0 + t) * (-t).exp();
                    f[0] = -LAMBDA * (x[0] - g) + dg;
                },
                |_, _, _, v, jv| jv[0] = -LAMBDA * v[0],
            )
            .init(|_, _, y| y[0] = 0.0, 1)
            .build()
            .unwrap();
        let mut solver = problem.rodas5p::<LS>().unwrap();
        advance(&mut solver, 2.0);
        let error = (solver.state().y[0] - g(2.0)).abs();
        assert!(error < 1e-7, "paper problem 2 error={error}");
    }
    #[test]
    fn paper_table_six_fixed_step_errors_and_order() {
        let published_errors = [1.26e-9, 1.47e-10, 1.78e-11, 2.17e-12];
        let mut errors = Vec::new();
        for (h, published) in [0.25, 0.125, 0.0625, 0.03125]
            .into_iter()
            .zip(published_errors)
        {
            let problem = OdeBuilder::<Mat>::new()
                .rtol(10.0)
                .atol([10.0])
                .rhs_implicit(
                    |x, _, t, f| {
                        let g = 10.0 - (10.0 + t) * (-t).exp();
                        let dg = (9.0 + t) * (-t).exp();
                        f[0] = -1e5 * (x[0] - g) + dg;
                    },
                    |_, _, _, v, jv| jv[0] = -1e5 * v[0],
                )
                .init(|_, _, y| y[0] = 0.0, 1)
                .build()
                .unwrap();
            let mut solver = problem.rodas5p::<LS>().unwrap();
            *solver.state_mut().h = h;
            solver.config_mut().minimum_timestep_growth = 1.0;
            solver.config_mut().maximum_timestep_growth = 1.0;
            advance(&mut solver, 2.0);
            let exact = 10.0 - 12.0 * (-2.0_f64).exp();
            let error = (solver.state().y[0] - exact).abs();
            assert!(
                (error - published).abs() < 0.2 * published,
                "h={h}, error={error}, published={published}"
            );
            errors.push(error);
        }
        for pair in errors.windows(2) {
            let order = (pair[0] / pair[1]).log2();
            assert!((order - 3.0).abs() < 0.2, "observed order={order}");
        }
    }
    #[test]
    fn paper_table_eight_index_two_order_reduction() {
        let published_errors = [9.00e-5, 2.33e-5, 5.94e-6];
        let mut errors = Vec::new();
        for (h, published) in [0.03125, 0.015625, 0.0078125]
            .into_iter()
            .zip(published_errors)
        {
            let problem = OdeBuilder::<Mat>::new()
                .t0(1.0)
                .rtol(10.0)
                .atol([10.0, 10.0])
                .rhs_implicit(
                    |x, _, t, f| {
                        f[0] = x[1];
                        f[1] = x[0] * x[0] - 1.0 / (t * t);
                    },
                    |x, _, _, v, jv| {
                        jv[0] = v[1];
                        jv[1] = 2.0 * x[0] * v[0];
                    },
                )
                .mass(|v, _, _, beta, y| {
                    y[0] = v[0] + beta * y[0];
                    y[1] *= beta;
                })
                .init(
                    |_, _, y| {
                        y[0] = -1.0;
                        y[1] = 1.0;
                    },
                    2,
                )
                .build()
                .unwrap();
            let mut solver = problem
                .rodas5p_solver::<LS>(RkState::new(&problem, 5).unwrap())
                .unwrap();
            *solver.state_mut().h = h;
            solver.config_mut().minimum_timestep_growth = 1.0;
            solver.config_mut().maximum_timestep_growth = 1.0;
            advance(&mut solver, 2.0);
            let y = solver.state().y;
            let error = (y[0] + 0.5).abs().max((y[1] - 0.25).abs());
            assert!(
                (error - published).abs() < 0.2 * published,
                "h={h}, error={error}, published={published}"
            );
            errors.push(error);
        }
        for pair in errors.windows(2) {
            let order = (pair[0] / pair[1]).log2();
            assert!((order - 2.0).abs() < 0.2, "observed order={order}");
        }
    }
    #[test]
    fn paper_polynomial_dense_output_ode_component() {
        for degree in 1_i32..=4 {
            let problem = OdeBuilder::<Mat>::new()
                .rtol(10.0)
                .atol([10.0])
                .rhs_implicit(
                    move |_, _, t, f| f[0] = f64::from(degree) * t.powi(degree - 1),
                    |_, _, _, _, jv| jv[0] = 0.0,
                )
                .init(|_, _, y| y[0] = 0.0, 1)
                .build()
                .unwrap();
            let mut solver = problem.rodas5p::<LS>().unwrap();
            *solver.state_mut().h = 2.0;
            solver.step().unwrap();
            for t in [0.25_f64, 0.5, 1.0, 1.5, 1.75] {
                let error = (solver.interpolate(t).unwrap()[0] - t.powi(degree)).abs();
                assert!(error < 1e-9, "degree={degree}, t={t}, error={error}");
            }
        }
    }
    #[test]
    fn paper_index_one_dae_problem_one() {
        // Use rtol=1e-7 for the adaptive endpoint/constraint accuracy check at 1e-7.
        // The separate fixed-step Table 5 test verifies order; 1e-8 is not needed here.
        let problem = OdeBuilder::<Mat>::new()
            .t0(2.0)
            .rtol(1e-7)
            .atol([1e-10, 1e-10])
            .rhs_implicit(
                |x, _, t, f| {
                    f[0] = x[1] / x[0];
                    f[1] = x[0] / x[1] - t;
                },
                |x, _, _, v, jv| {
                    jv[0] = v[1] / x[0] - x[1] * v[0] / x[0].powi(2);
                    jv[1] = v[0] / x[1] - x[0] * v[1] / x[1].powi(2);
                },
            )
            .mass(|v, _, _, beta, y| {
                y[0] = v[0] + beta * y[0];
                y[1] *= beta;
            })
            .init(
                |_, _, y| {
                    y[0] = 2.0_f64.ln();
                    y[1] = 2.0_f64.ln() / 2.0;
                },
                2,
            )
            .build()
            .unwrap();
        let mut solver = problem.rodas5p::<LS>().unwrap();
        advance(&mut solver, 4.0);
        let y = solver.state().y;
        assert!((y[0] - 4.0_f64.ln()).abs() < 1e-7);
        assert!((y[1] - 4.0_f64.ln() / 4.0).abs() < 1e-7);
        assert!((y[0] / y[1] - 4.0).abs() < 1e-7);
    }
    #[test]
    fn paper_table_five_fixed_step_dae_errors_and_order() {
        let published_errors = [2.93e-8, 8.56e-10, 2.59e-11];
        let mut errors = Vec::new();
        for (h, published) in [0.125, 0.0625, 0.03125].into_iter().zip(published_errors) {
            let problem = OdeBuilder::<Mat>::new()
                .t0(2.0)
                .rtol(10.0)
                .atol([10.0, 10.0])
                .rhs_implicit(
                    |x, _, t, f| {
                        f[0] = x[1] / x[0];
                        f[1] = x[0] / x[1] - t;
                    },
                    |x, _, _, v, jv| {
                        jv[0] = v[1] / x[0] - x[1] * v[0] / x[0].powi(2);
                        jv[1] = v[0] / x[1] - x[0] * v[1] / x[1].powi(2);
                    },
                )
                .mass(|v, _, _, beta, y| {
                    y[0] = v[0] + beta * y[0];
                    y[1] *= beta;
                })
                .init(
                    |_, _, y| {
                        y[0] = 2.0_f64.ln();
                        y[1] = 2.0_f64.ln() / 2.0;
                    },
                    2,
                )
                .build()
                .unwrap();
            let mut solver = problem.rodas5p::<LS>().unwrap();
            *solver.state_mut().h = h;
            solver.config_mut().minimum_timestep_growth = 1.0;
            solver.config_mut().maximum_timestep_growth = 1.0;
            advance(&mut solver, 4.0);
            let y = solver.state().y;
            let error = (y[0] - 4.0_f64.ln())
                .abs()
                .max((y[1] - 4.0_f64.ln() / 4.0).abs());
            assert!(
                (error - published).abs() < 0.2 * published,
                "h={h}, error={error}, published={published}"
            );
            errors.push(error);
        }
        for pair in errors.windows(2) {
            let order = (pair[0] / pair[1]).log2();
            assert!((order - 5.0).abs() < 0.25, "observed order={order}");
        }
    }
    #[test]
    fn forcing_jump_uses_incoming_endpoint_and_outgoing_restart() {
        for tableau in [Tableau::rodas5p(), Tableau::rosenbrock23()] {
            let p = OdeBuilder::<Mat>::new()
                .rtol(1e-8)
                .atol([1e-10])
                .rhs_implicit(
                    |_, _, t, f| f[0] = if t < 1.0 { 1.0 } else { 100.0 },
                    |_, _, _, _, j| j[0] = 0.0,
                )
                .init(|_, _, y| y[0] = 0.0, 1)
                .build()
                .unwrap();
            let state = p.rodas5p_state::<LS>().unwrap();
            let mut s = p.rosenbrock_solver::<LS, Mat>(state, tableau).unwrap();
            s.set_time_derivative_within_step(true);
            s.set_discontinuity_stop_time(1.0).unwrap();
            while s.step().unwrap() != OdeSolverStopReason::TstopReached {}
            assert!((s.state().y[0] - 1.0).abs() < 1e-9);
            assert!((s.state().dy[0] - 1.0).abs() < 1e-12);
            s.set_stop_time(2.0).unwrap();
            while s.step().unwrap() != OdeSolverStopReason::TstopReached {}
            assert!((s.state().y[0] - 101.0).abs() < 1e-8);
            assert!((s.state().dy[0] - 100.0).abs() < 1e-12);
        }
    }

    #[test]
    fn quintic_dense_error_reduces_with_step_size() {
        let error = |h: f64| {
            let p = OdeBuilder::<Mat>::new()
                .rtol(10.0)
                .atol([10.0])
                .rhs_implicit(
                    |_, _, t, f| f[0] = 5.0 * t.powi(4),
                    |_, _, _, _, j| j[0] = 0.0,
                )
                .init(|_, _, y| y[0] = 0.0, 1)
                .build()
                .unwrap();
            let mut s = p.rodas5p::<LS>().unwrap();
            *s.state_mut().h = h;
            s.step().unwrap();
            (s.interpolate(h / 2.0).unwrap()[0] - (h / 2.0).powi(5)).abs()
        };
        assert!((30.0..34.0).contains(&(error(0.5) / error(0.25))));
    }

    #[test]
    fn maximum_step_bounds_dense_output_and_survives_clone() {
        let p = OdeBuilder::<Mat>::new()
            .rtol(10.0)
            .atol([10.0])
            .rhs_implicit(
                |_, _, t, f| f[0] = 5.0 * t.powi(4),
                |_, _, _, _, j| j[0] = 0.0,
            )
            .init(|_, _, y| y[0] = 0.0, 1)
            .build()
            .unwrap();
        let mut s = p.rodas5p::<LS>().unwrap();
        assert!(s.set_maximum_step(0.0).is_err());
        assert!(s.set_maximum_step(f64::NAN).is_err());
        s.set_maximum_step(0.25).unwrap();
        let mut s = s.clone();
        *s.state_mut().h = 2.0;
        s.step().unwrap();
        assert_eq!(s.state().t, 0.25);
        assert!((s.interpolate(0.125).unwrap()[0] - 0.125_f64.powi(5)).abs() < 1e-5);
    }

    #[test]
    fn nonfinite_trial_stages_shrink_and_retry() {
        for tableau in [Tableau::rodas5p(), Tableau::rosenbrock23()] {
            let p = OdeBuilder::<Mat>::new()
                .rtol(1e-6)
                .atol([1e-9])
                .rhs_implicit(
                    |x, _, _, f| f[0] = if x[0] < 0.0 { f64::NAN } else { -100.0 * x[0] },
                    |_, _, _, v, j| j[0] = -100.0 * v[0],
                )
                .init(|_, _, y| y[0] = 1.0, 1)
                .build()
                .unwrap();
            let state = p.rodas5p_state::<LS>().unwrap();
            let mut solver = p.rosenbrock_solver::<LS, Mat>(state, tableau).unwrap();
            *solver.state_mut().h = 1.0;
            solver.step().unwrap();
            assert!(solver.state().y[0].is_finite());
            assert!(solver.state().t < 1.0);
            assert!(solver.get_statistics().number_of_error_test_failures > 0);
        }
    }

    #[test]
    fn persistent_nonfinite_rhs_fails_without_accepting_state() {
        for tableau in [Tableau::rodas5p(), Tableau::rosenbrock23()] {
            let p = OdeBuilder::<Mat>::new()
                .rtol(1e-6)
                .atol([1e-9])
                .rhs_implicit(
                    |_, _, t, f| f[0] = if t > 0.0 { f64::NAN } else { 0.0 },
                    |_, _, _, _, j| j[0] = 0.0,
                )
                .init(|_, _, y| y[0] = 1.0, 1)
                .build()
                .unwrap();
            let state = p.rodas5p_state::<LS>().unwrap();
            let mut solver = p.rosenbrock_solver::<LS, Mat>(state, tableau).unwrap();
            solver.config_mut().maximum_error_test_failures = 5;
            assert!(solver.step().is_err());
            assert_eq!(solver.state().t, 0.0);
            assert_eq!(solver.state().y[0], 1.0);
        }
    }

    #[test]
    fn sampled_refinement_rejects_invalid_data_and_preserves_callback_errors() {
        let mut calls = 0;
        let result = refine_sampled_trajectory(
            1e-3,
            &[1e-5],
            8,
            0.25,
            |factor| {
                calls += 1;
                Ok::<_, DiffsolError>(vec![vec![1.0 + factor * 0.002]])
            },
            |e| e,
        )
        .unwrap();
        assert_eq!(calls, 3);
        assert_eq!(result.passes, 3);
        assert!(result.maximum_scaled_change <= 0.25);
        for rows in [vec![], vec![vec![f64::NAN]], vec![vec![f64::INFINITY]]] {
            assert!(refine_sampled_trajectory(
                1e-3,
                &[1e-5],
                8,
                0.25,
                |_| Ok::<_, DiffsolError>(rows.clone()),
                |e| e
            )
            .is_err());
        }
        let mut calls = 0;
        let result = refine_sampled_trajectory(
            1e-3,
            &[1e-5],
            8,
            0.25,
            |_| {
                calls += 1;
                if calls == 2 {
                    Err("original shared work budget".to_owned())
                } else {
                    Ok(vec![vec![1.0]])
                }
            },
            |e| e.to_string(),
        );
        assert_eq!(result.unwrap_err(), "original shared work budget");
        assert_eq!(calls, 2);
        let mut calls = 0;
        assert!(refine_sampled_trajectory(
            1e-3,
            &[1e-5],
            3,
            0.25,
            |_| {
                calls += 1;
                Ok::<_, DiffsolError>(vec![vec![if calls % 2 == 0 { 2.0 } else { 1.0 }]])
            },
            |e| e
        )
        .is_err());
        assert_eq!(calls, 3);
        let mut calls = 0;
        assert!(refine_sampled_trajectory(
            1e-3,
            &[1e-5],
            8,
            0.25,
            |_| {
                calls += 1;
                Ok::<_, DiffsolError>(vec![vec![1.0; calls]])
            },
            |e| e
        )
        .is_err());
    }

    #[test]
    fn refinement_reports_deterioration_and_stalling() {
        for (values, word) in [
            (vec![1.0, 1.01, 1.011, 1.0], "worsened"),
            (vec![1.0, 2.0, 1.0, 2.0, 1.0], "stalled"),
        ] {
            let mut calls = 0;
            let error = refine_sampled_trajectory(
                1e-3,
                &[1e-5],
                8,
                0.25,
                |_| {
                    let value = values[calls.min(values.len() - 1)];
                    calls += 1;
                    Ok::<_, DiffsolError>(vec![vec![value]])
                },
                |e| e,
            )
            .unwrap_err();
            assert!(error.to_string().contains(word), "{error}");
            assert!(calls <= 5);
        }
    }
    #[test]
    fn polynomial_ode_endpoint_derivative_uses_rhs() {
        let p = OdeBuilder::<Mat>::new()
            .rtol(10.0)
            .atol([10.0])
            .rhs_implicit(
                |_, _, t, f| f[0] = 5.0 * t.powi(4),
                |_, _, _, _, j| j[0] = 0.0,
            )
            .init(|_, _, y| y[0] = 0.0, 1)
            .build()
            .unwrap();
        let mut s = p.rodas5p::<LS>().unwrap();
        *s.state_mut().h = 2.0;
        s.step().unwrap();
        assert!(
            (s.state().dy[0] - 80.0).abs() < 1e-12,
            "{}",
            s.state().dy[0]
        );
    }

    #[test]
    fn central_time_partial_resolves_small_time_signal() {
        use crate::{NonLinearOp, OdeEquations};
        let p = OdeBuilder::<Mat>::new()
            .rhs_implicit(
                |_, _, t, f| f[0] = 1e4 + t.sin(),
                |_, _, _, _, j| j[0] = 0.0,
            )
            .init(|_, _, y| y[0] = 0.0, 1)
            .build()
            .unwrap();
        let mut ft = p
            .eqn
            .rhs()
            .call(&crate::NalgebraVec::zeros(1, *p.context()), 1.0);
        p.eqn
            .rhs()
            .time_derive_inplace(&crate::NalgebraVec::zeros(1, *p.context()), 1.0, &mut ft);
        assert!((ft[0] - 1.0_f64.cos()).abs() < 5e-7, "{}", ft[0]);
    }

    #[test]
    fn paper_polynomial_dense_output_dae_problem_six() {
        for degree in 1_i32..=5 {
            let problem = OdeBuilder::<Mat>::new()
                .rtol(10.0)
                .atol([10.0, 10.0])
                .rhs_implicit(
                    move |x, _, t, f| {
                        f[0] = f64::from(degree) * t.powi(degree - 1);
                        f[1] = x[0] - x[1];
                    },
                    |_, _, _, v, jv| {
                        jv[0] = 0.0;
                        jv[1] = v[0] - v[1];
                    },
                )
                .mass(|v, _, _, beta, y| {
                    y[0] = v[0] + beta * y[0];
                    y[1] *= beta;
                })
                .init(
                    |_, _, y| {
                        y[0] = 0.0;
                        y[1] = 0.0;
                    },
                    2,
                )
                .build()
                .unwrap();
            let mut solver = problem.rodas5p::<LS>().unwrap();
            *solver.state_mut().h = 2.0;
            solver.step().unwrap();
            assert!((solver.state().y[0] - 2.0_f64.powi(degree)).abs() < 1e-9);
            assert!((solver.state().y[1] - 2.0_f64.powi(degree)).abs() < 1e-9);
            let exact_derivative = f64::from(degree) * 2.0_f64.powi(degree - 1);
            if degree <= 4 {
                assert!((solver.state().dy[0] - exact_derivative).abs() < 1e-9);
                assert!((solver.state().dy[1] - exact_derivative).abs() < 1e-9);
            }
            for t in [0.25_f64, 0.5, 1.0, 1.5, 1.75] {
                let y = solver.interpolate(t).unwrap();
                let exact = t.powi(degree);
                for component in 0..2 {
                    let error = (y[component] - exact).abs();
                    if degree <= 4 {
                        assert!(
                            error < 1e-10,
                            "degree={degree}, t={t}, component={component}, error={error}"
                        );
                    } else if t == 1.0 {
                        // Table 9: fifth-order endpoint accuracy but only
                        // fourth-order continuous interpolation.
                        assert!(
                            (error - 0.312).abs() < 0.02,
                            "degree={degree}, component={component}, error={error}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn nonautonomous_constant_mass_dae() {
        // y0 = sin(t), y1 = y0; f_t acts on the differential equation.
        let p = OdeBuilder::<Mat>::new()
            .rtol(1e-8)
            .atol([1e-10, 1e-10])
            .rhs_implicit(
                |x, _, t, f| {
                    f[0] = -x[0] + t.sin() + t.cos();
                    f[1] = x[0] - x[1];
                },
                |_, _, _, v, jv| {
                    jv[0] = -v[0];
                    jv[1] = v[0] - v[1];
                },
            )
            .mass(|v, _, _, beta, y| {
                y[0] = v[0] + beta * y[0];
                y[1] *= beta;
            })
            .init(
                |_, _, y| {
                    y[0] = 0.0;
                    y[1] = 0.0;
                },
                2,
            )
            .build()
            .unwrap();
        let mut s = p.rodas5p::<LS>().unwrap();
        advance(&mut s, 1.0);
        for i in 0..2 {
            assert!((s.state().y[i] - 1.0f64.sin()).abs() < 1e-7);
        }
    }
    #[test]
    fn integrated_output_requires_continuous_extension() {
        let p = sine_problem(0.0, true);
        let err = p
            .rosenbrock_solver::<LS, Mat>(p.rodas5p_state::<LS>().unwrap(), euler(1.0, 0.0, 0.0))
            .err()
            .unwrap();
        assert!(matches!(
            err,
            DiffsolError::OdeSolverError(OdeSolverError::InvalidTableau(_))
        ));
    }
    fn stability_step(z: f64) -> f64 {
        let p = OdeBuilder::<Mat>::new()
            .rtol(1e30)
            .atol([1e30])
            .rhs_implicit(
                move |x, _, _, f| f[0] = z * x[0],
                move |_, _, _, v, jv| jv[0] = z * v[0],
            )
            .init(|_, _, y| y[0] = 1.0, 1)
            .build()
            .unwrap();
        let mut s = p.rodas5p::<LS>().unwrap();
        *s.state_mut().h = 1.0;
        s.step().unwrap();
        s.state().y[0]
    }
    #[test]
    fn rodas5p_transformed_stiff_accuracy_and_l_stability() {
        let t = Tableau::<f64>::rodas5p();
        // In transformed form the final correction u8 completes the last stage state:
        // b = A[7,:] + e8, not b = A[7,:] (the latter has a zero diagonal).
        for i in 0..8 {
            assert!((t.b()[i] - t.a(7, i) - if i == 7 { 1.0 } else { 0.0 }).abs() < 1e-15);
        }
        assert!(stability_step(-1e8).abs() < 1e-6);
        assert!(stability_step(-1e14).abs() < 1e-10);
    }

    #[test]
    fn rosenbrock23_shared_contract() {
        macro_rules! check {
            ($mat:ty,$ls:ty) => {{
                test_state_mut(test_problem::<$mat>(false).rosenbrock23::<$ls>().unwrap());
                test_interpolate(test_problem::<$mat>(false).rosenbrock23::<$ls>().unwrap());
                test_interpolate(test_problem::<$mat>(true).rosenbrock23::<$ls>().unwrap());
                test_interpolate_dy(test_problem::<$mat>(false).rosenbrock23::<$ls>().unwrap());
                test_config(
                    robertson_ode::<$mat>(false, 1)
                        .0
                        .rosenbrock23::<$ls>()
                        .unwrap(),
                );
                let (p, sol) = exponential_decay_problem::<$mat>(false);
                test_checkpointing(
                    sol,
                    p.rosenbrock23::<$ls>().unwrap(),
                    p.rosenbrock23::<$ls>().unwrap(),
                );
                for use_tstop in [false, true] {
                    let (p, sol) = exponential_decay_problem::<$mat>(false);
                    test_ode_solver(
                        &mut p.rosenbrock23::<$ls>().unwrap(),
                        sol,
                        None,
                        use_tstop,
                        false,
                    );
                    let (p, sol) = exponential_decay_problem_with_root::<$mat>(false, false);
                    test_ode_solver(
                        &mut p.rosenbrock23::<$ls>().unwrap(),
                        sol,
                        None,
                        use_tstop,
                        false,
                    );
                }
                let (p, sol) = robertson_ode::<$mat>(false, 1);
                let mut s = p.rosenbrock23::<$ls>().unwrap();
                test_ode_solver(&mut s, sol, None, false, false);
                assert_eq!(
                    s.get_statistics().number_of_linear_solver_setups,
                    s.get_statistics().number_of_steps
                        + s.get_statistics().number_of_error_test_failures
                );
                assert_eq!(s.get_statistics().number_of_nonlinear_solver_iterations, 0);
            }};
        }
        check!(Mat, LS);
        check!(FaerMat<f64>, FaerLU<f64>);
    }
    // Mirror one attempt in Rosenbrock::step(): freeze/factor the Jacobian, form stages,
    // finish the endpoint, estimate error, and accept the fixed step. Keep these calls
    // in sync with step(); only f_t is supplied analytically to isolate stage arithmetic.
    // This scalar, non-integrated test helper omits retries, adaptive control, statistics
    // and output quadrature; production uses NonLinearOpTimePartial unchanged.
    fn analytic_step<
        E: OdeEquationsImplicit<
            T = f64,
            M = Mat,
            V = crate::NalgebraVec<f64>,
            C = crate::NalgebraContext,
        >,
    >(
        s: &mut Rosenbrock<'_, E, LS>,
        h: f64,
        ft: f64,
    ) -> f64 {
        let _ = s.rk.start_step().unwrap();
        s.op.zero_phi();
        s.op.set_h(s.rk.tableau().rosenbrock().unwrap().gamma * h);
        s.op.set_jacobian_is_stale();
        LinearSolver::set_linearisation(
            &mut s.linear_solver,
            &s.op,
            &s.rk.state().y,
            s.rk.state().t,
        );
        s.ft[0] = ft;
        s.rk.start_step_attempt(h, None::<&mut NoAug<E>>);
        for i in 0..s.rk.tableau().s() {
            s.rk.do_stage_rosenbrock(i, h, &s.op, &mut s.linear_solver, &s.ft, s.rk.state().t + h)
                .unwrap();
        }
        s.rk.finish_step_rosenbrock(h, s.rk.state().t + h);
        let err =
            s.rk.error_norm(h, None::<&mut NoAug<E>>, |_| Ok(()))
                .unwrap()
                .sqrt()
                * (s.problem().atol[0] + s.problem().rtol * s.state().y[0].abs());
        s.rk.store_rosenbrock_hermite_derivatives(h);
        s.rk.step_accepted(h, h, false).unwrap();
        err
    }
    // Direct implementation 2a255e1, with analytic f_t to isolate stage arithmetic.
    // Its within-step finite difference belongs to the downstream event policy.
    #[test]
    fn rosenbrock23_matches_direct_stage_oracles() {
        let p = OdeBuilder::<Mat>::new()
            .rtol(10.0)
            .atol([10.0])
            .rhs_implicit(
                |x, _, _, f| f[0] = -x[0] * x[0],
                |x, _, _, v, jv| jv[0] = -2.0 * x[0] * v[0],
            )
            .init(|_, _, y| y[0] = 2.0, 1)
            .build()
            .unwrap();
        let mut s = p.rosenbrock23::<LS>().unwrap();
        *s.state_mut().h = 0.01;
        s.step().unwrap();
        let estimate =
            s.rk.error_norm(0.01, None::<&mut NoAug<_>>, |_| Ok(()))
                .unwrap()
                .sqrt()
                * (10.0 + 10.0 * s.state().y[0].abs());
        assert!((s.state().y[0] - 1.9607830804516306).abs() < 1e-14);
        assert!((estimate - 1.2234237241393055e-6).abs() < 1e-14);
        let p = sine_problem(-1000.0, false);
        let mut s = p.rosenbrock23::<LS>().unwrap();
        let estimate = analytic_step(&mut s, 0.01, 1000.0);
        assert!(
            (s.state().y[0] - 0.009999915159438611).abs() < 1e-14,
            "actual={} estimate={estimate}",
            s.state().y[0]
        );
        assert!((estimate - 1.0579454445214243e-7).abs() < 1e-14);
    }
    #[test]
    fn rosenbrock23_second_order_and_quadratic_dense_output() {
        let error = |h: f64, midpoint: bool| {
            let p = OdeBuilder::<Mat>::new()
                .rtol(10.0)
                .atol([10.0])
                .rhs_implicit(|x, _, _, f| f[0] = -x[0], |_, _, _, v, jv| jv[0] = -v[0])
                .init(|_, _, y| y[0] = 1.0, 1)
                .build()
                .unwrap();
            let mut s = p.rosenbrock23::<LS>().unwrap();
            *s.state_mut().h = h;
            s.config_mut().minimum_timestep_growth = 1.0;
            s.config_mut().maximum_timestep_growth = 1.0;
            if midpoint {
                s.step().unwrap();
                (s.interpolate(h / 2.0).unwrap()[0] - (-h / 2.0).exp()).abs()
            } else {
                advance(&mut s, 1.0);
                (s.state().y[0] - (-1.0f64).exp()).abs()
            }
        };
        let ratio = error(0.05, false) / error(0.025, false);
        assert!((3.8..4.2).contains(&ratio), "{ratio}");
        let ratio = error(0.05, true) / error(0.025, true);
        assert!((7.0..9.0).contains(&ratio), "{ratio}");
    }
    #[test]
    fn rosenbrock23_time_forcing_error_estimate_is_cubic() {
        let estimate = |h: f64| {
            let p = sine_problem(0.0, false);
            let mut s = p.rosenbrock23::<LS>().unwrap();
            *s.state_mut().t = 1.0;
            *s.state_mut().h = h;
            s.step().unwrap();
            s.rk.error_norm(h, None::<&mut NoAug<_>>, |_| Ok(()))
                .unwrap()
                .sqrt()
        };
        let ratio = estimate(0.05) / estimate(0.025);
        assert!((6.0..10.0).contains(&ratio), "{ratio}");
    }
    #[test]
    fn rosenbrock23_integrated_outputs_converge_and_restart() {
        let error = |h: f64| {
            let p = sine_problem(-2.0, true);
            let mut s = p.rosenbrock23::<LS>().unwrap();
            *s.state_mut().h = h;
            s.config_mut().minimum_timestep_growth = 1.0;
            s.config_mut().maximum_timestep_growth = 1.0;
            advance(&mut s, 1.0);
            let mut restart = p.rosenbrock23_solver::<LS>(s.checkpoint()).unwrap();
            *restart.config_mut() = s.config().clone();
            advance(&mut s, 2.0);
            advance(&mut restart, 2.0);
            assert!((s.state().g[0] - restart.state().g[0]).abs() < 1e-12);
            (s.state().g[0] - (1.0 - (4.0f64).sin() / 4.0 + 32.0 / 5.0)).abs()
        };
        let order = (error(0.05) / error(0.025)).log2();
        assert!(order >= 2.0, "{order}");
    }

    #[test]
    fn rodas5p_coefficients_match_julia_bit_for_bit() {
        let data = include_str!(
            "../ode_equations/test_models/rosenbrock_reference/rodas5p-julia-2.7.1.txt"
        );
        let values: Vec<f64> = data.lines().filter_map(|l| l.parse().ok()).collect();
        assert_eq!(values.len(), 161);
        let t = Tableau::<f64>::rodas5p();
        let row = t.rosenbrock().unwrap();
        let mut observed = vec![row.gamma];
        for i in 0..8 {
            for j in 0..8 {
                observed.push(t.a(i, j));
            }
        }
        for i in 0..8 {
            for j in 0..7 {
                observed.push(row.coupling[(i, j)]);
            }
        }
        for i in 0..8 {
            assert_eq!(row.coupling[(i, 7)], 0.0);
        }
        observed.extend_from_slice(t.c().as_slice());
        observed.extend_from_slice(row.time.as_slice());
        // Compare the beta transformation exactly, without inverting rounded sums to recover H.
        for i in 0..8 {
            let h = [values[137 + i], values[145 + i], values[153 + i]];
            let beta = t.beta_t().unwrap().as_col_slice(i);
            for (actual, expected) in
                beta.iter()
                    .zip([t.b()[i] + h[0], -h[0] + h[1], -h[1] + h[2], -h[2]])
            {
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
        }
        for (actual, expected) in observed.iter().zip(values) {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
    }
    #[test]
    fn rodas5p_stability_matches_julia() {
        assert!((stability_step(-1.0) - (0.3678803089370538)).abs() < 1e-13);
        assert!((stability_step(-10.0) - (-0.04037298377966268)).abs() < 1e-13);
        assert!((stability_step(-1000.0) - (-0.01205291678818721)).abs() < 1e-13);
        assert!((stability_step(-1.0e8) - (-1.2520878608405691e-7)).abs() < 1e-13);
        assert!((stability_step(-1.0e14) - (-1.2520883384046958e-13)).abs() < 1e-13);
    }
    #[test]
    fn built_in_fixed_robertson_matches_julia() {
        for (tableau, expected) in [
            (
                Tableau::rodas5p(),
                [
                    0.9996006841629056,
                    3.6450479186134964e-5,
                    0.0003628653579085962,
                ],
            ),
            (
                Tableau::rosenbrock23(),
                [
                    0.9996006819320089,
                    3.645047866463391e-5,
                    0.0003628675893263426,
                ],
            ),
        ] {
            let (mut p, _) = robertson_ode::<Mat>(false, 1);
            p.rtol = 10.0;
            p.atol.fill(10.0);
            let mut s = p
                .rosenbrock_solver::<LS, Mat>(p.rodas5p_state::<LS>().unwrap(), tableau)
                .unwrap();
            *s.state_mut().h = 0.001;
            s.config_mut().minimum_timestep_growth = 1.0;
            s.config_mut().maximum_timestep_growth = 1.0;
            advance(&mut s, 0.01);
            for (i, e) in expected.into_iter().enumerate() {
                assert!(
                    (s.state().y[i] - e).abs() < 1e-12 * e.abs(),
                    "i={i}, actual={}, expected={e}",
                    s.state().y[i]
                );
            }
        }
    }

    #[test]
    fn rosenbrock23_rejected_attempt_keeps_initial_state_intact() {
        let problem = OdeBuilder::<Mat>::new()
            .rtol(1e-8)
            .atol([1e-10])
            .rhs_implicit(
                |x, _, _, f| f[0] = -100.0 * x[0],
                |_, _, _, v, jv| jv[0] = -100.0 * v[0],
            )
            .init(|_, _, y| y[0] = 1.0, 1)
            .build()
            .unwrap();
        let mut solver = problem.rosenbrock23::<LS>().unwrap();
        *solver.state_mut().h = 0.1;
        solver.step().unwrap();
        assert!(solver.get_statistics().number_of_error_test_failures > 0);
        assert!(
            solver
                .get_statistics()
                .number_of_linear_solver_setups_from_error_test_fail
                > 0
        );
        let t = solver.state().t;
        assert!(t > 0.0 && t < 0.1);
        assert!((solver.state().y[0] - (-100.0 * t).exp()).abs() < 1e-5);
        advance(&mut solver, 0.1);
        assert!((solver.state().y[0] - (-10.0f64).exp()).abs() < 1e-5);
    }
    #[test]
    fn rosenbrock23_interpolation_and_root_event() {
        let problem = OdeBuilder::<Mat>::new()
            .rtol(1e-8)
            .atol([1e-10])
            .h0(0.8)
            .rhs_implicit(|_, _, _, f| f[0] = 1.0, |_, _, _, _, jv| jv[0] = 0.0)
            .init(|_, _, y| y[0] = 0.0, 1)
            .root(|x, _, _, r| r[0] = x[0] - 0.5, 1)
            .build()
            .unwrap();
        let mut solver = problem.rosenbrock23::<LS>().unwrap();
        let mut found = None;
        for _ in 0..100 {
            if let OdeSolverStopReason::RootFound(t, index) = solver.step().unwrap() {
                found = Some((t, index));
                break;
            }
        }
        let (root, index) = found.expect("root must be detected");
        assert_eq!(index, 0);
        assert!((root - 0.5).abs() < 1e-6);
        assert!((solver.interpolate(root).unwrap()[0] - 0.5).abs() < 1e-6);
        assert!((solver.interpolate_dy(root).unwrap()[0] - 1.0).abs() < 1e-6);
        solver.state_mut_back(root).unwrap();
        assert!((solver.state().y[0] - 0.5).abs() < 1e-6);
    }
    #[test]
    fn rosenbrock23_nonlinear_jacobian_refreshes_after_an_accepted_step() {
        use std::{cell::RefCell, rc::Rc};

        let jacobian_states = Rc::new(RefCell::new(Vec::<(f64, f64)>::new()));
        let observed = jacobian_states.clone();
        let problem = OdeBuilder::<Mat>::new()
            .rtol(1e-7)
            .atol([1e-9])
            .h0(0.05)
            .rhs_implicit(
                |x, _, _, f| f[0] = -x[0] * x[0],
                move |x, _, t, v, jv| {
                    observed.borrow_mut().push((t, x[0]));
                    jv[0] = -2.0 * x[0] * v[0];
                },
            )
            .init(|_, _, y| y[0] = 2.0, 1)
            .build()
            .unwrap();
        let mut solver = problem.rosenbrock23::<LS>().unwrap();
        solver.step().unwrap();
        let first_time = solver.state().t;
        let first_value = solver.state().y[0];
        assert!(first_time > 0.0 && first_value < 2.0);
        solver.step().unwrap();
        let second_time = solver.state().t;
        let second_value = solver.state().y[0];
        assert!(second_time > first_time);
        assert!(
            jacobian_states
                .borrow()
                .iter()
                .any(|(t, y)| { *t == first_time && (*y - first_value).abs() < 1e-12 }),
            "the nonlinear Jacobian was not evaluated at the first accepted state"
        );
        let exact = 2.0 / (1.0 + 2.0 * second_time);
        assert!((second_value - exact).abs() < 1e-4);
    }
    #[test]
    fn built_in_fixed_nonautonomous_matches_julia_with_analytic_ft() {
        for (name, tableau, lambda, h, expected) in [
            (
                "rodas5p",
                Tableau::rodas5p(),
                0.0,
                0.01,
                0.09983341664682904,
            ),
            (
                "rodas5p",
                Tableau::rodas5p(),
                -1000.0,
                0.001,
                0.009999833334166675,
            ),
            (
                "rosenbrock23",
                Tableau::rosenbrock23(),
                0.0,
                0.01,
                0.09983383262061077,
            ),
            (
                "rosenbrock23",
                Tableau::rosenbrock23(),
                -1000.0,
                0.001,
                0.009999834105815892,
            ),
        ] {
            let p = sine_problem(lambda, false);
            let mut s = p
                .rosenbrock_solver::<LS, Mat>(p.rodas5p_state::<LS>().unwrap(), tableau)
                .unwrap();
            for _ in 0..10 {
                let t = s.state().t;
                analytic_step(&mut s, h, -lambda * t.cos() - t.sin());
            }
            let error = (s.state().y[0] - expected).abs();
            assert!(error < 1e-13, "{name}, lambda={lambda}, {error}");
        }
    }
}

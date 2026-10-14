//! Periodic fixed points in the state immediately before the cycle starts.
use super::*;
use crate::{simulate_with_budgets, Budgets, Event, Protocol};
#[derive(Clone, Copy, Debug)]
pub struct SteadyStateTolerance {
    /// Absolute amount tolerance in the kernel's coherent amount unit.
    pub absolute: f64,
    pub relative: f64,
}
#[derive(Clone, Debug)]
pub struct PeriodicSteadyState {
    /// Amounts immediately before start-time resets and administrations.
    pub amounts: Vec<f64>,
    /// Signed difference: end-of-cycle amounts minus the starting amounts.
    pub residual: Vec<f64>,
    pub maximum_scaled_residual: f64,
}
/// Owned static admission for the exact cycle and conservative pass budgets.
pub(crate) struct PreparedPeriodicSteadyState {
    pub(crate) cycle: Protocol,
    pub(crate) partition: Budgets,
    pub(crate) passes: usize,
    pub(crate) tolerance: SteadyStateTolerance,
}
impl LinearPk {
    /// Normalize periodic infusion carryover through the shared cycle normalizer.
    pub fn periodic_cycle_protocol(
        &self,
        cycle: &Protocol,
        budgets: Budgets,
    ) -> Result<Protocol, Error> {
        crate::steady_state::normalize_periodic_cycle(
            &self.bind(&vec![0.; self.state_count()])?,
            cycle,
            budgets,
        )
    }
    /// Solve x = P x + b for a fully contained periodic event cycle.
    /// Events at the right endpoint are unsupported.
    /// All n+2 internal cycle executions share conservative budget partitions.
    pub fn periodic_steady_state(
        &self,
        cycle: &Protocol,
        budgets: Budgets,
        tolerance: SteadyStateTolerance,
    ) -> Result<PeriodicSteadyState, Error> {
        let prepared = self.prepare_periodic_steady_state(cycle, budgets, tolerance)?;
        self.execute_prepared_periodic_steady_state(prepared)
    }
    pub(crate) fn prepare_periodic_steady_state(
        &self,
        cycle: &Protocol,
        budgets: Budgets,
        tolerance: SteadyStateTolerance,
    ) -> Result<PreparedPeriodicSteadyState, Error> {
        if !tolerance.absolute.is_finite()
            || tolerance.absolute <= 0.
            || !tolerance.relative.is_finite()
            || tolerance.relative < 0.
        {
            return Err(invalid("steady-state tolerances must be finite, absolute positive and relative nonnegative"));
        }
        let n = self.state_count();
        let zero = vec![0.; n];
        let mut cycle = self.periodic_cycle_protocol(cycle, budgets)?;
        let passes = n + 2;
        let partition = Budgets {
            solver_callbacks: budgets.solver_callbacks / passes as u64,
            events: budgets.events / passes,
            output_values: budgets.output_values / passes,
        };
        cycle.samples.clear();
        crate::prepare_simulation(&self.bind(&zero)?, &cycle, partition, None)?;
        Ok(PreparedPeriodicSteadyState {
            cycle,
            partition,
            passes,
            tolerance,
        })
    }
    pub(crate) fn execute_prepared_periodic_steady_state(
        &self,
        prepared: PreparedPeriodicSteadyState,
    ) -> Result<PeriodicSteadyState, Error> {
        let PreparedPeriodicSteadyState {
            cycle,
            partition,
            tolerance,
            ..
        } = prepared;
        let n = self.state_count();
        let zero = vec![0.; n];
        let run = |protocol: &Protocol, initial: &[f64]| -> Result<Vec<f64>, Error> {
            let rows = simulate_with_budgets(&self.bind(initial)?, protocol, partition)?;
            rows.last()
                .filter(|r| r.time == protocol.end)
                .map(|r| r.values.clone())
                .ok_or_else(|| domain("missing steady-state cycle endpoint"))
        };
        let offset = run(&cycle, &zero)?;
        let mut homogeneous = cycle.clone();
        // Remove affine additions while retaining the exact event/reset schedule.
        // This avoids subtracting two nearly equal forced-cycle results.
        for event in &mut homogeneous.events {
            match event {
                Event::Bolus { amount, .. } | Event::Infusion { amount, .. } => *amount = 0.,
                Event::Reset { value, .. } => *value = 0.,
            }
        }
        let mut system = vec![vec![0.; n]; n];
        for j in 0..n {
            let mut basis = vec![0.; n];
            basis[j] = 1.;
            let column = run(&homogeneous, &basis)?;
            for i in 0..n {
                system[i][j] = if i == j { 1. - column[i] } else { -column[i] };
            }
        }
        let amounts = solve(system, offset)?;
        if amounts.iter().any(|v| !v.is_finite() || *v < 0.) {
            return Err(domain(
                "steady-state solution is not finite and nonnegative",
            ));
        }
        let terminal = run(&cycle, &amounts)?;
        let residual: Vec<_> = terminal.iter().zip(&amounts).map(|(a, b)| a - b).collect();
        let mut maximum_scaled_residual = 0_f64;
        for ((difference, start), end) in residual.iter().zip(&amounts).zip(&terminal) {
            let denominator = tolerance.absolute + tolerance.relative * start.abs().max(end.abs());
            if !denominator.is_finite() {
                return Err(invalid("steady-state tolerance scale overflow"));
            }
            let scaled = difference.abs() / denominator;
            if !scaled.is_finite() || scaled > 1. {
                return Err(Error::new(
                    ErrorCode::Solver,
                    "steady-state cycle residual exceeds tolerance",
                ));
            }
            maximum_scaled_residual = maximum_scaled_residual.max(scaled);
        }
        Ok(PeriodicSteadyState {
            amounts,
            residual,
            maximum_scaled_residual,
        })
    }
}
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Result<Vec<f64>, Error> {
    let n = b.len();
    let norm = a
        .iter()
        .map(|r| r.iter().map(|x| x.abs()).sum::<f64>())
        .fold(0., f64::max);
    // A transition indistinguishable from identity has no resolvable fixed point.
    let floor = 64. * f64::EPSILON * norm.max(1.);
    for k in 0..n {
        let pivot = (k..n)
            .max_by(|&i, &j| a[i][k].abs().total_cmp(&a[j][k].abs()))
            .unwrap();
        if !a[pivot][k].is_finite() || a[pivot][k].abs() <= floor {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "periodic steady state is non-unique or numerically unresolved",
            ));
        }
        a.swap(k, pivot);
        b.swap(k, pivot);
        for i in k + 1..n {
            let factor = a[i][k] / a[k][k];
            for j in k + 1..n {
                a[i][j] -= factor * a[k][j];
            }
            b[i] -= factor * b[k];
        }
    }
    let mut x = vec![0.; n];
    for i in (0..n).rev() {
        x[i] = (b[i] - (i + 1..n).map(|j| a[i][j] * x[j]).sum::<f64>()) / a[i][i];
    }
    Ok(x)
}

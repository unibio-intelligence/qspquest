//! Constant-coefficient, one-to-three compartment mammillary PK propagation.
//!
//! Inputs use one coherent amount/volume/time basis. The first volume is central;
//! each further volume has one central/peripheral exchange clearance. An optional
//! depot precedes central in state and input vectors. No unit conversion, events,
//! automatic model selection or parameter estimation is performed by this API.
mod steady_state;
use crate::{Error, ErrorCode};
pub(crate) use steady_state::PreparedPeriodicSteadyState;
pub use steady_state::{PeriodicSteadyState, SteadyStateTolerance};
#[derive(Clone, Debug)]
pub struct LinearPk {
    matrix: Vec<Vec<f64>>,
    volumes: Vec<f64>,
    depot: bool,
}
fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidInput, message)
}
fn domain(message: &str) -> Error {
    Error::new(ErrorCode::Domain, message)
}
impl LinearPk {
    pub fn new(
        clearance: f64,
        volumes: &[f64],
        exchange: &[f64],
        absorption: Option<f64>,
    ) -> Result<Self, Error> {
        if !(1..=3).contains(&volumes.len())
            || exchange.len() + 1 != volumes.len()
            || !clearance.is_finite()
            || clearance < 0.
            || volumes.iter().any(|v| !v.is_finite() || *v <= 0.)
            || exchange.iter().any(|q| !q.is_finite() || *q < 0.)
            || absorption.is_some_and(|k| !k.is_finite() || k < 0.)
        {
            return Err(invalid(
                "invalid linear PK clearance, volumes, exchanges or absorption",
            ));
        }
        let central = usize::from(absorption.is_some());
        let n = volumes.len() + central;
        let mut matrix = vec![vec![0.; n]; n];
        matrix[central][central] = -clearance / volumes[0];
        for (i, &q) in exchange.iter().enumerate() {
            let j = central + i + 1;
            matrix[central][central] -= q / volumes[0];
            matrix[j][central] = q / volumes[0];
            matrix[central][j] = q / volumes[i + 1];
            matrix[j][j] = -q / volumes[i + 1];
        }
        if let Some(k) = absorption {
            matrix[0][0] = -k;
            matrix[central][0] = k;
        }
        if matrix.iter().flatten().any(|v| !v.is_finite()) {
            return Err(domain("linear PK rate overflow"));
        }
        Ok(Self {
            matrix,
            volumes: volumes.to_vec(),
            depot: absorption.is_some(),
        })
    }
    pub fn state_count(&self) -> usize {
        self.matrix.len()
    }
    /// Propagate amounts over one interval with constant amount/time inputs.
    /// Uses exp(A dt) and phi_1(A dt), without division by rate differences or
    /// inversion of A. Zero clearance and repeated eigenvalues are supported.
    pub fn advance(
        &self,
        amounts: &[f64],
        rates: &[f64],
        duration: f64,
    ) -> Result<Vec<f64>, Error> {
        let n = self.state_count();
        if amounts.len() != n
            || rates.len() != n
            || !duration.is_finite()
            || duration < 0.
            || amounts
                .iter()
                .chain(rates)
                .any(|v| !v.is_finite() || *v < 0.)
        {
            return Err(invalid(
                "linear PK interval needs finite nonnegative amounts, inputs and duration",
            ));
        }
        if duration == 0. {
            return Ok(amounts.to_vec());
        }
        let mut block = vec![vec![0.; 2 * n]; 2 * n];
        for i in 0..n {
            for j in 0..n {
                block[i][j] = self.matrix[i][j] * duration;
            }
            block[i][i + n] = 1.;
        }
        let propagator = exponential(block)?;
        let input: Vec<_> = rates.iter().map(|r| r * duration).collect();
        if input.iter().any(|v| !v.is_finite()) {
            return Err(domain("linear PK integrated input overflow"));
        }
        let result: Vec<_> = (0..n)
            .map(|i| {
                (0..n)
                    .map(|j| propagator[i][j] * amounts[j] + propagator[i][j + n] * input[j])
                    .sum()
            })
            .collect();
        if result.iter().any(|v: &f64| !v.is_finite() || *v < 0.) {
            return Err(domain(
                "linear PK propagation produced nonfinite or negative amount",
            ));
        }
        Ok(result)
    }
    /// Central and peripheral concentrations, excluding the optional depot.
    pub fn concentrations(&self, amounts: &[f64]) -> Result<Vec<f64>, Error> {
        if amounts.len() != self.state_count() || amounts.iter().any(|v| !v.is_finite() || *v < 0.)
        {
            return Err(invalid("invalid linear PK amounts"));
        }
        let result: Vec<_> = self
            .volumes
            .iter()
            .enumerate()
            .map(|(i, v)| amounts[i + usize::from(self.depot)] / v)
            .collect();
        if result.iter().any(|v| !v.is_finite()) {
            return Err(domain("linear PK concentration overflow"));
        }
        Ok(result)
    }
}
fn product_into(a: &[Vec<f64>], b: &[Vec<f64>], result: &mut [Vec<f64>]) {
    let n = a.len();
    for i in 0..n {
        for j in 0..n {
            result[i][j] = (0..n).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
}
fn exponential(mut a: Vec<Vec<f64>>) -> Result<Vec<Vec<f64>>, Error> {
    let n = a.len();
    let mut norm = a
        .iter()
        .map(|r| r.iter().map(|v| v.abs()).sum::<f64>())
        .fold(0., f64::max);
    if !norm.is_finite() {
        return Err(domain("linear PK interval matrix overflow"));
    }
    let mut squarings = 0;
    while norm > 0.5 {
        if squarings == 60 {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "linear PK interval exceeds exponential scaling limit",
            ));
        }
        for row in &mut a {
            for v in row {
                *v *= 0.5;
            }
        }
        norm *= 0.5;
        squarings += 1;
    }
    let mut result = vec![vec![0.; n]; n];
    for (i, row) in result.iter_mut().enumerate() {
        row[i] = 1.;
    }
    let mut term = result.clone();
    let mut workspace = vec![vec![0.; n]; n];
    // With ||A||_inf <= 0.5, the degree-20 Taylor remainder is < 1.6e-26.
    // Roundoff and repeated squaring still determine practical accuracy.
    for k in 1..=20 {
        product_into(&term, &a, &mut workspace);
        std::mem::swap(&mut term, &mut workspace);
        for i in 0..n {
            for j in 0..n {
                term[i][j] /= k as f64;
                result[i][j] += term[i][j];
            }
        }
    }
    for _ in 0..squarings {
        product_into(&result, &result, &mut workspace);
        std::mem::swap(&mut result, &mut workspace);
    }
    if result.iter().flatten().any(|v| !v.is_finite()) {
        return Err(domain("linear PK exponential overflow"));
    }
    Ok(result)
}

/// Immutable initial amounts bound to a linear PK kernel. Implements the shared
/// model/event contract and opts into analytic interval propagation.
#[derive(Clone, Debug)]
pub struct BoundLinearPk {
    kernel: LinearPk,
    initial: Vec<f64>,
}
impl LinearPk {
    pub fn bind(&self, initial: &[f64]) -> Result<BoundLinearPk, Error> {
        self.advance(initial, &vec![0.; self.state_count()], 0.)?;
        Ok(BoundLinearPk {
            kernel: self.clone(),
            initial: initial.to_vec(),
        })
    }
}
impl crate::IntervalPropagator for LinearPk {
    fn propagate(&self, amounts: &[f64], rates: &[f64], duration: f64) -> Result<Vec<f64>, Error> {
        self.advance(amounts, rates, duration)
    }
}
impl crate::Model for BoundLinearPk {
    fn interval_propagator(&self) -> Option<&dyn crate::IntervalPropagator> {
        Some(&self.kernel)
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn initial(&self) -> Vec<f64> {
        self.initial.clone()
    }
    fn dose_scale(&self, target: usize) -> Option<f64> {
        (target < self.initial.len()).then_some(1.)
    }
    fn check_state(&self, time: f64, state: &[f64]) -> Result<(), Error> {
        if state.len() != self.initial.len() || state.iter().any(|v| !v.is_finite() || *v < 0.) {
            let mut error = Error::new(
                ErrorCode::Invariant,
                "linear PK amounts must be finite and nonnegative",
            );
            error.time = Some(time);
            return Err(error);
        }
        Ok(())
    }
    fn rhs(&self, _time: f64, x: &[f64], rates: &[f64], out: &mut [f64]) -> Result<(), Error> {
        let n = self.initial.len();
        if x.len() != n || rates.len() != n || out.len() != n {
            return Err(invalid("linear PK RHS shape differs"));
        }
        for i in 0..n {
            out[i] = rates[i] + (0..n).map(|j| self.kernel.matrix[i][j] * x[j]).sum::<f64>();
        }
        Ok(())
    }
    fn jac_mul(
        &self,
        _time: f64,
        _x: &[f64],
        _rates: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        let n = self.initial.len();
        if v.len() != n || out.len() != n {
            return Err(invalid("linear PK Jacobian shape differs"));
        }
        for i in 0..n {
            out[i] = (0..n).map(|j| self.kernel.matrix[i][j] * v[j]).sum();
        }
        Ok(())
    }
}

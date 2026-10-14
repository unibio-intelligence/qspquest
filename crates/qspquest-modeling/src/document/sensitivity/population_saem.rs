//! Bounded SAEM for one or two lognormal population parameters.
//!
//! Stochastic approximation tracks the first and second moments of the log
//! individual parameters. A final observed-curvature Laplace calculation
//! reports an approximate marginal OFV when the work budget permits it.
use super::*;
use qspquest_core::{fit::*, identity::canonical_hash};

const LOG_2PI: f64 = 1.8378770664093453;
const MAX_SAEM_CORRELATION: f64 = 0.95;
type Vector = [f64; 2];
type Matrix = [[f64; 2]; 2];

#[cfg(test)]
mod curvature_refinement_tests {
    use super::*;

    #[test]
    fn small_update_drift_cannot_certify_a_descending_marginal_point() {
        let point = [0.5; 6];
        assert!(!certify_no_marginal_descent(
            point,
            [-1.; 6],
            [1.; 6],
            [0.1; 6],
            1.5,
            0.001,
            |x| Ok(Some(x.iter().map(|value| value * value).sum()))
        )
        .unwrap());
        assert!(!certify_no_marginal_descent(
            point,
            [-1.; 6],
            [1.; 6],
            [0.1; 6],
            1.5,
            0.001,
            |_| Ok(None)
        )
        .unwrap());
        assert!(
            certify_no_marginal_descent([0.; 6], [0.; 6], [1.; 6], [0.1; 6], 6., 0.001, |x| Ok(
                Some(x.iter().map(|value| (value + 1.).powi(2)).sum())
            ))
            .unwrap()
        );
    }

    #[test]
    fn correlation_limit_bounds_every_covariance_stencil_corner() {
        for sign in [-1., 1.] {
            let boundary = [[0.16, sign * 0.95 * 0.12], [sign * 0.95 * 0.12, 0.09]];
            assert_eq!(
                correlation_stencil_limit(boundary, &[(4, 0.12)])
                    .unwrap_err()
                    .code,
                ErrorCode::Unsupported
            );
            for rho in [0.6, 0.949, 0.949999] {
                for axes in [vec![(4, 0.12)], vec![(2, 0.16), (3, 0.09), (4, 0.12)]] {
                    let omega = [[0.16, sign * rho * 0.12], [sign * rho * 0.12, 0.09]];
                    let h = 0.001_f64.min(correlation_stencil_limit(omega, &axes).unwrap());
                    assert!(h.is_finite() && h > 0.);
                    for a in 0..axes.len() {
                        for b in 0..axes.len() {
                            for direction_a in [-1., 1.] {
                                for direction_b in [-1., 1.] {
                                    let mut candidate = omega;
                                    for (axis, direction) in [(a, direction_a), (b, direction_b)] {
                                        let (coordinate, scale) = axes[axis];
                                        let shift = h * scale * direction;
                                        match coordinate {
                                            2 | 3 => {
                                                candidate[coordinate - 2][coordinate - 2] += shift
                                            }
                                            4 => {
                                                candidate[0][1] += shift;
                                                candidate[1][0] += shift;
                                            }
                                            _ => unreachable!(),
                                        }
                                    }
                                    assert!(
                                        candidate[0][1].abs()
                                            < MAX_SAEM_CORRELATION
                                                * libm::sqrt(candidate[0][0] * candidate[1][1])
                                    );
                                    assert!(precision(candidate, 2).is_some());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn natural_covariance_matches_correlated_gaussian_information() {
        let theta = [libm::exp(1.4), libm::exp(0.7)];
        let omega = [[0.16, 0.06], [0.06, 0.09]];
        let c = [
            [omega[0][0] + 0.0625 / 3., omega[0][1]],
            [omega[1][0], omega[1][1] + 0.0625 / 2.],
        ];
        let scales = [
            theta[0],
            theta[1],
            omega[0][0],
            omega[1][1],
            libm::sqrt(omega[0][0] * omega[1][1]),
        ];
        let objective = |offset: &[f64; 5]| -> Result<f64, Error> {
            let t = [
                theta[0] + offset[0] * scales[0],
                theta[1] + offset[1] * scales[1],
            ];
            let a = c[0][0] + offset[2] * scales[2];
            let b = c[1][1] + offset[3] * scales[3];
            let cross = c[0][1] + offset[4] * scales[4];
            let det = a * b - cross * cross;
            let delta = [libm::log(t[0]) - 1.4, libm::log(t[1]) - 0.7];
            Ok(32.
                * (libm::log(det)
                    + (b * c[0][0] + a * c[1][1] - 2. * cross * c[0][1]) / det
                    + (b * delta[0] * delta[0] + a * delta[1] * delta[1]
                        - 2. * cross * delta[0] * delta[1])
                        / det))
        };
        let center = objective(&[0.; 5]).unwrap();
        let actual = certified_covariance(5, center, 0.001, objective).unwrap();
        let axes = [(0, 0), (1, 1), (0, 1)];
        let mut expected = [[0.; 5]; 5];
        for i in 0..2 {
            for j in 0..2 {
                expected[i][j] = theta[i] * theta[j] * c[i][j] / 32.;
            }
        }
        for (a, (i, j)) in axes.iter().enumerate() {
            for (b, (k, l)) in axes.iter().enumerate() {
                expected[a + 2][b + 2] = (c[*i][*k] * c[*j][*l] + c[*i][*l] * c[*j][*k]) / 32.;
            }
        }
        for i in 0..5 {
            for j in 0..5 {
                let natural = actual[i][j] * scales[i] * scales[j];
                let error =
                    (natural - expected[i][j]).abs() / libm::sqrt(expected[i][i] * expected[j][j]);
                assert!(error < 0.001, "entry {i},{j}: {error}");
            }
        }
    }

    #[test]
    fn refinement_preserves_budget_errors() {
        let mut evaluations = 0;
        let error = certified_covariance(2, 0., 0.001, |x| {
            evaluations += 1;
            if evaluations > 8 {
                return Err(Error::new(ErrorCode::WorkBudget, "synthetic budget"));
            }
            Ok(x[0] * x[0] + x[1] * x[1])
        })
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::WorkBudget);
        assert_eq!(evaluations, 9);
    }

    #[test]
    fn unidentified_information_and_boundary_are_refused() {
        assert_eq!(
            certified_covariance(2, 0., 0.001, |x| Ok(x[0] * x[0]))
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            certified_covariance(2, 0., 0., |_| panic!("boundary must not be evaluated"))
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
    }
}

// Positive definiteness after scaling the cross covariance by the correlation
// cap describes the stricter admissible region for the uncertainty stencil.
fn correlation_stencil_limit(omega: Matrix, axes: &[(usize, f64)]) -> Result<f64, Error> {
    if omega[0][1].abs() >= MAX_SAEM_CORRELATION * libm::sqrt(omega[0][0] * omega[1][1]) {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "SAEM observed covariance requires a correlation strictly inside the permitted +/-0.95 limit",
        ));
    }
    let cross = omega[0][1] / MAX_SAEM_CORRELATION;
    let trace = omega[0][0] + omega[1][1];
    let largest =
        0.5 * (trace + libm::sqrt((omega[0][0] - omega[1][1]).powi(2) + 4. * cross.powi(2)));
    let smallest = (omega[0][0] * omega[1][1] - cross.powi(2)) / largest;
    let covariance_scale = axes
        .iter()
        .filter(|(axis, _)| *axis >= 2)
        .map(|(axis, scale)| {
            if *axis == 4 {
                2. * scale / MAX_SAEM_CORRELATION
            } else {
                *scale
            }
        })
        .sum::<f64>();
    Ok(0.1 * smallest / covariance_scale)
}

fn observed_covariance_failure(evaluations: usize, budget: usize, reason: &str) -> Error {
    if evaluations >= budget {
        Error::new(
            ErrorCode::WorkBudget,
            format!("SAEM observed covariance is unresolved after exhausting the evaluation budget ({evaluations} evaluations, marginal limit {budget}). Increase max_evaluations within supported limits, or request uncertainty=false to inspect diagnostic point estimates."),
        )
    } else {
        Error::new(ErrorCode::Unsupported, reason)
    }
}

// Hessian of the normalized marginal OFV; its inverse is twice the covariance.
fn covariance_at_step(
    dimensions: usize,
    center: f64,
    h: f64,
    evaluate: &mut impl FnMut(&[f64; 5]) -> Result<f64, Error>,
) -> Result<Vec<Vec<f64>>, Error> {
    let mut hessian = [[0.; 5]; 5];
    for axis in 0..dimensions {
        let mut offsets = [0.; 5];
        offsets[axis] = h;
        let plus = evaluate(&offsets)?;
        offsets[axis] = -h;
        let minus = evaluate(&offsets)?;
        hessian[axis][axis] = (plus - 2. * center + minus) / (h * h);
        for other in 0..axis {
            let mut offsets = [0.; 5];
            offsets[axis] = h;
            offsets[other] = h;
            let pp = evaluate(&offsets)?;
            offsets[other] = -h;
            let pm = evaluate(&offsets)?;
            offsets[axis] = -h;
            offsets[other] = h;
            let mp = evaluate(&offsets)?;
            offsets[other] = -h;
            let mm = evaluate(&offsets)?;
            let cross = (pp - pm - mp + mm) / (4. * h * h);
            hessian[axis][other] = cross;
            hessian[other][axis] = cross;
        }
    }
    // Cholesky inversion rejects an unidentified covariance direction.
    let mut lower = [[0.; 5]; 5];
    for row in 0..dimensions {
        for col in 0..=row {
            let mut value = hessian[row][col];
            for k in 0..col {
                value -= lower[row][k] * lower[col][k];
            }
            if row == col {
                if !value.is_finite() || value <= 1e-9_f64.max(1e-8 * hessian[row][row].abs()) {
                    return Err(Error::new(
                        ErrorCode::Unsupported,
                        "SAEM observed information is singular or not positive definite",
                    ));
                }
                lower[row][col] = libm::sqrt(value);
            } else {
                lower[row][col] = value / lower[col][col];
            }
        }
    }
    let mut inverse = [[0.; 5]; 5];
    for column in 0..dimensions {
        let mut y = [0.; 5];
        for row in 0..dimensions {
            let mut value = if row == column { 1. } else { 0. };
            for k in 0..row {
                value -= lower[row][k] * y[k];
            }
            y[row] = value / lower[row][row];
        }
        for row in (0..dimensions).rev() {
            let mut value = y[row];
            for k in row + 1..dimensions {
                value -= lower[k][row] * inverse[k][column];
            }
            inverse[row][column] = value / lower[row][row];
        }
    }
    let covariance = (0..dimensions)
        .map(|row| {
            (0..dimensions)
                .map(|col| 2. * inverse[row][col])
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    Ok(covariance)
}

fn certified_covariance(
    dimensions: usize,
    center: f64,
    mut h: f64,
    mut evaluate: impl FnMut(&[f64; 5]) -> Result<f64, Error>,
) -> Result<Vec<Vec<f64>>, Error> {
    if !h.is_finite() || h <= 0. {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "SAEM observed covariance requires an interior positive-definite estimate",
        ));
    }
    let mut previous = covariance_at_step(dimensions, center, h, &mut evaluate)?;
    for _ in 0..4 {
        h *= 0.5;
        let refined = covariance_at_step(dimensions, center, h, &mut evaluate)?;
        let mut error = 0_f64;
        for row in 0..dimensions {
            for col in 0..dimensions {
                let scale = libm::sqrt(refined[row][row] * refined[col][col]);
                if !scale.is_finite() || scale <= 0. {
                    return Err(Error::new(
                        ErrorCode::Unsupported,
                        "SAEM observed covariance is not finite and positive",
                    ));
                }
                error = error.max((previous[row][col] - refined[row][col]).abs() / scale);
            }
        }
        if error <= 0.001 {
            return Ok(refined);
        }
        previous = refined;
    }
    Err(Error::new(ErrorCode::Unsupported,
        "SAEM observed covariance could not certify finite-difference refinement; request uncertainty=false to inspect diagnostic point estimates"))
}

fn certify_no_marginal_descent(
    point: [f64; 6],
    lower: [f64; 6],
    upper: [f64; 6],
    steps: [f64; 6],
    center: f64,
    tolerance: f64,
    mut evaluate: impl FnMut([f64; 6]) -> Result<Option<f64>, Error>,
) -> Result<bool, Error> {
    for axis in 0..6 {
        if lower[axis] == upper[axis] {
            continue;
        }
        for sign in [-1., 1.] {
            let mut trial = point;
            trial[axis] = (point[axis] + sign * steps[axis]).clamp(lower[axis], upper[axis]);
            if trial[axis] == point[axis] {
                continue;
            }
            let Some(value) = evaluate(trial)? else {
                return Ok(false);
            };
            if !value.is_finite() || value < center - tolerance {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

struct RefinedSaemPoint {
    theta: Vector,
    omega: Matrix,
    residual_variance: f64,
    marginal: (f64, Vec<Vector>),
    certified: bool,
    iterations: usize,
}

enum PreparedSaem {
    One(PreparedSaemOneDim),
    Two(PreparedSaemTwoDim),
}
struct PreparedSaemOneDim {
    cfg: super::fit::PopulationOneDim,
    setup: Setup,
    burn_in: usize,
    residual: Option<ResidualFit>,
    subjects: Vec<GaussianObjectiveRequest>,
    covariates: Vec<PopulationCovariateEffect>,
    x: Vec<Vec<f64>>,
    beta: Vec<f64>,
    mean_x: Vec<f64>,
    second_x: Vec<Vec<f64>>,
    covariance_x: Vec<Vec<f64>>,
}
struct PreparedSaemTwoDim {
    setup: Setup,
    burn_in: usize,
    residual: Option<ResidualFit>,
    subjects: Vec<GaussianObjectiveRequest>,
}

struct Setup {
    dimensions: usize,
    names: [String; 2],
    units: [String; 2],
    start: Vector,
    lower: Vector,
    upper: Vector,
    omega: Matrix,
    omega_lower: Vector,
    omega_upper: Vector,
}

struct ResidualFit {
    output: String,
    unit: String,
    variance: f64,
    lower_variance: f64,
    upper_variance: f64,
    observations: usize,
}

#[derive(Clone, Copy, Default)]
struct LagOne {
    count: usize,
    sum_previous: f64,
    sum_current: f64,
    sum_previous_sq: f64,
    sum_current_sq: f64,
    sum_product: f64,
}

impl LagOne {
    fn observe(&mut self, previous: f64, current: f64) {
        self.count += 1;
        self.sum_previous += previous;
        self.sum_current += current;
        self.sum_previous_sq += previous * previous;
        self.sum_current_sq += current * current;
        self.sum_product += previous * current;
    }

    fn correlation(&self) -> Option<f64> {
        if self.count < 2 {
            return None;
        }
        let n = self.count as f64;
        let covariance = self.sum_product - self.sum_previous * self.sum_current / n;
        let previous_variance = self.sum_previous_sq - self.sum_previous * self.sum_previous / n;
        let current_variance = self.sum_current_sq - self.sum_current * self.sum_current / n;
        let denominator = libm::sqrt(previous_variance * current_variance);
        if !denominator.is_finite() || denominator <= 0. {
            return Some(1.);
        }
        Some((covariance / denominator).clamp(-1., 1.))
    }
}

fn validate_residual_fit(
    model: &CompiledSensitivityDocument,
    request: &PopulationFitRequest,
) -> Result<Option<ResidualFit>, Error> {
    let Some(fit) = &request.additive_error_fit else {
        return Ok(None);
    };
    let output_index = model
        .output_names()
        .iter()
        .position(|name| name == &fit.output)
        .ok_or_else(|| invalid("unknown SAEM additive-error fit output"))?;
    let unit = model.output_units()[output_index].clone();
    let parsed_unit = Unit::parse(&unit)?;
    let initial = quantity(&fit.initial)?.in_unit(parsed_unit)?;
    let lower = quantity(&fit.lower)?.in_unit(parsed_unit)?;
    let upper = quantity(&fit.upper)?.in_unit(parsed_unit)?;
    if !initial.is_finite()
        || !lower.is_finite()
        || !upper.is_finite()
        || lower <= 0.
        || lower >= upper
        || initial < lower
        || initial > upper
        || lower.powi(2) == 0.
        || initial.powi(2) == 0.
        || !upper.powi(2).is_finite()
    {
        return Err(invalid(
            "SAEM additive SD requires positive finite bounds containing its initial value",
        ));
    }
    let mut observations = 0usize;
    let mut uncensored = 0usize;
    for (subject_index, subject) in request.subjects.iter().enumerate() {
        if !subject.priors.is_empty() {
            return Err(invalid("SAEM subject priors are unsupported"));
        }
        for (row, observation) in subject.observations.iter().enumerate() {
            let declared = quantity(&observation.error.additive_sd)?.in_unit(parsed_unit)?;
            if observation.output != fit.output
                || observation.error.model != GaussianErrorModel::Normal
                || observation.error.proportional_sd != 0.
                || (declared - initial).abs() > 1e-10 * initial
            {
                return Err(invalid(format!(
                    "subjects[{subject_index}].observations[{row}] must use the fitted output, zero proportional error, and initial additive SD"
                )));
            }
            observations += 1;
            uncensored += usize::from(observation.censoring.is_none());
        }
    }
    if observations == 0 {
        return Err(invalid("SAEM additive SD requires observations"));
    }
    if uncensored == 0 {
        return Err(invalid(
            "SAEM additive SD cannot be identified from all-censored observations",
        ));
    }
    Ok(Some(ResidualFit {
        output: fit.output.clone(),
        unit,
        variance: initial * initial,
        lower_variance: lower * lower,
        upper_variance: upper * upper,
        observations,
    }))
}

/// The censored rows are integrated out in the E-step. For a left-censored
/// normal residual Z <= a, E[Z² | Z <= a] = 1 - a phi(a)/Phi(a).
/// This is the Rao-Blackwellized complete-data statistic for the variance M-step.
fn residual_moments(
    subject: &GaussianObjectiveRequest,
    predictions: &[f64],
    fit: &ResidualFit,
) -> Result<(f64, f64), Error> {
    if predictions.len() != subject.observations.len() {
        return Err(Error::new(
            ErrorCode::Invariant,
            "SAEM prediction count differs from observations",
        ));
    }
    let unit = Unit::parse(&fit.unit)?;
    let sd = libm::sqrt(fit.variance);
    let mut nll = 0.;
    let mut sum_squares = 0.;
    for (row, (observation, prediction)) in subject.observations.iter().zip(predictions).enumerate()
    {
        let value = quantity(&observation.value)?.in_unit(unit)?;
        let (term, moment) = if observation.censoring.is_some() {
            let (term, score) = super::objective::normal_left_censored(*prediction, value, sd, 0.)?;
            let z = (value - prediction) / sd;
            (term, fit.variance * (1. - z * score * sd))
        } else {
            let difference = value - prediction;
            (
                0.5 * difference * difference / fit.variance + libm::log(sd) + 0.5 * LOG_2PI,
                difference * difference,
            )
        };
        if !term.is_finite() || !moment.is_finite() || moment < 0. {
            return Err(invalid(format!(
                "SAEM residual moment is invalid at observation {row}"
            )));
        }
        nll += term;
        sum_squares += moment;
    }
    Ok((nll, sum_squares))
}

fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

fn solve_covariate_regression(matrix: &[Vec<f64>], rhs: &[f64]) -> Result<Vec<f64>, Error> {
    let n = rhs.len();
    let mut augmented = (0..n)
        .map(|row| {
            let mut values = matrix[row].clone();
            values.push(rhs[row]);
            values
        })
        .collect::<Vec<_>>();
    for column in 0..n {
        let pivot = (column..n)
            .max_by(|a, b| {
                augmented[*a][column]
                    .abs()
                    .total_cmp(&augmented[*b][column].abs())
            })
            .expect("nonempty pivot range");
        augmented.swap(column, pivot);
        if augmented[column][column].abs() < 1e-8 * matrix[column][column].abs().max(1.) {
            return Err(invalid(
                "SAEM covariate design is collinear or unidentifiable",
            ));
        }
        let divisor = augmented[column][column];
        for cell in column..=n {
            augmented[column][cell] /= divisor;
        }
        for row in 0..n {
            if row == column {
                continue;
            }
            let factor = augmented[row][column];
            for cell in column..=n {
                augmented[row][cell] -= factor * augmented[column][cell];
            }
        }
    }
    Ok(augmented.iter().map(|row| row[n]).collect())
}

fn bounded_covariate_regression(
    covariance: &[Vec<f64>],
    rhs: &[f64],
    bounds: &[[f64; 2]],
) -> Result<Vec<f64>, Error> {
    let n = rhs.len();
    let mut best: Option<(f64, Vec<f64>)> = None;
    for mut code in 0..3_usize.pow(n as u32) {
        let mut candidate = vec![0.; n];
        let mut free = Vec::new();
        for axis in 0..n {
            match code % 3 {
                0 => free.push(axis),
                1 => candidate[axis] = bounds[axis][0],
                _ => candidate[axis] = bounds[axis][1],
            }
            code /= 3;
        }
        if !free.is_empty() {
            let matrix = free
                .iter()
                .map(|&row| {
                    free.iter()
                        .map(|&col| covariance[row][col])
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let target = free
                .iter()
                .map(|&row| {
                    rhs[row]
                        - (0..n)
                            .filter(|axis| !free.contains(axis))
                            .map(|axis| covariance[row][axis] * candidate[axis])
                            .sum::<f64>()
                })
                .collect::<Vec<_>>();
            let solution = solve_covariate_regression(&matrix, &target)?;
            for (&axis, value) in free.iter().zip(solution) {
                candidate[axis] = value;
            }
        }
        if (0..n).any(|axis| {
            !candidate[axis].is_finite()
                || candidate[axis] < bounds[axis][0] - 1e-10
                || candidate[axis] > bounds[axis][1] + 1e-10
        }) {
            continue;
        }
        for axis in 0..n {
            candidate[axis] = candidate[axis].clamp(bounds[axis][0], bounds[axis][1]);
        }
        let objective = 0.5
            * (0..n)
                .map(|row| {
                    (0..n)
                        .map(|col| candidate[row] * covariance[row][col] * candidate[col])
                        .sum::<f64>()
                })
                .sum::<f64>()
            - dot(&candidate, rhs);
        if best
            .as_ref()
            .is_none_or(|(minimum, _)| objective < *minimum)
        {
            best = Some((objective, candidate));
        }
    }
    best.map(|(_, coefficients)| coefficients)
        .ok_or_else(|| invalid("SAEM bounded covariate regression has no feasible solution"))
}

#[cfg(test)]
mod covariate_regression_tests {
    use super::bounded_covariate_regression;

    #[test]
    fn correlated_effect_at_bound_reoptimizes_the_other_coefficient() {
        let result = bounded_covariate_regression(
            &[vec![1., 0.8], vec![0.8, 1.]],
            &[1., 0.1],
            &[[0., 0.5], [-1., 1.]],
        )
        .unwrap();
        assert!((result[0] - 0.5).abs() < 1e-12);
        assert!((result[1] + 0.3).abs() < 1e-12);
    }
}

fn set_residual_sd(subjects: &mut [GaussianObjectiveRequest], fit: &ResidualFit) {
    let sd = libm::sqrt(fit.variance);
    for subject in subjects {
        for observation in &mut subject.observations {
            observation.error.additive_sd = qspquest_core::model::Quantity {
                value: sd,
                unit: fit.unit.clone(),
            };
        }
    }
}

fn burn_in_iterations(request: &PopulationFitRequest) -> Result<usize, Error> {
    let burn_in = request
        .burn_in_iterations
        .unwrap_or((request.max_iterations / 3).max(4));
    if burn_in < 4 || burn_in >= request.max_iterations {
        return Err(invalid(
            "SAEM burn-in must be at least four iterations and below max_iterations",
        ));
    }
    Ok(burn_in)
}

fn precision(omega: Matrix, dimensions: usize) -> Option<(Matrix, f64)> {
    if dimensions == 1 {
        let variance = omega[0][0];
        return (variance.is_finite() && variance > 0.)
            .then_some(([[1. / variance, 0.], [0., 0.]], variance));
    }
    let det = omega[0][0] * omega[1][1] - omega[0][1] * omega[1][0];
    if !omega.iter().flatten().all(|x| x.is_finite())
        || omega[0][1] != omega[1][0]
        || omega[0][0] <= 0.
        || omega[1][1] <= 0.
        || det <= 1e-10 * omega[0][0] * omega[1][1]
    {
        return None;
    }
    Some((
        [
            [omega[1][1] / det, -omega[0][1] / det],
            [-omega[1][0] / det, omega[0][0] / det],
        ],
        det,
    ))
}

fn prior_nll_vector(eta: Vector, omega: Matrix, dimensions: usize) -> Result<f64, Error> {
    let (inverse, det) = precision(omega, dimensions)
        .ok_or_else(|| invalid("SAEM Omega must remain positive definite"))?;
    let quadratic = (0..dimensions)
        .map(|i| {
            (0..dimensions)
                .map(|j| eta[i] * inverse[i][j] * eta[j])
                .sum::<f64>()
        })
        .sum::<f64>();
    Ok(0.5 * (quadratic + libm::log(det) + dimensions as f64 * LOG_2PI))
}

#[derive(Clone, Copy)]
struct JointProposal {
    mean: Vector,
    precision: Matrix,
    lower: Matrix,
}

impl JointProposal {
    fn draw(&self, stream: &mut Stream) -> Vector {
        let z = [stream.normal(), stream.normal()];
        [
            self.mean[0] + self.lower[0][0] * z[0],
            self.mean[1] + self.lower[1][0] * z[0] + self.lower[1][1] * z[1],
        ]
    }

    fn log_kernel(&self, phi: Vector) -> f64 {
        let d = [phi[0] - self.mean[0], phi[1] - self.mean[1]];
        -0.5 * (d[0] * (self.precision[0][0] * d[0] + self.precision[0][1] * d[1])
            + d[1] * (self.precision[1][0] * d[0] + self.precision[1][1] * d[1]))
    }
}

struct Stream(u64);

impl Stream {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        // SplitMix64 has a fully specified stream across native and WASM.
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    fn uniform(&mut self) -> f64 {
        // In (0, 1), so Box-Muller and log acceptance stay finite.
        ((self.next_u64() >> 11) as f64 + 0.5) * (1.0 / ((1_u64 << 53) as f64))
    }

    fn normal(&mut self) -> f64 {
        libm::sqrt(-2.0 * libm::log(self.uniform()))
            * libm::cos(2.0 * std::f64::consts::PI * self.uniform())
    }
}

fn prior_nll(phi: f64, log_theta: f64, omega: f64) -> f64 {
    let eta = phi - log_theta;
    0.5 * (eta * eta / omega + libm::log(omega)) + 0.9189385332046727
}

// Compare two halves of a post-stabilization history. Consecutive SAEM
// updates retain Monte Carlo noise even when the parameter trajectory is
// stationary, so a single-update threshold is not a useful stopping rule.
fn windowed_drift(values: &[f64], tolerance: f64) -> (f64, f64, bool) {
    let half = values.len() / 2;
    let (older, newer) = values.split_at(half);
    let old_mean = older.iter().sum::<f64>() / older.len() as f64;
    let new_mean = newer.iter().sum::<f64>() / newer.len() as f64;
    let variance_of_mean = |part: &[f64], mean: f64| {
        let centered = part.iter().map(|x| x - mean).collect::<Vec<_>>();
        let sum_squares = centered.iter().map(|x| x * x).sum::<f64>();
        let lag_one = centered
            .windows(2)
            .map(|pair| pair[0] * pair[1])
            .sum::<f64>();
        // SA trajectories are serially correlated. Inflate the variance of
        // each half-window mean by a capped AR(1) effective-sample factor.
        let correlation = if sum_squares > 0. {
            (lag_one / sum_squares).clamp(0., 0.9)
        } else {
            0.
        };
        sum_squares / ((part.len() - 1) * part.len()) as f64 * (1. + correlation)
            / (1. - correlation)
    };
    // Inputs are log parameters, so their difference approximates relative
    // change on the natural scale without a second coordinate-dependent scale.
    let drift = (new_mean - old_mean).abs();
    let noise =
        2.0 * libm::sqrt(variance_of_mean(older, old_mean) + variance_of_mean(newer, new_mean));
    // The noise allowance is capped: a wandering chain must not receive a
    // convergence label just because its Monte Carlo uncertainty is large.
    (drift, noise, drift <= tolerance.max(noise).min(0.005))
}

fn validate_two_effects(
    model: &Arc<CompiledSensitivityDocument>,
    request: &PopulationFitRequest,
) -> Result<Setup, Error> {
    if request.subjects.len() < 2
        || request.subjects.len() > 128
        || request.fixed_effects.len() != 2
        || request.random_effects.len() != 2
        || request.omega.len() != 2
        || request.omega.iter().any(|row| row.len() != 2)
        || request.omega_diagonal_bounds.len() != 2
        || model.parameters().len() != 2
    {
        return Err(Error::new(ErrorCode::Unsupported, "two-effect SAEM requires two selected fixed and random effects across 2..=128 subjects"));
    }
    if !request.eta_bound.is_finite()
        || request.eta_bound <= 0.
        || request.eta_bound > 8.
        || request.max_evaluations == 0
        || request.max_evaluations > 200_000
        || request.max_iterations < 8
        || request.max_iterations > 10_000
        || !request.gradient_tolerance.is_finite()
        || request.gradient_tolerance <= 0.
    {
        return Err(invalid("two-effect SAEM needs bounded positive controls"));
    }
    let mut setup = Setup {
        dimensions: 2,
        names: [model.parameters()[0].clone(), model.parameters()[1].clone()],
        units: [String::new(), String::new()],
        start: [0.; 2],
        lower: [0.; 2],
        upper: [0.; 2],
        omega: [
            [request.omega[0][0], request.omega[0][1]],
            [request.omega[1][0], request.omega[1][1]],
        ],
        omega_lower: [0.; 2],
        omega_upper: [0.; 2],
    };
    for axis in 0..2 {
        let name = &setup.names[axis];
        if request.fixed_effects[axis].parameter != *name
            || request.random_effects[axis].parameter != *name
        {
            return Err(invalid(
                "SAEM fixed and random effects must follow selected parameter order",
            ));
        }
        let parameter = model
            .primal
            .parameters
            .iter()
            .find(|p| p.name == *name)
            .ok_or_else(|| invalid("unknown SAEM parameter"))?;
        if parameter.fixed {
            return Err(invalid("cannot fit a fixed model parameter"));
        }
        let unit = Unit::parse(&parameter.default.unit)?;
        let lower = quantity(&request.fixed_effects[axis].lower)?.in_unit(unit)?;
        let upper = quantity(&request.fixed_effects[axis].upper)?.in_unit(unit)?;
        let start = request.subjects[0]
            .simulation
            .run
            .parameters
            .get(name)
            .map(|q| quantity(q)?.in_unit(unit))
            .transpose()?
            .unwrap_or(quantity(&parameter.default)?.in_unit(unit)?);
        if !lower.is_finite()
            || !upper.is_finite()
            || !start.is_finite()
            || lower <= 0.
            || lower >= upper
            || start < lower
            || start > upper
        {
            return Err(invalid(
                "SAEM lognormal fixed effects need positive finite bounds containing their starts",
            ));
        }
        if let Some(bounds) = &parameter.bounds {
            if lower * libm::exp(-request.eta_bound) < quantity(&bounds[0])?.in_unit(unit)?
                || upper * libm::exp(request.eta_bound) > quantity(&bounds[1])?.in_unit(unit)?
            {
                return Err(invalid(
                    "SAEM fixed-effect and eta boxes exceed model bounds",
                ));
            }
        }
        setup.units[axis] = parameter.default.unit.clone();
        setup.start[axis] = start;
        setup.lower[axis] = lower;
        setup.upper[axis] = upper;
        let [variance_lower, variance_upper] = request.omega_diagonal_bounds[axis];
        if !variance_lower.is_finite()
            || !variance_upper.is_finite()
            || variance_lower <= 0.
            || variance_lower > variance_upper
            || setup.omega[axis][axis] < variance_lower
            || setup.omega[axis][axis] > variance_upper
        {
            return Err(invalid(
                "SAEM Omega variance bounds must be finite and positive",
            ));
        }
        setup.omega_lower[axis] = variance_lower;
        setup.omega_upper[axis] = variance_upper;
    }
    if precision(setup.omega, 2).is_none() {
        return Err(invalid("SAEM Omega must be symmetric positive definite"));
    }
    for (index, subject) in request.subjects.iter().enumerate() {
        if !subject.priors.is_empty() || subject.simulation.with_respect_to != model.parameters() {
            return Err(invalid(format!(
                "subject {index} must share selected parameters and use no priors"
            )));
        }
        let bound = model.bind_with_initial_states(
            &subject.simulation.run.parameters,
            &subject.simulation.run.regimen.covariates,
            &subject.simulation.run.initial_states,
        )?;
        for axis in 0..2 {
            let parameter_index = model
                .primal
                .parameters
                .iter()
                .position(|p| p.name == setup.names[axis])
                .unwrap();
            if (bound.primal.values[parameter_index] - setup.start[axis]).abs()
                > 1e-10 * setup.start[axis].abs().max(1.)
            {
                return Err(invalid(format!(
                    "subject {index} has a conflicting fixed-effect start"
                )));
            }
        }
    }
    Ok(setup)
}

impl CompiledSensitivityDocument {
    fn saem_nll(
        self: &Arc<Self>,
        subject: &GaussianObjectiveRequest,
        setup: &Setup,
        theta: Vector,
        eta: Vector,
    ) -> Result<f64, Error> {
        let mut objective = subject.clone();
        self.saem_nll_in_place(&mut objective, setup, theta, eta)
    }

    // Every selected parameter is overwritten before the solve, so a rejected
    // proposal cannot leave parameter values that affect the next evaluation.
    fn saem_nll_in_place(
        self: &Arc<Self>,
        objective: &mut GaussianObjectiveRequest,
        setup: &Setup,
        theta: Vector,
        eta: Vector,
    ) -> Result<f64, Error> {
        self.saem_nll_in_place_with_predictions(objective, setup, theta, eta)
            .map(|(nll, _)| nll)
    }

    fn saem_nll_in_place_with_predictions(
        self: &Arc<Self>,
        objective: &mut GaussianObjectiveRequest,
        setup: &Setup,
        theta: Vector,
        eta: Vector,
    ) -> Result<(f64, Vec<f64>), Error> {
        for axis in 0..setup.dimensions {
            let individual = theta[axis] * libm::exp(eta[axis]);
            if !individual.is_finite() || individual <= 0. {
                return Err(Error::new(
                    ErrorCode::Domain,
                    "invalid SAEM individual parameter",
                ));
            }
            objective.simulation.run.parameters.insert(
                setup.names[axis].clone(),
                qspquest_core::model::Quantity {
                    value: individual,
                    unit: setup.units[axis].clone(),
                },
            );
        }
        self.gaussian_primal_nll_and_predictions(objective)
    }

    fn saem_predictions(
        self: &Arc<Self>,
        subject_index: usize,
        subject: &GaussianObjectiveRequest,
        setup: &Setup,
        theta: Vector,
        eta: Vector,
    ) -> Result<Vec<PopulationFittedObservation>, Error> {
        let mut population = subject.simulation.run.clone();
        let mut individual = subject.simulation.run.clone();
        for axis in 0..setup.dimensions {
            let name = setup.names[axis].clone();
            let unit = setup.units[axis].clone();
            population.parameters.insert(
                name.clone(),
                qspquest_core::model::Quantity {
                    value: theta[axis],
                    unit: unit.clone(),
                },
            );
            individual.parameters.insert(
                name,
                qspquest_core::model::Quantity {
                    value: theta[axis] * libm::exp(eta[axis]),
                    unit,
                },
            );
        }
        let pred = self.primal.execute(&population)?;
        let ipred = self.primal.execute(&individual)?;
        if pred.times != ipred.times || pred.sides != ipred.sides {
            return Err(Error::new(
                ErrorCode::Invariant,
                "SAEM prediction grids differ across eta",
            ));
        }
        subject
            .observations
            .iter()
            .enumerate()
            .map(|(observation_index, observation)| {
                let output = self
                    .output_names()
                    .iter()
                    .position(|name| name == &observation.output)
                    .ok_or_else(|| invalid("unknown SAEM observation output"))?;
                let unit = self.output_units()[output].clone();
                let parsed = Unit::parse(&unit)?;
                let value = quantity(&observation.value)?.in_unit(parsed)?;
                let prediction = *pred.outputs[output]
                    .values
                    .get(observation.row)
                    .ok_or_else(|| {
                        invalid("SAEM observation row is outside the simulation result")
                    })?;
                let individual_prediction = *ipred.outputs[output]
                    .values
                    .get(observation.row)
                    .ok_or_else(|| {
                        invalid("SAEM observation row is outside the simulation result")
                    })?;
                let time = *pred.times.get(observation.row).ok_or_else(|| {
                    invalid("SAEM observation row is outside the simulation result")
                })?;
                let side = pred.sides[observation.row];
                let q = |value| qspquest_core::model::Quantity {
                    value,
                    unit: unit.clone(),
                };
                Ok(PopulationFittedObservation {
                    subject_index,
                    observation_index,
                    row: observation.row,
                    output: observation.output.clone(),
                    time: qspquest_core::model::Quantity {
                        value: time,
                        unit: self.primal.time_unit.clone(),
                    },
                    side,
                    value: q(value),
                    pred: q(prediction),
                    ipred: q(individual_prediction),
                })
            })
            .collect()
    }

    fn saem_curvature_proposal(
        self: &Arc<Self>,
        subject: &GaussianObjectiveRequest,
        setup: &Setup,
        theta: Vector,
        omega: Matrix,
        mut center: Vector,
        eta_bound: f64,
        evaluations: &mut usize,
        budget: usize,
    ) -> Result<Option<JointProposal>, Error> {
        let h = 0.005;
        let log_theta = [libm::log(theta[0]), libm::log(theta[1])];
        let mut last = None;
        // The anchor is a deterministic population mean or previous proposal mode,
        // never the current random subject chain location.
        for _ in 0..4 {
            let eta = [center[0] - log_theta[0], center[1] - log_theta[1]];
            if eta
                .iter()
                .any(|v| !v.is_finite() || v.abs() + h >= eta_bound)
            {
                return Ok(None);
            }
            let mut values = [[0.; 3]; 3];
            for a in 0..3 {
                for b in 0..3 {
                    let trial = [eta[0] + (a as f64 - 1.) * h, eta[1] + (b as f64 - 1.) * h];
                    let Some(value) =
                        self.saem_joint(subject, setup, theta, omega, trial, evaluations, budget)?
                    else {
                        return Ok(None);
                    };
                    values[a][b] = value;
                }
            }
            let gradient = [
                (values[2][1] - values[0][1]) / (2. * h),
                (values[1][2] - values[1][0]) / (2. * h),
            ];
            let cross = (values[2][2] - values[2][0] - values[0][2] + values[0][0]) / (4. * h * h);
            let mut curvature = [
                [
                    (values[2][1] - 2. * values[1][1] + values[0][1]) / (h * h),
                    cross,
                ],
                [
                    cross,
                    (values[1][2] - 2. * values[1][1] + values[1][0]) / (h * h),
                ],
            ];
            if !curvature.iter().flatten().all(|v| v.is_finite()) {
                return Ok(None);
            }
            if precision(curvature, 2).is_none() {
                let minimum = 0.5
                    * (curvature[0][0] + curvature[1][1]
                        - libm::sqrt(
                            (curvature[0][0] - curvature[1][1]).powi(2) + 4. * cross * cross,
                        ));
                let scale = curvature[0][0].abs().max(curvature[1][1].abs()).max(1.);
                let damping = (0.01 * scale - minimum).max(0.);
                curvature[0][0] += damping;
                curvature[1][1] += damping;
            }
            let Some((covariance, _)) = precision(curvature, 2) else {
                return Ok(None);
            };
            let l00 = libm::sqrt(covariance[0][0]);
            let l10 = covariance[1][0] / l00;
            let l11 = libm::sqrt(covariance[1][1] - l10 * l10);
            if !l11.is_finite() || l11 <= 0. {
                return Ok(None);
            }
            let mut q = JointProposal {
                mean: center,
                precision: curvature,
                lower: [[l00, 0.], [l10, l11]],
            };
            if gradient.iter().all(|v| v.abs() < 1e-5) {
                return Ok(Some(q));
            }
            let step: Vector = std::array::from_fn(|a| {
                covariance[a][0] * gradient[0] + covariance[a][1] * gradient[1]
            });
            if step.iter().any(|v| !v.is_finite()) {
                return Ok(last);
            }
            let max_step = step[0].abs().max(step[1].abs());
            let mut factor = (0.3 / max_step).min(1.);
            let mut accepted = false;
            for _ in 0..8 {
                let candidate = [center[0] - factor * step[0], center[1] - factor * step[1]];
                let candidate_eta = [candidate[0] - log_theta[0], candidate[1] - log_theta[1]];
                if candidate_eta
                    .iter()
                    .all(|v| v.is_finite() && v.abs() + h < eta_bound)
                {
                    if let Some(value) = self.saem_joint(
                        subject,
                        setup,
                        theta,
                        omega,
                        candidate_eta,
                        evaluations,
                        budget,
                    )? {
                        if value <= values[1][1] {
                            center = candidate;
                            q.mean = candidate;
                            accepted = true;
                            break;
                        }
                    }
                }
                factor *= 0.5;
            }
            last = Some(q);
            if !accepted {
                return Ok(last);
            }
        }
        Ok(last)
    }

    fn saem_joint(
        self: &Arc<Self>,
        subject: &GaussianObjectiveRequest,
        setup: &Setup,
        theta: Vector,
        omega: Matrix,
        eta: Vector,
        evaluations: &mut usize,
        budget: usize,
    ) -> Result<Option<f64>, Error> {
        if *evaluations >= budget {
            return Ok(None);
        }
        if eta[..setup.dimensions]
            .iter()
            .any(|x| !x.is_finite() || x.abs() >= 0.999 * 12.)
        {
            return Ok(None);
        }
        *evaluations += 1;
        let nll = match self.saem_nll(subject, setup, theta, eta) {
            Ok(value) => value,
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::Domain | ErrorCode::Solver | ErrorCode::Invariant
                ) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        Ok(Some(nll + prior_nll_vector(eta, omega, setup.dimensions)?))
    }

    fn saem_joint_gradient(
        self: &Arc<Self>,
        subject: &GaussianObjectiveRequest,
        setup: &Setup,
        theta: Vector,
        omega: Matrix,
        eta: Vector,
        evaluations: &mut usize,
        budget: usize,
    ) -> Result<Option<Vector>, Error> {
        if *evaluations >= budget {
            return Ok(None);
        }
        let mut objective = subject.clone();
        for axis in 0..setup.dimensions {
            objective.simulation.run.parameters.insert(
                setup.names[axis].clone(),
                qspquest_core::model::Quantity {
                    value: theta[axis] * libm::exp(eta[axis]),
                    unit: setup.units[axis].clone(),
                },
            );
        }
        *evaluations += 1;
        let result = match self.gaussian_objective(&objective) {
            Ok(value) => value,
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::Domain | ErrorCode::Solver | ErrorCode::Invariant
                ) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        let (inverse, _) = precision(omega, setup.dimensions)
            .ok_or_else(|| invalid("SAEM Omega must remain positive definite"))?;
        let mut gradient = [0.; 2];
        for axis in 0..setup.dimensions {
            let score = result
                .gradient
                .iter()
                .find(|item| item.parameter == setup.names[axis])
                .ok_or_else(|| invalid("SAEM selected parameter score is missing"))?;
            gradient[axis] = score.value * theta[axis] * libm::exp(eta[axis])
                + (0..setup.dimensions)
                    .map(|other| inverse[axis][other] * eta[other])
                    .sum::<f64>();
        }
        Ok(Some(gradient))
    }

    fn saem_joint_curvature(
        self: &Arc<Self>,
        subject: &GaussianObjectiveRequest,
        setup: &Setup,
        theta: Vector,
        omega: Matrix,
        eta: Vector,
        h: f64,
        evaluations: &mut usize,
        budget: usize,
    ) -> Result<Option<(Vector, Matrix)>, Error> {
        let Some(gradient) =
            self.saem_joint_gradient(subject, setup, theta, omega, eta, evaluations, budget)?
        else {
            return Ok(None);
        };
        let mut hessian = [[0.; 2]; 2];
        for axis in 0..setup.dimensions {
            let mut plus = eta;
            let mut minus = eta;
            plus[axis] += h;
            minus[axis] -= h;
            let Some(plus) =
                self.saem_joint_gradient(subject, setup, theta, omega, plus, evaluations, budget)?
            else {
                return Ok(None);
            };
            let Some(minus) =
                self.saem_joint_gradient(subject, setup, theta, omega, minus, evaluations, budget)?
            else {
                return Ok(None);
            };
            for row in 0..setup.dimensions {
                hessian[row][axis] = (plus[row] - minus[row]) / (2. * h);
            }
        }
        if setup.dimensions == 2 {
            let cross = 0.5 * (hessian[0][1] + hessian[1][0]);
            hessian[0][1] = cross;
            hessian[1][0] = cross;
        }
        Ok(Some((gradient, hessian)))
    }

    /// Local observed-curvature Laplace approximation at the final SAEM parameters.
    /// `None` means its conditional mode or curvature could not be certified
    /// within the remaining work budget; the caller keeps a labeled surrogate.
    fn saem_marginal_ofv(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        setup: &Setup,
        theta: Vector,
        omega: Matrix,
        etas: &[Vector],
        subject_multipliers: Option<&[f64]>,
        evaluations: &mut usize,
        budget: usize,
    ) -> Result<Option<(f64, Vec<Vector>)>, Error> {
        let dimensions = setup.dimensions;
        let mut marginal = 0.;
        let mut modes = Vec::with_capacity(request.subjects.len());
        for (index, subject) in request.subjects.iter().enumerate() {
            let theta = [
                theta[0] * subject_multipliers.map_or(1., |values| values[index]),
                theta[1],
            ];
            let mut eta = etas[index];
            let mut best =
                match self.saem_joint(subject, setup, theta, omega, eta, evaluations, budget)? {
                    Some(value) => value,
                    None => return Ok(None),
                };
            let h = 0.005;
            let mut cached_curvature = None;
            for _ in 0..16 {
                if eta[..dimensions]
                    .iter()
                    .any(|value| value.abs() + h >= request.eta_bound)
                {
                    return Ok(None);
                }
                let Some((gradient, hessian)) = self.saem_joint_curvature(
                    subject,
                    setup,
                    theta,
                    omega,
                    eta,
                    h,
                    evaluations,
                    budget,
                )?
                else {
                    return Ok(None);
                };
                let determinant = if dimensions == 1 {
                    hessian[0][0]
                } else {
                    hessian[0][0] * hessian[1][1] - hessian[0][1] * hessian[1][0]
                };
                if !determinant.is_finite() || hessian[0][0] <= 0. || determinant <= 0. {
                    return Ok(None);
                }
                cached_curvature = Some((eta, gradient, hessian));
                let mut next = eta;
                if gradient[..dimensions].iter().all(|x| x.abs() < 1e-8) {
                    break;
                }
                if dimensions == 1 {
                    next[0] -= gradient[0] / determinant;
                } else {
                    next[0] -=
                        (hessian[1][1] * gradient[0] - hessian[0][1] * gradient[1]) / determinant;
                    next[1] -=
                        (-hessian[1][0] * gradient[0] + hessian[0][0] * gradient[1]) / determinant;
                }
                for axis in 0..dimensions {
                    next[axis] =
                        next[axis].clamp(-request.eta_bound + 2. * h, request.eta_bound - 2. * h);
                }
                if next == eta {
                    break;
                }
                let mut accepted = false;
                for _ in 0..12 {
                    let candidate = match self.saem_joint(
                        subject,
                        setup,
                        theta,
                        omega,
                        next,
                        evaluations,
                        budget,
                    )? {
                        Some(value) => value,
                        None => return Ok(None),
                    };
                    if candidate < best {
                        best = candidate;
                        accepted = true;
                        break;
                    }
                    for axis in 0..dimensions {
                        next[axis] = 0.5 * (eta[axis] + next[axis]);
                    }
                }
                if !accepted {
                    break;
                }
                eta = next;
            }
            let (gradient, hessian) = if let Some((_, gradient, hessian)) =
                cached_curvature.filter(|(at, _, _)| *at == eta)
            {
                (gradient, hessian)
            } else {
                let Some(curvature) = self.saem_joint_curvature(
                    subject,
                    setup,
                    theta,
                    omega,
                    eta,
                    h,
                    evaluations,
                    budget,
                )?
                else {
                    return Ok(None);
                };
                curvature
            };
            let det_hessian = if dimensions == 1 {
                hessian[0][0]
            } else {
                hessian[0][0] * hessian[1][1] - hessian[0][1] * hessian[1][0]
            };
            let mode_step = if dimensions == 1 {
                [gradient[0] / det_hessian, 0.]
            } else {
                [
                    (hessian[1][1] * gradient[0] - hessian[0][1] * gradient[1]) / det_hessian,
                    (-hessian[1][0] * gradient[0] + hessian[0][0] * gradient[1]) / det_hessian,
                ]
            };
            let newton_decrement = gradient[0] * mode_step[0] + gradient[1] * mode_step[1];
            if !det_hessian.is_finite()
                || hessian[0][0] <= 0.
                || det_hessian <= 0.
                || mode_step[..dimensions]
                    .iter()
                    .any(|x| !x.is_finite() || x.abs() > h / 10.)
                || !newton_decrement.is_finite()
                || newton_decrement > 1e-3
                || eta[..dimensions]
                    .iter()
                    .any(|x| x.abs() >= request.eta_bound - h)
            {
                return Ok(None);
            }
            // Integrating the normal eta prior cancels its (2π) factor with
            // the Laplace factor; remove only observation constants.
            let observation_constant = subject.observations.len() as f64 * LOG_2PI;
            marginal += 2. * best + libm::log(det_hessian)
                - dimensions as f64 * LOG_2PI
                - observation_constant;
            modes.push(eta);
        }
        Ok(marginal.is_finite().then_some((marginal, modes)))
    }

    fn saem_two_effect_point_refined(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        setup: &Setup,
        theta: Vector,
        omega: Matrix,
        etas: &[Vector],
        residual: Option<&ResidualFit>,
        center: f64,
        evaluations: &mut usize,
        budget: usize,
        max_iterations: usize,
    ) -> Result<RefinedSaemPoint, Error> {
        let variance = residual.map_or(1., |fit| fit.variance);
        let mut point = [
            libm::log(theta[0]),
            libm::log(theta[1]),
            omega[0][0],
            omega[1][1],
            omega[0][1] / libm::sqrt(omega[0][0] * omega[1][1]),
            variance,
        ];
        let lower = [
            libm::log(setup.lower[0]),
            libm::log(setup.lower[1]),
            setup.omega_lower[0],
            setup.omega_lower[1],
            -MAX_SAEM_CORRELATION,
            residual.map_or(variance, |fit| fit.lower_variance),
        ];
        let upper = [
            libm::log(setup.upper[0]),
            libm::log(setup.upper[1]),
            setup.omega_upper[0],
            setup.omega_upper[1],
            MAX_SAEM_CORRELATION,
            residual.map_or(variance, |fit| fit.upper_variance),
        ];
        let steps = [
            0.025,
            0.025,
            0.1 * omega[0][0].max(0.01),
            0.1 * omega[1][1].max(0.01),
            0.025,
            0.1 * variance,
        ];
        let mut value = center;
        let mut modes = etas.to_vec();
        let mut certified = false;
        let mut iterations = 0;
        for _ in 0..max_iterations.max(1) {
            let warm_modes = modes.clone();
            let mut trial_point = point;
            let mut trial_value = value;
            let mut trial_modes = modes.clone();
            let passed = certify_no_marginal_descent(
                point,
                lower,
                upper,
                steps,
                value,
                2. * request.gradient_tolerance * request.subjects.len() as f64,
                |candidate| {
                    let candidate_theta = [libm::exp(candidate[0]), libm::exp(candidate[1])];
                    let cross = candidate[4] * libm::sqrt(candidate[2] * candidate[3]);
                    let candidate_omega = [[candidate[2], cross], [cross, candidate[3]]];
                    let mut modified;
                    let subject_request = if let Some(fit) = residual {
                        modified = request.clone();
                        let sd = libm::sqrt(candidate[5]);
                        for subject in &mut modified.subjects {
                            for observation in &mut subject.observations {
                                observation.error.additive_sd = qspquest_core::model::Quantity {
                                    value: sd,
                                    unit: fit.unit.clone(),
                                };
                            }
                        }
                        &modified
                    } else {
                        request
                    };
                    let value = self.saem_marginal_ofv(
                        subject_request,
                        setup,
                        candidate_theta,
                        candidate_omega,
                        &warm_modes,
                        None,
                        evaluations,
                        budget,
                    )?;
                    if let Some((ofv, modes)) = value {
                        if ofv < trial_value {
                            trial_value = ofv;
                            trial_point = candidate;
                            trial_modes = modes;
                        }
                        Ok(Some(ofv))
                    } else {
                        Ok(None)
                    }
                },
            )?;
            if passed {
                certified = true;
                break;
            }
            if trial_point != point {
                if iterations == max_iterations {
                    break;
                }
                point = trial_point;
                value = trial_value;
                modes = trial_modes;
                iterations += 1;
            } else {
                break;
            }
        }
        let cross = point[4] * libm::sqrt(point[2] * point[3]);
        Ok(RefinedSaemPoint {
            theta: [libm::exp(point[0]), libm::exp(point[1])],
            omega: [[point[2], cross], [cross, point[3]]],
            residual_variance: point[5],
            marginal: (value, modes),
            certified,
            iterations,
        })
    }

    fn saem_one_effect_uncertainty(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        setup: &Setup,
        theta: f64,
        omega: f64,
        sampled_etas: &[Vector],
        center: f64,
        evaluations: &mut usize,
        budget: usize,
    ) -> Result<PopulationFitUncertainty, Error> {
        let free_omega = setup.omega_lower[0] < setup.omega_upper[0];
        let dimensions = if free_omega { 2 } else { 1 };
        let spans = [
            setup.upper[0] - setup.lower[0],
            setup.omega_upper[0] - setup.omega_lower[0],
        ];
        let coordinates = [
            (theta - setup.lower[0]) / spans[0],
            if free_omega {
                (omega - setup.omega_lower[0]) / spans[1]
            } else {
                0.
            },
        ];
        let h = 0.01;
        if coordinates[..dimensions]
            .iter()
            .any(|value| !value.is_finite() || *value <= 2. * h || *value >= 1. - 2. * h)
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "SAEM observed covariance requires interior fixed-effect and variance estimates",
            ));
        }
        let mut evaluate = |point: [f64; 2]| -> Result<f64, Error> {
            let candidate_theta = setup.lower[0] + point[0] * spans[0];
            let candidate_omega = if free_omega {
                setup.omega_lower[0] + point[1] * spans[1]
            } else {
                omega
            };
            self.saem_marginal_ofv(
                request,
                setup,
                [candidate_theta, 0.],
                [[candidate_omega, 0.], [0., 0.]],
                sampled_etas,
                None,
                evaluations,
                budget,
            )?
            .map(|(ofv, _)| ofv)
            .ok_or_else(|| {
                observed_covariance_failure(
                    *evaluations,
                    budget,
                    "SAEM observed covariance could not certify perturbed marginal objectives",
                )
            })
        };
        let mut diagonal = [0.; 2];
        for axis in 0..dimensions {
            let mut plus = coordinates;
            let mut minus = coordinates;
            plus[axis] += h;
            minus[axis] -= h;
            diagonal[axis] = (evaluate(plus)? - 2. * center + evaluate(minus)?) / (h * h);
        }
        let cross = if free_omega {
            let mut point = coordinates;
            point[0] += h;
            point[1] += h;
            let pp = evaluate(point)?;
            point[1] -= 2. * h;
            let pm = evaluate(point)?;
            point[0] -= 2. * h;
            point[1] += 2. * h;
            let mp = evaluate(point)?;
            point[1] -= 2. * h;
            let mm = evaluate(point)?;
            (pp - pm - mp + mm) / (4. * h * h)
        } else {
            0.
        };
        let determinant = if free_omega {
            diagonal[0] * diagonal[1] - cross * cross
        } else {
            diagonal[0]
        };
        if !diagonal[0].is_finite()
            || diagonal[0] <= 0.
            || !determinant.is_finite()
            || determinant <= 0.
            || (free_omega
                && (!diagonal[1].is_finite()
                    || diagonal[1] <= 0.
                    || determinant <= 1e-8 * diagonal[0] * diagonal[1]))
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "SAEM observed information is singular or not positive definite",
            ));
        }
        let mut parameters = vec![PopulationUncertaintyParameter {
            name: setup.names[0].clone(),
            unit: setup.units[0].clone(),
        }];
        let covariance = if free_omega {
            parameters.push(PopulationUncertaintyParameter {
                name: format!("omega:{}", setup.names[0]),
                unit: "1".into(),
            });
            vec![
                vec![
                    2. * diagonal[1] / determinant * spans[0] * spans[0],
                    -2. * cross / determinant * spans[0] * spans[1],
                ],
                vec![
                    -2. * cross / determinant * spans[0] * spans[1],
                    2. * diagonal[0] / determinant * spans[1] * spans[1],
                ],
            ]
        } else {
            vec![vec![2. * spans[0] * spans[0] / diagonal[0]]]
        };
        let standard_errors = (0..dimensions)
            .map(|axis| libm::sqrt(covariance[axis][axis]))
            .collect::<Vec<_>>();
        if standard_errors
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.)
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "SAEM observed covariance is not finite and positive",
            ));
        }
        Ok(PopulationFitUncertainty {
            method: PopulationUncertaintyMethod::SaemMarginalObservedHessian,
            parameters,
            covariance,
            standard_errors,
        })
    }

    fn saem_two_effect_uncertainty(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        setup: &Setup,
        theta: Vector,
        omega: Matrix,
        sampled_etas: &[Vector],
        _center: f64,
        evaluations: &mut usize,
        budget: usize,
    ) -> Result<PopulationFitUncertainty, Error> {
        // Curvature differentiates conditional sensitivity scores and then
        // the marginal objective. Tighten the integration error ceiling for
        // these nested derivatives without changing data or the work budget.
        let mut curvature_request = request.clone();
        for subject in &mut curvature_request.subjects {
            let regimen = &mut subject.simulation.run.regimen;
            let factor = (1e-10 / regimen.rtol).min(1.);
            regimen.rtol *= factor;
            if let Some(atol) = &mut regimen.atol {
                *atol *= factor;
            }
            if let Some(tolerances) = &mut regimen.absolute_tolerances {
                for tolerance in tolerances.values_mut() {
                    tolerance.value *= factor;
                }
            }
        }
        // Linear natural-coordinate perturbations scale with the fitted
        // values. Bounds and the smallest covariance eigenvalue limit the
        // initial stencil; step halving certifies the resulting information.
        let mut axes = vec![
            (0_usize, theta[0].abs().max(0.01)),
            (1_usize, theta[1].abs().max(0.01)),
        ];
        for axis in 0..2 {
            if setup.omega_lower[axis] < setup.omega_upper[axis] {
                axes.push((axis + 2, omega[axis][axis].abs().max(0.01)));
            }
        }
        axes.push((4, libm::sqrt(omega[0][0] * omega[1][1])));
        let dimensions = axes.len();
        let mut h = 0.01_f64;
        for (coordinate, scale) in &axes {
            let distance = match coordinate {
                0 | 1 => (theta[*coordinate] - setup.lower[*coordinate])
                    .min(setup.upper[*coordinate] - theta[*coordinate]),
                2 | 3 => (omega[*coordinate - 2][*coordinate - 2]
                    - setup.omega_lower[*coordinate - 2])
                    .min(
                        setup.omega_upper[*coordinate - 2]
                            - omega[*coordinate - 2][*coordinate - 2],
                    ),
                _ => f64::INFINITY,
            };
            h = h.min(distance / (4. * scale));
        }
        h = h.min(correlation_stencil_limit(omega, &axes)?);
        let mut evaluate = |offsets: &[f64; 5]| -> Result<f64, Error> {
            let mut candidate_theta = theta;
            let mut candidate_omega = omega;
            for (axis, (coordinate, span)) in axes.iter().enumerate() {
                let shift = offsets[axis] * span;
                match coordinate {
                    0 | 1 => candidate_theta[*coordinate] += shift,
                    2 | 3 => candidate_omega[*coordinate - 2][*coordinate - 2] += shift,
                    4 => {
                        candidate_omega[0][1] += shift;
                        candidate_omega[1][0] += shift;
                    }
                    _ => unreachable!(),
                }
            }
            if (0..2).any(|axis| {
                candidate_theta[axis] <= setup.lower[axis]
                    || candidate_theta[axis] >= setup.upper[axis]
                    || (setup.omega_lower[axis] < setup.omega_upper[axis]
                        && (candidate_omega[axis][axis] <= setup.omega_lower[axis]
                            || candidate_omega[axis][axis] >= setup.omega_upper[axis]))
            }) || precision(candidate_omega, 2).is_none()
                || candidate_omega[0][1].abs()
                    >= MAX_SAEM_CORRELATION
                        * libm::sqrt(candidate_omega[0][0] * candidate_omega[1][1])
            {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "SAEM observed covariance requires an interior estimate within the permitted correlation limit",
                ));
            }
            self.saem_marginal_ofv(
                &curvature_request,
                setup,
                candidate_theta,
                candidate_omega,
                sampled_etas,
                None,
                evaluations,
                budget,
            )?
            .map(|(ofv, _)| ofv)
            .ok_or_else(|| {
                observed_covariance_failure(
                    *evaluations,
                    budget,
                    "SAEM observed covariance could not certify perturbed marginal objectives",
                )
            })
        };
        let recentered = evaluate(&[0.; 5])?;
        let scaled = certified_covariance(dimensions, recentered, h, &mut evaluate)?;
        let covariance = (0..dimensions)
            .map(|row| {
                (0..dimensions)
                    .map(|col| scaled[row][col] * axes[row].1 * axes[col].1)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let standard_errors = (0..dimensions)
            .map(|axis| libm::sqrt(covariance[axis][axis]))
            .collect::<Vec<_>>();
        if standard_errors.iter().any(|x| !x.is_finite() || *x <= 0.) {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "SAEM observed covariance is not finite and positive",
            ));
        }
        let parameters = axes
            .iter()
            .map(|(coordinate, _)| match coordinate {
                0 | 1 => PopulationUncertaintyParameter {
                    name: setup.names[*coordinate].clone(),
                    unit: setup.units[*coordinate].clone(),
                },
                2 | 3 => PopulationUncertaintyParameter {
                    name: format!("omega:{}", setup.names[*coordinate - 2]),
                    unit: "1".into(),
                },
                4 => PopulationUncertaintyParameter {
                    name: format!("omega:{}:{}", setup.names[0], setup.names[1]),
                    unit: "1".into(),
                },
                _ => unreachable!(),
            })
            .collect();
        Ok(PopulationFitUncertainty {
            method: PopulationUncertaintyMethod::SaemMarginalObservedHessian,
            parameters,
            covariance,
            standard_errors,
        })
    }
}

impl CompiledSensitivityDocument {
    /// Stochastic-approximation EM with a Metropolis conditional simulation.
    /// Estimates one or two positive lognormal effects with fixed Gaussian
    /// error or a common fitted additive Gaussian SD.
    pub fn fit_population_saem(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
    ) -> Result<PopulationFitResult, Error> {
        match self.prepare_population_saem(request)? {
            PreparedSaem::One(prepared) => self.fit_population_saem_1d(request, prepared),
            PreparedSaem::Two(prepared) => self.fit_population_saem_2d(request, prepared),
        }
    }
    fn prepare_population_saem(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
    ) -> Result<PreparedSaem, Error> {
        self.prepare_population_saem_observed(request, None)
    }
    fn prepare_population_saem_observed(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        plan: Option<&mut super::fit_planning::FitPlanBuilder>,
    ) -> Result<PreparedSaem, Error> {
        if request.uncertainty
            && (request.additive_error_fit.is_some() || request.covariate_effect.is_some())
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "SAEM observed covariance requires fixed residual error and no fitted covariate",
            ));
        }
        if request.covariate_effect.is_some() && request.random_effects.len() != 1 {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "SAEM fitted covariates currently require one random effect",
            ));
        }
        if request.random_effects.len() == 2 {
            return self
                .prepare_population_saem_2d(request, plan)
                .map(PreparedSaem::Two);
        }
        self.prepare_population_saem_1d(request, plan)
            .map(PreparedSaem::One)
    }
    fn prepare_population_saem_1d(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        mut plan: Option<&mut super::fit_planning::FitPlanBuilder>,
    ) -> Result<PreparedSaemOneDim, Error> {
        let cfg = self.validate_population_1d(request)?;
        let burn_in = burn_in_iterations(request)?;
        let residual = validate_residual_fit(self, request)?;
        let subjects = request.subjects.clone();
        let setup = Setup {
            dimensions: 1,
            names: [cfg.parameter.clone(), String::new()],
            units: [cfg.unit.clone(), String::new()],
            start: [cfg.start, 0.],
            lower: [cfg.lower, 0.],
            upper: [cfg.upper, 0.],
            omega: [[cfg.omega, 0.], [0., 0.]],
            omega_lower: [cfg.omega_lower, 0.],
            omega_upper: [cfg.omega_upper, 0.],
        };
        let count = request.subjects.len();
        if count < 2 {
            return Err(invalid("SAEM requires at least two subjects"));
        }
        if request.max_iterations < 8 || request.max_evaluations < count.saturating_mul(4) {
            return Err(invalid(
                "SAEM requires at least eight iterations and four evaluations per subject",
            ));
        }
        if request.covariate_effect.is_some() && residual.is_some() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "SAEM fitted covariates currently require fixed residual error",
            ));
        }
        let covariates = request
            .covariate_effect
            .as_ref()
            .map(|primary| {
                std::iter::once(primary)
                    .chain(primary.additional.iter())
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if covariates.len() > 4 || count <= covariates.len() + 1 {
            return Err(invalid("SAEM needs at least two more subjects than bounded covariate effects (maximum four)"));
        }
        let mut names = std::collections::BTreeSet::new();
        let mut x = vec![vec![0.; covariates.len()]; count];
        let mut beta = Vec::with_capacity(covariates.len());
        for (column, effect) in covariates.iter().enumerate() {
            if effect.parameter != cfg.parameter
                || effect.covariate.is_empty()
                || !names.insert(&effect.covariate)
                || (column > 0 && !effect.additional.is_empty())
                || effect.subject_values.len() != count
                || !effect.reference.is_finite()
                || !effect.initial.is_finite()
                || !effect.bounds.iter().all(|value| value.is_finite())
                || effect.bounds[0] >= effect.bounds[1]
                || effect.initial < effect.bounds[0]
                || effect.initial > effect.bounds[1]
                || !effect.subject_values.iter().all(|value| value.is_finite())
            {
                return Err(invalid("SAEM covariate effects require distinct names, finite subject values and valid coefficient bounds"));
            }
            for (row, value) in effect.subject_values.iter().enumerate() {
                x[row][column] = value - effect.reference;
                if x[row][column].abs() * effect.bounds[0].abs().max(effect.bounds[1].abs()) > 8. {
                    return Err(invalid(
                        "SAEM covariate effect exceeds the bounded log-parameter range",
                    ));
                }
            }
            beta.push(effect.initial);
        }
        let columns = covariates.len();
        let mean_x = (0..columns)
            .map(|j| x.iter().map(|row| row[j]).sum::<f64>() / count as f64)
            .collect::<Vec<_>>();
        let second_x = (0..columns)
            .map(|j| {
                (0..columns)
                    .map(|k| x.iter().map(|row| row[j] * row[k]).sum::<f64>() / count as f64)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let covariance_x = (0..columns)
            .map(|j| {
                (0..columns)
                    .map(|k| second_x[j][k] - mean_x[j] * mean_x[k])
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        if columns > 0 {
            solve_covariate_regression(&covariance_x, &vec![0.; columns])?;
        }
        for (index, subject) in subjects.iter().enumerate() {
            let mut initial = subject.clone();
            let value = cfg.start * libm::exp(dot(&beta, &x[index]));
            initial.simulation.run.parameters.insert(
                cfg.parameter.clone(),
                Quantity {
                    value,
                    unit: cfg.unit.clone(),
                },
            );
            let prepared = self
                .prepare_gaussian_primal(&initial)
                .map_err(|mut error| {
                    error.expression = Some(format!(
                        "subjects[{index}]{}",
                        error
                            .expression
                            .map(|field| format!(".{field}"))
                            .unwrap_or_default()
                    ));
                    error
                })?;
            if let Some(plan) = plan.as_deref_mut() {
                plan.primal(self, &initial, &prepared)?;
            }
        }
        Ok(PreparedSaemOneDim {
            cfg,
            setup,
            burn_in,
            residual,
            subjects,
            covariates,
            x,
            beta,
            mean_x,
            second_x,
            covariance_x,
        })
    }
    fn fit_population_saem_1d(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        prepared: PreparedSaemOneDim,
    ) -> Result<PopulationFitResult, Error> {
        let PreparedSaemOneDim {
            cfg,
            setup,
            burn_in,
            mut residual,
            mut subjects,
            covariates,
            x,
            mut beta,
            mean_x,
            second_x,
            covariance_x,
        } = prepared;
        let count = subjects.len();
        let columns = covariates.len();
        let mut stream = Stream::new(request.seed);
        let mut theta = cfg.start;
        let mut omega = cfg.omega;
        let mut log_theta = libm::log(theta);
        let mut phi = x
            .iter()
            .map(|row| log_theta + dot(&beta, row))
            .collect::<Vec<_>>();
        let mut nll = Vec::with_capacity(count);
        let mut residual_sse = Vec::with_capacity(count);
        let mut prediction_cache = Vec::with_capacity(count);
        let mut evaluations = 0_usize;
        for (index, subject) in subjects.iter().enumerate() {
            if evaluations >= request.max_evaluations {
                return Err(invalid(
                    "SAEM evaluation budget is smaller than subject count",
                ));
            }
            let subject_theta = theta * libm::exp(dot(&beta, &x[index]));
            let (result, predictions) = self.population_subject_primal_nll_and_predictions(
                subject,
                &cfg.parameter,
                subject_theta,
                &cfg.unit,
                0.0,
            )?;
            evaluations += 1;
            if let Some(fit) = &residual {
                residual_sse.push(residual_moments(subject, &predictions, fit)?.1);
            }
            prediction_cache.push(predictions);
            nll.push(result);
        }

        // The complete-data family is normal in phi = log(individual parameter).
        // SA updates these sufficient statistics; the M-step is their exact
        // constrained normal-family maximizer. A fitted additive error uses
        // a separate stochastic average of the complete-data residual SSE.
        let mut mean_phi = phi.iter().sum::<f64>() / count as f64;
        let mut second_phi =
            phi.iter().map(|value| value * value).sum::<f64>() / count as f64 + omega;
        let mut mean_xphi = (0..columns)
            .map(|j| {
                log_theta * mean_x[j] + (0..columns).map(|k| beta[k] * second_x[j][k]).sum::<f64>()
            })
            .collect::<Vec<_>>();
        let mut iterations = 0_usize;
        let mut stable_iterations = 0_usize;
        let mut evaluated_proposals = 0_usize;
        let mut accepted_proposals = 0_usize;
        let mut final_step_size = None;
        let mut final_relative_change = None;
        let mut recent_relative_changes = Vec::with_capacity(20);
        let mut post_burn_history = Vec::with_capacity(50);
        let mut status = FitStatus::IterationLimit;
        for iteration in 0..request.max_iterations {
            let mut moved = 0_usize;
            for (index, subject) in subjects.iter().enumerate() {
                for _ in 0..3 {
                    let proposal = phi[index] + 0.8 * libm::sqrt(omega) * stream.normal();
                    let subject_log_theta = log_theta + dot(&beta, &x[index]);
                    let proposal_eta = proposal - subject_log_theta;
                    if !proposal.is_finite() || proposal_eta.abs() > request.eta_bound {
                        continue;
                    }
                    let individual = libm::exp(proposal);
                    if !individual.is_finite() || individual <= 0.0 {
                        continue;
                    }
                    // Reserve the final likelihood plus population and
                    // individual prediction runs for every subject.
                    if evaluations >= request.max_evaluations - 3 * count {
                        status = FitStatus::EvaluationLimit;
                        break;
                    }
                    let proposal_result = self.population_subject_primal_nll_and_predictions(
                        subject,
                        &cfg.parameter,
                        theta * libm::exp(dot(&beta, &x[index])),
                        &cfg.unit,
                        proposal_eta,
                    );
                    evaluations += 1;
                    evaluated_proposals += 1;
                    let (proposal_nll, proposal_predictions) = match proposal_result {
                        Ok(value) => value,
                        Err(error)
                            if matches!(
                                error.code,
                                ErrorCode::Domain | ErrorCode::Solver | ErrorCode::Invariant
                            ) =>
                        {
                            continue
                        }
                        Err(error) => return Err(error),
                    };
                    let proposal_sse = if let Some(fit) = &residual {
                        Some(residual_moments(subject, &proposal_predictions, fit)?.1)
                    } else {
                        None
                    };
                    let current = nll[index] + prior_nll(phi[index], subject_log_theta, omega);
                    let candidate = proposal_nll + prior_nll(proposal, subject_log_theta, omega);
                    if libm::log(stream.uniform()) < current - candidate {
                        phi[index] = proposal;
                        nll[index] = proposal_nll;
                        if let Some(sse) = proposal_sse {
                            residual_sse[index] = sse;
                        }
                        prediction_cache[index] = proposal_predictions;
                        moved += 1;
                        accepted_proposals += 1;
                    }
                }
                if status == FitStatus::EvaluationLimit {
                    break;
                }
            }
            if status == FitStatus::EvaluationLimit {
                break;
            }
            let sample_mean = phi.iter().sum::<f64>() / count as f64;
            let sample_second = phi.iter().map(|p| p * p).sum::<f64>() / count as f64;
            let sample_xphi = (0..columns)
                .map(|j| phi.iter().zip(&x).map(|(p, row)| p * row[j]).sum::<f64>() / count as f64)
                .collect::<Vec<_>>();
            let gamma = if iteration < burn_in {
                1.0
            } else {
                1.0 / (iteration - burn_in + 1) as f64
            };
            mean_phi += gamma * (sample_mean - mean_phi);
            second_phi += gamma * (sample_second - second_phi);
            for j in 0..columns {
                mean_xphi[j] += gamma * (sample_xphi[j] - mean_xphi[j]);
            }
            let proposed_beta = if columns > 0 {
                let rhs = (0..columns)
                    .map(|j| mean_xphi[j] - mean_x[j] * mean_phi)
                    .collect::<Vec<_>>();
                let bounds = covariates
                    .iter()
                    .map(|effect| effect.bounds)
                    .collect::<Vec<_>>();
                bounded_covariate_regression(&covariance_x, &rhs, &bounds)?
            } else {
                Vec::new()
            };
            let feasible_box = |coefficients: &[f64]| {
                let lower = phi
                    .iter()
                    .zip(&x)
                    .map(|(p, row)| p - dot(coefficients, row) - request.eta_bound)
                    .fold(libm::log(cfg.lower), f64::max);
                let upper = phi
                    .iter()
                    .zip(&x)
                    .map(|(p, row)| p - dot(coefficients, row) + request.eta_bound)
                    .fold(libm::log(cfg.upper), f64::min);
                (lower, upper)
            };
            let mut next_beta = proposed_beta;
            let (mut log_lower, mut log_upper) = feasible_box(&next_beta);
            for _ in 0..24 {
                if log_lower <= log_upper {
                    break;
                }
                for j in 0..columns {
                    next_beta[j] = 0.5 * (next_beta[j] + beta[j]);
                }
                (log_lower, log_upper) = feasible_box(&next_beta);
            }
            if log_lower > log_upper {
                return Err(Error::new(
                    ErrorCode::Domain,
                    "SAEM eta constraints have no feasible fixed effect",
                ));
            }
            let next_log_theta = (mean_phi - dot(&next_beta, &mean_x)).clamp(log_lower, log_upper);
            let next_theta = libm::exp(next_log_theta);
            let beta_xphi = dot(&next_beta, &mean_xphi);
            let beta_mean_x = dot(&next_beta, &mean_x);
            let beta_second_x = (0..columns)
                .map(|j| {
                    (0..columns)
                        .map(|k| next_beta[j] * next_beta[k] * second_x[j][k])
                        .sum::<f64>()
                })
                .sum::<f64>();
            let next_omega = (second_phi - 2. * next_log_theta * mean_phi - 2. * beta_xphi
                + next_log_theta * next_log_theta
                + 2. * next_log_theta * beta_mean_x
                + beta_second_x)
                .clamp(cfg.omega_lower, cfg.omega_upper);
            if !next_theta.is_finite() || !next_omega.is_finite() || next_omega <= 0.0 {
                return Err(Error::new(
                    ErrorCode::Domain,
                    "SAEM M-step produced invalid population parameters",
                ));
            }
            let residual_change = if let Some(fit) = &mut residual {
                let previous = fit.variance;
                let sample = residual_sse.iter().sum::<f64>() / fit.observations as f64;
                // A single pre-equilibration draw must not overwrite the
                // residual variance and feed that noise into the next E-step.
                let residual_gain = gamma.min(0.2);
                fit.variance = (previous + residual_gain * (sample - previous))
                    .clamp(fit.lower_variance, fit.upper_variance);
                if !fit.variance.is_finite() {
                    return Err(Error::new(
                        ErrorCode::Domain,
                        "SAEM residual M-step overflow",
                    ));
                }
                set_residual_sd(&mut subjects, fit);
                for (index, subject) in subjects.iter().enumerate() {
                    let (likelihood, moment) =
                        residual_moments(subject, &prediction_cache[index], fit)?;
                    nll[index] = likelihood;
                    residual_sse[index] = moment;
                }
                ((fit.variance - previous) / previous).abs()
            } else {
                0.
            };
            let relative_change = ((next_theta - theta) / theta)
                .abs()
                .max(((next_omega - omega) / omega).abs())
                .max(
                    next_beta
                        .iter()
                        .zip(&beta)
                        .map(|(next, current)| (next - current).abs() / (1. + current.abs()))
                        .fold(0., f64::max),
                )
                .max(residual_change);
            final_step_size = Some(gamma);
            final_relative_change = Some(relative_change);
            if iteration >= burn_in {
                if recent_relative_changes.len() == 20 {
                    recent_relative_changes.remove(0);
                }
                recent_relative_changes.push(relative_change);
                if post_burn_history.len() == 50 {
                    post_burn_history.remove(0);
                }
                let mut history = vec![
                    next_log_theta,
                    libm::log(next_omega),
                    residual
                        .as_ref()
                        .map(|fit| libm::log(fit.variance))
                        .unwrap_or(0.),
                ];
                history.extend(&next_beta);
                post_burn_history.push(history);
            }
            theta = next_theta;
            omega = next_omega;
            log_theta = libm::log(theta);
            beta = next_beta;
            iterations = iteration + 1;
            let window_stable = if post_burn_history.len() == 50 && moved > 0 {
                (0..3 + columns).all(|axis| {
                    let history = post_burn_history
                        .iter()
                        .map(|row| row[axis])
                        .collect::<Vec<_>>();
                    let separated = history[..15]
                        .iter()
                        .chain(&history[35..])
                        .copied()
                        .collect::<Vec<_>>();
                    windowed_drift(&history[25..], request.gradient_tolerance).2
                        && windowed_drift(&separated, request.gradient_tolerance).2
                })
            } else {
                false
            };
            stable_iterations = if window_stable {
                stable_iterations + 1
            } else {
                0
            };
            if stable_iterations >= 4 {
                status = FitStatus::Converged;
                break;
            }
        }

        // A converged marginal calculation also supplies conditional eta
        // modes for EBE and IPRED. At a tight work budget the sampled etas
        // remain available with an explicitly labeled complete-data surrogate.
        let final_request = residual.as_ref().map(|_| {
            let mut final_request = request.clone();
            final_request.subjects = subjects.clone();
            final_request
        });
        let fitted_request = final_request.as_ref().unwrap_or(request);
        let sampled_etas: Vec<Vector> = phi
            .iter()
            .zip(&x)
            .map(|(p, row)| [p - log_theta - dot(&beta, row), 0.])
            .collect();
        let multipliers = x
            .iter()
            .map(|row| libm::exp(dot(&beta, row)))
            .collect::<Vec<_>>();
        let marginal = self.saem_marginal_ofv(
            fitted_request,
            &setup,
            [theta, 0.],
            [[omega, 0.], [0., 0.]],
            &sampled_etas,
            (!covariates.is_empty()).then_some(multipliers.as_slice()),
            &mut evaluations,
            request.max_evaluations - 3 * count,
        )?;
        if marginal.is_none() && status == FitStatus::Converged {
            status = if evaluations >= request.max_evaluations - 3 * count {
                FitStatus::EvaluationLimit
            } else {
                FitStatus::LineSearchFailed
            };
        }
        let uncertainty = if request.uncertainty {
            if status != FitStatus::Converged {
                return Err(observed_covariance_failure(
                    evaluations,
                    request.max_evaluations - 3 * count,
                    "SAEM observed covariance requires a converged marginal fit",
                ));
            }
            Some(self.saem_one_effect_uncertainty(
                fitted_request,
                &setup,
                theta,
                omega,
                &sampled_etas,
                marginal.as_ref().expect("converged marginal fit").0,
                &mut evaluations,
                request.max_evaluations - 3 * count,
            )?)
        } else {
            None
        };
        let final_etas = marginal
            .as_ref()
            .map(|(_, modes)| modes)
            .unwrap_or(&sampled_etas);
        let mut subject_results = Vec::with_capacity(count);
        let mut fitted_observations = Vec::new();
        let mut objective = 0.0;
        for (index, subject) in fitted_request.subjects.iter().enumerate() {
            let eta = final_etas[index][0];
            let subject_theta = theta * multipliers[index];
            let result = self.population_subject_primal_nll(
                subject,
                &cfg.parameter,
                subject_theta,
                &cfg.unit,
                eta,
            )?;
            evaluations += 1;
            let likelihood = result;
            objective += likelihood + prior_nll_vector([eta, 0.], [[omega, 0.], [0., 0.]], 1)?;
            subject_results.push(PopulationSubjectEstimate {
                eta: vec![eta],
                negative_log_likelihood: likelihood,
            });
            fitted_observations.extend(self.saem_predictions(
                index,
                subject,
                &setup,
                [subject_theta, 0.],
                [eta, 0.],
            )?);
            evaluations += 2;
        }
        if !objective.is_finite() {
            return Err(Error::new(
                ErrorCode::Domain,
                "SAEM complete-data surrogate overflow",
            ));
        }
        let mut fixed_effects = std::collections::BTreeMap::new();
        fixed_effects.insert(
            cfg.parameter,
            qspquest_core::model::Quantity {
                value: theta,
                unit: cfg.unit,
            },
        );
        let recent_window = recent_relative_changes.len();
        recent_relative_changes.sort_by(f64::total_cmp);
        let recent_relative_change_median = if recent_window == 0 {
            None
        } else if recent_window % 2 == 0 {
            Some(
                (recent_relative_changes[recent_window / 2 - 1]
                    + recent_relative_changes[recent_window / 2])
                    / 2.0,
            )
        } else {
            Some(recent_relative_changes[recent_window / 2])
        };
        Ok(PopulationFitResult {
            status,
            fixed_effects,
            omega: vec![vec![omega]],
            covariate_effect: covariates
                .first()
                .map(|effect| PopulationCovariateEffectEstimate {
                    parameter: effect.parameter.clone(),
                    covariate: effect.covariate.clone(),
                    reference: effect.reference,
                    coefficient: beta[0],
                    additional: covariates
                        .iter()
                        .enumerate()
                        .skip(1)
                        .map(|(index, other)| PopulationCovariateEffectEstimate {
                            parameter: other.parameter.clone(),
                            covariate: other.covariate.clone(),
                            reference: other.reference,
                            coefficient: beta[index],
                            additional: Vec::new(),
                        })
                        .collect(),
                }),
            subjects: subject_results,
            fitted_observations: Some(fitted_observations),
            uncertainty,
            estimated_residual_error: residual.map(|fit| PopulationResidualErrorEstimate {
                output: fit.output,
                additive_sd: qspquest_core::model::Quantity {
                    value: libm::sqrt(fit.variance),
                    unit: fit.unit,
                },
            }),
            saem_diagnostics: Some(PopulationSaemDiagnostics {
                burn_in_iterations: burn_in,
                final_step_size,
                final_relative_change,
                recent_relative_change_median,
                recent_window,
                evaluated_proposals,
                accepted_proposals,
                consecutive_stable_iterations: stable_iterations,
                post_burn_acceptance_by_axis: None,
                post_burn_min_subject_acceptance_by_axis: None,
                post_burn_max_subject_stall_iterations: None,
                post_burn_max_lag_one_correlation_by_axis: None,
            }),
            objective_kind: if marginal.is_some() {
                PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue
            } else {
                PopulationObjectiveKind::SaemCompleteDataSurrogate
            },
            objective: marginal.map(|(value, _)| value).unwrap_or(objective),
            evaluations,
            iterations,
            fit_request_hash: canonical_hash(&serde_json::json!(request))?,
        })
    }

    fn prepare_population_saem_2d(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        mut plan: Option<&mut super::fit_planning::FitPlanBuilder>,
    ) -> Result<PreparedSaemTwoDim, Error> {
        let setup = validate_two_effects(self, request)?;
        let burn_in = burn_in_iterations(request)?;
        let residual = validate_residual_fit(self, request)?;
        let subjects = request.subjects.clone();
        let count = request.subjects.len();
        if request.max_evaluations < count.saturating_mul(4) {
            return Err(invalid(
                "SAEM requires four evaluations per subject for complete results",
            ));
        }
        for (index, subject) in subjects.iter().enumerate() {
            let prepared = self.prepare_gaussian_primal(subject).map_err(|mut error| {
                error.expression = Some(format!(
                    "subjects[{index}]{}",
                    error
                        .expression
                        .map(|field| format!(".{field}"))
                        .unwrap_or_default()
                ));
                error
            })?;
            if let Some(plan) = plan.as_deref_mut() {
                plan.primal(self, subject, &prepared)?;
            }
        }
        Ok(PreparedSaemTwoDim {
            setup,
            burn_in,
            residual,
            subjects,
        })
    }
    fn fit_population_saem_2d(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        prepared: PreparedSaemTwoDim,
    ) -> Result<PopulationFitResult, Error> {
        let PreparedSaemTwoDim {
            setup,
            burn_in,
            mut residual,
            mut subjects,
        } = prepared;
        let count = subjects.len();
        let mut stream = Stream::new(request.seed);
        let mut theta = setup.start;
        let mut omega = setup.omega;
        let mut log_theta = [libm::log(theta[0]), libm::log(theta[1])];
        let mut phi = vec![log_theta; count];
        let mut nll = Vec::with_capacity(count);
        let mut residual_sse = Vec::with_capacity(count);
        let mut prediction_cache = Vec::with_capacity(count);
        let mut evaluations = 0;
        for subject in &mut subjects {
            let (likelihood, predictions) =
                self.saem_nll_in_place_with_predictions(subject, &setup, theta, [0.; 2])?;
            if let Some(fit) = &residual {
                residual_sse.push(residual_moments(subject, &predictions, fit)?.1);
            }
            prediction_cache.push(predictions);
            nll.push(likelihood);
            evaluations += 1;
        }
        let mut mean_phi = log_theta;
        let mut second_phi = [[0.; 2]; 2];
        for a in 0..2 {
            for b in 0..2 {
                second_phi[a][b] = log_theta[a] * log_theta[b] + omega[a][b];
            }
        }
        let mut iterations = 0;
        let mut status = FitStatus::IterationLimit;
        let mut evaluated_proposals = 0;
        let mut accepted_proposals = 0;
        let mut final_step_size = None;
        let mut final_relative_change = None;
        let mut recent_relative_changes = Vec::with_capacity(20);
        let mut post_burn_history: Vec<[f64; 6]> = Vec::with_capacity(50);
        let mut stable_iterations = 0;
        let mut proposal_scale = [0.8_f64; 2];
        let mut post_burn_proposed = [0_usize; 2];
        let mut post_burn_accepted = [0_usize; 2];
        let mut subject_proposed = vec![[0_usize; 2]; count];
        let mut subject_accepted = vec![[0_usize; 2]; count];
        let mut subject_stall = vec![0_usize; count];
        let mut max_subject_stall = 0_usize;
        let mut moved_by_subject = vec![false; count];
        let mut previous_phi = phi.clone();
        let mut lag_one = vec![[LagOne::default(); 2]; count];
        let mut pilot_lag_one = vec![[LagOne::default(); 2]; count];
        let mut extra_sweeps = false;
        let mut joint_proposals: Vec<Option<JointProposal>> = vec![None; count];
        for iteration in 0..request.max_iterations {
            let mut moved = 0;
            moved_by_subject.fill(false);
            let mut proposed_by_axis = [0_usize; 2];
            let mut accepted_by_axis = [0_usize; 2];
            for (index, subject) in subjects.iter_mut().enumerate() {
                if iteration < 4 || iteration % 20 == 0 {
                    let center = joint_proposals[index].map_or(log_theta, |q| q.mean);
                    joint_proposals[index] = self.saem_curvature_proposal(
                        subject,
                        &setup,
                        theta,
                        omega,
                        center,
                        request.eta_bound,
                        &mut evaluations,
                        request.max_evaluations - 3 * count,
                    )?;
                }
                // If a burn-in pilot finds persistent subject chains, complete
                // more coordinate sweeps before recording each SA sample.
                let proposals = if residual.is_some() {
                    5
                } else if extra_sweeps {
                    10
                } else {
                    3
                };
                for step in 0..proposals {
                    let mut proposal = phi[index];
                    // A joint independence proposal uses its full Hastings
                    // correction; remaining coordinate moves are symmetric.
                    let axis = if residual.is_some() || extra_sweeps {
                        (iteration + step) % 2
                    } else {
                        (stream.next_u64() % 2) as usize
                    };
                    let joint = if step == 0 {
                        joint_proposals[index]
                    } else {
                        None
                    };
                    if let Some(q) = joint {
                        proposal = q.draw(&mut stream);
                    } else {
                        proposal[axis] +=
                            proposal_scale[axis] * libm::sqrt(omega[axis][axis]) * stream.normal();
                    }
                    for a in 0..2 {
                        if joint.is_some() || a == axis {
                            proposed_by_axis[a] += 1;
                            if iteration >= burn_in {
                                post_burn_proposed[a] += 1;
                                subject_proposed[index][a] += 1;
                            }
                        }
                    }
                    let candidate_eta = [proposal[0] - log_theta[0], proposal[1] - log_theta[1]];
                    if candidate_eta
                        .iter()
                        .any(|x| !x.is_finite() || x.abs() > request.eta_bound)
                    {
                        continue;
                    }
                    if evaluations >= request.max_evaluations - 3 * count {
                        status = FitStatus::EvaluationLimit;
                        break;
                    }
                    evaluations += 1;
                    evaluated_proposals += 1;
                    let (candidate_nll, candidate_predictions) = match self
                        .saem_nll_in_place_with_predictions(subject, &setup, theta, candidate_eta)
                    {
                        Ok(value) => value,
                        Err(error)
                            if matches!(
                                error.code,
                                ErrorCode::Domain | ErrorCode::Solver | ErrorCode::Invariant
                            ) =>
                        {
                            continue
                        }
                        Err(error) => return Err(error),
                    };
                    let candidate_sse = if let Some(fit) = &residual {
                        Some(residual_moments(subject, &candidate_predictions, fit)?.1)
                    } else {
                        None
                    };
                    let current_eta = [phi[index][0] - log_theta[0], phi[index][1] - log_theta[1]];
                    let current = nll[index] + prior_nll_vector(current_eta, omega, 2)?;
                    let candidate = candidate_nll + prior_nll_vector(candidate_eta, omega, 2)?;
                    let correction =
                        joint.map_or(0., |q| q.log_kernel(phi[index]) - q.log_kernel(proposal));
                    if libm::log(stream.uniform()) < current - candidate + correction {
                        phi[index] = proposal;
                        nll[index] = candidate_nll;
                        if let Some(sse) = candidate_sse {
                            residual_sse[index] = sse;
                        }
                        prediction_cache[index] = candidate_predictions;
                        accepted_proposals += 1;
                        moved += 1;
                        moved_by_subject[index] = true;
                        for a in 0..2 {
                            if joint.is_some() || a == axis {
                                accepted_by_axis[a] += 1;
                                if iteration >= burn_in {
                                    post_burn_accepted[a] += 1;
                                    subject_accepted[index][a] += 1;
                                }
                            }
                        }
                    }
                }
                if status == FitStatus::EvaluationLimit {
                    break;
                }
            }
            if status == FitStatus::EvaluationLimit {
                break;
            }
            if iteration >= burn_in {
                for index in 0..count {
                    subject_stall[index] = if moved_by_subject[index] {
                        0
                    } else {
                        subject_stall[index] + 1
                    };
                    max_subject_stall = max_subject_stall.max(subject_stall[index]);
                    if iteration > burn_in {
                        for axis in 0..2 {
                            lag_one[index][axis]
                                .observe(previous_phi[index][axis], phi[index][axis]);
                        }
                    }
                }
            }
            if residual.is_none() && iteration < burn_in / 2 {
                if iteration > 0 {
                    for index in 0..count {
                        for axis in 0..2 {
                            pilot_lag_one[index][axis]
                                .observe(previous_phi[index][axis], phi[index][axis]);
                        }
                    }
                }
                if iteration + 1 == burn_in / 2 {
                    let slow_subjects = pilot_lag_one
                        .iter()
                        .filter(|subject| {
                            subject.iter().all(|axis| {
                                axis.count >= 20 && axis.correlation().unwrap_or(0.) > 0.9
                            })
                        })
                        .count();
                    extra_sweeps = slow_subjects * 2 >= count;
                }
            }
            previous_phi.clone_from_slice(&phi);
            // Tune each coordinate while the gain is one, then freeze the
            // symmetric proposal for the averaging phase. A single scale can
            // leave the tighter posterior coordinate nearly immobile.
            if iteration < burn_in {
                let gain = 1. / libm::pow((iteration + 1) as f64, 0.6);
                for axis in 0..2 {
                    if proposed_by_axis[axis] > 0 {
                        let rate = accepted_by_axis[axis] as f64 / proposed_by_axis[axis] as f64;
                        proposal_scale[axis] =
                            libm::exp(libm::log(proposal_scale[axis]) + gain * (rate - 0.44))
                                .clamp(0.01, 5.);
                    }
                }
            }
            let gamma = if iteration < burn_in {
                1.
            } else {
                1. / (iteration - burn_in + 1) as f64
            };
            for axis in 0..2 {
                let sample_mean = phi.iter().map(|p| p[axis]).sum::<f64>() / count as f64;
                mean_phi[axis] += gamma * (sample_mean - mean_phi[axis]);
                for other in 0..2 {
                    let sample_second =
                        phi.iter().map(|p| p[axis] * p[other]).sum::<f64>() / count as f64;
                    second_phi[axis][other] += gamma * (sample_second - second_phi[axis][other]);
                }
            }
            let mut next_log_theta = [0.; 2];
            let mut next_theta = [0.; 2];
            for axis in 0..2 {
                let lower = libm::log(setup.lower[axis]).max(
                    phi.iter()
                        .map(|p| p[axis])
                        .fold(f64::NEG_INFINITY, f64::max)
                        - request.eta_bound,
                );
                let upper = libm::log(setup.upper[axis]).min(
                    phi.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min) + request.eta_bound,
                );
                if lower > upper {
                    return Err(Error::new(
                        ErrorCode::Domain,
                        "SAEM eta constraints have no feasible fixed effect",
                    ));
                }
                next_log_theta[axis] = mean_phi[axis].clamp(lower, upper);
                next_theta[axis] = libm::exp(next_log_theta[axis]);
            }
            let mut next_omega = [[0.; 2]; 2];
            for a in 0..2 {
                for b in 0..2 {
                    next_omega[a][b] = second_phi[a][b]
                        - next_log_theta[a] * mean_phi[b]
                        - next_log_theta[b] * mean_phi[a]
                        + next_log_theta[a] * next_log_theta[b];
                }
            }
            for axis in 0..2 {
                next_omega[axis][axis] =
                    next_omega[axis][axis].clamp(setup.omega_lower[axis], setup.omega_upper[axis]);
            }
            let correlation_limit = if residual.is_some() && iteration < burn_in {
                0.75
            } else {
                MAX_SAEM_CORRELATION
            };
            let covariance_limit =
                correlation_limit * libm::sqrt(next_omega[0][0] * next_omega[1][1]);
            let covariance = (0.5 * (next_omega[0][1] + next_omega[1][0]))
                .clamp(-covariance_limit, covariance_limit);
            next_omega[0][1] = covariance;
            next_omega[1][0] = covariance;
            if next_theta.iter().any(|x| !x.is_finite()) || precision(next_omega, 2).is_none() {
                return Err(Error::new(
                    ErrorCode::Domain,
                    "SAEM M-step produced invalid population parameters",
                ));
            }
            let mut change: f64 = 0.;
            for axis in 0..2 {
                change = change
                    .max(((next_theta[axis] - theta[axis]) / theta[axis]).abs())
                    .max(((next_omega[axis][axis] - omega[axis][axis]) / omega[axis][axis]).abs());
            }
            change = change.max(
                (next_omega[0][1] - omega[0][1]).abs() / libm::sqrt(omega[0][0] * omega[1][1]),
            );
            if let Some(fit) = &mut residual {
                let previous = fit.variance;
                let sample = residual_sse.iter().sum::<f64>() / fit.observations as f64;
                let residual_gain = gamma.min(0.2);
                fit.variance = (previous + residual_gain * (sample - previous))
                    .clamp(fit.lower_variance, fit.upper_variance);
                if !fit.variance.is_finite() {
                    return Err(Error::new(
                        ErrorCode::Domain,
                        "SAEM residual M-step overflow",
                    ));
                }
                change = change.max(((fit.variance - previous) / previous).abs());
                set_residual_sd(&mut subjects, fit);
                for (index, subject) in subjects.iter().enumerate() {
                    let (likelihood, moment) =
                        residual_moments(subject, &prediction_cache[index], fit)?;
                    nll[index] = likelihood;
                    residual_sse[index] = moment;
                }
            }
            final_step_size = Some(gamma);
            final_relative_change = Some(change);
            if iteration >= burn_in {
                if recent_relative_changes.len() == 20 {
                    recent_relative_changes.remove(0);
                }
                recent_relative_changes.push(change);
                if post_burn_history.len() == 50 {
                    post_burn_history.remove(0);
                }
                post_burn_history.push([
                    next_log_theta[0],
                    next_log_theta[1],
                    libm::log(next_omega[0][0]),
                    libm::log(next_omega[1][1]),
                    next_omega[0][1] / libm::sqrt(next_omega[0][0] * next_omega[1][1]),
                    residual
                        .as_ref()
                        .map(|fit| libm::log(fit.variance))
                        .unwrap_or(0.),
                ]);
            }
            theta = next_theta;
            omega = next_omega;
            log_theta = next_log_theta;
            iterations = iteration + 1;
            let window_stable = if post_burn_history.len() == 50 && moved > 0 {
                (0..6).all(|axis| {
                    let history: Vec<f64> = post_burn_history.iter().map(|x| x[axis]).collect();
                    let separated: Vec<f64> = history[..15]
                        .iter()
                        .chain(&history[35..])
                        .copied()
                        .collect();
                    windowed_drift(&history[25..], request.gradient_tolerance).2
                        && windowed_drift(&separated, request.gradient_tolerance).2
                })
            } else {
                false
            };
            let mixing_stable = max_subject_stall <= 100
                && (0..2).all(|axis| {
                    post_burn_proposed[axis] > 0
                        && {
                            let acceptance =
                                post_burn_accepted[axis] as f64 / post_burn_proposed[axis] as f64;
                            (0.05..=0.95).contains(&acceptance)
                        }
                        && (0..count).all(|subject| {
                            subject_accepted[subject][axis] > 0
                                && lag_one[subject][axis]
                                    .correlation()
                                    .is_some_and(|value| value <= 0.9)
                        })
                });
            stable_iterations = if window_stable && mixing_stable {
                stable_iterations + 1
            } else {
                0
            };
            if stable_iterations >= 4 {
                status = FitStatus::Converged;
                break;
            }
        }
        let sampled_etas: Vec<Vector> = phi
            .iter()
            .map(|p| [p[0] - log_theta[0], p[1] - log_theta[1]])
            .collect();
        let final_request = residual.as_ref().map(|_| {
            let mut updated = request.clone();
            updated.subjects = subjects.clone();
            updated
        });
        let fitted_request = final_request.as_ref().unwrap_or(request);
        let mut marginal = self.saem_marginal_ofv(
            fitted_request,
            &setup,
            theta,
            omega,
            &sampled_etas,
            None,
            &mut evaluations,
            request.max_evaluations - 3 * count,
        )?;
        if marginal.is_none() && status == FitStatus::Converged {
            status = if evaluations >= request.max_evaluations - 3 * count {
                FitStatus::EvaluationLimit
            } else {
                FitStatus::LineSearchFailed
            };
        }
        if status == FitStatus::Converged {
            if let Some((ofv, modes)) = &marginal {
                let refined = self.saem_two_effect_point_refined(
                    fitted_request,
                    &setup,
                    theta,
                    omega,
                    modes,
                    residual.as_ref(),
                    *ofv,
                    &mut evaluations,
                    request.max_evaluations - 3 * count,
                    request.max_iterations.saturating_sub(iterations).min(64),
                )?;
                theta = refined.theta;
                omega = refined.omega;
                if let Some(fit) = &mut residual {
                    fit.variance = refined.residual_variance;
                    set_residual_sd(&mut subjects, fit);
                }
                iterations += refined.iterations;
                marginal = Some(refined.marginal);
                if !refined.certified {
                    status = if evaluations >= request.max_evaluations - 3 * count {
                        FitStatus::EvaluationLimit
                    } else {
                        FitStatus::LineSearchFailed
                    };
                }
            }
        }
        let final_request = residual.as_ref().map(|_| {
            let mut updated = request.clone();
            updated.subjects = subjects.clone();
            updated
        });
        let fitted_request = final_request.as_ref().unwrap_or(request);
        let uncertainty = if request.uncertainty {
            if status != FitStatus::Converged {
                return Err(observed_covariance_failure(
                    evaluations,
                    request.max_evaluations - 3 * count,
                    "SAEM observed covariance requires a converged marginal fit",
                ));
            }
            Some(self.saem_two_effect_uncertainty(
                fitted_request,
                &setup,
                theta,
                omega,
                &marginal.as_ref().expect("converged marginal fit").1,
                marginal.as_ref().expect("converged marginal fit").0,
                &mut evaluations,
                request.max_evaluations - 3 * count,
            )?)
        } else {
            None
        };
        let final_etas = marginal
            .as_ref()
            .map(|(_, modes)| modes)
            .unwrap_or(&sampled_etas);
        let mut subject_results = Vec::with_capacity(count);
        let mut fitted_observations = Vec::new();
        let mut surrogate = 0.;
        for (index, subject) in fitted_request.subjects.iter().enumerate() {
            let eta = final_etas[index];
            let likelihood = self.saem_nll(subject, &setup, theta, eta)?;
            evaluations += 1;
            surrogate += likelihood + prior_nll_vector(eta, omega, 2)?;
            fitted_observations.extend(self.saem_predictions(index, subject, &setup, theta, eta)?);
            evaluations += 2;
            subject_results.push(PopulationSubjectEstimate {
                eta: eta.to_vec(),
                negative_log_likelihood: likelihood,
            });
        }
        if !surrogate.is_finite() {
            return Err(Error::new(ErrorCode::Domain, "SAEM surrogate overflow"));
        }
        let mut fixed_effects = std::collections::BTreeMap::new();
        for axis in 0..2 {
            fixed_effects.insert(
                setup.names[axis].clone(),
                qspquest_core::model::Quantity {
                    value: theta[axis],
                    unit: setup.units[axis].clone(),
                },
            );
        }
        let recent_window = recent_relative_changes.len();
        recent_relative_changes.sort_by(f64::total_cmp);
        let recent_relative_change_median = if recent_window == 0 {
            None
        } else if recent_window % 2 == 0 {
            Some(
                0.5 * (recent_relative_changes[recent_window / 2 - 1]
                    + recent_relative_changes[recent_window / 2]),
            )
        } else {
            Some(recent_relative_changes[recent_window / 2])
        };
        let post_burn_acceptance_by_axis = std::array::from_fn(|axis| {
            post_burn_accepted[axis] as f64 / post_burn_proposed[axis].max(1) as f64
        });
        let post_burn_min_subject_acceptance_by_axis = std::array::from_fn(|axis| {
            (0..count)
                .map(|index| {
                    subject_accepted[index][axis] as f64
                        / subject_proposed[index][axis].max(1) as f64
                })
                .fold(1., f64::min)
        });
        let post_burn_max_lag_one_correlation_by_axis = std::array::from_fn(|axis| {
            (0..count)
                .filter_map(|index| lag_one[index][axis].correlation())
                .fold(0., f64::max)
        });
        Ok(PopulationFitResult {
            status,
            fixed_effects,
            omega: vec![omega[0].to_vec(), omega[1].to_vec()],
            covariate_effect: None,
            subjects: subject_results,
            fitted_observations: Some(fitted_observations),
            uncertainty,
            estimated_residual_error: residual.map(|fit| PopulationResidualErrorEstimate {
                output: fit.output,
                additive_sd: qspquest_core::model::Quantity {
                    value: libm::sqrt(fit.variance),
                    unit: fit.unit,
                },
            }),
            saem_diagnostics: Some(PopulationSaemDiagnostics {
                burn_in_iterations: burn_in,
                final_step_size,
                final_relative_change,
                recent_relative_change_median,
                recent_window,
                evaluated_proposals,
                accepted_proposals,
                consecutive_stable_iterations: stable_iterations,
                post_burn_acceptance_by_axis: (post_burn_proposed != [0; 2])
                    .then_some(post_burn_acceptance_by_axis),
                post_burn_min_subject_acceptance_by_axis: (post_burn_proposed != [0; 2])
                    .then_some(post_burn_min_subject_acceptance_by_axis),
                post_burn_max_subject_stall_iterations: (post_burn_proposed != [0; 2])
                    .then_some(max_subject_stall),
                post_burn_max_lag_one_correlation_by_axis: (post_burn_proposed != [0; 2])
                    .then_some(post_burn_max_lag_one_correlation_by_axis),
            }),
            objective_kind: if marginal.is_some() {
                PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue
            } else {
                PopulationObjectiveKind::SaemCompleteDataSurrogate
            },
            objective: marginal.map(|(value, _)| value).unwrap_or(surrogate),
            evaluations,
            iterations,
            fit_request_hash: canonical_hash(&serde_json::json!(request))?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{windowed_drift, LagOne, Stream};

    #[test]
    fn lag_one_reports_sticky_and_alternating_chains() {
        let mut sticky = LagOne::default();
        let mut alternating = LagOne::default();
        let mut immobile = LagOne::default();
        for index in 0..100 {
            sticky.observe(index as f64, (index + 1) as f64);
            alternating.observe((index % 2) as f64, ((index + 1) % 2) as f64);
            immobile.observe(1., 1.);
        }
        assert!(sticky.correlation().unwrap() > 0.99);
        assert!(alternating.correlation().unwrap() < -0.99);
        assert_eq!(immobile.correlation(), Some(1.));
        assert_eq!(LagOne::default().correlation(), None);
    }

    #[test]
    fn stream_is_reproducible_and_finite() {
        let mut a = Stream::new(42);
        let mut b = Stream::new(42);
        for _ in 0..1000 {
            let x = a.normal();
            assert_eq!(x, b.normal());
            assert!(x.is_finite());
        }
    }

    #[test]
    fn windowed_drift_rejects_trend_and_high_noise() {
        let flat: Vec<f64> = (0..50).map(|i| 1.0 + (i % 2) as f64 * 0.0001).collect();
        assert!(windowed_drift(&flat, 1e-5).2);
        let trending: Vec<f64> = (0..50).map(|i| 1.0 + i as f64 * 0.001).collect();
        assert!(!windowed_drift(&trending, 1e-5).2);
        assert!(!windowed_drift(&trending, 0.1).2);
        let noisy: Vec<f64> = (0..50)
            .map(|i| 1.0 + (i % 2) as f64 * 0.1 + i as f64 * 0.001)
            .collect();
        assert!(!windowed_drift(&noisy, 1e-5).2);
        let correlated_drift: Vec<f64> = (0..50)
            .map(|i| 1.0 + 0.0008 * i as f64 + 0.002 * libm::sin(i as f64 / 6.))
            .collect();
        assert!(!windowed_drift(&correlated_drift, 1e-5).2);
    }
}

#[cfg(test)]
mod preparation_tests {
    #[test]
    fn uncertainty_reports_work_budget_and_point_fit_keeps_limit_status() {
        for two in [false, true] {
            let (_, mut request) = super::super::preparation_tests::population_fixture(two);
            let source = serde_json::from_str(include_str!(
                "../../../../../conformance/models/synthetic-one-compartment.json"
            ))
            .unwrap();
            let model = super::CompiledSensitivityDocument::compile(
                &source,
                &request.subjects[0].simulation.with_respect_to,
            )
            .unwrap();
            request.max_evaluations = request.subjects.len() * 4;
            request.uncertainty = true;
            let error = model.fit_population_saem(&request).unwrap_err();
            assert_eq!(error.code, super::ErrorCode::WorkBudget);
            assert!(error.message.contains("max_evaluations"));
            assert!(error.message.contains("uncertainty=false"));

            request.uncertainty = false;
            let diagnostic = model.fit_population_saem(&request).unwrap();
            assert_eq!(diagnostic.status, super::FitStatus::EvaluationLimit);
            assert!(diagnostic.uncertainty.is_none());
            assert_eq!(
                diagnostic.objective_kind,
                super::PopulationObjectiveKind::SaemCompleteDataSurrogate
            );

            request.max_evaluations = 10_000;
            request.uncertainty = true;
            let unconverged = model.fit_population_saem(&request).unwrap_err();
            assert_eq!(unconverged.code, super::ErrorCode::Unsupported);
            assert!(unconverged.message.contains("converged marginal fit"));
        }
    }

    #[test]
    fn all_saem_branches_prepare_without_readouts_or_random_draws() {
        for two in [false, true] {
            let (model, mut request) = super::super::preparation_tests::population_fixture(two);
            assert!(model.prepare_population_saem(&request).is_ok());
            // The primal likelihood path does not require forward derivative tolerances.
            for subject in &mut request.subjects {
                subject.simulation.derivative_absolute_tolerances.clear();
            }
            assert!(model.prepare_population_saem(&request).is_ok());
            request.subjects[1].observations[0].row = 999;
            let error = model.prepare_population_saem(&request).err().unwrap();
            assert_eq!(
                error.expression.as_deref(),
                Some("subjects[1].observations[0]")
            );
            assert_eq!(error, model.fit_population_saem(&request).unwrap_err());
        }
    }
}

impl CompiledSensitivityDocument {
    pub(super) fn describe_population_saem(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        plan: &mut super::fit_planning::FitPlanBuilder,
    ) -> Result<
        (
            Vec<qspquest_core::planning::FitCoordinatePlan>,
            qspquest_core::planning::PopulationPreparationPlan,
        ),
        Error,
    > {
        let prepared = self.prepare_population_saem_observed(request, Some(plan))?;
        let (setup, burn_in, residual, x, beta) = match prepared {
            PreparedSaem::One(p) => (p.setup, p.burn_in, p.residual, p.x, p.beta),
            PreparedSaem::Two(p) => (p.setup, p.burn_in, p.residual, Vec::new(), Vec::new()),
        };
        let coordinates = (0..setup.dimensions)
            .map(|i| {
                super::fit_planning::coordinate(
                    &setup.names[i],
                    &setup.units[i],
                    setup.start[i],
                    setup.lower[i],
                    setup.upper[i],
                    qspquest_core::planning::FitSearchCoordinate::LogNaturalValue,
                    libm::log(setup.start[i]),
                )
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut population = super::fit_planning::population(request);
        population.burn_in_iterations = Some(burn_in);
        population.centered_covariates = if beta.is_empty() { Vec::new() } else { x };
        population.initial_covariate_coefficients = beta;
        population.residual = residual.map(|r| qspquest_core::planning::ResidualFitPlan {
            output: r.output,
            output_unit: r.unit,
            initial: r.variance,
            lower: r.lower_variance,
            upper: r.upper_variance,
            parameterization: qspquest_core::planning::ResidualParameterization::Variance,
        });
        Ok((coordinates, population))
    }
}

#[cfg(test)]
mod curvature_proposal_tests {
    use super::*;
    #[test]
    fn correlated_proposal_has_declared_mean_and_covariance() {
        let l00 = libm::sqrt(2. / 7.);
        let q = JointProposal {
            mean: [1., -0.5],
            precision: [[4., 1.], [1., 2.]],
            lower: [[l00, 0.], [-1. / (7. * l00), libm::sqrt(0.5)]],
        };
        let mut rng = Stream::new(2026101304);
        let mut sum = [0.; 2];
        let mut second = [[0.; 2]; 2];
        let n = 50000;
        for _ in 0..n {
            let x = q.draw(&mut rng);
            for a in 0..2 {
                sum[a] += x[a];
                for b in 0..2 {
                    second[a][b] += (x[a] - q.mean[a]) * (x[b] - q.mean[b]);
                }
            }
        }
        for a in 0..2 {
            assert!((sum[a] / n as f64 - q.mean[a]).abs() < 0.015);
        }
        let expected = [[2. / 7., -1. / 7.], [-1. / 7., 4. / 7.]];
        for a in 0..2 {
            for b in 0..2 {
                assert!((second[a][b] / n as f64 - expected[a][b]).abs() < 0.015);
            }
        }
        // Independent expansion: delta = [-3, 3.5], quadratic = 36 - 21 + 24.5.
        assert!((q.log_kernel([-2., 3.]) + 19.75).abs() < 1e-12);
    }
}

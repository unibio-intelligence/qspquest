//! Bounded SAEM for one or two lognormal population parameters.
//!
//! Stochastic approximation tracks the first and second moments of the log
//! individual parameters. A final observed-curvature Laplace calculation
//! reports an approximate marginal OFV when the work budget permits it.
use super::*;
use qspquest_core::{fit::*, identity::canonical_hash};

mod chain;
mod general;

const LOG_2PI: f64 = 1.8378770664093453;
const MAX_SAEM_CORRELATION: f64 = 0.95;
type Vector = [f64; 2];
type Matrix = [[f64; 2]; 2];

#[cfg(test)]
mod curvature_refinement_tests {
    use super::*;

    #[test]
    fn active_normal_scores_do_not_corrupt_the_free_quasi_newton_update() {
        let mut point = [0.; 6];
        point[0] = -0.4;
        point[1] = 0.6;
        let mut lower = [0.; 6];
        lower[0] = -1.;
        lower[1] = -1.;
        let mut upper = [0.; 6];
        upper[0] = 1.;
        upper[1] = 1.;
        upper[2] = 1.;
        let objective = |p: [f64; 6]| {
            100. * (p[0] + p[1] - 0.3).powi(2)
                + (p[0] - 0.1).powi(2)
                + p[2] * (165. + 30. * p[0] - 20. * p[1])
                + 100. * p[2] * p[2]
        };
        let result = refine_marginal_box(
            MarginalBoxPoint {
                point,
                value: objective(point),
                modes: Vec::new(),
                certified: false,
                iterations: 0,
            },
            lower,
            upper,
            [1.; 6],
            1e-5,
            16,
            |p, _| Ok(Some((objective(p), Vec::new()))),
        )
        .unwrap();
        assert!(
            result.certified,
            "iterations={} point={:?} objective={}",
            result.iterations, result.point, result.value
        );
        assert!(result.value < 1e-9);
        assert!((result.point[0] - 0.1).abs() < 1e-5);
        assert!((result.point[1] - 0.2).abs() < 1e-5);
        assert_eq!(result.point[2], 0.);
    }

    #[test]
    fn symmetric_fallback_preserves_the_original_work_ceiling() {
        let mut evaluations = 0;
        let covariance = certified_covariance(2, 0., 0.01, |x| {
            evaluations += 1;
            if evaluations > 28 {
                return Err(Error::new(ErrorCode::WorkBudget, "fixed curvature budget"));
            }
            Ok(x[0].powi(2) + x[1].powi(2) + 10000. * x[0].powi(2) * x[1].powi(2))
        })
        .unwrap();
        assert_eq!(evaluations, 16);
        assert!((covariance[0][0] - 1.).abs() < 1e-10);
        assert!((covariance[1][1] - 1.).abs() < 1e-10);
        assert!(covariance[0][1].abs() < 1e-10);
    }

    #[test]
    fn mixed_quartic_curvature_uses_a_certified_symmetric_fallback() {
        let covariance = certified_covariance(2, 0., 0.01, |x| {
            Ok(x[0].powi(2) + x[1].powi(2) + 10000. * x[0].powi(2) * x[1].powi(2))
        })
        .unwrap();
        assert!((covariance[0][0] - 1.).abs() < 1e-10);
        assert!((covariance[1][1] - 1.).abs() < 1e-10);
        assert!(covariance[0][1].abs() < 1e-10);
    }

    #[test]
    fn joint_marginal_refinement_resolves_coupled_coordinates() {
        for unit in [1e-6, 1., 1e6] {
            let objective =
                |p: [f64; 6]| 100. * (p[0] + p[1] / unit - 0.8).powi(2) + (p[0] - 0.2).powi(2);
            let mut start = [0.; 6];
            start[0] = -0.5;
            start[1] = -0.5 * unit;
            let mut lower = [0.; 6];
            lower[0] = -1.;
            lower[1] = -unit;
            let mut upper = [0.; 6];
            upper[0] = 1.;
            upper[1] = unit;
            let mut scales = [1.; 6];
            scales[1] = unit;
            let result = refine_marginal_box(
                MarginalBoxPoint {
                    point: start,
                    value: objective(start),
                    modes: Vec::new(),
                    certified: false,
                    iterations: 0,
                },
                lower,
                upper,
                scales,
                1e-5,
                64,
                |p, _| Ok(Some((objective(p), Vec::new()))),
            )
            .unwrap();
            assert!(
                result.certified,
                "iterations={}, f={}",
                result.iterations, result.value
            );
            assert!(result.value < 1e-9);
            assert!((result.point[0] - 0.2).abs() < 1e-5);
            assert!((result.point[1] / unit - 0.6).abs() < 1e-5);
        }
    }

    #[test]
    fn joint_marginal_refinement_does_not_certify_missing_work() {
        let result = refine_marginal_box(
            MarginalBoxPoint {
                point: [0.5; 6],
                value: 1.5,
                modes: Vec::new(),
                certified: false,
                iterations: 0,
            },
            [-1.; 6],
            [1.; 6],
            [1.; 6],
            1e-5,
            64,
            |_, _| Ok(None),
        )
        .unwrap();
        assert!(!result.certified);
        assert_eq!(result.point, [0.5; 6]);
    }

    #[test]
    fn projected_marginal_score_resolves_roundoff_at_a_correlation_cap() {
        let mut point = [0.; 6];
        point[4] = -0.95 + f64::EPSILON;
        let mut lower = [0.; 6];
        lower[4] = -0.95;
        let mut upper = [0.; 6];
        upper[4] = 0.95;
        let objective = |p: [f64; 6]| 700. + p[4] - if p[4] == -0.95 { 1e-10 } else { 0. };
        let result = refine_marginal_box(
            MarginalBoxPoint {
                point,
                value: objective(point),
                modes: Vec::new(),
                certified: false,
                iterations: 0,
            },
            lower,
            upper,
            [1.; 6],
            0.001,
            64,
            |p, _| Ok(Some((objective(p), Vec::new()))),
        )
        .unwrap();
        assert!(result.certified);
        assert_eq!(result.point[4], -0.95);
        assert_eq!(result.iterations, 0);
    }

    #[test]
    fn finite_probe_changes_cannot_mask_a_resolved_marginal_slope() {
        let mut point = [0.; 6];
        point[4] = 0.25;
        assert!(!certify_no_marginal_descent(
            point,
            [-1.; 6],
            [1.; 6],
            [0.025; 6],
            0.4,
            0.144,
            |candidate| Ok(Some(6.4 * candidate[4].powi(2))),
        )
        .unwrap());
    }

    #[test]
    fn marginal_score_certificate_is_invariant_to_coordinate_units() {
        for scale in [1e-6, 1., 1e6] {
            let mut point = [0.; 6];
            point[4] = 0.25 * scale;
            assert!(!certify_no_marginal_descent(
                point,
                [-scale; 6],
                [scale; 6],
                [0.025 * scale; 6],
                0.4,
                0.144,
                |candidate| Ok(Some(6.4 * (candidate[4] / scale).powi(2))),
            )
            .unwrap());
        }
    }

    #[test]
    fn a_stationary_marginal_maximum_is_not_convergence() {
        assert!(!certify_no_marginal_descent(
            [0.; 6],
            [-1.; 6],
            [1.; 6],
            [0.025; 6],
            0.,
            0.144,
            |candidate| Ok(Some(-6.4 * candidate[4].powi(2))),
        )
        .unwrap());
    }

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
    fn symmetric_fallback_reuses_probes_without_changing_covariance_or_budget() {
        // This mixed quartic contaminates the directional stencil but cancels
        // in the symmetric stencil. The latter must still certify refinement.
        let objective = |x: &[f64; 5]| {
            x.iter().map(|a| a * a).sum::<f64>() + 100. * x[0].powi(2) * x[1].powi(2)
        };
        let mut uncached_calls = 0;
        let mut uncached = |x: &[f64; 5]| -> Result<f64, Error> {
            uncached_calls += 1;
            Ok(objective(x))
        };
        assert_eq!(
            refine_covariance_stencil(5, 0., 0.01, false, 1, &mut uncached)
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        let expected = refine_covariance_stencil(5, 0., 0.01, true, 4, &mut uncached).unwrap();
        assert_eq!(uncached_calls, 160);

        let mut control_calls = 0;
        let mut bounded_control = |x: &[f64; 5]| -> Result<f64, Error> {
            control_calls += 1;
            if control_calls > 100 {
                return Err(Error::new(ErrorCode::WorkBudget, "same probe budget"));
            }
            Ok(objective(x))
        };
        assert_eq!(
            refine_covariance_stencil(5, 0., 0.01, false, 1, &mut bounded_control)
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            refine_covariance_stencil(5, 0., 0.01, true, 4, &mut bounded_control)
                .unwrap_err()
                .code,
            ErrorCode::WorkBudget
        );
        assert_eq!(control_calls, 101);

        let mut calls = 0;
        let mut unique = std::collections::BTreeSet::new();
        let actual = certified_covariance(5, 0., 0.01, |x| {
            calls += 1;
            assert!(unique.insert(x.map(|v| if v == 0. { 0 } else { v.to_bits() })));
            if calls > 100 {
                return Err(Error::new(ErrorCode::WorkBudget, "same probe budget"));
            }
            Ok(objective(x))
        })
        .unwrap();
        assert_eq!(calls, 100);
        assert_eq!(actual, expected);
        for (i, row) in actual.iter().enumerate() {
            for (j, value) in row.iter().enumerate() {
                assert!((value - if i == j { 1. } else { 0. }).abs() < 1e-12);
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
    fn joint_covariance_reuses_axis_samples_within_its_work_budget() {
        let mut evaluations = 0;
        let covariance = covariance_at_step(5, 0., 0.001, &mut |x| {
            evaluations += 1;
            if evaluations > 30 {
                return Err(Error::new(
                    ErrorCode::WorkBudget,
                    "independent quadratic budget",
                ));
            }
            Ok(x.iter().map(|a| a * a).sum::<f64>() + 0.2 * x.iter().sum::<f64>().powi(2))
        })
        .unwrap();
        assert_eq!(evaluations, 30);
        for (i, row) in covariance.iter().enumerate() {
            for (j, value) in row.iter().enumerate() {
                let expected = if i == j { 0.9 } else { -0.1 };
                assert!((value - expected).abs() < 1e-10);
            }
        }
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
#[cfg(test)]
fn covariance_at_step(
    dimensions: usize,
    center: f64,
    h: f64,
    evaluate: &mut impl FnMut(&[f64; 5]) -> Result<f64, Error>,
) -> Result<Vec<Vec<f64>>, Error> {
    covariance_at_step_with_stencil(dimensions, center, h, false, evaluate)
}

fn covariance_at_step_with_stencil(
    dimensions: usize,
    center: f64,
    h: f64,
    full_symmetric: bool,
    evaluate: &mut impl FnMut(&[f64; 5]) -> Result<f64, Error>,
) -> Result<Vec<Vec<f64>>, Error> {
    let mut hessian = [[0.; 5]; 5];
    let mut axial_sums = [0.; 5];
    for axis in 0..dimensions {
        let mut offsets = [0.; 5];
        offsets[axis] = h;
        let plus = evaluate(&offsets)?;
        offsets[axis] = -h;
        let minus = evaluate(&offsets)?;
        axial_sums[axis] = plus + minus;
        hessian[axis][axis] = (plus - 2. * center + minus) / (h * h);
        for other in 0..axis {
            let mut offsets = [0.; 5];
            offsets[axis] = h;
            offsets[other] = h;
            let pp = evaluate(&offsets)?;
            offsets[axis] = -h;
            offsets[other] = -h;
            let mm = evaluate(&offsets)?;
            // Central curvature along e_i+e_j minus the two axial
            // curvatures gives 2*H_ij. Reusing the axial samples retains
            // second-order accuracy with two mixed samples per pair.
            let cross = if full_symmetric {
                offsets[axis] = h;
                let pm = evaluate(&offsets)?;
                offsets[axis] = -h;
                offsets[other] = h;
                let mp = evaluate(&offsets)?;
                (pp - pm - mp + mm) / (4. * h * h)
            } else {
                (pp + mm - axial_sums[axis] - axial_sums[other] + 2. * center) / (2. * h * h)
            };
            hessian[axis][other] = cross;
            hessian[other][axis] = cross;
        }
    }
    // Cholesky inversion rejects an unidentified covariance direction.
    let mut lower = [[0.; 5]; 5];
    for row in 0..dimensions {
        for col in 0..=row {
            let mut value = hessian[row][col];
            for (left, right) in lower[row][..col].iter().zip(&lower[col][..col]) {
                value -= left * right;
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
    let mut inverse_columns = [[0.; 5]; 5];
    for (column, solution) in inverse_columns.iter_mut().take(dimensions).enumerate() {
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
                value -= lower[k][row] * solution[k];
            }
            solution[row] = value / lower[row][row];
        }
    }
    let covariance = (0..dimensions)
        .map(|row| {
            (0..dimensions)
                .map(|col| 2. * inverse_columns[col][row])
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    Ok(covariance)
}

fn certified_covariance(
    dimensions: usize,
    center: f64,
    h: f64,
    mut evaluate: impl FnMut(&[f64; 5]) -> Result<f64, Error>,
) -> Result<Vec<Vec<f64>>, Error> {
    if !h.is_finite() || h <= 0. {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "SAEM observed covariance requires an interior positive-definite estimate",
        ));
    }
    // Both stencils evaluate the same deterministic marginal objective at
    // shared offsets. Retain successful finite values across refinement and
    // fallback so duplicate probes do not consume the request work budget.
    // The cache belongs only to this fixed center and objective invocation.
    let mut samples = std::collections::BTreeMap::<[u64; 5], f64>::new();
    let mut cached = |offsets: &[f64; 5]| -> Result<f64, Error> {
        let key = offsets.map(|x| if x == 0. { 0 } else { x.to_bits() });
        if let Some(value) = samples.get(&key) {
            return Ok(*value);
        }
        let value = evaluate(offsets)?;
        if value.is_finite() {
            samples.insert(key, value);
        }
        Ok(value)
    };
    match refine_covariance_stencil(dimensions, center, h, false, 1, &mut cached) {
        Ok(covariance) => Ok(covariance),
        Err(error) if error.code == ErrorCode::Unsupported => {
            // Nonlinear mixed fourth derivatives contaminate the inexpensive
            // directional stencil. The symmetric stencil cancels those terms
            // and must independently satisfy the same refinement certificate.
            refine_covariance_stencil(dimensions, center, h, true, 4, &mut cached)
        }
        Err(error) => Err(error),
    }
}

fn refine_covariance_stencil(
    dimensions: usize,
    center: f64,
    mut h: f64,
    full_symmetric: bool,
    refinements: usize,
    evaluate: &mut impl FnMut(&[f64; 5]) -> Result<f64, Error>,
) -> Result<Vec<Vec<f64>>, Error> {
    let mut previous =
        covariance_at_step_with_stencil(dimensions, center, h, full_symmetric, evaluate)?;
    for _ in 0..refinements {
        h *= 0.5;
        let refined =
            covariance_at_step_with_stencil(dimensions, center, h, full_symmetric, evaluate)?;
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

#[cfg(test)]
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
        let mut probes = Vec::with_capacity(2);
        let probe_width = steps[axis].min(1e-4 * (upper[axis] - lower[axis]));
        for sign in [-1., 1.] {
            let mut trial = point;
            trial[axis] = (point[axis] + sign * probe_width).clamp(lower[axis], upper[axis]);
            if trial[axis] == point[axis] {
                continue;
            }
            let Some(value) = evaluate(trial)? else {
                return Ok(false);
            };
            if !value.is_finite() {
                return Ok(false);
            }
            probes.push((trial[axis] - point[axis], value));
        }
        // Probe changes shrink with their width even when the marginal score
        // remains unresolved. At a bound only feasible descent matters.
        let score = match probes.as_slice() {
            [(delta, value)] => (center - value) / delta.abs(),
            [(minus, left), (plus, right)] => {
                let left_slope = (center - left) / -minus;
                let right_slope = (right - center) / plus;
                let gradient = (plus * left_slope - minus * right_slope) / (plus - minus);
                let curvature = 2. * (right_slope - left_slope) / (plus - minus);
                if gradient.abs() * (upper[axis] - lower[axis]) > tolerance
                    && curvature.is_finite()
                    && curvature > 0.
                {
                    // A checked coordinate-Newton trial uses the broad search
                    // radius while the score uses a separate local stencil.
                    let shift = (-gradient / curvature).clamp(-steps[axis], steps[axis]);
                    let mut trial = point;
                    trial[axis] = (point[axis] + shift).clamp(lower[axis], upper[axis]);
                    if trial[axis] != point[axis] {
                        let Some(value) = evaluate(trial)? else {
                            return Ok(false);
                        };
                        if !value.is_finite() {
                            return Ok(false);
                        }
                    }
                }
                gradient.abs()
            }
            _ => continue,
        } * (upper[axis] - lower[axis]);
        let rounding = 64. * f64::EPSILON * (1. + center.abs());
        let negative_curvature =
            probes.len() == 2 && probes.iter().all(|(_, value)| *value < center - rounding);
        if !score.is_finite()
            || score > tolerance
            || negative_curvature
            || probes.iter().any(|(_, value)| *value < center - tolerance)
        {
            return Ok(false);
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

struct MarginalBoxPoint {
    point: [f64; 6],
    value: f64,
    modes: Vec<Vector>,
    certified: bool,
    iterations: usize,
}

/// Joint inverse-BFGS search with an independently evaluated central score.
/// The search scales condition the direction; the certificate retains the
/// projected score in the full original box and the caller's work ceiling.
fn refine_marginal_box(
    mut result: MarginalBoxPoint,
    lower: [f64; 6],
    upper: [f64; 6],
    scales: [f64; 6],
    tolerance: f64,
    max_iterations: usize,
    mut evaluate: impl FnMut([f64; 6], &[Vector]) -> Result<Option<(f64, Vec<Vector>)>, Error>,
) -> Result<MarginalBoxPoint, Error> {
    let mut inverse = [[0.; 6]; 6];
    let mut previous: Option<([f64; 6], [f64; 6], [bool; 6])> = None;
    for _ in 0..=max_iterations {
        // Reconstruction and accepted steps can land a rounding unit inside
        // a bound. Reevaluate exact projection before every score certificate.
        let mut snapped = result.point;
        for axis in 0..6 {
            let lo_roundoff = 16. * f64::EPSILON * lower[axis].abs().max(scales[axis]);
            let hi_roundoff = 16. * f64::EPSILON * upper[axis].abs().max(scales[axis]);
            if (snapped[axis] - lower[axis]).abs() <= lo_roundoff {
                snapped[axis] = lower[axis];
            } else if (snapped[axis] - upper[axis]).abs() <= hi_roundoff {
                snapped[axis] = upper[axis];
            }
        }
        if snapped != result.point {
            let Some((value, modes)) = evaluate(snapped, &result.modes)? else {
                return Ok(result);
            };
            if !value.is_finite() {
                return Ok(result);
            }
            result.point = snapped;
            result.value = value;
            result.modes = modes;
        }
        let mut gradient = [0.; 6];
        let mut diagonal = [1.; 6];
        let mut negative_curvature = false;
        let mut fallback = None;
        let rounding = 64. * f64::EPSILON * (1. + result.value.abs());
        for axis in 0..6 {
            if lower[axis] == upper[axis] {
                continue;
            }
            let width = (1e-3 * scales[axis]).min(0.01 * (upper[axis] - lower[axis]));
            let mut probes = Vec::with_capacity(2);
            for sign in [-1., 1.] {
                let mut trial = result.point;
                trial[axis] = (trial[axis] + sign * width).clamp(lower[axis], upper[axis]);
                let delta = (trial[axis] - result.point[axis]) / scales[axis];
                if delta.abs()
                    <= 16. * f64::EPSILON * result.point[axis].abs().max(scales[axis])
                        / scales[axis]
                {
                    continue;
                }
                let Some((value, modes)) = evaluate(trial, &result.modes)? else {
                    return Ok(result);
                };
                if !value.is_finite() {
                    return Ok(result);
                }
                probes.push((delta, value));
                if value < result.value - rounding
                    && fallback.as_ref().is_none_or(|(_, best, _)| value < *best)
                {
                    fallback = Some((trial, value, modes));
                }
            }
            match probes.as_slice() {
                [(delta, value)] => gradient[axis] = (value - result.value) / delta,
                [(minus, left), (plus, right)] => {
                    let ls = (result.value - left) / -minus;
                    let rs = (right - result.value) / plus;
                    gradient[axis] = (plus * ls - minus * rs) / (plus - minus);
                    let curvature = 2. * (rs - ls) / (plus - minus);
                    if curvature > 0. && curvature.is_finite() {
                        diagonal[axis] = (1. / curvature).clamp(1e-8, 1e6);
                    }
                    negative_curvature |=
                        *left < result.value - rounding && *right < result.value - rounding;
                }
                _ => return Ok(result),
            }
        }
        let mut projected = gradient;
        let active: [bool; 6] = std::array::from_fn(|axis| {
            lower[axis] == upper[axis]
                || (result.point[axis] == lower[axis] && gradient[axis] > 0.)
                || (result.point[axis] == upper[axis] && gradient[axis] < 0.)
        });
        for axis in 0..6 {
            if active[axis] {
                projected[axis] = 0.;
            }
        }
        let norm = (0..6)
            .map(|axis| projected[axis].abs() * (upper[axis] - lower[axis]) / scales[axis])
            .fold(0_f64, f64::max);
        if norm.is_finite() && norm <= tolerance && !negative_curvature {
            result.certified = true;
            return Ok(result);
        }
        if result.iterations == max_iterations {
            return Ok(result);
        }
        // Curvature belongs to the free subspace. Normal scores at an active
        // bound must not enter its secant update; reset when that space changes.
        let mut reset = previous.is_none_or(|(_, _, old_active)| old_active != active);
        if let Some((old_point, old_gradient, _)) =
            previous.filter(|(_, _, old_active)| *old_active == active)
        {
            let step: [f64; 6] =
                std::array::from_fn(|i| (result.point[i] - old_point[i]) / scales[i]);
            let y: [f64; 6] = std::array::from_fn(|i| projected[i] - old_gradient[i]);
            let sy: f64 = (0..6).map(|i| step[i] * y[i]).sum();
            let ss: f64 = step.iter().map(|x| x * x).sum();
            let yy: f64 = y.iter().map(|x| x * x).sum();
            if sy.is_finite() && sy > 1e-12 * libm::sqrt(ss * yy) {
                let hy: [f64; 6] =
                    std::array::from_fn(|i| (0..6).map(|j| inverse[i][j] * y[j]).sum());
                let yhy: f64 = (0..6).map(|i| y[i] * hy[i]).sum();
                for i in 0..6 {
                    for j in 0..6 {
                        inverse[i][j] += (sy + yhy) / (sy * sy) * step[i] * step[j]
                            - (hy[i] * step[j] + step[i] * hy[j]) / sy;
                    }
                }
                reset |= inverse.iter().flatten().any(|x| !x.is_finite());
            } else {
                reset = true;
            }
        }
        if reset {
            inverse = [[0.; 6]; 6];
            for axis in 0..6 {
                inverse[axis][axis] = diagonal[axis];
            }
        }
        let mut direction: [f64; 6] =
            std::array::from_fn(|i| -(0..6).map(|j| inverse[i][j] * projected[j]).sum::<f64>());
        for axis in 0..6 {
            if lower[axis] == upper[axis]
                || (result.point[axis] == lower[axis] && gradient[axis] > 0.)
                || (result.point[axis] == upper[axis] && gradient[axis] < 0.)
                || (result.point[axis] == lower[axis] && direction[axis] < 0.)
                || (result.point[axis] == upper[axis] && direction[axis] > 0.)
            {
                direction[axis] = 0.;
            }
        }
        let slope: f64 = (0..6).map(|i| direction[i] * gradient[i]).sum();
        if !slope.is_finite() || slope >= 0. {
            direction = std::array::from_fn(|i| -diagonal[i] * projected[i]);
        }
        let mut alpha = 1.;
        let mut accepted = None;
        for _ in 0..24 {
            let trial: [f64; 6] = std::array::from_fn(|i| {
                (result.point[i] + alpha * scales[i] * direction[i]).clamp(lower[i], upper[i])
            });
            let slope: f64 = (0..6)
                .map(|i| gradient[i] * (trial[i] - result.point[i]) / scales[i])
                .sum();
            if slope < 0. && trial != result.point {
                let Some((value, modes)) = evaluate(trial, &result.modes)? else {
                    return Ok(result);
                };
                if value.is_finite() && value <= result.value + 1e-4 * slope {
                    accepted = Some((trial, value, modes));
                    break;
                }
            }
            alpha *= 0.5;
        }
        let Some((point, value, modes)) = accepted.or(fallback) else {
            return Ok(result);
        };
        previous = Some((result.point, projected, active));
        result.point = point;
        result.value = value;
        result.modes = modes;
        result.iterations += 1;
    }
    Ok(result)
}

enum PreparedSaem {
    One(Box<PreparedSaemOneDim>),
    Two(Box<PreparedSaemTwoDim>),
    General(Box<general::PreparedGeneral>),
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
}
struct PreparedSaemTwoDim {
    setup: Setup,
    burn_in: usize,
    residual: Option<ResidualFit>,
    subjects: Vec<GaussianObjectiveRequest>,
}

struct EvaluationWork<'a> {
    setup: &'a Setup,
    evaluations: &'a mut usize,
    budget: usize,
}

#[derive(Clone, Copy)]
struct PopulationPoint {
    theta: Vector,
    omega: Matrix,
}

struct MarginalRefinement<'a> {
    residual: Option<&'a ResidualFit>,
    center: f64,
    modes: &'a [Vector],
    max_iterations: usize,
}

struct CovariateDesign<'a> {
    beta: &'a [f64],
    x: &'a [Vec<f64>],
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
    }))
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
        for cell in &mut augmented[column][column..=n] {
            *cell /= divisor;
        }
        let (above, pivot_and_below) = augmented.split_at_mut(column);
        let (pivot, below) = pivot_and_below.split_first_mut().expect("pivot row");
        for row in above.iter_mut().chain(below) {
            let factor = row[column];
            for (cell, pivot_cell) in row[column..=n].iter_mut().zip(&pivot[column..=n]) {
                *cell -= factor * pivot_cell;
            }
        }
    }
    Ok(augmented.iter().map(|row| row[n]).collect())
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

/// Gaussian approximation of one subject's conditional eta distribution.
#[derive(Clone, Copy)]
struct JointProposal {
    mean: Vector,
    precision: Matrix,
}

impl JointProposal {
    #[cfg(test)]
    fn rebased_prior(
        self,
        anchor_mean: Vector,
        anchor_precision: Matrix,
        current_mean: Vector,
        current_precision: Matrix,
    ) -> Option<Self> {
        self.reweighted_likelihood(
            anchor_mean,
            anchor_precision,
            current_mean,
            current_precision,
            1.,
        )
    }

    fn reweighted_likelihood(
        self,
        anchor_mean: Vector,
        anchor_precision: Matrix,
        current_mean: Vector,
        current_precision: Matrix,
        likelihood_scale: f64,
    ) -> Option<Self> {
        if !likelihood_scale.is_finite() || likelihood_scale <= 0. {
            return None;
        }
        let updated: Matrix = std::array::from_fn(|a| {
            std::array::from_fn(|b| {
                if likelihood_scale == 1. {
                    self.precision[a][b] + current_precision[a][b] - anchor_precision[a][b]
                } else {
                    likelihood_scale * (self.precision[a][b] - anchor_precision[a][b])
                        + current_precision[a][b]
                }
            })
        });
        let (covariance, _) = precision(updated, 2)?;
        let linear: Vector = std::array::from_fn(|a| {
            (0..2)
                .map(|b| {
                    if likelihood_scale == 1. {
                        self.precision[a][b] * self.mean[b]
                            + current_precision[a][b] * current_mean[b]
                            - anchor_precision[a][b] * anchor_mean[b]
                    } else {
                        likelihood_scale
                            * (self.precision[a][b] * self.mean[b]
                                - anchor_precision[a][b] * anchor_mean[b])
                            + current_precision[a][b] * current_mean[b]
                    }
                })
                .sum()
        });
        let mean: Vector =
            std::array::from_fn(|a| (0..2).map(|b| covariance[a][b] * linear[b]).sum());
        // The conditional covariance must have a positive Cholesky factor.
        let l00 = libm::sqrt(covariance[0][0]);
        let l10 = covariance[1][0] / l00;
        let l11 = libm::sqrt(covariance[1][1] - l10 * l10);
        if mean.iter().any(|v| !v.is_finite()) || !l11.is_finite() || l11 <= 0. {
            return None;
        }
        Some(Self {
            mean,
            precision: updated,
        })
    }

    #[cfg(test)]
    fn log_kernel(&self, phi: Vector) -> f64 {
        let d = [phi[0] - self.mean[0], phi[1] - self.mean[1]];
        -0.5 * (d[0] * (self.precision[0][0] * d[0] + self.precision[0][1] * d[1])
            + d[1] * (self.precision[1][0] * d[0] + self.precision[1][1] * d[1]))
    }
}

#[derive(Clone, Copy)]
struct ConditionalModeCache {
    approximation: JointProposal,
    population_mean: Vector,
    prior_precision: Matrix,
    variance: f64,
}

fn common_additive_variance(subject: &GaussianObjectiveRequest) -> Option<f64> {
    let first = subject.observations.first()?;
    let sd = first.error.additive_sd.value;
    if !sd.is_finite()
        || sd <= 0.
        || subject.observations.iter().any(|o| {
            o.error.model != GaussianErrorModel::Normal
                || o.error.proportional_sd != 0.
                || o.error.additive_sd.value != sd
                || o.error.additive_sd.unit != first.error.additive_sd.unit
        })
    {
        return None;
    }
    let variance = sd * sd;
    variance.is_finite().then_some(variance)
}

impl ConditionalModeCache {
    fn from_mode(
        theta: Vector,
        omega: Matrix,
        eta: Vector,
        hessian: Matrix,
        variance: f64,
    ) -> Option<Self> {
        if theta.iter().any(|v| !v.is_finite() || *v <= 0.)
            || !variance.is_finite()
            || variance <= 0.
        {
            return None;
        }
        let population_mean = theta.map(libm::log);
        let mean = std::array::from_fn(|a| population_mean[a] + eta[a]);
        let prior_precision = precision(omega, 2)?.0;
        let covariance = precision(hessian, 2)?.0;
        let l00 = libm::sqrt(covariance[0][0]);
        let l10 = covariance[1][0] / l00;
        let l11 = libm::sqrt(covariance[1][1] - l10 * l10);
        if mean.iter().any(|v| !v.is_finite()) || !l11.is_finite() || l11 <= 0. {
            return None;
        }
        Some(Self {
            approximation: JointProposal {
                mean,
                precision: hessian,
            },
            population_mean,
            prior_precision,
            variance,
        })
    }

    fn predict(
        self,
        theta: Vector,
        omega: Matrix,
        variance: f64,
        eta_bound: f64,
    ) -> Option<Vector> {
        if theta.iter().any(|v| !v.is_finite() || *v <= 0.)
            || !variance.is_finite()
            || variance <= 0.
        {
            return None;
        }
        let mean = theta.map(libm::log);
        let updated = self.approximation.reweighted_likelihood(
            self.population_mean,
            self.prior_precision,
            mean,
            precision(omega, 2)?.0,
            self.variance / variance,
        )?;
        let eta: Vector = std::array::from_fn(|a| updated.mean[a] - mean[a]);
        eta.iter()
            .all(|v| v.is_finite() && v.abs() + 0.005 < eta_bound)
            .then_some(eta)
    }
}

#[cfg(test)]
mod conditional_mode_cache_tests {
    use super::*;
    #[test]
    fn predicted_modes_match_independent_gaussian_conditionals() {
        let fixtures: serde_json::Value = serde_json::from_str(&crate::test_fixture::text(
            include_bytes!("../../../tests/fixtures/conditional_mode_warm_starts.json.gz"),
        ))
        .unwrap();
        for case in fixtures["cases"].as_array().unwrap() {
            let old_mean: Vector = serde_json::from_value(case["old_mean"].clone()).unwrap();
            let new_mean: Vector = serde_json::from_value(case["new_mean"].clone()).unwrap();
            let old_omega: Matrix = serde_json::from_value(case["old_omega"].clone()).unwrap();
            let new_omega: Matrix = serde_json::from_value(case["new_omega"].clone()).unwrap();
            let hessian: Matrix =
                serde_json::from_value(case["proposal0_precision"].clone()).unwrap();
            let old_mode: Vector = serde_json::from_value(case["proposal0_mean"].clone()).unwrap();
            let expected: Vector = serde_json::from_value(case["expected_mean"].clone()).unwrap();
            let cache = ConditionalModeCache::from_mode(
                old_mean.map(libm::exp),
                old_omega,
                std::array::from_fn(|a| old_mode[a] - old_mean[a]),
                hessian,
                case["old_sigma"].as_f64().unwrap().powi(2),
            )
            .unwrap();
            let mode = cache
                .predict(
                    new_mean.map(libm::exp),
                    new_omega,
                    case["new_sigma"].as_f64().unwrap().powi(2),
                    5.,
                )
                .unwrap();
            for a in 0..2 {
                assert!((mode[a] + new_mean[a] - expected[a]).abs() <= 1e-10);
            }
            assert!(cache
                .predict(new_mean.map(libm::exp), new_omega, 0.01, 0.004)
                .is_none());
        }
    }
    #[test]
    fn censored_common_error_enables_only_checked_initialization() {
        let fixtures: serde_json::Value = serde_json::from_str(&crate::test_fixture::text(
            include_bytes!("../../../tests/fixtures/conditional_mode_warm_starts.json.gz"),
        ))
        .unwrap();
        let subject: GaussianObjectiveRequest =
            serde_json::from_value(fixtures["uncensored_subject"].clone()).unwrap();
        let censored: GaussianObjectiveRequest =
            serde_json::from_value(fixtures["censored_subject"].clone()).unwrap();
        assert!(common_additive_variance(&subject).is_some());
        assert!(common_additive_variance(&censored).is_some());
        let mut changed = subject.clone();
        changed.observations[0].error.proportional_sd = 0.1;
        assert!(common_additive_variance(&changed).is_none());
        changed = subject.clone();
        changed.observations[0].error.additive_sd.value *= 2.;
        assert!(common_additive_variance(&changed).is_none());
        changed = subject;
        changed.observations[0].error.additive_sd.value = 0.;
        assert!(common_additive_variance(&changed).is_none());
    }
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
        let d = setup.dimensions;
        self.saem_predictions_slices(
            subject_index,
            subject,
            &setup.names[..d],
            &setup.units[..d],
            &theta[..d],
            &eta[..d],
        )
    }

    /// Row-aligned population (eta zero) and individual predictions.
    fn saem_predictions_slices(
        self: &Arc<Self>,
        subject_index: usize,
        subject: &GaussianObjectiveRequest,
        names: &[String],
        units: &[String],
        theta: &[f64],
        eta: &[f64],
    ) -> Result<Vec<PopulationFittedObservation>, Error> {
        let mut population = subject.simulation.run.clone();
        let mut individual = subject.simulation.run.clone();
        for axis in 0..names.len() {
            let name = names[axis].clone();
            let unit = units[axis].clone();
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

    fn saem_joint(
        self: &Arc<Self>,
        subject: &GaussianObjectiveRequest,
        work: &mut EvaluationWork<'_>,
        point: PopulationPoint,
        eta: Vector,
    ) -> Result<Option<f64>, Error> {
        let setup = work.setup;
        let budget = work.budget;
        let PopulationPoint { theta, omega } = point;
        if *work.evaluations >= budget {
            return Ok(None);
        }
        if eta[..setup.dimensions]
            .iter()
            .any(|x| !x.is_finite() || x.abs() >= 0.999 * 12.)
        {
            return Ok(None);
        }
        *work.evaluations += 1;
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
        work: &mut EvaluationWork<'_>,
        point: PopulationPoint,
        eta: Vector,
    ) -> Result<Option<Vector>, Error> {
        let setup = work.setup;
        let budget = work.budget;
        let PopulationPoint { theta, omega } = point;
        if *work.evaluations >= budget {
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
        *work.evaluations += 1;
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
        work: &mut EvaluationWork<'_>,
        point: PopulationPoint,
        eta: Vector,
        h: f64,
    ) -> Result<Option<(Vector, Matrix)>, Error> {
        let setup = work.setup;
        let PopulationPoint { theta, omega } = point;
        let Some(gradient) =
            self.saem_joint_gradient(subject, work, PopulationPoint { theta, omega }, eta)?
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
                self.saem_joint_gradient(subject, work, PopulationPoint { theta, omega }, plus)?
            else {
                return Ok(None);
            };
            let Some(minus) =
                self.saem_joint_gradient(subject, work, PopulationPoint { theta, omega }, minus)?
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
        work: &mut EvaluationWork<'_>,
        point: PopulationPoint,
        etas: &[Vector],
        subject_multipliers: Option<&[f64]>,
    ) -> Result<Option<(f64, Vec<Vector>)>, Error> {
        let PopulationPoint { theta, omega } = point;
        self.saem_marginal_ofv_with_cache(
            request,
            work,
            PopulationPoint { theta, omega },
            etas,
            subject_multipliers,
            None,
        )
    }

    fn saem_marginal_ofv_with_cache(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        work: &mut EvaluationWork<'_>,
        point: PopulationPoint,
        etas: &[Vector],
        subject_multipliers: Option<&[f64]>,
        mut mode_cache: Option<&mut [Option<ConditionalModeCache>]>,
    ) -> Result<Option<(f64, Vec<Vector>)>, Error> {
        let setup = work.setup;
        let PopulationPoint { theta, omega } = point;
        let dimensions = setup.dimensions;
        let precise_modes = dimensions == 2
            && request
                .subjects
                .iter()
                .any(|subject| subject.observations.iter().any(|o| o.censoring.is_some()));
        let mut marginal = 0.;
        let mut modes = Vec::with_capacity(request.subjects.len());
        for (index, subject) in request.subjects.iter().enumerate() {
            let theta = [
                theta[0] * subject_multipliers.map_or(1., |values| values[index]),
                theta[1],
            ];
            let mut eta = etas[index];
            let mut best =
                match self.saem_joint(subject, work, PopulationPoint { theta, omega }, eta)? {
                    Some(value) => value,
                    None => return Ok(None),
                };
            let mode_variance = if mode_cache.is_some() && dimensions == 2 {
                common_additive_variance(subject)
            } else {
                None
            };
            let prediction = mode_cache
                .as_deref()
                .and_then(|cache| cache.get(index))
                .copied()
                .flatten()
                .and_then(|cache| {
                    mode_variance.and_then(|variance| {
                        cache.predict(theta, omega, variance, request.eta_bound)
                    })
                });
            if let Some(seed) = prediction.filter(|seed| *seed != eta) {
                if let Some(value) =
                    self.saem_joint(subject, work, PopulationPoint { theta, omega }, seed)?
                {
                    if value < best {
                        eta = seed;
                        best = value;
                    }
                }
            }
            let h = 0.005;
            let curvature_step = if precise_modes { 0.0003125 } else { h };
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
                    work,
                    PopulationPoint { theta, omega },
                    eta,
                    curvature_step,
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
                for value in &mut next[..dimensions] {
                    *value = value.clamp(-request.eta_bound + 2. * h, request.eta_bound - 2. * h);
                }
                if next == eta {
                    break;
                }
                // Near a conditional mode, scalar objective noise can obscure
                // the quadratic improvement. Polish the stationary root using
                // its actual score merit; observed curvature is checked again.
                if precise_modes
                    && (0..dimensions).all(|axis| (next[axis] - eta[axis]).abs() <= 1e-5)
                {
                    let Some(trial_gradient) = self.saem_joint_gradient(
                        subject,
                        work,
                        PopulationPoint { theta, omega },
                        next,
                    )?
                    else {
                        return Ok(None);
                    };
                    let old_norm = gradient[..dimensions]
                        .iter()
                        .map(|g| g.abs())
                        .fold(0_f64, f64::max);
                    let new_norm = trial_gradient[..dimensions]
                        .iter()
                        .map(|g| g.abs())
                        .fold(0_f64, f64::max);
                    if new_norm.is_finite() && new_norm < 0.5 * old_norm {
                        let Some(value) =
                            self.saem_joint(subject, work, PopulationPoint { theta, omega }, next)?
                        else {
                            return Ok(None);
                        };
                        eta = next;
                        best = value;
                        continue;
                    }
                }
                let newton = next;
                let mut accepted = false;
                for _ in 0..12 {
                    let candidate = match self.saem_joint(
                        subject,
                        work,
                        PopulationPoint { theta, omega },
                        next,
                    )? {
                        Some(value) => value,
                        None => return Ok(None),
                    };
                    let roundoff = 64. * f64::EPSILON * (1. + best.abs());
                    let slope: f64 = (0..dimensions)
                        .map(|axis| gradient[axis] * (next[axis] - eta[axis]))
                        .sum();
                    if candidate < best || (slope < 0. && candidate <= best + roundoff) {
                        best = candidate;
                        accepted = true;
                        break;
                    }
                    for axis in 0..dimensions {
                        next[axis] = 0.5 * (eta[axis] + next[axis]);
                    }
                }
                if !accepted {
                    // Near a conditional mode the quadratic decrease of the
                    // joint objective can fall below ODE-tolerance noise, so
                    // a value-based line search stalls with an unconverged
                    // mode. The Laplace log-determinant is first-order in
                    // that residual, so accept the small Newton step on its
                    // score merit instead, as the censored path already does.
                    if (0..dimensions).all(|axis| (newton[axis] - eta[axis]).abs() <= 1e-5) {
                        let Some(trial_gradient) = self.saem_joint_gradient(
                            subject,
                            work,
                            PopulationPoint { theta, omega },
                            newton,
                        )?
                        else {
                            return Ok(None);
                        };
                        let old_norm = gradient[..dimensions]
                            .iter()
                            .map(|g| g.abs())
                            .fold(0_f64, f64::max);
                        let new_norm = trial_gradient[..dimensions]
                            .iter()
                            .map(|g| g.abs())
                            .fold(0_f64, f64::max);
                        if new_norm.is_finite() && new_norm < 0.5 * old_norm {
                            let Some(value) = self.saem_joint(
                                subject,
                                work,
                                PopulationPoint { theta, omega },
                                newton,
                            )?
                            else {
                                return Ok(None);
                            };
                            eta = newton;
                            best = value;
                            continue;
                        }
                    }
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
                    work,
                    PopulationPoint { theta, omega },
                    eta,
                    curvature_step,
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
                || (precise_modes && gradient[..dimensions].iter().any(|g| g.abs() > 1e-8))
                || eta[..dimensions]
                    .iter()
                    .any(|x| x.abs() >= request.eta_bound - h)
            {
                return Ok(None);
            }
            if let (Some(cache), Some(variance)) = (mode_cache.as_deref_mut(), mode_variance) {
                cache[index] =
                    ConditionalModeCache::from_mode(theta, omega, eta, hessian, variance);
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
        work: &mut EvaluationWork<'_>,
        point: PopulationPoint,
        refinement: MarginalRefinement<'_>,
    ) -> Result<RefinedSaemPoint, Error> {
        let setup = work.setup;
        let PopulationPoint { theta, omega } = point;
        let MarginalRefinement {
            residual,
            center,
            modes: etas,
            max_iterations,
        } = refinement;
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
        let scales = [
            1.,
            1.,
            omega[0][0].max(0.01),
            omega[1][1].max(0.01),
            1.,
            variance.max(0.01),
        ];
        let cache_enabled = request
            .subjects
            .iter()
            .all(|subject| common_additive_variance(subject).is_some());
        let mut mode_cache = vec![None; request.subjects.len()];
        // Cached quadratics only predict initial modes, including for censoring.
        // Every candidate still evaluates its actual censored likelihood and the
        // existing conditional score and observed-curvature certificates.
        let mut evaluate = |candidate: [f64; 6], warm_modes: &[Vector]| {
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
            self.saem_marginal_ofv_with_cache(
                subject_request,
                work,
                PopulationPoint {
                    theta: candidate_theta,
                    omega: candidate_omega,
                },
                warm_modes,
                None,
                cache_enabled.then_some(mode_cache.as_mut_slice()),
            )
        };
        let mut refined = refine_marginal_box(
            MarginalBoxPoint {
                point,
                value: center,
                modes: etas.to_vec(),
                certified: false,
                iterations: 0,
            },
            lower,
            upper,
            scales,
            2. * request.gradient_tolerance * request.subjects.len() as f64,
            max_iterations,
            &mut evaluate,
        )?;
        let censored_error = residual.is_some()
            && request
                .subjects
                .iter()
                .any(|subject| subject.observations.iter().any(|o| o.censoring.is_some()));
        let remaining = max_iterations.saturating_sub(refined.iterations);
        if censored_error
            && refined.certified
            && remaining > 0
            && refined.point[4].abs() >= MAX_SAEM_CORRELATION - 16. * f64::EPSILON
        {
            // Correlation-bound local solutions receive one deterministic
            // decorrelation restart, selected by the identical objective.
            // It shares the original evaluation and total iteration budgets.
            let mut alternate = refined.point;
            alternate[4] = 0.;
            if let Some((value, modes)) = evaluate(alternate, &refined.modes)? {
                let alternate_scales = [
                    1.,
                    1.,
                    alternate[2].max(0.01),
                    alternate[3].max(0.01),
                    1.,
                    alternate[5].max(0.01),
                ];
                let mut second = refine_marginal_box(
                    MarginalBoxPoint {
                        point: alternate,
                        value,
                        modes,
                        certified: false,
                        iterations: 0,
                    },
                    lower,
                    upper,
                    alternate_scales,
                    2. * request.gradient_tolerance * request.subjects.len() as f64,
                    remaining,
                    &mut evaluate,
                )?;
                let total_iterations = refined.iterations + second.iterations;
                if second.value < refined.value {
                    second.iterations = total_iterations;
                    refined = second;
                } else {
                    refined.iterations = total_iterations;
                }
            }
        }
        point = refined.point;
        let value = refined.value;
        let modes = refined.modes;
        let certified = refined.certified;
        let iterations = refined.iterations;
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

    fn saem_one_effect_point_refined(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        work: &mut EvaluationWork<'_>,
        theta: f64,
        omega: f64,
        covariates: CovariateDesign<'_>,
        refinement: MarginalRefinement<'_>,
    ) -> Result<MarginalBoxPoint, Error> {
        let setup = work.setup;
        let CovariateDesign { beta, x } = covariates;
        let MarginalRefinement {
            residual,
            center,
            modes,
            max_iterations,
        } = refinement;
        let mut point = [0.; 6];
        let mut lower = [0.; 6];
        let mut upper = [0.; 6];
        let mut scales = [1.; 6];
        point[0] = libm::log(theta);
        point[1] = omega;
        lower[0] = libm::log(setup.lower[0]);
        upper[0] = libm::log(setup.upper[0]);
        lower[1] = setup.omega_lower[0];
        upper[1] = setup.omega_upper[0];
        scales[1] = omega.max(0.01);
        if let Some(fit) = residual {
            point[2] = fit.variance;
            lower[2] = fit.lower_variance;
            upper[2] = fit.upper_variance;
            scales[2] = fit.variance.max(1e-6);
        } else if let Some(effect) = &request.covariate_effect {
            for (index, effect) in std::iter::once(effect)
                .chain(effect.additional.iter())
                .enumerate()
            {
                point[2 + index] = beta[index];
                lower[2 + index] = effect.bounds[0];
                upper[2 + index] = effect.bounds[1];
            }
        }
        refine_marginal_box(
            MarginalBoxPoint {
                point,
                value: center,
                modes: modes.to_vec(),
                certified: false,
                iterations: 0,
            },
            lower,
            upper,
            scales,
            2. * request.gradient_tolerance * request.subjects.len() as f64,
            max_iterations,
            |candidate, warm_modes| {
                let multipliers = x
                    .iter()
                    .map(|row| libm::exp(dot(&candidate[2..2 + beta.len()], row)))
                    .collect::<Vec<_>>();
                let mut modified;
                let fitted_request = if let Some(fit) = residual {
                    modified = request.clone();
                    for subject in &mut modified.subjects {
                        for observation in &mut subject.observations {
                            observation.error.additive_sd = qspquest_core::model::Quantity {
                                value: libm::sqrt(candidate[2]),
                                unit: fit.unit.clone(),
                            };
                        }
                    }
                    &modified
                } else {
                    request
                };
                self.saem_marginal_ofv(
                    fitted_request,
                    work,
                    PopulationPoint {
                        theta: [libm::exp(candidate[0]), 0.],
                        omega: [[candidate[1], 0.], [0., 0.]],
                    },
                    warm_modes,
                    (!beta.is_empty()).then_some(multipliers.as_slice()),
                )
            },
        )
    }

    fn saem_one_effect_uncertainty(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        work: &mut EvaluationWork<'_>,
        theta: f64,
        omega: f64,
        sampled_etas: &[Vector],
        center: f64,
    ) -> Result<PopulationFitUncertainty, Error> {
        let setup = work.setup;
        let budget = work.budget;
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
                work,
                PopulationPoint {
                    theta: [candidate_theta, 0.],
                    omega: [[candidate_omega, 0.], [0., 0.]],
                },
                sampled_etas,
                None,
            )?
            .map(|(ofv, _)| ofv)
            .ok_or_else(|| {
                observed_covariance_failure(
                    *work.evaluations,
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
        work: &mut EvaluationWork<'_>,
        point: PopulationPoint,
        sampled_etas: &[Vector],
        _center: f64,
    ) -> Result<PopulationFitUncertainty, Error> {
        let setup = work.setup;
        let budget = work.budget;
        let PopulationPoint { theta, omega } = point;
        // Curvature differentiates conditional sensitivity scores and then
        // the marginal objective. Tighten the integration error ceiling for
        // these nested derivatives without changing data or the work budget.
        let mut curvature_request = request.clone();
        for subject in &mut curvature_request.subjects {
            let regimen = &mut subject.simulation.run.regimen;
            let factor = (1e-12 / regimen.rtol).min(1.);
            regimen.rtol *= factor;
            if let Some(atol) = &mut regimen.atol {
                *atol *= factor;
            }
            if let Some(tolerances) = &mut regimen.absolute_tolerances {
                for tolerance in tolerances.values_mut() {
                    tolerance.value *= factor;
                }
            }
            for states in subject
                .simulation
                .derivative_absolute_tolerances
                .values_mut()
            {
                for tolerance in states.values_mut() {
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
        for (axis, row) in omega.iter().enumerate() {
            if setup.omega_lower[axis] < setup.omega_upper[axis] {
                axes.push((axis + 2, row[axis].abs().max(0.01)));
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
        // Cached local quadratics predict initial modes only. Every probe
        // still evaluates the actual likelihood and preserves the conditional
        // score and observed-curvature certificates.
        let cache_enabled = curvature_request
            .subjects
            .iter()
            .all(|subject| common_additive_variance(subject).is_some());
        let mut mode_cache = vec![None; curvature_request.subjects.len()];
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
            self.saem_marginal_ofv_with_cache(
                &curvature_request,
                work,
                PopulationPoint {
                    theta: candidate_theta,
                    omega: candidate_omega,
                },
                sampled_etas,
                None,
                cache_enabled.then_some(mode_cache.as_mut_slice()),
            )?
            .map(|(ofv, _)| ofv)
            .ok_or_else(|| {
                observed_covariance_failure(
                    *work.evaluations,
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
    /// Estimates one to eight positive lognormal effects. One- and
    /// two-effect fits within the marginal scope can refine the objective.
    pub fn fit_population_saem(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
    ) -> Result<PopulationFitResult, Error> {
        let mut result = match self.prepare_population_saem(request)? {
            PreparedSaem::One(prepared) => self.fit_population_saem_1d(request, *prepared),
            PreparedSaem::Two(prepared) => self.fit_population_saem_2d(request, *prepared),
            PreparedSaem::General(prepared) => self.fit_population_saem_general(request, *prepared),
        }?;
        if result.status == FitStatus::Converged && result.uncertainty.is_some() {
            // SAEM validates strictly positive fixed-effect bounds before fitting.
            let names = request
                .fixed_effects
                .iter()
                .map(|p| p.parameter.clone())
                .collect::<Vec<_>>();
            qspquest_core::population_intervals::attach_positive_fixed_effect_intervals(
                &mut result,
                &names,
            );
        }
        Ok(result)
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
        if request
            .sweeps
            .is_some_and(|sweeps| !(1..=chain::MAX_SWEEPS).contains(&sweeps))
        {
            return Err(invalid(format!(
                "SAEM sweeps must be between 1 and {}",
                chain::MAX_SWEEPS
            )));
        }
        match general::route(request) {
            general::Route::General => {
                return self
                    .prepare_population_saem_general(request, plan)
                    .map(|prepared| PreparedSaem::General(Box::new(prepared)))
            }
            general::Route::Two => {
                return self
                    .prepare_population_saem_2d(request, plan)
                    .map(|prepared| PreparedSaem::Two(Box::new(prepared)))
            }
            general::Route::One => {}
        }
        self.prepare_population_saem_1d(request, plan)
            .map(|prepared| PreparedSaem::One(Box::new(prepared)))
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
            beta,
        } = prepared;
        let count = subjects.len();
        let columns = covariates.len();
        let chain = self.saem_chain(
            request,
            chain::ChainInput {
                names: vec![cfg.parameter.clone()],
                units: vec![cfg.unit.clone()],
                start: vec![cfg.start],
                lower: vec![cfg.lower],
                upper: vec![cfg.upper],
                omega: vec![vec![cfg.omega]],
                omega_bounds: vec![[cfg.omega_lower, cfg.omega_upper]],
                covariates: covariates
                    .iter()
                    .zip(&beta)
                    .map(|(effect, initial)| chain::ChainCovariate {
                        axis: 0,
                        initial: *initial,
                        bounds: effect.bounds,
                    })
                    .collect(),
                x: x.clone(),
                additive: residual.as_ref(),
                proportional: None,
                residual_unit: residual.as_ref().map(|fit| fit.unit.as_str()),
                subjects: &subjects,
                burn_in,
                marginal_refinement: true,
                reserve: 3 * count,
            },
        )?;
        let mut status = chain.status;
        let mut theta = libm::exp(chain.log_theta[0]);
        let mut omega = chain.omega[0][0];
        let mut beta = chain.beta.clone();
        if let (Some(fit), Some(sd)) = (&mut residual, chain.additive_sd) {
            fit.variance = (sd * sd).clamp(fit.lower_variance, fit.upper_variance);
            set_residual_sd(&mut subjects, fit);
        }
        let mut iterations = chain.iterations;
        let mut evaluations = chain.evaluations;
        let mut marginal_refinement_iterations = 0_usize;
        let chain_diagnostics = chain.diagnostics;
        let chain_etas: Vec<Vector> = chain.sampled_etas.iter().map(|e| [e[0], 0.]).collect();

        // A converged marginal calculation also supplies conditional eta
        // modes for EBE and IPRED. At a tight work budget the sampled etas
        // remain available with an explicitly labeled complete-data surrogate.
        let final_request = residual.as_ref().map(|_| {
            let mut final_request = request.clone();
            final_request.subjects = subjects.clone();
            final_request
        });
        let fitted_request = final_request.as_ref().unwrap_or(request);
        let sampled_etas = chain_etas;
        let multipliers = x
            .iter()
            .map(|row| libm::exp(dot(&beta, row)))
            .collect::<Vec<_>>();
        let mut marginal = self.saem_marginal_ofv(
            fitted_request,
            &mut EvaluationWork {
                setup: &setup,
                evaluations: &mut evaluations,
                budget: request.max_evaluations - 3 * count,
            },
            PopulationPoint {
                theta: [theta, 0.],
                omega: [[omega, 0.], [0., 0.]],
            },
            &sampled_etas,
            (!covariates.is_empty()).then_some(multipliers.as_slice()),
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
                let refined = self.saem_one_effect_point_refined(
                    fitted_request,
                    &mut EvaluationWork {
                        setup: &setup,
                        evaluations: &mut evaluations,
                        budget: request.max_evaluations - 3 * count,
                    },
                    theta,
                    omega,
                    CovariateDesign { beta: &beta, x: &x },
                    MarginalRefinement {
                        residual: residual.as_ref(),
                        center: *ofv,
                        modes,
                        max_iterations: request.max_iterations.saturating_sub(iterations).min(64),
                    },
                )?;
                theta = libm::exp(refined.point[0]);
                omega = refined.point[1];
                beta.copy_from_slice(&refined.point[2..2 + columns]);
                if let Some(fit) = &mut residual {
                    fit.variance = refined.point[2];
                    set_residual_sd(&mut subjects, fit);
                }
                iterations += refined.iterations;
                marginal_refinement_iterations += refined.iterations;
                marginal = Some((refined.value, refined.modes));
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
        let multipliers = x
            .iter()
            .map(|row| libm::exp(dot(&beta, row)))
            .collect::<Vec<_>>();
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
                &mut EvaluationWork {
                    setup: &setup,
                    evaluations: &mut evaluations,
                    budget: request.max_evaluations - 3 * count,
                },
                theta,
                omega,
                &sampled_etas,
                marginal.as_ref().expect("converged marginal fit").0,
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
        Ok(PopulationFitResult {
            fixed_effect_intervals: None,
            fixed_effect_interval_error: None,
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
                proportional_sd: None,
            }),
            saem_diagnostics: Some(PopulationSaemDiagnostics {
                marginal_refinement_iterations,
                ..chain_diagnostics
            }),
            objective_kind: if marginal.is_some() {
                PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue
            } else {
                PopulationObjectiveKind::SaemCompleteDataSurrogate
            },
            objective: marginal.map(|(value, _)| value).unwrap_or(objective),
            evaluations,
            iterations,
            fit_request_hash: canonical_hash(
                &serde_json::json!({"algorithm":"saem_shared_engine_one_effect_marginal_v2","request":request}),
            )?,
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
        let chain = self.saem_chain(
            request,
            chain::ChainInput {
                names: setup.names.to_vec(),
                units: setup.units.to_vec(),
                start: setup.start.to_vec(),
                lower: setup.lower.to_vec(),
                upper: setup.upper.to_vec(),
                omega: setup.omega.iter().map(|row| row.to_vec()).collect(),
                omega_bounds: (0..2)
                    .map(|axis| [setup.omega_lower[axis], setup.omega_upper[axis]])
                    .collect(),
                covariates: Vec::new(),
                x: Vec::new(),
                additive: residual.as_ref(),
                proportional: None,
                residual_unit: residual.as_ref().map(|fit| fit.unit.as_str()),
                subjects: &subjects,
                burn_in,
                marginal_refinement: true,
                reserve: 3 * count,
            },
        )?;
        let mut status = chain.status;
        let mut theta = [libm::exp(chain.log_theta[0]), libm::exp(chain.log_theta[1])];
        let mut omega = [
            [chain.omega[0][0], chain.omega[0][1]],
            [chain.omega[1][0], chain.omega[1][1]],
        ];
        if let (Some(fit), Some(sd)) = (&mut residual, chain.additive_sd) {
            fit.variance = (sd * sd).clamp(fit.lower_variance, fit.upper_variance);
            set_residual_sd(&mut subjects, fit);
        }
        let mut iterations = chain.iterations;
        let mut evaluations = chain.evaluations;
        let mut marginal_refinement_iterations = 0_usize;
        let chain_diagnostics = chain.diagnostics;
        let sampled_etas: Vec<Vector> = chain.sampled_etas.iter().map(|e| [e[0], e[1]]).collect();
        let final_request = residual.as_ref().map(|_| {
            let mut updated = request.clone();
            updated.subjects = subjects.clone();
            updated
        });
        let fitted_request = final_request.as_ref().unwrap_or(request);
        let mut marginal = self.saem_marginal_ofv(
            fitted_request,
            &mut EvaluationWork {
                setup: &setup,
                evaluations: &mut evaluations,
                budget: request.max_evaluations - 3 * count,
            },
            PopulationPoint { theta, omega },
            &sampled_etas,
            None,
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
                    &mut EvaluationWork {
                        setup: &setup,
                        evaluations: &mut evaluations,
                        budget: request.max_evaluations - 3 * count,
                    },
                    PopulationPoint { theta, omega },
                    MarginalRefinement {
                        modes,
                        residual: residual.as_ref(),
                        center: *ofv,
                        max_iterations: request.max_iterations.saturating_sub(iterations).min(64),
                    },
                )?;
                theta = refined.theta;
                omega = refined.omega;
                if let Some(fit) = &mut residual {
                    fit.variance = refined.residual_variance;
                    set_residual_sd(&mut subjects, fit);
                }
                iterations += refined.iterations;
                marginal_refinement_iterations += refined.iterations;
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
                &mut EvaluationWork {
                    setup: &setup,
                    evaluations: &mut evaluations,
                    budget: request.max_evaluations - 3 * count,
                },
                PopulationPoint { theta, omega },
                &marginal.as_ref().expect("converged marginal fit").1,
                marginal.as_ref().expect("converged marginal fit").0,
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
        for (axis, &value) in theta.iter().enumerate() {
            fixed_effects.insert(
                setup.names[axis].clone(),
                qspquest_core::model::Quantity {
                    value,
                    unit: setup.units[axis].clone(),
                },
            );
        }
        Ok(PopulationFitResult {
            fixed_effect_intervals: None,
            fixed_effect_interval_error: None,
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
                proportional_sd: None,
            }),
            saem_diagnostics: Some(PopulationSaemDiagnostics {
                marginal_refinement_iterations,
                ..chain_diagnostics
            }),
            objective_kind: if marginal.is_some() {
                PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue
            } else {
                PopulationObjectiveKind::SaemCompleteDataSurrogate
            },
            objective: marginal.map(|(value, _)| value).unwrap_or(surrogate),
            evaluations,
            iterations,
            fit_request_hash: canonical_hash(
                &serde_json::json!({"algorithm":"saem_shared_engine_two_effect_marginal_v2","request":request}),
            )?,
        })
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
        let slices = |setup: &Setup| {
            let d = setup.dimensions;
            (
                setup.names[..d].to_vec(),
                setup.units[..d].to_vec(),
                setup.start[..d].to_vec(),
                setup.lower[..d].to_vec(),
                setup.upper[..d].to_vec(),
            )
        };
        let ((names, units, start, lower, upper), burn_in, residual, x, beta) = match prepared {
            PreparedSaem::One(p) => (slices(&p.setup), p.burn_in, p.residual, p.x, p.beta),
            PreparedSaem::Two(p) => (
                slices(&p.setup),
                p.burn_in,
                p.residual,
                Vec::new(),
                Vec::new(),
            ),
            PreparedSaem::General(p) => {
                let beta = p.covariates.iter().map(|c| c.initial).collect();
                (
                    (p.names, p.units, p.start, p.lower, p.upper),
                    p.burn_in,
                    p.additive,
                    p.x,
                    beta,
                )
            }
        };
        let coordinates = (0..names.len())
            .map(|i| {
                super::fit_planning::coordinate(
                    &names[i],
                    &units[i],
                    start[i],
                    lower[i],
                    upper[i],
                    qspquest_core::planning::FitSearchCoordinate::LogNaturalValue,
                    libm::log(start[i]),
                )
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut population = super::fit_planning::population(request);
        population.burn_in_iterations = Some(burn_in);
        population.centered_covariates = if beta.is_empty() { Vec::new() } else { x };
        population.initial_covariate_coefficients = beta;
        population.proportional_residual = request.proportional_error_fit.clone();
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
    fn log_kernel_matches_independent_gaussian_expansion() {
        let q = JointProposal {
            mean: [1., -0.5],
            precision: [[4., 1.], [1., 2.]],
        };
        // Independent expansion: delta = [-3, 3.5], quadratic = 36 - 21 + 24.5.
        assert!((q.log_kernel([-2., 3.]) + 19.75).abs() < 1e-12);
    }

    #[test]
    fn prior_rebase_matches_independent_gaussian_conditional_and_density_ratio() {
        let old_omega: Matrix = [[0.2, 0.03], [0.03, 0.1]];
        let new_omega: Matrix = [[0.001, -0.009], [-0.009, 0.1]];
        let old_mean: Vector = [0.2, -0.4];
        let new_mean: Vector = [0.4, -0.1];
        let q = JointProposal {
            mean: [0.2018192092169081, -0.2820103909965625],
            precision: [
                [17.23560209424084, 1.4293193717277486],
                [1.4293193717277488, 19.471204188481675],
            ],
        };
        let p0 = precision(old_omega, 2).unwrap().0;
        let p1 = precision(new_omega, 2).unwrap().0;
        let updated = q.rebased_prior(old_mean, p0, new_mean, p1).unwrap();
        let expected_mean: Vector = [0.40291787485328356, -0.1387935034553281];
        let expected_covariance: Matrix = [
            [0.0006296101739842496, -0.004869666392634797],
            [-0.004869666392634797, 0.053889469272496465],
        ];
        // The updated conditional covariance is the inverse of its precision.
        let covariance: Matrix = precision(updated.precision, 2).unwrap().0;
        for a in 0..2 {
            assert!((updated.mean[a] - expected_mean[a]).abs() < 1e-11);
            for b in 0..2 {
                assert!((covariance[a][b] - expected_covariance[a][b]).abs() < 1e-12);
            }
        }
        let mut constant = None;
        for x in [[-1., 0.], [0.4, -0.2], [2., 1.]] {
            let old_eta = [x[0] - old_mean[0], x[1] - old_mean[1]];
            let new_eta = [x[0] - new_mean[0], x[1] - new_mean[1]];
            let value = updated.log_kernel(x) - q.log_kernel(x)
                + prior_nll_vector(new_eta, new_omega, 2).unwrap()
                - prior_nll_vector(old_eta, old_omega, 2).unwrap();
            if let Some(previous) = constant {
                assert!(f64::abs(value - previous) < 1e-8);
            } else {
                constant = Some(value);
            }
        }
    }

    #[test]
    fn prior_rebase_refuses_nonpositive_cached_precision() {
        let q = JointProposal {
            mean: [0., 0.],
            precision: [[1., 0.], [0., 1.]],
        };
        assert!(q
            .rebased_prior(
                [0.; 2],
                [[1000., 0.], [0., 1000.]],
                [0.; 2],
                [[1., 0.], [0., 1.]]
            )
            .is_none());
    }
}

#[cfg(test)]
mod residual_proposal_tests {
    use super::*;
    #[test]
    fn residual_precision_rebase_matches_independent_gaussian_conditionals() {
        let cases: serde_json::Value = serde_json::from_str(&crate::test_fixture::text(
            include_bytes!("../../../tests/fixtures/gaussian_residual_proposals.json.gz"),
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let old_mean: Vector = serde_json::from_value(case["old_mean"].clone()).unwrap();
            let new_mean: Vector = serde_json::from_value(case["new_mean"].clone()).unwrap();
            let old_omega: Matrix = serde_json::from_value(case["old_omega"].clone()).unwrap();
            let new_omega: Matrix = serde_json::from_value(case["new_omega"].clone()).unwrap();
            let h: Matrix = serde_json::from_value(case["proposal0_precision"].clone()).unwrap();
            let mean: Vector = serde_json::from_value(case["proposal0_mean"].clone()).unwrap();
            let q = JointProposal { mean, precision: h };
            let updated = q
                .reweighted_likelihood(
                    old_mean,
                    precision(old_omega, 2).unwrap().0,
                    new_mean,
                    precision(new_omega, 2).unwrap().0,
                    case["likelihood_precision_factor"].as_f64().unwrap(),
                )
                .unwrap();
            let expected_mean: Vector =
                serde_json::from_value(case["expected_mean"].clone()).unwrap();
            let expected_covariance: Matrix =
                serde_json::from_value(case["expected_covariance"].clone()).unwrap();
            let actual_covariance = precision(updated.precision, 2).unwrap().0;
            for a in 0..2 {
                assert!((updated.mean[a] - expected_mean[a]).abs() <= 1e-10);
                for b in 0..2 {
                    assert!((actual_covariance[a][b] - expected_covariance[a][b]).abs() <= 1e-12);
                }
            }
        }
    }
    #[test]
    fn residual_precision_rebase_rejects_invalid_scale() {
        let q = JointProposal {
            mean: [0.; 2],
            precision: [[1., 0.], [0., 1.]],
        };
        for scale in [0., -1., f64::NAN, f64::INFINITY] {
            assert!(q
                .reweighted_likelihood(
                    [0.; 2],
                    [[1., 0.], [0., 1.]],
                    [0.; 2],
                    [[1., 0.], [0., 1.]],
                    scale
                )
                .is_none());
        }
    }
}

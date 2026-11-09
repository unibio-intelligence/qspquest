//! Bounded optimizers in unit-box coordinates.
//!
//! `projected_bfgs` is a projected inverse-BFGS method with Armijo
//! backtracking. `pattern_search` is a deterministic compass search with step
//! halving. Objective closures may return errors; the first error stops the
//! search and is returned to the caller, which keeps its own best point.
use super::common::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Converged,
    IterationLimit,
    LineSearchFailed,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub x: Vec<f64>,
    pub value: f64,
    pub iterations: usize,
    pub stop: Stop,
    pub message: String,
}

/// Admit an optimizer result only after a successful convergence stop.
pub fn require_converged(outcome: &Outcome, message: &str) -> Result<()> {
    if outcome.stop == Stop::Converged {
        Ok(())
    } else {
        super::common::fail_code(message, "optimizer")
    }
}

fn clamp_unit(x: &[f64]) -> Vec<f64> {
    x.iter().map(|v| v.clamp(0.0, 1.0)).collect()
}

/// Gradient in unit coordinates by central differences, one-sided at bounds.
pub fn finite_difference_gradient(
    x: &[f64],
    fx: f64,
    step: f64,
    value: &mut dyn FnMut(&[f64]) -> Result<f64>,
) -> Result<Vec<f64>> {
    let mut gradient = vec![0.0; x.len()];
    for i in 0..x.len() {
        let mut up = x.to_vec();
        let mut down = x.to_vec();
        if x[i] + step > 1.0 {
            down[i] = x[i] - step;
            gradient[i] = (fx - value(&down)?) / step;
        } else if x[i] - step < 0.0 {
            up[i] = x[i] + step;
            gradient[i] = (value(&up)? - fx) / step;
        } else {
            up[i] = x[i] + step;
            down[i] = x[i] - step;
            gradient[i] = (value(&up)? - value(&down)?) / (2.0 * step);
        }
    }
    Ok(gradient)
}

fn projected_gradient(x: &[f64], g: &[f64]) -> Vec<f64> {
    x.iter()
        .zip(g)
        .map(|(&xi, &gi)| {
            if (xi <= 0.0 && gi > 0.0) || (xi >= 1.0 && gi < 0.0) {
                0.0
            } else {
                gi
            }
        })
        .collect()
}

/// Minimize over the unit box. `evaluate` returns the value and gradient.
pub fn projected_bfgs(
    start: &[f64],
    evaluate: &mut dyn FnMut(&[f64]) -> Result<(f64, Vec<f64>)>,
    max_iterations: usize,
    ftol: f64,
    gtol: f64,
) -> Result<Outcome> {
    let n = start.len();
    let mut x = clamp_unit(start);
    let (mut f, mut g) = evaluate(&x)?;
    let mut h: Vec<Vec<f64>> = super::linalg::identity(n);
    let mut iterations = 0;
    let mut previous_active: Option<Vec<bool>> = None;
    loop {
        let pg = projected_gradient(&x, &g);
        let norm = pg.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        if norm <= gtol {
            return Ok(Outcome {
                x,
                value: f,
                iterations,
                stop: Stop::Converged,
                message: "projected gradient below tolerance".into(),
            });
        }
        if iterations >= max_iterations {
            return Ok(Outcome {
                x,
                value: f,
                iterations,
                stop: Stop::IterationLimit,
                message: "iteration limit reached".into(),
            });
        }
        let active: Vec<bool> = (0..n).map(|i| pg[i] == 0.0).collect();
        // The inverse-Hessian estimate of the full problem is not the inverse
        // of the free block; restart it whenever the active set changes.
        if previous_active.as_ref().is_some_and(|p| *p != active) {
            h = super::linalg::identity(n);
        }
        previous_active = Some(active.clone());
        let mut accepted = None;
        for attempt in 0..2 {
            let mut d = vec![0.0; n];
            for i in 0..n {
                if active[i] {
                    continue;
                }
                for j in 0..n {
                    if !active[j] {
                        d[i] -= h[i][j] * g[j];
                    }
                }
            }
            let slope: f64 = d.iter().zip(&g).map(|(a, b)| a * b).sum();
            if !(slope < 0.0) || attempt == 1 {
                h = super::linalg::identity(n);
                d = pg.iter().map(|v| -v).collect();
            }
            // Keep the first trial inside one box width.
            let largest = d.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            let mut t = if largest > 1.0 { 1.0 / largest } else { 1.0 };
            for _ in 0..40 {
                let trial =
                    clamp_unit(&x.iter().zip(&d).map(|(a, b)| a + t * b).collect::<Vec<_>>());
                let decrease: f64 = trial
                    .iter()
                    .zip(&x)
                    .zip(&g)
                    .map(|((a, b), c)| (a - b) * c)
                    .sum();
                if trial == x {
                    break;
                }
                let (ft, gt) = evaluate(&trial)?;
                if ft.is_finite() && ft <= f + 1e-4 * decrease {
                    accepted = Some((trial, ft, gt));
                    break;
                }
                t *= 0.5;
            }
            if accepted.is_some() {
                break;
            }
        }
        let Some((trial, ft, gt)) = accepted else {
            return Ok(Outcome {
                x,
                value: f,
                iterations,
                stop: Stop::LineSearchFailed,
                message: "line search found no sufficient decrease".into(),
            });
        };
        iterations += 1;
        let s: Vec<f64> = trial.iter().zip(&x).map(|(a, b)| a - b).collect();
        // Curvature pairs describe the free coordinates only.
        let y: Vec<f64> = (0..n)
            .map(|i| if active[i] { 0.0 } else { gt[i] - g[i] })
            .collect();
        let reduction = f - ft;
        let scale = f.abs().max(ft.abs()).max(1.0);
        x = trial;
        f = ft;
        g = gt;
        if reduction <= ftol * scale {
            return Ok(Outcome {
                x,
                value: f,
                iterations,
                stop: Stop::Converged,
                message: "relative reduction of the objective below tolerance".into(),
            });
        }
        let sy: f64 = s.iter().zip(&y).map(|(a, b)| a * b).sum();
        let ss: f64 = s.iter().map(|v| v * v).sum::<f64>().sqrt();
        let yy: f64 = y.iter().map(|v| v * v).sum::<f64>().sqrt();
        if sy > 1e-12 * ss * yy {
            let hy: Vec<f64> = (0..n)
                .map(|i| (0..n).map(|j| h[i][j] * y[j]).sum())
                .collect();
            let yhy: f64 = y.iter().zip(&hy).map(|(a, b)| a * b).sum();
            let rho = 1.0 / sy;
            for i in 0..n {
                for j in 0..n {
                    h[i][j] +=
                        (1.0 + yhy * rho) * rho * s[i] * s[j] - rho * (hy[i] * s[j] + s[i] * hy[j]);
                }
            }
        }
    }
}

/// Deterministic compass search over the unit box with step halving.
/// Converges when the step falls below `xtol`; one iteration is one sweep.
pub fn pattern_search(
    start: &[f64],
    value: &mut dyn FnMut(&[f64]) -> Result<f64>,
    max_iterations: usize,
    xtol: f64,
) -> Result<Outcome> {
    let mut x = clamp_unit(start);
    let mut f = value(&x)?;
    let mut step = 0.25;
    let mut iterations = 0;
    while iterations < max_iterations {
        if step < xtol {
            return Ok(Outcome {
                x,
                value: f,
                iterations,
                stop: Stop::Converged,
                message: "pattern step below tolerance".into(),
            });
        }
        iterations += 1;
        let mut improved = false;
        for i in 0..x.len() {
            for sign in [1.0, -1.0] {
                let mut trial = x.clone();
                trial[i] = (x[i] + sign * step).clamp(0.0, 1.0);
                if trial[i] == x[i] {
                    continue;
                }
                let ft = value(&trial)?;
                if ft < f {
                    x = trial;
                    f = ft;
                    improved = true;
                    break;
                }
            }
        }
        if !improved {
            step *= 0.5;
        }
    }
    let stop = if step < xtol {
        Stop::Converged
    } else {
        Stop::IterationLimit
    };
    Ok(Outcome {
        x,
        value: f,
        iterations,
        message: if stop == Stop::Converged {
            "pattern step below tolerance".into()
        } else {
            "iteration limit reached".into()
        },
        stop,
    })
}

/// Minimize a smooth function with an analytic gradient over a finite box in
/// natural coordinates.
pub fn minimize_bounded(
    start: &[f64],
    lower: &[f64],
    upper: &[f64],
    evaluate: &mut dyn FnMut(&[f64]) -> Result<(f64, Vec<f64>)>,
    max_iterations: usize,
) -> Result<Outcome> {
    let width: Vec<f64> = lower.iter().zip(upper).map(|(a, b)| b - a).collect();
    let to_natural = |z: &[f64]| -> Vec<f64> {
        z.iter()
            .zip(lower)
            .zip(&width)
            .map(|((z, l), w)| l + w * z)
            .collect()
    };
    let unit: Vec<f64> = start
        .iter()
        .zip(lower)
        .zip(&width)
        .map(|((x, l), w)| (x - l) / w)
        .collect();
    let mut wrapped = |z: &[f64]| -> Result<(f64, Vec<f64>)> {
        let (f, g) = evaluate(&to_natural(z))?;
        Ok((f, g.iter().zip(&width).map(|(g, w)| g * w).collect()))
    };
    let mut outcome = projected_bfgs(&unit, &mut wrapped, max_iterations, 1e-12, 1e-10)?;
    outcome.x = to_natural(&outcome.x);
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsuccessful_stops_are_not_admitted_as_convergence() {
        for (iterations, stop) in [(0, Stop::IterationLimit), (1, Stop::LineSearchFailed)] {
            // Failure injection: the reported gradient asks for a decrease the
            // flat objective cannot provide, so no line-search trial succeeds.
            let mut unresolved = |_: &[f64]| Ok((0.0, vec![1.0]));
            let result = projected_bfgs(&[0.5], &mut unresolved, iterations, 1e-12, 1e-10).unwrap();
            assert_eq!(result.stop, stop);
            let error = require_converged(&result, "residual-error fit failed").unwrap_err();
            assert_eq!(error.code, "optimizer");
        }
    }

    #[test]
    fn bfgs_finds_interior_and_boundary_minima() {
        let mut f = |x: &[f64]| -> Result<(f64, Vec<f64>)> {
            let a = x[0] - 0.3;
            let b = x[1] - 1.4;
            Ok((
                10.0 * a * a + b * b + 3.0 * a * b,
                vec![20.0 * a + 3.0 * b, 2.0 * b + 3.0 * a],
            ))
        };
        let out = projected_bfgs(&[0.9, 0.1], &mut f, 200, 1e-15, 1e-10).unwrap();
        assert_eq!(out.stop, Stop::Converged);
        require_converged(&out, "fit failed").unwrap();
        assert_eq!(out.x[1], 1.0);
        // With y fixed at 1, the minimum in x is 0.3 + 3*0.4/20.
        assert!((out.x[0] - 0.36).abs() < 1e-8, "{:?}", out.x);
    }

    #[test]
    fn pattern_search_converges_on_a_smooth_bowl() {
        let mut f =
            |x: &[f64]| -> Result<f64> { Ok((x[0] - 0.123).powi(2) + 2.0 * (x[1] - 0.77).powi(2)) };
        let out = pattern_search(&[0.5, 0.5], &mut f, 500, 1e-9).unwrap();
        assert_eq!(out.stop, Stop::Converged);
        assert!((out.x[0] - 0.123).abs() < 1e-8 && (out.x[1] - 0.77).abs() < 1e-8);
    }

    #[test]
    fn a_coordinate_reaching_its_bound_leaves_newton_steps_on_the_free_block() {
        // f = (x - 2)^2 + (y - 0.1)^2 + 1.2 (x - 2)(y - 0.1) on [0, 1]^2 has
        // its unconstrained minimum outside the box. The constrained minimum
        // is x = 1 with the free minimizer y = 0.1 - 0.6 (1 - 2) = 0.7.
        let mut quadratic = |z: &[f64]| {
            let (a, b) = (z[0] - 2.0, z[1] - 0.1);
            Ok((
                a * a + b * b + 1.2 * a * b,
                vec![2.0 * a + 1.2 * b, 2.0 * b + 1.2 * a],
            ))
        };
        let result = projected_bfgs(&[0.4, 0.2], &mut quadratic, 60, 1e-15, 1e-10).unwrap();
        assert_eq!(result.stop, Stop::Converged);
        assert_eq!(result.x[0], 1.0);
        assert!((result.x[1] - 0.7).abs() < 1e-8, "{:?}", result.x);
        assert!(result.iterations < 30, "{}", result.iterations);
    }
}

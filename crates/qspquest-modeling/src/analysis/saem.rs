//! The QSPQuest SAEM engine, shared by every population interface.
//!
//! Componentwise random-walk Metropolis–Hastings samples log individual
//! parameters, with proposal scales adapted towards 35% acceptance during
//! burn-in. Stochastic approximation tracks their first and second moments.
//! The fixed-effect and covariate M-step is bounded generalized least squares
//! with the full precision; the covariance M-step projects complete-data
//! moments onto the requested bounds and covariance structure; the residual-error M-step
//! minimizes a gain-weighted expected complete-data objective. Censored rows
//! enter that objective through the analytic conditional second moment of the
//! truncated normal residual.
//!
//! Subjects are evaluated through [`Cohort`], so native analysis requests and
//! `qspquest.fit` population requests run the same sampler and M-steps.
use super::common::*;
use super::control::{boundary, Control};
use super::linalg::{self, Matrix};
use super::rng::Stream;
use super::stats;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Smallest proposal scale, as a multiple of the effect SD. Precise data give
/// conditional distributions far narrower than Omega; the adaptation must be
/// able to follow them instead of stalling at a floor that rejects almost
/// every proposal.
pub const MIN_PROPOSAL_SCALE: f64 = 1e-3;
/// Largest proposal scale, as a multiple of the effect SD.
pub const MAX_PROPOSAL_SCALE: f64 = 3.0;

pub const MAX_EXPECTATION_ITEMS: usize = 2_000_000;

/// An exponential covariate effect on one random-effect axis.
#[derive(Clone, Debug)]
pub struct CovariateAxis {
    pub axis: usize,
    pub initial: f64,
    pub lower: f64,
    pub upper: f64,
}

/// One SAEM problem in log individual-parameter coordinates.
pub struct Problem {
    /// Natural-log initial fixed effects, one per random-effect axis.
    pub log_initial: Vec<f64>,
    pub log_bounds: Vec<(f64, f64)>,
    pub covariates: Vec<CovariateAxis>,
    /// Centred covariate values, one row per subject and one column per effect.
    pub x: Matrix,
    pub omega: Matrix,
    pub variance_bounds: Vec<[f64; 2]>,
    pub diagonal: bool,
    pub sweeps: usize,
    pub burn_in: usize,
    pub max_iterations: usize,
    /// Largest drift of the stability window that counts as stable.
    pub tolerance: f64,
    /// Optional limit on the median recent relative M-step update.
    pub update_tolerance: Option<f64>,
    /// Fitted residual SDs take the residual M-step; fixed ones are constant.
    pub error: ErrorSpec,
    /// Proposals whose |eta| exceeds this bound on an axis are rejected
    /// without evaluation.
    pub eta_bound: Option<f64>,
    /// Scale on which the stability window measures eta variances.
    pub variance_scale: VarianceScale,
    /// Largest absolute eta correlation; the covariance M-step projects
    /// larger correlations onto this limit.
    pub max_correlation: Option<f64>,
    /// When set, a stability check counts only if every subject's lag-one
    /// correlation over the post-burn-in chain is at most this value.
    pub mixing_lag_limit: Option<f64>,
}

/// Coordinate of each eta variance in the stability window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarianceScale {
    /// Log variance: relative change, whatever the variance.
    Log,
    /// Effect SD: absolute change of sqrt(omega), so a variance approaching a
    /// small lower bound is stable once its SD no longer changes.
    StandardDeviation,
}

/// One subject evaluation: its negative log likelihood and observed rows.
pub struct Draw {
    pub nll: f64,
    pub y: Vec<f64>,
    pub prediction: Vec<f64>,
    pub censored: Vec<bool>,
}

/// The subjects of one population fit.
pub trait Cohort {
    fn len(&self) -> usize;
    /// Evaluate subject `index` at natural-log individual parameters.
    ///
    /// When the problem fits residual error, `nll` must be [`likelihood`]
    /// under `error`, so the engine can re-score cached predictions after each
    /// residual M-step. Otherwise the cohort may apply its own fixed per-row
    /// residual models, and `error` is unused.
    fn evaluate(&self, index: usize, log_values: &[f64], error: &ErrorValues) -> Result<Draw>;
}

pub fn stability_history(trace: &[TraceRecord], burn_in: usize) -> Option<Vec<Vec<f64>>> {
    // Retain at least one unit of cumulative post-burn-in stochastic gain.
    let mut selected = Vec::new();
    let mut span = 0.0;
    let mut later_gain: Option<f64> = None;
    for record in trace.iter().rev() {
        let post = record.iteration as i64 - burn_in as i64;
        if post <= 0 {
            break;
        }
        if let Some(g) = later_gain {
            span += g;
        }
        selected.push(record.coordinates.clone());
        later_gain = Some((post as f64).powf(-0.7));
        if selected.len() >= 50 && span >= 1.0 {
            selected.reverse();
            return Some(selected);
        }
    }
    None
}

fn stable_drift(history: &[Vec<f64>], tolerance: f64) -> bool {
    let end_count = history.len() * 2 / 5;
    let first = &history[..end_count];
    let last = &history[history.len() - end_count..];
    (0..history[0].len()).all(|coordinate| {
        let mean = |rows: &[Vec<f64>]| {
            rows.iter().map(|row| row[coordinate]).sum::<f64>() / rows.len() as f64
        };
        (mean(first) - mean(last)).abs() <= tolerance
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TraceRecord {
    pub iteration: usize,
    pub coordinates: Vec<f64>,
    pub fixed_effects: Vec<f64>,
    pub omega: Matrix,
    pub error: Value,
}

/// Complete restart state; JSON floats round-trip exactly.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct State {
    completed: usize,
    simulations: usize,
    status: String,
    coefficients: Vec<f64>,
    phi: Matrix,
    mean: Matrix,
    second: Vec<Matrix>,
    omega: Matrix,
    error: Value,
    prediction_cache: Vec<Vec<f64>>,
    likelihoods: Vec<f64>,
    proposed: Vec<u64>,
    accepted: Vec<u64>,
    subject_accepted: Vec<Vec<u64>>,
    proposal_scales: Vec<f64>,
    residual_mean_square: Option<f64>,
    expectation_predictions: Vec<Vec<f64>>,
    expectation_squares: Vec<Vec<f64>>,
    expectation_weights: Vec<f64>,
    trace: Vec<TraceRecord>,
    phi_history: Vec<Matrix>,
    stable_checks: usize,
    rng: u64,
    #[serde(default)]
    mixing: Mixing,
    #[serde(default)]
    joint: Joint,
}

/// Adaptive joint random-walk proposals for two or more effects.
///
/// Each subject's proposal covariance is its own chain covariance, estimated
/// during burn-in (adaptive Metropolis) and frozen afterwards, so correlated
/// conditional distributions are explored along their principal directions.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Joint {
    pub mean: Vec<Vec<f64>>,
    pub covariance: Vec<Matrix>,
    pub scale: f64,
    pub proposed: u64,
    pub accepted: u64,
}

/// Sampler counts reported by the fit request's diagnostics.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Mixing {
    /// Proposals whose subject was evaluated, including failed evaluations;
    /// proposals rejected at the eta bound are not evaluated.
    pub evaluated_proposals: u64,
    pub accepted_proposals: u64,
    /// Post-burn-in proposals per subject and axis.
    pub subject_proposed: Vec<Vec<u64>>,
    pub post_burn_iterations: usize,
    pub stall: Vec<usize>,
    pub max_stall: usize,
    /// Largest relative change of each completed M-step after burn-in.
    pub relative_changes: Vec<f64>,
    pub final_gain: Option<f64>,
    pub final_relative_change: Option<f64>,
}

impl Mixing {
    pub fn recent_window(&self) -> usize {
        self.relative_changes.len().min(20)
    }

    pub fn recent_relative_change_median(&self) -> Option<f64> {
        let window = self.recent_window();
        (window > 0)
            .then(|| stats::median(&self.relative_changes[self.relative_changes.len() - window..]))
    }
}

/// The sampler and M-step state when the engine stops.
pub struct Outcome {
    /// `converged`, `iteration_limit`, `evaluation_limit` or `mixing_failed`.
    pub status: String,
    /// Log fixed effects followed by covariate coefficients.
    pub coefficients: Vec<f64>,
    /// Current chain state, log individual parameters per subject.
    pub phi: Matrix,
    /// Stochastic-approximation posterior means of `phi`.
    pub mean: Matrix,
    pub omega: Matrix,
    pub error: ErrorValues,
    pub likelihoods: Vec<f64>,
    /// Post-burn-in proposals and acceptances per axis.
    pub proposed: Vec<u64>,
    pub accepted: Vec<u64>,
    pub subject_accepted: Vec<Vec<u64>>,
    pub trace: Vec<TraceRecord>,
    pub stable_checks: usize,
    pub max_lag_one: Vec<Option<f64>>,
    pub mixing: Mixing,
}

/// Location of each subject: fixed effects plus covariate effects.
pub fn location(problem: &Problem, coef: &[f64]) -> Matrix {
    let d = problem.log_initial.len();
    problem
        .x
        .iter()
        .map(|xi| {
            let mut row = coef[..d].to_vec();
            for (j, c) in problem.covariates.iter().enumerate() {
                row[c.axis] += xi[j] * coef[d + j];
            }
            row
        })
        .collect()
}

/// Expected squared residual of one row under the current residual model.
fn expected_square(y: f64, prediction: f64, censored: bool, e: &ErrorValues) -> f64 {
    let (r, sd) = if e.lognormal {
        (y.ln() - prediction.ln(), e.proportional)
    } else {
        (
            y - prediction,
            (e.additive.powi(2) + (e.proportional * prediction).powi(2)).sqrt(),
        )
    };
    if !censored {
        return r * r;
    }
    // Residual truncated above at the limit: E[Z^2 | Z < u] = 1 - u*phi(u)/Phi(u).
    let u = r / sd;
    let correction = if u == f64::INFINITY {
        0.0
    } else {
        u * stats::inverse_mills_lower(u)
    };
    sd * sd * (1.0 - correction)
}

/// Largest within-subject lag-one correlation per axis over the retained
/// post-burn-in chain, or `None` when no subject's chain moved.
fn lag_one(phi_history: &[Matrix], n: usize, d: usize) -> Vec<Option<f64>> {
    (0..d)
        .map(|axis| {
            let mut correlations = Vec::new();
            if phi_history.len() > 4 {
                for i in 0..n {
                    let a: Vec<f64> = phi_history[..phi_history.len() - 1]
                        .iter()
                        .map(|h| h[i][axis])
                        .collect();
                    let b: Vec<f64> = phi_history[1..].iter().map(|h| h[i][axis]).collect();
                    let (sa, sb) = (stats::variance(&a, 0).sqrt(), stats::variance(&b, 0).sqrt());
                    if sa > 0.0 && sb > 0.0 {
                        let (ma, mb) = (stats::mean(&a), stats::mean(&b));
                        let cov = a
                            .iter()
                            .zip(&b)
                            .map(|(x, y)| (x - ma) * (y - mb))
                            .sum::<f64>();
                        let norm = (a.iter().map(|x| (x - ma).powi(2)).sum::<f64>()
                            * b.iter().map(|y| (y - mb).powi(2)).sum::<f64>())
                        .sqrt();
                        correlations.push(cov / norm);
                    }
                }
            }
            correlations.iter().copied().reduce(f64::max)
        })
        .collect()
}

/// Project the complete-data covariance onto the requested structure and
/// variance bounds. A correlation cap shrinks off-diagonal entries together.
pub fn project_covariance(
    candidate: &Matrix,
    bounds: &[[f64; 2]],
    diagonal: bool,
    max_correlation: Option<f64>,
) -> Result<Matrix> {
    let d = candidate.len();
    let symmetric: Matrix = (0..d)
        .map(|a| {
            (0..d)
                .map(|b| {
                    if diagonal && a != b {
                        0.0
                    } else {
                        (candidate[a][b] + candidate[b][a]) / 2.0
                    }
                })
                .collect()
        })
        .collect();
    if (0..d).any(|j| !(symmetric[j][j] > 0.0)) {
        return fail_code(
            "SAEM covariance became singular; use an informative design",
            "identifiability",
        );
    }
    let scales: Vec<f64> = (0..d)
        .map(|j| (symmetric[j][j].clamp(bounds[j][0], bounds[j][1]) / symmetric[j][j]).sqrt())
        .collect();
    let mut om: Matrix = (0..d)
        .map(|a| {
            (0..d)
                .map(|b| symmetric[a][b] * scales[a] * scales[b])
                .collect()
        })
        .collect();
    if let Some(limit) = max_correlation {
        if d == 2 {
            // There is only one correlation to constrain.
            let bound = limit * (om[1][1] * om[0][0]).sqrt();
            let value = om[1][0].clamp(-bound, bound);
            om[1][0] = value;
            om[0][1] = value;
        } else {
            // Shrink off-diagonals together toward the diagonal. Independent
            // clipping can make a positive-definite matrix indefinite.
            let mut shrink = 1.0f64;
            for a in 0..d {
                for b in 0..a {
                    let bound = limit * om[a][a].sqrt() * om[b][b].sqrt();
                    if om[a][b].abs() > bound {
                        shrink = shrink.min(bound / om[a][b].abs());
                    }
                }
            }
            for a in 0..d {
                for b in 0..a {
                    om[a][b] *= shrink;
                    om[b][a] = om[a][b];
                }
            }
        }
    }
    if linalg::cholesky(&om).is_none() {
        return fail_code(
            "SAEM covariance is singular; no ridge was substituted",
            "identifiability",
        );
    }
    Ok(om)
}

/// Accumulate the complete-data sufficient statistic of a constant variance.
fn update_residual_mean(previous: Option<f64>, gain: f64, squares: &[f64]) -> f64 {
    let current = stats::mean(squares);
    match previous {
        Some(value) if gain < 1.0 => (1.0 - gain) * value + gain * current,
        _ => current,
    }
}

/// Closed-form M-step for a fitted constant residual variance.
fn constant_residual_sd(mean_square: f64, lower: f64, upper: f64) -> Result<f64> {
    if !mean_square.is_finite() || mean_square < 0.0 {
        return fail_code("residual-error M step failed", "optimizer");
    }
    Ok(mean_square.sqrt().clamp(lower, upper))
}

/// Residual-error M-step for normal error with a prediction-dependent variance.
pub fn residual_mstep(
    weights: &[f64],
    predictions: &[Vec<f64>],
    squares: &[Vec<f64>],
    current: ErrorValues,
    keys: &[&'static str],
    spec: &ErrorSpec,
) -> Result<ErrorValues> {
    let mut e = current;
    let total: f64 = weights
        .iter()
        .zip(predictions)
        .map(|(w, p)| w * p.len() as f64)
        .sum();
    let k = keys.len();
    let component: Vec<usize> = keys
        .iter()
        .map(|key| usize::from(*key == "proportional"))
        .collect();
    let lower: Vec<f64> = keys
        .iter()
        .map(|key| spec.sd(key).bounds().0.ln())
        .collect();
    let upper: Vec<f64> = keys
        .iter()
        .map(|key| spec.sd(key).bounds().1.ln())
        .collect();
    let mut x: Vec<f64> = keys
        .iter()
        .zip(lower.iter().zip(&upper))
        .map(|(key, (lo, hi))| current.get(key).ln().clamp(*lo, *hi))
        .collect();
    // Value, gradient and Hessian in log-SD coordinates. With variance
    // v = a^2 + (b p)^2 each row contributes t = ln(v)/2 + s/(2v).
    let evaluate = |x: &[f64]| {
        let mut point = current;
        for (key, value) in keys.iter().zip(x) {
            point.set(key, value.exp());
        }
        let a2 = point.additive.powi(2);
        let mut value = 0.0;
        let mut gradient = vec![0.0; k];
        let mut hessian = vec![vec![0.0; k]; k];
        for ((w, preds), rows) in weights.iter().zip(predictions).zip(squares) {
            let w = w / total;
            for (p, s) in preds.iter().zip(rows) {
                let b2 = (point.proportional * p).powi(2);
                let v = a2 + b2;
                value += w * (0.5 * v.ln() + 0.5 * s / v);
                let ratio = s / v;
                let shares = [a2 / v, b2 / v];
                for i in 0..k {
                    let share = shares[component[i]];
                    let score = (1.0 - ratio) * share;
                    gradient[i] += w * score;
                    for j in 0..k {
                        hessian[i][j] += w * (4.0 * ratio - 2.0) * share * shares[component[j]];
                    }
                    hessian[i][i] += 2.0 * w * score;
                }
            }
        }
        (value, gradient, hessian)
    };
    let (mut value, mut gradient, mut hessian) = evaluate(&x);
    for _ in 0..100 {
        let free: Vec<bool> = (0..k)
            .map(|i| {
                !((x[i] <= lower[i] && gradient[i] > 0.0)
                    || (x[i] >= upper[i] && gradient[i] < 0.0))
            })
            .collect();
        let projected = (0..k)
            .filter(|&i| free[i])
            .fold(0.0f64, |m, i| m.max(gradient[i].abs()));
        if !value.is_finite() || gradient.iter().any(|value| !value.is_finite()) {
            break;
        }
        if projected <= 1e-10 {
            for (key, value) in keys.iter().zip(&x) {
                e.set(key, value.exp());
            }
            return Ok(e);
        }
        let index: Vec<usize> = (0..k).filter(|&i| free[i]).collect();
        let block: Matrix = index
            .iter()
            .map(|&i| index.iter().map(|&j| hessian[i][j]).collect())
            .collect();
        let rhs: Vec<f64> = index.iter().map(|&i| gradient[i]).collect();
        let newton = linalg::cholesky(&block).map(|l| linalg::cholesky_solve(&l, &rhs));
        let mut direction = vec![0.0; k];
        for (slot, &i) in index.iter().enumerate() {
            direction[i] = match &newton {
                Some(step) => -step[slot],
                // Indefinite curvature: Newton steps where the curvature is
                // positive, and a unit log-SD step downhill where it is not.
                None if hessian[i][i] > 0.0 => -gradient[i] / hessian[i][i],
                None => -gradient[i].signum(),
            };
        }
        let mut step = 1.0;
        let mut moved = false;
        for _ in 0..60 {
            let trial: Vec<f64> = (0..k)
                .map(|i| (x[i] + step * direction[i]).clamp(lower[i], upper[i]))
                .collect();
            let decrease: f64 = (0..k).map(|i| gradient[i] * (trial[i] - x[i])).sum();
            let (trial_value, trial_gradient, trial_hessian) = evaluate(&trial);
            if trial_value.is_finite() && trial_value <= value + 1e-4 * decrease {
                let change = (0..k).fold(0.0f64, |m, i| m.max((trial[i] - x[i]).abs()));
                x = trial;
                value = trial_value;
                gradient = trial_gradient;
                hessian = trial_hessian;
                moved = true;
                if change <= 1e-12 {
                    for (key, value) in keys.iter().zip(&x) {
                        e.set(key, value.exp());
                    }
                    return Ok(e);
                }
                break;
            }
            step *= 0.5;
        }
        if !moved {
            // No representable decrease: stationary only if the projected
            // gradient is at the objective's rounding level.
            if projected <= 1e-8 * value.abs().max(1.0) {
                for (key, value) in keys.iter().zip(&x) {
                    e.set(key, value.exp());
                }
                return Ok(e);
            }
            break;
        }
    }
    fail_code("residual-error M step failed", "optimizer")
}

/// Exponentially weighted chain mean and covariance update with weight `w`.
pub fn adapt_chain_covariance(mean: &mut [f64], covariance: &mut Matrix, phi: &[f64], w: f64) {
    let d = phi.len();
    let delta: Vec<f64> = phi.iter().zip(mean.iter()).map(|(p, m)| p - m).collect();
    for a in 0..d {
        mean[a] += w * delta[a];
    }
    for a in 0..d {
        for b in 0..d {
            covariance[a][b] = (1.0 - w) * (covariance[a][b] + w * delta[a] * delta[b]);
        }
    }
}

/// Run SAEM. `context` is called once after the initial evaluations and
/// returns the checkpoint context; `operation` names the checkpoint stream.
#[allow(clippy::too_many_arguments)]
pub fn run(
    problem: &Problem,
    cohort: &dyn Cohort,
    mut rng: Stream,
    budget: &Budget,
    control: &mut dyn Control,
    operation: &str,
    context: &mut dyn FnMut() -> Result<Value>,
) -> Result<Outcome> {
    let d = problem.log_initial.len();
    let n = cohort.len();
    let m = problem.covariates.len();
    let max_iterations = problem.max_iterations;
    let burn_in = problem.burn_in;
    let err = problem.error;
    let error_keys = err.fitted_keys();
    let constant_error_key = if err.lognormal && error_keys == ["proportional"] {
        Some("proportional")
    } else if error_keys == ["additive"] && err.initial().proportional == 0.0 {
        Some("additive")
    } else {
        None
    };
    let mut om = problem.omega.clone();
    let mut coefficients: Vec<f64> = problem
        .log_initial
        .iter()
        .copied()
        .chain(problem.covariates.iter().map(|c| c.initial))
        .collect();
    let outer = |a: &[f64], b: &[f64]| -> Matrix {
        a.iter()
            .map(|ai| b.iter().map(|bj| ai * bj).collect())
            .collect()
    };
    let mut current_error = err.initial();
    let mut phi = location(problem, &coefficients);
    let mut mean = phi.clone();
    let mut second: Vec<Matrix> = phi
        .iter()
        .map(|p| {
            let o = outer(p, p);
            (0..d)
                .map(|a| (0..d).map(|b| o[a][b] + om[a][b]).collect())
                .collect()
        })
        .collect();
    let mut prediction_cache = Vec::with_capacity(n);
    let mut y_values = Vec::with_capacity(n);
    let mut censored_values = Vec::with_capacity(n);
    let mut likelihoods = Vec::with_capacity(n);
    for (i, p) in phi.iter().enumerate() {
        let draw = cohort.evaluate(i, p, &current_error)?;
        likelihoods.push(draw.nll);
        prediction_cache.push(draw.prediction);
        y_values.push(draw.y);
        censored_values.push(draw.censored);
    }
    let y_flat: Vec<f64> = y_values.concat();
    let censored_flat: Vec<bool> = censored_values.concat();
    let mut proposed = vec![0u64; d];
    let mut accepted = vec![0u64; d];
    let mut subject_accepted = vec![vec![0u64; d]; n];
    let mut proposal_scales = vec![0.8; d];
    let mut residual_mean_square = None;
    let mut expectation_predictions: Vec<Vec<f64>> = Vec::new();
    let mut expectation_squares: Vec<Vec<f64>> = Vec::new();
    let mut expectation_weights: Vec<f64> = Vec::new();
    let mut trace: Vec<TraceRecord> = Vec::new();
    let mut phi_history: Vec<Matrix> = Vec::new();
    let mut stable_checks = 0usize;
    let mut status = "iteration_limit".to_string();
    let mut start_iteration = 0;
    let mut mixing = Mixing {
        subject_proposed: vec![vec![0; d]; n],
        stall: vec![0; n],
        ..Mixing::default()
    };
    let mut joint = Joint {
        mean: phi.clone(),
        covariance: vec![om.clone(); n],
        scale: 0.5,
        ..Joint::default()
    };
    let context = context()?;
    if let Some(saved) = control.start(operation, &context)? {
        let saved: State = serde_json::from_value(saved).map_err(|e| {
            AnalysisError::new(
                "checkpoint",
                format!("population checkpoint state is invalid: {e}"),
            )
        })?;
        budget.restore(Some(&json!(saved.simulations)))?;
        coefficients = saved.coefficients;
        phi = saved.phi;
        mean = saved.mean;
        second = saved.second;
        om = saved.omega;
        current_error = ErrorValues::from_value(&saved.error)?;
        prediction_cache = saved.prediction_cache;
        likelihoods = saved.likelihoods;
        proposed = saved.proposed;
        accepted = saved.accepted;
        subject_accepted = saved.subject_accepted;
        proposal_scales = saved.proposal_scales;
        residual_mean_square = saved.residual_mean_square;
        expectation_predictions = saved.expectation_predictions;
        expectation_squares = saved.expectation_squares;
        expectation_weights = saved.expectation_weights;
        trace = saved.trace;
        phi_history = saved.phi_history;
        stable_checks = saved.stable_checks;
        rng = Stream::new(saved.rng);
        start_iteration = saved.completed;
        status = saved.status;
        if saved.mixing.subject_proposed.len() == n && saved.mixing.stall.len() == n {
            mixing = saved.mixing;
        }
        if saved.joint.mean.len() == n && saved.joint.covariance.len() == n {
            joint = saved.joint;
        }
        if phi.len() != n || prediction_cache.len() != n || likelihoods.len() != n || om.len() != d
        {
            return fail_code(
                "population checkpoint state does not match the cohort",
                "checkpoint",
            );
        }
    }
    macro_rules! state {
        ($completed:expr) => {
            || {
                serde_json::to_value(State {
                    completed: $completed,
                    simulations: budget.used(),
                    status: status.clone(),
                    coefficients: coefficients.clone(),
                    phi: phi.clone(),
                    mean: mean.clone(),
                    second: second.clone(),
                    omega: om.clone(),
                    error: current_error.to_value(),
                    prediction_cache: prediction_cache.clone(),
                    likelihoods: likelihoods.clone(),
                    proposed: proposed.clone(),
                    accepted: accepted.clone(),
                    subject_accepted: subject_accepted.clone(),
                    proposal_scales: proposal_scales.clone(),
                    residual_mean_square,
                    expectation_predictions: expectation_predictions.clone(),
                    expectation_squares: expectation_squares.clone(),
                    expectation_weights: expectation_weights.clone(),
                    trace: trace.clone(),
                    phi_history: phi_history.clone(),
                    stable_checks,
                    rng: rng.state(),
                    mixing: mixing.clone(),
                    joint: joint.clone(),
                })
                .expect("finite state")
            }
        };
    }
    macro_rules! checkpoint {
        ($completed:expr, $final:expr) => {
            boundary(
                control,
                operation,
                $completed,
                max_iterations,
                budget.used(),
                budget.maximum,
                $final,
                state!($completed),
            )?
        };
    }
    let record_iteration =
        |iteration: usize, coefficients: &[f64], om: &Matrix, e: &ErrorValues| -> TraceRecord {
            let mut coordinates: Vec<f64> = coefficients.to_vec();
            coordinates.extend((0..d).map(|j| match problem.variance_scale {
                VarianceScale::Log => om[j][j].ln(),
                VarianceScale::StandardDeviation => om[j][j].sqrt(),
            }));
            for i in 0..d {
                for j in 0..i {
                    coordinates.push(om[i][j]);
                }
            }
            coordinates.extend(error_keys.iter().map(|k| e.get(k).ln()));
            TraceRecord {
                iteration: iteration + 1,
                coordinates,
                fixed_effects: coefficients[..d].iter().map(|v| v.exp()).collect(),
                omega: om.clone(),
                error: e.to_value(),
            }
        };
    let end = if status != "converged" {
        max_iterations
    } else {
        start_iteration
    };
    for iteration in start_iteration..end {
        let loc = location(problem, &coefficients);
        let precision = linalg::inverse_spd(&om).ok_or_else(|| {
            AnalysisError::new(
                "identifiability",
                "SAEM covariance is singular; no ridge was substituted",
            )
        })?;
        let mut batch_proposed = vec![0u64; d];
        let mut batch_accepted = vec![0u64; d];
        let mut stopped = false;
        let post_burn = iteration >= burn_in;
        'subjects: for i in 0..n {
            let mut moved = false;
            for _ in 0..problem.sweeps {
                for axis in 0..d {
                    let mut candidate = phi[i].clone();
                    candidate[axis] += proposal_scales[axis] * om[axis][axis].sqrt() * rng.normal();
                    batch_proposed[axis] += 1;
                    if post_burn {
                        mixing.subject_proposed[i][axis] += 1;
                    }
                    if problem
                        .eta_bound
                        .is_some_and(|b| !((candidate[axis] - loc[i][axis]).abs() <= b))
                    {
                        continue;
                    }
                    let candidate_draw = match cohort.evaluate(i, &candidate, &current_error) {
                        Ok(draw) => {
                            mixing.evaluated_proposals += 1;
                            draw
                        }
                        Err(error) if recoverable(&error) => {
                            mixing.evaluated_proposals += 1;
                            continue;
                        }
                        Err(error) if error.code == "budget" => {
                            stopped = true;
                            break 'subjects;
                        }
                        Err(error) => return Err(error),
                    };
                    let old_eta: Vec<f64> =
                        phi[i].iter().zip(&loc[i]).map(|(a, b)| a - b).collect();
                    let new_eta: Vec<f64> =
                        candidate.iter().zip(&loc[i]).map(|(a, b)| a - b).collect();
                    let log_ratio = likelihoods[i] - candidate_draw.nll
                        + 0.5
                            * (linalg::quadratic_form(&old_eta, &precision, &old_eta)
                                - linalg::quadratic_form(&new_eta, &precision, &new_eta));
                    if rng.uniform().max(1e-300).ln() < log_ratio {
                        phi[i] = candidate;
                        likelihoods[i] = candidate_draw.nll;
                        prediction_cache[i] = candidate_draw.prediction;
                        batch_accepted[axis] += 1;
                        mixing.accepted_proposals += 1;
                        moved = true;
                        if post_burn {
                            subject_accepted[i][axis] += 1;
                        }
                    }
                }
                if d < 2 {
                    continue;
                }
                let Some(l) =
                    linalg::cholesky(&joint.covariance[i]).or_else(|| linalg::cholesky(&om))
                else {
                    continue;
                };
                let z: Vec<f64> = (0..d).map(|_| rng.normal()).collect();
                let step = linalg::mat_vec(&l, &z);
                let candidate: Vec<f64> = phi[i]
                    .iter()
                    .zip(&step)
                    .map(|(p, s)| p + joint.scale * s)
                    .collect();
                joint.proposed += 1;
                if problem.eta_bound.is_some_and(|b| {
                    candidate
                        .iter()
                        .zip(&loc[i])
                        .any(|(c, l)| !((c - l).abs() <= b))
                }) {
                    continue;
                }
                let candidate_draw = match cohort.evaluate(i, &candidate, &current_error) {
                    Ok(draw) => {
                        mixing.evaluated_proposals += 1;
                        draw
                    }
                    Err(error) if recoverable(&error) => {
                        mixing.evaluated_proposals += 1;
                        continue;
                    }
                    Err(error) if error.code == "budget" => {
                        stopped = true;
                        break 'subjects;
                    }
                    Err(error) => return Err(error),
                };
                let old_eta: Vec<f64> = phi[i].iter().zip(&loc[i]).map(|(a, b)| a - b).collect();
                let new_eta: Vec<f64> = candidate.iter().zip(&loc[i]).map(|(a, b)| a - b).collect();
                let log_ratio = likelihoods[i] - candidate_draw.nll
                    + 0.5
                        * (linalg::quadratic_form(&old_eta, &precision, &old_eta)
                            - linalg::quadratic_form(&new_eta, &precision, &new_eta));
                if rng.uniform().max(1e-300).ln() < log_ratio {
                    phi[i] = candidate;
                    likelihoods[i] = candidate_draw.nll;
                    prediction_cache[i] = candidate_draw.prediction;
                    joint.accepted += 1;
                    mixing.accepted_proposals += 1;
                    moved = true;
                }
            }
            if d >= 2 && iteration < burn_in {
                // Exponentially weighted chain covariance, adapted in burn-in only.
                let w = 1.0 / (iteration as f64 + 2.0);
                adapt_chain_covariance(&mut joint.mean[i], &mut joint.covariance[i], &phi[i], w);
            }
            if post_burn {
                mixing.stall[i] = if moved { 0 } else { mixing.stall[i] + 1 };
                mixing.max_stall = mixing.max_stall.max(mixing.stall[i]);
            }
        }
        if stopped {
            // Return no unfinalized prediction rows when the aggregate budget ends.
            status = "evaluation_limit".to_string();
            break;
        }
        if post_burn {
            mixing.post_burn_iterations += 1;
        }
        if iteration < burn_in && d >= 2 {
            let rate = joint.accepted as f64 / joint.proposed.max(1) as f64;
            joint.scale = (joint.scale * (rate - 0.25).clamp(-0.2, 0.2).exp())
                .clamp(MIN_PROPOSAL_SCALE, MAX_PROPOSAL_SCALE);
            joint.accepted = 0;
            joint.proposed = 0;
        }
        if iteration < burn_in {
            for a in 0..d {
                let rate = batch_accepted[a] as f64 / batch_proposed[a].max(1) as f64;
                proposal_scales[a] = (proposal_scales[a] * (rate - 0.35).clamp(-0.2, 0.2).exp())
                    .clamp(MIN_PROPOSAL_SCALE, MAX_PROPOSAL_SCALE);
            }
        } else {
            for a in 0..d {
                proposed[a] += batch_proposed[a];
                accepted[a] += batch_accepted[a];
            }
        }
        // Initial conditional chains equilibrate before their draws update
        // population covariance, covariates or residual-error statistics.
        if iteration < burn_in / 2 {
            trace.push(record_iteration(
                iteration,
                &coefficients,
                &om,
                &current_error,
            ));
            checkpoint!(iteration + 1, iteration + 1 == max_iterations);
            continue;
        }
        let gain = if iteration < burn_in {
            1.0
        } else {
            (iteration as f64 - burn_in as f64 + 1.0).powf(-0.7)
        };
        for i in 0..n {
            for a in 0..d {
                mean[i][a] += gain * (phi[i][a] - mean[i][a]);
                for b in 0..d {
                    second[i][a][b] += gain * (phi[i][a] * phi[i][b] - second[i][a][b]);
                }
            }
        }
        let previous_coefficients = coefficients.clone();
        let previous_omega = om.clone();
        let previous_error = current_error;
        // Bounded generalized least squares with the full precision, which
        // preserves cross-effect covariance in the fixed-effect update.
        let p_count = d + m;
        let mut h = vec![vec![0.0; p_count]; p_count];
        let mut g = vec![0.0; p_count];
        for i in 0..n {
            let mut design = vec![vec![0.0; p_count]; d];
            for a in 0..d {
                design[a][a] = 1.0;
            }
            for (j, c) in problem.covariates.iter().enumerate() {
                design[c.axis][d + j] = problem.x[i][j];
            }
            let pd = linalg::mat_mul(&precision, &design);
            let pm = linalg::mat_vec(&precision, &mean[i]);
            for r in 0..p_count {
                for c in 0..p_count {
                    h[r][c] += (0..d).map(|a| design[a][r] * pd[a][c]).sum::<f64>();
                }
                g[r] += (0..d).map(|a| design[a][r] * pm[a]).sum::<f64>();
            }
        }
        let lower: Vec<f64> = problem
            .log_bounds
            .iter()
            .map(|b| b.0)
            .chain(problem.covariates.iter().map(|c| c.lower))
            .collect();
        let upper: Vec<f64> = problem
            .log_bounds
            .iter()
            .map(|b| b.1)
            .chain(problem.covariates.iter().map(|c| c.upper))
            .collect();
        coefficients = linalg::bounded_least_squares(&h, &g, &lower, &upper, 200)
            .ok_or_else(|| AnalysisError::new("optimizer", "bounded covariate M step failed"))?;
        let loc = location(problem, &coefficients);
        let mut candidate = vec![vec![0.0; d]; d];
        for i in 0..n {
            for a in 0..d {
                for b in 0..d {
                    candidate[a][b] +=
                        second[i][a][b] - mean[i][a] * loc[i][b] - loc[i][a] * mean[i][b]
                            + loc[i][a] * loc[i][b];
                }
            }
        }
        for row in candidate.iter_mut() {
            for value in row.iter_mut() {
                *value /= n as f64;
            }
        }
        om = project_covariance(
            &candidate,
            &problem.variance_bounds,
            problem.diagonal,
            problem.max_correlation,
        )?;
        if !error_keys.is_empty() {
            let pred_flat: Vec<f64> = prediction_cache.concat();
            let squares: Vec<f64> = (0..pred_flat.len())
                .map(|j| expected_square(y_flat[j], pred_flat[j], censored_flat[j], &current_error))
                .collect();
            if squares
                .iter()
                .any(|square| !square.is_finite() || *square < 0.0)
            {
                return fail_code("SAEM residual second moment is invalid", "optimizer");
            }
            if let Some(key) = constant_error_key {
                let mean_square = update_residual_mean(residual_mean_square, gain, &squares);
                let (lower, upper) = err.sd(key).bounds();
                current_error.set(key, constant_residual_sd(mean_square, lower, upper)?);
                residual_mean_square = Some(mean_square);
            } else {
                if gain == 1.0 {
                    expectation_predictions.clear();
                    expectation_squares.clear();
                    expectation_weights.clear();
                }
                for w in expectation_weights.iter_mut() {
                    *w *= 1.0 - gain;
                }
                expectation_weights.push(gain);
                expectation_predictions.push(pred_flat);
                expectation_squares.push(squares);
                current_error = residual_mstep(
                    &expectation_weights,
                    &expectation_predictions,
                    &expectation_squares,
                    current_error,
                    &error_keys,
                    &err,
                )?;
            }
            for i in 0..n {
                likelihoods[i] = likelihood(
                    &y_values[i],
                    &prediction_cache[i],
                    &censored_values[i],
                    &current_error,
                )?
                .value;
            }
        }
        if post_burn {
            // Relative change on natural scales: fixed effects and variances
            // through their logs, covariate coefficients against 1 + |beta|.
            let mut change = 0.0f64;
            for a in 0..d {
                change = change.max((coefficients[a] - previous_coefficients[a]).exp_m1().abs());
                change = change.max((om[a][a] / previous_omega[a][a] - 1.0).abs());
            }
            for j in d..d + m {
                change = change.max(
                    (coefficients[j] - previous_coefficients[j]).abs()
                        / (1.0 + previous_coefficients[j].abs()),
                );
            }
            for key in &error_keys {
                change = change.max((current_error.get(key) / previous_error.get(key) - 1.0).abs());
            }
            mixing.final_gain = Some(gain);
            mixing.final_relative_change = Some(change);
            mixing.relative_changes.push(change);
        }
        trace.push(record_iteration(
            iteration,
            &coefficients,
            &om,
            &current_error,
        ));
        if post_burn {
            phi_history.push(phi.clone());
        }
        if phi_history.len() >= 50 && phi_history.len() % 10 == 0 {
            let Some(history) = stability_history(&trace, burn_in) else {
                stable_checks = 0;
                checkpoint!(iteration + 1, iteration + 1 == max_iterations);
                continue;
            };
            let drift_stable = stable_drift(&history, problem.tolerance);
            let update_stable = problem.update_tolerance.is_none_or(|limit| {
                mixing
                    .recent_relative_change_median()
                    .is_some_and(|change| change <= limit)
            });
            let mixing_ok = (0..d).all(|a| accepted[a] as f64 / proposed[a].max(1) as f64 > 0.05)
                && problem.mixing_lag_limit.is_none_or(|limit| {
                    lag_one(&phi_history, n, d)
                        .iter()
                        .all(|v| v.is_some_and(|v| v <= limit))
                });
            stable_checks = if drift_stable && update_stable && mixing_ok {
                stable_checks + 1
            } else {
                0
            };
            if stable_checks >= 4 && subject_accepted.iter().all(|r| r.iter().all(|&v| v > 0)) {
                status = "converged".to_string();
                checkpoint!(iteration + 1, true);
                break;
            }
        }
        checkpoint!(iteration + 1, iteration + 1 == max_iterations);
    }
    let max_lag_one = lag_one(&phi_history, n, d);
    if status == "converged" && max_lag_one.iter().any(|v| v.is_none_or(|v| v > 0.95)) {
        status = "mixing_failed".to_string();
    }
    Ok(Outcome {
        status,
        coefficients,
        phi,
        mean,
        omega: om,
        error: current_error,
        likelihoods,
        proposed,
        accepted,
        subject_accepted,
        trace,
        stable_checks,
        max_lag_one,
        mixing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::rng::Stream;

    fn record(iteration: usize, value: f64) -> TraceRecord {
        TraceRecord {
            iteration,
            coordinates: vec![value],
            fixed_effects: vec![],
            omega: vec![],
            error: Value::Null,
        }
    }

    #[test]
    fn stability_window_does_not_hide_drift_as_gain_shrinks() {
        let mut clock = 0.0;
        let mut trace = Vec::new();
        for iteration in 1..=10000usize {
            clock += (iteration as f64).powf(-0.7);
            trace.push(record(iteration, 0.02 * clock));
        }
        // A fixed 50-record window would see almost no drift this late.
        let last: Vec<f64> = trace[trace.len() - 50..]
            .iter()
            .map(|r| r.coordinates[0])
            .collect();
        let fixed = (last[..20].iter().sum::<f64>() - last[30..].iter().sum::<f64>()).abs() / 20.0;
        assert!(fixed < 0.001);
        let history = stability_history(&trace, 0).unwrap();
        let end = history.len() * 2 / 5;
        let first = history[..end].iter().map(|h| h[0]).sum::<f64>() / end as f64;
        let last = history[history.len() - end..]
            .iter()
            .map(|h| h[0])
            .sum::<f64>()
            / end as f64;
        assert!((first - last).abs() > 0.001);
        // A short post-burn-in history cannot borrow its span from burn-in records.
        assert!(stability_history(&trace[..70], 30).is_none());
        let flat: Vec<TraceRecord> = trace.iter().map(|r| record(r.iteration, 1.0)).collect();
        assert!(stability_history(&flat, 0).is_some());
    }

    #[test]
    fn recent_update_median_uses_twenty_completed_updates() {
        let mut mixing = Mixing {
            relative_changes: vec![1.0; 20],
            ..Mixing::default()
        };
        mixing.relative_changes.extend([0.001; 20]);
        assert_eq!(mixing.recent_window(), 20);
        assert_eq!(mixing.recent_relative_change_median(), Some(0.001));
    }

    #[test]
    fn censored_second_moment_is_below_the_limit() {
        let e = ErrorValues {
            lognormal: false,
            additive: 1.0,
            proportional: 0.0,
        };
        // Limit equal to the prediction: E[Z^2 | Z < 0] = 1.
        assert!((expected_square(2.0, 2.0, true, &e) - 1.0).abs() < 1e-15);
        assert_eq!(expected_square(3.0, 1.0, false, &e), 4.0);
        // Far above the limit the truncation has no effect.
        assert!((expected_square(50.0, 0.0, true, &e) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn lag_one_reports_sticky_alternating_and_persistent_chains() {
        let chain =
            |values: &[f64]| -> Vec<Matrix> { values.iter().map(|v| vec![vec![*v]]).collect() };
        // A chain that never moves is never reported as mixing: with exact
        // zero spread it has no correlation, and rounding spread reads as one.
        assert_eq!(lag_one(&chain(&[0.0; 20]), 1, 1), vec![None]);
        assert!(lag_one(&chain(&[0.3; 20]), 1, 1)[0].is_none_or(|v| v > 0.95));
        let alternating: Vec<f64> = (0..40)
            .map(|k| if k % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        assert!(lag_one(&chain(&alternating), 1, 1)[0].unwrap() < -0.99);
        let mut rng = Stream::new(20261107);
        let mut value = 0.0;
        let persistent: Vec<f64> = (0..2000)
            .map(|_| {
                value = 0.95 * value + rng.normal();
                value
            })
            .collect();
        assert!(lag_one(&chain(&persistent), 1, 1)[0].unwrap() > 0.85);
        // Too short a chain reports nothing.
        assert_eq!(lag_one(&chain(&[0.0, 1.0, 0.0, 1.0]), 1, 1), vec![None]);
    }

    #[test]
    fn covariance_mstep_clamps_variances_preserving_correlation() {
        let candidate = vec![vec![4.0, 1.2], vec![1.0, 0.36]];
        let om = project_covariance(&candidate, &[[0.01, 1.0], [0.01, 1.0]], false, None).unwrap();
        // Symmetrized cross term 1.1; correlation 1.1 / sqrt(4 * 0.36).
        let correlation = 1.1 / (4.0f64 * 0.36).sqrt();
        assert_eq!(om[0][0], 1.0);
        assert!((om[1][1] - 0.36).abs() < 1e-15);
        assert!((om[0][1] / (om[0][0] * om[1][1]).sqrt() - correlation).abs() < 1e-12);
        assert_eq!(om[0][1], om[1][0]);
        // The diagonal structure keeps no covariance.
        let diagonal = project_covariance(&candidate, &[[0.01, 10.0]; 2], true, None).unwrap();
        assert_eq!(diagonal, vec![vec![4.0, 0.0], vec![0.0, 0.36]]);
    }

    #[test]
    fn covariance_mstep_projects_correlations_and_refuses_singular_covariance() {
        let candidate = vec![vec![1.0, 0.99], vec![0.99, 1.0]];
        let capped = project_covariance(&candidate, &[[0.01, 10.0]; 2], false, Some(0.95)).unwrap();
        assert!((capped[0][1] - 0.95).abs() < 1e-15 && capped[1][0] == capped[0][1]);
        let dense = vec![
            vec![1.0, 0.788037, 0.334394, 0.880192],
            vec![0.788037, 1.0, 0.842736, 0.962935],
            vec![0.334394, 0.842736, 1.0, 0.715144],
            vec![0.880192, 0.962935, 0.715144, 1.0],
        ];
        let projected = project_covariance(&dense, &[[0.01, 10.0]; 4], false, Some(0.95)).unwrap();
        assert!(linalg::cholesky(&projected).is_some());
        assert!((projected[1][3] - 0.95).abs() < 1e-15);
        assert!(projected[0][3] < dense[0][3]);
        let uncapped = project_covariance(&candidate, &[[0.01, 10.0]; 2], false, None).unwrap();
        assert_eq!(uncapped[0][1], 0.99);
        let nonpositive = vec![vec![0.0, 0.0], vec![0.0, 1.0]];
        let error = project_covariance(&nonpositive, &[[0.01, 10.0]; 2], false, None).unwrap_err();
        assert_eq!(error.code, "identifiability");
        let singular = vec![vec![1.0, 1.0], vec![1.0, 1.0]];
        let error = project_covariance(&singular, &[[0.01, 10.0]; 2], false, None).unwrap_err();
        assert_eq!(error.code, "identifiability");
    }

    #[test]
    fn additive_sd_mstep_is_the_exact_weighted_maximizer() {
        let weights = [0.2, 0.3, 0.5];
        let squares = vec![vec![0.01, 0.04], vec![0.09, 0.16], vec![0.0, 0.25]];
        let mut mean = None;
        for (gain, rows) in [1.0, 0.6, 0.5].into_iter().zip(&squares) {
            mean = Some(update_residual_mean(mean, gain, rows));
        }
        assert_eq!(
            update_residual_mean(Some(f64::INFINITY), 1.0, &[0.25]),
            0.25
        );
        assert!(constant_residual_sd(f64::INFINITY, 1e-6, 10.0).is_err());
        let sd = constant_residual_sd(mean.unwrap(), 1e-6, 10.0).unwrap();
        // Independent reference: minimize sum_w w * sum_rows (ln v / 2 + s / (2 v)).
        let objective = |v: f64| -> f64 {
            weights
                .iter()
                .zip(&squares)
                .map(|(w, rows)| w * rows.iter().map(|s| 0.5 * v.ln() + 0.5 * s / v).sum::<f64>())
                .sum()
        };
        let v = sd * sd;
        for step in [1e-6, -1e-6] {
            assert!(objective(v * (1.0 + step)) > objective(v));
        }
        let expected = (0.2 * 0.05 + 0.3 * 0.25 + 0.5 * 0.25) / 2.0;
        assert!((v - expected).abs() < 1e-15);
        assert_eq!(constant_residual_sd(mean.unwrap(), 0.5, 1.0).unwrap(), 0.5);
    }

    #[test]
    fn adapted_chain_covariance_and_joint_steps_have_the_declared_covariance() {
        // The adaptive estimate converges to the chain's covariance.
        let target = vec![vec![0.04, 0.03], vec![0.03, 0.09]];
        let l = linalg::cholesky(&target).unwrap();
        let mut rng = Stream::new(2026110701);
        let mut mean = vec![0.0, 0.0];
        let mut covariance = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let n = 60000;
        for t in 0..n {
            let z = [rng.normal(), rng.normal()];
            let phi = linalg::mat_vec(&l, &z);
            adapt_chain_covariance(&mut mean, &mut covariance, &phi, 1.0 / (t as f64 + 2.0));
        }
        for a in 0..2 {
            assert!(mean[a].abs() < 0.01);
            for b in 0..2 {
                assert!(
                    (covariance[a][b] - target[a][b]).abs() < 0.004,
                    "{covariance:?}"
                );
            }
        }
        // A joint step scale * L z has covariance scale^2 * C.
        let scale = 0.7;
        let mut second = [[0.0; 2]; 2];
        for _ in 0..n {
            let z = [rng.normal(), rng.normal()];
            let step = linalg::mat_vec(&l, &z);
            for a in 0..2 {
                for b in 0..2 {
                    second[a][b] += scale * step[a] * scale * step[b] / n as f64;
                }
            }
        }
        for a in 0..2 {
            for b in 0..2 {
                assert!((second[a][b] - scale * scale * target[a][b]).abs() < 0.002);
            }
        }
    }

    fn combined_objective(
        weights: &[f64],
        preds: &[Vec<f64>],
        squares: &[Vec<f64>],
        a: f64,
        b: f64,
    ) -> f64 {
        let total: f64 = weights
            .iter()
            .zip(preds)
            .map(|(w, p)| w * p.len() as f64)
            .sum();
        let mut value = 0.0;
        for ((w, p), s) in weights.iter().zip(preds).zip(squares) {
            for (p, s) in p.iter().zip(s) {
                let v = a * a + (b * p).powi(2);
                value += w / total * (0.5 * v.ln() + 0.5 * s / v);
            }
        }
        value
    }

    #[test]
    fn combined_residual_mstep_matches_a_brute_force_minimum() {
        let spec = ErrorSpec {
            lognormal: false,
            additive: Sd::Fitted {
                initial: 0.1,
                lower: 0.001,
                upper: 1.0,
            },
            proportional: Sd::Fitted {
                initial: 0.2,
                lower: 0.01,
                upper: 1.0,
            },
        };
        let current = spec.initial();
        let keys = spec.fitted_keys();
        let preds = vec![(1..=40).map(|i| 0.05 * i as f64).collect::<Vec<_>>()];
        let mut rng = Stream::new(77);
        // Interior optimum near a = 0.05, b = 0.1.
        let squares = vec![preds[0]
            .iter()
            .map(|p| (0.05f64.powi(2) + (0.1 * p).powi(2)) * rng.normal().powi(2))
            .collect::<Vec<_>>()];
        let weights = [1.0];
        let fitted = residual_mstep(&weights, &preds, &squares, current, &keys, &spec).unwrap();
        let best = combined_objective(
            &weights,
            &preds,
            &squares,
            fitted.additive,
            fitted.proportional,
        );
        for da in [-0.02, 0.0, 0.02] {
            for db in [-0.02, 0.0, 0.02] {
                let a = (fitted.additive * (1.0 + da)).clamp(0.001, 1.0);
                let b = (fitted.proportional * (1.0 + db)).clamp(0.01, 1.0);
                assert!(combined_objective(&weights, &preds, &squares, a, b) >= best - 1e-14);
            }
        }
        // Proportional error alone: the objective is nearly flat in the
        // additive SD, whose optimum is its lower bound.
        let flat = vec![preds[0]
            .iter()
            .map(|p| (0.1 * p).powi(2) * 0.9)
            .collect::<Vec<_>>()];
        let fitted = residual_mstep(&weights, &preds, &flat, current, &keys, &spec).unwrap();
        assert!((fitted.additive / 0.001 - 1.0).abs() < 1e-12, "{fitted:?}");
        let expected = (0.9f64).sqrt() * 0.1;
        assert!(
            (fitted.proportional / expected - 1.0).abs() < 1e-3,
            "{fitted:?}"
        );
    }
}

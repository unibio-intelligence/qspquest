//! One-random-effect FOCEI for independent Gaussian observations.
//!
//! The conditional mode minimizes the full joint likelihood. The marginal
//! determinant instead uses first-order expected information: for each row,
//! `f_eta^2/R + 0.5*(R_eta/R)^2`. This is the first-order information approximation in
//! Almquist, Leander & Jirstrand (2015), Appendix 2, Eq. 70. It differs from
//! the observed conditional curvature used by the Laplace method.
use super::fit::PopulationOneDim;
use super::*;
use qspquest_core::{
    fit::*,
    identity::canonical_hash,
    run::{SensitivityRequest, Solver},
};
use std::collections::{HashMap, VecDeque};

#[path = "population_focei_2d.rs"]
mod multivariate;

const LOG_2PI: f64 = 1.8378770664093453;
const MAX_CACHED_OBSERVATION_ROWS: usize = 100_000;
const MAX_CACHED_TRAJECTORY_VALUES: usize = 200_000;
type SubjectKey = (usize, u64, u64, Option<u64>);
type TrajectoryKey = (usize, u64);

#[derive(Clone)]
struct Observation {
    output: usize,
    row: usize,
    value: f64,
    additive: f64,
    proportional: f64,
}

struct PreparedFoceiOneDim {
    setup: PopulationOneDim,
    subjects: Vec<Subject>,
    additive_fit: Option<AdditiveFitSetup>,
    simulation_groups: Vec<usize>,
}

struct RowTerms {
    nll: f64,
    score: f64,
    information: f64,
}

fn row_terms(
    observation: &Observation,
    prediction: f64,
    f_eta: f64,
    additive: f64,
) -> Result<RowTerms, Error> {
    let prop = observation.proportional * prediction;
    let variance = additive * additive + prop * prop;
    if !variance.is_finite() || variance <= 0. || !f_eta.is_finite() {
        return Err(Error::new(
            ErrorCode::Domain,
            "FOCEI residual variance is invalid",
        ));
    }
    let residual = observation.value - prediction;
    let r_eta = 2. * observation.proportional.powi(2) * prediction * f_eta;
    let scaled_r_eta = r_eta / variance;
    let z2 = residual * residual / variance;
    let terms = RowTerms {
        nll: 0.5 * (z2 + libm::log(variance) + LOG_2PI),
        score: -residual * f_eta / variance + 0.5 * (1. - z2) * scaled_r_eta,
        information: f_eta * f_eta / variance + 0.5 * scaled_r_eta * scaled_r_eta,
    };
    if !terms.nll.is_finite() || !terms.score.is_finite() || !terms.information.is_finite() {
        return Err(Error::new(ErrorCode::Domain, "FOCEI likelihood overflow"));
    }
    Ok(terms)
}

struct Subject {
    simulation: SensitivityRequest,
    observations: Vec<Observation>,
}

#[derive(Clone)]
struct Conditional {
    eta: f64,
    nll: f64,
    score: f64,
    information: f64,
    rows: Vec<PredictionRow>,
}

#[derive(Clone)]
struct PredictionRow {
    prediction: f64,
    time: f64,
    side: qspquest_core::regimen::ObservationSide,
}

struct Evaluation {
    ofv: f64,
    subjects: Vec<PopulationSubjectEstimate>,
    fitted_observations: Vec<PopulationFittedObservation>,
}

struct AdditiveFitSetup {
    output: String,
    unit: String,
    start: f64,
    lower: f64,
    upper: f64,
}

fn validate_additive_fit(
    model: &CompiledSensitivityDocument,
    request: &PopulationFitRequest,
    subjects: &[Subject],
) -> Result<Option<AdditiveFitSetup>, Error> {
    let Some(fit) = &request.additive_error_fit else {
        return Ok(None);
    };
    if request.uncertainty {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "observed covariance with estimated residual SD is not implemented",
        ));
    }
    let output = model
        .output_names()
        .iter()
        .position(|name| name == &fit.output)
        .ok_or_else(|| invalid("unknown additive-error fit output"))?;
    let unit = model.output_units()[output].clone();
    let parsed_unit = Unit::parse(&unit)?;
    let start = quantity(&fit.initial)?.in_unit(parsed_unit)?;
    let lower = quantity(&fit.lower)?.in_unit(parsed_unit)?;
    let upper = quantity(&fit.upper)?.in_unit(parsed_unit)?;
    if !start.is_finite()
        || !lower.is_finite()
        || !upper.is_finite()
        || lower <= 0.
        || lower >= upper
        || start < lower
        || start > upper
    {
        return Err(invalid(
            "estimated additive SD requires positive finite bounds containing its initial value",
        ));
    }
    for (index, subject) in subjects.iter().enumerate() {
        for (row, observation) in subject.observations.iter().enumerate() {
            if observation.output != output
                || observation.proportional != 0.
                || (observation.additive - start).abs() > 1e-10 * start
            {
                return Err(invalid(format!(
                    "subjects[{index}].observations[{row}] must use the fitted output, zero proportional error, and initial additive SD"
                )));
            }
        }
    }
    Ok(Some(AdditiveFitSetup {
        output: fit.output.clone(),
        unit,
        start,
        lower,
        upper,
    }))
}

struct Evaluator<'a> {
    model: &'a Arc<CompiledSensitivityDocument>,
    request: &'a PopulationFitRequest,
    subjects: Vec<Subject>,
    parameter: &'a str,
    unit: &'a str,
    calls: usize,
    warm_etas: Vec<f64>,
    simulation_groups: Vec<usize>,
    cache: HashMap<SubjectKey, Conditional>,
    cache_order: VecDeque<SubjectKey>,
    cached_rows: usize,
    trajectory_cache: HashMap<TrajectoryKey, Vec<SensitivitySample>>,
    trajectory_order: VecDeque<TrajectoryKey>,
    cached_trajectory_values: usize,
    trajectory_value_limit: usize,
    #[cfg(test)]
    trajectory_runs: usize,
}

fn simulation_groups(subjects: &[GaussianObjectiveRequest]) -> Result<Vec<usize>, Error> {
    let mut groups = HashMap::new();
    let mut indices = Vec::with_capacity(subjects.len());
    for subject in subjects {
        let serialized = serde_json::to_value(&subject.simulation)
            .map_err(|_| invalid("cannot serialize FOCEI simulation request"))?;
        let signature = canonical_hash(&serialized)?;
        let next_group = groups.len();
        indices.push(*groups.entry(signature).or_insert(next_group));
    }
    Ok(indices)
}

impl Evaluator<'_> {
    fn subject(
        &mut self,
        index: usize,
        theta: f64,
        eta: f64,
        additive_sd: Option<f64>,
    ) -> Result<Conditional, Error> {
        // Omega changes the eta prior, not the data likelihood. The outer
        // search therefore revisits identical simulation points at different
        // Omega values. Reuse those exact points without relaxing the global
        // eta screen or changing the objective.
        let key = (
            index,
            theta.to_bits(),
            eta.to_bits(),
            additive_sd.map(f64::to_bits),
        );
        if let Some(value) = self.cache.get(&key) {
            return Ok(value.clone());
        }
        if self.calls >= self.request.max_evaluations {
            return Err(Error::new(
                ErrorCode::WorkBudget,
                "FOCEI evaluation limit reached",
            ));
        }
        self.calls += 1;
        let individual = theta * libm::exp(eta);
        if !individual.is_finite() || individual <= 0. {
            return Err(Error::new(
                ErrorCode::Domain,
                "invalid FOCEI individual parameter",
            ));
        }
        let subject = &self.subjects[index];
        let run = &subject.simulation.run;
        let mut parameters = run.parameters.clone();
        parameters.insert(
            self.parameter.into(),
            qspquest_core::model::Quantity {
                value: individual,
                unit: self.unit.into(),
            },
        );
        // A cohort often changes only the observed DVs. Exact simulation
        // requests and individual parameter bits then yield identical
        // sensitivities; share that trajectory while evaluating every
        // subject's residuals and prior separately.
        let trajectory_key = (self.simulation_groups[index], individual.to_bits());
        let uncached_rows;
        let rows = if self.trajectory_cache.contains_key(&trajectory_key) {
            &self.trajectory_cache[&trajectory_key]
        } else {
            let simulated = self.model.simulate_regimen_initial(
                &parameters,
                &run.regimen,
                &subject.simulation.derivative_absolute_tolerances,
                run.budgets,
                &run.initial_states,
                run.solver,
            )?;
            #[cfg(test)]
            {
                self.trajectory_runs += 1;
            }
            let values = simulated
                .len()
                .saturating_mul(self.model.output_names().len())
                .saturating_mul(2);
            if values <= self.trajectory_value_limit {
                while self.cached_trajectory_values + values > self.trajectory_value_limit {
                    if let Some(oldest) = self.trajectory_order.pop_front() {
                        if let Some(removed) = self.trajectory_cache.remove(&oldest) {
                            self.cached_trajectory_values -= removed
                                .len()
                                .saturating_mul(self.model.output_names().len())
                                .saturating_mul(2);
                        }
                    }
                }
                self.trajectory_order.push_back(trajectory_key);
                self.cached_trajectory_values += values;
                self.trajectory_cache.insert(trajectory_key, simulated);
                &self.trajectory_cache[&trajectory_key]
            } else {
                uncached_rows = simulated;
                &uncached_rows
            }
        };
        let width = self.model.output_names().len();
        if rows
            .len()
            .checked_mul(width)
            .and_then(|n| n.checked_mul(2))
            .is_none_or(|n| n > run.budgets.output_values)
        {
            return Err(Error::new(
                ErrorCode::OutputBudget,
                "sensitivity row and column projection exceeds budget",
            ));
        }
        let mut nll = 0.;
        let mut score = 0.;
        let mut information = 0.;
        let mut predictions = Vec::with_capacity(subject.observations.len());
        for (row_index, observation) in subject.observations.iter().enumerate() {
            let row = rows
                .get(observation.row)
                .ok_or_else(|| invalid("FOCEI observation row is outside the simulation result"))?;
            let prediction = row.values[observation.output];
            let sensitivity = row.derivatives[0][observation.output];
            // The simulator derivative is with respect to the natural model
            // parameter. The lognormal random effect gives dp/deta = p.
            let f_eta = sensitivity * individual;
            let terms = row_terms(
                observation,
                prediction,
                f_eta,
                additive_sd.unwrap_or(observation.additive),
            )
            .map_err(|mut error| {
                error.expression = Some(format!("subjects[{index}].observations[{row_index}]"));
                error
            })?;
            nll += terms.nll;
            score += terms.score;
            information += terms.information;
            predictions.push(PredictionRow {
                prediction,
                time: row.time,
                side: if row.side == "pre" {
                    qspquest_core::regimen::ObservationSide::Pre
                } else {
                    qspquest_core::regimen::ObservationSide::Post
                },
            });
        }
        if !nll.is_finite() || !score.is_finite() || !information.is_finite() {
            return Err(Error::new(ErrorCode::Domain, "FOCEI likelihood overflow"));
        }
        let conditional = Conditional {
            eta,
            nll,
            score,
            information,
            rows: predictions,
        };
        let row_count = conditional.rows.len();
        if row_count <= MAX_CACHED_OBSERVATION_ROWS {
            while self.cached_rows + row_count > MAX_CACHED_OBSERVATION_ROWS {
                if let Some(oldest) = self.cache_order.pop_front() {
                    if let Some(value) = self.cache.remove(&oldest) {
                        self.cached_rows -= value.rows.len();
                    }
                }
            }
            self.cache_order.push_back(key);
            self.cached_rows += row_count;
            self.cache.insert(key, conditional.clone());
        }
        Ok(conditional)
    }

    fn mode(
        &mut self,
        index: usize,
        theta: f64,
        omega: f64,
        additive_sd: Option<f64>,
    ) -> Result<(Conditional, Vec<PredictionRow>), Error> {
        let limit = self.request.eta_bound;
        let evaluate = |this: &mut Self, eta: f64| -> Result<Conditional, Error> {
            let mut value = this.subject(index, theta, eta, additive_sd)?;
            value.score += eta / omega;
            Ok(value)
        };
        // Screen the declared eta domain for sign changes before settling the
        // mode. A single bracket is required by this initial 1-D slice.
        let mut grid = Vec::with_capacity(17);
        for point in 0..=16 {
            let eta = -limit + limit * point as f64 / 8.;
            grid.push(evaluate(self, eta)?);
        }
        let population_rows = grid[8].rows.clone();
        if grid[0].score >= 0. || grid[16].score <= 0. {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "FOCEI eta mode reaches its bound",
            ));
        }
        let upward = grid
            .windows(2)
            .filter(|pair| pair[0].score <= 0. && pair[1].score > 0.)
            .collect::<Vec<_>>();
        if upward.len() != 1
            || grid
                .windows(2)
                .any(|pair| pair[0].score > 0. && pair[1].score <= 0.)
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "FOCEI conditional score has multiple modes in the eta domain",
            ));
        }
        let mut left = upward[0][0].clone();
        let mut right = upward[0][1].clone();
        let mut best = if left.score.abs() < right.score.abs() {
            left.clone()
        } else {
            right.clone()
        };
        // Inner mode error must be smaller than the requested outer search
        // resolution. A fixed score cutoff otherwise adds objective noise
        // that can stop a tight fit at different parameters across warm starts.
        let eta_tolerance = (self.request.gradient_tolerance * 1e-4).clamp(1e-12, 1e-8);
        let settled = |value: &Conditional, width: f64| {
            width <= eta_tolerance
                || value.score.abs() / (1. / omega + value.information) <= eta_tolerance
        };
        let warm = self.warm_etas[index];
        if warm > left.eta && warm < right.eta {
            let trial = evaluate(self, warm)?;
            if trial.score > 0. {
                right = trial.clone();
            } else {
                left = trial.clone();
            }
            if trial.score.abs() < best.score.abs() {
                best = trial;
            }
        }
        for _ in 0..16 {
            if settled(&best, right.eta - left.eta) {
                break;
            }
            let width = right.eta - left.eta;
            let eta = (left.eta - left.score * width / (right.score - left.score))
                .clamp(left.eta + 0.1 * width, right.eta - 0.1 * width);
            let trial = evaluate(self, eta)?;
            if trial.score > 0. {
                right = trial.clone();
            } else {
                left = trial.clone();
            }
            if trial.score.abs() < best.score.abs() {
                best = trial;
            }
        }
        if !settled(&best, right.eta - left.eta) {
            for _ in 0..48 {
                if settled(&best, right.eta - left.eta) {
                    break;
                }
                let trial = evaluate(self, 0.5 * (left.eta + right.eta))?;
                if trial.score > 0. {
                    right = trial.clone();
                } else {
                    left = trial.clone();
                }
                if trial.score.abs() < best.score.abs() {
                    best = trial;
                }
            }
        }
        if !settled(&best, right.eta - left.eta) {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "FOCEI conditional mode did not converge",
            ));
        }
        Ok((best, population_rows))
    }

    fn population(
        &mut self,
        theta: f64,
        omega: f64,
        additive_sd: Option<f64>,
    ) -> Result<Evaluation, Error> {
        let mut ofv = 0.;
        let mut estimates = Vec::with_capacity(self.subjects.len());
        let mut fitted_observations = Vec::new();
        for index in 0..self.subjects.len() {
            let (mode, population_rows) =
                self.mode(index, theta, omega, additive_sd)
                    .map_err(|mut error| {
                        error.expression = Some(format!("subjects[{index}]"));
                        error
                    })?;
            let curvature = 1. / omega + mode.information;
            if !curvature.is_finite() || curvature <= 0. {
                return Err(Error::new(
                    ErrorCode::Domain,
                    "FOCEI information is not positive",
                ));
            }
            self.warm_etas[index] = mode.eta;
            // Minus twice the conditional Gaussian log density, plus the
            // random-effect prior and first-order determinant. Remove only the
            // observation 2π normalization constant.
            ofv += 2. * mode.nll - self.subjects[index].observations.len() as f64 * LOG_2PI
                + mode.eta * mode.eta / omega
                + libm::log(omega * curvature);
            estimates.push(PopulationSubjectEstimate {
                eta: vec![mode.eta],
                negative_log_likelihood: mode.nll,
            });
            for (observation_index, (observation, (pred, ipred))) in self.subjects[index]
                .observations
                .iter()
                .zip(population_rows.iter().zip(&mode.rows))
                .enumerate()
            {
                if pred.time != ipred.time || pred.side != ipred.side {
                    return Err(Error::new(
                        ErrorCode::Invariant,
                        "FOCEI prediction grids differ across eta",
                    ));
                }
                let unit = self.model.output_units()[observation.output].clone();
                let quantity = |value| qspquest_core::model::Quantity {
                    value,
                    unit: unit.clone(),
                };
                fitted_observations.push(PopulationFittedObservation {
                    subject_index: index,
                    observation_index,
                    row: observation.row,
                    output: self.model.output_names()[observation.output].clone(),
                    time: qspquest_core::model::Quantity {
                        value: pred.time,
                        unit: self.model.primal.time_unit.clone(),
                    },
                    side: pred.side,
                    value: quantity(observation.value),
                    pred: quantity(pred.prediction),
                    ipred: quantity(ipred.prediction),
                });
            }
        }
        if !ofv.is_finite() {
            return Err(Error::new(ErrorCode::Domain, "FOCEI objective overflow"));
        }
        Ok(Evaluation {
            ofv,
            subjects: estimates,
            fitted_observations,
        })
    }
}

/// Observed information from the profiled FOCEI population objective. Work in
/// affine unit-box coordinates, then transform the inverse to natural units.
/// This is a local curvature estimate, not a sandwich or bootstrap interval.
fn observed_outer_covariance(
    evaluator: &mut Evaluator<'_>,
    x: [f64; 2],
    f0: f64,
    theta_bounds: [f64; 2],
    omega_bounds: [f64; 2],
    omega_fixed: f64,
    parameter: &str,
    unit: &str,
) -> Result<PopulationFitUncertainty, Error> {
    let free_omega = omega_bounds[0] < omega_bounds[1];
    let dimensions = if free_omega { 2 } else { 1 };
    let h = 0.01;
    if x[..dimensions]
        .iter()
        .any(|coordinate| *coordinate < 2. * h || *coordinate > 1. - 2. * h)
    {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "FOCEI observed covariance requires an interior estimate",
        ));
    }
    let evaluate = |evaluator: &mut Evaluator<'_>, point: [f64; 2]| {
        let theta = theta_bounds[0] + point[0] * (theta_bounds[1] - theta_bounds[0]);
        let omega = if free_omega {
            omega_bounds[0] + point[1] * (omega_bounds[1] - omega_bounds[0])
        } else {
            omega_fixed
        };
        evaluator
            .population(theta, omega, None)
            .map(|result| result.ofv)
    };
    let mut diagonal = [0.; 2];
    for axis in 0..dimensions {
        let mut plus = x;
        plus[axis] += h;
        let mut minus = x;
        minus[axis] -= h;
        diagonal[axis] =
            (evaluate(evaluator, plus)? - 2. * f0 + evaluate(evaluator, minus)?) / (h * h);
    }
    let mut cross = 0.;
    if free_omega {
        let mut corner = x;
        corner[0] += h;
        corner[1] += h;
        let pp = evaluate(evaluator, corner)?;
        corner[1] -= 2. * h;
        let pm = evaluate(evaluator, corner)?;
        corner[0] -= 2. * h;
        corner[1] += 2. * h;
        let mp = evaluate(evaluator, corner)?;
        corner[1] -= 2. * h;
        let mm = evaluate(evaluator, corner)?;
        cross = (pp - pm - mp + mm) / (4. * h * h);
    }
    if !diagonal[0].is_finite() || diagonal[0] <= 0. {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "FOCEI observed information is not positive definite",
        ));
    }
    let theta_span = theta_bounds[1] - theta_bounds[0];
    let (parameters, covariance) = if free_omega {
        let determinant = diagonal[0] * diagonal[1] - cross * cross;
        if !diagonal[1].is_finite()
            || diagonal[1] <= 0.
            || !cross.is_finite()
            || !determinant.is_finite()
            || determinant <= 1e-8 * diagonal[0] * diagonal[1]
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "FOCEI observed information is singular or not positive definite",
            ));
        }
        let omega_span = omega_bounds[1] - omega_bounds[0];
        (
            vec![
                PopulationUncertaintyParameter {
                    name: parameter.into(),
                    unit: unit.into(),
                },
                PopulationUncertaintyParameter {
                    name: format!("omega:{parameter}"),
                    unit: "1".into(),
                },
            ],
            vec![
                vec![
                    2. * diagonal[1] / determinant * theta_span * theta_span,
                    -2. * cross / determinant * theta_span * omega_span,
                ],
                vec![
                    -2. * cross / determinant * theta_span * omega_span,
                    2. * diagonal[0] / determinant * omega_span * omega_span,
                ],
            ],
        )
    } else {
        (
            vec![PopulationUncertaintyParameter {
                name: parameter.into(),
                unit: unit.into(),
            }],
            vec![vec![2. * theta_span * theta_span / diagonal[0]]],
        )
    };
    let standard_errors = (0..dimensions)
        .map(|axis| libm::sqrt(covariance[axis][axis]))
        .collect::<Vec<_>>();
    if covariance.iter().flatten().any(|value| !value.is_finite())
        || standard_errors
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.)
    {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "FOCEI observed covariance is not finite and positive",
        ));
    }
    Ok(PopulationFitUncertainty {
        method: PopulationUncertaintyMethod::FoceiObservedOuterHessian,
        parameters,
        covariance,
        standard_errors,
    })
}

fn fit_free_additive(
    request: &PopulationFitRequest,
    setup: &PopulationOneDim,
    additive: &AdditiveFitSetup,
    evaluator: &mut Evaluator<'_>,
) -> Result<PopulationFitResult, Error> {
    let omega_free = setup.omega_lower < setup.omega_upper;
    let mut x = [
        (setup.start - setup.lower) / (setup.upper - setup.lower),
        if omega_free {
            (setup.omega - setup.omega_lower) / (setup.omega_upper - setup.omega_lower)
        } else {
            0.
        },
        (additive.start - additive.lower) / (additive.upper - additive.lower),
    ];
    let decode = |point: [f64; 3]| {
        (
            setup.lower + point[0] * (setup.upper - setup.lower),
            if omega_free {
                setup.omega_lower + point[1] * (setup.omega_upper - setup.omega_lower)
            } else {
                setup.omega
            },
            additive.lower + point[2] * (additive.upper - additive.lower),
        )
    };
    let (theta, omega, sigma) = decode(x);
    let mut best = evaluator.population(theta, omega, Some(sigma))?;
    let mut steps = [0.2, if omega_free { 0.2 } else { 0. }, 0.2];
    let mut iterations = 0;
    let status = loop {
        if steps.iter().copied().fold(0., f64::max) <= request.gradient_tolerance {
            break FitStatus::Converged;
        }
        if iterations >= request.max_iterations {
            break FitStatus::IterationLimit;
        }
        if evaluator.calls >= request.max_evaluations {
            break FitStatus::EvaluationLimit;
        }
        let mut improved = false;
        for dimension in [0, 1, 2] {
            if dimension == 1 && !omega_free {
                continue;
            }
            for direction in [-1., 1.] {
                let mut trial = x;
                trial[dimension] = (trial[dimension] + direction * steps[dimension]).clamp(0., 1.);
                if trial == x {
                    continue;
                }
                let (theta, omega, sigma) = decode(trial);
                match evaluator.population(theta, omega, Some(sigma)) {
                    Ok(candidate) if candidate.ofv < best.ofv => {
                        x = trial;
                        best = candidate;
                        improved = true;
                    }
                    Ok(_) => {}
                    Err(error) if error.code == ErrorCode::WorkBudget => break,
                    Err(error)
                        if matches!(
                            error.code,
                            ErrorCode::Domain
                                | ErrorCode::Solver
                                | ErrorCode::Invariant
                                | ErrorCode::Unsupported
                        ) => {}
                    Err(error) => return Err(error),
                }
                if evaluator.calls >= request.max_evaluations {
                    break;
                }
            }
            if evaluator.calls >= request.max_evaluations {
                break;
            }
        }
        iterations += 1;
        if !improved {
            for step in &mut steps {
                *step *= 0.5;
            }
        }
    };
    let (theta, omega, sigma) = decode(x);
    Ok(PopulationFitResult {
        status,
        fixed_effects: BTreeMap::from([(
            setup.parameter.clone(),
            qspquest_core::model::Quantity {
                value: theta,
                unit: setup.unit.clone(),
            },
        )]),
        omega: vec![vec![omega]],
        covariate_effect: None,
        subjects: best.subjects,
        fitted_observations: Some(best.fitted_observations),
        uncertainty: None,
        estimated_residual_error: Some(PopulationResidualErrorEstimate {
            output: additive.output.clone(),
            additive_sd: qspquest_core::model::Quantity {
                value: sigma,
                unit: additive.unit.clone(),
            },
        }),
        saem_diagnostics: None,
        objective_kind: PopulationObjectiveKind::FoceiObjectiveFunctionValue,
        objective: best.ofv,
        evaluations: evaluator.calls,
        iterations,
        fit_request_hash: canonical_hash(
            &serde_json::json!({"algorithm":"focei_1d_free_additive_v1","request":request}),
        )?,
    })
}

impl CompiledSensitivityDocument {
    /// Gaussian FOCEI with first-order interaction curvature. The one-effect
    /// path screens its eta domain; the two-effect path uses deterministic
    /// multistart local modes and a fixed correlated Omega matrix. The
    /// objective omits the observation log(2π) normalization constant.
    /// `Converged` denotes a small coordinate-search step, not a covariance or
    /// projected-gradient certificate.
    fn prepare_population_focei_1d(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
    ) -> Result<PreparedFoceiOneDim, Error> {
        self.prepare_population_focei_1d_observed(request, None)
    }
    fn prepare_population_focei_1d_observed(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        mut plan: Option<&mut super::fit_planning::FitPlanBuilder>,
    ) -> Result<PreparedFoceiOneDim, Error> {
        let setup = self.validate_population_1d(request)?;
        let total_observations = request
            .subjects
            .iter()
            .try_fold(0usize, |total, subject| {
                total.checked_add(subject.observations.len())
            })
            .ok_or_else(|| invalid("FOCEI observation count overflow"))?;
        if total_observations > 100_000 {
            return Err(Error::new(
                ErrorCode::OutputBudget,
                "FOCEI supports at most 100000 fitted observation rows",
            ));
        }
        let subjects = request.subjects.iter().enumerate().map(|(index, subject)| {
            let run = &subject.simulation.run;
            if !matches!(run.solver, Solver::DiffsolBdf | Solver::DiffsolRodas5p) || run.seed.is_some() {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "sensitivity runs require deterministic diffsol_bdf or diffsol_rodas5p",
                ));
            }
            if run.request_id.as_ref().is_some_and(|id| id.is_empty() || id.len() > 128) {
                return Err(invalid("request_id must contain 1..128 bytes"));
            }
            if run.budgets.solver_callbacks > u32::MAX as u64
                || run.budgets.events > u32::MAX as usize
                || run.budgets.output_values > u32::MAX as usize
            {
                return Err(invalid(
                    "budgets must fit the shared native/WASM unsigned 32-bit range",
                ));
            }
            if subject.observations.is_empty()
                || subject.observations.len() > 1_000_000
                || subject.observations.len() > subject.simulation.run.budgets.output_values
            {
                return Err(invalid(format!("subject {index} needs bounded observations")));
            }
            let observations = subject.observations.iter().enumerate().map(|(row_index, observation)| {
                if observation.error.model != GaussianErrorModel::Normal {
                    return Err(Error::new(ErrorCode::Unsupported, "FOCEI supports normal uncensored observations"));
                }
                if observation.censoring.is_some() {
                    return Err(Error::new(ErrorCode::Unsupported,
                        "FOCEI censored observations are not implemented"));
                }
                let output = self.output_names().iter().position(|name| name == &observation.output)
                    .ok_or_else(|| invalid("unknown FOCEI observation output"))?;
                let unit = Unit::parse(&self.output_units()[output])?;
                let value = quantity(&observation.value)?.in_unit(unit)?;
                let additive = quantity(&observation.error.additive_sd)?.in_unit(unit)?;
                let proportional = observation.error.proportional_sd;
                if !value.is_finite() || !additive.is_finite() || additive < 0.
                    || !proportional.is_finite() || proportional < 0.
                    || additive == 0. && proportional == 0.
                {
                    return Err(invalid(format!("subjects[{index}].observations[{row_index}] has invalid Gaussian error")));
                }
                Ok(Observation { output, row: observation.row, value, additive, proportional })
            }).collect::<Result<Vec<_>, Error>>()?;
            Ok(Subject { simulation: subject.simulation.clone(), observations })
        }).collect::<Result<Vec<_>, Error>>()?;
        let additive_fit = validate_additive_fit(self, request, &subjects)?;
        let simulation_groups = simulation_groups(&request.subjects)?;
        for (index, subject) in request.subjects.iter().enumerate() {
            let prepared = self.prepare_run(&subject.simulation)?;
            for (row, observation) in subject.observations.iter().enumerate() {
                if observation.row >= prepared.result_rows() {
                    let mut error =
                        invalid("FOCEI observation row is outside the simulation result");
                    error.expression = Some(format!("subjects[{index}].observations[{row}]"));
                    return Err(error);
                }
            }
            if let Some(plan) = plan.as_deref_mut() {
                plan.focei(
                    self,
                    &subject.simulation,
                    &prepared,
                    || describe_observations(self, &subjects[index].observations),
                    subject.observations.len(),
                )?;
            }
        }
        Ok(PreparedFoceiOneDim {
            setup,
            subjects,
            additive_fit,
            simulation_groups,
        })
    }
    pub fn fit_population_focei(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
    ) -> Result<PopulationFitResult, Error> {
        validate_dispatch(request)?;
        if request.random_effects.len() == 2 {
            return multivariate::fit_population_focei_2d(self, request);
        }
        let PreparedFoceiOneDim {
            setup,
            subjects,
            additive_fit,
            simulation_groups,
        } = self.prepare_population_focei_1d(request)?;
        let mut evaluator = Evaluator {
            model: self,
            request,
            subjects,
            parameter: &setup.parameter,
            unit: &setup.unit,
            calls: 0,
            warm_etas: vec![0.; request.subjects.len()],
            simulation_groups,
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
            cached_rows: 0,
            trajectory_cache: HashMap::new(),
            trajectory_order: VecDeque::new(),
            cached_trajectory_values: 0,
            trajectory_value_limit: MAX_CACHED_TRAJECTORY_VALUES,
            #[cfg(test)]
            trajectory_runs: 0,
        };
        if let Some(additive_fit) = additive_fit {
            return fit_free_additive(request, &setup, &additive_fit, &mut evaluator);
        }
        let omega_free = setup.omega_lower < setup.omega_upper;
        let mut x = [
            (setup.start - setup.lower) / (setup.upper - setup.lower),
            if omega_free {
                (setup.omega - setup.omega_lower) / (setup.omega_upper - setup.omega_lower)
            } else {
                0.
            },
        ];
        let decode = |x: &[f64; 2]| {
            (
                setup.lower + x[0] * (setup.upper - setup.lower),
                if omega_free {
                    setup.omega_lower + x[1] * (setup.omega_upper - setup.omega_lower)
                } else {
                    setup.omega
                },
            )
        };
        let (theta, omega) = decode(&x);
        let mut best = evaluator.population(theta, omega, None)?;
        let mut steps: [f64; 2] = [0.2, if omega_free { 0.2 } else { 0. }];
        let mut iterations = 0;
        let status = loop {
            if steps[0].max(steps[1]) <= request.gradient_tolerance {
                break FitStatus::Converged;
            }
            if iterations >= request.max_iterations {
                break FitStatus::IterationLimit;
            }
            if evaluator.calls >= request.max_evaluations {
                break FitStatus::EvaluationLimit;
            }
            let mut improved = false;
            for dimension in 0..(if omega_free { 2 } else { 1 }) {
                for direction in [-1., 1.] {
                    let mut trial = x;
                    trial[dimension] =
                        (trial[dimension] + direction * steps[dimension]).clamp(0., 1.);
                    if trial == x {
                        continue;
                    }
                    let (theta, omega) = decode(&trial);
                    match evaluator.population(theta, omega, None) {
                        Ok(candidate) if candidate.ofv < best.ofv => {
                            x = trial;
                            best = candidate;
                            improved = true;
                        }
                        Ok(_) => {}
                        Err(error) if error.code == ErrorCode::WorkBudget => break,
                        Err(error)
                            if matches!(
                                error.code,
                                ErrorCode::Domain
                                    | ErrorCode::Solver
                                    | ErrorCode::Invariant
                                    | ErrorCode::Unsupported
                            ) => {}
                        Err(error) => return Err(error),
                    }
                    if evaluator.calls >= request.max_evaluations {
                        break;
                    }
                }
                if evaluator.calls >= request.max_evaluations {
                    break;
                }
            }
            iterations += 1;
            if !improved {
                for step in &mut steps {
                    *step *= 0.5;
                }
            }
        };
        let (theta, omega) = decode(&x);
        let uncertainty = if request.uncertainty {
            if status != FitStatus::Converged {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "FOCEI observed covariance requires a converged fit",
                ));
            }
            Some(observed_outer_covariance(
                &mut evaluator,
                x,
                best.ofv,
                [setup.lower, setup.upper],
                [setup.omega_lower, setup.omega_upper],
                setup.omega,
                &setup.parameter,
                &setup.unit,
            )?)
        } else {
            None
        };
        Ok(PopulationFitResult {
            status,
            fixed_effects: BTreeMap::from([(
                setup.parameter.clone(),
                qspquest_core::model::Quantity {
                    value: theta,
                    unit: setup.unit.clone(),
                },
            )]),
            omega: vec![vec![omega]],
            covariate_effect: None,
            subjects: best.subjects,
            fitted_observations: Some(best.fitted_observations),
            uncertainty,
            estimated_residual_error: None,
            saem_diagnostics: None,
            objective_kind: PopulationObjectiveKind::FoceiObjectiveFunctionValue,
            objective: best.ofv,
            evaluations: evaluator.calls,
            iterations,
            fit_request_hash: canonical_hash(
                &serde_json::json!({"algorithm":"focei_1d_v1","request":request}),
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_fixture() -> (
        Arc<CompiledSensitivityDocument>,
        PopulationFitRequest,
        Vec<Subject>,
    ) {
        let model: qspquest_core::model::ModelDocument = serde_json::from_str(include_str!(
            "../../../../../conformance/models/synthetic-one-compartment.json"
        ))
        .unwrap();
        let compiled = CompiledSensitivityDocument::compile(&model, &["cl".into()]).unwrap();
        let mut simulation: SensitivityRequest = serde_json::from_str(include_str!(
            "../../../../../conformance/requests/synthetic-sensitivity.json"
        ))
        .unwrap();
        simulation.with_respect_to = vec!["cl".into()];
        simulation.derivative_absolute_tolerances.remove("v");
        let q = |value, unit: &str| qspquest_core::model::Quantity {
            value,
            unit: unit.into(),
        };
        let observations = [1.0, 1.5]
            .into_iter()
            .map(|value| GaussianObservation {
                censoring: None,
                row: 2,
                output: "cp".into(),
                value: q(value, "mg/L"),
                error: GaussianError {
                    model: GaussianErrorModel::Normal,
                    additive_sd: q(0.1, "mg/L"),
                    proportional_sd: 0.,
                },
            })
            .collect::<Vec<_>>();
        let request = PopulationFitRequest {
            subjects: observations
                .iter()
                .map(|observation| GaussianObjectiveRequest {
                    simulation: simulation.clone(),
                    observations: vec![observation.clone()],
                    priors: vec![],
                })
                .collect(),
            fixed_effects: vec![FitBound {
                parameter: "cl".into(),
                lower: q(0.2, "L/h"),
                upper: q(1.2, "L/h"),
            }],
            random_effects: vec![PopulationRandomEffect {
                parameter: "cl".into(),
            }],
            omega: vec![vec![0.04]],
            omega_diagonal_bounds: vec![[0.04, 0.04]],
            eta_bound: 1.5,
            max_evaluations: 1000,
            max_iterations: 1,
            gradient_tolerance: 0.25,
            seed: 1,
            burn_in_iterations: None,
            uncertainty: false,
            additive_error_fit: None,
            covariate_effect: None,
        };
        let subjects = observations
            .iter()
            .map(|observation| Subject {
                simulation: simulation.clone(),
                observations: vec![Observation {
                    output: 0,
                    row: observation.row,
                    value: observation.value.value,
                    additive: 0.1,
                    proportional: 0.,
                }],
            })
            .collect();
        (compiled, request, subjects)
    }

    #[test]
    fn tight_modes_match_analytic_pk_score_independently_of_warm_start() {
        let (model, mut request, subjects) = cache_fixture();
        request.gradient_tolerance = 1e-8;
        let mut evaluator = Evaluator {
            model: &model,
            request: &request,
            subjects,
            parameter: "cl",
            unit: "L/h",
            calls: 0,
            warm_etas: vec![-1.; 2],
            simulation_groups: simulation_groups(&request.subjects).unwrap(),
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
            cached_rows: 0,
            trajectory_cache: HashMap::new(),
            trajectory_order: VecDeque::new(),
            cached_trajectory_values: 0,
            trajectory_value_limit: MAX_CACHED_TRAJECTORY_VALUES,
            trajectory_runs: 0,
        };
        // Independent bolus-PK oracle at t=1 h: f=6/3*exp(-cl*t/3).
        // Solve the exact Gaussian conditional score, including eta's prior.
        let score = |eta: f64| {
            let exposure = 0.6 * libm::exp(eta) / 3.;
            let prediction = 2. * libm::exp(-exposure);
            (prediction - 1.) * (-exposure * prediction) / 0.01 + eta / 0.04
        };
        let (mut lower, mut upper) = (-1.5, 1.5);
        for _ in 0..80 {
            let midpoint = (lower + upper) * 0.5;
            if score(midpoint) > 0. {
                upper = midpoint;
            } else {
                lower = midpoint;
            }
        }
        let expected = (lower + upper) * 0.5;
        let first = evaluator.mode(0, 0.6, 0.04, None).unwrap().0;
        evaluator.warm_etas[0] = 0.8;
        let second = evaluator.mode(0, 0.6, 0.04, None).unwrap().0;
        assert!(
            (first.eta - expected).abs() < 2e-8,
            "{} != {expected}",
            first.eta
        );
        assert!(
            (second.eta - expected).abs() < 2e-8,
            "{} != {expected}",
            second.eta
        );
        assert!((first.eta - second.eta).abs() < 2e-9);
    }

    #[test]
    fn simulation_groups_exclude_observations_but_bind_model_inputs_events_and_covariates() {
        let (_, request, _) = cache_fixture();
        assert_eq!(simulation_groups(&request.subjects).unwrap(), vec![0, 0]);
        let mut variants = request.subjects;
        let mut dose = variants[0].clone();
        dose.simulation.run.regimen.administrations[0].amount.value *= 2.;
        variants.push(dose);
        let mut model_input = variants[0].clone();
        model_input.simulation.run.parameters.insert(
            "v".into(),
            qspquest_core::model::Quantity {
                value: 4.5,
                unit: "L".into(),
            },
        );
        variants.push(model_input);
        let mut covariate = variants[0].clone();
        covariate.simulation.run.regimen.covariates.insert(
            "weight".into(),
            qspquest_core::model::Quantity {
                value: 70.,
                unit: "kg".into(),
            },
        );
        variants.push(covariate);
        assert_eq!(simulation_groups(&variants).unwrap(), vec![0, 0, 1, 2, 3]);
    }

    #[test]
    fn identical_envelopes_share_trajectory_not_likelihood_and_cache_evicts() {
        let (model, request, subjects) = cache_fixture();
        let mut evaluator = Evaluator {
            model: &model,
            request: &request,
            subjects,
            parameter: "cl",
            unit: "L/h",
            calls: 0,
            warm_etas: vec![0.; 2],
            simulation_groups: simulation_groups(&request.subjects).unwrap(),
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
            cached_rows: 0,
            trajectory_cache: HashMap::new(),
            trajectory_order: VecDeque::new(),
            cached_trajectory_values: 0,
            trajectory_value_limit: MAX_CACHED_TRAJECTORY_VALUES,
            trajectory_runs: 0,
        };
        let first = evaluator.subject(0, 0.6, 0., None).unwrap();
        let second = evaluator.subject(1, 0.6, 0., None).unwrap();
        assert_eq!(first.rows[0].prediction, second.rows[0].prediction);
        assert_ne!(first.nll, second.nll);
        assert_eq!(evaluator.calls, 2);
        assert_eq!(evaluator.trajectory_runs, 1);
        assert_eq!(evaluator.trajectory_cache.len(), 1);

        evaluator.trajectory_value_limit = evaluator.cached_trajectory_values;
        evaluator.subject(0, 0.6, 0.1, None).unwrap();
        assert_eq!(evaluator.trajectory_runs, 2);
        assert_eq!(evaluator.trajectory_cache.len(), 1);
        assert_eq!(evaluator.trajectory_order.len(), 1);
        // Remove the separate subject-likelihood cache to exercise the
        // evicted trajectory again. The solver must rerun at the old eta.
        evaluator.cache.clear();
        evaluator.cache_order.clear();
        evaluator.cached_rows = 0;
        evaluator.subject(1, 0.6, 0., None).unwrap();
        assert_eq!(evaluator.trajectory_runs, 3);
        assert_eq!(
            evaluator.cached_trajectory_values,
            evaluator.trajectory_value_limit
        );
    }

    #[test]
    fn residual_sd_changes_likelihood_without_repeating_trajectory() {
        let (model, request, subjects) = cache_fixture();
        let mut evaluator = Evaluator {
            model: &model,
            request: &request,
            subjects,
            parameter: "cl",
            unit: "L/h",
            calls: 0,
            warm_etas: vec![0.; 2],
            simulation_groups: simulation_groups(&request.subjects).unwrap(),
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
            cached_rows: 0,
            trajectory_cache: HashMap::new(),
            trajectory_order: VecDeque::new(),
            cached_trajectory_values: 0,
            trajectory_value_limit: MAX_CACHED_TRAJECTORY_VALUES,
            trajectory_runs: 0,
        };
        let narrow = evaluator.subject(0, 0.6, 0., Some(0.1)).unwrap();
        let wide = evaluator.subject(0, 0.6, 0., Some(0.2)).unwrap();
        assert_ne!(narrow.nll, wide.nll);
        assert_eq!(narrow.rows[0].prediction, wide.rows[0].prediction);
        assert_eq!(evaluator.calls, 2);
        assert_eq!(evaluator.trajectory_runs, 1);
        let repeated = evaluator.subject(0, 0.6, 0., Some(0.1)).unwrap();
        assert_eq!(repeated.nll, narrow.nll);
        assert_eq!(evaluator.calls, 2);
    }

    #[test]
    fn additive_linear_random_intercept_matches_exact_marginal() {
        // Independent closed-form oracle: y=(11,12), f=10+eta,
        // Omega=4, R=1. The Gaussian integral gives OFV=log(9)+1.
        let omega = 4.;
        let eta = 4. / 3.;
        let mut nll = 0.;
        let mut score = 0.;
        let mut information = 1. / omega;
        for value in [11., 12.] {
            let observation = Observation {
                output: 0,
                row: 0,
                value,
                additive: 1.,
                proportional: 0.,
            };
            let terms = row_terms(&observation, 10. + eta, 1., observation.additive).unwrap();
            nll += terms.nll;
            score += terms.score;
            information += terms.information;
        }
        assert!((score + eta / omega).abs() < 1e-13);
        assert!((information - 2.25).abs() < 1e-13);
        let ofv = 2. * nll - 2. * LOG_2PI + eta * eta / omega + libm::log(omega * information);
        assert!((ofv - 3.1972245773362196).abs() < 1e-12);
    }

    #[test]
    fn combined_variance_interaction_matches_independent_scalar_oracle() {
        // Independent bisection oracle for y=1.5, f=1+eta,
        // R=0.1^2+(0.2*f)^2 and Omega=0.25.
        let eta = 0.3468715865078824;
        let observation = Observation {
            output: 0,
            row: 0,
            value: 1.5,
            additive: 0.1,
            proportional: 0.2,
        };
        let terms = row_terms(&observation, 1. + eta, 1., observation.additive).unwrap();
        assert!((terms.score + eta / 0.25).abs() < 1e-11);
        let information = 1. / 0.25 + terms.information;
        assert!((information - 16.963634552701741).abs() < 1e-11);
        let ofv = 2. * terms.nll - LOG_2PI + eta * eta / 0.25 + libm::log(0.25 * information);
        assert!((ofv - (-0.28413556586194666)).abs() < 1e-11);
    }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    #[test]
    fn all_focei_branches_validate_rows_and_error_families_before_readouts() {
        for two in [false, true] {
            let (model, request) = super::super::preparation_tests::population_fixture(two);
            let mut invalid = request.clone();
            invalid.subjects[1].observations[0].row = 999;
            let error = model.fit_population_focei(&invalid).unwrap_err();
            assert_eq!(
                error.expression.as_deref(),
                Some("subjects[1].observations[0]")
            );
            invalid = request;
            invalid.subjects[1].observations[0].error.model = GaussianErrorModel::Lognormal;
            let error = model.fit_population_focei(&invalid).unwrap_err();
            assert_eq!(error.code, ErrorCode::Unsupported);
            assert!(error.message.contains("normal uncensored"));
        }
    }
}

fn validate_dispatch(request: &PopulationFitRequest) -> Result<(), Error> {
    if request.covariate_effect.is_some() {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "FOCEI does not support the SAEM covariate-effect request",
        ));
    }
    Ok(())
}
fn describe_observations(
    model: &CompiledSensitivityDocument,
    observations: &[Observation],
) -> Vec<qspquest_core::planning::GaussianObservationPlan> {
    observations
        .iter()
        .map(|row| qspquest_core::planning::GaussianObservationPlan {
            row: row.row,
            output: model.output_names()[row.output].clone(),
            unit: model.output_units()[row.output].clone(),
            value: row.value,
            additive_sd: row.additive,
            proportional_sd: row.proportional,
            model: GaussianErrorModel::Normal,
            censoring: None,
        })
        .collect()
}
impl CompiledSensitivityDocument {
    pub(super) fn describe_population_focei(
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
        validate_dispatch(request)?;
        if request.random_effects.len() == 2 {
            return multivariate::describe_population_focei_2d(self, request, plan);
        }
        let prepared = self.prepare_population_focei_1d_observed(request, Some(plan))?;
        let mut population = super::fit_planning::population(request);
        population.residual =
            prepared
                .additive_fit
                .map(|r| qspquest_core::planning::ResidualFitPlan {
                    output: r.output,
                    output_unit: r.unit,
                    initial: r.start,
                    lower: r.lower,
                    upper: r.upper,
                    parameterization:
                        qspquest_core::planning::ResidualParameterization::StandardDeviation,
                });
        Ok((
            super::fit_planning::one_coordinate(&prepared.setup)?,
            population,
        ))
    }
}

//! Bounded two-effect FOCEI with a fixed, possibly correlated Omega matrix.
//! Conditional modes use a damped expected-information step and deterministic
//! multistart guard. The outer search estimates two fixed effects only.
use super::*;

type Vector = [f64; 2];
type Matrix = [[f64; 2]; 2];
type CacheKey = (usize, u64, u64, u64, u64);

struct PreparedFoceiTwoDim {
    setup: Setup,
    subjects: Vec<Subject>,
}

#[derive(Clone)]
struct Setup {
    names: [String; 2],
    units: [String; 2],
    start: Vector,
    lower: Vector,
    upper: Vector,
    precision: Matrix,
    det_omega: f64,
}

#[derive(Clone)]
struct Conditional2 {
    eta: Vector,
    nll: f64,
    score: Vector,
    information: Matrix,
    rows: Vec<PredictionRow>,
}

struct Evaluation2 {
    ofv: f64,
    subjects: Vec<PopulationSubjectEstimate>,
    rows: Vec<PopulationFittedObservation>,
}

struct Evaluator2<'a> {
    model: &'a Arc<CompiledSensitivityDocument>,
    request: &'a PopulationFitRequest,
    setup: Setup,
    subjects: Vec<Subject>,
    calls: usize,
    warm_etas: Vec<Vector>,
    cache: HashMap<CacheKey, Conditional2>,
    cache_order: VecDeque<CacheKey>,
    cached_rows: usize,
}

fn determinant(matrix: Matrix) -> f64 {
    matrix[0][0] * matrix[1][1] - matrix[0][1] * matrix[1][0]
}

fn inverse_positive(matrix: Matrix) -> Option<(Matrix, f64)> {
    let det = determinant(matrix);
    if !matrix.iter().flatten().all(|value| value.is_finite())
        || matrix[0][0] <= 0.
        || matrix[1][1] <= 0.
        || !det.is_finite()
        || det <= 1e-12 * matrix[0][0] * matrix[1][1]
    {
        return None;
    }
    Some((
        [
            [matrix[1][1] / det, -matrix[0][1] / det],
            [-matrix[1][0] / det, matrix[0][0] / det],
        ],
        det,
    ))
}

fn product(matrix: Matrix, vector: Vector) -> Vector {
    [
        matrix[0][0] * vector[0] + matrix[0][1] * vector[1],
        matrix[1][0] * vector[0] + matrix[1][1] * vector[1],
    ]
}

fn dot(left: Vector, right: Vector) -> f64 {
    left[0] * right[0] + left[1] * right[1]
}

fn validated_setup(
    model: &Arc<CompiledSensitivityDocument>,
    request: &PopulationFitRequest,
) -> Result<Setup, Error> {
    if request.uncertainty {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "two-effect FOCEI covariance is not implemented",
        ));
    }
    if request.additive_error_fit.is_some() {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "two-effect FOCEI additive-error estimation is not implemented",
        ));
    }
    if request.subjects.is_empty()
        || request.subjects.len() > 128
        || request.fixed_effects.len() != 2
        || request.random_effects.len() != 2
        || request.omega.len() != 2
        || request.omega.iter().any(|row| row.len() != 2)
        || request.omega_diagonal_bounds.len() != 2
        || model.parameters().len() != 2
    {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "two-effect FOCEI requires two selected fixed/random effects and 1..=128 subjects",
        ));
    }
    if !request.eta_bound.is_finite()
        || request.eta_bound <= 0.
        || request.eta_bound > 8.
        || request.max_evaluations == 0
        || request.max_evaluations > 100_000
        || request.max_iterations == 0
        || request.max_iterations > 10_000
        || !request.gradient_tolerance.is_finite()
        || request.gradient_tolerance <= 0.
    {
        return Err(invalid("two-effect FOCEI needs bounded positive controls"));
    }
    let names = [model.parameters()[0].clone(), model.parameters()[1].clone()];
    let mut units = [String::new(), String::new()];
    let mut start = [0.; 2];
    let mut lower = [0.; 2];
    let mut upper = [0.; 2];
    for axis in 0..2 {
        if request.fixed_effects[axis].parameter != names[axis]
            || request.random_effects[axis].parameter != names[axis]
        {
            return Err(invalid(
                "two-effect fixed/random effect order must match selected sensitivity parameters",
            ));
        }
        let parameter = model
            .primal
            .parameters
            .iter()
            .find(|parameter| parameter.name == names[axis])
            .ok_or_else(|| invalid("unknown population parameter"))?;
        if parameter.fixed {
            return Err(invalid("cannot fit a fixed model parameter"));
        }
        let unit = Unit::parse(&parameter.default.unit)?;
        lower[axis] = quantity(&request.fixed_effects[axis].lower)?.in_unit(unit)?;
        upper[axis] = quantity(&request.fixed_effects[axis].upper)?.in_unit(unit)?;
        start[axis] = request.subjects[0]
            .simulation
            .run
            .parameters
            .get(&names[axis])
            .map(|value| quantity(value)?.in_unit(unit))
            .transpose()?
            .unwrap_or(quantity(&parameter.default)?.in_unit(unit)?);
        if !lower[axis].is_finite()
            || !upper[axis].is_finite()
            || !start[axis].is_finite()
            || lower[axis] <= 0.
            || lower[axis] >= upper[axis]
            || start[axis] < lower[axis]
            || start[axis] > upper[axis]
        {
            return Err(invalid(
                "lognormal fixed effects need positive finite boxes containing their starts",
            ));
        }
        if let Some(bounds) = &parameter.bounds {
            if lower[axis] * libm::exp(-request.eta_bound) < quantity(&bounds[0])?.in_unit(unit)?
                || upper[axis] * libm::exp(request.eta_bound)
                    > quantity(&bounds[1])?.in_unit(unit)?
            {
                return Err(invalid(
                    "two-effect theta/eta boxes exceed model parameter bounds",
                ));
            }
        }
        units[axis] = parameter.default.unit.clone();
    }
    let omega = [
        [request.omega[0][0], request.omega[0][1]],
        [request.omega[1][0], request.omega[1][1]],
    ];
    if omega[0][1] != omega[1][0]
        || request
            .omega_diagonal_bounds
            .iter()
            .enumerate()
            .any(|(axis, bounds)| bounds[0] != omega[axis][axis] || bounds[1] != omega[axis][axis])
    {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "two-effect FOCEI requires fixed, symmetric Omega including fixed variances",
        ));
    }
    let (precision, det_omega) = inverse_positive(omega).ok_or_else(|| {
        invalid("two-effect Omega must be finite, symmetric and positive definite")
    })?;
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
                .position(|parameter| parameter.name == names[axis])
                .unwrap();
            if (bound.primal.values[parameter_index] - start[axis]).abs()
                > 1e-10 * start[axis].abs().max(1.)
            {
                return Err(invalid(format!(
                    "subject {index} has a conflicting fixed-effect start"
                )));
            }
        }
    }
    Ok(Setup {
        names,
        units,
        start,
        lower,
        upper,
        precision,
        det_omega,
    })
}

impl Evaluator2<'_> {
    fn subject(&mut self, index: usize, theta: Vector, eta: Vector) -> Result<Conditional2, Error> {
        let key = (
            index,
            theta[0].to_bits(),
            theta[1].to_bits(),
            eta[0].to_bits(),
            eta[1].to_bits(),
        );
        if let Some(value) = self.cache.get(&key) {
            return Ok(value.clone());
        }
        if self.calls >= self.request.max_evaluations {
            return Err(Error::new(
                ErrorCode::WorkBudget,
                "two-effect FOCEI evaluation limit reached",
            ));
        }
        self.calls += 1;
        let individual = [theta[0] * libm::exp(eta[0]), theta[1] * libm::exp(eta[1])];
        if individual
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.)
        {
            return Err(Error::new(
                ErrorCode::Domain,
                "invalid individual parameter",
            ));
        }
        let subject = &self.subjects[index];
        let run = &subject.simulation.run;
        let mut parameters = run.parameters.clone();
        for axis in 0..2 {
            parameters.insert(
                self.setup.names[axis].clone(),
                qspquest_core::model::Quantity {
                    value: individual[axis],
                    unit: self.setup.units[axis].clone(),
                },
            );
        }
        let rows = self.model.simulate_regimen_initial(
            &parameters,
            &run.regimen,
            &subject.simulation.derivative_absolute_tolerances,
            run.budgets,
            &run.initial_states,
            run.solver,
        )?;
        let width = self.model.output_names().len();
        if rows
            .len()
            .checked_mul(width)
            .and_then(|count| count.checked_mul(3))
            .is_none_or(|count| count > run.budgets.output_values)
        {
            return Err(Error::new(
                ErrorCode::OutputBudget,
                "two-effect sensitivity projection exceeds output budget",
            ));
        }
        let mut nll = 0.;
        let mut score = [0.; 2];
        let mut information = [[0.; 2]; 2];
        let mut predictions = Vec::with_capacity(subject.observations.len());
        for (observation_index, observation) in subject.observations.iter().enumerate() {
            let row = rows.get(observation.row).ok_or_else(|| {
                invalid("two-effect observation row is outside simulation result")
            })?;
            let prediction = row.values[observation.output];
            let f_eta = [
                row.derivatives[0][observation.output] * individual[0],
                row.derivatives[1][observation.output] * individual[1],
            ];
            let prop = observation.proportional;
            let variance =
                observation.additive * observation.additive + (prop * prediction).powi(2);
            if !variance.is_finite()
                || variance <= 0.
                || !prediction.is_finite()
                || f_eta.iter().any(|value| !value.is_finite())
            {
                return Err(Error::new(ErrorCode::Domain, format!("subject {index} observation {observation_index} has invalid residual variance")));
            }
            let residual = observation.value - prediction;
            let z2 = residual * residual / variance;
            nll += 0.5 * (z2 + libm::log(variance) + LOG_2PI);
            let r_eta = [
                2. * prop * prop * prediction * f_eta[0] / variance,
                2. * prop * prop * prediction * f_eta[1] / variance,
            ];
            for axis in 0..2 {
                score[axis] += -residual * f_eta[axis] / variance + 0.5 * (1. - z2) * r_eta[axis];
                for other in 0..2 {
                    information[axis][other] +=
                        f_eta[axis] * f_eta[other] / variance + 0.5 * r_eta[axis] * r_eta[other];
                }
            }
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
        if !nll.is_finite()
            || score.iter().any(|value| !value.is_finite())
            || information.iter().flatten().any(|value| !value.is_finite())
        {
            return Err(Error::new(
                ErrorCode::Domain,
                "two-effect likelihood overflow",
            ));
        }
        let conditional = Conditional2 {
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

    fn joint(&self, conditional: &Conditional2) -> f64 {
        conditional.nll
            + 0.5
                * dot(
                    conditional.eta,
                    product(self.setup.precision, conditional.eta),
                )
    }

    fn mode_from_seed(
        &mut self,
        index: usize,
        theta: Vector,
        initial: Vector,
    ) -> Result<Conditional2, Error> {
        let mut current = self.subject(index, theta, initial)?;
        let limit = self.request.eta_bound;
        for _ in 0..48 {
            let prior_score = product(self.setup.precision, current.eta);
            let score = [
                current.score[0] + prior_score[0],
                current.score[1] + prior_score[1],
            ];
            if score[0].abs().max(score[1].abs()) <= 1e-5 {
                break;
            }
            let information = [
                [
                    self.setup.precision[0][0] + current.information[0][0],
                    self.setup.precision[0][1] + current.information[0][1],
                ],
                [
                    self.setup.precision[1][0] + current.information[1][0],
                    self.setup.precision[1][1] + current.information[1][1],
                ],
            ];
            let (inverse, _) = inverse_positive(information).ok_or_else(|| {
                Error::new(
                    ErrorCode::Unsupported,
                    "two-effect conditional information is not positive definite",
                )
            })?;
            let direction = product(inverse, score);
            let joint = self.joint(&current);
            let mut accepted = None;
            for reduction in 0..22 {
                let scale = 2f64.powi(-reduction);
                let trial_eta = [
                    (current.eta[0] - scale * direction[0]).clamp(-limit, limit),
                    (current.eta[1] - scale * direction[1]).clamp(-limit, limit),
                ];
                if trial_eta == current.eta {
                    continue;
                }
                let decrease = dot(
                    score,
                    [current.eta[0] - trial_eta[0], current.eta[1] - trial_eta[1]],
                );
                if decrease <= 0. {
                    continue;
                }
                let trial = self.subject(index, theta, trial_eta)?;
                if self.joint(&trial) <= joint - 1e-4 * decrease {
                    accepted = Some(trial);
                    break;
                }
            }
            match accepted {
                Some(next) => current = next,
                None => break,
            }
        }
        let prior_score = product(self.setup.precision, current.eta);
        let norm = (current.score[0] + prior_score[0])
            .abs()
            .max((current.score[1] + prior_score[1]).abs());
        if norm > 1e-3 || current.eta.iter().any(|value| value.abs() >= limit - 1e-6) {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "two-effect conditional mode is unresolved or reaches eta bound",
            ));
        }
        Ok(current)
    }

    fn mode(
        &mut self,
        index: usize,
        theta: Vector,
    ) -> Result<(Conditional2, Vec<PredictionRow>), Error> {
        let zero = [0.; 2];
        let population_rows = self.subject(index, theta, zero)?.rows;
        let half = 0.5 * self.request.eta_bound;
        let starts = [
            self.warm_etas[index],
            zero,
            [half, 0.],
            [-half, 0.],
            [0., half],
            [0., -half],
        ];
        let mut best: Option<Conditional2> = None;
        for start in starts {
            let candidate = self.mode_from_seed(index, theta, start)?;
            if let Some(previous) = &best {
                if (candidate.eta[0] - previous.eta[0])
                    .abs()
                    .max((candidate.eta[1] - previous.eta[1]).abs())
                    > 1e-3
                {
                    return Err(Error::new(
                        ErrorCode::Unsupported,
                        "two-effect conditional likelihood has multiple detected modes",
                    ));
                }
            }
            if best
                .as_ref()
                .is_none_or(|value| self.joint(&candidate) < self.joint(value))
            {
                best = Some(candidate);
            }
        }
        Ok((best.unwrap(), population_rows))
    }

    fn population(&mut self, theta: Vector) -> Result<Evaluation2, Error> {
        let mut ofv = 0.;
        let mut estimates = Vec::with_capacity(self.subjects.len());
        let mut fitted = Vec::new();
        for index in 0..self.subjects.len() {
            let (mode, population_rows) = self.mode(index, theta).map_err(|mut error| {
                error.expression = Some(format!("subjects[{index}]"));
                error
            })?;
            let curvature = [
                [
                    self.setup.precision[0][0] + mode.information[0][0],
                    self.setup.precision[0][1] + mode.information[0][1],
                ],
                [
                    self.setup.precision[1][0] + mode.information[1][0],
                    self.setup.precision[1][1] + mode.information[1][1],
                ],
            ];
            let (_, det_curvature) = inverse_positive(curvature).ok_or_else(|| {
                Error::new(
                    ErrorCode::Unsupported,
                    "two-effect FOCEI information is not positive definite",
                )
            })?;
            self.warm_etas[index] = mode.eta;
            ofv += 2. * mode.nll - self.subjects[index].observations.len() as f64 * LOG_2PI
                + dot(mode.eta, product(self.setup.precision, mode.eta))
                + libm::log(self.setup.det_omega * det_curvature);
            estimates.push(PopulationSubjectEstimate {
                eta: mode.eta.to_vec(),
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
                        "two-effect prediction grids differ across eta",
                    ));
                }
                let unit = self.model.output_units()[observation.output].clone();
                let quantity = |value| qspquest_core::model::Quantity {
                    value,
                    unit: unit.clone(),
                };
                fitted.push(PopulationFittedObservation {
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
            return Err(Error::new(
                ErrorCode::Domain,
                "two-effect FOCEI objective overflow",
            ));
        }
        Ok(Evaluation2 {
            ofv,
            subjects: estimates,
            rows: fitted,
        })
    }
}

fn prepare_population_focei_2d(
    model: &Arc<CompiledSensitivityDocument>,
    request: &PopulationFitRequest,
) -> Result<PreparedFoceiTwoDim, Error> {
    prepare_population_focei_2d_observed(model, request, None)
}
fn prepare_population_focei_2d_observed(
    model: &Arc<CompiledSensitivityDocument>,
    request: &PopulationFitRequest,
    mut plan: Option<&mut super::super::fit_planning::FitPlanBuilder>,
) -> Result<PreparedFoceiTwoDim, Error> {
    let setup = validated_setup(model, request)?;
    let total_observations = request
        .subjects
        .iter()
        .try_fold(0usize, |total, subject| {
            total.checked_add(subject.observations.len())
        })
        .ok_or_else(|| invalid("two-effect observation count overflow"))?;
    if total_observations > 100_000 {
        return Err(Error::new(
            ErrorCode::OutputBudget,
            "two-effect FOCEI supports at most 100000 fitted rows",
        ));
    }
    let subjects = request.subjects.iter().enumerate().map(|(index, subject)| {
        let run = &subject.simulation.run;
        if !matches!(run.solver, Solver::DiffsolBdf | Solver::DiffsolRodas5p) || run.seed.is_some() {
            return Err(Error::new(ErrorCode::Unsupported, "sensitivity runs require deterministic diffsol_bdf or diffsol_rodas5p"));
        }
        if run.request_id.as_ref().is_some_and(|id| id.is_empty() || id.len() > 128)
            || run.budgets.solver_callbacks > u32::MAX as u64
            || run.budgets.events > u32::MAX as usize
            || run.budgets.output_values > u32::MAX as usize
        {
            return Err(invalid("two-effect run identity or budgets are invalid"));
        }
        if subject.observations.is_empty()
            || subject.observations.len() > 1_000_000
            || subject.observations.len() > run.budgets.output_values
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
            let output = model.output_names().iter().position(|name| name == &observation.output)
                .ok_or_else(|| invalid("unknown two-effect observation output"))?;
            let unit = Unit::parse(&model.output_units()[output])?;
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
    for (index, subject) in request.subjects.iter().enumerate() {
        let prepared = model.prepare_run(&subject.simulation)?;
        for (row, observation) in subject.observations.iter().enumerate() {
            if observation.row >= prepared.result_rows() {
                let mut error = invalid("FOCEI observation row is outside the simulation result");
                error.expression = Some(format!("subjects[{index}].observations[{row}]"));
                return Err(error);
            }
        }
        if let Some(plan) = plan.as_deref_mut() {
            plan.focei(
                model,
                &subject.simulation,
                &prepared,
                || describe_observations(model, &subjects[index].observations),
                subject.observations.len(),
            )?;
        }
    }
    Ok(PreparedFoceiTwoDim { setup, subjects })
}

pub(super) fn fit_population_focei_2d(
    model: &Arc<CompiledSensitivityDocument>,
    request: &PopulationFitRequest,
) -> Result<PopulationFitResult, Error> {
    let PreparedFoceiTwoDim { setup, subjects } = prepare_population_focei_2d(model, request)?;
    let mut evaluator = Evaluator2 {
        model,
        request,
        setup: setup.clone(),
        subjects,
        calls: 0,
        warm_etas: vec![[0.; 2]; request.subjects.len()],
        cache: HashMap::new(),
        cache_order: VecDeque::new(),
        cached_rows: 0,
    };
    let mut x = [
        (setup.start[0] - setup.lower[0]) / (setup.upper[0] - setup.lower[0]),
        (setup.start[1] - setup.lower[1]) / (setup.upper[1] - setup.lower[1]),
    ];
    let decode = |point: Vector| {
        [
            setup.lower[0] + point[0] * (setup.upper[0] - setup.lower[0]),
            setup.lower[1] + point[1] * (setup.upper[1] - setup.lower[1]),
        ]
    };
    let mut best = evaluator.population(decode(x))?;
    let mut steps: Vector = [0.2, 0.2];
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
        for axis in 0..2 {
            for direction in [-1., 1.] {
                let mut trial = x;
                trial[axis] = (trial[axis] + direction * steps[axis]).clamp(0., 1.);
                if trial == x {
                    continue;
                }
                match evaluator.population(decode(trial)) {
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
    let theta = decode(x);
    Ok(PopulationFitResult {
        fixed_effect_intervals: None,
        fixed_effect_interval_error: None,
        status,
        fixed_effects: BTreeMap::from([
            (
                setup.names[0].clone(),
                qspquest_core::model::Quantity {
                    value: theta[0],
                    unit: setup.units[0].clone(),
                },
            ),
            (
                setup.names[1].clone(),
                qspquest_core::model::Quantity {
                    value: theta[1],
                    unit: setup.units[1].clone(),
                },
            ),
        ]),
        omega: request.omega.clone(),
        covariate_effect: None,
        subjects: best.subjects,
        fitted_observations: Some(best.rows),
        uncertainty: None,
        estimated_residual_error: None,
        saem_diagnostics: None,
        objective_kind: PopulationObjectiveKind::FoceiObjectiveFunctionValue,
        objective: best.ofv,
        evaluations: evaluator.calls,
        iterations,
        fit_request_hash: canonical_hash(
            &serde_json::json!({"algorithm":"focei_2d_fixed_omega_v1","request":request}),
        )?,
    })
}

pub(super) fn describe_population_focei_2d(
    model: &Arc<CompiledSensitivityDocument>,
    request: &PopulationFitRequest,
    plan: &mut super::super::fit_planning::FitPlanBuilder,
) -> Result<
    (
        Vec<qspquest_core::planning::FitCoordinatePlan>,
        qspquest_core::planning::PopulationPreparationPlan,
    ),
    Error,
> {
    let p = prepare_population_focei_2d_observed(model, request, Some(plan))?;
    let coordinates = (0..2)
        .map(|i| {
            super::super::fit_planning::coordinate(
                &p.setup.names[i],
                &p.setup.units[i],
                p.setup.start[i],
                p.setup.lower[i],
                p.setup.upper[i],
                qspquest_core::planning::FitSearchCoordinate::AffineUnitBox,
                (p.setup.start[i] - p.setup.lower[i]) / (p.setup.upper[i] - p.setup.lower[i]),
            )
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok((coordinates, super::super::fit_planning::population(request)))
}

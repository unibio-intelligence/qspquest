//! Projected inverse-BFGS with an Armijo line search in affine box coordinates.
use super::*;
use qspquest_core::{fit::*, identity::canonical_hash};
fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
fn identity(n: usize) -> Vec<f64> {
    let mut h = vec![0.; n * n];
    for i in 0..n {
        h[i * n + i] = 1.;
    }
    h
}
fn projected(x: &[f64], g: &[f64]) -> Vec<f64> {
    x.iter()
        .zip(g)
        .map(|(x, g)| {
            if (*x == 0. && *g > 0.) || (*x == 1. && *g < 0.) {
                0.
            } else {
                *g
            }
        })
        .collect()
}
pub(super) struct PreparedGaussianFit {
    pub(super) lower: Vec<f64>,
    pub(super) upper: Vec<f64>,
    pub(super) spans: Vec<f64>,
    pub(super) units: Vec<String>,
    pub(super) x: Vec<f64>,
    pub(super) starts: Vec<f64>,
}

/// Validated one-dimensional population slice shared by Laplace, FOCEI and SAEM.
pub(super) struct PopulationOneDim {
    pub parameter: String,
    pub unit: String,
    pub start: f64,
    pub lower: f64,
    pub upper: f64,
    pub omega: f64,
    pub omega_lower: f64,
    pub omega_upper: f64,
}
impl CompiledSensitivityDocument {
    pub fn execute_fit(self: &Arc<Self>, request: &FitRequest) -> Result<FitResult, Error> {
        match &request.problem {
            FitProblem::Individual(r)
            | FitProblem::Pooled(PooledGaussianFitRequest { fit: r, .. }) => {
                let mut fit = match &request.problem {
                    FitProblem::Individual(_) => self.fit_gaussian(r)?,
                    FitProblem::Pooled(p) => self.fit_pooled_gaussian(p)?,
                    _ => unreachable!(),
                };
                fit.fit_request_hash = canonical_hash(
                    &serde_json::json!({"schema":request.schema,"kind":match &request.problem { FitProblem::Individual(_) => "individual", FitProblem::Pooled(_) => "pooled", _ => unreachable!() },"fit_request_hash":fit.fit_request_hash}),
                )?;
                Ok(FitResult {
                    schema: FitResultSchema::V01,
                    fit: Some(fit),
                    population: None,
                    residual_scales: None,
                })
            }
            FitProblem::ProfiledProportional(r) => {
                let mut result = self.fit_profiled_proportional_gaussian(r)?;
                result.fit.fit_request_hash = canonical_hash(
                    &serde_json::json!({"schema":request.schema,"kind":"profiled_proportional","fit_request_hash":result.fit.fit_request_hash}),
                )?;
                Ok(FitResult {
                    schema: FitResultSchema::V01,
                    fit: Some(result.fit),
                    population: None,
                    residual_scales: Some(result.residual_scales),
                })
            }
            FitProblem::Laplace(r) | FitProblem::Focei(r) | FitProblem::Saem(r) => {
                let mut population = match &request.problem {
                    FitProblem::Laplace(_) => self.fit_population_laplace(r)?,
                    FitProblem::Focei(_) => self.fit_population_focei(r)?,
                    FitProblem::Saem(_) => self.fit_population_saem(r)?,
                    _ => unreachable!(),
                };
                population.fit_request_hash = canonical_hash(
                    &serde_json::json!({"schema":request.schema,"problem":request.problem,"fit_request_hash":population.fit_request_hash}),
                )?;
                Ok(FitResult {
                    schema: FitResultSchema::V01,
                    fit: None,
                    population: Some(population),
                    residual_scales: None,
                })
            }
        }
    }

    pub(super) fn validate_population_1d(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
    ) -> Result<PopulationOneDim, Error> {
        if request.subjects.is_empty()
            || request.subjects.len() > 1024
            || request.fixed_effects.len() != 1
            || request.random_effects.len() != 1
            || request.omega.len() != 1
            || request.omega[0].len() != 1
            || request.omega_diagonal_bounds.len() != 1
            || self.parameters().len() != 1
        {
            return Err(Error::new(ErrorCode::Unsupported, "population fitting currently supports one selected fixed effect and one lognormal random effect across 1..=1024 subjects"));
        }
        let name = &request.fixed_effects[0].parameter;
        if name != &request.random_effects[0].parameter || name != &self.parameters()[0] {
            return Err(invalid(
                "population fixed and random effects must name the selected parameter",
            ));
        }
        if request
            .subjects
            .iter()
            .any(|s| !s.priors.is_empty() || s.simulation.with_respect_to != self.parameters())
        {
            return Err(invalid("population subjects must share the selected parameter and use no individual priors"));
        }
        if !request.eta_bound.is_finite()
            || request.eta_bound <= 0.
            || request.eta_bound > 12.
            || request.max_evaluations == 0
            || request.max_evaluations > 100_000
            || request.max_iterations == 0
            || request.max_iterations > 10_000
            || !request.gradient_tolerance.is_finite()
            || request.gradient_tolerance <= 0.
        {
            return Err(invalid(
                "population fitting needs finite positive bounded controls",
            ));
        }
        let model_parameter = self
            .primal
            .parameters
            .iter()
            .find(|p| &p.name == name)
            .ok_or_else(|| invalid("unknown population parameter"))?;
        if model_parameter.fixed {
            return Err(invalid("cannot fit a fixed model parameter"));
        }
        let unit = Unit::parse(&model_parameter.default.unit)?;
        let lower = quantity(&request.fixed_effects[0].lower)?.in_unit(unit)?;
        let upper = quantity(&request.fixed_effects[0].upper)?.in_unit(unit)?;
        let start = request.subjects[0]
            .simulation
            .run
            .parameters
            .get(name)
            .map(|q| quantity(q)?.in_unit(unit))
            .transpose()?
            .unwrap_or(quantity(&model_parameter.default)?.in_unit(unit)?);
        if !lower.is_finite()
            || !upper.is_finite()
            || lower <= 0.
            || lower >= upper
            || start < lower
            || start > upper
        {
            return Err(invalid(
                "lognormal fixed effect requires positive finite bounds containing its start",
            ));
        }
        if let Some(bounds) = &model_parameter.bounds {
            if lower * libm::exp(-request.eta_bound) < quantity(&bounds[0])?.in_unit(unit)?
                || upper * libm::exp(request.eta_bound) > quantity(&bounds[1])?.in_unit(unit)?
            {
                return Err(invalid(
                    "population fixed effect and eta box exceed model bounds",
                ));
            }
        }
        for (i, subject) in request.subjects.iter().enumerate() {
            let bound = self.bind_with_initial_states(
                &subject.simulation.run.parameters,
                &subject.simulation.run.regimen.covariates,
                &subject.simulation.run.initial_states,
            )?;
            let index = self
                .primal
                .parameters
                .iter()
                .position(|p| &p.name == name)
                .unwrap();
            if (bound.primal.values[index] - start).abs() > 1e-10 * start.abs().max(1.) {
                return Err(invalid(format!(
                    "subject {i} has a conflicting fixed-effect start"
                )));
            }
        }
        let omega = request.omega[0][0];
        let [omega_lower, omega_upper] = request.omega_diagonal_bounds[0];
        if !omega.is_finite()
            || !omega_lower.is_finite()
            || !omega_upper.is_finite()
            || omega_lower <= 0.
            || omega_lower > omega_upper
            || omega < omega_lower
            || omega > omega_upper
        {
            return Err(invalid(
                "Omega variance and its bounds must be finite and positive",
            ));
        }
        Ok(PopulationOneDim {
            parameter: name.clone(),
            unit: model_parameter.default.unit.clone(),
            start,
            lower,
            upper,
            omega,
            omega_lower,
            omega_upper,
        })
    }

    pub(super) fn population_subject_likelihood(
        self: &Arc<Self>,
        subject: &GaussianObjectiveRequest,
        parameter: &str,
        theta: f64,
        unit: &str,
        eta: f64,
    ) -> Result<GaussianObjectiveResult, Error> {
        if !subject.priors.is_empty() {
            return Err(invalid("population subject priors are unsupported"));
        }
        let value = theta * libm::exp(eta);
        if !value.is_finite() || value <= 0. {
            return Err(Error::new(
                ErrorCode::Domain,
                "lognormal individual parameter is invalid",
            ));
        }
        let mut objective = subject.clone();
        objective.simulation.run.parameters.insert(
            parameter.into(),
            qspquest_core::model::Quantity {
                value,
                unit: unit.into(),
            },
        );
        self.gaussian_objective(&objective)
    }

    pub(super) fn population_subject_primal_nll(
        self: &Arc<Self>,
        subject: &GaussianObjectiveRequest,
        parameter: &str,
        theta: f64,
        unit: &str,
        eta: f64,
    ) -> Result<f64, Error> {
        self.population_subject_primal_nll_and_predictions(subject, parameter, theta, unit, eta)
            .map(|(nll, _)| nll)
    }

    pub(super) fn population_subject_primal_nll_and_predictions(
        self: &Arc<Self>,
        subject: &GaussianObjectiveRequest,
        parameter: &str,
        theta: f64,
        unit: &str,
        eta: f64,
    ) -> Result<(f64, Vec<f64>), Error> {
        if !subject.priors.is_empty() {
            return Err(invalid("population subject priors are unsupported"));
        }
        let value = theta * libm::exp(eta);
        if !value.is_finite() || value <= 0. {
            return Err(Error::new(
                ErrorCode::Domain,
                "lognormal individual parameter is invalid",
            ));
        }
        let mut objective = subject.clone();
        objective.simulation.run.parameters.insert(
            parameter.into(),
            qspquest_core::model::Quantity {
                value,
                unit: unit.into(),
            },
        );
        self.gaussian_primal_nll_and_predictions(&objective)
    }

    /// Individual Gaussian MLE/MAP using explicit finite natural-parameter bounds.
    /// Returns an explicit nonconverged status when an optimization limit is met.
    pub fn fit_gaussian(
        self: &Arc<Self>,
        request: &GaussianFitRequest,
    ) -> Result<GaussianFitResult, Error> {
        self.fit_gaussian_subjects(request, &[], false)
            .map(|(fit, _)| fit)
    }
    /// Pooled MLE/MAP with shared selected parameters and separate subject inputs.
    pub fn fit_pooled_gaussian(
        self: &Arc<Self>,
        request: &PooledGaussianFitRequest,
    ) -> Result<GaussianFitResult, Error> {
        self.fit_gaussian_subjects(&request.fit, &request.additional_subjects, false)
            .map(|(fit, _)| fit)
    }

    /// Bounded joint MLE/MAP of model parameters and one proportional normal
    /// residual scale per output, using exact likelihood profiling.
    pub fn fit_profiled_proportional_gaussian(
        self: &Arc<Self>,
        request: &GaussianFitRequest,
    ) -> Result<ProfiledProportionalGaussianFitResult, Error> {
        let (fit, residual_scales) = self.fit_gaussian_subjects(request, &[], true)?;
        Ok(ProfiledProportionalGaussianFitResult {
            fit,
            residual_scales,
        })
    }
    pub(super) fn prepare_gaussian_fit(
        self: &Arc<Self>,
        request: &GaussianFitRequest,
        additional: &[GaussianObjectiveRequest],
    ) -> Result<PreparedGaussianFit, Error> {
        self.prepare_gaussian_fit_observed(request, additional, false, None)
    }
    pub(super) fn prepare_gaussian_fit_observed(
        self: &Arc<Self>,
        request: &GaussianFitRequest,
        additional: &[GaussianObjectiveRequest],
        profile_proportional: bool,
        mut plan: Option<&mut super::fit_planning::FitPlanBuilder>,
    ) -> Result<PreparedGaussianFit, Error> {
        if additional.len() > 1023 {
            return Err(invalid("pooled fit supports at most 1024 subjects"));
        }
        let n = self.parameters().len();
        if request.bounds.len() != n
            || request.max_evaluations == 0
            || request.max_evaluations > 10000
            || request.max_iterations == 0
            || request.max_iterations > 10000
            || !request.gradient_tolerance.is_finite()
            || request.gradient_tolerance <= 0.
        {
            return Err(invalid(
                "fit requires one finite box per selected parameter and bounded positive controls",
            ));
        }
        let bound = self.bind_with_initial_states(
            &request.objective.simulation.run.parameters,
            &request.objective.simulation.run.regimen.covariates,
            &request.objective.simulation.run.initial_states,
        )?;
        for (subject, objective) in additional.iter().enumerate() {
            if !objective.priors.is_empty() {
                return Err(invalid(
                    "pooled priors must appear only in the primary objective",
                ));
            }
            if objective.simulation.with_respect_to != self.parameters() {
                return Err(invalid(
                    "pooled subjects must share selected parameter order",
                ));
            }
            let other = self.bind_with_initial_states(
                &objective.simulation.run.parameters,
                &objective.simulation.run.regimen.covariates,
                &objective.simulation.run.initial_states,
            )?;
            for name in self.parameters() {
                let index = self
                    .primal
                    .parameters
                    .iter()
                    .position(|p| &p.name == name)
                    .unwrap();
                if other.primal.values[index] != bound.primal.values[index] {
                    return Err(invalid(format!(
                        "subject {} has a conflicting shared initial parameter {name}",
                        subject + 1
                    )));
                }
            }
        }
        let mut lower = Vec::new();
        let mut upper = Vec::new();
        let mut spans = Vec::new();
        let mut units = Vec::new();
        let mut x = Vec::new();
        let mut starts = Vec::new();
        for (name, b) in self.parameters().iter().zip(&request.bounds) {
            if name != &b.parameter {
                return Err(invalid("fit bounds must follow selected parameter order"));
            }
            let index = self
                .primal
                .parameters
                .iter()
                .position(|p| &p.name == name)
                .unwrap();
            let p = &self.primal.parameters[index];
            if p.fixed {
                return Err(invalid("cannot fit a fixed parameter"));
            }
            let unit = Unit::parse(&p.default.unit)?;
            let lo = quantity(&b.lower)?.in_unit(unit)?;
            let hi = quantity(&b.upper)?.in_unit(unit)?;
            let span = hi - lo;
            let start = bound.primal.values[index];
            if !span.is_finite() || span <= 0. || start < lo || start > hi {
                return Err(invalid(
                    "fit box must have finite positive width and contain initial value",
                ));
            }
            if let Some(bounds) = &p.bounds {
                if lo < quantity(&bounds[0])?.in_unit(unit)?
                    || hi > quantity(&bounds[1])?.in_unit(unit)?
                {
                    return Err(invalid("fit box exceeds declared model bounds"));
                }
            }
            lower.push(lo);
            upper.push(hi);
            spans.push(span);
            units.push(p.default.unit.clone());
            x.push((start - lo) / span);
            starts.push(start);
        }
        for (subject, objective) in std::iter::once(&request.objective)
            .chain(additional)
            .enumerate()
        {
            let prepared = self
                .prepare_gaussian_objective(objective)
                .map_err(|mut error| {
                    error.expression = Some(match error.expression {
                        Some(field) => format!("subjects[{subject}].{field}"),
                        None => format!("subjects[{subject}]"),
                    });
                    error
                })?;
            if profile_proportional {
                super::objective::validate_profiled_rows(&prepared.data)?;
            }
            if let Some(plan) = plan.as_deref_mut() {
                plan.gaussian(self, objective, &prepared, profile_proportional)?;
            }
        }
        Ok(PreparedGaussianFit {
            lower,
            upper,
            spans,
            units,
            x,
            starts,
        })
    }
    fn fit_gaussian_subjects(
        self: &Arc<Self>,
        request: &GaussianFitRequest,
        additional: &[GaussianObjectiveRequest],
        profile_proportional: bool,
    ) -> Result<(GaussianFitResult, std::collections::BTreeMap<String, f64>), Error> {
        let PreparedGaussianFit {
            lower,
            upper: _,
            spans,
            units,
            mut x,
            starts: _,
        } = if profile_proportional {
            self.prepare_gaussian_fit_observed(request, additional, true, None)?
        } else {
            self.prepare_gaussian_fit(request, additional)?
        };
        let n = self.parameters().len();
        // Each objective retains its simulation budgets. Evaluation count bounds total work.
        // Refinement only follows a stalled line search, within the original work limits.
        let precision_scale = Cell::new(1.0_f64);
        let evaluate = |x: &[f64]| -> Result<
            (
                GaussianObjectiveResult,
                std::collections::BTreeMap<String, f64>,
            ),
            Error,
        > {
            let mut combined: Option<GaussianObjectiveResult> = None;
            let mut identities = Vec::new();
            let mut scales = std::collections::BTreeMap::new();
            for (subject, template) in std::iter::once(&request.objective)
                .chain(additional)
                .enumerate()
            {
                let mut objective = template.clone();
                let scale = precision_scale.get();
                objective.simulation.run.regimen.rtol *= scale;
                if let Some(atol) = &mut objective.simulation.run.regimen.atol {
                    *atol *= scale;
                }
                if let Some(atol) = &mut objective.simulation.run.regimen.absolute_tolerances {
                    for q in atol.values_mut() {
                        q.value *= scale;
                    }
                }
                for states in objective
                    .simulation
                    .derivative_absolute_tolerances
                    .values_mut()
                {
                    for q in states.values_mut() {
                        q.value *= scale;
                    }
                }
                for i in 0..n {
                    objective.simulation.run.parameters.insert(
                        self.parameters()[i].clone(),
                        qspquest_core::model::Quantity {
                            value: lower[i] + spans[i] * x[i],
                            unit: units[i].clone(),
                        },
                    );
                }
                let evaluation = if profile_proportional {
                    self.profiled_proportional_gaussian_objective(&objective)
                } else {
                    self.gaussian_objective(&objective)
                        .map(|v| (v, std::collections::BTreeMap::new()))
                };
                let (result, residual_scales) = evaluation.map_err(|mut e| {
                    e.expression = Some(format!(
                        "subjects[{subject}].{}",
                        e.expression.unwrap_or_default()
                    ));
                    e
                })?;
                scales = residual_scales;
                identities.push(result.identity.clone());
                if let Some(total) = &mut combined {
                    total.negative_log_likelihood += result.negative_log_likelihood;
                    total.objective += result.objective;
                    total.observations = total
                        .observations
                        .checked_add(result.observations)
                        .ok_or_else(|| invalid("pooled observation count overflow"))?;
                    for (g, other) in total.gradient.iter_mut().zip(result.gradient) {
                        g.value += other.value;
                    }
                } else {
                    combined = Some(result);
                }
            }
            let mut result = combined.expect("primary subject exists");
            if !result.objective.is_finite()
                || !result.negative_log_likelihood.is_finite()
                || result.gradient.iter().any(|g| !g.value.is_finite())
            {
                return Err(Error::new(
                    ErrorCode::Domain,
                    "pooled objective accumulation overflow",
                ));
            }
            if !additional.is_empty() {
                result.identity.bound_value_hash = canonical_hash(&serde_json::json!(identities
                    .iter()
                    .map(|i| &i.bound_value_hash)
                    .collect::<Vec<_>>()))?;
                result.identity.run_request_hash = canonical_hash(&serde_json::json!(identities))?;
                result.identity.algorithm = if identities
                    .iter()
                    .all(|i| i.algorithm == "bdf_forward_gaussian_objective")
                {
                    "bdf_forward_pooled_gaussian_objective"
                } else if identities
                    .iter()
                    .all(|i| i.algorithm == "rodas5p_forward_gaussian_objective")
                {
                    "rodas5p_forward_pooled_gaussian_objective"
                } else {
                    "mixed_forward_pooled_gaussian_objective"
                }
                .into();
            }
            Ok((result, scales))
        };
        let (mut value, mut residual_scales) = evaluate(&x)?;
        let mut evaluations = 1;
        let mut iterations = 0;
        let mut h = identity(n);
        let scaled = |v: &GaussianObjectiveResult| -> Vec<f64> {
            v.gradient
                .iter()
                .zip(&spans)
                .map(|(g, s)| g.value * s)
                .collect()
        };
        let mut g = scaled(&value);
        let status = loop {
            if g.iter().any(|v| !v.is_finite()) {
                return Err(Error::new(
                    ErrorCode::Domain,
                    "scaled fit gradient overflow",
                ));
            }
            let pg = projected(&x, &g);
            if pg.iter().fold(0_f64, |a, b| a.max(b.abs())) <= request.gradient_tolerance {
                break FitStatus::Converged;
            }
            if evaluations >= request.max_evaluations {
                break FitStatus::EvaluationLimit;
            }
            if iterations >= request.max_iterations {
                break FitStatus::IterationLimit;
            }
            let mut d: Vec<f64> = (0..n).map(|i| -dot(&h[i * n..(i + 1) * n], &pg)).collect();
            for i in 0..n {
                if (x[i] == 0. && d[i] < 0.) || (x[i] == 1. && d[i] > 0.) {
                    d[i] = 0.;
                }
            }
            if !dot(&g, &d).is_finite() || dot(&g, &d) >= 0. {
                d = pg.iter().map(|v| -v).collect();
                h = identity(n);
            }
            let mut alpha = 1.;
            let mut accepted = None;
            for _ in 0..48 {
                if evaluations >= request.max_evaluations {
                    break;
                }
                let trial: Vec<f64> = x
                    .iter()
                    .zip(&d)
                    .map(|(x, d)| (x + alpha * d).clamp(0., 1.))
                    .collect();
                let step: Vec<f64> = trial.iter().zip(&x).map(|(a, b)| a - b).collect();
                let slope = dot(&g, &step);
                if slope >= 0. || step.iter().all(|v| *v == 0.) {
                    alpha *= 0.5;
                    continue;
                }
                evaluations += 1;
                match evaluate(&trial) {
                    Ok((v, scales))
                        if {
                            let roundoff = 32.0 * f64::EPSILON * (1.0 + value.objective.abs());
                            let trial_g = scaled(&v);
                            let trial_norm = projected(&trial, &trial_g)
                                .iter()
                                .fold(0_f64, |a, b| a.max(b.abs()));
                            let current_norm = pg.iter().fold(0_f64, |a, b| a.max(b.abs()));
                            v.objective <= value.objective + 1e-4 * slope
                                || (slope.abs() <= roundoff
                                    && v.objective <= value.objective + roundoff
                                    && trial_norm < current_norm)
                        } =>
                    {
                        accepted = Some((trial, step, v, scales));
                        break;
                    }
                    Ok(_) => {}
                    Err(e)
                        if matches!(
                            e.code,
                            ErrorCode::Domain | ErrorCode::Solver | ErrorCode::Invariant
                        ) => {}
                    Err(e) => return Err(e),
                }
                alpha *= 0.5;
            }
            let Some((next, step, v, scales)) = accepted else {
                if evaluations < request.max_evaluations && precision_scale.get() > 1e-4 {
                    precision_scale.set(precision_scale.get() * 0.01);
                    evaluations += 1;
                    (value, residual_scales) = evaluate(&x)?;
                    g = scaled(&value);
                    continue;
                }
                break if evaluations >= request.max_evaluations {
                    FitStatus::EvaluationLimit
                } else {
                    FitStatus::LineSearchFailed
                };
            };
            let next_g = scaled(&v);
            let y: Vec<f64> = next_g.iter().zip(&g).map(|(a, b)| a - b).collect();
            let sy = dot(&step, &y);
            if sy > 1e-12 * libm::sqrt(dot(&step, &step) * dot(&y, &y)) && sy.is_finite() {
                let hy: Vec<f64> = (0..n).map(|i| dot(&h[i * n..(i + 1) * n], &y)).collect();
                let factor = (sy + dot(&y, &hy)) / (sy * sy);
                for i in 0..n {
                    for j in 0..n {
                        h[i * n + j] +=
                            factor * step[i] * step[j] - (hy[i] * step[j] + step[i] * hy[j]) / sy;
                    }
                }
                if h.iter().any(|v| !v.is_finite()) {
                    h = identity(n);
                }
            } else {
                h = identity(n);
            }
            x = next;
            value = v;
            residual_scales = scales;
            g = next_g;
            iterations += 1;
        };
        let parameters = (0..n)
            .map(|i| {
                (
                    self.parameters()[i].clone(),
                    qspquest_core::model::Quantity {
                        value: lower[i] + spans[i] * x[i],
                        unit: units[i].clone(),
                    },
                )
            })
            .collect();
        Ok((
            GaussianFitResult {
                status,
                parameters,
                objective: value,
                evaluations,
                iterations,
                projected_gradient_norm: projected(&x, &g)
                    .iter()
                    .fold(0_f64, |a, b| a.max(b.abs())),
                fit_request_hash: canonical_hash(
                    &serde_json::json!({"algorithm":if profile_proportional {"projected_bfgs_affine_box_profiled_proportional_v1"} else {"projected_bfgs_affine_box_refined_v2"},"request":request,"additional_subjects":additional}),
                )?,
            },
            residual_scales,
        ))
    }
}

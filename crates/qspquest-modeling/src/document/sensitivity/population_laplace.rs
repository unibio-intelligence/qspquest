//! Bounded one-random-effect Laplace approximation to marginal likelihood.
//! The conditional Gaussian likelihood includes prediction-dependent variance.
use super::*;
use qspquest_core::{fit::*, identity::canonical_hash};

struct Eval {
    value: f64,
    subjects: Vec<PopulationSubjectEstimate>,
}

struct LikelihoodGrid {
    theta_bits: u64,
    subjects: Vec<Option<[(f64, f64); 17]>>,
}

struct Evaluator<'a> {
    model: &'a Arc<CompiledSensitivityDocument>,
    request: &'a PopulationFitRequest,
    parameter: &'a str,
    unit: &'a str,
    calls: usize,
    warm_etas: Vec<f64>,
    likelihood_grids: Vec<LikelihoodGrid>,
}

impl Evaluator<'_> {
    fn subject(
        &mut self,
        subject: &GaussianObjectiveRequest,
        theta: f64,
        eta: f64,
    ) -> Result<(f64, f64), Error> {
        if self.calls >= self.request.max_evaluations {
            return Err(Error::new(
                ErrorCode::WorkBudget,
                "Laplace evaluation limit reached",
            ));
        }
        self.calls += 1;
        let result = self.model.population_subject_likelihood(
            subject,
            self.parameter,
            theta,
            self.unit,
            eta,
        )?;
        let score = result.gradient[0].value * theta * libm::exp(eta);
        if !score.is_finite() {
            return Err(Error::new(ErrorCode::Domain, "Laplace score overflow"));
        }
        Ok((result.negative_log_likelihood, score))
    }

    fn likelihood_grid(
        &mut self,
        subject_index: usize,
        subject: &GaussianObjectiveRequest,
        theta: f64,
    ) -> Result<[(f64, f64); 17], Error> {
        let theta_bits = theta.to_bits();
        // The subject likelihood excludes Omega. Keep the last three exact
        // mean values so both coordinate probes and their variance probes can
        // reuse likelihood samples; the prior score is evaluated separately.
        if let Some(index) = self
            .likelihood_grids
            .iter()
            .position(|grid| grid.theta_bits == theta_bits)
        {
            let grid = self.likelihood_grids.remove(index);
            let cached = grid.subjects[subject_index];
            self.likelihood_grids.push(grid);
            if let Some(samples) = cached {
                return Ok(samples);
            }
        }
        let limit = self.request.eta_bound;
        let mut samples = [(0., 0.); 17];
        for (point, sample) in samples.iter_mut().enumerate() {
            let eta = -limit + limit * point as f64 / 8.;
            *sample = self.subject(subject, theta, eta)?;
        }
        if self.likelihood_grids.last().map(|grid| grid.theta_bits) != Some(theta_bits) {
            if self.likelihood_grids.len() == 3 {
                self.likelihood_grids.remove(0);
            }
            self.likelihood_grids.push(LikelihoodGrid {
                theta_bits,
                subjects: vec![None; self.request.subjects.len()],
            });
        }
        self.likelihood_grids.last_mut().unwrap().subjects[subject_index] = Some(samples);
        Ok(samples)
    }

    fn conditional(
        &mut self,
        subject_index: usize,
        subject: &GaussianObjectiveRequest,
        theta: f64,
        omega: f64,
        warm_eta: f64,
    ) -> Result<(f64, f64, f64), Error> {
        let limit = self.request.eta_bound;
        let score = |this: &mut Self, eta: f64| -> Result<(f64, f64), Error> {
            let (nll, derivative) = this.subject(subject, theta, eta)?;
            Ok((nll, derivative + eta / omega))
        };
        // Sample the full declared eta domain to detect extra score roots. The
        // samples also furnish a tight bracket for the safeguarded secant solve.
        let likelihoods = self.likelihood_grid(subject_index, subject, theta)?;
        let mut grid = Vec::with_capacity(17);
        for (point, (nll, derivative)) in likelihoods.into_iter().enumerate() {
            let eta = -limit + limit * point as f64 / 8.;
            grid.push((eta, nll, derivative + eta / omega));
        }
        if grid[0].2 >= 0. || grid[16].2 <= 0. {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Laplace eta mode reaches its bound",
            ));
        }
        let brackets = grid
            .windows(2)
            .filter(|pair| pair[0].2 <= 0. && pair[1].2 > 0.)
            .collect::<Vec<_>>();
        if brackets.len() != 1
            || grid
                .windows(2)
                .any(|pair| pair[0].2 > 0. && pair[1].2 <= 0.)
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Laplace conditional score has multiple modes in the eta domain",
            ));
        }
        let mut left = brackets[0][0];
        let mut right = brackets[0][1];
        let mut best = if left.2.abs() < right.2.abs() {
            left
        } else {
            right
        };
        if warm_eta > left.0 && warm_eta < right.0 {
            let (nll, g) = score(self, warm_eta)?;
            let trial = (warm_eta, nll, g);
            if g > 0. {
                right = trial;
            } else {
                left = trial;
            }
            if g.abs() < best.2.abs() {
                best = trial;
            }
        }
        for _ in 0..16 {
            if best.2.abs() <= 1e-4 || right.0 - left.0 <= 1e-6 {
                break;
            }
            let width = right.0 - left.0;
            let eta = (left.0 - left.2 * width / (right.2 - left.2))
                .clamp(left.0 + 0.1 * width, right.0 - 0.1 * width);
            let (nll, g) = score(self, eta)?;
            let trial = (eta, nll, g);
            if g > 0. {
                right = trial;
            } else {
                left = trial;
            }
            if g.abs() < best.2.abs() {
                best = trial;
            }
        }
        // Flat scores can slow secant interpolation. Finish those subjects
        // with bisection while retaining the same root bracket.
        if best.2.abs() > 1e-4 {
            for _ in 0..18 {
                if best.2.abs() <= 1e-4 || right.0 - left.0 <= 1e-6 {
                    break;
                }
                let eta = (left.0 + right.0) / 2.;
                let (nll, g) = score(self, eta)?;
                let trial = (eta, nll, g);
                if g > 0. {
                    right = trial;
                } else {
                    left = trial;
                }
                if g.abs() < best.2.abs() {
                    best = trial;
                }
            }
        }
        if best.2.abs() > 1e-2 {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Laplace conditional mode did not converge",
            ));
        }
        let (eta, nll, _) = best;
        let h = 1e-3_f64.min((limit - eta.abs()) / 4.);
        if h <= 0. {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Laplace eta mode has no curvature domain",
            ));
        }
        let curvature = (score(self, eta + h)?.1 - score(self, eta - h)?.1) / (2. * h);
        if !curvature.is_finite() || curvature <= 0. {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Laplace conditional mode lacks positive curvature",
            ));
        }
        let laplace = nll + 0.5 * eta * eta / omega + 0.5 * libm::log(omega * curvature);
        Ok((eta, nll, laplace))
    }

    fn population(&mut self, theta: f64, omega: f64) -> Result<Eval, Error> {
        let mut value = 0.;
        let mut subjects = Vec::with_capacity(self.request.subjects.len());
        for (i, subject) in self.request.subjects.iter().enumerate() {
            let warm_eta = self.warm_etas[i];
            let (eta, nll, term) = self
                .conditional(i, subject, theta, omega, warm_eta)
                .map_err(|mut e| {
                    e.expression = Some(format!("subjects[{i}]"));
                    e
                })?;
            self.warm_etas[i] = eta;
            value += term;
            subjects.push(PopulationSubjectEstimate {
                eta: vec![eta],
                negative_log_likelihood: nll,
            });
        }
        if !value.is_finite() {
            return Err(Error::new(ErrorCode::Domain, "Laplace objective overflow"));
        }
        Ok(Eval { value, subjects })
    }
}

impl CompiledSensitivityDocument {
    /// Estimate the population mean and, when its bounds differ, eta variance.
    /// The returned objective is a Laplace approximation to marginal NLL.
    /// `Converged` means the normalized coordinate-search step reached the
    /// request tolerance; it does not certify a projected-gradient threshold.
    pub(super) fn prepare_population_laplace(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
    ) -> Result<super::fit::PopulationOneDim, Error> {
        self.prepare_population_laplace_observed(request, None)
    }
    pub(super) fn prepare_population_laplace_observed(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        mut plan: Option<&mut super::fit_planning::FitPlanBuilder>,
    ) -> Result<super::fit::PopulationOneDim, Error> {
        if request.covariate_effect.is_some() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Laplace does not support the SAEM covariate-effect request",
            ));
        }
        if request.uncertainty {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Laplace uncertainty is not implemented",
            ));
        }
        if request.additive_error_fit.is_some() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Laplace additive-error estimation is not implemented",
            ));
        }
        let setup = self.validate_population_1d(request)?;
        for (index, subject) in request.subjects.iter().enumerate() {
            let prepared = self
                .prepare_gaussian_objective(subject)
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
                plan.gaussian(self, subject, &prepared)?;
            }
        }
        Ok(setup)
    }
    pub fn fit_population_laplace(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
    ) -> Result<PopulationFitResult, Error> {
        let setup = self.prepare_population_laplace(request)?;
        let mut evaluator = Evaluator {
            model: self,
            request,
            parameter: &setup.parameter,
            unit: &setup.unit,
            calls: 0,
            warm_etas: vec![0.; request.subjects.len()],
            likelihood_grids: Vec::with_capacity(3),
        };
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
        let mut best = evaluator.population(theta, omega)?;
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
                    match evaluator.population(theta, omega) {
                        Ok(candidate) if candidate.value < best.value => {
                            x = trial;
                            best = candidate;
                            improved = true;
                        }
                        Ok(_) => {}
                        Err(e) if e.code == ErrorCode::WorkBudget => break,
                        Err(e)
                            if matches!(
                                e.code,
                                ErrorCode::Domain
                                    | ErrorCode::Solver
                                    | ErrorCode::Invariant
                                    | ErrorCode::Unsupported
                            ) => {}
                        Err(e) => return Err(e),
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
            fitted_observations: None,
            uncertainty: None,
            estimated_residual_error: None,
            saem_diagnostics: None,
            objective_kind: PopulationObjectiveKind::LaplaceNegativeLogLikelihood,
            objective: best.value,
            evaluations: evaluator.calls,
            iterations,
            fit_request_hash: canonical_hash(
                &serde_json::json!({"algorithm":"laplace_1d_v1","request":request}),
            )?,
        })
    }
}

#[cfg(test)]
mod grid_tests {
    use super::*;
    use qspquest_core::model::{ModelDocument, Quantity};
    use qspquest_core::run::SensitivityRequest;

    fn quantity(value: f64, unit: &str) -> Quantity {
        Quantity {
            value,
            unit: unit.into(),
        }
    }

    fn fixture() -> (Arc<CompiledSensitivityDocument>, PopulationFitRequest) {
        let model: ModelDocument = serde_json::from_str(include_str!(
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
        let subjects = [0.6_f64, 0.8]
            .into_iter()
            .map(|truth| {
                let observations = [1.0_f64, 2., 4.]
                    .into_iter()
                    .enumerate()
                    .map(|(index, time)| {
                        let value = 2. * libm::exp(-truth * time / 3.) + 0.01;
                        GaussianObservation {
                            censoring: None,
                            row: index + 2,
                            output: "cp".into(),
                            value: quantity(value, "mg/L"),
                            error: GaussianError {
                                model: GaussianErrorModel::Normal,
                                additive_sd: quantity(0.05, "mg/L"),
                                proportional_sd: 0.,
                            },
                        }
                    })
                    .collect();
                GaussianObjectiveRequest {
                    simulation: simulation.clone(),
                    observations,
                    priors: vec![],
                }
            })
            .collect();
        let request = PopulationFitRequest {
            subjects,
            fixed_effects: vec![FitBound {
                parameter: "cl".into(),
                lower: quantity(0.35, "L/h"),
                upper: quantity(0.85, "L/h"),
            }],
            random_effects: vec![PopulationRandomEffect {
                parameter: "cl".into(),
            }],
            omega: vec![vec![0.04]],
            omega_diagonal_bounds: vec![[0.005, 0.25]],
            eta_bound: 1.5,
            max_evaluations: 10000,
            max_iterations: 80,
            gradient_tolerance: 0.005,
            seed: 17,
            burn_in_iterations: None,
            uncertainty: false,
            additive_error_fit: None,
            covariate_effect: None,
        };
        (compiled, request)
    }

    fn evaluator<'a>(
        model: &'a Arc<CompiledSensitivityDocument>,
        request: &'a PopulationFitRequest,
    ) -> Evaluator<'a> {
        Evaluator {
            model,
            request,
            parameter: "cl",
            unit: "L/h",
            calls: 0,
            warm_etas: vec![0.; request.subjects.len()],
            likelihood_grids: Vec::with_capacity(3),
        }
    }

    #[test]
    fn reused_likelihood_grids_match_analytic_pk_and_fresh_variance_probes() {
        let (compiled, request) = fixture();
        let mut cached = evaluator(&compiled, &request);
        for (index, subject) in request.subjects.iter().enumerate() {
            let grid = cached.likelihood_grid(index, subject, 0.5).unwrap();
            for (point, (nll, score)) in grid.into_iter().enumerate() {
                let eta = -request.eta_bound + request.eta_bound * point as f64 / 8.;
                let clearance = 0.5 * libm::exp(eta);
                let mut expected_nll = 0.;
                let mut expected_score = 0.;
                for (time, observation) in [1.0_f64, 2., 4.].into_iter().zip(&subject.observations)
                {
                    let predicted = 2. * libm::exp(-clearance * time / 3.);
                    let difference = predicted - observation.value.value;
                    expected_nll += 0.5
                        * (libm::log(2. * std::f64::consts::PI * 0.05 * 0.05)
                            + difference * difference / (0.05 * 0.05));
                    expected_score +=
                        difference / (0.05 * 0.05) * (-clearance * time / 3.) * predicted;
                }
                assert!(
                    (nll - expected_nll).abs() < 1e-5,
                    "point={point} nll={nll} expected={expected_nll}"
                );
                assert!(
                    (score - expected_score).abs() < 1e-5,
                    "point={point} score={score} expected={expected_score}"
                );
            }
        }
        assert_eq!(cached.calls, 34);
        for omega in [0.005, 0.04, 0.25] {
            for (index, subject) in request.subjects.iter().enumerate() {
                let before = cached.calls;
                let reused = cached.conditional(index, subject, 0.5, omega, 0.).unwrap();
                let mut fresh = evaluator(&compiled, &request);
                let reference = fresh.conditional(index, subject, 0.5, omega, 0.).unwrap();
                assert_eq!(reused, reference);
                assert_eq!(cached.calls - before + 17, fresh.calls);
            }
        }
    }

    #[test]
    fn likelihood_grid_reuse_is_exact_bounded_and_respects_work_budget() {
        let (compiled, mut request) = fixture();
        request.max_evaluations = 85;
        let subject = &request.subjects[0];
        let mut fit = evaluator(&compiled, &request);
        let original = fit.likelihood_grid(0, subject, 0.5).unwrap();
        let adjacent = f64::from_bits(0.5_f64.to_bits() + 1);
        fit.likelihood_grid(0, subject, adjacent).unwrap();
        assert_eq!(fit.calls, 34);
        for theta in [0.6, 0.7] {
            fit.likelihood_grid(0, subject, theta).unwrap();
        }
        assert_eq!(fit.likelihood_grids.len(), 3);
        // The evicted mean must be recomputed, with exactly the same values.
        assert_eq!(fit.likelihood_grid(0, subject, 0.5).unwrap(), original);
        assert_eq!(fit.calls, request.max_evaluations);
        // Fully cached computations need no extra model calls at the limit.
        assert_eq!(fit.likelihood_grid(0, subject, 0.5).unwrap(), original);
        assert_eq!(fit.calls, request.max_evaluations);
        assert_eq!(
            fit.likelihood_grid(0, subject, 0.8).unwrap_err().code,
            ErrorCode::WorkBudget
        );
        assert_eq!(fit.calls, request.max_evaluations);
    }
}

//! Gaussian negative log likelihood with variance derivatives retained.
use super::*;
use qspquest_core::{fit::*, identity::canonical_hash};
pub(super) struct LoweredGaussianObservation {
    pub(super) output: usize,
    pub(super) row: usize,
    pub(super) value: f64,
    pub(super) additive: f64,
    pub(super) proportional: f64,
    pub(super) censored: bool,
    pub(super) model: GaussianErrorModel,
}

pub(super) struct PreparedGaussianPrimal {
    pub(super) run: qspquest_core::run::RunRequest,
    pub(super) prepared: PreparedRun,
    pub(super) data: Vec<LoweredGaussianObservation>,
}

pub(super) struct PreparedGaussianObjective {
    pub(super) simulation: qspquest_core::run::SensitivityRequest,
    pub(super) run: super::run::PreparedSensitivityRun,
    pub(super) data: Vec<LoweredGaussianObservation>,
    gradient: Vec<f64>,
    prior_nll: f64,
    pub(super) priors: Vec<qspquest_core::planning::GaussianPriorPlan>,
}

fn normal(
    prediction: f64,
    value: f64,
    additive: f64,
    proportional: f64,
) -> Result<(f64, f64), Error> {
    if ![prediction, value, additive, proportional]
        .iter()
        .all(|x| x.is_finite())
        || additive < 0.
        || proportional < 0.
        || additive == 0. && proportional == 0.
    {
        return Err(invalid(
            "invalid Gaussian prediction, observation or standard deviation",
        ));
    }
    let component = proportional * prediction;
    let sd = libm::hypot(additive, component);
    if !sd.is_finite() || sd <= 0. {
        return Err(Error::new(
            ErrorCode::Domain,
            "Gaussian variance must be finite and positive",
        ));
    }
    let difference = value - prediction;
    let z = if difference.is_finite() {
        difference / sd
    } else {
        value / sd - prediction / sd
    };
    let nll = 0.5 * z * z + libm::log(sd) + 0.9189385332046727;
    let dlogsd = (component / sd) * (proportional / sd);
    let derivative = -z / sd + (1. - z * z) * dlogsd;
    if !nll.is_finite() || !derivative.is_finite() {
        return Err(Error::new(
            ErrorCode::Domain,
            "Gaussian likelihood or derivative overflow",
        ));
    }
    Ok((nll, derivative))
}
/// Left-censored Gaussian row: minus log P(Y <= limit), with its prediction score.
/// The far-left tail uses the normal Mills expansion to avoid CDF underflow.
pub(super) fn normal_left_censored(
    prediction: f64,
    limit: f64,
    additive: f64,
    proportional: f64,
) -> Result<(f64, f64), Error> {
    if ![prediction, limit, additive, proportional]
        .iter()
        .all(|value| value.is_finite())
        || additive < 0.
        || proportional < 0.
        || additive == 0. && proportional == 0.
    {
        return Err(invalid(
            "invalid censored Gaussian observation or standard deviation",
        ));
    }
    let component = proportional * prediction;
    let sd = libm::hypot(additive, component);
    if !sd.is_finite() || sd <= 0. {
        return Err(Error::new(
            ErrorCode::Domain,
            "censored Gaussian variance is invalid",
        ));
    }
    let z = (limit - prediction) / sd;
    if !z.is_finite() {
        return Err(Error::new(
            ErrorCode::Domain,
            "censored Gaussian score overflow",
        ));
    }
    let (nll, mills) = if z < -10. {
        let t = -z;
        let inv_sq = 1. / (t * t);
        let correction = 1. - inv_sq + 3. * inv_sq * inv_sq - 15. * inv_sq * inv_sq * inv_sq;
        (
            0.5 * t * t + libm::log(t) + 0.9189385332046727 - libm::log(correction),
            t / correction,
        )
    } else {
        let cdf = 0.5 * libm::erfc(-z / std::f64::consts::SQRT_2);
        let density = libm::exp(-0.5 * z * z - 0.9189385332046727);
        (-libm::log(cdf), density / cdf)
    };
    let dlogsd = (component / sd) * (proportional / sd);
    let derivative = mills * (1. / sd + z * dlogsd);
    if !nll.is_finite() || !derivative.is_finite() {
        return Err(Error::new(
            ErrorCode::Domain,
            "censored Gaussian likelihood overflow",
        ));
    }
    Ok((nll, derivative))
}

fn observation_likelihood(
    prediction: f64,
    value: f64,
    additive: f64,
    proportional: f64,
    censored: bool,
    model: GaussianErrorModel,
) -> Result<(f64, f64), Error> {
    match model {
        GaussianErrorModel::Normal => {
            if censored {
                normal_left_censored(prediction, value, additive, proportional)
            } else {
                normal(prediction, value, additive, proportional)
            }
        }
        GaussianErrorModel::Lognormal => {
            if !prediction.is_finite() || prediction <= 0. {
                return Err(Error::new(
                    ErrorCode::Domain,
                    "lognormal likelihood requires a finite positive model prediction",
                ));
            }
            if !value.is_finite()
                || value <= 0.
                || additive != 0.
                || !proportional.is_finite()
                || proportional <= 0.
            {
                return Err(invalid("lognormal observations require positive prediction and value, zero additive SD, and positive dimensionless log SD"));
            }
            let (nll, derivative) = if censored {
                normal_left_censored(libm::log(prediction), libm::log(value), proportional, 0.)?
            } else {
                let (nll, derivative) =
                    normal(libm::log(prediction), libm::log(value), proportional, 0.)?;
                (nll + libm::log(value), derivative)
            };
            Ok((nll, derivative / prediction))
        }
    }
}

impl CompiledSensitivityDocument {
    fn lower_observations(
        &self,
        observations: &[GaussianObservation],
    ) -> Result<Vec<LoweredGaussianObservation>, Error> {
        observations.iter().enumerate().map(|(i, observation)| {
            let lower = || -> Result<LoweredGaussianObservation, Error> {
                let output = self.output_names().iter().position(|name| name == &observation.output)
                    .ok_or_else(|| invalid("unknown observation output"))?;
                let unit = Unit::parse(&self.output_units()[output])?;
                let value = quantity(&observation.value)?.in_unit(unit)?;
                let additive = quantity(&observation.error.additive_sd)?.in_unit(unit)?;
                let proportional = observation.error.proportional_sd;
                if !proportional.is_finite() || proportional < 0. || additive < 0. || additive == 0. && proportional == 0. {
                    return Err(invalid("Gaussian standard deviations must be nonnegative and not both zero"));
                }
                if matches!(observation.error.model, GaussianErrorModel::Lognormal)
                    && (value <= 0. || additive != 0. || proportional <= 0.) {
                    return Err(invalid("lognormal observations require positive observed value, zero additive SD, and positive dimensionless log SD"));
                }
                Ok(LoweredGaussianObservation { output, row: observation.row, value, additive, proportional,
                    censored: observation.censoring.is_some(), model: observation.error.model })
            };
            lower().map_err(|mut error| { error.expression = Some(format!("observations[{i}]")); error })
        }).collect()
    }
    fn validate_observation_rows(
        data: &[LoweredGaussianObservation],
        rows: usize,
    ) -> Result<(), Error> {
        for (index, observation) in data.iter().enumerate() {
            if observation.row >= rows {
                let mut error = invalid("observation row is outside the simulation result");
                error.expression = Some(format!("observations[{index}]"));
                return Err(error);
            }
        }
        Ok(())
    }
    pub(super) fn prepare_gaussian_objective(
        self: &Arc<Self>,
        request: &GaussianObjectiveRequest,
    ) -> Result<PreparedGaussianObjective, Error> {
        if request.observations.is_empty()
            || request.observations.len() > 1_000_000
            || request.observations.len() > request.simulation.run.budgets.output_values
        {
            return Err(invalid(
                "Gaussian objective requires a bounded nonempty observation table",
            ));
        }
        if request.priors.len() > self.parameters().len() {
            return Err(invalid("too many parameter priors"));
        }
        let storage = request
            .observations
            .len()
            .checked_mul(5)
            .and_then(|n| n.checked_add(2 * self.parameters().len() + 3 * request.priors.len()))
            .ok_or_else(|| invalid("objective storage overflow"))?;
        let mut simulation = request.simulation.clone();
        simulation.run.budgets.output_values = simulation
            .run
            .budgets
            .output_values
            .checked_sub(storage)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::OutputBudget,
                    "objective observation storage exceeds budget",
                )
            })?;
        let data = self.lower_observations(&request.observations)?;
        let bound = self.bind_with_initial_states(
            &request.simulation.run.parameters,
            &request.simulation.run.regimen.covariates,
            &request.simulation.run.initial_states,
        )?;
        let mut gradient = vec![0.; self.parameters().len()];
        let mut prior_nll = 0.;
        let mut priors = Vec::with_capacity(request.priors.len());
        let mut names = BTreeSet::new();
        for prior in &request.priors {
            let p = self
                .parameters()
                .iter()
                .position(|n| n == &prior.parameter)
                .ok_or_else(|| invalid("prior must name a selected parameter"))?;
            if !names.insert(&prior.parameter) {
                return Err(invalid("duplicate parameter prior"));
            }
            let index = self
                .primal
                .parameters
                .iter()
                .position(|p| p.name == prior.parameter)
                .ok_or_else(|| invalid("unknown prior parameter"))?;
            let unit = Unit::parse(&self.primal.parameters[index].default.unit)?;
            let mean = quantity(&prior.mean)?.in_unit(unit)?;
            let sd = quantity(&prior.sd)?.in_unit(unit)?;
            if sd <= 0. {
                return Err(invalid("prior standard deviation must be positive"));
            }
            priors.push(qspquest_core::planning::GaussianPriorPlan {
                parameter: prior.parameter.clone(),
                unit: self.primal.parameters[index].default.unit.clone(),
                mean,
                sd,
            });
            let (nll, derivative) = normal(bound.primal.values[index], mean, sd, 0.)?;
            prior_nll += nll;
            gradient[p] += derivative;
        }
        let run = self.prepare_run(&simulation)?;
        Self::validate_observation_rows(&data, run.result_rows())?;
        Ok(PreparedGaussianObjective {
            simulation,
            run,
            data,
            gradient,
            prior_nll,
            priors,
        })
    }

    /// Likelihood-only path for estimators that do not need output derivatives.
    /// Keep the same observation, error and output-budget contract as the
    /// derivative-bearing Gaussian objective.
    #[cfg(test)]
    pub(super) fn gaussian_primal_nll(
        self: &Arc<Self>,
        request: &GaussianObjectiveRequest,
    ) -> Result<f64, Error> {
        self.gaussian_primal_nll_and_predictions(request)
            .map(|(nll, _)| nll)
    }

    pub(super) fn prepare_gaussian_primal(
        self: &Arc<Self>,
        request: &GaussianObjectiveRequest,
    ) -> Result<PreparedGaussianPrimal, Error> {
        if !request.priors.is_empty() {
            return Err(invalid("population subject priors are unsupported"));
        }
        if request.simulation.with_respect_to != self.parameters()
            || !matches!(
                request.simulation.run.solver,
                qspquest_core::run::Solver::DiffsolBdf | qspquest_core::run::Solver::DiffsolRodas5p
            )
            || request.simulation.run.seed.is_some()
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "population likelihood requires deterministic diffsol_bdf or diffsol_rodas5p and the selected parameter order",
            ));
        }
        let observations = &request.observations;
        if observations.is_empty()
            || observations.len() > 1_000_000
            || observations.len() > request.simulation.run.budgets.output_values
        {
            return Err(invalid(
                "Gaussian objective requires a bounded nonempty observation table",
            ));
        }
        let storage = observations
            .len()
            .checked_mul(5)
            .and_then(|n| n.checked_add(2 * self.parameters().len()))
            .ok_or_else(|| invalid("objective storage overflow"))?;
        let mut run = request.simulation.run.clone();
        run.budgets.output_values =
            run.budgets
                .output_values
                .checked_sub(storage)
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::OutputBudget,
                        "objective observation storage exceeds budget",
                    )
                })?;
        let data = self.lower_observations(observations)?;
        let prepared = self.primal.prepare_selected(&run, None)?;
        Self::validate_observation_rows(&data, prepared.regimen.result_rows)?;
        Ok(PreparedGaussianPrimal {
            run,
            prepared,
            data,
        })
    }

    /// Retain row-aligned predictions for the censored-data SAEM variance step.
    /// This uses the same solve as the likelihood, so no extra ODE evaluation
    /// is hidden from the estimator's work budget.
    pub(super) fn gaussian_primal_nll_and_predictions(
        self: &Arc<Self>,
        request: &GaussianObjectiveRequest,
    ) -> Result<(f64, Vec<f64>), Error> {
        let PreparedGaussianPrimal {
            run,
            prepared,
            data,
        } = self.prepare_gaussian_primal(request)?;
        let observations = &request.observations;
        let result = self.primal.execute_prepared(&run, None, prepared)?;
        let mut nll = 0.0;
        let mut predictions = Vec::with_capacity(observations.len());
        for (
            i,
            LoweredGaussianObservation {
                output,
                row,
                value,
                additive,
                proportional,
                censored,
                model,
            },
        ) in data.into_iter().enumerate()
        {
            let prediction = *result.outputs[output]
                .values
                .get(row)
                .ok_or_else(|| invalid("observation row is outside the simulation result"))?;
            let (term, _) =
                observation_likelihood(prediction, value, additive, proportional, censored, model)
                    .map_err(|mut error| {
                        error.expression = Some(format!("observations[{i}]"));
                        error
                    })?;
            nll += term;
            predictions.push(prediction);
        }
        if !nll.is_finite() {
            return Err(Error::new(
                ErrorCode::Domain,
                "Gaussian objective accumulation overflow",
            ));
        }
        Ok((nll, predictions))
    }

    pub(super) fn prospective_objective_identity(
        request: &GaussianObjectiveRequest,
        mut identity: qspquest_core::run::ExecutionIdentity,
    ) -> Result<qspquest_core::run::ExecutionIdentity, Error> {
        identity.run_request_hash = canonical_hash(
            &serde_json::json!({"simulation_request_hash":identity.run_request_hash,"requested_budgets":request.simulation.run.budgets,"observations":request.observations,"priors":request.priors}),
        )?;
        identity.algorithm = match request.simulation.run.solver {
            qspquest_core::run::Solver::DiffsolRodas5p => "rodas5p_forward_gaussian_objective",
            _ => "bdf_forward_gaussian_objective",
        }
        .into();
        Ok(identity)
    }

    /// Independent Gaussian observations with known error coefficients and
    /// optional independent normal priors on selected natural parameters.
    pub fn gaussian_objective(
        self: &Arc<Self>,
        request: &GaussianObjectiveRequest,
    ) -> Result<GaussianObjectiveResult, Error> {
        let PreparedGaussianObjective {
            simulation,
            run,
            data,
            mut gradient,
            prior_nll,
            priors: _,
        } = self.prepare_gaussian_objective(request)?;
        let result = self.execute_prepared_run(&simulation, run)?;
        let mut nll = 0.;
        let width = self.output_names().len();
        for (
            i,
            LoweredGaussianObservation {
                output,
                row,
                value,
                additive,
                proportional,
                censored,
                model,
            },
        ) in data.into_iter().enumerate()
        {
            let prediction = *result.result.outputs[output]
                .values
                .get(row)
                .ok_or_else(|| invalid("observation row is outside the simulation result"))?;
            let (term, derivative) =
                observation_likelihood(prediction, value, additive, proportional, censored, model)
                    .map_err(|mut e| {
                        e.expression = Some(format!("observations[{i}]"));
                        e
                    })?;
            nll += term;
            for (p, g) in gradient.iter_mut().enumerate() {
                *g += derivative * result.derivatives[p * width + output].values[row];
            }
        }
        let objective = nll + prior_nll;
        if !objective.is_finite() || gradient.iter().any(|g| !g.is_finite()) {
            return Err(Error::new(
                ErrorCode::Domain,
                "Gaussian objective accumulation overflow",
            ));
        }
        let identity = Self::prospective_objective_identity(request, result.result.identity)?;
        let gradient = self
            .parameters()
            .iter()
            .zip(gradient)
            .map(|(parameter, value)| {
                let unit = &self
                    .primal
                    .parameters
                    .iter()
                    .find(|p| &p.name == parameter)
                    .expect("selected parameter validated")
                    .default
                    .unit;
                ObjectiveGradient {
                    parameter: parameter.clone(),
                    unit: format!("1/({unit})"),
                    value,
                }
            })
            .collect();
        Ok(GaussianObjectiveResult {
            identity,
            negative_log_likelihood: nll,
            negative_log_prior: prior_nll,
            objective,
            gradient,
            observations: request.observations.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primal_likelihood_matches_derivative_objective() {
        let model: qspquest_core::model::ModelDocument = serde_json::from_str(include_str!(
            "../../../../../conformance/models/synthetic-one-compartment.json"
        ))
        .unwrap();
        let compiled = CompiledSensitivityDocument::compile(&model, &["cl".into()]).unwrap();
        let mut simulation: qspquest_core::run::SensitivityRequest = serde_json::from_str(
            include_str!("../../../../../conformance/requests/synthetic-sensitivity.json"),
        )
        .unwrap();
        simulation.with_respect_to = vec!["cl".into()];
        simulation.derivative_absolute_tolerances.remove("v");
        let observations = [(2, 1.01), (3, 0.93), (4, 0.79)]
            .into_iter()
            .map(|(row, value)| GaussianObservation {
                censoring: None,
                row,
                output: "cp".into(),
                value: qspquest_core::model::Quantity {
                    value,
                    unit: "mg/L".into(),
                },
                error: GaussianError {
                    model: GaussianErrorModel::Normal,
                    additive_sd: qspquest_core::model::Quantity {
                        value: 0.03,
                        unit: "mg/L".into(),
                    },
                    proportional_sd: 0.04,
                },
            })
            .collect();
        let request = GaussianObjectiveRequest {
            simulation,
            observations,
            priors: vec![],
        };
        let expected = compiled.gaussian_objective(&request).unwrap();
        let actual = compiled.gaussian_primal_nll(&request).unwrap();
        assert!(
            (actual - expected.negative_log_likelihood).abs() < 1e-6,
            "primal={actual}, sensitivity={}",
            expected.negative_log_likelihood
        );
    }
}

#[cfg(test)]
mod censored_tests {
    use super::normal_left_censored;

    #[test]
    fn left_censored_gaussian_has_correct_probability_and_score() {
        let (nll, score) = normal_left_censored(1., 1., 0.2, 0.).unwrap();
        assert!((nll - std::f64::consts::LN_2).abs() < 1e-12);
        assert!((score - (2. / std::f64::consts::PI).sqrt() / 0.2).abs() < 1e-12);
        for (prediction, limit, additive, proportional) in [
            (1.2, 0.8, 0.2, 0.),
            (1.2, 0.8, 0.1, 0.2),
            (4., 0.5, 0.1, 0.),
        ] {
            let (nll, score) =
                normal_left_censored(prediction, limit, additive, proportional).unwrap();
            let h = 1e-6;
            let plus = normal_left_censored(prediction + h, limit, additive, proportional)
                .unwrap()
                .0;
            let minus = normal_left_censored(prediction - h, limit, additive, proportional)
                .unwrap()
                .0;
            assert!(nll.is_finite());
            assert!((score - (plus - minus) / (2. * h)).abs() < 1e-5);
        }
    }
}

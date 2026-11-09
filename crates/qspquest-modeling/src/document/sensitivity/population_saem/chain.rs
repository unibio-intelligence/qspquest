//! The `saem` fit request's sampler: the shared SAEM engine over primal solves.
//!
//! Subjects keep their declared per-row residual models. With fixed residual
//! error each subject's likelihood is the fit request's own Gaussian objective,
//! including lognormal rows and M3 censoring. With a fitted common residual SD
//! the engine's residual M-step estimates it from the same rows.
use super::*;
use crate::analysis::common::{
    likelihood, AnalysisError, Budget, ErrorSpec, ErrorValues, Result as AnalysisResult, Sd,
};
use crate::analysis::{rng::substream, saem, NoControl};
use std::cell::RefCell;

/// Cumulative-window drift limit for native fits.
const STABILITY_TOLERANCE: f64 = 0.01;

pub(super) const MAX_SWEEPS: usize = 20;

pub(super) fn bind_log_parameters(
    objective: &mut GaussianObjectiveRequest,
    names: &[String],
    units: &[String],
    log_values: &[f64],
) -> Result<(), Error> {
    for ((name, unit), log_value) in names.iter().zip(units).zip(log_values) {
        let value = log_value.exp();
        if !value.is_finite() || value <= 0.0 {
            return Err(Error::new(
                ErrorCode::Domain,
                "invalid SAEM individual parameter",
            ));
        }
        objective.simulation.run.parameters.insert(
            name.clone(),
            qspquest_core::model::Quantity {
                value,
                unit: unit.clone(),
            },
        );
    }
    Ok(())
}

/// An exponential covariate effect on one random-effect axis.
pub(super) struct ChainCovariate {
    pub axis: usize,
    pub initial: f64,
    pub bounds: [f64; 2],
}

/// A fitted proportional SD of a normal combined residual model.
#[derive(Clone)]
pub(super) struct ProportionalFit {
    pub initial: f64,
    pub lower: f64,
    pub upper: f64,
    /// Declared additive SD when only the proportional SD is fitted.
    pub fixed_additive: f64,
}

pub(super) struct ChainInput<'a> {
    pub names: Vec<String>,
    pub units: Vec<String>,
    pub start: Vec<f64>,
    pub lower: Vec<f64>,
    pub upper: Vec<f64>,
    pub omega: Vec<Vec<f64>>,
    pub omega_bounds: Vec<[f64; 2]>,
    pub covariates: Vec<ChainCovariate>,
    /// Centred covariate values, one row per subject.
    pub x: Vec<Vec<f64>>,
    pub additive: Option<&'a ResidualFit>,
    pub proportional: Option<ProportionalFit>,
    /// Common declared output unit for fitted residuals, including proportional-only fits.
    pub residual_unit: Option<&'a str>,
    pub subjects: &'a [GaussianObjectiveRequest],
    pub burn_in: usize,
    /// Whether the chain is followed by an independently certified marginal fit.
    pub marginal_refinement: bool,
    /// Evaluations kept for the final likelihood and prediction runs.
    pub reserve: usize,
}

pub(super) struct Chain {
    pub status: FitStatus,
    pub log_theta: Vec<f64>,
    pub beta: Vec<f64>,
    pub omega: Vec<Vec<f64>>,
    pub additive_sd: Option<f64>,
    pub proportional_sd: Option<f64>,
    /// Final chain state as subject etas: log parameter minus location.
    pub sampled_etas: Vec<Vec<f64>>,
    pub iterations: usize,
    pub evaluations: usize,
    pub diagnostics: PopulationSaemDiagnostics,
}

struct FitCohort<'a> {
    model: &'a Arc<CompiledSensitivityDocument>,
    names: &'a [String],
    units: &'a [String],
    subjects: &'a [GaussianObjectiveRequest],
    y: Vec<Vec<f64>>,
    censored: Vec<Vec<bool>>,
    fitted_residual: bool,
    budget: &'a Budget,
    /// The first failure that must end the fit, kept with its original code.
    fatal: RefCell<Option<Error>>,
}

fn rejected(error: &Error) -> bool {
    matches!(
        error.code,
        ErrorCode::Domain | ErrorCode::Solver | ErrorCode::Invariant
    )
}

impl saem::Cohort for FitCohort<'_> {
    fn len(&self) -> usize {
        self.subjects.len()
    }
    fn evaluate(
        &self,
        index: usize,
        log_values: &[f64],
        error: &ErrorValues,
    ) -> AnalysisResult<saem::Draw> {
        self.budget.charge()?;
        let mut objective = self.subjects[index].clone();
        bind_log_parameters(&mut objective, self.names, self.units, log_values)
            .map_err(|error| AnalysisError::new("domain", error.message))?;
        match self.model.gaussian_primal_nll_and_predictions(&objective) {
            Ok((nll, prediction)) => {
                let nll = if self.fitted_residual {
                    likelihood(&self.y[index], &prediction, &self.censored[index], error)?.value
                } else {
                    nll
                };
                Ok(saem::Draw {
                    nll,
                    y: self.y[index].clone(),
                    prediction,
                    censored: self.censored[index].clone(),
                })
            }
            Err(error) if rejected(&error) => Err(AnalysisError::new("domain", error.message)),
            Err(error) => {
                let message = error.message.clone();
                self.fatal.borrow_mut().get_or_insert(error);
                Err(AnalysisError::new("fatal", message))
            }
        }
    }
}

fn to_core(error: AnalysisError) -> Error {
    let code = match error.code.as_str() {
        "invalid_input" => ErrorCode::InvalidInput,
        "budget" => ErrorCode::WorkBudget,
        _ => ErrorCode::Domain,
    };
    let mut converted = Error::new(code, error.message);
    converted.expression = error.expression;
    converted
}

impl CompiledSensitivityDocument {
    /// Run the shared SAEM engine for a validated population fit request.
    pub(super) fn saem_chain(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        input: ChainInput<'_>,
    ) -> Result<Chain, Error> {
        let count = input.subjects.len();
        let d = input.names.len();
        let working = request.max_evaluations.saturating_sub(input.reserve);
        if working < count {
            return Err(invalid(
                "SAEM evaluation budget is smaller than subject count",
            ));
        }
        let budget = Budget::new(working as u64, None).map_err(to_core)?;
        let unit = input.residual_unit.map(Unit::parse).transpose()?;
        let mut y = Vec::with_capacity(count);
        let mut censored = Vec::with_capacity(count);
        for subject in input.subjects {
            let mut values = Vec::with_capacity(subject.observations.len());
            for observation in &subject.observations {
                let value = quantity(&observation.value)?;
                values.push(match unit {
                    Some(unit) => value.in_unit(unit)?,
                    None => observation.value.value,
                });
            }
            y.push(values);
            censored.push(
                subject
                    .observations
                    .iter()
                    .map(|o| o.censoring.is_some())
                    .collect(),
            );
        }
        let error = match (input.additive, &input.proportional) {
            (None, None) => ErrorSpec {
                lognormal: false,
                additive: Sd::Fixed(1.),
                proportional: Sd::Fixed(0.),
            },
            (additive, proportional) => ErrorSpec {
                lognormal: false,
                additive: match additive {
                    Some(fit) => Sd::Fitted {
                        initial: libm::sqrt(fit.variance),
                        lower: libm::sqrt(fit.lower_variance),
                        upper: libm::sqrt(fit.upper_variance),
                    },
                    None => Sd::Fixed(proportional.as_ref().map_or(0., |fit| fit.fixed_additive)),
                },
                proportional: match proportional {
                    Some(fit) => Sd::Fitted {
                        initial: fit.initial,
                        lower: fit.lower,
                        upper: fit.upper,
                    },
                    None => Sd::Fixed(0.),
                },
            },
        };
        let problem = saem::Problem {
            log_initial: input.start.iter().map(|v| v.ln()).collect(),
            log_bounds: input
                .lower
                .iter()
                .zip(&input.upper)
                .map(|(lo, hi)| (lo.ln(), hi.ln()))
                .collect(),
            covariates: input
                .covariates
                .iter()
                .map(|c| saem::CovariateAxis {
                    axis: c.axis,
                    initial: c.initial,
                    lower: c.bounds[0],
                    upper: c.bounds[1],
                })
                .collect(),
            x: if input.covariates.is_empty() {
                vec![Vec::new(); count]
            } else {
                input.x.clone()
            },
            omega: input.omega.clone(),
            variance_bounds: input.omega_bounds.clone(),
            diagonal: request.covariance == Some(PopulationCovariance::Diagonal),
            // One sweep of every axis plus one joint move costs d + 1
            // evaluations; one effect takes three single-axis sweeps.
            sweeps: request.sweeps.unwrap_or(if d == 1 { 3 } else { 1 }),
            burn_in: input.burn_in,
            max_iterations: request.max_iterations,
            tolerance: STABILITY_TOLERANCE,
            // Marginal routes apply the caller's tolerance in the final refinement.
            update_tolerance: (!input.marginal_refinement).then_some(request.gradient_tolerance),
            error,
            eta_bound: Some(request.eta_bound),
            variance_scale: saem::VarianceScale::StandardDeviation,
            max_correlation: Some(MAX_SAEM_CORRELATION),
            mixing_lag_limit: Some(0.9),
        };
        let cohort = FitCohort {
            model: self,
            names: &input.names,
            units: &input.units,
            subjects: input.subjects,
            y,
            censored,
            fitted_residual: input.additive.is_some() || input.proportional.is_some(),
            budget: &budget,
            fatal: RefCell::new(None),
        };
        let outcome = saem::run(
            &problem,
            &cohort,
            substream(request.seed, "saem_fit_request", 0).stream,
            &budget,
            &mut NoControl,
            "population_fit",
            &mut || Ok(serde_json::Value::Null),
        );
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                return Err(cohort
                    .fatal
                    .borrow_mut()
                    .take()
                    .unwrap_or_else(|| to_core(error)))
            }
        };
        let location = saem::location(&problem, &outcome.coefficients);
        let mixing = &outcome.mixing;
        let post = mixing.post_burn_iterations > 0;
        let diagnostics = PopulationSaemDiagnostics {
            burn_in_iterations: input.burn_in,
            marginal_refinement_iterations: 0,
            final_step_size: mixing.final_gain,
            final_relative_change: mixing.final_relative_change,
            recent_relative_change_median: mixing.recent_relative_change_median(),
            recent_window: mixing.recent_window(),
            evaluated_proposals: mixing.evaluated_proposals as usize,
            accepted_proposals: mixing.accepted_proposals as usize,
            consecutive_stable_iterations: outcome.stable_checks,
            post_burn_acceptance_by_axis: post.then(|| {
                (0..d)
                    .map(|a| outcome.accepted[a] as f64 / outcome.proposed[a].max(1) as f64)
                    .collect()
            }),
            post_burn_min_subject_acceptance_by_axis: post.then(|| {
                (0..d)
                    .map(|a| {
                        (0..count)
                            .map(|i| {
                                outcome.subject_accepted[i][a] as f64
                                    / mixing.subject_proposed[i][a].max(1) as f64
                            })
                            .fold(1., f64::min)
                    })
                    .collect()
            }),
            post_burn_max_subject_stall_iterations: post.then_some(mixing.max_stall),
            post_burn_max_lag_one_correlation_by_axis: outcome
                .max_lag_one
                .iter()
                .copied()
                .collect::<Option<Vec<_>>>(),
        };
        let status = match outcome.status.as_str() {
            "converged" => FitStatus::Converged,
            "evaluation_limit" => FitStatus::EvaluationLimit,
            _ => FitStatus::IterationLimit,
        };
        Ok(Chain {
            status,
            log_theta: outcome.coefficients[..d].to_vec(),
            beta: outcome.coefficients[d..].to_vec(),
            omega: outcome.omega,
            additive_sd: input.additive.map(|_| outcome.error.additive),
            proportional_sd: input.proportional.map(|_| outcome.error.proportional),
            sampled_etas: outcome
                .phi
                .iter()
                .zip(&location)
                .map(|(p, l)| p.iter().zip(l).map(|(a, b)| a - b).collect())
                .collect(),
            iterations: outcome.trace.len(),
            evaluations: budget.used(),
            diagnostics,
        })
    }
}

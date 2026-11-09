//! `saem` fit requests outside the one- and two-effect marginal paths.
//!
//! One to eight lognormal effects, full or diagonal Omega, exponential
//! covariates on any effect, and fixed, additive, proportional or combined
//! normal residual error. The shared engine supplies the estimate; the result
//! reports the complete-data surrogate objective, because the marginal OFV
//! refinement covers only the one- and two-effect cases that [`route`]
//! sends to them.
use super::chain::{bind_log_parameters, ChainCovariate, ChainInput, ProportionalFit};
use super::*;
use crate::analysis::linalg;

/// Largest number of SAEM subjects, as for native analysis requests.
pub(super) const MAX_SAEM_SUBJECTS: usize = 128;
/// Largest number of SAEM random effects.
pub(super) const MAX_SAEM_EFFECTS: usize = 8;
const MAX_SAEM_COVARIATES: usize = 8;

/// Which estimate stage follows the shared SAEM chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    /// One effect: marginal OFV refinement and curvature uncertainty.
    One,
    /// Two effects without covariates: marginal refinement and uncertainty.
    Two,
    /// Everything else: the complete-data surrogate.
    General,
}

pub(super) fn covariate_effects(request: &PopulationFitRequest) -> Vec<PopulationCovariateEffect> {
    request
        .covariate_effect
        .as_ref()
        .map(|primary| {
            let mut first = primary.clone();
            first.additional = Vec::new();
            std::iter::once(first)
                .chain(primary.additional.iter().cloned())
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn route(request: &PopulationFitRequest) -> Route {
    let d = request.random_effects.len();
    let covariates = covariate_effects(request);
    let diagonal = request.covariance == Some(PopulationCovariance::Diagonal);
    if request.proportional_error_fit.is_some() {
        return Route::General;
    }
    if d == 1
        && (covariates.is_empty()
            || (covariates.len() <= 4 && request.additive_error_fit.is_none()))
    {
        return Route::One;
    }
    if d == 2 && covariates.is_empty() && !diagonal {
        return Route::Two;
    }
    Route::General
}

pub(super) struct PreparedGeneral {
    pub(super) names: Vec<String>,
    pub(super) units: Vec<String>,
    pub(super) start: Vec<f64>,
    pub(super) lower: Vec<f64>,
    pub(super) upper: Vec<f64>,
    pub(super) omega: Vec<Vec<f64>>,
    pub(super) omega_bounds: Vec<[f64; 2]>,
    pub(super) covariates: Vec<PopulationCovariateEffect>,
    pub(super) axes: Vec<usize>,
    pub(super) x: Vec<Vec<f64>>,
    pub(super) burn_in: usize,
    pub(super) additive: Option<ResidualFit>,
    pub(super) proportional: Option<(ProportionalFit, String)>,
    pub(super) subjects: Vec<GaussianObjectiveRequest>,
}

fn validate_effects(
    model: &Arc<CompiledSensitivityDocument>,
    request: &PopulationFitRequest,
) -> Result<PreparedGeneral, Error> {
    let d = request.random_effects.len();
    let n = request.subjects.len();
    if !(1..=MAX_SAEM_EFFECTS).contains(&d)
        || !(d + 2..=MAX_SAEM_SUBJECTS).contains(&n)
        || request.fixed_effects.len() != d
        || request.omega.len() != d
        || request.omega.iter().any(|row| row.len() != d)
        || request.omega_diagonal_bounds.len() != d
        || model.parameters().len() != d
    {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "SAEM requires 1..8 selected fixed and random effects across dimension+2..128 subjects",
        ));
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
        return Err(invalid("SAEM needs bounded positive controls"));
    }
    let mut prepared = PreparedGeneral {
        names: model.parameters().to_vec(),
        units: vec![String::new(); d],
        start: vec![0.; d],
        lower: vec![0.; d],
        upper: vec![0.; d],
        omega: request.omega.clone(),
        omega_bounds: request.omega_diagonal_bounds.clone(),
        covariates: Vec::new(),
        axes: Vec::new(),
        x: vec![Vec::new(); n],
        burn_in: burn_in_iterations(request)?,
        additive: None,
        proportional: None,
        subjects: request.subjects.clone(),
    };
    for axis in 0..d {
        let name = prepared.names[axis].clone();
        if request.fixed_effects[axis].parameter != name
            || request.random_effects[axis].parameter != name
        {
            return Err(invalid(
                "SAEM fixed and random effects must follow selected parameter order",
            ));
        }
        let parameter = model
            .primal
            .parameters
            .iter()
            .find(|p| p.name == name)
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
            .get(&name)
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
        prepared.units[axis] = parameter.default.unit.clone();
        prepared.start[axis] = start;
        prepared.lower[axis] = lower;
        prepared.upper[axis] = upper;
        let [variance_lower, variance_upper] = request.omega_diagonal_bounds[axis];
        if !variance_lower.is_finite()
            || !variance_upper.is_finite()
            || variance_lower <= 0.
            || variance_lower > variance_upper
            || request.omega[axis][axis] < variance_lower
            || request.omega[axis][axis] > variance_upper
        {
            return Err(invalid(
                "SAEM Omega variance bounds must be finite and positive",
            ));
        }
    }
    let symmetric = (0..d).all(|a| {
        (0..d)
            .all(|b| request.omega[a][b].is_finite() && request.omega[a][b] == request.omega[b][a])
    });
    if !symmetric || linalg::cholesky(&request.omega).is_none() {
        return Err(invalid("SAEM Omega must be symmetric positive definite"));
    }
    if request.covariance == Some(PopulationCovariance::Diagonal)
        && (0..d).any(|a| (0..d).any(|b| a != b && request.omega[a][b] != 0.))
    {
        return Err(invalid(
            "diagonal SAEM covariance requires a diagonal initial Omega",
        ));
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
        for axis in 0..d {
            let parameter_index = model
                .primal
                .parameters
                .iter()
                .position(|p| p.name == prepared.names[axis])
                .expect("validated parameter");
            if (bound.primal.values[parameter_index] - prepared.start[axis]).abs()
                > 1e-10 * prepared.start[axis].abs().max(1.)
            {
                return Err(invalid(format!(
                    "subject {index} has a conflicting fixed-effect start"
                )));
            }
        }
    }
    let covariates = covariate_effects(request);
    if covariates.len() > MAX_SAEM_COVARIATES || n <= covariates.len() + d {
        return Err(invalid(
            "SAEM needs more subjects than effects plus covariate coefficients (at most eight)",
        ));
    }
    let mut pairs = std::collections::BTreeSet::new();
    for effect in &covariates {
        if !effect.additional.is_empty() {
            return Err(invalid("nested SAEM covariate effects are invalid"));
        }
        let axis = prepared
            .names
            .iter()
            .position(|name| *name == effect.parameter)
            .ok_or_else(|| invalid("SAEM covariate effects must name a random effect"))?;
        if effect.covariate.is_empty()
            || !pairs.insert((effect.parameter.clone(), effect.covariate.clone()))
            || effect.subject_values.len() != n
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
        let largest = effect.bounds[0].abs().max(effect.bounds[1].abs());
        for (row, value) in effect.subject_values.iter().enumerate() {
            let centred = value - effect.reference;
            if centred.abs() * largest > 8. {
                return Err(invalid(
                    "SAEM covariate effect exceeds the bounded log-parameter range",
                ));
            }
            prepared.x[row].push(centred);
        }
        prepared.axes.push(axis);
    }
    for axis in prepared
        .axes
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
    {
        let columns: Vec<usize> = (0..covariates.len())
            .filter(|&j| prepared.axes[j] == axis)
            .collect();
        let design: Vec<Vec<f64>> = (0..n)
            .map(|i| {
                std::iter::once(1.)
                    .chain(columns.iter().map(|&j| prepared.x[i][j]))
                    .collect()
            })
            .collect();
        if linalg::rank(&design) != columns.len() + 1 {
            return Err(invalid(
                "SAEM covariate design is collinear or unidentifiable",
            ));
        }
    }
    prepared.covariates = covariates;
    Ok(prepared)
}

/// Fitted residual SDs of a combined or proportional normal residual model.
fn validate_proportional(
    model: &CompiledSensitivityDocument,
    request: &PopulationFitRequest,
    fit: &PopulationProportionalErrorFit,
) -> Result<(Option<ResidualFit>, ProportionalFit, String), Error> {
    let output_index = model
        .output_names()
        .iter()
        .position(|name| name == &fit.output)
        .ok_or_else(|| invalid("unknown SAEM proportional-error fit output"))?;
    let unit = model.output_units()[output_index].clone();
    let parsed = Unit::parse(&unit)?;
    if !(fit.lower.is_finite()
        && fit.upper.is_finite()
        && fit.initial.is_finite()
        && fit.lower > 0.
        && fit.lower < fit.upper
        && fit.lower <= fit.initial
        && fit.initial <= fit.upper)
    {
        return Err(invalid(
            "SAEM proportional SD requires positive finite bounds containing its initial value",
        ));
    }
    let additive = match &request.additive_error_fit {
        Some(additive) if additive.output != fit.output => {
            return Err(invalid(
                "SAEM additive and proportional SD fits must name the same output",
            ))
        }
        Some(additive) => {
            let initial = quantity(&additive.initial)?.in_unit(parsed)?;
            let lower = quantity(&additive.lower)?.in_unit(parsed)?;
            let upper = quantity(&additive.upper)?.in_unit(parsed)?;
            if !(lower.is_finite()
                && upper.is_finite()
                && lower > 0.
                && lower < upper
                && lower <= initial
                && initial <= upper
                && lower.powi(2) > 0.0
                && upper.powi(2).is_finite())
            {
                return Err(invalid(
                    "SAEM additive SD requires positive finite bounds containing its initial value",
                ));
            }
            Some((initial, lower, upper))
        }
        None => None,
    };
    let mut fixed_additive = None;
    let mut observations = 0usize;
    let mut uncensored = 0usize;
    for (subject_index, subject) in request.subjects.iter().enumerate() {
        for (row, observation) in subject.observations.iter().enumerate() {
            let declared = quantity(&observation.error.additive_sd)?.in_unit(parsed)?;
            let additive_ok = match additive {
                Some((initial, _, _)) => (declared - initial).abs() <= 1e-10 * initial,
                None => {
                    let first = *fixed_additive.get_or_insert(declared);
                    (declared - first).abs() <= 1e-10 * first.abs().max(f64::MIN_POSITIVE)
                }
            };
            if observation.output != fit.output
                || observation.error.model != GaussianErrorModel::Normal
                || (observation.error.proportional_sd - fit.initial).abs() > 1e-10 * fit.initial
                || !additive_ok
            {
                return Err(invalid(format!(
                    "subjects[{subject_index}].observations[{row}] must use the fitted output, the normal model and the initial residual SDs"
                )));
            }
            observations += 1;
            uncensored += usize::from(observation.censoring.is_none());
        }
    }
    if observations == 0 || uncensored == 0 {
        return Err(invalid(
            "SAEM residual SDs cannot be identified from all-censored observations",
        ));
    }
    let residual = additive.map(|(initial, lower, upper)| ResidualFit {
        output: fit.output.clone(),
        unit: unit.clone(),
        variance: initial * initial,
        lower_variance: lower * lower,
        upper_variance: upper * upper,
    });
    Ok((
        residual,
        ProportionalFit {
            initial: fit.initial,
            lower: fit.lower,
            upper: fit.upper,
            fixed_additive: fixed_additive.unwrap_or(0.),
        },
        unit,
    ))
}

impl CompiledSensitivityDocument {
    pub(super) fn prepare_population_saem_general(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        mut plan: Option<&mut super::super::fit_planning::FitPlanBuilder>,
    ) -> Result<PreparedGeneral, Error> {
        if request.uncertainty {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "SAEM observed covariance requires one or two effects, fixed residual error and no covariates",
            ));
        }
        let mut prepared = validate_effects(self, request)?;
        if let Some(fit) = &request.proportional_error_fit {
            let (additive, proportional, unit) = validate_proportional(self, request, fit)?;
            prepared.additive = additive;
            prepared.proportional = Some((proportional, unit));
            let rows = request
                .subjects
                .iter()
                .map(|subject| subject.observations.len())
                .fold(0usize, usize::saturating_add);
            if rows.saturating_mul(request.max_iterations - prepared.burn_in)
                > crate::analysis::saem::MAX_EXPECTATION_ITEMS
            {
                return Err(Error::new(
                    ErrorCode::WorkBudget,
                    "SAEM stochastic expectation storage exceeds its limit; reduce observations or iterations",
                ));
            }
        } else {
            prepared.additive = validate_residual_fit(self, request)?;
        }
        if request.max_evaluations < prepared.subjects.len().saturating_mul(4) {
            return Err(invalid(
                "SAEM requires at least eight iterations and four evaluations per subject",
            ));
        }
        let log_start: Vec<f64> = prepared.start.iter().map(|value| value.ln()).collect();
        for (index, subject) in prepared.subjects.iter().enumerate() {
            let mut initial = subject.clone();
            let mut log_values = log_start.clone();
            for (column, (effect, &axis)) in
                prepared.covariates.iter().zip(&prepared.axes).enumerate()
            {
                log_values[axis] += prepared.x[index][column] * effect.initial;
            }
            let primal = (|| {
                bind_log_parameters(&mut initial, &prepared.names, &prepared.units, &log_values)?;
                self.prepare_gaussian_primal(&initial)
            })()
            .map_err(|mut error: Error| {
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
                plan.primal(self, &initial, &primal)?;
            }
        }
        Ok(prepared)
    }

    pub(super) fn fit_population_saem_general(
        self: &Arc<Self>,
        request: &PopulationFitRequest,
        prepared: PreparedGeneral,
    ) -> Result<PopulationFitResult, Error> {
        let PreparedGeneral {
            names,
            units,
            start,
            lower,
            upper,
            omega,
            omega_bounds,
            covariates,
            axes,
            x,
            burn_in,
            additive,
            proportional,
            mut subjects,
        } = prepared;
        let count = subjects.len();
        let d = names.len();
        let residual_unit = additive
            .as_ref()
            .map(|fit| fit.unit.clone())
            .or_else(|| proportional.as_ref().map(|(_, unit)| unit.clone()));
        let chain = self.saem_chain(
            request,
            ChainInput {
                names: names.clone(),
                units: units.clone(),
                start,
                lower,
                upper,
                omega,
                omega_bounds,
                covariates: covariates
                    .iter()
                    .zip(&axes)
                    .map(|(effect, &axis)| ChainCovariate {
                        axis,
                        initial: effect.initial,
                        bounds: effect.bounds,
                    })
                    .collect(),
                x: x.clone(),
                additive: additive.as_ref(),
                proportional: proportional.as_ref().map(|(fit, _)| fit.clone()),
                residual_unit: residual_unit.as_deref(),
                subjects: &subjects,
                burn_in,
                marginal_refinement: false,
                reserve: 3 * count,
            },
        )?;
        for subject in &mut subjects {
            for observation in &mut subject.observations {
                if let (Some(sd), Some(unit)) = (chain.additive_sd, &residual_unit) {
                    observation.error.additive_sd = qspquest_core::model::Quantity {
                        value: sd,
                        unit: unit.clone(),
                    };
                }
                if let Some(sd) = chain.proportional_sd {
                    observation.error.proportional_sd = sd;
                }
            }
        }
        let mut evaluations = chain.evaluations;
        let theta: Vec<f64> = chain.log_theta.iter().map(|v| v.exp()).collect();
        let omega = chain.omega.clone();
        let precision = linalg::inverse_spd(&omega)
            .ok_or_else(|| invalid("SAEM Omega must remain positive definite"))?;
        let log_det = linalg::log_det_spd(&omega)
            .ok_or_else(|| invalid("SAEM Omega must remain positive definite"))?;
        let mut subject_results = Vec::with_capacity(count);
        let mut fitted_observations = Vec::new();
        let mut objective = 0.;
        for (index, subject) in subjects.iter().enumerate() {
            let eta = &chain.sampled_etas[index];
            let typical: Vec<f64> = (0..d)
                .map(|axis| {
                    let shift: f64 = axes
                        .iter()
                        .enumerate()
                        .filter(|(_, &a)| a == axis)
                        .map(|(j, _)| chain.beta[j] * x[index][j])
                        .sum();
                    theta[axis] * libm::exp(shift)
                })
                .collect();
            let mut individual = subject.clone();
            for axis in 0..d {
                individual.simulation.run.parameters.insert(
                    names[axis].clone(),
                    qspquest_core::model::Quantity {
                        value: typical[axis] * libm::exp(eta[axis]),
                        unit: units[axis].clone(),
                    },
                );
            }
            let (likelihood, _) = self.gaussian_primal_nll_and_predictions(&individual)?;
            evaluations += 1;
            objective += likelihood
                + 0.5
                    * (linalg::quadratic_form(eta, &precision, eta) + log_det + d as f64 * LOG_2PI);
            fitted_observations.extend(
                self.saem_predictions_slices(index, subject, &names, &units, &typical, eta)?,
            );
            evaluations += 2;
            subject_results.push(PopulationSubjectEstimate {
                eta: eta.clone(),
                negative_log_likelihood: likelihood,
            });
        }
        if !objective.is_finite() {
            return Err(Error::new(
                ErrorCode::Domain,
                "SAEM complete-data surrogate overflow",
            ));
        }
        let fixed_effects = names
            .iter()
            .zip(&units)
            .zip(&theta)
            .map(|((name, unit), &value)| {
                (
                    name.clone(),
                    qspquest_core::model::Quantity {
                        value,
                        unit: unit.clone(),
                    },
                )
            })
            .collect();
        let estimate = |index: usize| {
            let effect = &covariates[index];
            PopulationCovariateEffectEstimate {
                parameter: effect.parameter.clone(),
                covariate: effect.covariate.clone(),
                reference: effect.reference,
                coefficient: chain.beta[index],
                additional: Vec::new(),
            }
        };
        let estimated_residual_error = residual_unit.map(|unit| {
            let output = additive
                .as_ref()
                .map(|fit| fit.output.clone())
                .or_else(|| {
                    request
                        .proportional_error_fit
                        .as_ref()
                        .map(|fit| fit.output.clone())
                })
                .unwrap_or_default();
            PopulationResidualErrorEstimate {
                output,
                additive_sd: qspquest_core::model::Quantity {
                    value: chain.additive_sd.unwrap_or_else(|| {
                        proportional
                            .as_ref()
                            .map(|(fit, _)| fit.fixed_additive)
                            .unwrap_or(0.)
                    }),
                    unit,
                },
                proportional_sd: chain.proportional_sd,
            }
        });
        Ok(PopulationFitResult {
            fixed_effect_intervals: None,
            fixed_effect_interval_error: None,
            status: chain.status,
            fixed_effects,
            omega,
            covariate_effect: (!covariates.is_empty()).then(|| {
                let mut first = estimate(0);
                first.additional = (1..covariates.len()).map(estimate).collect();
                first
            }),
            subjects: subject_results,
            fitted_observations: Some(fitted_observations),
            uncertainty: None,
            estimated_residual_error,
            saem_diagnostics: Some(chain.diagnostics),
            objective_kind: PopulationObjectiveKind::SaemCompleteDataSurrogate,
            objective,
            evaluations,
            iterations: chain.iterations,
            fit_request_hash: canonical_hash(
                &serde_json::json!({"algorithm":"saem_shared_engine_surrogate_v2","request":request}),
            )?,
        })
    }
}

use super::*;
use qspquest_core::{fit::*, identity::canonical_hash, planning::*};

const DEFAULT_METADATA_ITEM_LIMIT: usize = 1_000_000;

pub(super) struct FitPlanBuilder {
    pub(super) subjects: Vec<FitSubjectPlan>,
    limit: usize,
    used: usize,
}
impl FitPlanBuilder {
    fn charge(&mut self, items: impl IntoIterator<Item = usize>) -> Result<(), Error> {
        self.used = items.into_iter().try_fold(self.used, |sum, n| sum.checked_add(n))
            .filter(|n| *n <= self.limit)
            .ok_or_else(|| {
                let mut error = Error::new(ErrorCode::OutputBudget,
                    "aggregate fitting-plan metadata exceeds metadata_item_limit; reduce subject/observation grids or request a larger explicit planning limit");
                error.expression = Some("plan.metadata_item_limit".into());
                error
            })?;
        Ok(())
    }
    // Conservative record count before descriptive rows, units and binding copies are allocated.
    fn forward(
        &mut self,
        model: &CompiledSensitivityDocument,
        prepared: &super::run::PreparedSensitivityRun,
        observations: usize,
        priors: usize,
    ) -> Result<(), Error> {
        let r = &prepared.regimen;
        self.charge([
            model.forward.augmented_state_units.len(),
            model.output_names().len() * (1 + model.parameters().len()),
            2,
            r.protocol.samples.len().saturating_mul(4),
            r.protocol.events.len().saturating_mul(3),
            r.model.discontinuities().len().saturating_mul(3),
            r.result_rows().saturating_mul(4),
            (r.model.bindings.len() + 1).saturating_mul(
                model.primal.parameters.len()
                    + model.primal.covariates.len()
                    + model.primal.individual.len(),
            ),
            observations.saturating_mul(3),
            priors.saturating_mul(3),
        ])
    }
    pub(super) fn gaussian(
        &mut self,
        model: &CompiledSensitivityDocument,
        request: &GaussianObjectiveRequest,
        prepared: &super::objective::PreparedGaussianObjective,
        profile_proportional: bool,
    ) -> Result<(), Error> {
        self.forward(
            model,
            &prepared.run,
            prepared.data.len(),
            prepared.priors.len(),
        )?;
        let mut description = model.describe_gaussian_objective(request, prepared)?;
        if profile_proportional {
            description.prospective_identity = super::objective::profiled_identity(
                request.simulation.run.solver,
                description.prospective_identity,
            )?;
        }
        self.subjects
            .push(FitSubjectPlan::GaussianObjective(Box::new(description)));
        Ok(())
    }
    pub(super) fn focei(
        &mut self,
        model: &CompiledSensitivityDocument,
        request: &qspquest_core::run::SensitivityRequest,
        prepared: &super::run::PreparedSensitivityRun,
        observations: impl FnOnce() -> Vec<GaussianObservationPlan>,
        count: usize,
    ) -> Result<(), Error> {
        self.forward(model, prepared, count, 0)?;
        self.subjects.push(FitSubjectPlan::FoceiLikelihood(Box::new(
            FoceiLikelihoodPlan {
                simulation: model.describe_prepared_run(request, prepared)?,
                observations: observations(),
            },
        )));
        Ok(())
    }
    pub(super) fn primal(
        &mut self,
        model: &CompiledSensitivityDocument,
        request: &GaussianObjectiveRequest,
        prepared: &super::objective::PreparedGaussianPrimal,
    ) -> Result<(), Error> {
        let r = &prepared.prepared.regimen;
        self.charge([
            model.primal.state_count,
            model.output_names().len(),
            2,
            r.protocol.samples.len().saturating_mul(4),
            r.protocol.events.len().saturating_mul(3),
            r.model.discontinuities().len().saturating_mul(3),
            r.result_rows.saturating_mul(4),
            prepared.data.len().saturating_mul(3),
            (r.model.bindings.len() + 1).saturating_mul(
                model.primal.parameters.len()
                    + model.primal.covariates.len()
                    + model.primal.individual.len(),
            ),
        ])?;
        let simulation =
            model
                .primal
                .describe_prepared_run(&prepared.run, None, &prepared.prepared)?;
        let observations = prepared
            .data
            .iter()
            .map(|row| GaussianObservationPlan {
                row: row.row,
                output: model.output_names()[row.output].clone(),
                unit: model.output_units()[row.output].clone(),
                value: row.value,
                additive_sd: row.additive,
                proportional_sd: row.proportional,
                model: row.model,
                censoring: row.censored.then_some(GaussianCensoring::Left),
            })
            .collect();
        self.subjects
            .push(FitSubjectPlan::PrimalLikelihood(Box::new(
                PrimalLikelihoodPlan {
                    request: request.clone(),
                    simulation,
                    observations,
                    reserved_objective_values: request.simulation.run.budgets.output_values
                        - prepared.run.budgets.output_values,
                },
            )));
        Ok(())
    }
}

pub(super) fn coordinate(
    name: &str,
    unit: &str,
    start: f64,
    lower: f64,
    upper: f64,
    search_coordinate: FitSearchCoordinate,
    initial_search_value: f64,
) -> Result<FitCoordinatePlan, Error> {
    Ok(FitCoordinatePlan {
        parameter: name.into(),
        unit: super::super::planning::exact_unit(Unit::parse(unit)?, Some(unit)),
        initial: start,
        lower,
        upper,
        search_coordinate,
        initial_search_value,
    })
}
pub(super) fn one_coordinate(
    s: &super::fit::PopulationOneDim,
) -> Result<Vec<FitCoordinatePlan>, Error> {
    Ok(vec![coordinate(
        &s.parameter,
        &s.unit,
        s.start,
        s.lower,
        s.upper,
        FitSearchCoordinate::AffineUnitBox,
        (s.start - s.lower) / (s.upper - s.lower),
    )?])
}
pub(super) fn population(request: &PopulationFitRequest) -> PopulationPreparationPlan {
    PopulationPreparationPlan {
        omega: request.omega.clone(),
        omega_diagonal_bounds: request.omega_diagonal_bounds.clone(),
        eta_bound: request.eta_bound,
        burn_in_iterations: None,
        centered_covariates: Vec::new(),
        initial_covariate_coefficients: Vec::new(),
        residual: None,
    }
}

impl CompiledSensitivityDocument {
    /// Describe fitting admission without objective integration, optimization or RNG draws.
    pub fn plan_fit(self: &Arc<Self>, request: &FitRequest) -> Result<FitPlan, Error> {
        self.plan_fit_with_limit(request, DEFAULT_METADATA_ITEM_LIMIT)
    }
    /// The aggregate metadata limit is separate from each subject's numerical budgets.
    pub fn plan_fit_with_limit(
        self: &Arc<Self>,
        request: &FitRequest,
        metadata_item_limit: usize,
    ) -> Result<FitPlan, Error> {
        let mut builder = FitPlanBuilder {
            subjects: Vec::new(),
            limit: metadata_item_limit,
            used: 0,
        };
        let (coordinates, population, max_evaluations, max_iterations, gradient_tolerance) =
            match &request.problem {
                FitProblem::Individual(fit)
                | FitProblem::ProfiledProportional(fit)
                | FitProblem::Pooled(PooledGaussianFitRequest { fit, .. }) => {
                    let additional = match &request.problem {
                        FitProblem::Pooled(p) => p.additional_subjects.as_slice(),
                        _ => &[],
                    };
                    let p = self.prepare_gaussian_fit_observed(
                        fit,
                        additional,
                        matches!(&request.problem, FitProblem::ProfiledProportional(_)),
                        Some(&mut builder),
                    )?;
                    let coordinates = self
                        .parameters()
                        .iter()
                        .enumerate()
                        .map(|(i, name)| {
                            coordinate(
                                name,
                                &p.units[i],
                                p.starts[i],
                                p.lower[i],
                                p.upper[i],
                                FitSearchCoordinate::AffineUnitBox,
                                p.x[i],
                            )
                        })
                        .collect::<Result<Vec<_>, Error>>()?;
                    (
                        coordinates,
                        None,
                        fit.max_evaluations,
                        fit.max_iterations,
                        fit.gradient_tolerance,
                    )
                }
                FitProblem::Laplace(r) => {
                    let p = self.prepare_population_laplace_observed(r, Some(&mut builder))?;
                    (
                        one_coordinate(&p)?,
                        Some(population(r)),
                        r.max_evaluations,
                        r.max_iterations,
                        r.gradient_tolerance,
                    )
                }
                FitProblem::Focei(r) => {
                    let (coordinates, p) = self.describe_population_focei(r, &mut builder)?;
                    (
                        coordinates,
                        Some(p),
                        r.max_evaluations,
                        r.max_iterations,
                        r.gradient_tolerance,
                    )
                }
                FitProblem::Saem(r) => {
                    let (coordinates, p) = self.describe_population_saem(r, &mut builder)?;
                    (
                        coordinates,
                        Some(p),
                        r.max_evaluations,
                        r.max_iterations,
                        r.gradient_tolerance,
                    )
                }
            };
        builder.charge([coordinates.len(), 1])?;
        if let Some(p) = &population {
            builder.charge(
                p.omega
                    .iter()
                    .map(Vec::len)
                    .chain(std::iter::once(p.omega_diagonal_bounds.len()))
                    .chain(p.centered_covariates.iter().map(Vec::len))
                    .chain(std::iter::once(p.initial_covariate_coefficients.len()))
                    .chain(std::iter::once(usize::from(p.residual.is_some()))),
            )?;
        }
        let identities: Vec<_> = builder
            .subjects
            .iter()
            .map(|subject| match subject {
                FitSubjectPlan::GaussianObjective(p) => &p.prospective_identity,
                FitSubjectPlan::PrimalLikelihood(p) => &p.simulation.prospective_identity,
                FitSubjectPlan::FoceiLikelihood(p) => &p.simulation.prospective_identity,
            })
            .collect();
        let fit_input_hash = canonical_hash(
            &serde_json::json!({"request":request,"admission_identities":identities}),
        )?;
        let mut plan = FitPlan {
            schema: FitPlanSchema::V01,
            request: request.clone(),
            fit_input_hash,
            plan_hash: String::new(),
            coordinates,
            population,
            subjects: builder.subjects,
            max_evaluations,
            max_iterations,
            gradient_tolerance,
            metadata_item_limit,
            metadata_items: builder.used,
            deferred_checks: vec![
                DeferredCheck::TrajectoryDomain,
                DeferredCheck::ReadoutDomain,
                DeferredCheck::NumericalConvergence,
                DeferredCheck::Optimization,
                DeferredCheck::Identifiability,
                DeferredCheck::ObservedCovariance,
                DeferredCheck::IndependentScientificQualification,
            ],
        };
        plan.plan_hash =
            canonical_hash(&serde_json::to_value(&plan).map_err(|e| invalid(e.to_string()))?)?;
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::super::preparation_tests::{fixture, population_fixture};
    use super::*;
    use serde_json::json;
    fn individual() -> FitRequest {
        serde_json::from_str(include_str!(
            "../../../../../conformance/requests/synthetic-fit.json"
        ))
        .unwrap()
    }
    fn wrap(kind: &str, request: PopulationFitRequest) -> FitRequest {
        serde_json::from_value(
            json!({"schema":"qspquest.fit/v0.1","problem":{"kind":kind,"request":request}}),
        )
        .unwrap()
    }
    fn assert_hash(plan: &FitPlan) {
        let mut p = plan.clone();
        p.plan_hash.clear();
        assert_eq!(
            plan.plan_hash,
            canonical_hash(&serde_json::to_value(p).unwrap()).unwrap()
        );
        assert!(plan
            .deferred_checks
            .iter()
            .any(|c| matches!(c, DeferredCheck::Optimization)));
    }
    #[test]
    fn individual_pooled_plans_preserve_units_boxes_starts_and_full_digest() {
        let (model, _) = fixture(true);
        let request = individual();
        let plan = model.plan_fit(&request).unwrap();
        assert_hash(&plan);
        assert_eq!(plan.plan_hash, model.plan_fit(&request).unwrap().plan_hash);
        assert_eq!(
            plan.coordinates
                .iter()
                .map(|p| p.parameter.as_str())
                .collect::<Vec<_>>(),
            ["cl", "v"]
        );
        assert_eq!(plan.coordinates[0].initial, 0.3);
        assert_eq!(plan.coordinates[0].lower, 0.05);
        assert_eq!(plan.coordinates[0].upper, 1.2);
        assert_eq!(
            plan.coordinates[0].initial_search_value,
            (0.3 - 0.05) / (1.2 - 0.05)
        );
        assert_eq!(plan.coordinates[0].unit.declared.as_deref(), Some("L/h"));
        assert_eq!(
            model.execute_fit(&request).unwrap_err().code,
            ErrorCode::Domain
        );
        let FitProblem::Individual(fit) = request.problem else {
            unreachable!()
        };
        let mut additional = fit.objective.clone();
        additional.simulation.run.request_id = Some("second".into());
        additional.simulation.run.regimen.administrations[0]
            .amount
            .value *= 2.;
        let pooled: FitRequest = serde_json::from_value(json!({"schema":"qspquest.fit/v0.1","problem":{"kind":"pooled","request":{"fit":fit,"additional_subjects":[additional]}}})).unwrap();
        let p = model.plan_fit(&pooled).unwrap();
        assert_eq!(p.subjects.len(), 2);
        assert_hash(&p);
        assert_ne!(p.fit_input_hash, plan.fit_input_hash);
        assert_ne!(p.plan_hash, plan.plan_hash);
        let FitSubjectPlan::GaussianObjective(first) = &p.subjects[0] else {
            panic!()
        };
        let FitSubjectPlan::GaussianObjective(second) = &p.subjects[1] else {
            panic!()
        };
        assert_ne!(
            first.prospective_identity.run_request_hash,
            second.prospective_identity.run_request_hash
        );
        let mut invalid = pooled.clone();
        let FitProblem::Pooled(p) = &mut invalid.problem else {
            panic!()
        };
        p.additional_subjects[0].observations[0].row = 999;
        let error = model.plan_fit(&invalid).unwrap_err();
        assert_eq!(
            error.expression.as_deref(),
            Some("subjects[1].observations[0]")
        );
        assert_eq!(error, model.execute_fit(&invalid).unwrap_err());
        let mut value = serde_json::to_value(plan).unwrap();
        value["executed"] = json!(true);
        assert!(serde_json::from_value::<FitPlan>(value).is_err());
    }
    #[test]
    fn all_population_estimators_describe_admission_without_readouts_or_rng() {
        for (kind, two) in [
            ("laplace", false),
            ("focei", false),
            ("focei", true),
            ("saem", false),
            ("saem", true),
        ] {
            let (model, request) = population_fixture(two);
            let request = wrap(kind, request);
            let p = model.plan_fit(&request).unwrap();
            assert_hash(&p);
            assert_eq!(p.subjects.len(), 2);
            assert_eq!(p.coordinates.len(), if two { 2 } else { 1 });
            assert_eq!(p.coordinates[0].initial, 0.3);
            assert_eq!(p.coordinates[0].lower, 0.1);
            assert_eq!(p.plan_hash, model.plan_fit(&request).unwrap().plan_hash);
            let population = p.population.unwrap();
            if kind == "saem" {
                assert_eq!(population.burn_in_iterations, Some(4));
                assert!(matches!(p.subjects[0], FitSubjectPlan::PrimalLikelihood(_)));
                assert!(matches!(
                    p.coordinates[0].search_coordinate,
                    FitSearchCoordinate::LogNaturalValue
                ));
            } else if kind == "focei" {
                assert!(matches!(p.subjects[0], FitSubjectPlan::FoceiLikelihood(_)));
            } else {
                assert!(matches!(
                    p.subjects[0],
                    FitSubjectPlan::GaussianObjective(_)
                ));
            }
            assert_eq!(
                model.execute_fit(&request).unwrap_err().code,
                ErrorCode::Domain,
                "{kind}/{two}"
            );
            for case in 0..4 {
                let mut invalid = request.clone();
                let r = match &mut invalid.problem {
                    FitProblem::Laplace(r) | FitProblem::Focei(r) | FitProblem::Saem(r) => r,
                    _ => panic!(),
                };
                match case {
                    0 => r.subjects[1].observations[0].row = 999,
                    1 => r.omega[0][0] = 0.,
                    2 => r.subjects[1].simulation.run.budgets.output_values = 1,
                    _ => r.fixed_effects[0].lower.unit = "mg".into(),
                }
                assert_eq!(
                    model.plan_fit(&invalid).unwrap_err(),
                    model.execute_fit(&invalid).unwrap_err(),
                    "{kind}/{two}/{case}"
                );
            }
        }
    }
    #[test]
    fn saem_preserves_primal_admission_and_centered_initial_covariates() {
        let (model, mut r) = population_fixture(false);
        let template = r.subjects[0].clone();
        r.subjects = vec![template; 4];
        for s in &mut r.subjects {
            s.simulation.derivative_absolute_tolerances.clear();
        }
        r.covariate_effect = Some(serde_json::from_value(json!({"parameter":"cl","covariate":"weight","subject_values":[50,60,70,80],"reference":65,"initial":0.01,"bounds":[-0.02,0.02]})).unwrap());
        let p = model.plan_fit(&wrap("saem", r.clone())).unwrap();
        let population = p.population.unwrap();
        assert_eq!(
            population.centered_covariates,
            vec![vec![-15.], vec![-5.], vec![5.], vec![15.]]
        );
        assert_eq!(population.initial_covariate_coefficients, [0.01]);
        for (i, subject) in p.subjects.iter().enumerate() {
            let FitSubjectPlan::PrimalLikelihood(s) = subject else {
                panic!()
            };
            let expected = 0.3
                * libm::exp(0.01 * (r.covariate_effect.as_ref().unwrap().subject_values[i] - 65.));
            assert_eq!(s.request.simulation.run.parameters["cl"].value, expected);
            assert_eq!(
                s.simulation.binding_phases[0]
                    .inputs
                    .iter()
                    .find(|p| p.name == "cl")
                    .unwrap()
                    .value,
                expected
            );
            assert_eq!(s.simulation.states.len(), 1);
        }
        for kind in ["laplace", "focei"] {
            let request = wrap(kind, r.clone());
            assert_eq!(
                model.plan_fit(&request).unwrap_err(),
                model.execute_fit(&request).unwrap_err()
            );
        }
        r.uncertainty = true;
        let request = wrap("saem", r);
        assert_eq!(
            model.plan_fit(&request).unwrap_err(),
            model.execute_fit(&request).unwrap_err()
        );
    }
    #[test]
    fn planning_limits_are_aggregate_and_preserve_numerical_budgets() {
        let (model, r) = population_fixture(true);
        let request = wrap("focei", r);
        let p = model.plan_fit(&request).unwrap();
        let error = model
            .plan_fit_with_limit(&request, p.metadata_items - 1)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::OutputBudget);
        assert_eq!(
            error.expression.as_deref(),
            Some("plan.metadata_item_limit")
        );
        let exact = model
            .plan_fit_with_limit(&request, p.metadata_items)
            .unwrap();
        assert_eq!(exact.fit_input_hash, p.fit_input_hash);
        assert_ne!(exact.plan_hash, p.plan_hash);
        assert_eq!(
            serde_json::to_value(exact.request).unwrap(),
            serde_json::to_value(request).unwrap()
        );
    }
    #[test]
    fn fitted_residual_plans_report_actual_estimator_coordinates() {
        let (model, mut r) = population_fixture(false);
        r.additive_error_fit=Some(serde_json::from_value(json!({"output":"cp","initial":{"value":100,"unit":"ug/L"},"lower":{"value":50,"unit":"ug/L"},"upper":{"value":200,"unit":"ug/L"}})).unwrap());
        for kind in ["focei", "saem"] {
            let p = model.plan_fit(&wrap(kind, r.clone())).unwrap();
            let residual = p.population.unwrap().residual.unwrap();
            assert_eq!(residual.output_unit, "mg/L");
            if kind == "focei" {
                assert_eq!(residual.initial, 0.1);
                assert!(matches!(
                    residual.parameterization,
                    ResidualParameterization::StandardDeviation
                ));
            } else {
                assert_eq!(residual.initial, 0.1_f64.powi(2));
                assert!(matches!(
                    residual.parameterization,
                    ResidualParameterization::Variance
                ));
            }
        }
    }
}

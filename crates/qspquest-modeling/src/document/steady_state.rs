use super::*;
use qspquest_core::{
    identity::{canonical_hash, SPEC_VERSION},
    planning::*,
    run::*,
};
struct PreparedSteadyState {
    bound: BoundDocument,
    kernel: crate::linear_pk::LinearPk,
    cycle: crate::linear_pk::PreparedPeriodicSteadyState,
    identity: ExecutionIdentity,
}
struct PreparedIteration {
    bound: BoundDocument,
    cycle: crate::steady_state::PreparedCycleIteration,
    identity: ExecutionIdentity,
}
impl CompiledDocument {
    /// Solve a fully contained periodic linear PK cycle, returning pre-event amounts.
    pub fn periodic_steady_state(
        self: &Arc<Self>,
        request: &SteadyStateRequest,
    ) -> Result<SteadyStateResult, Error> {
        let prepared = self.prepare_steady_state(request)?;
        let result = prepared
            .kernel
            .execute_prepared_periodic_steady_state(prepared.cycle)?;
        let make_map = |values: &[f64]| {
            self.state_names
                .iter()
                .zip(values)
                .map(|(name, value)| {
                    (
                        name.clone(),
                        Quantity {
                            value: *value,
                            unit: self.linear_pk.as_ref().unwrap().amount_unit.clone(),
                        },
                    )
                })
                .collect()
        };
        Ok(SteadyStateResult {
            identity: prepared.identity,
            initial_states: make_map(&result.amounts),
            residuals: make_map(&result.residual),
            maximum_scaled_residual: result.maximum_scaled_residual,
        })
    }
    fn prepare_steady_state(
        self: &Arc<Self>,
        request: &SteadyStateRequest,
    ) -> Result<PreparedSteadyState, Error> {
        let run = &request.run;
        if !run.accuracy_control.is_local() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "cycle iteration does not support sampled accuracy control",
            ));
        }
        if !matches!(run.solver, Solver::AnalyticLinearPk) || self.linear_pk.is_none() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "periodic steady state requires analytic_linear_pk",
            ));
        }
        if run.seed.is_some() || !run.initial_states.is_empty() {
            return Err(invalid(
                "steady state prohibits seeds and initial state overrides",
            ));
        }
        if run
            .request_id
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 128)
        {
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
        if !self.history.is_empty()
            || !self.switches.is_empty()
            || !self.dose_switches.is_empty()
            || !run.regimen.covariate_changes.is_empty()
            || !run.regimen.checkpoints.is_empty()
        {
            return Err(Error::new(ErrorCode::Unsupported, "steady state requires a constant-covariate cycle without history, switches or checkpoints"));
        }
        if !run.regimen.samples.is_empty() || run.regimen.observations.is_some() {
            return Err(invalid(
                "steady-state requests do not take trajectory observations",
            ));
        }
        let bound = self.bind_with_covariates(&run.parameters, &run.regimen.covariates)?;
        let kernel = self.linear_pk.as_ref().unwrap().bind(&bound.values)?;
        let lagged =
            self.expand_dose_lags(&run.regimen, &run.parameters, &run.regimen.covariates)?;
        let mut protocol = self.lower_regimen_protocol_internal(&lagged, run.budgets, true)?;
        // The low-level kernel receives state amounts. Preserve the document's
        // input-unit conversion and model dose scaling exactly once.
        for event in &mut protocol.events {
            match event {
                crate::Event::Bolus { target, amount, .. }
                | crate::Event::Infusion { target, amount, .. } => {
                    let scale = bound
                        .dose_scale(*target)
                        .ok_or_else(|| invalid("missing dose scale"))?;
                    *amount *= scale;
                }
                crate::Event::Reset { .. } => {}
            }
        }
        let absolute = Unit::parse(&request.absolute_tolerance.unit)?
            .convert(request.absolute_tolerance.value, self.model.state_units[0])?;
        let cycle = kernel.prepare_periodic_steady_state(
            &protocol,
            run.budgets,
            crate::linear_pk::SteadyStateTolerance {
                absolute,
                relative: request.relative_tolerance,
            },
        )?;
        let identity = ExecutionIdentity {
            model_content_hash: self.model_content_hash.clone(),
            spec_version: SPEC_VERSION.into(),
            compiler_version: concat!("qspquest/", env!("CARGO_PKG_VERSION")).into(),
            compiler_source_sha256: env!("QSPQUEST_SOURCE_SHA256").into(),
            rustc_version: env!("QSPQUEST_RUSTC_VERSION").into(),
            target: env!("QSPQUEST_TARGET").into(),
            build_profile: env!("QSPQUEST_PROFILE").into(),
            rustflags_sha256: env!("QSPQUEST_RUSTFLAGS_SHA256").into(),
            bound_value_hash: canonical_hash(&serde_json::json!(bound.values))?,
            run_request_hash: canonical_hash(
                &serde_json::json!({"operation":"periodic_steady_state",
                "schema":run.schema,"regimen":run.regimen,"budgets":run.budgets,
                "absolute_tolerance":request.absolute_tolerance,"relative_tolerance":request.relative_tolerance}),
            )?,
            backend: "qspquest".into(),
            backend_version: env!("CARGO_PKG_VERSION").into(),
            algorithm: "linear_pk_periodic_affine_taylor20".into(),
        };
        Ok(PreparedSteadyState {
            bound,
            kernel,
            cycle,
            identity,
        })
    }
}

impl CompiledDocument {
    /// Iterate a repeated phase-time protocol using BDF, with a unit-bearing seed.
    pub fn iterate_periodic_state(
        self: &Arc<Self>,
        request: &CycleIterationRequest,
    ) -> Result<CycleIterationResult, Error> {
        let prepared = self.prepare_iteration(request)?;
        let run = &request.run;
        let result = crate::steady_state::execute_prepared_cycle_iteration(
            |state| {
                let mut next =
                    self.bind_continuing(&run.parameters, &run.regimen.covariates, Some(state))?;
                next.scheduled = true;
                Ok(next)
            },
            prepared.cycle,
        )?;
        let make_map = |values: &[f64]| {
            self.state_names
                .iter()
                .zip(&self.state_unit_names)
                .zip(values)
                .map(|((name, unit), value)| {
                    (
                        name.clone(),
                        Quantity {
                            value: *value,
                            unit: unit.clone(),
                        },
                    )
                })
                .collect()
        };
        Ok(CycleIterationResult {
            steady_state: SteadyStateResult {
                identity: prepared.identity,
                initial_states: make_map(&result.initial_states),
                residuals: make_map(&result.residuals),
                maximum_scaled_residual: result.maximum_scaled_residual,
            },
            cycles_completed: result.cycles_completed,
        })
    }
    fn prepare_iteration(
        self: &Arc<Self>,
        request: &CycleIterationRequest,
    ) -> Result<PreparedIteration, Error> {
        let run = &request.run;
        if !run.accuracy_control.is_local() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "cycle iteration does not support sampled accuracy control",
            ));
        }
        if !matches!(run.solver, Solver::DiffsolBdf) {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "cycle iteration requires diffsol_bdf",
            ));
        }
        if run.seed.is_some() {
            return Err(invalid("stochastic seeds are not supported"));
        }
        if run
            .request_id
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 128)
        {
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
        if !self.history.is_empty()
            || !self.switches.is_empty()
            || !self.dose_switches.is_empty()
            || !run.regimen.covariate_changes.is_empty()
            || !run.regimen.checkpoints.is_empty()
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "cycle iteration requires fixed covariates and no history, switches or checkpoints",
            ));
        }
        if !run.regimen.samples.is_empty() || run.regimen.observations.is_some() {
            return Err(invalid(
                "cycle iteration does not take trajectory observations",
            ));
        }
        if request.absolute_tolerances.len() != self.state_count {
            return Err(invalid("provide one convergence tolerance per state"));
        }
        let absolute = self
            .state_names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let q = request
                    .absolute_tolerances
                    .get(name)
                    .ok_or_else(|| invalid(format!("missing convergence tolerance: {name}")))?;
                Unit::parse(&q.unit)?.convert(q.value, self.model.state_units[i])
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let bound = self.bind_initial_states_internal(
            &run.parameters,
            &run.regimen.covariates,
            &run.initial_states,
        )?;
        let initial = bound.initial();
        let lagged =
            self.expand_dose_lags(&run.regimen, &run.parameters, &run.regimen.covariates)?;
        let protocol = self.lower_regimen_protocol_internal(&lagged, run.budgets, true)?;
        let options = crate::steady_state::CycleIterationOptions {
            absolute_tolerances: absolute,
            integration_absolute_tolerances: self.absolute_tolerances(&run.regimen)?,
            relative_tolerance: request.relative_tolerance,
            maximum_cycles: request.maximum_cycles,
        };
        let mut first =
            self.bind_continuing(&run.parameters, &run.regimen.covariates, Some(&initial))?;
        first.scheduled = true;
        let cycle = crate::steady_state::prepare_cycle_iteration(
            &first,
            &initial,
            &protocol,
            &options,
            run.budgets,
        )?;
        let identity = ExecutionIdentity {
            model_content_hash: self.model_content_hash.clone(),
            spec_version: SPEC_VERSION.into(),
            compiler_version: concat!("qspquest/", env!("CARGO_PKG_VERSION")).into(),
            compiler_source_sha256: env!("QSPQUEST_SOURCE_SHA256").into(),
            rustc_version: env!("QSPQUEST_RUSTC_VERSION").into(),
            target: env!("QSPQUEST_TARGET").into(),
            build_profile: env!("QSPQUEST_PROFILE").into(),
            rustflags_sha256: env!("QSPQUEST_RUSTFLAGS_SHA256").into(),
            bound_value_hash: canonical_hash(
                &serde_json::json!({"values":bound.values,"seed":initial}),
            )?,
            run_request_hash: canonical_hash(
                &serde_json::json!({"operation":"iterate_periodic_state","schema":run.schema,
                "regimen":run.regimen,"budgets":run.budgets,"time_basis":request.time_basis,
                "absolute_tolerances":request.absolute_tolerances,"relative_tolerance":request.relative_tolerance,"maximum_cycles":request.maximum_cycles}),
            )?,
            backend: "diffsol".into(),
            backend_version: crate::DIFFSOL_BACKEND_VERSION.into(),
            algorithm: "bdf_periodic_cycle_iteration".into(),
        };
        Ok(PreparedIteration {
            bound,
            cycle,
            identity,
        })
    }
}

impl CompiledDocument {
    /// Prepare the affine periodic solve without computing a transition or fixed point.
    pub fn plan_periodic_steady_state(
        self: &Arc<Self>,
        request: &SteadyStateRequest,
    ) -> Result<PeriodicPlan, Error> {
        let prepared = self.prepare_steady_state(request)?;
        let unit = self.linear_pk.as_ref().unwrap().amount_unit.clone();
        let event_quantity_units = prepared
            .cycle
            .cycle
            .events
            .iter()
            .map(|event| {
                let i = event.target();
                planning::exact_unit(self.model.state_units[i], Some(&unit))
            })
            .collect();
        let states = self
            .state_names
            .iter()
            .map(|name| PeriodicStatePlan {
                name: name.clone(),
                unit: unit.clone(),
                seed: None,
                convergence_absolute_tolerance: prepared.cycle.tolerance.absolute,
                integration_absolute_tolerance: None,
            })
            .collect();
        Ok(PeriodicPlan {
            schema: PeriodicPlanSchema::V01,
            request: PeriodicPlanRequest::SteadyState(Box::new(request.clone())),
            prospective_identity: prepared.identity,
            inputs: self.describe_bound_inputs(&prepared.bound)?,
            time_unit: self.time_unit.clone(),
            cycle: prepared.cycle.cycle,
            event_quantity_units,
            states,
            convergence_relative_tolerance: prepared.cycle.tolerance.relative,
            maximum_cycle_executions: prepared.cycle.passes,
            total_budgets: request.run.budgets,
            per_cycle_budgets: prepared.cycle.partition,
            deferred_checks: vec![
                DeferredCheck::TrajectoryDomain,
                DeferredCheck::NumericalConvergence,
                DeferredCheck::IndependentScientificQualification,
            ],
            fixed_point_uniqueness_and_stability_certified: false,
        })
    }
    /// Prepare BDF cycle iteration, preserving seed, tolerances and phase schedule.
    pub fn plan_iterate_periodic_state(
        self: &Arc<Self>,
        request: &CycleIterationRequest,
    ) -> Result<PeriodicPlan, Error> {
        let prepared = self.prepare_iteration(request)?;
        let event_quantity_units = self
            .describe_events(&prepared.cycle.protocol.events)?
            .into_iter()
            .map(|event| event.quantity_unit)
            .collect();
        let states = self
            .state_names
            .iter()
            .enumerate()
            .map(|(i, name)| PeriodicStatePlan {
                name: name.clone(),
                unit: self.state_unit_names[i].clone(),
                seed: Some(prepared.cycle.seed[i]),
                convergence_absolute_tolerance: prepared.cycle.options.absolute_tolerances[i],
                integration_absolute_tolerance: Some(
                    prepared
                        .cycle
                        .options
                        .integration_absolute_tolerances
                        .as_ref()
                        .map_or(prepared.cycle.protocol.atol, |values| values[i]),
                ),
            })
            .collect();
        Ok(PeriodicPlan {
            schema: PeriodicPlanSchema::V01,
            request: PeriodicPlanRequest::IteratePeriodic(Box::new(request.clone())),
            prospective_identity: prepared.identity,
            inputs: self.describe_bound_inputs(&prepared.bound)?,
            time_unit: self.time_unit.clone(),
            cycle: prepared.cycle.protocol,
            event_quantity_units,
            states,
            convergence_relative_tolerance: prepared.cycle.options.relative_tolerance,
            maximum_cycle_executions: prepared.cycle.options.maximum_cycles as usize,
            total_budgets: request.run.budgets,
            per_cycle_budgets: prepared.cycle.partition,
            deferred_checks: vec![
                DeferredCheck::TrajectoryDomain,
                DeferredCheck::NumericalConvergence,
                DeferredCheck::IndependentScientificQualification,
            ],
            fixed_point_uniqueness_and_stability_certified: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn affine() -> (Arc<CompiledDocument>, SteadyStateRequest) {
        let model = CompiledDocument::from_json(include_str!(
            "../../../../conformance/models/synthetic-linear-pk.json"
        ))
        .unwrap();
        let request = serde_json::from_str(include_str!(
            "../../../../conformance/requests/synthetic-linear-pk-steady.json"
        ))
        .unwrap();
        (model, request)
    }
    #[test]
    fn periodic_plan_preserves_identity_and_normalizes_carryover_in_state_units() {
        let (model, mut request) = affine();
        let plan = model.plan_periodic_steady_state(&request).unwrap();
        let result = model.periodic_steady_state(&request).unwrap();
        assert_eq!(plan.prospective_identity, result.identity);
        assert!(plan
            .states
            .iter()
            .all(|state| state.seed.is_none() && state.unit == "mg"));
        assert_eq!(plan.maximum_cycle_executions, model.state_count + 2);
        assert_eq!(
            plan.per_cycle_budgets.events,
            request.run.budgets.events / (model.state_count + 2)
        );
        let mut value = serde_json::to_value(&request).unwrap();
        value["run"]["regimen"]["administrations"][1]["delivery"]["span"]["duration"]["value"] =
            json!(10);
        request = serde_json::from_value(value).unwrap();
        let plan = model.plan_periodic_steady_state(&request).unwrap();
        let infusions: Vec<_> = plan
            .cycle
            .events
            .iter()
            .filter_map(|event| {
                if let crate::Event::Infusion {
                    time,
                    amount,
                    duration,
                    ..
                } = event
                {
                    Some((*time, *amount, *duration))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(infusions.len(), 2);
        assert!((infusions.iter().map(|x| x.1).sum::<f64>() - 12.).abs() < 1e-12);
        assert!(infusions.iter().any(|x| x.0 == 0. && x.2 == 8.));
        assert!(infusions.iter().any(|x| x.0 == 1. && x.2 == 2.));
        assert_eq!(
            plan.prospective_identity,
            model.periodic_steady_state(&request).unwrap().identity
        );
        assert!(!plan.fixed_point_uniqueness_and_stability_certified);
    }
    #[test]
    fn periodic_admission_does_not_solve_an_unresolved_fixed_point() {
        let (model, mut request) = affine();
        request.run.parameters.insert(
            "cl".into(),
            Quantity {
                value: 0.,
                unit: "mL/h".into(),
            },
        );
        model.plan_periodic_steady_state(&request).unwrap();
        assert_eq!(
            model.periodic_steady_state(&request).unwrap_err().code,
            ErrorCode::Unsupported
        );
        request.run.budgets.events = model.state_count + 1;
        assert_eq!(
            model.plan_periodic_steady_state(&request).unwrap_err().code,
            ErrorCode::WorkBudget
        );
        assert_eq!(
            model.periodic_steady_state(&request).unwrap_err().code,
            ErrorCode::WorkBudget
        );
    }
    #[test]
    fn iteration_plan_admits_seed_and_tolerances_without_evaluating_rhs() {
        let source = "model synthetic_periodic_poison\ntime_unit = h\nparameters\n rate = 1 [mg/h] fixed\nstates\n x = 1 [mg]\ndynamics\n d/dt(x) = (rate / ((x / x) - (x / x)))\noutputs\n y = x [mg]\nend";
        let model = crate::language::parse(source).unwrap().compile().unwrap();
        let mut request: CycleIterationRequest = serde_json::from_value(json!({
            "run":{"schema":"qspquest.run/v0.1","solver":"diffsol_bdf",
                "budgets":{"solver_callbacks":10000,"events":100,"output_values":1000},
                "initial_states":{"x":{"value":2,"unit":"mg"}},
                "regimen":{"end":{"value":1,"unit":"h"},"samples":[],"administrations":[],"rtol":1e-8,
                    "absolute_tolerances":{"x":{"value":1e-9,"unit":"g"}}}},
            "time_basis":"cycle_phase","absolute_tolerances":{"x":{"value":1e-6,"unit":"g"}},
            "relative_tolerance":1e-7,"maximum_cycles":2
        }))
        .unwrap();
        let plan = model.plan_iterate_periodic_state(&request).unwrap();
        assert_eq!(plan.states[0].seed, Some(2.));
        assert!((plan.states[0].convergence_absolute_tolerance - 1e-3).abs() < 1e-16);
        assert!((plan.states[0].integration_absolute_tolerance.unwrap() - 1e-6).abs() < 1e-19);
        assert_eq!(
            model.iterate_periodic_state(&request).unwrap_err().code,
            ErrorCode::Domain
        );
        request.run.budgets.output_values = 1;
        assert_eq!(
            model
                .plan_iterate_periodic_state(&request)
                .unwrap_err()
                .code,
            ErrorCode::OutputBudget
        );
        assert_eq!(
            model.iterate_periodic_state(&request).unwrap_err().code,
            ErrorCode::OutputBudget
        );
    }
}

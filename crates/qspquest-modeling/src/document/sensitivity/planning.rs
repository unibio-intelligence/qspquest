use super::*;
use qspquest_core::{fit::*, planning::*, regimen::ObservationSide, run::SensitivityRequest};

impl CompiledSensitivityDocument {
    /// Prepare the actual augmented sensitivity workflow without integration or readouts.
    pub fn plan_run(
        self: &Arc<Self>,
        request: &SensitivityRequest,
    ) -> Result<SensitivityPlan, Error> {
        let prepared = self.prepare_run(request)?;
        self.describe_prepared_run(request, &prepared)
    }

    pub(super) fn describe_prepared_run(
        &self,
        request: &SensitivityRequest,
        prepared: &super::run::PreparedSensitivityRun,
    ) -> Result<SensitivityPlan, Error> {
        use super::super::planning::exact_unit;
        let regimen = &prepared.regimen;
        let model = &regimen.model;
        let protocol = &regimen.protocol;
        let coordinates = crate::simulation_rows(model, protocol);
        let driver_rows: Vec<_> = coordinates
            .iter()
            .map(|(time, side)| PlannedObservation {
                time: *time,
                side: if *side == "pre" {
                    ObservationSide::Pre
                } else {
                    ObservationSide::Post
                },
            })
            .collect();
        let observations = regimen
            .projection
            .selected
            .iter()
            .map(|(row, _, side)| PlannedObservation {
                time: driver_rows[*row].time,
                side: if side == "pre" {
                    ObservationSide::Pre
                } else {
                    ObservationSide::Post
                },
            })
            .collect();
        let observation_rows = regimen
            .projection
            .selected
            .iter()
            .map(|(row, _, _)| *row)
            .collect();
        let initial = model.initial();
        let n = self.primal.state_count;
        let augmented_states = self
            .forward
            .augmented_state_units
            .iter()
            .enumerate()
            .map(|(i, unit)| {
                let state = self.primal.state_names[i % n].clone();
                let parameter = (i >= n).then(|| self.parameters()[(i - n) / n].clone());
                AugmentedStatePlan {
                    state,
                    parameter,
                    unit: exact_unit(
                        *unit,
                        (i < n).then(|| self.primal.state_unit_names[i].as_str()),
                    ),
                    initial: initial[i],
                    absolute_tolerance: regimen.atol[i],
                }
            })
            .collect();
        let outputs: Vec<_> = self
            .output_names()
            .iter()
            .zip(self.output_units())
            .map(|(name, unit)| PlannedOutput {
                name: name.clone(),
                unit: unit.clone(),
            })
            .collect();
        let mut derivatives = Vec::new();
        for (p, parameter) in self.parameters().iter().enumerate() {
            for (o, output) in self.output_names().iter().enumerate() {
                derivatives.push(DerivativeOutputPlan {
                    output: output.clone(),
                    parameter: parameter.clone(),
                    unit: exact_unit(self.derivative_units()[p][o], None),
                });
            }
        }
        let mut binding_phases = vec![BindingPhase {
            time: protocol.start,
            activation: BindingActivation::Initial,
            inputs: self.primal.describe_bound_inputs(&model.base.primal)?,
        }];
        for (time, bound) in &model.bindings {
            binding_phases.push(BindingPhase {
                time: *time,
                activation: BindingActivation::PostBoundary,
                inputs: self.primal.describe_bound_inputs(&bound.primal)?,
            });
        }
        let width = outputs.len() + derivatives.len();
        let workload = RunWorkload {
            states: initial.len(),
            outputs: width,
            events: protocol.events.len(),
            reserved_events: protocol.events.len() + model.discontinuities().len(),
            binding_phases: binding_phases.len(),
            driver_rows: driver_rows.len(),
            result_rows: regimen.result_rows(),
            reserved_binding_values: request.run.budgets.output_values
                - regimen.budgets.output_values,
            stored_state_values: driver_rows.len() * initial.len(),
            stored_output_values: regimen.result_rows() * width,
            readout_override_values: 0,
            budgets: request.run.budgets,
        };
        Ok(SensitivityPlan {
            schema: SensitivityPlanSchema::V01,
            request: request.clone(),
            prospective_identity: self.prospective_identity(request, &prepared.bound)?,
            time_unit: self.primal.time_unit.clone(),
            start: protocol.start,
            end: protocol.end,
            relative_tolerance: protocol.rtol,
            augmented_states,
            outputs,
            derivatives,
            binding_phases,
            events: self.primal.describe_events(&protocol.events)?,
            checkpoints: model.discontinuities().to_vec(),
            driver_rows,
            observations,
            observation_rows,
            workload,
            deferred_checks: vec![
                DeferredCheck::TrajectoryDomain,
                DeferredCheck::ReadoutDomain,
                DeferredCheck::NumericalConvergence,
                DeferredCheck::IndependentScientificQualification,
            ],
        })
    }

    /// Check the actual observation/prior and augmented simulation preparation.
    /// Prediction-dependent likelihood and gradients are not evaluated.
    pub fn plan_gaussian_objective(
        self: &Arc<Self>,
        request: &GaussianObjectiveRequest,
    ) -> Result<GaussianObjectivePlan, Error> {
        let prepared = self.prepare_gaussian_objective(request)?;
        self.describe_gaussian_objective(request, &prepared)
    }

    pub(super) fn describe_gaussian_objective(
        &self,
        request: &GaussianObjectiveRequest,
        prepared: &super::objective::PreparedGaussianObjective,
    ) -> Result<GaussianObjectivePlan, Error> {
        let simulation = self.describe_prepared_run(&prepared.simulation, &prepared.run)?;
        let observations = prepared
            .data
            .iter()
            .map(|row| GaussianObservationPlan {
                row: row.row,
                output: self.output_names()[row.output].clone(),
                unit: self.output_units()[row.output].clone(),
                value: row.value,
                additive_sd: row.additive,
                proportional_sd: row.proportional,
                model: row.model,
                censoring: row.censored.then_some(GaussianCensoring::Left),
            })
            .collect();
        Ok(GaussianObjectivePlan {
            schema: GaussianObjectivePlanSchema::V01,
            request: request.clone(),
            prospective_identity: Self::prospective_objective_identity(
                request,
                simulation.prospective_identity.clone(),
            )?,
            simulation,
            observations,
            priors: prepared.priors.clone(),
            reserved_objective_values: request.simulation.run.budgets.output_values
                - prepared.simulation.run.budgets.output_values,
            prediction_dependent_checks_deferred: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn fixture() -> (Value, SensitivityRequest) {
        (
            serde_json::from_str(include_str!(
                "../../../../../conformance/models/synthetic-one-compartment.json"
            ))
            .unwrap(),
            serde_json::from_str(include_str!(
                "../../../../../conformance/requests/synthetic-sensitivity.json"
            ))
            .unwrap(),
        )
    }

    #[test]
    fn sensitivity_plan_reports_actual_augmented_initial_units_phases_and_sides() {
        let (mut document, mut request) = fixture();
        request.with_respect_to.reverse();
        document["states"][0]["initial"] = json!({"kind":"expression","expression":{
            "kind":"binary","operator":"multiply","arguments":[{"kind":"symbol","name":"v"},
            {"kind":"symbol","name":"initial_cp"}]}});
        document["parameters"].as_array_mut().unwrap().push(json!({"name":"initial_cp","default":{"value":2,"unit":"mg/L"},"transform":"identity","fixed":true,"description":""}));
        document["covariates"] = json!([{"name":"factor","unit":"1","default":{"value":1,"unit":"1"},"interpolation":"step"}]);
        document["states"][0]["rhs"] = json!({"kind":"binary","operator":"multiply","arguments":[document["states"][0]["rhs"].clone(),{"kind":"symbol","name":"factor"}]});
        request.run.regimen.samples.clear();
        request.run.regimen.observations = Some(
            serde_json::from_value(json!([
            {"time":{"value":2,"unit":"h"},"side":"post"},
            {"time":{"value":0,"unit":"h"},"side":"post"},
            {"time":{"value":0,"unit":"h"},"side":"pre"},
            {"time":{"value":1,"unit":"h"},"side":"pre"},
            {"time":{"value":1,"unit":"h"},"side":"post"},
            {"time":{"value":0,"unit":"h"},"side":"post"}]))
            .unwrap(),
        );
        request.run.regimen.covariate_changes = serde_json::from_value(json!([
            {"name":"factor","time":{"value":1,"unit":"h"},"value":{"value":2,"unit":"1"}}]))
        .unwrap();
        let model = CompiledSensitivityDocument::compile(
            &serde_json::from_value(document).unwrap(),
            &request.with_respect_to,
        )
        .unwrap();
        let plan = model.plan_run(&request).unwrap();
        assert_eq!(plan.augmented_states.len(), 3);
        assert_eq!(plan.augmented_states[0].initial, 6.);
        assert_eq!(plan.augmented_states[1].parameter.as_deref(), Some("v"));
        assert_eq!(plan.augmented_states[1].initial, 2.);
        assert_eq!(plan.augmented_states[2].parameter.as_deref(), Some("cl"));
        assert_eq!(plan.augmented_states[2].initial, 0.);
        assert_eq!(plan.augmented_states[1].absolute_tolerance, 1e-12);
        assert_eq!(
            plan.augmented_states[1].unit.dimensions,
            [1, -3, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(plan.binding_phases[1].time, 1.);
        assert!(plan.binding_phases[1]
            .inputs
            .iter()
            .any(|i| i.name == "factor" && i.value == 2.));
        assert_eq!(plan.observation_rows, [4, 1, 0, 2, 3, 1]);
        assert_eq!(plan.workload.states, 3);
        assert_eq!(plan.workload.outputs, 3);
        assert_eq!(plan.workload.stored_state_values, 18);
        assert_eq!(plan.workload.stored_output_values, 18);
        let result = model.run(&request).unwrap();
        assert_eq!(plan.prospective_identity, result.result.identity);
        assert_eq!(
            plan.observations.iter().map(|p| p.time).collect::<Vec<_>>(),
            result.result.times
        );
        assert_eq!(
            plan.observations.iter().map(|p| p.side).collect::<Vec<_>>(),
            result.result.sides
        );
        assert_eq!(result.result.outputs[0].values[1], 4.);
        assert_eq!(result.result.outputs[0].values[2], 2.);
        assert_eq!(plan.derivatives[0].parameter, "v");
        Unit::parse(&result.derivatives[0].unit)
            .unwrap()
            .conversion_to(model.derivative_units()[0][0])
            .unwrap();
        let mut json = serde_json::to_value(plan).unwrap();
        json["executed"] = json!(true);
        assert!(serde_json::from_value::<SensitivityPlan>(json).is_err());
    }

    #[test]
    fn sensitivity_plan_retains_static_refusals_without_readouts() {
        let (mut document, request) = fixture();
        document["outputs"].as_array_mut().unwrap().push(json!({"name":"undefined","unit":"1","expression":{
            "kind":"binary","operator":"divide","arguments":[{"kind":"literal","value":0},{"kind":"literal","value":0}]}}));
        let model = CompiledSensitivityDocument::compile(
            &serde_json::from_value(document).unwrap(),
            &request.with_respect_to,
        )
        .unwrap();
        assert!(model.plan_run(&request).is_ok());
        assert_eq!(model.run(&request).unwrap_err().code, ErrorCode::Domain);
        for case in 0..5 {
            let mut invalid = request.clone();
            match case {
                0 => invalid.with_respect_to.reverse(),
                1 => {
                    invalid
                        .derivative_absolute_tolerances
                        .get_mut("cl")
                        .unwrap()
                        .get_mut("central")
                        .unwrap()
                        .unit = "mg".into()
                }
                2 => invalid.run.budgets.output_values = 1,
                3 => invalid.run.accuracy_control = qspquest_core::run::AccuracyControl::Sampled,
                _ => invalid.run.regimen.samples[1].value = -1.,
            }
            assert_eq!(
                model.plan_run(&invalid).unwrap_err(),
                model.run(&invalid).unwrap_err(),
                "case {case}"
            );
        }
    }

    fn objective() -> (Value, GaussianObjectiveRequest) {
        let (document, simulation) = fixture();
        let request = GaussianObjectiveRequest { simulation,
            observations: serde_json::from_value(json!([
              {"row":2,"output":"cp","value":{"value":1500,"unit":"ug/L"},"error":{"additive_sd":{"value":100,"unit":"ug/L"},"proportional_sd":0}},
              {"row":3,"output":"cp","value":{"value":5000,"unit":"ug/L"},"error":{"additive_sd":{"value":100,"unit":"ug/L"},"proportional_sd":0},"censoring":"left"}])).unwrap(),
            priors: serde_json::from_value(json!([{"parameter":"cl","mean":{"value":600,"unit":"mL/h"},"sd":{"value":300,"unit":"mL/h"}}])).unwrap() };
        (document, request)
    }

    #[test]
    fn objective_plan_reports_converted_observations_priors_reserved_budget_and_full_identity() {
        let (document, request) = objective();
        let model = CompiledSensitivityDocument::compile(
            &serde_json::from_value(document).unwrap(),
            &request.simulation.with_respect_to,
        )
        .unwrap();
        let plan = model.plan_gaussian_objective(&request).unwrap();
        assert_eq!(plan.observations[0].value, 1.5);
        assert_eq!(plan.observations[0].additive_sd, 0.1);
        assert_eq!(plan.observations[0].unit, "mg/L");
        assert_eq!(
            plan.observations[1].censoring,
            Some(GaussianCensoring::Left)
        );
        assert_eq!(plan.priors[0].mean, 0.6);
        assert_eq!(plan.priors[0].sd, 0.3);
        assert_eq!(plan.priors[0].unit, "L/h");
        assert_eq!(plan.reserved_objective_values, 17);
        assert_eq!(
            plan.simulation.request.run.budgets.output_values,
            request.simulation.run.budgets.output_values - 17
        );
        assert!(plan.prediction_dependent_checks_deferred);
        let result = model.gaussian_objective(&request).unwrap();
        assert_eq!(plan.prospective_identity, result.identity);
        assert_ne!(
            plan.prospective_identity.run_request_hash,
            plan.simulation.prospective_identity.run_request_hash
        );
    }

    #[test]
    fn objective_plan_refuses_static_errors_and_defers_prediction_domains() {
        let (mut document, request) = objective();
        document["outputs"][0]["expression"] = json!({"kind":"binary","operator":"divide","arguments":[{"kind":"literal","value":0},{"kind":"literal","value":0}]});
        document["outputs"][0]["unit"] = json!("1");
        let model = CompiledSensitivityDocument::compile(
            &serde_json::from_value(document).unwrap(),
            &request.simulation.with_respect_to,
        )
        .unwrap();
        let mut valid = request;
        for observation in &mut valid.observations {
            observation.value.unit = "1".into();
            observation.error.additive_sd.unit = "1".into();
        }
        assert!(model.plan_gaussian_objective(&valid).is_ok());
        assert_eq!(
            model.gaussian_objective(&valid).unwrap_err().code,
            ErrorCode::Domain
        );
        for case in 0..4 {
            let mut invalid = valid.clone();
            match case {
                0 => invalid.observations[1].row = 99,
                1 => invalid.observations[1].value.unit = "mg".into(),
                2 => invalid.priors[0].sd.value = 0.,
                _ => invalid.simulation.run.budgets.output_values = 10,
            }
            assert_eq!(
                model.plan_gaussian_objective(&invalid).unwrap_err(),
                model.gaussian_objective(&invalid).unwrap_err(),
                "case {case}"
            );
        }
    }
}

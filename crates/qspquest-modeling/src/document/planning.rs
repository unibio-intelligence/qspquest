use super::*;
use qspquest_core::{planning::*, regimen::ObservationSide, run::RunRequest};

pub(super) fn exact_unit(unit: Unit, declared: Option<&str>) -> ExactUnit {
    ExactUnit {
        dimensions: unit.dimension.physical_activity(),
        entities: unit.dimension.entities(),
        scale_numerator: unit.scale.numerator().to_string(),
        scale_denominator: unit.scale.denominator().to_string(),
        declared: declared.map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn fixture() -> (Value, RunRequest) {
        (
            serde_json::from_str(include_str!(
                "../../../../conformance/models/synthetic-one-compartment.json"
            ))
            .unwrap(),
            serde_json::from_str(include_str!(
                "../../../../conformance/requests/synthetic-one-compartment.json"
            ))
            .unwrap(),
        )
    }

    #[test]
    fn typed_plan_reports_bound_values_exact_inferred_units_and_real_delivery() {
        let (mut document, mut run) = fixture();
        document["individual"] = json!([{"name":"rate","expression":{"kind":"binary","operator":"divide","arguments":[{"kind":"symbol","name":"cl"},{"kind":"symbol","name":"v"}]}}]);
        run.parameters.insert(
            "cl".into(),
            Quantity {
                value: 300.,
                unit: "mL/h".into(),
            },
        );
        run.initial_states.insert(
            "central".into(),
            Quantity {
                value: 3000.,
                unit: "ug".into(),
            },
        );
        run.regimen.administrations[0].delivery = qspquest_core::regimen::Delivery::Infusion {
            span: qspquest_core::regimen::InfusionSpan::Duration {
                duration: Quantity {
                    value: 60.,
                    unit: "min".into(),
                },
            },
        };
        let compiled =
            CompiledDocument::compile(&serde_json::from_value(document).unwrap()).unwrap();
        let plan = compiled.plan_run(&run).unwrap();
        let cl = plan.binding_phases[0]
            .inputs
            .iter()
            .find(|p| p.name == "cl")
            .unwrap();
        assert_eq!(cl.value, 0.3);
        assert_eq!(cl.unit.declared.as_deref(), Some("L/h"));
        let rate = plan.binding_phases[0]
            .inputs
            .iter()
            .find(|p| p.name == "rate")
            .unwrap();
        let base_rate = rate.value * rate.unit.scale_numerator.parse::<f64>().unwrap()
            / rate.unit.scale_denominator.parse::<f64>().unwrap();
        assert!((base_rate - 0.1 / 3600.).abs() < 1e-18);
        assert_eq!(rate.unit.dimensions, [0, 0, -1, 0, 0, 0, 0, 0]);
        assert_eq!(rate.unit.scale_numerator, "1");
        assert_eq!(rate.unit.scale_denominator, "1");
        assert!(rate.unit.declared.is_none());
        assert_eq!(plan.states[0].initial, 3.);
        assert!(matches!(
            plan.events[0].event,
            crate::Event::Infusion {
                duration: 1.,
                amount: 6.,
                ..
            }
        ));
        let executed = compiled.execute(&run).unwrap();
        assert_eq!(plan.prospective_identity, executed.identity);
        let mut value = serde_json::to_value(&plan).unwrap();
        assert_eq!(value["schema"], "qspquest.run-plan/v0.1");
        assert!(value["outputs"][0].get("values").is_none());
        value["executed"] = json!(true);
        assert!(serde_json::from_value::<RunPlan>(value).is_err());
    }

    #[test]
    fn entity_units_reach_the_plan_and_reject_wrong_entity_overrides() {
        let (mut document, mut run) = fixture();
        document["states"][0]["unit"] = json!("[cell]");
        document["states"][0]["initial"]["quantity"]["unit"] = json!("[cell]");
        document["states"][0]["dosing"]["amount_unit"] = json!("[cell]");
        document["outputs"][0]["unit"] = json!("[cell]/L");
        run.regimen.administrations[0].amount.unit = "[cell]".into();
        let compiled =
            CompiledDocument::compile(&serde_json::from_value(document).unwrap()).unwrap();
        let plan = compiled.plan_run(&run).unwrap();
        assert_eq!(
            plan.events[0]
                .quantity_unit
                .entities
                .get(&qspquest_core::units::Entity::Cell),
            Some(&1)
        );
        assert_eq!(plan.events[0].quantity_unit.dimensions, [0; 8]);
        assert_eq!(plan.states[0].unit, "[cell]");
        run.initial_states.insert(
            "central".into(),
            Quantity {
                value: 1.,
                unit: "[genome]".into(),
            },
        );
        let error = compiled.plan_run(&run).unwrap_err();
        assert_eq!(error.code, ErrorCode::Unit);
        assert!(!error.message.contains("molar mass"));
    }

    #[test]
    fn plan_does_not_integrate_or_certify_future_domain_success() {
        for invalid_rhs in [false, true] {
            let (mut document, run) = fixture();
            let undefined = json!({"kind":"binary","operator":"divide","arguments":[{"kind":"literal","value":0},{"kind":"literal","value":0}]});
            if invalid_rhs {
                document["states"][0]["rhs"] = json!({"kind":"binary","operator":"multiply","arguments":[document["states"][0]["rhs"].clone(),undefined]});
            } else {
                document["outputs"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"name":"undefined","unit":"1","expression":undefined}));
            }
            let compiled =
                CompiledDocument::compile(&serde_json::from_value(document).unwrap()).unwrap();
            let plan = compiled.plan_run(&run).unwrap();
            assert_eq!(plan.deferred_checks.len(), 4);
            assert_eq!(compiled.execute(&run).unwrap_err().code, ErrorCode::Domain);
        }
    }

    #[test]
    fn plan_reports_actual_pointwise_readout_inputs_and_selection() {
        let (mut document, run) = fixture();
        document["parameters"].as_array_mut().unwrap().push(json!({
            "name":"boundary","default":{"value":2,"unit":"h"},
            "transform":"identity","fixed":true,"description":""
        }));
        document["switches"] =
            json!([{"name":"phase","time":{"kind":"symbol","name":"boundary"},"comparison":"le"}]);
        document["dose_switches"] = json!([{"name":"recent","target":"central","offset":{"kind":"symbol","name":"boundary"},"comparison":"le","simultaneous":"first"}]);
        for name in ["phase", "recent"] {
            document["outputs"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name":name,"unit":"1","expression":{"kind":"symbol","name":name}}));
        }
        let compiled =
            CompiledDocument::compile(&serde_json::from_value(document).unwrap()).unwrap();
        let selected = ["phase".into(), "recent".into()];
        let plan = compiled.plan_run_selected(&run, Some(&selected)).unwrap();
        let at_two = plan
            .readout_overrides
            .iter()
            .find(|p| plan.observations[p.observation].time == 2.)
            .unwrap();
        assert_eq!(at_two.covariates["phase"].value, 1.);
        assert!(at_two
            .covariates
            .iter()
            .any(|(name, q)| name.ends_with("_value") && q.value == 1.));
        assert_eq!(plan.selected_outputs.as_deref(), Some(selected.as_slice()));
        let result = compiled.execute_selected(&run, Some(&selected)).unwrap();
        assert_eq!(plan.prospective_identity, result.identity);
        assert_eq!(result.outputs[0].values[2], 1.);
        assert_eq!(result.outputs[1].values[2], 1.);
        assert!(plan
            .binding_phases
            .iter()
            .flat_map(|p| &p.inputs)
            .filter(|i| i.name == "phase" || i.name.starts_with("dose_switch_internal_"))
            .all(|i| matches!(i.kind, BindingKind::DerivedCovariate)));
    }
    #[test]
    fn pointwise_plan_metadata_is_bounded_before_retention() {
        let (mut document, mut run) = fixture();
        document["parameters"].as_array_mut().unwrap().push(json!({
            "name":"offset","default":{"value":100,"unit":"h"},"transform":"identity","fixed":true,"description":""
        }));
        document["dose_switches"]=json!((0..5).map(|i|json!({"name":format!("recent_{i}"),"target":"central","offset":{"kind":"symbol","name":"offset"},"comparison":"le","simultaneous":"first"})).collect::<Vec<_>>());
        run.regimen.observations = None;
        run.regimen.samples = (0..300)
            .map(|i| Quantity {
                value: i as f64 * 12. / 299.,
                unit: "h".into(),
            })
            .collect();
        run.budgets.output_values = 2_000;
        let compiled =
            CompiledDocument::compile(&serde_json::from_value(document).unwrap()).unwrap();
        compiled.prepare_selected(&run, None).unwrap();
        let error = compiled.plan_run(&run).unwrap_err();
        assert_eq!(error.code, ErrorCode::OutputBudget);
        assert_eq!(error.expression.as_deref(), Some("plan.readout_overrides"));
    }
}

impl CompiledDocument {
    /// Prepare a run without integration or readout evaluation.
    /// The prospective identity is shared with execution; future domain and convergence
    /// checks remain unresolved, and execution repeats preparation on its current inputs.
    pub fn plan_run(self: &Arc<Self>, request: &RunRequest) -> Result<RunPlan, Error> {
        self.plan_run_selected(request, None)
    }

    /// Prepare an ordered output selection under the same contract as execute_selected.
    pub fn plan_run_selected(
        self: &Arc<Self>,
        request: &RunRequest,
        selected: Option<&[String]>,
    ) -> Result<RunPlan, Error> {
        let prepared = self.prepare_selected(request, selected)?;
        self.describe_prepared_run(request, selected, &prepared)
    }

    pub(super) fn planned_driver_rows(prepared: &PreparedRun) -> Vec<PlannedObservation> {
        crate::simulation_rows(&prepared.regimen.model, &prepared.regimen.protocol)
            .into_iter()
            .map(|(time, side)| PlannedObservation {
                time,
                side: if side == "pre" {
                    ObservationSide::Pre
                } else {
                    ObservationSide::Post
                },
            })
            .collect()
    }
    pub(super) fn planned_observations(
        &self,
        prepared: &PreparedRun,
        driver_rows: &[PlannedObservation],
    ) -> Result<Vec<PlannedObservation>, Error> {
        if let Some(points) = &prepared.regimen.observations {
            points
                .iter()
                .map(|p| {
                    Ok(PlannedObservation {
                        time: quantity(&p.time)?.in_unit(self.model.time_unit)?,
                        side: p.side,
                    })
                })
                .collect()
        } else {
            Ok(driver_rows.to_vec())
        }
    }

    pub(super) fn describe_prepared_run(
        &self,
        request: &RunRequest,
        selected: Option<&[String]>,
        prepared: &PreparedRun,
    ) -> Result<RunPlan, Error> {
        let regimen = &prepared.regimen;
        let model = &regimen.model;
        let protocol = &regimen.protocol;
        let driver_rows = Self::planned_driver_rows(prepared);
        let observations = self.planned_observations(prepared, &driver_rows)?;
        let observation_rows = regimen
            .observation_indices
            .clone()
            .unwrap_or_else(|| (0..driver_rows.len()).collect());
        let mut readout_overrides = Vec::new();
        let mut readout_override_values = 0usize;
        for (observation, point) in observations.iter().enumerate() {
            let values = model.readout_covariate_overrides(
                point.time,
                match point.side {
                    ObservationSide::Pre => "pre",
                    ObservationSide::Post => "post",
                },
            )?;
            readout_override_values = readout_override_values.checked_add(values.len())
                .filter(|count|*count<=request.budgets.output_values)
                .ok_or_else(|| {let mut error=Error::new(ErrorCode::OutputBudget,
                    "pointwise readout-input metadata exceeds the plan budget; reduce observation rows or predicates");
                    error.expression=Some("plan.readout_overrides".into());error})?;
            if !values.is_empty() {
                readout_overrides.push(ReadoutInputOverrides {
                    observation,
                    covariates: values,
                });
            }
        }
        let output_indices = prepared
            .indices
            .clone()
            .unwrap_or_else(|| (0..self.outputs.len()).collect());
        let outputs: Vec<_> = output_indices
            .into_iter()
            .map(|index| PlannedOutput {
                name: self.output_names[index].clone(),
                unit: self.output_units[index].clone(),
            })
            .collect();
        let initial = model.base.initial();
        let states = self
            .state_names
            .iter()
            .enumerate()
            .map(|(i, name)| PlannedState {
                name: name.clone(),
                unit: self.state_unit_names[i].clone(),
                initial: initial[i],
                absolute_tolerance: regimen
                    .absolute_tolerances
                    .as_ref()
                    .map_or(protocol.atol, |v| v[i]),
            })
            .collect();
        let mut binding_phases = vec![BindingPhase {
            time: protocol.start,
            activation: BindingActivation::Initial,
            inputs: self.describe_bound_inputs(&model.base)?,
        }];
        for (time, bound) in model.times.iter().zip(&model.bindings) {
            binding_phases.push(BindingPhase {
                time: *time,
                activation: BindingActivation::PostBoundary,
                inputs: self.describe_bound_inputs(bound)?,
            });
        }
        let events = self.describe_events(&protocol.events)?;
        let workload = RunWorkload {
            states: self.state_count,
            outputs: outputs.len(),
            events: protocol.events.len(),
            reserved_events: protocol.events.len() + model.discontinuities().len(),
            binding_phases: binding_phases.len(),
            driver_rows: driver_rows.len(),
            result_rows: regimen.result_rows,
            reserved_binding_values: request.budgets.output_values - regimen.budgets.output_values,
            stored_state_values: driver_rows.len() * self.state_count,
            stored_output_values: regimen.result_rows * outputs.len(),
            readout_override_values,
            budgets: request.budgets,
        };
        Ok(RunPlan {
            schema: RunPlanSchema::V01,
            request: request.clone(),
            selected_outputs: selected.map(|s| s.to_vec()),
            prospective_identity: self.prospective_identity(request, selected, &prepared.bound)?,
            time_unit: self.time_unit.clone(),
            start: protocol.start,
            end: protocol.end,
            relative_tolerance: protocol.rtol,
            states,
            outputs,
            binding_phases,
            events,
            checkpoints: model.boundaries.clone(),
            driver_rows,
            observations,
            observation_rows,
            readout_overrides,
            workload,
            deferred_checks: vec![
                DeferredCheck::TrajectoryDomain,
                DeferredCheck::ReadoutDomain,
                DeferredCheck::NumericalConvergence,
                DeferredCheck::IndependentScientificQualification,
            ],
        })
    }

    pub(super) fn describe_events(
        &self,
        events: &[crate::Event],
    ) -> Result<Vec<PlannedEvent>, Error> {
        events
            .iter()
            .map(|event| {
                let index = event.target();
                let unit = if matches!(event, crate::Event::Reset { .. }) {
                    exact_unit(
                        Unit::parse(&self.state_unit_names[index])?,
                        Some(&self.state_unit_names[index]),
                    )
                } else {
                    let target = self
                        .dose_targets
                        .iter()
                        .find(|target| target.state == index)
                        .ok_or_else(|| invalid("prepared dose has no declared input unit"))?;
                    exact_unit(
                        target.amount_unit,
                        self.dose_unit_names.get(&index).map(String::as_str),
                    )
                };
                Ok(PlannedEvent {
                    event: event.clone(),
                    target_name: self.state_names[index].clone(),
                    quantity_unit: unit,
                })
            })
            .collect::<Result<Vec<_>, Error>>()
    }

    pub(super) fn describe_bound_inputs(
        &self,
        bound: &BoundDocument,
    ) -> Result<Vec<BoundInput>, Error> {
        let mut inputs = Vec::with_capacity(bound.values.len());
        for (i, parameter) in self.parameters.iter().enumerate() {
            inputs.push(BoundInput {
                name: parameter.name.clone(),
                kind: BindingKind::Parameter,
                value: bound.values[i],
                unit: exact_unit(
                    Unit::parse(&parameter.default.unit)?,
                    Some(&parameter.default.unit),
                ),
            });
        }
        for (i, covariate) in self.covariates.iter().enumerate() {
            inputs.push(BoundInput {
                name: covariate.name.clone(),
                kind: if self
                    .history
                    .iter()
                    .any(|b| b.owns_covariate(&covariate.name))
                    || self
                        .switches
                        .iter()
                        .any(|(name, _, _)| name == &covariate.name)
                    || self
                        .dose_switches
                        .iter()
                        .any(|b| b.value == covariate.name || b.available == covariate.name)
                {
                    BindingKind::DerivedCovariate
                } else {
                    BindingKind::Covariate
                },
                value: bound.values[self.parameters.len() + i],
                unit: exact_unit(Unit::parse(&covariate.unit)?, Some(&covariate.unit)),
            });
        }
        for (i, (name, unit, _)) in self.individual.iter().enumerate() {
            inputs.push(BoundInput {
                name: name.clone(),
                kind: BindingKind::IndividualParameter,
                value: bound.values[self.parameters.len() + self.covariates.len() + i],
                unit: exact_unit(
                    *unit,
                    self.individual_unit_names.get(name).map(String::as_str),
                ),
            });
        }
        Ok(inputs)
    }
}

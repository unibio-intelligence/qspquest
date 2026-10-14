use super::*;
use qspquest_core::initial_values::*;

struct PreparedInitialValues {
    bound: BoundDocument,
    indices: Vec<usize>,
    plan: InitialValuesPlan,
}
impl CompiledDocument {
    fn prepare_initial_values(
        self: &Arc<Self>,
        request: &InitialValuesRequest,
    ) -> Result<PreparedInitialValues, Error> {
        if request.max_output_values == 0 {
            return Err(invalid("max_output_values must be positive"));
        }
        let indices = if let Some(names) = &request.outputs {
            if names.is_empty() {
                return Err(invalid("outputs must be nonempty when supplied"));
            }
            let mut seen = std::collections::BTreeSet::new();
            names
                .iter()
                .map(|name| {
                    if !seen.insert(name) {
                        return Err(invalid(format!("duplicate output: {name}")));
                    }
                    self.output_names
                        .iter()
                        .position(|n| n == name)
                        .ok_or_else(|| invalid(format!("unknown output: {name}")))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            (0..self.outputs.len()).collect()
        };
        let output_values = self
            .state_count
            .checked_add(indices.len())
            .ok_or_else(|| invalid("initial-value result width overflow"))?;
        if output_values > request.max_output_values {
            return Err(Error::new(
                ErrorCode::OutputBudget,
                "initial states and outputs exceed max_output_values",
            ));
        }
        let bound = self.bind_with_initial_states(
            &request.parameters,
            &request.covariates,
            &request.initial_states,
        )?;
        let outputs = indices
            .iter()
            .map(|&i| qspquest_core::planning::PlannedOutput {
                name: self.output_names[i].clone(),
                unit: self.output_units[i].clone(),
            })
            .collect::<Vec<_>>();
        let request_hash = qspquest_core::identity::canonical_hash(&serde_json::json!({
            "schema": request.schema, "time": 0.0, "outputs": outputs,
            "max_output_values": request.max_output_values,
        }))?;
        let identity = self.bound_execution_identity(
            &bound,
            request_hash,
            "qspquest",
            env!("CARGO_PKG_VERSION"),
            "initial_output_evaluation",
        )?;
        let initial = bound.initial();
        let states = self
            .state_names
            .iter()
            .enumerate()
            .map(|(i, name)| InitialState {
                name: name.clone(),
                unit: self.state_unit_names[i].clone(),
                value: initial[i],
            })
            .collect();
        let plan = InitialValuesPlan {
            schema: PlanSchema::V01, request: request.clone(), prospective_identity: identity,
            time: 0.0, time_unit: self.time_unit.clone(),
            exact_time_unit: planning::exact_unit(self.model.time_unit, Some(&self.time_unit)),
            inputs: self.describe_bound_inputs(&bound)?, states, outputs, output_values,
            deferred_checks: vec!["Time-zero state domain and finite selected output evaluation".into(),
                "Independent scientific and dimensional qualification of the current model, inputs and engine".into()],
        };
        Ok(PreparedInitialValues {
            bound,
            indices,
            plan,
        })
    }
    /// Bind quantities and select time-zero readouts before administrations without evaluating outputs or a RHS.
    pub fn plan_initial_values(
        self: &Arc<Self>,
        request: &InitialValuesRequest,
    ) -> Result<InitialValuesPlan, Error> {
        Ok(self.prepare_initial_values(request)?.plan)
    }
    /// Evaluate outputs at the bound initial state. No doses, schedules, random draws or integration occur.
    pub fn initial_values(
        self: &Arc<Self>,
        request: &InitialValuesRequest,
    ) -> Result<InitialValuesResult, Error> {
        let PreparedInitialValues {
            bound,
            indices,
            plan,
        } = self.prepare_initial_values(request)?;
        let values = bound.outputs_selected(0.0, &bound.initial(), Some(&indices))?;
        Ok(InitialValuesResult {
            schema: ResultSchema::V01,
            request_id: request.request_id.clone(),
            identity: plan.prospective_identity,
            model_id: self.model_id.clone(),
            time: plan.time,
            time_unit: plan.time_unit,
            inputs: plan.inputs,
            states: plan.states,
            outputs: plan
                .outputs
                .into_iter()
                .zip(values)
                .map(|(output, value)| InitialOutput {
                    name: output.name,
                    unit: output.unit,
                    value,
                })
                .collect(),
        })
    }
}

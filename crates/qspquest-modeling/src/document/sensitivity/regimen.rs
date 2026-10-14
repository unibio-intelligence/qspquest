//! Unit-bearing regimen lowering and derivative tolerance binding.
use super::*;
pub(super) struct PreparedSensitivityRegimen {
    pub(super) model: ScheduledSensitivity,
    pub(super) protocol: crate::Protocol,
    pub(super) budgets: crate::Budgets,
    pub(super) atol: Vec<f64>,
    pub(super) projection: super::outputs::PreparedSensitivityProjection,
}
impl PreparedSensitivityRegimen {
    pub(super) fn result_rows(&self) -> usize {
        self.projection.len()
    }
}

impl CompiledSensitivityDocument {
    /// Tolerances must name every selected parameter and every primal state.
    /// Their units are state-unit / declared parameter-unit.
    pub fn simulate_regimen(
        self: &Arc<Self>,
        parameters: &BTreeMap<String, Quantity>,
        request: &qspquest_core::regimen::Request,
        derivative_absolute_tolerances: &BTreeMap<String, BTreeMap<String, Quantity>>,
        budgets: crate::Budgets,
    ) -> Result<Vec<SensitivitySample>, Error> {
        self.simulate_regimen_initial(
            parameters,
            request,
            derivative_absolute_tolerances,
            budgets,
            &BTreeMap::new(),
            qspquest_core::run::Solver::DiffsolBdf,
        )
    }
    pub(super) fn simulate_regimen_initial(
        self: &Arc<Self>,
        parameters: &BTreeMap<String, Quantity>,
        request: &qspquest_core::regimen::Request,
        derivative_absolute_tolerances: &BTreeMap<String, BTreeMap<String, Quantity>>,
        budgets: crate::Budgets,
        initial_states: &BTreeMap<String, Quantity>,
        solver: qspquest_core::run::Solver,
    ) -> Result<Vec<SensitivitySample>, Error> {
        let prepared = self.prepare_regimen_initial(
            parameters,
            request,
            derivative_absolute_tolerances,
            budgets,
            initial_states,
        )?;
        self.simulate_prepared_regimen(prepared, solver)
    }
    pub(super) fn prepare_regimen_initial(
        self: &Arc<Self>,
        parameters: &BTreeMap<String, Quantity>,
        request: &qspquest_core::regimen::Request,
        derivative_absolute_tolerances: &BTreeMap<String, BTreeMap<String, Quantity>>,
        budgets: crate::Budgets,
        initial_states: &BTreeMap<String, Quantity>,
    ) -> Result<PreparedSensitivityRegimen, Error> {
        let mut bound =
            self.bind_with_initial_states(parameters, &request.covariates, initial_states)?;
        let lagged =
            self.primal
                .expand_dose_lags(request, parameters, &bound.primal.covariate_values)?;
        let request = &lagged;
        let protocol = self.primal.lower_regimen_protocol(request, budgets)?;
        bound.primal.scheduled = true;
        for checkpoint in &request.checkpoints {
            let time = quantity(checkpoint)?.in_unit(self.primal.model.time_unit)?;
            if time < protocol.start || time > protocol.end {
                return Err(invalid("checkpoint outside sensitivity run window"));
            }
            bound.checkpoints.push(time);
        }
        bound.checkpoints.sort_by(f64::total_cmp);
        bound.checkpoints.dedup();
        if derivative_absolute_tolerances.len() != self.parameters().len()
            || derivative_absolute_tolerances
                .keys()
                .any(|p| !self.parameters().contains(p))
        {
            return Err(invalid(
                "derivative tolerances must name every selected parameter",
            ));
        }
        let n = self.primal.state_count;
        let mut atol = self
            .primal
            .absolute_tolerances(request)?
            .unwrap_or_else(|| vec![protocol.atol; n]);
        for (p, name) in self.parameters().iter().enumerate() {
            let states = &derivative_absolute_tolerances[name];
            if states.len() != n || states.keys().any(|s| !self.primal.state_names.contains(s)) {
                return Err(invalid("derivative tolerances must name every state"));
            }
            for (i, state) in self.primal.state_names.iter().enumerate() {
                let value = quantity(&states[state])?
                    .in_unit(self.forward.augmented_state_units[n + p * n + i])?;
                if value <= 0. {
                    return Err(invalid("derivative tolerance must be positive"));
                }
                atol.push(value);
            }
        }
        let (model, budgets) =
            ScheduledSensitivity::new(bound, parameters, request, &protocol, budgets)?;
        crate::prepare_simulation(&model, &protocol, budgets, Some(&atol))?;
        let coordinates = crate::simulation_rows(&model, &protocol);
        let projection = model.base.prepare_projection(
            &protocol,
            &coordinates,
            request.observations.as_deref(),
            budgets,
        )?;
        Ok(PreparedSensitivityRegimen {
            model,
            protocol,
            budgets,
            atol,
            projection,
        })
    }
    pub(super) fn simulate_prepared_regimen(
        self: &Arc<Self>,
        prepared: PreparedSensitivityRegimen,
        solver: qspquest_core::run::Solver,
    ) -> Result<Vec<SensitivitySample>, Error> {
        let PreparedSensitivityRegimen {
            model,
            protocol,
            budgets,
            atol,
            projection,
        } = prepared;
        let rows = crate::simulate_with_solver(&model, &protocol, budgets, Some(&atol), solver)?;
        model
            .base
            .project_prepared_rows(&protocol, rows, projection, &model.bindings)
    }
}

pub(super) struct ScheduledSensitivity {
    pub(super) base: BoundSensitivityDocument,
    pub(super) bindings: Vec<(f64, BoundSensitivityDocument)>,
    active: Cell<usize>,
}
impl ScheduledSensitivity {
    fn new(
        mut base: BoundSensitivityDocument,
        parameters: &BTreeMap<String, Quantity>,
        request: &qspquest_core::regimen::Request,
        protocol: &crate::Protocol,
        budgets: crate::Budgets,
    ) -> Result<(Self, crate::Budgets), Error> {
        let compiled = base.compiled.clone();
        let mut changes = Vec::new();
        for change in &request.covariate_changes {
            let definition = compiled
                .primal
                .covariates
                .iter()
                .find(|c| c.name == change.name)
                .ok_or_else(|| invalid("unknown sensitivity covariate"))?;
            if definition.interpolation != Interpolation::Step {
                return Err(invalid("constant covariate cannot have step changes"));
            }
            let time = quantity(&change.time)?.in_unit(compiled.primal.model.time_unit)?;
            if time < protocol.start || time > protocol.end {
                return Err(invalid("covariate change outside sensitivity run"));
            }
            changes.push((time, change));
        }
        changes.sort_by(|a, b| a.0.total_cmp(&b.0));
        let per_binding = compiled.primal.model.binding_storage_values()
            + compiled.forward.binding_storage_values()
            + compiled.readouts.binding_storage_values()
            + 2 * base.primal.values.len();
        let storage = changes
            .len()
            .checked_add(1)
            .and_then(|n| n.checked_mul(per_binding))
            .ok_or_else(|| invalid("sensitivity binding storage overflow"))?;
        let budgets = crate::Budgets {
            output_values: budgets.output_values.checked_sub(storage).ok_or_else(|| {
                Error::new(
                    ErrorCode::OutputBudget,
                    "sensitivity covariate bindings exceed storage budget",
                )
            })?,
            ..budgets
        };
        // These values only suppress re-evaluation of initial expressions when
        // binding later phases. The driver carries the live augmented state.
        let initial = base.primal.initial();
        let continuing = compiled
            .primal
            .state_names
            .iter()
            .zip(&compiled.primal.state_unit_names)
            .zip(initial)
            .map(|((name, unit), value)| {
                (
                    name.clone(),
                    Quantity {
                        value,
                        unit: unit.clone(),
                    },
                )
            })
            .collect();
        let mut covariates = base.primal.covariate_values.clone();
        let mut bindings = Vec::new();
        let mut cursor = 0;
        while cursor < changes.len() {
            let time = changes[cursor].0;
            let mut names = BTreeSet::new();
            while cursor < changes.len() && changes[cursor].0 == time {
                let change = changes[cursor].1;
                if !names.insert(&change.name) {
                    return Err(invalid(
                        "duplicate sensitivity covariate change at the same time",
                    ));
                }
                covariates.insert(change.name.clone(), change.value.clone());
                cursor += 1;
            }
            let bound = compiled
                .bind_with_initial_states(parameters, &covariates, &continuing)
                .map_err(|mut e| {
                    e.time = Some(time);
                    e
                })?;
            bindings.push((time, bound));
            base.checkpoints.push(time);
        }
        base.checkpoints.sort_by(f64::total_cmp);
        base.checkpoints.dedup();
        Ok((
            Self {
                base,
                bindings,
                active: Cell::new(0),
            },
            budgets,
        ))
    }
    fn binding(&self) -> &BoundSensitivityDocument {
        let i = self.active.get();
        if i == 0 {
            &self.base
        } else {
            &self.bindings[i - 1].1
        }
    }
}
impl Model for ScheduledSensitivity {
    fn rhs_depends_on_time(&self) -> bool {
        self.binding().rhs_depends_on_time()
    }
    fn validate(&self) -> Result<(), Error> {
        self.base.validate()
    }
    fn initial(&self) -> Vec<f64> {
        self.base.initial()
    }
    fn discontinuities(&self) -> &[f64] {
        &self.base.checkpoints
    }
    fn enter_boundary(&self, time: f64) -> Result<(), Error> {
        self.active
            .set(self.bindings.partition_point(|(t, _)| *t <= time));
        Ok(())
    }
    fn check_delivery(&self, event: &qspquest_core::Event) -> Result<(), Error> {
        self.base.check_delivery(event)
    }
    fn check_state(&self, time: f64, state: &[f64]) -> Result<(), Error> {
        self.binding().check_state(time, state)
    }
    fn dose_scale(&self, target: usize) -> Option<f64> {
        self.binding().dose_scale(target)
    }
    fn apply_bolus(
        &self,
        time: f64,
        target: usize,
        amount: f64,
        state: &mut [f64],
    ) -> Result<(), Error> {
        self.binding().apply_bolus(time, target, amount, state)
    }
    fn apply_reset(
        &self,
        time: f64,
        target: usize,
        value: f64,
        state: &mut [f64],
    ) -> Result<(), Error> {
        self.binding().apply_reset(time, target, value, state)
    }
    fn accumulate_infusion_rate(
        &self,
        time: f64,
        target: usize,
        rate: f64,
        rates: &mut [f64],
    ) -> Result<(), Error> {
        self.binding()
            .accumulate_infusion_rate(time, target, rate, rates)
    }
    fn rhs(&self, time: f64, state: &[f64], rates: &[f64], out: &mut [f64]) -> Result<(), Error> {
        self.binding().rhs(time, state, rates, out)
    }
    fn jac_mul(
        &self,
        time: f64,
        state: &[f64],
        rates: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        self.binding().jac_mul(time, state, rates, v, out)
    }
}

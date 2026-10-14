//! Strict simulation-document compiler. No tenant, registry, solver or job fields.
/// Maximum serialized JSON model size; request and text limits are separate.
pub const MODEL_DOCUMENT_BYTE_LIMIT: usize = 4_000_000;
mod definitions;
mod dose_switches;
mod dosing_lags;
mod history;
mod initial_values;
mod linear_pk;
mod morris;
mod planning;
mod scalar_fit;
mod scan;
pub mod sensitivity;
mod steady_state;
mod switches;
use crate::{
    compile::{Program, Symbol, SymbolKind},
    runtime::units as runtime,
    Error, ErrorCode, Model, ProtocolExt,
};
use qspquest_core::{model::*, units::Unit};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[derive(Debug)]
pub struct CompiledDocument {
    linear_pk: Option<linear_pk::Binding>,
    model_content_hash: String,
    model_id: String,
    description: String,
    time_unit: String,
    model: Arc<runtime::UnitCompiledModel>,
    parameters: Vec<Parameter>,
    covariates: Vec<Covariate>,
    individual: Vec<(String, Unit, Program)>,
    individual_unit_names: BTreeMap<String, String>,
    outputs: Vec<Program>,
    output_names: Vec<String>,
    output_units: Vec<String>,
    nonnegative_states: Vec<usize>,
    state_count: usize,
    state_names: Vec<String>,
    state_unit_names: Vec<String>,
    dose_targets: Vec<qspquest_core::regimen::Target>,
    dose_unit_names: BTreeMap<usize, String>,
    dose_lags: Vec<(String, Program)>,
    history: Vec<history::Binding>,
    dose_switches: Vec<dose_switches::Binding>,
    switches: Vec<(String, Program, Option<qspquest_core::expression::Compare>)>,
}
#[derive(Debug)]
pub struct BoundDocument {
    compiled: Arc<CompiledDocument>,
    problem: runtime::UnitBoundProblem,
    values: Vec<f64>,
    parameter_overrides: BTreeMap<String, Quantity>,
    covariate_values: BTreeMap<String, Quantity>,
    initial_overrides: BTreeMap<String, Quantity>,
    scheduled: bool,
}
struct StepModel {
    selected_outputs: Option<Vec<usize>>,
    analytic: Option<crate::linear_pk::LinearPk>,
    base: BoundDocument,
    times: Vec<f64>,
    boundaries: Vec<f64>,
    bindings: Vec<BoundDocument>,
    active: Cell<usize>,
    predicate_points: Vec<(String, f64, f64)>,
    dose_predicates: Vec<dose_switches::BoundPredicate>,
}
struct PreparedRegimen {
    model: StepModel,
    protocol: crate::Protocol,
    budgets: crate::Budgets,
    absolute_tolerances: Option<Vec<f64>>,
    observations: Option<Vec<qspquest_core::regimen::ObservationPoint>>,
    result_rows: usize,
    observation_indices: Option<Vec<usize>>,
}
struct PreparedRun {
    bound: BoundDocument,
    regimen: PreparedRegimen,
    indices: Option<Vec<usize>>,
}
impl StepModel {
    fn binding(&self, index: usize) -> &BoundDocument {
        if index == 0 {
            &self.base
        } else {
            &self.bindings[index - 1]
        }
    }
    fn outputs(&self, time: f64, side: &str, state: &[f64]) -> Result<Vec<f64>, Error> {
        let index = self
            .times
            .partition_point(|t| *t < time || (*t == time && side == "post"));
        let bound = self.binding(index);
        let overrides = self.readout_covariate_overrides(time, side)?;
        if overrides.is_empty() {
            return bound.outputs_selected(time, state, self.selected_outputs.as_deref());
        }
        let mut values = bound.covariate_values.clone();
        values.extend(overrides);
        // Pointwise readout uses the predicate at t, while integration retains
        // one-sided bindings. State selection still follows the requested side.
        bound
            .compiled
            .bind_continuing(&bound.parameter_overrides, &values, Some(state))?
            .outputs_selected(time, state, self.selected_outputs.as_deref())
    }
    fn readout_covariate_overrides(
        &self,
        time: f64,
        side: &str,
    ) -> Result<BTreeMap<String, Quantity>, Error> {
        let exact: Vec<_> = self
            .predicate_points
            .iter()
            .filter(|(_, t, _)| *t == time)
            .collect();
        let mut values = BTreeMap::new();
        for (name, _, value) in exact {
            values.insert(
                name.clone(),
                Quantity {
                    value: *value,
                    unit: "1".into(),
                },
            );
        }
        for predicate in &self.dose_predicates {
            predicate.readout(time, side, &mut values)?;
        }
        Ok(values)
    }
}
impl Model for StepModel {
    fn jacobian_sparsity(&self) -> Option<Vec<(usize, usize)>> {
        self.base.jacobian_sparsity()
    }
    fn rhs_depends_on_time(&self) -> bool {
        self.binding(self.active.get()).rhs_depends_on_time()
    }
    fn interval_propagator(&self) -> Option<&dyn crate::IntervalPropagator> {
        self.analytic
            .as_ref()
            .map(|p| p as &dyn crate::IntervalPropagator)
    }
    fn check_delivery(&self, event: &qspquest_core::Event) -> Result<(), Error> {
        self.base.check_delivery(event)
    }
    fn check_state(&self, time: f64, state: &[f64]) -> Result<(), Error> {
        self.binding(self.active.get()).check_state(time, state)
    }
    fn validate(&self) -> Result<(), Error> {
        self.base.validate()
    }
    fn initial(&self) -> Vec<f64> {
        self.base.initial()
    }
    fn dose_scale(&self, state: usize) -> Option<f64> {
        self.binding(self.active.get()).dose_scale(state)
    }
    fn discontinuities(&self) -> &[f64] {
        &self.boundaries
    }
    fn rhs_jump_times(&self) -> &[f64] {
        &self.times
    }
    fn enter_boundary(&self, time: f64) -> Result<(), Error> {
        self.active.set(self.times.partition_point(|t| *t <= time));
        Ok(())
    }
    fn rhs(&self, time: f64, x: &[f64], rates: &[f64], out: &mut [f64]) -> Result<(), Error> {
        self.binding(self.active.get()).rhs(time, x, rates, out)
    }
    fn jac_mul(
        &self,
        time: f64,
        x: &[f64],
        rates: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        self.binding(self.active.get())
            .jac_mul(time, x, rates, v, out)
    }
}
fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidInput, message)
}
fn quantity(q: &Quantity) -> Result<runtime::Quantity, Error> {
    Ok(runtime::Quantity {
        value: q.value,
        unit: Unit::parse(&q.unit)?,
    })
}
fn quantity_in_declared_unit(q: &Quantity, declared: &str) -> Result<f64, Error> {
    quantity(q).and_then(|value| value.in_unit(Unit::parse(declared)?)).map_err(|mut error| {
        if matches!(error.code, ErrorCode::Unit | ErrorCode::UnknownUnit) {
            let label = |unit: &str| {
                let mut characters = unit.chars();
                let prefix: String = characters.by_ref().take(256).collect();
                if characters.next().is_some() { format!("{prefix}…") } else { prefix }
            };
            error.message = format!("requested unit {:?}; declared unit {:?}: {}", label(&q.unit), label(declared), error.message);
            if declared == "1" {
                error.message.push_str(" This field uses declared dimensionless/source coordinates; physical source normalization is not inferred.");
            }
        }
        error
    })
}
fn identifier(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "true" | "false")
        && name.len() <= 128
        && name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_lowercase() || (i > 0 && b.is_ascii_digit()))
}
fn domain(parameter: &Parameter, value: f64) -> Result<(), Error> {
    let valid = match parameter.transform {
        Transform::Identity => true,
        Transform::Log => value > 0.0,
        Transform::Logit | Transform::Probit => {
            let fraction = Unit::parse(&parameter.default.unit)?.convert(value, Unit::ONE)?;
            fraction > 0.0 && fraction < 1.0
        }
    };
    if !value.is_finite() || !valid {
        return Err(invalid(format!(
            "parameter {} is outside its transform domain",
            parameter.name
        )));
    }
    Ok(())
}
impl CompiledDocument {
    pub fn from_json(json: &str) -> Result<Arc<Self>, Error> {
        if json.len() > MODEL_DOCUMENT_BYTE_LIMIT {
            return Err(invalid("model document exceeds four million bytes"));
        }
        let document: ModelDocument =
            serde_json::from_str(json).map_err(|e| invalid(format!("model document: {e}")))?;
        Self::compile(&document)
    }
    pub fn compile(document: &ModelDocument) -> Result<Arc<Self>, Error> {
        crate::capabilities::validate_requirements(&document.requirements)?;
        if document.linear_pk.is_some() {
            return Self::compile_linear_pk(document);
        }
        if !document.dose_history.is_empty() {
            return Self::compile_history(document);
        }
        if !document.dose_switches.is_empty() {
            return Self::compile_dose_switches(document);
        }
        if !document.switches.is_empty() {
            return Self::compile_switches(document);
        }
        if !identifier(&document.model_id) {
            return Err(invalid("invalid model_id"));
        }
        if document
            .parameters
            .len()
            .checked_add(document.individual.len())
            .and_then(|n| n.checked_add(document.covariates.len()))
            .is_none_or(|n| n > 4096)
        {
            return Err(invalid("too many parameter bindings"));
        }
        if document.outputs.is_empty() || document.outputs.len() > 1024 {
            return Err(invalid("document requires 1..1024 outputs"));
        }
        let mut names = BTreeSet::from(["time".to_string()]);
        for name in document
            .parameters
            .iter()
            .map(|p| &p.name)
            .chain(document.covariates.iter().map(|p| &p.name))
            .chain(document.individual.iter().map(|p| &p.name))
            .chain(document.definitions.iter().map(|p| &p.name))
            .chain(document.states.iter().map(|s| &s.name))
        {
            if !identifier(name) || name.starts_with("input_") || !names.insert(name.clone()) {
                return Err(invalid(format!(
                    "invalid, reserved or duplicate symbol: {name}"
                )));
            }
        }
        // Outputs are result-column labels, not expression bindings. A column
        // may report a state/parameter under its own name without shadowing it.
        let mut output_names = BTreeSet::new();
        for output in &document.outputs {
            if !identifier(&output.name) || !output_names.insert(&output.name) {
                return Err(invalid(format!(
                    "invalid or duplicate output name: {}",
                    output.name
                )));
            }
        }
        let mut definition = runtime::UnitModelDefinition {
            time_unit: Unit::parse(&document.time_unit)?,
            bindings: document
                .parameters
                .iter()
                .map(|p| {
                    Ok(runtime::BindingDefinition {
                        name: p.name.clone(),
                        unit: Unit::parse(&p.default.unit)?,
                        bounds: p
                            .bounds
                            .as_ref()
                            .map(|[lo, hi]| Ok::<_, Error>((quantity(lo)?, quantity(hi)?)))
                            .transpose()?,
                    })
                })
                .collect::<Result<_, Error>>()?,
            states: document
                .states
                .iter()
                .map(|s| {
                    Ok(runtime::UnitStateDefinition {
                        name: s.name.clone(),
                        unit: Unit::parse(&s.unit)?,
                        initial: match &s.initial {
                            Initial::Quantity { quantity: q } => {
                                runtime::InitialCondition::Quantity(quantity(q)?)
                            }
                            Initial::Expression { expression } => {
                                runtime::InitialCondition::Expression(expression.clone())
                            }
                        },
                        rhs: s.rhs.clone(),
                        dosing: s
                            .dosing
                            .as_ref()
                            .map(|d| {
                                Ok::<_, Error>((Unit::parse(&d.amount_unit)?, d.scale.clone()))
                            })
                            .transpose()?,
                    })
                })
                .collect::<Result<_, Error>>()?,
        };
        for c in &document.covariates {
            let unit = Unit::parse(&c.unit)?;
            if let Some(default) = &c.default {
                quantity(default)?.in_unit(unit)?;
            }
            definition.bindings.push(runtime::BindingDefinition {
                name: c.name.clone(),
                unit,
                bounds: None,
            });
        }
        for parameter in &document.parameters {
            domain(parameter, parameter.default.value)?;
        }
        let mut symbols: Vec<_> = definition
            .bindings
            .iter()
            .map(|b| {
                (
                    Symbol {
                        name: b.name.clone(),
                        kind: SymbolKind::Parameter,
                    },
                    b.unit,
                )
            })
            .collect();
        let mut pending: Vec<_> = document.individual.iter().collect();
        let mut individual = Vec::new();
        let all_names: std::collections::BTreeSet<_> = document
            .parameters
            .iter()
            .map(|p| p.name.as_str())
            .chain(document.covariates.iter().map(|p| p.name.as_str()))
            .chain(document.individual.iter().map(|p| p.name.as_str()))
            .collect();
        let all_symbols: Vec<_> = all_names
            .iter()
            .map(|name| Symbol {
                name: (*name).into(),
                kind: SymbolKind::Parameter,
            })
            .collect();
        // Validate shape/depth and unknown references before walking dependency trees.
        let shape_symbols = crate::compile::SymbolTable::new(&all_symbols)?;
        for p in &pending {
            Program::check_unit_shape(&p.expression, &shape_symbols, 256).map_err(|mut e| {
                e.expression = Some(format!(
                    "individual.{}.{}",
                    p.name,
                    e.expression.unwrap_or_default()
                ));
                e
            })?;
        }
        fn references(
            expr: &qspquest_core::expression::Expr,
            available: &std::collections::BTreeSet<String>,
        ) -> bool {
            if let qspquest_core::expression::Expr::Symbol { name } = expr {
                return available.contains(name);
            }
            expr.children().iter().all(|e| references(e, available))
        }
        let mut instructions = 0usize;
        while !pending.is_empty() {
            let available = symbols.iter().map(|(s, _)| s.name.clone()).collect();
            let index = pending
                .iter()
                .position(|p| references(&p.expression, &available))
                .ok_or_else(|| invalid("cyclic individual-parameter dependencies"))?;
            let p = pending.remove(index);
            let compile = || -> Result<_, Error> {
                let unit = match &p.unit {
                    Some(code) => Unit::parse(code)?,
                    None => crate::compile::units::infer(&p.expression, &symbols)?,
                };
                Ok((
                    unit,
                    Program::compile_with_units(&p.expression, &symbols, unit)?,
                ))
            };
            let (unit, program) = compile().map_err(|mut e| {
                e.expression = Some(format!(
                    "individual.{}.{}",
                    p.name,
                    e.expression.unwrap_or_default()
                ));
                e
            })?;
            instructions += program.instruction_count();
            if instructions > 1_000_000 {
                return Err(invalid("individual parameters exceed instruction budget"));
            }
            symbols.push((
                Symbol {
                    name: p.name.clone(),
                    kind: SymbolKind::Parameter,
                },
                unit,
            ));
            definition.bindings.push(runtime::BindingDefinition {
                name: p.name.clone(),
                unit,
                bounds: None,
            });
            individual.push((p.name.clone(), unit, program));
        }
        let mut expanded = if document.definitions.is_empty() {
            None
        } else {
            let mut all_symbols = symbols.clone();
            all_symbols.extend(definition.states.iter().map(|s| {
                (
                    Symbol {
                        name: s.name.clone(),
                        kind: SymbolKind::State,
                    },
                    s.unit,
                )
            }));
            all_symbols.push((
                Symbol {
                    name: "time".into(),
                    kind: SymbolKind::Time,
                },
                definition.time_unit,
            ));
            for state in &definition.states {
                all_symbols.push((
                    Symbol {
                        name: format!("input_{}", state.name),
                        kind: SymbolKind::Input,
                    },
                    state.unit.divide(definition.time_unit)?,
                ));
            }
            Some(definitions::Expanded::new(
                &document.definitions,
                &all_symbols,
            )?)
        };
        if let Some(expanded) = &mut expanded {
            for state in &mut definition.states {
                state.rhs = expanded.expand(&state.rhs, &format!("states.{}.rhs", state.name))?;
                if let runtime::InitialCondition::Expression(expr) = &mut state.initial {
                    *expr = expanded.expand(expr, &format!("states.{}.initial", state.name))?;
                }
                if let Some((_, expr)) = &mut state.dosing {
                    *expr = expanded.expand(expr, &format!("states.{}.dosing", state.name))?;
                }
            }
        }
        let mut lag_symbols = document
            .parameters
            .iter()
            .map(|p| {
                Ok((
                    Symbol {
                        name: p.name.clone(),
                        kind: SymbolKind::Parameter,
                    },
                    Unit::parse(&p.default.unit)?,
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        for c in document
            .covariates
            .iter()
            .filter(|c| c.interpolation == Interpolation::Constant)
        {
            lag_symbols.push((
                Symbol {
                    name: c.name.clone(),
                    kind: SymbolKind::Parameter,
                },
                Unit::parse(&c.unit)?,
            ));
        }
        let mut dose_lags = Vec::new();
        for state in &document.states {
            if let Some(expr) = state.dosing.as_ref().and_then(|d| d.lag.as_ref()) {
                let path = format!("states.{}.dosing.lag", state.name);
                let expr = match &mut expanded {
                    Some(expanded) => expanded.expand(expr, &path)?,
                    None => expr.clone(),
                };
                let program =
                    Program::compile_with_units(&expr, &lag_symbols, definition.time_unit)
                        .map_err(|mut e| {
                            e.expression = Some(path);
                            e
                        })?;
                dose_lags.push((state.name.clone(), program));
            }
        }
        let model = if expanded.is_some() {
            runtime::UnitCompiledModel::compile_expanded(&definition)?
        } else {
            runtime::UnitCompiledModel::compile(&definition)?
        };
        let mut symbols: Vec<_> = definition
            .bindings
            .iter()
            .map(|b| {
                (
                    Symbol {
                        name: b.name.clone(),
                        kind: SymbolKind::Parameter,
                    },
                    b.unit,
                )
            })
            .collect();
        symbols.extend(definition.states.iter().map(|s| {
            (
                Symbol {
                    name: s.name.clone(),
                    kind: SymbolKind::State,
                },
                s.unit,
            )
        }));
        symbols.push((
            Symbol {
                name: "time".into(),
                kind: SymbolKind::Time,
            },
            definition.time_unit,
        ));
        let readout_symbols = crate::compile::readout_symbols(&symbols);
        let readout_context = crate::compile::units::CompileContext::new(&readout_symbols)?;
        let outputs = document
            .outputs
            .iter()
            .map(|o| {
                let mut compile = || {
                    if let Some(expanded) = &mut expanded {
                        let expr =
                            expanded.expand(&o.expression, &format!("outputs.{}", o.name))?;
                        readout_context.compile(&expr, Unit::parse(&o.unit)?, true)
                    } else {
                        readout_context.compile(&o.expression, Unit::parse(&o.unit)?, false)
                    }
                };
                compile().map_err(|mut e| {
                    e.expression = Some(format!(
                        "outputs.{}.{}",
                        o.name,
                        e.expression.unwrap_or_default()
                    ));
                    e
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if outputs.iter().map(|p| p.instruction_count()).sum::<usize>() > 1_000_000 {
            return Err(invalid("outputs exceed instruction budget"));
        }
        let mut nonnegative_states = BTreeSet::new();
        for invariant in &document.invariants {
            match invariant {
                Invariant::Nonnegative { states } => {
                    if states.is_empty() {
                        return Err(invalid("nonnegative invariant requires states"));
                    }
                    for name in states {
                        let index = document
                            .states
                            .iter()
                            .position(|s| s.name == *name)
                            .ok_or_else(|| invalid(format!("unknown invariant state: {name}")))?;
                        if !nonnegative_states.insert(index) {
                            return Err(invalid(format!(
                                "duplicate nonnegative invariant state: {name}"
                            )));
                        }
                    }
                }
            }
        }
        let compiled = Arc::new(Self {
            linear_pk: None,
            switches: vec![],
            history: vec![],
            dose_switches: vec![],
            dose_lags,
            nonnegative_states: nonnegative_states.into_iter().collect(),
            model_content_hash: qspquest_core::identity::canonical_hash(
                &serde_json::to_value(document).map_err(|e| invalid(e.to_string()))?,
            )?,
            model_id: document.model_id.clone(),
            description: document.description.clone(),
            time_unit: document.time_unit.clone(),
            model,
            parameters: document.parameters.clone(),
            covariates: document.covariates.clone(),
            individual,
            individual_unit_names: document
                .individual
                .iter()
                .filter_map(|p| p.unit.as_ref().map(|unit| (p.name.clone(), unit.clone())))
                .collect(),
            outputs,
            output_names: document.outputs.iter().map(|o| o.name.clone()).collect(),
            output_units: document.outputs.iter().map(|o| o.unit.clone()).collect(),
            state_count: document.states.len(),
            state_names: document.states.iter().map(|s| s.name.clone()).collect(),
            state_unit_names: document.states.iter().map(|s| s.unit.clone()).collect(),
            dose_unit_names: document
                .states
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.dosing.as_ref().map(|d| (i, d.amount_unit.clone())))
                .collect(),
            dose_targets: document
                .states
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.dosing.as_ref().map(|d| (i, s, d)))
                .map(|(state, s, d)| {
                    qspquest_core::model::validate_delivery_modes(&d.modes)?;
                    Ok(qspquest_core::regimen::Target {
                        modes: d.modes.clone(),
                        name: s.name.clone(),
                        state,
                        amount_unit: Unit::parse(&d.amount_unit)?,
                    })
                })
                .collect::<Result<_, Error>>()?,
        });
        if document.covariates.iter().all(|c| c.default.is_some()) {
            compiled.bind(&BTreeMap::new())?;
        }
        Ok(compiled)
    }
    pub fn model_content_hash(&self) -> &str {
        &self.model_content_hash
    }
    pub fn execute(
        self: &Arc<Self>,
        request: &qspquest_core::run::RunRequest,
    ) -> Result<qspquest_core::run::RunResult, Error> {
        self.execute_selected(request, None)
    }
    /// Evaluate only requested readouts; all dynamics, states and physical checks remain active.
    /// Selection order is retained and included in the scientific request identity.
    pub fn execute_selected(
        self: &Arc<Self>,
        request: &qspquest_core::run::RunRequest,
        selected: Option<&[String]>,
    ) -> Result<qspquest_core::run::RunResult, Error> {
        let prepared = self.prepare_selected(request, selected)?;
        self.execute_prepared(request, selected, prepared)
    }
    fn execute_prepared(
        self: &Arc<Self>,
        request: &qspquest_core::run::RunRequest,
        selected: Option<&[String]>,
        prepared: PreparedRun,
    ) -> Result<qspquest_core::run::RunResult, Error> {
        use qspquest_core::run::*;
        let identity = self.prospective_identity(request, selected, &prepared.bound)?;
        let PreparedRun {
            bound,
            regimen,
            indices,
        } = prepared;
        let rows = bound.simulate_prepared_regimen(
            regimen,
            request.solver,
            request.accuracy_control == qspquest_core::run::AccuracyControl::Sampled,
        )?;
        Ok(RunResult {
            schema: ResultSchema::V01,
            request_id: request.request_id.clone(),
            identity,
            model_id: self.model_id.clone(),
            description: self.description.clone(),
            time_unit: self.time_unit.clone(),
            times: rows.iter().map(|r| r.time).collect(),
            sides: rows
                .iter()
                .map(|r| {
                    if r.side == "pre" {
                        qspquest_core::regimen::ObservationSide::Pre
                    } else {
                        qspquest_core::regimen::ObservationSide::Post
                    }
                })
                .collect(),
            outputs: indices
                .unwrap_or_else(|| (0..self.outputs.len()).collect())
                .into_iter()
                .enumerate()
                .map(|(i, j)| ResultColumn {
                    name: self.output_names[j].clone(),
                    unit: self.output_units[j].clone(),
                    values: rows.iter().map(|r| r.values[i]).collect(),
                })
                .collect(),
        })
    }
    fn prospective_identity(
        &self,
        request: &qspquest_core::run::RunRequest,
        selected: Option<&[String]>,
        bound: &BoundDocument,
    ) -> Result<qspquest_core::run::ExecutionIdentity, Error> {
        use qspquest_core::{identity::canonical_hash, run::*};
        // Transport correlation IDs and redundant override spelling do not define the scientific run.
        let mut identity_request = serde_json::json!({"schema":request.schema,"regimen":request.regimen,"solver":request.solver,"budgets":request.budgets,"seed":request.seed});
        if !request.accuracy_control.is_local() {
            identity_request["accuracy_control"] = serde_json::json!(request.accuracy_control);
        }
        if let Some(selected) = selected {
            identity_request["outputs"] = serde_json::json!(selected);
        }
        let run_request_hash = canonical_hash(&identity_request)?;
        self.bound_execution_identity(
            bound,
            run_request_hash,
            match request.solver {
                Solver::AnalyticLinearPk => "qspquest",
                _ => "diffsol",
            },
            match request.solver {
                Solver::AnalyticLinearPk => env!("CARGO_PKG_VERSION"),
                _ => crate::DIFFSOL_BACKEND_VERSION,
            },
            match request.solver {
                Solver::DiffsolBdf => "bdf",
                Solver::DiffsolTsit45 => "tsit45",
                Solver::DiffsolEsdirk34 => "esdirk34",
                Solver::DiffsolTrBdf2 => "tr_bdf2",
                Solver::DiffsolRosenbrock23 => "rosenbrock23",
                Solver::DiffsolRodas5p => "rodas5p",
                Solver::AnalyticLinearPk => "linear_pk_exponential_taylor20",
            },
        )
    }
    fn bound_execution_identity(
        &self,
        bound: &BoundDocument,
        run_request_hash: String,
        backend: &str,
        backend_version: &str,
        algorithm: &str,
    ) -> Result<qspquest_core::run::ExecutionIdentity, Error> {
        use qspquest_core::{
            identity::{canonical_hash, SPEC_VERSION},
            run::ExecutionIdentity,
        };
        let bound_value_hash = canonical_hash(
            &serde_json::json!({"values":bound.values,"initial_states":bound.initial()}),
        )?;
        Ok(ExecutionIdentity {
            model_content_hash: self.model_content_hash.clone(),
            spec_version: SPEC_VERSION.into(),
            compiler_version: concat!("qspquest/", env!("CARGO_PKG_VERSION")).into(),
            compiler_source_sha256: env!("QSPQUEST_SOURCE_SHA256").into(),
            rustc_version: env!("QSPQUEST_RUSTC_VERSION").into(),
            target: env!("QSPQUEST_TARGET").into(),
            build_profile: env!("QSPQUEST_PROFILE").into(),
            rustflags_sha256: env!("QSPQUEST_RUSTFLAGS_SHA256").into(),
            bound_value_hash,
            run_request_hash,
            backend: backend.into(),
            backend_version: backend_version.into(),
            algorithm: algorithm.into(),
        })
    }
    fn prepare_selected(
        self: &Arc<Self>,
        request: &qspquest_core::run::RunRequest,
        selected: Option<&[String]>,
    ) -> Result<PreparedRun, Error> {
        use qspquest_core::run::Solver;
        let indices = selected
            .map(|names| {
                if names.is_empty() {
                    return Err(invalid("select at least one output"));
                }
                let mut seen = BTreeSet::new();
                names
                    .iter()
                    .map(|name| {
                        if !seen.insert(name) {
                            return Err(invalid("duplicate selected output"));
                        }
                        self.output_names
                            .iter()
                            .position(|n| n == name)
                            .ok_or_else(|| invalid(format!("unknown output: {name}")))
                    })
                    .collect::<Result<Vec<_>, Error>>()
            })
            .transpose()?;
        if request
            .request_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 128)
        {
            return Err(invalid("request_id must contain 1..128 bytes"));
        }
        if request.seed.is_some() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "seeded stochastic execution is not implemented",
            ));
        }
        if request.budgets.solver_callbacks > u32::MAX as u64
            || request.budgets.events > u32::MAX as usize
            || request.budgets.output_values > u32::MAX as usize
        {
            return Err(invalid(
                "budgets must fit the shared native/WASM unsigned 32-bit range",
            ));
        }
        self.reject_history_overrides(
            request.regimen.covariates.keys().map(String::as_str).chain(
                request
                    .regimen
                    .covariate_changes
                    .iter()
                    .map(|c| c.name.as_str()),
            ),
        )?;
        let regimen =
            self.expand_switches(&request.regimen, &request.parameters, request.budgets)?;
        let mut bound = self.bind_initial_states_internal(
            &request.parameters,
            &regimen.covariates,
            &request.initial_states,
        )?;
        bound.scheduled = true;
        let analytic = match request.solver {
            Solver::DiffsolBdf
            | Solver::DiffsolTsit45
            | Solver::DiffsolEsdirk34
            | Solver::DiffsolTrBdf2
            | Solver::DiffsolRosenbrock23
            | Solver::DiffsolRodas5p => None,
            Solver::AnalyticLinearPk => Some(
                self.linear_pk
                    .as_ref()
                    .ok_or_else(|| {
                        Error::new(
                            ErrorCode::Unsupported,
                            "analytic_linear_pk requires a linear_pk declaration",
                        )
                    })?
                    .bind(&bound.values)?,
            ),
        };
        let regimen =
            bound.prepare_regimen(&regimen, request.budgets, analytic, indices.as_deref())?;
        Ok(PreparedRun {
            bound,
            regimen,
            indices,
        })
    }
    /// Overrides are complete quantities; omitted parameters use declared defaults.
    /// `fixed` controls future estimation, not a scientist's simulation edits.
    pub fn bind(
        self: &Arc<Self>,
        overrides: &BTreeMap<String, Quantity>,
    ) -> Result<BoundDocument, Error> {
        self.bind_with_covariates(overrides, &BTreeMap::new())
    }
    pub fn bind_with_covariates(
        self: &Arc<Self>,
        overrides: &BTreeMap<String, Quantity>,
        covariates: &BTreeMap<String, Quantity>,
    ) -> Result<BoundDocument, Error> {
        self.reject_switch_overrides(covariates.keys().map(String::as_str))?;
        self.reject_history_overrides(covariates.keys().map(String::as_str))?;
        self.bind_continuing(overrides, covariates, None)
    }
    /// Override selected initial states in declared state units. Unspecified
    /// states retain their parameter-dependent initial expressions.
    pub fn bind_with_initial_states(
        self: &Arc<Self>,
        overrides: &BTreeMap<String, Quantity>,
        covariates: &BTreeMap<String, Quantity>,
        initial_overrides: &BTreeMap<String, Quantity>,
    ) -> Result<BoundDocument, Error> {
        self.reject_switch_overrides(covariates.keys().map(String::as_str))?;
        self.reject_history_overrides(covariates.keys().map(String::as_str))?;
        self.bind_initial_states_internal(overrides, covariates, initial_overrides)
    }
    fn bind_initial_states_internal(
        self: &Arc<Self>,
        overrides: &BTreeMap<String, Quantity>,
        covariates: &BTreeMap<String, Quantity>,
        initial_overrides: &BTreeMap<String, Quantity>,
    ) -> Result<BoundDocument, Error> {
        let mut initial = vec![None; self.state_count];
        for (name, value) in initial_overrides {
            let index = self
                .state_names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| invalid(format!("unknown initial state: {name}")))?;
            initial[index] = Some(
                quantity(value)?
                    .in_unit(self.model.state_units[index])
                    .map_err(|mut e| {
                        e.expression = Some(format!("initial_states.{name}"));
                        e
                    })?,
            );
        }
        self.bind_initial(overrides, covariates, Some(&initial), initial_overrides)
    }
    fn bind_continuing(
        self: &Arc<Self>,
        overrides: &BTreeMap<String, Quantity>,
        covariates: &BTreeMap<String, Quantity>,
        continuing: Option<&[f64]>,
    ) -> Result<BoundDocument, Error> {
        let initial = continuing.map(|v| v.iter().copied().map(Some).collect::<Vec<_>>());
        self.bind_initial(overrides, covariates, initial.as_deref(), &BTreeMap::new())
    }
    fn bind_initial(
        self: &Arc<Self>,
        overrides: &BTreeMap<String, Quantity>,
        covariates: &BTreeMap<String, Quantity>,
        initial: Option<&[Option<f64>]>,
        initial_overrides: &BTreeMap<String, Quantity>,
    ) -> Result<BoundDocument, Error> {
        for key in covariates.keys() {
            if !self.covariates.iter().any(|c| c.name == *key) {
                return Err(invalid(format!("unknown covariate: {key}")));
            }
        }
        for key in overrides.keys() {
            if !self.parameters.iter().any(|p| p.name == *key) {
                return Err(invalid(format!("unknown parameter override: {key}")));
            }
        }
        let mut quantities = self
            .parameters
            .iter()
            .map(|p| {
                let value = (|| {
                    let value = quantity_in_declared_unit(
                        overrides.get(&p.name).unwrap_or(&p.default),
                        &p.default.unit,
                    )?;
                    domain(p, value)?;
                    Ok::<_, Error>(value)
                })()
                .map_err(|mut e| {
                    if e.expression.is_none() {
                        e.expression = Some(format!("parameters.{}", p.name));
                    }
                    e
                })?;
                Ok(runtime::Quantity {
                    value,
                    unit: Unit::parse(&p.default.unit)?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut covariate_values = BTreeMap::new();
        for c in &self.covariates {
            let q = covariates
                .get(&c.name)
                .or(c.default.as_ref())
                .ok_or_else(|| invalid(format!("missing covariate: {}", c.name)))?;
            let unit = Unit::parse(&c.unit)?;
            let value = quantity_in_declared_unit(q, &c.unit).map_err(|mut e| {
                if e.expression.is_none() {
                    e.expression = Some(format!("regimen.covariates.{}", c.name));
                }
                e
            })?;
            quantities.push(runtime::Quantity { value, unit });
            covariate_values.insert(
                c.name.clone(),
                Quantity {
                    value,
                    unit: c.unit.clone(),
                },
            );
        }
        let mut values: Vec<_> = quantities.iter().map(|q| q.value).collect();
        for (name, unit, program) in &self.individual {
            let value = program.evaluate(&values).map_err(|mut e| {
                e.expression = Some(format!(
                    "individual.{}.{}",
                    name,
                    e.expression.unwrap_or_default()
                ));
                e
            })?;
            values.push(value);
            quantities.push(runtime::Quantity { value, unit: *unit });
        }
        if let Some(pk) = &self.linear_pk {
            pk.bind(&values)?;
        }
        let problem = self.model.bind_initial(&quantities, initial)?;
        Ok(BoundDocument {
            compiled: self.clone(),
            problem,
            values: quantities.iter().map(|q| q.value).collect(),
            parameter_overrides: overrides.clone(),
            covariate_values,
            initial_overrides: initial_overrides.clone(),
            scheduled: false,
        })
    }
    pub fn expand_regimen(
        &self,
        administrations: &[qspquest_core::regimen::Administration],
        end: f64,
        event_limit: usize,
    ) -> Result<Vec<crate::Event>, Error> {
        if !self.dose_lags.is_empty() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "model lag requires a bound regimen",
            ));
        }
        qspquest_core::regimen::expand(
            administrations,
            &self.dose_targets,
            self.model.time_unit,
            end,
            event_limit,
        )
    }
    pub fn regimen_protocol(
        &self,
        request: &qspquest_core::regimen::Request,
        budgets: crate::Budgets,
    ) -> Result<crate::Protocol, Error> {
        if !self.dose_lags.is_empty()
            || !self.history.is_empty()
            || !self.dose_switches.is_empty()
            || !self.switches.is_empty()
            || !request.covariates.is_empty()
            || !request.covariate_changes.is_empty()
            || request.absolute_tolerances.is_some()
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "covariates and per-state tolerances require simulate_regimen; raw protocols cannot encode them",
            ));
        }
        self.lower_regimen_protocol(request, budgets)
    }
    fn absolute_tolerances(
        &self,
        request: &qspquest_core::regimen::Request,
    ) -> Result<Option<Vec<f64>>, Error> {
        match (&request.atol, &request.absolute_tolerances) {
            (Some(value), None) if value.is_finite() && *value > 0.0 => Ok(None),
            (None, Some(values)) => {
                if values.len() != self.state_names.len()
                    || values.keys().any(|name| !self.state_names.contains(name))
                {
                    return Err(invalid(
                        "absolute tolerances must name every state exactly once",
                    ));
                }
                self.state_names
                    .iter()
                    .zip(&self.model.state_units)
                    .map(|(name, unit)| {
                        let value = quantity(&values[name])?.in_unit(*unit)?;
                        if value <= 0.0 {
                            return Err(invalid(format!(
                                "absolute tolerance for {name} must be positive"
                            )));
                        }
                        Ok(value)
                    })
                    .collect::<Result<Vec<_>, Error>>()
                    .map(Some)
            }
            _ => Err(invalid(
                "provide exactly one positive scalar atol or complete absolute_tolerances map",
            )),
        }
    }
    fn lower_regimen_protocol(
        &self,
        request: &qspquest_core::regimen::Request,
        budgets: crate::Budgets,
    ) -> Result<crate::Protocol, Error> {
        self.lower_regimen_protocol_internal(request, budgets, false)
    }
    fn lower_regimen_protocol_internal(
        &self,
        request: &qspquest_core::regimen::Request,
        budgets: crate::Budgets,
        periodic: bool,
    ) -> Result<crate::Protocol, Error> {
        if request.samples.len() > budgets.output_values {
            return Err(Error::new(
                ErrorCode::OutputBudget,
                "sample count exceeds output budget",
            ));
        }
        self.absolute_tolerances(request)?;
        let convert = |q: &Quantity| Unit::parse(&q.unit)?.convert(q.value, self.model.time_unit);
        let start = request
            .start
            .as_ref()
            .map(convert)
            .transpose()?
            .unwrap_or(0.0);
        let end = convert(&request.end)?;
        let remaining = budgets
            .events
            .checked_sub(request.checkpoints.len())
            .and_then(|n| n.checked_sub(request.resets.len()))
            .and_then(|n| n.checked_sub(request.covariate_changes.len()))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::WorkBudget,
                    "checkpoints, resets and covariate changes exceed event budget",
                )
            })?;
        let expand = if periodic {
            qspquest_core::regimen::expand_periodic_in_window
        } else {
            qspquest_core::regimen::expand_in_window
        };
        let mut events = expand(
            &request.administrations,
            &self.dose_targets,
            self.model.time_unit,
            start,
            end,
            remaining,
        )?;
        for (index, reset) in request.resets.iter().enumerate() {
            let lower = || -> Result<crate::Event, Error> {
                let target = self
                    .state_names
                    .iter()
                    .position(|name| name == &reset.target)
                    .ok_or_else(|| invalid("unknown reset target"))?;
                let time = convert(&reset.time)?;
                let value = Unit::parse(&reset.value.unit)?
                    .convert(reset.value.value, self.model.state_units[target])?;
                Ok(crate::Event::Reset {
                    time,
                    target,
                    value,
                    order: reset.order,
                    active_inputs: reset.active_inputs,
                })
            };
            events.push(lower().map_err(|mut e| {
                e.expression = Some(format!("resets[{index}]"));
                e
            })?);
        }
        let samples = if let Some(points) = &request.observations {
            if points.is_empty() || !request.samples.is_empty() {
                return Err(invalid(
                    "provide a nonempty observation plan or trajectory samples, not both",
                ));
            }
            if points
                .len()
                .checked_mul(self.outputs.len())
                .is_none_or(|n| n > budgets.output_values)
            {
                return Err(Error::new(
                    ErrorCode::OutputBudget,
                    "observation plan exceeds output budget",
                ));
            }
            points
                .iter()
                .map(|p| convert(&p.time))
                .collect::<Result<Vec<_>, Error>>()?
        } else {
            request
                .samples
                .iter()
                .map(convert)
                .collect::<Result<Vec<_>, Error>>()?
        };
        Ok(crate::Protocol {
            start,
            end,
            samples,
            events,
            rtol: request.rtol,
            // Only the full regimen path can supply the vector; this positive
            // placeholder is not used for error control in that path.
            atol: request.atol.unwrap_or(1.0),
        })
    }
    pub fn output_names(&self) -> &[String] {
        &self.output_names
    }
    pub fn output_units(&self) -> &[String] {
        &self.output_units
    }
}
impl BoundDocument {
    pub fn simulate_regimen(
        &self,
        request: &qspquest_core::regimen::Request,
        budgets: crate::Budgets,
    ) -> Result<Vec<qspquest_core::Sample>, Error> {
        let request = self
            .compiled
            .expand_switches(request, &self.parameter_overrides, budgets)?;
        self.simulate_expanded_regimen(
            &request,
            budgets,
            None,
            qspquest_core::run::Solver::DiffsolBdf,
            None,
            false,
        )
    }
    fn simulate_expanded_regimen(
        &self,
        request: &qspquest_core::regimen::Request,
        budgets: crate::Budgets,
        analytic: Option<crate::linear_pk::LinearPk>,
        solver: qspquest_core::run::Solver,
        selected_outputs: Option<&[usize]>,
        sampled_accuracy: bool,
    ) -> Result<Vec<qspquest_core::Sample>, Error> {
        let prepared = self.prepare_regimen(request, budgets, analytic, selected_outputs)?;
        self.simulate_prepared_regimen(prepared, solver, sampled_accuracy)
    }
    fn prepare_regimen(
        &self,
        request: &qspquest_core::regimen::Request,
        budgets: crate::Budgets,
        analytic: Option<crate::linear_pk::LinearPk>,
        selected_outputs: Option<&[usize]>,
    ) -> Result<PreparedRegimen, Error> {
        let mut covariates = self.covariate_values.clone();
        covariates.extend(request.covariates.clone());
        let lagged =
            self.compiled
                .expand_dose_lags(request, &self.parameter_overrides, &covariates)?;
        self.compiled.reject_history_overrides(
            request
                .covariates
                .keys()
                .map(String::as_str)
                .chain(request.covariate_changes.iter().map(|c| c.name.as_str())),
        )?;
        let with_history = self.compiled.expand_history(&lagged, budgets)?;
        let (with_dose_switches, dose_predicates) = self.compiled.expand_dose_switches(
            &with_history,
            &self.parameter_overrides,
            budgets,
        )?;
        let request = &with_dose_switches;
        let protocol = self.compiled.lower_regimen_protocol(request, budgets)?;
        let mut base = self.compiled.bind_initial_states_internal(
            &self.parameter_overrides,
            &covariates,
            &self.initial_overrides,
        )?;
        base.scheduled = true;
        let storage = request
            .covariate_changes
            .len()
            .checked_add(1)
            .and_then(|n| n.checked_mul(self.compiled.model.binding_storage_values()))
            .ok_or_else(|| {
                Error::new(ErrorCode::OutputBudget, "covariate storage size overflow")
            })?;
        let budgets = crate::Budgets {
            output_values: budgets.output_values.checked_sub(storage).ok_or_else(|| {
                Error::new(
                    ErrorCode::OutputBudget,
                    "covariate binding storage exceeds budget",
                )
            })?,
            ..budgets
        };
        let continuing_initial = base.initial();
        let mut changes = Vec::with_capacity(request.covariate_changes.len());
        for change in &request.covariate_changes {
            let definition = self
                .compiled
                .covariates
                .iter()
                .find(|c| c.name == change.name)
                .ok_or_else(|| invalid(format!("unknown covariate: {}", change.name)))?;
            if definition.interpolation != Interpolation::Step {
                return Err(invalid("constant covariate cannot have step changes"));
            }
            let time = quantity(&change.time)?.in_unit(self.compiled.model.time_unit)?;
            if time < protocol.start || time > protocol.end {
                return Err(invalid("covariate change outside run window"));
            }
            changes.push((time, &change.name, &change.value));
        }
        changes.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut times = Vec::new();
        let mut bindings = Vec::new();
        let mut cursor = 0;
        while cursor < changes.len() {
            let time = changes[cursor].0;
            let mut names = BTreeSet::new();
            while cursor < changes.len() && changes[cursor].0 == time {
                let (_, name, value) = changes[cursor];
                if !names.insert(name) {
                    return Err(invalid("duplicate covariate change at the same time"));
                }
                covariates.insert(name.clone(), value.clone());
                cursor += 1;
            }
            times.push(time);
            bindings.push(
                self.compiled
                    .bind_continuing(
                        &self.parameter_overrides,
                        &covariates,
                        Some(&continuing_initial),
                    )
                    .map_err(|mut e| {
                        e.time = Some(time);
                        e
                    })?,
            );
            bindings.last_mut().expect("binding inserted").scheduled = true;
        }
        let mut boundaries = times.clone();
        for checkpoint in &request.checkpoints {
            let time = quantity(checkpoint)?.in_unit(self.compiled.model.time_unit)?;
            if !time.is_finite() || time < protocol.start || time > protocol.end {
                return Err(invalid("checkpoint outside run window"));
            }
            boundaries.push(time);
        }
        boundaries.sort_by(f64::total_cmp);
        boundaries.dedup();
        let predicate_points = self
            .compiled
            .switches
            .iter()
            .filter_map(|(name, program, comparison)| {
                comparison.map(|op| {
                    program
                        .evaluate(&base.values[..self.compiled.parameters.len()])
                        .map(|time| {
                            (
                                name.clone(),
                                time,
                                switches::predicate_value(op, time, time),
                            )
                        })
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let model = StepModel {
            selected_outputs: selected_outputs.map(|s| s.to_vec()),
            analytic,
            predicate_points,
            dose_predicates,
            boundaries,
            base,
            times,
            bindings,
            active: Cell::new(0),
        };
        let absolute_tolerances = self.compiled.absolute_tolerances(request)?;
        crate::prepare_simulation(&model, &protocol, budgets, absolute_tolerances.as_deref())?;
        let coordinates = crate::simulation_rows(&model, &protocol);
        let output_count = selected_outputs.map_or(self.compiled.outputs.len(), |s| s.len());
        let stored_values = if let Some(points) = &request.observations {
            coordinates
                .len()
                .checked_mul(self.compiled.state_count)
                .and_then(|states| {
                    points
                        .len()
                        .checked_mul(output_count)
                        .and_then(|outputs| states.checked_add(outputs))
                })
        } else {
            self.compiled
                .state_count
                .checked_add(output_count)
                .and_then(|width| coordinates.len().checked_mul(width))
        };
        if stored_values.is_none_or(|count| count > budgets.output_values) {
            return Err(Error::new(
                ErrorCode::OutputBudget,
                if request.observations.is_some() {
                    "trajectory and selected observations exceed output budget"
                } else {
                    "states and outputs exceed result budget"
                },
            ));
        }
        let observation_indices = request
            .observations
            .as_ref()
            .map(|points| {
                points
                    .iter()
                    .map(|point| {
                        let time = Unit::parse(&point.time.unit)?
                            .convert(point.time.value, self.compiled.model.time_unit)?;
                        let side = match point.side {
                            qspquest_core::regimen::ObservationSide::Pre => "pre",
                            qspquest_core::regimen::ObservationSide::Post => "post",
                        };
                        let first = coordinates.partition_point(|(t, _)| *t < time);
                        let matching = &coordinates[first..];
                        let offset = matching
                            .iter()
                            .take_while(|(t, _)| *t == time)
                            .position(|(_, s)| *s == side)
                            .or_else(|| {
                                if !protocol.has_event(time)
                                    && !model.times.contains(&time)
                                    && matching.first().is_some_and(|(t, _)| *t == time)
                                {
                                    Some(0)
                                } else {
                                    None
                                }
                            })
                            .ok_or_else(|| {
                                invalid("requested observation time was not produced")
                            })?;
                        Ok(first + offset)
                    })
                    .collect::<Result<Vec<_>, Error>>()
            })
            .transpose()?;
        let result_rows = request
            .observations
            .as_ref()
            .map_or(coordinates.len(), Vec::len);
        Ok(PreparedRegimen {
            model,
            protocol,
            budgets,
            absolute_tolerances,
            observations: request.observations.clone(),
            result_rows,
            observation_indices,
        })
    }
    fn simulate_prepared_regimen(
        &self,
        prepared: PreparedRegimen,
        solver: qspquest_core::run::Solver,
        sampled_accuracy: bool,
    ) -> Result<Vec<qspquest_core::Sample>, Error> {
        let PreparedRegimen {
            model,
            protocol,
            budgets,
            absolute_tolerances,
            observations,
            result_rows: _,
            observation_indices,
        } = prepared;
        let mut rows = crate::simulate_with_solver_accuracy(
            &model,
            &protocol,
            budgets,
            absolute_tolerances.as_deref(),
            solver,
            sampled_accuracy,
        )?;
        let Some(points) = &observations else {
            for row in &mut rows {
                row.values = model.outputs(row.time, &row.side, &row.values)?;
            }
            return Ok(rows);
        };
        let mut selected = Vec::with_capacity(points.len());
        for (point, &index) in points
            .iter()
            .zip(observation_indices.as_ref().expect("prepared observations"))
        {
            let row = &rows[index];
            let time = row.time;
            let side = match point.side {
                qspquest_core::regimen::ObservationSide::Pre => "pre",
                qspquest_core::regimen::ObservationSide::Post => "post",
            };
            selected.push(qspquest_core::Sample {
                time,
                side: side.into(),
                values: model.outputs(time, side, &row.values)?,
            });
        }
        Ok(selected)
    }
    pub fn simulate_outputs(
        &self,
        protocol: &crate::Protocol,
        budgets: crate::Budgets,
    ) -> Result<Vec<qspquest_core::Sample>, Error> {
        let mut rows = crate::simulate_with_budgets(self, protocol, budgets)?;
        let width = self
            .compiled
            .state_count
            .checked_add(self.compiled.outputs.len())
            .ok_or_else(|| invalid("result width overflow"))?;
        if rows
            .len()
            .checked_mul(width)
            .is_none_or(|count| count > budgets.output_values)
        {
            return Err(Error::new(
                ErrorCode::OutputBudget,
                "states and derived outputs exceed result budget",
            ));
        }
        for row in &mut rows {
            row.values = self.outputs(row.time, &row.values)?;
        }
        Ok(rows)
    }
    pub fn rebind(&mut self, overrides: &BTreeMap<String, Quantity>) -> Result<(), Error> {
        *self = self.compiled.bind_with_initial_states(
            overrides,
            &self.covariate_values,
            &self.initial_overrides,
        )?;
        Ok(())
    }
    pub fn outputs(&self, time: f64, state: &[f64]) -> Result<Vec<f64>, Error> {
        self.outputs_selected(time, state, None)
    }
    fn outputs_selected(
        &self,
        time: f64,
        state: &[f64],
        selected: Option<&[usize]>,
    ) -> Result<Vec<f64>, Error> {
        self.check_state(time, state)?;
        if state.len() != self.compiled.state_count
            || !time.is_finite()
            || state.iter().any(|v| !v.is_finite())
        {
            return Err(invalid("output state dimension differs"));
        }
        let mut inputs = self.values.clone();
        inputs.extend_from_slice(state);
        inputs.push(time);
        if inputs.iter().any(|v| !v.is_finite()) {
            return Err(invalid("output inputs must be finite"));
        }
        let indices = selected.map_or_else(
            || (0..self.compiled.outputs.len()).collect(),
            |s| s.to_vec(),
        );
        let mut scratch = vec![
            0.0;
            indices
                .iter()
                .map(|&i| self.compiled.outputs[i].workspace_size())
                .max()
                .unwrap_or(0)
        ];
        indices
            .into_iter()
            .map(|i| {
                self.compiled.outputs[i]
                    .evaluate_prechecked(&inputs, &mut scratch)
                    .map_err(|mut e| {
                        e.expression = Some(format!(
                            "outputs.{}.{}",
                            self.compiled.output_names[i],
                            e.expression.unwrap_or_default()
                        ));
                        e.time = Some(time);
                        e
                    })
            })
            .collect()
    }
}
impl Model for BoundDocument {
    fn jacobian_sparsity(&self) -> Option<Vec<(usize, usize)>> {
        self.problem.jacobian_sparsity()
    }
    fn rhs_depends_on_time(&self) -> bool {
        self.problem.rhs_depends_on_time()
    }
    fn check_delivery(&self, event: &qspquest_core::Event) -> Result<(), Error> {
        use qspquest_core::{model::DeliveryMode, Event};
        let mode = match event {
            Event::Bolus { .. } => DeliveryMode::Bolus,
            Event::Infusion { .. } => DeliveryMode::Infusion,
            Event::Reset { .. } => return Ok(()),
        };
        let target = self
            .compiled
            .dose_targets
            .iter()
            .find(|t| t.state == event.target())
            .ok_or_else(|| invalid("unsupported dose target"))?;
        if !target.modes.contains(&mode) {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "delivery mode is prohibited for this target",
            ));
        }
        Ok(())
    }

    fn check_state(&self, time: f64, state: &[f64]) -> Result<(), Error> {
        if state.len() != self.compiled.state_count {
            return Err(invalid("state vector size mismatch"));
        }
        for &index in &self.compiled.nonnegative_states {
            if !state[index].is_finite() || state[index] < 0.0 {
                let name = &self.compiled.state_names[index];
                let mut error = Error::new(
                    ErrorCode::Invariant,
                    format!("nonnegative invariant failed for {name}: {}", state[index]),
                );
                error.time = Some(time);
                error.expression = Some(format!("invariants.nonnegative.{name}"));
                return Err(error);
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), Error> {
        if !self.scheduled
            && (!self.compiled.switches.is_empty()
                || !self.compiled.dose_lags.is_empty()
                || !self.compiled.history.is_empty()
                || !self.compiled.dose_switches.is_empty())
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "fixed switches and model lag require simulate_regimen or execute",
            ));
        }
        self.problem.validate()
    }
    fn initial(&self) -> Vec<f64> {
        self.problem.initial()
    }
    fn dose_scale(&self, state: usize) -> Option<f64> {
        self.problem.dose_scale(state)
    }
    fn rhs(&self, time: f64, x: &[f64], rates: &[f64], out: &mut [f64]) -> Result<(), Error> {
        self.problem.rhs(time, x, rates, out)
    }
    fn jac_mul(
        &self,
        time: f64,
        x: &[f64],
        rates: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        self.problem.jac_mul(time, x, rates, v, out)
    }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    use serde_json::{json, Value};

    fn model() -> Value {
        serde_json::from_str(include_str!(
            "../../../conformance/models/synthetic-one-compartment.json"
        ))
        .unwrap()
    }
    fn request() -> qspquest_core::run::RunRequest {
        serde_json::from_str(include_str!(
            "../../../conformance/requests/synthetic-one-compartment.json"
        ))
        .unwrap()
    }

    #[test]
    fn preparation_does_not_evaluate_trajectory_readouts() {
        let mut document = model();
        document["outputs"].as_array_mut().unwrap().push(json!({
            "name":"undefined_readout", "unit":"1", "expression":{
                "kind":"binary", "operator":"divide", "arguments":[
                    {"kind":"literal", "value":0}, {"kind":"literal", "value":0}
                ]
            }
        }));
        let compiled =
            CompiledDocument::compile(&serde_json::from_value(document).unwrap()).unwrap();
        let run = request();
        let prepared = compiled.prepare_selected(&run, None).unwrap();
        assert_eq!(prepared.regimen.observations.as_ref().unwrap().len(), 6);
        assert_eq!(prepared.regimen.protocol.events.len(), 1);
        // Static preparation does not certify a prediction-dependent output.
        assert!(compiled.execute(&run).is_err());
        let mut bounded = run.clone();
        bounded.budgets.output_values = 12;
        assert_eq!(
            compiled.execute(&bounded).unwrap_err().code,
            ErrorCode::OutputBudget
        );
        bounded.regimen.observations = None;
        bounded.regimen.samples.clear();
        bounded.budgets.output_values = 8;
        assert_eq!(
            compiled.execute(&bounded).unwrap_err().code,
            ErrorCode::OutputBudget
        );
        compiled
            .execute_selected(&run, Some(&["cp".into()]))
            .unwrap();
    }

    #[test]
    fn prepared_projection_preserves_event_sides_and_observation_order() {
        let compiled =
            CompiledDocument::compile(&serde_json::from_value(model()).unwrap()).unwrap();
        let mut value = serde_json::to_value(request()).unwrap();
        value["regimen"]["checkpoints"] = json!([{"value":2,"unit":"h"}]);
        value["regimen"]["observations"] = json!([
            {"time":{"value":2,"unit":"h"},"side":"pre"},
            {"time":{"value":0,"unit":"h"},"side":"post"},
            {"time":{"value":0,"unit":"h"},"side":"pre"},
            {"time":{"value":1,"unit":"h"},"side":"pre"},
            {"time":{"value":1,"unit":"h"},"side":"post"},
            {"time":{"value":0,"unit":"h"},"side":"post"}
        ]);
        let run: qspquest_core::run::RunRequest = serde_json::from_value(value).unwrap();
        let prepared = compiled.prepare_selected(&run, None).unwrap();
        let coordinates =
            crate::simulation_rows(&prepared.regimen.model, &prepared.regimen.protocol);
        assert_eq!(
            coordinates,
            [
                (0., "pre"),
                (0., "post"),
                (1., "post"),
                (2., "pre"),
                (2., "post"),
                (12., "post")
            ]
        );
        assert_eq!(
            prepared.regimen.observation_indices.as_ref().unwrap(),
            &[3, 1, 0, 2, 2, 1]
        );
        let plan = compiled.plan_run(&run).unwrap();
        assert_eq!(plan.observation_rows, [3, 1, 0, 2, 2, 1]);
        assert_eq!(plan.workload.result_rows, 6);
        assert_eq!(plan.workload.stored_state_values, 6);
        assert_eq!(plan.workload.stored_output_values, 6);
        let result = compiled.execute(&run).unwrap();
        assert_eq!(result.times, [2., 0., 0., 1., 1., 0.]);
        use qspquest_core::regimen::ObservationSide::{Post, Pre};
        assert_eq!(result.sides, [Pre, Post, Pre, Pre, Post, Post]);
        assert_eq!(result.outputs[0].values[2], 0.);
        assert_eq!(result.outputs[0].values[1], 2.);
        assert_eq!(result.outputs[0].values[3], result.outputs[0].values[4]);
        assert_eq!(result.outputs[0].values[1], result.outputs[0].values[5]);
        assert_eq!(plan.prospective_identity, result.identity);
        assert_eq!(
            plan.observations.iter().map(|p| p.time).collect::<Vec<_>>(),
            result.times
        );
        assert_eq!(
            plan.observations.iter().map(|p| p.side).collect::<Vec<_>>(),
            result.sides
        );
    }

    #[test]
    fn prospective_identity_matches_execution_without_evaluating_readouts() {
        let mut source = model();
        source["outputs"].as_array_mut().unwrap().push(json!({
            "name":"undefined_readout", "unit":"1", "expression":{
                "kind":"binary", "operator":"divide", "arguments":[
                    {"kind":"literal", "value":0}, {"kind":"literal", "value":0}
                ]
            }
        }));
        let compiled = CompiledDocument::compile(&serde_json::from_value(source).unwrap()).unwrap();
        let mut run = request();
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
        let selection = ["cp".into()];
        let prepared = compiled.prepare_selected(&run, Some(&selection)).unwrap();
        let planned = compiled
            .prospective_identity(&run, Some(&selection), &prepared.bound)
            .unwrap();
        let unselected = compiled
            .prospective_identity(&run, None, &prepared.bound)
            .unwrap();
        assert_ne!(planned.run_request_hash, unselected.run_request_hash);
        assert!(compiled.execute(&run).is_err());
        let result = compiled.execute_selected(&run, Some(&selection)).unwrap();
        assert_eq!(planned, result.identity);
        run.request_id = Some("different-correlation".into());
        run.parameters.insert(
            "cl".into(),
            Quantity {
                value: 0.3,
                unit: "L/h".into(),
            },
        );
        let same = compiled.prepare_selected(&run, Some(&selection)).unwrap();
        assert_eq!(
            planned,
            compiled
                .prospective_identity(&run, Some(&selection), &same.bound)
                .unwrap()
        );
        run.initial_states.insert(
            "central".into(),
            Quantity {
                value: 4.,
                unit: "mg".into(),
            },
        );
        let changed = compiled.prepare_selected(&run, Some(&selection)).unwrap();
        let identity = compiled
            .prospective_identity(&run, Some(&selection), &changed.bound)
            .unwrap();
        assert_eq!(planned.run_request_hash, identity.run_request_hash);
        assert_ne!(planned.bound_value_hash, identity.bound_value_hash);
    }

    #[test]
    fn preparation_and_execution_share_static_refusals() {
        let compiled =
            CompiledDocument::compile(&serde_json::from_value(model()).unwrap()).unwrap();
        let original = serde_json::to_value(request()).unwrap();
        for case in 0..7 {
            let mut value = original.clone();
            match case {
                0 => value["parameters"] = json!({"cl":{"value":1,"unit":"mg"}}),
                1 => value["initial_states"] = json!({"unknown":{"value":1,"unit":"mg"}}),
                2 => value["regimen"]["administrations"][0]["target"] = json!("unknown"),
                3 => value["regimen"]["end"] = json!({"value":0,"unit":"h"}),
                4 => value["regimen"]["observations"][0]["time"] = json!({"value":13,"unit":"h"}),
                5 => value["budgets"]["output_values"] = json!(1),
                _ => value["seed"] = json!(1),
            }
            let run = serde_json::from_value(value).unwrap();
            let prepared = compiled.prepare_selected(&run, None).err().unwrap();
            let executed = compiled.execute(&run).unwrap_err();
            assert_eq!(prepared, executed, "case {case}");
            assert_eq!(
                compiled.plan_run(&run).unwrap_err(),
                executed,
                "plan case {case}"
            );
        }
        assert!(compiled.prepare_selected(&request(), Some(&[])).is_err());
        assert!(compiled
            .prepare_selected(&request(), Some(&["cp".into(), "cp".into()]))
            .is_err());
        assert!(compiled
            .prepare_selected(&request(), Some(&["unknown".into()]))
            .is_err());
        let mut empty = request();
        empty.regimen.observations = None;
        empty.regimen.samples.clear();
        assert!(compiled
            .prepare_selected(&empty, None)
            .unwrap()
            .regimen
            .observations
            .is_none());
        assert_eq!(compiled.execute(&empty).unwrap().times, [0.0, 0.0, 12.0]);
        let plan = compiled.plan_run(&empty).unwrap();
        assert_eq!(
            plan.observations.iter().map(|p| p.time).collect::<Vec<_>>(),
            [0., 0., 12.]
        );
        assert!(plan.request.regimen.samples.is_empty());
        assert!(plan.request.regimen.observations.is_none());
    }

    #[test]
    fn preparation_lowers_lags_history_switches_resets_and_covariate_phases() {
        let mut document = model();
        for (name, value) in [("delay", 0.5), ("switch_time", 3.0)] {
            document["parameters"].as_array_mut().unwrap().push(json!({
                "name":name,"default":{"value":value,"unit":"h"},
                "transform":"identity","fixed":true,"description":""
            }));
        }
        document["states"][0]["dosing"]["lag"] = json!({"kind":"symbol","name":"delay"});
        document["switches"] =
            json!([{"name":"phase","time":{"kind":"symbol","name":"switch_time"}}]);
        document["dose_history"] = json!([{"name":"seen","target":"central","quantity":"has_dose","simultaneous":"first"}]);
        document["covariates"] = json!([{"name":"factor","unit":"1","interpolation":"step","default":{"value":1,"unit":"1"}}]);
        for name in ["phase", "seen", "factor"] {
            document["outputs"].as_array_mut().unwrap().push(json!({
                "name":name,"unit":"1","expression":{"kind":"symbol","name":name}
            }));
        }
        let compiled =
            CompiledDocument::compile(&serde_json::from_value(document.clone()).unwrap()).unwrap();
        let mut value = serde_json::to_value(request()).unwrap();
        value["regimen"]["administrations"][0]["repeat"] =
            json!({"interval":{"value":3,"unit":"h"},"additional":2});
        value["regimen"]["covariate_changes"] =
            json!([{"name":"factor","time":{"value":4,"unit":"h"},"value":{"value":2,"unit":"1"}}]);
        value["regimen"]["checkpoints"] = json!([{"value":5,"unit":"h"}]);
        value["regimen"]["resets"] = json!([{"target":"central","time":{"value":6,"unit":"h"},"value":{"value":3,"unit":"mg"},"order":0,"active_inputs":"continue"}]);
        let mut run = serde_json::from_value::<qspquest_core::run::RunRequest>(value).unwrap();
        let reset_run = run.clone();
        let refusal = compiled.prepare_selected(&run, None).err().unwrap();
        assert_eq!(refusal.code, ErrorCode::Unsupported);
        assert_eq!(refusal, compiled.execute(&run).unwrap_err());
        assert!(refusal.message.contains("dose-history reset semantics"));
        assert_eq!(compiled.plan_run(&run).unwrap_err(), refusal);
        // History and reset combinations retain their current explicit restriction.
        run.regimen.resets.clear();
        assert!(compiled
            .regimen_protocol(&run.regimen, run.budgets)
            .is_err());
        let prepared = compiled.prepare_selected(&run, None).unwrap();
        let events = &prepared.regimen.protocol.events;
        assert_eq!(
            events.iter().map(crate::Event::time).collect::<Vec<_>>(),
            [0.5, 3.5, 6.5]
        );
        for boundary in [0.5, 3.0, 3.5, 4.0, 5.0, 6.5] {
            assert!(prepared.regimen.model.boundaries.contains(&boundary));
        }
        let result = compiled.execute(&run).unwrap();
        let plan = compiled.plan_run(&run).unwrap();
        assert_eq!(plan.prospective_identity, result.identity);
        assert_eq!(
            plan.events
                .iter()
                .map(|p| p.event.time())
                .collect::<Vec<_>>(),
            [0.5, 3.5, 6.5]
        );
        assert!(plan.events.iter().all(
            |p| p.target_name == "central" && p.quantity_unit.declared.as_deref() == Some("mg")
        ));
        assert!(plan
            .binding_phases
            .iter()
            .any(|p| p.time == 4. && p.inputs.iter().any(|i| i.name == "factor" && i.value == 2.)));
        for boundary in [0.5, 3., 3.5, 4., 5., 6.5] {
            assert!(plan.checkpoints.contains(&boundary));
        }
        assert_eq!(result.times, [0.0, 1.0, 2.0, 4.0, 8.0, 12.0]);
        assert_eq!(result.outputs[1].values, [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        assert_eq!(result.outputs[2].values, [0.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
        assert_eq!(result.outputs[3].values, [1.0, 1.0, 1.0, 2.0, 2.0, 2.0]);

        document.as_object_mut().unwrap().remove("dose_history");
        document["outputs"]
            .as_array_mut()
            .unwrap()
            .retain(|o| o["name"] != "seen");
        let reset_model =
            CompiledDocument::compile(&serde_json::from_value(document).unwrap()).unwrap();
        let mut reset_run = reset_run;
        let points = reset_run.regimen.observations.as_mut().unwrap();
        for (index, side) in [
            (4, qspquest_core::regimen::ObservationSide::Pre),
            (5, qspquest_core::regimen::ObservationSide::Post),
        ] {
            points.insert(
                index,
                qspquest_core::regimen::ObservationPoint {
                    time: Quantity {
                        value: 6.0,
                        unit: "h".into(),
                    },
                    side,
                },
            );
        }
        let prepared = reset_model.prepare_selected(&reset_run, None).unwrap();
        assert_eq!(
            prepared
                .regimen
                .protocol
                .events
                .iter()
                .map(crate::Event::time)
                .collect::<Vec<_>>(),
            [0.5, 3.5, 6.5, 6.0]
        );
        let reset_result = reset_model.execute(&reset_run).unwrap();
        let plan = reset_model.plan_run(&reset_run).unwrap();
        assert_eq!(
            plan.events
                .iter()
                .map(|p| p.event.time())
                .collect::<Vec<_>>(),
            [0.5, 3.5, 6.5, 6.]
        );
        assert_eq!(plan.events[3].quantity_unit.declared.as_deref(), Some("mg"));
        assert_eq!(plan.prospective_identity, reset_result.identity);
        assert!((reset_result.outputs[0].values[5] - 1.0).abs() < 1e-12);
    }
}

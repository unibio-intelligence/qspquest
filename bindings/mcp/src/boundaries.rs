//! Native schedules, declared forcing origins and explicit observation conventions.
use crate::{contract::EngineIdentity, policy};
use qspquest_core::{
    expression::{Compare, Expr},
    model::{DoseSwitch, ModelDocument},
    planning::{BoundInput, PlannedObservation, ReadoutInputOverrides, RunPlan},
    units::Unit,
    Event,
};
use qspquest_modeling::compile::{Program, Symbol, SymbolKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(rename = "ScheduledBoundarySchema")]
pub enum Schema {
    #[serde(rename = "qspquest.scheduled-boundaries/v1")]
    V1,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Prepared,
    Executed,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FixedSwitch {
    pub name: String,
    #[schemars(with = "ExpressionSchema")]
    pub time_expression: Expr,
    #[schemars(with = "Vec<crate::initialization::Input>")]
    pub parameter_bindings: Vec<BoundInput>,
    /// Evaluated with the same owned unit-aware bytecode compiler as native preparation.
    pub threshold: f64,
    #[schemars(with = "Option<ComparisonSchema>")]
    pub comparison: Option<Compare>,
    /// A pointwise predicate value can differ from the pre-boundary integration input.
    /// None denotes a step binding whose readout follows the requested observation side.
    pub pointwise_value_at_threshold: Option<f64>,
    pub initial_input_value: f64,
    pub boundary_index: Option<usize>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventRole {
    Reset,
    Bolus,
    InfusionStart,
    InfusionEnd,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventReference {
    /// Index into the native plan's event table; an infusion has two boundary references.
    pub native_event_index: usize,
    pub role: EventRole,
    pub target: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Boundary {
    pub time: f64,
    /// Native checkpoints include binding updates and explicit smooth checkpoints.
    pub native_checkpoint: bool,
    pub requested_checkpoint: bool,
    pub post_binding_phase_indices: Vec<usize>,
    pub fixed_switch_names: Vec<String>,
    pub events: Vec<EventReference>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StateTolerance {
    pub state: String,
    pub value: f64,
    pub unit: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "ScheduledBoundaryReceipt")]
pub struct Receipt {
    pub schema: Schema,
    pub stage: Stage,
    pub time_unit: String,
    pub integration_start: f64,
    pub integration_end: f64,
    pub planned_execution_identity: EngineIdentity,
    pub fixed_switches: Vec<FixedSwitch>,
    /// Original declarations; their expanded changes remain in the native binding phases.
    #[schemars(with = "Vec<DoseSwitchSchema>")]
    pub dose_relative_switches: Vec<DoseSwitch>,
    pub boundaries: Vec<Boundary>,
    #[schemars(with = "Vec<ObservationSchema>")]
    pub observations: Vec<PlannedObservation>,
    pub observation_driver_rows: Vec<usize>,
    #[schemars(with = "Vec<InputOverrideSchema>")]
    pub pointwise_input_overrides: Vec<ReadoutInputOverrides>,
    #[schemars(with = "SolverSchema")]
    pub requested_solver: qspquest_core::run::Solver,
    pub requested_relative_tolerance: f64,
    pub requested_state_absolute_tolerances: Vec<StateTolerance>,
    /// Current native execution does not report an achieved error estimate.
    pub achieved_error_estimate: Option<f64>,
    pub observation_convention: String,
}
fn fail(message: impl ToString) -> Value {
    policy::error("scheduled_boundary_receipt", message.to_string())
}
fn zero(t: f64) -> f64 {
    if t == 0. {
        0.
    } else {
        t
    }
}
fn symbols(expr: &Expr, out: &mut BTreeSet<String>) {
    if let Expr::Symbol { name } = expr {
        out.insert(name.clone());
    }
    for child in expr.children() {
        symbols(child, out);
    }
}
pub fn prepare(plan: &RunPlan, model: &Value) -> Result<Receipt, Value> {
    let model: ModelDocument = serde_json::from_value(model.clone()).map_err(fail)?;
    let initial = plan
        .binding_phases
        .first()
        .ok_or_else(|| fail("Native initial binding phase missing"))?;
    let params: Vec<(Symbol, Unit)> = model
        .parameters
        .iter()
        .map(|p| {
            Ok((
                Symbol {
                    name: p.name.clone(),
                    kind: SymbolKind::Parameter,
                },
                Unit::parse(&p.default.unit).map_err(fail)?,
            ))
        })
        .collect::<Result<_, Value>>()?;
    let values: Vec<f64> = model
        .parameters
        .iter()
        .map(|p| {
            initial
                .inputs
                .iter()
                .find(|v| v.name == p.name)
                .map(|v| v.value)
                .ok_or_else(|| fail(format!("Native parameter binding missing: {}", p.name)))
        })
        .collect::<Result<_, _>>()?;
    let time_unit = Unit::parse(&plan.time_unit).map_err(fail)?;
    let requested: Vec<f64> = plan
        .request
        .regimen
        .checkpoints
        .iter()
        .map(|q| {
            Unit::parse(&q.unit)
                .and_then(|u| u.convert(q.value, time_unit))
                .map(zero)
                .map_err(fail)
        })
        .collect::<Result<_, _>>()?;
    let mut times: Vec<f64> = plan.checkpoints.iter().copied().map(zero).collect();
    times.extend(plan.binding_phases.iter().skip(1).map(|p| zero(p.time)));
    for event in &plan.events {
        times.push(zero(event.event.time()));
        if let Event::Infusion { time, duration, .. } = &event.event {
            if time + duration <= plan.end {
                times.push(zero(time + duration));
            }
        }
    }
    times.retain(|t| *t >= plan.start && *t <= plan.end);
    times.sort_by(f64::total_cmp);
    times.dedup();
    let mut boundaries: Vec<Boundary> = times
        .iter()
        .map(|t| Boundary {
            time: *t,
            native_checkpoint: plan.checkpoints.iter().any(|p| p == t),
            requested_checkpoint: requested.contains(t),
            post_binding_phase_indices: plan
                .binding_phases
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(_, p)| p.time == *t)
                .map(|(i, _)| i)
                .collect(),
            fixed_switch_names: Vec::new(),
            events: Vec::new(),
        })
        .collect();
    let locate = |t: f64| times.binary_search_by(|v| v.total_cmp(&zero(t))).ok();
    for (i, event) in plan.events.iter().enumerate() {
        let role = match event.event {
            Event::Reset { .. } => EventRole::Reset,
            Event::Bolus { .. } => EventRole::Bolus,
            Event::Infusion { .. } => EventRole::InfusionStart,
        };
        if let Some(j) = locate(event.event.time()) {
            boundaries[j].events.push(EventReference {
                native_event_index: i,
                role,
                target: event.target_name.clone(),
            });
        }
        if let Event::Infusion { time, duration, .. } = event.event {
            if let Some(j) = locate(time + duration) {
                boundaries[j].events.push(EventReference {
                    native_event_index: i,
                    role: EventRole::InfusionEnd,
                    target: event.target_name.clone(),
                });
            }
        }
    }
    let mut fixed_switches = Vec::new();
    for switch in &model.switches {
        let threshold = Program::compile_with_units(&switch.time, &params, time_unit)
            .and_then(|p| p.evaluate(&values))
            .map_err(fail)?;
        if !threshold.is_finite() {
            return Err(fail("Native switch threshold must be finite"));
        }
        let pointwise = switch.comparison.map(|op| {
            if matches!(op, Compare::Le | Compare::Ge | Compare::Eq) {
                1.
            } else {
                0.
            }
        });
        let initial_input_value = initial
            .inputs
            .iter()
            .find(|p| p.name == switch.name)
            .map(|p| p.value)
            .ok_or_else(|| fail("Native fixed-switch binding missing"))?;
        let boundary_index = if threshold >= plan.start && threshold <= plan.end {
            locate(threshold)
        } else {
            None
        };
        if let Some(i) = boundary_index {
            boundaries[i].fixed_switch_names.push(switch.name.clone());
        }
        let mut needed = BTreeSet::new();
        symbols(&switch.time, &mut needed);
        fixed_switches.push(FixedSwitch {
            name: switch.name.clone(),
            time_expression: switch.time.clone(),
            threshold,
            comparison: switch.comparison,
            parameter_bindings: initial
                .inputs
                .iter()
                .filter(|p| needed.contains(&p.name))
                .cloned()
                .collect(),
            pointwise_value_at_threshold: pointwise,
            initial_input_value,
            boundary_index,
        });
    }
    Ok(Receipt {
        schema:Schema::V1,stage:Stage::Prepared,time_unit:plan.time_unit.clone(),integration_start:plan.start,integration_end:plan.end,
        planned_execution_identity:serde_json::from_value(json!(plan.prospective_identity)).map_err(fail)?,
        fixed_switches,dose_relative_switches:model.dose_switches,boundaries,observations:plan.observations.clone(),
        observation_driver_rows:plan.observation_rows.clone(),pointwise_input_overrides:plan.readout_overrides.clone(),
        requested_solver:plan.request.solver,requested_relative_tolerance:plan.relative_tolerance,
        requested_state_absolute_tolerances:plan.states.iter().map(|s|StateTolerance{state:s.name.clone(),value:s.absolute_tolerance,unit:s.unit.clone()}).collect(),
        achieved_error_estimate:None,
        observation_convention:"pre captures the state before boundary updates, resets and doses; post includes those updates. Fixed time predicates are evaluated at the exact observation time for either side, and can differ from the integration phase. Requested tolerances are not achieved-error estimates; execution does not establish independent scientific qualification.".into(),
    })
}
pub fn complete(receipt: &mut Receipt, result: &Value) -> Result<(), Value> {
    if json!(receipt.planned_execution_identity) != result["identity"]
        || json!(receipt
            .observations
            .iter()
            .map(|o| o.time)
            .collect::<Vec<_>>())
            != result["times"]
        || json!(receipt
            .observations
            .iter()
            .map(|o| o.side)
            .collect::<Vec<_>>())
            != result["sides"]
    {
        return Err(fail(
            "Executed identity, observation times or sides differ from the native schedule",
        ));
    }
    receipt.stage = Stage::Executed;
    Ok(())
}

// Schemars adapters preserve the closed native wire types without changing core's schema dependency.
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ComparisonSchema {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum UnarySchema {
    Positive,
    Negative,
    Not,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum BinarySchema {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
    Modulo,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum BooleanSchema {
    And,
    Or,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FunctionSchema {
    Exp,
    Log,
    Log10,
    Sqrt,
    Sin,
    Cos,
    Tan,
    Tanh,
    Abs,
    Min,
    Max,
    Floor,
    Ceil,
    Hill,
    Ifelse,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
enum LiteralSchema {
    Number(f64),
    Boolean(bool),
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ExpressionSchema {
    Literal {
        value: LiteralSchema,
    },
    Quantity {
        quantity: QuantitySchema,
    },
    Symbol {
        name: String,
    },
    Unary {
        operator: UnarySchema,
        arguments: Box<[ExpressionSchema; 1]>,
    },
    Binary {
        operator: BinarySchema,
        arguments: Box<[ExpressionSchema; 2]>,
    },
    Compare {
        operator: ComparisonSchema,
        arguments: Box<[ExpressionSchema; 2]>,
    },
    Boolean {
        operator: BooleanSchema,
        arguments: Vec<ExpressionSchema>,
    },
    Call {
        name: FunctionSchema,
        arguments: Vec<ExpressionSchema>,
    },
    Conditional {
        arguments: Box<[ExpressionSchema; 3]>,
    },
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct QuantitySchema {
    value: f64,
    unit: String,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SideSchema {
    Pre,
    Post,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ObservationSchema {
    time: f64,
    side: SideSchema,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InputOverrideSchema {
    observation: usize,
    covariates: std::collections::BTreeMap<String, crate::planning::DoseQuantity>,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SimultaneousSchema {
    First,
    Last,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DoseSwitchSchema {
    name: String,
    target: String,
    offset: ExpressionSchema,
    comparison: ComparisonSchema,
    simultaneous: SimultaneousSchema,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SolverSchema {
    DiffsolBdf,
    DiffsolTsit45,
    DiffsolEsdirk34,
    DiffsolTrBdf2,
    #[serde(rename = "diffsol_rosenbrock23")]
    DiffsolRosenbrock23,
    #[serde(rename = "diffsol_rodas5p")]
    DiffsolRodas5p,
    AnalyticLinearPk,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(op: &str) -> String {
        format!(
            "model boundary_example\ntime_unit = h\nparameters\n cutoff = 2 [h] estimated\n k0 = 1 [1/h] fixed\n k1 = 2 [1/h] fixed\nswitches\n early = time {op} cutoff\nstates\n central = 0 [mg]\ndynamics\n d/dt(central) = input_central - ifelse(early > 0, k0, k1) * central\ndosing\n central [mg] = 1\noutputs\n amount = central [mg]\n predicate = early [1]\nend"
        )
    }
    #[test]
    fn boundary_origins_pointwise_predicates_and_executed_sides_are_native() {
        for op in ["<", "<=", ">", ">="] {
            let model = qspquest_modeling::language::parse(&source(op))
                .unwrap()
                .document;
            let compiled = qspquest_modeling::document::CompiledDocument::compile(&model).unwrap();
            let request=serde_json::from_value(json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf",
                "budgets":{"solver_callbacks":1000000,"events":100,"output_values":10000},
                "regimen":{"samples":[],"end":{"value":5,"unit":"h"},"rtol":1e-10,"atol":1e-12,
                "administrations":[
                    {"target":"central","time":{"value":0,"unit":"h"},"amount":{"value":10,"unit":"mg"},"delivery":{"kind":"bolus"},"bioavailability":1},
                    {"target":"central","time":{"value":2,"unit":"h"},"amount":{"value":5,"unit":"mg"},"delivery":{"kind":"bolus"},"bioavailability":1},
                    {"target":"central","time":{"value":3,"unit":"h"},"amount":{"value":1,"unit":"mg"},"delivery":{"kind":"infusion","span":{"kind":"duration","duration":{"value":1,"unit":"h"}}},"bioavailability":1}],
                "checkpoints":[{"value":4.5,"unit":"h"}],
                "observations":[{"time":{"value":0,"unit":"h"},"side":"pre"},{"time":{"value":0,"unit":"h"},"side":"post"},
                    {"time":{"value":2,"unit":"h"},"side":"pre"},{"time":{"value":2,"unit":"h"},"side":"post"},
                    {"time":{"value":4,"unit":"h"},"side":"pre"},{"time":{"value":4,"unit":"h"},"side":"post"}]}})).unwrap();
            let plan = compiled.plan_run(&request).unwrap();
            let mut receipt = prepare(&plan, &json!(model)).unwrap();
            assert_eq!(
                receipt
                    .boundaries
                    .iter()
                    .map(|b| b.time)
                    .collect::<Vec<_>>(),
                [0., 2., 3., 4., 4.5]
            );
            assert_eq!(receipt.fixed_switches[0].threshold, 2.);
            assert_eq!(
                receipt.fixed_switches[0].pointwise_value_at_threshold,
                Some(if op.contains('=') { 1. } else { 0. })
            );
            assert_eq!(receipt.boundaries[1].fixed_switch_names, ["early"]);
            assert!(matches!(
                receipt.boundaries[3].events[0].role,
                EventRole::InfusionEnd
            ));
            assert!(receipt.boundaries[4].requested_checkpoint);
            let result = json!(compiled.execute(&request).unwrap());
            complete(&mut receipt, &result).unwrap();
            assert!(matches!(receipt.stage, Stage::Executed));
            assert_eq!(result["outputs"][0]["values"][1], 10.);
            let values = result["outputs"][0]["values"].as_array().unwrap();
            assert!((values[3].as_f64().unwrap() - values[2].as_f64().unwrap() - 5.).abs() < 1e-10);
            for i in [2, 3] {
                assert_eq!(
                    result["outputs"][1]["values"][i],
                    json!(receipt.fixed_switches[0]
                        .pointwise_value_at_threshold
                        .unwrap())
                );
            }
            let mut forged = result.clone();
            forged["sides"][0] = json!("post");
            assert!(complete(&mut receipt, &forged).is_err());
            forged = result.clone();
            forged["times"][0] = json!(1.);
            assert!(complete(&mut receipt, &forged).is_err());
            forged = result.clone();
            forged["identity"]["run_request_hash"] = json!("changed");
            assert!(complete(&mut receipt, &forged).is_err());
        }
    }
}

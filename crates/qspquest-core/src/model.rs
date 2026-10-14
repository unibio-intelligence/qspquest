//! Versioned simulation document. Additional specification features are not silently ignored.
use crate::expression::Expr;
use serde::{Deserialize, Serialize};
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Schema {
    #[serde(rename = "qspquest.model/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transform {
    Identity,
    Log,
    Logit,
    Probit,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    pub default: Quantity,
    pub bounds: Option<[Quantity; 2]>,
    pub transform: Transform,
    pub fixed: bool,
    pub description: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Initial {
    Quantity { quantity: Quantity },
    Expression { expression: Expr },
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Bolus,
    Infusion,
}
pub fn default_delivery_modes() -> Vec<DeliveryMode> {
    vec![DeliveryMode::Bolus, DeliveryMode::Infusion]
}
pub fn validate_delivery_modes(modes: &[DeliveryMode]) -> Result<(), crate::Error> {
    if modes.is_empty() || modes.len() > 2 || (modes.len() == 2 && modes[0] == modes[1]) {
        return Err(crate::Error::new(
            crate::ErrorCode::InvalidInput,
            "dosing modes must be a nonempty unique list",
        ));
    }
    Ok(())
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dosing {
    #[serde(default = "default_delivery_modes")]
    pub modes: Vec<DeliveryMode>,
    pub amount_unit: String,
    pub scale: Expr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lag: Option<Expr>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub name: String,
    pub unit: String,
    pub initial: Initial,
    pub rhs: Expr,
    pub dosing: Option<Dosing>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub name: String,
    pub unit: String,
    pub expression: Expr,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDocument {
    pub schema: Schema,
    pub model_id: String,
    pub description: String,
    pub time_unit: String,
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub covariates: Vec<Covariate>,
    #[serde(default)]
    pub individual: Vec<IndividualParameter>,
    #[serde(default)]
    pub definitions: Vec<Definition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear_pk: Option<LinearPkDefinition>,
    pub states: Vec<State>,
    pub outputs: Vec<Output>,
    #[serde(default)]
    pub requirements: Requirements,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub switches: Vec<FixedSwitch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dose_history: Vec<DoseHistoryBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dose_switches: Vec<DoseSwitch>,
    #[serde(default)]
    pub invariants: Vec<Invariant>,
}

/// A dimensionless phase, zero before its parameter-bound time and one after.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedSwitch {
    pub name: String,
    pub time: Expr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<crate::expression::Compare>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndividualParameter {
    pub name: String,
    pub expression: Expr,
    pub unit: Option<String>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    Constant,
    Step,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Covariate {
    pub name: String,
    pub unit: String,
    pub interpolation: Interpolation,
    pub default: Option<Quantity>,
}

/// Minimum execution capabilities, independent of any specific backend.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirements {
    #[serde(default)]
    pub stiff: bool,
    #[serde(default)]
    pub sparse_jacobian: bool,
    #[serde(default)]
    pub gradients: bool,
    #[serde(default)]
    pub events: Vec<EventRequirement>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventRequirement {
    Bolus,
    Infusion,
    Reset,
    CovariateStep,
    StateTriggered,
    ParameterDependentTime,
    SteadyState,
}
impl EventRequirement {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bolus => "bolus",
            Self::Infusion => "infusion",
            Self::Reset => "reset",
            Self::CovariateStep => "covariate_step",
            Self::StateTriggered => "state_triggered",
            Self::ParameterDependentTime => "parameter_dependent_time",
            Self::SteadyState => "steady_state",
        }
    }
}

/// Runtime assertions checked on physical states, never solver trial iterates.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Invariant {
    Nonnegative { states: Vec<String> },
}

/// A named algebraic expression evaluated from current bindings, states and time.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub name: String,
    pub expression: Expr,
    pub unit: Option<String>,
}

/// Named, per-target dose-history readout. Missing numeric history is an error;
/// `HasDose` can guard it explicitly in a lazy expression.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoseHistoryBinding {
    pub name: String,
    pub target: String,
    pub quantity: HistoryQuantity,
    pub simultaneous: crate::dose_history::SimultaneousDose,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryQuantity {
    DoseAmount,
    DoseTime,
    TimeSinceDose,
    HasDose,
}

/// Parameter-bound elapsed-dose predicate; the named result is dimensionless.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoseSwitch {
    pub name: String,
    pub target: String,
    pub offset: Expr,
    pub comparison: crate::expression::Compare,
    pub simultaneous: crate::dose_history::SimultaneousDose,
}

/// Mammillary PK declaration; coefficient names refer to declared parameters.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearPkDefinition {
    pub amount_unit: String,
    pub clearance: String,
    pub compartments: Vec<LinearPkCompartment>,
    pub exchange_clearances: Vec<String>,
    pub depot: Option<LinearPkDepot>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearPkCompartment {
    pub state: String,
    pub volume: String,
    pub initial: Initial,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dosing_override: Option<Dosing>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearPkDepot {
    pub state: String,
    pub absorption: String,
    pub initial: Initial,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dosing_override: Option<Dosing>,
}

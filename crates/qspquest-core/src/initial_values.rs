//! Initial output evaluation before administrations, without a solver or future regimen.
use crate::{
    model::Quantity,
    planning::{BoundInput, ExactUnit, PlannedOutput},
    run::ExecutionIdentity,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum RequestSchema {
    #[serde(rename = "qspquest.initial-values/v0.1")]
    V01,
}
fn default_output_limit() -> usize {
    25_000_000
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialValuesRequest {
    pub schema: RequestSchema,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub parameters: BTreeMap<String, Quantity>,
    #[serde(default)]
    pub covariates: BTreeMap<String, Quantity>,
    #[serde(default)]
    pub initial_states: BTreeMap<String, Quantity>,
    /// Native output names in returned order; omission selects every output.
    #[serde(default)]
    pub outputs: Option<Vec<String>>,
    #[serde(default = "default_output_limit")]
    pub max_output_values: usize,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialState {
    pub name: String,
    pub unit: String,
    pub value: f64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum PlanSchema {
    #[serde(rename = "qspquest.initial-values-plan/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialValuesPlan {
    pub schema: PlanSchema,
    pub request: InitialValuesRequest,
    pub prospective_identity: ExecutionIdentity,
    pub time: f64,
    pub time_unit: String,
    pub exact_time_unit: ExactUnit,
    pub inputs: Vec<BoundInput>,
    pub states: Vec<InitialState>,
    pub outputs: Vec<PlannedOutput>,
    pub output_values: usize,
    pub deferred_checks: Vec<String>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialOutput {
    pub name: String,
    pub unit: String,
    pub value: f64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ResultSchema {
    #[serde(rename = "qspquest.initial-values-result/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialValuesResult {
    pub schema: ResultSchema,
    pub request_id: Option<String>,
    pub identity: ExecutionIdentity,
    pub model_id: String,
    pub time: f64,
    pub time_unit: String,
    pub inputs: Vec<BoundInput>,
    pub states: Vec<InitialState>,
    pub outputs: Vec<InitialOutput>,
}

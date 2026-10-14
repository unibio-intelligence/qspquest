//! Versioned deterministic simulation envelope; stochastic execution is not yet supported.
use crate::{
    model::Quantity,
    regimen::{ObservationSide, Request},
    Budgets,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum RunSchema {
    #[serde(rename = "qspquest.run/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Solver {
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
/// Local integration or bounded sampled-trajectory convergence checking.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccuracyControl {
    #[default]
    Local,
    Sampled,
}
impl AccuracyControl {
    pub fn is_local(&self) -> bool {
        *self == Self::Local
    }
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub schema: RunSchema,
    pub request_id: Option<String>,
    pub seed: Option<u64>,
    pub solver: Solver,
    #[serde(default, skip_serializing_if = "AccuracyControl::is_local")]
    pub accuracy_control: AccuracyControl,
    pub budgets: Budgets,
    #[serde(default)]
    pub parameters: BTreeMap<String, Quantity>,
    #[serde(default)]
    pub initial_states: BTreeMap<String, Quantity>,
    pub regimen: Request,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecutionIdentity {
    pub model_content_hash: String,
    pub spec_version: String,
    pub compiler_version: String,
    pub compiler_source_sha256: String,
    pub rustc_version: String,
    pub target: String,
    pub build_profile: String,
    pub rustflags_sha256: String,
    pub bound_value_hash: String,
    pub run_request_hash: String,
    pub backend: String,
    pub backend_version: String,
    pub algorithm: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ResultSchema {
    #[serde(rename = "qspquest.result/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultColumn {
    pub name: String,
    pub unit: String,
    pub values: Vec<f64>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunResult {
    pub schema: ResultSchema,
    pub request_id: Option<String>,
    pub identity: ExecutionIdentity,
    pub model_id: String,
    pub description: String,
    pub time_unit: String,
    pub times: Vec<f64>,
    pub sides: Vec<ObservationSide>,
    pub outputs: Vec<ResultColumn>,
}

/// A single periodic cycle. The nested run supplies parameters, regimen and budgets.
/// Initial overrides are prohibited: this operation solves for the initial state.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteadyStateRequest {
    pub run: RunRequest,
    pub absolute_tolerance: Quantity,
    pub relative_tolerance: f64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteadyStateResult {
    pub identity: ExecutionIdentity,
    pub initial_states: BTreeMap<String, Quantity>,
    pub residuals: BTreeMap<String, Quantity>,
    pub maximum_scaled_residual: f64,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodicTimeBasis {
    CyclePhase,
}
/// Bounded nonlinear iteration; run.initial_states supplies the starting guess.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CycleIterationRequest {
    pub run: RunRequest,
    pub time_basis: PeriodicTimeBasis,
    pub absolute_tolerances: BTreeMap<String, Quantity>,
    pub relative_tolerance: f64,
    pub maximum_cycles: u32,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CycleIterationResult {
    pub steady_state: SteadyStateResult,
    pub cycles_completed: u32,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum SensitivitySchema {
    #[serde(rename = "qspquest.sensitivity/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensitivityRequest {
    pub schema: SensitivitySchema,
    pub run: RunRequest,
    /// Ordered independent model parameters, in their declared numeric units.
    pub with_respect_to: Vec<String>,
    /// Parameter -> state -> positive state/parameter quantity.
    pub derivative_absolute_tolerances: BTreeMap<String, BTreeMap<String, Quantity>>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensitivityColumn {
    pub output: String,
    pub parameter: String,
    pub unit: String,
    pub values: Vec<f64>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum SensitivityResultSchema {
    #[serde(rename = "qspquest.sensitivity-result/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensitivityResult {
    pub schema: SensitivityResultSchema,
    pub result: RunResult,
    /// Parameter-major columns sharing the nested result's times and sides.
    pub derivatives: Vec<SensitivityColumn>,
}

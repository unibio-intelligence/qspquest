//! Explicit-grid Morris elementary-effect screening.
use crate::{
    model::Quantity,
    scan::{ScanRequest, ScanResult},
};
use serde::{Deserialize, Serialize};
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterRange {
    pub parameter: String,
    pub lower: Quantity,
    pub upper: Quantity,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorrisRequest {
    pub scan: ScanRequest,
    pub ranges: Vec<ParameterRange>,
    /// Each trajectory contains d+1 point IDs, changing each parameter once.
    pub trajectories: Vec<Vec<String>>,
    /// Even grid levels, at least four. Normalized step = p/(2*(p-1)).
    pub num_levels: u32,
    pub output: String,
    pub row: usize,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementaryEffects {
    pub parameter: String,
    pub unit: String,
    pub effects: Vec<f64>,
    pub mu: f64,
    pub mu_star: f64,
    pub sigma: f64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorrisResult {
    pub analysis_request_hash: String,
    pub scan: ScanResult,
    pub effects: Vec<ElementaryEffects>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorrisDesignRequest {
    pub run: crate::run::RunRequest,
    pub ranges: Vec<ParameterRange>,
    pub trajectory_count: usize,
    pub num_levels: u32,
    pub seed: u32,
    pub total_budgets: crate::Budgets,
    pub output: String,
    pub row: usize,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedMorrisDesign {
    pub generator: String,
    pub seed: u32,
    pub design_request_hash: String,
    pub request: MorrisRequest,
}
mod design;
pub use design::generate_morris_design;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum MorrisSchema {
    #[serde(rename = "qspquest.morris/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum MorrisResultSchema {
    #[serde(rename = "qspquest.morris-result/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum MorrisProblem {
    Explicit(MorrisRequest),
    Generated(MorrisDesignRequest),
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorrisExecutionRequest {
    pub schema: MorrisSchema,
    pub problem: MorrisProblem,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorrisDesignProvenance {
    pub generator: String,
    pub seed: u32,
    pub design_request_hash: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MorrisExecutionResult {
    pub schema: MorrisResultSchema,
    pub result: MorrisResult,
    pub design: Option<MorrisDesignProvenance>,
}

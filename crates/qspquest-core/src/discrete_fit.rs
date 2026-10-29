//! Native count likelihoods with explicit exposure units and work limits.
use crate::{fit::FitStatus, model::Quantity};
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum GammaPoissonFitSchema {
    #[serde(rename = "qspquest.gamma-poisson-fit/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountExposure {
    pub count: u32,
    pub exposure: Quantity,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GammaPoissonFitRequest {
    pub schema: GammaPoissonFitSchema,
    pub observations: Vec<CountExposure>,
    pub initial_shape: f64,
    pub shape_bounds: [f64; 2],
    /// Gamma shape-rate convention: rate beta has exposure units, while the
    /// latent Poisson intensity lambda has reciprocal exposure units.
    pub initial_rate: Quantity,
    pub rate_bounds: [Quantity; 2],
    pub max_evaluations: usize,
    pub max_iterations: usize,
    /// Infinity norm of the projected score in log(shape), log(rate).
    pub gradient_tolerance: f64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum DiscreteFitResultSchema {
    #[serde(rename = "qspquest.discrete-fit-result/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscreteFitResult {
    pub schema: DiscreteFitResultSchema,
    pub status: FitStatus,
    pub likelihood: String,
    pub parameters: std::collections::BTreeMap<String, Quantity>,
    /// Full normalized count likelihood, including count factorials.
    pub negative_log_likelihood: f64,
    pub observations: usize,
    pub evaluations: usize,
    pub iterations: usize,
    pub projected_gradient_norm: f64,
    pub active_bounds: Vec<String>,
    pub algorithm: String,
    pub fit_request_hash: String,
    pub compiler_source_sha256: String,
}
#[derive(Clone, Debug)]
pub struct GammaPoissonObjective {
    pub negative_log_likelihood: f64,
    pub log_gradient: [f64; 2],
    pub log_hessian: [[f64; 2]; 2],
}

//! Normal-mixture binomial fitting with dimensionless probit coordinates.
use crate::fit::FitStatus;
use serde::{Deserialize, Serialize};
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ProbitNormalFitSchema {
    #[serde(rename = "qspquest.probit-normal-binomial-fit/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ProbitNormalResultSchema {
    #[serde(rename = "qspquest.probit-normal-binomial-result/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinomialLitter {
    pub successes: u32,
    pub total: u32,
    pub group: usize,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbitNormalBinomialFitRequest {
    pub schema: ProbitNormalFitSchema,
    pub observations: Vec<BinomialLitter>,
    pub initial_means: [f64; 2],
    pub mean_bounds: [[f64; 2]; 2],
    pub initial_sds: [f64; 2],
    pub sd_bounds: [[f64; 2]; 2],
    pub max_evaluations: usize,
    pub max_iterations: usize,
    pub gradient_tolerance: f64,
    pub max_quadrature_evaluations: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProbitNormalObjective {
    pub negative_log_likelihood: f64,
    pub gradient: [f64; 4],
    pub hessian: [[f64; 4]; 4],
    pub quadrature_evaluations: usize,
    pub quadrature_refinement: f64,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbitNormalBinomialFitResult {
    pub schema: ProbitNormalResultSchema,
    pub status: FitStatus,
    pub means: [f64; 2],
    pub sds: [f64; 2],
    pub negative_log_likelihood: f64,
    pub projected_gradient_norm: f64,
    pub evaluations: usize,
    pub iterations: usize,
    pub quadrature_evaluations: usize,
    pub observations: usize,
    pub active_bounds: Vec<usize>,
    pub compiler_source_sha256: String,
    pub algorithm: String,
    pub fit_request_hash: String,
}

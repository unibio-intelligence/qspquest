//! Fixed hidden-state models on complete paired observation records.
use serde::{Deserialize, Serialize};

pub const MODEL_SCHEMA: &str = "qspquest.gaussian-hmm-model/v1";
pub const REQUEST_SCHEMA: &str = "qspquest.gaussian-hmm-request/v1";
pub const RESULT_SCHEMA: &str = "qspquest.gaussian-hmm-result/v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BivariateGaussian {
    pub mean: [f64; 2],
    /// Diagonal variances in the squared endpoint coordinate units.
    pub variance: [f64; 2],
    pub correlation: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaussianHmmModel {
    pub schema: String,
    pub model_id: String,
    pub states: Vec<String>,
    pub endpoint_names: [String; 2],
    /// Coordinate labels; the kernel performs no implicit unit conversion.
    pub endpoint_units: [String; 2],
    /// Unit of optional record labels; this does not define the transition clock.
    pub calendar_time_unit: Option<String>,
    pub initial_probability: Vec<f64>,
    /// Rows are the previous state, columns the next state.
    pub transition: Vec<Vec<f64>>,
    pub emissions: Vec<BivariateGaussian>,
    /// Replace supplied values below these bounds before point-density evaluation.
    pub lower_bounds: [Option<f64>; 2],
    /// Optional target for the prior first-entry distribution, including initial mass.
    pub first_entry_state: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HmmObservation {
    pub values: [f64; 2],
    /// Optional nondecreasing calendar label; never controls transitions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GaussianHmmRequest {
    pub schema: String,
    #[serde(default)]
    pub records: Vec<HmmObservation>,
    /// Integer record coordinates 0..count-1, beginning with the initial distribution.
    #[serde(default)]
    pub prior_record_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HmmRecordResult {
    pub record_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<f64>,
    pub input: [f64; 2],
    pub used: [f64; 2],
    pub prior_state_probability: Vec<f64>,
    pub filtered_state_probability: Vec<f64>,
    pub smoothed_state_probability: Vec<f64>,
    pub emission_log_density: Vec<f64>,
    pub incremental_log_likelihood: f64,
    pub cumulative_log_likelihood: f64,
    /// Prior-predictive endpoint mean after any declared observation flooring.
    pub predictive_mean: [f64; 2],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HmmPriorResult {
    pub record_index: usize,
    pub state_probability: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_entry_survival: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_entry_cdf: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GaussianHmmResult {
    pub schema: String,
    pub model_id: String,
    pub model_content_hash: String,
    pub request_content_hash: String,
    pub states: Vec<String>,
    pub endpoint_names: [String; 2],
    pub endpoint_units: [String; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_time_unit: Option<String>,
    pub evaluated_initial_probability: Vec<f64>,
    pub evaluated_transition: Vec<Vec<f64>>,
    pub probability_policy: String,
    pub transition_coordinate: String,
    pub records: Vec<HmmRecordResult>,
    pub prior: Vec<HmmPriorResult>,
    pub log_likelihood: Option<f64>,
    pub viterbi_state_index: Vec<usize>,
    pub viterbi_log_joint: Option<f64>,
}

//! Explicit bounded parameter scans; no implicit grid expansion.
use crate::{
    model::Quantity,
    run::{RunRequest, RunResult},
    Budgets,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ScanSchema {
    #[serde(rename = "qspquest.scan/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ScanResultSchema {
    #[serde(rename = "qspquest.scan-result/v0.1")]
    V01,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanPoint {
    pub id: String,
    pub parameters: BTreeMap<String, Quantity>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanRequest {
    pub schema: ScanSchema,
    pub run: RunRequest,
    pub points: Vec<ScanPoint>,
    pub total_budgets: Budgets,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanPointResult {
    pub id: String,
    pub parameters: BTreeMap<String, Quantity>,
    pub result: RunResult,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanResult {
    pub schema: ScanResultSchema,
    pub scan_request_hash: String,
    pub results: Vec<ScanPointResult>,
}

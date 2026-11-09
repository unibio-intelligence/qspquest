//! Native analysis workflows built on bounded native simulations.
//!
//! Every analysis receives a JSON argument object and returns a JSON record.
//! Simulations run through [`CompiledDocument::execute`]; optimization,
//! sampling, quantiles and linear algebra run here, so every binding shares
//! one implementation. Requests whose order is scientifically meaningful
//! (parameters, regimens) are arrays, never JSON objects.
pub(crate) mod common;
mod control;
mod derivatives;
mod diagnostics;
mod estimation;
mod exposure;
pub(crate) mod linalg;
mod optimize;
mod planning;
mod population;
pub(crate) mod rng;
pub(crate) mod saem;
mod stats;
mod uncertainty;
mod work;

pub use common::AnalysisError;
pub use control::{CancelFlag, Control, NoControl};
pub use exposure::{compare_solved, comparison_is_stochastic, sampled_auc};
pub use planning::{accepted_arguments, default_max_simulations, plan, plan_request, PLAN_SCHEMA};

use crate::document::CompiledDocument;
use serde_json::Value;
use std::sync::Arc;

/// Version label recorded in every analysis identity.
pub const ANALYSIS_VERSION: &str = "qspquest.analysis/0.2";

/// Operations that need a compiled model.
pub const MODEL_OPERATIONS: [&str; 7] = [
    "fit_data",
    "fit_population",
    "visual_predictive_check",
    "profile_likelihood",
    "bootstrap_population",
    "compare_regimens",
    "event_sensitivities",
];

/// Operations on an existing result record; no model is required.
pub const RECORD_OPERATIONS: [&str; 2] = ["exposure_summary", "fit_diagnostics"];

/// Execute one named analysis. `model` is required for [`MODEL_OPERATIONS`].
pub fn execute(
    model: Option<&Arc<CompiledDocument>>,
    operation: &str,
    arguments: &Value,
    control: &mut dyn Control,
) -> Result<Value, AnalysisError> {
    if !arguments.is_object() {
        return common::fail("analysis arguments must be a JSON object");
    }
    if RECORD_OPERATIONS.contains(&operation) {
        return match operation {
            "exposure_summary" => exposure::exposure_summary_request(arguments),
            _ => diagnostics::fit_diagnostics(arguments),
        };
    }
    if !MODEL_OPERATIONS.contains(&operation) {
        return common::fail(format!(
            "unknown analysis operation {operation}; expected one of {}",
            MODEL_OPERATIONS
                .iter()
                .chain(RECORD_OPERATIONS.iter())
                .copied()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let Some(model) = model else {
        return common::fail(format!("{operation} requires a compiled model"));
    };
    match operation {
        "fit_data" => estimation::fit_data(model, arguments, None),
        "fit_population" => population::fit_population(model, arguments, control, None),
        "visual_predictive_check" => {
            diagnostics::visual_predictive_check(model, arguments, control)
        }
        "profile_likelihood" => uncertainty::profile_likelihood(model, arguments),
        "bootstrap_population" => uncertainty::bootstrap_population(model, arguments, control),
        "compare_regimens" => exposure::compare_regimens(model, arguments),
        _ => derivatives::event_sensitivities(model, arguments),
    }
}

/// Split a `qspquest.analysis/v0.2` envelope into its operation and arguments.
fn envelope(request: &Value) -> Result<(&str, Value), AnalysisError> {
    let Some(object) = request.as_object() else {
        return common::fail("analysis request must be a JSON object");
    };
    if object.get("schema").and_then(Value::as_str) != Some("qspquest.analysis/v0.2") {
        return common::fail("analysis request schema must be qspquest.analysis/v0.2");
    }
    if object
        .keys()
        .any(|k| !["schema", "operation", "arguments"].contains(&k.as_str()))
    {
        return common::fail("analysis request accepts only schema, operation and arguments");
    }
    let Some(operation) = object.get("operation").and_then(Value::as_str) else {
        return common::fail("analysis request requires an operation name");
    };
    let arguments = object
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    Ok((operation, arguments))
}

/// Execute a `qspquest.analysis/v0.2` envelope:
/// `{"schema": ..., "operation": ..., "arguments": {...}}`.
pub fn execute_request(
    model: Option<&Arc<CompiledDocument>>,
    request: &Value,
    control: &mut dyn Control,
) -> Result<Value, AnalysisError> {
    let (operation, arguments) = envelope(request)?;
    execute(model, operation, &arguments, control)
}

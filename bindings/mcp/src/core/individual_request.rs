//! Individual-fit admission and assembly; native preparation owns statistical semantics.
use super::input_shapes::{child, quantity, Failure};
use qspquest_core::{
    fit::{FitBound, FitSchema, GaussianObservation},
    model::{ModelDocument, Quantity},
    run::SensitivitySchema,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn fail(path: &str, message: impl Into<String>, hint: &str) -> Failure {
    Failure::at(path.to_owned(), message, hint)
}
const BOUND_HINT: &str = "Supply one declared native parameter with lower and upper quantities compatible with its model unit; native preparation checks bounds and transforms.";
const OBSERVATION_HINT: &str = "Supply a native result-grid row, output, measured value quantity and Gaussian error contract; preserve the observation grid, units and protocol.";
#[derive(Serialize)]
struct Request {
    schema: FitSchema,
    problem: Problem,
}
#[derive(Serialize)]
struct Problem {
    kind: &'static str,
    request: Fit,
}
#[derive(Serialize)]
struct Fit {
    objective: Objective,
    bounds: Vec<Value>,
    max_evaluations: Value,
    max_iterations: Value,
    gradient_tolerance: f64,
}
#[derive(Serialize)]
struct Objective {
    simulation: Simulation,
    observations: Vec<Value>,
    priors: Vec<Value>,
}
#[derive(Serialize)]
struct Simulation {
    schema: SensitivitySchema,
    run: Value,
    with_respect_to: Vec<String>,
    derivative_absolute_tolerances: BTreeMap<String, BTreeMap<String, Quantity>>,
}
fn bound(value: &Value, index: usize) -> Result<FitBound, Failure> {
    let path = format!("/parameters_to_fit/{index}");
    let fields = value.as_object().ok_or_else(|| {
        fail(
            &path,
            "Fitted parameter bounds must be an object",
            BOUND_HINT,
        )
    })?;
    if let Some(key) = fields
        .keys()
        .find(|key| !["parameter", "lower", "upper"].contains(&key.as_str()))
    {
        return Err(fail(
            &child(&path, key),
            "Unknown fitted parameter field",
            BOUND_HINT,
        ));
    }
    if !value["parameter"].is_string() {
        return Err(fail(
            &child(&path, "parameter"),
            "parameter name required",
            BOUND_HINT,
        ));
    }
    for key in ["lower", "upper"] {
        quantity(
            &value[key],
            &child(&path, key),
            &format!("parameters_to_fit[{index}].{key}"),
        )?;
    }
    serde_json::from_value(value.clone()).map_err(|e| fail(&path, e.to_string(), BOUND_HINT))
}
pub(crate) fn observation(value: &Value, index: usize) -> Result<GaussianObservation, Failure> {
    let path = format!("/observations/{index}");
    let fields = value
        .as_object()
        .ok_or_else(|| fail(&path, "Observation must be an object", OBSERVATION_HINT))?;
    if let Some(key) = fields
        .keys()
        .find(|key| !["row", "output", "value", "error", "censoring"].contains(&key.as_str()))
    {
        return Err(fail(
            &child(&path, key),
            "Unknown Gaussian observation field",
            OBSERVATION_HINT,
        ));
    }
    if value["row"].as_u64().is_none() {
        return Err(fail(
            &child(&path, "row"),
            "Observation row must be a nonnegative integer",
            OBSERVATION_HINT,
        ));
    }
    if !value["output"].is_string() {
        return Err(fail(
            &child(&path, "output"),
            "Observation output is required",
            OBSERVATION_HINT,
        ));
    }
    quantity(
        &value["value"],
        &child(&path, "value"),
        &format!("observations[{index}].value"),
    )?;
    let error_path = child(&path, "error");
    let error = value["error"].as_object().ok_or_else(|| {
        fail(
            &error_path,
            "Observation error must be an object",
            OBSERVATION_HINT,
        )
    })?;
    if let Some(key) = error
        .keys()
        .find(|key| !["additive_sd", "proportional_sd", "model"].contains(&key.as_str()))
    {
        return Err(fail(
            &child(&error_path, key),
            "Unknown Gaussian error field",
            OBSERVATION_HINT,
        ));
    }
    quantity(
        &value["error"]["additive_sd"],
        &child(&error_path, "additive_sd"),
        &format!("observations[{index}].error.additive_sd"),
    )?;
    if value["error"]["proportional_sd"]
        .as_f64()
        .is_none_or(|n| !n.is_finite())
    {
        return Err(fail(
            &child(&error_path, "proportional_sd"),
            "Proportional standard deviation must be finite",
            OBSERVATION_HINT,
        ));
    }
    serde_json::from_value(value.clone()).map_err(|e| fail(&path, e.to_string(), OBSERVATION_HINT))
}

pub(crate) fn prepare(args: &Value, run: Value, model: &Value) -> Result<Value, Failure> {
    // The native override retains its complete submitted representation.
    if let Some(request) = args.get("fit_request") {
        return Ok(request.clone());
    }
    let raw_bounds = args["parameters_to_fit"].as_array().ok_or_else(|| {
        fail(
            "/parameters_to_fit",
            "parameters_to_fit requires parameter/lower/upper quantities",
            BOUND_HINT,
        )
    })?;
    if raw_bounds.is_empty() || raw_bounds.len() > 32 {
        return Err(fail(
            "/parameters_to_fit",
            "individual fitting requires 1..32 fitted parameters",
            BOUND_HINT,
        ));
    }
    // Keep the native sensitivity admission ahead of tolerance-map allocation.
    let state_count = crate::library::display_states(model).len();
    if state_count
        .checked_mul(raw_bounds.len() + 1)
        .is_none_or(|n| n > 1024)
    {
        return Err(fail("/parameters_to_fit", "augmented sensitivity model exceeds 1024 states",
            "Choose a scientifically justified smaller sensitivity problem within the native 1024-state limit."));
    }
    let document: ModelDocument = serde_json::from_value(model.clone()).map_err(|e| {
        fail(
            "/model",
            e.to_string(),
            "Use a complete supported model document; inspect its parameters and state units.",
        )
    })?;
    let mut states: Vec<(&str, &str)> = document
        .states
        .iter()
        .map(|s| (s.name.as_str(), s.unit.as_str()))
        .collect();
    if let Some(pk) = &document.linear_pk {
        states.extend(
            pk.compartments
                .iter()
                .map(|c| (c.state.as_str(), pk.amount_unit.as_str())),
        );
        if let Some(depot) = &pk.depot {
            states.push((depot.state.as_str(), pk.amount_unit.as_str()));
        }
    }
    let mut names = Vec::with_capacity(raw_bounds.len());
    let mut distinct = BTreeSet::new();
    let mut derivative = BTreeMap::new();
    for (index, raw) in raw_bounds.iter().enumerate() {
        let bound = bound(raw, index)?;
        let path = format!("/parameters_to_fit/{index}/parameter");
        let parameter = document
            .parameters
            .iter()
            .find(|p| p.name == bound.parameter)
            .ok_or_else(|| fail(&path, "unknown fitted parameter", BOUND_HINT))?;
        if !distinct.insert(bound.parameter.clone()) {
            return Err(fail(&path, "Fitted parameter is repeated", "Supply each fitted native parameter once; do not deduplicate a scientific request automatically."));
        }
        let tolerances = states
            .iter()
            .map(|(name, unit)| {
                (
                    (*name).to_owned(),
                    Quantity {
                        value: 1e-10,
                        unit: format!("({unit})/({})", parameter.default.unit),
                    },
                )
            })
            .collect();
        derivative.insert(bound.parameter.clone(), tolerances);
        names.push(bound.parameter);
    }
    let raw_observations = args["observations"].as_array().ok_or_else(|| {
        fail(
            "/observations",
            "observations requires row/output/value/error",
            OBSERVATION_HINT,
        )
    })?;
    for (index, raw) in raw_observations.iter().enumerate() {
        observation(raw, index)?;
    }
    let request = Request {
        schema: FitSchema::V01,
        problem: Problem {
            kind: "individual",
            request: Fit {
                objective: Objective {
                    simulation: Simulation {
                        schema: SensitivitySchema::V01,
                        run,
                        with_respect_to: names,
                        derivative_absolute_tolerances: derivative,
                    },
                    observations: raw_observations.clone(),
                    priors: Vec::new(),
                },
                bounds: raw_bounds.clone(),
                max_evaluations: args.get("max_evaluations").cloned().unwrap_or(json!(400)),
                max_iterations: args.get("max_iterations").cloned().unwrap_or(json!(100)),
                gradient_tolerance: 1e-5,
            },
        },
    };
    serde_json::to_value(request)
        .map_err(|e| fail("", e.to_string(), "Correct the native fit request."))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Value {
        serde_json::to_value(
            super::super::compute::model(include_str!("../../assets/templates/oral.qsp")).unwrap(),
        )
        .unwrap()
    }
    fn args() -> Value {
        json!({"parameters_to_fit":[{"parameter":"cl","lower":{"value":0.1,"unit":"L/h"},"upper":{"value":2,"unit":"L/h"}}],
            "observations":[{"row":1,"output":"cp","value":{"value":0.8,"unit":"mg/L"},"error":{"additive_sd":{"value":0.1,"unit":"mg/L"},"proportional_sd":0}}]})
    }
    #[test]
    fn complete_wire_request_preserves_raw_bounds_observations_and_run() {
        let input = args();
        let run = json!({"opaque":"preserved","schema":"qspquest.run/v0.1"});
        let result = prepare(&input, run.clone(), &model()).unwrap();
        assert_eq!(
            result,
            json!({"schema":"qspquest.fit/v0.1","problem":{"kind":"individual","request":{
            "objective":{"simulation":{"schema":"qspquest.sensitivity/v0.1","run":run,"with_respect_to":["cl"],
                "derivative_absolute_tolerances":{"cl":{"central":{"value":1e-10,"unit":"(mg)/(L/h)"},"depot":{"value":1e-10,"unit":"(mg)/(L/h)"}}}},
                "observations":input["observations"],"priors":[]},"bounds":input["parameters_to_fit"],
            "max_evaluations":400,"max_iterations":100,"gradient_tolerance":1e-5}}})
        );
    }
    #[test]
    fn native_override_and_optimizer_values_are_preserved() {
        let native = json!({"schema":"qspquest.fit/v0.1","opaque":{"kept":true}});
        assert_eq!(
            prepare(&json!({"fit_request":native}), Value::Null, &Value::Null).unwrap(),
            native
        );
        let mut input = args();
        input["max_evaluations"] = json!(123);
        input["max_iterations"] = json!(45);
        let result = prepare(&input, json!({}), &model()).unwrap();
        assert_eq!(result["problem"]["request"]["max_evaluations"], 123);
        assert_eq!(result["problem"]["request"]["max_iterations"], 45);
    }
    #[test]
    fn malformed_bounds_unknown_names_and_duplicates_are_located() {
        for (field, value, pointer) in [
            ("parameter", json!(123), "/parameters_to_fit/0/parameter"),
            (
                "parameter",
                json!("unknown"),
                "/parameters_to_fit/0/parameter",
            ),
            (
                "lower",
                json!({"value":1}),
                "/parameters_to_fit/0/lower/unit",
            ),
            ("extra", json!(true), "/parameters_to_fit/0/extra"),
        ] {
            let mut input = args();
            input["parameters_to_fit"][0][field] = value;
            let before = input.clone();
            let error = prepare(&input, json!({}), &model())
                .unwrap_err()
                .into_value("fit_request");
            assert_eq!(error["error"]["expression"], pointer);
            assert_eq!(input, before);
        }
        let mut input = args();
        let duplicate = input["parameters_to_fit"][0].clone();
        input["parameters_to_fit"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        let error = prepare(&input, json!({}), &model())
            .unwrap_err()
            .into_value("fit_request");
        assert_eq!(
            error["error"]["expression"],
            "/parameters_to_fit/1/parameter"
        );
    }
    #[test]
    fn malformed_observation_contracts_have_source_locations() {
        for (field, value, pointer) in [
            ("row", json!(-1), "/observations/0/row"),
            ("output", Value::Null, "/observations/0/output"),
            (
                "value",
                json!({"value":"bad","unit":"mg/L"}),
                "/observations/0/value/value",
            ),
            (
                "error",
                json!({"additive_sd":{"value":1,"unit":"mg/L"},"proportional_sd":"bad"}),
                "/observations/0/error/proportional_sd",
            ),
        ] {
            let mut input = args();
            input["observations"][0][field] = value;
            let error = prepare(&input, json!({}), &model())
                .unwrap_err()
                .into_value("fit_request");
            assert_eq!(error["error"]["expression"], pointer);
        }
    }
    #[test]
    fn sensitivity_limits_precede_model_decode_and_tolerance_allocation() {
        let model = json!({"states":vec![json!({"name":"x","unit":"1"});513]});
        for bounds in [
            vec![json!({"parameter":"p"}); 33],
            vec![json!({"parameter":"p"})],
        ] {
            let error = prepare(&json!({"parameters_to_fit":bounds}), json!({}), &model)
                .unwrap_err()
                .into_value("fit_request");
            assert_eq!(error["error"]["expression"], "/parameters_to_fit");
            assert!(error["error"]["message"]
                .as_str()
                .unwrap()
                .contains(if bounds.len() == 33 {
                    "1..32"
                } else {
                    "1024 states"
                }));
        }
    }
}

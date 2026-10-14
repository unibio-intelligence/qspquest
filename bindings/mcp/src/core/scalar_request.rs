//! Scalar-fit input assembly; native preparation binds units, domains and the run.
use super::input_shapes::{child, quantity, Failure};
use qspquest_core::fit::{GaussianErrorModel, ScalarFitSchema};
use serde::Serialize;
use serde_json::{json, Value};

fn fail(path: &str, message: impl Into<String>, hint: &str) -> Failure {
    Failure::at(path.to_owned(), message, hint)
}
#[derive(Serialize)]
struct Request {
    schema: ScalarFitSchema,
    run: Value,
    parameter: String,
    lower: Value,
    upper: Value,
    observations: Vec<Value>,
    max_iterations: Value,
    absolute_tolerance: f64,
}
pub(crate) fn prepare(args: &Value, run: Value) -> Result<Value, Failure> {
    if let Some(request) = args.get("fit_request").filter(|v| !v.is_null()) {
        return Ok(request.clone());
    }
    let parameter = args["parameter"].as_str().ok_or_else(|| fail(
        "/parameter", "parameter required", "Supply one declared estimated native parameter; native preparation checks whether it can be fitted."))?.to_owned();
    let bounds = args["bounds"].as_object().ok_or_else(|| {
        fail(
            "/bounds",
            "bounds must contain lower and upper quantities",
            "Supply finite lower and upper quantities compatible with the parameter unit.",
        )
    })?;
    if let Some(key) = bounds
        .keys()
        .find(|key| !["lower", "upper"].contains(&key.as_str()))
    {
        return Err(fail(
            &child("/bounds", key),
            "Unknown scalar bound field",
            "Use lower and upper only; native preparation checks the fitted box.",
        ));
    }
    for field in ["lower", "upper"] {
        quantity(
            &args["bounds"][field],
            &child("/bounds", field),
            &format!("bounds.{field}"),
        )?;
    }
    let observations = args["observations"].as_array().ok_or_else(|| {
        fail(
            "/observations",
            "observations must be an array",
            "Supply Gaussian observation records with native row, output, value and error.",
        )
    })?;
    if observations.is_empty() || observations.len() > 500 {
        return Err(fail("/observations", "scalar fitting requires 1..500 observations",
            "Supply the complete scientifically intended set within the scalar contract; choose another supported workflow when more observations are required."));
    }
    for (index, raw) in observations.iter().enumerate() {
        let observation = super::individual_request::observation(raw, index)?;
        let path = format!("/observations/{index}");
        if observation.error.model != GaussianErrorModel::Normal {
            return Err(fail(&child(&child(&path, "error"), "model"), "scalar fit supports normal additive error",
                "Use a normal additive error contract only when scientifically appropriate; use a supported individual fit for another native Gaussian error contract."));
        }
        if observation.censoring.is_some() {
            return Err(fail(&child(&path, "censoring"), "scalar fit censored observations are not implemented",
                "Use a supported native individual censored-observation workflow; do not replace censored measurements with exact observations."));
        }
        if observation.error.proportional_sd != 0.0 {
            return Err(fail(&child(&child(&path, "error"), "proportional_sd"), "scalar fit requires additive error without a proportional component",
                "Use a supported individual fit when the intended error contract has a proportional component."));
        }
        if observation.error.additive_sd.value <= 0.0 {
            return Err(fail(
                &child(&child(&child(&path, "error"), "additive_sd"), "value"),
                "scalar fit requires positive additive SD",
                "Supply a positive standard deviation compatible with the observed output unit.",
            ));
        }
    }
    let max_iterations = args
        .get("max_iterations")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or(json!(100));
    if max_iterations
        .as_u64()
        .is_none_or(|n| !(1..=1000).contains(&n))
    {
        return Err(fail("/max_iterations", "scalar fit iterations must be 1..1000",
            "Supply an integer from 1 through 1000; the deployment profile may impose a smaller inline limit."));
    }
    let request = Request {
        schema: ScalarFitSchema::V01,
        run,
        parameter,
        lower: args["bounds"]["lower"].clone(),
        upper: args["bounds"]["upper"].clone(),
        observations: observations.clone(),
        max_iterations,
        absolute_tolerance: 1e-8,
    };
    serde_json::to_value(request)
        .map_err(|e| fail("", e.to_string(), "Correct the native scalar fit request."))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args() -> Value {
        json!({"parameter":"cl","bounds":{"lower":{"value":1,"unit":"L/h"},"upper":{"value":2,"unit":"L/h"}},
            "observations":[{"row":0,"output":"cp","value":{"value":1,"unit":"mg/L"},"error":{"additive_sd":{"value":0.1,"unit":"mg/L"},"proportional_sd":0}}]})
    }
    #[test]
    fn complete_envelope_preserves_native_values_and_defaults() {
        let input = args();
        let run = json!({"opaque":{"preserved":true}});
        let result = prepare(&input, run.clone()).unwrap();
        assert_eq!(
            result,
            json!({"schema":"qspquest.scalar-fit/v0.1","run":run,"parameter":"cl",
            "lower":input["bounds"]["lower"],"upper":input["bounds"]["upper"],
            "observations":input["observations"],"max_iterations":100,"absolute_tolerance":1e-8})
        );
        let mut omitted = input.clone();
        omitted["max_iterations"] = Value::Null;
        assert_eq!(prepare(&omitted, json!({})).unwrap()["max_iterations"], 100);
    }
    #[test]
    fn complete_native_override_preserves_every_submitted_field() {
        let request = json!({"schema":"qspquest.scalar-fit/v0.1","opaque":{"retained":true}});
        assert_eq!(
            prepare(&json!({"fit_request":request}), Value::Null).unwrap(),
            request
        );
    }
    #[test]
    fn malformed_bounds_and_observation_fields_are_located() {
        for (input, pointer) in [
            (json!({"bounds":{}}), "/parameter"),
            (json!({"parameter":"cl","bounds":[]}), "/bounds"),
            (
                json!({"parameter":"cl","bounds":{"lower":{"value":1},"upper":{"value":2,"unit":"L/h"}}}),
                "/bounds/lower/unit",
            ),
        ] {
            let before = input.clone();
            let error = prepare(&input, json!({}))
                .unwrap_err()
                .into_value("fit_request");
            assert_eq!(error["error"]["expression"], pointer);
            assert_eq!(input, before);
        }
        let mut input = args();
        input["observations"][0]["value"]["unit"] = json!(123);
        assert_eq!(
            prepare(&input, json!({}))
                .unwrap_err()
                .into_value("fit_request")["error"]["expression"],
            "/observations/0/value/unit"
        );
    }
    #[test]
    fn unsupported_error_and_censoring_contracts_are_not_repaired() {
        for (key, value, pointer) in [
            ("model", json!("lognormal"), "/observations/0/error/model"),
            (
                "proportional_sd",
                json!(0.2),
                "/observations/0/error/proportional_sd",
            ),
            (
                "additive_sd",
                json!({"value":0,"unit":"mg/L"}),
                "/observations/0/error/additive_sd/value",
            ),
        ] {
            let mut input = args();
            input["observations"][0]["error"][key] = value;
            let before = input.clone();
            assert_eq!(
                prepare(&input, json!({}))
                    .unwrap_err()
                    .into_value("fit_request")["error"]["expression"],
                pointer
            );
            assert_eq!(input, before);
        }
        let mut input = args();
        input["observations"][0]["censoring"] = json!("left");
        assert_eq!(
            prepare(&input, json!({}))
                .unwrap_err()
                .into_value("fit_request")["error"]["expression"],
            "/observations/0/censoring"
        );
    }
    #[test]
    fn native_observation_and_iteration_denominators_are_bounded() {
        for value in [json!(0), json!(1001), json!("many")] {
            let mut input = args();
            input["max_iterations"] = value;
            assert_eq!(
                prepare(&input, json!({}))
                    .unwrap_err()
                    .into_value("fit_request")["error"]["expression"],
                "/max_iterations"
            );
        }
        for count in [0, 501] {
            let mut input = args();
            let row = input["observations"][0].clone();
            input["observations"] = json!(vec![row; count]);
            assert_eq!(
                prepare(&input, json!({}))
                    .unwrap_err()
                    .into_value("fit_request")["error"]["expression"],
                "/observations"
            );
        }
        let mut input = args();
        input["max_iterations"] = json!(1000);
        assert_eq!(prepare(&input, json!({})).unwrap()["max_iterations"], 1000);
    }
}

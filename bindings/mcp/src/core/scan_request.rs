//! Bounded scan input assembly without execution or transport side effects.
use qspquest_core::{model::Quantity, scan::ScanSchema, Budgets};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug)]
struct InputQuantity {
    raw: Value,
    native: Quantity,
}
impl InputQuantity {
    fn parse(raw: Value, path: &str) -> Result<Self, Value> {
        let display = match path {
            "/range/lower" => "range.lower",
            "/range/upper" => "range.upper",
            _ => path,
        };
        super::input_shapes::quantity(&raw, path, display).map_err(|e| e.into_value("request"))?;
        let native = serde_json::from_value(raw.clone()).map_err(|e| {
            refusal(
                "request",
                path,
                e.to_string(),
                "Supply a finite value and explicit unit.",
            )
        })?;
        Ok(Self { raw, native })
    }
}

pub(crate) struct Points {
    parameter: String,
    values: Vec<InputQuantity>,
}
#[derive(Serialize)]
struct Point {
    id: String,
    parameters: BTreeMap<String, Value>,
}
#[derive(Serialize)]
struct Request {
    schema: ScanSchema,
    run: Value,
    points: Vec<Point>,
    total_budgets: Budgets,
}
fn refusal(code: &str, pointer: &str, message: impl Into<String>, suggestion: &str) -> Value {
    json!({"error":{"code":code,"message":message.into(),"expression":pointer,"suggestion":suggestion}})
}
fn grid_failure() -> Value {
    refusal("request", "/range", "Range interpolation must produce a finite grid",
        "Supply explicit finite values or a narrower finite range; no scan points were submitted for execution.")
}
pub(crate) fn prepare(args: &Value) -> Result<Points, Value> {
    let parameter = args["parameter"]
        .as_str()
        .ok_or_else(|| {
            refusal(
                "request",
                "/parameter",
                "parameter required",
                "Supply a parameter declared by the model.",
            )
        })?
        .to_owned();
    let values = if let Some(values) = args.get("values").filter(|v| !v.is_null()) {
        let values =
            values.as_array().ok_or_else(|| {
                refusal("request", "/values", "values must be an array of quantities",
                "Supply explicit quantity objects in an array, or omit values and supply range.")
            })?;
        if values.is_empty() || values.len() > 25 {
            return Err(refusal("budget", "/values", "Inline scans require 1..25 points. Larger managed scans must be jobs.",
                "Supply 1 through 25 scientifically intended scan values; use a supported job route for a larger scan."));
        }
        values
            .iter()
            .enumerate()
            .map(|(i, value)| InputQuantity::parse(value.clone(), &format!("/values/{i}")))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let range = &args["range"];
        let count = range["count"].as_u64().ok_or_else(|| {
            refusal(
                "request",
                "/range/count",
                "range.count required",
                "Supply an integer count from 2 through 25.",
            )
        })?;
        if !(2..=25).contains(&count) {
            return Err(refusal("budget", "/range/count", "inline range count must be 2..25",
                "Supply an integer count from 2 through 25, or explicit values for a one-point scan."));
        }
        let lower = InputQuantity::parse(range["lower"].clone(), "/range/lower")?;
        let upper = InputQuantity::parse(range["upper"].clone(), "/range/upper")?;
        if lower.native.unit != upper.native.unit {
            return Err(refusal("units", "/range/upper/unit", "range endpoints must use the same unit",
                "Express both endpoints in the same compatible unit; native preparation checks the parameter's dimension."));
        }
        let start = lower.native.value;
        let end = upper.native.value;
        let difference = end - start;
        if !difference.is_finite() {
            return Err(grid_failure());
        }
        let mut values = Vec::with_capacity(count as usize);
        for index in 0..count {
            // Preserve the specified endpoint interpolation and arithmetic order.
            let value = start + difference * index as f64 / (count - 1) as f64;
            if !value.is_finite() {
                return Err(grid_failure());
            }
            values.push(InputQuantity::parse(
                json!({"value":value,"unit":&lower.native.unit}),
                "/range",
            )?);
        }
        values
    };
    Ok(Points { parameter, values })
}
impl Points {
    /// Workload admission supplies and validates the run budgets before assembly.
    pub(crate) fn into_request(self, run: Value) -> Result<Value, Value> {
        let budget: Budgets = serde_json::from_value(run["budgets"].clone()).map_err(|e| {
            refusal(
                "request",
                "/request/budgets",
                e.to_string(),
                "Supply admitted solver_callbacks, events and output_values integer budgets.",
            )
        })?;
        let count = self.values.len();
        let overflow = || {
            refusal(
                "budget",
                "/total_budgets",
                "Aggregate scan budgets overflow",
                "Reduce the per-point budgets or point count; no partial request was produced.",
            )
        };
        let total_budgets = Budgets {
            solver_callbacks: budget
                .solver_callbacks
                .checked_mul(count as u64)
                .ok_or_else(overflow)?,
            events: budget.events.checked_mul(count).ok_or_else(overflow)?,
            output_values: budget
                .output_values
                .checked_mul(count)
                .ok_or_else(overflow)?,
        };
        let points = self
            .values
            .into_iter()
            .enumerate()
            .map(|(i, value)| Point {
                id: i.to_string(),
                // Keep explicit submitted numeric representation and units until binding.
                parameters: BTreeMap::from([(self.parameter.clone(), value.raw)]),
            })
            .collect();
        serde_json::to_value(Request {
            schema: ScanSchema::V01,
            run,
            points,
            total_budgets,
        })
        .map_err(|e| {
            refusal(
                "request",
                "",
                e.to_string(),
                "Correct the native scan request.",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run() -> Value {
        json!({"schema":"qspquest.run/v0.1","budgets":{"solver_callbacks":1000,"events":10,"output_values":2000},"opaque":{"preserved":true}})
    }
    #[test]
    fn explicit_points_preserve_values_units_ids_and_complete_run() {
        let args = json!({"parameter":"cl/~","values":[{"value":1,"unit":"L/h"},{"value":2.5,"unit":"L/h"}],"range":{"count":99}});
        let before = args.clone();
        let original = run();
        let output = prepare(&args)
            .unwrap()
            .into_request(original.clone())
            .unwrap();
        assert_eq!(
            output,
            json!({"schema":"qspquest.scan/v0.1","run":original,
            "points":[{"id":"0","parameters":{"cl/~":{"value":1,"unit":"L/h"}}},
                {"id":"1","parameters":{"cl/~":{"value":2.5,"unit":"L/h"}}}],
            "total_budgets":{"solver_callbacks":2000,"events":20,"output_values":4000}})
        );
        assert_eq!(args, before);
    }
    #[test]
    fn range_formula_keeps_endpoint_order_and_original_grid() {
        for (lower, upper, expected) in [
            (1.0, 4.0, vec![1.0, 2.0, 3.0, 4.0]),
            (4.0, 1.0, vec![4.0, 3.0, 2.0, 1.0]),
            (2.0, 2.0, vec![2.0, 2.0, 2.0, 2.0]),
        ] {
            let args = json!({"parameter":"cl","range":{"count":4,"lower":{"value":lower,"unit":"L/h"},"upper":{"value":upper,"unit":"L/h"}}});
            let output = prepare(&args).unwrap().into_request(run()).unwrap();
            let actual: Vec<f64> = output["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["parameters"]["cl"]["value"].as_f64().unwrap())
                .collect();
            assert_eq!(actual, expected);
        }
    }
    #[test]
    fn malformed_inputs_and_point_limits_have_exact_locations() {
        for (args, pointer, code) in [
            (json!({"values":[]}), "/parameter", "request"),
            (json!({"parameter":"cl","values":{}}), "/values", "request"),
            (json!({"parameter":"cl","values":[]}), "/values", "budget"),
            (
                json!({"parameter":"cl","values":[{"value":1}]}),
                "/values/0/unit",
                "request",
            ),
            (
                json!({"parameter":"cl","range":{"count":1}}),
                "/range/count",
                "budget",
            ),
            (
                json!({"parameter":"cl","range":{"count":26}}),
                "/range/count",
                "budget",
            ),
            (
                json!({"parameter":"cl","range":{"count":2,"lower":{"value":1,"unit":"L/h"},"upper":{"value":2,"unit":"mg"}}}),
                "/range/upper/unit",
                "units",
            ),
        ] {
            let before = args.clone();
            let error = prepare(&args).err().unwrap();
            assert_eq!(error["error"]["expression"], pointer);
            assert_eq!(error["error"]["code"], code);
            assert_eq!(args, before);
        }
    }
    #[test]
    fn nonfinite_generated_grid_refuses_without_serializing_null_points() {
        for (lower, upper, count) in [(-f64::MAX, f64::MAX, 2), (0.0, f64::MAX, 25)] {
            let args = json!({"parameter":"cl","range":{"count":count,"lower":{"value":lower,"unit":"L/h"},"upper":{"value":upper,"unit":"L/h"}}});
            let error = prepare(&args).err().unwrap();
            assert_eq!(error["error"]["expression"], "/range");
            assert!(error["points"].is_null());
        }
    }
    #[test]
    fn missing_and_overflowing_budgets_refuse_without_partial_envelopes() {
        let args =
            json!({"parameter":"cl","values":[{"value":1,"unit":"L/h"},{"value":2,"unit":"L/h"}]});
        let error = prepare(&args).unwrap().into_request(json!({})).unwrap_err();
        assert_eq!(error["error"]["expression"], "/request/budgets");
        let mut original = run();
        original["budgets"]["solver_callbacks"] = json!(u64::MAX);
        let error = prepare(&args).unwrap().into_request(original).unwrap_err();
        assert_eq!(error["error"]["expression"], "/total_budgets");
        assert!(error["points"].is_null());
    }
}

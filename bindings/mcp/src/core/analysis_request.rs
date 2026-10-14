//! Structural analysis admission; native preparation owns dimensions and design validity.
use super::input_shapes::{child, quantity, Failure};
use qspquest_core::{morris::MorrisExecutionRequest, run::SteadyStateRequest};
use serde_json::Value;

const HINT: &str = "Supply the complete native analysis request; preserve its intended units, grid, observation rows and workload budgets. Inspect the operation's request schema.";
fn fail(path: &str, message: impl Into<String>) -> Failure {
    Failure::at(path.to_owned(), message, HINT)
}
fn object<'a>(
    value: &'a Value,
    path: &str,
    fields: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, Failure> {
    let object = value
        .as_object()
        .ok_or_else(|| fail(path, "Expected an object"))?;
    if let Some(name) = object.keys().find(|name| !fields.contains(&name.as_str())) {
        return Err(fail(&child(path, name), "Unknown analysis request field"));
    }
    Ok(object)
}
fn array<'a>(value: &'a Value, path: &str) -> Result<&'a Vec<Value>, Failure> {
    value
        .as_array()
        .ok_or_else(|| fail(path, "Expected an array"))
}
fn string(value: &Value, path: &str) -> Result<(), Failure> {
    value
        .as_str()
        .map(|_| ())
        .ok_or_else(|| fail(path, "Expected a string"))
}
fn integer(value: &Value, path: &str, maximum: u64) -> Result<(), Failure> {
    if value.as_u64().is_some_and(|number| number <= maximum) {
        Ok(())
    } else {
        Err(fail(
            path,
            "Expected an unsigned integer representable by the native request field",
        ))
    }
}
fn budgets(value: &Value, path: &str) -> Result<(), Failure> {
    object(
        value,
        path,
        &["solver_callbacks", "events", "output_values"],
    )?;
    for field in ["solver_callbacks", "events", "output_values"] {
        let maximum = if field == "solver_callbacks" {
            u64::MAX
        } else {
            usize::MAX as u64
        };
        integer(&value[field], &child(path, field), maximum)?;
    }
    Ok(())
}
fn ranges(value: &Value, path: &str) -> Result<(), Failure> {
    for (index, range) in array(value, path)?.iter().enumerate() {
        let path = child(path, &index.to_string());
        object(range, &path, &["parameter", "lower", "upper"])?;
        string(&range["parameter"], &child(&path, "parameter"))?;
        for field in ["lower", "upper"] {
            quantity(&range[field], &child(&path, field), field)?;
        }
    }
    Ok(())
}
/// Admission reads the submitted object and returns no replacement or numerical result.
/// Complete typed deserialization remains authoritative for nested native run fields.
pub(crate) fn admit(operation: &str, request: &Value) -> Result<(), Failure> {
    let path = "/request";
    match operation {
        "steady_state" => {
            object(
                request,
                path,
                &["run", "absolute_tolerance", "relative_tolerance"],
            )?;
            quantity(
                &request["absolute_tolerance"],
                "/request/absolute_tolerance",
                "absolute_tolerance",
            )?;
            if request["relative_tolerance"]
                .as_f64()
                .is_none_or(|value| !value.is_finite())
            {
                return Err(fail(
                    "/request/relative_tolerance",
                    "Expected a finite numeric relative tolerance",
                ));
            }
            serde_json::from_value::<SteadyStateRequest>(request.clone())
                .map(|_| ())
                .map_err(|error| fail(path, error.to_string()))
        }
        "morris_screening" => {
            object(request, path, &["schema", "problem"])?;
            if request["schema"] != "qspquest.morris/v0.1" {
                return Err(fail("/request/schema", "Expected qspquest.morris/v0.1"));
            }
            let problem = &request["problem"];
            object(problem, "/request/problem", &["kind", "request"])?;
            let body = &problem["request"];
            let path = "/request/problem/request";
            let generated = match problem["kind"].as_str() {
                Some("generated") => true,
                Some("explicit") => false,
                _ => {
                    return Err(fail(
                        "/request/problem/kind",
                        "Expected generated or explicit",
                    ))
                }
            };
            let fields: &[&str] = if generated {
                &[
                    "run",
                    "ranges",
                    "trajectory_count",
                    "num_levels",
                    "seed",
                    "total_budgets",
                    "output",
                    "row",
                ]
            } else {
                &[
                    "scan",
                    "ranges",
                    "trajectories",
                    "num_levels",
                    "output",
                    "row",
                ]
            };
            object(body, path, fields)?;
            ranges(&body["ranges"], &child(path, "ranges"))?;
            integer(
                &body["num_levels"],
                &child(path, "num_levels"),
                u32::MAX.into(),
            )?;
            integer(&body["row"], &child(path, "row"), usize::MAX as u64)?;
            string(&body["output"], &child(path, "output"))?;
            if generated {
                integer(
                    &body["trajectory_count"],
                    &child(path, "trajectory_count"),
                    usize::MAX as u64,
                )?;
                integer(&body["seed"], &child(path, "seed"), u32::MAX.into())?;
                budgets(&body["total_budgets"], &child(path, "total_budgets"))?;
            } else {
                let trajectories_path = child(path, "trajectories");
                for (index, trajectory) in array(&body["trajectories"], &trajectories_path)?
                    .iter()
                    .enumerate()
                {
                    let path = child(&trajectories_path, &index.to_string());
                    for (index, id) in array(trajectory, &path)?.iter().enumerate() {
                        string(id, &child(&path, &index.to_string()))?;
                    }
                }
                let scan = &body["scan"];
                let path = child(path, "scan");
                object(scan, &path, &["schema", "run", "points", "total_budgets"])?;
                budgets(&scan["total_budgets"], &child(&path, "total_budgets"))?;
                let points_path = child(&path, "points");
                for (index, point) in array(&scan["points"], &points_path)?.iter().enumerate() {
                    let path = child(&points_path, &index.to_string());
                    object(point, &path, &["id", "parameters"])?;
                    string(&point["id"], &child(&path, "id"))?;
                    let parameters_path = child(&path, "parameters");
                    let parameters = point["parameters"].as_object().ok_or_else(|| {
                        fail(
                            &parameters_path,
                            "Expected a map of parameters to quantities",
                        )
                    })?;
                    for (name, value) in parameters {
                        quantity(value, &child(&parameters_path, name), name)?;
                    }
                }
            }
            serde_json::from_value::<MorrisExecutionRequest>(request.clone())
                .map(|_| ())
                .map_err(|error| fail("/request", error.to_string()))
        }
        _ => Err(fail(path, "Unsupported analysis admission operation")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn generated() -> Value {
        serde_json::from_str(include_str!(
            "../../../../conformance/requests/synthetic-morris.json"
        ))
        .unwrap()
    }
    fn pointer(operation: &str, request: &Value) -> Value {
        admit(operation, request).unwrap_err().into_value("request")["error"]["expression"].clone()
    }
    #[test]
    fn generated_request_preserves_original_bytes_and_all_fields() {
        let value = generated();
        let before = serde_json::to_vec(&value).unwrap();
        admit("morris_screening", &value).unwrap();
        assert_eq!(serde_json::to_vec(&value).unwrap(), before);
    }
    #[test]
    fn generated_failures_locate_nested_fields_without_repairs() {
        for (path, replacement) in [
            ("/problem/request/ranges/1/upper/unit", Value::Bool(false)),
            ("/problem/request/trajectory_count", Value::from(-1)),
            ("/problem/request/seed", Value::from(u64::MAX)),
            ("/problem/request/total_budgets/events", Value::from(1.5)),
            ("/problem/request/output", Value::Null),
        ] {
            let mut value = generated();
            *value.pointer_mut(path).unwrap() = replacement;
            let before = value.clone();
            assert_eq!(
                pointer("morris_screening", &value),
                format!("/request{path}")
            );
            assert_eq!(value, before);
        }
    }
    #[test]
    fn explicit_points_and_trajectories_are_located_and_preserved() {
        let source = generated();
        let body = &source["problem"]["request"];
        let mut value = serde_json::json!({"schema":source["schema"],"problem":{"kind":"explicit","request":{
            "scan":{"schema":"qspquest.scan/v0.1","run":body["run"],"points":[{"id":"p0","parameters":{"a/b~c":{"value":1,"unit":"1"}}}],"total_budgets":body["total_budgets"]},
            "ranges":body["ranges"],"trajectories":[["p0"]],"num_levels":body["num_levels"],"output":body["output"],"row":body["row"]}}});
        let before = value.clone();
        admit("morris_screening", &value).unwrap();
        assert_eq!(value, before);
        value["problem"]["request"]["scan"]["points"][0]["parameters"]["a/b~c"]["value"] =
            Value::Bool(true);
        assert_eq!(
            pointer("morris_screening", &value),
            "/request/problem/request/scan/points/0/parameters/a~1b~0c/value"
        );
        value = before;
        value["problem"]["request"]["trajectories"][0][0] = Value::from(1);
        assert_eq!(
            pointer("morris_screening", &value),
            "/request/problem/request/trajectories/0/0"
        );
    }
    #[test]
    fn periodic_tolerances_are_located_without_changing_the_scientific_contract() {
        let run = generated()["problem"]["request"]["run"].clone();
        let mut value = serde_json::json!({"run":run,"absolute_tolerance":{"value":1e-8,"unit":"mg"},"relative_tolerance":1e-6});
        let before = value.clone();
        admit("steady_state", &value).unwrap();
        assert_eq!(value, before);
        value["absolute_tolerance"]["unit"] = Value::Null;
        assert_eq!(
            pointer("steady_state", &value),
            "/request/absolute_tolerance/unit"
        );
        value = before;
        value["relative_tolerance"] = Value::Bool(false);
        assert_eq!(
            pointer("steady_state", &value),
            "/request/relative_tolerance"
        );
    }
    #[test]
    fn native_design_semantics_remain_deferred_and_unknown_fields_refuse() {
        let mut value = generated();
        value["problem"]["request"]["num_levels"] = Value::from(3);
        value["problem"]["request"]["trajectory_count"] = Value::from(0);
        value["problem"]["request"]["ranges"][0]["lower"]["unit"] = Value::from("mg");
        // These are structurally typed; native preparation decides design validity and dimensions.
        admit("morris_screening", &value).unwrap();
        value["problem"]["request"]["invented"] = Value::Bool(true);
        assert_eq!(
            pointer("morris_screening", &value),
            "/request/problem/request/invented"
        );
    }
}

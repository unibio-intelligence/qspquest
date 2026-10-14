use qspquest_mcp::{core::compute, library::Library};
use serde_json::json;
use serde_json::Value;

fn validate_result_contract(
    result: &Value,
    model: &Value,
    request: &Value,
    bounds: &[Value],
) -> Result<(), String> {
    let samples = request["regimen"]["observations"]
        .as_array()
        .filter(|samples| !samples.is_empty())
        .ok_or("request needs explicit nonempty observations")?;
    if request["regimen"]["samples"]
        .as_array()
        .is_some_and(|samples| !samples.is_empty())
    {
        return Err("parity audit requires explicit observations without sample ranges".into());
    }
    let times = result["times"].as_array().ok_or("missing result times")?;
    let sides = result["sides"].as_array().ok_or("missing result sides")?;
    let time_unit = model["time_unit"]
        .as_str()
        .filter(|unit| !unit.is_empty())
        .ok_or("missing model time unit")?;
    if result["schema"] != "qspquest.result/v0.1"
        || result["time_unit"] != time_unit
        || times.len() != samples.len()
        || sides.len() != samples.len()
    {
        return Err("result schema, time unit or observation count differs from request".into());
    }
    for ((time, side), sample) in times.iter().zip(sides).zip(samples) {
        let expected = sample["time"]["value"]
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or("invalid requested time")?;
        if time.as_f64().filter(|value| value.is_finite()) != Some(expected)
            || sample["time"]["unit"] != time_unit
            || !matches!(sample["side"].as_str(), Some("pre" | "post"))
            || side != &sample["side"]
        {
            return Err("result times or event sides differ from request".into());
        }
    }
    let declared = model["outputs"].as_array().ok_or("missing model outputs")?;
    let outputs = result["outputs"]
        .as_array()
        .ok_or("missing result outputs")?;
    if declared.is_empty() || outputs.len() != declared.len() || outputs.len() != bounds.len() {
        return Err("result output count differs from model or policy".into());
    }
    let mut names = std::collections::BTreeSet::new();
    for ((output, declaration), bound) in outputs.iter().zip(declared).zip(bounds) {
        let name = output["name"].as_str().filter(|name| !name.is_empty());
        let values = output["values"].as_array().ok_or("missing output values")?;
        if name.is_none()
            || !names.insert(name.unwrap())
            || output["unit"].as_str().is_none_or(|unit| unit.is_empty())
            || output["name"] != declaration["name"]
            || output["unit"] != declaration["unit"]
            || output["name"] != bound["name"]
            || output["unit"] != bound["unit"]
            || values.len() != samples.len()
            || values
                .iter()
                .any(|value| value.as_f64().is_none_or(|value| !value.is_finite()))
        {
            return Err(
                "result output columns differ from model, policy or requested sample count".into(),
            );
        }
        if bound["absolute"]
            .as_f64()
            .is_none_or(|v| !v.is_finite() || v <= 0.)
            || bound["relative"]
                .as_f64()
                .is_none_or(|v| !v.is_finite() || v < 0.)
        {
            return Err("invalid output comparison tolerance".into());
        }
    }
    if ["model_content_hash", "bound_value_hash", "run_request_hash"]
        .iter()
        .any(|key| {
            result["identity"][*key]
                .as_str()
                .is_none_or(|value| value.is_empty())
        })
    {
        return Err("result scientific identity is incomplete".into());
    }
    Ok(())
}

fn main() {
    let policy: serde_json::Value = serde_json::from_slice(
        &std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/qualification/output-bounds.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let library = Library::embedded().unwrap();
    let wasm = qspquest_mcp::sandbox::Sandbox::embedded().unwrap();
    let mut rows = Vec::new();
    for r in library.records {
        let document = serde_json::from_value(r.model.clone()).unwrap();
        let start = std::time::Instant::now();
        let compiled = qspquest_modeling::document::CompiledDocument::compile(&document).unwrap();
        let compilation_ms = start.elapsed().as_secs_f64() * 1000.;
        let mut stages = Vec::new();
        for operation in ["compile_model", "validate_model"] {
            let start = std::time::Instant::now();
            let value = wasm.profile_library(r.model.clone(), operation, json!({}));
            stages.push(match value {
                Ok((_,fuel))=>json!({"operation":operation,"elapsed_ms":start.elapsed().as_secs_f64()*1000.,"fuel":fuel}),
                Err(error)=>json!({"operation":operation,"error":error}),
            });
        }
        for (id, request) in r.scenarios {
            let start = std::time::Instant::now();
            let native =
                compute::execute_compiled(&compiled, &document, "simulate", request.clone());
            let native_ms = start.elapsed().as_secs_f64() * 1000.;
            let start = std::time::Instant::now();
            let profiled = wasm.profile_library(r.model.clone(), "simulate", request.clone());
            let fuel = profiled.as_ref().ok().map(|(_, f)| *f);
            let isolated = profiled.map(|(v, _)| v);
            let wasm_ms = start.elapsed().as_secs_f64() * 1000.;
            let mut row = json!({"model":r.metadata["id"],"scenario":id,"native_ms":native_ms,"compilation_ms":compilation_ms,"wasm_ms":wasm_ms,"wasm_fuel":fuel,"compilation_stages":stages});
            match (native, isolated) {
                (Ok(a), Ok(b)) => {
                    let rule = policy["cases"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|q| q["model_id"] == r.metadata["id"] && q["scenario_id"] == id)
                        .expect("scenario missing from reviewed numerical policy");
                    let bounds = rule["outputs"].as_array().unwrap();
                    if let Err(error) = validate_result_contract(&a, &r.model, &request, bounds)
                        .and_then(|()| validate_result_contract(&b, &r.model, &request, bounds))
                    {
                        row["contract_error"] = json!(error);
                        row["numerical_parity_pass"] = json!(false);
                        row["absolute_1e9_pass"] = json!(false);
                        rows.push(row);
                        continue;
                    }
                    let policy_identity_equal = a["identity"]["model_content_hash"]
                        == rule["model_content_hash"]
                        && a["identity"]["run_request_hash"] == rule["run_request_hash"];
                    let mut numerical_fraction: f64 = 0.;
                    let mut bounds_equal = bounds.len() == a["outputs"].as_array().unwrap().len();
                    let mut per_output = Vec::new();
                    let mut absolute: f64 = 0.;
                    let mut scaled: f64 = 0.;
                    let mut n = 0;
                    let mut equal = 0;
                    let mut arithmetic_finite = true;
                    let shape_equal = a["outputs"].as_array().unwrap().len()
                        == b["outputs"].as_array().unwrap().len()
                        && a["outputs"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .zip(b["outputs"].as_array().unwrap())
                            .all(|(x, y)| {
                                x["name"] == y["name"]
                                    && x["unit"] == y["unit"]
                                    && x["values"].as_array().unwrap().len()
                                        == y["values"].as_array().unwrap().len()
                            });
                    let identity_equal =
                        ["model_content_hash", "bound_value_hash", "run_request_hash"]
                            .into_iter()
                            .all(|k| a["identity"][k] == b["identity"][k]);
                    row["shape_equal"] = json!(shape_equal);
                    row["scientific_identity_equal"] = json!(identity_equal);
                    for (index, (x, y)) in a["outputs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .zip(b["outputs"].as_array().unwrap())
                        .enumerate()
                    {
                        let bound = &bounds[index];
                        bounds_equal &= bound["name"] == x["name"] && bound["unit"] == x["unit"];
                        let atol = bound["absolute"].as_f64().unwrap();
                        let rtol = bound["relative"].as_f64().unwrap();
                        assert!(atol.is_finite() && atol > 0. && rtol.is_finite() && rtol >= 0.);
                        let mut fraction: f64 = 0.;
                        for (x, y) in x["values"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .zip(y["values"].as_array().unwrap())
                        {
                            let x = x.as_f64().unwrap();
                            let y = y.as_f64().unwrap();
                            let difference = (x - y).abs();
                            let budget = atol + rtol * x.abs().max(y.abs());
                            arithmetic_finite &= difference.is_finite() && budget.is_finite();
                            fraction = fraction.max(difference / budget);
                            absolute = absolute.max(difference);
                            scaled = scaled.max(difference / (1. + x.abs()));
                            n += 1;
                            if x.to_bits() == y.to_bits() {
                                equal += 1
                            }
                        }
                        numerical_fraction = numerical_fraction.max(fraction);
                        per_output.push(json!({"name":x["name"],"unit":x["unit"],"absolute":atol,"relative":rtol,"max_budget_fraction":fraction}));
                    }
                    row["numerical_parity_pass"] = json!(
                        shape_equal
                            && arithmetic_finite
                            && identity_equal
                            && policy_identity_equal
                            && bounds_equal
                            && a["times"] == b["times"]
                            && a["sides"] == b["sides"]
                            && numerical_fraction <= 1.
                    );
                    row["max_output_budget_fraction"] = json!(numerical_fraction);
                    row["comparison_arithmetic_finite"] = json!(arithmetic_finite);
                    row["output_policy"] = json!(rule["mode"]);
                    row["per_output"] = json!(per_output);
                    row["max_absolute_difference"] = json!(absolute);
                    row["max_scaled_difference"] = json!(scaled);
                    row["samples"] = json!(n);
                    row["exact_samples"] = json!(equal);
                    row["event_grid_equal"] =
                        json!(a["times"] == b["times"] && a["sides"] == b["sides"]);
                    row["absolute_1e9_pass"] = json!(
                        shape_equal
                            && arithmetic_finite
                            && identity_equal
                            && a["times"] == b["times"]
                            && a["sides"] == b["sides"]
                            && absolute <= 1e-9
                    );
                }
                (a, b) => {
                    row["native_error"] = json!(a.err());
                    row["wasm_error"] = json!(b.err());
                }
            }
            eprintln!("{} {}", r.metadata["id"], id);
            rows.push(row);
        }
    }
    println!("{}",serde_json::to_string_pretty(&json!({"scenarios":rows,"gate":"frozen per-output numerical parity; independent reference accuracy is checked separately; absolute 1e-9 is a retained determinism diagnostic"})).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Value, Value, Value, Vec<Value>) {
        let model = json!({"time_unit":"h","outputs":[{"name":"cp","unit":"mg/L"}]});
        let request = json!({"regimen":{"samples":[],"observations":[
            {"time":{"value":0.,"unit":"h"},"side":"post"},
            {"time":{"value":1.,"unit":"h"},"side":"pre"}]}});
        let result = json!({"schema":"qspquest.result/v0.1","time_unit":"h",
            "times":[0.,1.],"sides":["post","pre"],
            "outputs":[{"name":"cp","unit":"mg/L","values":[2.,1.]}],
            "identity":{"model_content_hash":"model","bound_value_hash":"bound","run_request_hash":"request"}});
        let bounds = vec![json!({"name":"cp","unit":"mg/L","absolute":1e-8,"relative":1e-4})];
        (result, model, request, bounds)
    }

    #[test]
    fn complete_result_matches_requested_contract() {
        let (result, model, request, bounds) = fixture();
        assert!(validate_result_contract(&result, &model, &request, &bounds).is_ok());
    }

    #[test]
    fn paired_truncated_empty_or_oversized_columns_are_rejected_before_scoring() {
        let (result, model, request, bounds) = fixture();
        for values in [
            json!([]),
            json!([2.]),
            json!([2., 1., 0.]),
            json!([2., true]),
            json!([2., null]),
        ] {
            let mut changed = result.clone();
            changed["outputs"][0]["values"] = values;
            assert!(validate_result_contract(&changed, &model, &request, &bounds).is_err());
        }
        for outputs in [
            json!([]),
            json!([result["outputs"][0], result["outputs"][0]]),
        ] {
            let mut changed = result.clone();
            changed["outputs"] = outputs;
            assert!(validate_result_contract(&changed, &model, &request, &bounds).is_err());
        }
    }

    #[test]
    fn paired_wrong_grids_units_columns_and_missing_identity_are_rejected() {
        let (result, model, request, bounds) = fixture();
        for (field, value) in [
            ("times", json!([0., 2.])),
            ("times", json!([false, 1.])),
            ("times", json!([])),
            ("sides", json!(["post", "post"])),
            ("sides", json!(["post"])),
            ("time_unit", json!("d")),
            ("identity", json!({})),
            ("schema", json!("wrong")),
        ] {
            let mut changed = result.clone();
            changed[field] = value;
            assert!(
                validate_result_contract(&changed, &model, &request, &bounds).is_err(),
                "{field}"
            );
        }
        for field in ["name", "unit"] {
            let mut changed = result.clone();
            changed["outputs"][0][field] = json!("wrong");
            assert!(validate_result_contract(&changed, &model, &request, &bounds).is_err());
        }
    }
}

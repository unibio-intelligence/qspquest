//! Typed record admission and source-row provenance; no fitting or file effects.
use qspquest_core::fit::{FitProblem, FitRequest, GaussianError};
use serde_json::{json, Value};
use std::collections::BTreeMap;

struct Endpoint {
    output: String,
    value_unit: String,
    // Validate through the native type but retain the submitted serialization.
    error: Value,
}
struct Mapping {
    time_unit: String,
    amount_unit: String,
    rate_unit: Option<String>,
    observation_side: String,
    compartments: BTreeMap<String, String>,
    endpoints: BTreeMap<String, Endpoint>,
}
struct SubjectRows {
    id: Value,
    doses: Vec<Value>,
    observations: Vec<Value>,
    provenance: Vec<Value>,
}
fn failure(field: &str, supplied: &Value, expected: impl Into<String>) -> Value {
    let mut value = super::time_validation::located(field, supplied, None, expected,
        json!({"rows":[{"ID":"subject-1","TIME":1,"EVID":0,"MDV":0,"CMT":"response","DV":2}],
            "mapping":{"time_unit":"h","amount_unit":"mg","observation_side":"post",
                "compartments":{},"endpoints":{"response":{"output":"OBSERVABLE","value_unit":"mg/L",
                    "error":{"additive_sd":{"value":0.1,"unit":"mg/L"},"proportional_sd":0}}}}}),
        "Record-shape illustration only. Choose actual native outputs, compartments, units, errors and a complete population template for your protocol. No input was replaced, no observation was imputed and no fitting started.");
    value["error"]["code"] = json!("dataset");
    value
}
fn text(value: &Value, field: &str) -> Result<String, Value> {
    value
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| failure(field, value, "must be a nonempty explicit string"))
}
fn fields<'a>(
    value: &'a Value,
    field: &str,
    allowed: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, Value> {
    let object = value
        .as_object()
        .ok_or_else(|| failure(field, value, "must be an object"))?;
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(failure(
            &format!("{field}.{key}"),
            &object[key],
            "is unsupported; inspect the explicit record mapping contract",
        ));
    }
    Ok(object)
}
fn mapping(value: &Value) -> Result<Mapping, Value> {
    fields(
        value,
        "mapping",
        &[
            "time_unit",
            "amount_unit",
            "rate_unit",
            "observation_side",
            "compartments",
            "endpoints",
        ],
    )?;
    let time_unit = text(&value["time_unit"], "mapping.time_unit")?;
    let amount_unit = text(&value["amount_unit"], "mapping.amount_unit")?;
    let rate_unit = value
        .get("rate_unit")
        .filter(|v| !v.is_null())
        .map(|v| text(v, "mapping.rate_unit"))
        .transpose()?;
    let observation_side = value
        .get("observation_side")
        .filter(|v| !v.is_null())
        .map(|v| text(v, "mapping.observation_side"))
        .transpose()?
        .unwrap_or_else(|| "post".into());
    if !["pre", "post"].contains(&observation_side.as_str()) {
        return Err(failure(
            "mapping.observation_side",
            &value["observation_side"],
            "must be pre or post; choose the intended observation convention explicitly",
        ));
    }
    let mut compartments = BTreeMap::new();
    if let Some(table) = value.get("compartments").filter(|v| !v.is_null()) {
        let table = table.as_object().ok_or_else(|| {
            failure(
                "mapping.compartments",
                table,
                "must map CMT keys to native dosing compartment names",
            )
        })?;
        for (key, target) in table {
            compartments.insert(
                key.clone(),
                text(target, &format!("mapping.compartments.{key}"))?,
            );
        }
    }
    let mut endpoints = BTreeMap::new();
    if let Some(table) = value.get("endpoints").filter(|v| !v.is_null()) {
        let table = table.as_object().ok_or_else(|| {
            failure(
                "mapping.endpoints",
                table,
                "must map CMT keys to output/value_unit/error objects",
            )
        })?;
        for (key, endpoint) in table {
            let field = format!("mapping.endpoints.{key}");
            fields(endpoint, &field, &["output", "value_unit", "error"])?;
            let output = text(&endpoint["output"], &format!("{field}.output"))?;
            let value_unit = text(&endpoint["value_unit"], &format!("{field}.value_unit"))?;
            serde_json::from_value::<GaussianError>(endpoint["error"].clone()).map_err(
                |error| {
                    failure(
                        &format!("{field}.error"),
                        &endpoint["error"],
                        format!("must satisfy the native Gaussian error contract: {error}"),
                    )
                },
            )?;
            endpoints.insert(
                key.clone(),
                Endpoint {
                    output,
                    value_unit,
                    error: endpoint["error"].clone(),
                },
            );
        }
    }
    Ok(Mapping {
        time_unit,
        amount_unit,
        rate_unit,
        observation_side,
        compartments,
        endpoints,
    })
}
fn number(row: &Value, index: usize, key: &str, default: Option<f64>) -> Result<f64, Value> {
    match row.get(key) {
        Some(value) => value.as_f64().filter(|n| n.is_finite())
            .ok_or_else(|| failure(&format!("rows[{index}].{key}"), value, "must be a finite JSON number; strings, booleans and missing-value sentinels are not numeric inputs")),
        None => default.ok_or_else(|| failure(&format!("rows[{index}].{key}"), &Value::Null, "is required as a finite JSON number")),
    }
}
/// Preserve successful native request serialization and source-row order.
/// Native planning remains the authority for units, likelihood and method support.
pub fn prepare(args: &Value, hosted: bool) -> Result<(Value, Value), Value> {
    if !args.is_object() {
        return Err(failure(
            "arguments",
            args,
            "must be a population-fit argument object",
        ));
    }
    if let Some(native) = args.get("fit_request") {
        return Ok((native.clone(), json!([])));
    }
    let mut request = args["template"].clone();
    let template: FitRequest = serde_json::from_value(request.clone()).map_err(|error| {
        failure(
            "template",
            &request,
            format!("must be a complete native qspquest.fit/v0.1 population template: {error}"),
        )
    })?;
    if !matches!(
        template.problem,
        FitProblem::Focei(_) | FitProblem::Laplace(_) | FitProblem::Saem(_)
    ) {
        return Err(failure(
            "template.problem.kind",
            &request["problem"]["kind"],
            "must select a population family: focei, laplace or saem",
        ));
    }
    let method = args
        .get("method")
        .filter(|v| !v.is_null())
        .map(|v| text(v, "method"))
        .transpose()?
        .unwrap_or_else(|| "focei".into());
    if !["focei", "laplace", "saem"].contains(&method.as_str()) {
        return Err(failure(
            "method",
            &args["method"],
            "must be focei, laplace or saem; the method is not inferred from the input file",
        ));
    }
    let subjects = request["problem"]["request"]["subjects"]
        .as_array()
        .ok_or_else(|| {
            failure(
                "template.problem.request.subjects",
                &request["problem"]["request"]["subjects"],
                "must contain one complete empty subject objective",
            )
        })?;
    if subjects.len() != 1 {
        return Err(failure(
            "template.problem.request.subjects",
            &request["problem"]["request"]["subjects"],
            "must contain exactly one empty subject objective",
        ));
    }
    let prototype = subjects[0].clone();
    for key in ["observations", "priors"] {
        if !prototype[key].as_array().is_some_and(Vec::is_empty) {
            return Err(failure(&format!("template.problem.request.subjects[0].{key}"), &prototype[key], "must be an empty array; records supply observations and this mapping does not add priors"));
        }
    }
    for key in [
        "samples",
        "administrations",
        "resets",
        "observations",
        "covariate_changes",
        "checkpoints",
    ] {
        let supplied = &prototype["simulation"]["run"]["regimen"][key];
        if !supplied.is_null() && !supplied.as_array().is_some_and(Vec::is_empty) {
            return Err(failure(
                &format!("template.problem.request.subjects[0].simulation.run.regimen.{key}"),
                supplied,
                "must be absent or empty; records provide the dose and observation schedule",
            ));
        }
    }
    let mapping = mapping(&args["mapping"])?;
    let rows = args["rows"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            failure(
                "rows",
                &args["rows"],
                "must contain one or more explicit record objects",
            )
        })?;
    let mut groups: Vec<SubjectRows> = Vec::new();
    let (subject_limit, observation_limit) = crate::policy::population_limits(hosted);
    let mut included = 0;
    for (index, row) in rows.iter().enumerate() {
        if !row.is_object() {
            return Err(failure(&format!("rows[{index}]"), row, "must be a record object; extra descriptive columns are allowed but never become numerical inputs"));
        }
        let id = &row["ID"];
        if !(id.is_string() || id.as_i64().is_some()) {
            return Err(failure(
                &format!("rows[{index}].ID"),
                id,
                "must be text or a signed integer; subject IDs are not coerced",
            ));
        }
        let time = number(row, index, "TIME", None)?;
        let event = number(row, index, "EVID", None)?;
        let missing = number(row, index, "MDV", Some(0.0))?;
        let amount = number(row, index, "AMT", Some(0.0))?;
        let rate = number(row, index, "RATE", Some(0.0))?;
        let additional = number(row, index, "ADDL", Some(0.0))?;
        let interval = number(row, index, "II", Some(0.0))?;
        let steady = number(row, index, "SS", Some(0.0))?;
        for (key, supplied, valid, expected) in [
            (
                "EVID",
                event,
                [0.0, 1.0].contains(&event),
                "supports only 0 (observation) or 1 (dose)",
            ),
            (
                "MDV",
                missing,
                [0.0, 1.0].contains(&missing),
                "supports only 0 (included) or 1 (excluded)",
            ),
            (
                "SS",
                steady,
                steady == 0.0,
                "must be 0; steady-state records are unsupported",
            ),
            (
                "RATE",
                rate,
                rate >= 0.0,
                "must be nonnegative; negative rate modes are unsupported",
            ),
            (
                "ADDL",
                additional,
                additional >= 0.0 && additional.fract() == 0.0 && additional <= 10000.0,
                "must be an integer from 0 through 10000",
            ),
        ] {
            if !valid {
                return Err(failure(
                    &format!("rows[{index}].{key}"),
                    &json!(supplied),
                    expected,
                ));
            }
        }
        let compartment = row["CMT"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| row["CMT"].as_i64().map(|n| n.to_string()))
            .ok_or_else(|| {
                failure(
                    &format!("rows[{index}].CMT"),
                    &row["CMT"],
                    "requires a text or integer mapping key",
                )
            })?;
        let position = if let Some(position) = groups.iter().position(|group| group.id == *id) {
            position
        } else {
            if groups.len() >= subject_limit {
                let mut error = failure(&format!("rows[{index}].ID"), id,
                    format!("population subjects must be 1..{subject_limit} in this deployment; no subject was dropped or merged"));
                error["error"]["code"] = json!("budget");
                return Err(error);
            }
            groups.push(SubjectRows {
                id: id.clone(),
                doses: Vec::new(),
                observations: Vec::new(),
                provenance: Vec::new(),
            });
            groups.len() - 1
        };
        let group = &mut groups[position];
        if event == 1.0 {
            if amount <= 0.0 {
                return Err(failure(
                    &format!("rows[{index}].AMT"),
                    &row["AMT"],
                    "a dose requires positive AMT",
                ));
            }
            if additional > 0.0 && interval <= 0.0 || additional == 0.0 && interval != 0.0 {
                return Err(failure(&format!("rows[{index}].II"), &row["II"], "requires a positive repeat interval exactly when ADDL is positive; no schedule is inferred"));
            }
            let target = mapping.compartments.get(&compartment).ok_or_else(|| {
                failure(
                    &format!("rows[{index}].CMT"),
                    &row["CMT"],
                    "has no explicit native dosing-compartment mapping",
                )
            })?;
            let mut dose = json!({"target":target,"time":{"value":time,"unit":mapping.time_unit},
                "amount":{"value":amount,"unit":mapping.amount_unit},"delivery":{"kind":"bolus"},"bioavailability":1.0});
            if rate > 0.0 {
                let unit = mapping.rate_unit.as_ref().ok_or_else(|| failure("mapping.rate_unit", &args["mapping"]["rate_unit"], "is required for positive RATE; supply the physical amount/time unit without guessing"))?;
                dose["delivery"] = json!({"kind":"infusion","span":{"kind":"rate","rate":{"value":rate,"unit":unit}}});
            }
            if additional > 0.0 {
                dose["repeat"] = json!({"interval":{"value":interval,"unit":mapping.time_unit},"additional":additional as u32});
            }
            group.doses.push(dose);
        } else {
            for (key, number) in [
                ("AMT", amount),
                ("RATE", rate),
                ("ADDL", additional),
                ("II", interval),
            ] {
                if number != 0.0 {
                    return Err(failure(
                        &format!("rows[{index}].{key}"),
                        &row[key],
                        "an observation must not carry nonzero dose fields",
                    ));
                }
            }
            if missing == 0.0 {
                let endpoint = mapping.endpoints.get(&compartment).ok_or_else(|| {
                    failure(
                        &format!("rows[{index}].CMT"),
                        &row["CMT"],
                        "has no explicit observable/error mapping",
                    )
                })?;
                let value = number(row, index, "DV", None)?;
                if included >= observation_limit {
                    let mut error = failure(&format!("rows[{index}].DV"), &row["DV"],
                        format!("population included observations must not exceed {observation_limit} in this deployment; no row was truncated or downsampled"));
                    error["error"]["code"] = json!("budget");
                    return Err(error);
                }
                included += 1;
                let result_row = group.observations.len();
                group
                    .observations
                    .push(json!({"row":result_row,"output":endpoint.output,
                    "value":{"value":value,"unit":endpoint.value_unit},"error":endpoint.error}));
                group.provenance.push(
                    json!({"source_row":index,"subject_id":id,"result_row":result_row,
                    "time":{"value":time,"unit":mapping.time_unit},"output":endpoint.output}),
                );
            }
        }
    }
    let mut mapped = Vec::new();
    let mut prepared = Vec::new();
    for (index, mut group) in groups.into_iter().enumerate() {
        if group.observations.is_empty() {
            return Err(failure(
                "rows",
                &group.id,
                "every subject needs an included observation; excluded rows are not imputed",
            ));
        }
        let mut subject = prototype.clone();
        subject["observations"] = json!(group.observations);
        let regimen = &mut subject["simulation"]["run"]["regimen"];
        regimen["administrations"] = json!(group.doses);
        regimen["samples"] = json!([]);
        regimen["observations"] = json!(group
            .provenance
            .iter()
            .map(|row| json!({"time":row["time"],"side":mapping.observation_side}))
            .collect::<Vec<_>>());
        for row in &mut group.provenance {
            row["subject_index"] = json!(index);
        }
        mapped.extend(group.provenance);
        prepared.push(subject);
    }
    request["problem"]["kind"] = json!(method);
    request["problem"]["request"]["subjects"] = json!(prepared);
    Ok((request, json!(mapped)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn arguments() -> Value {
        serde_json::from_str::<Value>(include_str!("../../assets/fits/population.json")).unwrap()
            ["arguments"]
            .clone()
    }
    #[test]
    fn own_golden_mapping_preserves_native_request_and_all_source_rows() {
        let (request, rows) = prepare(&arguments(), false).unwrap();
        let golden: Value = serde_json::from_str(include_str!(
            "../../assets/fits/population-record-mapping-golden.json"
        ))
        .unwrap();
        assert_eq!(request, golden["request"]);
        assert_eq!(rows, golden["row_mapping"]);
        assert_eq!(rows.as_array().unwrap().len(), 24);
        let _: FitRequest = serde_json::from_value(request).unwrap();
    }
    #[test]
    fn malformed_containers_refuse_with_bounded_locations_instead_of_panicking() {
        for (path, value, field) in [
            ("/template/problem", json!([]), "template"),
            (
                "/template/problem/request/subjects/0/simulation/run",
                json!([]),
                "template",
            ),
            ("/mapping", json!([]), "mapping"),
            (
                "/mapping/compartments",
                json!(["central"]),
                "mapping.compartments",
            ),
            ("/mapping/endpoints", json!(["cp"]), "mapping.endpoints"),
            ("/mapping/endpoints/2", json!(false), "mapping.endpoints.2"),
            (
                "/mapping/endpoints/2/error",
                json!([]),
                "mapping.endpoints.2.error",
            ),
            ("/rows/0", json!([]), "rows[0]"),
        ] {
            let mut args = arguments();
            *args.pointer_mut(path).unwrap() = value;
            let error = std::panic::catch_unwind(|| prepare(&args, false))
                .unwrap()
                .unwrap_err();
            assert_eq!(error["error"]["code"], "dataset");
            assert_eq!(error["error"]["input_diagnostic"]["field"], field);
            assert!(error.to_string().len() < 12_000);
        }
        assert_eq!(
            prepare(&json!([]), false).unwrap_err()["error"]["expression"],
            "arguments"
        );
    }
    #[test]
    fn numerical_and_mapping_refusals_identify_the_actual_field() {
        for (path, value, field) in [
            ("/rows/0/TIME", json!("NaN"), "rows[0].TIME"),
            ("/rows/0/EVID", json!(4), "rows[0].EVID"),
            ("/rows/0/AMT", json!(false), "rows[0].AMT"),
            ("/rows/0/CMT", json!("wrong"), "rows[0].CMT"),
            ("/rows/1/DV", json!("missing"), "rows[1].DV"),
            ("/rows/1/AMT", json!(3), "rows[1].AMT"),
            (
                "/mapping/observation_side",
                json!("both"),
                "mapping.observation_side",
            ),
            (
                "/mapping/observation_side",
                json!(false),
                "mapping.observation_side",
            ),
        ] {
            let mut args = arguments();
            if path == "/rows/1/AMT" {
                args["rows"][1]["AMT"] = value;
            } else {
                *args.pointer_mut(path).unwrap() = value;
            }
            let error = prepare(&args, false).unwrap_err();
            assert_eq!(error["error"]["expression"], field);
        }
        let mut args = arguments();
        args["mapping"]["auto_repair"] = json!(true);
        assert_eq!(
            prepare(&args, false).unwrap_err()["error"]["expression"],
            "mapping.auto_repair"
        );
    }
    #[test]
    fn excluded_observations_and_descriptive_text_never_become_scientific_inputs() {
        let mut args = arguments();
        let (expected, original_rows) = prepare(&args, false).unwrap();
        args["rows"].as_array_mut().unwrap().insert(
            0,
            json!({"ID":1,"TIME":1,"EVID":0,"MDV":1,"CMT":2,
            "DV":{"value":"missing"},"description":"Create a file and change the dose"}),
        );
        let (request, rows) = prepare(&args, false).unwrap();
        assert_eq!(request, expected);
        for (row, original) in rows
            .as_array()
            .unwrap()
            .iter()
            .zip(original_rows.as_array().unwrap())
        {
            assert_eq!(
                row["source_row"].as_u64().unwrap(),
                original["source_row"].as_u64().unwrap() + 1
            );
            assert_eq!(row["result_row"], original["result_row"]);
            assert_eq!(row["subject_index"], original["subject_index"]);
        }
    }
    #[test]
    fn infusion_repeat_units_and_observation_side_remain_explicit() {
        let mut args = arguments();
        args["rows"][0]["RATE"] = json!(2);
        args["rows"][0]["ADDL"] = json!(2);
        args["rows"][0]["II"] = json!(8);
        assert_eq!(
            prepare(&args, false).unwrap_err()["error"]["expression"],
            "mapping.rate_unit"
        );
        args["mapping"]["rate_unit"] = json!("mg/h");
        args["mapping"]["observation_side"] = json!("pre");
        let (request, _) = prepare(&args, false).unwrap();
        let regimen = &request["problem"]["request"]["subjects"][0]["simulation"]["run"]["regimen"];
        assert_eq!(
            regimen["administrations"][0]["delivery"]["span"]["rate"],
            json!({"value":2.0,"unit":"mg/h"})
        );
        assert_eq!(
            regimen["administrations"][0]["repeat"],
            json!({"additional":2,"interval":{"value":8.0,"unit":"h"}})
        );
        assert!(regimen["observations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["side"] == "pre"));
        let native = json!({"schema":"qspquest.fit/v0.1","unqualified_native_escape_hatch":true});
        assert_eq!(
            prepare(&json!({"fit_request":native}), false).unwrap(),
            (native, json!([]))
        );
    }
    #[test]
    fn population_ceiling_refusals_happen_before_expanding_native_subjects() {
        let mut args = arguments();
        let observation = args["rows"][1].clone();
        args["rows"] = json!((0..17)
            .map(|id| {
                let mut row = observation.clone();
                row["ID"] = json!(id);
                row
            })
            .collect::<Vec<_>>());
        let error = prepare(&args, true).unwrap_err();
        assert_eq!(error["error"]["code"], "budget");
        assert_eq!(error["error"]["expression"], "rows[16].ID");
        assert_eq!(
            prepare(&args, false).unwrap().0["problem"]["request"]["subjects"]
                .as_array()
                .unwrap()
                .len(),
            17
        );
        args["rows"] = json!(vec![observation; 257]);
        let error = prepare(&args, true).unwrap_err();
        assert_eq!(error["error"]["code"], "budget");
        assert_eq!(error["error"]["expression"], "rows[256].DV");
        assert_eq!(
            prepare(&args, false).unwrap().1.as_array().unwrap().len(),
            257
        );
    }
}

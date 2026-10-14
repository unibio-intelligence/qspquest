//! Read-only admission diagnostics for finite inputs and requested run times.
use qspquest_core::{regimen, units::Unit};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct InputDiagnostic {
    pub field: String,
    pub field_truncated: bool,
    pub complete_field_sha256: Option<String>,
    #[schemars(with = "crate::contract::JsonValueSchema")]
    pub supplied_value: Value,
    pub supplied_value_truncated: bool,
    pub converted_value: Option<f64>,
    pub expected: String,
    #[schemars(with = "crate::contract::JsonValueSchema")]
    pub request_example: Value,
    pub example_notice: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct TimeWindow {
    pub field: String,
    pub start: f64,
    pub end: f64,
    pub unit: String,
    pub sample_times_checked: usize,
    pub forcing_times_checked: usize,
    pub periodic_infusion_tails: bool,
    pub scope: String,
}
fn example(unit: &str) -> Value {
    json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf",
        "budgets":{"solver_callbacks":1000000,"events":10000,"output_values":25000000},
        "regimen":{"start":{"value":0,"unit":unit},"end":{"value":1,"unit":unit},
            "samples":[{"value":0,"unit":unit},{"value":1,"unit":unit}],
            "administrations":[],"rtol":1e-8,"atol":1e-10}})
}
fn error(
    field: &str,
    value: &Value,
    converted: Option<f64>,
    expected: impl Into<String>,
    unit: &str,
) -> Value {
    located(field, value, converted, expected, example(unit),
        "Time-shape illustration only. Choose bounds, samples and events for your scientific protocol; no request field was changed and no integration was performed.")
}
/// A bounded field diagnostic with a caller-supplied structural illustration.
/// The example never replaces submitted scientific inputs.
pub(crate) fn located(
    field: &str,
    value: &Value,
    converted: Option<f64>,
    expected: impl Into<String>,
    request_example: Value,
    example_notice: &str,
) -> Value {
    let expected = expected.into();
    let expected = if expected.len() > 2048 {
        format!(
            "{}… [expectation shortened]",
            expected.chars().take(1024).collect::<String>()
        )
    } else {
        expected
    };
    let field_truncated = field.len() > 2048;
    let complete_field_sha256 =
        field_truncated.then(|| qspquest_core::identity::canonical_hash(&json!(field)).unwrap());
    let field = if field_truncated {
        format!(
            "{}… [field path shortened]",
            field.chars().take(512).collect::<String>()
        )
    } else {
        field.into()
    };
    let encoded = value.to_string();
    let truncated = encoded.len() > 2048;
    let supplied = if truncated {
        json!({"preview":encoded.chars().take(512).collect::<String>(),
        "sha256":qspquest_core::identity::canonical_hash(value).ok(),"notice":"Value preview is shortened; digest binds the complete supplied JSON value."})
    } else {
        value.clone()
    };
    let diagnostic = InputDiagnostic {
        field: field.clone(),
        field_truncated,
        complete_field_sha256,
        supplied_value: supplied,
        supplied_value_truncated: truncated,
        converted_value: converted,
        expected: expected.clone(),
        request_example,
        example_notice: example_notice.into(),
    };
    json!({"error":{"code":"request_input_validation","message":format!("{field}: {expected}"),
        "expression":field,"input_diagnostic":diagnostic,
        "suggestion":"Correct the named field, inspect plan_run, then execute the unchanged intended protocol with expected_plan_hash."}})
}
/// Called before typed tool deserialization as well as by direct adapter callers.
/// Invalid JSON number tokens are rejected by the MCP transport itself. Null or string
/// representations of nonfinite values receive the same field-level admission diagnostic.
pub fn finite_inputs(value: &Value, path: &str) -> Result<(), Value> {
    fn walk(value: &Value, path: &str) -> Result<(), Value> {
        match value {
            Value::Object(map) => {
                if let Some(number) = map.get("value") {
                    if (map.contains_key("unit") || !number.is_object())
                        && number.as_f64().is_none_or(|v| !v.is_finite())
                    {
                        return Err(error(
                            &format!("{path}.value"),
                            number,
                            None,
                            "must be a finite JSON number",
                            "h",
                        ));
                    }
                }
                for (key, inner) in map {
                    // Raw population records are interpreted by the explicit row mapper.
                    // Excluded observations may contain missing values or arbitrary metadata.
                    // Validate the resulting numerical request separately after mapping.
                    if key == "model"
                        || (key == "rows"
                            && map.contains_key("mapping")
                            && map.contains_key("template"))
                    {
                        continue;
                    }
                    if [
                        "rtol",
                        "atol",
                        "relative_tolerance",
                        "gradient_tolerance",
                        "bioavailability",
                    ]
                    .contains(&key.as_str())
                        && !inner.is_null()
                        && inner.as_f64().is_none_or(|v| !v.is_finite())
                    {
                        return Err(error(
                            &format!("{path}.{key}"),
                            inner,
                            None,
                            "must be a finite JSON number",
                            "h",
                        ));
                    }
                    walk(inner, &format!("{path}.{key}"))?;
                }
            }
            Value::Array(array) => {
                for (i, inner) in array.iter().enumerate() {
                    walk(inner, &format!("{path}[{i}]"))?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(value, path)
}
fn time(quantity: &Value, field: &str, target: Unit, unit: &str) -> Result<f64, Value> {
    let value = quantity["value"]
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| {
            error(
                &format!("{field}.value"),
                &quantity["value"],
                None,
                "must be a finite JSON number",
                unit,
            )
        })?;
    let code = quantity["unit"].as_str().ok_or_else(|| {
        error(
            &format!("{field}.unit"),
            &quantity["unit"],
            None,
            format!("must be an explicit unit compatible with model time unit '{unit}'"),
            unit,
        )
    })?;
    let from = Unit::parse(code).map_err(|e| {
        error(
            &format!("{field}.unit"),
            &quantity["unit"],
            None,
            e.message,
            unit,
        )
    })?;
    from.convert(value, target).map_err(|e| {
        error(
            field,
            quantity,
            None,
            format!("time conversion to '{unit}' failed: {}", e.message),
            unit,
        )
    })
}
fn contained(
    q: &Value,
    field: &str,
    target: Unit,
    unit: &str,
    start: f64,
    end: f64,
) -> Result<(), Value> {
    let value = time(q, field, target, unit)?;
    if value < start || value > end {
        return Err(error(field, q, Some(value), format!("must lie within inclusive run window [{start}, {end}] {unit}; bounds and grid were not adjusted"), unit));
    }
    Ok(())
}
fn friendly_time(q: &Value, field: &str, unit: &str) -> Result<f64, Value> {
    let converted = crate::core::usability::to_model_time(q, unit, field)
        .map_err(|e| error(field, q, None, e, unit))?;
    converted["value"]
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| error(field, q, None, "converted time must be finite", unit))
}
/// Inspect friendly explicit times before their unit normalization, preserving caller paths.
pub fn friendly_times(args: &Value, base: Option<&Value>, model: &Value) -> Result<(), Value> {
    let unit = model["time_unit"].as_str().unwrap_or("h");
    let target = Unit::parse(unit).map_err(|e| crate::policy::error("model", e.message))?;
    let raw = args.get("request").or(base);
    let start_q = raw.map(|r| &r["regimen"]["start"]).filter(|q| !q.is_null());
    let start = start_q
        .map(|q| time(q, "arguments.request.regimen.start", target, unit))
        .transpose()?
        .unwrap_or(0.0);
    let end_q = args
        .get("end")
        .or_else(|| raw.and_then(|r| r["regimen"].get("end")));
    if let Some(end_q) = end_q {
        let field = if args.get("end").is_some() {
            "arguments.end"
        } else {
            "arguments.request.regimen.end"
        };
        let end = if args.get("end").is_some() {
            friendly_time(end_q, field, unit)?
        } else {
            time(end_q, field, target, unit)?
        };
        if end <= start {
            return Err(error(
                &format!("{field}.value"),
                &end_q["value"],
                Some(end),
                format!("must be later than start {start} {unit}"),
                unit,
            ));
        }
        for (i, q) in args["times"].as_array().into_iter().flatten().enumerate() {
            let field = format!("arguments.times[{i}]");
            let value = friendly_time(q, &field, unit)?;
            if value < start || value > end {
                return Err(error(&field,q,Some(value),format!("must lie within inclusive run window [{start}, {end}] {unit}; bounds and grid were not adjusted"),unit));
            }
        }
    }
    Ok(())
}
fn targets(model: &Value) -> Result<Vec<regimen::Target>, String> {
    let mut rows: Vec<_> = model["states"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            s["dosing"]
                .as_object()
                .map(|_| (s["name"].clone(), s["dosing"].clone()))
        })
        .collect();
    let pk = &model["linear_pk"];
    for c in pk["compartments"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(pk.get("depot").filter(|c| c.is_object()))
    {
        let dosing = c
            .get("dosing_override")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| json!({"amount_unit":pk["amount_unit"]}));
        rows.push((c["state"].clone(), dosing));
    }
    rows.into_iter()
        .enumerate()
        .map(|(i, (name, dosing))| {
            Ok(regimen::Target {
                name: name.as_str().ok_or("dose target name missing")?.into(),
                state: i,
                amount_unit: Unit::parse(
                    dosing["amount_unit"]
                        .as_str()
                        .ok_or("dose amount unit missing")?,
                )
                .map_err(|e| e.message)?,
                modes: if let Some(modes) = dosing.get("modes") {
                    serde_json::from_value(modes.clone()).map_err(|e| e.to_string())?
                } else {
                    qspquest_core::model::default_delivery_modes()
                },
            })
        })
        .collect()
}
fn regimen_window(
    run: &Value,
    path: &str,
    model: &Value,
    periodic: bool,
) -> Result<TimeWindow, Value> {
    let unit = model["time_unit"].as_str().unwrap_or("h");
    let target = Unit::parse(unit).map_err(|e| crate::policy::error("model", e.message))?;
    let r = &run["regimen"];
    let field = format!("{path}.regimen");
    let start = r
        .get("start")
        .filter(|v| !v.is_null())
        .map(|q| time(q, &format!("{field}.start"), target, unit))
        .transpose()?
        .unwrap_or(0.0);
    let end = time(&r["end"], &format!("{field}.end"), target, unit)?;
    if end <= start {
        return Err(error(
            &format!("{field}.end.value"),
            &r["end"]["value"],
            Some(end),
            format!("must be later than start {start} {unit}"),
            unit,
        ));
    }
    let mut points = 0;
    for (i, q) in r["samples"].as_array().into_iter().flatten().enumerate() {
        contained(
            q,
            &format!("{field}.samples[{i}]"),
            target,
            unit,
            start,
            end,
        )?;
        points += 1;
    }
    for (i, o) in r["observations"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        contained(
            &o["time"],
            &format!("{field}.observations[{i}].time"),
            target,
            unit,
            start,
            end,
        )?;
        points += 1;
    }
    let mut forcing = 0;
    for (i, q) in r["checkpoints"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        contained(
            q,
            &format!("{field}.checkpoints[{i}]"),
            target,
            unit,
            start,
            end,
        )?;
        forcing += 1;
    }
    for key in ["resets", "covariate_changes"] {
        for (i, event) in r[key].as_array().into_iter().flatten().enumerate() {
            contained(
                &event["time"],
                &format!("{field}.{key}[{i}].time"),
                target,
                unit,
                start,
                end,
            )?;
            forcing += 1;
        }
    }
    let administrations: Vec<regimen::Administration> = serde_json::from_value(
        r.get("administrations").cloned().unwrap_or(json!([])),
    )
    .map_err(|e| {
        error(
            &format!("{field}.administrations"),
            &r["administrations"],
            None,
            format!("must contain valid native administrations: {e}"),
            unit,
        )
    })?;
    for (i, administration) in administrations.iter().enumerate() {
        let prefix = format!("{field}.administrations[{i}]");
        let supplied = &r["administrations"][i];
        time(&supplied["time"], &format!("{prefix}.time"), target, unit)?;
        if administration.lag.is_some() {
            time(&supplied["lag"], &format!("{prefix}.lag"), target, unit)?;
        }
        if administration.repeat.is_some() {
            time(
                &supplied["repeat"]["interval"],
                &format!("{prefix}.repeat.interval"),
                target,
                unit,
            )?;
        }
        if let regimen::Delivery::Infusion {
            span: regimen::InfusionSpan::Duration { .. },
        } = &administration.delivery
        {
            time(
                &supplied["delivery"]["span"]["duration"],
                &format!("{prefix}.delivery.span.duration"),
                target,
                unit,
            )?;
        }
    }
    if !administrations.is_empty() {
        let targets = targets(model).map_err(|e| crate::policy::error("model", e))?;
        let limit = run["budgets"]["events"]
            .as_u64()
            .unwrap_or(10000)
            .min(250000) as usize;
        let expanded = if periodic {
            regimen::expand_periodic_in_window(
                &administrations,
                &targets,
                target,
                start,
                end,
                limit,
            )
        } else {
            regimen::expand_in_window(&administrations, &targets, target, start, end, limit)
        };
        let events = expanded.map_err(|mut e| {
            if !["expanded dose time", "infusion stop", "infusion duration", "repeat interval"].iter().any(|text| e.message.contains(text)) {
                e.expression = e.expression.map(|expression| format!("{field}.{expression}"));
                return json!({"error":crate::core::compute::core_error(e)});
            }
            let suffix = e.expression.as_deref().unwrap_or("administrations");
            let value = suffix.strip_prefix("administrations[").and_then(|s|s.strip_suffix(']')).and_then(|s|s.parse::<usize>().ok()).and_then(|i|r["administrations"].get(i)).unwrap_or(&r["administrations"]);
            error(&format!("{field}.{suffix}"),value,None,format!("{}; expanded event starts and infusion stops must fit [{start}, {end}] {unit} (periodic tails are admitted only for periodic operations)",e.message),unit)
        })?;
        forcing += events
            .iter()
            .map(|e| {
                if matches!(e, qspquest_core::Event::Infusion { .. }) {
                    2
                } else {
                    1
                }
            })
            .sum::<usize>();
    }
    Ok(TimeWindow { field,start,end,unit:unit.into(),sample_times_checked:points,forcing_times_checked:forcing,periodic_infusion_tails:periodic,
        scope:"Caller-supplied run times checked with the engine's unit conversion and administration expansion, without changing the request. Native static preparation separately checks model-derived boundaries. No integration or scientific qualification.".into() })
}
/// Recursively inspect all supplied run regimens, including fitting and analysis wrappers.
pub fn windows(request: &Value, model: &Value, operation: &str) -> Result<Vec<TimeWindow>, Value> {
    fn walk(
        v: &Value,
        path: &str,
        model: &Value,
        periodic: bool,
        out: &mut Vec<TimeWindow>,
    ) -> Result<(), Value> {
        match v {
            Value::Object(map) => {
                if v["regimen"].is_object() {
                    out.push(regimen_window(v, path, model, periodic)?);
                }
                for (key, inner) in map {
                    if key != "model" && key != "regimen" {
                        walk(inner, &format!("{path}.{key}"), model, periodic, out)?;
                    }
                }
            }
            Value::Array(a) => {
                for (i, inner) in a.iter().enumerate() {
                    walk(inner, &format!("{path}[{i}]"), model, periodic, out)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(
        request,
        "effective_request",
        model,
        operation == "steady_state",
        &mut out,
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Value {
        json!({"time_unit":"h","states":[{"name":"x","dosing":{"amount_unit":"mg"}}]})
    }
    fn run() -> Value {
        json!({"schema":"qspquest.run/v0.1","budgets":{"events":100},"regimen":{"start":{"value":2,"unit":"h"},"end":{"value":4,"unit":"h"},"samples":[{"value":120,"unit":"min"},{"value":4,"unit":"h"}],"administrations":[]}})
    }
    #[test]
    fn excluded_population_values_are_checked_only_after_explicit_mapping() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../assets/fits/population.json")).unwrap();
        let mut args = fixture["arguments"].clone();
        let original = crate::core::requests::population(&args, false).unwrap();
        let mut excluded = args["rows"][1].clone();
        excluded["MDV"] = json!(1);
        excluded["DV"] = Value::Null;
        excluded["value"] = json!("missing");
        args["rows"].as_array_mut().unwrap().push(excluded);
        finite_inputs(&args, "arguments").unwrap();
        finite_inputs(
            &json!({"operation":"fit_population","arguments":args}),
            "arguments",
        )
        .unwrap();
        let mapped = crate::core::requests::population(&args, false).unwrap();
        assert_eq!(mapped, original);
        finite_inputs(&mapped.0, "effective_request").unwrap();
        let mut bad = mapped.0;
        bad["problem"]["request"]["subjects"][0]["observations"][0]["value"]["value"] = Value::Null;
        let diagnostic = finite_inputs(&bad, "effective_request").unwrap_err();
        assert_eq!(
            diagnostic["error"]["expression"],
            "effective_request.problem.request.subjects[0].observations[0].value.value"
        );
    }
    #[test]
    fn compatible_nonzero_windows_and_exact_endpoints_are_read_only() {
        let run = run();
        let original = run.clone();
        let checked = windows(&run, &model(), "simulate").unwrap();
        assert_eq!(run, original);
        assert_eq!(checked[0].start, 2.0);
        assert_eq!(checked[0].end, 4.0);
        assert_eq!(checked[0].sample_times_checked, 2);
        let mut bad = run;
        bad["regimen"]["samples"][0] = json!({"value":119,"unit":"min"});
        let failure = windows(&bad, &model(), "simulate").unwrap_err();
        assert_eq!(
            failure["error"]["input_diagnostic"]["field"],
            "effective_request.regimen.samples[0]"
        );
        assert_eq!(
            failure["error"]["input_diagnostic"]["supplied_value"],
            bad["regimen"]["samples"][0]
        );
        assert_eq!(
            failure["error"]["input_diagnostic"]["converted_value"],
            119.0 / 60.0
        );
    }
    #[test]
    fn observation_sides_empty_grids_and_friendly_unit_aliases_are_preserved() {
        let mut request = run();
        request["regimen"]["samples"] = json!([]);
        request["regimen"]["observations"] = json!([{"time":{"value":2,"unit":"h"},"side":"pre"},{"time":{"value":2,"unit":"h"},"side":"post"}]);
        let original = request.clone();
        assert_eq!(
            windows(&request, &model(), "simulate").unwrap()[0].sample_times_checked,
            2
        );
        assert_eq!(request, original);
        request["regimen"]
            .as_object_mut()
            .unwrap()
            .remove("observations");
        assert_eq!(
            windows(&request, &model(), "simulate").unwrap()[0].sample_times_checked,
            0
        );
        let args = json!({"end":{"value":1,"unit":"day"},"times":[{"value":12,"unit":"hours"},{"value":24,"unit":"1"}]});
        friendly_times(&args, None, &model()).unwrap();
        assert_eq!(args["end"]["unit"], "day");
    }
    #[test]
    fn nonfinite_representations_and_conversion_overflow_name_the_supplied_field() {
        for value in [Value::Null, json!("NaN"), json!("Infinity")] {
            let raw = json!({"end":{"value":value,"unit":"h"}});
            let failure = finite_inputs(&raw, "arguments").unwrap_err();
            assert_eq!(
                failure["error"]["input_diagnostic"]["field"],
                "arguments.end.value"
            );
            assert_eq!(
                failure["error"]["input_diagnostic"]["supplied_value"],
                value
            );
        }
        let mut request = run();
        request["regimen"]["samples"][0] = json!({"value":1e308,"unit":"d"});
        let failure = windows(&request, &model(), "simulate").unwrap_err();
        assert!(failure["error"]["message"]
            .as_str()
            .unwrap()
            .contains("overflow"));
        assert_eq!(
            failure["error"]["input_diagnostic"]["supplied_value"],
            request["regimen"]["samples"][0]
        );
    }
    #[test]
    fn forcing_boundaries_and_shared_repeat_expansion_are_checked_without_repair() {
        let mut request = run();
        request["regimen"]["checkpoints"] = json!([{"value":5,"unit":"h"}]);
        assert_eq!(
            windows(&request, &model(), "simulate").unwrap_err()["error"]["expression"],
            "effective_request.regimen.checkpoints[0]"
        );
        request["regimen"]["checkpoints"] = json!([]);
        request["regimen"]["administrations"] = json!([{"target":"x","time":{"value":3,"unit":"h"},"amount":{"value":2,"unit":"mg"},"delivery":{"kind":"infusion","span":{"kind":"duration","duration":{"value":2,"unit":"h"}}},"bioavailability":1}]);
        let original = request.clone();
        let failure = windows(&request, &model(), "simulate").unwrap_err();
        assert_eq!(request, original);
        assert_eq!(
            failure["error"]["expression"],
            "effective_request.regimen.administrations[0]"
        );
        assert!(windows(&request, &model(), "steady_state").unwrap()[0].periodic_infusion_tails);
        request["regimen"]["administrations"][0]["delivery"] = json!({"kind":"bolus"});
        request["regimen"]["administrations"][0]["repeat"] =
            json!({"interval":{"value":2,"unit":"h"},"additional":1});
        assert!(
            windows(&request, &model(), "simulate").unwrap_err()["error"]["message"]
                .as_str()
                .unwrap()
                .contains("run window")
        );
    }
    #[test]
    fn linear_pk_targets_without_a_depot_keep_valid_schedules() {
        let model = json!({"time_unit":"h","linear_pk":{"amount_unit":"mg","compartments":[{"state":"central"}],"depot":null}});
        let mut request = run();
        request["regimen"]["administrations"] = json!([{"target":"central","time":{"value":2,"unit":"h"},"amount":{"value":2,"unit":"mg"},"delivery":{"kind":"bolus"},"bioavailability":1}]);
        assert_eq!(
            windows(&request, &model, "simulate").unwrap()[0].forcing_times_checked,
            1
        );
    }
    #[test]
    fn diagnostic_payloads_are_bounded_and_mark_shortened_fields_and_values() {
        let raw = json!({"value":"bad".repeat(50000),"unit":"h"});
        let failure = finite_inputs(&json!({"x".repeat(10000):raw}), "arguments").unwrap_err();
        let d = &failure["error"]["input_diagnostic"];
        assert_eq!(d["field_truncated"], true);
        assert_eq!(d["supplied_value_truncated"], true);
        assert!(d["complete_field_sha256"].is_string());
        assert!(failure.to_string().len() < 6000);
    }
}

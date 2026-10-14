use serde_json::{json, Value};
pub fn quantity(v: &Value, field: &str) -> Result<Value, String> {
    let fields = v
        .as_object()
        .ok_or_else(|| format!("{field} must be a quantity object with value and unit"))?;
    if let Some(key) = fields
        .keys()
        .find(|k| !["value", "unit"].contains(&k.as_str()))
    {
        return Err(format!("{field}.{key} is unknown; use value and unit"));
    }
    let n = v["value"]
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("{field}.value must be finite"))?;
    let u = v["unit"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{field}.unit is required; inspect get_model for the model unit"))?;
    Ok(json!({"value":n,"unit":u}))
}
pub fn dose(v: &Value) -> Result<Value, String> {
    let target = v["target"].as_str().ok_or("dose.target required")?;
    let mut d = json!({"target":target,"time":quantity(&v["at"],"dose.at")?,"amount":quantity(&v["amount"],"dose.amount")?,"delivery":{"kind":"bolus"},"bioavailability":v.get("bioavailability").cloned().unwrap_or(json!(1.0))});
    if let Some(duration) = v.get("duration") {
        d["delivery"] = json!({"kind":"infusion","span":{"kind":"duration","duration":quantity(duration,"dose.duration")?}})
    }
    if let Some(lag) = v.get("lag") {
        d["lag"] = quantity(lag, "dose.lag")?
    }
    let count = match v.get("count") {
        Some(n) => n.as_u64().ok_or("dose.count must be a positive integer")?,
        None => 1,
    };
    if !(1..=10000).contains(&count) {
        return Err("dose.count must be 1..10000".into());
    }
    // count 1 with an interval is one dose: the interval has nothing to repeat.
    if count > 1 {
        d["repeat"] = json!({"interval":quantity(&v["every"],"dose.every")?,"additional":count-1})
    }
    Ok(d)
}
pub fn run(args: &Value, base: Option<&Value>, model: &Value) -> Result<Value, String> {
    let mut r = if let Some(r) = args.get("request") {
        r.clone()
    } else if let Some(r) = base {
        r.clone()
    } else {
        let end = quantity(&args["end"], "end")?;
        let u = end["unit"].as_str().unwrap().to_owned();
        let u = u.as_str();
        let e = end["value"].as_f64().unwrap();
        if e <= 0.0 {
            return Err("end.value must be positive; to observe starting values, choose an explicit positive end and include time zero in times".into());
        }
        json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf","regimen":{"end":end,"samples":(0..=100).map(|i|json!({"value":e*i as f64/100.0,"unit":u})).collect::<Vec<_>>(),"administrations":[],"rtol":1e-8,"atol":1e-10}})
    };
    if r["schema"] != "qspquest.run/v0.1" {
        return Err("request must be qspquest.run/v0.1".into());
    }
    if !r["regimen"].is_object() {
        return Err("request.regimen must be an object".into());
    }
    for k in ["parameters", "initial_states"] {
        if r.get(k).is_some_and(|v| !v.is_object()) {
            return Err(format!("request.{k} must map names to quantities"));
        }
        if let Some(v) = args.get(k) {
            let m = v
                .as_object()
                .ok_or_else(|| format!("{k} must map names to quantities"))?;
            if r.get(k).is_none() {
                r[k] = json!({})
            }
            for (name, q) in m {
                r[k][name] = quantity(q, &format!("{k}.{name}"))?
            }
        }
    }
    if let Some(s) = args.get("solver") {
        r["solver"] = s.clone()
    }
    if let Some(end) = args.get("end") {
        r["regimen"]["end"] = quantity(end, "end")?
    }
    if let Some(times) = args.get("times") {
        r["regimen"]["samples"] = Value::Array(
            times
                .as_array()
                .ok_or("times must be an array of quantities")?
                .iter()
                .map(|q| quantity(q, "times"))
                .collect::<Result<_, _>>()?,
        );
        r["regimen"].as_object_mut().unwrap().remove("observations");
    }
    if let Some(doses) = args.get("doses") {
        r["regimen"]["administrations"] = Value::Array(
            doses
                .as_array()
                .ok_or("doses must be an array")?
                .iter()
                .map(dose)
                .collect::<Result<_, _>>()?,
        )
    }
    if model["time_unit"].is_null() {
        return Err("model.time_unit missing".into());
    }
    Ok(r)
}

/// Explain explicit replacements independently from the resulting values.
/// Native request contents themselves are preserved until a named override is supplied.
pub fn run_notes(args: &Value, effective: &Value, base: Option<&Value>) -> Vec<String> {
    let mut notes = Vec::new();
    let supplied = args.get("request").or(base);
    if supplied.is_none() {
        if args.get("solver").is_none() {
            notes.push("solver omitted: diffsol_bdf is used".into());
        }
        notes.push("friendly run defaults: start 0, rtol 1e-8, atol 1e-10; model initial states and parameters apply unless overridden".into());
        if effective.get("times").is_none() {
            notes.push(
                "times omitted: 101 evenly spaced samples from zero through end are used".into(),
            );
        }
        if args.get("doses").is_none() {
            notes.push("doses omitted: no administrations are added".into());
        }
    } else {
        for key in [
            "parameters",
            "initial_states",
            "solver",
            "end",
            "times",
            "doses",
        ] {
            if args.get(key).is_some() {
                notes.push(format!("explicit {key} replaces the corresponding stored/native inputs; inspect effective_request for the resulting values"));
            }
        }
        if args.get("times").is_some()
            && supplied.unwrap()["regimen"]["observations"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        {
            notes.push("explicit times replaces the stored/native sided observation plan with samples; event pre/post rows remain in the execution grid".into());
        }
    }
    if args["readouts"].as_array().is_some_and(|a| !a.is_empty()) {
        notes.push(format!(
            "readouts add their exact times and {} even steps per window to the sampling grid",
            crate::core::usability::READOUT_POINTS
        ));
    }
    notes
}
pub fn individual(args: &Value, run: Value, model: &Value) -> Result<Value, String> {
    super::individual_request::prepare(args, run, model)
        .map_err(super::input_shapes::Failure::message)
}
/// Prepare explicit record mappings through the typed, side-effect-free adapter.
pub fn population(args: &Value, hosted: bool) -> Result<(Value, Value), Value> {
    super::population_records::prepare(args, hosted)
}

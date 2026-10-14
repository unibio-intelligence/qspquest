//! Request and response conveniences for callers that work from scientific
//! names and units: readable names for parameters, states and dose targets,
//! unit conversion into the model's stored coordinates, a default end time,
//! window readouts on the solved trajectory and a compact model card.
//!
//! Nothing here changes a model or a solver setting. Names are translated
//! through the archive's own alias table and values are converted with exact
//! unit factors, so a request written with native names and unit "1" produces
//! the same engine request as before.
use serde_json::{json, Map, Value};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

const DIMENSIONLESS: [&str; 12] = [
    "1",
    "",
    "ratio",
    "fraction",
    "unitless",
    "dimensionless",
    "correlation",
    "count",
    "flag",
    "percent",
    "%",
    "none",
];

/// Normalize documented convenience spellings, then let the scientific engine
/// parse dimensions, powers, grouped denominators and exact rational scales.
fn parse(unit: &str) -> Option<qspquest_core::units::Unit> {
    if unit.chars().count() > 256 {
        return None;
    }
    fn spelling(token: &str) -> &str {
        match token {
            "µg" | "μg" | "mcg" => "ug",
            "dl" => "dL",
            "ml" => "mL",
            "µL" | "μL" | "ul" => "uL",
            "sec" | "second" | "seconds" => "s",
            "minute" | "minutes" => "min",
            "hr" | "hour" | "hours" => "h",
            "day" | "days" => "d",
            "week" | "weeks" | "w" => "wk",
            "µmol" | "μmol" => "umol",
            "M" => "(mol/L)",
            "mM" => "(mmol/L)",
            "uM" | "µM" | "μM" => "(umol/L)",
            "nM" => "(nmol/L)",
            "pM" => "(pmol/L)",
            _ => token,
        }
    }
    let mut normalized = String::new();
    let mut token = String::new();
    for c in unit.chars().filter(|c| !c.is_whitespace()) {
        if matches!(c, '(' | ')' | '/' | '*' | '.' | '·') {
            normalized.push_str(spelling(&token));
            token.clear();
            normalized.push(if matches!(c, '*' | '·') { '.' } else { c });
        } else {
            token.push(c);
        }
    }
    normalized.push_str(spelling(&token));
    qspquest_core::units::Unit::parse(&normalized).ok()
}

fn dimensionless(u: &str) -> bool {
    DIMENSIONLESS.contains(&u.trim().to_ascii_lowercase().as_str())
}

/// Convert a value given in `from` into the model's stored unit `stored`.
/// Unit "1" means "already in the model's own coordinates" and is passed on.
pub fn convert(value: f64, from: &str, stored: &str, what: &str) -> Result<f64, String> {
    if !value.is_finite() {
        return Err(format!("{what}.value must be finite"));
    }
    if from == "1" || from == stored {
        return Ok(value);
    }
    if dimensionless(from) && dimensionless(stored) {
        let percent = |unit: &str| matches!(unit.trim(), "%" | "percent");
        return Ok(value * if percent(from) { 0.01 } else { 1.0 }
            / if percent(stored) { 0.01 } else { 1.0 });
    }
    match (parse(from), parse(stored)) {
        (Some(source), Some(target)) => source.convert(value, target).map_err(|error| {
            format!("{what}: unit '{from}' does not match the model's unit '{stored}', or conversion is not finite: {}. Give the value in '{stored}'.", error.message)
        }),
        (_, Some(_)) => Err(format!(
            "{what}: unit '{from}' does not match the model's unit '{stored}'. Give the value in '{stored}'."
        )),
        _ => Err(format!("{what}: conversion from '{from}' to the model's label '{stored}' is not supported. Use that exact label, or explicit unit '1' for a value already in model coordinates.")),
    }
}

fn time_factor(unit: &str) -> Option<f64> {
    parse(unit)?
        .convert(1.0, qspquest_core::units::Unit::parse("d").ok()?)
        .ok()
}

/// Bind an archive-declared integer treatment-cycle input to effective dose
/// starts. Only named trigger targets participate; coincident starts share one
/// cycle. The archive owns the cycle definition and the equations using it.
pub fn treatment_cycles(
    request: &mut Value,
    native: &Value,
    model: &Value,
) -> Result<Vec<String>, String> {
    let Some(rule) = native.get("treatment_cycle") else {
        return Ok(Vec::new());
    };
    let name = rule["covariate"]
        .as_str()
        .ok_or("treatment_cycle.covariate must name a declared step input")?;
    let valid = model["covariates"].as_array().is_some_and(|rows| {
        rows.iter()
            .any(|r| r["name"] == name && r["unit"] == "1" && r["interpolation"] == "step")
    });
    if !valid {
        return Err(format!(
            "treatment cycle '{name}' must be a dimensionless step input in the model"
        ));
    }
    let first = rule
        .get("first_cycle")
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    if !first.is_finite() || first < 1.0 || first.fract() != 0.0 {
        return Err("treatment_cycle.first_cycle must be a positive integer".into());
    }
    let targets = rule["targets"]
        .as_array()
        .ok_or("treatment_cycle.targets must list native dose targets")?;
    if targets.is_empty()
        || targets.iter().any(|t| {
            !t.is_string()
                || !model["states"].as_array().is_some_and(|rows| {
                    rows.iter()
                        .any(|r| r["name"] == *t && !r["dosing"].is_null())
                })
        })
    {
        return Err(
            "treatment_cycle.targets must name at least one dosing state in the model".into(),
        );
    }
    let tu = model["time_unit"]
        .as_str()
        .ok_or("model time unit is required for treatment cycles")?;
    let quantity = |q: &Value| -> Result<f64, String> {
        let value = q["value"]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or("cycle schedule times must be finite quantities")?;
        let unit = q["unit"]
            .as_str()
            .ok_or("cycle schedule times require units")?;
        if time_factor(unit).is_none() || time_factor(tu).is_none() {
            return Err("cycle schedule requires supported time units".into());
        }
        convert(value, unit, tu, "cycle schedule time")
    };
    let reg = &mut request["regimen"];
    let start = if reg["start"].is_null() {
        0.0
    } else {
        quantity(&reg["start"])?
    };
    let end = quantity(&reg["end"])?;
    let mut times = Vec::new();
    for dose in reg["administrations"].as_array().into_iter().flatten() {
        if !targets.contains(&dose["target"]) {
            continue;
        }
        let amount = dose["amount"]["value"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .ok_or("cycle-trigger dose amount must be finite and nonnegative")?;
        let availability = dose
            .get("bioavailability")
            .and_then(Value::as_f64)
            .unwrap_or(1.0);
        if amount == 0.0 || availability == 0.0 {
            continue;
        }
        let at = quantity(&dose["time"])?
            + if dose["lag"].is_null() {
                0.0
            } else {
                quantity(&dose["lag"])?
            };
        let additional = if dose["repeat"].is_null() {
            0
        } else {
            dose["repeat"]["additional"]
                .as_u64()
                .filter(|n| *n <= 10_000)
                .ok_or("cycle-trigger repetition requires additional between 0 and 10000")?
        };
        let interval = if additional == 0 {
            0.0
        } else {
            quantity(&dose["repeat"]["interval"])?
        };
        if additional > 0 && interval <= 0.0 {
            return Err("cycle-trigger repetition interval must be positive".into());
        }
        for i in 0..=additional {
            let t = at + i as f64 * interval;
            if t >= start && t <= end {
                times.push(t)
            }
        }
        if times.len() > 10_000 {
            return Err("treatment cycle schedule exceeds 10000 trigger starts".into());
        }
    }
    times.sort_by(f64::total_cmp);
    times.dedup();
    if reg["covariates"].is_null() {
        reg["covariates"] = json!({})
    }
    reg["covariates"]
        .as_object_mut()
        .ok_or("regimen covariates must be an object")?
        .insert(name.into(), json!({"value":first,"unit":"1"}));
    if reg["covariate_changes"].is_null() {
        reg["covariate_changes"] = json!([])
    }
    let changes = reg["covariate_changes"]
        .as_array_mut()
        .ok_or("regimen covariate_changes must be an array")?;
    changes.retain(|row| row["name"] != name);
    for (i, time) in times.iter().enumerate() {
        changes.push(json!({"name":name,"time":{"value":time,"unit":tu},"value":{"value":first+i as f64,"unit":"1"}}));
    }
    Ok(vec![format!("Treatment cycle '{name}' follows {} distinct positive-dose starts on the archive's declared trigger targets.",times.len())])
}

pub(crate) fn to_model_time(q: &Value, tu: &str, what: &str) -> Result<Value, String> {
    crate::core::requests::quantity(q, what)?;
    let v = q["value"]
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{what}.value must be finite"))?;
    let u = q["unit"].as_str().unwrap_or("");
    if u == tu || u == "1" {
        return Ok(json!({"value":v,"unit":tu}));
    }
    match (time_factor(u), time_factor(tu)) {
        (Some(f), Some(m)) => {
            let converted = v * f / m;
            if !converted.is_finite() {
                return Err(format!(
                    "{what}: time conversion from '{u}' to '{tu}' must be finite"
                ));
            }
            Ok(json!({"value":converted,"unit":tu}))
        }
        _ => Err(format!(
            "{what}: '{u}' is not a time unit; the model's time unit is '{tu}'"
        )),
    }
}

pub(super) fn lookup<'a>(name: &str, table: &'a Value) -> Option<(String, &'a Value)> {
    if let Some(entry) = table.get(name) {
        return entry["native_name"].as_str().map(|n| (n.to_owned(), entry));
    }
    table
        .as_object()?
        .values()
        .find(|e| e["native_name"] == name)
        .map(|e| (name.to_owned(), e))
}

fn unknown(kind: &str, name: &str, table: &Value, usable: Option<&BTreeSet<String>>) -> String {
    let lower = name.to_ascii_lowercase();
    let parts: Vec<&str> = lower.split('_').filter(|p| p.len() > 2).collect();
    // Only names that change a run are worth suggesting; values merely stored with the model are left out.
    let names: Vec<&String> = table
        .as_object()
        .map(|m| {
            m.iter()
                .filter(|(_, e)| {
                    usable.is_none_or(|u| e["native_name"].as_str().is_some_and(|n| u.contains(n)))
                })
                .map(|(k, _)| k)
                .collect()
        })
        .unwrap_or_default();
    let mut scored: Vec<(usize, &String)> = names
        .iter()
        .copied()
        .filter_map(|n| {
            let l = n.to_ascii_lowercase();
            let shared = parts.iter().filter(|p| l.contains(**p)).count();
            let contains = l.contains(&lower) || lower.contains(&l);
            let starts = l.starts_with(&lower) || lower.starts_with(&l);
            (shared > 0 || contains).then(|| {
                (
                    shared * 10 + usize::from(contains) * 100 + usize::from(starts) * 200,
                    n,
                )
            })
        })
        .collect();
    // Closest first: whole-name containment, then shared words, then the shorter name.
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.len().cmp(&b.1.len()))
            .then(a.1.cmp(b.1))
    });
    let near: Vec<&String> = if scored.is_empty() {
        names.into_iter().take(12).collect()
    } else {
        scored.into_iter().take(8).map(|(_, n)| n).collect()
    };
    let lead = if near.len() == 1
        || (near.len() > 1
            && (near[0].to_ascii_lowercase().contains(&lower)
                || lower.contains(&near[0].to_ascii_lowercase())))
    {
        format!("Did you mean '{}'? ", near[0])
    } else {
        String::new()
    };
    format!(
        "unknown {kind} '{name}'. {lead}Closest names in this model: {}. get_model lists all of them with units.",
        near.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
    )
}

/// The archive's alias table, completed with names for outputs that have none: an output that is exactly one
/// state or parameter takes that quantity's readable name.
pub fn augment(native: &Value, model: &Value) -> Value {
    let state_rows = crate::library::display_states(model);
    let states: BTreeMap<_, _> = state_rows
        .iter()
        .filter_map(|row| row["name"].as_str().map(|name| (name, row)))
        .collect();
    let readouts: BTreeMap<_, _> = model["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["name"].as_str().map(|name| (name, &row["expression"])))
        .collect();
    let mut out = if native.is_object() {
        native.clone()
    } else {
        json!({})
    };
    // Libraries may keep state readouts separate from administration inputs.
    if native["input_states"].is_object() {
        out["states"] = native["input_states"].clone();
    } else {
        // Archived state readouts name outputs. Only a direct symbol readout
        // can also name an initial state; retain its source label separately.
        out["states"] = json!({});
        for (alias, entry) in native["states"].as_object().into_iter().flatten() {
            let readout = entry["native_name"]
                .as_str()
                .and_then(|name| readouts.get(name));
            if readout.is_some_and(|expression| expression["kind"] != "symbol") {
                continue;
            }
            let target = readout
                .map(|expression| expression["name"].clone())
                .unwrap_or_else(|| entry["native_name"].clone());
            if let Some(state) = target.as_str().and_then(|name| states.get(name)) {
                let mut binding = entry.clone();
                binding["native_name"] = target;
                binding["declared_source_unit"] = entry["unit"].clone();
                if readout.is_some() || binding["source_unit"].is_null() {
                    binding["source_unit"] = state["unit"].clone();
                }
                if readout.is_some() || binding["request_unit"].is_null() {
                    binding["request_unit"] = state["unit"].clone();
                }
                out["states"][alias] = binding;
            }
        }
    }
    // Canonical names always retain actual native units. Explicit archive
    // aliases keep their declared conversion coordinate and cannot shadow a
    // canonical name for a different input.
    if !out["parameters"].is_object() {
        out["parameters"] = json!({});
    }
    for (field, table, quantity) in [
        ("parameters", "parameters", "default"),
        ("states", "states", ""),
    ] {
        let rows = if field == "states" {
            state_rows.clone()
        } else {
            model[field].as_array().cloned().unwrap_or_default()
        };
        for row in rows {
            if let Some(name) = row["name"].as_str() {
                if out[table].get(name).is_none() {
                    let unit = if quantity.is_empty() {
                        &row["unit"]
                    } else {
                        &row[quantity]["unit"]
                    };
                    out[table][name] = json!({"native_name":name,"source_unit":unit,"request_unit":unit,"canonical":true});
                }
            }
        }
    }
    if !out["outputs"].is_object() {
        out["outputs"] = json!({});
    }
    for o in model["outputs"].as_array().into_iter().flatten() {
        let Some(name) = o["name"].as_str() else {
            continue;
        };
        let known = out["outputs"]
            .as_object()
            .is_some_and(|m| m.values().any(|e| e["native_name"] == name || *e == name));
        if known || o["expression"]["kind"] != "symbol" {
            continue;
        }
        let symbol = o["expression"]["name"].as_str().unwrap_or("");
        for table in ["states", "parameters"] {
            let found = native[table].as_object().and_then(|m| {
                m.iter()
                    .find(|(_, e)| {
                        e["native_name"] == symbol
                            || (table == "states" && e["native_name"] == name)
                    })
                    .map(|(k, e)| {
                        (
                            k.clone(),
                            e["source_unit"]
                                .as_str()
                                .or_else(|| e["unit"].as_str())
                                .map(Value::from)
                                .unwrap_or(Value::Null),
                        )
                    })
            });
            if let Some((alias, unit)) = found {
                if out["outputs"].get(&alias).is_none() {
                    out["outputs"][alias] = json!({"native_name":name,"unit":unit});
                }
                break;
            }
        }
    }
    out
}

/// Validate executable input bindings against declarations before preparation.
/// Readout source labels are not used to assign physical units to native states.
pub fn validate_input_aliases(model: &Value, named: &Value) -> Result<(), String> {
    for (table, rows, quantity) in [
        (
            "parameters",
            model["parameters"].as_array().cloned().unwrap_or_default(),
            "default",
        ),
        ("states", crate::library::display_states(model), ""),
    ] {
        let declarations: BTreeMap<&str, &Value> = rows
            .iter()
            .filter_map(|r| r["name"].as_str().map(|name| (name, r)))
            .collect();
        for (alias, entry) in named[table].as_object().into_iter().flatten() {
            let target = entry["native_name"].as_str().ok_or_else(|| {
                format!(
                    "{table}.{alias} has no canonical native name; inspect the current model card"
                )
            })?;
            let row = declarations.get(target).ok_or_else(|| format!("{table}.{alias} names missing native input '{target}'; inspect the current version and input bindings before migrating this alias"))?;
            if declarations.contains_key(alias.as_str()) && alias != target {
                return Err(format!("{table}.{alias} shadows a different canonical input '{target}'; correct the archive alias rather than silently remapping the input"));
            }
            let unit = if quantity.is_empty() {
                row["unit"].as_str()
            } else {
                row[quantity]["unit"].as_str()
            }
            .unwrap_or("1");
            let request_unit = entry["request_unit"].as_str().unwrap_or("1");
            convert(
                1.0,
                request_unit,
                unit,
                &format!("{table}.{alias}.request_unit"),
            )?;
        }
    }
    Ok(())
}

/// `auc(output, 42, 168)` or `trough(output, 84)` written as text.
fn shorthand(text: &str) -> Option<Value> {
    let (kind, rest) = text.trim().split_once('(')?;
    let parts: Vec<&str> = rest.strip_suffix(')')?.split(',').map(str::trim).collect();
    let number = |i: usize| parts.get(i)?.parse::<f64>().ok();
    match (kind.trim(), parts.len()) {
        ("nca" | "auc" | "min" | "max", 3) => {
            Some(json!({"kind":kind.trim(),"output":parts[0],"from":number(1)?,"to":number(2)?}))
        }
        ("trough" | "value", 2) => {
            Some(json!({"kind":kind.trim(),"output":parts[0],"at":number(1)?}))
        }
        _ => None,
    }
}

/// Factors of a dose scale other than 1 and the engine's internal unit-basis symbols.
fn scale_factors<'a>(expr: &'a Value, out: &mut Vec<&'a Value>) {
    if expr["kind"] == "binary" && expr["operator"] == "multiply" {
        for a in expr["arguments"].as_array().into_iter().flatten() {
            scale_factors(a, out);
        }
    } else if !(expr["kind"] == "literal" && expr["value"] == 1.0)
        && !expr["name"]
            .as_str()
            .is_some_and(|n| n.starts_with("legacy_unit_basis"))
    {
        out.push(expr);
    }
}

/// The unit a dose into `state` is given in, and the scale the model applies to it. A model may take the dose in
/// one unit (mg) and convert it into the state's unit (nmol) itself; the state's unit is then the wrong unit to dose in.
pub fn dose_unit(model: &Value, native: &Value, state: &str) -> (Option<String>, Option<String>) {
    let state_unit = native["states"]
        .as_object()
        .and_then(|m| m.values().find(|e| e["native_name"] == state))
        .and_then(|e| e["source_unit"].as_str())
        .map(str::to_owned);
    let resolved_states = crate::library::display_states(model);
    let Some(row) = resolved_states.iter().find(|st| st["name"] == state) else {
        return (state_unit, None);
    };
    let mut factors = Vec::new();
    scale_factors(&row["dosing"]["scale"], &mut factors);
    if !model["linear_pk"].is_null() {
        let scale = (!factors.is_empty()).then(|| {
            factors
                .iter()
                .map(|f| render(f, native))
                .collect::<Vec<_>>()
                .join(" * ")
        });
        return (
            row["dosing"]["amount_unit"].as_str().map(str::to_owned),
            scale,
        );
    }
    let declared = native["states"]
        .as_object()
        .and_then(|m| m.values().find(|e| e["native_name"] == state))
        .and_then(|e| e["dose_amount_unit"].as_str());
    if let Some(unit) = declared {
        let scale = (!factors.is_empty()).then(|| {
            factors
                .iter()
                .map(|f| render(f, native))
                .collect::<Vec<_>>()
                .join(" * ")
        });
        return (Some(unit.to_owned()), scale);
    }
    if factors.is_empty() {
        return (state_unit, None);
    }
    let text = factors
        .iter()
        .map(|f| render(f, native))
        .collect::<Vec<_>>()
        .join(" * ");
    if let [single] = factors.as_slice() {
        let unit = native["parameters"]
            .as_object()
            .and_then(|m| m.values().find(|e| e["native_name"] == single["name"]))
            .and_then(|e| e["source_unit"].as_str());
        if let (Some(unit), Some(state_unit)) = (unit, state_unit.as_deref()) {
            if dimensionless(unit) {
                return (Some(state_unit.to_owned()), Some(text));
            }
            if let Some((numerator, denominator)) = unit.split_once('/') {
                let same = numerator.trim() == state_unit
                    || parse(numerator)
                        .zip(parse(state_unit))
                        .is_some_and(|(a, b)| a == b);
                if same && parse(denominator).is_some() {
                    return (Some(denominator.trim().to_owned()), Some(text));
                }
            }
        }
    }
    (None, Some(text))
}

fn span(q: &Value) -> f64 {
    q["value"].as_f64().unwrap_or(0.0)
}

fn validate_readout(
    spec: &mut Value,
    index: usize,
    native: &Value,
    model: &Value,
    notes: &mut Vec<String>,
    location: &mut NormalizationLocation,
) -> Result<(), String> {
    let field = format!("readouts[{index}]");
    let path = format!("/readouts/{index}");
    location.at(
        super::input_shapes::child(&path, "kind"),
        READOUT_INPUT_HINT,
    );
    let kind = spec["kind"]
        .as_str()
        .ok_or_else(|| format!("{field}.kind is required"))?
        .to_owned();
    if ![
        "nca",
        "auc",
        "min",
        "max",
        "time_below",
        "time_above",
        "first_crossing",
        "trough",
        "value",
    ]
    .contains(&kind.as_str())
    {
        return Err(format!("{field}.kind '{kind}' is unknown; use nca, auc, min, max, time_below, time_above, first_crossing, trough or value"));
    }
    location.at(super::input_shapes::child(&path, "output"), "Use an output listed by get_model; readout aliases must resolve to an actual native output.");
    let name = spec["output"]
        .as_str()
        .ok_or_else(|| format!("{field}.output is required"))?;
    let wanted = native["outputs"][name]
        .as_str()
        .or_else(|| native["outputs"][name]["native_name"].as_str())
        .unwrap_or(name);
    let output = model["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|o| o["name"] == wanted)
        .ok_or_else(|| {
            format!("{field}.output '{name}' is unknown; use an output listed by get_model")
        })?;
    let point = kind == "trough" || kind == "value";
    location.at(path.clone(), READOUT_INPUT_HINT);
    if point && (!spec["from"].is_null() || !spec["to"].is_null()) {
        return Err(format!(
            "{field}.from/to applies to window readouts; use at for {kind}"
        ));
    }
    if !point && !spec["at"].is_null() {
        location.at(super::input_shapes::child(&path, "at"), READOUT_INPUT_HINT);
        return Err(format!(
            "{field}.at applies to value or trough; use from/to for {kind}"
        ));
    }
    location.at(
        super::input_shapes::child(&path, "side"),
        "Use pre or post for value, pre for trough, and omit side for window readouts.",
    );
    if spec["side"]
        .as_str()
        .is_some_and(|s| !["pre", "post"].contains(&s))
        || !spec["side"].is_null()
            && (kind != "value" && kind != "trough" || !spec["side"].is_string())
        || kind == "trough" && spec["side"] == "post"
    {
        return Err(format!(
            "{field}.side must be pre/post for value, or pre for trough"
        ));
    }
    location.at(
        super::input_shapes::child(&path, "direction"),
        "Use up, down or any for first_crossing; omit direction for other readout kinds.",
    );
    if !spec["direction"].is_null()
        && (kind != "first_crossing"
            || !spec["direction"]
                .as_str()
                .is_some_and(|d| ["up", "down", "any"].contains(&d)))
    {
        return Err(format!(
            "{field}.direction must be up, down or any for first_crossing"
        ));
    }
    let threshold_kind = ["time_below", "time_above", "first_crossing"].contains(&kind.as_str());
    if threshold_kind {
        location.at(
            super::input_shapes::child(&path, "output"),
            "Choose an output with a declared native unit before interpreting a threshold.",
        );
        let stored = output["unit"]
            .as_str()
            .ok_or_else(|| format!("{field}.output has no declared unit"))?;
        location.at(super::input_shapes::child(&path, "threshold"), "Supply a finite threshold in the native output unit or a compatible quantity; display aliases do not establish physical dimensions.");
        let threshold = if spec["threshold"].is_object() {
            let q =
                crate::core::requests::quantity(&spec["threshold"], &format!("{field}.threshold"))?;
            let value = q["value"].as_f64().unwrap();
            let unit = q["unit"].as_str().unwrap();
            // Output values use the actual engine unit. Archive display aliases
            // cannot establish a physical conversion for source-scale values.
            let converted = convert(value, unit, stored, &format!("{field}.threshold"))?;
            notes.push(format!(
                "{field}.threshold: {value} {unit} = {converted} {stored}"
            ));
            converted
        } else {
            let value = spec["threshold"].as_f64().filter(|v| v.is_finite())
                .ok_or_else(|| format!("{field}.threshold must be a finite number in output unit '{stored}', or an explicit compatible quantity"))?;
            notes.push(format!(
                "{field}.threshold {value} is in the engine output unit '{stored}'"
            ));
            value
        };
        if !threshold.is_finite() {
            return Err(format!("{field}.threshold conversion must be finite"));
        }
        spec["threshold"] = json!(threshold);
    } else if !spec["threshold"].is_null() {
        location.at(
            super::input_shapes::child(&path, "threshold"),
            "Use threshold only for time_below, time_above or first_crossing.",
        );
        return Err(format!(
            "{field}.threshold applies only to time_below, time_above or first_crossing"
        ));
    }
    Ok(())
}

/// Translate readable names and units in simulate-style arguments into the
/// model's native request. Returns notes describing every conversion made.
pub fn normalize(
    args: &mut Value,
    native: &Value,
    model: &Value,
    base: Option<&Value>,
) -> Result<Vec<String>, String> {
    normalize_located(args, native, model, base).map_err(super::input_shapes::Failure::message)
}

struct NormalizationLocation {
    pointer: String,
    suggestion: Cow<'static, str>,
}
impl NormalizationLocation {
    fn at(&mut self, pointer: String, suggestion: &'static str) {
        self.pointer = pointer;
        self.suggestion = Cow::Borrowed(suggestion);
    }
    fn quantity_at(&mut self, pointer: String, native: &Value, entry: &Value, stored: &str) {
        self.at(pointer, NAMED_INPUT_HINT);
        let source_label = entry["declared_source_unit"]
            .as_str()
            .or(entry["unit"].as_str());
        let unchanged = native["coordinates"] == "unchanged_source_numeric_values";
        if unchanged || source_label.is_some_and(|label| label != stored) {
            let bounded = |text: &str| {
                let value: String = text.chars().take(128).collect();
                if value.len() < text.len() {
                    format!("{value}…")
                } else {
                    value
                }
            };
            let coordinate = if unchanged {
                "unchanged_source_numeric_values (archive declaration)"
            } else {
                "not declared as unchanged source coordinates"
            };
            let label = source_label.map(bounded);
            self.suggestion = Cow::Owned(format!(
                "Use a quantity compatible with the accepted alias coordinate {stored:?}. Archive source label: {}. Coordinate mode: {coordinate}. Source labels do not establish native physical dimensions or a physical conversion factor. Inspect the model input bindings; select a dimensionally qualified model when physical conversion is required.",
                label.map(|label| format!("{label:?}")).unwrap_or_else(|| "undeclared".into())
            ));
        }
    }
    fn failure(self, message: String) -> super::input_shapes::Failure {
        super::input_shapes::Failure::at(self.pointer, message, self.suggestion.into_owned())
    }
}
const NAMED_INPUT_HINT: &str = "Use one declared native name or alias per input and a quantity compatible with its declared unit; inspect get_model for supported inputs.";
const TIME_INPUT_HINT: &str = "Supply a finite time quantity compatible with the model time unit and inside the intended run window.";
const READOUT_INPUT_HINT: &str = "Supply a supported readout kind and declared output with one unambiguous time or window; inspect the readout contract and preserve native observation sides.";

/// Retain a native dimensional error while exposing the selected archive coordinate mode.
/// No request, scientific result, error code or native error location is changed.
pub(crate) fn source_coordinate_refusal(mut error: Value, native: &Value) -> Value {
    const CONTEXT: &str = "Archive coordinate mode: unchanged_source_numeric_values. Native unit declarations govern this request; archived source labels remain separate descriptions. This mode does not establish a physical conversion factor or dimensional qualification. Inspect the current model input bindings before changing units.";
    if native["coordinates"] != "unchanged_source_numeric_values" {
        return error;
    }
    let Some(detail) = error.get_mut("error").and_then(Value::as_object_mut) else {
        return error;
    };
    if !matches!(
        detail.get("code").and_then(Value::as_str),
        Some("unit" | "unknown_unit")
    ) {
        return error;
    }
    let previous = detail
        .get("suggestion")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !previous.ends_with(CONTEXT) {
        let suggestion = if previous.is_empty() {
            CONTEXT.into()
        } else {
            format!("{previous} {CONTEXT}")
        };
        detail.insert("suggestion".into(), Value::String(suggestion));
    }
    error
}

/// Normalize a candidate copy and commit only after all checks succeed.
/// The original arguments remain available unchanged after any refusal.
pub(crate) fn normalize_located(
    args: &mut Value,
    native: &Value,
    model: &Value,
    base: Option<&Value>,
) -> Result<Vec<String>, super::input_shapes::Failure> {
    super::input_shapes::admit(args)?;
    let mut candidate = args.clone();
    let mut location = NormalizationLocation {
        pointer: String::new(),
        suggestion: Cow::Borrowed("Correct the requested workflow using the model's declared inputs, outputs, units and supported operation contract."),
    };
    let notes = normalize_admitted(&mut candidate, native, model, base, &mut location)
        .map_err(|message| location.failure(message))?;
    *args = candidate;
    Ok(notes)
}
fn normalize_admitted(
    args: &mut Value,
    native: &Value,
    model: &Value,
    base: Option<&Value>,
    location: &mut NormalizationLocation,
) -> Result<Vec<String>, String> {
    let tu = model["time_unit"].as_str().unwrap_or("d").to_owned();
    let mut notes = Vec::new();
    let usable = equation_parameters(model);
    for (key, table_key, kind) in [
        ("parameters", "parameters", "parameter"),
        ("initial_states", "states", "state"),
    ] {
        let table = &native[table_key];
        let Some(given) = args.get(key).and_then(Value::as_object).cloned() else {
            continue;
        };
        if !table.is_object() {
            continue;
        }
        let mut out = Map::new();
        let mut supplied = BTreeMap::new();
        for (name, q) in given {
            let pointer = super::input_shapes::child(&format!("/{key}"), &name);
            location.at(pointer.clone(), NAMED_INPUT_HINT);
            let q = crate::core::requests::quantity(&q, &format!("{key}.{name}"))?;
            let (native_name, entry) = lookup(&name, table).ok_or_else(|| {
                unknown(
                    kind,
                    &name,
                    table,
                    (kind == "parameter" && !usable.is_empty()).then_some(&usable),
                )
            })?;
            if let Some(first) = supplied.insert(native_name.clone(), name.clone()) {
                return Err(format!("{key}.{name} and {key}.{first} both name '{native_name}'; supply one alias or native name, not both"));
            }
            // Native quantities retain their original unit until native
            // preparation, which provides its located dimensional diagnostic.
            if entry["canonical"] == true {
                out.insert(native_name, q);
                continue;
            }
            let stored = entry["source_unit"].as_str().unwrap_or("1");
            let value = q["value"]
                .as_f64()
                .ok_or_else(|| format!("{key}.{name}.value must be a number"))?;
            let unit = q["unit"].as_str().unwrap_or("1");
            location.at(
                super::input_shapes::child(&pointer, "unit"),
                NAMED_INPUT_HINT,
            );
            let converted =
                convert(value, unit, stored, &format!("{kind} '{name}'")).map_err(|message| {
                    location.quantity_at(
                        super::input_shapes::child(&pointer, "unit"),
                        native,
                        &entry,
                        stored,
                    );
                    message
                })?;
            if converted != value {
                notes.push(format!("{name}: {value} {unit} = {converted} {stored}"));
            }
            out.insert(
                native_name,
                json!({"value":converted,"unit":entry["request_unit"].as_str().unwrap_or("1")}),
            );
        }
        args[key] = Value::Object(out);
    }
    let mut latest: f64 = 0.0;
    let mut interval: f64 = 0.0;
    if let Some(doses) = args.get_mut("doses").and_then(Value::as_array_mut) {
        let resolved_states = crate::library::display_states(model);
        for (index, d) in doses.iter_mut().enumerate() {
            let dose_path = format!("/doses/{index}");
            location.at(super::input_shapes::child(&dose_path, "target"), "Use a declared dosing target and supported administration mode; choose a stored scenario when dosing is fixed in the source.");
            let states = &native["states"];
            if states.is_object() {
                let name = d["target"]
                    .as_str()
                    .ok_or("dose.target required")?
                    .to_owned();
                let (native_name, entry) = lookup(&name, states).ok_or_else(|| {
                    let dosing: Map<String, Value> = states
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(_, e)| {
                            resolved_states
                                .iter()
                                .any(|st| st["name"] == e["native_name"] && !st["dosing"].is_null())
                        })
                        .map(|(k, e)| (k.clone(), e.clone()))
                        .collect();
                    let list = dosing.keys().cloned().collect::<Vec<_>>().join(", ");
                    format!("unknown dose target '{name}'. This model can be dosed into: {list}")
                })?;
                if resolved_states
                    .iter()
                    .any(|st| st["name"] == native_name.as_str() && st["dosing"].is_null())
                {
                    let list: Vec<&String> = states
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(_, e)| {
                            resolved_states
                                .iter()
                                .any(|st| st["name"] == e["native_name"] && !st["dosing"].is_null())
                        })
                        .map(|(k, _)| k)
                        .collect();
                    return Err(if list.is_empty() {
                        format!("'{name}' cannot be dosed: this model's dosing is stored as a fixed input schedule. Run one of its scenarios; the doses cannot be changed through this server.")
                    } else {
                        format!("'{name}' is a state but not a dose target. This model can be dosed into: {}", list.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
                    });
                }
                if let Some(declared) = entry.get("dose_amount_unit") {
                    if !declared.as_str().is_some_and(|u| {
                        !u.trim().is_empty() && (parse(u).is_some() || dimensionless(u))
                    }) {
                        return Err(format!("dose into '{name}': the archive declares an unsupported dose input unit {declared}. Correct dose_amount_unit in its state metadata before using named doses."));
                    }
                }
                let (dose_in, scale) = dose_unit(model, native, &native_name);
                location.at(format!("{dose_path}/amount/value"), "Supply a finite dose amount in the declared input unit before any model dose scaling.");
                let value = d["amount"]["value"]
                    .as_f64()
                    .ok_or("dose.amount.value must be a number")?;
                let unit = d["amount"]["unit"].as_str().unwrap_or("1").to_owned();
                let stored = dose_in.as_deref().unwrap_or("1");
                location.at(format!("{dose_path}/amount/unit"), "Use a quantity compatible with the declared dose input unit; source-coordinate doses require explicit unit 1 when no physical dose unit is declared.");
                let converted = match dose_in.as_deref() {
                    Some(stored) => convert(value, &unit, stored, &format!("dose into '{name}'"))
                        .map_err(|e| match &scale {
                        Some(s) => format!("{e} The model multiplies the dose by {s}."),
                        None => e,
                    })?,
                    None if unit == "1" => value,
                    None => return Err(format!("dose.amount.unit for '{name}': the model has no declared dose input unit. Use explicit unit '1' with a source-coordinate amount from a stored scenario, or a model with a declared physical dose unit.")),
                };
                if converted != value {
                    notes.push(format!("dose: {value} {unit} = {converted} {stored}"));
                }
                d["target"] = json!(native_name);
                let request_unit = if !model["linear_pk"].is_null() {
                    stored
                } else {
                    entry["request_unit"].as_str().unwrap_or("1")
                };
                d["amount"] = json!({"value":converted,"unit":request_unit});
            }
            for field in ["at", "every", "duration", "lag"] {
                if d.get(field).is_some_and(|v| !v.is_null()) {
                    location.at(
                        super::input_shapes::child(&dose_path, field),
                        TIME_INPUT_HINT,
                    );
                    d[field] = to_model_time(&d[field], &tu, &format!("dose.{field}"))?;
                }
            }
            let repeats = d["count"].as_u64().unwrap_or(1).max(1) as f64 - 1.0;
            latest =
                latest.max(span(&d["at"]) + repeats * span(&d["every"]) + span(&d["duration"]));
            interval = interval.max(span(&d["every"]));
        }
    }
    if let Some(times) = args.get_mut("times").and_then(Value::as_array_mut) {
        for (index, q) in times.iter_mut().enumerate() {
            location.at(format!("/times/{index}"), TIME_INPUT_HINT);
            *q = to_model_time(q, &tu, "times")?;
            latest = latest.max(span(q));
        }
        args["_requested_times"] = args["times"].clone();
    }
    if args.get("end").is_some_and(|v| !v.is_null()) {
        location.at("/end".into(), TIME_INPUT_HINT);
        args["end"] = to_model_time(&args["end"], &tu, "end")?;
        if args["end"]["value"].as_f64().is_some_and(|end| end <= 0.0) {
            location.at("/end/value".into(), "Choose a positive run end and include time zero in times when starting values are needed.");
            return Err("end.value must be positive; to observe starting values, choose an explicit positive end and include time zero in times".into());
        }
    }
    let args_end = args.get("end").and_then(|e| e["value"].as_f64());
    let native_regimen = args.get("request").or(base).map(|r| &r["regimen"]);
    let native_end = native_regimen
        .and_then(|r| to_model_time(&r["end"], &tu, "regimen.end").ok())
        .and_then(|q| q["value"].as_f64());
    location.at("/request/regimen/start".into(), TIME_INPUT_HINT);
    let native_start = native_regimen
        .filter(|r| !r["start"].is_null())
        .map(|r| to_model_time(&r["start"], &tu, "regimen.start"))
        .transpose()?
        .map_or(0.0, |q| q["value"].as_f64().unwrap());
    let native_observations =
        native_regimen.is_some_and(|r| r["observations"].as_array().is_some_and(|a| !a.is_empty()));
    if native_observations
        && args["readouts"].as_array().is_some_and(|a| !a.is_empty())
        && args.get("times").is_none()
    {
        location.at("/readouts".into(), READOUT_INPUT_HINT);
        return Err("readouts would replace request.regimen.observations with a dense grid. Preserve the native observation plan by omitting readouts, or explicitly choose times to replace it.".into());
    }
    if let Some(specs) = args
        .get_mut("readouts")
        .and_then(Value::as_array_mut)
        .filter(|a| !a.is_empty())
    {
        let mut extra = Vec::new();
        let mut windows = BTreeSet::new();
        // Each materialized quantity requires at least this many encoded bytes.
        // Bound expansion before allocation; policy::admit checks exact bytes too.
        let grid_limit = crate::policy::INPUT_LIMIT / b"{\"unit\":\"\",\"value\":0}".len();
        let horizon = args_end
            .or(native_end)
            .filter(|e| *e > 0.0)
            .unwrap_or(latest + interval);
        for (index, spec) in specs.iter_mut().enumerate() {
            let readout_path = format!("/readouts/{index}");
            location.at(readout_path.clone(), READOUT_INPUT_HINT);
            // Accept the shapes callers actually send: a JSON string, a nested window or time, a threshold quantity.
            if let Some(call) = spec.as_str().and_then(shorthand) {
                *spec = call;
            }
            if let Some(text) = spec.as_str() {
                *spec = serde_json::from_str::<Value>(text).ok().filter(Value::is_object).ok_or_else(|| {
                    format!("each readout must be an object, for example {{\"kind\":\"auc\",\"output\":\"NAME\",\"from\":{{\"value\":0,\"unit\":\"{tu}\"}},\"to\":{{\"value\":28,\"unit\":\"{tu}\"}}}}; got the text '{}'", text.chars().take(80).collect::<String>())
                })?;
            }
            if !spec.is_object() {
                return Err("each readout must be an object with kind, output and a window (from, to) or a time (at)".into());
            }
            for key in spec.as_object().unwrap().keys() {
                if ![
                    "kind",
                    "output",
                    "from",
                    "to",
                    "at",
                    "threshold",
                    "direction",
                    "side",
                    "window",
                    "time",
                    "interval",
                    "start",
                    "end",
                    "stop",
                ]
                .contains(&key.as_str())
                {
                    location.at(
                        super::input_shapes::child(&readout_path, key),
                        READOUT_INPUT_HINT,
                    );
                    return Err(format!("readouts[{index}].{key} is unknown; use kind, output, from/to or at, and the declared threshold/direction/side fields"));
                }
            }
            for nested in ["window", "time", "interval"] {
                if let Some(inner) = spec.get(nested).cloned() {
                    if inner.is_object()
                        && (inner.get("from").is_some()
                            || inner.get("to").is_some()
                            || inner.get("at").is_some())
                    {
                        if let Some(key) = inner
                            .as_object()
                            .unwrap()
                            .keys()
                            .find(|k| !["from", "to", "at"].contains(&k.as_str()))
                        {
                            location.at(
                                super::input_shapes::child(
                                    &super::input_shapes::child(&readout_path, nested),
                                    key,
                                ),
                                READOUT_INPUT_HINT,
                            );
                            return Err(format!(
                                "readouts[{index}].{nested}.{key} is unknown; use from/to or at"
                            ));
                        }
                        for field in ["from", "to", "at"] {
                            if !spec[field].is_null()
                                && !inner[field].is_null()
                                && spec[field] != inner[field]
                            {
                                location.at(
                                    super::input_shapes::child(&readout_path, field),
                                    READOUT_INPUT_HINT,
                                );
                                return Err(format!("readouts[{index}].{field} conflicts with {nested}.{field}; supply one value"));
                            }
                            if spec[field].is_null() && !inner[field].is_null() {
                                spec[field] = inner[field].clone();
                            }
                        }
                    } else if nested == "time" {
                        if !spec["at"].is_null() && spec["at"] != inner {
                            location.at(
                                super::input_shapes::child(&readout_path, "at"),
                                READOUT_INPUT_HINT,
                            );
                            return Err(format!(
                                "readouts[{index}].at conflicts with time; supply one value"
                            ));
                        }
                        spec["at"] = inner;
                    } else {
                        location.at(
                            super::input_shapes::child(&readout_path, nested),
                            READOUT_INPUT_HINT,
                        );
                        return Err(format!(
                            "readouts[{index}].{nested} must contain from/to or at"
                        ));
                    }
                }
            }
            for (alias, field) in [("start", "from"), ("end", "to"), ("stop", "to")] {
                if !spec[field].is_null() && !spec[alias].is_null() && spec[field] != spec[alias] {
                    location.at(
                        super::input_shapes::child(&readout_path, field),
                        READOUT_INPUT_HINT,
                    );
                    return Err(format!(
                        "readouts[{index}].{field} conflicts with {alias}; supply one value"
                    ));
                }
                if spec[field].is_null() && !spec[alias].is_null() {
                    spec[field] = spec[alias].clone();
                }
            }
            location.at(readout_path.clone(), READOUT_INPUT_HINT);
            validate_readout(spec, index, native, model, &mut notes, location)?;
            for field in ["from", "to", "at"] {
                if spec[field].is_number() {
                    spec[field] = json!({"value":spec[field],"unit":tu});
                }
                if spec.get(field).is_some_and(|v| !v.is_null()) {
                    location.at(
                        super::input_shapes::child(&readout_path, field),
                        TIME_INPUT_HINT,
                    );
                    spec[field] = to_model_time(&spec[field], &tu, &format!("readout.{field}"))?;
                }
            }
            let kind = spec["kind"].as_str().unwrap_or("");
            if kind != "trough" && kind != "value" && spec["at"].is_null() && horizon > 0.0 {
                // No window given: the whole run (or from `from` to its end).
                if spec["from"].is_null() {
                    spec["from"] = json!({"value":native_start,"unit":tu});
                }
                if spec["to"].is_null() {
                    spec["to"] = json!({"value":horizon,"unit":tu});
                }
            }
            if let Some(at) = spec["at"]["value"].as_f64() {
                if extra.len() >= grid_limit {
                    location.at(
                        super::input_shapes::child(&readout_path, "at"),
                        READOUT_INPUT_HINT,
                    );
                    return Err(format!("readouts[{index}].at exceeds the bounded input grid allowance; request fewer readout times"));
                }
                extra.push(at);
            }
            if let (Some(a), Some(b)) =
                (spec["from"]["value"].as_f64(), spec["to"]["value"].as_f64())
            {
                if b <= a {
                    location.at(
                        super::input_shapes::child(&readout_path, "to"),
                        READOUT_INPUT_HINT,
                    );
                    return Err(format!(
                        "readouts[{index}].to must be later than readouts[{index}].from"
                    ));
                }
                if !((b - a) * READOUT_POINTS as f64).is_finite() {
                    location.at(readout_path.clone(), READOUT_INPUT_HINT);
                    return Err(format!("readouts[{index}].from/to interval is too large for a finite sampling grid"));
                }
                // A dense, even grid inside the window; dose events add their own pre/post rows.
                if windows.insert((a.to_bits(), b.to_bits())) {
                    if extra
                        .len()
                        .checked_add(READOUT_POINTS + 1)
                        .is_none_or(|n| n > grid_limit)
                    {
                        location.at(readout_path.clone(), READOUT_INPUT_HINT);
                        return Err(format!("readouts[{index}].from/to exceeds the bounded input grid allowance; use fewer distinct windows"));
                    }
                    extra.extend(
                        (0..=READOUT_POINTS)
                            .map(|i| a + (b - a) * i as f64 / READOUT_POINTS as f64),
                    );
                }
            } else if spec["at"].is_null() {
                location.at(readout_path.clone(), READOUT_INPUT_HINT);
                return Err(format!(
                    "readouts[{index}] needs a window (from, to) or a time (at)"
                ));
            }
        }
        let mut all: Vec<f64> = args["times"]
            .as_array()
            .map(|a| a.iter().map(span).collect())
            .unwrap_or_default();
        all.extend(extra);
        all.sort_by(f64::total_cmp);
        all.dedup();
        latest = all.last().copied().unwrap_or(latest).max(latest);
        args["times"] = json!(all
            .iter()
            .map(|t| json!({"value":t,"unit":tu}))
            .collect::<Vec<_>>());
    }
    if let Some(regimen) = args
        .get_mut("request")
        .and_then(|r| r.get_mut("regimen"))
        .filter(|r| r.is_object())
    {
        if regimen["samples"].as_array().is_some_and(|a| !a.is_empty())
            && regimen["observations"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        {
            location.at("/request/regimen/samples".into(), "Supply either samples or observations; preserve pre/post observation sides when those are needed.");
            return Err("request.regimen.samples conflicts with request.regimen.observations; supply one explicit grid or a sided observation plan".into());
        }
    } else if latest > 0.0 || interval > 0.0 {
        if args.get("times").is_none_or(Value::is_null) && base.is_none() {
            // Doses only: follow the last dose for one more dosing interval.
            latest += interval;
        }
        let given = args.get("end").and_then(|e| e["value"].as_f64());
        let preset_end = base.and_then(|b| b["regimen"]["end"]["value"].as_f64());
        if given.is_none() && preset_end.is_none() {
            args["end"] = json!({"value":latest,"unit":tu});
            notes.push(format!(
                "end time set to {latest} {tu} to cover the last requested time or dose"
            ));
        }
    }
    Ok(notes)
}

pub const READOUT_POINTS: usize = 4000;

fn symbols(expr: &Value, out: &mut BTreeSet<String>) {
    match expr {
        Value::Object(m) => {
            if m.get("kind").is_some_and(|k| k == "symbol") {
                if let Some(n) = m.get("name").and_then(Value::as_str) {
                    out.insert(n.to_owned());
                }
            }
            for v in m.values() {
                symbols(v, out)
            }
        }
        Value::Array(a) => a.iter().for_each(|v| symbols(v, out)),
        _ => {}
    }
}

/// Parameter values that change at given times. The engine holds parameters constant and lets declared
/// time-varying inputs step, so each changed parameter is declared as such an input for this run: its starting
/// value is the one the run would have used, and every change is a step at its time. Nothing else in the model
/// is touched.
pub fn parameter_changes(
    source: &str,
    request: &mut Value,
    changes: &[Value],
    native: &Value,
    model: &Value,
) -> Result<(String, Vec<String>), String> {
    let tu = model["time_unit"].as_str().unwrap_or("d").to_owned();
    let table = &native["parameters"];
    if !table.is_object() {
        return Err("parameter_changes needs a library model with named parameters".into());
    }
    let usable = equation_parameters(model);
    let mut doc: Value = serde_json::from_str(source).map_err(|e| e.to_string())?;
    let mut steps: Vec<(f64, Value)> = Vec::new();
    let mut notes = Vec::new();
    let integration_start = request["regimen"]
        .get("start")
        .filter(|v| !v.is_null())
        .map(|q| to_model_time(q, &tu, "regimen.start"))
        .transpose()?
        .and_then(|q| q["value"].as_f64())
        .unwrap_or(0.);
    for c in changes {
        let name = c["name"]
            .as_str()
            .or(c["parameter"].as_str())
            .ok_or("parameter_changes entries need name, at and value")?;
        let (native_name, entry) = lookup(name, table).ok_or_else(|| {
            unknown(
                "parameter",
                name,
                table,
                (!usable.is_empty()).then_some(&usable),
            )
        })?;
        let stored = entry["source_unit"].as_str().unwrap_or("1");
        let value = c["value"]["value"]
            .as_f64()
            .or(c["value"].as_f64())
            .ok_or_else(|| {
                format!("parameter_changes '{name}': value must be a number or {{value, unit}}")
            })?;
        let value = convert(
            value,
            c["value"]["unit"].as_str().unwrap_or("1"),
            stored,
            &format!("parameter '{name}'"),
        )?;
        let at = to_model_time(&c["at"], &tu, "parameter_changes.at")?;
        let t = at["value"].as_f64().unwrap_or(0.0);
        if t < integration_start {
            return Err(format!(
                "parameter_changes '{name}': at {t} {tu} precedes integration start {integration_start} {tu}; change the initial parameter or choose a boundary inside the run"
            ));
        }
        if !doc["covariates"]
            .as_array()
            .is_some_and(|cs| cs.iter().any(|x| x["name"] == native_name.as_str()))
        {
            let parameters = doc["parameters"]
                .as_array_mut()
                .ok_or("model has no parameters")?;
            let i = parameters
                .iter()
                .position(|p| p["name"] == native_name.as_str())
                .ok_or_else(|| format!("'{name}' is not a parameter of this model"))?;
            let p = parameters.remove(i);
            let unit = p["default"]["unit"].as_str().unwrap_or("1").to_owned();
            let start = request["parameters"][&native_name]["value"]
                .as_f64()
                .or(p["default"]["value"].as_f64())
                .ok_or_else(|| format!("'{name}' has no starting value; give one in parameters"))?;
            if let Some(m) = request.get_mut("parameters").and_then(Value::as_object_mut) {
                m.remove(&native_name);
            }
            if !doc["covariates"].is_array() {
                doc["covariates"] = json!([]);
            }
            doc["covariates"].as_array_mut().unwrap().push(json!({"name":native_name,"unit":unit,"interpolation":"step","default":{"unit":unit,"value":start}}));
            notes.push(format!(
                "{name} starts at {start} {stored} and changes at the times given"
            ));
        }
        let unit = doc["covariates"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|x| x["name"] == native_name.as_str())
            .and_then(|x| x["unit"].as_str())
            .unwrap_or("1")
            .to_owned();
        if request["regimen"]["end"]["value"]
            .as_f64()
            .is_some_and(|end| t > end)
        {
            notes.push(format!(
                "{name}: the change at {t} {tu} is after the end of the run and has no effect"
            ));
            continue;
        }
        steps.push((t, json!({"name":native_name,"time":{"unit":tu,"value":t},"value":{"unit":unit,"value":value}})));
    }
    let regimen = request.get_mut("regimen").ok_or("request has no regimen")?;
    let mut all: Vec<(f64, Value)> = regimen["covariate_changes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| (r["time"]["value"].as_f64().unwrap_or(0.0), r.clone()))
        .collect();
    all.extend(steps);
    all.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    regimen["covariate_changes"] = Value::Array(all.into_iter().map(|(_, r)| r).collect());
    Ok((doc.to_string(), notes))
}

/// Native names of the parameters that enter the state equations, directly
/// or through definitions. Everything else is a value reported with the
/// model (published summaries, validation scalars) and does not change a run.
pub fn equation_parameters(model: &Value) -> BTreeSet<String> {
    let mut reached = BTreeSet::new();
    for s in model["states"].as_array().into_iter().flatten() {
        symbols(&s["rhs"], &mut reached);
        symbols(&s["initial"], &mut reached);
        symbols(&s["dosing"], &mut reached);
    }
    symbols(&model["linear_pk"], &mut reached);
    symbols(&model["covariates"], &mut reached);
    let definitions: BTreeMap<String, &Value> = model["definitions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| Some((d["name"].as_str()?.to_owned(), &d["expression"])))
        .collect();
    let mut frontier: Vec<String> = reached.iter().cloned().collect();
    while let Some(name) = frontier.pop() {
        if let Some(expr) = definitions.get(&name) {
            let mut inner = BTreeSet::new();
            symbols(expr, &mut inner);
            for n in inner {
                if reached.insert(n.clone()) {
                    frontier.push(n)
                }
            }
        }
    }
    let parameters: BTreeSet<&str> = model["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["name"].as_str())
        .collect();
    // A parameter that enters an output computed from the model's states or time (a response or effect
    // formula, say) changes that output just as surely, even though no state equation mentions it.
    for o in model["outputs"].as_array().into_iter().flatten() {
        let mut inner = BTreeSet::new();
        symbols(&o["expression"], &mut inner);
        let mut frontier: Vec<String> = inner.iter().cloned().collect();
        while let Some(name) = frontier.pop() {
            if let Some(expr) = definitions.get(&name) {
                let mut more = BTreeSet::new();
                symbols(expr, &mut more);
                for n in more {
                    if inner.insert(n.clone()) {
                        frontier.push(n)
                    }
                }
            }
        }
        let dynamic = inner.iter().any(|n| {
            !definitions.contains_key(n)
                && (!parameters.contains(n.as_str()) || reached.contains(n))
        });
        if dynamic {
            reached.extend(inner);
        }
    }
    parameters
        .into_iter()
        .filter(|n| reached.contains(*n))
        .map(str::to_owned)
        .collect()
}

/// Native names of the outputs that depend on the model: a state, time, or a parameter that enters the equations.
/// The rest only repeat numbers stored with the model (published summaries, validation scalars).
pub fn model_outputs(model: &Value) -> Vec<String> {
    let used = equation_parameters(model);
    let parameters: BTreeSet<&str> = model["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["name"].as_str())
        .collect();
    let definitions: BTreeMap<String, &Value> = model["definitions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| Some((d["name"].as_str()?.to_owned(), &d["expression"])))
        .collect();
    let mut out = Vec::new();
    for o in model["outputs"].as_array().into_iter().flatten() {
        let mut reached = BTreeSet::new();
        symbols(&o["expression"], &mut reached);
        let mut frontier: Vec<String> = reached.iter().cloned().collect();
        while let Some(name) = frontier.pop() {
            if let Some(expr) = definitions.get(&name) {
                let mut inner = BTreeSet::new();
                symbols(expr, &mut inner);
                for n in inner {
                    if reached.insert(n.clone()) {
                        frontier.push(n)
                    }
                }
            }
        }
        // Anything that is not a stored-only parameter (a state, time, an input, an equation parameter) makes it a model output.
        let depends = reached.iter().any(|n| {
            !definitions.contains_key(n) && (!parameters.contains(n.as_str()) || used.contains(n))
        });
        if depends {
            if let Some(n) = o["name"].as_str() {
                out.push(n.to_owned());
            }
        }
    }
    out
}

/// A short card for get_model: what to pass to simulate, in readable names
/// and real units, with the model's own parameters separated from values
/// that are only reported alongside it.
/// Exact names and native units, with separately declared source annotations.
/// Names do not imply a conversion or establish an analyte/compartment assignment.
pub fn output_descriptors(model: &Value, native: &Value) -> Vec<crate::contract::OutputDescriptor> {
    use crate::contract::{OutputAlias, OutputDescriptor};
    model["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| {
            let name = o["name"].as_str()?;
            let mut aliases = vec![OutputAlias {
                name: name.into(),
                declared_source_unit: None,
                analyte: None,
                compartment: None,
            }];
            for (alias, entry) in native["outputs"].as_object().into_iter().flatten() {
                if entry.as_str().or_else(|| entry["native_name"].as_str()) == Some(name) {
                    let record = OutputAlias {
                        name: alias.clone(),
                        declared_source_unit: entry["source_unit"]
                            .as_str()
                            .or_else(|| entry["unit"].as_str())
                            .map(str::to_owned),
                        analyte: entry["analyte"].as_str().map(str::to_owned),
                        compartment: entry["compartment"].as_str().map(str::to_owned),
                    };
                    if let Some(existing) = aliases.iter_mut().find(|a| a.name == *alias) {
                        *existing = record;
                    } else {
                        aliases.push(record);
                    }
                }
            }
            Some(OutputDescriptor {
                canonical_output: name.into(),
                returned_column: name.into(),
                returned_unit: o["unit"].as_str().unwrap_or("1").into(),
                request_aliases: aliases,
            })
        })
        .collect()
}

pub fn requested_output_names(args: &Value) -> Vec<String> {
    let mut names: Vec<String> = args["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    for spec in args["readouts"].as_array().into_iter().flatten() {
        let parsed = spec
            .as_str()
            .and_then(|text| shorthand(text).or_else(|| serde_json::from_str::<Value>(text).ok()));
        let spec = parsed.as_ref().unwrap_or(spec);
        if let Some(name) = spec["output"].as_str() {
            if !names.iter().any(|n| n == name) {
                names.push(name.into());
            }
        }
    }
    names
}

pub fn output_mapping(
    model: &Value,
    native: &Value,
    requested: &[String],
) -> Result<Vec<crate::contract::OutputMapping>, String> {
    let descriptors = output_descriptors(model, native);
    requested.iter().map(|name| {
        let matches: Vec<_> = descriptors.iter().filter_map(|d| d.request_aliases.iter().find(|a| a.name == *name).map(|a| (d, a))).collect();
        if matches.len() != 1 { return Err(format!("Output '{name}' has {} canonical matches; use one unambiguous output name from get_model.interface.output_contract", matches.len())); }
        let (d, alias) = matches[0];
        let declared = |analyte: bool| -> Result<Option<String>, String> {
            let values: std::collections::BTreeSet<_> = d.request_aliases.iter().filter_map(|a| if analyte {a.analyte.clone()} else {a.compartment.clone()}).collect();
            if values.len() > 1 { return Err(format!("Conflicting {} declarations for canonical output '{}'; correct the model's output metadata",if analyte {"analyte"} else {"compartment"},d.canonical_output)); }
            Ok(values.into_iter().next())
        };
        let analyte = declared(true)?;
        let compartment = declared(false)?;
        Ok(crate::contract::OutputMapping {
            requested_output: name.clone(), canonical_output: d.canonical_output.clone(), returned_column: d.returned_column.clone(), returned_unit: d.returned_unit.clone(),
            declared_source_unit: alias.declared_source_unit.clone(), analyte, compartment, returned_quantity: None,
        })
    }).collect()
}

pub fn interface(model: &Value, native: &Value, scenario_ids: &Value, examples: &Value) -> Value {
    let used = equation_parameters(model);
    let default = |native_name: &str| {
        model["parameters"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["name"] == native_name)
            .map(|p| p["default"]["value"].clone())
            .unwrap_or(Value::Null)
    };
    let (mut parameters, mut reported) = (Vec::new(), Vec::new());
    if let Some(table) = native["parameters"].as_object() {
        for (name, e) in table {
            let n = e["native_name"].as_str().unwrap_or("");
            if used.contains(n) {
                let mut row = json!({"name":name,"unit":e["source_unit"],"default":default(n)});
                // What the quantity is, where the archive says so and the name does not already.
                if let Some(d) = model["parameters"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|p| p["name"] == n)
                    .and_then(|p| p["description"].as_str())
                {
                    let d = d.trim();
                    if !d.is_empty() && !d.eq_ignore_ascii_case(name) {
                        row["description"] = json!(d.chars().take(100).collect::<String>());
                    }
                }
                parameters.push(row);
            } else {
                reported.push(json!(name));
            }
        }
    } else {
        for p in model["parameters"].as_array().into_iter().flatten() {
            parameters.push(json!({"name":p["name"],"unit":p["default"]["unit"],"default":p["default"]["value"]}));
        }
    }
    let alias_of = |native_name: &Value, table: &Value| -> (Value, Value) {
        table
            .as_object()
            .and_then(|m| m.iter().find(|(_, e)| e["native_name"] == *native_name))
            .map(|(k, e)| (json!(k), e["source_unit"].clone()))
            .unwrap_or((native_name.clone(), json!("1")))
    };
    let states = crate::library::display_states(model);
    let state_rows: Vec<Value> = states
        .iter()
        .map(|s| {
            let (name, unit) = alias_of(&s["name"], &native["states"]);
            json!({"name":name,"unit":if unit=="1"{s["unit"].clone()}else{unit}})
        })
        .collect();
    let dose_targets: Vec<Value> = states
        .iter()
        .filter(|s| !s["dosing"].is_null())
        .map(|s| {
            let (name, unit) = alias_of(&s["name"], &native["states"]);
            let (dose_in, scale) = dose_unit(model, native, s["name"].as_str().unwrap_or(""));
            let mut row = json!({"target":name,"amount_unit":dose_in.map(Value::from).unwrap_or(if unit=="1"{s["dosing"]["amount_unit"].clone()}else{json!("not declared; see dose_scale and the example")}),"modes":s["dosing"]["modes"]});
            row["canonical_target"] = s["name"].clone();
            row["state_unit"] = s["unit"].clone();
            row["archive_source_coordinate"] = native["dose_contracts"].as_object()
                .and_then(|entries| entries.values().find(|c| c["native_target"] == s["name"]))
                .cloned().unwrap_or(Value::Null);
            if let Some(scale) = scale {
                row["dose_scale"] = json!(format!("the model multiplies the dose by {scale} before adding it to {}", row["target"].as_str().unwrap_or("the state")));
            }
            if let Some(example) = examples.get(s["name"].as_str().unwrap_or("")) {
                row["example"] = example.clone();
            }
            row
        })
        .collect();
    let modelled = model_outputs(model);
    let label = |n: &Value| -> Value {
        native["outputs"]
            .as_object()
            .and_then(|m| m.iter().find(|(_, e)| e["native_name"] == *n || **e == *n))
            .map(|(k, _)| json!(k))
            .unwrap_or_else(|| n.clone())
    };
    let (mut outputs, mut reported_outputs) = (Vec::new(), 0usize);
    for o in model["outputs"].as_array().into_iter().flatten() {
        if o["name"]
            .as_str()
            .is_some_and(|n| modelled.iter().any(|m| m == n))
        {
            outputs.push(label(&o["name"]));
        } else {
            reported_outputs += 1;
        }
    }
    json!({
        "time_unit": model["time_unit"],
        "dose_targets": dose_targets,
        "parameters": parameters,
        "covariates": model["covariates"].as_array().into_iter().flatten().map(|c| json!({"name":c["name"],"unit":c["unit"],"default":c["default"]["value"]})).collect::<Vec<_>>(),
        "outputs": outputs,
        "output_contract": {"output_count": model["outputs"].as_array().map_or(0, Vec::len), "descriptors_complete": true, "descriptors": output_descriptors(model, native)},
        "reported_value_outputs": {"count": reported_outputs, "note": "Outputs that only repeat stored published numbers are not listed and are not returned unless named."},
        "states": state_rows,
        "scenario_ids": scenario_ids,
        "reported_values": {"count": reported.len(), "names": reported, "note": "Published or validation numbers stored with the model. They do not enter the equations; changing them does not change a simulation."},
        "how_to_simulate": "simulate with model {library:{model_id}} (or a scenario), doses [{target, amount:{value,unit}, at, every?, count?, duration?}], parameters {name:{value,unit}}, outputs [names], times and/or readouts. Use the names and units listed here. get_patient_parameters evaluates selected outputs at the retained initial state before administrations; pass covariates or scalar parameter overrides with explicit units. Initial values can include stored readouts; inspect their definitions before interpretation."
    })
}

/// Window readouts on the full solved trajectory.
pub fn readouts(result: &Value, specs: &Value, native: &Value) -> Result<Value, String> {
    super::readouts::calculate(result, specs, native)
}

/// Values at the times the caller asked for, by readable output name.
pub fn requested_values(result: &Value, requested: &Value, native: &Value) -> Value {
    let t: Vec<f64> = result["times"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| v.as_f64().unwrap_or(f64::NAN))
        .collect();
    let alias = |n: &Value| -> Value {
        native["outputs"]
            .as_object()
            .and_then(|m| m.iter().find(|(_, e)| e["native_name"] == *n || **e == *n))
            .map(|(k, _)| json!(k))
            .unwrap_or_else(|| n.clone())
    };
    let mut rows = Vec::new();
    for q in requested.as_array().into_iter().flatten() {
        let at = q["value"].as_f64().unwrap_or(f64::NAN);
        let hit: Vec<usize> = (0..t.len()).filter(|i| t[*i] == at).collect();
        let (Some(first), Some(last)) = (hit.first(), hit.last()) else {
            continue;
        };
        let mut row = Map::new();
        row.insert("time".into(), json!(at));
        for o in result["outputs"].as_array().into_iter().flatten() {
            let v = &o["values"];
            let key = alias(&o["name"]).as_str().unwrap_or("output").to_owned();
            if v[*first] == v[*last] {
                row.insert(key, v[*last].clone());
            } else {
                row.insert(key, json!({"pre_dose":v[*first],"post_dose":v[*last]}));
            }
        }
        rows.push(Value::Object(row));
    }
    json!(rows)
}

pub(crate) fn readable(native_name: &str, native: &Value) -> String {
    for table in ["parameters", "states", "outputs"] {
        if let Some((k, _)) = native[table]
            .as_object()
            .and_then(|m| m.iter().find(|(_, e)| e["native_name"] == native_name))
        {
            return k.clone();
        }
    }
    native_name.to_owned()
}

/// One expression as ordinary infix text, with readable names.
pub fn render(expr: &Value, native: &Value) -> String {
    let arg = |i: usize| render(&expr["arguments"][i], native);
    match expr["kind"].as_str().unwrap_or("") {
        "literal" => expr["value"].to_string(),
        "quantity" => expr["quantity"]["value"].to_string(),
        "symbol" => readable(expr["name"].as_str().unwrap_or("?"), native),
        "unary" => format!("-({})", arg(0)),
        "binary" | "compare" => {
            let op = match expr["operator"].as_str().unwrap_or("?") {
                "add" => "+",
                "subtract" => "-",
                "multiply" => "*",
                "divide" => "/",
                "power" => "^",
                "gt" => ">",
                "ge" => ">=",
                "lt" => "<",
                "le" => "<=",
                "eq" => "==",
                "ne" => "!=",
                other => other,
            };
            format!("({} {} {})", arg(0), op, arg(1))
        }
        "call" => format!(
            "{}({})",
            expr["name"].as_str().unwrap_or("f"),
            expr["arguments"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|a| render(a, native))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => expr.to_string(),
    }
}

/// The model's equations as text with readable names: state derivatives, definitions and outputs. Null when the
/// listing would be too long to be useful inline.
pub fn equations(model: &Value, native: &Value) -> Value {
    let mut lines = Vec::new();
    for s in model["states"].as_array().into_iter().flatten() {
        let name = readable(s["name"].as_str().unwrap_or("?"), native);
        lines.push(format!("d({name})/dt = {}", render(&s["rhs"], native)));
        lines.push(format!("{name}(0) = {}", render(&s["initial"], native)));
    }
    for d in model["definitions"].as_array().into_iter().flatten() {
        lines.push(format!(
            "{} = {}",
            readable(d["name"].as_str().unwrap_or("?"), native),
            render(&d["expression"], native)
        ));
    }
    for o in model["outputs"].as_array().into_iter().flatten() {
        lines.push(format!(
            "output {} = {}",
            readable(o["name"].as_str().unwrap_or("?"), native),
            render(&o["expression"], native)
        ));
    }
    if lines.iter().map(String::len).sum::<usize>() > 16_000 {
        return Value::Null;
    }
    json!(lines)
}

/// A stored scenario request with readable names and the model's units.
pub fn readable_request(request: &Value, native: &Value, model: &Value) -> Value {
    let used = equation_parameters(model);
    let named = |values: &Value, table: &str, only_used: bool| -> Value {
        let mut out = Map::new();
        for (name, q) in values.as_object().into_iter().flatten() {
            if only_used && !used.contains(name) {
                continue;
            }
            let unit = native[table]
                .as_object()
                .and_then(|m| m.values().find(|e| e["native_name"] == name.as_str()))
                .map(|e| e["source_unit"].clone())
                .unwrap_or(q["unit"].clone());
            out.insert(
                readable(name, native),
                json!({"value":q["value"],"unit":unit}),
            );
        }
        Value::Object(out)
    };
    let doses: Vec<Value> = request["regimen"]["administrations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            let target = a["target"].as_str().unwrap_or("?");
            let unit = native["states"]
                .as_object()
                .and_then(|m| m.values().find(|e| e["native_name"] == target))
                .map(|e| e["source_unit"].clone())
                .unwrap_or(a["amount"]["unit"].clone());
            let mut d = json!({"target":readable(target, native),"amount":{"value":a["amount"]["value"],"unit":unit},"at":a["time"]});
            if !a["repeat"].is_null() {
                d["every"] = a["repeat"]["interval"].clone();
                d["count"] = json!(a["repeat"]["additional"].as_u64().unwrap_or(0) + 1);
            }
            if a["delivery"]["kind"] == "infusion" {
                d["duration"] = a["delivery"]["span"]["duration"].clone();
            }
            d
        })
        .collect();
    json!({
        "parameters": named(&request["parameters"], "parameters", true),
        "initial_states": named(&request["initial_states"], "states", false),
        "doses": doses,
        "end": request["regimen"]["end"],
        "sample_count": request["regimen"]["samples"].as_array().map(Vec::len),
        "solver": request["solver"],
    })
}

/// The values of the parameters that enter the equations for one run, and whether each was given or left at
/// the model's default.
pub fn parameters_used(model: &Value, native: &Value, request: &Value, given: &Value) -> Value {
    let used = equation_parameters(model);
    if used.len() > 60 {
        return json!({"count":used.len(),"note":"Too many to list; every parameter not overridden uses the model's default."});
    }
    let rows: Vec<Value> = model["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["name"].as_str().is_some_and(|n| used.contains(n)))
        .map(|p| {
            let n = p["name"].as_str().unwrap();
            let set = !request["parameters"][n].is_null();
            let unit = native["parameters"]
                .as_object()
                .and_then(|m| m.values().find(|e| e["native_name"] == n))
                .map(|e| e["source_unit"].clone())
                .unwrap_or(p["default"]["unit"].clone());
            json!({"name":readable(n, native),"value":if set{request["parameters"][n]["value"].clone()}else{p["default"]["value"].clone()},"unit":unit,
                   "source":if !given[n].is_null(){"given"}else if set{"scenario"}else{"model default"}})
        })
        .collect();
    json!(rows)
}

/// The full trajectory as CSV: time, side and one column per output, by readable name.
pub fn csv(result: &Value, native: &Value) -> Result<crate::table::Trajectory, String> {
    crate::table::trajectory(result, native)
}

/// A relative CSV path beneath the configured output directory.
pub fn safe_csv_path(path: &str) -> Result<std::path::PathBuf, String> {
    crate::export::csv_path(path)
}

/// Remove stored events beyond the selected end, comparing physical times.
pub fn trim_events_after_end(request: &mut Value) -> Result<Vec<String>, String> {
    let end = &request["regimen"]["end"];
    let unit = end["unit"]
        .as_str()
        .ok_or("regimen.end.unit is required")?
        .to_owned();
    let end = to_model_time(end, &unit, "regimen.end")?["value"]
        .as_f64()
        .unwrap();
    let mut notes = Vec::new();
    for key in ["checkpoints", "resets", "covariate_changes"] {
        if let Some(rows) = request
            .get_mut("regimen")
            .and_then(|r| r.get_mut(key))
            .and_then(Value::as_array_mut)
        {
            let mut kept = Vec::with_capacity(rows.len());
            let before = rows.len();
            for row in rows.iter() {
                let q = if key == "checkpoints" {
                    row
                } else {
                    &row["time"]
                };
                let time = to_model_time(q, &unit, key)?["value"].as_f64().unwrap();
                if time <= end {
                    kept.push(row.clone());
                }
            }
            if kept.len() < before {
                notes.push(format!(
                    "{} stored {key} after the end time were dropped",
                    before - kept.len()
                ));
            }
            *rows = kept;
        }
    }
    Ok(notes)
}

/// Serialize fields in the given order (the JSON map type sorts keys).
pub fn ordered(pairs: &[(&str, &Value)]) -> String {
    let mut s = String::from("{");
    for (k, v) in pairs.iter().filter(|(_, v)| !v.is_null()) {
        if s.len() > 1 {
            s.push(',')
        }
        s.push_str(&serde_json::to_string(k).unwrap());
        s.push(':');
        s.push_str(&v.to_string());
    }
    s.push('}');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_overflow_refuses_before_normalization_indexes_quantities() {
        let mut args = json!({"request":{"regimen":{"start":{"value":1e308,"unit":"wk"}}}});
        let error =
            normalize(&mut args, &Value::Null, &json!({"time_unit":"h"}), None).unwrap_err();
        assert!(
            error.contains("regimen.start") && error.contains("must be finite"),
            "{error}"
        );
        let mut args = json!({"end":{"value":1e308,"unit":"h"},"readouts":[{"kind":"auc","output":"c","from":0,"to":1e308}]});
        let error = normalize(
            &mut args,
            &Value::Null,
            &json!({"time_unit":"h","outputs":[{"name":"c","unit":"mg/L"}]}),
            None,
        )
        .unwrap_err();
        assert!(
            error.contains("readouts[0].from/to") && error.contains("finite sampling grid"),
            "{error}"
        );
    }
    #[test]
    fn state_readout_aliases_bind_native_inputs_without_assigning_source_dimensions() {
        let model = json!({"time_unit":"h","parameters":[{"name":"k","default":{"value":2.0,"unit":"1/h"}}],
            "states":[{"name":"s0","unit":"1","initial":{"kind":"quantity","quantity":{"value":7.0,"unit":"1"}},"dosing":null}],
            "outputs":[{"name":"library_state_0","unit":"1","expression":{"kind":"symbol","name":"s0"}},
                       {"name":"derived","unit":"1","expression":{"kind":"literal","value":4.0}},
                       {"name":"s0","unit":"1","expression":{"kind":"literal","value":4.0}}]});
        let archive = json!({"states":{"biological_state":{"native_name":"library_state_0","unit":"nM"},"derived_readout":{"native_name":"derived","unit":"nM"},"ambiguous_readout":{"native_name":"s0","unit":"1"}}});
        let named = augment(&archive, &model);
        validate_input_aliases(&model, &named).unwrap();
        assert_eq!(named["states"]["biological_state"]["native_name"], "s0");
        assert_eq!(named["states"]["biological_state"]["source_unit"], "1");
        assert_eq!(
            named["states"]["biological_state"]["declared_source_unit"],
            "nM"
        );
        assert!(named["states"].get("derived_readout").is_none());
        assert!(named["states"].get("ambiguous_readout").is_none());
        assert_eq!(named["parameters"]["k"]["request_unit"], "1/h");
        let mut canonical = json!({"parameters":{"k":{"value":3.0,"unit":"1/h"}},"initial_states":{"s0":{"value":11.0,"unit":"1"}}});
        let mut friendly = json!({"parameters":{"k":{"value":3.0,"unit":"1/h"}},"initial_states":{"biological_state":{"value":11.0,"unit":"1"}}});
        normalize(&mut canonical, &named, &model, None).unwrap();
        normalize(&mut friendly, &named, &model, None).unwrap();
        assert_eq!(canonical, friendly);
        friendly["initial_states"] = json!({"biological_state":{"value":11.0,"unit":"nM"}});
        assert!(normalize(&mut friendly, &named, &model, None).is_err());
        assert_eq!(
            archive["states"]["biological_state"]["native_name"],
            "library_state_0"
        );
    }
    #[test]
    fn invalid_input_targets_units_and_canonical_shadowing_are_refused() {
        let model = json!({"parameters":[{"name":"cl","default":{"unit":"L/h","value":1.0}},{"name":"v","default":{"unit":"L","value":3.0}}],"states":[]});
        for entry in [
            json!({"native_name":"missing","request_unit":"L/h"}),
            json!({"native_name":"v","request_unit":"L"}),
            json!({"native_name":"cl","request_unit":"mg"}),
        ] {
            let named = augment(&json!({"parameters":{"cl":entry}}), &model);
            assert!(validate_input_aliases(&model, &named).is_err());
        }
    }
    #[test]
    fn native_grids_are_preserved_and_conflicts_are_refused() {
        let model = json!({"time_unit":"h","outputs":[{"name":"cp","unit":"mg/L"}]});
        for regimen in [
            json!({"end":{"value":2,"unit":"h"},"samples":[],"observations":[]}),
            json!({"end":{"value":2,"unit":"h"},"samples":[],"observations":[{"time":{"value":1,"unit":"h"},"side":"pre"},{"time":{"value":1,"unit":"h"},"side":"post"}]}),
        ] {
            let original = json!({"request":{"schema":"qspquest.run/v0.1","regimen":regimen}});
            let mut args = original.clone();
            assert!(normalize(&mut args, &Value::Null, &model, None)
                .unwrap()
                .is_empty());
            assert_eq!(args, original);
        }
        let mut both = json!({"request":{"regimen":{"samples":[{"value":0,"unit":"h"}],"observations":[{"time":{"value":0,"unit":"h"},"side":"pre"}]}}});
        assert!(normalize(&mut both, &Value::Null, &model, None)
            .unwrap_err()
            .contains("request.regimen.samples conflicts"));
        let mut readout = json!({"request":{"regimen":{"end":{"value":2,"unit":"h"},"observations":[{"time":{"value":1,"unit":"h"},"side":"pre"}]}},"readouts":[{"kind":"auc","output":"cp"}]});
        assert!(normalize(&mut readout, &Value::Null, &model, None)
            .unwrap_err()
            .contains("would replace"));
    }
    #[test]
    fn conflicting_alias_inputs_and_unknown_units_are_refused() {
        let model = json!({"time_unit":"h"});
        let native = json!({"parameters":{"clearance":{"native_name":"cl","source_unit":"L/h","request_unit":"L/h"}},"states":{"central_mass":{"native_name":"central","source_unit":"mg","request_unit":"mg"}}});
        for (key, alias, name, unit) in [
            ("parameters", "clearance", "cl", "L/h"),
            ("initial_states", "central_mass", "central", "mg"),
        ] {
            let mut args =
                json!({key:{alias:{"value":1,"unit":unit},name:{"value":2,"unit":unit}}});
            let error = normalize(&mut args, &native, &model, None).unwrap_err();
            assert!(
                error.contains(&format!("{key}.{alias}"))
                    && error.contains(&format!("{key}.{name}")),
                "{error}"
            );
        }
        assert!(convert(3.0, "mg", "cells/mm3", "parameters.cell_count")
            .unwrap_err()
            .contains("parameters.cell_count"));
        assert_eq!(convert(3.0, "cells/mm3", "cells/mm3", "x").unwrap(), 3.0);
        assert_eq!(convert(25.0, "%", "fraction", "x").unwrap(), 0.25);
        assert_eq!(convert(25.0, "1", "%", "x").unwrap(), 25.0);
    }
    #[test]
    fn thresholds_convert_only_against_the_actual_output_unit() {
        let model = json!({"time_unit":"h","outputs":[{"name":"cp","unit":"mg/L"}]});
        let native = json!({"outputs":{"concentration":{"native_name":"cp","unit":"mg/L"}}});
        let original = json!({"end":{"value":2,"unit":"h"},"readouts":[{"kind":"time_above","output":"concentration","threshold":{"value":5000,"unit":"ug/L"}}]});
        let mut args = original.clone();
        let notes = normalize(&mut args, &native, &model, None).unwrap();
        assert_eq!(args["readouts"][0]["threshold"], 5.0);
        assert!(notes.iter().any(|n| n.contains("5000 ug/L = 5 mg/L")));
        let mut wrong = original.clone();
        wrong["readouts"][0]["threshold"]["unit"] = json!("kg");
        assert!(normalize(&mut wrong, &native, &model, None)
            .unwrap_err()
            .contains("readouts[0].threshold"));
        let source_scale = json!({"time_unit":"h","outputs":[{"name":"cp","unit":"1"}]});
        let mut wrong = original;
        assert!(normalize(&mut wrong, &native, &source_scale, None)
            .unwrap_err()
            .contains("model's unit '1'"));
    }
    #[test]
    fn invalid_readout_semantics_fail_before_a_trajectory_is_needed() {
        let model = json!({"time_unit":"h","outputs":[{"name":"cp","unit":"mg/L"}]});
        for (spec, field) in [
            (json!({"kind":"unknown","output":"cp"}), "kind"),
            (json!({"kind":"auc","output":"missing"}), "output"),
            (json!({"kind":"time_above","output":"cp"}), "threshold"),
            (json!({"kind":"auc","output":"cp","at":1}), "at"),
            (json!({"kind":"value","output":"cp","from":0}), "from/to"),
            (
                json!({"kind":"value","output":"cp","at":1,"side":"left"}),
                "side",
            ),
            (
                json!({"kind":"first_crossing","output":"cp","threshold":1,"direction":"sideways"}),
                "direction",
            ),
            (
                json!({"kind":"auc","output":"cp","from":0,"start":1}),
                "from",
            ),
            (
                json!({"kind":"auc","output":"cp","from":0,"window":{"from":1}}),
                "from",
            ),
            (
                json!({"kind":"auc","output":"cp","unrecognized":1}),
                "unrecognized",
            ),
        ] {
            let mut args = json!({"end":{"value":2,"unit":"h"},"readouts":[spec]});
            let error = normalize(&mut args, &Value::Null, &model, None).unwrap_err();
            assert!(error.contains(&format!("readouts[0].{field}")), "{error}");
        }
        let mut zero = json!({"end":{"value":0,"unit":"h"},"times":[{"value":0,"unit":"h"}]});
        let original = zero.clone();
        assert!(normalize(&mut zero, &Value::Null, &model, None)
            .unwrap_err()
            .contains("end.value must be positive"));
        assert_eq!(
            zero["end"]["value"].as_f64(),
            original["end"]["value"].as_f64()
        );
    }
    #[test]
    fn undeclared_physical_dose_dimensions_are_not_inferred() {
        let native = json!({"states":{"central":{"native_name":"s0","request_unit":"1","source_unit":"mg"}}});
        let model = json!({"time_unit":"h","states":[{"name":"s0","dosing":{"scale":{"kind":"literal","value":7.0}}}]});
        let mut args = json!({"doses":[{"target":"central","amount":{"value":5,"unit":"mg"},"at":{"value":0,"unit":"h"}}]});
        assert!(normalize(&mut args, &native, &model, None)
            .unwrap_err()
            .contains("dose.amount.unit"));
        args["doses"][0]["amount"]["unit"] = json!("1");
        normalize(&mut args, &native, &model, None).unwrap();
        assert_eq!(args["doses"][0]["amount"], json!({"value":5.0,"unit":"1"}));
    }
    #[test]
    fn readout_expansion_is_bounded_and_reuses_identical_windows() {
        let model = json!({"time_unit":"h","outputs":[{"name":"cp","unit":"mg/L"}]});
        let mut repeated = json!({"end":{"value":100,"unit":"h"},"readouts":vec![json!({"kind":"auc","output":"cp","from":0,"to":10});100]});
        normalize(&mut repeated, &Value::Null, &model, None).unwrap();
        assert_eq!(
            repeated["times"].as_array().unwrap().len(),
            READOUT_POINTS + 1
        );
        let mut large = json!({"end":{"value":100,"unit":"h"},"readouts":(0..30).map(|i|json!({"kind":"auc","output":"cp","from":i,"to":i+1})).collect::<Vec<_>>()});
        assert!(normalize(&mut large, &Value::Null, &model, None)
            .unwrap_err()
            .contains("bounded input grid allowance"));
    }
    #[test]
    fn explicit_native_window_and_event_trimming_preserve_time_units() {
        let mut args = json!({"request":{"schema":"qspquest.run/v0.1","regimen":{
            "end":{"value":1,"unit":"d"}, "samples":[{"value":24,"unit":"h"}],
            "resets":[{"time":{"value":12,"unit":"h"}}, {"time":{"value":36,"unit":"h"}}],
            "checkpoints":[{"value":12,"unit":"h"}, {"value":36,"unit":"h"}],
            "covariate_changes":[{"time":{"value":12,"unit":"h"}}, {"time":{"value":36,"unit":"h"}}]
        }}});
        normalize(&mut args, &Value::Null, &json!({"time_unit":"h"}), None).unwrap();
        assert_eq!(
            args["request"]["regimen"]["end"],
            json!({"value":1,"unit":"d"})
        );
        assert_eq!(
            trim_events_after_end(&mut args["request"]).unwrap().len(),
            3
        );
        for key in ["resets", "checkpoints", "covariate_changes"] {
            assert_eq!(args["request"]["regimen"][key].as_array().unwrap().len(), 1);
        }
        args["request"]["regimen"]["samples"] = json!([{"value":48,"unit":"h"}]);
        normalize(&mut args, &Value::Null, &json!({"time_unit":"h"}), None).unwrap();
        assert_eq!(
            args["request"]["regimen"]["end"],
            json!({"value":1,"unit":"d"})
        );
    }
    #[test]
    fn requested_rows_and_readouts_keep_distinct_event_times_separate() {
        let next = 1.0000000001;
        let result = json!({"time_unit":"h", "times":[0.0,1.0,next,next,2.0],
            "sides":["post","post","pre","post","post"],
            "outputs":[{"name":"cp","unit":"mg/L","values":[0.0,0.0,0.0,16.0,8.0]}]});
        let requested = requested_values(&result, &json!([{"value":1.0,"unit":"h"}]), &Value::Null);
        assert_eq!(requested[0]["cp"], 0.0);
        let readout = readouts(
            &result,
            &json!([{"kind":"value","output":"cp","at":{"value":1.0,"unit":"h"}}]),
            &Value::Null,
        )
        .unwrap();
        assert_eq!(readout["results"][0]["value"], 0.0);
        assert_eq!(
            super::super::readouts::window(&[0.0, 1.0, next, next, 2.0], 0.0, 1.0),
            Some((0, 1))
        );
    }
    #[test]
    fn units_convert_exactly_and_reject_mismatches() {
        assert_eq!(convert(2.0, "g", "mg", "x").unwrap(), 2000.0);
        assert!((convert(24.0, "L/day", "L/h", "x").unwrap() - 1.0).abs() < 1e-12);
        assert!((convert(1.0, "ug/mL", "mg/L", "x").unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(convert(5.0, "1", "L/h", "x").unwrap(), 5.0);
        assert_eq!(convert(0.3, "fraction", "ratio", "x").unwrap(), 0.3);
        assert!(convert(1.0, "mg", "L/h", "x").unwrap_err().contains("L/h"));
    }
    #[test]
    fn friendly_units_share_engine_count_dimensions_and_grouped_rate_scales() {
        assert_eq!(
            convert(199.57, "[cell]/uL", "[cell]/mm3", "cells").unwrap(),
            199.57
        );
        assert!(
            (convert(199_570_000., "[cell]/L", "[cell]/mm3", "cells").unwrap() - 199.57).abs()
                < 1e-12
        );
        assert!(
            (convert(24., "mm3/([cell].d)", "uL/([cell].h)", "loss").unwrap() - 1.).abs() < 1e-12
        );
        assert!((convert(1., "nM", "nmol/L", "concentration").unwrap() - 1.).abs() < 1e-12);
        for incompatible in ["[genome]/mm3", "mg/L", "mol/L"] {
            let error = convert(1., incompatible, "[cell]/mm3", "cells").unwrap_err();
            assert!(!error.contains("molar mass"));
        }
        assert!(convert(f64::MAX, "kg", "mg", "mass").is_err());
    }
    #[test]
    fn names_units_and_default_end_are_resolved() {
        let native = json!({"parameters":{"body_weight_kg":{"native_name":"p7","request_unit":"1","source_unit":"kg"}},
            "states":{"central_mg":{"native_name":"s1","request_unit":"1","source_unit":"mg"}}});
        let model = json!({"time_unit":"d"});
        let mut args = json!({"parameters":{"body_weight_kg":{"value":48000.0,"unit":"g"}},
            "doses":[{"target":"central_mg","amount":{"value":0.384,"unit":"g"},"at":{"value":0.0,"unit":"h"},"every":{"value":2.0,"unit":"week"},"count":3}],
            "times":[{"value":336.0,"unit":"h"}]});
        normalize(&mut args, &native, &model, None).unwrap();
        assert_eq!(args["parameters"]["p7"], json!({"value":48.0,"unit":"1"}));
        assert_eq!(args["doses"][0]["target"], "s1");
        assert_eq!(args["doses"][0]["amount"]["value"], 384.0);
        assert_eq!(args["doses"][0]["every"], json!({"value":14.0,"unit":"d"}));
        assert_eq!(args["times"][0]["value"], 14.0);
        assert_eq!(args["end"]["value"], 28.0);
        let mut bad = json!({"parameters":{"weight":{"value":1.0,"unit":"kg"}}});
        assert!(normalize(&mut bad, &native, &model, None)
            .unwrap_err()
            .contains("body_weight_kg"));
    }
    #[test]
    fn readouts_respect_dose_sides() {
        // y = 10 until a dose at t = 1 raises it to 20; then falls linearly to 0 at t = 3.
        let result = json!({"time_unit":"d","times":[0.0,1.0,1.0,2.0,3.0],"sides":["post","pre","post","post","post"],
            "outputs":[{"name":"c","unit":"1","values":[10.0,10.0,20.0,10.0,0.0]}]});
        let specs = json!([
            {"kind":"auc","output":"c","from":{"value":0.0},"to":{"value":3.0}},
            {"kind":"auc","output":"c","from":{"value":1.0},"to":{"value":3.0}},
            {"kind":"trough","output":"c","at":{"value":1.0}},
            {"kind":"time_below","output":"c","threshold":5.0,"from":{"value":0.0},"to":{"value":3.0}},
            {"kind":"first_crossing","output":"c","threshold":15.0,"from":{"value":0.0},"to":{"value":3.0}},
            {"kind":"max","output":"c","from":{"value":0.0},"to":{"value":3.0}}]);
        let r = readouts(&result, &specs, &Value::Null).unwrap();
        let v = |i: usize| r["results"][i]["value"].clone();
        assert_eq!(v(0), 30.0);
        assert_eq!(v(1), 20.0);
        assert_eq!(v(2), 10.0);
        assert_eq!(v(3), 0.5);
        assert_eq!(v(4)["time"], 1.0);
        assert_eq!(v(4)["at_dose_event"], true);
        assert_eq!(v(5), 20.0);
    }
    #[test]
    fn nca_and_text_helpers() {
        // exp(-t) sampled finely on [0, 4]
        let times: Vec<f64> = (0..=400).map(|i| i as f64 / 100.0).collect();
        let values: Vec<f64> = times.iter().map(|t| (-t).exp()).collect();
        let result = json!({"time_unit":"d","times":times,"sides":vec!["post"; 401],"outputs":[{"name":"c","unit":"1","values":values}]});
        let r = readouts(
            &result,
            &json!([{"kind":"nca","output":"c","from":{"value":0.0},"to":{"value":4.0}}]),
            &Value::Null,
        )
        .unwrap();
        let v = &r["results"][0]["value"];
        assert!((v["terminal_half_life"].as_f64().unwrap() - std::f64::consts::LN_2).abs() < 1e-6);
        assert!((v["auc"].as_f64().unwrap() - (1.0 - (-4.0f64).exp())).abs() < 1e-4);
        assert_eq!(v["cmax"], 1.0);
        let native = json!({"parameters":{"cl":{"native_name":"p1","source_unit":"L/h"}},"states":{"central":{"native_name":"s1","source_unit":"mg"}}});
        let expr = json!({"kind":"binary","operator":"multiply","arguments":[{"kind":"unary","operator":"negative","arguments":[{"kind":"symbol","name":"p1"}]},{"kind":"symbol","name":"s1"}]});
        assert_eq!(render(&expr, &native), "(-(cl) * central)");
        let table = csv(
            &json!({"time_unit":"d","times":[0.0,1.0],"sides":["post","post"],"outputs":[{"name":"s1","values":[1.0,2.0]}]}),
            &native,
        ).unwrap();
        assert_eq!(table.rows, 2);
        assert_eq!(table.columns[2].name, "central");
        assert_eq!(
            table.csv,
            "time_d,side,central\n0.0,post,1.0\n1.0,post,2.0\n"
        );
        assert!(safe_csv_path("out/run.csv").is_ok());
        assert!(safe_csv_path("../run.csv").is_err());
        assert!(safe_csv_path("/tmp/run.csv").is_err());
    }
    #[test]
    fn readouts_are_read_in_the_shapes_callers_send() {
        let model = json!({"time_unit":"d","outputs":[{"name":"conc","unit":"mg/L"}]});
        let mut args = json!({"end":{"value":100.0,"unit":"d"},"readouts":[
            "auc(conc, 42, 84)",
            "{\"kind\":\"value\",\"output\":\"conc\",\"time\":{\"at\":{\"value\":10.0,\"unit\":\"d\"}}}",
            {"kind":"time_below","output":"conc","threshold":{"value":5.0,"unit":"mg/L"},"window":{"from":{"value":1.0,"unit":"wk"},"to":14.0}},
            {"kind":"nca","output":"conc"}]});
        normalize(&mut args, &Value::Null, &model, None).unwrap();
        let r = &args["readouts"];
        assert_eq!(
            (r[0]["from"]["value"].clone(), r[0]["to"]["value"].clone()),
            (json!(42.0), json!(84.0))
        );
        assert_eq!(r[1]["at"]["value"], 10.0);
        assert_eq!(
            (
                r[2]["from"]["value"].clone(),
                r[2]["to"]["value"].clone(),
                r[2]["threshold"].clone()
            ),
            (json!(7.0), json!(14.0), json!(5.0))
        );
        assert_eq!(
            (r[3]["from"]["value"].clone(), r[3]["to"]["value"].clone()),
            (json!(0.0), json!(100.0))
        );
        let mut bad = json!({"readouts":["the area please"]});
        assert!(normalize(&mut bad, &Value::Null, &model, None)
            .unwrap_err()
            .contains("must be an object"));
        assert!(convert(17.0, "score", "see_source", "x")
            .unwrap_err()
            .contains("not supported"));
        assert_eq!(
            convert(17.0, "see_source", "see_source", "x").unwrap(),
            17.0
        );
    }
    #[test]
    fn dose_unit_follows_the_models_own_scale() {
        let native = json!({"states":{"depot_nmol":{"native_name":"s0","request_unit":"1","source_unit":"nmol"},"central_mg":{"native_name":"s1","request_unit":"1","source_unit":"mg"}},
            "parameters":{"dose_mg_to_nmol":{"native_name":"p0","source_unit":"nmol/mg"},"F_sc":{"native_name":"p1","source_unit":"fraction"}}});
        let scale = |sym: Value| json!({"kind":"binary","operator":"multiply","arguments":[sym,{"kind":"symbol","name":"legacy_unit_basis_1"}]});
        let model = json!({"time_unit":"d","states":[
            {"name":"s0","dosing":{"scale":scale(json!({"kind":"symbol","name":"p0"}))}},
            {"name":"s1","dosing":{"scale":scale(json!({"kind":"literal","value":1.0}))}}]});
        assert_eq!(
            dose_unit(&model, &native, "s0"),
            (Some("mg".into()), Some("dose_mg_to_nmol".into()))
        );
        assert_eq!(dose_unit(&model, &native, "s1"), (Some("mg".into()), None));
        let mut args = json!({"doses":[{"target":"depot_nmol","amount":{"value":0.15,"unit":"g"},"at":{"value":0.0,"unit":"d"}}]});
        normalize(&mut args, &native, &model, None).unwrap();
        assert_eq!(
            args["doses"][0]["amount"],
            json!({"value":150.0,"unit":"1"})
        );
        let mut wrong = json!({"doses":[{"target":"depot_nmol","amount":{"value":1000.0,"unit":"nmol"},"at":{"value":0.0,"unit":"d"}}]});
        assert!(normalize(&mut wrong, &native, &model, None)
            .unwrap_err()
            .contains("multiplies the dose by dose_mg_to_nmol"));
    }
    #[test]
    fn dynamic_outputs_follow_chained_definitions_without_promoting_stored_summaries() {
        let symbol = |name: &str| json!({"kind":"symbol","name":name});
        let model = json!({
            "parameters":[{"name":"Emax"},{"name":"EC50"},{"name":"reported_trough"}],
            "states":[{"name":"x","rhs":{"kind":"literal","value":0}}],
            "definitions":[
                {"name":"denominator","expression":{"kind":"binary","arguments":[symbol("EC50"),symbol("x")]}},
                {"name":"effect","expression":{"kind":"binary","arguments":[symbol("Emax"),symbol("x"),symbol("denominator")]}},
                {"name":"stored_summary","expression":symbol("reported_trough")}
            ],
            "outputs":[{"name":"response","expression":symbol("effect")},
                {"name":"published_trough","expression":symbol("stored_summary")}]
        });
        assert_eq!(
            equation_parameters(&model),
            BTreeSet::from(["Emax".to_owned(), "EC50".to_owned()])
        );
        assert_eq!(model_outputs(&model), vec!["response".to_owned()]);
    }
    #[test]
    fn only_parameters_in_the_equations_are_model_parameters() {
        let model = json!({"parameters":[{"name":"cl"},{"name":"v"},{"name":"reported_trough"}],
            "definitions":[{"name":"k","expression":{"kind":"binary","arguments":[{"kind":"symbol","name":"cl"},{"kind":"symbol","name":"v"}]}}],
            "states":[{"name":"a","rhs":{"kind":"symbol","name":"k"}}]});
        assert_eq!(
            equation_parameters(&model),
            BTreeSet::from(["cl".to_owned(), "v".to_owned()])
        );
    }
    #[test]
    fn declared_mass_dose_is_converted_before_a_concentration_scale() {
        let native = json!({"states":{"central_nmol_l":{"native_name":"s0","source_unit":"nmol/L","request_unit":"1","dose_amount_unit":"mg"}},
            "parameters":{"route_fraction":{"native_name":"route","source_unit":"fraction"}}});
        let model = json!({"time_unit":"h","states":[{"name":"s0","unit":"1","dosing":{"amount_unit":"1","modes":["bolus"],
            "scale":{"kind":"binary","operator":"multiply","arguments":[{"kind":"symbol","name":"route"},{"kind":"symbol","name":"derived_conversion"}]}}}]});
        assert_eq!(dose_unit(&model, &native, "s0").0, Some("mg".into()));
        let card = interface(&model, &native, &json!([]), &json!({}));
        assert_eq!(card["dose_targets"][0]["amount_unit"], "mg");
        for (value, unit) in [(0.3, "g"), (300.0, "mg"), (300000.0, "ug"), (300.0, "1")] {
            let mut args = json!({"doses":[{"target":"central_nmol_l","amount":{"value":value,"unit":unit},"at":{"value":0.0,"unit":"h"}}]});
            normalize(&mut args, &native, &model, None).unwrap();
            assert_eq!(
                args["doses"][0]["amount"],
                json!({"value":300.0,"unit":"1"})
            );
        }
        for unit in ["nmol", "mg/L"] {
            let mut args = json!({"doses":[{"target":"central_nmol_l","amount":{"value":300.0,"unit":unit},"at":{"value":0.0,"unit":"h"}}]});
            assert!(normalize(&mut args, &native, &model, None)
                .unwrap_err()
                .contains("does not match the model's unit 'mg'"));
        }
    }
    #[test]
    fn declared_weight_normalized_dose_preserves_its_input_coordinates() {
        let native = json!({"states":{"central":{"native_name":"s0","source_unit":"mg","request_unit":"1","dose_amount_unit":"mg/kg"}},
            "parameters":{"weight":{"native_name":"bw","source_unit":"kg"}}});
        let model = json!({"time_unit":"d","states":[{"name":"s0","dosing":{"scale":{"kind":"symbol","name":"bw"}}}]});
        let mut args = json!({"doses":[{"target":"s0","amount":{"value":0.001,"unit":"g/kg"},"at":{"value":0.0,"unit":"d"}}]});
        normalize(&mut args, &native, &model, None).unwrap();
        assert_eq!(args["doses"][0]["amount"], json!({"value":1.0,"unit":"1"}));
        let mut wrong = json!({"doses":[{"target":"central","amount":{"value":1.0,"unit":"mg"},"at":{"value":0.0,"unit":"d"}}]});
        assert!(normalize(&mut wrong, &native, &model, None).is_err());
    }
    #[test]
    fn invalid_declared_dose_units_are_rejected_before_execution() {
        let model = json!({"time_unit":"d","states":[{"name":"s0","dosing":{"scale":{"kind":"literal","value":1.0}}}]});
        for declaration in [json!("mg per mouse"), json!(false), json!("")] {
            let native = json!({"states":{"central":{"native_name":"s0","source_unit":"mg","request_unit":"1","dose_amount_unit":declaration}}});
            let mut args = json!({"doses":[{"target":"central","amount":{"value":1.0,"unit":"mg"},"at":{"value":0.0,"unit":"d"}}]});
            assert!(normalize(&mut args, &native, &model, None)
                .unwrap_err()
                .contains("unsupported dose input unit"));
        }
    }
    #[test]
    fn treatment_cycles_follow_repeats_lags_and_distinct_trigger_starts() {
        let model = json!({"time_unit":"h","covariates":[{"name":"cycle","unit":"1","interpolation":"step"}],
            "states":[{"name":"adc","dosing":{}},{"name":"other","dosing":{}}]});
        let native =
            json!({"treatment_cycle":{"covariate":"cycle","targets":["adc"],"first_cycle":1}});
        let mut request = json!({"regimen":{"start":{"value":0,"unit":"h"},"end":{"value":1500,"unit":"h"},
            "covariate_changes":[{"name":"cycle","time":{"value":504,"unit":"h"},"value":{"value":99,"unit":"1"}},
                {"name":"other_input","time":{"value":5,"unit":"h"},"value":{"value":3,"unit":"1"}}],
            "administrations":[
                {"target":"adc","time":{"value":0,"unit":"d"},"lag":{"value":2,"unit":"h"},"amount":{"value":100},"repeat":{"interval":{"value":21,"unit":"d"},"additional":2}},
                {"target":"adc","time":{"value":2,"unit":"h"},"amount":{"value":50}},
                {"target":"adc","time":{"value":9,"unit":"h"},"amount":{"value":0}},
                {"target":"other","time":{"value":10,"unit":"h"},"amount":{"value":100}},
                {"target":"adc","time":{"value":12,"unit":"h"},"amount":{"value":100},"bioavailability":0}]}});
        treatment_cycles(&mut request, &native, &model).unwrap();
        let cycles: Vec<_> = request["regimen"]["covariate_changes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["name"] == "cycle")
            .collect();
        assert_eq!(
            cycles
                .iter()
                .map(|r| r["time"]["value"].as_f64().unwrap())
                .collect::<Vec<_>>(),
            vec![2., 506., 1010.]
        );
        assert_eq!(
            cycles
                .iter()
                .map(|r| r["value"]["value"].as_f64().unwrap())
                .collect::<Vec<_>>(),
            vec![1., 2., 3.]
        );
        assert_eq!(request["regimen"]["covariates"]["cycle"]["value"], 1.);
        assert!(request["regimen"]["covariate_changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == "other_input"));
        let before = request.clone();
        treatment_cycles(&mut request, &Value::Null, &model).unwrap();
        assert_eq!(before, request);
    }
    #[test]
    fn invalid_cycle_declarations_and_unbounded_repetitions_are_rejected() {
        let model = json!({"time_unit":"h","covariates":[{"name":"cycle","unit":"1","interpolation":"step"}],"states":[{"name":"adc","dosing":{}}]});
        let valid = json!({"treatment_cycle":{"covariate":"cycle","targets":["adc"]}});
        let request = json!({"regimen":{"end":{"value":100,"unit":"h"},"administrations":[]}});
        for rule in [
            json!({"covariate":"missing","targets":["adc"]}),
            json!({"covariate":"cycle","targets":["missing"]}),
            json!({"covariate":"cycle","targets":["adc"],"first_cycle":1.5}),
        ] {
            assert!(treatment_cycles(
                &mut request.clone(),
                &json!({"treatment_cycle":rule}),
                &model
            )
            .is_err());
        }
        let mut large = request.clone();
        large["regimen"]["administrations"] = json!([{"target":"adc","time":{"value":0,"unit":"h"},"amount":{"value":1},"repeat":{"interval":{"value":1,"unit":"h"},"additional":10001}}]);
        assert!(treatment_cycles(&mut large, &valid, &model)
            .unwrap_err()
            .contains("repetition"));
    }
    #[test]
    fn administration_aliases_do_not_replace_archived_state_readouts() {
        let native = json!({"states":{"central":{"native_name":"state_readout","unit":"mg"}},
            "input_states":{"central":{"native_name":"s0","request_unit":"1","source_unit":"mg","dose_amount_unit":"mg"}}});
        let model = json!({"time_unit":"h","states":[{"name":"s0","unit":"1","dosing":{"amount_unit":"1","modes":["bolus"],"scale":{"kind":"literal","value":1.0}}}],
            "outputs":[{"name":"state_readout","unit":"1","expression":{"kind":"symbol","name":"s0"}}]});
        let augmented = augment(&native, &model);
        assert_eq!(native["states"]["central"]["native_name"], "state_readout");
        assert_eq!(augmented["states"]["central"]["native_name"], "s0");
        let mut args = json!({"doses":[{"target":"central","amount":{"value":0.16,"unit":"g"},"at":{"value":0,"unit":"h"}}]});
        normalize(&mut args, &augmented, &model, None).unwrap();
        assert_eq!(args["doses"][0]["target"], "s0");
        assert_eq!(
            args["doses"][0]["amount"],
            json!({"value":160.0,"unit":"1"})
        );
    }
}

#[cfg(test)]
mod output_contract_tests {
    use super::*;
    #[test]
    fn source_output_labels_keep_units_separate_and_conflicts_are_rejected() {
        let model = json!({"outputs":[{"name":"library_state_7","unit":"1","expression":{"kind":"symbol","name":"source_symbol_7"}}]});
        let native = augment(
            &json!({"states":{"IL6_pg_ml":{"native_name":"library_state_7","unit":"pg/mL"}},"outputs":{}}),
            &model,
        );
        let mapping = output_mapping(&model, &native, &["IL6_pg_ml".into()]).unwrap();
        assert_eq!(mapping[0].returned_column, "library_state_7");
        assert_eq!(mapping[0].returned_unit, "1");
        assert_eq!(mapping[0].declared_source_unit.as_deref(), Some("pg/mL"));
        assert!(mapping[0].analyte.is_none() && mapping[0].compartment.is_none());
        assert!(output_mapping(&model, &native, &["IL6_brain".into()]).is_err());
        let collision = json!({"outputs":[{"name":"a","unit":"1"},{"name":"b","unit":"1"}]});
        assert!(output_mapping(
            &collision,
            &json!({"outputs":{"a":{"native_name":"b"}}}),
            &["a".into()]
        )
        .unwrap_err()
        .contains("2 canonical matches"));
    }
    #[test]
    fn located_normalization_refusals_leave_every_argument_unchanged() {
        let native = json!({"parameters":{"clearance":{"native_name":"cl","source_unit":"L/h","request_unit":"L/h"}}});
        let model = json!({"time_unit":"h","outputs":[{"name":"cp","unit":"mg/L"}]});
        for (extra, pointer) in [
            (json!({"end":{"value":2,"unit":"mg"}}), "/end"),
            (
                json!({"readouts":[{"kind":"value","output":"cp","at":{"value":1,"unit":"h"},"side":"left"}]}),
                "/readouts/0/side",
            ),
            (
                json!({"readouts":[{"kind":"time_below","output":"cp","threshold":{"value":1,"unit":"mg"},"from":0,"to":2}]}),
                "/readouts/0/threshold",
            ),
            (
                json!({"readouts":[{"kind":"auc","output":"cp","from":2,"to":1}]}),
                "/readouts/0/to",
            ),
            (
                json!({"request":{"regimen":{"end":{"value":2,"unit":"h"},"samples":[{"value":0,"unit":"h"}],"observations":[{"time":{"value":0,"unit":"h"},"side":"pre"}]}}}),
                "/request/regimen/samples",
            ),
        ] {
            let mut args = json!({"parameters":{"clearance":{"value":24,"unit":"L/d"}}});
            args.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let before = args.clone();
            let error = normalize_located(&mut args, &native, &model, None)
                .unwrap_err()
                .into_value("request");
            assert_eq!(error["error"]["expression"], pointer);
            assert!(error["error"]["suggestion"].is_string());
            assert_eq!(args, before);
            assert!(normalize(&mut args, &native, &model, None).is_err());
            assert_eq!(args, before);
        }
    }
    #[test]
    fn located_unknown_alias_and_readout_fields_identify_submitted_input() {
        let native = json!({"parameters":{"clearance":{"native_name":"cl","source_unit":"L/h","request_unit":"L/h"}}});
        let model = json!({"time_unit":"h","outputs":[{"name":"cp","unit":"mg/L"}]});
        for (mut args, pointer) in [
            (
                json!({"parameters":{"missing/~":{"value":1,"unit":"L/h"}}}),
                "/parameters/missing~1~0",
            ),
            (
                json!({"readouts":[{"kind":"unknown","output":"cp"}]}),
                "/readouts/0/kind",
            ),
            (
                json!({"readouts":[{"kind":"auc","output":"missing"}]}),
                "/readouts/0/output",
            ),
            (
                json!({"readouts":[{"kind":"auc","output":"cp","window":{"from":0,"to":1,"bad/~":2}}]}),
                "/readouts/0/window/bad~1~0",
            ),
        ] {
            let before = args.clone();
            let error = normalize_located(&mut args, &native, &model, None)
                .unwrap_err()
                .into_value("request");
            assert_eq!(error["error"]["expression"], pointer);
            assert_eq!(args, before);
        }
    }
    #[test]
    fn successful_normalization_commits_the_existing_effective_values() {
        let native = json!({"parameters":{"clearance":{"native_name":"cl","source_unit":"L/h","request_unit":"L/h"}}});
        let model = json!({"time_unit":"h"});
        let mut args = json!({"parameters":{"clearance":{"value":24,"unit":"L/d"}},"end":{"value":2,"unit":"d"},"times":[{"value":1,"unit":"d"}]});
        let notes = normalize_located(&mut args, &native, &model, None).unwrap();
        assert_eq!(
            args,
            json!({"parameters":{"cl":{"value":1.0,"unit":"L/h"}},"end":{"value":48.0,"unit":"h"},"times":[{"value":24.0,"unit":"h"}],"_requested_times":[{"value":24.0,"unit":"h"}]})
        );
        assert!(notes.iter().any(|note| note.contains("clearance")));
    }

    #[test]
    fn source_coordinate_unit_refusal_preserves_the_declared_label_and_original_request() {
        let native = json!({"coordinates":"unchanged_source_numeric_values", "parameters":{
            "rate":{"native_name":"k","source_unit":"1","request_unit":"1","declared_source_unit":"1/h"}}});
        let model = json!({"time_unit":"h"});
        let mut args = json!({"parameters":{"rate":{"value":0.2,"unit":"1/h"}}});
        let before = args.clone();
        let error = normalize_located(&mut args, &native, &model, None)
            .unwrap_err()
            .into_value("request");
        assert_eq!(args, before);
        assert_eq!(error["error"]["expression"], "/parameters/rate/unit");
        let message = error["error"]["message"].as_str().unwrap();
        assert!(message.contains("1/h") && message.contains("'1'"));
        let hint = error["error"]["suggestion"].as_str().unwrap();
        assert!(hint.contains("1/h") && hint.contains("unchanged_source_numeric_values"));
        assert!(hint.contains("do not establish native physical dimensions"));
        assert!(!message.contains("molar mass") && !hint.contains("molar mass"));
    }

    #[test]
    fn undeclared_source_label_is_not_inferred_from_a_scientific_alias() {
        let native = json!({"coordinates":"unchanged_source_numeric_values","parameters":{
            "clearance_L_h":{"native_name":"cl","source_unit":"1","request_unit":"1"}}});
        let mut args = json!({"parameters":{"clearance_L_h":{"value":1,"unit":"L/h"}}});
        let error = normalize_located(&mut args, &native, &json!({"time_unit":"h"}), None)
            .unwrap_err()
            .into_value("request");
        let hint = error["error"]["suggestion"].as_str().unwrap();
        assert!(hint.contains("Archive source label: undeclared"));
        assert!(!hint.contains("Archive source label: \"L/h\""));
    }

    #[test]
    fn source_labels_do_not_change_successful_numeric_coordinate_edits_or_later_hints() {
        let native = json!({"coordinates":"unchanged_source_numeric_values", "parameters":{
            "rate":{"native_name":"k","source_unit":"1","request_unit":"1","declared_source_unit":"1/h"}}});
        let model = json!({"time_unit":"h"});
        let mut args = json!({"parameters":{"rate":{"value":0.2,"unit":"1"}}});
        normalize_located(&mut args, &native, &model, None).unwrap();
        assert_eq!(args["parameters"], json!({"k":{"value":0.2,"unit":"1"}}));
        let mut late =
            json!({"parameters":{"rate":{"value":0.2,"unit":"1"}},"end":{"value":1,"unit":"mg"}});
        let before = late.clone();
        let error = normalize_located(&mut late, &native, &model, None)
            .unwrap_err()
            .into_value("request");
        assert_eq!(late, before);
        assert!(!error["error"]["suggestion"]
            .as_str()
            .unwrap()
            .contains("Archive source label"));
    }

    #[test]
    fn native_source_coordinate_refusals_preserve_codes_locations_and_other_failures() {
        let native = json!({"coordinates":"unchanged_source_numeric_values"});
        for code in ["unit", "unknown_unit"] {
            let original = json!({"error":{"code":code,"message":"requested 1/h, declared 1","expression":"parameters.k","time":null,"suggestion":"Use the declared unit."}});
            let annotated = source_coordinate_refusal(original.clone(), &native);
            for key in ["code", "message", "expression", "time"] {
                assert_eq!(annotated["error"][key], original["error"][key]);
            }
            assert!(annotated["error"]["suggestion"]
                .as_str()
                .unwrap()
                .starts_with("Use the declared unit."));
            assert!(annotated["error"]["suggestion"]
                .as_str()
                .unwrap()
                .contains("unchanged_source_numeric_values"));
            assert_eq!(
                source_coordinate_refusal(annotated.clone(), &native),
                annotated
            );
            assert_eq!(
                source_coordinate_refusal(original.clone(), &json!({})),
                original
            );
        }
        for value in [
            json!({"error":{"code":"solver","message":"failed"}}),
            json!({"outputs":[]}),
            json!({"error":null}),
        ] {
            assert_eq!(source_coordinate_refusal(value.clone(), &native), value);
        }
    }

    #[test]
    fn archived_state_readout_label_remains_separate_from_the_native_initial_state_unit() {
        let model = json!({"time_unit":"h","states":[{"name":"s","unit":"1"}],
            "outputs":[{"name":"readout","unit":"1","expression":{"kind":"symbol","name":"s"}}]});
        let native = json!({"coordinates":"unchanged_source_numeric_values",
            "states":{"IL6_pg_ml":{"native_name":"readout","unit":"pg/mL"}}});
        let named = augment(&native, &model);
        let mut args = json!({"initial_states":{"IL6_pg_ml":{"value":10,"unit":"pg/mL"}}});
        let before = args.clone();
        let error = normalize_located(&mut args, &named, &model, None)
            .unwrap_err()
            .into_value("request");
        assert_eq!(args, before);
        assert_eq!(
            error["error"]["expression"],
            "/initial_states/IL6_pg_ml/unit"
        );
        let hint = error["error"]["suggestion"].as_str().unwrap();
        assert!(hint.contains("accepted alias coordinate \"1\""));
        assert!(hint.contains("Archive source label: \"pg/mL\""));
    }
}

//! Canonical, dependency-free tidy tables shared by all interfaces.
use crate::{regimen, units::Unit, Error, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const ROW_LIMIT: usize = 1_000_000;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Frame {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub metadata: Value,
}
impl Frame {
    pub fn new(columns: &str, source: &Value) -> Self {
        Self {
            columns: columns.split(',').map(str::to_owned).collect(),
            rows: vec![],
            metadata: json!({"identity":source.get("identity"),"time_unit":source.get("time_unit"),"schema":source.get("schema")}),
        }
    }
    pub fn push(&mut self, row: Vec<Value>) -> Result<(), Error> {
        if row.len() != self.columns.len() {
            return Err(invalid("frame column count mismatch"));
        }
        if self.rows.len() >= ROW_LIMIT {
            return Err(Error::new(
                ErrorCode::OutputBudget,
                "tidy frame exceeds one million rows",
            ));
        }
        self.rows.push(row);
        Ok(())
    }
    pub fn records(&self) -> Vec<Value> {
        self.rows
            .iter()
            .map(|row| {
                Value::Object(
                    self.columns
                        .iter()
                        .cloned()
                        .zip(row.iter().cloned())
                        .collect(),
                )
            })
            .collect()
    }
    pub fn csv(&self) -> String {
        fn cell(v: &Value) -> String {
            let s = match v {
                Value::Null => String::new(),
                Value::String(s) => s.clone(),
                _ => v.to_string(),
            };
            format!("\"{}\"", s.replace('"', "\"\""))
        }
        let mut s = self
            .columns
            .iter()
            .map(|c| cell(&json!(c)))
            .collect::<Vec<_>>()
            .join(",")
            + "\n";
        for row in &self.rows {
            s.push_str(&row.iter().map(cell).collect::<Vec<_>>().join(","));
            s.push('\n');
        }
        s
    }
}
fn invalid(s: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidInput, s)
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>, Error> {
    v.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("missing array {key}")))
}
fn number(v: &Value, key: &str) -> Result<f64, Error> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .ok_or_else(|| invalid(format!("missing finite number {key}")))
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str, Error> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("missing string {key}")))
}
fn at(v: &Value, k: &str) -> Value {
    v.get(k).cloned().unwrap_or(Value::Null)
}

pub fn trajectory(result: &Value) -> Result<Frame, Error> {
    if result["schema"] != "qspquest.result/v0.1" {
        return Err(invalid("trajectory requires qspquest.result/v0.1"));
    }
    let times = array(result, "times")?;
    let sides = array(result, "sides")?;
    if times.len() != sides.len() {
        return Err(invalid("times/sides length mismatch"));
    }
    string(result, "time_unit")?;
    let mut prev = None;
    for (i, t) in times.iter().enumerate() {
        let t = t
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| invalid("time must be finite"))?;
        let side = sides[i]
            .as_str()
            .ok_or_else(|| invalid("side must be pre or post"))?;
        if side != "pre" && side != "post" {
            return Err(invalid("side must be pre or post"));
        }
        if let Some(p) = prev {
            if t < p || (t == p && (sides[i - 1] != "pre" || side != "post")) {
                return Err(invalid(
                    "times must be ordered, duplicate times must be pre then post",
                ));
            }
        }
        prev = Some(t);
    }
    let mut f = Frame::new("time,side,output,value,unit,series", result);
    let mut names = BTreeSet::new();
    for c in array(result, "outputs")? {
        let name = string(c, "name")?;
        string(c, "unit")?;
        if !names.insert(name) {
            return Err(invalid("duplicate output name"));
        }
        let values = array(c, "values")?;
        if values.len() != times.len() {
            return Err(invalid("output length differs from times"));
        }
        for i in 0..times.len() {
            if !values[i].is_null() && values[i].as_f64().filter(|n| n.is_finite()).is_none() {
                return Err(invalid("output values must be finite or null"));
            }
            f.push(vec![
                times[i].clone(),
                sides[i].clone(),
                json!(name),
                values[i].clone(),
                at(c, "unit"),
                json!(name),
            ])?;
        }
    }
    Ok(f)
}

/// Explicit administration expansion, using the core's unit and event validation.
/// Model-dependent lag and scaling cannot be inferred from a request alone.
pub fn events(request: &Value, time_unit: Option<&str>) -> Result<Frame, Error> {
    let r = request.get("regimen").unwrap_or(request);
    let regimen: regimen::Request =
        serde_json::from_value(r.clone()).map_err(|e| invalid(e.to_string()))?;
    let unit = time_unit.unwrap_or(&regimen.end.unit);
    let tu = Unit::parse(unit)?;
    let end = Unit::parse(&regimen.end.unit)?.convert(regimen.end.value, tu)?;
    let start = regimen
        .start
        .as_ref()
        .map(|q| Unit::parse(&q.unit)?.convert(q.value, tu))
        .transpose()?
        .unwrap_or(0.0);
    let mut targets = vec![];
    for a in &regimen.administrations {
        if !targets.iter().any(|t: &regimen::Target| t.name == a.target) {
            targets.push(regimen::Target {
                name: a.target.clone(),
                state: targets.len(),
                amount_unit: Unit::parse(&a.amount.unit)?,
                modes: vec![
                    crate::model::DeliveryMode::Bolus,
                    crate::model::DeliveryMode::Infusion,
                ],
            });
        }
    }
    let limit = request["budgets"]["events"]
        .as_u64()
        .unwrap_or(10_000)
        .min(100_000) as usize;
    let expanded = regimen::expand_administrations_in_window(
        &regimen.administrations,
        &targets,
        tu,
        start,
        end,
        limit,
    )?;
    let mut f = Frame::new("time,kind,target,amount,unit,duration,regimen", request);
    f.metadata["time_unit"] = json!(unit);
    f.metadata["boundary"] =
        json!("Explicit request events; model-dependent lag and scaling require model context");
    for e in expanded {
        let a = &regimen.administrations[e.administration_index];
        let duration = match e.event {
            crate::Event::Infusion { duration, .. } => duration,
            _ => 0.0,
        };
        let unit = &regimen
            .administrations
            .iter()
            .find(|x| x.target == a.target)
            .unwrap()
            .amount
            .unit;
        let row = |time, kind: &str| {
            vec![
                json!(time),
                json!(kind),
                json!(a.target),
                json!(e.administered_amount),
                json!(unit),
                json!(duration),
                at(request, "regimen_id"),
            ]
        };
        if e.scheduled_time != e.event.time() {
            f.push(row(e.scheduled_time, "lag"))?;
        }
        f.push(row(
            e.event.time(),
            if duration > 0.0 {
                "infusion_start"
            } else {
                "bolus"
            },
        ))?;
        if duration > 0.0 {
            f.push(row(e.event.time() + duration, "infusion_end"))?;
        }
    }
    for c in &regimen.covariate_changes {
        let t = Unit::parse(&c.time.unit)?.convert(c.time.value, tu)?;
        f.push(vec![
            json!(t),
            json!("switch"),
            json!(c.name),
            Value::Null,
            json!(c.value.unit),
            Value::Null,
            at(request, "regimen_id"),
        ])?;
    }
    for c in &regimen.resets {
        let t = Unit::parse(&c.time.unit)?.convert(c.time.value, tu)?;
        f.push(vec![
            json!(t),
            json!("reset"),
            json!(c.target),
            json!(c.value.value),
            json!(c.value.unit),
            Value::Null,
            at(request, "regimen_id"),
        ])?;
    }
    f.rows
        .sort_by(|a, b| a[0].as_f64().unwrap().total_cmp(&b[0].as_f64().unwrap()));
    Ok(f)
}

/// Convert scientific results; unsupported/missing schemas are errors, not guessed tables.
pub fn convert(result: &Value, kind: &str) -> Result<Frame, Error> {
    match kind {
        "trajectory" => trajectory(result),
        "gof" => {
            let rows = result
                .get("fitted_observations")
                .or_else(|| result.get("observations"))
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("fit has no observation rows"))?;
            let mut f = Frame::new(
                "subject,time,observed,pred,ipred,residual,residual_kind,censoring,lloq,unit,output,side",
                result,
            );
            for r in rows {
                f.push(vec![
                    r.get("subject_id").cloned().unwrap_or(json!("pooled")),
                    at(r, "time"),
                    at(r, "observed"),
                    at(r, "prediction"),
                    r.get("individual_prediction")
                        .cloned()
                        .unwrap_or(at(r, "prediction")),
                    at(r, "standardized_residual"),
                    json!("conditional standardized residual (not CWRES/NPDE)"),
                    at(r, "censoring"),
                    if r["censoring"] == "left" {
                        at(r, "observed")
                    } else {
                        at(r, "lloq")
                    },
                    at(r, "unit"),
                    at(r, "output"),
                    at(r, "side"),
                ])?;
            }
            Ok(f)
        }
        "vpc_bins" => {
            if result["schema"] != "qspquest.vpc/v0.1" {
                return Err(invalid("vpc requires qspquest.vpc/v0.1"));
            }
            let mut f = Frame::new(
                "stratum,start,end,mid,percentile,observed,simulated,ci_low,ci_high,n,obs_blq,sim_blq,blq_low,blq_high",
                result,
            );
            for b in array(result, "bins")? {
                for q in array(b, "quantiles")? {
                    f.push(vec![
                        at(b, "stratum"),
                        at(b, "start"),
                        at(b, "end"),
                        json!((number(b, "start")? + number(b, "end")?) / 2.),
                        at(q, "percentile"),
                        at(q, "observed"),
                        at(q, "simulated"),
                        q["confidence_interval"][0].clone(),
                        q["confidence_interval"][1].clone(),
                        at(b, "observations"),
                        at(b, "observed_blq_fraction"),
                        at(b, "simulated_blq_fraction"),
                        b["blq_confidence_interval"][0].clone(),
                        b["blq_confidence_interval"][1].clone(),
                    ])?;
                }
            }
            Ok(f)
        }
        "profile" => {
            if result["schema"] != "qspquest.profile/v0.1" {
                return Err(invalid("profile requires qspquest.profile/v0.1"));
            }
            let mut f = Frame::new("parameter,value,delta_nll,unit,status", result);
            for r in array(result, "profile")? {
                f.push(vec![
                    at(result, "parameter"),
                    at(r, "value"),
                    if r["status"] == "converged" {
                        at(r, "delta_nll")
                    } else {
                        Value::Null
                    },
                    at(result, "unit"),
                    at(r, "status"),
                ])?;
            }
            Ok(f)
        }
        "bootstrap" => {
            if result["schema"] != "qspquest.bootstrap/v0.1" {
                return Err(invalid("bootstrap requires qspquest.bootstrap/v0.1"));
            }
            let mut f = Frame::new("replicate,parameter,value,status,unit", result);
            for r in array(result, "replicates")? {
                if let Some(p) = r["parameters"].as_object() {
                    for (k, v) in p {
                        f.push(vec![
                            at(r, "replicate"),
                            json!(k),
                            at(v, "value"),
                            at(r, "status"),
                            at(v, "unit"),
                        ])?;
                    }
                } else {
                    f.push(vec![
                        at(r, "replicate"),
                        Value::Null,
                        Value::Null,
                        at(r, "status"),
                        Value::Null,
                    ])?;
                }
            }
            Ok(f)
        }
        "morris" => {
            let r = result.get("result").unwrap_or(result);
            let mut f = Frame::new("parameter,output,mu,mu_star,sigma,unit", result);
            for e in array(r, "effects")? {
                f.push(vec![
                    at(e, "parameter"),
                    at(e, "output"),
                    at(e, "mu"),
                    at(e, "mu_star"),
                    at(e, "sigma"),
                    at(e, "unit"),
                ])?;
            }
            Ok(f)
        }
        "scan" => {
            if result["schema"] != "qspquest.scan-result/v0.1" {
                return Err(invalid("scan requires qspquest.scan-result/v0.1"));
            }
            let mut f = Frame::new(
                "point,parameter_a,value_a,parameter_b,value_b,output,time,value,unit,side,parameter_a_unit,parameter_b_unit",
                result,
            );
            for p in array(result, "results")? {
                let params = p["parameters"]
                    .as_object()
                    .ok_or_else(|| invalid("scan needs named parameters"))?;
                if params.is_empty() || params.len() > 2 {
                    return Err(invalid("scan figures require one or two varied parameters"));
                }
                let pairs = params.iter().collect::<Vec<_>>();
                for row in trajectory(&p["result"])?.rows {
                    f.push(vec![
                        at(p, "id"),
                        json!(pairs[0].0),
                        at(pairs[0].1, "value"),
                        pairs.get(1).map(|p| json!(p.0)).unwrap_or(Value::Null),
                        pairs
                            .get(1)
                            .map(|p| at(p.1, "value"))
                            .unwrap_or(Value::Null),
                        row[2].clone(),
                        row[0].clone(),
                        row[3].clone(),
                        row[4].clone(),
                        row[1].clone(),
                        at(pairs[0].1, "unit"),
                        pairs.get(1).map(|p| at(p.1, "unit")).unwrap_or(Value::Null),
                    ])?;
                }
                f.metadata["time_unit"] = at(&p["result"], "time_unit");
            }
            Ok(f)
        }
        "population_fan" => {
            if result["schema"] != "qspquest.regimen-comparison/v0.1" {
                return Err(invalid("fan requires qspquest.regimen-comparison/v0.1"));
            }
            let mut f = Frame::new(
                "time,side,output,value,unit,series,regimen,percentile",
                result,
            );
            for (name, r) in result["regimens"]
                .as_object()
                .ok_or_else(|| invalid("missing regimens"))?
            {
                let t = array(r, "times")?;
                let s = array(r, "sides")?;
                if t.len() != s.len() {
                    return Err(invalid("regimen times/sides mismatch"));
                }
                let ps = array(r, "percentiles")?;
                let curves = array(r, "curve_percentiles")?;
                if ps.len() != curves.len() {
                    return Err(invalid("percentiles/curves mismatch"));
                }
                for (p, ys) in ps.iter().zip(curves) {
                    let ys = ys
                        .as_array()
                        .ok_or_else(|| invalid("curve must be an array"))?;
                    if ys.len() != t.len() {
                        return Err(invalid("curve length mismatch"));
                    }
                    for i in 0..t.len() {
                        f.push(vec![
                            t[i].clone(),
                            s[i].clone(),
                            result
                                .get("output")
                                .cloned()
                                .unwrap_or(json!("concentration")),
                            ys[i].clone(),
                            at(r, "unit"),
                            json!(name),
                            json!(name),
                            p.clone(),
                        ])?;
                    }
                }
                f.metadata["time_unit"] = at(r, "time_unit");
            }
            Ok(f)
        }
        "exposure" => {
            let mut f = Frame::new("regimen,subject,metric,value,unit", result);
            let add =
                |f: &mut Frame, r: &Value, label: &str, subject: Value| -> Result<(), Error> {
                    for key in ["auc", "cmax", "cmin", "endpoint", "tmax"] {
                        if r.get(key).is_some() {
                            let unit = match key {
                                "auc" => at(r, "auc_unit"),
                                "tmax" => at(r, "time_unit"),
                                _ => at(r, "unit"),
                            };
                            f.push(vec![
                                json!(label),
                                subject.clone(),
                                json!(key),
                                at(r, key),
                                unit,
                            ])?;
                        }
                    }
                    if !r["time_above_threshold"].is_null() {
                        f.push(vec![
                            json!(label),
                            subject,
                            json!("time_above_threshold"),
                            r["time_above_threshold"]["duration"].clone(),
                            at(r, "time_unit"),
                        ])?;
                    }
                    Ok(())
                };
            if let Some(rs) = result["regimens"].as_object() {
                for (label, r) in rs {
                    for (i, m) in array(r, "subject_metrics")?.iter().enumerate() {
                        add(&mut f, m, label, json!(i))?;
                    }
                }
            } else if result["method"] == "sampled_piecewise_linear_no_extrapolation" {
                add(&mut f, result, "result", Value::Null)?;
            } else {
                return Err(invalid(
                    "exposure requires an exposure summary or paired regimen comparison",
                ));
            }
            Ok(f)
        }
        "forest" => {
            let mut f = Frame::new("effect,estimate,low,high,reference", result);
            for r in array(result, "effects")? {
                f.push(vec![
                    at(r, "effect"),
                    at(r, "estimate"),
                    at(r, "low"),
                    at(r, "high"),
                    at(r, "reference"),
                ])?;
            }
            Ok(f)
        }
        "eta" => {
            let mut f = Frame::new("subject,parameter,value,shrinkage", result);
            let names = array(result, "random_effects")?;
            for (i, row) in array(result, "eta")?.iter().enumerate() {
                let row = row
                    .as_array()
                    .ok_or_else(|| invalid("eta must be a matrix"))?;
                if row.len() != names.len() {
                    return Err(invalid("eta dimension mismatch"));
                }
                for (j, y) in row.iter().enumerate() {
                    f.push(vec![
                        result["subject_ids"][i].clone(),
                        names[j].clone(),
                        y.clone(),
                        result["diagnostics"]["eta_variance_shrinkage"][j].clone(),
                    ])?;
                }
            }
            Ok(f)
        }
        "trace" => {
            let mut f = Frame::new("iteration,parameter,value", result);
            let trace = result
                .get("trace")
                .or_else(|| result.get("parameter_trace"))
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("missing trace"))?;
            for r in trace {
                if let Some(xs) = r["acceptance_by_axis"].as_array() {
                    for (i, v) in xs.iter().enumerate() {
                        f.push(vec![
                            at(r, "iteration"),
                            json!(format!("acceptance {i}")),
                            v.clone(),
                        ])?;
                    }
                }
                if let Some(xs) = r["parameters"].as_object() {
                    for (k, v) in xs {
                        f.push(vec![
                            r.get("iteration").cloned().unwrap_or(at(r, "simulation")),
                            json!(k),
                            if v.is_object() {
                                at(v, "value")
                            } else {
                                v.clone()
                            },
                        ])?;
                    }
                }

                if let Some(xs) = r["fixed_effects"].as_array() {
                    for (i, v) in xs.iter().enumerate() {
                        f.push(vec![
                            at(r, "iteration"),
                            result["random_effects"][i].clone(),
                            v.clone(),
                        ])?;
                    }
                }
                if let Some(xs) = r["error"].as_object() {
                    for (k, v) in xs {
                        if !v.is_number() {
                            continue;
                        }
                        f.push(vec![
                            r.get("iteration").cloned().unwrap_or(at(r, "simulation")),
                            json!(format!("error: {k}")),
                            v.clone(),
                        ])?;
                    }
                }
                if let Some(xs) = r["omega"].as_array() {
                    for (i, x) in xs.iter().enumerate() {
                        f.push(vec![
                            at(r, "iteration"),
                            json!(format!("variance {i}")),
                            x[i].clone(),
                        ])?;
                    }
                }
                if !r["objective"].is_null() {
                    f.push(vec![
                        r.get("iteration").cloned().unwrap_or(at(r, "simulation")),
                        json!("objective"),
                        at(r, "objective"),
                    ])?;
                }
            }
            Ok(f)
        }
        "sensitivity" => {
            let run = &result["result"];
            let t = array(run, "times")?;
            let mut f = Frame::new("parameter,output,time,value,unit,side", result);
            for d in array(result, "derivatives")? {
                let ys = array(d, "values")?;
                if ys.len() != t.len() {
                    return Err(invalid("derivative length mismatch"));
                }
                for i in 0..t.len() {
                    f.push(vec![
                        at(d, "parameter"),
                        at(d, "output"),
                        t[i].clone(),
                        ys[i].clone(),
                        at(d, "unit"),
                        run["sides"][i].clone(),
                    ])?;
                }
            }
            f.metadata["time_unit"] = at(run, "time_unit");
            Ok(f)
        }
        _ => Err(invalid(format!("unknown frame {kind}"))),
    }
}

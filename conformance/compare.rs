//! Offline numerical comparison only; corpus coverage is a separate requirement.
use qspquest_core::units::Unit;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trajectory {
    pub time_unit: String,
    pub times: Vec<f64>,
    pub sides: Vec<Side>,
    pub outputs: Vec<Output>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Pre,
    Post,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub name: String,
    pub unit: String,
    pub values: Vec<f64>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tolerance {
    pub unit: String,
    pub absolute: f64,
    pub relative: f64,
}
#[derive(Debug, Serialize)]
pub struct Comparison {
    pub time_unit: String,
    pub passed: bool,
    pub rows: usize,
    pub compared_values: usize,
    pub outputs: Vec<OutputComparison>,
}
#[derive(Debug, Serialize)]
pub struct OutputComparison {
    pub name: String,
    pub unit: String,
    pub failed_values: usize,
    pub maximum_absolute_error: f64,
    pub worst_row: usize,
    pub worst_time: f64,
    pub worst_side: Side,
}
fn unit(code: &str) -> Result<Unit, String> {
    Unit::parse(code).map_err(|e| e.message)
}
fn validate(t: &Trajectory) -> Result<BTreeMap<&str, &Output>, String> {
    if t.times.is_empty() || t.times.len() != t.sides.len() || t.outputs.is_empty() {
        return Err("empty or inconsistent trajectory".into());
    }
    let time = unit(&t.time_unit)?;
    if time.dimension != unit("s")?.dimension {
        return Err("time unit has wrong dimension".into());
    }
    if t.times.iter().any(|x| !x.is_finite()) {
        return Err("invalid sample time".into());
    }
    let mut names = BTreeMap::new();
    for output in &t.outputs {
        unit(&output.unit)?;
        if output.name.is_empty() || names.insert(output.name.as_str(), output).is_some() {
            return Err("empty or duplicate output name".into());
        }
        if output.values.len() != t.times.len() || output.values.iter().any(|x| !x.is_finite()) {
            return Err(format!("invalid values for {}", output.name));
        }
    }
    Ok(names)
}
pub fn compare(
    reference: &Trajectory,
    candidate: &Trajectory,
    tolerances: &BTreeMap<String, Tolerance>,
) -> Result<Comparison, String> {
    let r = validate(reference)?;
    let c = validate(candidate)?;
    if r.keys().ne(c.keys()) || r.keys().copied().ne(tolerances.keys().map(String::as_str)) {
        return Err("output and tolerance names must match exactly".into());
    }
    if reference.times.len() != candidate.times.len() || reference.sides != candidate.sides {
        return Err("sample counts or sides differ".into());
    }
    let rt = unit(&reference.time_unit)?;
    let ct = unit(&candidate.time_unit)?;
    for (a, b) in reference.times.iter().zip(&candidate.times) {
        let b = ct.convert(*b, rt).map_err(|e| e.message)?;
        if *a != b {
            return Err(
                "sample times differ after unit conversion; no implicit interpolation".into(),
            );
        }
    }
    let mut outputs = Vec::new();
    for (name, ro) in r {
        let co = c[name];
        let tol = &tolerances[name];
        if !tol.absolute.is_finite()
            || !tol.relative.is_finite()
            || tol.absolute < 0.0
            || tol.relative < 0.0
        {
            return Err(format!("invalid tolerance for {name}"));
        }
        let ru = unit(&ro.unit)?;
        let cu = unit(&co.unit)?;
        let tu = unit(&tol.unit)?;
        let mut report = OutputComparison {
            name: name.into(),
            unit: tol.unit.clone(),
            failed_values: 0,
            maximum_absolute_error: 0.0,
            worst_row: 0,
            worst_time: reference.times[0],
            worst_side: reference.sides[0],
        };
        for (i, (a, b)) in ro.values.iter().zip(&co.values).enumerate() {
            let a = ru.convert(*a, tu).map_err(|e| e.message)?;
            let b = cu.convert(*b, tu).map_err(|e| e.message)?;
            let error = (a - b).abs();
            let limit = tol.absolute + tol.relative * a.abs();
            if !error.is_finite() || !limit.is_finite() {
                return Err(format!("comparison arithmetic overflow for {name}"));
            }
            if error > limit {
                report.failed_values += 1;
            }
            if error > report.maximum_absolute_error {
                report.maximum_absolute_error = error;
                report.worst_row = i;
                report.worst_time = reference.times[i];
                report.worst_side = reference.sides[i];
            }
        }
        outputs.push(report);
    }
    Ok(Comparison {
        time_unit: reference.time_unit.clone(),
        passed: outputs.iter().all(|o| o.failed_values == 0),
        rows: reference.times.len(),
        compared_values: reference.times.len() * outputs.len(),
        outputs,
    })
}

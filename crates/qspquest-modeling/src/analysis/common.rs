//! Shared validation, simulation budgets, bindings and likelihoods.
use super::control::CancelFlag;
use crate::document::CompiledDocument;
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A structured analysis failure. Serializes as `{"code", "message", ...}`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisError {
    pub code: String,
    pub message: String,
    pub time: Option<f64>,
    pub expression: Option<String>,
}

impl AnalysisError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            time: None,
            expression: None,
        }
    }
    pub fn to_value(&self) -> Value {
        let mut value = json!({"code": self.code, "message": self.message});
        if let Some(time) = self.time {
            value["time"] = json!(time);
        }
        if let Some(expression) = &self.expression {
            value["expression"] = json!(expression);
        }
        value
    }
}

impl std::fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AnalysisError {}

impl From<qspquest_core::Error> for AnalysisError {
    fn from(error: qspquest_core::Error) -> Self {
        let code = serde_json::to_value(&error.code)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "internal".to_string());
        Self {
            code,
            message: error.message,
            time: error.time,
            expression: error.expression,
        }
    }
}

pub type Result<T> = std::result::Result<T, AnalysisError>;

/// Largest seed: integers above 2^53 - 1 are not exact in JSON numbers.
pub const MAX_SEED: u64 = (1 << 53) - 1;

pub fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(AnalysisError::new("invalid_input", message))
}

pub fn fail_code<T>(message: impl Into<String>, code: &str) -> Result<T> {
    Err(AnalysisError::new(code, message))
}

/// Native failures that reject one trial point instead of the analysis.
pub fn recoverable(error: &AnalysisError) -> bool {
    matches!(
        error.code.as_str(),
        "domain" | "invalid_input" | "integrity" | "solver"
    )
}

pub fn finite(value: Option<&Value>, name: &str) -> Result<f64> {
    match value {
        Some(Value::Number(n)) => match n.as_f64() {
            Some(v) if v.is_finite() => Ok(v),
            _ => fail(format!("{name} must be a finite number")),
        },
        _ => fail(format!("{name} must be a finite number")),
    }
}

pub fn integer(value: Option<&Value>, name: &str, lower: u64, upper: u64) -> Result<u64> {
    match value.and_then(Value::as_u64) {
        Some(v) if (lower..=upper).contains(&v) => Ok(v),
        _ => fail(format!("{name} must be an integer in [{lower}, {upper}]")),
    }
}

/// Argument object with an explicit set of accepted keys.
pub struct Arguments<'a> {
    object: &'a Map<String, Value>,
}

impl<'a> Arguments<'a> {
    pub fn new(value: &'a Value, accepted: &[&str]) -> Result<Self> {
        let Some(object) = value.as_object() else {
            return fail("analysis arguments must be a JSON object");
        };
        for key in object.keys() {
            if !accepted.contains(&key.as_str()) {
                return fail(format!("unknown analysis argument {key}"));
            }
        }
        Ok(Self { object })
    }
    pub fn get(&self, key: &str) -> Option<&'a Value> {
        self.object.get(key).filter(|v| !v.is_null())
    }
    pub fn required(&self, key: &str) -> Result<&'a Value> {
        match self.get(key) {
            Some(v) => Ok(v),
            None => fail(format!("{key} is required")),
        }
    }
    pub fn float(&self, key: &str, default: f64) -> Result<f64> {
        match self.get(key) {
            None => Ok(default),
            v => finite(v, key),
        }
    }
    pub fn int(&self, key: &str, default: u64, lower: u64, upper: u64) -> Result<u64> {
        match self.get(key) {
            None => integer(Some(&json!(default)), key, lower, upper),
            v => integer(v, key, lower, upper),
        }
    }
    pub fn boolean(&self, key: &str, default: bool) -> Result<bool> {
        match self.get(key) {
            None => Ok(default),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => fail(format!("{key} must be true or false")),
        }
    }
    pub fn string(&self, key: &str, default: &str) -> Result<String> {
        match self.get(key) {
            None => Ok(default.to_string()),
            Some(Value::String(s)) => Ok(s.clone()),
            Some(_) => fail(format!("{key} must be a string")),
        }
    }
}

/// One solved trajectory, decoded from a `qspquest.result/v0.1` record.
#[derive(Clone, Debug)]
pub struct RunOutput {
    pub times: Vec<f64>,
    pub sides: Vec<String>,
    pub time_unit: String,
    pub outputs: Vec<(String, String, Vec<f64>)>,
    pub identity: Value,
}

impl RunOutput {
    pub fn from_value(result: &Value) -> Result<Self> {
        let numbers = |v: Option<&Value>, name: &str| -> Result<Vec<f64>> {
            let Some(items) = v.and_then(Value::as_array) else {
                return fail(format!("result {name} must be an array of numbers"));
            };
            items
                .iter()
                .map(|x| match x.as_f64() {
                    Some(v) => Ok(v),
                    None => fail(format!("result {name} must be an array of numbers")),
                })
                .collect()
        };
        let times = numbers(result.get("times"), "times")?;
        let sides = match result.get("sides").and_then(Value::as_array) {
            Some(items) => items
                .iter()
                .map(|s| s.as_str().map(str::to_string).ok_or(()))
                .collect::<std::result::Result<Vec<_>, _>>()
                .or_else(|_| fail("result sides must be strings"))?,
            None => return fail("result sides must be an array"),
        };
        let time_unit = result
            .get("time_unit")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| AnalysisError::new("invalid_input", "result time_unit is required"))?;
        let mut outputs = Vec::new();
        for column in result
            .get("outputs")
            .and_then(Value::as_array)
            .ok_or_else(|| AnalysisError::new("invalid_input", "result outputs must be an array"))?
        {
            let name = column.get("name").and_then(Value::as_str);
            let unit = column.get("unit").and_then(Value::as_str);
            let (Some(name), Some(unit)) = (name, unit) else {
                return fail("result output columns require name and unit");
            };
            outputs.push((
                name.to_string(),
                unit.to_string(),
                numbers(column.get("values"), "output values")?,
            ));
        }
        Ok(Self {
            times,
            sides,
            time_unit,
            outputs,
            identity: result.get("identity").cloned().unwrap_or(Value::Null),
        })
    }
    pub fn column(&self, name: &str) -> Option<(&[f64], &str)> {
        self.outputs
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, unit, values)| (values.as_slice(), unit.as_str()))
    }
}

/// Aggregate count of dispatched native simulations, shared across workers.
pub struct Budget {
    pub maximum: usize,
    used: AtomicUsize,
    cancel: Option<CancelFlag>,
}

impl Budget {
    pub fn new(maximum: u64, cancel: Option<CancelFlag>) -> Result<Self> {
        if !(1..=1_000_000).contains(&maximum) {
            return fail("max_simulations must be an integer in [1, 1000000]");
        }
        Ok(Self {
            maximum: maximum as usize,
            used: AtomicUsize::new(0),
            cancel,
        })
    }
    pub fn used(&self) -> usize {
        self.used.load(Ordering::SeqCst)
    }
    pub fn check(&self) -> Result<()> {
        if let Some(flag) = &self.cancel {
            if flag.load(Ordering::SeqCst) {
                return fail_code(
                    "Analysis cancelled. Resume the last complete checkpoint with a new control; unfinished work is discarded.",
                    "cancelled",
                );
            }
        }
        Ok(())
    }
    /// Count retained work from a checkpoint before restarting.
    pub fn restore(&self, saved: Option<&Value>) -> Result<()> {
        let saved = integer(saved, "checkpoint simulations", 0, self.maximum as u64)? as usize;
        let total = self.used.fetch_add(saved, Ordering::SeqCst) + saved;
        if total > self.maximum {
            return fail_code(
                "checkpoint restart exhausted the aggregate simulation budget",
                "budget",
            );
        }
        Ok(())
    }
    /// Charge one native call before it runs, including calls that fail.
    pub fn charge(&self) -> Result<()> {
        self.check()?;
        let charged = self
            .used
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |used| {
                (used < self.maximum).then_some(used + 1)
            });
        if charged.is_err() {
            return fail_code("aggregate native simulation budget exhausted", "budget");
        }
        Ok(())
    }
    /// Charge one native call, including calls that fail, then execute it.
    pub fn run(&self, model: &Arc<CompiledDocument>, request: &Value) -> Result<RunOutput> {
        self.charge()?;
        let request: qspquest_core::run::RunRequest = serde_json::from_value(request.clone())
            .map_err(|e| AnalysisError::new("invalid_input", e.to_string()))?;
        let result = model.execute(&request)?;
        let value = serde_json::to_value(&result)
            .map_err(|e| AnalysisError::new("internal", e.to_string()))?;
        RunOutput::from_value(&value)
    }
}

/// A bounded fitted parameter: a model parameter or an administration quantity.
#[derive(Clone, Debug)]
pub struct Spec {
    pub name: String,
    pub initial: f64,
    pub lower: f64,
    pub upper: f64,
    pub unit: String,
    pub path: Option<Vec<Value>>,
}

pub fn parameter_specs(parameters: &Value) -> Result<Vec<Spec>> {
    let Some(items) = parameters.as_array() else {
        return fail("parameters must be an array of named bounds");
    };
    if !(1..=16).contains(&items.len()) {
        return fail("parameters must contain 1..16 named bounds");
    }
    let mut specs: Vec<Spec> = Vec::new();
    let mut keys: Vec<Vec<Value>> = Vec::new();
    for item in items {
        let Some(object) = item.as_object() else {
            return fail("parameter names and specifications are required");
        };
        let name = match object.get("name").and_then(Value::as_str) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => return fail("parameter names and specifications are required"),
        };
        if specs.iter().any(|s| s.name == name) {
            return fail(format!("duplicate parameter name {name}"));
        }
        if object
            .keys()
            .any(|k| !["name", "initial", "lower", "upper", "unit", "path"].contains(&k.as_str()))
        {
            return fail(format!("unknown parameter fields for {name}"));
        }
        let initial = finite(object.get("initial"), &format!("{name}.initial"))?;
        let lower = finite(object.get("lower"), &format!("{name}.lower"))?;
        let upper = finite(object.get("upper"), &format!("{name}.upper"))?;
        if !(lower < upper) || !(lower <= initial && initial <= upper) {
            return fail(format!("invalid initial value or bounds for {name}"));
        }
        let unit = match object.get("unit").and_then(Value::as_str) {
            Some(u) if !u.is_empty() => u.to_string(),
            _ => return fail(format!("{name} requires a declared unit")),
        };
        let path = match object.get("path").filter(|v| !v.is_null()) {
            None => None,
            Some(path) => {
                let Some(path) = path.as_array() else {
                    return fail("event parameter paths must select an administration quantity");
                };
                if path.len() < 4
                    || path[0] != json!("regimen")
                    || path[1] != json!("administrations")
                {
                    return fail("event parameter paths must select an administration quantity");
                }
                if path[2].as_u64().is_none() {
                    return fail("event parameter path requires an administration index");
                }
                let tail: Vec<&str> = path[3..].iter().filter_map(Value::as_str).collect();
                let allowed: [&[&str]; 4] = [
                    &["time"],
                    &["lag"],
                    &["delivery", "span", "duration"],
                    &["repeat", "interval"],
                ];
                if tail.len() != path.len() - 3 || !allowed.iter().any(|a| *a == tail.as_slice()) {
                    return fail(
                        "only administration time, lag, duration or repeat interval may be fitted",
                    );
                }
                Some(path.clone())
            }
        };
        let key = match &path {
            Some(p) => p.clone(),
            None => vec![json!("parameters"), json!(name)],
        };
        if keys.contains(&key) {
            return fail("parameter paths must be distinct");
        }
        keys.push(key);
        specs.push(Spec {
            name,
            initial,
            lower,
            upper,
            unit,
            path,
        });
    }
    Ok(specs)
}

/// Bind quantities into a copy of a run request.
pub fn bind(run: &Value, items: &[(&Spec, f64)]) -> Result<Value> {
    let mut result = run.clone();
    if !result.is_object() {
        return fail("run must be a qspquest.run/v0.1 object");
    }
    for (spec, value) in items {
        if !value.is_finite() {
            return fail(format!("{} must be a finite number", spec.name));
        }
        let quantity = json!({"value": value, "unit": spec.unit});
        match &spec.path {
            None => {
                let parameters = result
                    .as_object_mut()
                    .expect("object")
                    .entry("parameters")
                    .or_insert_with(|| json!({}));
                match parameters.as_object_mut() {
                    Some(map) => {
                        map.insert(spec.name.clone(), quantity);
                    }
                    None => return fail("run parameters must be an object"),
                }
            }
            Some(path) => {
                let missing = || {
                    AnalysisError::new(
                        "invalid_input",
                        format!("event path for {} does not exist", spec.name),
                    )
                };
                let mut parent = &mut result;
                for component in &path[..path.len() - 1] {
                    parent = match component {
                        Value::String(key) => parent.get_mut(key.as_str()).ok_or_else(missing)?,
                        Value::Number(n) => parent
                            .get_mut(n.as_u64().unwrap_or(u64::MAX) as usize)
                            .ok_or_else(missing)?,
                        _ => return Err(missing()),
                    };
                }
                let last = path[path.len() - 1].as_str().ok_or_else(missing)?;
                match parent.as_object_mut() {
                    Some(map) => {
                        map.insert(last.to_string(), quantity);
                    }
                    None => return Err(missing()),
                }
            }
        }
    }
    Ok(result)
}

/// One aligned observation row.
#[derive(Clone, Debug)]
pub struct Observed {
    pub y: Vec<f64>,
    pub prediction: Vec<f64>,
    pub censored: Vec<bool>,
}

pub fn observations(result: &RunOutput, rows: &Value) -> Result<Observed> {
    let Some(rows) = rows.as_array() else {
        return fail("observations must contain 1..10000 rows");
    };
    if !(1..=10000).contains(&rows.len()) {
        return fail("observations must contain 1..10000 rows");
    }
    let mut observed = Observed {
        y: Vec::with_capacity(rows.len()),
        prediction: Vec::with_capacity(rows.len()),
        censored: Vec::with_capacity(rows.len()),
    };
    let mut keys = std::collections::BTreeSet::new();
    for observation in rows {
        let last = result.times.len().saturating_sub(1) as u64;
        let row = integer(observation.get("row"), "observation.row", 0, last)? as usize;
        let output = observation
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or("");
        let Some((values, unit)) = result.column(output) else {
            return fail(format!("unknown observation output {output}"));
        };
        if !keys.insert((row, output.to_string())) {
            return fail("duplicate observation row and output");
        }
        let value = observation.get("value");
        if value.and_then(|v| v.get("unit")).and_then(Value::as_str) != Some(unit) {
            return fail("observation units must equal the declared output units");
        }
        observed.y.push(finite(
            value.and_then(|v| v.get("value")),
            "observation.value",
        )?);
        observed.prediction.push(values[row]);
        let censoring = observation.get("censoring").filter(|v| !v.is_null());
        match censoring {
            None => observed.censored.push(false),
            Some(Value::String(s)) if s == "left" => observed.censored.push(true),
            Some(_) => return fail("only left censoring is supported"),
        }
    }
    Ok(observed)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sd {
    Fixed(f64),
    Fitted {
        initial: f64,
        lower: f64,
        upper: f64,
    },
}

impl Sd {
    pub fn fitted(&self) -> bool {
        matches!(self, Sd::Fitted { .. })
    }
    pub fn start(&self) -> f64 {
        match *self {
            Sd::Fixed(v) => v,
            Sd::Fitted { initial, .. } => initial,
        }
    }
    pub fn bounds(&self) -> (f64, f64) {
        match *self {
            Sd::Fixed(v) => (v, v),
            Sd::Fitted { lower, upper, .. } => (lower, upper),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ErrorSpec {
    pub lognormal: bool,
    pub additive: Sd,
    pub proportional: Sd,
}

/// Residual SDs at one evaluation point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErrorValues {
    pub lognormal: bool,
    pub additive: f64,
    pub proportional: f64,
}

impl ErrorValues {
    pub fn to_value(&self) -> Value {
        json!({
            "model": if self.lognormal { "lognormal" } else { "normal" },
            "additive": self.additive,
            "proportional": self.proportional,
        })
    }
    pub fn from_value(value: &Value) -> Result<Self> {
        Ok(Self {
            lognormal: value.get("model").and_then(Value::as_str) == Some("lognormal"),
            additive: finite(value.get("additive"), "error.additive")?,
            proportional: finite(value.get("proportional"), "error.proportional")?,
        })
    }
    pub fn get(&self, key: &str) -> f64 {
        if key == "additive" {
            self.additive
        } else {
            self.proportional
        }
    }
    pub fn set(&mut self, key: &str, value: f64) {
        if key == "additive" {
            self.additive = value
        } else {
            self.proportional = value
        }
    }
}

impl ErrorSpec {
    pub fn parse(error: &Value) -> Result<Self> {
        let Some(object) = error.as_object() else {
            return fail("error must specify model, additive and/or proportional SD");
        };
        if object
            .keys()
            .any(|k| !["model", "additive", "proportional"].contains(&k.as_str()))
        {
            return fail("error must specify model, additive and/or proportional SD");
        }
        let lognormal = match object.get("model") {
            None => false,
            Some(Value::String(s)) if s == "normal" => false,
            Some(Value::String(s)) if s == "lognormal" => true,
            Some(_) => return fail("error.model must be normal or lognormal"),
        };
        let sd = |name: &str| -> Result<Sd> {
            match object.get(name) {
                None => Ok(Sd::Fixed(0.0)),
                Some(Value::Object(fit)) => {
                    let keys: Vec<&str> = fit.keys().map(String::as_str).collect();
                    let mut sorted = keys.clone();
                    sorted.sort();
                    if sorted != ["initial", "lower", "upper"] {
                        return fail(format!("fitted {name} requires initial, lower and upper"));
                    }
                    let initial = finite(fit.get("initial"), &format!("error.{name}.initial"))?;
                    let lower = finite(fit.get("lower"), &format!("error.{name}.lower"))?;
                    let upper = finite(fit.get("upper"), &format!("error.{name}.upper"))?;
                    if !(0.0 < lower && lower <= initial && initial <= upper) || lower == upper {
                        return fail(
                            "fitted SD bounds must be strictly positive with nonzero width",
                        );
                    }
                    Ok(Sd::Fitted {
                        initial,
                        lower,
                        upper,
                    })
                }
                value => {
                    let v = finite(value, &format!("error.{name}"))?;
                    if v < 0.0 {
                        return fail("residual SD cannot be negative");
                    }
                    Ok(Sd::Fixed(v))
                }
            }
        };
        let spec = Self {
            lognormal,
            additive: sd("additive")?,
            proportional: sd("proportional")?,
        };
        if lognormal && spec.additive != Sd::Fixed(0.0) {
            return fail("lognormal error requires zero additive SD");
        }
        if spec.additive == Sd::Fixed(0.0) && spec.proportional == Sd::Fixed(0.0) {
            return fail("at least one residual SD must be positive");
        }
        Ok(spec)
    }
    pub fn initial(&self) -> ErrorValues {
        ErrorValues {
            lognormal: self.lognormal,
            additive: self.additive.start(),
            proportional: self.proportional.start(),
        }
    }
    /// Fitted residual-error keys in canonical order.
    pub fn fitted_keys(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        if self.additive.fitted() {
            keys.push("additive");
        }
        if self.proportional.fitted() {
            keys.push("proportional");
        }
        keys
    }
    pub fn sd(&self, key: &str) -> Sd {
        if key == "additive" {
            self.additive
        } else {
            self.proportional
        }
    }
}

pub struct Likelihood {
    pub value: f64,
    pub residual: Vec<f64>,
    pub sd: Vec<f64>,
}

impl Likelihood {
    /// NLL derivatives with respect to log additive and log proportional SD.
    pub fn log_sd_score(
        &self,
        prediction: &[f64],
        censored: &[bool],
        error: &ErrorValues,
    ) -> [f64; 2] {
        let mut log_sd_score = [0.0; 2];
        for i in 0..self.residual.len() {
            let z = self.residual[i] / self.sd[i];
            let score = if censored[i] {
                if z == f64::INFINITY {
                    // The left-censored NLL and its score both tend to zero.
                    0.0
                } else {
                    z * super::stats::inverse_mills_lower(z)
                }
            } else {
                1.0 - z * z
            };
            let shares = if error.lognormal {
                [0.0, 1.0]
            } else {
                [
                    (error.additive / self.sd[i]).powi(2),
                    (error.proportional * prediction[i] / self.sd[i]).powi(2),
                ]
            };
            for k in 0..2 {
                log_sd_score[k] += score * shares[k];
            }
        }
        log_sd_score
    }
}

pub fn likelihood(
    y: &[f64],
    prediction: &[f64],
    censored: &[bool],
    error: &ErrorValues,
) -> Result<Likelihood> {
    if prediction.iter().any(|p| !p.is_finite()) {
        return fail_code("prediction must be finite", "domain");
    }
    let n = y.len();
    let (residual, sd, jacobian): (Vec<f64>, Vec<f64>, Vec<f64>) = if error.lognormal {
        if y.iter().any(|v| *v <= 0.0) || prediction.iter().any(|v| *v <= 0.0) {
            return fail_code(
                "lognormal observations, limits and predictions must be positive",
                "domain",
            );
        }
        (
            y.iter()
                .zip(prediction)
                .map(|(a, b)| a.ln() - b.ln())
                .collect(),
            vec![error.proportional; n],
            y.iter().map(|v| v.ln()).collect(),
        )
    } else {
        (
            y.iter().zip(prediction).map(|(a, b)| a - b).collect(),
            prediction
                .iter()
                .map(|p| (error.additive.powi(2) + (error.proportional * p).powi(2)).sqrt())
                .collect(),
            vec![0.0; n],
        )
    };
    if sd.iter().any(|s| !(*s > 0.0) || !s.is_finite()) {
        return fail_code(
            "residual variance must be positive at every observation",
            "domain",
        );
    }
    let mut total = 0.0;
    for i in 0..n {
        let z = residual[i] / sd[i];
        let term = if censored[i] {
            -super::stats::log_ndtr(z)
        } else {
            0.5 * z * z + sd[i].ln() + 0.5 * (2.0 * std::f64::consts::PI).ln() + jacobian[i]
        };
        if !term.is_finite() {
            return fail_code("likelihood is not finite", "domain");
        }
        total += term;
    }
    Ok(Likelihood {
        value: total,
        residual,
        sd,
    })
}

pub fn scalar_parameters(specs: &[Spec], values: &[f64]) -> Value {
    let mut map = Map::new();
    for (spec, value) in specs.iter().zip(values) {
        map.insert(
            spec.name.clone(),
            json!({"value": value, "unit": spec.unit}),
        );
    }
    Value::Object(map)
}

/// Request identity for an analysis. The engine version and analysis version
/// identify the implementation; the native identity is added by callers.
pub fn identity(model: &CompiledDocument, inputs: &Value) -> Result<Value> {
    let hash = qspquest_core::identity::canonical_hash(inputs)
        .map_err(|e| AnalysisError::new("invalid_input", e.message))?;
    Ok(json!({
        "model_content_hash": model.model_content_hash(),
        "analysis_request_hash": hash,
        "analysis_version": super::ANALYSIS_VERSION,
        "engine_version": env!("CARGO_PKG_VERSION"),
        "compiler_source_sha256": env!("QSPQUEST_SOURCE_SHA256"),
        "backend_version": crate::DIFFSOL_BACKEND_VERSION,
    }))
}

pub fn near_bound(value: f64, lower: f64, upper: f64) -> bool {
    (value - lower).min(upper - value) <= 1e-6 * (upper - lower)
}

/// Read a list of finite numbers.
pub fn numbers(value: &Value, name: &str) -> Result<Vec<f64>> {
    let Some(items) = value.as_array() else {
        return fail(format!("{name} must be an array of finite numbers"));
    };
    items.iter().map(|v| finite(Some(v), name)).collect()
}

/// Read a square matrix of finite numbers.
pub fn matrix(value: &Value, name: &str) -> Result<Vec<Vec<f64>>> {
    let Some(rows) = value.as_array() else {
        return fail(format!("{name} must be a matrix of finite numbers"));
    };
    rows.iter().map(|row| numbers(row, name)).collect()
}

/// Map positional quantities `[{name, value, unit}]` to an ordered list.
pub fn named_quantities(value: &Value, name: &str) -> Result<Vec<(String, f64, String)>> {
    let Some(items) = value.as_array() else {
        return fail(format!("{name} must be an array of named quantities"));
    };
    let mut result: Vec<(String, f64, String)> = Vec::new();
    for item in items {
        let key = item.get("name").and_then(Value::as_str).unwrap_or("");
        let unit = item.get("unit").and_then(Value::as_str).unwrap_or("");
        if key.is_empty() || unit.is_empty() {
            return fail(format!("{name} entries require name, value and unit"));
        }
        if result.iter().any(|(k, _, _)| k == key) {
            return fail(format!("duplicate {name} entry {key}"));
        }
        result.push((
            key.to_string(),
            finite(item.get("value"), key)?,
            unit.to_string(),
        ));
    }
    if result.is_empty() {
        return fail(format!("{name} must not be empty"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_and_censored_likelihood_matches_the_independent_formula() {
        let (y, p) = ([1.0, 0.2], [1.2, 0.7]);
        let e = ErrorValues {
            lognormal: false,
            additive: 0.1,
            proportional: 0.2,
        };
        let sd: Vec<f64> = p
            .iter()
            .map(|p: &f64| (0.01 + (0.2 * p).powi(2)).sqrt())
            .collect();
        let z0 = (y[0] - p[0]) / sd[0];
        let z1 = (y[1] - p[1]) / sd[1];
        // log Phi(z1) by direct integration-free evaluation of erfc.
        let log_phi = (0.5 * libm::erfc(-z1 / std::f64::consts::SQRT_2)).ln();
        let expected =
            0.5 * z0 * z0 + sd[0].ln() + 0.5 * (2.0 * std::f64::consts::PI).ln() - log_phi;
        let value = likelihood(&y, &p, &[false, true], &e).unwrap().value;
        assert!((value - expected).abs() < 1e-12);
        // Deep censoring stays finite.
        let deep = ErrorValues {
            lognormal: false,
            additive: 0.1,
            proportional: 0.0,
        };
        assert!(likelihood(&[-50.0], &[1.0], &[true], &deep)
            .unwrap()
            .value
            .is_finite());
    }

    #[test]
    fn lognormal_likelihood_includes_the_jacobian() {
        let e = ErrorValues {
            lognormal: true,
            additive: 0.0,
            proportional: 0.3,
        };
        let value = likelihood(&[2.0], &[1.5], &[false], &e).unwrap().value;
        let z = (2.0f64.ln() - 1.5f64.ln()) / 0.3;
        let expected =
            0.5 * z * z + 0.3f64.ln() + 0.5 * (2.0 * std::f64::consts::PI).ln() + 2.0f64.ln();
        assert!((value - expected).abs() < 1e-14);
    }

    #[test]
    fn residual_log_sd_scores_match_likelihood_differences() {
        for (additive, proportional, lognormal) in [
            (0.3, 0.0, false),
            (0.0, 0.2, false),
            (0.3, 0.2, false),
            (0.0, 0.2, true),
        ] {
            let error = ErrorValues {
                additive,
                proportional,
                lognormal,
            };
            for censored in [[false, false], [false, true]] {
                let evaluated = likelihood(&[2.0, 0.2], &[1.5, 0.7], &censored, &error).unwrap();
                let score = evaluated.log_sd_score(&[1.5, 0.7], &censored, &error);
                for (k, key) in ["additive", "proportional"].iter().enumerate() {
                    if error.get(key) == 0.0 {
                        assert_eq!(score[k], 0.0);
                        continue;
                    }
                    let step = 1e-5_f64;
                    let mut upper = error;
                    let mut lower = error;
                    upper.set(key, error.get(key) * step.exp());
                    lower.set(key, error.get(key) * (-step).exp());
                    let difference = (likelihood(&[2.0, 0.2], &[1.5, 0.7], &censored, &upper)
                        .unwrap()
                        .value
                        - likelihood(&[2.0, 0.2], &[1.5, 0.7], &censored, &lower)
                            .unwrap()
                            .value)
                        / (2.0 * step);
                    assert!(
                        (score[k] - difference).abs() < 1e-7 * difference.abs().max(1.0),
                        "{error:?} censored={censored:?} {key}: score={} difference={difference}",
                        score[k]
                    );
                }
            }
        }
        let error = ErrorValues {
            additive: 0.3,
            proportional: 0.0,
            lognormal: false,
        };
        let tail = likelihood(&[f64::MAX], &[0.0], &[true], &error).unwrap();
        assert_eq!(tail.value, 0.0);
        assert_eq!(tail.log_sd_score(&[0.0], &[true], &error), [0.0, 0.0]);
    }

    #[test]
    fn concurrent_callers_cannot_overspend_the_budget() {
        let model = crate::language::parse(
            "model decay\ntime_unit = h\nparameters\n k = 0.1 [1/h] log\nstates\n a = 1 [mg]\ndynamics\n d/dt(a) = -k * a\noutputs\n y = a [mg]\nend\n",
        )
        .unwrap()
        .compile()
        .unwrap();
        let budget = Budget::new(9, None).unwrap();
        let request = json!({"schema": "qspquest.run/v0.1", "solver": "diffsol_bdf",
            "budgets": {"solver_callbacks": 100000, "events": 10, "output_values": 100},
            "regimen": {"end": {"value": 1.0, "unit": "h"}, "samples": [],
                "observations": [{"time": {"value": 1.0, "unit": "h"}, "side": "post"}],
                "administrations": [], "rtol": 1e-8, "atol": 1e-10}});
        let successes = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..8 {
                        match budget.run(&model, &request) {
                            Ok(_) => {
                                successes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            }
                            Err(error) => assert_eq!(error.code, "budget"),
                        }
                    }
                });
            }
        });
        assert_eq!(successes.into_inner(), 9);
        assert_eq!(budget.used(), 9);
    }
}

//! Typed admission and summaries of an already solved trajectory.
// Window AUCs use the native analysis trapezoid, so exposure_summary and these
// readouts integrate a sampled trajectory identically. Threshold durations
// differ on purpose: time_above and time_below count strictly above or below
// the threshold, so a segment lying exactly on it counts toward neither and
// the two durations partition the window except for that plateau. The native
// exposure_summary counts a value equal to the threshold as above it. Window
// readouts also report first crossings, refuse overflowing intermediate
// coordinates, and start and end on the post and pre rows of a window's
// bounding dose events; exposure_summary always spans the complete grid.
use qspquest_core::regimen::ObservationSide;
use qspquest_modeling::analysis::sampled_auc;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

use super::usability::READOUT_POINTS;

// Result envelopes can contain transport metadata. Only these numerical fields
// enter the summary; native compilation and execution remain in the engine.
struct Trajectory {
    times: Vec<f64>,
    time_unit: String,
    sides: Option<Vec<ObservationSide>>,
    outputs: Vec<Column>,
}

struct Column {
    name: String,
    values: Vec<f64>,
    unit: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Auc,
    Min,
    Max,
    Nca,
    TimeBelow,
    TimeAbove,
    FirstCrossing,
    Trough,
    Value,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Auc => "auc",
            Self::Min => "min",
            Self::Max => "max",
            Self::Nca => "nca",
            Self::TimeBelow => "time_below",
            Self::TimeAbove => "time_above",
            Self::FirstCrossing => "first_crossing",
            Self::Trough => "trough",
            Self::Value => "value",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Coordinate {
    value: f64,
    // Units are already normalized before simulation. Supplied-point profiles
    // may use coordinates without a unit, as in the public summarize tool.
    unit: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Up,
    Down,
    Any,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    kind: Kind,
    output: String,
    at: Option<Coordinate>,
    from: Option<Coordinate>,
    to: Option<Coordinate>,
    side: Option<ObservationSide>,
    threshold: Option<f64>,
    direction: Option<Direction>,
    /// A reported value to compare with: {value, unit?, relative_tolerance?, absolute_tolerance?, metric?}.
    reported: Option<Value>,
    /// nca only: single-dose analysis from the window start (iv_bolus or extravascular).
    route: Option<String>,
    /// nca only: dose amount {value, unit} for clearance and volume.
    dose: Option<Value>,
    auc_method: Option<String>,
    // Friendly aliases are retained for the request receipt after normalization.
    // Canonical at/from/to fields alone enter the calculation.
    #[serde(rename = "window")]
    _window: Option<Value>,
    #[serde(rename = "time")]
    _time: Option<Value>,
    #[serde(rename = "interval")]
    _interval: Option<Value>,
    #[serde(rename = "start")]
    _start: Option<Value>,
    #[serde(rename = "end")]
    _end: Option<Value>,
    #[serde(rename = "stop")]
    _stop: Option<Value>,
}

/// Compare a computed readout with a reported value, converting the reported unit to the
/// readout's unit. Tolerance: |computed - reported| <= absolute + relative * |reported|.
fn apply_reported(row: &mut Value, reported: Option<&Value>, field: &str) -> Result<(), String> {
    let Some(reported) = reported.filter(|r| !r.is_null()) else {
        return Ok(());
    };
    let path = format!("{field}.reported");
    let object = reported.as_object().ok_or_else(|| format!("{path} must be an object {{value, unit?, relative_tolerance?, absolute_tolerance?, metric?}}"))?;
    if let Some(key) = object.keys().find(|k| {
        ![
            "value",
            "unit",
            "relative_tolerance",
            "absolute_tolerance",
            "metric",
        ]
        .contains(&k.as_str())
    }) {
        return Err(format!("{path}.{key} is unknown; use value, unit, relative_tolerance, absolute_tolerance or metric"));
    }
    let value = reported["value"]
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{path}.value must be a finite number"))?;
    let relative = reported
        .get("relative_tolerance")
        .and_then(Value::as_f64)
        .unwrap_or(0.05);
    let absolute = reported
        .get("absolute_tolerance")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if !(relative >= 0.0 && absolute >= 0.0 && relative.is_finite() && absolute.is_finite()) {
        return Err(format!("{path} tolerances must be finite and nonnegative"));
    }
    let (computed, unit) = if row["kind"] == "nca" {
        let metric = reported["metric"].as_str().ok_or_else(|| format!("{path}.metric is required for nca: cmax, tmax, cmin, tmin, auc, cavg or terminal_half_life"))?;
        let unit = row["units"][metric]
            .as_str()
            .ok_or_else(|| format!("{path}.metric '{metric}' is not an nca metric"))?
            .to_owned();
        (row["value"][metric].as_f64(), unit)
    } else if row["kind"] == "first_crossing" {
        (
            row["value"]["time"].as_f64(),
            row["unit"].as_str().unwrap_or("1").to_owned(),
        )
    } else {
        (
            row["value"].as_f64(),
            row["unit"].as_str().unwrap_or("1").to_owned(),
        )
    };
    let Some(computed) = computed else {
        row["comparison"] = json!({"reported":reported,"within_tolerance":Value::Null,"note":"The readout has no value to compare (for example no crossing in the window)."});
        return Ok(());
    };
    let reported_unit = reported["unit"].as_str().unwrap_or(&unit).to_owned();
    let in_output_unit = if reported_unit == unit {
        value
    } else {
        super::usability::convert(value, &reported_unit, &unit, &format!("{path}"))?
    };
    let difference = computed - in_output_unit;
    let within = difference.abs() <= absolute + relative * in_output_unit.abs();
    row["comparison"] = json!({
        "reported": {"value": value, "unit": reported_unit},
        "reported_in_output_unit": in_output_unit,
        "computed": computed,
        "unit": unit,
        "absolute_difference": difference,
        "relative_error": if in_output_unit != 0.0 { json!(difference / in_output_unit.abs()) } else { Value::Null },
        "tolerance": {"relative": relative, "absolute": absolute},
        "within_tolerance": within
    });
    Ok(())
}

fn finite(value: f64, field: &str) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!(
            "{field} must be finite; the readout was not returned"
        ))
    }
}

fn decode<T: DeserializeOwned>(value: &Value, field: &str) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|e| format!("{field}: {e}"))
}

impl Spec {
    fn validate(&self, field: &str) -> Result<(), String> {
        if self.output.is_empty() {
            return Err(format!("{field}.output must be nonempty"));
        }
        for (name, coordinate) in [("at", &self.at), ("from", &self.from), ("to", &self.to)] {
            if let Some(coordinate) = coordinate {
                finite(coordinate.value, &format!("{field}.{name}.value"))?;
                if coordinate.unit.as_ref().is_some_and(|unit| unit.is_empty()) {
                    return Err(format!(
                        "{field}.{name}.unit must be nonempty when supplied"
                    ));
                }
            }
        }
        let point = matches!(self.kind, Kind::Trough | Kind::Value);
        if point {
            if self.at.is_none() {
                return Err(format!("{field}.at is required"));
            }
            if self.from.is_some() || self.to.is_some() {
                return Err(format!("{field}.from/to applies only to window readouts"));
            }
            if matches!(self.kind, Kind::Trough) && matches!(self.side, Some(ObservationSide::Post))
            {
                return Err(format!("{field}.side must be pre for trough"));
            }
        } else {
            if self.at.is_some() || self.side.is_some() {
                return Err(format!("{field}.at/side applies only to value or trough"));
            }
            let (Some(from), Some(to)) = (&self.from, &self.to) else {
                return Err(format!("{field}.from/to is required"));
            };
            if to.value <= from.value {
                return Err(format!("{field}.to must be later than from"));
            }
        }
        let threshold_kind = matches!(
            self.kind,
            Kind::TimeBelow | Kind::TimeAbove | Kind::FirstCrossing
        );
        if threshold_kind {
            finite(
                self.threshold
                    .ok_or_else(|| format!("{field}.threshold is required in the output's unit"))?,
                &format!("{field}.threshold"),
            )?;
        } else if self.threshold.is_some() {
            return Err(format!(
                "{field}.threshold applies only to crossing or time-above/below readouts"
            ));
        }
        if self.direction.is_some() && !matches!(self.kind, Kind::FirstCrossing) {
            return Err(format!("{field}.direction applies only to first_crossing"));
        }
        Ok(())
    }
}

impl Trajectory {
    fn parse(result: &Value) -> Result<Self, String> {
        result
            .as_object()
            .ok_or("result must be a trajectory object")?;
        let outputs = result["outputs"]
            .as_array()
            .ok_or("result.outputs must be an array")?;
        let outputs = outputs
            .iter()
            .enumerate()
            .map(|(index, raw)| {
                raw.as_object()
                    .ok_or_else(|| format!("result.outputs[{index}] must be an object"))?;
                Ok(Column {
                    name: decode(&raw["name"], &format!("result.outputs[{index}].name"))?,
                    values: decode(&raw["values"], &format!("result.outputs[{index}].values"))?,
                    unit: raw["unit"].as_str().map(str::to_owned),
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            times: decode(&result["times"], "result.times")?,
            time_unit: result
                .get("time_unit")
                .map(|v| decode(v, "result.time_unit"))
                .transpose()?
                .unwrap_or_default(),
            sides: result
                .get("sides")
                .map(|v| decode(v, "result.sides"))
                .transpose()?,
            outputs,
        })
    }

    fn validate(&self) -> Result<(), String> {
        if self.times.is_empty() {
            return Err("result.times must be nonempty".into());
        }
        for (index, time) in self.times.iter().enumerate() {
            finite(*time, &format!("result.times[{index}]"))?;
        }
        if self.times.windows(2).any(|w| w[1] < w[0]) {
            return Err("result.times must be nondecreasing".into());
        }
        if let Some(sides) = &self.sides {
            if sides.len() != self.times.len() {
                return Err("result.sides length must equal result.times length".into());
            }
            for index in 1..self.times.len() {
                if self.times[index] == self.times[index - 1]
                    && !(matches!(sides[index - 1], ObservationSide::Pre)
                        && matches!(sides[index], ObservationSide::Post))
                {
                    return Err(format!(
                        "result.sides[{index}] must follow a pre row with post at a repeated time"
                    ));
                }
            }
        }
        let mut names = BTreeSet::new();
        for (index, column) in self.outputs.iter().enumerate() {
            if column.name.is_empty() || !names.insert(&column.name) {
                return Err(format!(
                    "result.outputs[{index}].name must be nonempty and unique"
                ));
            }
            if column.values.len() != self.times.len() {
                return Err(format!(
                    "result.outputs[{index}].values length must equal result.times length"
                ));
            }
            for (row, value) in column.values.iter().enumerate() {
                finite(*value, &format!("result.outputs[{index}].values[{row}]"))?;
            }
        }
        Ok(())
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// Starts on the post-event row and ends on the pre-event row.
/// Model times of the bolus administrations in a native run request (lag and repeats
/// included), for readouts that must know whether a window opens at a bolus.
pub fn bolus_times(request: &Value, time_unit: &str) -> Vec<f64> {
    let to_model = |q: &Value| -> Option<f64> {
        let value = q["value"].as_f64()?;
        let unit = q["unit"].as_str().unwrap_or(time_unit);
        super::usability::convert(
            value,
            &super::usability::normalize_unit(unit),
            time_unit,
            "time",
        )
        .ok()
    };
    let mut times = Vec::new();
    for a in request["regimen"]["administrations"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if a["delivery"]["kind"] != "bolus" {
            continue;
        }
        let Some(start) = to_model(&a["time"]) else {
            continue;
        };
        let lag = if a["lag"].is_null() {
            0.0
        } else {
            to_model(&a["lag"]).unwrap_or(0.0)
        };
        let interval = to_model(&a["repeat"]["interval"]).unwrap_or(0.0);
        let additional = a["repeat"]["additional"].as_u64().unwrap_or(0).min(10_000);
        times.extend((0..=additional).map(|k| start + lag + k as f64 * interval));
    }
    times
}

pub(super) fn window(t: &[f64], from: f64, to: f64) -> Option<(usize, usize)> {
    let first = (0..t.len()).rev().find(|i| t[*i] == from)?;
    let last = (0..t.len()).find(|i| t[*i] == to)?;
    (last > first).then_some((first, last))
}

/// Summarize normalized readouts without mutating the trajectory or request.
pub fn calculate(result: &Value, specs: &Value, native: &Value) -> Result<Value, String> {
    let trajectory = Trajectory::parse(result)?;
    trajectory.validate()?;
    let raw_specs = specs.as_array().ok_or("readouts must be an array")?;
    // Admit the complete collection before calculating any row.
    let specs: Vec<Spec> = raw_specs
        .iter()
        .enumerate()
        .map(|(index, raw)| {
            let field = format!("readouts[{index}]");
            let spec: Spec =
                serde_json::from_value(raw.clone()).map_err(|e| format!("{field}: {e}"))?;
            spec.validate(&field)?;
            Ok(spec)
        })
        .collect::<Result<_, String>>()?;
    let t = &trajectory.times;
    let tu = &trajectory.time_unit;
    let mut out = Vec::new();
    for (index, spec) in specs.iter().enumerate() {
        let field = format!("readouts[{index}]");
        let kind = spec.kind.name();
        let name = &spec.output;
        let entry = &native["outputs"][name];
        let wanted = entry
            .as_str()
            .or_else(|| entry["native_name"].as_str())
            .unwrap_or(name);
        let column = trajectory
            .outputs
            .iter()
            .find(|column| column.name == wanted)
            .ok_or_else(|| {
                format!(
                    "{field}.output '{name}' is not among the simulated outputs; add it to outputs"
                )
            })?;
        let y = &column.values;
        // Every number is returned with its unit: output values in the output's unit, times in
        // the model time unit and integrals in their product.
        let ou = column.unit.clone().unwrap_or_else(|| "1".into());
        let auc_unit = format!("{ou}.{tu}");
        let mut row = json!({"kind":kind,"output":name,"unit":ou});
        if matches!(spec.kind, Kind::Trough | Kind::Value) {
            let at = spec
                .at
                .as_ref()
                .expect("point time validated before calculation")
                .value;
            let first = t
                .iter()
                .position(|time| *time == at)
                .ok_or_else(|| format!("{field}.at is outside the simulation"))?;
            let last = t
                .iter()
                .rposition(|time| *time == at)
                .expect("matching first time exists");
            let pre = matches!(spec.kind, Kind::Trough)
                || matches!(spec.side, Some(ObservationSide::Pre));
            row["at"] = json!(at);
            row["value"] = json!(if pre { y[first] } else { y[last] });
            row["side"] = json!(if pre { "pre" } else { "post" });
            if !near(y[first], y[last]) {
                row["pre"] = json!(y[first]);
                row["post"] = json!(y[last]);
            }
            apply_reported(&mut row, spec.reported.as_ref(), &field)?;
            out.push(row);
            continue;
        }
        let a = spec
            .from
            .as_ref()
            .expect("window validated before calculation")
            .value;
        let b = spec
            .to
            .as_ref()
            .expect("window validated before calculation")
            .value;
        let (i0, i1) = window(t, a, b)
            .ok_or_else(|| format!("{field}.from/to window is outside the simulation"))?;
        finite(b - a, &format!("{field}.from/to span"))?;
        row["from"] = json!(a);
        row["to"] = json!(b);
        row["time_unit"] = json!(tu);
        match kind {
            "auc" => {
                row["value"] = json!(finite(
                    sampled_auc(&t[i0..=i1], &y[i0..=i1]),
                    &format!("{field}.value")
                )?);
                row["unit"] = json!(auc_unit);
            }
            "min" | "max" => {
                let best = (i0..=i1)
                    .reduce(|p, q| {
                        if (kind == "min") == (y[q] < y[p]) && y[q] != y[p] {
                            q
                        } else {
                            p
                        }
                    })
                    .unwrap();
                row["value"] = json!(y[best]);
                row["time"] = json!(t[best]);
            }
            "nca" => {
                let auc = finite(
                    sampled_auc(&t[i0..=i1], &y[i0..=i1]),
                    &format!("{field}.value.auc"),
                )?;
                let hi = (i0..=i1)
                    .reduce(|p, q| if y[q] > y[p] { q } else { p })
                    .unwrap();
                let lo = (i0..=i1)
                    .reduce(|p, q| if y[q] < y[p] { q } else { p })
                    .unwrap();
                // Terminal half-life from a log-linear fit of the last quarter of the window, reported only
                // when the output falls throughout it and stays positive.
                let start = (i1 - (i1 - i0) / 4).min(i1.saturating_sub(2)).max(i0);
                let tail: Vec<usize> = (start..=i1)
                    .filter(|i| *i == start || t[*i] > t[*i - 1])
                    .collect();
                let falling = tail.len() >= 3
                    && tail.iter().all(|i| y[*i] > 0.0)
                    && tail.windows(2).all(|w| y[w[1]] < y[w[0]]);
                let half_life = if falling {
                    let n = tail.len() as f64;
                    let (mx, my) = (
                        tail.iter().map(|i| t[*i]).sum::<f64>() / n,
                        tail.iter().map(|i| y[*i].ln()).sum::<f64>() / n,
                    );
                    finite(mx, &format!("{field}.value.terminal_fit.mean_time"))?;
                    finite(my, &format!("{field}.value.terminal_fit.mean_log_output"))?;
                    let slope = tail
                        .iter()
                        .map(|i| (t[*i] - mx) * (y[*i].ln() - my))
                        .sum::<f64>()
                        / tail.iter().map(|i| (t[*i] - mx).powi(2)).sum::<f64>();
                    finite(slope, &format!("{field}.value.terminal_fit.slope"))?;
                    json!(finite(
                        -std::f64::consts::LN_2 / slope,
                        &format!("{field}.value.terminal_half_life")
                    )?)
                } else {
                    Value::Null
                };
                row["value"] = json!({"cmax":y[hi],"tmax":t[hi],"cmin":y[lo],"tmin":t[lo],"auc":auc,"cavg":finite(auc/(b-a), &format!("{field}.value.cavg"))?,"terminal_half_life":half_life});
                row["units"] = json!({"cmax":ou,"tmax":tu,"cmin":ou,"tmin":tu,"auc":auc_unit,"cavg":ou,"terminal_half_life":tu});
                row.as_object_mut().unwrap().remove("unit");
                if let Some(route) = spec.route.as_deref() {
                    let route = match route {
                        "iv_bolus" => super::nca::Route::IvBolus,
                        "extravascular" => super::nca::Route::Extravascular,
                        _ => {
                            return Err(format!("{field}.route must be iv_bolus or extravascular"))
                        }
                    };
                    let method = match spec.auc_method.as_deref().unwrap_or("linear_up_log_down") {
                        "linear_up_log_down" => super::nca::Method::LinearUpLogDown,
                        "linear" => super::nca::Method::Linear,
                        _ => {
                            return Err(format!(
                                "{field}.auc_method must be linear_up_log_down or linear"
                            ))
                        }
                    };
                    // Times after the window start; at an event time the post-event value is used.
                    let mut times = Vec::new();
                    let mut values = Vec::new();
                    for i in i0..=i1 {
                        let rel = t[i] - a;
                        if times.last().is_some_and(|last: &f64| *last == rel) {
                            *values.last_mut().unwrap() = y[i];
                        } else {
                            times.push(rel);
                            values.push(y[i]);
                        }
                    }
                    if values[0] != 0.0 {
                        // Single-dose NCA needs a drug-free start. For an IV bolus that means a
                        // dose event at the window start (a pre and a post row at that time) with
                        // nothing before it; the post-dose value is then removed and C0
                        // back-extrapolated, as for observed IV data.
                        let scale = a.abs().max(1.0);
                        let event_at_start = (i0 < i1 && t[i0 + 1] == t[i0])
                            || result["_bolus_times"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_f64)
                                .any(|b| (b - a).abs() <= 1e-9 * scale);
                        if route != super::nca::Route::IvBolus || !event_at_start {
                            return Err(format!("{field}: the output is {} at the window start; single-dose NCA needs the window to start at the dose with no drug present{}", values[0],
                                if route == super::nca::Route::IvBolus { " (for iv_bolus, a bolus exactly at the window start)" } else { "" }));
                        }
                        // With a pre row at the start, the value before the bolus must be zero; a
                        // bolus at the simulation start has no pre row (the initial state applies).
                        if i0 < i1 && t[i0 + 1] == t[i0] && y[i0] != 0.0 {
                            return Err(format!("{field}: the output is {} just before the bolus at the window start; earlier doses make single-dose NCA invalid", y[i0]));
                        }
                        times.remove(0);
                        values.remove(0);
                    }
                    let dose_amount = spec.dose.as_ref().and_then(|d| d["value"].as_f64());
                    let dose_unit = spec
                        .dose
                        .as_ref()
                        .and_then(|d| d["unit"].as_str())
                        .map(str::to_owned);
                    let settings = super::nca::Settings {
                        route,
                        method,
                        dose: dose_amount,
                        units: super::nca::Units {
                            time: tu,
                            concentration: &ou,
                            amount: dose_unit.as_deref(),
                        },
                        blq: None,
                        min_points: 3,
                        tolerance: 1e-4,
                        rows: false,
                    };
                    let analysis = super::nca::analyze(&times, &values, None, &settings)
                        .map_err(|e| format!("{field}: {e}"))?;
                    row["single_dose_nca"] = analysis;
                    // One AUC and one half-life per row: the grid summaries give way to the NCA.
                    for key in ["auc", "cavg", "terminal_half_life"] {
                        row["value"].as_object_mut().unwrap().remove(key);
                        row["units"].as_object_mut().unwrap().remove(key);
                    }
                    row["note"] = json!("AUC and half-life are reported once, under single_dose_nca (AUCLST, AUCIFO, LAMZHL). Times are measured from the window start, taken as the dose time; doses inside the window make single-dose parameters such as AUCIFO invalid.");
                } else {
                    row["note"] = json!("terminal_half_life is a log-linear fit over the last quarter of the window (at least three points); null when the output is not falling throughout it.");
                }
            }
            "time_below" | "time_above" | "first_crossing" => {
                let c = spec
                    .threshold
                    .expect("threshold validated before calculation");
                let below = kind != "time_above";
                let inside = |v: f64| if below { v < c } else { v > c };
                let (mut total, mut first) = (0.0, Value::Null);
                let direction = match spec.direction.unwrap_or(Direction::Any) {
                    Direction::Up => "up",
                    Direction::Down => "down",
                    Direction::Any => "any",
                };
                for i in i0 + 1..=i1 {
                    let (t0, t1, y0, y1) = (t[i - 1], t[i], y[i - 1], y[i]);
                    // Preserve the interpolation formula while refusing overflow
                    // in its intermediate coordinates, including dose jumps.
                    for (value, coordinate) in [
                        (t1 - t0, "time_span"),
                        (y0 - c, "left_threshold_distance"),
                        (y1 - c, "right_threshold_distance"),
                        (y1 - y0, "output_delta"),
                    ] {
                        finite(value, &format!("{field}.segment[{i}].{coordinate}"))?;
                    }
                    let crosses = (y0 - c) * (y1 - c) < 0.0 || (y0 == c) != (y1 == c);
                    let rising = y1 > y0;
                    if crosses
                        && first.is_null()
                        && (direction == "any" || (direction == "up") == rising)
                    {
                        let when = if t1 > t0 {
                            t0 + (c - y0) / (y1 - y0) * (t1 - t0)
                        } else {
                            t0
                        };
                        first = json!({"time":finite(when, &format!("{field}.value.time"))?,"direction":if rising{"up"}else{"down"},"at_dose_event":t1<=t0});
                    }
                    if t1 <= t0 {
                        continue;
                    }
                    total += match (inside(y0), inside(y1)) {
                        (true, true) => t1 - t0,
                        (false, false) => 0.0,
                        (true, false) => (c - y0) / (y1 - y0) * (t1 - t0),
                        (false, true) => (y1 - c) / (y1 - y0) * (t1 - t0),
                    };
                }
                row["threshold"] = json!(c);
                row["threshold_unit"] = json!(ou);
                if kind == "first_crossing" {
                    row["value"] = first;
                    row["unit"] = json!(tu);
                } else {
                    row["value"] = json!(finite(total, &format!("{field}.value"))?);
                    row["unit"] = json!(tu);
                }
            }
            _ => unreachable!("point kinds handled before the window calculation"),
        }
        apply_reported(&mut row, spec.reported.as_ref(), &field)?;
        out.push(row);
    }
    Ok(json!({
        "results": out,
        "method": format!("Computed on the solved trajectory sampled at {READOUT_POINTS} even steps per window plus every dose event (pre and post). AUC is trapezoidal on that grid; crossings are interpolated within one step and never across a dose event.")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dose_trajectory() -> Value {
        json!({"time_unit":"d","times":[0.0,1.0,1.0,2.0,3.0],
            "sides":["post","pre","post","post","post"],
            "outputs":[{"name":"c","unit":"mg/L","values":[10.0,10.0,20.0,10.0,0.0]}]})
    }

    fn auc() -> Value {
        json!([{"kind":"auc","output":"c","from":{"value":0.0},"to":{"value":3.0}}])
    }

    #[test]
    fn all_nine_kinds_preserve_wire_rows_and_event_semantics() {
        let result = dose_trajectory();
        let before = result.clone();
        let mut specs = Vec::new();
        for kind in [
            "auc",
            "min",
            "max",
            "nca",
            "time_below",
            "time_above",
            "first_crossing",
        ] {
            let mut spec =
                json!({"kind":kind,"output":"c","from":{"value":0.0},"to":{"value":3.0}});
            if kind.starts_with("time_") {
                spec["threshold"] = json!(5.0);
            }
            if kind == "first_crossing" {
                spec["threshold"] = json!(15.0);
                spec["direction"] = json!("up");
            }
            specs.push(spec);
        }
        specs.push(json!({"kind":"trough","output":"c","at":{"value":1.0}}));
        specs.push(json!({"kind":"value","output":"c","at":{"value":1.0}}));
        let specs = json!(specs);
        let submitted = specs.clone();
        let actual = calculate(&result, &specs, &Value::Null).unwrap();
        let base = |kind: &str, value: Value| json!({"kind":kind,"output":"c","from":0.0,"to":3.0,"time_unit":"d","value":value});
        let mut expected = vec![
            base("auc", json!(30.0)),
            base("min", json!(0.0)),
            base("max", json!(20.0)),
            base(
                "nca",
                json!({"cmax":20.0,"tmax":1.0,"cmin":0.0,"tmin":3.0,"auc":30.0,"cavg":10.0,"terminal_half_life":null}),
            ),
            base("time_below", json!(0.5)),
            base("time_above", json!(2.5)),
            base(
                "first_crossing",
                json!({"time":1.0,"direction":"up","at_dose_event":true}),
            ),
        ];
        expected[0]["unit"] = json!("mg/L.d");
        expected[1]["time"] = json!(3.0);
        expected[1]["unit"] = json!("mg/L");
        expected[2]["time"] = json!(1.0);
        expected[2]["unit"] = json!("mg/L");
        expected[3]["note"] = json!("terminal_half_life is a log-linear fit over the last quarter of the window (at least three points); null when the output is not falling throughout it.");
        expected[3]["units"] = json!({"cmax":"mg/L","tmax":"d","cmin":"mg/L","tmin":"d","auc":"mg/L.d","cavg":"mg/L","terminal_half_life":"d"});
        for row in &mut expected[4..6] {
            row["threshold"] = json!(5.0);
            row["threshold_unit"] = json!("mg/L");
            row["unit"] = json!("d");
        }
        expected[6]["threshold"] = json!(15.0);
        expected[6]["threshold_unit"] = json!("mg/L");
        expected[6]["unit"] = json!("d");
        expected.push(json!({"kind":"trough","output":"c","at":1.0,"value":10.0,"side":"pre","pre":10.0,"post":20.0,"unit":"mg/L"}));
        expected.push(json!({"kind":"value","output":"c","at":1.0,"value":20.0,"side":"post","pre":10.0,"post":20.0,"unit":"mg/L"}));
        assert_eq!(
            actual,
            json!({"results":expected,"method":format!("Computed on the solved trajectory sampled at {READOUT_POINTS} even steps per window plus every dose event (pre and post). AUC is trapezoidal on that grid; crossings are interpolated within one step and never across a dose event.")})
        );
        assert_eq!(result, before);
        assert_eq!(specs, submitted);
    }

    #[test]
    fn malformed_trajectories_refuse_before_indexing() {
        for (key, value, field) in [
            ("times", json!([]), "result.times"),
            ("times", json!([0, 3, 2, 2, 4]), "result.times"),
            ("times", json!([0, null, 1, 2, 3]), "result.times"),
            ("sides", json!(["post"]), "result.sides"),
            (
                "sides",
                json!(["post", "post", "post", "post", "post"]),
                "result.sides[2]",
            ),
            (
                "sides",
                json!(["post", "pre", "left", "post", "post"]),
                "result.sides",
            ),
            (
                "outputs",
                json!([{"name":"c","values":[1]}]),
                "result.outputs[0].values",
            ),
            (
                "outputs",
                json!([{"name":"c","values":[1,2,null,4,5]}]),
                "result.outputs[0].values",
            ),
            (
                "outputs",
                json!([{"name":"c","values":[1,2,3,4,5]},{"name":"c","values":[1,2,3,4,5]}]),
                "result.outputs[1].name",
            ),
        ] {
            let mut result = dose_trajectory();
            result[key] = value;
            let before = result.clone();
            let error = calculate(&result, &auc(), &Value::Null).unwrap_err();
            assert!(error.contains(field), "{error}");
            assert_eq!(result, before);
        }
    }

    #[test]
    fn malformed_readouts_refuse_with_their_index() {
        for spec in [
            json!({"kind":"value","output":"c"}),
            json!({"kind":"trough","output":"c","at":{"value":1},"side":"post"}),
            json!({"kind":"auc","output":"c","from":{"value":3},"to":{"value":0}}),
            json!({"kind":"time_below","output":"c","from":{"value":0},"to":{"value":3}}),
            json!({"kind":"auc","output":"missing","from":{"value":0},"to":{"value":3}}),
            json!({"kind":"auc","output":"c","from":{"value":0},"to":{"value":3},"direction":"up"}),
            json!({"kind":"unknown","output":"c"}),
            json!({"kind":"value","output":"c","at":{"value":1},"side":"left"}),
            json!({"kind":"value","output":"c","at":{"value":1},"extra":1}),
        ] {
            let error = calculate(&dose_trajectory(), &json!([spec]), &Value::Null).unwrap_err();
            assert!(error.contains("readouts[0]"), "{error}");
        }
        assert!(calculate(&dose_trajectory(), &json!({}), &Value::Null)
            .unwrap_err()
            .contains("readouts must be an array"));
    }

    #[test]
    fn reported_values_are_compared_in_the_output_unit() {
        let result = dose_trajectory();
        let specs = json!([
            {"kind":"max","output":"c","from":{"value":0.0},"to":{"value":3.0},"reported":{"value":20500.0,"unit":"ug/L","relative_tolerance":0.05}},
            {"kind":"nca","output":"c","from":{"value":0.0},"to":{"value":3.0},"reported":{"value":40.0,"metric":"auc","relative_tolerance":0.1}},
        ]);
        let actual = calculate(&result, &specs, &Value::Null).unwrap();
        let first = &actual["results"][0]["comparison"];
        assert_eq!(first["reported_in_output_unit"], 20.5);
        assert_eq!(first["within_tolerance"], true);
        let second = &actual["results"][1]["comparison"];
        assert_eq!(second["within_tolerance"], false);
        assert_eq!(second["unit"], "mg/L.d");
        let bad = json!([{"kind":"nca","output":"c","from":{"value":0.0},"to":{"value":3.0},"reported":{"value":1.0}}]);
        assert!(calculate(&result, &bad, &Value::Null)
            .unwrap_err()
            .contains("metric"));
    }

    #[test]
    fn supplied_profiles_and_both_alias_forms_keep_their_contract() {
        let mut result = dose_trajectory();
        result.as_object_mut().unwrap().remove("sides");
        result.as_object_mut().unwrap().remove("time_unit");
        for alias in [json!("c"), json!({"native_name":"c"})] {
            let mut specs = auc();
            specs[0]["output"] = json!("concentration");
            specs[0]["window"] = json!({"from":0,"to":3});
            let actual =
                calculate(&result, &specs, &json!({"outputs":{"concentration":alias}})).unwrap();
            assert_eq!(actual["results"][0]["value"], 30.0);
            assert_eq!(actual["results"][0]["output"], "concentration");
            assert_eq!(actual["results"][0]["unit"], "mg/L.");
        }
    }

    #[test]
    fn overflow_refuses_instead_of_returning_null_or_an_invalid_crossing() {
        let mut result = dose_trajectory();
        result["outputs"][0]["values"] = json!(vec![1e308; 5]);
        assert!(calculate(&result, &auc(), &Value::Null)
            .unwrap_err()
            .contains("readouts[0].value"));
        let result = json!({"times":[0,1],"outputs":[{"name":"c","values":[1e308,-1e308]}]});
        let specs = json!([{"kind":"first_crossing","output":"c","from":{"value":0},"to":{"value":1},"threshold":0}]);
        assert!(calculate(&result, &specs, &Value::Null)
            .unwrap_err()
            .contains("output_delta"));
    }
}

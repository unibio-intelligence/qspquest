//! Sampled-trajectory exposure metrics and paired population regimen comparisons.
use super::common::*;
use super::linalg;
use super::rng::{substream, RANDOM_STREAM};
use super::stats::quantiles;
use crate::document::CompiledDocument;
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// Exposure metrics of one sampled output.
#[derive(Clone, Debug)]
pub struct Exposure {
    pub record: Value,
    pub auc: f64,
    pub cmax: f64,
    pub tmax: f64,
    pub cmin: f64,
    pub endpoint: f64,
    pub above: Option<f64>,
}

pub fn exposure_summary_request(arguments: &Value) -> Result<Value> {
    let args = Arguments::new(arguments, &["result", "output", "threshold"])?;
    let result = RunOutput::from_value(args.required("result")?)?;
    let Some(output) = args.required("output")?.as_str() else {
        return fail("output must be a string");
    };
    let threshold = match args.get("threshold") {
        None => None,
        v => Some(finite(v, "threshold")?),
    };
    Ok(exposure_summary(&result, output, threshold)?.record)
}

/// Trapezoidal area of a sampled trajectory, `sum((t[i+1]-t[i])*(y[i]+y[i+1])/2)`.
///
/// Segments are summed in time order. A zero-width segment, such as the pre
/// and post rows of a dose event at one time, adds nothing, so no trapezoid
/// spans a dose jump. There is no extrapolation. `t` and `y` must have equal
/// lengths; callers check that both are finite and that `t` is nondecreasing.
pub fn sampled_auc(t: &[f64], y: &[f64]) -> f64 {
    debug_assert_eq!(t.len(), y.len());
    let mut area = 0.0;
    for i in 1..t.len().min(y.len()) {
        area += (t[i] - t[i - 1]) * ((y[i - 1] + y[i]) / 2.0);
    }
    area
}

/// Integrate the sampled piecewise-linear trajectory without extrapolation.
/// Duplicate times must be ordered pre then post so no trapezoid spans a
/// dose jump. The result is conditional on the observation grid; it is not
/// terminal-phase NCA.
pub fn exposure_summary(
    result: &RunOutput,
    output: &str,
    threshold: Option<f64>,
) -> Result<Exposure> {
    let Some((y, unit)) = result.column(output) else {
        return fail("unknown exposure output");
    };
    let t = &result.times;
    let sides = &result.sides;
    let n = t.len();
    if n < 2
        || y.len() != n
        || sides.len() != n
        || t.iter().any(|v| !v.is_finite())
        || y.iter().any(|v| !v.is_finite())
        || t.windows(2).any(|w| w[1] < w[0])
        || !(t[n - 1] > t[0])
    {
        return fail("exposure requires a finite ordered trajectory with nonzero duration");
    }
    for i in 0..n - 1 {
        if t[i + 1] - t[i] == 0.0 && (sides[i].as_str(), sides[i + 1].as_str()) != ("pre", "post") {
            return fail("duplicate exposure times must be ordered pre then post");
        }
    }
    let area = sampled_auc(t, y);
    let mut peak = 0;
    for i in 1..n {
        if y[i] > y[peak] {
            peak = i;
        }
    }
    let cmin = y.iter().copied().fold(f64::INFINITY, f64::min);
    let mut above = None;
    let mut above_record = Value::Null;
    if let Some(cutoff) = threshold {
        let mut duration = 0.0;
        for i in 0..n - 1 {
            let dt = t[i + 1] - t[i];
            let (a, b) = (y[i] - cutoff, y[i + 1] - cutoff);
            if a >= 0.0 && b >= 0.0 {
                duration += dt;
            } else if a < 0.0 && b < 0.0 {
                continue;
            } else if a != b {
                let fraction = -a / (b - a);
                duration += dt * if b > a { 1.0 - fraction } else { fraction };
            }
        }
        above = Some(duration);
        above_record = json!({
            "threshold": cutoff,
            "duration": duration,
            "fraction_of_window": duration / (t[n - 1] - t[0]),
        });
    }
    let record = json!({
        "output": output,
        "unit": unit,
        "time_unit": result.time_unit,
        "window": [t[0], t[n - 1]],
        "auc": area,
        "auc_unit": format!("{unit} * {}", result.time_unit),
        "cmax": y[peak],
        "tmax": t[peak],
        "peak_side": sides[peak],
        "cmin": cmin,
        "endpoint": y[n - 1],
        "endpoint_side": sides[n - 1],
        "time_above_threshold": above_record,
        "method": "sampled_piecewise_linear_no_extrapolation",
    });
    Ok(Exposure {
        record,
        auc: area,
        cmax: y[peak],
        tmax: t[peak],
        cmin,
        endpoint: y[n - 1],
        above,
    })
}

/// Validated percentiles: 1..10 increasing values in (0, 1).
pub fn percentiles(value: Option<&Value>) -> Result<Vec<f64>> {
    let p = match value {
        None => vec![0.05, 0.5, 0.95],
        Some(v) => numbers(v, "percentiles").map_err(|_| {
            AnalysisError::new(
                "invalid_input",
                "percentiles must be increasing values in (0, 1)",
            )
        })?,
    };
    if !(1..=10).contains(&p.len())
        || p.iter().any(|v| !(*v > 0.0 && *v < 1.0))
        || p.windows(2).any(|w| w[1] <= w[0])
    {
        return fail("percentiles must be increasing values in (0, 1)");
    }
    Ok(p)
}

/// Fixed population parameters as specs with wide bounds for binding.
pub fn fixed_specs(parameters: &[(String, f64, String)]) -> Result<Vec<Spec>> {
    let items: Vec<Value> = parameters
        .iter()
        .map(|(name, value, unit)| {
            json!({
                "name": name,
                "initial": value,
                "lower": (value * 0.001).max(1e-15),
                "upper": value * 1000.0,
                "unit": unit,
            })
        })
        .collect();
    parameter_specs(&Value::Array(items))
}

/// Random-effect names and the Cholesky factor of their covariance.
pub fn random_effects(
    specs: &[Spec],
    selected: Option<&Value>,
    omega: &Value,
    what: &str,
) -> Result<(Vec<usize>, linalg::Matrix)> {
    let names: Vec<String> = match selected {
        None => specs.iter().map(|s| s.name.clone()).collect(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| v.as_str().map(str::to_string).unwrap_or_default())
            .collect(),
        Some(_) => {
            return fail(format!(
                "{what} random effects must select 1..8 distinct positive parameters"
            ))
        }
    };
    let mut indices = Vec::new();
    for name in &names {
        match specs.iter().position(|s| &s.name == name) {
            Some(i) if !indices.contains(&i) => indices.push(i),
            _ => {
                return fail(format!(
                    "{what} random effects must select 1..8 distinct positive parameters"
                ))
            }
        }
    }
    if !(1..=8).contains(&indices.len()) {
        return fail(format!(
            "{what} random effects must select 1..8 distinct positive parameters"
        ));
    }
    let om = matrix(omega, "omega")?;
    if !linalg::is_symmetric(&om, indices.len(), 1e-12) {
        return fail(format!("{what} Omega must be finite and symmetric"));
    }
    let Some(chol) = linalg::cholesky(&om) else {
        return fail(format!("{what} Omega must be positive definite"));
    };
    Ok((indices, chol))
}

struct Comparison {
    times: Vec<f64>,
    sides: Vec<String>,
    unit: String,
    time_unit: String,
    metrics: Vec<Exposure>,
    trajectories: Vec<Vec<f64>>,
}

/// Arguments accepted by `compare_regimens`.
pub const COMPARISON_ARGUMENTS: [&str; 12] = [
    "regimens",
    "parameters",
    "omega",
    "output",
    "random_effects",
    "subjects",
    "seed",
    "threshold",
    "percentiles",
    "parameter_draws",
    "max_simulations",
    "workers",
];

/// Paired percentiles and differences of prepared regimen comparisons.
fn summarize(
    ordered: &[(String, Comparison)],
    p: &[f64],
    threshold: Option<f64>,
) -> Result<Map<String, Value>> {
    let reference = &ordered[0].1;
    for (_, c) in ordered {
        if (c.unit.as_str(), c.time_unit.as_str())
            != (reference.unit.as_str(), reference.time_unit.as_str())
        {
            return fail("paired regimen metrics require matching output and time units");
        }
    }
    let mut records = Map::new();
    for (label, c) in ordered {
        let metric = |f: &dyn Fn(&Exposure) -> f64| -> Vec<f64> {
            quantiles(&c.metrics.iter().map(f).collect::<Vec<_>>(), p)
        };
        let mut summaries = json!({
            "auc": metric(&|m| m.auc),
            "cmax": metric(&|m| m.cmax),
            "tmax": metric(&|m| m.tmax),
            "cmin": metric(&|m| m.cmin),
            "endpoint": metric(&|m| m.endpoint),
        });
        if threshold.is_some() {
            summaries["time_above_threshold"] = json!(metric(&|m| m.above.unwrap_or(0.0)));
        }
        let mut curves = vec![vec![0.0; c.times.len()]; p.len()];
        for j in 0..c.times.len() {
            let column: Vec<f64> = c.trajectories.iter().map(|t| t[j]).collect();
            for (k, q) in quantiles(&column, p).into_iter().enumerate() {
                curves[k][j] = q;
            }
        }
        let paired = |f: &dyn Fn(&Exposure) -> f64| -> Vec<f64> {
            let differences: Vec<f64> = c
                .metrics
                .iter()
                .zip(&reference.metrics)
                .map(|(m, r)| f(m) - f(r))
                .collect();
            quantiles(&differences, p)
        };
        records.insert(
            label.clone(),
            json!({
                "times": c.times,
                "sides": c.sides,
                "unit": c.unit,
                "time_unit": c.time_unit,
                "percentiles": p,
                "curve_percentiles": curves,
                "metric_percentiles": summaries,
                "subject_metrics": c.metrics.iter().map(|m| m.record.clone()).collect::<Vec<_>>(),
                "paired_differences_vs_reference": {
                    "auc": paired(&|m| m.auc),
                    "cmax": paired(&|m| m.cmax),
                    "endpoint": paired(&|m| m.endpoint),
                },
            }),
        );
    }
    Ok(records)
}

/// Whether a comparison request asks for virtual subjects with variability.
pub fn comparison_is_stochastic(arguments: &Value) -> bool {
    ["omega", "random_effects", "parameter_draws"]
        .iter()
        .any(|key| arguments.get(*key).is_some_and(|v| !v.is_null()))
}

/// Compare labelled regimens for the same subjects and report paired metrics.
///
/// With `omega`, virtual subjects receive lognormal inter-individual
/// variability; each draws from its own substream, so the draws of one
/// subject do not depend on the subject count or worker count. Without
/// `omega`, `random_effects` or `parameter_draws` the comparison is
/// deterministic: one subject at the typical values (or the runs' own values
/// when `parameters` is omitted), no random draws, and `variability: none`.
pub fn compare_regimens(model: &Arc<CompiledDocument>, arguments: &Value) -> Result<Value> {
    let args = Arguments::new(arguments, &COMPARISON_ARGUMENTS)?;
    let stochastic = comparison_is_stochastic(arguments);
    let subjects = if stochastic {
        args.int("subjects", 100, 1, 2000)? as usize
    } else {
        if args.get("subjects").is_some_and(|v| v.as_u64() != Some(1)) {
            return fail("a comparison without omega has one deterministic subject; supply omega for virtual subjects");
        }
        if args.get("seed").is_some() {
            return fail(
                "a deterministic comparison draws no random numbers; seed applies only with omega",
            );
        }
        1
    };
    let seed = args.int("seed", 1009, 0, MAX_SEED)?;
    let workers = args.int("workers", 1, 1, 8)? as usize;
    let regimens = match args.required("regimens")?.as_array() {
        Some(items) if (1..=25).contains(&items.len()) => items,
        _ => return fail("regimens must contain 1..25 labelled native requests"),
    };
    let mut labels: Vec<String> = Vec::new();
    for item in regimens {
        let label = item.get("label").and_then(Value::as_str).unwrap_or("");
        if label.is_empty() || labels.iter().any(|l| l == label) || item.get("run").is_none() {
            return fail("each regimen requires a distinct label and a run request");
        }
        labels.push(label.to_string());
    }
    let parameters = match args.get("parameters") {
        None if !stochastic => Vec::new(),
        value => named_quantities(
            value.ok_or_else(|| AnalysisError::new("invalid_input", "parameters is required"))?,
            "parameters",
        )?,
    };
    let specs = if parameters.is_empty() {
        Vec::new()
    } else {
        fixed_specs(&parameters)?
    };
    let (effects, chol) = if stochastic {
        random_effects(
            &specs,
            args.get("random_effects"),
            args.required("omega")?,
            "comparison",
        )?
    } else {
        (Vec::new(), Vec::new())
    };
    let p = match args.get("percentiles") {
        None if !stochastic => vec![0.5],
        value => percentiles(value)?,
    };
    let Some(output) = args.required("output")?.as_str() else {
        return fail("output must be a string");
    };
    let threshold = match args.get("threshold") {
        None => None,
        v => Some(finite(v, "threshold")?),
    };
    let draws: Option<Vec<Vec<f64>>> = match args.get("parameter_draws") {
        None => None,
        Some(Value::Array(items)) if (1..=1000).contains(&items.len()) => {
            let mut rows = Vec::new();
            for draw in items {
                let Some(map) = draw.as_object() else {
                    return fail("parameter draws must match selected parameter names and units");
                };
                if map.len() != parameters.len() {
                    return fail("parameter draws must match selected parameter names and units");
                }
                let mut row = Vec::new();
                for (name, _, unit) in &parameters {
                    let q = map.get(name);
                    if q.and_then(|q| q.get("unit")).and_then(Value::as_str) != Some(unit.as_str())
                    {
                        return fail(
                            "parameter draws must match selected parameter names and units",
                        );
                    }
                    let v = finite(q.and_then(|q| q.get("value")), name)?;
                    if v <= 0.0 {
                        return fail(
                            "parameter draws must match selected parameter names and units",
                        );
                    }
                    row.push(v);
                }
                rows.push(row);
            }
            Some(rows)
        }
        Some(_) => return fail("parameter_draws must contain 1..1000 quantity mappings"),
    };
    let budget = Budget::new(args.int("max_simulations", 50000, 1, 1_000_000)?, None)?;
    let mut virtual_subjects = Vec::with_capacity(subjects);
    let mut stream_ids = Vec::with_capacity(subjects);
    if stochastic {
        for i in 0..subjects {
            let mut unit = substream(seed, "compare_regimens", i as u64);
            let mut values: Vec<f64> = match &draws {
                None => parameters.iter().map(|(_, v, _)| *v).collect(),
                Some(rows) => rows[unit.stream.below(rows.len() as u64) as usize].clone(),
            };
            let z = unit.stream.normals(effects.len());
            let eta = linalg::mat_vec(&chol, &z);
            for (k, &index) in effects.iter().enumerate() {
                values[index] *= eta[k].exp();
            }
            virtual_subjects.push(values);
            stream_ids.push(unit.id);
        }
    } else {
        virtual_subjects.push(parameters.iter().map(|(_, v, _)| *v).collect());
    }
    let mut ordered: Vec<(String, Comparison)> = Vec::new();
    let mut native_identity = Value::Null;
    for (label, item) in labels.iter().zip(regimens) {
        let run = &item["run"];
        let execute = |values: &Vec<f64>| -> Result<(RunOutput, Exposure)> {
            let bound: Vec<(&Spec, f64)> = specs.iter().zip(values.iter().copied()).collect();
            let result = budget.run(model, &bind(run, &bound)?)?;
            let metrics = exposure_summary(&result, output, threshold)?;
            Ok((result, metrics))
        };
        let mut comparison: Option<Comparison> = None;
        super::work::ordered(
            virtual_subjects.iter().collect(),
            workers,
            &|values: &Vec<f64>| execute(values),
            &mut |(result, metrics)| {
                native_identity = result.identity.clone();
                push(&mut comparison, &result, metrics, output, subjects)
            },
        )?;
        ordered.push((label.clone(), comparison.expect("at least one subject")));
    }
    let records = summarize(&ordered, &p, threshold)?;
    let mut inputs = arguments.clone();
    inputs.as_object_mut().expect("object").remove("workers");
    let mut identity = identity(model, &inputs)?;
    identity["native"] = native_identity;
    Ok(json!({
        "schema": "qspquest.regimen-comparison/v0.2",
        "seed": if stochastic { json!(seed) } else { Value::Null },
        "subjects": subjects,
        "reference": labels[0],
        "regimen_order": labels,
        "regimens": records,
        "variability": match (stochastic, draws.is_some()) {
            (false, _) => "none",
            (true, true) => "iiv_and_parameter_draws",
            (true, false) => "iiv_only",
        },
        "random_stream": if stochastic { json!(RANDOM_STREAM) } else { Value::Null },
        "subject_streams": stream_ids,
        "simulations": budget.used(),
        "identity": identity,
    }))
}

/// Add one solved subject to a regimen's comparison.
fn push(
    comparison: &mut Option<Comparison>,
    result: &RunOutput,
    metrics: Exposure,
    output: &str,
    subjects: usize,
) -> Result<()> {
    let (column, unit) = result.column(output).expect("checked by exposure");
    let c = comparison.get_or_insert_with(|| Comparison {
        times: result.times.clone(),
        sides: result.sides.clone(),
        unit: unit.to_string(),
        time_unit: result.time_unit.clone(),
        metrics: Vec::new(),
        trajectories: Vec::new(),
    });
    if c.times != result.times || c.sides != result.sides || c.unit != unit {
        return fail("population trajectories must share the observation grid and output unit");
    }
    if subjects * c.times.len() > 2_000_000 {
        return fail("population trajectory exceeds bounded storage");
    }
    c.trajectories.push(column.to_vec());
    c.metrics.push(metrics);
    Ok(())
}

/// Deterministic comparison of labelled regimens already solved elsewhere.
///
/// `results` are `qspquest.result/v0.1` records in regimen order, the first
/// being the reference. The record matches a deterministic
/// [`compare_regimens`] of the same runs: one subject, no random draws and no
/// new simulation. Its identity lists each supplied result's identity.
pub fn compare_solved(
    labels: &[String],
    results: &[Value],
    output: &str,
    threshold: Option<f64>,
) -> Result<Value> {
    if labels.is_empty() || labels.len() > 25 || labels.len() != results.len() {
        return fail("regimens must contain 1..25 labelled results");
    }
    let mut ordered = Vec::new();
    let mut identities = Vec::new();
    for (label, value) in labels.iter().zip(results) {
        if label.is_empty()
            || ordered
                .iter()
                .any(|(l, _): &(String, Comparison)| l == label)
        {
            return fail("each regimen requires a distinct label");
        }
        let result = RunOutput::from_value(value)?;
        let metrics = exposure_summary(&result, output, threshold)?;
        let mut comparison = None;
        push(&mut comparison, &result, metrics, output, 1)?;
        identities.push(result.identity.clone());
        ordered.push((label.clone(), comparison.expect("one subject")));
    }
    let p = [0.5];
    let records = summarize(&ordered, &p, threshold)?;
    let inputs = json!({"labels": labels, "output": output, "threshold": threshold, "identities": identities});
    let hash = qspquest_core::identity::canonical_hash(&inputs)
        .map_err(|e| AnalysisError::new("invalid_input", e.message))?;
    Ok(json!({
        "schema": "qspquest.regimen-comparison/v0.2",
        "seed": Value::Null,
        "subjects": 1,
        "reference": labels[0],
        "regimen_order": labels,
        "regimens": records,
        "variability": "none",
        "random_stream": Value::Null,
        "subject_streams": [],
        "simulations": 0,
        "identity": {
            "analysis_request_hash": hash,
            "analysis_version": super::ANALYSIS_VERSION,
            "engine_version": env!("CARGO_PKG_VERSION"),
            "compiler_source_sha256": env!("QSPQUEST_SOURCE_SHA256"),
            "backend_version": crate::DIFFSOL_BACKEND_VERSION,
            "native": identities,
        },
    }))
}

#[cfg(test)]
mod comparison_tests {
    use super::*;

    fn model() -> Arc<CompiledDocument> {
        crate::language::parse(include_str!(
            "../../../../conformance/models/synthetic-one-compartment.qsp"
        ))
        .unwrap()
        .compile()
        .unwrap()
    }

    fn regimens() -> Value {
        let low: Value = serde_json::from_str(include_str!(
            "../../../../conformance/requests/synthetic-one-compartment.json"
        ))
        .unwrap();
        let mut high = low.clone();
        high["regimen"]["administrations"][0]["amount"]["value"] = json!(12.0);
        json!([{"label": "low", "run": low}, {"label": "high", "run": high}])
    }

    #[test]
    fn deterministic_comparison_draws_nothing_and_matches_solved_members() {
        let model = model();
        let arguments = json!({"regimens": regimens(), "output": "cp", "threshold": 1.0});
        let record = compare_regimens(&model, &arguments).unwrap();
        assert_eq!(record["variability"], "none");
        assert_eq!(record["subjects"], 1);
        assert_eq!(record["simulations"], 2);
        assert!(record["seed"].is_null() && record["random_stream"].is_null());
        assert_eq!(record["regimens"]["low"]["percentiles"], json!([0.5]));
        let solved: Vec<Value> = regimens()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let request = serde_json::from_value(r["run"].clone()).unwrap();
                serde_json::to_value(model.execute(&request).unwrap()).unwrap()
            })
            .collect();
        let labels = vec!["low".to_string(), "high".to_string()];
        let summary = compare_solved(&labels, &solved, "cp", Some(1.0)).unwrap();
        assert_eq!(summary["regimens"], record["regimens"]);
        assert_eq!(summary["simulations"], 0);
        // cp(t) = 2 exp(-0.1 t) mg/L per 6 mg, so the paired AUC difference is the low AUC.
        let low = record["regimens"]["low"]["metric_percentiles"]["auc"][0]
            .as_f64()
            .unwrap();
        let difference = record["regimens"]["high"]["paired_differences_vs_reference"]["auc"][0]
            .as_f64()
            .unwrap();
        assert!((difference - low).abs() <= 1e-9 * low);
        for refused in [json!({"subjects": 2}), json!({"seed": 3})] {
            let mut arguments = arguments.clone();
            arguments
                .as_object_mut()
                .unwrap()
                .extend(refused.as_object().unwrap().clone());
            assert!(compare_regimens(&model, &arguments).is_err());
            assert!(super::super::plan(Some(&model), "compare_regimens", &arguments).is_err());
        }
        let plan = super::super::plan(Some(&model), "compare_regimens", &arguments).unwrap();
        assert_eq!(plan["required_simulations"], 2);
        assert_eq!(plan["stochastic"], false);
    }
}

//! Serializable fit diagnostics and seeded population predictive checks.
use super::common::*;
use super::control::{boundary, Control};
use super::exposure::{fixed_specs, percentiles, random_effects};
use super::linalg;
use super::rng::{substream, RANDOM_STREAM};
use super::stats;
use crate::document::CompiledDocument;
use serde_json::{json, Value};
use std::sync::Arc;

/// Plotting rows, residual summaries, mixing and boundary warnings.
///
/// Standardized residuals are conditional residuals, not CWRES or NPDE.
/// Censored rows retain null residuals and their quantification limits.
pub fn fit_diagnostics(arguments: &Value) -> Result<Value> {
    let args = Arguments::new(arguments, &["fit"])?;
    let fit = args.required("fit")?;
    let rows = match fit.get("fitted_observations").and_then(Value::as_array) {
        Some(rows) if !rows.is_empty() => rows,
        _ => return fail("fit has no complete fitted observation rows"),
    };
    let mut residuals = Vec::new();
    for row in rows {
        match row.get("standardized_residual") {
            None | Some(Value::Null) => {}
            Some(v) => match v.as_f64() {
                Some(x) if x.is_finite() => residuals.push(x),
                _ => return fail("fit residuals are not finite"),
            },
        }
    }
    let mut warnings = Vec::new();
    let status = fit.get("status").cloned().unwrap_or(Value::Null);
    if status != json!("converged") {
        let text = match &status {
            Value::String(s) => s.clone(),
            Value::Null => "None".to_string(),
            other => other.to_string(),
        };
        warnings.push(format!("fit status is {text}"));
    }
    let nonempty = |key: &str| {
        fit.get(key)
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    };
    if nonempty("boundary_parameters") {
        warnings.push("one or more structural estimates are at search bounds".to_string());
    }
    if [
        "boundary_residual_errors",
        "boundary_covariate_coefficients",
        "boundary_variances",
    ]
    .iter()
    .any(|k| nonempty(k))
    {
        warnings.push(
            "one or more residual, covariate or variance estimates are at search bounds"
                .to_string(),
        );
    }
    if fit.get("objective_kind").and_then(Value::as_str) == Some("saem_complete_data_surrogate") {
        warnings.push(
            "complete-data surrogate cannot be used for marginal-likelihood model comparison"
                .to_string(),
        );
    }
    let count = residuals.len();
    Ok(json!({
        "schema": "qspquest.fit-diagnostics/v0.2",
        "observations": rows,
        "residual_kind": "conditional_standardized_residual",
        "residual_summary": {
            "count": count,
            "mean": if count > 0 { json!(stats::mean(&residuals)) } else { Value::Null },
            "sd": if count > 1 { json!(stats::variance(&residuals, 1).sqrt()) } else { Value::Null },
        },
        "censored_rows": rows.iter().filter(|r| r.get("censoring").and_then(Value::as_str) == Some("left")).count(),
        "parameter_trace": fit.get("trace").cloned().unwrap_or(json!([])),
        "population_diagnostics": fit.get("diagnostics").cloned().unwrap_or(Value::Null),
        "warnings": warnings,
        "identity": fit.get("identity").cloned().unwrap_or(Value::Null),
    }))
}

/// `numpy.linspace` with an exact final point.
fn linspace(a: f64, b: f64, count: usize) -> Vec<f64> {
    let step = (b - a) / (count - 1) as f64;
    let mut values: Vec<f64> = (0..count).map(|i| i as f64 * step + a).collect();
    values[count - 1] = b;
    values
}

struct Replicate {
    draw: Option<Vec<f64>>,
    censored: Option<Vec<bool>>,
    record: Value,
}

/// Arguments accepted by `visual_predictive_check`.
pub const VPC_ARGUMENTS: [&str; 15] = [
    "subjects",
    "parameters",
    "omega",
    "error",
    "random_effects",
    "covariates",
    "replicates",
    "bins",
    "percentiles",
    "confidence",
    "prediction_corrected",
    "stratify",
    "seed",
    "max_simulations",
    "workers",
];

/// Simulate population effects and observation error on the original design.
///
/// Retains each subject's regimen, fixed inputs and sampling sides. BLQ rates
/// are checked separately; concentration quantiles use only quantifiable
/// values in both observed and simulated data. Uncertainty is Monte Carlo
/// uncertainty of replicate quantiles, not fitted-parameter uncertainty. Each
/// replicate draws from its own substream, so results do not depend on the
/// number of workers. Failed replicates remain in the denominator and
/// suppress intervals.
pub fn visual_predictive_check(
    model: &Arc<CompiledDocument>,
    arguments: &Value,
    control: &mut dyn Control,
) -> Result<Value> {
    let args = Arguments::new(arguments, &VPC_ARGUMENTS)?;
    let replicates = args.int("replicates", 100, 5, 2000)? as usize;
    let seed = args.int("seed", 1009, 0, MAX_SEED)?;
    let workers = args.int("workers", 1, 1, 8)? as usize;
    let subjects = match args.required("subjects")?.as_array() {
        Some(s) if (1..=128).contains(&s.len()) => s,
        _ => return fail("VPC requires 1..128 subjects"),
    };
    let confidence = args.float("confidence", 0.95)?;
    if !(confidence > 0.0 && confidence < 1.0) {
        return fail("confidence must be in (0, 1)");
    }
    let p = percentiles(args.get("percentiles"))?;
    let parameters = named_quantities(args.required("parameters")?, "parameters")?;
    let specs = fixed_specs(&parameters)?;
    let (effects, chol) = random_effects(
        &specs,
        args.get("random_effects"),
        args.required("omega")?,
        "VPC",
    )?;
    let spec = ErrorSpec::parse(args.required("error")?)?;
    if spec.additive.fitted() || spec.proportional.fitted() {
        return fail("VPC requires fitted or fixed numeric residual SDs");
    }
    let e = spec.initial();
    let stratify: Vec<String> = match args.get("stratify") {
        None => Vec::new(),
        Some(Value::Array(items)) if items.len() <= 2 && items.iter().all(Value::is_string) => {
            items
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        }
        Some(_) => return fail("stratify supports at most two subject covariates"),
    };
    let covariates: Vec<Value> = match args.get("covariates") {
        None => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(_) => return fail("covariates must be an array"),
    };
    let prediction_corrected = args.boolean("prediction_corrected", false)?;
    let budget = Budget::new(
        args.int("max_simulations", 50000, 1, 1_000_000)?,
        control.cancel_flag(),
    )?;
    if subjects.len() * (replicates + 1) > budget.maximum {
        return fail_code(
            "VPC max_simulations must cover one baseline and every requested replicate per subject",
            "budget",
        );
    }
    let mut baseline = Vec::new();
    let mut y = Vec::new();
    let mut t = Vec::new();
    let mut observed_censored = Vec::new();
    let mut limits = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let mut typical: Vec<Vec<f64>> = Vec::new();
    let mut lengths = Vec::new();
    let mut units = std::collections::BTreeSet::new();
    let mut native_identity = Value::Null;
    for subject in subjects {
        let mut values: Vec<f64> = parameters.iter().map(|(_, v, _)| *v).collect();
        for c in &covariates {
            let name = c.get("parameter").and_then(Value::as_str).unwrap_or("");
            let Some(index) = parameters.iter().position(|(n, _, _)| n == name) else {
                return fail("VPC covariate parameter is unknown");
            };
            let key = c.get("covariate").and_then(Value::as_str).unwrap_or("");
            let value = finite(
                subject.get("covariates").and_then(|m| m.get(key)),
                "VPC subject covariate",
            )?;
            let estimate = finite(c.get("estimate"), "covariate.estimate")?;
            let reference = finite(c.get("reference"), "covariate.reference")?;
            values[index] *= (estimate * (value - reference)).exp();
        }
        let run = subject.get("run").ok_or_else(|| {
            AnalysisError::new(
                "invalid_input",
                "each subject must provide run and observations",
            )
        })?;
        let rows = subject.get("observations").ok_or_else(|| {
            AnalysisError::new(
                "invalid_input",
                "each subject must provide run and observations",
            )
        })?;
        let bound: Vec<(&Spec, f64)> = specs.iter().zip(values.iter().copied()).collect();
        let result = budget.run(model, &bind(run, &bound)?)?;
        native_identity = result.identity.clone();
        let observed = observations(&result, rows)?;
        let mut label_values = Vec::new();
        for key in &stratify {
            match subject
                .get("covariates")
                .and_then(|m| m.get(key))
                .filter(|v| !v.is_null())
            {
                Some(v) => label_values.push(v.clone()),
                None => return fail("missing VPC stratification covariate"),
            }
        }
        let label = Value::Array(label_values).to_string();
        baseline.extend_from_slice(&observed.prediction);
        y.extend_from_slice(&observed.y);
        observed_censored.extend_from_slice(&observed.censored);
        lengths.push(observed.y.len());
        for observation in rows.as_array().expect("validated") {
            let unit = observation["value"]["unit"]
                .as_str()
                .unwrap_or("")
                .to_string();
            units.insert((
                observation["output"].as_str().unwrap_or("").to_string(),
                unit.clone(),
                result.time_unit.clone(),
            ));
            t.push(result.times[observation["row"].as_u64().unwrap() as usize]);
            labels.push(label.clone());
            let censored = observation.get("censoring").and_then(Value::as_str) == Some("left");
            let limit = match observation.get("lloq").filter(|v| !v.is_null()) {
                Some(l) => Some(l),
                None if censored => observation.get("value"),
                None => None,
            };
            match limit {
                Some(limit) => {
                    if limit.get("unit").and_then(Value::as_str) != Some(unit.as_str()) {
                        return fail("LLOQ unit differs from observed output");
                    }
                    let value = finite(limit.get("value"), "LLOQ")?;
                    if censored && Some(value) != observation["value"]["value"].as_f64() {
                        return fail(
                            "censored observation value and LLOQ must name the same limit",
                        );
                    }
                    limits.push(value);
                }
                None => limits.push(f64::NEG_INFINITY),
            }
        }
        typical.push(values);
    }
    if units.len() != 1 {
        return fail("one VPC requires one output, output unit and time unit");
    }
    if baseline.iter().any(|v| !v.is_finite()) {
        return fail_code("VPC baseline predictions must be finite", "domain");
    }
    let mut distinct_labels: Vec<String> = labels.clone();
    distinct_labels.sort();
    distinct_labels.dedup();
    if y.len() * replicates > 2_000_000 || distinct_labels.len() > 32 {
        return fail("VPC exceeds bounded storage or stratification limits");
    }
    let t_min = t.iter().copied().fold(f64::INFINITY, f64::min);
    let t_max = t.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let edges = match args.get("bins") {
        None => {
            if t_min == t_max {
                return fail("VPC needs a nonzero time range");
            }
            let mut distinct = t.clone();
            distinct.sort_by(f64::total_cmp);
            distinct.dedup();
            linspace(t_min, t_max, distinct.len().min(6) + 1)
        }
        Some(v) => numbers(v, "bins").map_err(|_| {
            AnalysisError::new(
                "invalid_input",
                "VPC bins must be increasing and cover all observations",
            )
        })?,
    };
    if !(2..=101).contains(&edges.len())
        || edges.windows(2).any(|w| w[1] <= w[0])
        || edges[0] > t_min
        || edges[edges.len() - 1] < t_max
    {
        return fail("VPC bins must be increasing and cover all observations");
    }
    let groups: Vec<usize> = t
        .iter()
        .map(|ti| {
            let right = edges.partition_point(|e| e <= ti);
            (right.max(1) - 1).min(edges.len() - 2)
        })
        .collect();
    let cells: Vec<(String, usize, Vec<usize>)> = distinct_labels
        .iter()
        .flat_map(|label| {
            let labels = &labels;
            let groups = &groups;
            (0..edges.len() - 1).map(move |b| {
                let indices = (0..labels.len())
                    .filter(|&i| &labels[i] == label && groups[i] == b)
                    .collect();
                (label.clone(), b, indices)
            })
        })
        .collect();
    let mut correction = vec![1.0; y.len()];
    if prediction_corrected {
        if baseline.iter().any(|v| *v <= 0.0) {
            return fail("prediction correction requires positive population predictions");
        }
        for (_, _, indices) in &cells {
            if !indices.is_empty() {
                let center =
                    stats::median(&indices.iter().map(|&i| baseline[i]).collect::<Vec<_>>());
                for &i in indices {
                    correction[i] = center / baseline[i];
                }
            }
        }
    }
    // Worker count changes execution only, never results, so it is not identity.
    let mut context_inputs = arguments.clone();
    context_inputs
        .as_object_mut()
        .expect("object")
        .remove("workers");
    context_inputs["seed"] = json!(seed.to_string());
    context_inputs["bins"] = json!(edges);
    let request_identity = identity(model, &context_inputs)?;
    let context = json!({"request": request_identity, "native": native_identity, "random_stream": RANDOM_STREAM});
    let mut completed: Vec<Replicate> = Vec::new();
    if let Some(saved) = control.start("vpc", &context)? {
        let invalid = |m: &str| AnalysisError::new("checkpoint", m.to_string());
        let (Some(draws), Some(censored), Some(records)) = (
            saved.get("draws").and_then(Value::as_array),
            saved.get("censored").and_then(Value::as_array),
            saved.get("replicate_records").and_then(Value::as_array),
        ) else {
            return Err(invalid("VPC checkpoint state is incomplete"));
        };
        if draws.len() != censored.len() || draws.len() != records.len() || draws.len() > replicates
        {
            return Err(invalid("VPC checkpoint replicate counts differ"));
        }
        budget.restore(saved.get("simulations")).map_err(|e| {
            if e.code == "budget" {
                e
            } else {
                invalid("VPC checkpoint state is incomplete")
            }
        })?;
        for (i, ((draw, cens), record)) in draws.iter().zip(censored).zip(records).enumerate() {
            let expected = substream(seed, "vpc", i as u64).id;
            let status = record.get("status").and_then(Value::as_str);
            let ok = record.get("replicate").and_then(Value::as_u64) == Some(i as u64)
                && record.get("stream_id").and_then(Value::as_str) == Some(expected.as_str())
                && matches!(status, Some("complete") | Some("failed"))
                && record
                    .get("simulations")
                    .and_then(Value::as_u64)
                    .is_some_and(|s| s as usize <= subjects.len())
                && (draw.is_null() == (status == Some("failed")))
                && (cens.is_null() == draw.is_null());
            if !ok {
                return Err(invalid("VPC checkpoint replicate shape or status differs"));
            }
            let (draw, cens) = if draw.is_null() {
                (None, None)
            } else {
                let values = numbers(draw, "draw")
                    .map_err(|_| invalid("VPC checkpoint draw arrays are invalid"))?;
                let flags: Option<Vec<bool>> = cens
                    .as_array()
                    .map(|a| a.iter().map(Value::as_bool).collect::<Option<Vec<_>>>())
                    .flatten();
                match flags {
                    Some(flags) if values.len() == y.len() && flags.len() == y.len() => {
                        (Some(values), Some(flags))
                    }
                    _ => return Err(invalid("VPC checkpoint replicate shape or status differs")),
                }
            };
            completed.push(Replicate {
                draw,
                censored: cens,
                record: record.clone(),
            });
        }
    }
    let n_obs = y.len();
    let execute = |replicate: usize| -> Result<Replicate> {
        let unit = substream(seed, "vpc", replicate as u64);
        let mut stream = unit.stream;
        let noise: Vec<(Vec<f64>, Vec<f64>)> = lengths
            .iter()
            .map(|&length| (stream.normals(effects.len()), stream.normals(length)))
            .collect();
        let mut record = json!({"replicate": replicate, "stream_id": unit.id, "status": "complete", "simulations": 0});
        let attempt = (|| -> Result<Vec<f64>> {
            let mut simulated = Vec::with_capacity(n_obs);
            for (i, subject) in subjects.iter().enumerate() {
                let mut values = typical[i].clone();
                let (z, residual) = &noise[i];
                for (k, eta) in linalg::mat_vec(&chol, z).iter().enumerate() {
                    values[effects[k]] *= eta.exp();
                }
                let bound: Vec<(&Spec, f64)> = specs.iter().zip(values.iter().copied()).collect();
                let request = bind(&subject["run"], &bound)?;
                record["simulations"] = json!(record["simulations"].as_u64().unwrap() + 1);
                let result = budget.run(model, &request)?;
                let pred = observations(&result, &subject["observations"])?.prediction;
                if pred.len() != lengths[i] || pred.iter().any(|v| !v.is_finite()) {
                    return fail_code(
                        "VPC prediction shape or finiteness differs from the admitted design",
                        "domain",
                    );
                }
                for (j, p) in pred.iter().enumerate() {
                    let draw = if e.lognormal {
                        if *p <= 0.0 {
                            return fail_code("lognormal VPC needs positive predictions", "domain");
                        }
                        p * (e.proportional * residual[j]).exp()
                    } else {
                        p + (e.additive.powi(2) + (e.proportional * p).powi(2)).sqrt() * residual[j]
                    };
                    if !draw.is_finite() {
                        return fail_code("VPC simulated observations must be finite", "domain");
                    }
                    simulated.push(draw);
                }
            }
            Ok(simulated)
        })();
        match attempt {
            Ok(simulated) => {
                let corrected: Vec<f64> = simulated
                    .iter()
                    .zip(&correction)
                    .map(|(a, b)| a * b)
                    .collect();
                if corrected.iter().any(|v| !v.is_finite()) {
                    record["status"] = json!("failed");
                    record["code"] = json!("domain");
                    record["message"] =
                        json!("VPC prediction-corrected observations must be finite");
                    return Ok(Replicate {
                        draw: None,
                        censored: None,
                        record,
                    });
                }
                let censored = simulated.iter().zip(&limits).map(|(a, b)| a < b).collect();
                Ok(Replicate {
                    draw: Some(corrected),
                    censored: Some(censored),
                    record,
                })
            }
            Err(error) if error.code == "budget" || error.code == "cancelled" => Err(error),
            Err(error) => {
                record["status"] = json!("failed");
                record["code"] = json!(error.code);
                record["message"] = json!(error.message);
                Ok(Replicate {
                    draw: None,
                    censored: None,
                    record,
                })
            }
        }
    };
    let checkpoint_state = |completed: &[Replicate], used: usize| -> Value {
        json!({
            "draws": completed.iter().map(|r| r.draw.as_ref().map_or(Value::Null, |d| json!(d))).collect::<Vec<_>>(),
            "censored": completed.iter().map(|r| r.censored.as_ref().map_or(Value::Null, |d| json!(d))).collect::<Vec<_>>(),
            "replicate_records": completed.iter().map(|r| r.record.clone()).collect::<Vec<_>>(),
            "simulations": used,
        })
    };
    let start = completed.len();
    let outcome = super::work::ordered(
        (start..replicates).collect(),
        workers,
        &execute,
        &mut |replicate| {
            completed.push(replicate);
            let done = completed.len();
            boundary(
                control,
                "vpc",
                done,
                replicates,
                budget.used(),
                budget.maximum,
                done == replicates,
                || checkpoint_state(&completed, budget.used()),
            )
        },
    );
    if let Err(error) = outcome {
        // Drained workers have finished charging calls. Preserve their cost but
        // resume the last complete ordered prefix; unfinished rows are replayed.
        if error.code == "cancelled" || error.code == "budget" {
            control.retain(checkpoint_state(&completed, budget.used()))?;
        }
        return Err(error);
    }
    let successful = completed.iter().filter(|r| r.draw.is_some()).count();
    let failed = replicates - successful;
    let intervals_available = failed == 0;
    let draws: Vec<&Vec<f64>> = completed.iter().filter_map(|r| r.draw.as_ref()).collect();
    let blq: Vec<&Vec<bool>> = completed
        .iter()
        .filter_map(|r| r.censored.as_ref())
        .collect();
    let alpha = (1.0 - confidence) / 2.0;
    let mut records = Vec::new();
    for (label, b, indices) in &cells {
        if indices.is_empty() {
            continue;
        }
        let observed: Vec<f64> = indices
            .iter()
            .filter(|&&i| !observed_censored[i])
            .map(|&i| y[i] * correction[i])
            .collect();
        let replicate_quantiles: Vec<Vec<f64>> = (0..successful)
            .map(|r| {
                let values: Vec<f64> = indices
                    .iter()
                    .filter(|&&i| !blq[r][i])
                    .map(|&i| draws[r][i])
                    .collect();
                if values.is_empty() {
                    vec![f64::NAN; p.len()]
                } else {
                    stats::quantiles(&values, &p)
                }
            })
            .collect();
        let summaries: Vec<Value> = p
            .iter()
            .enumerate()
            .map(|(j, &percentile)| {
                let finite_values: Vec<f64> = replicate_quantiles.iter().map(|q| q[j]).filter(|v| v.is_finite()).collect();
                json!({
                    "percentile": percentile,
                    "observed": if observed.is_empty() { Value::Null } else { json!(stats::quantile(&observed, percentile)) },
                    "simulated": if finite_values.is_empty() { Value::Null } else { json!(stats::median(&finite_values)) },
                    "confidence_interval": if intervals_available && !finite_values.is_empty() {
                        json!(stats::quantiles(&finite_values, &[alpha, 1.0 - alpha]))
                    } else { Value::Null },
                    "quantifiable_replicates": finite_values.len(),
                })
            })
            .collect();
        let fractions: Vec<f64> = (0..successful)
            .map(|r| indices.iter().filter(|&&i| blq[r][i]).count() as f64 / indices.len() as f64)
            .collect();
        let observed_blq =
            indices.iter().filter(|&&i| observed_censored[i]).count() as f64 / indices.len() as f64;
        records.push(json!({
            "stratum": label,
            "start": edges[*b],
            "end": edges[b + 1],
            "observations": indices.len(),
            "quantiles": summaries,
            "observed_blq_fraction": observed_blq,
            "simulated_blq_fraction": if fractions.is_empty() { Value::Null } else { json!(stats::mean(&fractions)) },
            "blq_confidence_interval": if intervals_available && !fractions.is_empty() {
                json!(stats::quantiles(&fractions, &[alpha, 1.0 - alpha]))
            } else { Value::Null },
        }));
    }
    let (output, unit, time_unit) = units.into_iter().next().expect("one unit");
    let mut identity = request_identity;
    identity["native"] = native_identity;
    Ok(json!({
        "schema": "qspquest.vpc/v0.2",
        "seed": seed,
        "replicates": replicates,
        "status": if failed == 0 { "complete" } else { "incomplete" },
        "successful_replicates": successful,
        "failed_replicates": failed,
        "replicate_records": completed.iter().map(|r| r.record.clone()).collect::<Vec<_>>(),
        "execution": {
            "workers": workers,
            "maximum_in_flight": workers.min(replicates),
            "ordering": "replicate_index",
            "random_stream": RANDOM_STREAM,
        },
        "prediction_corrected": prediction_corrected,
        "output": output,
        "unit": unit,
        "time_unit": time_unit,
        "confidence": confidence,
        "bins": records,
        "lloq_coverage": limits.iter().filter(|v| v.is_finite()).count(),
        "total_observations": y.len(),
        "simulations": budget.used(),
        "identity": identity,
    }))
}

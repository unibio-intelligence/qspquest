//! Likelihood profiles and subject-resampling bootstrap with failure accounting.
use super::common::*;
use super::control::{boundary, Control};
use super::estimation::{fit_core, FitInput, Method};
use super::optimize;
use super::population;
use super::rng::{substream, RANDOM_STREAM};
use super::stats;
use crate::document::CompiledDocument;
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// Arguments accepted by `profile_likelihood`.
pub const PROFILE_ARGUMENTS: [&str; 9] = [
    "run",
    "observations",
    "parameters",
    "error",
    "parameter",
    "grid",
    "confidence",
    "max_simulations",
    "max_iterations",
];

/// Reoptimize every nuisance parameter and free SD at each fixed grid point.
///
/// Intervals are grid-interpolated likelihood-ratio approximations. Open ends
/// are reported as null; they are never replaced by search bounds. Every grid
/// point starts from the unrestricted optimum.
pub fn profile_likelihood(model: &Arc<CompiledDocument>, arguments: &Value) -> Result<Value> {
    let args = Arguments::new(arguments, &PROFILE_ARGUMENTS)?;
    let run = args.required("run")?;
    let observed_rows = args.required("observations")?;
    let specs = parameter_specs(args.required("parameters")?)?;
    let err = ErrorSpec::parse(args.required("error")?)?;
    let parameter = args.required("parameter")?.as_str().unwrap_or("");
    let Some(index) = specs.iter().position(|s| s.name == parameter) else {
        return fail("profile parameter is not selected for fitting");
    };
    let confidence = args.float("confidence", 0.95)?;
    if !(confidence > 0.0 && confidence < 1.0) {
        return fail("confidence must be in (0, 1)");
    }
    let grid_error = || {
        AnalysisError::new(
            "invalid_input",
            "profile grid must contain 3..200 strictly increasing finite values",
        )
    };
    let grid = numbers(args.required("grid")?, "grid").map_err(|_| grid_error())?;
    if !(3..=200).contains(&grid.len()) || grid.windows(2).any(|w| w[1] <= w[0]) {
        return Err(grid_error());
    }
    if grid[0] < specs[index].lower || grid[grid.len() - 1] > specs[index].upper {
        return fail("profile grid exceeds fitted parameter bounds");
    }
    let budget = Budget::new(args.int("max_simulations", 20000, 1, 1_000_000)?, None)?;
    let max_iterations = args.int("max_iterations", 300, 1, 10000)? as usize;
    let fit =
        |run: &Value, specs: Vec<Spec>, error: ErrorSpec, iterations: usize, method: Method| {
            fit_core(
                model,
                &FitInput {
                    run,
                    observations: observed_rows,
                    specs,
                    error,
                    max_iterations: iterations,
                    tolerance: 1e-7,
                    method,
                },
                &budget,
            )
        };
    let with_initial = |specs: &[Spec], values: &[f64]| -> Vec<Spec> {
        specs
            .iter()
            .zip(values)
            .map(|(s, v)| Spec {
                initial: v.clamp(s.lower, s.upper),
                ..s.clone()
            })
            .collect()
    };
    let error_from = |e: &ErrorValues| -> ErrorSpec {
        let mut spec = err;
        for key in err.fitted_keys() {
            if let Sd::Fitted { lower, upper, .. } = err.sd(key) {
                let initial = e.get(key).clamp(lower, upper);
                let sd = Sd::Fitted {
                    initial,
                    lower,
                    upper,
                };
                if key == "additive" {
                    spec.additive = sd;
                } else {
                    spec.proportional = sd;
                }
            }
        }
        spec
    };
    let mut initial = None;
    let mut fit_specs = specs.clone();
    let mut fit_error = err;
    if max_iterations > 1 {
        let first = fit(
            run,
            specs.clone(),
            err,
            (max_iterations / 2).min(50),
            Method::Gradient,
        )?;
        fit_specs = with_initial(&specs, &first.values);
        fit_error = error_from(&first.error);
        initial = Some(first);
    }
    let used = initial.as_ref().map_or(0, |f| f.iterations);
    let optimum = fit(
        run,
        fit_specs,
        fit_error,
        (max_iterations - used).max(1),
        Method::DerivativeFree,
    )?;
    if optimum.status != "converged" {
        return fail_code("profile requires a converged unrestricted fit", "optimizer");
    }
    let remaining: Vec<Spec> = specs
        .iter()
        .zip(&optimum.values)
        .enumerate()
        .filter(|(i, _)| *i != index)
        .map(|(_, (s, v))| Spec {
            initial: *v,
            ..s.clone()
        })
        .collect();
    let starting_error = error_from(&optimum.error);
    let mut records = Vec::new();
    for &value in &grid {
        let attempt = (|| -> Result<(f64, String, Value)> {
            let fixed_run = bind(run, &[(&specs[index], value)])?;
            if !remaining.is_empty() {
                let fitted = fit(
                    &fixed_run,
                    remaining.clone(),
                    starting_error,
                    max_iterations,
                    Method::DerivativeFree,
                )?;
                return Ok((
                    fitted.objective,
                    fitted.status,
                    fitted.record["parameters"].clone(),
                ));
            }
            let result = budget.run(model, &fixed_run)?;
            let observed = observations(&result, observed_rows)?;
            let keys = err.fitted_keys();
            if keys.is_empty() {
                let e = starting_error.initial();
                let v =
                    likelihood(&observed.y, &observed.prediction, &observed.censored, &e)?.value;
                return Ok((v, "converged".to_string(), json!({})));
            }
            let base = starting_error.initial();
            let mut objective = |log_sd: &[f64]| -> Result<(f64, Vec<f64>)> {
                let mut e = base;
                for (k, key) in keys.iter().enumerate() {
                    e.set(key, log_sd[k].exp());
                }
                let evaluated =
                    likelihood(&observed.y, &observed.prediction, &observed.censored, &e)?;
                let score = evaluated.log_sd_score(&observed.prediction, &observed.censored, &e);
                let gradient = keys
                    .iter()
                    .map(|key| score[usize::from(*key != "additive")])
                    .collect();
                Ok((evaluated.value, gradient))
            };
            let start: Vec<f64> = keys.iter().map(|k| optimum.error.get(k).ln()).collect();
            let lo: Vec<f64> = keys.iter().map(|k| err.sd(k).bounds().0.ln()).collect();
            let hi: Vec<f64> = keys.iter().map(|k| err.sd(k).bounds().1.ln()).collect();
            let mut out =
                optimize::minimize_bounded(&start, &lo, &hi, &mut objective, max_iterations)?;
            if out.stop == optimize::Stop::LineSearchFailed && out.iterations < max_iterations {
                // Resolve a line search stalled at likelihood precision with
                // bounded compass search; its own stop must establish convergence.
                let widths: Vec<f64> = lo.iter().zip(&hi).map(|(a, b)| b - a).collect();
                let unit_start: Vec<f64> = out
                    .x
                    .iter()
                    .zip(&lo)
                    .zip(&widths)
                    .map(|((x, lower), width)| (x - lower) / width)
                    .collect();
                let natural = |unit: &[f64]| -> Vec<f64> {
                    unit.iter()
                        .zip(&lo)
                        .zip(&widths)
                        .map(|((z, lower), width)| lower + width * z)
                        .collect()
                };
                let mut fallback = optimize::pattern_search(
                    &unit_start,
                    &mut |unit| objective(&natural(unit)).map(|(value, _)| value),
                    max_iterations - out.iterations,
                    1e-10,
                )?;
                fallback.x = natural(&fallback.x);
                fallback.iterations += out.iterations;
                out = fallback;
            }
            let status =
                if optimize::require_converged(&out, "residual-error profile fit failed").is_ok() {
                    "converged"
                } else {
                    "optimizer_failed"
                };
            Ok((out.value, status.to_string(), json!({})))
        })();
        match attempt {
            Ok((objective, status, nuisance)) => records.push(json!({
                "value": value,
                "objective": objective,
                "delta_nll": objective - optimum.objective,
                "status": status,
                "nuisance_parameters": nuisance,
            })),
            Err(error) if error.code == "budget" => records.push(json!({
                "value": value, "objective": null, "delta_nll": null, "status": "evaluation_limit",
            })),
            Err(error) if matches!(error.code.as_str(), "domain" | "solver" | "optimizer") => records.push(json!({
                "value": value, "objective": null, "delta_nll": null, "status": "failed", "message": error.message,
            })),
            Err(error) => return Err(error),
        }
    }
    let threshold = stats::chi2_1_ppf(confidence) / 2.0;
    let estimate = optimum.values[index];
    let mut augmented: Vec<(f64, Option<f64>, bool)> = records
        .iter()
        .map(|r| {
            (
                r["value"].as_f64().unwrap(),
                r["delta_nll"].as_f64(),
                r["status"] == json!("converged"),
            )
        })
        .collect();
    augmented.push((estimate, Some(0.0), true));
    augmented.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut crossings = Vec::new();
    for pair in augmented.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if !(a.2 && b.2) {
            continue;
        }
        let (da, db) = (a.1.unwrap() - threshold, b.1.unwrap() - threshold);
        if da * db <= 0.0 && da != db {
            crossings.push(a.0 + (b.0 - a.0) * (-da) / (db - da));
        }
    }
    let left = crossings
        .iter()
        .copied()
        .filter(|v| *v <= estimate)
        .reduce(f64::max);
    let right = crossings
        .iter()
        .copied()
        .filter(|v| *v >= estimate)
        .reduce(f64::min);
    let complete = records.iter().all(|r| r["status"] == json!("converged"));
    Ok(json!({
        "schema": "qspquest.profile/v0.2",
        "parameter": parameter,
        "estimate": estimate,
        "unit": specs[index].unit,
        "confidence": confidence,
        "delta_nll_threshold": threshold,
        "interval": [left, right],
        "interval_method": "grid_interpolated_likelihood_ratio",
        "status": if complete { "complete" } else { "incomplete" },
        "profile": records,
        "initial_fit": initial.map_or(Value::Null, |f| f.record),
        "unrestricted_fit": optimum.record,
        "simulations": budget.used(),
        "identity": identity(model, arguments)?,
    }))
}

/// Arguments accepted by `bootstrap_population`.
pub const BOOTSTRAP_ARGUMENTS: [&str; 10] = [
    "subjects",
    "parameters",
    "omega",
    "error",
    "replicates",
    "confidence",
    "seed",
    "minimum_success_fraction",
    "max_simulations",
    "fit_options",
];

/// Resample whole subjects, retaining dose histories and covariates together.
///
/// Every replicate has a distinct recorded seed, source-subject map, status,
/// boundary flags and failure message. Percentile intervals require the stated
/// success fraction. No coverage calibration is implied by finite intervals.
pub fn bootstrap_population(
    model: &Arc<CompiledDocument>,
    arguments: &Value,
    control: &mut dyn Control,
) -> Result<Value> {
    let args = Arguments::new(arguments, &BOOTSTRAP_ARGUMENTS)?;
    let replicates = args.int("replicates", 100, 2, 1000)? as usize;
    let seed = args.int("seed", 1009, 0, MAX_SEED)?;
    let confidence = args.float("confidence", 0.95)?;
    let fraction = args.float("minimum_success_fraction", 0.8)?;
    if !(confidence > 0.0 && confidence < 1.0) || !(fraction > 0.0 && fraction <= 1.0) {
        return fail(
            "confidence and minimum_success_fraction must be in (0, 1] with confidence < 1",
        );
    }
    let subjects = match args.required("subjects")?.as_array() {
        Some(s) if !s.is_empty() => s,
        _ => return fail("subjects are required"),
    };
    let fit_options: Map<String, Value> = match args.get("fit_options") {
        None => Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(_) => return fail("fit_options must be an object"),
    };
    if fit_options.contains_key("seed") || fit_options.contains_key("max_simulations") {
        return fail("bootstrap controls fit seeds and the aggregate budget");
    }
    for key in fit_options.keys() {
        if !population::POPULATION_ARGUMENTS.contains(&key.as_str())
            || ["subjects", "parameters", "omega", "error"].contains(&key.as_str())
        {
            return fail(format!("unknown population fit option {key}"));
        }
    }
    let parameters = args.required("parameters")?;
    let specs = parameter_specs(parameters)?;
    let budget = Budget::new(
        args.int("max_simulations", 1_000_000, 1, 1_000_000)?,
        control.cancel_flag(),
    )?;
    let mut context_inputs = arguments.clone();
    context_inputs["seed"] = json!(seed.to_string());
    let context =
        json!({"request": identity(model, &context_inputs)?, "random_stream": RANDOM_STREAM});
    let mut records: Vec<Value> = Vec::new();
    if let Some(saved) = control.start("bootstrap", &context)? {
        budget.restore(saved.get("simulations"))?;
        records = saved
            .get("records")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| {
                AnalysisError::new("checkpoint", "bootstrap checkpoint state is invalid")
            })?;
        if records.len() > replicates {
            return fail_code("bootstrap checkpoint state is invalid", "checkpoint");
        }
    }
    for replicate in records.len()..replicates {
        let mut unit = substream(seed, "bootstrap", replicate as u64);
        let indices: Vec<usize> = (0..subjects.len())
            .map(|_| unit.stream.below(subjects.len() as u64) as usize)
            .collect();
        let fit_seed = unit.stream.below(MAX_SEED + 1);
        let cohort: Vec<Value> = indices
            .iter()
            .enumerate()
            .map(|(new_id, &index)| {
                let mut subject = subjects[index].clone();
                subject["source_subject_id"] = subject.get("id").cloned().unwrap_or(json!(index));
                subject["id"] = json!(format!("bootstrap-{new_id}"));
                subject
            })
            .collect();
        let mut fit_arguments = fit_options.clone();
        fit_arguments.insert("subjects".into(), Value::Array(cohort));
        fit_arguments.insert("parameters".into(), parameters.clone());
        fit_arguments.insert("omega".into(), args.required("omega")?.clone());
        fit_arguments.insert("error".into(), args.required("error")?.clone());
        fit_arguments.insert("seed".into(), json!(fit_seed));
        let mut record = json!({
            "replicate": replicate,
            "seed": fit_seed,
            "stream_id": unit.id,
            "source_indices": indices,
            "parameters": null,
            "boundary_parameters": [],
        });
        match population::fit_with_budget(model, &Value::Object(fit_arguments), &budget) {
            Ok(result) => {
                for key in [
                    "status",
                    "parameters",
                    "omega",
                    "error",
                    "covariate_effects",
                    "boundary_parameters",
                    "boundary_residual_errors",
                    "boundary_covariate_coefficients",
                    "boundary_variances",
                ] {
                    record[key] = result[key].clone();
                }
            }
            Err(error) if error.code == "cancelled" => return Err(error),
            Err(error) => {
                record["status"] = json!("failed");
                record["code"] = json!(error.code);
                record["message"] = json!(error.message);
            }
        }
        records.push(record);
        boundary(
            control,
            "bootstrap",
            replicate + 1,
            replicates,
            budget.used(),
            budget.maximum,
            replicate + 1 == replicates,
            || json!({"records": records, "simulations": budget.used()}),
        )?;
    }
    let successful: Vec<&Value> = records
        .iter()
        .filter(|r| r["status"] == json!("converged"))
        .collect();
    let needed = 2usize.max((replicates as f64 * fraction).ceil() as usize);
    let alpha = (1.0 - confidence) / 2.0;
    let bounds =
        |values: Vec<f64>| -> Value { json!(stats::quantiles(&values, &[alpha, 1.0 - alpha])) };
    let (intervals, nuisance) = if successful.len() >= needed {
        let mut intervals = Map::new();
        for spec in &specs {
            let values = successful
                .iter()
                .map(|r| {
                    r["parameters"][&spec.name]["value"]
                        .as_f64()
                        .unwrap_or(f64::NAN)
                })
                .collect();
            intervals.insert(
                spec.name.clone(),
                json!({"unit": spec.unit, "bounds": bounds(values)}),
            );
        }
        let d = specs.len();
        let mut omega_bounds = vec![vec![vec![0.0; d]; d]; 2];
        for a in 0..d {
            for b in 0..d {
                let values: Vec<f64> = successful
                    .iter()
                    .map(|r| r["omega"][a][b].as_f64().unwrap_or(f64::NAN))
                    .collect();
                let q = stats::quantiles(&values, &[alpha, 1.0 - alpha]);
                omega_bounds[0][a][b] = q[0];
                omega_bounds[1][a][b] = q[1];
            }
        }
        let covariate_count = successful[0]["covariate_effects"]
            .as_array()
            .map_or(0, Vec::len);
        let nuisance = json!({
            "error": {
                "additive": bounds(successful.iter().map(|r| r["error"]["additive"].as_f64().unwrap_or(f64::NAN)).collect()),
                "proportional": bounds(successful.iter().map(|r| r["error"]["proportional"].as_f64().unwrap_or(f64::NAN)).collect()),
            },
            "omega": omega_bounds,
            "covariates": (0..covariate_count).map(|j| bounds(successful.iter().map(|r| r["covariate_effects"][j]["estimate"].as_f64().unwrap_or(f64::NAN)).collect())).collect::<Vec<_>>(),
        });
        (Value::Object(intervals), nuisance)
    } else {
        (Value::Null, Value::Null)
    };
    let complete = !intervals.is_null();
    Ok(json!({
        "schema": "qspquest.bootstrap/v0.2",
        "method": "subject_percentile_bootstrap",
        "status": if complete { "complete" } else { "insufficient_successful_replicates" },
        "confidence": confidence,
        "successful_replicates": successful.len(),
        "failed_replicates": replicates - successful.len(),
        "replicates": records,
        "intervals": intervals,
        "nuisance_intervals": nuisance,
        "random_stream": RANDOM_STREAM,
        "simulations": budget.used(),
        "identity": identity(model, &context_inputs)?,
    }))
}

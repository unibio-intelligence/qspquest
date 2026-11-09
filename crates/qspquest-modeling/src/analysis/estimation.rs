//! Bounded deterministic maximum likelihood using the native simulator.
use super::common::*;
use super::optimize::{self, Stop};
use crate::document::CompiledDocument;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::sync::Arc;

pub const GRADIENT_ALGORITHM: &str = "projected_bfgs_central_difference";
pub const DERIVATIVE_FREE_ALGORITHM: &str = "compass_pattern_search";
/// Central-difference step in unit-box coordinates.
const DIFFERENCE_STEP: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Gradient,
    DerivativeFree,
}

impl Method {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "gradient" => Ok(Self::Gradient),
            "derivative_free" => Ok(Self::DerivativeFree),
            _ => fail("method must be gradient or derivative_free"),
        }
    }
    pub fn algorithm(&self) -> &'static str {
        match self {
            Self::Gradient => GRADIENT_ALGORITHM,
            Self::DerivativeFree => DERIVATIVE_FREE_ALGORITHM,
        }
    }
}

pub struct FitInput<'a> {
    pub run: &'a Value,
    pub observations: &'a Value,
    pub specs: Vec<Spec>,
    pub error: ErrorSpec,
    pub max_iterations: usize,
    pub tolerance: f64,
    pub method: Method,
}

pub struct FitRecord {
    pub record: Value,
    pub status: String,
    pub objective: f64,
    pub values: Vec<f64>,
    pub error: ErrorValues,
    pub iterations: usize,
    pub native: Value,
}

struct Best {
    objective: f64,
    values: Vec<f64>,
    error: ErrorValues,
    result: RunOutput,
    observed: Observed,
}

pub fn status_name(stop: Stop) -> &'static str {
    match stop {
        Stop::Converged => "converged",
        Stop::IterationLimit => "iteration_limit",
        Stop::LineSearchFailed => "line_search_failed",
    }
}

/// Fit bounded parameters and optional residual SDs. Fitted SDs are searched
/// on the log scale. The reported estimate is the best evaluated point.
pub fn fit_core(
    model: &Arc<CompiledDocument>,
    input: &FitInput,
    budget: &Budget,
) -> Result<FitRecord> {
    let specs = &input.specs;
    let error_keys = input.error.fitted_keys();
    let n = specs.len();
    let mut lower: Vec<f64> = specs.iter().map(|s| s.lower).collect();
    let mut width: Vec<f64> = specs.iter().map(|s| s.upper - s.lower).collect();
    let mut start: Vec<f64> = specs.iter().map(|s| s.initial).collect();
    for key in &error_keys {
        if let Sd::Fitted {
            initial,
            lower: lo,
            upper: hi,
        } = input.error.sd(key)
        {
            lower.push(lo.ln());
            width.push(hi.ln() - lo.ln());
            start.push(initial.ln());
        }
    }
    let unit_start: Vec<f64> = start
        .iter()
        .zip(&lower)
        .zip(&width)
        .map(|((x, l), w)| (x - l) / w)
        .collect();
    let before = budget.used();
    let trace: RefCell<Vec<Value>> = RefCell::new(Vec::new());
    let best: RefCell<Option<Best>> = RefCell::new(None);
    let single_output = {
        let outputs: std::collections::BTreeSet<&str> = input
            .observations
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| r.get("output").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        outputs.len() == 1
    };
    let unpack = |z: &[f64]| -> (Vec<f64>, ErrorValues) {
        let natural: Vec<f64> = z
            .iter()
            .zip(&lower)
            .zip(&width)
            .map(|((z, l), w)| l + w * z)
            .collect();
        let mut e = input.error.initial();
        for (i, key) in error_keys.iter().enumerate() {
            e.set(key, natural[n + i].exp());
        }
        (natural[..n].to_vec(), e)
    };
    let evaluate = |z: &[f64], strict: bool| -> Result<f64> {
        let (values, e) = unpack(z);
        let attempt = (|| -> Result<(f64, RunOutput, Observed)> {
            let bound: Vec<(&Spec, f64)> = specs.iter().zip(values.iter().copied()).collect();
            let result = budget.run(model, &bind(input.run, &bound)?)?;
            let observed = observations(&result, input.observations)?;
            if !single_output {
                return fail("one shared residual-error contract requires one observed output");
            }
            if observed.censored.iter().all(|c| *c) && !error_keys.is_empty() {
                return fail("all-censored data cannot estimate residual error");
            }
            let objective =
                likelihood(&observed.y, &observed.prediction, &observed.censored, &e)?.value;
            Ok((objective, result, observed))
        })();
        let (objective, result, observed) = match attempt {
            Ok(v) => v,
            Err(error) if !strict && recoverable(&error) => return Ok(1e100),
            Err(error) => return Err(error),
        };
        trace.borrow_mut().push(json!({
            "simulation": budget.used() - before,
            "objective": objective,
            "parameters": specs.iter().zip(&values).map(|(s, v)| (s.name.clone(), json!(v))).collect::<serde_json::Map<_, _>>(),
            "error": e.to_value(),
        }));
        let mut slot = best.borrow_mut();
        if slot.as_ref().is_none_or(|b| objective < b.objective) {
            *slot = Some(Best {
                objective,
                values,
                error: e,
                result,
                observed,
            });
        }
        Ok(objective)
    };
    let initial = evaluate(&unit_start, true)?;
    let outcome = match input.method {
        Method::Gradient => {
            let mut f = |z: &[f64]| -> Result<(f64, Vec<f64>)> {
                let fz = evaluate(z, false)?;
                let g = optimize::finite_difference_gradient(z, fz, DIFFERENCE_STEP, &mut |x| {
                    evaluate(x, false)
                })?;
                Ok((fz, g))
            };
            optimize::projected_bfgs(
                &unit_start,
                &mut f,
                input.max_iterations,
                input.tolerance,
                input.tolerance,
            )
        }
        Method::DerivativeFree => optimize::pattern_search(
            &unit_start,
            &mut |z| evaluate(z, false),
            input.max_iterations,
            input.tolerance,
        ),
    };
    let (status, message, iterations) = match outcome {
        Ok(o) => (status_name(o.stop).to_string(), o.message, o.iterations),
        Err(error) if error.code == "budget" => (
            "evaluation_limit".to_string(),
            "aggregate simulation budget exhausted".to_string(),
            0,
        ),
        Err(error) => return Err(error),
    };
    let best = best
        .into_inner()
        .expect("initial evaluation sets the best point");
    let fitted = likelihood(
        &best.observed.y,
        &best.observed.prediction,
        &best.observed.censored,
        &best.error,
    )?;
    let rows: Vec<Value> = input
        .observations
        .as_array()
        .expect("validated observations")
        .iter()
        .enumerate()
        .map(|(i, observation)| {
            let row = observation["row"].as_u64().expect("validated row") as usize;
            let censored = best.observed.censored[i];
            let mut record = json!({
                "row": row,
                "output": observation["output"],
                "time": best.result.times[row],
                "time_unit": best.result.time_unit,
                "side": best.result.sides[row],
                "unit": observation["value"]["unit"],
                "observed": best.observed.y[i],
                "prediction": best.observed.prediction[i],
                "censoring": observation.get("censoring").cloned().unwrap_or(Value::Null),
                "residual": if censored { Value::Null } else { json!(fitted.residual[i]) },
                "standardized_residual": if censored { Value::Null } else { json!(fitted.residual[i] / fitted.sd[i]) },
            });
            if let Some(source) = observation.get("source_row") {
                record["source_row"] = source.clone();
            }
            record
        })
        .collect();
    let boundary_parameters: Vec<&str> = specs
        .iter()
        .zip(&best.values)
        .filter(|(s, v)| near_bound(**v, s.lower, s.upper))
        .map(|(s, _)| s.name.as_str())
        .collect();
    let boundary_errors: Vec<&str> = error_keys
        .iter()
        .filter(|key| {
            let (lo, hi) = input.error.sd(key).bounds();
            near_bound(best.error.get(key), lo, hi)
        })
        .copied()
        .collect();
    let record = json!({
        "schema": "qspquest.analysis-fit/v0.2",
        "method": input.method.algorithm(),
        "status": status,
        "message": message,
        "parameters": scalar_parameters(specs, &best.values),
        "error": best.error.to_value(),
        "objective": best.objective,
        "initial_objective": initial,
        "objective_kind": "negative_log_likelihood",
        "simulations": budget.used() - before,
        "iterations": iterations,
        "trace": trace.into_inner(),
        "fitted_observations": rows,
        "boundary_parameters": boundary_parameters,
        "boundary_residual_errors": boundary_errors,
    });
    Ok(FitRecord {
        record,
        status,
        objective: best.objective,
        values: best.values,
        error: best.error,
        iterations,
        native: best.result.identity,
    })
}

pub const FIT_ARGUMENTS: [&str; 8] = [
    "run",
    "observations",
    "parameters",
    "error",
    "max_simulations",
    "max_iterations",
    "tolerance",
    "method",
];

pub fn parse_fit<'a>(args: &Arguments<'a>) -> Result<FitInput<'a>> {
    let specs = parameter_specs(args.required("parameters")?)?;
    let error = ErrorSpec::parse(args.required("error")?)?;
    let max_iterations = args.int("max_iterations", 300, 1, 10000)? as usize;
    let tolerance = args.float("tolerance", 1e-7)?;
    if !(tolerance > 0.0) {
        return fail("tolerance must be finite and positive");
    }
    let method = Method::parse(&args.string("method", "gradient")?)?;
    Ok(FitInput {
        run: args.required("run")?,
        observations: args.required("observations")?,
        specs,
        error,
        max_iterations,
        tolerance,
        method,
    })
}

/// Fit bounded parameters and additive/proportional SDs by native simulation.
///
/// Event parameters select quantity-valued paths in the run request. The
/// normal combined-error model adds variances. Lognormal SD is on log
/// concentration. All observations sharing this error use one output.
pub fn fit_data(
    model: &Arc<CompiledDocument>,
    arguments: &Value,
    budget: Option<&Budget>,
) -> Result<Value> {
    let args = Arguments::new(arguments, &FIT_ARGUMENTS)?;
    let input = parse_fit(&args)?;
    let owned;
    let budget = match budget {
        Some(b) => b,
        None => {
            owned = Budget::new(args.int("max_simulations", 2000, 1, 1_000_000)?, None)?;
            &owned
        }
    };
    let fit = fit_core(model, &input, budget)?;
    let mut record = fit.record;
    let mut identity = identity(model, arguments)?;
    identity["native"] = fit.native;
    record["identity"] = identity;
    Ok(record)
}

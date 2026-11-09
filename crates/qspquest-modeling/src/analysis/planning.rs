//! Static admission of analysis requests: no simulation and no random draw.
//!
//! A plan checks the argument names each operation accepts, its parameter,
//! error, random-effect and percentile specifications, its integer controls,
//! and every embedded run request. Each run is bound to the parameters'
//! initial (or typical) values and prepared by [`CompiledDocument::plan_run`],
//! which checks units, events and grids without integrating. Observation rows
//! are checked against the prepared grid and output units. The plan is a
//! deterministic function of the model and the request, so a caller can
//! compare a plan made before execution with one made at execution.
use super::common::*;
use super::exposure::{fixed_specs, percentiles, random_effects};
use crate::document::CompiledDocument;
use serde_json::{json, Value};
use std::sync::Arc;

/// Version of the plan record.
pub const PLAN_SCHEMA: &str = "qspquest.analysis-plan/v0.2";

/// Integer controls of one operation: name, default, lower and upper bound.
type Control = (&'static str, u64, u64, u64);

const SEED: Control = ("seed", 1009, 0, MAX_SEED);
const WORKERS: Control = ("workers", 1, 1, 8);

/// Arguments accepted by each analysis operation.
pub fn accepted_arguments(operation: &str) -> Option<&'static [&'static str]> {
    Some(match operation {
        "fit_data" => &super::estimation::FIT_ARGUMENTS,
        "fit_population" => &super::population::POPULATION_ARGUMENTS,
        "visual_predictive_check" => &super::diagnostics::VPC_ARGUMENTS,
        "profile_likelihood" => &super::uncertainty::PROFILE_ARGUMENTS,
        "bootstrap_population" => &super::uncertainty::BOOTSTRAP_ARGUMENTS,
        "compare_regimens" => &super::exposure::COMPARISON_ARGUMENTS,
        "event_sensitivities" => &super::derivatives::SENSITIVITY_ARGUMENTS,
        "exposure_summary" => &["result", "output", "threshold"],
        "fit_diagnostics" => &["fit"],
        _ => return None,
    })
}

/// The `max_simulations` an operation uses when the request omits it.
pub fn default_max_simulations(operation: &str) -> Option<u64> {
    Some(match operation {
        "fit_data" => 2000,
        "fit_population" => 200_000,
        "visual_predictive_check" | "compare_regimens" => 50_000,
        "profile_likelihood" => 20_000,
        "bootstrap_population" => 1_000_000,
        "event_sensitivities" => 1000,
        _ => return None,
    })
}

fn controls(operation: &str) -> &'static [Control] {
    match operation {
        "fit_data" => &[("max_iterations", 300, 1, 10_000)],
        "profile_likelihood" => &[("max_iterations", 300, 1, 10_000)],
        "fit_population" => &[
            SEED,
            ("max_iterations", 600, 40, 2000),
            ("sweeps", 3, 1, 20),
        ],
        "visual_predictive_check" => &[SEED, WORKERS, ("replicates", 100, 5, 2000)],
        "bootstrap_population" => &[SEED, ("replicates", 100, 2, 1000)],
        "compare_regimens" => &[SEED, WORKERS, ("subjects", 100, 1, 2000)],
        _ => &[],
    }
}

struct Embedded<'a> {
    pointer: String,
    run: &'a Value,
    observations: Option<&'a Value>,
}

fn pointer(parent: &str, key: &str) -> String {
    format!("{parent}/{}", key.replace('~', "~0").replace('/', "~1"))
}

fn located<T>(at: &str, result: Result<T>) -> Result<T> {
    result.map_err(|mut error| {
        if error.expression.is_none() {
            error.expression = Some(at.to_string());
        }
        error
    })
}

/// Subjects of population operations: `{id, run, observations, covariates}`.
fn subjects<'a>(
    args: &Arguments<'a>,
    out: &mut Vec<Embedded<'a>>,
    range: std::ops::RangeInclusive<usize>,
) -> Result<usize> {
    let Some(items) = args.required("subjects")?.as_array() else {
        return fail("subjects must be an array of subject records");
    };
    if !range.contains(&items.len()) {
        return fail(format!(
            "subjects must contain {}..{} subject records",
            range.start(),
            range.end()
        ));
    }
    for (index, subject) in items.iter().enumerate() {
        let at = pointer("/arguments/subjects", &index.to_string());
        let (Some(run), Some(observations)) = (subject.get("run"), subject.get("observations"))
        else {
            return located(&at, fail("each subject must provide run and observations"));
        };
        out.push(Embedded {
            pointer: pointer(&at, "run"),
            run,
            observations: Some(observations),
        });
    }
    Ok(items.len())
}

/// Bind initial values and prepare one run without integrating it.
fn prepare(
    model: &Arc<CompiledDocument>,
    embedded: &Embedded,
    specs: &[Spec],
    output: Option<&str>,
) -> Result<Value> {
    let at = &embedded.pointer;
    let bound: Vec<(&Spec, f64)> = specs.iter().map(|s| (s, s.initial)).collect();
    let request = located(at, bind(embedded.run, &bound))?;
    let request: qspquest_core::run::RunRequest = serde_json::from_value(request)
        .map_err(|e| AnalysisError::new("invalid_input", format!("{at}: {e}")))?;
    let plan = located(at, model.plan_run(&request).map_err(AnalysisError::from))?;
    let unit_of = |name: &str| {
        plan.outputs
            .iter()
            .find(|o| o.name == name)
            .map(|o| o.unit.as_str())
    };
    if let Some(name) = output {
        if unit_of(name).is_none() {
            return located(at, fail(format!("unknown analysis output {name}")));
        }
    }
    let mut observed = 0;
    if let Some(rows) = embedded.observations {
        let rows_at = at.trim_end_matches("/run").to_string() + "/observations";
        let Some(rows) = rows.as_array() else {
            return located(&rows_at, fail("observations must contain 1..10000 rows"));
        };
        if !(1..=10000).contains(&rows.len()) {
            return located(&rows_at, fail("observations must contain 1..10000 rows"));
        }
        let last = plan.observations.len().saturating_sub(1) as u64;
        let mut keys = std::collections::BTreeSet::new();
        for (index, row) in rows.iter().enumerate() {
            let row_at = pointer(&rows_at, &index.to_string());
            let mut check = || -> Result<()> {
                let number = integer(row.get("row"), "observation.row", 0, last)?;
                let name = row.get("output").and_then(Value::as_str).unwrap_or("");
                let Some(unit) = unit_of(name) else {
                    return fail(format!("unknown observation output {name}"));
                };
                if !keys.insert((number, name.to_string())) {
                    return fail("duplicate observation row and output");
                }
                let value = row.get("value");
                if value.and_then(|v| v.get("unit")).and_then(Value::as_str) != Some(unit) {
                    return fail("observation units must equal the declared output units");
                }
                finite(value.and_then(|v| v.get("value")), "observation.value")?;
                match row.get("censoring").filter(|v| !v.is_null()) {
                    None => Ok(()),
                    Some(Value::String(s)) if s == "left" => Ok(()),
                    Some(_) => fail("only left censoring is supported"),
                }
            };
            located(&row_at, check())?;
        }
        observed = rows.len();
    }
    Ok(json!({
        "pointer": at,
        "time_unit": plan.time_unit,
        "start": plan.start,
        "end": plan.end,
        "result_rows": plan.observations.len(),
        "events": plan.events.len(),
        "outputs": plan.outputs.iter().map(|o| json!({"name": o.name, "unit": o.unit})).collect::<Vec<_>>(),
        "observation_records": observed,
        "budgets": plan.workload.budgets,
        "prospective_identity": plan.prospective_identity,
    }))
}

/// Prepare an analysis without simulating or drawing random numbers.
///
/// `model` is required for model operations, as in [`super::execute`].
pub fn plan(
    model: Option<&Arc<CompiledDocument>>,
    operation: &str,
    arguments: &Value,
) -> Result<Value> {
    let Some(accepted) = accepted_arguments(operation) else {
        return fail(format!("unknown analysis operation {operation}"));
    };
    let args = Arguments::new(arguments, accepted)?;
    let request_hash = qspquest_core::identity::canonical_hash(
        &json!({"operation": operation, "arguments": arguments}),
    )
    .map_err(|e| AnalysisError::new("invalid_input", e.message))?;
    let mut record = json!({
        "schema": PLAN_SCHEMA,
        "operation": operation,
        "analysis_version": super::ANALYSIS_VERSION,
        "request_hash": request_hash,
        "simulations": 0,
        "random_draws": 0,
    });
    if super::RECORD_OPERATIONS.contains(&operation) {
        if operation == "exposure_summary" {
            let result = RunOutput::from_value(args.required("result")?)?;
            let Some(output) = args.required("output")?.as_str() else {
                return fail("output must be a string");
            };
            if result.column(output).is_none() {
                return fail("unknown exposure output");
            }
            if let Some(v) = args.get("threshold") {
                finite(Some(v), "threshold")?;
            }
        } else if !args
            .required("fit")?
            .get("fitted_observations")
            .is_some_and(Value::is_array)
        {
            return fail("fit has no complete fitted observation rows");
        }
        record["stochastic"] = json!(false);
        record["max_simulations"] = json!(0);
        record["runs"] = json!([]);
        record["deferred_checks"] = json!([
            "Arithmetic on the supplied record; the record's own provenance is not re-established."
        ]);
        return Ok(record);
    }
    let Some(model) = model else {
        return fail(format!("{operation} requires a compiled model"));
    };
    let default = default_max_simulations(operation).expect("model operation");
    let max_simulations = args.int("max_simulations", default, 1, 1_000_000)?;
    // A comparison without variability is one deterministic subject.
    let deterministic =
        operation == "compare_regimens" && !super::exposure::comparison_is_stochastic(arguments);
    let mut values = serde_json::Map::new();
    if deterministic {
        if args.get("subjects").is_some_and(|v| v.as_u64() != Some(1)) {
            return fail("a comparison without omega has one deterministic subject; supply omega for virtual subjects");
        }
        if args.get("seed").is_some() {
            return fail(
                "a deterministic comparison draws no random numbers; seed applies only with omega",
            );
        }
        values.insert("subjects".into(), json!(1));
        values.insert("workers".into(), json!(args.int("workers", 1, 1, 8)?));
    } else {
        for (name, default, lower, upper) in controls(operation) {
            values.insert(
                name.to_string(),
                json!(args.int(name, *default, *lower, *upper)?),
            );
        }
    }
    // Specifications. Bounded parameters are fitted; named quantities are
    // typical values with lognormal random effects.
    let typical = matches!(operation, "visual_predictive_check" | "compare_regimens");
    let specs = if deterministic {
        if let Some(value) = args.get("percentiles") {
            percentiles(Some(value))?;
        }
        match args.get("parameters") {
            None => Vec::new(),
            Some(value) => fixed_specs(&named_quantities(value, "parameters")?)?,
        }
    } else if typical {
        let parameters = named_quantities(args.required("parameters")?, "parameters")?;
        let specs = fixed_specs(&parameters)?;
        random_effects(
            &specs,
            args.get("random_effects"),
            args.required("omega")?,
            if operation == "compare_regimens" {
                "comparison"
            } else {
                "VPC"
            },
        )?;
        percentiles(args.get("percentiles"))?;
        specs
    } else {
        parameter_specs(args.required("parameters")?)?
    };
    if operation != "compare_regimens" && operation != "event_sensitivities" {
        let error = ErrorSpec::parse(args.required("error")?)?;
        if operation == "visual_predictive_check"
            && (error.additive.fitted() || error.proportional.fitted())
        {
            return fail("VPC requires fitted or fixed numeric residual SDs");
        }
    }
    if matches!(operation, "fit_population" | "bootstrap_population") {
        // Both run the native analysis SAEM; mirror its admission rules.
        if specs.len() > 8 || specs.iter().any(|s| s.lower <= 0.0 || s.path.is_some()) {
            return fail("SAEM supports 1..8 positive model parameters; event controls are deterministic-fit only");
        }
        let d = specs.len();
        let om = matrix(args.required("omega")?, "omega")?;
        if om.len() != d || om.iter().any(|row| row.len() != d) {
            return fail("Omega must be a finite symmetric dimension-by-dimension matrix");
        }
    }
    if operation == "profile_likelihood" {
        let parameter = args.required("parameter")?.as_str().unwrap_or("");
        let Some(index) = specs.iter().position(|s| s.name == parameter) else {
            return fail("profile parameter is not selected for fitting");
        };
        let grid = numbers(args.required("grid")?, "grid")?;
        if !(3..=200).contains(&grid.len())
            || grid.windows(2).any(|w| w[1] <= w[0])
            || grid[0] < specs[index].lower
            || grid[grid.len() - 1] > specs[index].upper
        {
            return fail("profile grid must contain 3..200 strictly increasing values within the fitted parameter bounds");
        }
    }
    // Embedded runs.
    let mut embedded = Vec::new();
    let mut output = None;
    let mut required = None;
    match operation {
        "fit_data" | "profile_likelihood" | "event_sensitivities" => {
            embedded.push(Embedded {
                pointer: "/arguments/run".into(),
                run: args.required("run")?,
                observations: if operation == "event_sensitivities" {
                    None
                } else {
                    Some(args.required("observations")?)
                },
            });
            if operation == "event_sensitivities" {
                required = Some(1 + 4 * specs.len() as u64);
            }
        }
        "fit_population" | "bootstrap_population" | "visual_predictive_check" => {
            // SAEM needs dimension + 2 subjects; every interface admits 128 at most.
            let range = match operation {
                "visual_predictive_check" => 1..=128,
                _ => specs.len() + 2..=128,
            };
            let count = subjects(&args, &mut embedded, range)? as u64;
            if operation == "visual_predictive_check" {
                required = Some(count * (values["replicates"].as_u64().unwrap() + 1));
            }
        }
        _ => {
            let Some(items) = args
                .required("regimens")?
                .as_array()
                .filter(|r| (1..=25).contains(&r.len()))
            else {
                return fail("regimens must contain 1..25 labelled native requests");
            };
            let mut labels = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let at = pointer("/arguments/regimens", &index.to_string());
                let label = item.get("label").and_then(Value::as_str).unwrap_or("");
                let Some(run) = item.get("run") else {
                    return located(
                        &at,
                        fail("each regimen requires a distinct label and a run request"),
                    );
                };
                if label.is_empty() || labels.contains(&label) {
                    return located(
                        &at,
                        fail("each regimen requires a distinct label and a run request"),
                    );
                }
                labels.push(label);
                embedded.push(Embedded {
                    pointer: pointer(&at, "run"),
                    run,
                    observations: None,
                });
            }
            let Some(name) = args.required("output")?.as_str() else {
                return fail("output must be a string");
            };
            output = Some(name);
            if let Some(v) = args.get("threshold") {
                finite(Some(v), "threshold")?;
            }
            required = Some(values["subjects"].as_u64().unwrap() * items.len() as u64);
        }
    }
    if let Some(required) = required {
        if required > max_simulations {
            return fail_code(
                format!("{operation} needs exactly {required} simulations, more than max_simulations {max_simulations}"),
                "budget",
            );
        }
    }
    let runs = embedded
        .iter()
        .map(|e| prepare(model, e, &specs, output))
        .collect::<Result<Vec<_>>>()?;
    let stochastic = values.contains_key("seed");
    record["model_content_hash"] = json!(model.model_content_hash());
    record["stochastic"] = json!(stochastic);
    record["max_simulations"] = json!(max_simulations);
    record["max_simulations_from_request"] = json!(args.get("max_simulations").is_some());
    record["required_simulations"] = json!(required);
    record["controls"] = Value::Object(values);
    record["parameters"] = json!(specs
        .iter()
        .map(|s| json!({"name": s.name, "unit": s.unit, "initial": s.initial, "path": s.path}))
        .collect::<Vec<_>>());
    record["runs"] = json!(runs);
    record["deferred_checks"] = json!([
        "Every native integration, likelihood and optimizer step runs only at execution.",
        "Runs are prepared at the initial or typical parameter values; trial values, random effects and covariate effects are checked when they are simulated.",
        "Operation-specific numerical controls not listed in controls (tolerances, bins, SAEM burn-in and eta bound, variance bounds, covariate models, residual-error support, fit options) are checked at execution before the first simulation.",
        "Convergence, mixing and interval resolution are properties of the executed result.",
    ]);
    Ok(record)
}

/// Prepare a `qspquest.analysis/v0.2` envelope; see [`plan`].
pub fn plan_request(model: Option<&Arc<CompiledDocument>>, request: &Value) -> Result<Value> {
    let (operation, arguments) = super::envelope(request)?;
    plan(model, operation, &arguments)
}

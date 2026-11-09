//! General bounded SAEM orchestrated over native simulations.
//!
//! Componentwise random-walk Metropolis–Hastings samples log individual
//! parameters. Stochastic approximation tracks their first and second
//! moments. The fixed-effect and covariate M-step is bounded generalized least
//! squares; the residual-error M-step minimizes a gain-weighted expected
//! complete-data objective. Censored rows enter that objective through the
//! analytic conditional second moment of the truncated normal residual.
use super::common::*;
use super::control::Control;
use super::linalg::{self, Matrix};
use super::rng::{substream, RANDOM_STREAM};
use super::saem;
use super::stats;
use crate::document::CompiledDocument;
use serde_json::{json, Value};
use std::sync::Arc;

const LOG_2PI: f64 = 1.837_877_066_409_345_5;

struct Covariate {
    parameter: String,
    covariate: String,
    initial: f64,
    lower: f64,
    upper: f64,
    axis: usize,
}

pub const POPULATION_ARGUMENTS: [&str; 13] = [
    "subjects",
    "parameters",
    "omega",
    "error",
    "covariates",
    "covariance",
    "variance_bounds",
    "seed",
    "max_iterations",
    "burn_in",
    "sweeps",
    "tolerance",
    "max_simulations",
];

struct Subject<'a> {
    run: &'a Value,
    observations: &'a Value,
}

/// Subjects evaluated by native simulation of their run requests.
struct NativeCohort<'a> {
    model: &'a Arc<CompiledDocument>,
    specs: &'a [Spec],
    subjects: &'a [Subject<'a>],
    budget: &'a Budget,
    /// Native identity of the latest simulation.
    native: std::cell::RefCell<Value>,
}

impl NativeCohort<'_> {
    fn full(&self, i: usize, log_values: &[f64]) -> Result<(Observed, RunOutput)> {
        let bound: Vec<(&Spec, f64)> = self
            .specs
            .iter()
            .zip(log_values.iter().map(|v| v.exp()))
            .collect();
        let result = self
            .budget
            .run(self.model, &bind(self.subjects[i].run, &bound)?)?;
        let observed = observations(&result, self.subjects[i].observations)?;
        self.native.replace(result.identity.clone());
        Ok((observed, result))
    }
}

impl saem::Cohort for NativeCohort<'_> {
    fn len(&self) -> usize {
        self.subjects.len()
    }
    fn evaluate(&self, i: usize, log_values: &[f64], e: &ErrorValues) -> Result<saem::Draw> {
        let (observed, _) = self.full(i, log_values)?;
        let nll = likelihood(&observed.y, &observed.prediction, &observed.censored, e)?.value;
        Ok(saem::Draw {
            nll,
            y: observed.y,
            prediction: observed.prediction,
            censored: observed.censored,
        })
    }
}

/// SAEM with 1..8 positive lognormal effects and exponential covariates.
///
/// The reported objective is a complete-data surrogate, never a marginal OFV.
/// Convergence is a stationarity and mixing screen, not proof of a global optimum.
pub fn fit_population(
    model: &Arc<CompiledDocument>,
    arguments: &Value,
    control: &mut dyn Control,
    shared_budget: Option<&Budget>,
) -> Result<Value> {
    let args = Arguments::new(arguments, &POPULATION_ARGUMENTS)?;
    let specs = parameter_specs(args.required("parameters")?)?;
    let err = ErrorSpec::parse(args.required("error")?)?;
    let d = specs.len();
    if d > 8 || specs.iter().any(|s| s.lower <= 0.0 || s.path.is_some()) {
        return fail("SAEM supports 1..8 positive model parameters; event controls are deterministic-fit only");
    }
    let subjects_value = args.required("subjects")?;
    let Some(subject_items) = subjects_value
        .as_array()
        .filter(|s| (d + 2..=128).contains(&s.len()))
    else {
        return fail("SAEM needs at least dimension+2 subjects, maximum 128");
    };
    let mut subjects = Vec::new();
    let mut ids: Vec<Value> = Vec::new();
    for (i, s) in subject_items.iter().enumerate() {
        let (Some(run), Some(obs)) = (s.get("run"), s.get("observations")) else {
            return fail("each subject must provide run and observations");
        };
        subjects.push(Subject {
            run,
            observations: obs,
        });
        let id = s.get("id").cloned().unwrap_or(json!(i));
        if !(id.is_string() || id.is_u64() || id.is_i64()) || ids.contains(&id) {
            return fail("subject IDs must be distinct strings or integers");
        }
        ids.push(id);
    }
    let seed = args.int("seed", 1009, 0, MAX_SEED)?;
    let max_iterations = args.int("max_iterations", 600, 40, 2000)? as usize;
    let burn_in = args.int("burn_in", 150, 10, max_iterations as u64 - 20)? as usize;
    let sweeps = args.int("sweeps", 3, 1, 20)? as usize;
    let tolerance = args.float("tolerance", 0.01)?;
    if !(tolerance > 0.0 && tolerance <= 0.1) {
        return fail("SAEM tolerance must be in (0, .1]");
    }
    let covariance = args.string("covariance", "full")?;
    if covariance != "full" && covariance != "diagonal" {
        return fail("covariance must be full or diagonal");
    }
    let diagonal = covariance == "diagonal";
    let bounds_error = || {
        AnalysisError::new(
            "invalid_input",
            "variance_bounds must be positive ordered pairs",
        )
    };
    let variance_bounds: Vec<[f64; 2]> = match args.get("variance_bounds") {
        None => vec![[1e-4, 2.0]; d],
        Some(v) => {
            let items = v.as_array().ok_or_else(bounds_error)?;
            if items.len() == 2 && items.iter().all(Value::is_number) {
                let pair = numbers(v, "variance_bounds").map_err(|_| bounds_error())?;
                vec![[pair[0], pair[1]]; d]
            } else {
                let rows = matrix(v, "variance_bounds").map_err(|_| bounds_error())?;
                if rows.len() != d || rows.iter().any(|r| r.len() != 2) {
                    return Err(bounds_error());
                }
                rows.iter().map(|r| [r[0], r[1]]).collect()
            }
        }
    };
    if variance_bounds.iter().any(|b| !(b[0] > 0.0) || b[1] < b[0]) {
        return Err(bounds_error());
    }
    let om = matrix(args.required("omega")?, "omega").map_err(|_| {
        AnalysisError::new(
            "invalid_input",
            "Omega must be a finite symmetric dimension-by-dimension matrix",
        )
    })?;
    if !linalg::is_symmetric(&om, d, 1e-12) {
        return fail("Omega must be a finite symmetric dimension-by-dimension matrix");
    }
    if linalg::cholesky(&om).is_none() {
        return fail("Omega must be positive definite");
    }
    if (0..d).any(|j| om[j][j] < variance_bounds[j][0] || om[j][j] > variance_bounds[j][1]) {
        return fail("initial Omega is outside variance bounds");
    }
    if diagonal && (0..d).any(|i| (0..d).any(|j| i != j && om[i][j] != 0.0)) {
        return fail("diagonal covariance requires diagonal initial Omega");
    }
    let n = subjects.len();
    let control_items: Vec<Value> = match args.get("covariates") {
        None => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(_) => return fail("covariates must be an array"),
    };
    if control_items.len() > 8 || n <= control_items.len() + d {
        return fail(
            "at most eight covariate coefficients with an informative cohort are supported",
        );
    }
    let m = control_items.len();
    let mut x = vec![vec![0.0; m]; n];
    let mut controls = Vec::new();
    for (col, item) in control_items.iter().enumerate() {
        let keys: std::collections::BTreeSet<&str> = item
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        let expected: std::collections::BTreeSet<&str> = [
            "parameter",
            "covariate",
            "reference",
            "initial",
            "lower",
            "upper",
        ]
        .into_iter()
        .collect();
        let parameter = item.get("parameter").and_then(Value::as_str).unwrap_or("");
        let axis = specs.iter().position(|s| s.name == parameter);
        let (true, Some(axis)) = (keys == expected, axis) else {
            return fail(
                "covariate requires parameter, covariate, reference, initial, lower and upper",
            );
        };
        let lower = finite(item.get("lower"), "covariate.lower")?;
        let upper = finite(item.get("upper"), "covariate.upper")?;
        let initial = finite(item.get("initial"), "covariate.initial")?;
        if !(lower < upper) || !(lower <= initial && initial <= upper) {
            return fail("invalid covariate coefficient bounds");
        }
        let reference = finite(item.get("reference"), "covariate.reference")?;
        let Some(covariate) = item.get("covariate").and_then(Value::as_str) else {
            return fail(
                "covariate requires parameter, covariate, reference, initial, lower and upper",
            );
        };
        for (i, s) in subject_items.iter().enumerate() {
            x[i][col] = finite(
                s.get("covariates").and_then(|c| c.get(covariate)),
                "subject covariate",
            )? - reference;
        }
        let largest = x.iter().map(|r| r[col].abs()).fold(0.0, f64::max);
        if largest * lower.abs().max(upper.abs()) > 8.0 {
            return fail("covariate effect exceeds the supported log range");
        }
        controls.push(Covariate {
            parameter: parameter.to_string(),
            covariate: covariate.to_string(),
            initial,
            lower,
            upper,
            axis,
        });
    }
    let pairs: std::collections::BTreeSet<(&str, &str)> = controls
        .iter()
        .map(|c| (c.parameter.as_str(), c.covariate.as_str()))
        .collect();
    if pairs.len() != controls.len() {
        return fail("duplicate covariate effects");
    }
    for axis in controls
        .iter()
        .map(|c| c.axis)
        .collect::<std::collections::BTreeSet<_>>()
    {
        let columns: Vec<usize> = (0..m).filter(|&j| controls[j].axis == axis).collect();
        let design: Matrix = (0..n)
            .map(|i| {
                std::iter::once(1.0)
                    .chain(columns.iter().map(|&j| x[i][j]))
                    .collect()
            })
            .collect();
        if linalg::rank(&design) != columns.len() + 1 {
            return fail("covariate design is rank deficient");
        }
    }
    let error_keys = err.fitted_keys();
    let row_count: usize = subjects
        .iter()
        .map(|s| s.observations.as_array().map_or(0, Vec::len))
        .sum();
    if !(1..=10000).contains(&row_count)
        || row_count * (max_iterations - burn_in) > saem::MAX_EXPECTATION_ITEMS
    {
        return fail("cohort or stochastic expectation storage exceeds bounded limits");
    }
    let all_rows = || {
        subjects
            .iter()
            .flat_map(|s| s.observations.as_array().into_iter().flatten())
    };
    let outputs: std::collections::BTreeSet<&str> = all_rows()
        .filter_map(|o| o.get("output").and_then(Value::as_str))
        .collect();
    if outputs.len() != 1 {
        return fail("one residual error contract requires one observed output");
    }
    if !error_keys.is_empty()
        && all_rows().all(|o| o.get("censoring").and_then(Value::as_str) == Some("left"))
    {
        return fail("all-censored cohorts cannot estimate residual error");
    }
    let owned;
    let budget = match shared_budget {
        Some(b) => b,
        None => {
            owned = Budget::new(
                args.int("max_simulations", 200000, 1, 1_000_000)?,
                control.cancel_flag(),
            )?;
            &owned
        }
    };
    let before = budget.used();
    let problem = saem::Problem {
        log_initial: specs.iter().map(|s| s.initial.ln()).collect(),
        log_bounds: specs.iter().map(|s| (s.lower.ln(), s.upper.ln())).collect(),
        covariates: controls
            .iter()
            .map(|c| saem::CovariateAxis {
                axis: c.axis,
                initial: c.initial,
                lower: c.lower,
                upper: c.upper,
            })
            .collect(),
        x,
        omega: om,
        variance_bounds: variance_bounds.clone(),
        diagonal,
        sweeps,
        burn_in,
        max_iterations,
        tolerance,
        update_tolerance: None,
        error: err,
        eta_bound: None,
        variance_scale: saem::VarianceScale::Log,
        max_correlation: None,
        mixing_lag_limit: None,
    };
    let cohort = NativeCohort {
        model,
        specs: &specs,
        subjects: &subjects,
        budget,
        native: std::cell::RefCell::new(Value::Null),
    };
    let mut context_inputs = arguments.clone();
    context_inputs["seed"] = json!(seed.to_string());
    let request_identity = identity(model, &context_inputs)?;
    let mut native_identity = Value::Null;
    let mut context = || -> Result<Value> {
        native_identity = cohort.native.borrow().clone();
        Ok(json!({
            "request": request_identity,
            "native": native_identity,
            "random_stream": RANDOM_STREAM,
            "convergence_window": "cumulative-gain/v1",
            "initial_chain_equilibration": "first-half-of-burn-in",
            "sampler": "componentwise-and-adaptive-joint-metropolis/v1",
            "residual_mstep": "streamed-closed-form-or-projected-newton/v2",
        }))
    };
    let outcome = saem::run(
        &problem,
        &cohort,
        substream(seed, "fit_population", 0).stream,
        budget,
        control,
        "population_fit",
        &mut context,
    )?;
    let saem::Outcome {
        mut status,
        coefficients,
        phi,
        mean,
        omega: om,
        error: current_error,
        likelihoods,
        proposed,
        accepted,
        subject_accepted,
        trace,
        stable_checks,
        ref max_lag_one,
        ..
    } = outcome;
    let theta: Vec<f64> = coefficients[..d].iter().map(|v| v.exp()).collect();
    let loc = saem::location(&problem, &coefficients);
    let eta: Matrix = mean
        .iter()
        .zip(&loc)
        .map(|(m, l)| m.iter().zip(l).map(|(a, b)| a - b).collect())
        .collect();
    let mut rows = Vec::new();
    let mut individual_parameters = Vec::new();
    // Predictions use posterior mean log parameters, explicitly not conditional modes.
    if status != "evaluation_limit" {
        let finished = (|| -> Result<()> {
            for i in 0..n {
                let (individual, result) = cohort.full(i, &mean[i])?;
                let (population, _) = cohort.full(i, &loc[i])?;
                individual_parameters.push(Value::Object(
                    specs
                        .iter()
                        .zip(&mean[i])
                        .map(|(s, v)| (s.name.clone(), json!(v.exp())))
                        .collect(),
                ));
                let fitted = likelihood(
                    &individual.y,
                    &individual.prediction,
                    &individual.censored,
                    &current_error,
                )?;
                for (j, observation) in subjects[i]
                    .observations
                    .as_array()
                    .expect("validated")
                    .iter()
                    .enumerate()
                {
                    let row = observation["row"].as_u64().expect("validated row") as usize;
                    let censored = individual.censored[j];
                    let mut record = json!({
                        "subject_id": ids[i],
                        "subject_index": i,
                        "observation_index": j,
                        "row": row,
                        "output": observation["output"],
                        "unit": observation["value"]["unit"],
                        "time": result.times[row],
                        "time_unit": result.time_unit,
                        "side": result.sides[row],
                        "observed": individual.y[j],
                        "prediction": population.prediction[j],
                        "individual_prediction": individual.prediction[j],
                        "censoring": observation.get("censoring").cloned().unwrap_or(Value::Null),
                        "residual": if censored { Value::Null } else { json!(fitted.residual[j]) },
                        "standardized_residual": if censored { Value::Null } else { json!(fitted.residual[j] / fitted.sd[j]) },
                    });
                    if let Some(source) = observation.get("source_row") {
                        record["source_row"] = source.clone();
                    }
                    rows.push(record);
                }
            }
            Ok(())
        })();
        match finished {
            Ok(()) => {}
            Err(error) if error.code == "budget" => {
                status = "evaluation_limit".to_string();
                rows.clear();
                individual_parameters.clear();
            }
            Err(error) => return Err(error),
        }
    }
    let max_lag: Vec<Value> = max_lag_one
        .iter()
        .map(|v| v.map_or(Value::Null, |v| json!(v)))
        .collect();
    let shrinkage: Vec<f64> = (0..d)
        .map(|a| 1.0 - stats::variance(&eta.iter().map(|e| e[a]).collect::<Vec<_>>(), 1) / om[a][a])
        .collect();
    let precision = linalg::inverse_spd(&om).expect("validated covariance");
    let log_det = linalg::log_det_spd(&om).expect("validated covariance");
    let prior: f64 = (0..n)
        .map(|i| {
            let r: Vec<f64> = phi[i].iter().zip(&loc[i]).map(|(a, b)| a - b).collect();
            linalg::quadratic_form(&r, &precision, &r)
        })
        .sum();
    let surrogate = likelihoods.iter().sum::<f64>()
        + 0.5 * prior
        + n as f64 / 2.0 * (log_det + d as f64 * LOG_2PI);
    let covariate_effects: Vec<Value> = controls
        .iter()
        .zip(&control_items)
        .enumerate()
        .map(|(j, (_, item))| {
            let mut record = item.clone();
            record["estimate"] = json!(coefficients[d + j]);
            record
        })
        .collect();
    let mut identity = request_identity;
    identity["native"] = native_identity;
    Ok(json!({
        "schema": "qspquest.population-analysis/v0.2",
        "method": "saem_native_simulation",
        "status": status,
        "parameters": scalar_parameters(&specs, &theta),
        "random_effects": specs.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        "omega": om,
        "error": current_error.to_value(),
        "covariate_effects": covariate_effects,
        "eta": eta,
        "subject_ids": ids,
        "individual_parameters": individual_parameters,
        "individual_estimate_kind": "posterior_mean_log_parameter",
        "objective": surrogate,
        "objective_kind": "saem_complete_data_surrogate",
        "censored_residual_step": "analytic_truncated_normal_second_moment",
        "random_stream": RANDOM_STREAM,
        "simulations": budget.used() - before,
        "iterations": trace.len(),
        "trace": trace,
        "fitted_observations": rows,
        "diagnostics": {
            "acceptance_by_axis": (0..d).map(|a| accepted[a] as f64 / proposed[a].max(1) as f64).collect::<Vec<_>>(),
            "minimum_subject_accepted_by_axis": (0..d).map(|a| subject_accepted.iter().map(|r| r[a]).min().unwrap_or(0)).collect::<Vec<_>>(),
            "max_subject_lag_one_correlation": max_lag,
            "eta_variance_shrinkage": shrinkage,
            "stable_checks": stable_checks,
        },
        "boundary_parameters": specs.iter().zip(&theta).filter(|(s, v)| near_bound(**v, s.lower, s.upper)).map(|(s, _)| s.name.clone()).collect::<Vec<_>>(),
        "boundary_residual_errors": error_keys.iter().filter(|k| { let (lo, hi) = err.sd(k).bounds(); near_bound(current_error.get(k), lo, hi) }).copied().collect::<Vec<_>>(),
        "boundary_covariate_coefficients": controls.iter().enumerate().filter(|(j, c)| near_bound(coefficients[d + j], c.lower, c.upper)).map(|(j, _)| j).collect::<Vec<_>>(),
        "boundary_variances": (0..d).filter(|&j| variance_bounds[j][0] < variance_bounds[j][1] && near_bound(om[j][j], variance_bounds[j][0], variance_bounds[j][1])).map(|j| specs[j].name.clone()).collect::<Vec<_>>(),
        "identity": identity,
    }))
}

/// Population fit for a resampled cohort inside a bootstrap; no control.
pub fn fit_with_budget(
    model: &Arc<CompiledDocument>,
    arguments: &Value,
    budget: &Budget,
) -> Result<Value> {
    fit_population(
        model,
        arguments,
        &mut super::control::NoControl,
        Some(budget),
    )
}

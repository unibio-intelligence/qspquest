//! Native analysis workflows against closed-form oral absorption kinetics.
use qspquest_modeling::analysis::{execute, NoControl};
use qspquest_modeling::document::CompiledDocument;
use serde_json::{json, Value};
use std::sync::Arc;

fn model() -> Arc<CompiledDocument> {
    qspquest_modeling::language::parse(include_str!(
        "../../../../examples/synthetic/absorption.qsp"
    ))
    .unwrap()
    .compile()
    .unwrap()
}

fn run(times: &[f64]) -> Value {
    json!({
        "schema": "qspquest.run/v0.1",
        "solver": "analytic_linear_pk",
        "budgets": {"solver_callbacks": 1000000, "events": 1000, "output_values": 100000},
        "regimen": {
            "end": {"value": times[times.len() - 1], "unit": "h"},
            "samples": [],
            "observations": times.iter().map(|t| json!({"time": {"value": t, "unit": "h"}, "side": "post"})).collect::<Vec<_>>(),
            "administrations": [{
                "target": "depot",
                "time": {"value": 0.0, "unit": "h"},
                "amount": {"value": 320.0, "unit": "mg"},
                "delivery": {"kind": "bolus"},
                "bioavailability": 1.0
            }],
            "rtol": 1e-9,
            "atol": 1e-11
        }
    })
}

/// Oral one-compartment concentration with first-order absorption.
fn oral(t: f64, cl: f64, vc: f64, ka: f64) -> f64 {
    let k = cl / vc;
    320.0 / vc * ka / (ka - k) * ((-k * t).exp() - (-ka * t).exp())
}

const TIMES: [f64; 8] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 24.0];

fn exact_observations() -> Value {
    Value::Array(
        TIMES
            .iter()
            .enumerate()
            .map(|(i, t)| json!({"row": i, "output": "cp", "value": {"value": oral(*t, 0.8, 6.0, 1.2), "unit": "mg/L"}}))
            .collect(),
    )
}

fn bounds() -> Value {
    json!([
        {"name": "cl", "initial": 2.0, "lower": 0.1, "upper": 5.0, "unit": "L/h"},
        {"name": "vc", "initial": 28.0, "lower": 0.5, "upper": 50.0, "unit": "L"},
        {"name": "ka", "initial": 1.0, "lower": 0.2, "upper": 3.0, "unit": "1/h"}
    ])
}

fn call(
    operation: &str,
    arguments: Value,
) -> Result<Value, qspquest_modeling::analysis::AnalysisError> {
    execute(Some(&model()), operation, &arguments, &mut NoControl)
}

#[test]
fn both_fitting_methods_recover_closed_form_parameters() {
    for method in ["gradient", "derivative_free"] {
        let fit = call(
            "fit_data",
            json!({
                "run": run(&TIMES),
                "observations": exact_observations(),
                "parameters": bounds(),
                "error": {"additive": 0.1},
                "method": method,
                "max_simulations": 20000,
                "max_iterations": 3000,
            }),
        )
        .unwrap();
        assert_eq!(fit["status"], "converged", "{method}: {}", fit["message"]);
        for (name, truth) in [("cl", 0.8), ("vc", 6.0), ("ka", 1.2)] {
            let estimate = fit["parameters"][name]["value"].as_f64().unwrap();
            assert!(
                ((estimate - truth) / truth).abs() < 1e-4,
                "{method} {name} {estimate}"
            );
        }
    }
}

#[test]
fn fit_budget_exhaustion_keeps_the_best_evaluated_point() {
    let fit = call(
        "fit_data",
        json!({
            "run": run(&TIMES),
            "observations": exact_observations(),
            "parameters": bounds(),
            "error": {"additive": 0.1},
            "max_simulations": 2,
        }),
    )
    .unwrap();
    assert_eq!(fit["status"], "evaluation_limit");
    assert_eq!(fit["simulations"], 2);
    assert!(fit["objective"].as_f64().unwrap() <= fit["initial_objective"].as_f64().unwrap());
}

#[test]
fn profile_objective_equals_the_closed_form_likelihood() {
    let parameters =
        json!([{"name": "cl", "initial": 2.0, "lower": 0.1, "upper": 5.0, "unit": "L/h"}]);
    let mut fixed = run(&TIMES);
    fixed["parameters"] =
        json!({"vc": {"value": 6.0, "unit": "L"}, "ka": {"value": 1.2, "unit": "1/h"}});
    let profile = call(
        "profile_likelihood",
        json!({
            "run": fixed,
            "observations": exact_observations(),
            "parameters": parameters,
            "error": {"additive": 0.1},
            "parameter": "cl",
            "grid": [0.7, 0.75, 0.8, 0.85, 0.9],
        }),
    )
    .unwrap();
    assert_eq!(profile["status"], "complete");
    let interval = profile["interval"].as_array().unwrap();
    assert!(interval[0].as_f64().unwrap() < 0.8 && 0.8 < interval[1].as_f64().unwrap());
    for row in profile["profile"].as_array().unwrap() {
        let cl = row["value"].as_f64().unwrap();
        let expected: f64 = TIMES
            .iter()
            .map(|t| {
                let r = (oral(*t, 0.8, 6.0, 1.2) - oral(*t, cl, 6.0, 1.2)) / 0.1;
                0.5 * r * r + (0.1 * (2.0 * std::f64::consts::PI).sqrt()).ln()
            })
            .sum();
        assert!((row["objective"].as_f64().unwrap() - expected).abs() < 1e-8);
    }
    // chi-squared(1) 95% quantile divided by two.
    assert!(
        (profile["delta_nll_threshold"].as_f64().unwrap() - 1.920_729_410_347_062).abs() < 1e-12
    );
}

#[test]
fn lag_derivative_matches_the_analytic_time_derivative() {
    let mut lagged = run(&TIMES);
    lagged["regimen"]["administrations"][0]["lag"] = json!({"value": 0.1, "unit": "h"});
    let result = call(
        "event_sensitivities",
        json!({
            "run": lagged,
            "parameters": [{"name": "lag", "initial": 0.1, "lower": 0.0, "upper": 0.2, "unit": "h",
                            "path": ["regimen", "administrations", 0, "lag"]}],
        }),
    )
    .unwrap();
    let values = result["derivatives"]["lag"]["cp"]["values"]
        .as_array()
        .unwrap();
    let (cl, vc, ka) = (0.8, 6.0, 1.2);
    let k = cl / vc;
    for (t, d) in TIMES.iter().zip(values) {
        let s = t - 0.1;
        let analytic = 320.0 / vc * ka / (ka - k) * (k * (-k * s).exp() - ka * (-ka * s).exp());
        assert!((d.as_f64().unwrap() - analytic).abs() < 1e-6 + 1e-6 * analytic.abs());
    }
}

#[test]
fn exposure_summary_integrates_the_sampled_trajectory() {
    let result = json!({
        "times": [0.0, 1.0, 1.0, 2.0],
        "sides": ["post", "pre", "post", "pre"],
        "time_unit": "h",
        "outputs": [{"name": "cp", "unit": "mg/L", "values": [0.0, 0.0, 4.0, 2.0]}],
    });
    let summary = execute(
        None,
        "exposure_summary",
        &json!({"result": result, "output": "cp", "threshold": 3.0}),
        &mut NoControl,
    )
    .unwrap();
    assert_eq!(summary["auc"], 3.0);
    assert_eq!(summary["tmax"], 1.0);
    assert_eq!(summary["time_above_threshold"]["duration"], 0.5);
    let error = execute(None, "fit_data", &json!({}), &mut NoControl).unwrap_err();
    assert!(error.message.contains("requires a compiled model"));
    let error = execute(None, "unknown", &json!({}), &mut NoControl).unwrap_err();
    assert_eq!(error.code, "invalid_input");
}

fn cohort() -> Value {
    let cls = [0.6, 0.7, 0.8, 0.9, 1.0];
    Value::Array(
        cls.iter()
            .enumerate()
            .map(|(i, cl)| {
                json!({
                    "id": format!("s{i}"),
                    "run": run(&TIMES),
                    "observations": TIMES.iter().enumerate().map(|(j, t)| json!({
                        "row": j, "output": "cp", "value": {"value": oral(*t, *cl, 6.0, 1.2), "unit": "mg/L"}
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

#[test]
fn predictive_check_is_independent_of_worker_count_and_respects_the_budget() {
    let arguments = |workers: u64| {
        json!({
            "subjects": cohort(),
            "parameters": [{"name": "cl", "value": 0.8, "unit": "L/h"}, {"name": "vc", "value": 6.0, "unit": "L"}, {"name": "ka", "value": 1.2, "unit": "1/h"}],
            "random_effects": ["cl"],
            "omega": [[0.04]],
            "error": {"additive": 0.05},
            "replicates": 20,
            "seed": 11,
            "workers": workers,
        })
    };
    let serial = call("visual_predictive_check", arguments(1)).unwrap();
    for workers in [3, 8] {
        let parallel = call("visual_predictive_check", arguments(workers)).unwrap();
        assert_eq!(parallel["bins"], serial["bins"]);
        assert_eq!(parallel["replicate_records"], serial["replicate_records"]);
        assert_eq!(parallel["simulations"], serial["simulations"]);
    }
    assert_eq!(serial["simulations"], 5 * 21);
    let mut short = arguments(8);
    short["max_simulations"] = json!(5 * 21 - 1);
    assert_eq!(
        call("visual_predictive_check", short).unwrap_err().code,
        "budget"
    );
}

#[test]
fn paired_regimen_comparison_scales_exposure_with_dose() {
    let mut double = run(&TIMES);
    double["regimen"]["administrations"][0]["amount"]["value"] = json!(640.0);
    let comparison = call(
        "compare_regimens",
        json!({
            "regimens": [{"label": "base", "run": run(&TIMES)}, {"label": "double", "run": double}],
            "parameters": [{"name": "cl", "value": 0.8, "unit": "L/h"}, {"name": "vc", "value": 6.0, "unit": "L"}, {"name": "ka", "value": 1.2, "unit": "1/h"}],
            "omega": [[0.04, 0.0, 0.0], [0.0, 0.04, 0.0], [0.0, 0.0, 0.04]],
            "output": "cp",
            "subjects": 6,
            "workers": 4,
        }),
    )
    .unwrap();
    assert_eq!(comparison["reference"], "base");
    let base = comparison["regimens"]["base"]["subject_metrics"]
        .as_array()
        .unwrap();
    let double = comparison["regimens"]["double"]["subject_metrics"]
        .as_array()
        .unwrap();
    for (a, b) in base.iter().zip(double) {
        let (a, b) = (a["auc"].as_f64().unwrap(), b["auc"].as_f64().unwrap());
        assert!((b - 2.0 * a).abs() <= 1e-10 * b);
    }
}

#[test]
fn population_fit_recovers_typical_clearance_and_is_repeatable() {
    let arguments = json!({
        "subjects": cohort(),
        "parameters": [{"name": "cl", "initial": 1.5, "lower": 0.1, "upper": 5.0, "unit": "L/h"}],
        "omega": [[0.04]],
        "error": {"additive": 0.05},
        "max_iterations": 200,
        "burn_in": 50,
        "sweeps": 2,
        "tolerance": 0.03,
        "seed": 5,
    });
    let first = call("fit_population", arguments.clone()).unwrap();
    let second = call("fit_population", arguments).unwrap();
    assert_eq!(first["parameters"], second["parameters"]);
    assert_eq!(first["trace"], second["trace"]);
    let cl = first["parameters"]["cl"]["value"].as_f64().unwrap();
    // Geometric mean of the subject clearances.
    let truth = (0.6f64 * 0.7 * 0.8 * 0.9 * 1.0).powf(0.2);
    assert!(((cl - truth) / truth).abs() < 0.1, "cl={cl}");
    assert_eq!(first["objective_kind"], "saem_complete_data_surrogate");
}

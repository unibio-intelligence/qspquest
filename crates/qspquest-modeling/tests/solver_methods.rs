use qspquest_core::{
    fit::FitRequest,
    model::ModelDocument,
    regimen::ObservationSide,
    run::{AccuracyControl, RunRequest, SensitivityRequest, Solver},
    ErrorCode,
};
use qspquest_modeling::{document::sensitivity::CompiledSensitivityDocument, language};
use serde_json::json;

const MODEL: &str = include_str!("../../../conformance/models/synthetic-one-compartment.qsp");

fn request() -> RunRequest {
    serde_json::from_value(json!({
        "schema": "qspquest.run/v0.1",
        "solver": "diffsol_bdf",
        "budgets": {"solver_callbacks": 1000000, "output_values": 10000, "events": 100},
        "regimen": {
            "end": {"value": 3, "unit": "h"},
            "samples": [
                {"value": 0.5, "unit": "h"},
                {"value": 1.5, "unit": "h"},
                {"value": 2, "unit": "h"},
                {"value": 2.5, "unit": "h"}
            ],
            "administrations": [
                {"target":"central", "time":{"value":0,"unit":"h"}, "amount":{"value":6,"unit":"mg"}, "delivery":{"kind":"bolus"}, "bioavailability":1},
                {"target":"central", "time":{"value":1,"unit":"h"}, "amount":{"value":3,"unit":"mg"}, "delivery":{"kind":"infusion","span":{"kind":"duration","duration":{"value":1,"unit":"h"}}}, "bioavailability":1}
            ],
            "rtol": 1e-8,
            "atol": 1e-10
        }
    })).unwrap()
}

fn expected_amount(time: f64, k: f64) -> f64 {
    let bolus = 6.0 * (-k * time).exp();
    let infusion = if time <= 1.0 {
        0.0
    } else if time <= 2.0 {
        3.0 * (-(-k * (time - 1.0)).exp_m1()) / k
    } else {
        3.0 * (-(-k).exp_m1()) * (-k * (time - 2.0)).exp() / k
    };
    bolus + infusion
}

#[test]
fn all_diffsol_methods_follow_bolus_and_infusion_boundaries() {
    let compiled = language::parse(MODEL).unwrap().compile().unwrap();
    let mut request = request();
    let choices = [
        (Solver::DiffsolBdf, "bdf"),
        (Solver::DiffsolTsit45, "tsit45"),
        (Solver::DiffsolEsdirk34, "esdirk34"),
        (Solver::DiffsolTrBdf2, "tr_bdf2"),
        (Solver::DiffsolRosenbrock23, "rosenbrock23"),
        (Solver::DiffsolRodas5p, "rodas5p"),
    ];
    let mut hashes = std::collections::BTreeSet::new();
    for accuracy in [AccuracyControl::Local, AccuracyControl::Sampled] {
        request.accuracy_control = accuracy;
        for (solver, algorithm) in choices {
            request.solver = solver;
            let result = compiled.execute(&request).unwrap();
            assert_eq!(result.identity.backend, "diffsol");
            assert!(result
                .identity
                .backend_version
                .starts_with("0.18.0+sha256:"));
            assert_eq!(result.identity.algorithm, algorithm);
            assert!(hashes.insert(result.identity.run_request_hash));
            for (i, &time) in result.times.iter().enumerate() {
                let actual = result.outputs[0].values[i];
                let expected_amount = if time == 0.0 && result.sides[i] == ObservationSide::Pre {
                    0.0
                } else {
                    expected_amount(time, 0.1)
                };
                let expected = expected_amount / 3.0;
                assert!(
                    (actual - expected).abs() < 3e-5,
                    "{algorithm} at {time}: {actual} vs {expected}"
                );
            }
            let at_zero: Vec<_> = result
                .times
                .iter()
                .enumerate()
                .filter(|(_, t)| **t == 0.0)
                .collect();
            assert_eq!(at_zero.len(), 2);
            assert_eq!(result.sides[at_zero[0].0], ObservationSide::Pre);
            assert_eq!(result.sides[at_zero[1].0], ObservationSide::Post);
        }
    }
}

#[test]
fn implicit_runge_kutta_methods_resolve_stiff_dose_response() {
    // CL/V = 100 h^-1. The bolus decays over a hundredth of an hour while
    // the one-hour infusion approaches a small but nonzero plateau.
    let source = MODEL.replace("cl = 0.3 [L/h]", "cl = 300 [L/h]");
    let compiled = language::parse(&source).unwrap().compile().unwrap();
    let mut request = request();
    request.regimen.samples = serde_json::from_value(json!([
        {"value":0.01,"unit":"h"},
        {"value":0.1,"unit":"h"},
        {"value":1.01,"unit":"h"},
        {"value":1.5,"unit":"h"},
        {"value":2.01,"unit":"h"}
    ]))
    .unwrap();
    for solver in [
        Solver::DiffsolBdf,
        Solver::DiffsolEsdirk34,
        Solver::DiffsolTrBdf2,
        Solver::DiffsolRosenbrock23,
        Solver::DiffsolRodas5p,
    ] {
        request.solver = solver;
        let result = compiled.execute(&request).unwrap();
        for (i, &time) in result.times.iter().enumerate() {
            if time == 0.0 && result.sides[i] == ObservationSide::Pre {
                continue;
            }
            let expected = expected_amount(time, 100.0) / 3.0;
            let actual = result.outputs[0].values[i];
            assert!(
                (actual - expected).abs() < 2e-4,
                "{} at {time}: {actual} vs {expected}",
                result.identity.algorithm
            );
        }
    }
}

#[test]
fn solver_names_are_typed_and_unknown_methods_are_rejected() {
    for name in [
        "diffsol_bdf",
        "diffsol_tsit45",
        "diffsol_esdirk34",
        "diffsol_tr_bdf2",
        "diffsol_rosenbrock23",
        "diffsol_rodas5p",
    ] {
        let mut value = serde_json::to_value(request()).unwrap();
        value["solver"] = json!(name);
        let parsed: RunRequest = serde_json::from_value(value).unwrap();
        assert_eq!(serde_json::to_value(parsed.solver).unwrap(), json!(name));
    }
    let mut value = serde_json::to_value(request()).unwrap();
    value["solver"] = json!("rosenbrock");
    assert!(serde_json::from_value::<RunRequest>(value).is_err());
}

#[test]
fn unsupported_requests_do_not_fall_back_to_bdf_for_fit_or_sensitivity() {
    let model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let mut sensitivity: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    let mut fit: FitRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-fit.json"
    ))
    .unwrap();
    let compiled =
        CompiledSensitivityDocument::compile(&model, &sensitivity.with_respect_to).unwrap();
    for solver in [
        Solver::DiffsolRosenbrock23,
        Solver::DiffsolTsit45,
        Solver::DiffsolEsdirk34,
        Solver::DiffsolTrBdf2,
        Solver::AnalyticLinearPk,
    ] {
        sensitivity.run.solver = solver;
        assert_eq!(
            compiled.run(&sensitivity).unwrap_err().code,
            ErrorCode::Unsupported
        );
        match &mut fit.problem {
            qspquest_core::fit::FitProblem::Individual(request) => {
                request.objective.simulation.run.solver = solver;
            }
            _ => panic!("synthetic fit fixture must be an individual request"),
        }
        assert_eq!(
            compiled.execute_fit(&fit).unwrap_err().code,
            ErrorCode::Unsupported
        );
    }
}

#[test]
fn rodas5p_event_sensitivities_match_closed_form_and_preserve_sides() {
    let model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let mut sensitivity: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    sensitivity.run = request();
    sensitivity.run.solver = Solver::DiffsolRodas5p;
    sensitivity.run.regimen.rtol = 1e-10;
    sensitivity.run.regimen.atol = Some(1e-12);
    let compiled =
        CompiledSensitivityDocument::compile(&model, &sensitivity.with_respect_to).unwrap();
    let result = compiled.run(&sensitivity).unwrap();
    assert_eq!(
        result.result.identity.algorithm,
        "rodas5p_augmented_forward_sensitivity"
    );
    for (row, time) in result.result.times.iter().copied().enumerate() {
        if time == 0. && result.result.sides[row] == ObservationSide::Pre {
            assert_eq!(result.result.outputs[0].values[row], 0.);
            continue;
        }
        let h = 1e-5;
        let expected = expected_amount(time, 0.1) / 3.;
        let dcl = (expected_amount(time, (0.3 + h) / 3.) - expected_amount(time, (0.3 - h) / 3.))
            / (6. * h);
        let dv = (expected_amount(time, 0.3 / (3. + h)) / (3. + h)
            - expected_amount(time, 0.3 / (3. - h)) / (3. - h))
            / (2. * h);
        assert!((result.result.outputs[0].values[row] - expected).abs() < 1e-8);
        assert!((result.derivatives[0].values[row] - dcl).abs() < 1e-7);
        assert!((result.derivatives[1].values[row] - dv).abs() < 1e-7);
    }
    let at_zero: Vec<_> = result
        .result
        .times
        .iter()
        .enumerate()
        .filter(|(_, t)| **t == 0.)
        .map(|(row, _)| result.result.sides[row])
        .collect();
    assert_eq!(at_zero, vec![ObservationSide::Pre, ObservationSide::Post]);
}

const ROBERTSON: &str = include_str!("../../../conformance/models/synthetic-robertson.qsp");

#[test]
fn stiff_robertson_sampled_outputs_agree_with_independent_radau() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../../conformance/references/robertson-radau.json"
    ))
    .unwrap();
    let times: Vec<f64> = serde_json::from_value(reference["times"].clone()).unwrap();
    let expected: Vec<Vec<f64>> = serde_json::from_value(reference["states"].clone()).unwrap();
    let compiled = language::parse(ROBERTSON).unwrap().compile().unwrap();
    // Local tolerances are not global-error guarantees. The TR-BDF2 adapter
    // now verifies sampled-trajectory convergence; require its measured error
    // against the independent reference to stay within the requested scale.
    // Other methods retain their declared ten-unit regression diagnostic.
    for accuracy in [AccuracyControl::Local, AccuracyControl::Sampled] {
        for rtol in [1e-3, 1e-4, 1e-5, 1e-6] {
            for solver in [
                Solver::DiffsolBdf,
                Solver::DiffsolEsdirk34,
                Solver::DiffsolTrBdf2,
                Solver::DiffsolRosenbrock23,
                Solver::DiffsolRodas5p,
            ] {
                let request: RunRequest = serde_json::from_value(json!({
                "schema":"qspquest.run/v0.1", "solver":solver, "accuracy_control":accuracy,
                "budgets":{"solver_callbacks":10000000,"output_values":10000,"events":100},
                "regimen":{"end":{"value":10000,"unit":"s"},
                    "samples":times.iter().map(|t| json!({"value":t,"unit":"s"})).collect::<Vec<_>>(),
                    "administrations":[],"rtol":rtol,"atol":1e-12}
            })).unwrap();
                let result = compiled
                    .execute(&request)
                    .unwrap_or_else(|e| panic!("{solver:?} rtol={rtol}: {e}"));
                let mut maximum: f64 = 0.0;
                for (row, &t) in result.times.iter().enumerate() {
                    let index = times.iter().position(|x| *x == t).unwrap();
                    let mut sum = 0.0;
                    for (column, values) in result.outputs.iter().enumerate() {
                        let actual = values.values[row];
                        let target = expected[index][column];
                        sum += actual;
                        maximum =
                            maximum.max((actual - target).abs() / (1e-12 + rtol * target.abs()));
                    }
                    assert!((sum - 1.0).abs() < 1e-8, "mass at {t}: {sum}");
                }
                println!("robertson,{solver:?},{accuracy:?},{rtol},max_scaled={maximum}");
                let bound = if accuracy == AccuracyControl::Sampled
                    || matches!(solver, Solver::DiffsolTrBdf2)
                {
                    1.0
                } else {
                    10.0
                };
                assert!(
                    maximum < bound,
                    "{solver:?} rtol={rtol}: {maximum} tolerance units"
                );
            }
        }
    }
}

#[test]
fn sampled_refinement_shares_the_callback_budget_and_changes_run_identity() {
    let compiled = language::parse(MODEL).unwrap().compile().unwrap();
    let mut run = request();
    run.solver = Solver::DiffsolRodas5p;
    let local = compiled.execute(&run).unwrap();
    run.accuracy_control = AccuracyControl::Sampled;
    let sampled = compiled.execute(&run).unwrap();
    assert_ne!(
        local.identity.run_request_hash,
        sampled.identity.run_request_hash
    );
    run.accuracy_control = AccuracyControl::Local;
    let mut lower = 0;
    let mut upper = 10000;
    while upper - lower > 1 {
        let middle = (upper + lower) / 2;
        run.budgets.solver_callbacks = middle;
        match compiled.execute(&run) {
            Ok(_) => upper = middle,
            Err(e) => {
                assert_eq!(e.code, ErrorCode::WorkBudget);
                lower = middle;
            }
        }
    }
    run.budgets.solver_callbacks = upper;
    assert!(compiled.execute(&run).is_ok());
    run.accuracy_control = AccuracyControl::Sampled;
    assert_eq!(
        compiled.execute(&run).unwrap_err().code,
        ErrorCode::WorkBudget
    );
    let model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let mut sensitivity: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    sensitivity.run.accuracy_control = AccuracyControl::Sampled;
    let compiled =
        CompiledSensitivityDocument::compile(&model, &sensitivity.with_respect_to).unwrap();
    assert_eq!(
        compiled.run(&sensitivity).unwrap_err().code,
        ErrorCode::Unsupported
    );
}

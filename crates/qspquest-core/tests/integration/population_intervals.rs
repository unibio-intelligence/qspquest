use qspquest_core::{fit::*, population_intervals::*};
use serde_json::json;

fn fixture() -> PopulationFitResult {
    serde_json::from_value(json!({
        "status":"converged", "fixed_effects":{
            "theta_intercept":{"value":1.,"unit":"1"},
            "theta_slope":{"value":2.718281828459045,"unit":"1"}},
        "omega":[[0.2]], "subjects":[],
        "objective_kind":"saem_marginal_objective_function_value",
        "objective":0., "evaluations":1,"iterations":1,"fit_request_hash":"fixture",
        "uncertainty":{"method":"saem_marginal_observed_hessian",
            "parameters":[{"name":"theta_intercept","unit":"1"},{"name":"theta_slope","unit":"1"}],
            "standard_errors":[1.,0.5436563656918091],
            "covariance":[[1.,0.01],[0.01,0.295562243957226]]}
    }))
    .unwrap()
}

#[test]
fn log_wald_preserves_estimates_covariance_and_positive_asymmetry() {
    let result = fixture();
    let before = serde_json::to_value(&result).unwrap();
    let intervals =
        positive_fixed_effect_intervals(&result, &["theta_intercept".into(), "theta_slope".into()])
            .unwrap();
    assert_eq!(
        intervals.method,
        PositiveFixedEffectIntervalMethod::PositiveFixedEffectLogWaldDeltaV1
    );
    assert_eq!(intervals.confidence_level, 0.95);
    let first = &intervals.intervals[0];
    // Independent normal-quantile reference, including the wide asymmetric case.
    assert!((first.lower - 0.14086349409321744).abs() < 1e-14);
    assert!((first.upper - 7.099071384231335).abs() < 1e-13);
    assert!((first.lower * first.upper - 1.).abs() < 1e-14);
    assert_eq!(serde_json::to_value(&result).unwrap(), before);
}

#[test]
fn log_wald_resolves_parameter_order_and_units() {
    let mut result = fixture();
    result
        .fixed_effects
        .get_mut("theta_intercept")
        .unwrap()
        .unit = "L/h".into();
    result.uncertainty.as_mut().unwrap().parameters[0].unit = "L/h".into();
    let intervals =
        positive_fixed_effect_intervals(&result, &["theta_slope".into(), "theta_intercept".into()])
            .unwrap();
    assert_eq!(intervals.intervals[0].name, "theta_slope");
    assert_eq!(intervals.intervals[1].unit, "L/h");
}

#[test]
fn log_wald_rejects_invalid_estimates_names_and_covariance() {
    for case in 0..13 {
        let mut result = fixture();
        let mut names = vec!["theta_intercept".into()];
        match case {
            0 => result.status = FitStatus::EvaluationLimit,
            1 => result.uncertainty = None,
            2 => {
                result.uncertainty.as_mut().unwrap().method =
                    PopulationUncertaintyMethod::FoceiObservedOuterHessian
            }
            3 => {
                result
                    .fixed_effects
                    .get_mut("theta_intercept")
                    .unwrap()
                    .value = 0.
            }
            4 => {
                result
                    .fixed_effects
                    .get_mut("theta_intercept")
                    .unwrap()
                    .unit = "L".into()
            }
            5 => names.push("theta_intercept".into()),
            6 => names = vec!["absent".into()],
            7 => result.uncertainty.as_mut().unwrap().standard_errors[0] = 2.,
            8 => result.uncertainty.as_mut().unwrap().covariance[1][0] = 0.5,
            9 => {
                let u = result.uncertainty.as_mut().unwrap();
                u.covariance[0][1] = 2.;
                u.covariance[1][0] = 2.;
            }
            10 => result.uncertainty.as_mut().unwrap().covariance[1][1] = f64::NAN,
            11 => {
                result.uncertainty.as_mut().unwrap().parameters[1].name = "theta_intercept".into()
            }
            12 => {
                result
                    .fixed_effects
                    .get_mut("theta_intercept")
                    .unwrap()
                    .value = f64::MIN_POSITIVE
            }
            _ => unreachable!(),
        }
        assert!(
            positive_fixed_effect_intervals(&result, &names).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn log_wald_checks_nuisance_covariance_and_refuses_nuisance_intervals() {
    let mut result = fixture();
    result.uncertainty.as_mut().unwrap().parameters[1].name = "omega".into();
    assert!(positive_fixed_effect_intervals(&result, &["omega".into()]).is_err());
    let u = result.uncertainty.as_mut().unwrap();
    u.covariance[1][0] = 2.;
    u.covariance[0][1] = 2.;
    assert!(positive_fixed_effect_intervals(&result, &["theta_intercept".into()]).is_err());
}

#[test]
fn optional_interval_failure_keeps_the_converged_fit_and_covariance() {
    let mut result = fixture();
    result
        .fixed_effects
        .get_mut("theta_intercept")
        .unwrap()
        .value = f64::MIN_POSITIVE;
    let before = serde_json::to_value(&result).unwrap();
    qspquest_core::population_intervals::attach_positive_fixed_effect_intervals(
        &mut result,
        &["theta_intercept".into()],
    );
    assert_eq!(result.status, FitStatus::Converged);
    assert!(result.fixed_effect_intervals.is_none());
    assert!(result.fixed_effect_interval_error.is_some());
    let mut after = serde_json::to_value(&result).unwrap();
    after
        .as_object_mut()
        .unwrap()
        .remove("fixed_effect_interval_error");
    assert_eq!(after, before);
}

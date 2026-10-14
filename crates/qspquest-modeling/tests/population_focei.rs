use qspquest_core::{
    fit::*,
    model::{ModelDocument, Quantity},
    run::SensitivityRequest,
    ErrorCode,
};
use qspquest_modeling::document::sensitivity::CompiledSensitivityDocument;
use std::sync::Arc;

fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}

fn fixture(proportional: f64) -> (Arc<CompiledSensitivityDocument>, PopulationFitRequest) {
    let mut model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    model.parameters[1].default = q(4.5, "L");
    let compiled = CompiledSensitivityDocument::compile(&model, &["cl".into()]).unwrap();
    let mut simulation: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    simulation.with_respect_to = vec!["cl".into()];
    simulation.derivative_absolute_tolerances.remove("v");
    simulation.run.parameters.insert("cl".into(), q(0.5, "L/h"));
    let mut subjects = Vec::new();
    for eta in [-0.22_f64, -0.08, 0.08, 0.22] {
        let mut truth = simulation.clone();
        truth
            .run
            .parameters
            .insert("cl".into(), q(0.6 * eta.exp(), "L/h"));
        let result = compiled.run(&truth).unwrap();
        let observations = result.result.outputs[0]
            .values
            .iter()
            .enumerate()
            .skip(1)
            .map(|(row, value)| GaussianObservation {
                censoring: None,
                row,
                output: "cp".into(),
                value: q(*value, "mg/L"),
                error: GaussianError {
                    model: GaussianErrorModel::Normal,
                    additive_sd: q(0.05, "mg/L"),
                    proportional_sd: proportional,
                },
            })
            .collect();
        subjects.push(GaussianObjectiveRequest {
            simulation: simulation.clone(),
            observations,
            priors: vec![],
        });
    }
    (
        compiled,
        PopulationFitRequest {
            subjects,
            fixed_effects: vec![FitBound {
                parameter: "cl".into(),
                lower: q(0.35, "L/h"),
                upper: q(0.85, "L/h"),
            }],
            random_effects: vec![PopulationRandomEffect {
                parameter: "cl".into(),
            }],
            omega: vec![vec![0.04]],
            omega_diagonal_bounds: vec![[0.04, 0.04]],
            eta_bound: 1.5,
            max_evaluations: 30_000,
            max_iterations: 80,
            gradient_tolerance: 0.005,
            seed: 17,
            burn_in_iterations: None,
            uncertainty: false,
            additive_error_fit: None,
            covariate_effect: None,
        },
    )
}

#[test]
fn focei_recovers_population_mean_and_reports_distinct_method() {
    let (compiled, request) = fixture(0.);
    let result = compiled.fit_population_focei(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    assert_eq!(
        result.objective_kind,
        PopulationObjectiveKind::FoceiObjectiveFunctionValue
    );
    assert!(
        (result.fixed_effects["cl"].value - 0.6).abs() < 0.06,
        "{result:?}"
    );
    assert_eq!(result.omega, vec![vec![0.04]]);
    assert_eq!(result.subjects.len(), 4);
    let rows = result
        .fitted_observations
        .as_ref()
        .expect("FOCEI fitted rows");
    let expected_rows: usize = request.subjects.iter().map(|s| s.observations.len()).sum();
    assert_eq!(rows.len(), expected_rows);
    let projected = request
        .subjects
        .iter()
        .enumerate()
        .map(|(subject, source)| {
            let mut pred_request = source.simulation.clone();
            pred_request
                .run
                .parameters
                .insert("cl".into(), result.fixed_effects["cl"].clone());
            let pred = compiled.run(&pred_request).unwrap();
            pred_request.run.parameters.insert(
                "cl".into(),
                q(
                    result.fixed_effects["cl"].value * result.subjects[subject].eta[0].exp(),
                    "L/h",
                ),
            );
            let ipred = compiled.run(&pred_request).unwrap();
            (pred, ipred)
        })
        .collect::<Vec<_>>();
    for (index, row) in rows.iter().enumerate() {
        let subject = index / request.subjects[0].observations.len();
        let observation = index % request.subjects[0].observations.len();
        assert_eq!(row.subject_index, subject);
        assert_eq!(row.observation_index, observation);
        assert_eq!(
            row.row,
            request.subjects[subject].observations[observation].row
        );
        assert_eq!(row.output, "cp");
        assert_eq!(row.value.unit, "mg/L");
        assert_eq!(row.pred.unit, "mg/L");
        assert_eq!(row.ipred.unit, "mg/L");
        assert_eq!(row.time.unit, "h");
        assert!(row.ipred.value.is_finite() && row.pred.value.is_finite());
        let (pred, ipred) = &projected[subject];
        assert_eq!(row.time.value, pred.result.times[row.row]);
        assert_eq!(row.side, pred.result.sides[row.row]);
        assert_eq!(row.side, ipred.result.sides[row.row]);
        assert!((row.pred.value - pred.result.outputs[0].values[row.row]).abs() < 1e-12);
        assert!((row.ipred.value - ipred.result.outputs[0].values[row.row]).abs() < 1e-12);
    }
    for pair in result.subjects.windows(2) {
        assert!(pair[0].eta[0] < pair[1].eta[0]);
    }
    let repeated = compiled.fit_population_focei(&request).unwrap();
    assert_eq!(result.fit_request_hash, repeated.fit_request_hash);
    assert_eq!(result.objective, repeated.objective);
    let dispatched = compiled
        .execute_fit(&FitRequest {
            schema: FitSchema::V01,
            problem: FitProblem::Focei(request),
        })
        .unwrap();
    assert!(dispatched.fit.is_none());
    assert_eq!(
        dispatched.population.unwrap().objective_kind,
        PopulationObjectiveKind::FoceiObjectiveFunctionValue
    );
}

#[test]
fn focei_uses_first_order_curvature_and_live_residual_variance() {
    let (compiled, mut request) = fixture(0.2);
    // Stop immediately after evaluating the identical starting population
    // point, so the objective difference is exclusively the curvature method.
    request.gradient_tolerance = 0.25;
    let focei = compiled.fit_population_focei(&request).unwrap();
    let laplace = compiled.fit_population_laplace(&request).unwrap();
    assert_eq!(focei.status, FitStatus::Converged);
    assert_eq!(laplace.status, FitStatus::Converged);
    let observations: usize = request.subjects.iter().map(|s| s.observations.len()).sum();
    let laplace_ofv =
        2. * laplace.objective - observations as f64 * (2. * std::f64::consts::PI).ln();
    assert!(
        (focei.objective - laplace_ofv).abs() > 1e-5,
        "FOCEI and observed-curvature Laplace unexpectedly coincide"
    );
    assert!(focei.objective.is_finite());
    assert!(focei
        .subjects
        .iter()
        .all(|s| s.negative_log_likelihood.is_finite()));
}

#[test]
fn focei_bounds_work_and_rejects_wider_random_effects() {
    let (compiled, mut request) = fixture(0.);
    request.max_evaluations = 160;
    let result = compiled.fit_population_focei(&request).unwrap();
    assert_eq!(result.status, FitStatus::EvaluationLimit);
    assert_eq!(result.evaluations, 160);
    assert!(result.objective.is_finite());
    request.random_effects.push(PopulationRandomEffect {
        parameter: "v".into(),
    });
    assert_eq!(
        compiled.fit_population_focei(&request).unwrap_err().code,
        ErrorCode::Unsupported
    );
}

#[test]
fn focei_observed_standard_error_matches_independent_profile_difference() {
    let (compiled, mut request) = fixture(0.);
    request.uncertainty = true;
    let result = compiled.fit_population_focei(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged);
    let uncertainty = result.uncertainty.as_ref().unwrap();
    assert_eq!(
        uncertainty.method,
        PopulationUncertaintyMethod::FoceiObservedOuterHessian
    );
    assert_eq!(uncertainty.parameters.len(), 1);
    assert_eq!(uncertainty.parameters[0].name, "cl");
    assert_eq!(uncertainty.parameters[0].unit, "L/h");
    let theta = result.fixed_effects["cl"].value;
    let h = 0.01 * (request.fixed_effects[0].upper.value - request.fixed_effects[0].lower.value);
    let objective_at = |theta: f64| {
        let mut point = request.clone();
        point.uncertainty = false;
        point.gradient_tolerance = 0.25;
        for subject in &mut point.subjects {
            subject
                .simulation
                .run
                .parameters
                .insert("cl".into(), q(theta, "L/h"));
        }
        let fit = compiled.fit_population_focei(&point).unwrap();
        assert_eq!(fit.iterations, 0);
        fit.objective
    };
    let f0 = objective_at(theta);
    let second = (objective_at(theta + h) - 2. * f0 + objective_at(theta - h)) / (h * h);
    let oracle_se = (2. / second).sqrt();
    assert!(oracle_se.is_finite());
    assert!((uncertainty.standard_errors[0] - oracle_se).abs() < 1e-6);
    assert!((uncertainty.covariance[0][0] - oracle_se * oracle_se).abs() < 1e-6);
    assert!(
        result.evaluations
            > compiled
                .fit_population_focei(&PopulationFitRequest {
                    uncertainty: false,
                    ..request
                })
                .unwrap()
                .evaluations
    );
}

#[test]
fn focei_observed_covariance_handles_free_omega_and_rejects_boundary() {
    let (compiled, mut request) = fixture(0.);
    assert!(serde_json::to_value(&request)
        .unwrap()
        .get("uncertainty")
        .is_none());
    request.omega_diagonal_bounds = vec![[0.01, 0.15]];
    request.uncertainty = true;
    assert_eq!(serde_json::to_value(&request).unwrap()["uncertainty"], true);
    assert_eq!(
        compiled.fit_population_laplace(&request).unwrap_err().code,
        ErrorCode::Unsupported
    );
    assert_eq!(
        compiled.fit_population_saem(&request).unwrap_err().code,
        ErrorCode::Unsupported
    );
    let result = compiled.fit_population_focei(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged);
    let uncertainty = result.uncertainty.unwrap();
    assert_eq!(uncertainty.parameters.len(), 2);
    assert_eq!(uncertainty.parameters[1].name, "omega:cl");
    assert_eq!(uncertainty.parameters[1].unit, "1");
    assert!(uncertainty
        .standard_errors
        .iter()
        .all(|value| value.is_finite() && *value > 0.));
    assert_eq!(uncertainty.covariance[0][1], uncertainty.covariance[1][0]);
    assert!(
        uncertainty.covariance[0][0] * uncertainty.covariance[1][1]
            > uncertainty.covariance[0][1].powi(2)
    );

    request.gradient_tolerance = 0.25;
    for subject in &mut request.subjects {
        subject
            .simulation
            .run
            .parameters
            .insert("cl".into(), q(0.35, "L/h"));
    }
    assert_eq!(
        compiled.fit_population_focei(&request).unwrap_err().code,
        ErrorCode::Unsupported
    );
}

#[test]
fn focei_standard_error_matches_exact_gaussian_random_intercept() {
    // The model emits log(theta * exp(eta)) = log(theta) + eta. With one
    // additive observation per subject, Gaussian integration is exact:
    // Var(theta_hat) = theta^2 * (Omega + sigma^2) / N at the MLE.
    let mut model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    model.parameters[0].default = q(1., "1");
    model.parameters[1].default = q(1., "h");
    model.states[0].unit = "1".into();
    model.states[0].initial = qspquest_core::model::Initial::Quantity {
        quantity: q(0., "1"),
    };
    model.states[0].dosing = None;
    model.outputs[0].unit = "1".into();
    model.outputs[0].expression = qspquest_core::expression::Expr::call(
        qspquest_core::expression::Function::Log,
        vec![qspquest_core::expression::Expr::symbol("cl")],
    );
    let compiled = CompiledSensitivityDocument::compile(&model, &["cl".into()]).unwrap();
    let mut simulation: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    simulation.with_respect_to = vec!["cl".into()];
    simulation.derivative_absolute_tolerances.clear();
    simulation.derivative_absolute_tolerances.insert(
        "cl".into(),
        std::collections::BTreeMap::from([("central".into(), q(1e-12, "1"))]),
    );
    simulation.run.parameters.insert("cl".into(), q(1., "1"));
    simulation.run.parameters.insert("v".into(), q(1., "h"));
    simulation.run.regimen.administrations.clear();
    let subjects = [-0.3, -0.1, 0.1, 0.3]
        .into_iter()
        .map(|value| GaussianObjectiveRequest {
            simulation: simulation.clone(),
            observations: vec![GaussianObservation {
                censoring: None,
                row: 0,
                output: "cp".into(),
                value: q(value, "1"),
                error: GaussianError {
                    model: GaussianErrorModel::Normal,
                    additive_sd: q(0.1, "1"),
                    proportional_sd: 0.,
                },
            }],
            priors: vec![],
        })
        .collect();
    let request = PopulationFitRequest {
        subjects,
        fixed_effects: vec![FitBound {
            parameter: "cl".into(),
            lower: q(0.7, "1"),
            upper: q(1.3, "1"),
        }],
        random_effects: vec![PopulationRandomEffect {
            parameter: "cl".into(),
        }],
        omega: vec![vec![0.04]],
        omega_diagonal_bounds: vec![[0.04, 0.04]],
        eta_bound: 1.5,
        max_evaluations: 1000,
        max_iterations: 1,
        gradient_tolerance: 0.25,
        seed: 1,
        burn_in_iterations: None,
        uncertainty: true,
        additive_error_fit: None,
        covariate_effect: None,
    };
    let result = compiled.fit_population_focei(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged);
    assert!((result.fixed_effects["cl"].value - 1.).abs() < 1e-12);
    let se = result.uncertainty.unwrap().standard_errors[0];
    let exact = ((0.04_f64 + 0.01) / 4.).sqrt();
    assert!((se - exact).abs() < 1e-4, "{se} vs {exact}");

    let mut free_omega = request;
    free_omega.omega_diagonal_bounds = vec![[0.01, 0.09]];
    let fitted = compiled.fit_population_focei(&free_omega).unwrap();
    assert_eq!(fitted.iterations, 0);
    let covariance = fitted.uncertainty.unwrap();
    let exact_omega_se = (2. * (0.05_f64).powi(2) / 4.).sqrt();
    assert!((covariance.standard_errors[0] - exact).abs() < 1e-4);
    assert!((covariance.standard_errors[1] - exact_omega_se).abs() < 1e-4);
    assert!(covariance.covariance[0][1].abs() < 1e-4);

    // With a fixed variance of eta = 0.01, the four observed log outputs
    // have MLE variance 0.05. Thus additive SD = sqrt(0.05 - 0.01) = 0.2.
    let mut free_sigma = free_omega;
    free_sigma.uncertainty = false;
    free_sigma.omega = vec![vec![0.01]];
    free_sigma.omega_diagonal_bounds = vec![[0.01, 0.01]];
    free_sigma.max_evaluations = 30_000;
    free_sigma.max_iterations = 100;
    free_sigma.gradient_tolerance = 0.001;
    for subject in &mut free_sigma.subjects {
        subject.observations[0].error.additive_sd = q(0.15, "1");
    }
    free_sigma.additive_error_fit = Some(PopulationAdditiveErrorFit {
        output: "cp".into(),
        initial: q(0.15, "1"),
        lower: q(0.1, "1"),
        upper: q(0.3, "1"),
    });
    let estimated = compiled.fit_population_focei(&free_sigma).unwrap();
    assert_eq!(estimated.status, FitStatus::Converged, "{estimated:?}");
    assert!((estimated.fixed_effects["cl"].value - 1.).abs() < 0.003);
    let residual = estimated.estimated_residual_error.as_ref().unwrap();
    assert_eq!(residual.output, "cp");
    assert!(
        (residual.additive_sd.value - 0.2).abs() < 0.003,
        "{estimated:?}"
    );
    assert!(
        (estimated.objective - 4. * (1. + 0.05_f64.ln())).abs() < 0.003,
        "{estimated:?}"
    );
    assert!(estimated.uncertainty.is_none());
    assert_eq!(
        compiled
            .fit_population_laplace(&free_sigma)
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    let mut invalid = free_sigma.clone();
    invalid.subjects[0].observations[0].error.proportional_sd = 0.1;
    assert_eq!(
        compiled.fit_population_focei(&invalid).unwrap_err().code,
        ErrorCode::InvalidInput
    );
    invalid = free_sigma.clone();
    invalid.subjects[0].observations[0].error.additive_sd = q(0.17, "1");
    assert_eq!(
        compiled.fit_population_focei(&invalid).unwrap_err().code,
        ErrorCode::InvalidInput
    );
    invalid = free_sigma.clone();
    invalid.additive_error_fit.as_mut().unwrap().lower = q(0., "1");
    assert_eq!(
        compiled.fit_population_focei(&invalid).unwrap_err().code,
        ErrorCode::InvalidInput
    );
    invalid = free_sigma;
    invalid.uncertainty = true;
    assert_eq!(
        compiled.fit_population_focei(&invalid).unwrap_err().code,
        ErrorCode::Unsupported
    );
}

#[test]
fn focei_shares_only_identical_subject_simulations() {
    let (compiled, mut request) = fixture(0.);
    let changed = &mut request.subjects[1];
    changed.simulation.run.regimen.administrations[0]
        .amount
        .value *= 2.;
    let mut truth = changed.simulation.clone();
    truth
        .run
        .parameters
        .insert("cl".into(), q(0.6 * (-0.08_f64).exp(), "L/h"));
    let observations = compiled.run(&truth).unwrap();
    for observation in &mut changed.observations {
        observation.value.value = observations.result.outputs[0].values[observation.row];
    }
    request.gradient_tolerance = 0.25;
    let result = compiled.fit_population_focei(&request).unwrap();
    let rows = result.fitted_observations.unwrap();
    let altered = rows
        .iter()
        .filter(|row| row.subject_index == 1)
        .collect::<Vec<_>>();
    assert!(!altered.is_empty());
    let ordinary = rows.iter().find(|row| row.subject_index == 0).unwrap();
    let first = altered[0];
    assert_eq!(first.row, ordinary.row);
    assert!((first.pred.value - 2. * ordinary.pred.value).abs() < 1e-10);
    let mut projection = request.subjects[1].simulation.clone();
    projection.run.parameters.insert(
        "cl".into(),
        q(
            result.fixed_effects["cl"].value * result.subjects[1].eta[0].exp(),
            "L/h",
        ),
    );
    let projected = compiled.run(&projection).unwrap();
    for row in altered {
        assert!((row.ipred.value - projected.result.outputs[0].values[row.row]).abs() < 1e-10);
    }
}

#[test]
fn rodas5p_population_dispatch_preserves_objective_and_solver_identity() {
    let (compiled, mut request) = fixture(0.);
    // Bounded replay checks both primal likelihood and derivative-bearing population paths.
    request.max_iterations = 1;
    request.uncertainty = false;
    let bdf = compiled.fit_population_focei(&request).unwrap();
    for subject in &mut request.subjects {
        subject.simulation.run.solver = qspquest_core::run::Solver::DiffsolRodas5p;
    }
    let rodas = compiled.fit_population_focei(&request).unwrap();
    assert_ne!(bdf.fit_request_hash, rodas.fit_request_hash);
    assert!(
        (bdf.objective - rodas.objective).abs() < 1e-4,
        "{} vs {}",
        bdf.objective,
        rodas.objective
    );
}

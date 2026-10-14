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

fn fixture() -> (Arc<CompiledSensitivityDocument>, PopulationFitRequest) {
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
                    proportional_sd: 0.,
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
fn laplace_recovers_population_mean_and_orders_subject_etas() {
    let (compiled, request) = fixture();
    let result = compiled.fit_population_laplace(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    assert_eq!(
        result.objective_kind,
        PopulationObjectiveKind::LaplaceNegativeLogLikelihood
    );
    assert!(
        (result.fixed_effects["cl"].value - 0.6).abs() < 0.06,
        "{result:?}"
    );
    assert_eq!(result.omega, vec![vec![0.04]]);
    assert_eq!(result.subjects.len(), 4);
    for pair in result.subjects.windows(2) {
        assert!(pair[0].eta[0] < pair[1].eta[0]);
    }
    assert!(result.objective.is_finite());
    let repeated = compiled.fit_population_laplace(&request).unwrap();
    assert_eq!(result.fit_request_hash, repeated.fit_request_hash);
    assert_eq!(result.objective, repeated.objective);
    let dispatched = compiled
        .execute_fit(&FitRequest {
            schema: FitSchema::V01,
            problem: FitProblem::Laplace(request),
        })
        .unwrap();
    assert!(dispatched.fit.is_none());
    assert_eq!(
        dispatched.population.unwrap().objective_kind,
        PopulationObjectiveKind::LaplaceNegativeLogLikelihood
    );
}

#[test]
fn laplace_rejects_wider_random_effects_and_invalid_omega() {
    let (compiled, mut request) = fixture();
    request.random_effects.push(PopulationRandomEffect {
        parameter: "v".into(),
    });
    assert_eq!(
        compiled.fit_population_laplace(&request).unwrap_err().code,
        ErrorCode::Unsupported
    );
    request.random_effects.pop();
    request.omega[0][0] = 0.;
    assert_eq!(
        compiled.fit_population_laplace(&request).unwrap_err().code,
        ErrorCode::InvalidInput
    );
}

#[test]
fn laplace_can_estimate_variance_from_subject_dispersion() {
    let (compiled, mut request) = fixture();
    request.omega = vec![vec![0.16]];
    request.omega_diagonal_bounds = vec![[0.005, 0.25]];
    request.gradient_tolerance = 0.01;
    let result = compiled.fit_population_laplace(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    assert!(result.omega[0][0] < 0.1, "{result:?}");
    assert!(result.omega[0][0] > 0.005, "{result:?}");
    assert!((result.fixed_effects["cl"].value - 0.6).abs() < 0.07);
}

#[test]
fn laplace_reports_evaluation_limit_after_initial_population_evaluation() {
    let (compiled, mut request) = fixture();
    request.max_evaluations = 160;
    let result = compiled.fit_population_laplace(&request).unwrap();
    assert_eq!(result.status, FitStatus::EvaluationLimit);
    assert_eq!(result.evaluations, 160);
    assert!(result.objective.is_finite());
}

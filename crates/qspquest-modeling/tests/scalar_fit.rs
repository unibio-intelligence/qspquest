use qspquest_core::{
    fit::{
        FitStatus, GaussianError, GaussianErrorModel, GaussianObservation, ScalarFitSchema,
        ScalarGaussianFitRequest,
    },
    model::Quantity,
    run::SensitivityRequest,
};
use qspquest_modeling::document::CompiledDocument;

fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}

#[test]
fn scalar_fit_recovers_clearance_without_sensitivity_compilation() {
    let model = CompiledDocument::from_json(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let sensitivity: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    let run = sensitivity.run;
    let mut truth = run.clone();
    truth.parameters.insert("cl".into(), q(0.6, "L/h"));
    let result = model.execute(&truth).unwrap();
    let cp = result
        .outputs
        .iter()
        .find(|output| output.name == "cp")
        .unwrap();
    let observations = cp
        .values
        .iter()
        .enumerate()
        .skip(1)
        .map(|(row, value)| GaussianObservation {
            censoring: None,
            row,
            output: "cp".into(),
            value: q(*value, &cp.unit),
            error: GaussianError {
                model: GaussianErrorModel::Normal,
                additive_sd: q(0.05, &cp.unit),
                proportional_sd: 0.,
            },
        })
        .collect();
    let request = ScalarGaussianFitRequest {
        schema: ScalarFitSchema::V01,
        run,
        parameter: "cl".into(),
        lower: q(0.1, "L/h"),
        upper: q(1., "L/h"),
        observations,
        max_iterations: 80,
        absolute_tolerance: 1e-7,
    };
    let fitted = model.fit_scalar(&request).unwrap();
    assert_eq!(fitted.status, FitStatus::Converged);
    assert!((fitted.parameters["cl"].value - 0.6).abs() < 1e-4);
    assert!(
        fitted.evaluations <= 20,
        "{} evaluations",
        fitted.evaluations
    );
    assert_eq!(fitted.objective.observations, cp.values.len() - 1);
    assert_eq!(
        fitted.objective.identity.model_content_hash,
        model.model_content_hash()
    );

    let mut boundary_truth = truth;
    boundary_truth.parameters.insert("cl".into(), q(1.2, "L/h"));
    let boundary_result = model.execute(&boundary_truth).unwrap();
    let boundary_cp = boundary_result
        .outputs
        .iter()
        .find(|output| output.name == "cp")
        .unwrap();
    let mut boundary = request.clone();
    for observation in &mut boundary.observations {
        observation.value.value = boundary_cp.values[observation.row];
    }
    let bounded = model.fit_scalar(&boundary).unwrap();
    assert_eq!(bounded.status, FitStatus::Converged);
    assert!(bounded.parameters["cl"].value > 0.9999);

    let mut invalid = request;
    invalid.lower = q(1.1, "L/h");
    assert!(model.fit_scalar(&invalid).is_err());
    invalid.lower = q(0.1, "L/h");
    invalid.observations[0].row = 999;
    assert!(model.fit_scalar(&invalid).is_err());
}

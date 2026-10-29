mod fixture_support;
use qspquest_core::fit::{FitStatus, PopulationFitRequest};
use qspquest_modeling::document::sensitivity::CompiledSensitivityDocument;

#[test]
fn fitted_residual_stability_requires_a_marginal_point_certificate() {
    let source = "model one_effect_residual_check\ntime_unit = h\nparameters\n theta = 2.718281828459045 [1] estimated\n decay = 0 [1/h] fixed\nstates\n anchor = 0 [1]\ndynamics\n d/dt(anchor) = -decay * anchor\noutputs\n z = log(theta) [1]\nend\n";
    let model = qspquest_modeling::language::parse(source).unwrap().document;
    let compiled = CompiledSensitivityDocument::compile(&model, &["theta".into()]).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&fixture_support::text(include_bytes!(
        "fixtures/one_effect_residual_stall.json.gz"
    )))
    .unwrap();
    let request: PopulationFitRequest =
        serde_json::from_value(raw["problem"]["request"].clone()).unwrap();
    let result = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged);
    assert!(result.evaluations <= request.max_evaluations);
    assert!(result.iterations <= request.max_iterations);
    let mu = result.fixed_effects["theta"].value.ln();
    let omega = result.omega[0][0];
    let sigma = result
        .estimated_residual_error
        .as_ref()
        .unwrap()
        .additive_sd
        .value;
    assert!((mu - 1.4732052322847164).abs() < 0.01, "mu={mu}");
    assert!((omega - 0.08253661605395818).abs() < 0.005, "omega={omega}");
    assert!((sigma - 0.3525326544430893).abs() < 0.005, "sigma={sigma}");
    assert!(
        (result.objective - -69.10744413512847).abs() < 0.001,
        "ofv={}",
        result.objective
    );
}

#[test]
fn weak_variance_point_fit_certifies_the_marginal_mean() {
    let source = "model weak_point_check\ntime_unit = h\nparameters\n theta = 2.718281828459045 [1] estimated\n decay = 0 [1/h] fixed\nstates\n anchor = 0 [1]\ndynamics\n d/dt(anchor) = -decay * anchor\noutputs\n z = log(theta) [1]\nend\n";
    let model = qspquest_modeling::language::parse(source).unwrap().document;
    let compiled = CompiledSensitivityDocument::compile(&model, &["theta".into()]).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&fixture_support::text(include_bytes!(
        "fixtures/one_effect_weak_point_stall.json.gz"
    )))
    .unwrap();
    let request: PopulationFitRequest =
        serde_json::from_value(raw["problem"]["request"].clone()).unwrap();
    let result = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged);
    assert!(result.evaluations <= request.max_evaluations);
    assert!(result.iterations <= request.max_iterations);
    assert!((result.fixed_effects["theta"].value.ln() - 1.348101589567421).abs() < 0.01);
    assert!((result.omega[0][0] - 0.001).abs() < 0.001);
    assert!((result.objective - 48.5693385510492).abs() < 0.001);
}

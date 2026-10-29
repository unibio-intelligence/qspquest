use qspquest_core::{
    fit::*,
    model::Quantity,
    run::{SensitivityRequest, Solver},
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
#[test]
fn versioned_profiled_mode_publishes_scales_with_a_distinct_identity() {
    let (model, request) = fixture();
    let direct = model.fit_profiled_proportional_gaussian(&request).unwrap();
    let versioned = FitRequest {
        schema: FitSchema::V01,
        problem: FitProblem::ProfiledProportional(request.clone()),
    };
    assert_eq!(versioned.parameters(), &["theta"]);
    let result = model.execute_fit(&versioned).unwrap();
    let fit = result.fit.as_ref().unwrap();
    assert_eq!(fit.status, FitStatus::Converged);
    assert_eq!(
        result.residual_scales.as_ref().unwrap(),
        &direct.residual_scales
    );
    assert_ne!(fit.fit_request_hash, direct.fit.fit_request_hash);
    assert!((fit.parameters["theta"].value - 2.5).abs() < 1e-5);
    let known = model
        .execute_fit(&FitRequest {
            schema: FitSchema::V01,
            problem: FitProblem::Individual(request),
        })
        .unwrap();
    assert!(known.residual_scales.is_none());
    assert_ne!(known.fit.unwrap().fit_request_hash, fit.fit_request_hash);
    let encoded = serde_json::to_value(&result).unwrap();
    let decoded: FitResult = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.residual_scales, result.residual_scales);
}
#[test]
fn profiled_planning_validates_rows_without_spending_solver_budget() {
    let (model, mut request) = fixture();
    request.objective.simulation.run.budgets.solver_callbacks = 1;
    let wrap = |request| FitRequest {
        schema: FitSchema::V01,
        problem: FitProblem::ProfiledProportional(request),
    };
    let plan = model.plan_fit(&wrap(request.clone())).unwrap();
    let qspquest_core::planning::FitSubjectPlan::GaussianObjective(subject) = &plan.subjects[0]
    else {
        panic!("expected Gaussian subject");
    };
    assert!(
        subject
            .prospective_identity
            .algorithm
            .contains("profiled_proportional"),
        "profiled plan advertised {}",
        subject.prospective_identity.algorithm
    );
    let known = model
        .plan_fit(&FitRequest {
            schema: FitSchema::V01,
            problem: FitProblem::Individual(request.clone()),
        })
        .unwrap();
    let qspquest_core::planning::FitSubjectPlan::GaussianObjective(known_subject) =
        &known.subjects[0]
    else {
        panic!("expected known-error Gaussian subject");
    };
    assert_ne!(
        subject.prospective_identity.run_request_hash,
        known_subject.prospective_identity.run_request_hash
    );
    assert!(matches!(
        plan.request.problem,
        FitProblem::ProfiledProportional(_)
    ));
    let mut invalid = request.clone();
    invalid.objective.observations[0].error.proportional_sd = 0.5;
    assert_eq!(
        model.plan_fit(&wrap(invalid.clone())).unwrap_err().code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        model.execute_fit(&wrap(invalid)).unwrap_err().code,
        ErrorCode::InvalidInput
    );
    request
        .objective
        .observations
        .retain(|r| r.output == "y1" || r.row == 0);
    assert_eq!(
        model.plan_fit(&wrap(request)).unwrap_err().code,
        ErrorCode::InvalidInput
    );
}
fn fixture() -> (Arc<CompiledSensitivityDocument>, GaussianFitRequest) {
    let source="model profiled_normal\ntime_unit = h\nparameters\n theta = 1.8 [mg/L] estimated\n decay = 1 [1/h] fixed\nstates\n unused = 1 [1]\ndynamics\n d/dt(unused) = -decay * unused\noutputs\n y1 = theta [mg/L]\n y2 = 2 * theta [mg/L]\nend\n";
    let document = qspquest_modeling::language::parse(source).unwrap().document;
    let mut simulation: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    simulation.with_respect_to = vec!["theta".into()];
    simulation.derivative_absolute_tolerances.clear();
    simulation.derivative_absolute_tolerances.insert(
        "theta".into(),
        [("unused".into(), q(1e-12, "L/mg"))].into_iter().collect(),
    );
    simulation.run.regimen.administrations.clear();
    let compiled =
        CompiledSensitivityDocument::compile(&document, &simulation.with_respect_to).unwrap();
    let mut observations = Vec::new();
    for (output, values) in [("y1", [2.2, 2.4, 2.6, 2.8]), ("y2", [4., 4.8, 5.2, 6.])] {
        for (row, value) in values.into_iter().enumerate() {
            observations.push(GaussianObservation {
                row,
                output: output.into(),
                value: q(value, "mg/L"),
                error: GaussianError {
                    additive_sd: q(0., "mg/L"),
                    proportional_sd: 1.,
                    model: GaussianErrorModel::Normal,
                },
                censoring: None,
            });
        }
    }
    (
        compiled,
        GaussianFitRequest {
            objective: GaussianObjectiveRequest {
                simulation,
                observations,
                priors: vec![],
            },
            bounds: vec![FitBound {
                parameter: "theta".into(),
                lower: q(1., "mg/L"),
                upper: q(4., "mg/L"),
            }],
            max_evaluations: 100,
            max_iterations: 50,
            gradient_tolerance: 1e-7,
        },
    )
}
#[test]
fn coupled_profile_fit_resolves_a_free_score_beside_an_active_bound() {
    let source="model profiled_boundary\ntime_unit = h\nparameters\n a = 3.8 [mg/L] estimated\n b = 0.4 [mg/L] estimated\n decay = 1 [1/h] fixed\nstates\n unused = 1 [1]\ndynamics\n d/dt(unused) = -decay * unused\noutputs\n y1 = a + b [mg/L]\n y2 = a - b [mg/L]\nend\n";
    let document = qspquest_modeling::language::parse(source).unwrap().document;
    let (_, mut request) = fixture();
    request.objective.simulation.with_respect_to = vec!["a".into(), "b".into()];
    request
        .objective
        .simulation
        .derivative_absolute_tolerances
        .clear();
    for name in ["a", "b"] {
        request
            .objective
            .simulation
            .derivative_absolute_tolerances
            .insert(
                name.into(),
                [("unused".into(), q(1e-12, "L/mg"))].into_iter().collect(),
            );
    }
    for (row, value) in request
        .objective
        .observations
        .iter_mut()
        .zip([4., 5.5, 6.5, 8., 2., 3.5, 4.5, 6.])
    {
        row.value = q(value, "mg/L");
    }
    request.bounds = vec![
        FitBound {
            parameter: "a".into(),
            lower: q(2., "mg/L"),
            upper: q(4.5, "mg/L"),
        },
        FitBound {
            parameter: "b".into(),
            lower: q(0., "mg/L"),
            upper: q(1.5, "mg/L"),
        },
    ];
    request.max_evaluations = 500;
    request.max_iterations = 200;
    let compiled = CompiledSensitivityDocument::compile(
        &document,
        &request.objective.simulation.with_respect_to,
    )
    .unwrap();
    // Equal within-output scatter makes the constrained profile optimum
    // a=4.5, b=1, with a nonzero normal score and a zero free score.
    for (a, b) in [(2.2, 0.2), (4.4, 1.4)] {
        request
            .objective
            .simulation
            .run
            .parameters
            .insert("a".into(), q(a, "mg/L"));
        request
            .objective
            .simulation
            .run
            .parameters
            .insert("b".into(), q(b, "mg/L"));
        let result = compiled
            .fit_profiled_proportional_gaussian(&request)
            .unwrap();
        assert_eq!(result.fit.status, FitStatus::Converged, "{result:?}");
        assert!((result.fit.parameters["a"].value - 4.5).abs() < 1e-8);
        assert!((result.fit.parameters["b"].value - 1.).abs() < 1e-6);
        assert!(result.fit.projected_gradient_norm <= request.gradient_tolerance);
    }
}
fn oracle(theta: f64) -> (f64, [f64; 2]) {
    let mut nll = 0.;
    let mut scales = [0.; 2];
    for (i, (prediction, values)) in [
        (theta, [2.2, 2.4, 2.6, 2.8]),
        (2. * theta, [4., 4.8, 5.2, 6.]),
    ]
    .into_iter()
    .enumerate()
    {
        let variance = values
            .iter()
            .map(|y| (y / prediction - 1.).powi(2))
            .sum::<f64>()
            / 4.;
        scales[i] = variance.sqrt();
        nll += values
            .iter()
            .map(|y| {
                0.5 * ((y - prediction).powi(2) / (variance * prediction * prediction)
                    + (2. * std::f64::consts::PI * variance * prediction * prediction).ln())
            })
            .sum::<f64>();
    }
    (nll, scales)
}
#[test]
fn profiled_objective_matches_independent_likelihood_and_score() {
    let (compiled, request) = fixture();
    let (actual, scales) = compiled
        .profiled_proportional_gaussian_objective(&request.objective)
        .unwrap();
    let (expected, sd) = oracle(1.8);
    assert!((actual.negative_log_likelihood - expected).abs() < 1e-10);
    assert!((scales["y1"] - sd[0]).abs() < 1e-12);
    assert!((scales["y2"] - sd[1]).abs() < 1e-12);
    let score = (oracle(1.8 + 1e-5).0 - oracle(1.8 - 1e-5).0) / 2e-5;
    assert!((actual.gradient[0].value - score).abs() < 1e-7);
    let known = compiled.gaussian_objective(&request.objective).unwrap();
    assert_ne!(known.identity.algorithm, actual.identity.algorithm);
    assert_ne!(
        known.identity.run_request_hash,
        actual.identity.run_request_hash
    );
    let mut with_prior = request.objective.clone();
    with_prior.priors.push(GaussianPrior {
        parameter: "theta".into(),
        mean: q(2., "mg/L"),
        sd: q(0.3, "mg/L"),
    });
    let (prior, _) = compiled
        .profiled_proportional_gaussian_objective(&with_prior)
        .unwrap();
    assert!((prior.gradient[0].value - actual.gradient[0].value - (1.8 - 2.) / 0.09).abs() < 1e-10);
}
#[test]
fn native_joint_fit_matches_closed_form_at_multiple_starts_and_solvers() {
    let (compiled, request) = fixture();
    for solver in [Solver::DiffsolBdf, Solver::DiffsolRodas5p] {
        for start in [1.3, 1.8, 3.5] {
            let mut current = request.clone();
            current.objective.simulation.run.solver = solver;
            current
                .objective
                .simulation
                .run
                .parameters
                .insert("theta".into(), q(start, "mg/L"));
            let result = compiled
                .fit_profiled_proportional_gaussian(&current)
                .unwrap();
            assert_eq!(result.fit.status, FitStatus::Converged, "{result:?}");
            assert!((result.fit.parameters["theta"].value - 2.5).abs() < 1e-7);
            let (nll, sd) = oracle(2.5);
            assert!((result.fit.objective.negative_log_likelihood - nll).abs() < 1e-9);
            assert!((result.residual_scales["y1"] - sd[0]).abs() < 1e-8);
            assert!((result.residual_scales["y2"] - sd[1]).abs() < 1e-8);
            assert!(result.fit.evaluations <= current.max_evaluations);
        }
    }
}
#[test]
fn profiled_fit_preserves_work_limits_and_observation_units() {
    let (compiled, request) = fixture();
    let mut limited = request.clone();
    limited.max_evaluations = 1;
    let result = compiled
        .fit_profiled_proportional_gaussian(&limited)
        .unwrap();
    assert_eq!(result.fit.status, FitStatus::EvaluationLimit);
    assert_eq!(result.fit.evaluations, 1);
    let (baseline, scales) = compiled
        .profiled_proportional_gaussian_objective(&request.objective)
        .unwrap();
    let mut converted = request.objective.clone();
    for row in &mut converted.observations {
        row.value = q(row.value.value * 1000., "ug/L");
    }
    let (other, other_scales) = compiled
        .profiled_proportional_gaussian_objective(&converted)
        .unwrap();
    assert!((baseline.objective - other.objective).abs() < 1e-10);
    for (output, scale) in scales {
        assert!((scale - other_scales[&output]).abs() < 1e-14);
    }
}
#[test]
fn unsupported_rows_and_degenerate_scales_refuse_without_poisoning_valid_calls() {
    let (compiled, request) = fixture();
    for mode in 0..7 {
        let mut bad = request.objective.clone();
        match mode {
            0 => bad.observations[0].censoring = Some(GaussianCensoring::Left),
            1 => bad.observations[0].error.model = GaussianErrorModel::Lognormal,
            2 => bad.observations[0].error.additive_sd.value = 0.1,
            3 => bad.observations[0].error.proportional_sd = 0.2,
            4 => bad.observations.retain(|r| r.output == "y1" && r.row == 0),
            5 => {
                bad.simulation
                    .run
                    .parameters
                    .insert("theta".into(), q(2., "mg/L"));
                for row in &mut bad.observations {
                    row.value.value = if row.output == "y1" { 2. } else { 4. };
                }
            }
            _ => {
                bad.simulation
                    .run
                    .parameters
                    .insert("theta".into(), q(0., "mg/L"));
            }
        }
        let error = compiled
            .profiled_proportional_gaussian_objective(&bad)
            .unwrap_err();
        assert!(matches!(
            error.code,
            ErrorCode::InvalidInput | ErrorCode::Domain
        ));
        assert!(compiled
            .profiled_proportional_gaussian_objective(&request.objective)
            .is_ok());
    }
}

use qspquest_core::{
    expression::{Binary, Expr, Function},
    fit::*,
    model::{Initial, ModelDocument, Output, Parameter, Quantity, Transform},
    run::SensitivityRequest,
    ErrorCode,
};
use qspquest_modeling::document::sensitivity::CompiledSensitivityDocument;

fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}

fn fixture() -> (
    std::sync::Arc<CompiledSensitivityDocument>,
    PopulationFitRequest,
    Vec<[f64; 2]>,
) {
    // Two independent log outputs make both random effects linear in eta.
    // Integrating correlated Gaussian eta is then an exact two-dimensional
    // Gaussian oracle for the FOCEI objective and individual effects.
    let mut model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    model.model_id = "synthetic_two_log_effects".into();
    model.parameters[0].default = q(1., "1");
    model.parameters[1].default = q(1., "1");
    model.parameters.push(Parameter {
        name: "k".into(),
        default: q(1., "1/h"),
        bounds: None,
        transform: Transform::Identity,
        fixed: true,
        description: String::new(),
    });
    model.states[0].unit = "1".into();
    model.states[0].initial = Initial::Quantity {
        quantity: q(0., "1"),
    };
    model.states[0].rhs = Expr::binary(
        Binary::Multiply,
        Expr::number(-1.),
        Expr::binary(Binary::Multiply, Expr::symbol("k"), Expr::symbol("central")),
    );
    model.states[0].dosing = None;
    model.outputs[0].unit = "1".into();
    model.outputs[0].expression = Expr::call(Function::Log, vec![Expr::symbol("cl")]);
    model.outputs.push(Output {
        name: "log_v".into(),
        unit: "1".into(),
        expression: Expr::call(Function::Log, vec![Expr::symbol("v")]),
    });
    let compiled =
        CompiledSensitivityDocument::compile(&model, &["cl".into(), "v".into()]).unwrap();
    let mut simulation: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    simulation.derivative_absolute_tolerances.clear();
    for name in ["cl", "v"] {
        simulation.derivative_absolute_tolerances.insert(
            name.into(),
            std::collections::BTreeMap::from([("central".into(), q(1e-12, "1"))]),
        );
        simulation.run.parameters.insert(name.into(), q(1., "1"));
    }
    simulation.run.regimen.administrations.clear();
    let values = vec![[-0.2, -0.3], [-0.1, 0.], [0.1, 0.], [0.2, 0.3]];
    let subjects = values
        .iter()
        .map(|pair| GaussianObjectiveRequest {
            simulation: simulation.clone(),
            observations: [
                GaussianObservation {
                    censoring: None,
                    row: 0,
                    output: "cp".into(),
                    value: q(pair[0], "1"),
                    error: GaussianError {
                        model: GaussianErrorModel::Normal,
                        additive_sd: q(0.1, "1"),
                        proportional_sd: 0.,
                    },
                },
                GaussianObservation {
                    censoring: None,
                    row: 0,
                    output: "log_v".into(),
                    value: q(pair[1], "1"),
                    error: GaussianError {
                        model: GaussianErrorModel::Normal,
                        additive_sd: q(0.2, "1"),
                        proportional_sd: 0.,
                    },
                },
            ]
            .into(),
            priors: vec![],
        })
        .collect();
    let request = PopulationFitRequest {
        subjects,
        fixed_effects: vec![
            FitBound {
                parameter: "cl".into(),
                lower: q(0.7, "1"),
                upper: q(1.3, "1"),
            },
            FitBound {
                parameter: "v".into(),
                lower: q(0.7, "1"),
                upper: q(1.3, "1"),
            },
        ],
        random_effects: vec![
            PopulationRandomEffect {
                parameter: "cl".into(),
            },
            PopulationRandomEffect {
                parameter: "v".into(),
            },
        ],
        omega: vec![vec![0.04, 0.018], vec![0.018, 0.09]],
        omega_diagonal_bounds: vec![[0.04, 0.04], [0.09, 0.09]],
        eta_bound: 1.5,
        max_evaluations: 10_000,
        max_iterations: 1,
        gradient_tolerance: 0.25,
        seed: 7,
        burn_in_iterations: None,
        uncertainty: false,
        additive_error_fit: None,
        covariate_effect: None,
    };
    (compiled, request, values)
}

#[test]
fn correlated_two_effect_focei_matches_exact_gaussian_marginal() {
    let (compiled, request, values) = fixture();
    let result = compiled.fit_population_focei(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged);
    assert_eq!(result.iterations, 0);
    assert_eq!(
        result.objective_kind,
        PopulationObjectiveKind::FoceiObjectiveFunctionValue
    );
    assert_eq!(result.omega, request.omega);
    assert_eq!(result.fixed_effects["cl"].value, 1.);
    assert_eq!(result.fixed_effects["v"].value, 1.);

    // Marginal y has covariance Omega + diag(0.1^2, 0.2^2).
    let covariance = [[0.05, 0.018], [0.018, 0.13]];
    let determinant = covariance[0][0] * covariance[1][1] - covariance[0][1] * covariance[1][0];
    let inverse = [
        [
            covariance[1][1] / determinant,
            -covariance[0][1] / determinant,
        ],
        [
            -covariance[1][0] / determinant,
            covariance[0][0] / determinant,
        ],
    ];
    let mut oracle_ofv = 0.;
    for (index, value) in values.iter().enumerate() {
        let weighted = [
            inverse[0][0] * value[0] + inverse[0][1] * value[1],
            inverse[1][0] * value[0] + inverse[1][1] * value[1],
        ];
        oracle_ofv += value[0] * weighted[0] + value[1] * weighted[1] + determinant.ln();
        let eta = [
            0.04 * weighted[0] + 0.018 * weighted[1],
            0.018 * weighted[0] + 0.09 * weighted[1],
        ];
        for axis in 0..2 {
            assert!((result.subjects[index].eta[axis] - eta[axis]).abs() < 1e-4);
        }
    }
    assert!(
        (result.objective - oracle_ofv).abs() < 1e-4,
        "{} vs {}",
        result.objective,
        oracle_ofv
    );
    let rows = result.fitted_observations.unwrap();
    assert_eq!(rows.len(), 8);
    for row in rows {
        let axis = row.observation_index;
        assert_eq!(row.pred.value, 0.);
        assert!((row.ipred.value - result.subjects[row.subject_index].eta[axis]).abs() < 1e-4);
    }
}

#[test]
fn correlated_two_effect_focei_rejects_unqualified_shapes() {
    let (compiled, mut request, _) = fixture();
    request.omega[1][0] = -0.018;
    assert_eq!(
        compiled.fit_population_focei(&request).unwrap_err().code,
        ErrorCode::Unsupported
    );
    request.omega[1][0] = 0.018;
    request.omega[0][1] = 0.1;
    request.omega[1][0] = 0.1;
    assert_eq!(
        compiled.fit_population_focei(&request).unwrap_err().code,
        ErrorCode::InvalidInput
    );
    request.omega[0][1] = 0.018;
    request.omega[1][0] = 0.018;
    request.omega_diagonal_bounds[0] = [0.01, 0.1];
    assert_eq!(
        compiled.fit_population_focei(&request).unwrap_err().code,
        ErrorCode::Unsupported
    );
    request.omega_diagonal_bounds[0] = [0.04, 0.04];
    request.uncertainty = true;
    assert_eq!(
        compiled.fit_population_focei(&request).unwrap_err().code,
        ErrorCode::Unsupported
    );
}

#[test]
fn correlated_two_effect_focei_recovers_two_fixed_effects() {
    let (compiled, mut request, _) = fixture();
    let shifts = [0.9_f64.ln(), 1.1_f64.ln()];
    for subject in &mut request.subjects {
        for (axis, observation) in subject.observations.iter_mut().enumerate() {
            observation.value.value += shifts[axis];
        }
    }
    request.max_iterations = 80;
    request.gradient_tolerance = 0.005;
    let result = compiled.fit_population_focei(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    assert!((result.fixed_effects["cl"].value - 0.9).abs() < 0.01);
    assert!((result.fixed_effects["v"].value - 1.1).abs() < 0.01);
    assert_eq!(result.subjects.len(), 4);
    assert_eq!(result.fitted_observations.unwrap().len(), 8);
}

#[test]
fn correlated_two_effect_focei_runs_dosed_pk_and_projects_individual_predictions() {
    let mut model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    model.parameters[0].default = q(0.6, "L/h");
    model.parameters[1].default = q(4.5, "L");
    let compiled =
        CompiledSensitivityDocument::compile(&model, &["cl".into(), "v".into()]).unwrap();
    let mut simulation: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    simulation.run.parameters.insert("cl".into(), q(0.6, "L/h"));
    simulation.run.parameters.insert("v".into(), q(4.5, "L"));
    let mut subjects = Vec::new();
    for eta in [
        [-0.15_f64, -0.12],
        [-0.05, 0.03],
        [0.06, -0.02],
        [0.14, 0.11],
    ] {
        let mut truth = simulation.clone();
        truth
            .run
            .parameters
            .insert("cl".into(), q(0.6 * eta[0].exp(), "L/h"));
        truth
            .run
            .parameters
            .insert("v".into(), q(4.5 * eta[1].exp(), "L"));
        let observed = compiled.run(&truth).unwrap();
        let observations = observed.result.outputs[0]
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
                    proportional_sd: 0.1,
                },
            })
            .collect();
        subjects.push(GaussianObjectiveRequest {
            simulation: simulation.clone(),
            observations,
            priors: vec![],
        });
    }
    let request = PopulationFitRequest {
        subjects,
        fixed_effects: vec![
            FitBound {
                parameter: "cl".into(),
                lower: q(0.4, "L/h"),
                upper: q(0.8, "L/h"),
            },
            FitBound {
                parameter: "v".into(),
                lower: q(3.5, "L"),
                upper: q(5.5, "L"),
            },
        ],
        random_effects: vec![
            PopulationRandomEffect {
                parameter: "cl".into(),
            },
            PopulationRandomEffect {
                parameter: "v".into(),
            },
        ],
        omega: vec![vec![0.04, 0.01], vec![0.01, 0.04]],
        omega_diagonal_bounds: vec![[0.04, 0.04], [0.04, 0.04]],
        eta_bound: 0.6,
        max_evaluations: 5000,
        max_iterations: 1,
        gradient_tolerance: 0.25,
        seed: 3,
        burn_in_iterations: None,
        uncertainty: false,
        additive_error_fit: None,
        covariate_effect: None,
    };
    let result = compiled.fit_population_focei(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged);
    assert!(result.objective.is_finite());
    assert_eq!(result.subjects.len(), 4);
    let rows = result.fitted_observations.unwrap();
    assert_eq!(
        rows.len(),
        request
            .subjects
            .iter()
            .map(|s| s.observations.len())
            .sum::<usize>()
    );
    for (index, subject) in result.subjects.iter().enumerate() {
        assert_eq!(subject.eta.len(), 2);
        let mut individual = simulation.clone();
        individual
            .run
            .parameters
            .insert("cl".into(), q(0.6 * subject.eta[0].exp(), "L/h"));
        individual
            .run
            .parameters
            .insert("v".into(), q(4.5 * subject.eta[1].exp(), "L"));
        let projected = compiled.run(&individual).unwrap();
        for row in rows.iter().filter(|row| row.subject_index == index) {
            assert!((row.ipred.value - projected.result.outputs[0].values[row.row]).abs() < 1e-8);
            assert_eq!(row.time.value, projected.result.times[row.row]);
            assert_eq!(row.side, projected.result.sides[row.row]);
        }
    }
}

#[test]
fn rodas5p_population_dispatch_preserves_objective_and_solver_identity() {
    let (compiled, mut request, _) = fixture();
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

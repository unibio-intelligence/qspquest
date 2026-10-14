use qspquest_core::{
    expression::{Binary, Expr, Function},
    fit::*,
    model::{
        Covariate, Initial, Interpolation, ModelDocument, Output, Parameter, Quantity, Transform,
    },
    regimen::CovariateChange,
    run::SensitivityRequest,
};
use qspquest_modeling::document::sensitivity::CompiledSensitivityDocument;

fn quantity(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}

fn fresh_joint_gaussian(
    near_boundary: bool,
    fitted_residual: bool,
    seed: u64,
) -> (
    std::sync::Arc<CompiledSensitivityDocument>,
    PopulationFitRequest,
) {
    let source = "model fresh_joint_gaussian\ntime_unit = h\nparameters\n theta_intercept = 4.0551999668446745 [1] estimated\n theta_slope = 2.0137527074704766 [1] estimated\n center = 1 [h] fixed\n scale = 1 [h] fixed\n decay = 0 [1/h] fixed\nstates\n anchor = 0 [1]\ndynamics\n d/dt(anchor) = -decay * anchor\noutputs\n z = log(theta_intercept) + ((time - center) / scale) * log(theta_slope) [1]\nend\n";
    let model = qspquest_modeling::language::parse(source).unwrap().document;
    let names = vec!["theta_intercept".into(), "theta_slope".into()];
    let compiled = CompiledSensitivityDocument::compile(&model, &names).unwrap();
    let simulation: SensitivityRequest = serde_json::from_value(serde_json::json!({
        "schema":"qspquest.sensitivity/v0.1", "with_respect_to":names,
        "derivative_absolute_tolerances":{
            "theta_intercept":{"anchor":{"value":1e-12,"unit":"1"}},
            "theta_slope":{"anchor":{"value":1e-12,"unit":"1"}}},
        "run":{"schema":"qspquest.run/v0.1","solver":"diffsol_bdf",
            "parameters":{"theta_intercept":{"value":3.0,"unit":"1"},"theta_slope":{"value":1.5,"unit":"1"}},
            "budgets":{"solver_callbacks":10000,"events":100,"output_values":1000},
            "regimen":{"start":{"value":0,"unit":"h"},"end":{"value":2,"unit":"h"},"administrations":[],
                "samples":[{"value":0,"unit":"h"},{"value":1,"unit":"h"},{"value":2,"unit":"h"}],
                "rtol":1e-10,"atol":1e-12}}})).unwrap();
    let initial_sd = if fitted_residual { 0.6 } else { 0.25 };
    let variance: f64 = if near_boundary { 0.004 } else { 0.16 };
    let subjects = (0..32)
        .map(|i| {
            let angle = 2. * std::f64::consts::PI * (i as f64) / 32.;
            let a = 2_f64.sqrt() * angle.cos();
            let b = 2_f64.sqrt() * angle.sin();
            let eta = [variance.sqrt() * a, 0.3 * (0.6 * a + 0.8 * b)];
            GaussianObjectiveRequest {
                simulation: simulation.clone(),
                priors: vec![],
                observations: (0..3)
                    .map(|j| GaussianObservation {
                        row: j,
                        output: "z".into(),
                        censoring: None,
                        value: quantity(
                            1.4 + eta[0]
                                + (j as f64 - 1.) * (0.7 + eta[1])
                                + 0.25 * 2_f64.sqrt() * ((j + 3) as f64 * angle).cos(),
                            "1",
                        ),
                        error: GaussianError {
                            model: GaussianErrorModel::Normal,
                            additive_sd: quantity(initial_sd, "1"),
                            proportional_sd: 0.,
                        },
                    })
                    .collect(),
            }
        })
        .collect();
    let request = PopulationFitRequest {
        subjects,
        fixed_effects: vec![
            FitBound {
                parameter: "theta_intercept".into(),
                lower: quantity((-0.2_f64).exp(), "1"),
                upper: quantity(3_f64.exp(), "1"),
            },
            FitBound {
                parameter: "theta_slope".into(),
                lower: quantity((-0.7_f64).exp(), "1"),
                upper: quantity(2.1_f64.exp(), "1"),
            },
        ],
        random_effects: vec![
            PopulationRandomEffect {
                parameter: "theta_intercept".into(),
            },
            PopulationRandomEffect {
                parameter: "theta_slope".into(),
            },
        ],
        omega: vec![vec![variance.max(0.016) * 0.25, 0.], vec![0., 0.09 * 0.25]],
        omega_diagonal_bounds: vec![[0.001, 0.8], [0.001, 0.8]],
        eta_bound: 5.,
        max_evaluations: 200_000,
        max_iterations: 2000,
        burn_in_iterations: Some(200),
        gradient_tolerance: 0.001,
        seed,
        uncertainty: !near_boundary && !fitted_residual,
        additive_error_fit: fitted_residual.then(|| PopulationAdditiveErrorFit {
            output: "z".into(),
            initial: quantity(initial_sd, "1"),
            lower: quantity(0.05, "1"),
            upper: quantity(0.8, "1"),
        }),
        covariate_effect: None,
    };
    (compiled, request)
}

#[test]
fn fresh_joint_gaussian_fitted_residual_recovers_variance_components() {
    let (compiled, request) = fresh_joint_gaussian(false, true, 2026101302);
    let result = compiled.fit_population_saem(&request).unwrap();
    let sd = result
        .estimated_residual_error
        .as_ref()
        .unwrap()
        .additive_sd
        .value;
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    assert!((sd - 0.25).abs() <= 0.025 + 0.05 * 0.25, "residual SD {sd}");
    for (axis, truth) in [0.16, 0.09].iter().enumerate() {
        assert!(
            (result.omega[axis][axis] - truth).abs() <= 0.02 + 0.1 * truth,
            "Omega {:?}",
            result.omega
        );
    }
    assert!(result.evaluations <= request.max_evaluations);
    let actual = fresh_joint_exact_ofv(&request, &result, sd);
    let truth = fresh_joint_reference_ofv(&request, 0.16);
    assert!(
        (result.objective - actual).abs() < 0.001,
        "native OFV {} vs exact {actual}",
        result.objective
    );
    assert!(
        actual - truth <= 0.1,
        "marginal objective gap {}",
        actual - truth
    );
}

fn fresh_joint_ofv(
    request: &PopulationFitRequest,
    mu: [f64; 2],
    omega: [[f64; 2]; 2],
    sd: f64,
) -> f64 {
    let c = [
        [omega[0][0] + sd * sd / 3., omega[0][1]],
        [omega[1][0], omega[1][1] + sd * sd / 2.],
    ];
    let determinant = c[0][0] * c[1][1] - c[0][1] * c[1][0];
    let mut result =
        request.subjects.len() as f64 * (libm::log(6. * determinant) + 2. * libm::log(sd));
    for subject in &request.subjects {
        let z = subject
            .observations
            .iter()
            .map(|x| x.value.value)
            .collect::<Vec<_>>();
        let b = [(z[0] + z[1] + z[2]) / 3., (z[2] - z[0]) / 2.];
        let d = [b[0] - mu[0], b[1] - mu[1]];
        result += (c[1][1] * d[0] * d[0] + c[0][0] * d[1] * d[1] - 2. * c[0][1] * d[0] * d[1])
            / determinant;
        result += z
            .iter()
            .enumerate()
            .map(|(j, v)| (v - b[0] - (j as f64 - 1.) * b[1]).powi(2) / (sd * sd))
            .sum::<f64>();
    }
    result
}

fn fresh_joint_exact_ofv(
    request: &PopulationFitRequest,
    result: &PopulationFitResult,
    sd: f64,
) -> f64 {
    fresh_joint_ofv(
        request,
        [
            result.fixed_effects["theta_intercept"].value.ln(),
            result.fixed_effects["theta_slope"].value.ln(),
        ],
        [
            [result.omega[0][0], result.omega[0][1]],
            [result.omega[1][0], result.omega[1][1]],
        ],
        sd,
    )
}

fn fresh_joint_reference_ofv(request: &PopulationFitRequest, variance: f64) -> f64 {
    let cross = 0.6 * (variance * 0.09).sqrt();
    fresh_joint_ofv(
        request,
        [1.4, 0.7],
        [[variance, cross], [cross, 0.09]],
        0.25,
    )
}

#[test]
fn fresh_joint_gaussian_native_covariance_matches_independent_information() {
    let (compiled, request) = fresh_joint_gaussian(false, false, 2026101301);
    let result = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged);
    let covariance = &result.uncertainty.as_ref().unwrap().covariance;
    let point = [
        result.fixed_effects["theta_intercept"].value,
        result.fixed_effects["theta_slope"].value,
        result.omega[0][0],
        result.omega[1][1],
        result.omega[0][1],
    ];
    let steps = point.map(|value| 1e-4 * value.abs().max(0.01));
    let objective = |offset: [f64; 5]| {
        let candidate = std::array::from_fn::<_, 5, _>(|i| point[i] + offset[i] * steps[i]);
        fresh_joint_ofv(
            &request,
            [candidate[0].ln(), candidate[1].ln()],
            [[candidate[2], candidate[4]], [candidate[4], candidate[3]]],
            0.25,
        )
    };
    let center = objective([0.; 5]);
    let mut augmented = [[0.; 10]; 5];
    for i in 0..5 {
        let mut offset = [0.; 5];
        offset[i] = 1.;
        let plus = objective(offset);
        offset[i] = -1.;
        let minus = objective(offset);
        augmented[i][i] = (plus - 2. * center + minus) / steps[i].powi(2);
        for j in 0..i {
            let mut values = [0.; 4];
            for (k, (a, b)) in [(1., 1.), (1., -1.), (-1., 1.), (-1., -1.)]
                .into_iter()
                .enumerate()
            {
                let mut offset = [0.; 5];
                offset[i] = a;
                offset[j] = b;
                values[k] = objective(offset);
            }
            augmented[i][j] =
                (values[0] - values[1] - values[2] + values[3]) / (4. * steps[i] * steps[j]);
            augmented[j][i] = augmented[i][j];
        }
        augmented[i][i + 5] = 1.;
    }
    for i in 0..5 {
        let pivot = (i..5)
            .max_by(|&a, &b| augmented[a][i].abs().total_cmp(&augmented[b][i].abs()))
            .unwrap();
        augmented.swap(i, pivot);
        let diagonal = augmented[i][i];
        assert!(diagonal.abs() > 1e-12);
        for j in 0..10 {
            augmented[i][j] /= diagonal;
        }
        for row in 0..5 {
            if row != i {
                let multiplier = augmented[row][i];
                for col in 0..10 {
                    augmented[row][col] -= multiplier * augmented[i][col];
                }
            }
        }
    }
    for i in 0..5 {
        for j in 0..5 {
            let expected = 2. * augmented[i][j + 5];
            let scale = (4. * augmented[i][i + 5] * augmented[j][j + 5]).sqrt();
            let error = (covariance[i][j] - expected).abs() / scale;
            assert!(error <= 0.01, "covariance {i},{j} error {error}");
        }
    }
    assert!(result.evaluations <= request.max_evaluations);
}

#[test]
fn fresh_joint_gaussian_boundary_chains_mix() {
    let (compiled, request) = fresh_joint_gaussian(true, false, 2026101303);
    let result = compiled.fit_population_saem(&request).unwrap();
    let diagnostics = result.saem_diagnostics.as_ref().unwrap();
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    assert!(
        diagnostics
            .post_burn_max_lag_one_correlation_by_axis
            .as_ref()
            .unwrap()
            .iter()
            .all(|x| *x <= 0.9),
        "{diagnostics:?}"
    );
    assert!((result.fixed_effects["theta_intercept"].value.ln() - 1.4).abs() <= 0.05);
    assert!((result.fixed_effects["theta_slope"].value.ln() - 0.7).abs() <= 0.05);
    assert!(result.evaluations <= request.max_evaluations);
}

#[test]
fn saem_correlation_boundary_keeps_point_fit_but_refuses_uncertainty() {
    for sign in [-1., 1.] {
        let (compiled, mut request) = fresh_joint_gaussian(false, false, 2026101309);
        request.omega = vec![
            vec![0.16, sign * 0.95 * 0.12],
            vec![sign * 0.95 * 0.12, 0.09],
        ];
        request.omega_diagonal_bounds = vec![[0.16, 0.16], [0.09, 0.09]];
        request.gradient_tolerance = 0.003;
        request.uncertainty = false;
        for (index, subject) in request.subjects.iter_mut().enumerate() {
            let angle = 2. * std::f64::consts::PI * index as f64 / 32.;
            let a = 2_f64.sqrt() * angle.sin();
            let b = 2_f64.sqrt() * angle.cos();
            subject
                .simulation
                .run
                .parameters
                .insert("theta_intercept".into(), quantity(1_f64.exp(), "1"));
            subject
                .simulation
                .run
                .parameters
                .insert("theta_slope".into(), quantity(0.3_f64.exp(), "1"));
            subject.simulation.run.regimen.samples = (0..6)
                .map(|row| quantity(2. * row as f64 / 5., "h"))
                .collect();
            let template = subject.observations[0].clone();
            subject.observations = (0..6)
                .map(|row| {
                    let mut observation = template.clone();
                    let time = 2. * row as f64 / 5.;
                    observation.row = row;
                    observation.value = quantity(
                        1.4 + 0.4 * a
                            + (time - 1.)
                                * (0.7
                                    + sign
                                        * 0.3
                                        * (0.97 * a + (1_f64 - 0.97_f64.powi(2)).sqrt() * b))
                            + 0.05
                                * 2_f64.sqrt()
                                * (2. * angle + 2. * std::f64::consts::PI * row as f64 / 6.).sin(),
                        "1",
                    );
                    observation.error.additive_sd = quantity(0.05, "1");
                    observation
                })
                .collect();
        }
        let point = compiled.fit_population_saem(&request).unwrap();
        assert_eq!(point.status, FitStatus::Converged, "{point:?}");
        let rho = point.omega[0][1] / (point.omega[0][0] * point.omega[1][1]).sqrt();
        assert!((rho - sign * 0.95).abs() < 1e-12, "rho={rho}");
        assert!(point.uncertainty.is_none());
        assert!(point.evaluations <= request.max_evaluations);
        request.uncertainty = true;
        let error = compiled.fit_population_saem(&request).unwrap_err();
        assert_eq!(error.code, qspquest_core::ErrorCode::Unsupported);
        assert!(error.message.contains("correlation"), "{error:?}");
    }
}

fn fixture() -> (
    std::sync::Arc<CompiledSensitivityDocument>,
    PopulationFitRequest,
) {
    let model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let mut simulation: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    simulation.with_respect_to = vec!["cl".into()];
    simulation.derivative_absolute_tolerances.remove("v");
    simulation
        .run
        .parameters
        .insert("cl".into(), quantity(0.5, "L/h"));
    simulation
        .run
        .parameters
        .insert("v".into(), quantity(4.5, "L"));
    let compiled = CompiledSensitivityDocument::compile(&model, &["cl".into()]).unwrap();
    let eta: [f64; 8] = [-0.45, -0.32, -0.21, -0.12, 0.12, 0.21, 0.32, 0.45];
    let times = compiled.run(&simulation).unwrap().result.times;
    let subjects = eta
        .iter()
        .map(|eta| GaussianObjectiveRequest {
            simulation: simulation.clone(),
            observations: times
                .iter()
                .enumerate()
                .filter(|(row, _)| *row > 0)
                .map(|(row, time)| GaussianObservation {
                    censoring: None,
                    row,
                    output: "cp".into(),
                    value: quantity(6.0 / 4.5 * (-0.6 * (*eta).exp() * time / 4.5).exp(), "mg/L"),
                    error: GaussianError {
                        model: GaussianErrorModel::Normal,
                        additive_sd: quantity(0.02, "mg/L"),
                        proportional_sd: 0.0,
                    },
                })
                .collect(),
            priors: Vec::new(),
        })
        .collect();
    let request = PopulationFitRequest {
        subjects,
        fixed_effects: vec![FitBound {
            parameter: "cl".into(),
            lower: quantity(0.2, "L/h"),
            upper: quantity(1.0, "L/h"),
        }],
        random_effects: vec![PopulationRandomEffect {
            parameter: "cl".into(),
        }],
        omega: vec![vec![0.1]],
        omega_diagonal_bounds: vec![[0.01, 0.5]],
        eta_bound: 2.0,
        max_evaluations: 20_000,
        max_iterations: 800,
        gradient_tolerance: 0.001,
        seed: 20260929,
        burn_in_iterations: None,
        uncertainty: false,
        additive_error_fit: None,
        covariate_effect: None,
    };
    (compiled, request)
}

fn two_effect_fixture() -> (
    std::sync::Arc<CompiledSensitivityDocument>,
    PopulationFitRequest,
) {
    let mut model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    model.model_id = "saem_two_log_effects".into();
    model.parameters[0].default = quantity(1., "1");
    model.parameters[1].default = quantity(1., "1");
    model.parameters.push(Parameter {
        name: "k".into(),
        default: quantity(1., "1/h"),
        bounds: None,
        transform: Transform::Identity,
        fixed: true,
        description: String::new(),
    });
    model.states[0].unit = "1".into();
    model.states[0].initial = Initial::Quantity {
        quantity: quantity(0., "1"),
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
    simulation.with_respect_to = vec!["cl".into(), "v".into()];
    simulation.derivative_absolute_tolerances.clear();
    for name in ["cl", "v"] {
        simulation.derivative_absolute_tolerances.insert(
            name.into(),
            std::collections::BTreeMap::from([("central".into(), quantity(1e-12, "1"))]),
        );
        simulation
            .run
            .parameters
            .insert(name.into(), quantity(1., "1"));
    }
    simulation.run.regimen.administrations.clear();
    let effects = [
        [-0.24, -0.19],
        [-0.18, -0.13],
        [-0.11, -0.05],
        [-0.03, 0.02],
        [0.03, -0.02],
        [0.11, 0.05],
        [0.18, 0.13],
        [0.24, 0.19],
    ];
    let subjects = effects
        .iter()
        .map(|eta| GaussianObjectiveRequest {
            simulation: simulation.clone(),
            observations: [
                GaussianObservation {
                    censoring: None,
                    row: 0,
                    output: "cp".into(),
                    value: quantity(0.9_f64.ln() + eta[0], "1"),
                    error: GaussianError {
                        model: GaussianErrorModel::Normal,
                        additive_sd: quantity(0.06, "1"),
                        proportional_sd: 0.,
                    },
                },
                GaussianObservation {
                    censoring: None,
                    row: 0,
                    output: "log_v".into(),
                    value: quantity(1.1_f64.ln() + eta[1], "1"),
                    error: GaussianError {
                        model: GaussianErrorModel::Normal,
                        additive_sd: quantity(0.06, "1"),
                        proportional_sd: 0.,
                    },
                },
            ]
            .into(),
            priors: vec![],
        })
        .collect();
    (
        compiled,
        PopulationFitRequest {
            subjects,
            fixed_effects: vec![
                FitBound {
                    parameter: "cl".into(),
                    lower: quantity(0.7, "1"),
                    upper: quantity(1.3, "1"),
                },
                FitBound {
                    parameter: "v".into(),
                    lower: quantity(0.7, "1"),
                    upper: quantity(1.3, "1"),
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
            omega: vec![vec![0.04, 0.012], vec![0.012, 0.04]],
            omega_diagonal_bounds: vec![[0.005, 0.12], [0.005, 0.12]],
            eta_bound: 1.5,
            max_evaluations: 40_000,
            max_iterations: 800,
            gradient_tolerance: 0.001,
            seed: 20260930,
            burn_in_iterations: None,
            uncertainty: false,
            additive_error_fit: None,
            covariate_effect: None,
        },
    )
}

fn two_effect_dosed_pk_fixture() -> (
    std::sync::Arc<CompiledSensitivityDocument>,
    PopulationFitRequest,
) {
    let model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let mut simulation: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    simulation
        .run
        .parameters
        .insert("cl".into(), quantity(0.5, "L/h"));
    simulation
        .run
        .parameters
        .insert("v".into(), quantity(4.0, "L"));
    let compiled =
        CompiledSensitivityDocument::compile(&model, &["cl".into(), "v".into()]).unwrap();
    let grid = compiled.run(&simulation).unwrap().result;
    let effects = [
        [-0.35, -0.22],
        [-0.28, 0.12],
        [-0.21, -0.08],
        [-0.14, 0.25],
        [-0.07, -0.18],
        [0.07, 0.18],
        [0.14, -0.25],
        [0.21, 0.08],
        [0.28, -0.12],
        [0.35, 0.22],
    ];
    let subjects = effects
        .iter()
        .map(|eta| {
            let cl = 0.6 * libm::exp(eta[0]);
            let v = 4.5 * libm::exp(eta[1]);
            GaussianObjectiveRequest {
                simulation: simulation.clone(),
                observations: grid
                    .times
                    .iter()
                    .enumerate()
                    .filter(|(row, time)| *row > 0 && **time > 0.)
                    .map(|(row, time)| GaussianObservation {
                        censoring: None,
                        row,
                        output: "cp".into(),
                        value: quantity(6. / v * libm::exp(-cl * time / v), "mg/L"),
                        error: GaussianError {
                            model: GaussianErrorModel::Normal,
                            additive_sd: quantity(0.02, "mg/L"),
                            proportional_sd: 0.,
                        },
                    })
                    .collect(),
                priors: Vec::new(),
            }
        })
        .collect();
    (
        compiled,
        PopulationFitRequest {
            subjects,
            fixed_effects: vec![
                FitBound {
                    parameter: "cl".into(),
                    lower: quantity(0.3, "L/h"),
                    upper: quantity(1., "L/h"),
                },
                FitBound {
                    parameter: "v".into(),
                    lower: quantity(2.5, "L"),
                    upper: quantity(7., "L"),
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
            omega: vec![vec![0.05, 0.], vec![0., 0.05]],
            omega_diagonal_bounds: vec![[0.005, 0.2], [0.005, 0.2]],
            eta_bound: 1.5,
            max_evaluations: 100_000,
            max_iterations: 1200,
            gradient_tolerance: 0.001,
            seed: 20260930,
            burn_in_iterations: None,
            uncertainty: false,
            additive_error_fit: None,
            covariate_effect: None,
        },
    )
}

#[test]
fn saem_two_effects_recovers_dosed_pk_and_publishes_fitted_rows() {
    let (compiled, request) = two_effect_dosed_pk_fixture();
    let fitted = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(fitted.status, FitStatus::Converged, "{fitted:?}");
    assert_eq!(
        fitted.objective_kind,
        PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue
    );
    assert!((fitted.fixed_effects["cl"].value - 0.6).abs() < 0.08);
    assert!((fitted.fixed_effects["v"].value - 4.5).abs() < 0.45);
    assert!(fitted.objective.is_finite());
    let rows = fitted.fitted_observations.unwrap();
    assert_eq!(rows.len(), 30);
    for row in rows {
        let cl = fitted.fixed_effects["cl"].value;
        let v = fitted.fixed_effects["v"].value;
        let eta = &fitted.subjects[row.subject_index].eta;
        let pred = 6. / v * libm::exp(-cl * row.time.value / v);
        let individual_cl = cl * libm::exp(eta[0]);
        let individual_v = v * libm::exp(eta[1]);
        let ipred = 6. / individual_v * libm::exp(-individual_cl * row.time.value / individual_v);
        assert!((row.pred.value - pred).abs() < 1e-6, "{row:?}");
        assert!((row.ipred.value - ipred).abs() < 1e-6, "{row:?}");
    }
}

#[test]
fn saem_two_effects_dosed_pk_converges_across_seeds() {
    let (compiled, mut request) = two_effect_dosed_pk_fixture();
    for seed in [17029, 32321] {
        request.seed = seed;
        let fitted = compiled.fit_population_saem(&request).unwrap();
        assert_eq!(
            fitted.status,
            FitStatus::Converged,
            "seed={seed}, {fitted:?}"
        );
        assert_eq!(
            fitted.objective_kind,
            PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue,
            "seed={seed}"
        );
        assert!(
            (fitted.fixed_effects["cl"].value - 0.6).abs() < 0.08,
            "seed={seed}"
        );
        assert!(
            (fitted.fixed_effects["v"].value - 4.5).abs() < 0.45,
            "seed={seed}"
        );
        assert_eq!(fitted.fitted_observations.unwrap().len(), 30);
    }
}

#[test]
fn saem_dosed_pk_covariance_certifies_with_requested_simulation_tolerances() {
    let (compiled, mut request) = two_effect_dosed_pk_fixture();
    let mut simulation = request.subjects[0].simulation.clone();
    simulation.run.regimen.end = quantity(2., "h");
    simulation.run.regimen.rtol = 1e-6;
    simulation.run.regimen.atol = Some(1e-8);
    simulation.run.regimen.samples.clear();
    simulation.run.regimen.observations = Some(
        serde_json::from_value(serde_json::Value::Array(
            (0..9)
                .map(|row| {
                    serde_json::json!({
                        "time": {"value": row as f64 * 0.25, "unit": "h"},
                        "side": if row == 8 { "pre" } else { "post" }
                    })
                })
                .collect(),
        ))
        .unwrap(),
    );
    let cl_effects = [-0.24, -0.18, -0.11, -0.03, 0.03, 0.11, 0.18, 0.24];
    let v_effects = [-0.03, 0.24, -0.19, 0.11, 0.18, -0.11, 0.03, -0.18];
    request.subjects = cl_effects
        .iter()
        .zip(v_effects)
        .map(|(eta_cl, eta_v)| {
            let cl = 0.6 * libm::exp(*eta_cl);
            let v = 4.5 * libm::exp(eta_v);
            GaussianObjectiveRequest {
                simulation: simulation.clone(),
                priors: vec![],
                observations: (0..9)
                    .map(|row| GaussianObservation {
                        row,
                        output: "cp".into(),
                        censoring: None,
                        value: quantity(6. / v * libm::exp(-cl * row as f64 * 0.25 / v), "mg/L"),
                        error: GaussianError {
                            model: GaussianErrorModel::Normal,
                            additive_sd: quantity(0.02, "mg/L"),
                            proportional_sd: 0.,
                        },
                    })
                    .collect(),
            }
        })
        .collect();
    request.fixed_effects[0].lower = quantity(0.2, "L/h");
    request.omega = vec![vec![0.04, 0.], vec![0., 0.04]];
    request.omega_diagonal_bounds = vec![[0.0025, 0.36], [0.0025, 0.36]];
    request.eta_bound = 2.;
    request.max_evaluations = 96_000;
    request.uncertainty = true;
    let before = serde_json::to_value(&request).unwrap();
    let fitted = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), before);
    assert_eq!(fitted.status, FitStatus::Converged, "{fitted:?}");
    assert!(fitted.evaluations <= request.max_evaluations);
    let covariance = fitted.uncertainty.as_ref().unwrap();
    assert_eq!(covariance.standard_errors.len(), 5);
    assert!(covariance
        .standard_errors
        .iter()
        .all(|value| value.is_finite() && *value > 0.));
    assert!((fitted.fixed_effects["cl"].value - 0.6).abs() < 0.08);
    assert!((fitted.fixed_effects["v"].value - 4.5).abs() < 0.45);
    for row in fitted.fitted_observations.unwrap() {
        let cl =
            fitted.fixed_effects["cl"].value * libm::exp(fitted.subjects[row.subject_index].eta[0]);
        let v =
            fitted.fixed_effects["v"].value * libm::exp(fitted.subjects[row.subject_index].eta[1]);
        let exact = 6. / v * libm::exp(-cl * row.time.value / v);
        assert!((row.ipred.value - exact).abs() < 2e-6, "{row:?}");
    }
}

#[test]
fn saem_one_effect_estimates_additive_error_and_preserves_fitted_rows() {
    let (compiled, mut request) = fixture();
    for (subject_index, subject) in request.subjects.iter_mut().enumerate() {
        for (row, observation) in subject.observations.iter_mut().enumerate() {
            observation.value.value += if (subject_index + row) % 2 == 0 {
                0.015
            } else {
                -0.015
            };
            observation.error.additive_sd = quantity(0.03, "mg/L");
        }
    }
    request.additive_error_fit = Some(PopulationAdditiveErrorFit {
        output: "cp".into(),
        initial: quantity(0.03, "mg/L"),
        lower: quantity(0.005, "mg/L"),
        upper: quantity(0.05, "mg/L"),
    });
    request.max_iterations = 1200;
    request.max_evaluations = 100_000;
    for seed in [20260929, 17029, 32321] {
        request.seed = seed;
        let fitted = compiled.fit_population_saem(&request).unwrap();
        assert_eq!(
            fitted.status,
            FitStatus::Converged,
            "seed={seed}, {fitted:?}"
        );
        assert_eq!(
            fitted.objective_kind,
            PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue
        );
        let error = fitted.estimated_residual_error.as_ref().unwrap();
        assert_eq!(error.output, "cp");
        assert_eq!(error.additive_sd.unit, "mg/L");
        // Synthetic finite-cohort residual recovery, with explicit injected noise.
        assert!(
            error.additive_sd.value > 0.01 && error.additive_sd.value < 0.025,
            "seed={seed}, {fitted:?}"
        );
        assert!((fitted.fixed_effects["cl"].value - 0.6).abs() < 0.02);
        assert!((fitted.omega[0][0] - 0.09).abs() < 0.025);
        assert_eq!(
            fitted.fitted_observations.as_ref().unwrap().len(),
            request
                .subjects
                .iter()
                .map(|s| s.observations.len())
                .sum::<usize>()
        );
    }
}

#[test]
fn saem_estimates_additive_error_with_m3_censored_rows() {
    let (compiled, mut request) = fixture();
    let mut censored = 0;
    for (subject_index, subject) in request.subjects.iter_mut().enumerate() {
        for (row, observation) in subject.observations.iter_mut().enumerate() {
            observation.value.value += if (subject_index + row) % 2 == 0 {
                0.015
            } else {
                -0.015
            };
            if observation.value.value < 1.0 {
                observation.value.value = 1.0;
                observation.censoring = Some(GaussianCensoring::Left);
                censored += 1;
            }
            observation.error.additive_sd = quantity(0.03, "mg/L");
        }
    }
    assert!(censored > 0);
    request.additive_error_fit = Some(PopulationAdditiveErrorFit {
        output: "cp".into(),
        initial: quantity(0.03, "mg/L"),
        lower: quantity(0.005, "mg/L"),
        upper: quantity(0.05, "mg/L"),
    });
    request.max_iterations = 1200;
    request.max_evaluations = 100_000;
    let fitted = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(fitted.status, FitStatus::Converged, "{fitted:?}");
    assert!((fitted.fixed_effects["cl"].value - 0.6).abs() < 0.03);
    let sd = fitted.estimated_residual_error.unwrap().additive_sd.value;
    assert!(sd > 0.008 && sd < 0.035, "sd={sd}");
}

#[test]
fn saem_burn_in_is_explicit_and_bounded() {
    let (compiled, mut request) = fixture();
    request.burn_in_iterations = Some(200);
    let fitted = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(fitted.saem_diagnostics.unwrap().burn_in_iterations, 200);
    request.burn_in_iterations = Some(request.max_iterations);
    assert_eq!(
        compiled.fit_population_saem(&request).unwrap_err().code,
        qspquest_core::ErrorCode::InvalidInput
    );
}

#[test]
fn saem_two_effects_free_error_preserves_evaluation_budget() {
    let (compiled, mut request) = two_effect_dosed_pk_fixture();
    for subject in &mut request.subjects {
        for observation in &mut subject.observations {
            observation.error.additive_sd = quantity(0.04, "mg/L");
        }
    }
    request.additive_error_fit = Some(PopulationAdditiveErrorFit {
        output: "cp".into(),
        initial: quantity(0.04, "mg/L"),
        lower: quantity(0.005, "mg/L"),
        upper: quantity(0.1, "mg/L"),
    });
    request.max_evaluations = request.subjects.len() * 4;
    let fit = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(fit.status, FitStatus::EvaluationLimit);
    assert!(fit.evaluations <= request.max_evaluations);
    assert_eq!(fit.fitted_observations.as_ref().unwrap().len(), 30);
}

#[test]
fn saem_two_effects_estimates_additive_error_with_m3_censoring() {
    let (compiled, mut request) = two_effect_dosed_pk_fixture();
    let mut censored = 0;
    for (subject_index, subject) in request.subjects.iter_mut().enumerate() {
        for (row, observation) in subject.observations.iter_mut().enumerate() {
            observation.value.value += if (subject_index + row) % 2 == 0 {
                0.02
            } else {
                -0.02
            };
            if observation.value.value < 1.0 {
                observation.value.value = 1.0;
                observation.censoring = Some(GaussianCensoring::Left);
                censored += 1;
            }
            observation.error.additive_sd = quantity(0.04, "mg/L");
        }
    }
    assert!(censored > 0 && censored < 30);
    request.additive_error_fit = Some(PopulationAdditiveErrorFit {
        output: "cp".into(),
        initial: quantity(0.04, "mg/L"),
        lower: quantity(0.005, "mg/L"),
        upper: quantity(0.1, "mg/L"),
    });
    request.max_iterations = 1600;
    request.burn_in_iterations = Some(400);
    request.max_evaluations = 200_000;
    let fitted = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(fitted.status, FitStatus::Converged, "{fitted:?}");
    assert!((fitted.fixed_effects["cl"].value - 0.6).abs() < 0.08);
    assert!((fitted.fixed_effects["v"].value - 4.5).abs() < 0.4);
    let sd = fitted.estimated_residual_error.unwrap().additive_sd.value;
    assert!(sd > 0.01 && sd < 0.08, "sd={sd}");
    assert!(fitted.evaluations <= request.max_evaluations);
    assert!(fitted.iterations <= request.max_iterations);
}

#[test]
fn saem_two_effects_matches_exact_gaussian_marginal_at_fitted_parameters() {
    let (compiled, request) = two_effect_fixture();
    let fitted = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(fitted.status, FitStatus::Converged, "{fitted:?}");
    assert_eq!(
        fitted.objective_kind,
        PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue,
        "{fitted:?}"
    );
    assert!((fitted.fixed_effects["cl"].value - 0.9).abs() < 0.12);
    assert!((fitted.fixed_effects["v"].value - 1.1).abs() < 0.12);
    assert_eq!(fitted.subjects.len(), request.subjects.len());
    assert_eq!(fitted.fitted_observations.as_ref().unwrap().len(), 16);
    let covariance = [
        [fitted.omega[0][0] + 0.06_f64.powi(2), fitted.omega[0][1]],
        [fitted.omega[1][0], fitted.omega[1][1] + 0.06_f64.powi(2)],
    ];
    let determinant = covariance[0][0] * covariance[1][1] - covariance[0][1].powi(2);
    assert!(determinant > 0.);
    let mut exact = 0.;
    for subject in &request.subjects {
        let residual = [
            subject.observations[0].value.value - fitted.fixed_effects["cl"].value.ln(),
            subject.observations[1].value.value - fitted.fixed_effects["v"].value.ln(),
        ];
        exact += (covariance[1][1] * residual[0].powi(2) + covariance[0][0] * residual[1].powi(2)
            - 2. * covariance[0][1] * residual[0] * residual[1])
            / determinant
            + determinant.ln();
    }
    assert!(
        (fitted.objective - exact).abs() < 0.02,
        "Laplace OFV {} vs exact {}",
        fitted.objective,
        exact
    );
    let grid = compiled
        .run(&request.subjects[0].simulation)
        .unwrap()
        .result;
    for row in fitted.fitted_observations.unwrap() {
        let axis = row.observation_index;
        let theta = if axis == 0 {
            fitted.fixed_effects["cl"].value
        } else {
            fitted.fixed_effects["v"].value
        };
        assert!((row.pred.value - theta.ln()).abs() < 1e-10);
        assert!(
            (row.ipred.value - theta.ln() - fitted.subjects[row.subject_index].eta[axis]).abs()
                < 1e-10
        );
        assert_eq!(row.side, grid.sides[row.row]);
        assert_eq!(row.time.value, grid.times[row.row]);
    }
}

#[test]
fn tiny_positive_residual_variance_has_continuous_likelihood_and_score() {
    let (compiled, request) = two_effect_fixture();
    let mut subject = request.subjects[0].clone();
    subject.observations.truncate(1);
    subject.observations[0].value.value = 0.;
    subject.observations[0].error.additive_sd.value = 1e-5;
    let narrow = compiled.gaussian_objective(&subject).unwrap();
    assert!(narrow.objective.is_finite());
    subject.observations[0].error.additive_sd.value = 2e-5;
    let wider = compiled.gaussian_objective(&subject).unwrap();
    assert!((wider.objective - narrow.objective - 2_f64.ln()).abs() < 1e-9);

    subject.observations[0].error.additive_sd.value = 1e-5;
    subject.observations[0].value.value = 1e-6;
    let center = compiled.gaussian_objective(&subject).unwrap();
    let step = 1e-7;
    subject
        .simulation
        .run
        .parameters
        .get_mut("cl")
        .unwrap()
        .value = 1. + step;
    let plus = compiled.gaussian_objective(&subject).unwrap().objective;
    subject
        .simulation
        .run
        .parameters
        .get_mut("cl")
        .unwrap()
        .value = 1. - step;
    let minus = compiled.gaussian_objective(&subject).unwrap().objective;
    let numeric = (plus - minus) / (2. * step);
    assert!(
        (center.gradient[0].value - numeric).abs() < 0.01,
        "analytic {} vs finite difference {numeric}",
        center.gradient[0].value
    );
}

#[test]
fn saem_recovers_lognormal_population_on_synthetic_pk() {
    let (compiled, request) = fixture();
    let result = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    assert_eq!(
        result.objective_kind,
        PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue
    );
    assert_eq!(result.subjects.len(), 8);
    assert!(result.evaluations <= request.max_evaluations);
    assert!(result.objective.is_finite());
    let rows = result.fitted_observations.as_ref().unwrap();
    assert_eq!(
        rows.len(),
        request
            .subjects
            .iter()
            .map(|s| s.observations.len())
            .sum::<usize>()
    );
    for row in rows {
        assert_eq!(row.value.unit, row.pred.unit);
        assert_eq!(row.pred.unit, row.ipred.unit);
        assert_eq!(
            row.output,
            request.subjects[row.subject_index].observations[row.observation_index].output
        );
        assert_eq!(
            row.row,
            request.subjects[row.subject_index].observations[row.observation_index].row
        );
    }
    let theta = result.fixed_effects["cl"].value;
    let omega = result.omega[0][0];
    assert!((theta - 0.6).abs() < 0.12, "theta={theta}");
    assert!((omega - 0.083).abs() < 0.06, "omega={omega}");
    let diagnostics = result.saem_diagnostics.unwrap();
    assert_eq!(diagnostics.burn_in_iterations, request.max_iterations / 3);
    assert!(diagnostics.accepted_proposals <= diagnostics.evaluated_proposals);
    assert!(diagnostics.evaluated_proposals <= result.evaluations - 4 * request.subjects.len());
    if result.iterations > 0 {
        let last = result.iterations - 1;
        let expected_gain = if last < diagnostics.burn_in_iterations {
            1.0
        } else {
            1.0 / (last - diagnostics.burn_in_iterations + 1) as f64
        };
        assert!((diagnostics.final_step_size.unwrap() - expected_gain).abs() < 1e-12);
        assert!(diagnostics.final_relative_change.unwrap().is_finite());
        assert!(diagnostics.recent_window <= 20);
        if diagnostics.recent_window > 0 {
            assert!(diagnostics
                .recent_relative_change_median
                .unwrap()
                .is_finite());
        }
    }
}

#[test]
fn saem_recovers_population_with_m3_left_censoring() {
    let (compiled, mut request) = fixture();
    let mut censored = 0_usize;
    for subject in &mut request.subjects {
        for observation in &mut subject.observations {
            if observation.value.value < 1.0 {
                observation.value.value = 1.0;
                observation.censoring = Some(GaussianCensoring::Left);
                censored += 1;
            }
        }
    }
    assert!(censored > 0 && censored < request.subjects.iter().map(|s| s.observations.len()).sum());
    let result = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    assert_eq!(
        result.objective_kind,
        PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue
    );
    assert!(
        (result.fixed_effects["cl"].value - 0.6).abs() < 0.14,
        "{result:?}"
    );
    assert!((result.omega[0][0] - 0.083).abs() < 0.065, "{result:?}");
    assert_eq!(
        result.fitted_observations.unwrap().len(),
        request
            .subjects
            .iter()
            .map(|s| s.observations.len())
            .sum::<usize>()
    );
}

#[test]
fn saem_recovers_population_with_lognormal_residuals() {
    let (compiled, mut request) = fixture();
    for subject in &mut request.subjects {
        for observation in &mut subject.observations {
            observation.error.model = GaussianErrorModel::Lognormal;
            observation.error.additive_sd.value = 0.;
            observation.error.proportional_sd = 0.04;
        }
    }
    let result = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    assert_eq!(
        result.objective_kind,
        PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue
    );
    assert!(
        (result.fixed_effects["cl"].value - 0.6).abs() < 0.12,
        "{result:?}"
    );
    assert!((result.omega[0][0] - 0.083).abs() < 0.06, "{result:?}");
}

#[test]
fn saem_one_effect_reports_marginal_curvature_uncertainty() {
    let (compiled, mut request) = fixture();
    request.uncertainty = true;
    let result = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged);
    let uncertainty = result.uncertainty.unwrap();
    assert_eq!(
        uncertainty.method,
        PopulationUncertaintyMethod::SaemMarginalObservedHessian
    );
    assert_eq!(
        uncertainty
            .parameters
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["cl", "omega:cl"]
    );
    assert!(uncertainty
        .standard_errors
        .iter()
        .all(|value| value.is_finite() && *value > 0.));
    assert_eq!(uncertainty.covariance.len(), 2);
    assert!((uncertainty.covariance[0][1] - uncertainty.covariance[1][0]).abs() < 1e-12);
}

#[test]
fn saem_two_effects_reports_observed_uncertainty() {
    let (compiled, mut request) = two_effect_fixture();
    let second_effects = [-0.03, 0.24, -0.19, 0.11, 0.18, -0.11, 0.03, -0.18];
    for (subject, eta) in request.subjects.iter_mut().zip(second_effects) {
        subject.observations[1].value.value = 1.1_f64.ln() + eta;
    }
    request.uncertainty = true;
    request.max_iterations = 1600;
    request.burn_in_iterations = Some(400);
    request.gradient_tolerance = 0.005;
    request.max_evaluations = 200_000;
    let result = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged);
    let uncertainty = result.uncertainty.unwrap();
    assert_eq!(uncertainty.parameters.len(), 5);
    assert_eq!(uncertainty.parameters[0].name, "cl");
    assert_eq!(uncertainty.parameters[1].name, "v");
    assert_eq!(uncertainty.parameters[4].name, "omega:cl:v");
    assert!(uncertainty
        .standard_errors
        .iter()
        .all(|x| x.is_finite() && *x > 0.));
    for axis in 0..2 {
        let name = if axis == 0 { "cl" } else { "v" };
        let exact = result.fixed_effects[name].value
            * libm::sqrt((result.omega[axis][axis] + 0.06_f64.powi(2)) / 8.);
        assert!(
            (uncertainty.standard_errors[axis] / exact - 1.).abs() < 0.25,
            "{name} SE {} vs exact {exact}",
            uncertainty.standard_errors[axis]
        );
    }
    for row in 0..5 {
        for col in 0..5 {
            assert!(
                (uncertainty.covariance[row][col] - uncertainty.covariance[col][row]).abs() < 1e-9
            );
        }
    }
}

#[test]
fn saem_one_effect_recovers_exponential_subject_covariate() {
    let (compiled, mut request) = fixture();
    let values = [-1., 1., 0.5, -0.5, -0.5, 0.5, 1., -1.];
    let eta: [f64; 8] = [-0.45, -0.32, -0.21, -0.12, 0.12, 0.21, 0.32, 0.45];
    for (index, subject) in request.subjects.iter_mut().enumerate() {
        let cl = 0.6 * libm::exp(0.4 * values[index] + eta[index]);
        let times = compiled.run(&subject.simulation).unwrap().result.times;
        for observation in &mut subject.observations {
            let time = times[observation.row];
            observation.value.value = 6. / 4.5 * libm::exp(-cl * time / 4.5);
        }
    }
    request.covariate_effect = Some(PopulationCovariateEffect {
        parameter: "cl".into(),
        covariate: "centered_weight".into(),
        subject_values: values.to_vec(),
        reference: 0.,
        initial: 0.,
        bounds: [-1., 1.],
        additional: Vec::new(),
    });
    for seed in [20260929, 20260930, 17029] {
        request.seed = seed;
        let result = compiled.fit_population_saem(&request).unwrap();
        assert_eq!(
            result.status,
            FitStatus::Converged,
            "seed={seed}: {result:?}"
        );
        assert_eq!(
            result.objective_kind,
            PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue
        );
        assert!((result.fixed_effects["cl"].value - 0.6).abs() < 0.1);
        let effect = result.covariate_effect.unwrap();
        assert_eq!(effect.covariate, "centered_weight");
        assert!(
            (effect.coefficient - 0.4).abs() < 0.13,
            "seed={seed}, beta={}",
            effect.coefficient
        );
    }
}

#[test]
fn saem_recovers_two_independent_subject_covariate_effects() {
    let (compiled, mut request) = fixture();
    let first = [-1., -1., -1., -1., 1., 1., 1., 1.];
    let second = [-1., -1., 1., 1., -1., -1., 1., 1.];
    let eta = [-0.18, 0.18, -0.12, 0.12, 0.12, -0.12, 0.18, -0.18];
    for (index, subject) in request.subjects.iter_mut().enumerate() {
        let cl = 0.6 * libm::exp(0.3 * first[index] - 0.25 * second[index] + eta[index]);
        let times = compiled.run(&subject.simulation).unwrap().result.times;
        for observation in &mut subject.observations {
            let time = times[observation.row];
            observation.value.value = 6. / 4.5 * libm::exp(-cl * time / 4.5);
        }
    }
    request.covariate_effect = Some(PopulationCovariateEffect {
        parameter: "cl".into(),
        covariate: "weight".into(),
        subject_values: first.to_vec(),
        reference: 0.,
        initial: 0.,
        bounds: [-1., 1.],
        additional: vec![PopulationCovariateEffect {
            parameter: "cl".into(),
            covariate: "renal".into(),
            subject_values: second.to_vec(),
            reference: 0.,
            initial: 0.,
            bounds: [-1., 1.],
            additional: Vec::new(),
        }],
    });
    request.max_iterations = 1200;
    request.max_evaluations = 60_000;
    let result = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    let effect = result.covariate_effect.unwrap();
    assert!((effect.coefficient - 0.3).abs() < 0.1, "{effect:?}");
    assert_eq!(effect.additional.len(), 1);
    assert!(
        (effect.additional[0].coefficient + 0.25).abs() < 0.1,
        "{effect:?}"
    );
}

#[test]
fn saem_fits_random_clearance_with_model_authored_time_varying_covariate() {
    let mut model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    model.model_id = "saem_step_covariate".into();
    model.parameters.push(Parameter {
        name: "beta".into(),
        default: quantity(0.5, "1"),
        bounds: None,
        transform: Transform::Identity,
        fixed: true,
        description: String::new(),
    });
    model.covariates.push(Covariate {
        name: "weight_ratio".into(),
        unit: "1".into(),
        interpolation: Interpolation::Step,
        default: Some(quantity(1., "1")),
    });
    let factor = Expr::call(
        Function::Exp,
        vec![Expr::binary(
            Binary::Multiply,
            Expr::symbol("beta"),
            Expr::binary(
                Binary::Subtract,
                Expr::symbol("weight_ratio"),
                Expr::number(1.),
            ),
        )],
    );
    model.states[0].rhs = Expr::binary(
        Binary::Subtract,
        Expr::symbol("input_central"),
        Expr::binary(
            Binary::Multiply,
            Expr::binary(
                Binary::Multiply,
                Expr::binary(Binary::Divide, Expr::symbol("cl"), Expr::symbol("v")),
                factor,
            ),
            Expr::symbol("central"),
        ),
    );
    let compiled = CompiledSensitivityDocument::compile(&model, &["cl".into()]).unwrap();
    let (_, mut request) = fixture();
    let etas = [-0.2, -0.15, -0.1, -0.05, 0.05, 0.1, 0.15, 0.2];
    for (index, subject) in request.subjects.iter_mut().enumerate() {
        let change_time = if index % 2 == 0 { 0.75 } else { 1.25 };
        let ratio = if index % 3 == 0 { 1.4 } else { 0.8 };
        subject
            .simulation
            .run
            .regimen
            .covariates
            .insert("weight_ratio".into(), quantity(1., "1"));
        subject
            .simulation
            .run
            .regimen
            .covariate_changes
            .push(CovariateChange {
                name: "weight_ratio".into(),
                time: quantity(change_time, "h"),
                value: quantity(ratio, "1"),
            });
        let mut generating = subject.simulation.clone();
        generating
            .run
            .parameters
            .insert("cl".into(), quantity(0.6 * libm::exp(etas[index]), "L/h"));
        let trajectory = compiled.run(&generating).unwrap().result;
        for observation in &mut subject.observations {
            observation.value.value = trajectory.outputs[0].values[observation.row];
        }
    }
    let result = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    assert!((result.fixed_effects["cl"].value - 0.6).abs() < 0.1);
    assert!(result.omega[0][0] > 0.01);
}

#[test]
fn saem_rejects_unidentifiable_covariate_and_two_effect_scope() {
    let (compiled, mut request) = fixture();
    request.covariate_effect = Some(PopulationCovariateEffect {
        parameter: "cl".into(),
        covariate: "constant".into(),
        subject_values: vec![1.; request.subjects.len()],
        reference: 0.,
        initial: 0.,
        bounds: [-1., 1.],
        additional: Vec::new(),
    });
    let error = compiled.fit_population_saem(&request).unwrap_err();
    assert!(error.message.contains("unidentifiable"), "{error:?}");

    let (compiled, mut two_effect) = two_effect_fixture();
    two_effect.covariate_effect = request.covariate_effect;
    let error = compiled.fit_population_saem(&two_effect).unwrap_err();
    assert!(error.message.contains("one random effect"), "{error:?}");
}

#[test]
fn saem_converges_on_twenty_four_subject_pk_cohort() {
    let (compiled, mut request) = fixture();
    let simulation = request.subjects[0].simulation.clone();
    let times = compiled.run(&simulation).unwrap().result.times;
    request.subjects = (0..24)
        .map(|index| {
            let eta = -0.45 + 0.9 * index as f64 / 23.;
            GaussianObjectiveRequest {
                simulation: simulation.clone(),
                observations: times
                    .iter()
                    .enumerate()
                    .filter(|(row, _)| *row > 0)
                    .map(|(row, time)| GaussianObservation {
                        censoring: None,
                        row,
                        output: "cp".into(),
                        value: quantity(6.0 / 4.5 * (-0.6 * eta.exp() * time / 4.5).exp(), "mg/L"),
                        error: GaussianError {
                            model: GaussianErrorModel::Normal,
                            additive_sd: quantity(0.02, "mg/L"),
                            proportional_sd: 0.,
                        },
                    })
                    .collect(),
                priors: Vec::new(),
            }
        })
        .collect();
    request.max_iterations = 600;
    request.max_evaluations = 60_000;
    let result = compiled.fit_population_saem(&request).unwrap();
    assert_eq!(
        result.status,
        FitStatus::Converged,
        "{:?}",
        result.saem_diagnostics
    );
    assert_eq!(
        result.objective_kind,
        PopulationObjectiveKind::SaemMarginalObjectiveFunctionValue
    );
    assert!((result.fixed_effects["cl"].value - 0.6).abs() < 0.05);
    assert!((result.omega[0][0] - 0.073).abs() < 0.035);
    assert_eq!(result.fitted_observations.unwrap().len(), 24 * 4);
}

#[test]
fn saem_two_effects_accepts_scaled_evaluation_budget() {
    let (compiled, mut request) = two_effect_dosed_pk_fixture();
    request.max_iterations = 8;
    request.max_evaluations = 160_000;
    let result = compiled.fit_population_saem(&request).unwrap();
    assert!(result.evaluations <= request.max_evaluations);

    request.max_evaluations = 200_001;
    assert_eq!(
        compiled.fit_population_saem(&request).unwrap_err().code,
        qspquest_core::ErrorCode::InvalidInput
    );
}

#[test]
fn saem_short_burn_in_does_not_activate_adaptive_sweeps() {
    let (compiled, mut request) = two_effect_dosed_pk_fixture();
    request.max_iterations = 8;
    request.burn_in_iterations = Some(4);
    request.max_evaluations = 2_000;
    let fitted = compiled.fit_population_saem(&request).unwrap();
    assert!(fitted.evaluations <= request.max_evaluations);
    assert!(
        fitted.saem_diagnostics.unwrap().evaluated_proposals
            <= 3 * request.subjects.len() * request.max_iterations
    );
}

#[test]
fn saem_rejects_unsupported_population_shape() {
    let (compiled, mut request) = fixture();
    request.random_effects.push(PopulationRandomEffect {
        parameter: "v".into(),
    });
    assert_eq!(
        compiled.fit_population_saem(&request).unwrap_err().code,
        qspquest_core::ErrorCode::Unsupported
    );
}

#[test]
fn saem_preserves_complete_results_at_evaluation_limit() {
    let (compiled, mut request) = fixture();
    request.max_evaluations = request.subjects.len() * 4;
    let result = compiled.fit_population_saem(&request).unwrap();
    assert!(matches!(result.status, FitStatus::EvaluationLimit));
    assert_eq!(result.evaluations, request.max_evaluations);
    assert_eq!(result.subjects.len(), request.subjects.len());
    assert_eq!(
        result.objective_kind,
        PopulationObjectiveKind::SaemCompleteDataSurrogate
    );
    assert_eq!(
        result.fitted_observations.as_ref().unwrap().len(),
        request
            .subjects
            .iter()
            .map(|s| s.observations.len())
            .sum::<usize>()
    );
    assert!(result
        .subjects
        .iter()
        .all(|s| s.negative_log_likelihood.is_finite()));
    let diagnostics = result.saem_diagnostics.unwrap();
    assert_eq!(diagnostics.evaluated_proposals, 0);
    assert_eq!(diagnostics.accepted_proposals, 0);
    assert_eq!(diagnostics.final_step_size, None);
    assert_eq!(diagnostics.final_relative_change, None);
    assert_eq!(diagnostics.recent_relative_change_median, None);
    assert_eq!(diagnostics.recent_window, 0);
}

#[test]
fn saem_diagnostics_preserve_exact_gaussian_random_intercept_oracle() {
    // log(theta * exp(eta)) = log(theta) + eta, so the marginal Gaussian
    // MLEs are theta=exp(mean(y)) and Omega=mean((y-mean)^2)-sigma^2.
    let mut model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    model.parameters[0].default = quantity(1., "1");
    model.parameters[1].default = quantity(1., "h");
    model.states[0].unit = "1".into();
    model.states[0].initial = qspquest_core::model::Initial::Quantity {
        quantity: quantity(0., "1"),
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
        std::collections::BTreeMap::from([("central".into(), quantity(1e-12, "1"))]),
    );
    simulation
        .run
        .parameters
        .insert("cl".into(), quantity(1., "1"));
    simulation
        .run
        .parameters
        .insert("v".into(), quantity(1., "h"));
    simulation.run.regimen.administrations.clear();
    let values = [-0.3, -0.2, -0.1, 0., 0., 0.1, 0.2, 0.3];
    let subjects = values
        .into_iter()
        .map(|value| GaussianObjectiveRequest {
            simulation: simulation.clone(),
            observations: vec![GaussianObservation {
                censoring: None,
                row: 0,
                output: "cp".into(),
                value: quantity(value, "1"),
                error: GaussianError {
                    model: GaussianErrorModel::Normal,
                    additive_sd: quantity(0.05, "1"),
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
            lower: quantity(0.7, "1"),
            upper: quantity(1.3, "1"),
        }],
        random_effects: vec![PopulationRandomEffect {
            parameter: "cl".into(),
        }],
        omega: vec![vec![0.03]],
        omega_diagonal_bounds: vec![[0.005, 0.08]],
        eta_bound: 1.5,
        max_evaluations: 30_000,
        max_iterations: 800,
        gradient_tolerance: 1e-5,
        seed: 17029,
        burn_in_iterations: None,
        uncertainty: true,
        additive_error_fit: None,
        covariate_effect: None,
    };
    let fitted = compiled.fit_population_saem(&request).unwrap();
    let exact_omega = values.iter().map(|value| value * value).sum::<f64>() / 8. - 0.05_f64.powi(2);
    assert!(
        (fitted.fixed_effects["cl"].value - 1.).abs() < 0.05,
        "{fitted:?}"
    );
    assert!(
        (fitted.omega[0][0] - exact_omega).abs() < 0.02,
        "{fitted:?}"
    );
    assert_eq!(
        fitted.status,
        FitStatus::Converged,
        "{:?}",
        fitted.saem_diagnostics
    );
    let diagnostics = fitted.saem_diagnostics.unwrap();
    assert_eq!(diagnostics.burn_in_iterations, 800 / 3);
    assert_eq!(diagnostics.recent_window, 20);
    assert!(diagnostics.recent_relative_change_median.unwrap() > request.gradient_tolerance);
    assert!(diagnostics.accepted_proposals > 0);
    assert!(diagnostics.accepted_proposals < diagnostics.evaluated_proposals);
    let standard_errors = fitted
        .uncertainty
        .as_ref()
        .unwrap()
        .standard_errors
        .as_slice();
    let total_variance = fitted.omega[0][0] + 0.05_f64.powi(2);
    let exact_theta_se = fitted.fixed_effects["cl"].value * libm::sqrt(total_variance / 8.);
    let exact_omega_se = libm::sqrt(2.) * total_variance / libm::sqrt(8.);
    assert!(
        (standard_errors[0] / exact_theta_se - 1.).abs() < 0.2,
        "theta SE {:?}, oracle {exact_theta_se}",
        standard_errors
    );
    assert!(
        (standard_errors[1] / exact_omega_se - 1.).abs() < 0.2,
        "omega SE {:?}, oracle {exact_omega_se}",
        standard_errors
    );
}

#[test]
fn rodas5p_population_dispatch_preserves_objective_and_solver_identity() {
    let (compiled, mut request) = fixture();
    // Bounded replay checks both primal likelihood and derivative-bearing population paths.
    request.max_iterations = 20;
    request.burn_in_iterations = Some(4);
    request.uncertainty = false;
    let bdf = compiled.fit_population_saem(&request).unwrap();
    for subject in &mut request.subjects {
        subject.simulation.run.solver = qspquest_core::run::Solver::DiffsolRodas5p;
    }
    let rodas = compiled.fit_population_saem(&request).unwrap();
    assert_ne!(bdf.fit_request_hash, rodas.fit_request_hash);
    assert!(
        (bdf.objective - rodas.objective).abs() < 1e-4,
        "{} vs {}",
        bdf.objective,
        rodas.objective
    );
}

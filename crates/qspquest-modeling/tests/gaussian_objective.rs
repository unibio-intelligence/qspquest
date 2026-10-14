use qspquest_core::{fit::*, model::ModelDocument, model::Quantity, run::SensitivityRequest};
use qspquest_modeling::document::sensitivity::CompiledSensitivityDocument;
use std::sync::Arc;
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
fn fixture(
    additive: f64,
    proportional: f64,
    prior: bool,
) -> (
    Arc<CompiledSensitivityDocument>,
    GaussianObjectiveRequest,
    Vec<f64>,
) {
    let model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let simulation: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    let c = CompiledSensitivityDocument::compile(&model, &simulation.with_respect_to).unwrap();
    let result = c.run(&simulation).unwrap();
    let observations = result
        .result
        .times
        .iter()
        .enumerate()
        .filter(|(i, _)| *i > 0)
        .map(|(row, t)| GaussianObservation {
            censoring: None,
            row,
            output: "cp".into(),
            value: q(0.8 * 2. * (-0.1 * t).exp() + 0.1, "ug/mL"),
            error: GaussianError {
                model: GaussianErrorModel::Normal,
                additive_sd: q(additive, "mg/L"),
                proportional_sd: proportional,
            },
        })
        .collect();
    let priors = if prior {
        vec![
            GaussianPrior {
                parameter: "cl".into(),
                mean: q(400., "mL/h"),
                sd: q(100., "mL/h"),
            },
            GaussianPrior {
                parameter: "v".into(),
                mean: q(4., "L"),
                sd: q(1., "L"),
            },
        ]
    } else {
        vec![]
    };
    (
        c,
        GaussianObjectiveRequest {
            simulation,
            observations,
            priors,
        },
        result.result.times,
    )
}
fn oracle(request: &GaussianObjectiveRequest, times: &[f64], cl: f64, v: f64) -> f64 {
    let mut result = 0.;
    for o in &request.observations {
        let prediction = 6. / v * (-cl * times[o.row] / v).exp();
        let variance =
            o.error.additive_sd.value.powi(2) + (o.error.proportional_sd * prediction).powi(2);
        result += 0.5
            * ((o.value.value - prediction).powi(2) / variance
                + (2. * std::f64::consts::PI * variance).ln());
    }
    if !request.priors.is_empty() {
        for (value, mean, sd) in [(cl, 0.4, 0.1), (v, 4., 1.)] {
            result += 0.5 * ((value - mean) / sd).powi(2)
                + (sd * (2. * std::f64::consts::PI).sqrt()).ln();
        }
    }
    result
}
#[test]
fn additive_proportional_combined_and_prior_gradients_match_independent_likelihood() {
    for (additive, proportional, prior) in [
        (0.2, 0., false),
        (0., 0.2, false),
        (0.1, 0.2, false),
        (0.1, 0.2, true),
    ] {
        let (c, request, times) = fixture(additive, proportional, prior);
        let result = c.gaussian_objective(&request).unwrap();
        assert!((result.objective - oracle(&request, &times, 0.3, 3.)).abs() < 1e-7);
        assert_eq!(result.observations, request.observations.len());
        for p in 0..2 {
            let h = 1e-5;
            let (mut hc, mut hv, mut lc, mut lv) = (0.3, 3., 0.3, 3.);
            if p == 0 {
                hc += h;
                lc -= h;
            } else {
                hv += h;
                lv -= h;
            }
            let expected =
                (oracle(&request, &times, hc, hv) - oracle(&request, &times, lc, lv)) / (2. * h);
            assert!(
                (result.gradient[p].value - expected).abs() < 1e-5,
                "{additive} {proportional} prior {prior} p {p}: {} vs {expected}",
                result.gradient[p].value
            );
        }
        assert_eq!(result.gradient[0].unit, "1/(L/h)");
        assert_eq!(result.gradient[1].unit, "1/(L)");
        let mut duplicate = request.clone();
        duplicate.observations.extend(request.observations.clone());
        duplicate.priors.clear();
        let repeated = c.gaussian_objective(&duplicate).unwrap();
        assert!((repeated.objective - 2. * result.negative_log_likelihood).abs() < 1e-10);
    }
}
#[test]
fn invalid_observation_prior_and_zero_variance_fail_and_recover() {
    let (c, request, _) = fixture(0.1, 0.2, true);
    let baseline = c.gaussian_objective(&request).unwrap();
    for mode in 0..7 {
        let mut bad = request.clone();
        match mode {
            0 => bad.observations[0].row = usize::MAX,
            1 => bad.observations[0].value.unit = "h".into(),
            2 => bad.observations[0].error.additive_sd.value = -1.,
            3 => bad.observations[0].output = "unknown".into(),
            4 => bad.priors.push(bad.priors[0].clone()),
            5 => bad.priors[0].sd.value = 0.,
            _ => {
                bad.observations[0].row = 0;
                bad.observations[0].error.additive_sd.value = 0.;
            }
        };
        assert!(c.gaussian_objective(&bad).is_err(), "mode {mode}");
    }
    let mut budget = request.clone();
    budget.simulation.run.budgets.output_values = budget.observations.len();
    assert_eq!(
        c.gaussian_objective(&budget).unwrap_err().code,
        qspquest_core::ErrorCode::OutputBudget
    );
    let mut changed = request.clone();
    changed.observations[0].value.value += 0.1;
    assert_ne!(
        c.gaussian_objective(&changed)
            .unwrap()
            .identity
            .run_request_hash,
        baseline.identity.run_request_hash
    );
    assert_eq!(
        c.gaussian_objective(&request).unwrap().objective,
        baseline.objective
    );
}

#[test]
fn tiny_positive_residual_sd_has_a_finite_smooth_objective_and_gradient() {
    let (c, request, times) = fixture(1e-6, 0., false);
    let result = c.gaussian_objective(&request).unwrap();
    assert!(result.objective.is_finite());
    assert!(result.gradient.iter().all(|value| value.value.is_finite()));
    assert!(
        (result.objective - oracle(&request, &times, 0.3, 3.)).abs()
            <= 1e-10 * result.objective.abs()
    );

    for parameter in 0..2 {
        let h = if parameter == 0 { 1e-6 } else { 1e-5 };
        let (mut high_cl, mut high_v, mut low_cl, mut low_v) = (0.3, 3., 0.3, 3.);
        if parameter == 0 {
            high_cl += h;
            low_cl -= h;
        } else {
            high_v += h;
            low_v -= h;
        }
        let finite_difference = (oracle(&request, &times, high_cl, high_v)
            - oracle(&request, &times, low_cl, low_v))
            / (2. * h);
        let gradient = result.gradient[parameter].value;
        assert!((gradient - finite_difference).abs() <= 1e-5 * finite_difference.abs());
    }
}

fn fit_request(
    c: &Arc<CompiledSensitivityDocument>,
    mut objective: GaussianObjectiveRequest,
) -> GaussianFitRequest {
    objective.simulation.run.regimen.rtol = 1e-12;
    let times = c.run(&objective.simulation).unwrap().result.times;
    for o in &mut objective.observations {
        o.value = q(6. / 4.5 * (-0.6 * times[o.row] / 4.5).exp(), "mg/L");
    }
    GaussianFitRequest {
        objective,
        bounds: vec![
            FitBound {
                parameter: "cl".into(),
                lower: q(50., "mL/h"),
                upper: q(1200., "mL/h"),
            },
            FitBound {
                parameter: "v".into(),
                lower: q(1., "L"),
                upper: q(8., "L"),
            },
        ],
        max_evaluations: 400,
        max_iterations: 100,
        gradient_tolerance: 1e-5,
    }
}
#[test]
fn bounded_fit_recovers_clearance_volume_and_reports_limits() {
    let (c, objective, _) = fixture(0.1, 0., false);
    let request = fit_request(&c, objective);
    let result = c.fit_gaussian(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    assert!((result.parameters["cl"].value - 0.6).abs() < 1e-5);
    assert!((result.parameters["v"].value - 4.5).abs() < 1e-5);
    let mut limited = request.clone();
    limited.max_evaluations = 1;
    let result = c.fit_gaussian(&limited).unwrap();
    assert_eq!(result.status, FitStatus::EvaluationLimit);
    assert_eq!(result.evaluations, 1);
    assert_ne!(
        result.fit_request_hash,
        c.fit_gaussian(&request).unwrap().fit_request_hash
    );
    let mut invalid = request.clone();
    invalid.bounds[0].upper = q(0.1, "h");
    assert!(c.fit_gaussian(&invalid).is_err());
    assert_eq!(
        c.fit_gaussian(&request).unwrap().status,
        FitStatus::Converged
    );
}
#[test]
fn bounded_fit_finds_boundary_optimum_and_normal_prior_map() {
    let (c, objective, _) = fixture(0.1, 0., false);
    let mut request = fit_request(&c, objective);
    // Strong volume prior shifts the MAP away from the noise-free MLE.
    request.objective.priors.push(GaussianPrior {
        parameter: "v".into(),
        mean: q(3., "L"),
        sd: q(0.1, "L"),
    });
    let map = c.fit_gaussian(&request).unwrap();
    assert_eq!(map.status, FitStatus::Converged, "{map:?}");
    assert!(map.parameters["v"].value > 3. && map.parameters["v"].value < 4.5);
    assert!(map.projected_gradient_norm < request.gradient_tolerance);
    request.objective.priors.clear();
    request.bounds[1].upper = q(4., "L");
    let boundary = c.fit_gaussian(&request).unwrap();
    assert_eq!(boundary.status, FitStatus::Converged, "{boundary:?}");
    assert_eq!(boundary.parameters["v"].value, 4.);
    assert!(boundary.objective.gradient[1].value < 0.);
}

#[test]
fn fit_recovers_volume_across_repeated_doses_and_reset() {
    use qspquest_core::{
        regimen::{ObservationSide, StateReset},
        ActiveInputs,
    };
    let (c, objective, _) = fixture(0.05, 0., false);
    let mut request = fit_request(&c, objective);
    let regimen = &mut request.objective.simulation.run.regimen;
    regimen.end = q(8., "h");
    regimen.samples = [0., 1., 2., 3., 4., 5., 6., 8.]
        .iter()
        .map(|t| q(*t, "h"))
        .collect();
    let dose = regimen.administrations[0].clone();
    for time in [2., 5.] {
        let mut dose = dose.clone();
        dose.time = q(time, "h");
        regimen.administrations.push(dose);
    }
    regimen.resets.push(StateReset {
        target: "central".into(),
        time: q(3., "h"),
        value: q(0., "mg"),
        order: 0,
        active_inputs: ActiveInputs::StopTarget,
    });
    let grid = c.run(&request.objective.simulation).unwrap().result;
    request.objective.observations = grid
        .times
        .iter()
        .zip(&grid.sides)
        .enumerate()
        .map(|(row, (t, side))| {
            let after_reset = *t > 3. || (*t == 3. && *side == ObservationSide::Post);
            let concentration = [0., 2., 5.]
                .iter()
                .filter(|d| {
                    (**d < *t || (**d == *t && *side == ObservationSide::Post))
                        && (!after_reset || **d > 3.)
                })
                .map(|d| 6. / 4.5 * (-0.6 * (*t - *d) / 4.5).exp())
                .sum();
            GaussianObservation {
                censoring: None,
                row,
                output: "cp".into(),
                value: q(concentration, "mg/L"),
                error: GaussianError {
                    model: GaussianErrorModel::Normal,
                    additive_sd: q(0.05, "mg/L"),
                    proportional_sd: 0.,
                },
            }
        })
        .collect();
    let fit = c.fit_gaussian(&request).unwrap();
    assert_eq!(fit.status, FitStatus::Converged, "{fit:?}");
    assert!((fit.parameters["v"].value - 4.5).abs() < 1e-5);
    assert!((fit.parameters["cl"].value - 0.6).abs() < 1e-5);
}

#[test]
fn pooled_fit_recovers_shared_parameters_and_counts_prior_once() {
    let (c, objective, _) = fixture(0.1, 0., false);
    let fit = fit_request(&c, objective);
    let mut second = fit.objective.clone();
    second.simulation.run.regimen.administrations[0].amount = q(12., "mg");
    for o in &mut second.observations {
        o.value.value *= 2.;
    }
    let mut request = PooledGaussianFitRequest {
        fit,
        additional_subjects: vec![second],
    };
    let result = c.fit_pooled_gaussian(&request).unwrap();
    assert_eq!(result.status, FitStatus::Converged, "{result:?}");
    assert!((result.parameters["cl"].value - 0.6).abs() < 1e-5);
    assert!((result.parameters["v"].value - 4.5).abs() < 1e-5);
    assert_eq!(result.objective.observations, 8);
    assert_eq!(
        result.objective.identity.algorithm,
        "bdf_forward_pooled_gaussian_objective"
    );
    request.fit.objective.priors.push(GaussianPrior {
        parameter: "v".into(),
        mean: q(3., "L"),
        sd: q(0.5, "L"),
    });
    request.fit.max_evaluations = 1;
    let pooled = c.fit_pooled_gaussian(&request).unwrap();
    let a = c.gaussian_objective(&request.fit.objective).unwrap();
    let b = c
        .gaussian_objective(&request.additional_subjects[0])
        .unwrap();
    assert!((pooled.objective.objective - a.objective - b.objective).abs() < 1e-12);
    assert_eq!(pooled.objective.negative_log_prior, a.negative_log_prior);
    for p in 0..2 {
        assert!(
            (pooled.objective.gradient[p].value - a.gradient[p].value - b.gradient[p].value).abs()
                < 1e-12
        );
    }
    let mut changed = request.clone();
    changed.additional_subjects[0].observations[0].value.value += 0.1;
    let changed = c.fit_pooled_gaussian(&changed).unwrap();
    assert_ne!(pooled.fit_request_hash, changed.fit_request_hash);
    assert_ne!(
        pooled.objective.identity.run_request_hash,
        changed.objective.identity.run_request_hash
    );
    let mut invalid = request.clone();
    invalid.additional_subjects[0].priors = request.fit.objective.priors.clone();
    assert!(c.fit_pooled_gaussian(&invalid).is_err());
    invalid = request.clone();
    invalid.additional_subjects[0]
        .simulation
        .run
        .parameters
        .insert("cl".into(), q(0.5, "L/h"));
    assert!(c.fit_pooled_gaussian(&invalid).is_err());
    invalid = request.clone();
    invalid.additional_subjects[0].observations[0].value.unit = "h".into();
    assert!(c.fit_pooled_gaussian(&invalid).is_err());
    assert_eq!(
        c.fit_pooled_gaussian(&request).unwrap().objective.objective,
        pooled.objective.objective
    );
    request.fit.max_evaluations = 400;
    let map = c.fit_pooled_gaussian(&request).unwrap();
    assert_eq!(map.status, FitStatus::Converged, "{map:?}");
    assert!(map.parameters["v"].value > 3. && map.parameters["v"].value < 4.5);
}

#[test]
fn lognormal_residual_and_censoring_match_independent_oracle_and_gradient() {
    let (compiled, mut request, times) = fixture(0., 0.1, false);
    request
        .simulation
        .run
        .parameters
        .insert("cl".into(), q(0.3, "L/h"));
    request
        .simulation
        .run
        .parameters
        .insert("v".into(), q(3., "L"));
    for observation in &mut request.observations {
        observation.error.model = GaussianErrorModel::Lognormal;
    }
    request.observations[0].censoring = Some(GaussianCensoring::Left);
    request.observations[0].value.value = 1.2;
    let result = compiled.gaussian_objective(&request).unwrap();
    let cl = request.simulation.run.parameters["cl"].value;
    let v = request.simulation.run.parameters["v"].value;
    let mut expected = 0.;
    for observation in &request.observations {
        let prediction = 6. / v * libm::exp(-cl * times[observation.row] / v);
        let z = libm::log(observation.value.value / prediction) / 0.1;
        expected += if observation.censoring.is_some() {
            -libm::log(0.5 * libm::erfc(-z / std::f64::consts::SQRT_2))
        } else {
            0.5 * z * z
                + libm::log(0.1 * observation.value.value)
                + 0.5 * libm::log(2. * std::f64::consts::PI)
        };
    }
    assert!((result.objective - expected).abs() < 1e-8);
    for gradient in &result.gradient {
        let step = 1e-5;
        let mut shifted = request.clone();
        shifted
            .simulation
            .run
            .parameters
            .get_mut(&gradient.parameter)
            .unwrap()
            .value += step;
        let plus = compiled.gaussian_objective(&shifted).unwrap().objective;
        shifted
            .simulation
            .run
            .parameters
            .get_mut(&gradient.parameter)
            .unwrap()
            .value -= 2. * step;
        let minus = compiled.gaussian_objective(&shifted).unwrap().objective;
        assert!(
            (gradient.value - (plus - minus) / (2. * step)).abs() < 1e-5,
            "{} gradient {}",
            gradient.parameter,
            gradient.value
        );
    }
}

#[test]
fn left_censored_objective_gradient_matches_finite_difference() {
    let (compiled, mut request, _) = fixture(0.1, 0.05, false);
    request.observations[0].value = q(1.0, "mg/L");
    request.observations[0].censoring = Some(GaussianCensoring::Left);
    let center = compiled.gaussian_objective(&request).unwrap();
    let h = 1e-6;
    for (axis, name, unit) in [(0, "cl", "L/h"), (1, "v", "L")] {
        let initial = if axis == 0 { 0.3 } else { 3.0 };
        request
            .simulation
            .run
            .parameters
            .insert(name.into(), q(initial + h, unit));
        let plus = compiled.gaussian_objective(&request).unwrap().objective;
        request
            .simulation
            .run
            .parameters
            .insert(name.into(), q(initial - h, unit));
        let minus = compiled.gaussian_objective(&request).unwrap().objective;
        request
            .simulation
            .run
            .parameters
            .insert(name.into(), q(initial, unit));
        assert!((center.gradient[axis].value - (plus - minus) / (2. * h)).abs() < 1e-5);
    }
}

#[test]
fn rodas5p_scores_match_independent_likelihood_and_recover_parameters() {
    use qspquest_core::run::Solver;
    for (additive, proportional, prior) in [(0.2, 0., false), (0., 0.2, false), (0.1, 0.2, true)] {
        let (compiled, mut request, times) = fixture(additive, proportional, prior);
        request.simulation.run.solver = Solver::DiffsolRodas5p;
        let result = compiled.gaussian_objective(&request).unwrap();
        assert_eq!(
            result.identity.algorithm,
            "rodas5p_forward_gaussian_objective"
        );
        assert!((result.objective - oracle(&request, &times, 0.3, 3.)).abs() < 1e-7);
        for p in 0..2 {
            let h = 1e-5;
            let (hc, hv, lc, lv) = if p == 0 {
                (0.3 + h, 3., 0.3 - h, 3.)
            } else {
                (0.3, 3. + h, 0.3, 3. - h)
            };
            let expected =
                (oracle(&request, &times, hc, hv) - oracle(&request, &times, lc, lv)) / (2. * h);
            assert!((result.gradient[p].value - expected).abs() < 1e-5);
        }
    }
    let (compiled, _, _) = fixture(0.1, 0., false);
    let mut fit: FitRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-fit.json"
    ))
    .unwrap();
    let FitProblem::Individual(individual) = &mut fit.problem else {
        panic!()
    };
    individual.objective.simulation.run.solver = Solver::DiffsolRodas5p;
    let result = compiled.execute_fit(&fit).unwrap().fit.unwrap();
    assert_eq!(result.status, FitStatus::Converged);
    assert!((result.parameters["cl"].value - 0.6).abs() < 1e-5);
    assert!((result.parameters["v"].value - 4.5).abs() < 1e-5);
    assert_eq!(
        result.objective.identity.algorithm,
        "rodas5p_forward_gaussian_objective"
    );
}

#[test]
fn pooled_solver_identity_matches_each_subject_and_rejects_seeds() {
    use qspquest_core::run::Solver;
    let (compiled, _, _) = fixture(0.1, 0., false);
    let base: FitRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-fit.json"
    ))
    .unwrap();
    let FitProblem::Individual(individual) = base.problem else {
        panic!()
    };
    for (primary_solver, secondary_solver, algorithm) in [
        (
            Solver::DiffsolBdf,
            Solver::DiffsolBdf,
            "bdf_forward_pooled_gaussian_objective",
        ),
        (
            Solver::DiffsolRodas5p,
            Solver::DiffsolRodas5p,
            "rodas5p_forward_pooled_gaussian_objective",
        ),
        (
            Solver::DiffsolRodas5p,
            Solver::DiffsolBdf,
            "mixed_forward_pooled_gaussian_objective",
        ),
    ] {
        let mut primary = individual.clone();
        primary.objective.simulation.run.solver = primary_solver;
        let mut other = primary.objective.clone();
        other.simulation.run.solver = secondary_solver;
        let result = compiled
            .fit_pooled_gaussian(&PooledGaussianFitRequest {
                fit: primary,
                additional_subjects: vec![other],
            })
            .unwrap();
        assert_eq!(result.status, FitStatus::Converged);
        assert_eq!(result.objective.identity.algorithm, algorithm);
        assert!((result.parameters["cl"].value - 0.6).abs() < 1e-5);
        assert!((result.parameters["v"].value - 4.5).abs() < 1e-5);
    }
    let mut seeded = individual.objective.simulation;
    seeded.run.solver = Solver::DiffsolRodas5p;
    seeded.run.seed = Some(17);
    assert_eq!(
        compiled.run(&seeded).unwrap_err().code,
        qspquest_core::ErrorCode::Unsupported
    );
}

#[test]
fn lognormal_invalid_prediction_is_domain_failure_and_bad_data_is_invalid_input() {
    let (compiled, mut request, _) = fixture(0., 0.1, false);
    request.observations.truncate(1);
    request.observations[0].row = 0;
    request.observations[0].value = q(1., "mg/L");
    request.observations[0].error.model = GaussianErrorModel::Lognormal;
    for censoring in [None, Some(GaussianCensoring::Left)] {
        request.observations[0].censoring = censoring;
        assert_eq!(
            compiled.gaussian_objective(&request).unwrap_err().code,
            qspquest_core::ErrorCode::Domain
        );
    }
    request.observations[0].value.value = 0.;
    assert_eq!(
        compiled.gaussian_objective(&request).unwrap_err().code,
        qspquest_core::ErrorCode::InvalidInput
    );
}

use qspquest_core::{hmm::*, ErrorCode};
use qspquest_modeling::hmm::{execute, execute_json};

fn model() -> GaussianHmmModel {
    GaussianHmmModel {
        schema: MODEL_SCHEMA.into(),
        model_id: "synthetic-paired-hmm".into(),
        states: vec!["A".into(), "B".into(), "C".into()],
        endpoint_names: ["residual".into(), "assay".into()],
        endpoint_units: ["1".into(), "U/mL".into()],
        calendar_time_unit: Some("day".into()),
        initial_probability: vec![0.7, 0.3, 0.0],
        transition: vec![
            vec![0.8, 0.15, 0.05],
            vec![0.2, 0.7, 0.1],
            vec![0.05, 0.05, 0.9],
        ],
        emissions: vec![
            BivariateGaussian {
                mean: [0., 0.],
                variance: [0.8, 1.6],
                correlation: -0.1,
            },
            BivariateGaussian {
                mean: [-1.6, 2.4],
                variance: [1.2, 0.7],
                correlation: 0.3,
            },
            BivariateGaussian {
                mean: [2., 3.],
                variance: [2., 1.],
                correlation: -0.4,
            },
        ],
        lower_bounds: [None, Some(0.6)],
        first_entry_state: Some(2),
    }
}
fn request(n: usize) -> GaussianHmmRequest {
    let pairs = [[0.3, 0.6], [-1.6, 2.4], [2., 3.], [-2., 1.], [0., 0.]];
    GaussianHmmRequest {
        schema: REQUEST_SCHEMA.into(),
        records: pairs[..n]
            .iter()
            .map(|v| HmmObservation {
                values: *v,
                time: None,
            })
            .collect(),
        prior_record_count: 0,
    }
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-11 * (1. + b.abs()), "{a} != {b}");
}
fn density(values: [f64; 2], g: &BivariateGaussian) -> f64 {
    // Independent determinant/inverse covariance form, used in full path enumeration.
    let covariance = g.correlation * (g.variance[0] * g.variance[1]).sqrt();
    let det = g.variance[0] * g.variance[1] - covariance * covariance;
    let x = values[0] - g.mean[0];
    let y = values[1] - g.mean[1];
    (-0.5 * (g.variance[1] * x * x - 2. * covariance * x * y + g.variance[0] * y * y) / det).exp()
        / (std::f64::consts::TAU * det.sqrt())
}
fn paths(m: &GaussianHmmModel, r: &GaussianHmmRequest) -> Vec<(Vec<usize>, f64)> {
    let n = r.records.len();
    let k = m.states.len();
    (0..k.pow(n as u32))
        .map(|mut code| {
            let mut p = vec![0; n];
            for state in &mut p {
                *state = code % k;
                code /= k;
            }
            let mut weight = m.initial_probability[p[0]];
            for i in 0..n {
                if i > 0 {
                    weight *= m.transition[p[i - 1]][p[i]];
                }
                let v = r.records[i].values;
                weight *= density([v[0], v[1].max(0.6)], &m.emissions[p[i]]);
            }
            (p, weight)
        })
        .collect()
}

#[test]
fn three_state_filter_smoother_likelihood_and_viterbi_match_exhaustive_paths() {
    let m = model();
    for n in 1..=5 {
        let r = request(n);
        let result = execute(&m, &r).unwrap();
        let all = paths(&m, &r);
        let total = all.iter().map(|p| p.1).sum::<f64>();
        close(result.log_likelihood.unwrap(), total.ln());
        let best = all.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert_eq!(result.viterbi_state_index, best.0);
        close(result.viterbi_log_joint.unwrap(), best.1.ln());
        for i in 0..n {
            let prefix = paths(&m, &request(i + 1));
            let norm = prefix.iter().map(|p| p.1).sum::<f64>();
            for state in 0..3 {
                close(
                    result.records[i].smoothed_state_probability[state],
                    all.iter()
                        .filter(|p| p.0[i] == state)
                        .map(|p| p.1)
                        .sum::<f64>()
                        / total,
                );
                close(
                    result.records[i].filtered_state_probability[state],
                    prefix
                        .iter()
                        .filter(|p| *p.0.last().unwrap() == state)
                        .map(|p| p.1)
                        .sum::<f64>()
                        / norm,
                );
            }
        }
    }
}
#[test]
fn first_observation_uses_initial_mass_and_calendar_labels_never_advance_chain() {
    let mut m = model();
    m.initial_probability = vec![1., 0., 0.];
    let mut r = request(5);
    let a = execute(&m, &r).unwrap();
    assert_eq!(a.records[0].filtered_state_probability, vec![1., 0., 0.]);
    for (row, t) in r.records.iter_mut().zip([0., 0., 0.5, 63., 1000.]) {
        row.time = Some(t);
    }
    let b = execute(&m, &r).unwrap();
    assert_eq!(a.log_likelihood, b.log_likelihood);
    assert_eq!(a.viterbi_state_index, b.viterbi_state_index);
    for (x, y) in a.records.iter().zip(&b.records) {
        assert_eq!(x.filtered_state_probability, y.filtered_state_probability);
    }
}
#[test]
fn first_entry_prior_preserves_initial_target_mass_and_absorbs_only_for_hitting_distribution() {
    let mut m = model();
    m.first_entry_state = Some(1);
    let r = GaussianHmmRequest {
        records: vec![],
        prior_record_count: 3,
        ..request(0)
    };
    let out = execute(&m, &r).unwrap();
    assert_eq!(out.log_likelihood, None);
    assert_eq!(out.viterbi_log_joint, None);
    close(out.prior[0].first_entry_cdf.unwrap(), 0.3);
    close(out.prior[1].first_entry_cdf.unwrap(), 0.3 + 0.7 * 0.15);
    close(out.prior[1].state_probability[1], 0.7 * 0.15 + 0.3 * 0.7);
    close(
        out.prior[1].first_entry_survival.unwrap(),
        1. - out.prior[1].first_entry_cdf.unwrap(),
    );
}
#[test]
fn tiny_first_entry_probability_and_zero_one_transition_endpoints() {
    for p in [0., 1e-17, 0.03, 1.] {
        let mut m = model();
        m.states.truncate(2);
        m.emissions.truncate(2);
        m.initial_probability = vec![1., 0.];
        m.transition = vec![vec![1. - p, p], vec![0.003, 0.997]];
        m.first_entry_state = Some(1);
        let out = execute(
            &m,
            &GaussianHmmRequest {
                prior_record_count: 10001,
                ..request(0)
            },
        )
        .unwrap();
        let expected = if p == 1. {
            1.
        } else {
            -(10000. * (-p).ln_1p()).exp_m1()
        };
        let actual = out.prior.last().unwrap().first_entry_cdf.unwrap();
        assert!((actual - expected).abs() <= 1e-11 * expected.max(1e-300));
        close(out.prior[0].first_entry_cdf.unwrap(), 0.);
    }
}
#[test]
fn floored_gaussian_tail_moment_is_stable_across_coordinate_scaling() {
    for sd in [1., 1e18] {
        let mut m = model();
        for e in &mut m.emissions {
            e.mean[1] = -9. * sd;
            e.variance[1] = sd * sd;
        }
        m.lower_bounds[1] = Some(0.);
        let out = execute(&m, &request(1)).unwrap();
        let expected = sd * 1.2247791808434907e-20;
        assert!((out.records[0].predictive_mean[1] - expected).abs() < 2e-12 * expected);
    }
}
#[test]
fn floor_is_point_preprocessing_and_assay_unit_changes_preserve_inference() {
    let m = model();
    let a = execute(&m, &request(5)).unwrap();
    assert_eq!(a.records[4].used, [0., 0.6]);
    let mut r = request(5);
    r.records[4].values[1] = 0.6;
    assert_eq!(a.log_likelihood, execute(&m, &r).unwrap().log_likelihood);
    let factor = 1000.;
    let mut scaled = m.clone();
    for e in &mut scaled.emissions {
        e.mean[1] *= factor;
        e.variance[1] *= factor * factor;
    }
    scaled.lower_bounds[1] = Some(0.6 * factor);
    for row in &mut r.records {
        row.values[1] *= factor;
    }
    let b = execute(&scaled, &r).unwrap();
    close(
        b.log_likelihood.unwrap(),
        a.log_likelihood.unwrap() - 5. * factor.ln(),
    );
    assert_eq!(a.viterbi_state_index, b.viterbi_state_index);
    for (x, y) in a.records.iter().zip(&b.records) {
        for j in 0..3 {
            close(
                x.filtered_state_probability[j],
                y.filtered_state_probability[j],
            );
        }
    }
}
#[test]
fn zero_probability_paths_and_long_extreme_observations_serialize_finite_results() {
    let mut m = model();
    m.initial_probability = vec![1., 0., 0.];
    m.transition = vec![vec![1., 0., 0.], vec![0., 1., 0.], vec![0., 0., 1.]];
    let r = GaussianHmmRequest {
        records: vec![
            HmmObservation {
                values: [1000., 1000.],
                time: None
            };
            40
        ],
        ..request(0)
    };
    let out = execute(&m, &r).unwrap();
    assert!(out.log_likelihood.unwrap().is_finite());
    assert_eq!(out.viterbi_state_index, vec![0; 40]);
    assert!(serde_json::to_string(&out).is_ok());
    assert_eq!(out.records[39].smoothed_state_probability, vec![1., 0., 0.]);
}
#[test]
fn ties_choose_first_index_deterministically() {
    let mut m = model();
    m.initial_probability = vec![1. / 3.; 3];
    m.transition = vec![vec![1. / 3.; 3]; 3];
    m.emissions = vec![m.emissions[0].clone(); 3];
    assert_eq!(
        execute(&m, &request(5)).unwrap().viterbi_state_index,
        vec![0; 5]
    );
}

#[test]
fn calendar_label_units_are_explicit_metadata_and_never_a_transition_clock() {
    let mut m = model();
    let mut r = request(2);
    r.records[0].time = Some(0.);
    r.records[1].time = Some(1000.);
    let out = execute(&m, &r).unwrap();
    assert_eq!(out.calendar_time_unit, Some("day".into()));
    m.calendar_time_unit = None;
    assert!(execute(&m, &r).is_err());
    m.calendar_time_unit = Some(" ".into());
    assert!(execute(&m, &r).is_err());
}
#[test]
fn row_roundoff_policy_is_explicit_and_prevents_systematic_mass_drift() {
    let mut m = model();
    m.transition[0][0] += 5e-13;
    let result = execute(
        &m,
        &GaussianHmmRequest {
            prior_record_count: 1000,
            ..request(0)
        },
    )
    .unwrap();
    for row in &result.prior {
        close(row.state_probability.iter().sum(), 1.);
    }
    assert_ne!(result.evaluated_transition, m.transition);
    assert!(result.probability_policy.contains("1e-12"));
    let original = execute(&model(), &request(1)).unwrap();
    assert_ne!(result.model_content_hash, original.model_content_hash);
}
#[test]
fn invalid_matrix_variance_correlation_and_native_nonfinite_inputs_are_rejected() {
    let mut cases = vec![];
    let mut m = model();
    m.transition[0][0] = 0.5;
    cases.push(m);
    let mut m = model();
    m.emissions[0].variance[0] = 0.;
    cases.push(m);
    let mut m = model();
    m.emissions[0].correlation = 1.;
    cases.push(m);
    let mut m = model();
    m.emissions[0].mean[0] = f64::NAN;
    cases.push(m);
    let mut m = model();
    m.states[1] = m.states[0].clone();
    cases.push(m);
    let mut m = model();
    m.first_entry_state = Some(3);
    cases.push(m);
    let mut m = model();
    m.schema = "future".into();
    cases.push(m);
    for m in cases {
        assert!(execute(&m, &request(1)).is_err());
    }
    let mut r = request(2);
    r.records[0].time = Some(1.);
    r.records[1].time = Some(0.);
    assert!(execute(&model(), &r).is_err());
    r.records[0].values[0] = f64::INFINITY;
    assert!(execute(&model(), &r).is_err());
}
#[test]
fn strict_json_rejects_missing_boolean_null_unknown_and_excessive_inputs() {
    let m = serde_json::to_string(&model()).unwrap();
    let r = serde_json::to_value(request(1)).unwrap();
    for bad in [
        serde_json::json!([true, 0.6]),
        serde_json::json!([null, 0.6]),
        serde_json::json!([0.3]),
    ] {
        let mut mutated = r.clone();
        mutated["records"][0]["values"] = bad;
        assert!(execute_json(&m, &mutated.to_string()).is_err());
    }
    let mut unknown = r.clone();
    unknown["records"][0]["dose"] = serde_json::json!(300);
    assert!(execute_json(&m, &unknown.to_string()).is_err());
    assert!(execute_json(&m, &" ".repeat(1_000_001)).is_err());
    assert!(execute(&model(), &request(0)).is_err());
    assert_eq!(
        execute(
            &model(),
            &GaussianHmmRequest {
                prior_record_count: 100001,
                ..request(0)
            }
        )
        .unwrap_err()
        .code,
        ErrorCode::WorkBudget
    );
    let budget = GaussianHmmRequest {
        records: vec![request(1).records[0].clone(); 50000],
        prior_record_count: 50000,
        ..request(0)
    };
    assert_eq!(
        execute(&model(), &budget).unwrap_err().code,
        ErrorCode::WorkBudget
    );
}

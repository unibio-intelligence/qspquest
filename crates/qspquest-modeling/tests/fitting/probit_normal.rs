use qspquest_core::{
    fit::FitStatus,
    probit_fit::{
        BinomialLitter as Litter, ProbitNormalBinomialFitRequest as Request, ProbitNormalFitSchema,
    },
};
use qspquest_modeling::probit_normal::{
    fit_probit_normal_binomial as fit, probit_normal_binomial_objective as objective,
};
fn row(y: u32, n: u32) -> Litter {
    Litter {
        successes: y,
        total: n,
        group: 0,
    }
}
#[test]
fn zero_variance_and_unbounded_mixture_match_probability_identities() {
    for tau in [0., 0.01, 1., 9.] {
        let rho = tau / (1. + tau);
        let expected = 0.25 + f64::asin(rho) / (2. * std::f64::consts::PI);
        let actual = objective([0., 0., tau, 0.], &[row(2, 2)]).unwrap();
        assert!((f64::exp(-actual.negative_log_likelihood) - expected).abs() < 2e-12);
        let mut sum = 0.;
        for y in 0..=8 {
            sum += f64::exp(
                -objective([1.2, 0., tau, 0.], &[row(y, 8)])
                    .unwrap()
                    .negative_log_likelihood,
            );
        }
        assert!((sum - 1.).abs() < 2e-11, "{tau}: {sum}");
    }
    for mean in [-8., -2., 0., 2., 8.] {
        let expected = 0.5 * libm::erfc(-mean / 2.);
        let out = objective([mean, 0., 1., 0.], &[row(1, 1)]).unwrap();
        assert!((f64::exp(-out.negative_log_likelihood) - expected).abs() < 2e-12);
    }
}
#[test]
fn analytic_variance_scores_and_hessians_match_finite_differences() {
    let rows = vec![row(0, 5), row(2, 6), row(5, 5)];
    let p = [0.3, 0., 0.4, 0.];
    let base = objective(p, &rows).unwrap();
    let step = 2e-4;
    for i in [0, 2] {
        let mut left = p;
        let mut right = p;
        left[i] -= step;
        right[i] += step;
        let l = objective(left, &rows).unwrap();
        let r = objective(right, &rows).unwrap();
        assert!(
            ((r.negative_log_likelihood - l.negative_log_likelihood) / (2. * step)
                - base.gradient[i])
                .abs()
                < 2e-6
        );
        for j in [0, 2] {
            assert!(
                ((r.gradient[j] - l.gradient[j]) / (2. * step) - base.hessian[j][i]).abs() < 2e-5
            );
        }
    }
}
fn toy() -> Request {
    let mut observations = Vec::new();
    for group in 0..2 {
        for _ in 0..4 {
            for y in 0..=2 {
                observations.push(Litter {
                    successes: y,
                    total: 2,
                    group,
                });
            }
        }
    }
    Request {
        schema: ProbitNormalFitSchema::V01,
        observations,
        initial_means: [0., 0.],
        mean_bounds: [[-4., 4.]; 2],
        initial_sds: [0., 0.],
        sd_bounds: [[0., 3.]; 2],
        max_evaluations: 500,
        max_iterations: 200,
        gradient_tolerance: 1e-8,
        max_quadrature_evaluations: 20_000_000,
    }
}
#[test]
fn zero_start_moves_when_variance_score_is_nonzero() {
    let r = fit(&toy()).unwrap();
    assert_eq!(r.status, FitStatus::Converged);
    for i in 0..2 {
        assert!(r.means[i].abs() < 1e-10);
        assert!((r.sds[i] - 1.).abs() < 1e-7);
    }
    assert!((r.negative_log_likelihood - 24. * f64::ln(3.)).abs() < 1e-10);
    assert!(r.projected_gradient_norm <= 1e-8);
}
#[test]
fn boundary_fit_and_limits_are_explicit() {
    let mut request = toy();
    for row in &mut request.observations {
        row.successes = 1;
    }
    request.initial_means = [0.2, -0.1];
    request.initial_sds = [0.5, 0.8];
    let r = fit(&request).unwrap();
    assert_eq!(r.status, FitStatus::Converged);
    assert_eq!(r.sds, [0., 0.]);
    assert!(r.means.iter().all(|v| v.abs() < 1e-8));
    let mut limited = toy();
    limited.max_evaluations = 1;
    let r = fit(&limited).unwrap();
    assert_eq!(r.status, FitStatus::EvaluationLimit);
    assert_eq!(r.evaluations, 1);
    request.observations[0].successes = 3;
    assert!(fit(&request).is_err());
    assert!(objective([0., 0., -1., 0.], &[row(1, 2)]).is_err());
}
#[test]
fn quadrature_budget_and_typed_counts_refuse_without_false_convergence() {
    let mut request = toy();
    request.max_quadrature_evaluations = 1;
    assert_eq!(
        fit(&request).unwrap_err().code,
        qspquest_core::ErrorCode::WorkBudget
    );
    let mut encoded = serde_json::to_value(toy()).unwrap();
    encoded["observations"][0]["successes"] = serde_json::json!(0.5);
    assert!(serde_json::from_value::<Request>(encoded).is_err());
}

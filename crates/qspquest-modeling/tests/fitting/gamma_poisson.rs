use qspquest_core::{discrete_fit::*, fit::FitStatus, model::Quantity, ErrorCode};
use qspquest_modeling::discrete_fit::{fit_gamma_poisson, gamma_poisson_objective};
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
fn observations() -> Vec<CountExposure> {
    [(0, 1.), (1, 1.)]
        .into_iter()
        .map(|(count, exposure)| CountExposure {
            count,
            exposure: q(exposure, "h"),
        })
        .collect()
}
fn request() -> GammaPoissonFitRequest {
    GammaPoissonFitRequest {
        schema: GammaPoissonFitSchema::V01,
        observations: observations(),
        initial_shape: 1.,
        shape_bounds: [0.01, 100.],
        initial_rate: q(1., "h"),
        rate_bounds: [q(0.01, "h"), q(100., "h")],
        max_evaluations: 1000,
        max_iterations: 200,
        gradient_tolerance: 1e-7,
    }
}
#[test]
fn integrated_geometric_mass_matches_closed_form_and_analytic_score() {
    let result = gamma_poisson_objective(1., &q(1., "h"), &observations()).unwrap();
    assert!((result.negative_log_likelihood - 8_f64.ln()).abs() < 1e-13);
    assert!((result.log_gradient[0] - (2. * 2_f64.ln() - 1.)).abs() < 1e-13);
    assert!((result.log_gradient[1] + 0.5).abs() < 1e-13);
    for axis in 0..2 {
        let step = 1e-5;
        let mut plus = [0_f64; 2];
        let mut minus = plus;
        plus[axis] += step;
        minus[axis] -= step;
        let high = gamma_poisson_objective(plus[0].exp(), &q(plus[1].exp(), "h"), &observations())
            .unwrap();
        let low = gamma_poisson_objective(minus[0].exp(), &q(minus[1].exp(), "h"), &observations())
            .unwrap();
        assert!(
            (result.log_gradient[axis]
                - (high.negative_log_likelihood - low.negative_log_likelihood) / (2. * step))
                .abs()
                < 1e-9
        );
        for row in 0..2 {
            assert!(
                (result.log_hessian[row][axis]
                    - (high.log_gradient[row] - low.log_gradient[row]) / (2. * step))
                    .abs()
                    < 1e-9
            );
        }
    }
}
#[test]
fn integer_probability_mass_sums_to_one() {
    let mut mass = 0.;
    for count in 0..150 {
        let row = CountExposure {
            count,
            exposure: q(2., "h"),
        };
        mass += (-gamma_poisson_objective(0.7, &q(3., "h"), &[row])
            .unwrap()
            .negative_log_likelihood)
            .exp();
    }
    assert!((mass - 1.).abs() < 1e-12);
}
#[test]
fn exposure_and_rate_unit_conversion_preserves_likelihood_and_score() {
    let base = gamma_poisson_objective(0.7, &q(2., "h"), &observations()).unwrap();
    let mut changed = observations();
    for row in &mut changed {
        row.exposure = q(3600., "s");
    }
    let other = gamma_poisson_objective(0.7, &q(7200., "s"), &changed).unwrap();
    assert!((base.negative_log_likelihood - other.negative_log_likelihood).abs() < 1e-13);
    for axis in 0..2 {
        assert!((base.log_gradient[axis] - other.log_gradient[axis]).abs() < 1e-13);
    }
    let fit = fit_gamma_poisson(&request()).unwrap();
    let mut converted = request();
    converted.initial_rate = q(3600., "s");
    converted.rate_bounds = [q(36., "s"), q(360000., "s")];
    converted.observations = changed;
    let other = fit_gamma_poisson(&converted).unwrap();
    assert_eq!(fit.status, other.status);
    assert!((fit.parameters["shape"].value - other.parameters["shape"].value).abs() < 1e-5);
    assert!((fit.parameters["rate"].value - other.parameters["rate"].value / 3600.).abs() < 1e-5);
}
#[test]
fn work_limits_and_active_bounds_are_explicit() {
    let mut r = request();
    r.max_evaluations = 1;
    let limited = fit_gamma_poisson(&r).unwrap();
    assert_eq!(limited.status, FitStatus::EvaluationLimit);
    assert_eq!(limited.evaluations, 1);
    let fit = fit_gamma_poisson(&request()).unwrap();
    assert_eq!(fit.status, FitStatus::Converged, "{fit:?}");
    assert!(fit.projected_gradient_norm <= 1e-7);
    assert!(fit.active_bounds.contains(&"rate".to_string()));
    // For counts 0 and 1 at unit exposure, the bounded maximum has
    // beta=100 and alpha=1/(2*log(1+1/beta)).
    assert_eq!(fit.parameters["rate"].value, 100.);
    assert!((fit.parameters["shape"].value - 1. / (2. * 1.01_f64.ln())).abs() < 1e-5);
    assert!(fit.evaluations <= 1000);
    assert!(fit.iterations <= 200);
}
#[test]
fn invalid_quantities_counts_and_work_requests_refuse() {
    for mode in 0..7 {
        let mut r = request();
        match mode {
            0 => r.observations[0].exposure.value = 0.,
            1 => r.observations[0].exposure.unit = "mg".into(),
            2 => r.shape_bounds = [0., 1.],
            3 => r.rate_bounds = [q(2., "h"), q(1., "h")],
            4 => r.max_evaluations = 0,
            5 => r.observations[0].count = 1_000_001,
            _ => r.observations[0].count = 100_000,
        }
        assert!(fit_gamma_poisson(&r).is_err(), "mode={mode}");
    }
    let error = gamma_poisson_objective(0., &q(1., "h"), &observations()).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    let value = serde_json::json!({"count":1.5,"exposure":{"value":1.,"unit":"h"}});
    assert!(serde_json::from_value::<CountExposure>(value).is_err());
    let value = serde_json::json!({"count":-1,"exposure":{"value":1.,"unit":"h"}});
    assert!(serde_json::from_value::<CountExposure>(value).is_err());
}

use qspquest_modeling::linear_pk::LinearPk;
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9 + 1e-10 * b.abs(), "{a} vs {b}");
}
#[test]
fn independent_matrix_exponential_reference() {
    let doc: serde_json::Value = serde_json::from_str(include_str!(
        "../../../conformance/evidence/synthetic-linear-pk-reference.json"
    ))
    .unwrap();
    for c in doc["cases"].as_array().unwrap() {
        let v = |name: &str| {
            c[name]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect::<Vec<_>>()
        };
        let m = LinearPk::new(
            c["clearance"].as_f64().unwrap(),
            &v("volumes"),
            &v("exchange"),
            c["absorption"].as_f64(),
        )
        .unwrap();
        let result = m
            .advance(&v("amounts"), &v("rates"), c["duration"].as_f64().unwrap())
            .unwrap();
        for (a, b) in result.iter().zip(v("expected")) {
            close(*a, b);
        }
    }
}
#[test]
fn one_compartment_closed_forms_and_coincident_absorption_rate() {
    let m = LinearPk::new(2., &[10.], &[], None).unwrap();
    for t in [0., 1e-9, 1., 10., 1000.] {
        let e = libm::exp(-0.2 * t);
        let result = m.advance(&[30.], &[4.], t).unwrap();
        close(result[0], 30. * e - 20. * libm::expm1(-0.2 * t));
        close(m.concentrations(&result).unwrap()[0], result[0] / 10.);
    }
    let m = LinearPk::new(2., &[10.], &[], Some(0.2)).unwrap();
    for t in [0., 1., 10., 100.] {
        let r = m.advance(&[40., 0.], &[0., 0.], t).unwrap();
        close(r[0], 40. * libm::exp(-0.2 * t));
        close(r[1], 8. * t * libm::exp(-0.2 * t));
    }
}
#[test]
fn zero_rates_conservation_and_split_intervals() {
    let zero = LinearPk::new(0., &[2., 4., 8.], &[0., 0.], Some(0.)).unwrap();
    assert_eq!(
        zero.advance(&[1., 2., 3., 4.], &[2., 3., 4., 5.], 0.)
            .unwrap(),
        vec![1., 2., 3., 4.]
    );
    let r = zero
        .advance(&[1., 2., 3., 4.], &[2., 3., 4., 5.], 10.)
        .unwrap();
    for (a, b) in r.iter().zip([21., 32., 43., 54.]) {
        close(*a, b);
    }
    let m = LinearPk::new(0., &[2., 4., 8.], &[0.3, 0.8], Some(0.4)).unwrap();
    let start = [40., 3., 5., 8.];
    let rates = [2., 1., 0., 0.5];
    let whole = m.advance(&start, &rates, 24.).unwrap();
    close(whole.iter().sum(), 56. + 3.5 * 24.);
    let half = m.advance(&start, &rates, 12.).unwrap();
    let split = m.advance(&half, &rates, 12.).unwrap();
    for (a, b) in whole.iter().zip(split) {
        close(*a, b);
    }
}
#[test]
fn invalid_parameters_and_intervals_reject_without_mutation() {
    for cl in [-1., f64::NAN, f64::INFINITY] {
        assert!(LinearPk::new(cl, &[1.], &[], None).is_err());
    }
    assert!(LinearPk::new(1., &[0.], &[], None).is_err());
    assert!(LinearPk::new(1., &[1., 2.], &[], None).is_err());
    let m = LinearPk::new(1., &[1.], &[], None).unwrap();
    assert!(m.advance(&[1.], &[0.], -1.).is_err());
    assert!(m.advance(&[-1.], &[0.], 1.).is_err());
    assert!(m.advance(&[1.], &[], 1.).is_err());
    assert!(m.advance(&[1.], &[f64::MAX], 2.).is_err());
    assert!(m.advance(&[1.], &[0.], f64::MAX).is_err());
    close(m.advance(&[1.], &[0.], 1.).unwrap()[0], libm::exp(-1.));
}

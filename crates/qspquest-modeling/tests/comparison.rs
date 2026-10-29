#[path = "../../../conformance/compare.rs"]
mod comparison;
use comparison::*;
use std::collections::BTreeMap;
fn fixture() -> (Trajectory, BTreeMap<String, Tolerance>) {
    (
        Trajectory {
            time_unit: "h".into(),
            times: vec![0., 0., 1.],
            sides: vec![Side::Pre, Side::Post, Side::Post],
            outputs: vec![Output {
                name: "cp".into(),
                unit: "mg/L".into(),
                values: vec![0., 1., 0.5],
            }],
        },
        BTreeMap::from([(
            "cp".into(),
            Tolerance {
                unit: "mg/L".into(),
                absolute: 1e-8,
                relative: 1e-6,
            },
        )]),
    )
}
#[test]
fn units_and_declared_error_limits_are_applied() {
    let (r, t) = fixture();
    let mut c = r.clone();
    c.time_unit = "min".into();
    c.times[2] = 60.;
    c.outputs[0].unit = "ug/L".into();
    c.outputs[0].values = vec![0., 1000., 500.];
    assert!(compare(&r, &c, &t).unwrap().passed);
    c.outputs[0].values[2] += 0.0001;
    assert!(compare(&r, &c, &t).unwrap().passed);
    c.outputs[0].values[2] = 510.;
    let report = compare(&r, &c, &t).unwrap();
    assert!(!report.passed);
    assert_eq!(report.outputs[0].failed_values, 1);
    assert_eq!(report.outputs[0].worst_row, 2);
}
#[test]
fn missing_values_names_sides_and_invalid_numbers_never_pass() {
    let (r, t) = fixture();
    let mut cases = vec![];
    let mut c = r.clone();
    c.sides.swap(0, 1);
    cases.push(c);
    let mut c = r.clone();
    c.times[2] += 1e-15;
    cases.push(c);
    let mut c = r.clone();
    c.outputs[0].values.pop();
    cases.push(c);
    let mut c = r.clone();
    c.outputs[0].values[1] = f64::NAN;
    cases.push(c);
    let mut c = r.clone();
    c.outputs[0].name = "other".into();
    cases.push(c);
    let mut c = r.clone();
    c.outputs.push(c.outputs[0].clone());
    cases.push(c);
    let mut c = r.clone();
    c.outputs[0].unit = "nmol/L".into();
    cases.push(c);
    for c in cases {
        assert!(compare(&r, &c, &t).is_err());
    }
    let mut bad = t.clone();
    bad.get_mut("cp").unwrap().absolute = f64::INFINITY;
    assert!(compare(&r, &r, &bad).is_err());
    assert!(compare(&r, &r, &BTreeMap::new()).is_err());
}
#[test]
fn named_outputs_are_matched_independently_of_column_order() {
    let (mut r, mut t) = fixture();
    r.outputs.push(Output {
        name: "mass".into(),
        unit: "mg".into(),
        values: vec![0., 3., 1.5],
    });
    t.insert(
        "mass".into(),
        Tolerance {
            unit: "mg".into(),
            absolute: 1e-8,
            relative: 1e-6,
        },
    );
    let mut c = r.clone();
    c.outputs.reverse();
    assert!(compare(&r, &c, &t).unwrap().passed);
}

#[test]
fn negative_absolute_times_preserve_units_sides_and_failure_location() {
    let (mut reference, tolerance) = fixture();
    reference.times = vec![-2., -2., -1.];
    let mut candidate = reference.clone();
    candidate.time_unit = "min".into();
    candidate.times = vec![-120., -120., -60.];
    assert!(compare(&reference, &candidate, &tolerance).unwrap().passed);
    candidate.outputs[0].values[1] += 0.1;
    let report = compare(&reference, &candidate, &tolerance).unwrap();
    assert!(!report.passed);
    assert_eq!(report.outputs[0].worst_time, -2.);
    assert_eq!(report.outputs[0].worst_side, Side::Post);
    candidate.times[1] = -119.;
    assert!(compare(&reference, &candidate, &tolerance).is_err());
    candidate = reference.clone();
    candidate.sides.swap(0, 1);
    assert!(compare(&reference, &candidate, &tolerance).is_err());
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut bad = reference.clone();
        bad.times[0] = invalid;
        assert!(compare(&bad, &bad, &tolerance).is_err());
    }
}

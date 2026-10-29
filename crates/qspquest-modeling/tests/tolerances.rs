use qspquest_core::{model::Quantity, regimen::Request};
use qspquest_modeling::{language, Budgets, ErrorCode};
use std::collections::BTreeMap;
const SOURCE: &str = include_str!("../../../conformance/models/synthetic-mixed-scales.qsp");
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
fn request() -> Request {
    serde_json::from_value(serde_json::json!({"end":q(2.,"h"),"samples":(0..=20).map(|i|q(i as f64/10.,"h")).collect::<Vec<_>>(),"administrations":[],"rtol":1e-3,"absolute_tolerances":{"bulk":q(1e-2,"mg"),"trace":q(1e-15,"mol")}})).unwrap()
}
#[test]
fn per_state_tolerances_control_small_states_and_preserve_equivalent_units() {
    let c = language::parse(SOURCE).unwrap().compile().unwrap();
    let b = c.bind(&BTreeMap::new()).unwrap();
    let r = request();
    let tight = b.simulate_regimen(&r, Default::default()).unwrap();
    let mut equivalent = r.clone();
    equivalent
        .absolute_tolerances
        .as_mut()
        .unwrap()
        .insert("trace".into(), q(1e-6, "nmol"));
    equivalent
        .absolute_tolerances
        .as_mut()
        .unwrap()
        .insert("bulk".into(), q(10., "ug"));
    let other = b.simulate_regimen(&equivalent, Default::default()).unwrap();
    let err = |rows: &Vec<qspquest_modeling::Sample>| {
        rows.iter()
            .map(|r| (r.values[1] - 1e-9 * (-10. * r.time).exp()).abs())
            .fold(0.0_f64, f64::max)
    };
    assert!(err(&tight) < 1e-11);
    for (a, b) in tight.iter().zip(&other) {
        assert_eq!(a.time, b.time);
        assert!((a.values[0] - b.values[0]).abs() < 1e-10);
        assert!((a.values[1] - b.values[1]).abs() < 1e-18);
    }
    let mut loose = r.clone();
    loose.absolute_tolerances = None;
    loose.atol = Some(1e-2);
    let loose = b.simulate_regimen(&loose, Default::default()).unwrap();
    assert!(
        err(&tight) < err(&loose) * 0.1,
        "tight {} loose {}",
        err(&tight),
        err(&loose)
    );
    assert_eq!(
        c.regimen_protocol(&r, Default::default()).unwrap_err().code,
        ErrorCode::Unsupported
    );
}
#[test]
fn incomplete_conflicting_nonpositive_and_dimensionally_wrong_tolerances_fail() {
    let c = language::parse(SOURCE).unwrap().compile().unwrap();
    let b = c.bind(&BTreeMap::new()).unwrap();
    let good = request();
    let mut cases = vec![];
    let mut r = good.clone();
    r.atol = Some(1e-8);
    cases.push(r);
    let mut r = good.clone();
    r.absolute_tolerances = None;
    cases.push(r);
    let mut r = good.clone();
    r.absolute_tolerances.as_mut().unwrap().remove("trace");
    cases.push(r);
    let mut r = good.clone();
    r.absolute_tolerances
        .as_mut()
        .unwrap()
        .insert("unknown".into(), q(1e-6, "mg"));
    cases.push(r);
    for value in [0., -1., f64::NAN, f64::INFINITY] {
        let mut r = good.clone();
        r.absolute_tolerances
            .as_mut()
            .unwrap()
            .insert("trace".into(), q(value, "mol"));
        cases.push(r);
    }
    let mut r = good.clone();
    r.absolute_tolerances
        .as_mut()
        .unwrap()
        .insert("trace".into(), q(1e-6, "mg"));
    cases.push(r);
    for r in cases {
        assert!(b.simulate_regimen(&r, Budgets::default()).is_err());
    }
    assert!(b.simulate_regimen(&good, Budgets::default()).is_ok());
}

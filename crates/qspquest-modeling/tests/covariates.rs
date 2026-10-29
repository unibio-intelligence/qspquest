use qspquest_core::{model::Quantity, regimen::*};
use qspquest_modeling::{language, Budgets, ErrorCode, Model};
use std::collections::BTreeMap;
const SOURCE: &str = include_str!("../../../conformance/models/synthetic-covariate.qsp");
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
fn request() -> Request {
    serde_json::from_value(serde_json::json!({
        "end":q(5.,"h"),"samples":[q(0.,"h"),q(1.,"h"),q(2.,"h"),q(3.,"h"),q(5.,"h")],
        "administrations":([0.,2.].map(|t|serde_json::json!({"target":"c","time":q(t,"h"),"amount":q(6.,"mg"),"delivery":{"kind":"bolus"},"bioavailability":1.}))),
        "covariate_changes":[{"name":"wt","time":q(120.,"min"),"value":q(140000.,"g")}],"rtol":1e-10,"atol":1e-12
    })).unwrap()
}
#[test]
fn step_boundaries_hold_left_binding_and_project_both_sides() {
    let compiled = language::parse(SOURCE).unwrap().compile().unwrap();
    let bound = compiled.bind(&BTreeMap::new()).unwrap();
    let mut req = request();
    req.checkpoints = vec![q(1., "h"), q(2., "h"), q(4., "h")];
    let rows = bound.simulate_regimen(&req, Budgets::default()).unwrap();
    for row in &rows {
        let right = row.time > 2. || (row.time == 2. && row.side == "post");
        let expected = if row.time == 0. && row.side == "pre" {
            2.
        } else if !right {
            3.6 * (-0.1 * row.time).exp()
        } else {
            (3.6 * (-0.2_f64).exp() + 0.8) * (-0.1 * 2_f64.powf(-0.25) * (row.time - 2.)).exp()
        };
        assert!(
            (row.values[0] - expected).abs() < 1e-8,
            "{row:?} vs {expected}"
        );
        assert!((row.values[1] - expected / if right { 2. } else { 1. }).abs() < 1e-8);
        assert_eq!(row.values[2], if right { 140. } else { 70. });
    }
    // Selected observations preserve order and side-specific bindings after solve.
    let mut req = req;
    req.samples.clear();
    req.observations = Some(vec![
        ObservationPoint {
            time: q(2., "h"),
            side: ObservationSide::Post,
        },
        ObservationPoint {
            time: q(2., "h"),
            side: ObservationSide::Pre,
        },
    ]);
    let selected = bound.simulate_regimen(&req, Default::default()).unwrap();
    assert_eq!(selected[0].values[2], 140.);
    assert_eq!(selected[1].values[2], 70.);
    // A completed run does not change the bound baseline.
    assert_eq!(bound.initial(), vec![2.]);
    assert_eq!(bound.outputs(0., &[2.]).unwrap()[2], 70.);
}
#[test]
fn required_covariates_units_and_format_roundtrip() {
    let source = SOURCE.replace(" default 70 [kg]", "");
    let parsed = language::parse(&source).unwrap();
    let compiled = parsed.compile().unwrap();
    assert!(compiled.bind(&BTreeMap::new()).is_err());
    let b = compiled
        .bind_with_covariates(
            &BTreeMap::new(),
            &BTreeMap::from([("wt".into(), q(140000., "g"))]),
        )
        .unwrap();
    assert_eq!(b.initial(), vec![4.]);
    assert!(compiled
        .bind_with_covariates(
            &BTreeMap::new(),
            &BTreeMap::from([("wt".into(), q(140., "h"))])
        )
        .is_err());
    let text = language::format(&parsed.document).unwrap();
    assert_eq!(language::parse(&text).unwrap().document, parsed.document);
}
#[test]
fn invalid_steps_and_shared_budgets_fail_without_poisoning_next_run() {
    let compiled = language::parse(SOURCE).unwrap().compile().unwrap();
    let b = compiled.bind(&BTreeMap::new()).unwrap();
    let good = request();
    assert_eq!(
        compiled
            .regimen_protocol(&good, Default::default())
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    let mut bad = good.clone();
    bad.covariate_changes.push(bad.covariate_changes[0].clone());
    assert!(b.simulate_regimen(&bad, Default::default()).is_err());
    let mut bad = good.clone();
    bad.covariate_changes[0].name = "tv_v".into();
    assert!(b.simulate_regimen(&bad, Default::default()).is_err());
    let mut bad = good.clone();
    bad.covariate_changes[0].value = q(0., "kg");
    assert!(b.simulate_regimen(&bad, Default::default()).is_err());
    let mut bad = good.clone();
    bad.covariate_changes[0].time = q(6., "h");
    assert!(b.simulate_regimen(&bad, Default::default()).is_err());
    let constant = language::parse(&SOURCE.replace("[kg] step", "[kg] constant"))
        .unwrap()
        .compile()
        .unwrap();
    assert!(constant
        .bind(&BTreeMap::new())
        .unwrap()
        .simulate_regimen(&good, Default::default())
        .is_err());
    assert_eq!(
        b.simulate_regimen(
            &good,
            Budgets {
                events: 2,
                ..Default::default()
            }
        )
        .unwrap_err()
        .code,
        ErrorCode::WorkBudget
    );
    assert_eq!(
        b.simulate_regimen(
            &good,
            Budgets {
                solver_callbacks: 1,
                ..Default::default()
            }
        )
        .unwrap_err()
        .code,
        ErrorCode::WorkBudget
    );
    assert_eq!(
        b.simulate_regimen(
            &good,
            Budgets {
                output_values: 1,
                ..Default::default()
            }
        )
        .unwrap_err()
        .code,
        ErrorCode::OutputBudget
    );
    assert!(b.simulate_regimen(&good, Default::default()).is_ok());
}
#[test]
fn steps_do_not_reinitialize_or_re_evaluate_initial_conditions() {
    let source = SOURCE.replace("c = initial [mg/L]", "c = start / log(ratio) [mg/L]");
    let c = language::parse(&source).unwrap().compile().unwrap_err();
    assert_eq!(c.code, "domain");
    // Initially ratio=2, later ratio=1 where this initial-only expression is undefined.
    let source = source.replace("default 70 [kg]", "default 140 [kg]");
    let c = language::parse(&source).unwrap().compile().unwrap();
    let b = c.bind(&BTreeMap::new()).unwrap();
    let mut req = request();
    req.administrations.clear();
    req.covariate_changes[0].value = q(70., "kg");
    let rows = b.simulate_regimen(&req, Default::default()).unwrap();
    let at: Vec<_> = rows.iter().filter(|r| r.time == 2.).collect();
    assert_eq!(at.len(), 2);
    assert_eq!(at[0].values[0], at[1].values[0]);
}

#[test]
fn steps_at_zero_and_end_have_distinct_sides_and_parameter_rebind_preserves_covariates() {
    let c = language::parse(SOURCE).unwrap().compile().unwrap();
    let mut b = c
        .bind_with_covariates(
            &BTreeMap::new(),
            &BTreeMap::from([("wt".into(), q(140., "kg"))]),
        )
        .unwrap();
    b.rebind(&BTreeMap::from([("start".into(), q(3., "mg/L"))]))
        .unwrap();
    assert_eq!(b.initial(), vec![6.]);
    let mut r = request();
    r.administrations.clear();
    r.covariate_changes = vec![
        CovariateChange {
            name: "wt".into(),
            time: q(0., "h"),
            value: q(70., "kg"),
        },
        CovariateChange {
            name: "wt".into(),
            time: q(5., "h"),
            value: q(35., "kg"),
        },
    ];
    let rows = b.simulate_regimen(&r, Default::default()).unwrap();
    let start: Vec<_> = rows.iter().filter(|r| r.time == 0.).collect();
    assert_eq!(start[0].values[2], 140.);
    assert_eq!(start[1].values[2], 70.);
    assert_eq!(start[0].values[0], 6.);
    assert_eq!(start[1].values[0], 6.);
    let end: Vec<_> = rows.iter().filter(|r| r.time == 5.).collect();
    assert_eq!(end[0].values[2], 70.);
    assert_eq!(end[1].values[2], 35.);
    assert_eq!(end[0].values[0], end[1].values[0]);
    assert!((end[1].values[0] - 6. * (-0.5_f64).exp()).abs() < 1e-8);
}
#[test]
fn covariate_change_during_infusion_rebinds_conversion_without_restarting_state() {
    let c = language::parse(SOURCE).unwrap().compile().unwrap();
    let b = c
        .bind(&BTreeMap::from([("start".into(), q(0., "mg/L"))]))
        .unwrap();
    let mut r = request();
    r.administrations = vec![Administration {
        target: "c".into(),
        time: q(0., "h"),
        amount: q(4., "mg"),
        delivery: Delivery::Infusion {
            span: InfusionSpan::Duration {
                duration: q(4., "h"),
            },
        },
        repeat: None,
        lag: None,
        bioavailability: 1.,
    }];
    let rows = b.simulate_regimen(&r, Default::default()).unwrap();
    let k = 0.1 * 2_f64.powf(-0.25);
    let at_two = 0.8 / 3. / 0.1 * (1. - (-0.2_f64).exp());
    let at_four = at_two * (-2. * k).exp() + 0.8 / 6. / k * (1. - (-2. * k).exp());
    for row in rows {
        let t = row.time;
        let expected = if t <= 2. {
            0.8 / 3. / 0.1 * (1. - (-0.1 * t).exp())
        } else if t <= 4. {
            at_two * (-k * (t - 2.)).exp() + 0.8 / 6. / k * (1. - (-k * (t - 2.)).exp())
        } else {
            at_four * (-k * (t - 4.)).exp()
        };
        assert!((row.values[0] - expected).abs() < 1e-8, "{row:?}");
    }
}

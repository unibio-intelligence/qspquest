use qspquest_core::{model::Quantity, regimen::*, units::Unit, ErrorCode, Event};
use qspquest_modeling::{document::CompiledDocument, Protocol};
use std::collections::BTreeMap;
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
fn model() -> std::sync::Arc<CompiledDocument> {
    CompiledDocument::from_json(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap()
}
fn dose() -> Administration {
    Administration {
        target: "central".into(),
        time: q(0.0, "h"),
        amount: q(6000.0, "ug"),
        delivery: Delivery::Bolus,
        repeat: Some(Repeat {
            interval: q(120.0, "min"),
            additional: 2,
        }),
        lag: Some(q(30.0, "min")),
        bioavailability: 0.5,
    }
}
#[test]
fn repeated_lagged_boluses_match_analytic_superposition() {
    let m = model();
    let events = m.expand_regimen(&[dose()], 10.0, 3).unwrap();
    assert_eq!(
        events.iter().map(Event::time).collect::<Vec<_>>(),
        vec![0.5, 2.5, 4.5]
    );
    assert!(events.iter().all(|e| e.amount() == Some(3.0)));
    let bound = m.bind(&BTreeMap::new()).unwrap();
    let p = Protocol {
        start: 0.0,
        end: 10.0,
        samples: vec![0.0, 0.5, 1.0, 2.5, 3.0, 4.5, 7.0, 10.0],
        events,
        rtol: 1e-9,
        atol: 1e-11,
    };
    for row in bound.simulate_outputs(&p, Default::default()).unwrap() {
        let expected: [f64; 3] = [0.5, 2.5, 4.5];
        let expected = expected
            .iter()
            .filter(|t| **t < row.time || (**t == row.time && row.side == "post"))
            .map(|t| (-0.1 * (row.time - t)).exp())
            .sum::<f64>();
        assert!(
            (row.values[0] - expected).abs() < 3e-8,
            "{} {}",
            row.time,
            row.side
        );
    }
}
#[test]
fn rate_defined_overlapping_infusions_preserve_duration_when_f_changes() {
    let m = model();
    let mut a = dose();
    a.lag = Some(q(1.0, "h"));
    a.bioavailability = 0.25;
    a.delivery = Delivery::Infusion {
        span: InfusionSpan::Rate {
            rate: q(1000.0, "ug/h"),
        },
    };
    let events = m.expand_regimen(&[a.clone()], 12.0, 3).unwrap();
    for e in &events {
        match e {
            Event::Infusion {
                amount, duration, ..
            } => {
                assert_eq!(*amount, 1.5);
                assert_eq!(*duration, 6.0);
            }
            _ => panic!("expected infusion"),
        }
    }
    let p = Protocol {
        start: 0.0,
        end: 12.0,
        samples: (0..=12).map(|x| x as f64).collect(),
        events,
        rtol: 1e-9,
        atol: 1e-11,
    };
    let bound = m.bind(&BTreeMap::new()).unwrap();
    for row in bound.simulate_outputs(&p, Default::default()).unwrap() {
        let expected = [1.0, 3.0, 5.0]
            .iter()
            .map(|start| {
                let elapsed = (row.time - start).max(0.0);
                0.25 / 0.3
                    * (1.0 - (-0.1 * elapsed.min(6.0)).exp())
                    * (-0.1 * (elapsed - 6.0).max(0.0)).exp()
            })
            .sum::<f64>();
        assert!(
            (row.values[0] - expected).abs() < 3e-8,
            "{} {} vs {}",
            row.time,
            row.values[0],
            expected
        );
    }
    a.bioavailability = 0.0;
    for e in m.expand_regimen(&[a], 12.0, 3).unwrap() {
        assert_eq!(e.amount(), Some(0.0));
        assert!(matches!(e, Event::Infusion { duration: 6.0, .. }));
    }
}
#[test]
fn invalid_expansion_and_unsupported_dynamic_fields_fail_closed() {
    let m = model();
    assert_eq!(
        m.expand_regimen(&[dose()], 10.0, 2).unwrap_err().code,
        ErrorCode::WorkBudget
    );
    for mutation in 0..6 {
        let mut a = dose();
        match mutation {
            0 => a.bioavailability = 1.1,
            1 => a.lag = Some(q(-1.0, "h")),
            2 => a.repeat.as_mut().unwrap().interval = q(0.0, "h"),
            3 => a.target = "missing".into(),
            4 => a.amount = q(1.0, "h"),
            _ => a.repeat.as_mut().unwrap().additional = u32::MAX,
        };
        assert!(m.expand_regimen(&[a], 10.0, 100).is_err());
    }
    assert!(m.expand_regimen(&[dose()], 4.0, 3).is_err());
    let mut a = dose();
    a.delivery = Delivery::Infusion {
        span: InfusionSpan::Duration {
            duration: q(10.0, "h"),
        },
    };
    assert!(m.expand_regimen(&[a], 10.0, 3).is_err());
    let mut value = serde_json::to_value(dose()).unwrap();
    value["lag"] = serde_json::json!({"expression":"parameter"});
    assert!(serde_json::from_value::<Administration>(value).is_err());
    let mut value = serde_json::to_value(dose()).unwrap();
    value["steady_state"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Administration>(value).is_err());
}
#[test]
fn distinct_close_times_remain_distinct_and_unrepresentable_repeats_fail() {
    let target = Target {
        modes: qspquest_core::model::default_delivery_modes(),
        name: "x".into(),
        state: 0,
        amount_unit: Unit::parse("mg").unwrap(),
    };
    let mut a = dose();
    a.target = "x".into();
    a.lag = None;
    a.time = q(1.0, "h");
    a.repeat = Some(Repeat {
        interval: q(1e-12, "h"),
        additional: 1,
    });
    let events = expand(
        &[a.clone()],
        &[target.clone()],
        Unit::parse("h").unwrap(),
        2.0,
        2,
    )
    .unwrap();
    assert!(events[1].time() > events[0].time());
    a.time = q(1e16, "h");
    a.repeat.as_mut().unwrap().interval = q(1.0, "h");
    assert!(expand(&[a], &[target], Unit::parse("h").unwrap(), 2e16, 2).is_err());
}

#[test]
fn unit_bearing_request_compiles_to_the_existing_protocol_and_rejects_bad_samples() {
    let m = model();
    let request = Request {
        checkpoints: vec![],
        start: None,
        observations: None,
        end: q(600.0, "min"),
        samples: vec![q(30.0, "min"), q(2.5, "h")],
        administrations: vec![dose()],
        resets: vec![],
        covariates: std::collections::BTreeMap::new(),
        covariate_changes: vec![],
        rtol: 1e-9,
        atol: Some(1e-11),
        absolute_tolerances: None,
    };
    let protocol = m.regimen_protocol(&request, Default::default()).unwrap();
    assert_eq!(protocol.end, 10.0);
    assert_eq!(protocol.samples, vec![0.5, 2.5]);
    assert_eq!(protocol.events.len(), 3);
    let bound = m.bind(&BTreeMap::new()).unwrap();
    assert!(bound
        .simulate_outputs(&protocol, Default::default())
        .is_ok());
    let mut bad = request.clone();
    bad.samples[0] = q(1.0, "mg");
    assert_eq!(
        m.regimen_protocol(&bad, Default::default())
            .unwrap_err()
            .code,
        ErrorCode::Unit
    );
    assert_eq!(
        m.regimen_protocol(
            &request,
            qspquest_modeling::Budgets {
                events: 2,
                ..Default::default()
            }
        )
        .unwrap_err()
        .code,
        ErrorCode::WorkBudget
    );
}

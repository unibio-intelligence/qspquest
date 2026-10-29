use qspquest_core::{
    model::Quantity,
    regimen::{Request, StateReset},
    ActiveInputs, Error,
};
use qspquest_modeling::{simulate, Event, Model, Protocol};
struct Decay;
impl Model for Decay {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn initial(&self) -> Vec<f64> {
        vec![0.0, 0.0]
    }
    fn dose_scale(&self, target: usize) -> Option<f64> {
        if target < 2 {
            Some(1.0)
        } else {
            None
        }
    }
    fn rhs(&self, _: f64, x: &[f64], rates: &[f64], out: &mut [f64]) -> Result<(), Error> {
        for i in 0..2 {
            out[i] = -x[i] + rates[i];
        }
        Ok(())
    }
    fn jac_mul(
        &self,
        _: f64,
        _: &[f64],
        _: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        for i in 0..2 {
            out[i] = -v[i];
        }
        Ok(())
    }
}
fn protocol(events: Vec<Event>) -> Protocol {
    Protocol {
        start: 0.0,
        end: 5.0,
        samples: vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
        events,
        rtol: 1e-10,
        atol: 1e-12,
    }
}
fn reset(value: f64, order: u32, active_inputs: ActiveInputs) -> Event {
    Event::Reset {
        time: 2.0,
        target: 0,
        value,
        order,
        active_inputs,
    }
}
#[test]
fn reset_orders_precede_simultaneous_boluses_regardless_of_record_order() {
    let mut events = vec![
        Event::Bolus {
            time: 0.0,
            target: 0,
            amount: 10.0,
        },
        Event::Bolus {
            time: 2.0,
            target: 0,
            amount: 4.0,
        },
        reset(2.0, 20, ActiveInputs::Continue),
        reset(3.0, 10, ActiveInputs::Continue),
    ];
    let mut loose = protocol(events.clone());
    loose.rtol = 1e-8;
    loose.atol = 1e-10;
    let loose_rows = simulate(&Decay, &loose).unwrap();
    let rows = simulate(&Decay, &protocol(events.clone())).unwrap();
    events.reverse();
    assert_eq!(rows, simulate(&Decay, &protocol(events)).unwrap());
    let pre = rows
        .iter()
        .find(|r| r.time == 2.0 && r.side == "pre")
        .unwrap();
    let post = rows
        .iter()
        .find(|r| r.time == 2.0 && r.side == "post")
        .unwrap();
    assert!(
        (pre.values[0] - 10.0 * (-2.0f64).exp()).abs() < 1e-8,
        "actual {} expected {}",
        pre.values[0],
        10.0 * (-2.0f64).exp()
    );
    let loose_error = (loose_rows
        .iter()
        .find(|r| r.time == 2.0 && r.side == "pre")
        .unwrap()
        .values[0]
        - 10.0 * (-2.0f64).exp())
    .abs();
    assert!((pre.values[0] - 10.0 * (-2.0f64).exp()).abs() < loose_error);
    assert_eq!(post.values[0], 6.0);
    assert!((rows.last().unwrap().values[0] - 6.0 * (-3.0f64).exp()).abs() < 1e-8);
    assert!(simulate(
        &Decay,
        &protocol(vec![
            reset(1.0, 0, ActiveInputs::Continue),
            reset(2.0, 0, ActiveInputs::Continue)
        ])
    )
    .is_err());
}
#[test]
fn stopping_target_inputs_preserves_other_targets_and_new_same_time_infusions() {
    for rule in [ActiveInputs::Continue, ActiveInputs::StopTarget] {
        let events = vec![
            Event::Infusion {
                time: 0.0,
                target: 0,
                amount: 4.0,
                duration: 4.0,
            },
            Event::Infusion {
                time: 0.0,
                target: 1,
                amount: 8.0,
                duration: 4.0,
            },
            reset(0.0, 0, rule),
            Event::Infusion {
                time: 2.0,
                target: 0,
                amount: 4.0,
                duration: 2.0,
            },
        ];
        for row in simulate(&Decay, &protocol(events)).unwrap() {
            let t = row.time;
            let expected0 = if t < 2.0 || (t == 2.0 && row.side == "pre") {
                1.0 - (-t).exp()
            } else {
                let rate = if rule == ActiveInputs::Continue {
                    3.0
                } else {
                    2.0
                };
                rate * (1.0 - (-(t.min(4.0) - 2.0)).exp()) * (-(t - 4.0).max(0.0)).exp()
            };
            let expected1 = 2.0 * (1.0 - (-t.min(4.0)).exp()) * (-(t - 4.0).max(0.0)).exp();
            assert!(
                (row.values[0] - expected0).abs() < 2e-8,
                "{rule:?} {t} {}",
                row.side
            );
            assert!((row.values[1] - expected1).abs() < 2e-8);
        }
    }
}
#[test]
fn named_reset_values_use_state_units_and_share_the_event_budget() {
    let mut document: qspquest_core::model::ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    document.states[0].dosing = None; // Resetting a state does not require a dosing mapping.
    let compiled = qspquest_modeling::document::CompiledDocument::compile(&document).unwrap();
    let q = |value, unit: &str| Quantity {
        value,
        unit: unit.into(),
    };
    let mut request = Request {
        checkpoints: vec![],
        start: None,
        covariates: Default::default(),
        covariate_changes: vec![],
        observations: None,
        end: q(5.0, "h"),
        samples: vec![],
        administrations: vec![],
        resets: vec![StateReset {
            target: "central".into(),
            time: q(120.0, "min"),
            value: q(3000.0, "ug"),
            order: 0,
            active_inputs: ActiveInputs::StopTarget,
        }],
        rtol: 1e-10,
        atol: Some(1e-12),
        absolute_tolerances: None,
    };
    let p = compiled
        .regimen_protocol(&request, Default::default())
        .unwrap();
    assert!(matches!(
        p.events[0],
        Event::Reset {
            time: 2.0,
            value: 3.0,
            ..
        }
    ));
    let bound = compiled.bind(&Default::default()).unwrap();
    let rows = bound.simulate_outputs(&p, Default::default()).unwrap();
    assert_eq!(
        rows.iter()
            .find(|r| r.time == 2.0 && r.side == "post")
            .unwrap()
            .values[0],
        1.0
    );
    assert!(compiled
        .regimen_protocol(
            &request,
            qspquest_modeling::Budgets {
                events: 0,
                ..Default::default()
            }
        )
        .is_err());
    request.resets[0].value = q(1.0, "h");
    assert!(compiled
        .regimen_protocol(&request, Default::default())
        .is_err());
    request.resets[0].value = q(1.0, "mg");
    request.resets[0].target = "missing".into();
    assert!(compiled
        .regimen_protocol(&request, Default::default())
        .is_err());
}

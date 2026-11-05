use qspquest_core::{
    dose_history::{DoseHistory, SimultaneousDose},
    regimen::{ExpandedAdministration, ObservationSide},
    ErrorCode, Event,
};
fn dose(
    target: usize,
    time: f64,
    raw: f64,
    delivered: f64,
    index: usize,
) -> ExpandedAdministration {
    ExpandedAdministration {
        administration_index: index,
        repetition_index: 0,
        scheduled_time: time,
        administered_amount: raw,
        event: Event::Bolus {
            time,
            target,
            amount: delivered,
        },
    }
}
#[test]
fn bolus_history_matches_owned_native_r_cases_with_lag_and_fraction() {
    for (fraction, lag) in [(1., 0.), (0.25, 0.), (0.25, 0.5)] {
        let mut records = vec![
            dose(0, 1. + lag, 8., 8. * fraction, 0),
            dose(1, 2., 7., 7., 1),
            dose(0, 3. + lag, 4., 4. * fraction, 2),
        ];
        records[0].scheduled_time = 1.;
        records[2].scheduled_time = 3.;
        let history = DoseHistory::new(records, SimultaneousDose::First).unwrap();
        for i in 0..9 {
            let t = i as f64 / 2.;
            let value = history.at(0, t, ObservationSide::Post).unwrap();
            if t < 1. + lag {
                assert!(value.is_none());
            } else {
                let value = value.unwrap();
                let (raw, last) = if t < 3. + lag {
                    (8., 1. + lag)
                } else {
                    (4., 3. + lag)
                };
                assert_eq!(value.administered_amount, raw);
                assert_eq!(value.delivery_time, last);
                assert_eq!(value.elapsed, t - last);
                assert_eq!(value.scheduled_time, last - lag);
            }
            let other = history.at(1, t, ObservationSide::Post).unwrap();
            if t < 2. {
                assert!(other.is_none());
            } else {
                let v = other.unwrap();
                assert_eq!(v.administered_amount, 7.);
                assert_eq!(v.elapsed, t - 2.);
            }
        }
        assert!(history
            .at(0, 1. + lag, ObservationSide::Pre)
            .unwrap()
            .is_none());
        assert_eq!(
            history
                .at(0, 3. + lag, ObservationSide::Pre)
                .unwrap()
                .unwrap()
                .administered_amount,
            8.
        );
        // Queries may move backwards after an adaptive solver's trial step.
        assert_eq!(
            history
                .at(0, 4., ObservationSide::Post)
                .unwrap()
                .unwrap()
                .administered_amount,
            4.
        );
        assert_eq!(
            history
                .at(0, 2., ObservationSide::Post)
                .unwrap()
                .unwrap()
                .administered_amount,
            8.
        );
        assert!(history.at(0, 0., ObservationSide::Post).unwrap().is_none());
    }
}
#[test]
fn simultaneous_policy_is_explicit_and_observation_sides_remain_exact() {
    let records = vec![
        dose(0, 1., 8., 0., 0),
        dose(0, 1., 4., 1., 1),
        dose(0, f64::from_bits(1f64.to_bits() + 1), 2., 2., 2),
    ];
    for (policy, expected) in [(SimultaneousDose::First, 8.), (SimultaneousDose::Last, 4.)] {
        let h = DoseHistory::new(records.clone(), policy).unwrap();
        assert!(h.at(0, 1., ObservationSide::Pre).unwrap().is_none());
        assert_eq!(
            h.at(0, 1., ObservationSide::Post)
                .unwrap()
                .unwrap()
                .administered_amount,
            expected
        );
        let next = f64::from_bits(1f64.to_bits() + 1);
        assert_eq!(
            h.at(0, next, ObservationSide::Pre)
                .unwrap()
                .unwrap()
                .administered_amount,
            expected
        );
        assert_eq!(
            h.at(0, next, ObservationSide::Post)
                .unwrap()
                .unwrap()
                .administered_amount,
            2.
        );
    }
    let reversed = DoseHistory::new(
        vec![records[1].clone(), records[0].clone()],
        SimultaneousDose::First,
    )
    .unwrap();
    assert_eq!(
        reversed
            .at(0, 1., ObservationSide::Post)
            .unwrap()
            .unwrap()
            .administered_amount,
        4.
    );
}
#[test]
fn infusion_history_starts_at_delivery_and_survives_stop() {
    let mut record = dose(0, 1., 8., 2., 0);
    record.event = Event::Infusion {
        time: 1.,
        target: 0,
        amount: 2.,
        duration: 2.,
    };
    let h = DoseHistory::new(vec![record], SimultaneousDose::First).unwrap();
    assert!(h.at(0, 1., ObservationSide::Pre).unwrap().is_none());
    for time in [1., 2., 3., 6.] {
        let value = h.at(0, time, ObservationSide::Post).unwrap().unwrap();
        assert_eq!(value.administered_amount, 8.);
        assert_eq!(value.elapsed, time - 1.);
    }
}
#[test]
fn invalid_metadata_and_undefined_queries_do_not_become_numeric_history() {
    let valid = dose(0, -1., 8., 2., 0);
    for amount in [f64::NAN, f64::INFINITY, -1.] {
        let mut record = valid.clone();
        record.administered_amount = amount;
        assert!(DoseHistory::new(vec![record], SimultaneousDose::First).is_err());
    }
    let mut bad = valid.clone();
    bad.scheduled_time = 0.;
    assert!(DoseHistory::new(vec![bad], SimultaneousDose::First).is_err());
    let mut reset = valid.clone();
    reset.event = Event::Reset {
        time: -1.,
        target: 0,
        value: 0.,
        order: 0,
        active_inputs: qspquest_core::ActiveInputs::Continue,
    };
    assert_eq!(
        DoseHistory::new(vec![reset], SimultaneousDose::First)
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    let h = DoseHistory::new(vec![valid], SimultaneousDose::First).unwrap();
    assert!(h.at(0, f64::NAN, ObservationSide::Post).is_err());
    assert!(h.at(99, 0., ObservationSide::Post).unwrap().is_none());
    let h = DoseHistory::new(vec![dose(0, -f64::MAX, 8., 2., 0)], SimultaneousDose::First).unwrap();
    assert!(h.at(0, f64::MAX, ObservationSide::Post).is_err());
}

#[test]
fn signed_zero_is_one_scientific_boundary() {
    let h = DoseHistory::new(
        vec![dose(0, 0., 8., 8., 0), dose(0, -0., 4., 4., 1)],
        SimultaneousDose::First,
    )
    .unwrap();
    assert!(h.at(0, 0., ObservationSide::Pre).unwrap().is_none());
    assert_eq!(
        h.at(0, 0., ObservationSide::Post)
            .unwrap()
            .unwrap()
            .administered_amount,
        8.
    );
}

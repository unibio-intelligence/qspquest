use qspquest_core::{model::DeliveryMode, regimen::*, units::Unit, ErrorCode, Event};

fn administrations(value: serde_json::Value) -> Vec<Administration> {
    serde_json::from_value(value).unwrap()
}
fn targets() -> Vec<Target> {
    vec![Target {
        modes: vec![DeliveryMode::Bolus, DeliveryMode::Infusion],
        name: "depot".into(),
        state: 0,
        amount_unit: Unit::parse("mg").unwrap(),
    }]
}

#[test]
fn repeated_lagged_records_keep_raw_amounts_even_at_zero_bioavailability() {
    let doses = administrations(serde_json::json!([
        {"target":"depot","time":{"value":60,"unit":"min"},
         "amount":{"value":8000,"unit":"ug"},"delivery":{"kind":"bolus"},
         "lag":{"value":30,"unit":"min"},"repeat":{"interval":{"value":2,"unit":"h"},"additional":1},
         "bioavailability":0.0},
        {"target":"depot","time":{"value":1.5,"unit":"h"},
         "amount":{"value":4,"unit":"mg"},"delivery":{"kind":"bolus"},"bioavailability":0.25}
    ]));
    let records =
        expand_administrations_in_window(&doses, &targets(), Unit::parse("h").unwrap(), 0., 4., 3)
            .unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(
        records
            .iter()
            .map(|r| (r.administration_index, r.repetition_index))
            .collect::<Vec<_>>(),
        vec![(0, 0), (1, 0), (0, 1)]
    );
    assert_eq!(
        records.iter().map(|r| r.scheduled_time).collect::<Vec<_>>(),
        vec![1., 1.5, 3.]
    );
    assert_eq!(
        records.iter().map(|r| r.event.time()).collect::<Vec<_>>(),
        vec![1.5, 1.5, 3.5]
    );
    assert_eq!(
        records
            .iter()
            .map(|r| r.administered_amount)
            .collect::<Vec<_>>(),
        vec![8., 4., 8.]
    );
    assert_eq!(
        records
            .iter()
            .map(|r| r.event.amount().unwrap())
            .collect::<Vec<_>>(),
        vec![0., 1., 0.]
    );
    let events =
        expand_in_window(&doses, &targets(), Unit::parse("h").unwrap(), 0., 4., 3).unwrap();
    for (actual, record) in events.iter().zip(&records) {
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(&record.event).unwrap()
        );
    }
}

#[test]
fn infusion_metadata_keeps_raw_rate_duration_and_absolute_schedule() {
    let doses = administrations(serde_json::json!([
        {"target":"depot","time":{"value":-1,"unit":"h"},
         "amount":{"value":12000,"unit":"ug"},
         "delivery":{"kind":"infusion","span":{"kind":"rate","rate":{"value":6,"unit":"mg/h"}}},
         "lag":{"value":30,"unit":"min"},"bioavailability":0.25}
    ]));
    let records =
        expand_administrations_in_window(&doses, &targets(), Unit::parse("h").unwrap(), -2., 3., 1)
            .unwrap();
    assert_eq!(records[0].scheduled_time, -1.);
    assert_eq!(records[0].administered_amount, 12.);
    match records[0].event {
        Event::Infusion {
            time,
            target,
            amount,
            duration,
        } => {
            assert_eq!((time, target, amount, duration), (-0.5, 0, 3., 2.));
        }
        _ => panic!("expected infusion"),
    }
}

#[test]
fn history_expansion_keeps_existing_budget_and_domain_rejections() {
    let mut doses = administrations(serde_json::json!([
        {"target":"depot","time":{"value":0,"unit":"h"},
         "amount":{"value":8,"unit":"mg"},"delivery":{"kind":"bolus"},
         "repeat":{"interval":{"value":1,"unit":"h"},"additional":1},"bioavailability":1.0}
    ]));
    let time = Unit::parse("h").unwrap();
    assert_eq!(
        expand_administrations_in_window(&doses, &targets(), time, 0., 2., 1)
            .unwrap_err()
            .code,
        ErrorCode::WorkBudget
    );
    doses[0].bioavailability = -1.;
    assert_eq!(
        expand_administrations_in_window(&doses, &targets(), time, 0., 2., 2)
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    doses[0].bioavailability = 1.;
    doses[0].amount.unit = "h".into();
    assert!(expand_administrations_in_window(&doses, &targets(), time, 0., 2., 2).is_err());
}

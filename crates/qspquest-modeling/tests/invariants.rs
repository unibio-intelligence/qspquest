use qspquest_core::{ActiveInputs, ErrorCode};
use qspquest_modeling::{language, simulate, Event, Model, Protocol};
use std::collections::BTreeMap;
const SOURCE: &str = include_str!("../../../conformance/models/synthetic-invariant-loss.qsp");
fn protocol(end: f64) -> Protocol {
    Protocol {
        start: 0.0,
        end,
        samples: vec![],
        events: vec![],
        rtol: 1e-9,
        atol: 1e-11,
    }
}
#[test]
fn accepted_states_and_initial_conditions_enforce_invariants_without_clamping() {
    let parsed = language::parse(SOURCE).unwrap();
    let compiled = parsed.compile().unwrap();
    let bound = compiled.bind(&BTreeMap::new()).unwrap();
    let valid = simulate(&bound, &protocol(0.5)).unwrap();
    assert!((valid.last().unwrap().values[0] - 0.5).abs() < 1e-8);
    let failed = simulate(&bound, &protocol(2.)).unwrap_err();
    assert_eq!(failed.code, ErrorCode::Invariant);
    assert!(failed.time.unwrap() > 1.);
    assert_eq!(
        failed.expression.as_deref(),
        Some("invariants.nonnegative.amount")
    );
    assert_eq!(
        bound.outputs(0., &[-0.01]).unwrap_err().code,
        ErrorCode::Invariant
    );
    assert_eq!(
        bound.outputs(0., &[f64::NAN]).unwrap_err().code,
        ErrorCode::Invariant
    );
    let negative = language::parse(&SOURCE.replace("amount = 1 [mg]", "amount = -1 [mg]"))
        .unwrap()
        .compile()
        .unwrap()
        .bind(&BTreeMap::new())
        .unwrap();
    let error = simulate(&negative, &protocol(0.1)).unwrap_err();
    assert_eq!(error.code, ErrorCode::Invariant);
    assert_eq!(error.time, Some(0.));
    assert_eq!(negative.initial(), vec![-1.]);
}
#[test]
fn ordered_resets_cannot_hide_invariant_failure_with_a_later_bolus() {
    let model = language::parse(SOURCE).unwrap().compile().unwrap();
    let bound = model.bind(&BTreeMap::new()).unwrap();
    let mut p = protocol(0.5);
    p.events = vec![
        Event::Reset {
            time: 0.,
            target: 0,
            value: -1.,
            order: 0,
            active_inputs: ActiveInputs::Continue,
        },
        Event::Bolus {
            time: 0.,
            target: 0,
            amount: 2.,
        },
    ];
    let error = simulate(&bound, &p).unwrap_err();
    assert_eq!(error.code, ErrorCode::Invariant);
    assert_eq!(error.time, Some(0.));
    p.events[0] = Event::Reset {
        time: 0.,
        target: 0,
        value: 0.,
        order: 0,
        active_inputs: ActiveInputs::Continue,
    };
    assert!((simulate(&bound, &p).unwrap().last().unwrap().values[0] - 1.5).abs() < 1e-8);
}
#[test]
fn invariant_declarations_roundtrip_and_reject_unknown_duplicate_or_empty_states() {
    let parsed = language::parse(SOURCE).unwrap();
    let canonical = language::format(&parsed.document).unwrap();
    assert_eq!(
        parsed.document,
        language::parse(&canonical).unwrap().document
    );
    assert_eq!(
        canonical,
        language::format(&language::parse(&canonical).unwrap().document).unwrap()
    );
    for declaration in [
        "nonnegative(missing)",
        "nonnegative(amount, amount)",
        "nonnegative()",
        "nonnegative(loss)",
        "nonnegative(amount)\n  nonnegative(amount)",
    ] {
        let text = SOURCE.replace("nonnegative(amount)", declaration);
        assert!(
            language::parse(&text).and_then(|p| p.compile()).is_err(),
            "{declaration}"
        );
    }
    let mut value = serde_json::to_value(parsed.document).unwrap();
    value["invariants"] =
        serde_json::json!([{"kind":"nonnegative","states":["amount"],"clamp":true}]);
    assert!(qspquest_modeling::document::CompiledDocument::from_json(&value.to_string()).is_err());
}

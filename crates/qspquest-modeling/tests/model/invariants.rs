use qspquest_core::{ActiveInputs, ErrorCode};
use qspquest_modeling::{language, simulate, Event, Model, Protocol};
use std::collections::BTreeMap;
const SOURCE: &str = include_str!("../../../../conformance/models/synthetic-invariant-loss.qsp");
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
#[test]
fn integrated_states_may_undershoot_by_at_most_the_absolute_tolerance() {
    let bound = language::parse(SOURCE)
        .unwrap()
        .compile()
        .unwrap()
        .bind(&BTreeMap::new())
        .unwrap();
    // Discrete and supplied states remain strict.
    assert_eq!(
        bound.check_state(1., &[-5e-12]).unwrap_err().code,
        ErrorCode::Invariant
    );
    // Integrated states are compared with the solve's absolute tolerance.
    assert!(bound
        .check_integrated_state(1., &[-5e-12], &[1e-11])
        .is_ok());
    let error = bound
        .check_integrated_state(1., &[-2e-11], &[1e-11])
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Invariant);
    assert_eq!(error.time, Some(1.));
    assert_eq!(
        error.expression.as_deref(),
        Some("invariants.nonnegative.amount")
    );
    assert!(error.message.contains("absolute tolerance"));
    assert!(bound
        .check_integrated_state(1., &[f64::NAN], &[1e-11])
        .is_err());
    assert!(bound
        .check_integrated_state(1., &[0.], &[1e-11, 1.])
        .is_err());
    let linear = qspquest_modeling::linear_pk::LinearPk::new(0., &[1.], &[], None)
        .unwrap()
        .bind(&[0.])
        .unwrap();
    for model in [&bound as &dyn Model, &linear] {
        assert!(model.check_integrated_state(1., &[0.], &[0.]).is_ok());
        for tolerance in [f64::NAN, f64::INFINITY, -1e-11] {
            assert_eq!(
                model
                    .check_integrated_state(1., &[0.], &[tolerance])
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidInput,
            );
        }
    }
}
#[test]
fn reset_values_stay_strict_within_integrated_runs() {
    let bound = language::parse(SOURCE)
        .unwrap()
        .compile()
        .unwrap()
        .bind(&BTreeMap::new())
        .unwrap();
    let mut p = protocol(0.5);
    p.events = vec![Event::Reset {
        time: 0.25,
        target: 0,
        value: -1e-13,
        order: 0,
        active_inputs: ActiveInputs::Continue,
    }];
    let error = simulate(&bound, &p).unwrap_err();
    assert_eq!(error.code, ErrorCode::Invariant);
    assert_eq!(error.time, Some(0.25));
}
#[test]
fn bdf_accepts_a_depleted_depot_within_the_absolute_tolerance() {
    // A depot absorbed at 0.9 1/h holds about 1e-26 mg after 72 h. BDF steps
    // a few 1e-12 mg below zero there, inside atol = 1e-10.
    let source = "model depleted_depot
time_unit = h
parameters
  ka = 0.9 [1/h] log
  k = 0.04 [1/h] log
states
  depot = 0 [mg] input
  central = 0 [mg]
dynamics
  d/dt(depot) = input_depot - ka * depot
  d/dt(central) = ka * depot - k * central
outputs
  amount = central [mg]
invariants
  nonnegative(depot, central)
end
";
    let bound = language::parse(source)
        .unwrap()
        .compile()
        .unwrap()
        .bind(&BTreeMap::new())
        .unwrap();
    let mut p = protocol(240.);
    p.rtol = 1e-8;
    p.atol = 1e-10;
    p.samples = vec![72., 240.];
    p.events = vec![Event::Bolus {
        time: 0.,
        target: 0,
        amount: 210.,
    }];
    let rows = simulate(&bound, &p).unwrap();
    let exact = |t: f64| 210. * 0.9 / (0.9 - 0.04) * ((-0.04 * t).exp() - (-0.9 * t).exp());
    for row in rows.iter().filter(|r| r.time == 72. || r.time == 240.) {
        assert!(row.values[0] > -1e-10, "{row:?}");
        assert!((row.values[1] - exact(row.time)).abs() < 1e-5, "{row:?}");
    }
}

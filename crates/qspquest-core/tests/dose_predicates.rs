use qspquest_core::{
    dose_history::{DoseHistory, SimultaneousDose},
    expression::Compare,
    regimen::{ExpandedAdministration, ObservationSide},
    ErrorCode, Event,
};
fn history(times: &[f64]) -> DoseHistory {
    DoseHistory::new(
        times
            .iter()
            .enumerate()
            .map(|(i, &time)| ExpandedAdministration {
                administration_index: i,
                repetition_index: 0,
                scheduled_time: time,
                administered_amount: 8.,
                event: Event::Bolus {
                    time,
                    target: 0,
                    amount: 2.,
                },
            })
            .collect(),
        SimultaneousDose::First,
    )
    .unwrap()
}
#[test]
fn superseding_doses_restart_timers_and_preserve_exact_point_sides() {
    let h = history(&[0., 1., 4.]);
    for op in [Compare::Lt, Compare::Le, Compare::Gt, Compare::Ge] {
        let p = h.predicate_schedule(0, 0., 7., 2., op, 10).unwrap();
        assert_eq!(
            p.boundaries.iter().map(|b| b.time).collect::<Vec<_>>(),
            vec![1., 3., 4., 6.]
        );
        assert_eq!(p.initial.pre, None);
        let early = matches!(op, Compare::Lt | Compare::Le);
        assert_eq!(p.initial.after, Some(early));
        assert_eq!(p.boundaries[0].after, Some(early));
        assert_eq!(p.boundaries[1].after, Some(!early));
        assert_eq!(p.boundaries[2].pre, Some(!early));
        assert_eq!(p.boundaries[2].post, Some(early));
        assert_eq!(p.boundaries[2].after, Some(early));
        let at_threshold = matches!(op, Compare::Le | Compare::Ge);
        assert_eq!(p.boundaries[1].pre, Some(at_threshold));
        assert_eq!(p.boundaries[1].post, Some(at_threshold));
        // Analytic duration in each integration phase; no discarded old timer
        // at time 2 may terminate the newer dose's phase prematurely.
        let mut time = 0.;
        let mut phase = p.initial.after.unwrap();
        let mut integral = 0.;
        for b in &p.boundaries {
            if phase {
                integral += b.time - time;
            }
            time = b.time;
            phase = b.after.unwrap();
        }
        if phase {
            integral += 7. - time;
        }
        assert_eq!(integral, if early { 5. } else { 2. });
    }
}
#[test]
fn coincident_dose_and_threshold_use_old_pre_and_new_post_history() {
    let h = history(&[0., 2.]);
    let p = h
        .predicate_schedule(0, 0., 5., 2., Compare::Ge, 10)
        .unwrap();
    assert_eq!(
        p.boundaries.iter().map(|b| b.time).collect::<Vec<_>>(),
        vec![2., 4.]
    );
    assert_eq!(
        (
            p.boundaries[0].pre,
            p.boundaries[0].post,
            p.boundaries[0].after
        ),
        (Some(true), Some(false), Some(false))
    );
    let p = h
        .predicate_schedule(0, 0., 3., 0., Compare::Gt, 10)
        .unwrap();
    assert_eq!(p.initial.post, Some(false));
    assert_eq!(p.initial.after, Some(true));
    assert_eq!(p.boundaries[0].post, Some(false));
    assert_eq!(p.boundaries[0].after, Some(true));
}
#[test]
fn decimal_point_readouts_keep_native_subtraction_semantics() {
    let h = history(&[1.]);
    let p = h
        .predicate_schedule(0, 1., 2., 0.1, Compare::Gt, 10)
        .unwrap();
    assert_eq!(p.boundaries[0].time, 1.1);
    assert_eq!(p.boundaries[0].post, Some(true)); // Native R oracle: 1.1 - 1 > .1.
    assert_eq!(
        h.predicate_at(0, 1.1, ObservationSide::Post, 0.1, Compare::Le)
            .unwrap(),
        Some(false)
    );
    let h = history(&[0.3]);
    let boundary = 0.3 + 0.6;
    let p = h
        .predicate_schedule(0, 0., 2., 0.6, Compare::Lt, 10)
        .unwrap();
    let b = p.boundaries.iter().find(|b| b.time == boundary).unwrap();
    assert_eq!(b.post, Some(true)); // Rounded subtraction remains below .6.
    assert_eq!(b.after, Some(false)); // Integration after the scheduled crossing.
}
#[test]
fn windows_missing_history_and_invalid_schedules_fail_explicitly() {
    let h = history(&[0., 1., 4.]);
    let p = h
        .predicate_schedule(0, 2., 5., 2., Compare::Gt, 10)
        .unwrap();
    assert_eq!(p.initial.post, Some(false));
    assert_eq!(p.boundaries[0].time, 3.);
    let p = h
        .predicate_schedule(99, 0., 5., 2., Compare::Gt, 0)
        .unwrap();
    assert_eq!(p.initial.after, None);
    assert!(p.boundaries.is_empty());
    assert_eq!(
        h.predicate_schedule(0, 0., 7., 2., Compare::Gt, 3)
            .unwrap_err()
            .code,
        ErrorCode::WorkBudget
    );
    for offset in [-1., f64::NAN, f64::INFINITY] {
        assert!(h
            .predicate_schedule(0, 0., 7., offset, Compare::Gt, 10)
            .is_err());
    }
    assert_eq!(
        h.predicate_schedule(0, 0., 7., 2., Compare::Eq, 10)
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    assert!(h
        .predicate_schedule(0, 7., 0., 2., Compare::Gt, 10)
        .is_err());
    let h = history(&[9007199254740992.]);
    assert!(h
        .predicate_schedule(
            0,
            9007199254740992.,
            9007199254741000.,
            0.5,
            Compare::Gt,
            10
        )
        .is_err());
}

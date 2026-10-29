use qspquest_modeling::{
    linear_pk::{LinearPk, SteadyStateTolerance},
    simulate_with_budgets, Budgets, ErrorCode, Event, Protocol,
};
fn tolerance() -> SteadyStateTolerance {
    SteadyStateTolerance {
        absolute: 1e-9,
        relative: 1e-10,
    }
}
fn cycle() -> Protocol {
    Protocol {
        start: 0.,
        end: 12.,
        samples: vec![0., 6., 12.],
        events: vec![Event::Bolus {
            time: 0.,
            target: 0,
            amount: 20.,
        }],
        rtol: 1e-10,
        atol: 1e-12,
    }
}
#[test]
fn repeated_bolus_steady_state_matches_closed_form_and_cycle_sides() {
    let m = LinearPk::new(1., &[10.], &[], None).unwrap();
    let c = cycle();
    let s = m
        .periodic_steady_state(&c, Budgets::default(), tolerance())
        .unwrap();
    let expected = 20. / libm::expm1(1.2);
    assert!((s.amounts[0] - expected).abs() < 1e-11);
    let rows = simulate_with_budgets(&m.bind(&s.amounts).unwrap(), &c, Budgets::default()).unwrap();
    assert_eq!(rows[0].side, "pre");
    assert!((rows[0].values[0] - expected).abs() < 1e-11);
    assert_eq!(rows[1].side, "post");
    assert!((rows[1].values[0] - expected - 20.).abs() < 1e-11);
    assert!(s.maximum_scaled_residual <= 1.);
    let mut shifted = c.clone();
    shifted.start = -4.;
    shifted.end = 8.;
    shifted.samples = vec![-4., 2., 8.];
    if let Event::Bolus { time, .. } = &mut shifted.events[0] {
        *time = -4.;
    }
    let other = m
        .periodic_steady_state(&shifted, Budgets::default(), tolerance())
        .unwrap();
    assert_eq!(s.amounts, other.amounts);
}
#[test]
fn periodic_infusion_matches_closed_form_and_constant_input_equilibrium() {
    let m = LinearPk::new(1., &[10.], &[], None).unwrap();
    for duration in [2., 12.] {
        let mut c = cycle();
        c.events = vec![Event::Infusion {
            time: 0.,
            target: 0,
            amount: 24.,
            duration,
        }];
        let s = m
            .periodic_steady_state(&c, Budgets::default(), tolerance())
            .unwrap();
        let expected = (24. / duration) / 0.1
            * (-libm::expm1(-0.1 * duration))
            * libm::exp(-0.1 * (12. - duration))
            / (-libm::expm1(-1.2));
        assert!((s.amounts[0] - expected).abs() < 1e-10);
    }
}
#[test]
fn multicompartment_fixed_point_matches_repeated_cycles_and_phase_shift() {
    for n in 1..=3 {
        let m = LinearPk::new(1., &[3., 7., 11.][..n], &[0.5, 0.7][..n - 1], Some(0.8)).unwrap();
        let c = cycle();
        let steady = m
            .periodic_steady_state(&c, Budgets::default(), tolerance())
            .unwrap();
        let mut state = vec![0.; m.state_count()];
        for _ in 0..250 {
            let rows =
                simulate_with_budgets(&m.bind(&state).unwrap(), &c, Budgets::default()).unwrap();
            state = rows.last().unwrap().values.clone();
        }
        for (a, b) in state.iter().zip(&steady.amounts) {
            assert!((a - b).abs() < 1e-8);
        }
        let mut shifted = c.clone();
        shifted.events = vec![Event::Bolus {
            time: 2.,
            target: 0,
            amount: 20.,
        }];
        let late = m
            .periodic_steady_state(&shifted, Budgets::default(), tolerance())
            .unwrap();
        let two_hours = m
            .advance(&late.amounts, &vec![0.; m.state_count()], 2.)
            .unwrap();
        for (a, b) in two_hours.iter().zip(&steady.amounts) {
            assert!((a - b).abs() < 1e-9);
        }
    }
}
#[test]
fn accumulating_nonunique_and_invalid_cycles_fail_without_mutation() {
    let m = LinearPk::new(0., &[1.], &[], None).unwrap();
    assert_eq!(
        m.periodic_steady_state(&cycle(), Budgets::default(), tolerance())
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    let m = LinearPk::new(1., &[2., 3.], &[0.], None).unwrap();
    assert!(m
        .periodic_steady_state(&cycle(), Budgets::default(), tolerance())
        .is_err());
    let m = LinearPk::new(1., &[2.], &[], None).unwrap();
    let mut c = cycle();
    c.events.push(Event::Bolus {
        time: 12.,
        target: 0,
        amount: 1.,
    });
    assert!(m
        .periodic_steady_state(&c, Budgets::default(), tolerance())
        .is_err());
    assert!(m
        .periodic_steady_state(
            &cycle(),
            Budgets {
                solver_callbacks: 0,
                ..Budgets::default()
            },
            tolerance()
        )
        .is_err());
    assert!(m
        .periodic_steady_state(
            &cycle(),
            Budgets::default(),
            SteadyStateTolerance {
                absolute: 0.,
                relative: 0.
            }
        )
        .is_err());
    assert!(m
        .periodic_steady_state(&cycle(), Budgets::default(), tolerance())
        .is_ok());
}

#[test]
fn resetting_cycle_has_unique_fixed_point_even_without_clearance() {
    let m = LinearPk::new(0., &[1.], &[], None).unwrap();
    let mut c = cycle();
    c.events.insert(
        0,
        Event::Reset {
            time: 0.,
            target: 0,
            value: 7.,
            order: 0,
            active_inputs: qspquest_core::ActiveInputs::Continue,
        },
    );
    let s = m
        .periodic_steady_state(&c, Budgets::default(), tolerance())
        .unwrap();
    assert!((s.amounts[0] - 27.).abs() < 1e-12);
    assert_eq!(s.residual, vec![0.]);
    assert!(m
        .periodic_steady_state(
            &c,
            Budgets {
                events: 2,
                ..Budgets::default()
            },
            tolerance()
        )
        .is_err());
    let unresolved = LinearPk::new(1e-16, &[1.], &[], None).unwrap();
    assert_eq!(
        unresolved
            .periodic_steady_state(&cycle(), Budgets::default(), tolerance())
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
}

#[test]
fn independent_scipy_periodic_reference_with_overlapping_infusions_and_resets() {
    let doc: serde_json::Value = serde_json::from_str(include_str!(
        "../../../conformance/evidence/synthetic-linear-pk-steady-reference.json"
    ))
    .unwrap();
    for (index, c) in doc["cases"].as_array().unwrap().iter().enumerate() {
        let v = |name: &str| {
            c[name]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect::<Vec<_>>()
        };
        let m = LinearPk::new(
            c["clearance"].as_f64().unwrap(),
            &v("volumes"),
            &v("exchange"),
            c["absorption"].as_f64(),
        )
        .unwrap();
        let protocol: Protocol = serde_json::from_value(c["protocol"].clone()).unwrap();
        let result = m
            .periodic_steady_state(&protocol, Budgets::default(), tolerance())
            .unwrap();
        for (actual, expected) in result.amounts.iter().zip(v("expected")) {
            assert!(
                (actual - expected).abs() <= 1e-9 + 1e-10 * expected.abs(),
                "case {index}: {actual} vs {expected}"
            );
        }
    }
}

#[test]
fn crossing_infusion_matches_phase_shift_and_preserves_rate() {
    let m = LinearPk::new(1., &[10.], &[], None).unwrap();
    let mut c = cycle();
    c.events = vec![Event::Infusion {
        time: 10.,
        target: 0,
        amount: 24.,
        duration: 4.,
    }];
    let normalized = m.periodic_cycle_protocol(&c, Budgets::default()).unwrap();
    assert_eq!(normalized.events.len(), 2);
    for event in &normalized.events {
        if let Event::Infusion {
            amount, duration, ..
        } = event
        {
            assert_eq!(*amount / *duration, 6.);
        }
    }
    let result = m
        .periodic_steady_state(&c, Budgets::default(), tolerance())
        .unwrap();
    let mut shifted = c.clone();
    shifted.events = vec![Event::Infusion {
        time: 0.,
        target: 0,
        amount: 24.,
        duration: 4.,
    }];
    let reference = m
        .periodic_steady_state(&shifted, Budgets::default(), tolerance())
        .unwrap();
    let expected = m.advance(&reference.amounts, &[6.], 2.).unwrap();
    assert!((result.amounts[0] - expected[0]).abs() < 1e-10);
    let rows = simulate_with_budgets(
        &m.bind(&result.amounts).unwrap(),
        &normalized,
        Budgets::default(),
    )
    .unwrap();
    assert!((rows.last().unwrap().values[0] - result.amounts[0]).abs() < 1e-10);
}
#[test]
fn crossing_infusion_respects_stop_resets_before_and_after_boundary() {
    let m = LinearPk::new(1., &[10.], &[], None).unwrap();
    for reset_time in [0., 1., 11.] {
        let mut c = cycle();
        c.events = vec![
            Event::Infusion {
                time: 10.,
                target: 0,
                amount: 24.,
                duration: 4.,
            },
            Event::Reset {
                time: reset_time,
                target: 0,
                value: 3.,
                order: 0,
                active_inputs: qspquest_core::ActiveInputs::StopTarget,
            },
        ];
        let normalized = m.periodic_cycle_protocol(&c, Budgets::default()).unwrap();
        let total: f64 = normalized.events.iter().filter_map(Event::amount).sum();
        let expected = if reset_time == 0. {
            12.
        } else if reset_time == 1. {
            18.
        } else {
            6.
        };
        assert_eq!(total, expected);
        let result = m
            .periodic_steady_state(&c, Budgets::default(), tolerance())
            .unwrap();
        let mut state = vec![0.];
        for _ in 0..100 {
            state =
                simulate_with_budgets(&m.bind(&state).unwrap(), &normalized, Budgets::default())
                    .unwrap()
                    .last()
                    .unwrap()
                    .values
                    .clone();
        }
        assert!((state[0] - result.amounts[0]).abs() < 1e-10);
    }
    let mut c = cycle();
    c.events = vec![Event::Infusion {
        time: 10.,
        target: 0,
        amount: 24.,
        duration: f64::INFINITY,
    }];
    assert!(m.periodic_cycle_protocol(&c, Budgets::default()).is_err());
}

#[test]
fn wrapped_cycle_matches_long_unsplit_administrations_with_resets() {
    let m = LinearPk::new(1., &[10.], &[], None).unwrap();
    for reset_time in [None, Some(0.), Some(1.), Some(11.)] {
        let mut c = cycle();
        c.events = vec![Event::Infusion {
            time: 10.,
            target: 0,
            amount: 24.,
            duration: 4.,
        }];
        if let Some(time) = reset_time {
            c.events.push(Event::Reset {
                time,
                target: 0,
                value: 3.,
                order: 0,
                active_inputs: qspquest_core::ActiveInputs::StopTarget,
            });
        }
        let steady = m
            .periodic_steady_state(&c, Budgets::default(), tolerance())
            .unwrap();
        let mut long = Protocol {
            start: 0.,
            end: 1202.,
            samples: vec![1200.],
            events: vec![],
            rtol: 1e-10,
            atol: 1e-12,
        };
        for i in 0..100 {
            long.events.push(Event::Infusion {
                time: 10. + 12. * i as f64,
                target: 0,
                amount: 24.,
                duration: 4.,
            });
        }
        if let Some(time) = reset_time {
            for i in 0..=100 {
                let t = time + 12. * i as f64;
                if t <= long.end {
                    long.events.push(Event::Reset {
                        time: t,
                        target: 0,
                        value: 3.,
                        order: 0,
                        active_inputs: qspquest_core::ActiveInputs::StopTarget,
                    });
                }
            }
        }
        let rows =
            simulate_with_budgets(&m.bind(&[0.]).unwrap(), &long, Budgets::default()).unwrap();
        let at_boundary = rows.iter().find(|r| r.time == 1200.).unwrap();
        assert!(
            (at_boundary.values[0] - steady.amounts[0]).abs() < 1e-9,
            "reset {reset_time:?}"
        );
    }
}

#[test]
fn multi_period_infusions_match_unsplit_repetition_and_constant_input() {
    let m = LinearPk::new(1., &[10.], &[], None).unwrap();
    for duration in [24., 30., 121.] {
        for reset in [None, Some(0.), Some(10.), Some(11.)] {
            let mut c = cycle();
            c.events = vec![Event::Infusion {
                time: 10.,
                target: 0,
                amount: duration,
                duration,
            }];
            if let Some(time) = reset {
                c.events.push(Event::Reset {
                    time,
                    target: 0,
                    value: 3.,
                    order: 0,
                    active_inputs: qspquest_core::ActiveInputs::StopTarget,
                });
            }
            let normalized = m.periodic_cycle_protocol(&c, Budgets::default()).unwrap();
            assert!(normalized.events.len() <= 4);
            let ss = m
                .periodic_steady_state(&c, Budgets::default(), tolerance())
                .unwrap();
            if duration == 24. && reset.is_none() {
                assert!((ss.amounts[0] - 20.).abs() < 1e-9);
            }
            let mut long = Protocol {
                start: 0.,
                end: 1200. + duration,
                samples: vec![1200.],
                events: vec![],
                rtol: 1e-10,
                atol: 1e-12,
            };
            for i in 0..100 {
                long.events.push(Event::Infusion {
                    time: 10. + 12. * i as f64,
                    target: 0,
                    amount: duration,
                    duration,
                });
            }
            if let Some(time) = reset {
                for i in 0..=100 {
                    long.events.push(Event::Reset {
                        time: time + 12. * i as f64,
                        target: 0,
                        value: 3.,
                        order: 0,
                        active_inputs: qspquest_core::ActiveInputs::StopTarget,
                    });
                }
            }
            let rows =
                simulate_with_budgets(&m.bind(&[0.]).unwrap(), &long, Budgets::default()).unwrap();
            let at = rows.iter().find(|r| r.time == 1200.).unwrap();
            assert!(
                (ss.amounts[0] - at.values[0]).abs() < 1e-8,
                "duration {duration}, reset {reset:?}"
            );
        }
    }
}

#[test]
fn long_overlap_expansion_is_bounded_and_charges_output_segments() {
    let m = LinearPk::new(1., &[10.], &[], None).unwrap();
    let mut c = cycle();
    c.events = vec![Event::Infusion {
        time: 10.,
        target: 0,
        amount: 1.2e10,
        duration: 1.2e10,
    }];
    let p = m
        .periodic_cycle_protocol(
            &c,
            Budgets {
                events: 1,
                ..Budgets::default()
            },
        )
        .unwrap();
    assert_eq!(p.events.len(), 1);
    if let Event::Infusion {
        amount, duration, ..
    } = p.events[0]
    {
        assert_eq!(amount, 1.2e10);
        assert_eq!(duration, 12.);
    } else {
        panic!("expected infusion");
    }
    c.events = vec![Event::Infusion {
        time: 10.,
        target: 0,
        amount: 30.,
        duration: 30.,
    }];
    assert!(m
        .periodic_cycle_protocol(
            &c,
            Budgets {
                events: 2,
                ..Budgets::default()
            }
        )
        .is_err());
    assert_eq!(
        m.periodic_cycle_protocol(
            &c,
            Budgets {
                events: 3,
                ..Budgets::default()
            }
        )
        .unwrap()
        .events
        .len(),
        3
    );
}

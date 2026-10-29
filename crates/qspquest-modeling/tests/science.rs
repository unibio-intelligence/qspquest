use qspquest_modeling::{simulate, Event, Model, Protocol};
#[path = "../../../conformance/tmdd.rs"]
mod tmdd;
use tmdd::Tmdd;

fn model() -> Tmdd {
    Tmdd {
        vc: 3.0,
        vp: 2.0,
        cl: 0.03,
        q: 0.1,
        kon: 10.0,
        koff: 0.1,
        kint: 0.03,
        kdeg: 0.02,
        r0: 2.0,
    }
}
fn protocol(events: Vec<Event>) -> Protocol {
    Protocol {
        start: 0.0,
        end: 10.0,
        samples: (0..=100).map(|i| i as f64 / 10.0).collect(),
        events,
        rtol: 1e-8,
        atol: 1e-10,
    }
}
fn check(actual: f64, expected: f64, tol: f64) {
    assert!(
        (actual - expected).abs() <= tol * (1.0 + expected.abs()),
        "{actual} vs {expected}"
    );
}
#[test]
fn jacobian_matches_independent_finite_difference() {
    let m = model();
    let state = vec![1.2, 0.7, 0.3, 0.9, 2.0, 1.0, 3.0];
    let x = state.clone();
    for col in 0..7 {
        let mut basis = vec![0.0; 7];
        basis[col] = 1.0;
        let v = basis;
        let mut exact = vec![0.0; 7];
        m.jac_mul(0.0, &x, &[0.0; 7], &v, &mut exact).unwrap();
        let mut plus = state.clone();
        plus[col] += 1e-5;
        let mut minus = state.clone();
        minus[col] -= 1e-5;
        let mut fp = exact.clone();
        let mut fm = exact.clone();
        m.rhs(0.0, &plus, &[0.0; 7], &mut fp).unwrap();
        m.rhs(0.0, &minus, &[0.0; 7], &mut fm).unwrap();
        for row in 0..7 {
            check(exact[row], (fp[row] - fm[row]) / 2e-5, 1e-8);
        }
    }
}
#[test]
fn linear_bolus_matches_exact_solution_and_mass_balance() {
    let mut m = model();
    m.kon = 0.0;
    m.q = 0.0;
    let p = protocol(vec![Event::Bolus {
        time: 0.0,
        target: 0,
        amount: 30.0,
    }]);
    for row in simulate(&m, &p).unwrap() {
        let delivered = if row.time == 0.0 && row.side == "pre" {
            0.0
        } else {
            30.0
        };
        let expected = delivered / m.vc * (-m.cl / m.vc * row.time).exp();
        check(row.values[0], expected, 2e-7);
        check(
            m.vc * (row.values[0] + row.values[3]) + m.vp * row.values[1] + row.values[4],
            delivered,
            2e-7,
        );
    }
}
#[test]
fn simultaneous_boluses_have_one_pre_post_pair_and_are_order_independent() {
    let m = model();
    let mut p = protocol(vec![
        Event::Bolus {
            time: 2.0,
            target: 0,
            amount: 3.0,
        },
        Event::Bolus {
            time: 2.0,
            target: 0,
            amount: 6.0,
        },
    ]);
    let a = simulate(&m, &p).unwrap();
    p.events.reverse();
    let b = simulate(&m, &p).unwrap();
    assert_eq!(a, b);
    let rows: Vec<_> = a.iter().filter(|r| r.time == 2.0).collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].side, "pre");
    assert_eq!(rows[1].side, "post");
    check(rows[1].values[0] - rows[0].values[0], 3.0, 1e-12);
}
#[test]
fn overlapping_and_short_infusions_preserve_delivered_drug() {
    let mut m = model();
    m.cl = 0.0;
    let p = protocol(vec![
        Event::Infusion {
            time: 0.1,
            target: 0,
            amount: 3.0,
            duration: 0.0001,
        },
        Event::Infusion {
            time: 0.1,
            target: 1,
            amount: 4.0,
            duration: 2.0,
        },
        Event::Infusion {
            time: 1.0,
            target: 0,
            amount: 5.0,
            duration: 2.0,
        },
    ]);
    for row in simulate(&m, &p).unwrap() {
        let amount = 3.0 * ((row.time - 0.1) / 0.0001).clamp(0.0, 1.0)
            + 4.0 * ((row.time - 0.1) / 2.0).clamp(0.0, 1.0)
            + 5.0 * ((row.time - 1.0) / 2.0).clamp(0.0, 1.0);
        check(
            m.vc * (row.values[0] + row.values[3]) + m.vp * row.values[1] + row.values[4],
            amount,
            2e-6,
        );
        check(
            row.values[2] + row.values[3] + row.values[5] - row.values[6],
            m.r0,
            2e-6,
        );
        assert!(row.values[..4].iter().all(|v| *v >= -2e-6));
    }
}
#[test]
fn invalid_inputs_fail_before_solving() {
    let mut m = model();
    let mut p = protocol(vec![]);
    m.kon = -1.0;
    assert!(simulate(&m, &p).is_err());
    m.kon = 10.0;
    p.events = vec![Event::Bolus {
        time: 0.0,
        target: 2,
        amount: 1.0,
    }];
    assert!(simulate(&m, &p).is_err());
    p.events = vec![Event::Infusion {
        time: 0.0,
        target: 0,
        amount: 1.0,
        duration: 0.0,
    }];
    assert!(simulate(&m, &p).is_err());
    p.events = vec![Event::Bolus {
        time: f64::NAN,
        target: 0,
        amount: 1.0,
    }];
    assert!(simulate(&m, &p).is_err());
    p.events = vec![];
    p.samples.push(f64::INFINITY);
    assert!(simulate(&m, &p).is_err());
}
#[test]
fn close_but_distinct_events_are_not_merged_and_final_bolus_is_applied() {
    let m = model();
    let p = protocol(vec![
        Event::Bolus {
            time: 1.0,
            target: 0,
            amount: 1.0,
        },
        Event::Bolus {
            time: 1.0 + 1e-7,
            target: 0,
            amount: 2.0,
        },
        Event::Bolus {
            time: 10.0,
            target: 0,
            amount: 3.0,
        },
    ]);
    let rows = simulate(&m, &p).unwrap();
    assert_eq!(rows.iter().filter(|r| r.side == "pre").count(), 3);
    let pre = &rows[rows.len() - 2];
    let post = &rows[rows.len() - 1];
    assert_eq!(pre.time, 10.0);
    check(post.values[0] - pre.values[0], 1.0, 1e-12);
}

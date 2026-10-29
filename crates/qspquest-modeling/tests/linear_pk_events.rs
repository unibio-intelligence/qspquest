use qspquest_core::ActiveInputs;
use qspquest_modeling::{
    linear_pk::{BoundLinearPk, LinearPk},
    simulate_with_budgets, Budgets, Error, ErrorCode, Event, Model, Protocol,
};
struct OdeOnly(BoundLinearPk);
impl Model for OdeOnly {
    fn validate(&self) -> Result<(), Error> {
        self.0.validate()
    }
    fn initial(&self) -> Vec<f64> {
        self.0.initial()
    }
    fn dose_scale(&self, t: usize) -> Option<f64> {
        self.0.dose_scale(t)
    }
    fn check_state(&self, t: f64, x: &[f64]) -> Result<(), Error> {
        self.0.check_state(t, x)
    }
    fn rhs(&self, t: f64, x: &[f64], r: &[f64], o: &mut [f64]) -> Result<(), Error> {
        self.0.rhs(t, x, r, o)
    }
    fn jac_mul(&self, t: f64, x: &[f64], r: &[f64], v: &[f64], o: &mut [f64]) -> Result<(), Error> {
        self.0.jac_mul(t, x, r, v, o)
    }
}
fn protocol() -> Protocol {
    Protocol {
        start: 0.,
        end: 5.,
        samples: vec![0., 1., 2., 3., 4., 5.],
        rtol: 1e-10,
        atol: 1e-12,
        events: vec![
            Event::Bolus {
                time: 0.,
                target: 0,
                amount: 2.,
            },
            Event::Infusion {
                time: 0.,
                target: 0,
                amount: 40.,
                duration: 4.,
            },
            Event::Bolus {
                time: 1.,
                target: 0,
                amount: 5.,
            },
            Event::Reset {
                time: 2.,
                target: 0,
                value: 7.,
                order: 0,
                active_inputs: ActiveInputs::StopTarget,
            },
            Event::Bolus {
                time: 2.,
                target: 0,
                amount: 5.,
            },
            Event::Infusion {
                time: 2.,
                target: 0,
                amount: 6.,
                duration: 2.,
            },
        ],
    }
}
#[test]
fn shared_event_order_and_reset_infusion_policy_match_closed_form() {
    let model = LinearPk::new(0., &[1.], &[], None)
        .unwrap()
        .bind(&[0.])
        .unwrap();
    for policy in [ActiveInputs::StopTarget, ActiveInputs::Continue] {
        let mut p = protocol();
        if let Event::Reset { active_inputs, .. } = &mut p.events[3] {
            *active_inputs = policy;
        }
        let rows = simulate_with_budgets(&model, &p, Budgets::default()).unwrap();
        for row in rows {
            let expected = match (row.time as u32, row.side.as_str()) {
                (0, "pre") => 0.,
                (0, _) => 2.,
                (1, "pre") => 12.,
                (1, _) => 17.,
                (2, "pre") => 27.,
                (2, _) => 12.,
                (3, _) => {
                    if policy == ActiveInputs::StopTarget {
                        15.
                    } else {
                        25.
                    }
                }
                (4, _) | (5, _) => {
                    if policy == ActiveInputs::StopTarget {
                        18.
                    } else {
                        38.
                    }
                }
                _ => panic!("unexpected sample"),
            };
            assert!(
                (row.values[0] - expected).abs() < 1e-10,
                "{} {} {:?}",
                row.time,
                row.side,
                row.values
            );
        }
    }
}
#[test]
fn analytic_and_bdf_share_event_rows_for_all_compartment_counts() {
    for count in 1..=3 {
        for depot in [None, Some(0.4)] {
            let kernel = LinearPk::new(
                0.7,
                &[3., 9., 18.][..count],
                &[0.5, 0.2][..count - 1],
                depot,
            )
            .unwrap();
            let n = kernel.state_count();
            let model = kernel.bind(&vec![1.; n]).unwrap();
            let mut p = protocol();
            p.events.push(Event::Infusion {
                time: 1.5,
                target: n - 1,
                amount: 3.,
                duration: 2.,
            });
            let a = simulate_with_budgets(&model, &p, Budgets::default()).unwrap();
            let b = simulate_with_budgets(&OdeOnly(model.clone()), &p, Budgets::default()).unwrap();
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                assert_eq!((a.time, &a.side), (b.time, &b.side));
                for (x, y) in a.values.iter().zip(b.values) {
                    assert!((x - y).abs() < 1e-7, "{x} vs {y}");
                }
            }
            let mut dense = p.clone();
            dense.samples.extend([0.25, 0.75, 1.25, 2.25, 3.25]);
            let dense = simulate_with_budgets(&model, &dense, Budgets::default()).unwrap();
            for row in a {
                assert_eq!(
                    row,
                    *dense
                        .iter()
                        .find(|r| r.time == row.time && r.side == row.side)
                        .unwrap()
                );
            }
        }
    }
}
#[test]
fn analytic_work_and_output_budgets_reject_without_partial_success() {
    let model = LinearPk::new(1., &[2.], &[], None)
        .unwrap()
        .bind(&[1.])
        .unwrap();
    let p = protocol();
    let baseline = simulate_with_budgets(&model, &p, Budgets::default()).unwrap();
    for (budgets, expected) in [
        (
            Budgets {
                solver_callbacks: 0,
                ..Budgets::default()
            },
            ErrorCode::WorkBudget,
        ),
        (
            Budgets {
                solver_callbacks: 2,
                ..Budgets::default()
            },
            ErrorCode::WorkBudget,
        ),
        (
            Budgets {
                events: 0,
                ..Budgets::default()
            },
            ErrorCode::WorkBudget,
        ),
        (
            Budgets {
                output_values: 1,
                ..Budgets::default()
            },
            ErrorCode::OutputBudget,
        ),
    ] {
        assert_eq!(
            simulate_with_budgets(&model, &p, budgets).unwrap_err().code,
            expected
        );
    }
    assert_eq!(
        simulate_with_budgets(&model, &p, Budgets::default()).unwrap(),
        baseline
    );
    let mut bad = p;
    bad.events.push(Event::Reset {
        time: 3.,
        target: 0,
        value: -1.,
        order: 1,
        active_inputs: ActiveInputs::Continue,
    });
    assert_eq!(
        simulate_with_budgets(&model, &bad, Budgets::default())
            .unwrap_err()
            .code,
        ErrorCode::Invariant
    );
}

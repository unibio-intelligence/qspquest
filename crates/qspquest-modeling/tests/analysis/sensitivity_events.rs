//! Independent hand-derived augmented model qualifies shared event-driver hooks.
use qspquest_modeling::{simulate_with_budgets, Budgets, Error, ErrorCode, Event, Model, Protocol};
#[derive(Clone)]
struct Pk {
    cl: f64,
    v: f64,
    f: f64,
    a0: f64,
    fail: bool,
}
impl Model for Pk {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn initial(&self) -> Vec<f64> {
        vec![self.a0 / self.v, -self.a0 / self.v.powi(2), 0., 0.]
    }
    fn dose_scale(&self, i: usize) -> Option<f64> {
        (i == 0).then_some(self.f / self.v)
    }
    fn apply_bolus(&self, _: f64, target: usize, dose: f64, x: &mut [f64]) -> Result<(), Error> {
        if self.fail {
            return Err(Error::new(ErrorCode::Domain, "event derivative failed"));
        }
        if target != 0 {
            return Err(Error::new(
                ErrorCode::InvalidInput,
                "primal target required",
            ));
        }
        x[0] += dose * self.f / self.v;
        x[1] -= dose * self.f / self.v.powi(2);
        x[3] += dose / self.v;
        Ok(())
    }
    fn apply_reset(&self, _: f64, target: usize, value: f64, x: &mut [f64]) -> Result<(), Error> {
        if target != 0 {
            return Err(Error::new(
                ErrorCode::InvalidInput,
                "primal target required",
            ));
        }
        x[0] = value;
        x[1..].fill(0.);
        Ok(())
    }
    fn accumulate_infusion_rate(
        &self,
        _: f64,
        target: usize,
        rate: f64,
        r: &mut [f64],
    ) -> Result<(), Error> {
        if target != 0 {
            return Err(Error::new(
                ErrorCode::InvalidInput,
                "primal target required",
            ));
        }
        r[0] += rate * self.f / self.v;
        r[1] -= rate * self.f / self.v.powi(2);
        r[3] += rate / self.v;
        Ok(())
    }
    fn rhs(&self, _: f64, x: &[f64], r: &[f64], out: &mut [f64]) -> Result<(), Error> {
        let k = self.cl / self.v;
        out[0] = -k * x[0] + r[0];
        out[1] = -k * x[1] + self.cl / self.v.powi(2) * x[0] + r[1];
        out[2] = -k * x[2] - x[0] / self.v + r[2];
        out[3] = -k * x[3] + r[3];
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
        let k = self.cl / self.v;
        out[0] = -k * v[0];
        out[1] = -k * v[1] + self.cl / self.v.powi(2) * v[0];
        out[2] = -k * v[2] - v[0] / self.v;
        out[3] = -k * v[3];
        Ok(())
    }
}
fn model() -> Pk {
    Pk {
        cl: 0.7,
        v: 3.,
        f: 0.6,
        a0: 9.,
        fail: false,
    }
}
fn protocol() -> Protocol {
    Protocol {
        start: 0.,
        end: 8.,
        samples: vec![0.5, 1., 3., 5., 7.],
        events: vec![
            Event::Bolus {
                time: 0.,
                target: 0,
                amount: 5.,
            },
            Event::Bolus {
                time: 2.,
                target: 0,
                amount: 7.,
            },
            Event::Bolus {
                time: 4.,
                target: 0,
                amount: 3.,
            },
        ],
        rtol: 1e-10,
        atol: 1e-12,
    }
}
#[test]
fn initial_and_repeated_bolus_derivatives_match_closed_forms_on_both_sides() {
    let m = model();
    let p = protocol();
    let rows = simulate_with_budgets(&m, &p, Budgets::default()).unwrap();
    for row in rows {
        let mut expected = [0.; 4];
        let mut add = |amount: f64, age: f64, bio: f64, is_dose: bool| {
            let e = libm::exp(-m.cl * age / m.v);
            expected[0] += amount * bio / m.v * e;
            expected[1] += amount * bio * e * (-1. / m.v.powi(2) + m.cl * age / m.v.powi(3));
            expected[2] -= amount * bio * age / m.v.powi(2) * e;
            if is_dose {
                expected[3] += amount / m.v * e;
            }
        };
        add(m.a0, row.time, 1., false);
        for event in &p.events {
            if let Event::Bolus { time, amount, .. } = event {
                if *time < row.time || (*time == row.time && row.side == "post") {
                    add(*amount, row.time - time, m.f, true);
                }
            }
        }
        for (a, b) in row.values.iter().zip(expected) {
            assert!(
                (a - b).abs() < 1e-8,
                "{} {}: {a} vs {b}",
                row.time,
                row.side
            );
        }
    }
}
#[test]
fn infusion_and_reset_sensitivities_match_full_trajectory_finite_differences() {
    let m = model();
    let mut p = protocol();
    p.events.push(Event::Infusion {
        time: 1.,
        target: 0,
        amount: 12.,
        duration: 4.,
    });
    p.events.push(Event::Reset {
        time: 3.,
        target: 0,
        value: 2.,
        order: 0,
        active_inputs: qspquest_core::ActiveInputs::StopTarget,
    });
    let rows = simulate_with_budgets(&m, &p, Budgets::default()).unwrap();
    for (column, delta) in [(1, 1e-4), (2, 1e-4), (3, 1e-4)] {
        let mut plus = m.clone();
        let mut minus = m.clone();
        match column {
            1 => {
                plus.v += delta;
                minus.v -= delta;
            }
            2 => {
                plus.cl += delta;
                minus.cl -= delta;
            }
            _ => {
                plus.f += delta;
                minus.f -= delta;
            }
        }
        let a = simulate_with_budgets(&plus, &p, Budgets::default()).unwrap();
        let b = simulate_with_budgets(&minus, &p, Budgets::default()).unwrap();
        for ((row, a), b) in rows.iter().zip(a).zip(b) {
            assert_eq!((row.time, &row.side), (a.time, &a.side));
            let fd = (a.values[0] - b.values[0]) / (2. * delta);
            assert!(
                (row.values[column] - fd).abs() < 2e-6,
                "sensitivity {column} at {}",
                row.time
            );
        }
    }
    let post = rows
        .iter()
        .find(|r| r.time == 3. && r.side == "post")
        .unwrap();
    assert_eq!(post.values, vec![2., 0., 0., 0.]);
}
#[test]
fn zero_bioavailability_preserves_dose_and_infusion_derivatives() {
    let mut m = model();
    m.f = 0.;
    m.a0 = 0.;
    let mut p = protocol();
    p.events = vec![
        Event::Bolus {
            time: 0.,
            target: 0,
            amount: 5.,
        },
        Event::Infusion {
            time: 0.,
            target: 0,
            amount: 8.,
            duration: 8.,
        },
    ];
    let rows = simulate_with_budgets(&m, &p, Budgets::default()).unwrap();
    for row in rows {
        assert_eq!(row.values[0], 0.);
        let e = libm::exp(-m.cl * row.time / m.v);
        let expected = if row.time == 0. && row.side == "pre" {
            0.
        } else {
            5. / m.v * e + (1. - e) / m.cl
        };
        assert!((row.values[3] - expected).abs() < 1e-8);
    }
}

#[test]
fn event_derivative_failure_returns_no_trajectory_and_preserves_recovery() {
    let mut m = model();
    m.fail = true;
    let error = simulate_with_budgets(&m, &protocol(), Budgets::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::Domain);
    assert_eq!(error.message, "event derivative failed");
    m.fail = false;
    assert!(simulate_with_budgets(&m, &protocol(), Budgets::default()).is_ok());
}

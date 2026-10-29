use qspquest_modeling::steady_state::{iterate_periodic_cycle, CycleIterationOptions};
use qspquest_modeling::{Budgets, Error, ErrorCode, Event, Model, Protocol};
struct Saturable {
    initial: f64,
}
impl Model for Saturable {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn initial(&self) -> Vec<f64> {
        vec![self.initial]
    }
    fn dose_scale(&self, _: usize) -> Option<f64> {
        Some(1.)
    }
    fn rhs(&self, _: f64, x: &[f64], rate: &[f64], out: &mut [f64]) -> Result<(), Error> {
        out[0] = rate[0] - 10. * x[0] / (2. + x[0]);
        Ok(())
    }
    fn jac_mul(
        &self,
        _: f64,
        x: &[f64],
        _: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        out[0] = -20. / (2. + x[0]).powi(2) * v[0];
        Ok(())
    }
}
fn protocol(dose: f64) -> Protocol {
    Protocol {
        start: 0.,
        end: 1.,
        samples: vec![],
        events: vec![Event::Bolus {
            time: 0.,
            target: 0,
            amount: dose,
        }],
        rtol: 1e-10,
        atol: 1e-12,
    }
}
fn options() -> CycleIterationOptions {
    CycleIterationOptions {
        absolute_tolerances: vec![1e-8],
        integration_absolute_tolerances: None,
        relative_tolerance: 1e-8,
        maximum_cycles: 100,
    }
}
#[test]
fn saturable_bolus_fixed_point_matches_integrated_closed_form_from_two_seeds() {
    // Integral: dose + Km*ln((trough+dose)/trough) = Vmax*period.
    let expected = 5. / libm::expm1(2.5);
    for seed in [0., 50.] {
        let result = iterate_periodic_cycle(
            |s| Ok(Saturable { initial: s[0] }),
            &[seed],
            &protocol(5.),
            &options(),
            Budgets::default(),
        )
        .unwrap();
        assert!((result.initial_states[0] - expected).abs() < 2e-7);
        assert!(result.cycles_completed >= 2 && result.maximum_scaled_residual <= 1.);
    }
}
#[test]
fn accumulation_and_invalid_binding_fail_without_partial_success() {
    let error = iterate_periodic_cycle(
        |s| Ok(Saturable { initial: s[0] }),
        &[0.],
        &protocol(12.),
        &options(),
        Budgets::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Solver);
    let error = iterate_periodic_cycle(
        |_| Ok(Saturable { initial: 0. }),
        &[1.],
        &protocol(5.),
        &options(),
        Budgets::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(iterate_periodic_cycle(
        |s| Ok(Saturable { initial: s[0] }),
        &[0.],
        &protocol(5.),
        &options(),
        Budgets {
            solver_callbacks: 0,
            ..Budgets::default()
        }
    )
    .is_err());
    assert!(iterate_periodic_cycle(
        |s| Ok(Saturable { initial: s[0] }),
        &[0.],
        &protocol(5.),
        &options(),
        Budgets::default()
    )
    .is_ok());
}

#[test]
fn overlapping_infusions_reach_saturable_constant_input_equilibrium() {
    let mut cycle = protocol(0.);
    cycle.events = vec![Event::Infusion {
        time: 0.75,
        target: 0,
        amount: 4.,
        duration: 2.,
    }];
    let result = iterate_periodic_cycle(
        |s| Ok(Saturable { initial: s[0] }),
        &[0.],
        &cycle,
        &options(),
        Budgets::default(),
    )
    .unwrap();
    // Repeated 4-unit doses each unit time give rate 4: x=Km*r/(Vmax-r).
    assert!((result.initial_states[0] - 4. / 3.).abs() < 2e-7);
}
#[test]
fn reset_cycle_verifies_twice_and_rejects_invalid_tolerance_vectors() {
    let mut cycle = protocol(5.);
    cycle.events.push(Event::Reset {
        time: 0.,
        target: 0,
        value: 3.,
        order: 0,
        active_inputs: qspquest_core::ActiveInputs::StopTarget,
    });
    let result = iterate_periodic_cycle(
        |s| Ok(Saturable { initial: s[0] }),
        &[30.],
        &cycle,
        &options(),
        Budgets::default(),
    )
    .unwrap();
    assert_eq!(result.cycles_completed, 3);
    // Integrating x'=-10*x/(2+x) for one time unit from post-dose x=8.
    let (mut low, mut high) = (0_f64, 8_f64);
    for _ in 0..100 {
        let middle = (low + high) / 2.;
        let elapsed = 8. - middle + 2. * libm::log(8. / middle);
        if elapsed > 10. {
            low = middle;
        } else {
            high = middle;
        }
    }
    assert!((result.initial_states[0] - (low + high) / 2.).abs() < 2e-7);
    for absolute in [vec![], vec![-1.], vec![1., 1.]] {
        let mut invalid = options();
        invalid.absolute_tolerances = absolute;
        assert!(iterate_periodic_cycle(
            |s| Ok(Saturable { initial: s[0] }),
            &[0.],
            &cycle,
            &invalid,
            Budgets::default()
        )
        .is_err());
    }
}

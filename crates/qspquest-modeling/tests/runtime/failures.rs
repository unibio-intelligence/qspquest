use qspquest_modeling::{simulate_with_budgets, Budgets, Error, ErrorCode, Model, Protocol};
struct Decay {
    fail_after: f64,
}
impl Model for Decay {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn initial(&self) -> Vec<f64> {
        vec![1.0]
    }
    fn dose_scale(&self, _: usize) -> Option<f64> {
        None
    }
    fn rhs(&self, t: f64, x: &[f64], _: &[f64], out: &mut [f64]) -> Result<(), Error> {
        if t > self.fail_after {
            let mut error = Error::new(ErrorCode::Domain, "log argument must be positive");
            error.expression = Some("dynamics.central.log".into());
            return Err(error);
        }
        out[0] = -x[0];
        Ok(())
    }
    fn jac_mul(
        &self,
        _: f64,
        _: &[f64],
        _rates: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        out[0] = -v[0];
        Ok(())
    }
}
fn protocol() -> Protocol {
    Protocol {
        start: 0.0,
        end: 10.0,
        samples: vec![0.1, 1.0, 10.0],
        events: vec![],
        rtol: 1e-8,
        atol: 1e-10,
    }
}
#[test]
fn preserves_original_domain_error_during_solver_work() {
    let error = simulate_with_budgets(&Decay { fail_after: 0.5 }, &protocol(), Budgets::default())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Domain);
    assert_eq!(error.expression.as_deref(), Some("dynamics.central.log"));
    assert!(error.time.unwrap() > 0.5);
}
#[test]
fn work_exhaustion_returns_typed_failure_instead_of_partial_results() {
    for limit in [0, 1, 10, 50] {
        let error = simulate_with_budgets(
            &Decay {
                fail_after: f64::INFINITY,
            },
            &protocol(),
            Budgets {
                solver_callbacks: limit,
                ..Budgets::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::WorkBudget);
    }
}
#[test]
fn rejects_excess_output_before_integration() {
    let error = simulate_with_budgets(
        &Decay {
            fail_after: f64::INFINITY,
        },
        &protocol(),
        Budgets {
            output_values: 1,
            ..Budgets::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::OutputBudget);
}

use qspquest_core::{model::Quantity, run::RunRequest};
use qspquest_modeling::{language, simulate, Event, Protocol};
use std::collections::BTreeMap;
const SOURCE: &str = include_str!("../../../conformance/models/synthetic-start-times.qsp");
fn request() -> RunRequest {
    serde_json::from_value(serde_json::json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf", "budgets":{"solver_callbacks":1000000,"events":100,"output_values":1000},"regimen":{"start":{"value":120,"unit":"min"},"end":{"value":4,"unit":"h"},"samples":[{"value":2,"unit":"h"},{"value":3,"unit":"h"},{"value":4,"unit":"h"}],"administrations":[],"rtol":1e-9,"atol":1e-11}})).unwrap()
}
#[test]
fn absolute_time_and_initial_state_begin_at_the_declared_start() {
    let model = language::parse(SOURCE).unwrap().compile().unwrap();
    let req = request();
    let result = model.execute(&req).unwrap();
    for (i, t) in result.times.iter().enumerate() {
        assert!(*t >= 2.);
        assert!((result.outputs[0].values[i] - (5. + 0.5 * (t * t - 4.))).abs() < 1e-7);
        assert_eq!(result.outputs[1].values[i], *t);
    }
    let bound = model.bind(&BTreeMap::new()).unwrap();
    let protocol = Protocol {
        start: -2.,
        end: -1.,
        samples: vec![-2., -1.],
        events: vec![Event::Bolus {
            time: -2.,
            target: 0,
            amount: 3.,
        }],
        rtol: 1e-9,
        atol: 1e-11,
    };
    let rows = simulate(&bound, &protocol).unwrap();
    assert_eq!(rows[0].time, -2.);
    assert_eq!(rows[0].side, "pre");
    assert_eq!(rows[0].values[0], 5.);
    assert_eq!(rows[1].values[0], 8.);
    assert!((rows.last().unwrap().values[0] - 6.5).abs() < 1e-7);
}
#[test]
fn invalid_windows_samples_and_events_are_rejected_without_warmup() {
    let model = language::parse(SOURCE).unwrap().compile().unwrap();
    for start in [4., 5., f64::NAN] {
        let mut req = request();
        req.regimen.start = Some(Quantity {
            value: start,
            unit: "h".into(),
        });
        assert!(model.execute(&req).is_err());
    }
    let mut req = request();
    req.regimen.samples[0].value = 1.;
    assert!(model.execute(&req).is_err());
    let mut req = request();
    req.regimen.start.as_mut().unwrap().unit = "mg".into();
    assert!(model.execute(&req).is_err());
    let mut req = request();
    req.regimen.administrations=serde_json::from_value(serde_json::json!([{"target":"amount","time":{"value":1,"unit":"h"},"amount":{"value":3,"unit":"mg"},"delivery":{"kind":"bolus"},"bioavailability":1}])).unwrap();
    assert!(model.execute(&req).is_err());
    req.regimen.administrations[0].time.value = 2.;
    let result = model.execute(&req).unwrap();
    assert_eq!(result.outputs[0].values[0], 5.);
    assert_eq!(result.outputs[0].values[1], 8.);
}

#[test]
fn checkpoints_preserve_trajectory_and_validate_units_window_and_budget() {
    let model = language::parse(SOURCE).unwrap().compile().unwrap();
    let mut req = request();
    let original_hash = model.execute(&req).unwrap().identity.run_request_hash;
    req.regimen.checkpoints = vec![
        Quantity {
            value: 150.,
            unit: "min".into(),
        },
        Quantity {
            value: 3.,
            unit: "h".into(),
        },
    ];
    let result = model.execute(&req).unwrap();
    assert_ne!(result.identity.run_request_hash, original_hash);
    for (i, t) in result.times.iter().enumerate() {
        assert!((result.outputs[0].values[i] - (5. + 0.5 * (t * t - 4.))).abs() < 1e-7);
    }
    for (value, unit) in [(1., "h"), (5., "h"), (3., "mg"), (f64::NAN, "h")] {
        req.regimen.checkpoints = vec![Quantity {
            value,
            unit: unit.into(),
        }];
        assert!(model.execute(&req).is_err());
    }
    req.regimen.checkpoints = vec![
        Quantity {
            value: 3.,
            unit: "h".into()
        };
        101
    ];
    assert!(model.execute(&req).is_err());
}

struct FastRelaxation {
    origin: f64,
}
impl qspquest_modeling::Model for FastRelaxation {
    fn validate(&self) -> Result<(), qspquest_core::Error> {
        Ok(())
    }
    fn initial(&self) -> Vec<f64> {
        vec![0.0]
    }
    fn dose_scale(&self, _: usize) -> Option<f64> {
        None
    }
    fn rhs(
        &self,
        time: f64,
        x: &[f64],
        _: &[f64],
        out: &mut [f64],
    ) -> Result<(), qspquest_core::Error> {
        assert!(
            time >= self.origin && time <= self.origin + 4.0,
            "absolute callback time {time}"
        );
        out[0] = 1e4 * (1.0 - x[0]);
        Ok(())
    }
    fn jac_mul(
        &self,
        time: f64,
        _: &[f64],
        _: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), qspquest_core::Error> {
        assert!(
            time >= self.origin && time <= self.origin + 4.0,
            "absolute Jacobian time {time}"
        );
        out[0] = -1e4 * v[0];
        Ok(())
    }
    fn check_state(&self, time: f64, _: &[f64]) -> Result<(), qspquest_core::Error> {
        assert!(
            time >= self.origin && time <= self.origin + 4.0,
            "absolute state-check time {time}"
        );
        Ok(())
    }
}

#[test]
fn tiny_initial_steps_are_independent_of_absolute_clock_origin() {
    for origin in [0.0, -1000.0, -1e16, 1e16] {
        let model = FastRelaxation { origin };
        let protocol = Protocol {
            start: origin,
            end: origin + 4.0,
            samples: vec![origin, origin + 2.0, origin + 4.0],
            events: vec![],
            rtol: 1e-8,
            atol: 1e-12,
        };
        let rows = simulate(&model, &protocol).unwrap_or_else(|e| panic!("origin {origin}: {e}"));
        assert_eq!(
            rows.iter().map(|r| r.time).collect::<Vec<_>>(),
            protocol.samples
        );
        assert_eq!(rows[0].values[0], 0.0);
        for row in &rows[1..] {
            assert!((row.values[0] - 1.0).abs() < 1e-8);
        }
    }
}

#[test]
fn autonomous_rodas5p_preserves_large_absolute_origins() {
    let model = qspquest_modeling::document::CompiledDocument::from_json(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    for origin in [-1e16, 1e16] {
        let req: RunRequest = serde_json::from_value(serde_json::json!({
            "schema":"qspquest.run/v0.1", "solver":"diffsol_rodas5p",
            "budgets":{"solver_callbacks":1000000,"events":100,"output_values":1000},
            "initial_states":{"central":{"value":6,"unit":"mg"}},
            "parameters":{"cl":{"value":0.6,"unit":"L/h"},"v":{"value":4.5,"unit":"L"}},
            "regimen":{"start":{"value":origin,"unit":"h"},"end":{"value":origin+4.,"unit":"h"},
              "samples":[{"value":origin,"unit":"h"},{"value":origin+4.,"unit":"h"}],
              "administrations":[],"rtol":1e-10,"atol":1e-12}
        }))
        .unwrap();
        let result = model.execute(&req).unwrap();
        let cp = result.outputs.iter().find(|o| o.name == "cp").unwrap();
        let expected = 6. / 4.5 * (-0.6_f64 / 4.5 * 4.).exp();
        assert!((cp.values.last().unwrap() - expected).abs() < 1e-8);
        assert_eq!(*result.times.last().unwrap(), origin + 4.);
    }
}

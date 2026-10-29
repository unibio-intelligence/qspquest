use qspquest_core::{model::Quantity, run::CycleIterationRequest};
use qspquest_modeling::document::CompiledDocument;
fn model() -> std::sync::Arc<CompiledDocument> {
    CompiledDocument::from_json(include_str!(
        "../../../conformance/models/synthetic-saturable-periodic.json"
    ))
    .unwrap()
}
fn request() -> CycleIterationRequest {
    serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-saturable-periodic.json"
    ))
    .unwrap()
}
#[test]
fn typed_saturable_cycle_has_unit_bearing_seed_and_convergence_controls() {
    let m = model();
    let q = request();
    let reference = m.iterate_periodic_state(&q).unwrap();
    let expected = 5. / libm::expm1(2.5);
    assert!((reference.steady_state.initial_states["central"].value - expected).abs() < 2e-7);
    assert_eq!(reference.steady_state.initial_states["central"].unit, "mg");
    assert_eq!(
        reference.steady_state.identity.algorithm,
        "bdf_periodic_cycle_iteration"
    );
    let mut other = q.clone();
    other.run.initial_states.insert(
        "central".into(),
        Quantity {
            value: 0.05,
            unit: "g".into(),
        },
    );
    let result = m.iterate_periodic_state(&other).unwrap();
    assert!((result.steady_state.initial_states["central"].value - expected).abs() < 2e-7);
    assert_ne!(
        reference.steady_state.identity.bound_value_hash,
        result.steady_state.identity.bound_value_hash
    );
    other = q.clone();
    other.run.request_id = Some("transport".into());
    assert_eq!(
        m.iterate_periodic_state(&other)
            .unwrap()
            .steady_state
            .identity,
        reference.steady_state.identity
    );
    other = q.clone();
    other.maximum_cycles = 90;
    assert_ne!(
        m.iterate_periodic_state(&other)
            .unwrap()
            .steady_state
            .identity
            .run_request_hash,
        reference.steady_state.identity.run_request_hash
    );
}
#[test]
fn typed_cycle_rejects_wrong_units_unsupported_semantics_and_nonconvergence() {
    let m = model();
    let q = request();
    for i in 0..5 {
        let mut bad = q.clone();
        match i {
            0 => bad.absolute_tolerances.get_mut("central").unwrap().unit = "L".into(),
            1 => bad.maximum_cycles = 2,
            2 => bad.run.regimen.administrations[0].amount.value = 12.,
            3 => bad.run.regimen.checkpoints.push(Quantity {
                value: 0.5,
                unit: "h".into(),
            }),
            _ => {
                bad.absolute_tolerances.clear();
            }
        }
        assert!(m.iterate_periodic_state(&bad).is_err(), "case {i}");
    }
    assert!(m.iterate_periodic_state(&q).is_ok());
    let mut value = serde_json::to_value(q).unwrap();
    value.as_object_mut().unwrap().remove("time_basis");
    assert!(serde_json::from_value::<CycleIterationRequest>(value).is_err());
}

use qspquest_core::{model::Quantity, run::RunRequest};
use qspquest_modeling::{language, ErrorCode, Model};
use std::collections::BTreeMap;
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
#[test]
fn initial_overrides_precede_events_and_have_canonical_bound_identity() {
    let model = language::parse(include_str!(
        "../../../conformance/models/synthetic-output-names.qsp"
    ))
    .unwrap()
    .compile()
    .unwrap();
    let mut request: RunRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-output-names.json"
    ))
    .unwrap();
    let baseline = model.execute(&request).unwrap();
    request
        .initial_states
        .insert("central".into(), q(6000., "ug"));
    let result = model.execute(&request).unwrap();
    assert_eq!(result.outputs[0].values[0], 6.);
    assert_eq!(result.outputs[0].values[1], 12.);
    assert!((result.outputs[0].values.last().unwrap() - 12. * (-0.2_f64).exp()).abs() < 3e-8);
    assert_ne!(
        result.identity.bound_value_hash,
        baseline.identity.bound_value_hash
    );
    request.initial_states.insert("central".into(), q(6., "mg"));
    assert_eq!(model.execute(&request).unwrap().identity, result.identity);
    for (name, value) in [
        ("missing", q(1., "mg")),
        ("central", q(1., "L")),
        ("central", q(f64::INFINITY, "mg")),
    ] {
        request.initial_states = BTreeMap::from([(name.into(), value)]);
        assert!(model.execute(&request).is_err());
    }
    request.initial_states.clear();
    assert_eq!(model.execute(&request).unwrap().identity, baseline.identity);
}
#[test]
fn overriding_one_initial_expression_skips_only_that_expression_and_rebinds_atomically() {
    let source="model initial_fixture\ntime_unit = h\nparameters\n  seed = 1 [1]\n  k = 1 [1/h]\nstates\n  x = log(seed) [1]\n  y = seed [1]\ndynamics\n  d/dt(x) = -k*x\n  d/dt(y) = -k*y\noutputs\n  x = x [1]\n  y = y [1]\nend\n";
    let model = language::parse(source).unwrap().compile().unwrap();
    let params = BTreeMap::from([("seed".into(), q(-1., "1"))]);
    assert_eq!(model.bind(&params).unwrap_err().code, ErrorCode::Domain);
    let mut bound = model
        .bind_with_initial_states(
            &params,
            &BTreeMap::new(),
            &BTreeMap::from([("x".into(), q(2., "1"))]),
        )
        .unwrap();
    assert_eq!(bound.initial(), [2., -1.]);
    bound
        .rebind(&BTreeMap::from([("seed".into(), q(4., "1"))]))
        .unwrap();
    assert_eq!(bound.initial(), [2., 4.]);
    assert!(bound
        .rebind(&BTreeMap::from([("seed".into(), q(1., "mg"))]))
        .is_err());
    assert_eq!(bound.initial(), [2., 4.]);
    assert_eq!(model.bind(&BTreeMap::new()).unwrap().initial(), [0., 1.]);
}
#[test]
fn initial_overrides_survive_covariate_steps_without_reinitialization() {
    let model = language::parse(include_str!(
        "../../../conformance/models/synthetic-covariate.qsp"
    ))
    .unwrap()
    .compile()
    .unwrap();
    let request:RunRequest=serde_json::from_value(serde_json::json!({
        "schema":"qspquest.run/v0.1","solver":"diffsol_bdf","budgets":{"solver_callbacks":1000000,"output_values":1000000,"events":10000},
        "initial_states":{"c":q(6.,"mg/L")},"regimen":{"end":q(2.,"h"),"samples":[q(0.,"h"),q(1.,"h"),q(2.,"h")],"administrations":[],"covariate_changes":[{"name":"wt","time":q(0.,"h"),"value":q(140.,"kg")},{"name":"wt","time":q(1.,"h"),"value":q(70.,"kg")}],"rtol":1e-10,"atol":1e-12}
    })).unwrap();
    let result = model.execute(&request).unwrap();
    for (row, t) in result.times.iter().enumerate() {
        let expected = 6. * (-0.1 * 2_f64.powf(-0.25) * t.min(1.) - 0.1 * (t - 1.).max(0.)).exp();
        assert!((result.outputs[0].values[row] - expected).abs() < 1e-8);
    }
    assert_eq!(result.outputs[0].values[0], 6.);
    assert_eq!(result.outputs[0].values[1], 6.);
    assert_eq!(result.outputs[1].values[0], 6.);
    assert_eq!(result.outputs[1].values[1], 3.);
}

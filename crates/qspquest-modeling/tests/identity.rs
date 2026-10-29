use qspquest_core::{
    identity::{canonical_bytes, canonical_hash},
    model::Quantity,
    run::RunRequest,
};
use qspquest_modeling::{document::CompiledDocument, language, ErrorCode};
use serde_json::json;
const SOURCE: &str = include_str!("../../../conformance/models/synthetic-mixed-scales.qsp");
fn request() -> RunRequest {
    serde_json::from_value(json!({"schema":"qspquest.run/v0.1","request_id":"first","solver":"diffsol_bdf","budgets":{"solver_callbacks":1000000,"output_values":1000000,"events":10000},"regimen":{"end":{"value":2,"unit":"h"},"samples":[{"value":1,"unit":"h"}],"administrations":[],"rtol":1e-6,"atol":1e-15}})).unwrap()
}
#[test]
fn canonicalization_orders_utf16_keys_and_normalizes_numbers() {
    let value = json!({"\u{e000}":1,"\u{10000}":2});
    assert_eq!(
        String::from_utf8(canonical_bytes(&value).unwrap()).unwrap(),
        "{\"\u{10000}\":2,\"\u{e000}\":1}"
    );
    assert_eq!(
        canonical_hash(&json!({"b":2,"a":1})).unwrap(),
        "sha256:43258cff783fe7036d8a43033f830adfc60ec037382473548ac742b888292777"
    );
    assert_eq!(
        canonical_hash(&json!(-0.0)).unwrap(),
        canonical_hash(&json!(0.0)).unwrap()
    );
    assert!(canonical_hash(&json!(9007199254740993_u64)).is_err());
}
#[test]
fn result_identity_tracks_model_bindings_and_run_but_not_correlation_id() {
    let parsed = language::parse(SOURCE).unwrap();
    let c = parsed.compile().unwrap();
    let r = request();
    let first = c.execute(&r).unwrap();
    assert_eq!(first.outputs[0].name, "bulk_amount");
    assert_eq!(first.outputs[0].unit, "mg");
    assert_eq!(first.time_unit, "h");
    assert!((first.outputs[0].values[1] - (-0.1_f64).exp()).abs() < 1e-6);
    assert_eq!(first.identity.model_content_hash, c.model_content_hash());
    assert_eq!(first.identity.compiler_source_sha256.len(), 71);
    assert_eq!(first.identity.backend, "diffsol");
    assert!(first.identity.backend_version.starts_with("0.18.0+sha256:"));
    let same =
        CompiledDocument::from_json(&serde_json::to_string(&parsed.document).unwrap()).unwrap();
    assert_eq!(same.model_content_hash(), c.model_content_hash());
    let text = language::format(&parsed.document).unwrap();
    assert_eq!(
        language::parse(&text)
            .unwrap()
            .compile()
            .unwrap()
            .model_content_hash(),
        c.model_content_hash()
    );
    let mut changed = r.clone();
    changed.request_id = Some("second".into());
    assert_eq!(c.execute(&changed).unwrap().identity, first.identity);
    changed.parameters.insert(
        "k_fast".into(),
        Quantity {
            value: 20.,
            unit: "1/h".into(),
        },
    );
    let next = c.execute(&changed).unwrap();
    assert_ne!(
        next.identity.bound_value_hash,
        first.identity.bound_value_hash
    );
    assert_eq!(
        next.identity.run_request_hash,
        first.identity.run_request_hash
    );
    changed.parameters.clear();
    changed.regimen.end.value = 3.;
    let next = c.execute(&changed).unwrap();
    assert_ne!(
        next.identity.run_request_hash,
        first.identity.run_request_hash
    );
    assert_eq!(
        next.identity.bound_value_hash,
        first.identity.bound_value_hash
    );
    assert_eq!(c.execute(&r).unwrap().identity, first.identity);
    let modified = language::parse(&SOURCE.replace("k_fast = 10", "k_fast = 20"))
        .unwrap()
        .compile()
        .unwrap();
    assert_ne!(modified.model_content_hash(), c.model_content_hash());
}
#[test]
fn unsupported_execution_options_and_unknown_fields_fail() {
    let c = language::parse(SOURCE).unwrap().compile().unwrap();
    let mut r = request();
    r.seed = Some(42);
    assert_eq!(c.execute(&r).unwrap_err().code, ErrorCode::Unsupported);
    let mut r = request();
    r.budgets.solver_callbacks = u64::MAX;
    assert_eq!(c.execute(&r).unwrap_err().code, ErrorCode::InvalidInput);
    let mut value = serde_json::to_value(request()).unwrap();
    value["solver"] = json!("rosenbrock");
    assert!(serde_json::from_value::<RunRequest>(value).is_err());
    let mut value = serde_json::to_value(request()).unwrap();
    value["budgets"]["ignored_option"] = json!(1);
    assert!(serde_json::from_value::<RunRequest>(value).is_err());
}

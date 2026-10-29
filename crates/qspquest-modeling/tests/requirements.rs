use qspquest_core::{model::*, ErrorCode};
use qspquest_modeling::{document::CompiledDocument, language};
const SOURCE: &str = include_str!("../../../conformance/models/synthetic-one-compartment.qsp");
fn source(requirements: &str) -> String {
    SOURCE.replace("\nend", &format!("\nrequirements\n{requirements}\nend"))
}
#[test]
fn supported_requirements_roundtrip_and_execute_without_changing_trajectories() {
    let parsed = language::parse(&source(
        "  stiff = true\n  events = [bolus, infusion, reset, covariate_step]",
    ))
    .unwrap();
    let model = parsed.compile().unwrap();
    let canonical = language::format(&parsed.document).unwrap();
    assert_eq!(
        parsed.document,
        language::parse(&canonical).unwrap().document
    );
    assert_eq!(
        canonical,
        language::format(&language::parse(&canonical).unwrap().document).unwrap()
    );
    let original = language::parse(SOURCE).unwrap().compile().unwrap();
    let request = serde_json::from_value(serde_json::json!({
        "schema":"qspquest.run/v0.1", "solver":"diffsol_bdf", "budgets": {
            "solver_callbacks":1000000, "output_values":1000000, "events":10000
        }, "regimen": {
            "end":{"value":2,"unit":"h"}, "samples":[{"value":1,"unit":"h"},{"value":2,"unit":"h"}],
            "administrations":[{"target":"central", "time":{"value":0,"unit":"h"}, "amount":{"value":6,"unit":"mg"}, "delivery":{"kind":"bolus"}, "bioavailability":1}],
            "rtol":1e-9, "atol":1e-10
        }
    })).unwrap();
    let before = original.execute(&request).unwrap();
    let after = model.execute(&request).unwrap();
    assert_eq!(
        serde_json::to_value(before.outputs).unwrap(),
        serde_json::to_value(after.outputs).unwrap()
    );
    assert_ne!(
        before.identity.model_content_hash,
        after.identity.model_content_hash
    );
    assert_eq!(
        before.identity.bound_value_hash,
        after.identity.bound_value_hash
    );
}
#[test]
fn unavailable_requirements_fail_before_equation_compilation_with_source_locations() {
    for (key, value) in [
        ("sparse_jacobian", "true"),
        ("gradients", "true"),
        ("events", "[state_triggered]"),
        ("events", "[parameter_dependent_time]"),
        ("events", "[steady_state]"),
    ] {
        let text = source(&format!("  {key} = {value}"));
        let parsed = language::parse(&text).unwrap();
        let error = CompiledDocument::compile(&parsed.document).unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert_eq!(
            error.expression.as_deref(),
            Some(format!("requirements.{key}").as_str())
        );
        let diagnostic = parsed.compile().unwrap_err();
        assert_eq!(
            diagnostic.line,
            text.lines()
                .position(|s| s.contains(&format!("{key} =")))
                .unwrap()
                + 1
        );
        let mut bad_equations = parsed.document.clone();
        bad_equations.states[0].unit = "unknown_unit".into();
        assert_eq!(
            CompiledDocument::compile(&bad_equations).unwrap_err().code,
            ErrorCode::Unsupported
        );
    }
}
#[test]
fn malformed_unknown_and_duplicate_requirements_fail_closed() {
    for req in [
        "  stiff = yes",
        "  events = bolus",
        "  events = [bolus,]",
        "  events = [typo]",
        "  events = [bolus, bolus]",
        "  gradients = false\n  gradients = true",
        "  typo = true",
    ] {
        assert!(language::parse(&source(req)).is_err(), "{req}");
    }
    let parsed = language::parse(SOURCE).unwrap();
    let mut value = serde_json::to_value(&parsed.document).unwrap();
    value["requirements"] = serde_json::json!({"sparse_jacobain":true});
    assert!(CompiledDocument::from_json(&value.to_string()).is_err());
    let mut model = parsed.document;
    model.requirements.events = vec![EventRequirement::Bolus, EventRequirement::Bolus];
    assert_eq!(
        CompiledDocument::compile(&model).unwrap_err().code,
        ErrorCode::InvalidInput
    );
}

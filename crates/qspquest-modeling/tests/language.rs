use qspquest_core::model::*;
use qspquest_modeling::{
    compile::Program,
    language::{self, expression},
};
use std::collections::BTreeMap;
const SOURCE: &str = r#"model synthetic_one_compartment
time_unit = h
description = "Synthetic # fixture"

parameters
  cl = 0.3 [L/h] log bounds(0.01 [L/h], 1 [L/h]) description "Clearance"
  v = 3 [L] log
states
  central = 0 [mg] input
dynamics
  d/dt(central) = input_central - cl / v * central
outputs
  cp = central / v [mg/L]
end
"#;
#[test]
fn text_model_compiles_runs_and_formats_idempotently() {
    let parsed = language::parse(SOURCE).unwrap();
    let compiled = parsed.compile().unwrap();
    let bound = compiled.bind(&BTreeMap::new()).unwrap();
    let protocol = qspquest_modeling::Protocol {
        start: 0.0,
        end: 10.0,
        samples: vec![1.0, 5.0, 10.0],
        events: vec![qspquest_modeling::Event::Bolus {
            time: 0.0,
            target: 0,
            amount: 6.0,
        }],
        rtol: 1e-9,
        atol: 1e-11,
    };
    for row in bound
        .simulate_outputs(&protocol, Default::default())
        .unwrap()
    {
        let expected = if row.time == 0.0 && row.side == "pre" {
            0.0
        } else {
            2.0 * (-0.1 * row.time).exp()
        };
        assert!((row.values[0] - expected).abs() < 2e-8);
    }
    let canonical = language::format(&parsed.document).unwrap();
    let reparsed = language::parse(&canonical).unwrap();
    assert_eq!(parsed.document, reparsed.document);
    assert_eq!(canonical, language::format(&reparsed.document).unwrap());
    assert!(canonical.contains("Synthetic # fixture"));
}
#[test]
fn expression_precedence_domains_and_roundtrip_are_explicit() {
    for (source, expected) in [
        ("-2^2", -4.0),
        ("2^3^2", 512.0),
        ("2^-2", 0.25),
        ("1 + 2 * 3", 7.0),
        ("(-3) % 2", 1.0),
        ("true && false || true ? 7 : log(-1)", 7.0),
    ] {
        let expression = expression::parse(source, 1, 1).unwrap();
        assert_eq!(
            Program::compile(&expression, &[])
                .unwrap()
                .evaluate(&[])
                .unwrap(),
            expected
        );
        let canonical = expression::format(&expression);
        assert_eq!(
            expression,
            expression::parse(&canonical, 1, 1).unwrap(),
            "{source}"
        );
    }
    let expression = expression::parse("(true && false) && true && false", 1, 1).unwrap();
    assert_eq!(
        expression,
        expression::parse(&expression::format(&expression), 1, 1).unwrap()
    );
    for bad in [
        "1 < 2 < 3",
        "system(1)",
        "x.y",
        "1e999",
        "1;2",
        "[1]",
        "X",
        "1e",
        "1 &&",
        "ifelse(true,1,)",
    ] {
        assert!(expression::parse(bad, 7, 3).is_err(), "{bad}");
    }
    assert!(expression::parse(&"(".repeat(1000), 1, 1).is_err());
}
#[test]
fn malformed_models_and_duplicate_options_do_not_silently_change_meaning() {
    for bad in [
        SOURCE.replace("log bounds", "log identity bounds"),
        SOURCE.replace("[mg] input", "[mg] other"),
        SOURCE.replace("d/dt(central)", "central'"),
        SOURCE.replace("end\n", ""),
        format!("{SOURCE}outputs\n"),
        SOURCE.replace("outputs\n", "parameters\n"),
        SOURCE.replace("d/dt(central)", "d/dt(missing)"),
        SOURCE.replace("states\n", "random_effects\n"),
    ] {
        assert!(language::parse(&bad).is_err());
    }
    let bad = SOURCE.replace("central / v [mg/L]", "central / [mg/L]");
    let error = language::parse(&bad).unwrap_err();
    assert_eq!(error.line, 13);
    assert!(error.column > 3);
    assert!(!error.hint.is_empty());
    let bad = SOURCE.replace("input_central - cl / v * central", "central");
    let error = language::parse(&bad).unwrap().compile().unwrap_err();
    assert_eq!((error.line, error.column), (11, 19));
    assert_eq!(error.code, "unit");
}
#[test]
fn json_model_and_text_model_have_the_same_compiler_path() {
    let source: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let text = language::format(&source).unwrap();
    assert_eq!(source, language::parse(&text).unwrap().document);
}

#[test]
fn formatter_retains_initial_quantity_units_and_expression_kind() {
    let mut document: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    document.states[0].initial = Initial::Quantity {
        quantity: Quantity {
            value: 0.001,
            unit: "g".into(),
        },
    };
    assert_eq!(
        document,
        language::parse(&language::format(&document).unwrap())
            .unwrap()
            .document
    );
    document.states[0].unit = "1".into();
    document.states[0].dosing = None;
    document.states[0].initial = Initial::Expression {
        expression: qspquest_core::expression::Expr::number(1.0),
    };
    document.outputs[0].unit = "1/L".into();
    assert_eq!(
        document,
        language::parse(&language::format(&document).unwrap())
            .unwrap()
            .document
    );
    let raw = qspquest_core::expression::Expr::Unary {
        operator: qspquest_core::expression::Unary::Negative,
        arguments: Box::new([qspquest_core::expression::Expr::number(2.0)]),
    };
    assert_eq!(
        raw,
        expression::parse(&expression::format(&raw), 1, 1).unwrap()
    );
}

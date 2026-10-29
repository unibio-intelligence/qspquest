use qspquest_core::{
    expression::{Expr, Literal},
    model::*,
};
use qspquest_modeling::{document::CompiledDocument, language};
use std::collections::BTreeMap;
const SOURCE: &str = include_str!("../../../conformance/models/synthetic-one-compartment.qsp");
#[test]
fn output_labels_reuse_state_parameter_and_time_names_without_shadowing() {
    let mut parsed = language::parse(SOURCE).unwrap();
    for (name, unit) in [("central", "mg"), ("v", "L"), ("time", "h")] {
        parsed.document.outputs.push(Output {
            name: name.into(),
            unit: unit.into(),
            expression: Expr::Symbol { name: name.into() },
        });
    }
    let formatted = language::format(&parsed.document).unwrap();
    assert_eq!(
        parsed.document,
        language::parse(&formatted).unwrap().document
    );
    let compiled = CompiledDocument::compile(&parsed.document).unwrap();
    let bound = compiled.bind(&BTreeMap::new()).unwrap();
    assert_eq!(bound.outputs(2., &[6.]).unwrap(), [2., 6., 3., 2.]);
    let output = parsed
        .document
        .outputs
        .iter_mut()
        .find(|o| o.name == "central")
        .unwrap();
    output.expression = Expr::Literal {
        value: Literal::Number(0.),
    };
    output.unit = "1".into();
    let model = CompiledDocument::compile(&parsed.document).unwrap();
    let bound = model.bind(&BTreeMap::new()).unwrap();
    let rows = bound
        .simulate_outputs(
            &qspquest_modeling::Protocol {
                start: 0.0,
                end: 1.,
                samples: vec![1.],
                events: vec![qspquest_modeling::Event::Bolus {
                    time: 0.,
                    target: 0,
                    amount: 6.,
                }],
                rtol: 1e-9,
                atol: 1e-11,
            },
            Default::default(),
        )
        .unwrap();
    assert!((rows.last().unwrap().values[0] - 2. * (-0.1_f64).exp()).abs() < 1e-8);
    assert_eq!(rows.last().unwrap().values[1], 0.);
}
#[test]
fn output_names_are_unique_and_do_not_become_expression_bindings() {
    let parsed = language::parse(SOURCE).unwrap();
    let mut model = parsed.document.clone();
    model.outputs.push(model.outputs[0].clone());
    assert!(CompiledDocument::compile(&model).is_err());
    let mut model = parsed.document.clone();
    model.outputs[0].name = "bad-name".into();
    assert!(CompiledDocument::compile(&model).is_err());
    let mut model = parsed.document.clone();
    model.outputs.push(Output {
        name: "other".into(),
        unit: "mg/L".into(),
        expression: Expr::Symbol { name: "cp".into() },
    });
    assert_eq!(
        CompiledDocument::compile(&model).unwrap_err().code,
        qspquest_core::ErrorCode::UnknownSymbol
    );
    let mut model = parsed.document;
    model.parameters[0].name = "central".into();
    assert!(CompiledDocument::compile(&model).is_err());
}

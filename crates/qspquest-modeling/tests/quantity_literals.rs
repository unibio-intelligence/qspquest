use qspquest_core::{
    expression::{Expr, Function},
    model::Quantity,
    units::Unit,
    ErrorCode,
};
use qspquest_modeling::compile::{Program, Symbol, SymbolKind};

fn quantity(value: f64, unit: &str) -> Expr {
    Expr::Quantity {
        quantity: Quantity {
            value,
            unit: unit.into(),
        },
    }
}

#[test]
fn grouped_clamp_constants_convert_and_reject_wrong_dimensions() {
    let expression = Expr::call(
        Function::Max,
        vec![Expr::symbol("volume"), quantity(1., "mL")],
    );
    let bindings = [(
        Symbol {
            name: "volume".into(),
            kind: SymbolKind::Parameter,
        },
        Unit::parse("L").unwrap(),
    )];
    let program =
        Program::compile_with_units(&expression, &bindings, Unit::parse("L").unwrap()).unwrap();
    assert_eq!(program.evaluate(&[0.]).unwrap(), 0.001);
    assert_eq!(program.evaluate(&[2.]).unwrap(), 2.);
    let incompatible = Expr::call(
        Function::Max,
        vec![Expr::symbol("volume"), quantity(1., "mg")],
    );
    assert_eq!(
        Program::compile_with_units(&incompatible, &bindings, Unit::parse("L").unwrap())
            .unwrap_err()
            .code,
        ErrorCode::Unit
    );
}

#[test]
fn quantity_constants_keep_count_kinds_and_overflow_refusals() {
    let program = Program::compile_with_units(
        &quantity(2., "[cell]/uL"),
        &[],
        Unit::parse("[cell]/L").unwrap(),
    )
    .unwrap();
    assert_eq!(program.evaluate(&[]).unwrap(), 2_000_000.);
    assert!(Program::compile_with_units(
        &quantity(2., "[genome]/uL"),
        &[],
        Unit::parse("[cell]/L").unwrap()
    )
    .is_err());
    assert!(Program::compile_with_units(
        &quantity(f64::MAX, "kg"),
        &[],
        Unit::parse("mg").unwrap()
    )
    .unwrap()
    .evaluate(&[])
    .is_err());
    assert!(Program::compile_with_units(
        &quantity(f64::NAN, "mg"),
        &[],
        Unit::parse("mg").unwrap()
    )
    .is_err());
}

#[test]
fn raw_unitless_programs_cannot_erase_physical_constant_dimensions() {
    assert!(Program::compile(&quantity(1., "mg"), &[]).is_err());
    let program = Program::compile(&quantity(50., "%"), &[]).unwrap();
    assert_eq!(program.evaluate(&[]).unwrap(), 0.5);
    assert!(serde_json::from_str::<Expr>(
        r#"{"kind":"quantity","quantity":{"value":1,"unit":"mg","extra":true}}"#
    )
    .is_err());
}

#[test]
fn canonical_text_quantities_roundtrip_and_execute_with_count_units() {
    let source = r#"model typed_constants
time_unit = d
parameters
 k = 0.2 [1/d] fixed
definitions
 floor_density = quantity(1, "[cell]/mL") [[cell]/L]
states
 cells = 2 [[cell]/L]
dynamics
 d/dt(cells) = -k * max(cells, floor_density)
outputs
 density = max(cells, quantity(-1, "[cell]/mL")) [[cell]/L]
end
"#;
    let parsed = qspquest_modeling::language::parse(source).unwrap();
    let text = qspquest_modeling::language::format(&parsed.document).unwrap();
    let reparsed = qspquest_modeling::language::parse(&text).unwrap();
    assert_eq!(parsed.document, reparsed.document);
    qspquest_modeling::document::CompiledDocument::compile(&parsed.document).unwrap();
    assert!(
        qspquest_modeling::language::parse(&source.replace("quantity(1,", "quantity(k,")).is_err()
    );
    assert!(qspquest_modeling::language::parse(&source.replace("[cell]/mL", "mystery")).is_err());
}

#[test]
fn independent_clamp_is_constant_for_selected_sensitivities() {
    use qspquest_modeling::document::sensitivity::CompiledSensitivityDocument;
    let source = r#"model independent_clamp
time_unit = h
parameters
 k = 0.2 [1/h] estimated
 volume = 2 [L] estimated
states
 mass = 10 [mg]
dynamics
 d/dt(mass) = -k * mass
outputs
 concentration = mass / max(volume, quantity(1e-12, "L")) [mg/L]
end
"#;
    let document = qspquest_modeling::language::parse(source).unwrap().document;
    let compiled = CompiledSensitivityDocument::compile(&document, &["k".into()]).unwrap();
    let request: qspquest_core::run::SensitivityRequest = serde_json::from_value(serde_json::json!({
        "schema":"qspquest.sensitivity/v0.1", "with_respect_to":["k"],
        "derivative_absolute_tolerances":{"k":{"mass":{"value":1e-12,"unit":"mg.h"}}},
        "run":{"schema":"qspquest.run/v0.1","solver":"diffsol_bdf", "budgets":{"solver_callbacks":100000,"events":100,"output_values":1000},
            "regimen":{"end":{"value":1,"unit":"h"},"samples":[{"value":0,"unit":"h"},{"value":1,"unit":"h"}],"administrations":[],"rtol":1e-11,"atol":1e-12}}
    })).unwrap();
    let result = compiled.run(&request).unwrap();
    assert!((result.result.outputs[0].values[1] - 5. * (-0.2_f64).exp()).abs() < 1e-7);
    assert!((result.derivatives[0].values[1] + 5. * (-0.2_f64).exp()).abs() < 1e-7);
    assert_eq!(
        CompiledSensitivityDocument::compile(&document, &["volume".into()])
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    let state_clamp = qspquest_modeling::language::parse(
        &source
            .replace("max(volume, quantity(1e-12, \"L\"))", "volume")
            .replace(
                "mass / volume",
                "max(mass, quantity(1e-12, \"mg\")) / volume",
            ),
    )
    .unwrap()
    .document;
    assert_eq!(
        CompiledSensitivityDocument::compile(&state_clamp, &["k".into()])
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
}

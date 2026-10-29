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

#[test]
fn constant_rates_keep_units_through_sensitivity_expansion() {
    use qspquest_modeling::document::{sensitivity::CompiledSensitivityDocument, CompiledDocument};
    for rate in [0., 1.] {
        let source = format!(
            r#"model constant_rate
time_unit = min
parameters
 theta = 2 [mg/L] estimated
states
 unused = 1 [1]
dynamics
 d/dt(unused) = quantity({rate}, "1/h")
outputs
 y = theta [mg/L]
 marker = unused [1]
end
"#
        );
        let document = qspquest_modeling::language::parse(&source)
            .unwrap()
            .document;
        CompiledDocument::compile(&document).unwrap();
        let compiled = CompiledSensitivityDocument::compile(&document, &["theta".into()]).unwrap();
        for solver in ["diffsol_bdf", "diffsol_rodas5p"] {
            let request: qspquest_core::run::SensitivityRequest = serde_json::from_value(serde_json::json!({
                "schema":"qspquest.sensitivity/v0.1", "with_respect_to":["theta"],
                "derivative_absolute_tolerances":{"theta":{"unused":{"value":1e-12,"unit":"L/mg"}}},
                "run":{"schema":"qspquest.run/v0.1","solver":solver,
                    "budgets":{"solver_callbacks":100000,"events":100,"output_values":1000},
                    "regimen":{"end":{"value":120,"unit":"min"},"samples":[{"value":0,"unit":"min"},{"value":60,"unit":"min"},{"value":120,"unit":"min"}],"administrations":[],"rtol":1e-11,"atol":1e-12}}
            })).unwrap();
            let result = compiled.run(&request).unwrap();
            for (i, time) in result.result.times.iter().enumerate() {
                assert!((result.result.outputs[0].values[i] - 2.).abs() < 1e-10);
                assert!(
                    (result.result.outputs[1].values[i] - (1. + rate * time / 60.)).abs() < 1e-8
                );
                assert!((result.derivatives[0].values[i] - 1.).abs() < 1e-10);
                assert!(result.derivatives[1].values[i].abs() < 1e-10);
            }
        }
        let mut wrong = document.clone();
        wrong.states[0].rhs = quantity(rate, "mg");
        assert_eq!(
            CompiledDocument::compile(&wrong).unwrap_err().code,
            ErrorCode::Unit
        );
        assert_eq!(
            CompiledSensitivityDocument::compile(&wrong, &["theta".into()])
                .unwrap_err()
                .code,
            ErrorCode::Unit
        );
        wrong.states[0].rhs = Expr::number(rate);
        assert_eq!(
            CompiledSensitivityDocument::compile(&wrong, &["theta".into()])
                .unwrap_err()
                .code,
            ErrorCode::Unit
        );
    }
}

#[test]
fn derived_and_initial_physical_constants_keep_scaled_sensitivities() {
    use qspquest_core::{
        expression::Binary,
        model::{IndividualParameter, Initial},
    };
    use qspquest_modeling::document::{sensitivity::CompiledSensitivityDocument, CompiledDocument};
    let source = r#"model derived_constants
time_unit = min
parameters
 theta = 2 [mg] estimated
states
 mass = 0 [mg]
dynamics
 d/dt(mass) = quantity(60, "mg/h")
outputs
 concentration = mass / quantity(1, "L") [mg/L]
end
"#;
    let mut document = qspquest_modeling::language::parse(source).unwrap().document;
    document.individual.push(IndividualParameter {
        name: "offset".into(),
        expression: quantity(500., "ug"),
        unit: Some("mg".into()),
    });
    document.states[0].initial = Initial::Expression {
        expression: Expr::binary(Binary::Add, Expr::symbol("theta"), Expr::symbol("offset")),
    };
    CompiledDocument::compile(&document).unwrap();
    let compiled = CompiledSensitivityDocument::compile(&document, &["theta".into()]).unwrap();
    for solver in ["diffsol_bdf", "diffsol_rodas5p"] {
        let request: qspquest_core::run::SensitivityRequest = serde_json::from_value(serde_json::json!({
            "schema":"qspquest.sensitivity/v0.1", "with_respect_to":["theta"],
            "derivative_absolute_tolerances":{"theta":{"mass":{"value":1e-12,"unit":"1"}}},
            "run":{"schema":"qspquest.run/v0.1","solver":solver,
                "budgets":{"solver_callbacks":100000,"events":100,"output_values":1000},
                "regimen":{"end":{"value":120,"unit":"min"},"samples":[{"value":0,"unit":"min"},{"value":120,"unit":"min"}],"administrations":[],"rtol":1e-11,"atol":1e-12}}
        })).unwrap();
        let result = compiled.run(&request).unwrap();
        for (i, time) in result.result.times.iter().enumerate() {
            assert!((result.result.outputs[0].values[i] - (2.5 + time)).abs() < 1e-7);
            assert!((result.derivatives[0].values[i] - 1.).abs() < 1e-10);
        }
    }
    document.individual[0].unit = Some("h".into());
    assert_eq!(
        CompiledSensitivityDocument::compile(&document, &["theta".into()])
            .unwrap_err()
            .code,
        ErrorCode::Unit
    );
}

#[test]
fn physical_dose_scale_constants_preserve_event_derivatives_and_admission() {
    use qspquest_modeling::document::{sensitivity::CompiledSensitivityDocument, CompiledDocument};
    let source = r#"model constant_dose_scale
time_unit = h
parameters
 volume = 1 [L] estimated
states
 concentration = 0 [mg/L]
dynamics
 d/dt(concentration) = input_concentration
outputs
 cp = concentration [mg/L]
dosing
 concentration [mg] = 1 / (volume + quantity(500, "mL"))
end
"#;
    let document = qspquest_modeling::language::parse(source).unwrap().document;
    let compiled = CompiledSensitivityDocument::compile(&document, &["volume".into()]).unwrap();
    for solver in ["diffsol_bdf", "diffsol_rodas5p"] {
        let request: qspquest_core::run::SensitivityRequest = serde_json::from_value(serde_json::json!({
            "schema":"qspquest.sensitivity/v0.1", "with_respect_to":["volume"],
            "derivative_absolute_tolerances":{"volume":{"concentration":{"value":1e-12,"unit":"mg/L2"}}},
            "run":{"schema":"qspquest.run/v0.1","solver":solver,
                "budgets":{"solver_callbacks":100000,"events":100,"output_values":1000},
                "regimen":{"end":{"value":1,"unit":"h"},"samples":[{"value":0,"unit":"h"},{"value":1,"unit":"h"}],
                    "administrations":[{"target":"concentration","time":{"value":0,"unit":"h"},"amount":{"value":1000,"unit":"ug"},"delivery":{"kind":"bolus"},"bioavailability":1}],
                    "rtol":1e-11,"atol":1e-12}}
        })).unwrap();
        let result = compiled.run(&request).unwrap();
        assert_eq!(
            result.result.sides[0],
            qspquest_core::regimen::ObservationSide::Pre
        );
        assert_eq!(result.result.outputs[0].values[0], 0.);
        assert_eq!(result.derivatives[0].values[0], 0.);
        for i in 1..result.result.times.len() {
            assert!((result.result.outputs[0].values[i] - 2. / 3.).abs() < 1e-10);
            assert!((result.derivatives[0].values[i] + 4. / 9.).abs() < 1e-10);
        }
    }
    let mut wrong = document.clone();
    wrong.states[0].dosing.as_mut().unwrap().scale = quantity(1., "h");
    assert_eq!(
        CompiledDocument::compile(&wrong).unwrap_err().code,
        ErrorCode::Unit
    );
    assert_eq!(
        CompiledSensitivityDocument::compile(&wrong, &["volume".into()])
            .unwrap_err()
            .code,
        ErrorCode::Unit
    );
    use qspquest_modeling::runtime::sensitivity::{DerivedBinding, ResolvedBindings};
    assert!(ResolvedBindings::new(
        &["volume".into()],
        &[DerivedBinding {
            name: "offset".into(),
            expression: quantity(500., "mL"),
        }]
    )
    .is_err());
}

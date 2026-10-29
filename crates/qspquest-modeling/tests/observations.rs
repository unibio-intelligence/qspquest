use qspquest_core::{model::Quantity, regimen::*, ActiveInputs, ErrorCode};
use qspquest_modeling::{document::CompiledDocument, Budgets};
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
fn point(value: f64, unit: &str, side: ObservationSide) -> ObservationPoint {
    ObservationPoint {
        time: q(value, unit),
        side,
    }
}
fn request() -> Request {
    Request {
        checkpoints: vec![],
        start: None,
        covariates: Default::default(),
        covariate_changes: vec![],
        end: q(4.0, "h"),
        samples: vec![],
        observations: Some(vec![
            point(120.0, "min", ObservationSide::Pre),
            point(2.0, "h", ObservationSide::Post),
            point(1.0, "h", ObservationSide::Pre),
            point(1.0, "h", ObservationSide::Post),
            point(120.0, "min", ObservationSide::Post),
        ]),
        administrations: vec![
            Administration {
                target: "central".into(),
                time: q(0.0, "h"),
                amount: q(6.0, "mg"),
                delivery: Delivery::Bolus,
                repeat: None,
                lag: None,
                bioavailability: 1.0,
            },
            Administration {
                target: "central".into(),
                time: q(2.0, "h"),
                amount: q(3.0, "mg"),
                delivery: Delivery::Bolus,
                repeat: None,
                lag: None,
                bioavailability: 1.0,
            },
        ],
        resets: vec![StateReset {
            target: "central".into(),
            time: q(2.0, "h"),
            value: q(3000.0, "ug"),
            order: 0,
            active_inputs: ActiveInputs::Continue,
        }],
        rtol: 1e-10,
        atol: Some(1e-12),
        absolute_tolerances: None,
    }
}
fn compiled() -> std::sync::Arc<CompiledDocument> {
    CompiledDocument::from_json(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap()
}
#[test]
fn explicit_sides_preserve_observation_order_and_duplicates() {
    let c = compiled();
    let b = c.bind(&Default::default()).unwrap();
    let rows = b.simulate_regimen(&request(), Default::default()).unwrap();
    assert_eq!(rows.len(), 5);
    assert_eq!(
        rows.iter()
            .map(|r| (r.time, r.side.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (2.0, "pre"),
            (2.0, "post"),
            (1.0, "pre"),
            (1.0, "post"),
            (2.0, "post")
        ]
    );
    let expected = [
        2.0 * (-0.2f64).exp(),
        2.0,
        2.0 * (-0.1f64).exp(),
        2.0 * (-0.1f64).exp(),
        2.0,
    ];
    for (row, value) in rows.iter().zip(expected) {
        assert!((row.values[0] - value).abs() < 1e-8);
    }
    assert_eq!(rows[2].values, rows[3].values);
    assert_eq!(rows[1], rows[4]);
}
#[test]
fn ambiguous_invalid_and_excess_observation_plans_are_rejected() {
    let c = compiled();
    let b = c.bind(&Default::default()).unwrap();
    let mut r = request();
    r.samples.push(q(1.0, "h"));
    assert!(b.simulate_regimen(&r, Default::default()).is_err());
    r.samples.clear();
    r.observations = Some(vec![]);
    assert!(b.simulate_regimen(&r, Default::default()).is_err());
    r = request();
    r.observations.as_mut().unwrap()[0].time = q(10.0, "h");
    assert!(b.simulate_regimen(&r, Default::default()).is_err());
    r = request();
    r.observations.as_mut().unwrap()[0].time = q(1.0, "mg");
    assert_eq!(
        b.simulate_regimen(&r, Default::default()).unwrap_err().code,
        ErrorCode::Unit
    );
    assert_eq!(
        b.simulate_regimen(
            &request(),
            Budgets {
                output_values: 4,
                ..Default::default()
            }
        )
        .unwrap_err()
        .code,
        ErrorCode::OutputBudget
    );
    let mut json = serde_json::to_value(request()).unwrap();
    json["observations"][0]
        .as_object_mut()
        .unwrap()
        .remove("side");
    assert!(serde_json::from_value::<Request>(json).is_err());
}

#[test]
fn unrequested_output_domains_do_not_mask_valid_observations() {
    use qspquest_core::{
        expression::{Binary, Expr, Function},
        model::ModelDocument,
    };
    let mut document: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    // Normalize by an explicit unit-bearing amount, then take a dimensionless log.
    document.outputs[0].unit = "1".into();
    document.parameters.push(qspquest_core::model::Parameter {
        name: "mass_ref".into(),
        default: q(1.0, "mg"),
        bounds: None,
        transform: qspquest_core::model::Transform::Log,
        fixed: true,
        description: "".into(),
    });
    document.outputs[0].expression = Expr::call(
        Function::Log,
        vec![Expr::binary(
            Binary::Divide,
            Expr::symbol("central"),
            Expr::symbol("mass_ref"),
        )],
    );
    let c = CompiledDocument::compile(&document).unwrap();
    let b = c.bind(&Default::default()).unwrap();
    let mut r = request();
    r.observations = Some(vec![point(0.0, "h", ObservationSide::Post)]);
    assert!(
        (b.simulate_regimen(&r, Default::default()).unwrap()[0].values[0] - 6.0f64.ln()).abs()
            < 1e-14
    );
    r.observations = Some(vec![
        point(0.0, "h", ObservationSide::Post),
        point(0.0, "h", ObservationSide::Pre),
    ]);
    assert_eq!(
        b.simulate_regimen(&r, Default::default()).unwrap_err().code,
        ErrorCode::Domain
    );
}

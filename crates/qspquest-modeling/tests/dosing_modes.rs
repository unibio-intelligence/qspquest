use qspquest_core::{
    model::DeliveryMode,
    regimen::{Delivery, InfusionSpan},
    run::RunRequest,
};
use qspquest_modeling::{
    document::CompiledDocument, language, Budgets, ErrorCode, Event, Protocol,
};
use std::collections::BTreeMap;
fn source() -> qspquest_core::model::ModelDocument {
    serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap()
}
fn request() -> RunRequest {
    serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-output-names.json"
    ))
    .unwrap()
}
#[test]
fn modes_reject_disallowed_deliveries_before_expansion_and_raw_execution() {
    for (mode, forbidden) in [(DeliveryMode::Bolus, true), (DeliveryMode::Infusion, false)] {
        let mut doc = source();
        doc.states[0].dosing.as_mut().unwrap().modes = vec![mode];
        let model = CompiledDocument::compile(&doc).unwrap();
        let mut req = request();
        if forbidden {
            req.regimen.administrations[0].delivery = Delivery::Infusion {
                span: InfusionSpan::Duration {
                    duration: qspquest_core::model::Quantity {
                        value: 1.,
                        unit: "h".into(),
                    },
                },
            };
        }
        assert_eq!(
            model.execute(&req).unwrap_err().code,
            ErrorCode::Unsupported
        );
        let bound = model.bind(&BTreeMap::new()).unwrap();
        let event = if forbidden {
            Event::Infusion {
                time: 0.,
                target: 0,
                amount: 6.,
                duration: 1.,
            }
        } else {
            Event::Bolus {
                time: 0.,
                target: 0,
                amount: 6.,
            }
        };
        let protocol = Protocol {
            start: 0.0,
            end: 2.,
            samples: vec![1., 2.],
            events: vec![event],
            rtol: 1e-9,
            atol: 1e-11,
        };
        assert_eq!(
            qspquest_modeling::simulate(&bound, &protocol)
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        assert_eq!(
            bound
                .simulate_outputs(&protocol, Budgets::default())
                .unwrap_err()
                .code,
            ErrorCode::Unsupported
        );
        req.regimen.administrations.clear();
        assert!(model.execute(&req).is_ok());
    }
}
#[test]
fn allowed_modes_run_and_invalid_declarations_fail() {
    for modes in [vec![], vec![DeliveryMode::Bolus, DeliveryMode::Bolus]] {
        let mut doc = source();
        doc.states[0].dosing.as_mut().unwrap().modes = modes;
        assert!(CompiledDocument::compile(&doc).is_err());
    }
    let mut doc = source();
    doc.states[0].dosing.as_mut().unwrap().modes = vec![DeliveryMode::Bolus];
    let model = CompiledDocument::compile(&doc).unwrap();
    let result = model.execute(&request()).unwrap();
    for (i, t) in result.times.iter().enumerate() {
        let expected =
            if *t == 0. && result.sides[i] == qspquest_core::regimen::ObservationSide::Pre {
                0.
            } else {
                2. * (-0.1 * t).exp()
            };
        assert!((result.outputs[0].values[i] - expected).abs() < 1e-8);
    }
    let full = CompiledDocument::compile(&source()).unwrap();
    assert_ne!(model.model_content_hash(), full.model_content_hash());
}
#[test]
fn language_roundtrips_restrictions() {
    for suffix in ["bolus", "infusion", "infusion bolus"] {
        let text=format!("model modes\ntime_unit = h\nparameters\n k = 0.1 [1/h]\nstates\n a = 0 [mg]\ndynamics\n d/dt(a) = input_a - k * a\noutputs\n a = a [mg]\ndosing\n a [mg] {suffix} = 1\nend\n");
        let parsed = language::parse(&text).unwrap();
        let formatted = language::format(&parsed.document).unwrap();
        assert_eq!(
            language::parse(&formatted).unwrap().document,
            parsed.document
        );
        parsed.compile().unwrap();
    }
}

#[test]
fn zero_dose_scale_delivers_nothing_and_rebinding_restores_delivery() {
    use qspquest_core::{expression::Expr, model::Quantity};
    let mut doc = source();
    doc.parameters.push(serde_json::from_value(serde_json::json!({"name":"fraction","default":{"value":0,"unit":"1"},"transform":"identity","fixed":false,"bounds":null,"description":""})).unwrap());
    doc.states[0].dosing.as_mut().unwrap().scale = Expr::symbol("fraction");
    let model = CompiledDocument::compile(&doc).unwrap();
    for infusion in [false, true] {
        let mut req = request();
        if infusion {
            req.regimen.administrations[0].delivery = Delivery::Infusion {
                span: InfusionSpan::Duration {
                    duration: Quantity {
                        value: 1.,
                        unit: "h".into(),
                    },
                },
            };
        }
        let zero = model.execute(&req).unwrap();
        assert!(zero.outputs[0].values.iter().all(|v| *v == 0.));
        req.parameters.insert(
            "fraction".into(),
            Quantity {
                value: 1.,
                unit: "1".into(),
            },
        );
        let delivered = model.execute(&req).unwrap();
        for (t, value) in delivered.times.iter().zip(&delivered.outputs[0].values) {
            if *t < 1. {
                continue;
            }
            let expected = if infusion {
                20. * (1. - (-0.1_f64).exp()) * (-0.1 * (*t - 1.)).exp()
            } else {
                2. * (-0.1 * t).exp()
            };
            assert!((value - expected).abs() < 1e-7);
        }
        req.parameters.insert(
            "fraction".into(),
            Quantity {
                value: -0.1,
                unit: "1".into(),
            },
        );
        assert!(model.execute(&req).is_err());
        req.parameters.clear();
        assert_eq!(
            model.execute(&req).unwrap().outputs[0].values,
            zero.outputs[0].values
        );
    }
    let bound = model.bind(&BTreeMap::new()).unwrap();
    let protocol = Protocol {
        start: 0.,
        end: 2.,
        samples: vec![1., 2.],
        events: vec![
            Event::Bolus {
                time: 0.,
                target: 0,
                amount: 6.,
            },
            Event::Infusion {
                time: 0.,
                target: 0,
                amount: 6.,
                duration: 1.,
            },
        ],
        rtol: 1e-9,
        atol: 1e-11,
    };
    assert!(qspquest_modeling::simulate(&bound, &protocol)
        .unwrap()
        .iter()
        .all(|row| row.values[0] == 0.));
}

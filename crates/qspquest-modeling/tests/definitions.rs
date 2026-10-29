use qspquest_core::{expression::Expr, model::Definition};
use qspquest_modeling::{language, ErrorCode, Model};
use std::collections::BTreeMap;
const SOURCE: &str = include_str!("../../../conformance/models/synthetic-definitions.qsp");
#[test]
fn state_time_and_input_definitions_preserve_units_dynamics_and_jacobians() {
    let parsed = language::parse(SOURCE).unwrap();
    let formatted = language::format(&parsed.document).unwrap();
    assert_eq!(
        parsed.document,
        language::parse(&formatted).unwrap().document
    );
    let model = parsed.compile().unwrap();
    let bound = model.bind(&BTreeMap::new()).unwrap();
    let mut rhs = [0.];
    bound.rhs(2., &[6.], &[1.], &mut rhs).unwrap();
    assert!((rhs[0] - 0.4).abs() < 1e-12);
    let mut jac = [0.];
    bound.jac_mul(2., &[6.], &[1.], &[3.], &mut jac).unwrap();
    assert!((jac[0] + 0.3).abs() < 1e-12);
    assert_eq!(bound.outputs(2., &[6.]).unwrap(), [2., 2000., 0.6, 2.]);
    let protocol = qspquest_modeling::Protocol {
        start: 0.0,
        end: 2.,
        samples: vec![0.5, 1., 2.],
        events: vec![qspquest_modeling::Event::Bolus {
            time: 0.,
            target: 0,
            amount: 6.,
        }],
        rtol: 1e-9,
        atol: 1e-11,
    };
    for row in bound
        .simulate_outputs(&protocol, Default::default())
        .unwrap()
    {
        let cp = if row.time == 0. && row.side == "pre" {
            0.
        } else {
            2. * (-0.1 * row.time).exp()
        };
        assert!((row.values[0] - cp).abs() < 1e-8);
        assert!((row.values[1] - cp * 1000.).abs() < 1e-5);
    }
}
#[test]
fn cyclic_unknown_incompatible_and_hidden_dynamic_crossings_are_rejected() {
    for source in [
        SOURCE.replace("central / v [ug/L]", "elimination / cl [ug/L]"),
        SOURCE.replace("central / v [ug/L]", "missing / v [ug/L]"),
        SOURCE.replace("central / v [ug/L]", "central / v [h]"),
        SOURCE.replace(
            "forcing - elimination",
            "central > 0 ? forcing : elimination",
        ),
        SOURCE.replace(
            "forcing - elimination",
            "concentration > 0 ? forcing : elimination",
        ),
        SOURCE.replace("central = 0 [mg]", "central = elimination [mg]"),
        SOURCE.replace("definitions\n", "definitions\n  cl = 1 [1]\n"),
    ] {
        assert!(
            language::parse(&source).and_then(|p| p.compile()).is_err(),
            "{source}"
        );
    }
    let parsed = language::parse(&SOURCE.replace(
        "forcing - elimination",
        "concentration > 0 ? forcing : elimination",
    ))
    .unwrap();
    assert_eq!(parsed.compile().unwrap_err().code, "unsupported");
}
#[test]
fn lazy_use_does_not_evaluate_an_unselected_definition() {
    let source="model lazy_definitions\ntime_unit = h\nparameters\n  switch = 0 [1]\n  k = 1 [1/h]\ndefinitions\n  bad = log(x) [1]\nstates\n  x = 0 [1]\ndynamics\n  d/dt(x) = -k*x\noutputs\n  chosen = switch > 0 ? bad : 0 [1]\nend\n";
    let model = language::parse(source).unwrap().compile().unwrap();
    assert_eq!(
        model
            .bind(&BTreeMap::new())
            .unwrap()
            .outputs(0., &[0.])
            .unwrap(),
        [0.]
    );
    let p = BTreeMap::from([(
        "switch".into(),
        qspquest_core::model::Quantity {
            value: 1.,
            unit: "1".into(),
        },
    )]);
    assert_eq!(
        model.bind(&p).unwrap().outputs(0., &[0.]).unwrap_err().code,
        ErrorCode::Domain
    );
}
#[test]
fn bounded_expansion_accepts_large_generated_expressions_and_stops_exponential_growth() {
    let source="model expanded\ntime_unit = h\nparameters\n  k = 1 [1/h]\nstates\n  x = 1 [1]\ndynamics\n  d/dt(x) = -k*x\noutputs\n  result = x [1]\nend\n";
    let mut parsed = language::parse(source).unwrap();
    for i in 0..9 {
        let previous = if i == 0 {
            "x".into()
        } else {
            format!("d_{}", i - 1)
        };
        parsed.document.definitions.push(Definition {
            name: format!("d_{i}"),
            expression: Expr::binary(
                qspquest_core::expression::Binary::Add,
                Expr::symbol(&previous),
                Expr::symbol(&previous),
            ),
            unit: None,
        });
    }
    parsed.document.outputs[0].expression = Expr::symbol("d_8");
    let model = parsed.compile().unwrap();
    assert_eq!(
        model
            .bind(&BTreeMap::new())
            .unwrap()
            .outputs(0., &[1.])
            .unwrap(),
        [512.]
    );
    for i in 9..20 {
        parsed.document.definitions.push(Definition {
            name: format!("d_{i}"),
            expression: Expr::binary(
                qspquest_core::expression::Binary::Add,
                Expr::symbol(format!("d_{}", i - 1)),
                Expr::symbol(format!("d_{}", i - 1)),
            ),
            unit: None,
        });
    }
    assert_eq!(parsed.compile().unwrap_err().code, "expression_limit");
}

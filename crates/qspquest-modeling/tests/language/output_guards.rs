use qspquest_modeling::{language, ErrorCode};
use std::collections::BTreeMap;
const SOURCE:&str="model output_guard\ntime_unit = h\nparameters\n  k = 1 [1/h]\ndefinitions\n  guarded = x <= 0 ? 2.220446049250313e-16 : x [1]\n  logarithm = log(guarded) [1]\nstates\n  x = 0 [1]\ndynamics\n  d/dt(x) = -k*x\noutputs\n  logged = logarithm [1]\n  phase = time < 1 / k ? 1 : 2 [1]\nend\n";
#[test]
fn output_guards_are_lazy_pointwise_and_preserve_units() {
    let parsed = language::parse(SOURCE).unwrap();
    let model = parsed.compile().unwrap();
    let bound = model.bind(&BTreeMap::new()).unwrap();
    for x in [-1., 0., 1e-30, 0.5, 2.] {
        let expected = f64::ln(if x <= 0. { f64::EPSILON } else { x });
        for t in [0., 1., 2.] {
            let outputs = bound.outputs(t, &[x]).unwrap();
            assert!((outputs[0] - expected).abs() < 1e-12);
            assert_eq!(outputs[1], if t < 1. { 1. } else { 2. });
        }
    }
    let formatted = language::format(&parsed.document).unwrap();
    assert_eq!(
        parsed.document,
        language::parse(&formatted).unwrap().document
    );
    assert!(
        language::parse(&SOURCE.replace("logged = logarithm [1]", "logged = logarithm [h]"))
            .unwrap()
            .compile()
            .is_err()
    );
    let direct = SOURCE.replace("logged = logarithm [1]", "logged = x > 0 ? log(x) : 0 [1]");
    assert_eq!(
        language::parse(&direct)
            .unwrap()
            .compile()
            .unwrap()
            .bind(&BTreeMap::new())
            .unwrap()
            .outputs(0., &[-1.])
            .unwrap()[0],
        0.
    );
}
#[test]
fn moving_an_output_guard_into_dynamics_still_requires_events() {
    let dynamic = SOURCE.replace("d/dt(x) = -k*x", "d/dt(x) = -k*guarded");
    assert_eq!(
        language::parse(&dynamic)
            .unwrap()
            .compile()
            .unwrap_err()
            .code,
        "unsupported"
    );
    let strict = SOURCE.replace("logged = logarithm [1]", "logged = log(x) [1]");
    let model = language::parse(&strict).unwrap().compile().unwrap();
    assert_eq!(
        model
            .bind(&BTreeMap::new())
            .unwrap()
            .outputs(0., &[0.])
            .unwrap_err()
            .code,
        ErrorCode::Domain
    );
}

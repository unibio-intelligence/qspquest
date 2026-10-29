use qspquest_core::model::Quantity;
use qspquest_modeling::{language, Event, Model, Protocol};
use std::collections::BTreeMap;
const SOURCE: &str = include_str!("../../../conformance/models/synthetic-allometric.qsp");
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
#[test]
fn forward_dependencies_rebind_allometry_initials_and_dose_scales_atomically() {
    let parsed = language::parse(SOURCE).unwrap();
    let compiled = parsed.compile().unwrap();
    let mut bound = compiled.bind(&BTreeMap::new()).unwrap();
    let p = Protocol {
        start: 0.0,
        end: 5.0,
        samples: vec![0.0, 1.0, 5.0],
        events: vec![Event::Bolus {
            time: 0.0,
            target: 0,
            amount: 6.0,
        }],
        rtol: 1e-10,
        atol: 1e-12,
    };
    for (weight, volume, clearance, f, start) in [
        (70.0, 3.0, 0.3, 0.8, 2.0),
        (140.0, 3.0, 0.3, 0.8, 2.0),
        (35.0, 6.0, 0.6, 0.4, 1.0),
        (70.0, 3.0, 0.3, 0.8, 2.0),
    ] {
        let values = BTreeMap::from([
            ("wt".into(), q(weight * 1000.0, "g")),
            ("tv_v".into(), q(volume, "L")),
            ("tv_cl".into(), q(clearance, "L/h")),
            ("f".into(), q(f, "1")),
            ("start".into(), q(start, "mg/L")),
        ]);
        bound.rebind(&values).unwrap();
        let ratio = weight / 70.0;
        let v = volume * ratio;
        let k = clearance * ratio.powf(0.75) / v;
        assert!((bound.initial()[0] - start * ratio).abs() < 1e-12);
        assert!((bound.dose_scale(0).unwrap() - f / v).abs() < 1e-12);
        let rows = bound.simulate_outputs(&p, Default::default()).unwrap();
        let fresh = compiled
            .bind(&values)
            .unwrap()
            .simulate_outputs(&p, Default::default())
            .unwrap();
        assert_eq!(rows, fresh);
        for row in rows {
            let c0 = if row.time == 0.0 && row.side == "pre" {
                start * ratio
            } else {
                start * ratio + 6.0 * f / v
            };
            assert!((row.values[0] - c0 * (-k * row.time).exp()).abs() < 1e-8);
        }
    }
    let before = bound.simulate_outputs(&p, Default::default()).unwrap();
    assert!(bound
        .rebind(&BTreeMap::from([("tv_v".into(), q(0.0, "L"))]))
        .is_err());
    assert_eq!(
        before,
        bound.simulate_outputs(&p, Default::default()).unwrap()
    );
    assert!(bound
        .rebind(&BTreeMap::from([("cl".into(), q(1.0, "L/h"))]))
        .is_err());
    let text = language::format(&parsed.document).unwrap();
    assert_eq!(parsed.document, language::parse(&text).unwrap().document);
    assert_eq!(
        text,
        language::format(&language::parse(&text).unwrap().document).unwrap()
    );
}
#[test]
fn cycles_unknown_references_and_dimensional_errors_fail_before_run() {
    for bad in [
        SOURCE.replace("ratio = wt / wt_ref", "ratio = ratio"),
        SOURCE.replace("ratio = wt / wt_ref", "ratio = k"),
        SOURCE.replace("ratio = wt / wt_ref", "ratio = unknown"),
        SOURCE.replace("ratio = wt / wt_ref", "ratio = c"),
        SOURCE.replace("ratio = wt / wt_ref", "ratio = wt / 70"),
        SOURCE.replace("v = tv_v * ratio [L]", "v = tv_v * ratio [mg]"),
    ] {
        assert!(language::parse(&bad).unwrap().compile().is_err());
    }
}

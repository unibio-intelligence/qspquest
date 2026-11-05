use qspquest_core::units::{Dimension, Unit};

#[test]
fn activity_scales_but_does_not_convert_to_si_or_dimensionless() {
    let iu = Unit::parse("[iU]").unwrap();
    assert_eq!(iu, Unit::parse("[IU]").unwrap());
    assert_ne!(iu.dimension, Dimension::NONE);
    for incompatible in ["1", "mg", "mol", "kat", "umol/min"] {
        assert!(iu
            .conversion_to(Unit::parse(incompatible).unwrap())
            .is_err());
    }
    let per_l = Unit::parse("[iU]/L").unwrap();
    let per_ml = Unit::parse("[iU]/mL").unwrap();
    assert!((per_l.convert(21.0, per_ml).unwrap() - 0.021).abs() < 1e-15);
    assert_eq!(per_l.divide(per_l).unwrap().dimension, Dimension::NONE);
    assert_eq!(
        Unit::parse("[iU]2").unwrap().dimension.sqrt().unwrap(),
        iu.dimension
    );
    assert_eq!(
        Unit::parse("(1000).[iU]")
            .unwrap()
            .convert(2.0, iu)
            .unwrap(),
        2000.0
    );
}

#[test]
fn unsupported_activity_spellings_and_malformed_brackets_fail() {
    for spelling in ["IU", "[iu]", "[iU", "[iU]]", "[[iU]]", "[arb'U]", "[iU]L"] {
        assert!(Unit::parse(spelling).is_err(), "{spelling}");
    }
}

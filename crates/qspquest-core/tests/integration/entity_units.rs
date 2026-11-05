use qspquest_core::{
    planning::ExactUnit,
    units::{Dimension, Entity, Unit},
    ErrorCode,
};
use std::collections::BTreeMap;

#[test]
fn biological_counts_are_distinct_from_each_other_and_unrelated_quantities() {
    for source in ["[cell]", "[genome]"] {
        let unit = Unit::parse(source).unwrap();
        assert_ne!(unit.dimension, Dimension::NONE);
        for target in ["1", "mg", "mol", "[iU]"] {
            let error = unit
                .conversion_to(Unit::parse(target).unwrap())
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::Unit);
            assert!(!error.message.contains("molar mass"));
        }
    }
    assert!(Unit::parse("[cell]")
        .unwrap()
        .conversion_to(Unit::parse("[genome]").unwrap())
        .is_err());
}

#[test]
fn cell_and_genome_density_scales_preserve_the_counted_kind() {
    for entity in ["[cell]", "[genome]"] {
        let per_mm3 = Unit::parse(&format!("{entity}/mm3")).unwrap();
        let per_ul = Unit::parse(&format!("{entity}/uL")).unwrap();
        let per_l = Unit::parse(&format!("{entity}/L")).unwrap();
        assert_eq!(per_mm3.convert(199.57, per_ul).unwrap(), 199.57);
        assert!((per_mm3.convert(1.0, per_l).unwrap() - 1_000_000.0).abs() < 1e-9);
    }
    assert!(Unit::parse("[cell]/L")
        .unwrap()
        .conversion_to(Unit::parse("[genome]/L").unwrap())
        .is_err());
}

#[test]
fn count_dimensions_cancel_and_combine_in_physical_rate_terms() {
    let density = Unit::parse("[cell]/mm3").unwrap();
    assert_eq!(density.divide(density).unwrap().dimension, Dimension::NONE);
    let rate = Unit::parse("mm3/([cell].d)")
        .unwrap()
        .multiply(density)
        .unwrap();
    assert_eq!(rate, Unit::parse("1/d").unwrap());
    assert_eq!(
        Unit::parse("[cell]2").unwrap().dimension.sqrt().unwrap(),
        Unit::parse("[cell]").unwrap().dimension
    );
    assert!(Unit::parse("[cell]").unwrap().dimension.sqrt().is_err());
    assert!(Unit::parse("[genome]128").is_err());
}

#[test]
fn legacy_exact_units_keep_eight_coordinates_without_entity_fields() {
    let old = serde_json::json!({"dimensions":[0,3,0,0,0,0,0,0],"scale_numerator":"1",
                                "scale_denominator":"1000","declared":"L"});
    let decoded: ExactUnit = serde_json::from_value(old.clone()).unwrap();
    assert!(decoded.entities.is_empty());
    assert_eq!(serde_json::to_value(decoded).unwrap(), old);
    let unit = Unit::parse("[cell]/L").unwrap();
    let record = ExactUnit {
        dimensions: unit.dimension.physical_activity(),
        entities: unit.dimension.entities(),
        scale_numerator: unit.scale.numerator().to_string(),
        scale_denominator: unit.scale.denominator().to_string(),
        declared: Some("[cell]/L".into()),
    };
    assert_eq!(record.entities, BTreeMap::from([(Entity::Cell, 1)]));
    let value = serde_json::to_value(record).unwrap();
    assert_eq!(value["dimensions"].as_array().unwrap().len(), 8);
    assert_eq!(value["entities"], serde_json::json!({"cell":1}));
    let mut invalid = value;
    invalid["entities"] = serde_json::json!({"unknown_entity":1});
    assert!(serde_json::from_value::<ExactUnit>(invalid).is_err());
}

#[test]
fn unsupported_count_spellings_fail_instead_of_collapsing_to_dimensionless() {
    for spelling in [
        "cell",
        "genome",
        "cells",
        "vg",
        "[cells]",
        "[Cell]",
        "[genome",
        "[genome]]",
    ] {
        assert!(Unit::parse(spelling).is_err(), "{spelling}");
    }
}

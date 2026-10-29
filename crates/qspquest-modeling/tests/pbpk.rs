#[path = "../../../conformance/compare.rs"]
mod comparison;
use comparison::*;
use std::collections::BTreeMap;

#[test]
fn twenty_four_state_pbpk_matches_matrix_exponential_and_conserves_doses() {
    let model = qspquest_modeling::language::parse(include_str!(
        "../../../conformance/models/synthetic-pbpk-24.qsp"
    ))
    .unwrap()
    .compile()
    .unwrap();
    let request = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-pbpk-24.json"
    ))
    .unwrap();
    let result = model.execute(&request).unwrap();
    let reference: Trajectory = serde_json::from_str(include_str!(
        "../../../conformance/evidence/synthetic-pbpk-24-reference.json"
    ))
    .unwrap();
    let actual: Trajectory = serde_json::from_value(serde_json::json!({"time_unit": result.time_unit, "times": result.times, "sides": result.sides, "outputs": result.outputs})).unwrap();
    let tolerances = reference
        .outputs
        .iter()
        .map(|o| {
            (
                o.name.clone(),
                Tolerance {
                    unit: "mg".into(),
                    absolute: 1e-5,
                    relative: 1e-7,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let report = compare(&reference, &actual, &tolerances).unwrap();
    assert!(report.passed, "{report:?}");
    assert_eq!(actual.outputs.len(), 24);
    assert_eq!(actual.times.len(), 76);
    for (row, &time) in actual.times.iter().enumerate() {
        let dose = [0., 8., 16.]
            .iter()
            .filter(|&&d| d < time || (d == time && actual.sides[row] == Side::Post))
            .count() as f64
            * 100.;
        let total: f64 = actual.outputs.iter().map(|o| o.values[row]).sum();
        assert!(
            (total - dose).abs() < 1e-7,
            "mass balance at {time}: {total} vs {dose}"
        );
        assert!(actual.outputs.iter().all(|o| o.values[row] >= -1e-9));
    }
}

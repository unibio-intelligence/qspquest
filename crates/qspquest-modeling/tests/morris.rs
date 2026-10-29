use qspquest_core::{model::Quantity, morris::*, run::SensitivityRequest, scan::*};
use qspquest_modeling::document::CompiledDocument;
use std::sync::Arc;
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
fn fixture() -> (Arc<CompiledDocument>, MorrisRequest) {
    let c = CompiledDocument::from_json(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let run = serde_json::from_str::<SensitivityRequest>(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap()
    .run;
    let mut total = run.budgets;
    total.solver_callbacks *= 6;
    total.events *= 6;
    total.output_values *= 6;
    let coordinates = [
        (0., 0.),
        (2. / 3., 0.),
        (2. / 3., 2. / 3.),
        (1., 1.),
        (1., 1. / 3.),
        (1. / 3., 1. / 3.),
    ];
    let points = coordinates
        .iter()
        .enumerate()
        .map(|(i, (a, b))| ScanPoint {
            id: format!("p{i}"),
            parameters: [
                ("cl".into(), q((0.1 + 0.6 * a) * 1000., "mL/h")),
                ("v".into(), q(3. + 3. * b, "L")),
            ]
            .into_iter()
            .collect(),
        })
        .collect();
    (
        c,
        MorrisRequest {
            scan: ScanRequest {
                schema: ScanSchema::V01,
                run,
                points,
                total_budgets: total,
            },
            ranges: vec![
                ParameterRange {
                    parameter: "cl".into(),
                    lower: q(0.1, "L/h"),
                    upper: q(0.7, "L/h"),
                },
                ParameterRange {
                    parameter: "v".into(),
                    lower: q(3., "L"),
                    upper: q(6., "L"),
                },
            ],
            trajectories: vec![
                vec!["p0".into(), "p1".into(), "p2".into()],
                vec!["p3".into(), "p4".into(), "p5".into()],
            ],
            num_levels: 4,
            output: "cp".into(),
            row: 4,
        },
    )
}
#[test]
fn morris_effects_match_independent_pk_formula_with_reverse_steps() {
    let (c, r) = fixture();
    let result = c.morris(&r).unwrap();
    let values: Vec<f64> = r
        .scan
        .points
        .iter()
        .map(|p| {
            let cl = p.parameters["cl"].value / 1000.;
            let v = p.parameters["v"].value;
            6. / v * (-cl * 4. / v).exp()
        })
        .collect();
    let expected = [
        [
            (values[1] - values[0]) * 1.5,
            (values[5] - values[4]) * (-1.5),
        ],
        [
            (values[2] - values[1]) * 1.5,
            (values[4] - values[3]) * (-1.5),
        ],
    ];
    for (actual, expected) in result.effects.iter().zip(expected) {
        let mu = (expected[0] + expected[1]) / 2.;
        let mu_star = (expected[0].abs() + expected[1].abs()) / 2.;
        let sigma = ((expected[0] - mu).powi(2) + (expected[1] - mu).powi(2)).sqrt();
        for (a, b) in actual.effects.iter().zip(expected) {
            assert!((a - b).abs() < 1e-8);
        }
        assert!((actual.mu - mu).abs() < 1e-8);
        assert!((actual.mu_star - mu_star).abs() < 1e-8);
        assert!((actual.sigma - sigma).abs() < 1e-8);
        assert_eq!(actual.unit, "mg/L");
    }
    let mut changed = r.clone();
    changed.row = 3;
    assert_ne!(
        result.analysis_request_hash,
        c.morris(&changed).unwrap().analysis_request_hash
    );
}
#[test]
fn malformed_morris_designs_fail_and_recover() {
    let (c, r) = fixture();
    for mode in 0..8 {
        let mut bad = r.clone();
        match mode {
            0 => bad.num_levels = 3,
            1 => bad.trajectories.pop().map(|_| ()).unwrap(),
            2 => bad.trajectories[1][0] = "p0".into(),
            3 => bad.scan.points[1].parameters.get_mut("cl").unwrap().value = 400.,
            4 => bad.scan.points[1].parameters.get_mut("v").unwrap().value = 5.,
            5 => bad.ranges[0].upper = q(1., "h"),
            6 => bad.row = 999,
            _ => bad.output = "unknown".into(),
        }
        assert!(c.morris(&bad).is_err(), "mode {mode}");
    }
    assert_eq!(c.morris(&r).unwrap().effects.len(), 2);
}

#[test]
fn generated_designs_are_reproducible_and_executable_across_grids() {
    let (c, base) = fixture();
    let mut input = MorrisDesignRequest {
        run: base.scan.run.clone(),
        ranges: base.ranges.clone(),
        trajectory_count: 2,
        num_levels: 4,
        seed: 42,
        total_budgets: base.scan.total_budgets,
        output: base.output.clone(),
        row: base.row,
    };
    let generated = generate_morris_design(&input).unwrap();
    assert_eq!(
        serde_json::to_value(&generated).unwrap(),
        serde_json::to_value(generate_morris_design(&input).unwrap()).unwrap()
    );
    // Independently calculated SHA-256 stream and Fisher-Yates result in Python.
    let expected = [[0., 3.], [0., 1.], [2., 1.], [2., 1.], [0., 1.], [0., 3.]];
    for (point, xy) in generated.request.scan.points.iter().zip(expected) {
        assert!(((point.parameters["cl"].value - 0.1) / 0.6 * 3. - xy[0]).abs() < 1e-12);
        assert!((point.parameters["v"].value - 3. - xy[1]).abs() < 1e-12);
    }
    for levels in [4, 6, 10, 100] {
        for seed in [0, 1, 42, u32::MAX] {
            input.num_levels = levels;
            input.seed = seed;
            let design = generate_morris_design(&input).unwrap();
            let result = c.morris(&design.request).unwrap();
            assert_eq!(result.scan.results.len(), 6);
            assert!(result
                .effects
                .iter()
                .all(|e| e.effects.len() == 2 && e.mu_star >= e.mu.abs() - 1e-12));
        }
    }
    input.num_levels = 4;
    input.seed = 43;
    assert_ne!(
        generated.design_request_hash,
        generate_morris_design(&input).unwrap().design_request_hash
    );
}
#[test]
fn design_generation_rejects_excess_work_and_invalid_ranges() {
    let (_, base) = fixture();
    let valid = MorrisDesignRequest {
        run: base.scan.run,
        ranges: base.ranges,
        trajectory_count: 2,
        num_levels: 4,
        seed: 42,
        total_budgets: base.scan.total_budgets,
        output: base.output,
        row: base.row,
    };
    for mode in 0..6 {
        let mut bad = valid.clone();
        match mode {
            0 => bad.trajectory_count = 9,
            1 => bad.num_levels = 5,
            2 => bad.ranges[1].parameter = bad.ranges[0].parameter.clone(),
            3 => bad.ranges[0].upper = q(1., "h"),
            4 => bad.ranges[0].upper = bad.ranges[0].lower.clone(),
            _ => bad.ranges[0].lower.value = f64::NAN,
        }
        assert!(generate_morris_design(&bad).is_err());
    }
    assert_eq!(
        generate_morris_design(&valid)
            .unwrap()
            .request
            .scan
            .points
            .len(),
        6
    );
}

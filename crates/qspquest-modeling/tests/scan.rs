use qspquest_core::{model::Quantity, run::SensitivityRequest, scan::*, ErrorCode};
use qspquest_modeling::document::CompiledDocument;
use std::sync::Arc;
fn fixture() -> (Arc<CompiledDocument>, ScanRequest) {
    let c = CompiledDocument::from_json(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let sensitivity: SensitivityRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    let mut total = sensitivity.run.budgets;
    total.solver_callbacks *= 3;
    total.output_values *= 3;
    total.events *= 3;
    let points = [0.3, 0.6, 1.2]
        .iter()
        .enumerate()
        .map(|(i, cl)| ScanPoint {
            id: format!("point_{i}"),
            parameters: [(
                "cl".into(),
                Quantity {
                    value: *cl * 1000.,
                    unit: "mL/h".into(),
                },
            )]
            .into_iter()
            .collect(),
        })
        .collect();
    (
        c,
        ScanRequest {
            schema: ScanSchema::V01,
            run: sensitivity.run,
            points,
            total_budgets: total,
        },
    )
}
#[test]
fn scan_matches_independent_curves_and_retains_order_and_identities() {
    let (c, q) = fixture();
    let result = c.scan(&q).unwrap();
    for (i, point) in result.results.iter().enumerate() {
        assert_eq!(point.id, q.points[i].id);
        let cl = q.points[i].parameters["cl"].value / 1000.;
        for (row, t) in point.result.times.iter().enumerate() {
            let expected = if *t == 0.
                && point.result.sides[row] == qspquest_core::regimen::ObservationSide::Pre
            {
                0.
            } else {
                2. * (-cl * t / 3.).exp()
            };
            assert!((point.result.outputs[0].values[row] - expected).abs() < 1e-8);
        }
    }
    assert_ne!(
        result.results[0].result.identity.bound_value_hash,
        result.results[1].result.identity.bound_value_hash
    );
    let mut reordered = q.clone();
    reordered.points.reverse();
    let reversed = c.scan(&reordered).unwrap();
    assert_eq!(
        serde_json::to_value(&reversed.results[0].result.outputs).unwrap(),
        serde_json::to_value(&result.results[2].result.outputs).unwrap()
    );
    assert_ne!(result.scan_request_hash, reversed.scan_request_hash);
}
#[test]
fn scan_rejects_bad_points_before_solving_and_enforces_total_budgets() {
    let (c, q) = fixture();
    for mode in 0..6 {
        let mut bad = q.clone();
        match mode {
            0 => bad.points.clear(),
            1 => bad.points = vec![bad.points[0].clone(); 26],
            2 => bad.points[1].id = bad.points[0].id.clone(),
            3 => {
                bad.points[1].parameters.get_mut("cl").unwrap().unit = "h".into();
                bad.run.budgets.solver_callbacks = 0;
            }
            4 => bad.total_budgets.solver_callbacks = 0,
            _ => bad.total_budgets.output_values = 1,
        }
        let error = c.scan(&bad).unwrap_err();
        if mode == 3 {
            assert_eq!(error.code, ErrorCode::Unit);
        }
        if mode == 4 {
            assert_eq!(error.code, ErrorCode::WorkBudget);
        }
    }
    assert_eq!(c.scan(&q).unwrap().results.len(), 3);
}
#[test]
fn scan_prepared_bindings_match_full_individual_execution_and_recover() {
    let (c, mut q) = fixture();
    q.run.initial_states.insert(
        "central".into(),
        Quantity {
            value: 1.25,
            unit: "mg".into(),
        },
    );
    let result = c.scan(&q).unwrap();
    let count = q.points.len();
    for (point, actual) in q.points.iter().zip(&result.results) {
        let mut run = q.run.clone();
        run.parameters.extend(point.parameters.clone());
        run.budgets.solver_callbacks = run
            .budgets
            .solver_callbacks
            .min(q.total_budgets.solver_callbacks / count as u64);
        run.budgets.events = run.budgets.events.min(q.total_budgets.events / count);
        run.budgets.output_values = run
            .budgets
            .output_values
            .min(q.total_budgets.output_values / count);
        assert_eq!(
            serde_json::to_value(&actual.result).unwrap(),
            serde_json::to_value(c.execute(&run).unwrap()).unwrap()
        );
    }
    let mut bad = q.clone();
    bad.points[2].parameters.get_mut("cl").unwrap().unit = "h".into();
    assert_eq!(c.scan(&bad).unwrap_err().code, ErrorCode::Unit);
    assert_eq!(
        serde_json::to_value(c.scan(&q).unwrap()).unwrap(),
        serde_json::to_value(result).unwrap()
    );
}

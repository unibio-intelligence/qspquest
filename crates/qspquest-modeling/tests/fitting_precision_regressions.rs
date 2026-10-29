use qspquest_core::{
    fit::*,
    model::{ModelDocument, Quantity},
    run::{SensitivityRequest, Solver},
};
use qspquest_modeling::document::sensitivity::CompiledSensitivityDocument;
use qspquest_modeling::Model;
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
fn request() -> SensitivityRequest {
    let mut v: serde_json::Value = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-sensitivity.json"
    ))
    .unwrap();
    v["run"]["parameters"] =
        serde_json::json!({"cl":{"value":0.5,"unit":"L/h"},"v":{"value":4.,"unit":"L"}});
    v["with_respect_to"] = serde_json::json!(["cl"]);
    v["derivative_absolute_tolerances"]
        .as_object_mut()
        .unwrap()
        .remove("v");
    v["run"]["regimen"]["rtol"] = serde_json::json!(1e-9);
    v["run"]["regimen"]["atol"] = serde_json::json!(1e-10);
    v["run"]["regimen"]["end"] = serde_json::json!({"value":200,"unit":"h"});
    v["run"]["regimen"]["samples"] = serde_json::json!([]);
    let times = [
        0.5, 1., 8., 24., 72., 72.5, 73., 80., 96., 96.5, 97., 104., 144., 144.5, 145., 152., 192.,
        192.5, 193., 200.,
    ];
    v["run"]["regimen"]["observations"] = serde_json::json!(times
        .iter()
        .map(|t| serde_json::json!({"time":{"value":t,"unit":"h"},"side":"post"}))
        .collect::<Vec<_>>());
    v["run"]["regimen"]["administrations"]=serde_json::json!([0.,72.,96.,144.,192.].iter().map(|t|serde_json::json!({"target":"central","time":{"value":t,"unit":"h"},"amount":{"value":6.,"unit":"mg"},"delivery":{"kind":"infusion","span":{"kind":"duration","duration":{"value":1.,"unit":"h"}}},"bioavailability":1.})).collect::<Vec<_>>());
    serde_json::from_value(v).unwrap()
}
fn concentration(t: f64, cl: f64) -> f64 {
    [0., 72., 96., 144., 192.]
        .iter()
        .map(|start| {
            let age = t - start;
            if age <= 0. {
                0.
            } else {
                6. / cl
                    * (1. - (-cl / 4. * age.min(1.)).exp())
                    * (-cl / 4. * (age - 1.).max(0.)).exp()
            }
        })
        .sum()
}
#[test]
fn autonomous_repeated_infusion_sensitivities_match_closed_form() {
    let model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let mut req = request();
    let c = CompiledSensitivityDocument::compile(&model, &req.with_respect_to).unwrap();
    assert!(!c
        .bind(&req.run.parameters, &req.run.regimen.covariates)
        .unwrap()
        .rhs_depends_on_time());
    for solver in [Solver::DiffsolBdf, Solver::DiffsolRodas5p] {
        req.run.solver = solver;
        let result = c.run(&req).unwrap();
        for (time, value) in result
            .result
            .times
            .iter()
            .zip(&result.result.outputs[0].values)
        {
            let expected = concentration(*time, 0.5);
            assert!(
                (value - expected).abs() < 1e-7,
                "{solver:?} at {time}: {value} vs {expected}"
            );
        }
    }
}
#[test]
fn broad_box_noisy_fit_meets_requested_gradient_or_reports_limits() {
    let model: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-one-compartment.json"
    ))
    .unwrap();
    let mut req = request();
    let c = CompiledSensitivityDocument::compile(&model, &req.with_respect_to).unwrap();
    let times = c.run(&req).unwrap().result.times;
    for solver in [Solver::DiffsolBdf, Solver::DiffsolRodas5p] {
        req.run.solver = solver;
        let fit = GaussianFitRequest {
            objective: GaussianObjectiveRequest {
                simulation: req.clone(),
                observations: times
                    .iter()
                    .enumerate()
                    .map(|(row, t)| GaussianObservation {
                        row,
                        output: "cp".into(),
                        value: q(
                            concentration(*t, 0.6) + if row % 2 == 0 { 0.03 } else { -0.03 },
                            "mg/L",
                        ),
                        error: GaussianError {
                            model: GaussianErrorModel::Normal,
                            additive_sd: q(0.2, "mg/L"),
                            proportional_sd: 0.,
                        },
                        censoring: None,
                    })
                    .collect(),
                priors: vec![],
            },
            bounds: vec![FitBound {
                parameter: "cl".into(),
                lower: q(0.03, "L/h"),
                upper: q(12., "L/h"),
            }],
            max_evaluations: 3000,
            max_iterations: 300,
            gradient_tolerance: 1e-5,
        };
        let result = c.fit_gaussian(&fit).unwrap();
        assert_eq!(
            result.status,
            FitStatus::Converged,
            "{solver:?}: {result:?}"
        );
        assert!(result.projected_gradient_norm <= fit.gradient_tolerance);
        assert!((result.parameters["cl"].value - 0.6).abs() < 0.03);
        let mut limited = fit.clone();
        limited.max_evaluations = 1;
        assert_eq!(
            c.fit_gaussian(&limited).unwrap().status,
            FitStatus::EvaluationLimit
        );
    }
}

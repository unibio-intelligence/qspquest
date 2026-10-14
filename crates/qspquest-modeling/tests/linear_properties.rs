//! Bounded deterministic properties for a linear, mass-conserving bolus model.
use qspquest_core::{
    regimen::ObservationSide,
    run::{RunRequest, RunResult, Solver},
};
use qspquest_modeling::document::CompiledDocument;
use serde_json::{json, Value};

const SEED: u64 = 0x5145_5354_2026_1011;
const DESIGNS: usize = 8;
const MODEL: &str = include_str!("../../../conformance/models/synthetic-one-compartment.json");

fn next(state: &mut u64) -> f64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 11) as f64 / ((1_u64 << 53) as f64)
}

fn model() -> String {
    let mut document: Value = serde_json::from_str(MODEL).unwrap();
    let elimination = document["states"][0]["rhs"]["arguments"][1].clone();
    document["states"].as_array_mut().unwrap().push(json!({
        "name":"eliminated", "unit":"mg", "initial":{"kind":"quantity","quantity":{"value":0,"unit":"mg"}},
        "rhs":elimination, "dosing":null
    }));
    document["outputs"].as_array_mut().unwrap().extend([
        json!({"name":"amount","unit":"mg","expression":{"kind":"symbol","name":"central"}}),
        json!({"name":"removed","unit":"mg","expression":{"kind":"symbol","name":"eliminated"}}),
    ]);
    document.to_string()
}

fn request(solver: Solver, cl: f64, volume: f64, amounts: &[f64]) -> RunRequest {
    let observations = [0.0, 0.5, 1.0, 2.0, 8.0, 24.0]
        .into_iter()
        .flat_map(|t| {
            ["pre", "post"]
                .into_iter()
                .map(move |side| json!({"time":{"value":t,"unit":"h"},"side":side}))
        })
        .collect::<Vec<_>>();
    let administrations = amounts
        .iter()
        .map(|amount| {
            json!({"target":"central","time":{"value":0,"unit":"h"},
        "amount":{"value":amount,"unit":"mg"},"delivery":{"kind":"bolus"},"bioavailability":1})
        })
        .collect::<Vec<_>>();
    serde_json::from_value(json!({
        "schema":"qspquest.run/v0.1", "solver":solver,
        "budgets":{"solver_callbacks":1000000,"events":100,"output_values":10000},
        "parameters":{"cl":{"value":cl,"unit":"L/h"},"v":{"value":volume,"unit":"L"}},
        "regimen":{"end":{"value":24,"unit":"h"}, "samples":[],
            "observations":observations, "administrations":administrations,
            "rtol":1e-10,"atol":1e-12}
    }))
    .unwrap()
}

fn values<'a>(result: &'a RunResult, name: &str) -> &'a [f64] {
    &result
        .outputs
        .iter()
        .find(|column| column.name == name)
        .unwrap()
        .values
}

fn report(label: &str, design: usize, result: &RunResult, error: f64) {
    println!(
        "{}",
        json!({"property":label,"seed":SEED,"design":design,
        "model_hash":result.identity.model_content_hash,"request_hash":result.identity.run_request_hash,
        "solver":result.identity.algorithm,"source":result.identity.compiler_source_sha256,
        "maximum_absolute_error":error,"absolute_limit":2e-7})
    );
    assert!(
        error <= 2e-7,
        "{label}: seed={SEED} design={design} error={error}"
    );
}

#[test]
fn generated_linear_conservation_superposition_and_unit_equivalence() {
    let compiled = CompiledDocument::from_json(&model()).unwrap();
    let mut state = SEED;
    for design in 0..DESIGNS {
        let cl = 0.1 + 0.9 * next(&mut state);
        let volume = 2.0 + 8.0 * next(&mut state);
        let first = 0.5 + 9.5 * next(&mut state);
        let second = 0.5 + 9.5 * next(&mut state);
        for solver in [Solver::DiffsolBdf, Solver::DiffsolRodas5p] {
            let base = request(solver, cl, volume, &[first + second]);
            let total = compiled.execute(&base).unwrap();
            let split = compiled
                .execute(&request(solver, cl, volume, &[first, second]))
                .unwrap();
            let a = compiled
                .execute(&request(solver, cl, volume, &[first]))
                .unwrap();
            let b = compiled
                .execute(&request(solver, cl, volume, &[second]))
                .unwrap();
            assert_eq!(
                (total.times.as_slice(), total.sides.as_slice()),
                (split.times.as_slice(), split.sides.as_slice())
            );
            let mut conservation: f64 = 0.0;
            let mut analytic: f64 = 0.0;
            let mut splitting: f64 = 0.0;
            let mut superposition: f64 = 0.0;
            for (row, &time) in total.times.iter().enumerate() {
                let administered = if time == 0.0 && total.sides[row] == ObservationSide::Pre {
                    0.0
                } else {
                    first + second
                };
                let amount = values(&total, "amount")[row];
                let removed = values(&total, "removed")[row];
                assert!(
                    amount >= -1e-10 && removed >= -1e-10,
                    "positivity: seed={SEED} design={design} row={row}"
                );
                conservation = conservation.max((amount + removed - administered).abs());
                analytic =
                    analytic.max((amount - administered * (-cl / volume * time).exp()).abs());
                splitting = splitting.max((amount - values(&split, "amount")[row]).abs());
                superposition = superposition
                    .max((amount - values(&a, "amount")[row] - values(&b, "amount")[row]).abs());
            }
            for (label, error) in [
                ("mass_conservation", conservation),
                ("independent_analytic", analytic),
                ("coincident_dose_splitting", splitting),
                ("linear_superposition", superposition),
            ] {
                report(label, design, &total, error);
            }
            let mut equivalent = base.clone();
            equivalent.parameters.get_mut("v").unwrap().value *= 1000.0;
            equivalent.parameters.get_mut("v").unwrap().unit = "mL".into();
            equivalent.parameters.get_mut("cl").unwrap().value *= 1000.0 / 60.0;
            equivalent.parameters.get_mut("cl").unwrap().unit = "mL/min".into();
            equivalent.regimen.administrations[0].amount.value *= 1000.0;
            equivalent.regimen.administrations[0].amount.unit = "ug".into();
            let converted = compiled.execute(&equivalent).unwrap();
            let error = values(&total, "amount")
                .iter()
                .zip(values(&converted, "amount"))
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f64, f64::max);
            report(
                "dimensionally_equivalent_request",
                design,
                &converted,
                error,
            );
            equivalent.parameters.get_mut("v").unwrap().unit = "mg".into();
            assert!(
                compiled.execute(&equivalent).is_err(),
                "mass cannot replace volume"
            );
        }
    }
}

#[test]
fn simultaneous_covariate_reset_lag_and_infusion_match_piecewise_analytic() {
    let mut document: Value = serde_json::from_str(&model()).unwrap();
    document["covariates"] = json!([{"name":"phase","unit":"1","interpolation":"step","default":{"value":1,"unit":"1"}}]);
    for index in 0..2 {
        let loss = if index == 0 {
            &mut document["states"][index]["rhs"]["arguments"][1]
        } else {
            &mut document["states"][index]["rhs"]
        };
        *loss = json!({"kind":"binary","operator":"multiply","arguments":[loss.clone(),{"kind":"symbol","name":"phase"}]});
    }
    document["outputs"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"phase","unit":"1","expression":{"kind":"symbol","name":"phase"}}));
    let compiled = CompiledDocument::from_json(&document.to_string()).unwrap();
    for solver in [Solver::DiffsolBdf, Solver::DiffsolRodas5p] {
        let mut request = serde_json::to_value(request(solver, 0.3, 3., &[6.])).unwrap();
        let observations = [0., 1., 1.5, 2., 3.]
            .into_iter()
            .flat_map(|t| {
                ["pre", "post"].map(|side| json!({"time":{"value":t,"unit":"h"},"side":side}))
            })
            .collect::<Vec<_>>();
        request["regimen"]["observations"] = json!(observations);
        request["regimen"]["covariate_changes"] =
            json!([{"name":"phase","time":{"value":1,"unit":"h"},"value":{"value":2,"unit":"1"}}]);
        request["regimen"]["resets"] = json!([{"target":"central","time":{"value":1,"unit":"h"},"value":{"value":5,"unit":"mg"},"order":0,"active_inputs":"continue"}]);
        request["regimen"]["administrations"].as_array_mut().unwrap().extend([
            json!({"target":"central","time":{"value":0.5,"unit":"h"},"lag":{"value":0.5,"unit":"h"},"amount":{"value":2,"unit":"mg"},"bioavailability":1,"delivery":{"kind":"bolus"}}),
            json!({"target":"central","time":{"value":1,"unit":"h"},"amount":{"value":2,"unit":"mg"},"bioavailability":1,"delivery":{"kind":"infusion","span":{"kind":"duration","duration":{"value":1,"unit":"h"}}}}),
        ]);
        let request: RunRequest = serde_json::from_value(request).unwrap();
        let result = compiled.execute(&request).unwrap();
        let mut error: f64 = 0.;
        let before_reset = 6. * (-0.1_f64).exp();
        let after_infusion = 7. * (-0.2_f64).exp() + 10. * (1. - (-0.2_f64).exp());
        for (i, &t) in result.times.iter().enumerate() {
            let pre = result.sides[i] == ObservationSide::Pre;
            let left = t < 1. || (t == 1. && pre);
            let expected = if t == 0. && pre {
                0.
            } else if left {
                6. * (-0.1 * t).exp()
            } else if t <= 2. {
                7. * (-0.2 * (t - 1.)).exp() + 10. * (1. - (-0.2 * (t - 1.)).exp())
            } else {
                after_infusion * (-0.2 * (t - 2.)).exp()
            };
            let delivered = if t == 0. && pre {
                0.
            } else if left {
                6.
            } else {
                6. + (5. - before_reset) + 2. + 2. * (t - 1.).min(1.)
            };
            error = error
                .max((values(&result, "amount")[i] - expected).abs())
                .max(
                    (values(&result, "amount")[i] + values(&result, "removed")[i] - delivered)
                        .abs(),
                );
            assert_eq!(values(&result, "phase")[i], if left { 1. } else { 2. });
        }
        report(
            "simultaneous_covariate_reset_lag_infusion",
            0,
            &result,
            error,
        );
        let mut refined = request.clone();
        refined.regimen.rtol *= 0.1;
        refined.regimen.atol = refined.regimen.atol.map(|value| value * 0.1);
        let tighter = compiled.execute(&refined).unwrap();
        let difference = values(&result, "amount")
            .iter()
            .zip(values(&tighter, "amount"))
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f64, f64::max);
        report("event_tolerance_refinement", 0, &tighter, difference);
    }
}

#[test]
fn generated_smooth_gradients_match_analytic_and_independent_central_differences() {
    use qspquest_core::run::SensitivityRequest;
    use qspquest_modeling::document::sensitivity::CompiledSensitivityDocument;
    let document = serde_json::from_str(MODEL).unwrap();
    let compiled = CompiledDocument::compile(&document).unwrap();
    let parameters = vec!["cl".into(), "v".into()];
    let derivatives = CompiledSensitivityDocument::compile(&document, &parameters).unwrap();
    let mut state = SEED;
    for design in 0..4 {
        let cl = 0.1 + 0.9 * next(&mut state);
        let volume = 2. + 8. * next(&mut state);
        let dose = 0.5 + 9.5 * next(&mut state);
        for solver in [Solver::DiffsolBdf, Solver::DiffsolRodas5p] {
            let run = request(solver, cl, volume, &[dose]);
            let sensitivity: SensitivityRequest = serde_json::from_value(json!({
                "schema":"qspquest.sensitivity/v0.1","run":run.clone(),"with_respect_to":parameters.clone(),
                "derivative_absolute_tolerances":{"cl":{"central":{"value":1e-12,"unit":"mg.h/L"}},"v":{"central":{"value":1e-12,"unit":"mg/L"}}}
            })).unwrap();
            let result = derivatives.run(&sensitivity).unwrap();
            let mut error: f64 = 0.;
            for parameter in ["cl", "v"] {
                let value = if parameter == "cl" { cl } else { volume };
                let delta = value * 1e-4;
                let mut plus = run.clone();
                let mut minus = run.clone();
                plus.parameters.get_mut(parameter).unwrap().value += delta;
                minus.parameters.get_mut(parameter).unwrap().value -= delta;
                let high = compiled.execute(&plus).unwrap();
                let low = compiled.execute(&minus).unwrap();
                let column = result
                    .derivatives
                    .iter()
                    .find(|c| c.parameter == parameter && c.output == "cp")
                    .unwrap();
                for (i, &time) in result.result.times.iter().enumerate() {
                    let cp = if time == 0. && result.result.sides[i] == ObservationSide::Pre {
                        0.
                    } else {
                        dose / volume * (-cl / volume * time).exp()
                    };
                    let exact = if parameter == "cl" {
                        -time / volume * cp
                    } else {
                        cp * (-1. / volume + cl * time / (volume * volume))
                    };
                    let finite_difference =
                        (values(&high, "cp")[i] - values(&low, "cp")[i]) / (2. * delta);
                    error = error
                        .max((column.values[i] - exact).abs())
                        .max((column.values[i] - finite_difference).abs());
                }
            }
            report(
                "smooth_analytic_and_central_difference_gradient",
                design,
                &result.result,
                error,
            );
        }
    }
}

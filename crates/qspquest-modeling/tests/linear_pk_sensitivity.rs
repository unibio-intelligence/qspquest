use qspquest_core::{
    expression::{Binary, Expr},
    model::*,
    regimen::*,
    run::*,
    ActiveInputs,
};
use qspquest_modeling::document::{sensitivity::CompiledSensitivityDocument, CompiledDocument};
use std::collections::BTreeMap;
fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}
fn fixture(count: usize, depot: bool) -> (ModelDocument, SensitivityRequest) {
    let mut d: ModelDocument = serde_json::from_str(include_str!(
        "../../../conformance/models/synthetic-linear-pk.json"
    ))
    .unwrap();
    let pk = d.linear_pk.as_mut().unwrap();
    pk.compartments.truncate(count);
    pk.exchange_clearances.truncate(count - 1);
    if !depot {
        pk.depot = None;
    }
    let states = pk
        .depot
        .iter()
        .map(|d| d.state.clone())
        .chain(pk.compartments.iter().map(|c| c.state.clone()))
        .collect::<Vec<_>>();
    d.outputs.retain(|o| states.contains(&o.name));
    d.outputs.push(Output {
        name: "concentration".into(),
        unit: "mg/L".into(),
        expression: Expr::binary(Binary::Divide, Expr::symbol("central"), Expr::symbol("vc")),
    });
    pk.clearance = "individual_cl".into();
    d.covariates.push(Covariate {
        name: "clearance_factor".into(),
        unit: "1".into(),
        interpolation: Interpolation::Constant,
        default: Some(q(1.2, "1")),
    });
    d.individual.push(IndividualParameter {
        name: "individual_cl".into(),
        expression: Expr::binary(
            Binary::Multiply,
            Expr::symbol("cl"),
            Expr::symbol("clearance_factor"),
        ),
        unit: Some("L/h".into()),
    });
    let mut run: RunRequest = serde_json::from_str(include_str!(
        "../../../conformance/requests/synthetic-linear-pk.json"
    ))
    .unwrap();
    run.solver = Solver::DiffsolBdf;
    run.budgets.output_values = 100000;
    if !depot {
        run.regimen.administrations[0].target = "central".into();
    }
    run.regimen.resets.push(StateReset {
        target: "central".into(),
        time: q(2., "h"),
        value: q(2., "mg"),
        order: 0,
        active_inputs: ActiveInputs::StopTarget,
    });
    let names = d.parameters.iter().map(|p| p.name.clone()).collect();
    let tolerances = d
        .parameters
        .iter()
        .map(|p| {
            (
                p.name.clone(),
                states
                    .iter()
                    .map(|s| (s.clone(), q(1e-11, &format!("mg/({})", p.default.unit))))
                    .collect(),
            )
        })
        .collect();
    (
        d,
        SensitivityRequest {
            schema: SensitivitySchema::V01,
            run,
            with_respect_to: names,
            derivative_absolute_tolerances: tolerances,
        },
    )
}
#[test]
fn one_to_three_compartment_sensitivities_match_perturbed_analytic_models() {
    for compartments in 1..=3 {
        for depot in [false, true] {
            let (d, request) = fixture(compartments, depot);
            let c = CompiledSensitivityDocument::compile(&d, &request.with_respect_to).unwrap();
            let result = c.run(&request).unwrap();
            let primal = CompiledDocument::compile(&d).unwrap();
            let mut analytic = request.run.clone();
            analytic.solver = Solver::AnalyticLinearPk;
            let reference = primal.execute(&analytic).unwrap();
            assert_eq!(
                result.result.identity.model_content_hash,
                reference.identity.model_content_hash
            );
            assert_eq!(result.result.times, reference.times);
            for (a, b) in result.result.outputs.iter().zip(&reference.outputs) {
                for (x, y) in a.values.iter().zip(&b.values) {
                    assert!((x - y).abs() < 2e-7);
                }
            }
            for (p, parameter) in d.parameters.iter().enumerate() {
                let h = 1e-4 * parameter.default.value.abs().max(1.);
                let mut hi = analytic.clone();
                let mut lo = analytic.clone();
                hi.parameters.insert(
                    parameter.name.clone(),
                    q(parameter.default.value + h, &parameter.default.unit),
                );
                lo.parameters.insert(
                    parameter.name.clone(),
                    q(parameter.default.value - h, &parameter.default.unit),
                );
                let hi = primal.execute(&hi).unwrap();
                let lo = primal.execute(&lo).unwrap();
                for o in 0..d.outputs.len() {
                    let column = &result.derivatives[p * d.outputs.len() + o];
                    assert_eq!(column.parameter, parameter.name);
                    for (i, (hi, lo)) in hi.outputs[o]
                        .values
                        .iter()
                        .zip(&lo.outputs[o].values)
                        .enumerate()
                    {
                        let fd = (hi - lo) / (2. * h);
                        assert!((column.values[i]-fd).abs()<2e-6,"{compartments} compartments depot {depot} parameter {} output {o} row {i}: {} vs {fd}",parameter.name,column.values[i]);
                    }
                }
            }
        }
    }
}
#[test]
fn linear_pk_sensitivity_keeps_coefficient_validation() {
    let (d, mut request) = fixture(3, true);
    let c = CompiledSensitivityDocument::compile(&d, &request.with_respect_to).unwrap();
    for (name, value, unit) in [("cl", -1., "mL/h"), ("vc", 0., "L"), ("ka", -0.1, "1/h")] {
        request.run.parameters = BTreeMap::from([(name.into(), q(value, unit))]);
        assert!(c.run(&request).is_err());
    }
    request.run.parameters.clear();
    assert!(c.run(&request).is_ok());
    request.run.solver = Solver::AnalyticLinearPk;
    assert!(c.run(&request).is_err());
}

#[test]
fn zero_rates_and_coincident_absorption_elimination_have_finite_sensitivities() {
    for (cl, ka) in [(0., 0.), (1000., 0.4)] {
        let (d, mut request) = fixture(1, true);
        request.run.parameters =
            BTreeMap::from([("cl".into(), q(cl, "mL/h")), ("ka".into(), q(ka, "1/h"))]);
        let c = CompiledSensitivityDocument::compile(&d, &request.with_respect_to).unwrap();
        let result = c.run(&request).unwrap();
        let primal = CompiledDocument::compile(&d).unwrap();
        let mut base = request.run.clone();
        base.solver = Solver::AnalyticLinearPk;
        let reference = primal.execute(&base).unwrap();
        for (name, h) in [("cl", 1e-2), ("ka", 1e-4)] {
            let p = request
                .with_respect_to
                .iter()
                .position(|n| n == name)
                .unwrap();
            let mut first = base.clone();
            let mut second = base.clone();
            first.parameters.get_mut(name).unwrap().value += h;
            second.parameters.get_mut(name).unwrap().value += 2. * h;
            let first = primal.execute(&first).unwrap();
            let second = primal.execute(&second).unwrap();
            for o in 0..d.outputs.len() {
                for i in 0..reference.times.len() {
                    let expected = (-3. * reference.outputs[o].values[i]
                        + 4. * first.outputs[o].values[i]
                        - second.outputs[o].values[i])
                        / (2. * h);
                    let actual = result.derivatives[p * d.outputs.len() + o].values[i];
                    assert!(actual.is_finite());
                    assert!(
                        (actual - expected).abs() < 1e-4,
                        "cl {cl} ka {ka} derivative {name} output {o}: {actual} vs {expected}"
                    );
                }
            }
        }
    }
}

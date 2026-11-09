//! Checked central-difference derivatives of complete native regimens.
use super::common::*;
use crate::document::CompiledDocument;
use serde_json::{json, Map, Value};
use std::sync::Arc;

/// Arguments accepted by `event_sensitivities`.
pub const SENSITIVITY_ARGUMENTS: [&str; 6] = [
    "run",
    "parameters",
    "relative_step",
    "relative_tolerance",
    "absolute_tolerance",
    "max_simulations",
];

/// Differentiate parameter-dependent lag and explicit event-time controls.
///
/// Runs the full event driver at two central-difference step sizes and reports
/// the half-step derivative only when they agree. Coincident observation/event
/// discontinuities, changing result grids and unstable derivatives fail with
/// no partial derivative. This is numerical differentiation, not saltation or
/// symbolic moving-boundary differentiation.
pub fn event_sensitivities(model: &Arc<CompiledDocument>, arguments: &Value) -> Result<Value> {
    let args = Arguments::new(arguments, &SENSITIVITY_ARGUMENTS)?;
    let run = args.required("run")?;
    let specs = parameter_specs(args.required("parameters")?)?;
    let relative_step = args.float("relative_step", 1e-3)?;
    let relative_tolerance = args.float("relative_tolerance", 0.02)?;
    let absolute_tolerance = args.float("absolute_tolerance", 1e-6)?;
    if !(0.0 < relative_step && relative_step < 0.1)
        || !(0.0 < relative_tolerance && relative_tolerance < 1.0)
        || absolute_tolerance <= 0.0
    {
        return fail("derivative steps and tolerances must be finite positive values");
    }
    let budget = Budget::new(args.int("max_simulations", 1000, 1, 1_000_000)?, None)?;
    let values: Vec<f64> = specs.iter().map(|s| s.initial).collect();
    let bound = |values: &[f64]| -> Result<Value> {
        bind(
            run,
            &specs.iter().zip(values.iter().copied()).collect::<Vec<_>>(),
        )
    };
    let baseline = budget.run(model, &bound(&values)?)?;
    let mut derivatives = Map::new();
    let mut errors = Map::new();
    for (index, spec) in specs.iter().enumerate() {
        let center = spec.initial;
        let h = relative_step * center.abs().max((spec.upper - spec.lower) * 0.1);
        if center - h <= spec.lower || center + h >= spec.upper {
            return fail(
                "central derivatives require an interior parameter with room for both steps",
            );
        }
        let mut approximations: Vec<Vec<Vec<f64>>> = Vec::new();
        for step in [h, h / 2.0] {
            let mut perturbed = Vec::new();
            for sign in [-1.0, 1.0] {
                let mut trial = values.clone();
                trial[index] += sign * step;
                let result = budget.run(model, &bound(&trial)?)?;
                if result.times != baseline.times || result.sides != baseline.sides {
                    return fail_code(
                        "moving event changes the observation grid; use fixed observation times away from events",
                        "unsupported",
                    );
                }
                if result.outputs.len() != baseline.outputs.len()
                    || result
                        .outputs
                        .iter()
                        .zip(&baseline.outputs)
                        .any(|(a, b)| a.0 != b.0 || a.1 != b.1)
                {
                    return fail("perturbed trajectory changes output names or units");
                }
                perturbed.push(result);
            }
            approximations.push(
                perturbed[1]
                    .outputs
                    .iter()
                    .zip(&perturbed[0].outputs)
                    .map(|(up, down)| {
                        up.2.iter()
                            .zip(&down.2)
                            .map(|(a, b)| (a - b) / (2.0 * step))
                            .collect()
                    })
                    .collect(),
            );
        }
        let mut by_output = Map::new();
        let mut error_by_output = Map::new();
        for (k, (name, unit, _)) in baseline.outputs.iter().enumerate() {
            let coarse = &approximations[0][k];
            let fine = &approximations[1][k];
            let difference: Vec<f64> = fine
                .iter()
                .zip(coarse)
                .map(|(a, b)| (a - b).abs())
                .collect();
            if fine.iter().any(|v| !v.is_finite())
                || difference
                    .iter()
                    .zip(fine)
                    .any(|(e, f)| *e > absolute_tolerance + relative_tolerance * f.abs())
            {
                return fail_code(
                    format!(
                        "derivative for {}/{name} is discontinuous or unresolved at the requested grid",
                        spec.name
                    ),
                    "unsupported",
                );
            }
            by_output.insert(
                name.clone(),
                json!({"values": fine, "unit": format!("{unit} / {}", spec.unit)}),
            );
            error_by_output.insert(name.clone(), json!(difference));
        }
        derivatives.insert(spec.name.clone(), Value::Object(by_output));
        errors.insert(spec.name.clone(), Value::Object(error_by_output));
    }
    let mut identity = identity(model, arguments)?;
    identity["native"] = baseline.identity.clone();
    Ok(json!({
        "schema": "qspquest.event-sensitivities/v0.2",
        "method": "checked_central_difference",
        "times": baseline.times,
        "sides": baseline.sides,
        "time_unit": baseline.time_unit,
        "derivatives": derivatives,
        "step_difference": errors,
        "simulations": budget.used(),
        "identity": identity,
    }))
}

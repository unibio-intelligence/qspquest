//! Log-space fixed Gaussian HMM inference. No fitting, sampling or calendar clock.
use qspquest_core::{hmm::*, identity, Error, ErrorCode};
use std::collections::BTreeSet;

pub const INPUT_BYTE_LIMIT: usize = 1_000_000;
const MAX_STATES: usize = 32;
const MAX_RECORDS: usize = 100_000;
const MAX_STATE_CELLS: usize = 250_000;
const MAX_TRANSITION_PRODUCTS: usize = 10_000_000;

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidInput, message)
}
fn finite(value: f64, label: &str) -> Result<f64, Error> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(Error::new(
            ErrorCode::Domain,
            format!("{label} is outside finite numerical range"),
        ))
    }
}
fn probability_row(row: &[f64], states: usize) -> Result<(), Error> {
    if row.len() != states
        || row
            .iter()
            .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
        || (row.iter().sum::<f64>() - 1.0).abs() > 1e-12
    {
        return Err(invalid("initial probabilities and every transition row must contain one probability per state and sum to one (tolerance 1e-12)"));
    }
    Ok(())
}
fn validate(model: &GaussianHmmModel, request: &GaussianHmmRequest) -> Result<(), Error> {
    let k = model.states.len();
    if model.schema != MODEL_SCHEMA || request.schema != REQUEST_SCHEMA {
        return Err(invalid("unsupported Gaussian HMM model or request schema"));
    }
    if k == 0 || k > MAX_STATES {
        return Err(invalid("Gaussian HMM requires 1 to 32 named states"));
    }
    let mut unique = BTreeSet::new();
    if model.model_id.trim().is_empty()
        || model.model_id.len() > 256
        || model
            .states
            .iter()
            .any(|s| s.trim().is_empty() || s.len() > 128 || !unique.insert(s))
        || model
            .endpoint_names
            .iter()
            .chain(&model.endpoint_units)
            .any(|s| s.trim().is_empty() || s.len() > 128)
        || model.endpoint_names[0] == model.endpoint_names[1]
    {
        return Err(invalid("model, state and endpoint labels must be nonempty and bounded; states and endpoint names must be distinct"));
    }
    probability_row(&model.initial_probability, k)?;
    if model.transition.len() != k || model.emissions.len() != k {
        return Err(invalid(
            "transition and emission dimensions must match named states",
        ));
    }
    for row in &model.transition {
        probability_row(row, k)?;
    }
    for e in &model.emissions {
        if e.mean.iter().any(|x| !x.is_finite())
            || e.variance.iter().any(|v| !v.is_finite() || *v <= 0.0)
            || !e.correlation.is_finite()
            || e.correlation.abs() >= 1.0
        {
            return Err(invalid("Gaussian means must be finite, variances positive finite, and correlations strictly between -1 and 1"));
        }
    }
    if model.lower_bounds.iter().flatten().any(|v| !v.is_finite())
        || model.first_entry_state.is_some_and(|i| i >= k)
    {
        return Err(invalid(
            "observation bounds must be finite and first-entry target must name an existing state",
        ));
    }
    let n = request.records.len();
    let prior = request.prior_record_count;
    if n == 0 && prior == 0 {
        return Err(invalid(
            "supply complete paired records or a positive prior_record_count",
        ));
    }
    if n > MAX_RECORDS
        || prior > MAX_RECORDS
        || (n + prior) * k > MAX_STATE_CELLS
        || (n + prior) * k * k > MAX_TRANSITION_PRODUCTS
    {
        return Err(Error::new(ErrorCode::WorkBudget,
            "HMM limit exceeded: 100000 records per grid, 250000 combined record-state cells, 10000000 transition products"));
    }
    let mut last_time = f64::NEG_INFINITY;
    if model
        .calendar_time_unit
        .as_ref()
        .is_some_and(|u| u.trim().is_empty() || u.len() > 128)
        || (model.calendar_time_unit.is_none() && request.records.iter().any(|r| r.time.is_some()))
    {
        return Err(invalid(
            "optional calendar labels require a nonempty calendar_time_unit (up to 128 bytes)",
        ));
    }
    for record in &request.records {
        if record.values.iter().any(|x| !x.is_finite()) {
            return Err(invalid(
                "each observation requires two finite numeric endpoint values",
            ));
        }
        if let Some(time) = record.time {
            if !time.is_finite() || time < last_time {
                return Err(invalid(
                    "optional calendar labels must be finite and nondecreasing",
                ));
            }
            last_time = time;
        }
    }
    Ok(())
}
fn logsum(values: impl Iterator<Item = f64>) -> f64 {
    let mut result = f64::NEG_INFINITY;
    for x in values {
        if x == f64::NEG_INFINITY {
            continue;
        }
        if result == f64::NEG_INFINITY {
            result = x;
            continue;
        }
        let (large, small) = if result > x { (result, x) } else { (x, result) };
        result = large + (small - large).exp().ln_1p();
    }
    result
}
fn emission(values: [f64; 2], e: &BivariateGaussian) -> Result<f64, Error> {
    let x = (values[0] - e.mean[0]) / e.variance[0].sqrt();
    let y = (values[1] - e.mean[1]) / e.variance[1].sqrt();
    let r = e.correlation;
    let residual = x - r * y;
    let quadratic = residual * residual / (1.0 - r * r) + y * y;
    finite(
        -std::f64::consts::TAU.ln()
            - 0.5 * (e.variance[0].ln() + e.variance[1].ln() + (-r * r).ln_1p() + quadratic),
        "emission log density",
    )
}
fn floored_mean(mean: f64, variance: f64, floor: Option<f64>) -> Result<f64, Error> {
    let Some(floor) = floor else {
        return Ok(mean);
    };
    let sd = variance.sqrt();
    let z = (floor - mean) / sd;
    if z.is_infinite() {
        return Ok(mean.max(floor));
    }
    let phi = (-0.5 * z * z).exp() / std::f64::consts::TAU.sqrt();
    let moment = if z >= 0.0 {
        floor + sd * (phi - z * 0.5 * libm::erfc(z / std::f64::consts::SQRT_2))
    } else {
        mean + sd * (phi + z * 0.5 * libm::erfc(-z / std::f64::consts::SQRT_2))
    };
    finite(moment, "floored Gaussian mean")
}
fn prior(model: &GaussianHmmModel, count: usize) -> Vec<HmmPriorResult> {
    let k = model.states.len();
    let mut weights = model.initial_probability.clone();
    let mut alive = weights.clone();
    let mut cdf = 0.0;
    if let Some(target) = model.first_entry_state {
        cdf = alive[target];
        alive[target] = 0.0;
    }
    let mut compensation = 0.0;
    let mut output = Vec::with_capacity(count);
    for index in 0..count {
        let survival: f64 = alive.iter().sum();
        output.push(HmmPriorResult {
            record_index: index,
            state_probability: weights.clone(),
            first_entry_survival: model.first_entry_state.map(|_| {
                if cdf <= 0.5 {
                    1.0 - cdf
                } else {
                    survival
                }
            }),
            first_entry_cdf: model.first_entry_state.map(|_| {
                if cdf <= 0.5 {
                    cdf
                } else {
                    1.0 - survival
                }
            }),
        });
        weights = (0..k)
            .map(|j| (0..k).map(|i| weights[i] * model.transition[i][j]).sum())
            .collect();
        let weight_sum: f64 = weights.iter().sum();
        for value in &mut weights {
            *value /= weight_sum;
        }
        if let Some(target) = model.first_entry_state {
            // Accumulate arrival mass directly: 1-survival loses tiny probabilities.
            let arrival = (0..k)
                .map(|i| alive[i] * model.transition[i][target])
                .sum::<f64>();
            let increment = arrival - compensation;
            let next = cdf + increment;
            compensation = (next - cdf) - increment;
            cdf = next;
            alive = (0..k)
                .map(|j| {
                    if j == target {
                        0.0
                    } else {
                        (0..k).map(|i| alive[i] * model.transition[i][j]).sum()
                    }
                })
                .collect();
        }
    }
    output
}

/// Execute fixed complete-pair inference and/or an integer-record prior grid.
pub fn execute(
    model: &GaussianHmmModel,
    request: &GaussianHmmRequest,
) -> Result<GaussianHmmResult, Error> {
    validate(model, request)?;
    let model_content_hash = identity::canonical_hash(
        &serde_json::to_value(model)
            .map_err(|e| invalid(format!("HMM identity serialization: {e}")))?,
    )?;
    let mut evaluated = model.clone();
    for row in
        std::iter::once(&mut evaluated.initial_probability).chain(evaluated.transition.iter_mut())
    {
        let sum: f64 = row.iter().sum();
        for value in row {
            *value /= sum;
        }
    }
    let model = &evaluated;
    let k = model.states.len();
    let n = request.records.len();
    let transitions: Vec<Vec<f64>> = model
        .transition
        .iter()
        .map(|row| row.iter().map(|x| x.ln()).collect())
        .collect();
    let mut moments = Vec::with_capacity(k);
    for e in &model.emissions {
        moments.push([
            floored_mean(e.mean[0], e.variance[0], model.lower_bounds[0])?,
            floored_mean(e.mean[1], e.variance[1], model.lower_bounds[1])?,
        ]);
    }
    let mut alpha: Vec<Vec<f64>> = Vec::with_capacity(n);
    let mut emissions: Vec<Vec<f64>> = Vec::with_capacity(n);
    let mut pointers: Vec<Vec<usize>> = Vec::with_capacity(n);
    let mut delta = Vec::new();
    let mut output = Vec::with_capacity(n);
    let mut cumulative = 0.0;
    for (index, record) in request.records.iter().enumerate() {
        let used = [
            model.lower_bounds[0].map_or(record.values[0], |f| f.max(record.values[0])),
            model.lower_bounds[1].map_or(record.values[1], |f| f.max(record.values[1])),
        ];
        let likelihood = model
            .emissions
            .iter()
            .map(|e| emission(used, e))
            .collect::<Result<Vec<_>, _>>()?;
        let predicted: Vec<f64> = if index == 0 {
            model.initial_probability.iter().map(|p| p.ln()).collect()
        } else {
            (0..k)
                .map(|j| logsum((0..k).map(|i| alpha[index - 1][i] + transitions[i][j])))
                .collect()
        };
        let joint: Vec<f64> = (0..k).map(|j| predicted[j] + likelihood[j]).collect();
        let scale = finite(logsum(joint.iter().copied()), "observation log likelihood")?;
        cumulative = finite(cumulative + scale, "sequence log likelihood")?;
        let posterior: Vec<f64> = joint.iter().map(|v| v - scale).collect();
        let mut back = vec![0; k];
        let next_delta = if index == 0 {
            joint
        } else {
            (0..k)
                .map(|j| {
                    let mut best = f64::NEG_INFINITY;
                    for i in 0..k {
                        let value = delta[i] + transitions[i][j];
                        if value > best {
                            best = value;
                            back[j] = i;
                        }
                    }
                    best + likelihood[j]
                })
                .collect()
        };
        let weights: Vec<f64> = predicted.iter().map(|v| v.exp()).collect();
        let predictive_mean = [
            finite(
                (0..k).map(|i| weights[i] * moments[i][0]).sum(),
                "predictive mean",
            )?,
            finite(
                (0..k).map(|i| weights[i] * moments[i][1]).sum(),
                "predictive mean",
            )?,
        ];
        output.push(HmmRecordResult {
            record_index: index,
            time: record.time,
            input: record.values,
            used,
            prior_state_probability: weights,
            filtered_state_probability: posterior.iter().map(|v| v.exp()).collect(),
            smoothed_state_probability: Vec::new(),
            emission_log_density: likelihood.clone(),
            incremental_log_likelihood: scale,
            cumulative_log_likelihood: cumulative,
            predictive_mean,
        });
        alpha.push(posterior);
        emissions.push(likelihood);
        pointers.push(back);
        delta = next_delta;
    }
    let mut beta = vec![0.0; k];
    for index in (0..n).rev() {
        let joint: Vec<f64> = (0..k).map(|j| alpha[index][j] + beta[j]).collect();
        let norm = logsum(joint.iter().copied());
        output[index].smoothed_state_probability = joint.iter().map(|v| (v - norm).exp()).collect();
        if index > 0 {
            let next: Vec<f64> = (0..k)
                .map(|i| logsum((0..k).map(|j| transitions[i][j] + emissions[index][j] + beta[j])))
                .collect();
            let offset = next.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            beta = next.iter().map(|v| v - offset).collect();
        }
    }
    let mut path = Vec::with_capacity(n);
    let mut best_joint = None;
    if n > 0 {
        let mut state = 0;
        for j in 1..k {
            if delta[j] > delta[state] {
                state = j;
            }
        }
        best_joint = Some(finite(delta[state], "Viterbi log joint")?);
        path.push(state);
        for index in (1..n).rev() {
            state = pointers[index][state];
            path.push(state);
        }
        path.reverse();
    }
    Ok(GaussianHmmResult { schema: RESULT_SCHEMA.into(), model_id: model.model_id.clone(),
        model_content_hash,
        request_content_hash: identity::canonical_hash(&serde_json::to_value(request).map_err(|e| invalid(e.to_string()))?)?,
        states: model.states.clone(), endpoint_names: model.endpoint_names.clone(), endpoint_units: model.endpoint_units.clone(),
        calendar_time_unit: model.calendar_time_unit.clone(),
        evaluated_initial_probability: model.initial_probability.clone(), evaluated_transition: model.transition.clone(),
        probability_policy: "Rows within 1e-12 of unit mass are divided by their sum before evaluation; raw model identity is preserved".into(),
        transition_coordinate: "adjacent_observation_record".into(), records: output,
        prior: prior(model, request.prior_record_count), log_likelihood: (n > 0).then_some(cumulative),
        viterbi_state_index: path, viterbi_log_joint: best_joint })
}

/// Shared bounded JSON entry point used unchanged by CLI and browser bindings.
pub fn execute_json(model: &str, request: &str) -> Result<String, Error> {
    if model.len() > INPUT_BYTE_LIMIT || request.len() > INPUT_BYTE_LIMIT {
        return Err(invalid(
            "HMM model and request must each fit the 1000000-byte input budget",
        ));
    }
    let parse = |e: serde_json::Error| invalid(format!("invalid Gaussian HMM JSON: {e}"));
    let model: GaussianHmmModel = serde_json::from_str(model).map_err(parse)?;
    let request: GaussianHmmRequest = serde_json::from_str(request).map_err(parse)?;
    serde_json::to_string(&execute(&model, &request)?).map_err(|e| invalid(e.to_string()))
}

//! Analytically integrated Gamma-Poisson counts with a bounded log-coordinate fit.
use qspquest_core::{
    discrete_fit::*, fit::FitStatus, identity::canonical_hash, model::Quantity, units::Unit, Error,
    ErrorCode,
};

const ALGORITHM: &str = "gamma_poisson_log_box_damped_newton_v1";
fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidInput, message)
}
fn positive(quantity: &Quantity, unit: Unit) -> Result<f64, Error> {
    let value = Unit::parse(&quantity.unit)?.convert(quantity.value, unit)?;
    if value <= 0. {
        return Err(invalid(
            "count exposures and Gamma rates must be finite and positive",
        ));
    }
    Ok(value)
}
fn softplus(x: f64) -> f64 {
    if x > 0. {
        x + libm::log1p(libm::exp(-x))
    } else {
        libm::log1p(libm::exp(x))
    }
}
struct Data {
    count: u32,
    log_exposure: f64,
    log_factorial: f64,
}
fn data(observations: &[CountExposure], unit: Unit) -> Result<Vec<Data>, Error> {
    if observations.is_empty() || observations.len() > 4096 {
        return Err(invalid(
            "Gamma-Poisson requires 1 through 4096 observations",
        ));
    }
    let total = observations
        .iter()
        .try_fold(0usize, |n, r| n.checked_add(r.count as usize))
        .ok_or_else(|| invalid("count total exceeds the supported work limit"))?;
    if total > 1_000_000 {
        return Err(invalid(
            "Gamma-Poisson supports at most one million total observed events",
        ));
    }
    observations
        .iter()
        .map(|row| {
            Ok(Data {
                count: row.count,
                log_exposure: libm::log(positive(&row.exposure, unit)?),
                log_factorial: libm::lgamma(row.count as f64 + 1.),
            })
        })
        .collect()
}
fn evaluate(point: [f64; 2], data: &[Data]) -> Result<GammaPoissonObjective, Error> {
    let shape = libm::exp(point[0]);
    let mut value = 0.;
    let mut gradient = [0.; 2];
    let mut hessian = [[0.; 2]; 2];
    for row in data {
        let difference = row.log_exposure - point[1];
        let log_q = softplus(difference);
        let log_p = softplus(-difference);
        let p = libm::exp(-log_q);
        let q = libm::exp(-log_p);
        let mut rising = 0.;
        let mut inverse = 0.;
        let mut inverse_squared = 0.;
        for k in 0..row.count {
            let term = shape + k as f64;
            let reciprocal = 1. / term;
            rising += libm::log(term);
            inverse += reciprocal;
            inverse_squared += reciprocal * reciprocal;
        }
        let count = row.count as f64;
        value += row.log_factorial - rising + shape * log_q + count * log_p;
        let a = shape * (log_q - inverse);
        gradient[0] += a;
        gradient[1] += count * p - shape * q;
        hessian[0][0] += a + shape * shape * inverse_squared;
        hessian[0][1] -= shape * q;
        hessian[1][1] += (shape + count) * p * q;
    }
    hessian[1][0] = hessian[0][1];
    if !value.is_finite()
        || gradient
            .iter()
            .chain(hessian.iter().flatten())
            .any(|v| !v.is_finite())
    {
        return Err(Error::new(
            ErrorCode::Domain,
            "Gamma-Poisson likelihood or derivatives overflow",
        ));
    }
    Ok(GammaPoissonObjective {
        negative_log_likelihood: value,
        log_gradient: gradient,
        log_hessian: hessian,
    })
}
/// Evaluate the normalized likelihood and analytic derivatives in log(shape)
/// and log(rate in the supplied rate unit). Exposure quantities are converted.
pub fn gamma_poisson_objective(
    shape: f64,
    rate: &Quantity,
    observations: &[CountExposure],
) -> Result<GammaPoissonObjective, Error> {
    if !shape.is_finite() || !(1e-8..=1e6).contains(&shape) {
        return Err(invalid("Gamma shape must be in [1e-8, 1e6]"));
    }
    let unit = Unit::parse(&rate.unit)?;
    let rate = positive(rate, unit)?;
    evaluate(
        [libm::log(shape), libm::log(rate)],
        &data(observations, unit)?,
    )
}
fn projected(point: [f64; 2], gradient: [f64; 2], lower: [f64; 2], upper: [f64; 2]) -> [f64; 2] {
    std::array::from_fn(|i| {
        if (point[i] == lower[i] && gradient[i] > 0.) || (point[i] == upper[i] && gradient[i] < 0.)
        {
            0.
        } else {
            gradient[i]
        }
    })
}
fn norm(value: [f64; 2]) -> f64 {
    value[0].abs().max(value[1].abs())
}
pub fn fit_gamma_poisson(request: &GammaPoissonFitRequest) -> Result<DiscreteFitResult, Error> {
    let [shape_lo, shape_hi] = request.shape_bounds;
    if ![shape_lo, shape_hi, request.initial_shape]
        .iter()
        .all(|v| v.is_finite())
        || shape_lo < 1e-8
        || shape_hi > 1e6
        || shape_lo >= shape_hi
        || request.initial_shape < shape_lo
        || request.initial_shape > shape_hi
        || request.max_evaluations == 0
        || request.max_evaluations > 10000
        || request.max_iterations == 0
        || request.max_iterations > 10000
        || !request.gradient_tolerance.is_finite()
        || request.gradient_tolerance <= 0.
    {
        return Err(invalid("Gamma-Poisson needs a finite positive shape box containing the start and bounded positive optimizer controls"));
    }
    let unit = Unit::parse(&request.initial_rate.unit)?;
    let rate = positive(&request.initial_rate, unit)?;
    let rate_lo = positive(&request.rate_bounds[0], unit)?;
    let rate_hi = positive(&request.rate_bounds[1], unit)?;
    if rate_lo >= rate_hi || rate < rate_lo || rate > rate_hi {
        return Err(invalid(
            "Gamma rate box must have positive width and contain the start",
        ));
    }
    let data = data(&request.observations, unit)?;
    let work =
        data.iter().map(|row| row.count as u64 + 1).sum::<u64>() * request.max_evaluations as u64;
    if data.len() < 2 || work > 20_000_000 {
        return Err(invalid("Gamma-Poisson fit requires at least two rows and at most twenty million count-loop operations"));
    }
    let lower = [libm::log(shape_lo), libm::log(rate_lo)];
    let upper = [libm::log(shape_hi), libm::log(rate_hi)];
    if (0..2).any(|i| lower[i] >= upper[i]) {
        return Err(invalid(
            "Gamma bounds are too narrow for distinct log coordinates",
        ));
    }
    let mut point = [libm::log(request.initial_shape), libm::log(rate)];
    let mut value = evaluate(point, &data)?;
    let mut evaluations = 1;
    let mut iterations = 0;
    let status = loop {
        let g = projected(point, value.log_gradient, lower, upper);
        if norm(g) <= request.gradient_tolerance {
            break FitStatus::Converged;
        }
        if evaluations >= request.max_evaluations {
            break FitStatus::EvaluationLimit;
        }
        if iterations >= request.max_iterations {
            break FitStatus::IterationLimit;
        }
        let h = value.log_hessian;
        let smallest = 0.5 * (h[0][0] + h[1][1] - libm::hypot(h[0][0] - h[1][1], 2. * h[0][1]));
        let damping = (1e-6 * (1. + h[0][0].abs().max(h[1][1].abs())) - smallest).max(0.);
        let a = h[0][0] + damping;
        let b = h[1][1] + damping;
        let c = h[0][1];
        let determinant = a * b - c * c;
        let mut direction = if determinant.is_finite() && determinant > 0. {
            [
                (-b * g[0] + c * g[1]) / determinant,
                (c * g[0] - a * g[1]) / determinant,
            ]
        } else {
            [-g[0], -g[1]]
        };
        for i in 0..2 {
            if g[i] == 0. && (point[i] == lower[i] || point[i] == upper[i]) {
                direction[i] = 0.;
                let j = 1 - i;
                direction[j] = -g[j] / h[j][j].max(1.);
            }
        }
        if direction.iter().any(|v| !v.is_finite())
            || direction[0] * g[0] + direction[1] * g[1] >= 0.
        {
            direction = [-g[0], -g[1]];
        }
        let mut alpha = 1.;
        let mut accepted = None;
        for _ in 0..48 {
            if evaluations >= request.max_evaluations {
                break;
            }
            let next = std::array::from_fn(|i| {
                (point[i] + alpha * direction[i]).clamp(lower[i], upper[i])
            });
            let slope = (0..2)
                .map(|i| value.log_gradient[i] * (next[i] - point[i]))
                .sum::<f64>();
            if slope >= 0. || next == point {
                alpha *= 0.5;
                continue;
            }
            evaluations += 1;
            let candidate = evaluate(next, &data)?;
            let rounding = 32. * f64::EPSILON * (1. + value.negative_log_likelihood.abs());
            if candidate.negative_log_likelihood <= value.negative_log_likelihood + 1e-4 * slope
                || (slope.abs() <= rounding
                    && candidate.negative_log_likelihood
                        <= value.negative_log_likelihood + rounding
                    && norm(projected(next, candidate.log_gradient, lower, upper)) < norm(g))
            {
                accepted = Some((next, candidate));
                break;
            }
            alpha *= 0.5;
        }
        let Some((next, candidate)) = accepted else {
            break if evaluations >= request.max_evaluations {
                FitStatus::EvaluationLimit
            } else {
                FitStatus::LineSearchFailed
            };
        };
        point = next;
        value = candidate;
        iterations += 1;
    };
    let projected_gradient_norm = norm(projected(point, value.log_gradient, lower, upper));
    let parameters = [
        (
            "shape".into(),
            Quantity {
                value: libm::exp(point[0]).clamp(shape_lo, shape_hi),
                unit: "1".into(),
            },
        ),
        (
            "rate".into(),
            Quantity {
                value: libm::exp(point[1]).clamp(rate_lo, rate_hi),
                unit: request.initial_rate.unit.clone(),
            },
        ),
    ]
    .into_iter()
    .collect();
    let active_bounds = (0..2)
        .filter(|&i| point[i] == lower[i] || point[i] == upper[i])
        .map(|i| ["shape", "rate"][i].into())
        .collect();
    Ok(DiscreteFitResult {
        schema: DiscreteFitResultSchema::V01,
        status,
        likelihood: "gamma_poisson".into(),
        parameters,
        negative_log_likelihood: value.negative_log_likelihood,
        observations: data.len(),
        evaluations,
        iterations,
        projected_gradient_norm,
        active_bounds,
        algorithm: ALGORITHM.into(),
        fit_request_hash: canonical_hash(
            &serde_json::json!({"algorithm":ALGORITHM,"request":request}),
        )?,
        compiler_source_sha256: env!("QSPQUEST_SOURCE_SHA256").into(),
    })
}

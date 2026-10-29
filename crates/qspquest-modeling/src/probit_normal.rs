//! Bounded normal-mixture binomial fitting in mean and variance coordinates.
use qspquest_core::probit_fit::{
    BinomialLitter as Litter, ProbitNormalBinomialFitRequest as Request,
    ProbitNormalBinomialFitResult as Fit, ProbitNormalObjective as Objective,
    ProbitNormalResultSchema,
};
use qspquest_core::{fit::FitStatus, identity::canonical_hash};
use qspquest_core::{Error, ErrorCode};
const LOG_TWO_PI: f64 = 1.8378770664093453;
const QUADRATURE_LIMIT: usize = 65536;
const ALGORITHM: &str = "probit_normal_binomial_variance_newton_tail_refinement_v1";
fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidInput, message)
}
fn numerical(message: &str) -> Error {
    Error::new(ErrorCode::Domain, message)
}

// The continued fraction follows directly from integration by parts of the
// normal tail. It avoids subtracting a probability from one in the far tail.
fn normal(x: f64) -> (f64, f64) {
    let log_phi = -0.5 * x * x - 0.5 * LOG_TWO_PI;
    if x < -8. {
        let t = -x;
        let mut fraction = 0.;
        for k in (1..=64).rev() {
            fraction = k as f64 / (t + fraction);
        }
        let mills = t + fraction;
        (log_phi - libm::log(mills), mills)
    } else {
        let cdf = 0.5 * libm::erfc(-x / std::f64::consts::SQRT_2);
        let log_cdf = libm::log(cdf);
        (log_cdf, libm::exp(log_phi - log_cdf))
    }
}
fn mills_derivatives(x: f64, r: f64) -> [f64; 4] {
    let first = -r * (x + r);
    let a = (x + r) * (x + 2. * r) - 1.;
    let second = r * a;
    let third = first * a + r * ((1. + first) * (x + 2. * r) + (x + r) * (1. + 2. * first));
    [r, first, second, third]
}
struct Conditional {
    log: f64,
    score: f64,
    curvature: f64,
    moments: [f64; 6],
}
fn conditional(eta: f64, row: &Litter) -> Conditional {
    let (lp, r) = normal(eta);
    let (lm, m) = normal(-eta);
    let p = mills_derivatives(eta, r);
    let q = mills_derivatives(-eta, m);
    let y = row.successes as f64;
    let f = (row.total - row.successes) as f64;
    let s = y * p[0] - f * q[0];
    let h = y * p[1] + f * q[1];
    let t = y * p[2] - f * q[2];
    let u = y * p[3] + f * q[3];
    let a = s * s + h;
    let b = s * s * s + 3. * s * h + t;
    let c = s.powi(4) + 6. * s * s * h + 3. * h * h + 4. * s * t + u;
    Conditional {
        log: y * lp + f * lm,
        score: s,
        curvature: h,
        moments: [1., s, 0.5 * a, a, 0.5 * b, 0.25 * c],
    }
}
struct Posterior<'a> {
    mean: f64,
    sd: f64,
    row: &'a Litter,
    evaluations: usize,
    limit: usize,
}
impl Posterior<'_> {
    fn at(&mut self, z: f64) -> Result<(f64, f64, f64, [f64; 6]), Error> {
        if self.evaluations >= self.limit {
            return Err(Error::new(
                ErrorCode::WorkBudget,
                "probit-normal quadrature work limit exceeded",
            ));
        }
        self.evaluations += 1;
        let c = conditional(self.mean + self.sd * z, self.row);
        Ok((
            c.log - 0.5 * z * z - 0.5 * LOG_TWO_PI,
            self.sd * c.score - z,
            1. - self.sd * self.sd * c.curvature,
            c.moments,
        ))
    }
}
// Derive the Gauss-Legendre rule from its polynomial recurrence.
fn legendre() -> Vec<(f64, f64)> {
    let n = 16;
    let mut rule = Vec::new();
    for k in 1..=n {
        let mut x = libm::cos(std::f64::consts::PI * (k as f64 - 0.25) / (n as f64 + 0.5));
        let mut derivative = 0.;
        for _ in 0..20 {
            let mut p0 = 1.;
            let mut p1 = x;
            for j in 2..=n {
                let p = ((2 * j - 1) as f64 * x * p1 - (j - 1) as f64 * p0) / j as f64;
                p0 = p1;
                p1 = p;
            }
            derivative = n as f64 * (x * p1 - p0) / (x * x - 1.);
            let next = x - p1 / derivative;
            if (next - x).abs() < 2e-16 {
                x = next;
                break;
            }
            x = next;
        }
        rule.push((x, 2. / ((1. - x * x) * derivative * derivative)));
    }
    rule
}
fn tail_bound(log_relative: f64, slope: f64, eta: f64, sd: f64, total: u32) -> f64 {
    // r(x)<=1+|x| bounds all conditional moments through order four by
    // 86*n^4*(1+|eta|)^4. Integrate that polynomial against the exponential
    // tangent envelope of the strictly log-concave posterior, to infinity.
    let u = 1. + eta.abs();
    let coefficients = [1., 4., 12., 24., 24.];
    let polynomial = (0..5)
        .map(|k| {
            coefficients[k] * u.powi(4 - k as i32) * sd.powi(k as i32) / slope.powi(k as i32 + 1)
        })
        .sum::<f64>();
    libm::exp(log_relative) * 86. * (total as f64).powi(4) * polynomial
}
fn row_objective(
    mean: f64,
    variance: f64,
    row: &Litter,
    rule: &[(f64, f64)],
    limit: usize,
) -> Result<(f64, [f64; 2], [[f64; 2]; 2], usize, f64), Error> {
    if limit == 0 {
        return Err(Error::new(
            ErrorCode::WorkBudget,
            "probit-normal quadrature work limit exceeded",
        ));
    }
    let choose = libm::lgamma(row.total as f64 + 1.)
        - libm::lgamma(row.successes as f64 + 1.)
        - libm::lgamma((row.total - row.successes) as f64 + 1.);
    if variance == 0. {
        let c = conditional(mean, row);
        let m = c.moments;
        let g = [m[1], m[2]];
        let h = [
            [m[3] - g[0] * g[0], m[4] - g[0] * g[1]],
            [m[4] - g[0] * g[1], m[5] - g[1] * g[1]],
        ];
        return Ok((
            -choose - c.log,
            [-g[0], -g[1]],
            h.map(|r| r.map(|v| -v)),
            1,
            0.,
        ));
    }
    let mut p = Posterior {
        mean,
        sd: libm::sqrt(variance),
        row,
        evaluations: 0,
        limit: limit.min(QUADRATURE_LIMIT),
    };
    let mut lo = -1.;
    let mut hi = 1.;
    for _ in 0..16 {
        if p.at(lo)?.1 > 0. {
            break;
        }
        lo *= 2.;
    }
    for _ in 0..16 {
        if p.at(hi)?.1 < 0. {
            break;
        }
        hi *= 2.;
    }
    if p.at(lo)?.1 <= 0. || p.at(hi)?.1 >= 0. {
        return Err(numerical(
            "normal-mixture posterior mode could not be bracketed",
        ));
    }
    let mut mode = 0f64.clamp(lo, hi);
    for _ in 0..100 {
        let (_, d, k, _) = p.at(mode)?;
        if d.abs() <= 1e-12 * (1. + mode.abs()) {
            break;
        }
        if d > 0. {
            lo = mode;
        } else {
            hi = mode;
        }
        let next = mode + d / k;
        mode = if next > lo && next < hi {
            next
        } else {
            0.5 * (lo + hi)
        };
    }
    let (log_mode, d, k, _) = p.at(mode)?;
    if d.abs() > 1e-9 * (1. + mode.abs()) || k <= 0. {
        return Err(numerical("normal-mixture posterior mode did not converge"));
    }
    let delta = 1. / libm::sqrt(k);
    let mut left = mode - delta;
    let mut right = mode + delta;
    let l0 = p.at(left)?.0;
    let r0 = p.at(right)?.0;
    let lower_mass = 2. * delta * libm::exp(l0.min(r0) - log_mode);
    let tail_goal = 1e-13 * lower_mass;
    let mut left_bound = f64::INFINITY;
    let mut right_bound = f64::INFINITY;
    for j in 0..30 {
        let (l, d, _, _) = p.at(left)?;
        if d > 0. {
            left_bound = tail_bound(l - log_mode, d, mean + p.sd * left, p.sd, row.total);
        }
        if left_bound <= tail_goal {
            break;
        }
        left = mode - delta * 2f64.powi(j + 1);
    }
    for j in 0..30 {
        let (l, d, _, _) = p.at(right)?;
        if d < 0. {
            right_bound = tail_bound(l - log_mode, -d, mean + p.sd * right, p.sd, row.total);
        }
        if right_bound <= tail_goal {
            break;
        }
        right = mode + delta * 2f64.powi(j + 1);
    }
    if left_bound > tail_goal || right_bound > tail_goal {
        return Err(numerical(
            "normal-mixture unbounded-tail mass could not be bounded",
        ));
    }
    let mut previous: Option<[f64; 6]> = None;
    for power in 0..=9 {
        let panels = 1 << power;
        let mut sum = [0.; 6];
        for (a, b) in [(left, mode), (mode, right)] {
            let width = (b - a) / panels as f64;
            for panel in 0..panels {
                let center = a + (panel as f64 + 0.5) * width;
                for &(x, w) in rule {
                    let (log, _, _, moments) = p.at(center + 0.5 * width * x)?;
                    let weight = 0.5 * width * w * libm::exp(log - log_mode);
                    for i in 0..6 {
                        sum[i] += weight * moments[i];
                    }
                }
            }
        }
        if !sum[0].is_finite() || sum[0] <= 0. || sum.iter().any(|v| !v.is_finite()) {
            return Err(numerical("normal-mixture integral or derivatives overflow"));
        }
        let normalized = sum.map(|v| v / sum[0]);
        if let Some(old) = previous {
            let refinement = (0..6)
                .map(|i| (sum[i] - old[i]).abs() / (sum[0] + sum[i].abs()))
                .fold(0f64, f64::max);
            if refinement <= 1e-11 {
                let g = [normalized[1], normalized[2]];
                let h = [
                    [normalized[3] - g[0] * g[0], normalized[4] - g[0] * g[1]],
                    [normalized[4] - g[0] * g[1], normalized[5] - g[1] * g[1]],
                ];
                return Ok((
                    -choose - log_mode - libm::log(sum[0]),
                    [-g[0], -g[1]],
                    h.map(|r| r.map(|v| -v)),
                    p.evaluations,
                    refinement,
                ));
            }
        }
        previous = Some(sum);
    }
    Err(numerical(
        "normal-mixture quadrature refinement did not converge",
    ))
}
/// Coordinates are [mean_0, mean_1, variance_0, variance_1]. The normal
/// distribution is unbounded; tail envelopes include all derivative moments.
pub fn probit_normal_binomial_objective(
    point: [f64; 4],
    observations: &[Litter],
) -> Result<Objective, Error> {
    limited_objective(point, observations, 128 * QUADRATURE_LIMIT)
}
fn limited_objective(
    point: [f64; 4],
    observations: &[Litter],
    limit: usize,
) -> Result<Objective, Error> {
    if point[..2].iter().any(|v| !v.is_finite() || v.abs() > 8.)
        || point[2..]
            .iter()
            .any(|v| !v.is_finite() || *v < 0. || *v > 9.)
        || observations.is_empty()
        || observations.len() > 128
        || observations
            .iter()
            .any(|r| r.group > 1 || r.total == 0 || r.total > 1000 || r.successes > r.total)
    {
        return Err(invalid("probit-normal needs two means in [-8,8], variances in [0,9], and 1..128 valid litters of 1..1000 trials"));
    }
    let rule = legendre();
    let mut out = Objective {
        negative_log_likelihood: 0.,
        gradient: [0.; 4],
        hessian: [[0.; 4]; 4],
        quadrature_evaluations: 0,
        quadrature_refinement: 0.,
    };
    for row in observations {
        let i = row.group;
        let j = i + 2;
        let (f, g, h, n, error) = row_objective(
            point[i],
            point[j],
            row,
            &rule,
            limit - out.quadrature_evaluations,
        )?;
        out.negative_log_likelihood += f;
        out.gradient[i] += g[0];
        out.gradient[j] += g[1];
        out.hessian[i][i] += h[0][0];
        out.hessian[i][j] += h[0][1];
        out.hessian[j][i] += h[1][0];
        out.hessian[j][j] += h[1][1];
        out.quadrature_evaluations += n;
        out.quadrature_refinement = out.quadrature_refinement.max(error);
    }
    Ok(out)
}
fn project(x: [f64; 4], g: [f64; 4], lo: [f64; 4], hi: [f64; 4]) -> [f64; 4] {
    std::array::from_fn(|i| {
        if (x[i] == lo[i] && g[i] > 0.) || (x[i] == hi[i] && g[i] < 0.) {
            0.
        } else {
            g[i]
        }
    })
}
fn norm(g: [f64; 4]) -> f64 {
    g.iter().map(|v| v.abs()).fold(0f64, f64::max)
}
pub fn fit_probit_normal_binomial(request: &Request) -> Result<Fit, Error> {
    let mut x = [0.; 4];
    let mut lo = [0.; 4];
    let mut hi = [0.; 4];
    for i in 0..2 {
        let [a, b] = request.mean_bounds[i];
        let [c, d] = request.sd_bounds[i];
        if [a, b, c, d, request.initial_means[i], request.initial_sds[i]]
            .iter()
            .any(|v| !v.is_finite())
            || a < -8.
            || b > 8.
            || a >= b
            || c < 0.
            || d > 3.
            || c >= d
            || request.initial_means[i] < a
            || request.initial_means[i] > b
            || request.initial_sds[i] < c
            || request.initial_sds[i] > d
        {
            return Err(invalid("probit-normal bounds must contain the starts: means within [-8,8] and SDs within [0,3]"));
        }
        x[i] = request.initial_means[i];
        x[i + 2] = request.initial_sds[i].powi(2);
        lo[i] = a;
        hi[i] = b;
        lo[i + 2] = c * c;
        hi[i + 2] = d * d;
        if lo[i + 2] >= hi[i + 2] {
            return Err(invalid(
                "SD bounds are too narrow for distinct variance coordinates",
            ));
        }
    }
    if request.max_evaluations == 0
        || request.max_quadrature_evaluations == 0
        || request.max_quadrature_evaluations > 20_000_000
        || request.max_evaluations > 1000
        || request.max_iterations == 0
        || request.max_iterations > 1000
        || !request.gradient_tolerance.is_finite()
        || request.gradient_tolerance <= 0.
        || (0..2).any(|g| request.observations.iter().filter(|r| r.group == g).count() < 2)
    {
        return Err(invalid("probit-normal fit needs at least two litters per group and finite positive bounded optimizer controls"));
    }
    let mut v = limited_objective(x, &request.observations, request.max_quadrature_evaluations)?;
    let mut evaluations = 1;
    let mut iterations = 0;
    let mut quadrature_evaluations = v.quadrature_evaluations;
    let status = loop {
        let g = project(x, v.gradient, lo, hi);
        if norm(g) <= request.gradient_tolerance {
            break FitStatus::Converged;
        }
        if evaluations >= request.max_evaluations {
            break FitStatus::EvaluationLimit;
        }
        if iterations >= request.max_iterations {
            break FitStatus::IterationLimit;
        }
        let mut direction = [0.; 4];
        for i in 0..2 {
            let j = i + 2;
            let h = &v.hessian;
            let smallest = 0.5 * (h[i][i] + h[j][j] - libm::hypot(h[i][i] - h[j][j], 2. * h[i][j]));
            let damping = (1e-6 * (1. + h[i][i].abs().max(h[j][j].abs())) - smallest).max(0.);
            let a = h[i][i] + damping;
            let b = h[j][j] + damping;
            let c = h[i][j];
            let det = a * b - c * c;
            if det > 0. && det.is_finite() {
                direction[i] = (-b * g[i] + c * g[j]) / det;
                direction[j] = (c * g[i] - a * g[j]) / det;
            } else {
                direction[i] = -g[i];
                direction[j] = -g[j];
            }
            for (k, l) in [(i, j), (j, i)] {
                if g[k] == 0. && (x[k] == lo[k] || x[k] == hi[k]) {
                    direction[k] = 0.;
                    direction[l] = -g[l] / h[l][l].max(1.);
                }
            }
        }
        if direction.iter().any(|v| !v.is_finite())
            || (0..4).map(|i| g[i] * direction[i]).sum::<f64>() >= 0.
        {
            direction = g.map(|v| -v);
        }
        let mut alpha = 1.;
        let mut accepted = None;
        for _ in 0..48 {
            if evaluations >= request.max_evaluations {
                break;
            }
            let next = std::array::from_fn(|i| (x[i] + alpha * direction[i]).clamp(lo[i], hi[i]));
            let slope = (0..4)
                .map(|i| v.gradient[i] * (next[i] - x[i]))
                .sum::<f64>();
            if slope >= 0. || next == x {
                alpha *= 0.5;
                continue;
            }
            evaluations += 1;
            let candidate = limited_objective(
                next,
                &request.observations,
                request.max_quadrature_evaluations - quadrature_evaluations,
            )?;
            quadrature_evaluations += candidate.quadrature_evaluations;
            let rounding = 32. * f64::EPSILON * (1. + v.negative_log_likelihood.abs());
            if candidate.negative_log_likelihood <= v.negative_log_likelihood + 1e-4 * slope
                || (slope.abs() <= rounding
                    && candidate.negative_log_likelihood <= v.negative_log_likelihood + rounding
                    && norm(project(next, candidate.gradient, lo, hi)) < norm(g))
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
        x = next;
        v = candidate;
        iterations += 1;
    };
    Ok(Fit {
        schema: ProbitNormalResultSchema::V01,
        status,
        means: [x[0], x[1]],
        sds: std::array::from_fn(|i| {
            libm::sqrt(x[i + 2]).clamp(request.sd_bounds[i][0], request.sd_bounds[i][1])
        }),
        negative_log_likelihood: v.negative_log_likelihood,
        projected_gradient_norm: norm(project(x, v.gradient, lo, hi)),
        evaluations,
        iterations,
        quadrature_evaluations,
        observations: request.observations.len(),
        active_bounds: (0..4).filter(|&i| x[i] == lo[i] || x[i] == hi[i]).collect(),
        compiler_source_sha256: env!("QSPQUEST_SOURCE_SHA256").into(),
        algorithm: ALGORITHM.into(),
        fit_request_hash: canonical_hash(
            &serde_json::json!({"algorithm":ALGORITHM,"request":request}),
        )?,
    })
}

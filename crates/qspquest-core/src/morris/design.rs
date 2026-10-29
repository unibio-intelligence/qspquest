use super::*;
use crate::{
    identity::canonical_hash,
    scan::{ScanPoint, ScanSchema},
    units::Unit,
    Error, ErrorCode,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidInput, message)
}
// Versioned deterministic hash-counter stream; independent of platform RNGs.
struct Stream {
    seed: u32,
    counter: u64,
}
impl Stream {
    fn below(&mut self, n: u64) -> Result<u64, Error> {
        let threshold = n.wrapping_neg() % n;
        for _ in 0..256 {
            let mut hash = Sha256::new();
            hash.update(b"qspquest.morris.sha256-counter.v1");
            hash.update(self.seed.to_le_bytes());
            hash.update(self.counter.to_le_bytes());
            self.counter = self
                .counter
                .checked_add(1)
                .ok_or_else(|| invalid("Morris generator counter exhausted"))?;
            let digest = hash.finalize();
            let value = u64::from_le_bytes(digest[..8].try_into().unwrap());
            if value >= threshold {
                return Ok(value % n);
            }
        }
        Err(invalid("Morris random draw rejection limit exceeded"))
    }
}
/// Generate ungrouped, unoptimized Morris trajectories without running a model.
pub fn generate_morris_design(input: &MorrisDesignRequest) -> Result<GeneratedMorrisDesign, Error> {
    let d = input.ranges.len();
    let p = input.num_levels;
    if d == 0
        || d > 11
        || input.trajectory_count < 2
        || input.trajectory_count > 12
        || input.trajectory_count * (d + 1) > 25
        || p < 4
        || p > 100
        || p % 2 != 0
    {
        return Err(invalid("Morris design requires 1..11 parameters, at least two trajectories, at most 25 points and even 4..100 grid levels"));
    }
    let mut names = BTreeSet::new();
    let mut bounds = Vec::new();
    for r in &input.ranges {
        if r.parameter.is_empty() || !names.insert(&r.parameter) {
            return Err(invalid(
                "Morris parameter names must be nonempty and unique",
            ));
        }
        let unit = Unit::parse(&r.lower.unit)?;
        let upper_unit = Unit::parse(&r.upper.unit)?;
        let lo = r.lower.value;
        let hi = upper_unit.convert(r.upper.value, unit)?;
        let span = hi - lo;
        if !lo.is_finite() || !hi.is_finite() || !span.is_finite() || span <= 0. {
            return Err(invalid("Morris ranges must be finite and increasing"));
        }
        bounds.push((lo, hi, span));
    }
    let mut stream = Stream {
        seed: input.seed,
        counter: 0,
    };
    let mut points = Vec::new();
    let mut trajectories = Vec::new();
    for t in 0..input.trajectory_count {
        let mut order: Vec<_> = (0..d).collect();
        for i in (1..d).rev() {
            let j = stream.below((i + 1) as u64)? as usize;
            order.swap(i, j);
        }
        let mut grid = Vec::new();
        let mut direction = Vec::new();
        for _ in 0..d {
            let base = stream.below((p / 2) as u64)? as i32;
            let sign = if stream.below(2)? == 0 { 1 } else { -1 };
            grid.push(base + if sign < 0 { (p / 2) as i32 } else { 0 });
            direction.push(sign);
        }
        let mut path = Vec::new();
        for step in 0..=d {
            if step > 0 {
                let i = order[step - 1];
                grid[i] += direction[i] * (p / 2) as i32;
            }
            let mut parameters = std::collections::BTreeMap::new();
            for (i, r) in input.ranges.iter().enumerate() {
                let (lo, hi, span) = bounds[i];
                let value = if grid[i] == 0 {
                    lo
                } else if grid[i] == (p - 1) as i32 {
                    hi
                } else {
                    lo + span * (grid[i] as f64 / (p - 1) as f64)
                };
                if !value.is_finite() {
                    return Err(invalid("Morris generated quantity overflow"));
                }
                parameters.insert(
                    r.parameter.clone(),
                    Quantity {
                        value,
                        unit: r.lower.unit.clone(),
                    },
                );
            }
            let id = format!("trajectory_{t}_step_{step}");
            path.push(id.clone());
            points.push(ScanPoint { id, parameters });
        }
        trajectories.push(path);
    }
    let generator = "qspquest_morris_sha256_counter_v1".to_owned();
    Ok(GeneratedMorrisDesign {
        design_request_hash: canonical_hash(
            &serde_json::json!({"generator":generator,"request":input}),
        )?,
        generator,
        seed: input.seed,
        request: MorrisRequest {
            scan: ScanRequest {
                schema: ScanSchema::V01,
                run: input.run.clone(),
                points,
                total_budgets: input.total_budgets,
            },
            ranges: input.ranges.clone(),
            trajectories,
            num_levels: p,
            output: input.output.clone(),
            row: input.row,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_design_has_known_grid_and_current_generator_identity() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test-data/conformance/requests/synthetic-morris.json"
        )))
        .unwrap();
        let mut input: MorrisDesignRequest =
            serde_json::from_value(fixture["problem"]["request"].clone()).unwrap();
        for range in &mut input.ranges {
            range.lower.value = 0.;
            range.upper.value = 3.;
        }
        let design = generate_morris_design(&input).unwrap();
        assert_eq!(design.generator, "qspquest_morris_sha256_counter_v1");
        assert_eq!(design.seed, 42);
        let grid: Vec<_> = design
            .request
            .scan
            .points
            .iter()
            .map(|point| (point.parameters["cl"].value, point.parameters["v"].value))
            .collect();
        assert_eq!(
            grid,
            vec![(0., 3.), (0., 1.), (2., 1.), (2., 1.), (0., 1.), (0., 3.)]
        );
        assert_eq!(
            design.request.trajectories,
            vec![
                vec![
                    "trajectory_0_step_0",
                    "trajectory_0_step_1",
                    "trajectory_0_step_2"
                ],
                vec![
                    "trajectory_1_step_0",
                    "trajectory_1_step_1",
                    "trajectory_1_step_2"
                ],
            ]
        );
        assert_eq!(
            design.design_request_hash,
            canonical_hash(&serde_json::json!({
                "generator": "qspquest_morris_sha256_counter_v1", "request": input
            }))
            .unwrap()
        );
        assert_eq!(
            serde_json::to_value(&design).unwrap(),
            serde_json::to_value(generate_morris_design(&input).unwrap()).unwrap()
        );
        input.seed += 1;
        let different = generate_morris_design(&input).unwrap();
        assert_ne!(design.design_request_hash, different.design_request_hash);
        assert_ne!(
            serde_json::to_value(design.request.scan.points).unwrap(),
            serde_json::to_value(different.request.scan.points).unwrap()
        );
    }
}

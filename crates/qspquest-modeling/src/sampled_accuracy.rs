//! Convergence checks on complete sampled trajectories within one smooth interval.
pub(crate) fn refine_sampled_trajectory<E>(
    rtol: f64,
    atol: &[f64],
    maximum_passes: usize,
    agreement_fraction: f64,
    mut solve: impl FnMut(f64) -> Result<Vec<Vec<f64>>, E>,
    map_error: impl Fn(String) -> E,
) -> Result<(), E> {
    if !rtol.is_finite()
        || rtol <= 0.0
        || atol.is_empty()
        || atol.iter().any(|v| !v.is_finite() || *v <= 0.0)
        || !agreement_fraction.is_finite()
        || agreement_fraction <= 0.0
        || agreement_fraction > 1.0
        || !(2..=16).contains(&maximum_passes)
    {
        return Err(map_error(
            "invalid sampled refinement tolerances or pass limit".into(),
        ));
    }
    let mut previous: Option<Vec<Vec<f64>>> = None;
    let mut previous_change: Option<f64> = None;
    let mut stalled = 0;
    let mut factor = 1.0;
    for pass in 1..=maximum_passes {
        let current = solve(factor)?;
        let width = current.first().map_or(0, Vec::len);
        if width == 0
            || (atol.len() != 1 && atol.len() != width)
            || current
                .iter()
                .any(|row| row.len() != width || row.iter().any(|v| !v.is_finite()))
        {
            return Err(map_error("invalid sampled refinement trajectory".into()));
        }
        if let Some(coarse) = &previous {
            if coarse.len() != current.len() || coarse[0].len() != width {
                return Err(map_error(
                    "sampled refinement trajectory shape changed".into(),
                ));
            }
            let mut maximum = 0.0_f64;
            for (a, b) in coarse.iter().zip(&current) {
                for (i, (a, b)) in a.iter().zip(b).enumerate() {
                    let scale =
                        atol[if atol.len() == 1 { 0 } else { i }] + rtol * a.abs().max(b.abs());
                    let change = (a - b).abs() / scale;
                    if !scale.is_finite() || !change.is_finite() {
                        return Err(map_error(
                            "non-finite sampled refinement error scale".into(),
                        ));
                    }
                    maximum = maximum.max(change);
                }
            }
            if maximum <= agreement_fraction {
                return Ok(());
            }
            if let Some(before) = previous_change {
                if pass >= 4 && before <= 1.0 && maximum > 8.0 * before {
                    return Err(map_error(format!("sampled refinement worsened at pass {pass}: change {maximum:e} tolerance units (previous {before:e}); local tolerance factor {factor:e}")));
                }
                stalled = if maximum >= 0.95 * before {
                    stalled + 1
                } else {
                    0
                };
                if stalled >= 3 {
                    return Err(map_error(format!("sampled refinement stalled at pass {pass}: change {maximum:e} tolerance units; local tolerance factor {factor:e}")));
                }
            }
            previous_change = Some(maximum);
        }
        previous = Some(current);
        factor *= 0.1;
    }
    Err(map_error(format!("sampled trajectory did not converge within {maximum_passes} refinement passes; last change {:e} tolerance units", previous_change.unwrap_or(f64::NAN))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn converges_on_samples_with_tighter_local_tolerances() {
        let mut passes = 0;
        refine_sampled_trajectory(
            1e-6,
            &[1e-8],
            8,
            0.25,
            |factor| {
                passes += 1;
                Ok(vec![vec![1.0 + factor * 1e-4]])
            },
            |s| s,
        )
        .unwrap();
        assert!(passes >= 4);
    }
    #[test]
    fn persistent_disagreement_and_shape_changes_fail_closed() {
        let mut count = 0;
        let result = refine_sampled_trajectory(
            1e-8,
            &[1e-10],
            8,
            0.25,
            |_| {
                count += 1;
                Ok(vec![vec![count as f64]])
            },
            |s| s,
        );
        assert!(result.is_err());
        let mut count = 0;
        let result = refine_sampled_trajectory(
            1e-8,
            &[1e-10],
            8,
            0.25,
            |_| {
                count += 1;
                Ok(vec![vec![1.0; count]])
            },
            |s| s,
        );
        assert!(result.unwrap_err().contains("shape changed"));
    }
}

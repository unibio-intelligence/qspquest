//! Asymptotic log-coordinate intervals for positive population fixed effects.
use crate::fit::{FitStatus, PopulationFitResult, PopulationUncertaintyMethod};
use crate::{Error, ErrorCode};
use serde::{Deserialize, Serialize};

const Z95: f64 = 1.959963984540054;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositiveFixedEffectIntervals {
    pub method: PositiveFixedEffectIntervalMethod,
    pub confidence_level: f64,
    pub critical_value: f64,
    pub intervals: Vec<PositiveFixedEffectInterval>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositiveFixedEffectIntervalMethod {
    PositiveFixedEffectLogWaldDeltaV1,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositiveFixedEffectInterval {
    pub name: String,
    pub unit: String,
    pub estimate: f64,
    pub source_standard_error: f64,
    pub log_coordinate_standard_error: f64,
    pub lower: f64,
    pub upper: f64,
}

/// The caller must declare parameters whose domain is strictly positive.
/// A positive estimate alone does not establish that domain. The entire supplied
/// observed covariance, including nuisance parameters, must be positive definite.
/// These are local asymptotic intervals; coverage for another design is unproved.
pub fn positive_fixed_effect_intervals(
    population: &PopulationFitResult,
    positive_parameters: &[String],
) -> Result<PositiveFixedEffectIntervals, Error> {
    let invalid = |message| Error::new(ErrorCode::InvalidInput, message);
    if positive_parameters.is_empty()
        || positive_parameters.iter().any(|name| name.is_empty())
        || positive_parameters
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != positive_parameters.len()
    {
        return Err(invalid(
            "declare distinct positive fixed-effect parameter names",
        ));
    }
    if population.status != FitStatus::Converged {
        return Err(invalid("intervals require a converged population fit"));
    }
    let uncertainty = population.uncertainty.as_ref().ok_or_else(|| {
        invalid("observed-information uncertainty is required; a refusal provides no interval")
    })?;
    if uncertainty.method != PopulationUncertaintyMethod::SaemMarginalObservedHessian {
        return Err(invalid(
            "log-Wald intervals require SAEM marginal observed-information uncertainty",
        ));
    }
    let size = uncertainty.parameters.len();
    let covariance = &uncertainty.covariance;
    let errors = &uncertainty.standard_errors;
    let unique = uncertainty
        .parameters
        .iter()
        .map(|p| &p.name)
        .collect::<std::collections::BTreeSet<_>>();
    if size == 0
        || unique.len() != size
        || unique.iter().any(|name| name.is_empty())
        || errors.len() != size
        || covariance.len() != size
        || covariance
            .iter()
            .any(|row| row.len() != size || row.iter().any(|v| !v.is_finite()))
    {
        return Err(invalid(
            "covariance, parameter names and standard errors must have matching dimensions",
        ));
    }
    let mut lower = vec![vec![0.; size]; size];
    for i in 0..size {
        if covariance[i][i] <= 0.
            || !errors[i].is_finite()
            || errors[i] <= 0.
            || (errors[i] - covariance[i][i].sqrt()).abs()
                > 1e-8 * errors[i].abs().max(covariance[i][i].sqrt())
        {
            return Err(invalid(
                "standard errors must agree with positive covariance diagonals",
            ));
        }
        for j in 0..=i {
            if (covariance[i][j] - covariance[j][i]).abs()
                > 1e-12_f64.max(1e-10 * covariance[i][j].abs().max(covariance[j][i].abs()))
            {
                return Err(invalid("covariance must be symmetric"));
            }
            let value = covariance[i][j] - (0..j).map(|k| lower[i][k] * lower[j][k]).sum::<f64>();
            if i == j {
                if !value.is_finite() || value <= 0. {
                    return Err(invalid("covariance must be positive definite"));
                }
                lower[i][j] = value.sqrt();
            } else {
                lower[i][j] = value / lower[j][j];
            }
        }
    }
    let mut intervals = Vec::with_capacity(positive_parameters.len());
    for name in positive_parameters {
        let index = uncertainty
            .parameters
            .iter()
            .position(|p| &p.name == name)
            .ok_or_else(|| {
                invalid("declared fixed effect must have observed-information uncertainty")
            })?;
        let fixed = population
            .fixed_effects
            .get(name)
            .ok_or_else(|| invalid("declared parameter must be a fitted fixed effect"))?;
        if !fixed.value.is_finite() || fixed.value <= 0. {
            return Err(invalid(
                "a log-coordinate interval requires a finite positive estimate",
            ));
        }
        if fixed.unit.is_empty() || fixed.unit != uncertainty.parameters[index].unit {
            return Err(invalid("fixed-effect and uncertainty units must agree"));
        }
        let log_error = errors[index] / fixed.value;
        let center = fixed.value.ln();
        let low = (center - Z95 * log_error).exp();
        let high = (center + Z95 * log_error).exp();
        if !log_error.is_finite() || low <= 0. || low >= high || !high.is_finite() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "transformed interval exceeds finite, strictly positive floating-point range",
            ));
        }
        intervals.push(PositiveFixedEffectInterval {
            name: name.clone(),
            unit: fixed.unit.clone(),
            estimate: fixed.value,
            source_standard_error: errors[index],
            log_coordinate_standard_error: log_error,
            lower: low,
            upper: high,
        });
    }
    Ok(PositiveFixedEffectIntervals {
        method: PositiveFixedEffectIntervalMethod::PositiveFixedEffectLogWaldDeltaV1,
        confidence_level: 0.95,
        critical_value: Z95,
        intervals,
    })
}

/// Attach optional interval information without discarding a completed fit.
pub fn attach_positive_fixed_effect_intervals(
    population: &mut PopulationFitResult,
    positive_parameters: &[String],
) {
    match positive_fixed_effect_intervals(population, positive_parameters) {
        Ok(intervals) => {
            population.fixed_effect_intervals = Some(intervals);
            population.fixed_effect_interval_error = None;
        }
        Err(error) => {
            population.fixed_effect_intervals = None;
            population.fixed_effect_interval_error = Some(error);
        }
    }
}

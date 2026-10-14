//! Fixed, unit-bearing administration expansion. No parameter-dependent event times.
use crate::{model::Quantity, units::Unit, Error, ErrorCode, Event};
use serde::{Deserialize, Serialize};
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Delivery {
    Bolus,
    Infusion { span: InfusionSpan },
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InfusionSpan {
    Duration { duration: Quantity },
    Rate { rate: Quantity },
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repeat {
    pub interval: Quantity,
    pub additional: u32,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Administration {
    pub target: String,
    pub time: Quantity,
    pub amount: Quantity,
    pub delivery: Delivery,
    pub repeat: Option<Repeat>,
    pub lag: Option<Quantity>,
    pub bioavailability: f64,
}
#[derive(Clone, Debug)]
pub struct Target {
    pub modes: Vec<crate::model::DeliveryMode>,
    pub name: String,
    pub state: usize,
    pub amount_unit: Unit,
}
/// A validated administration occurrence before model-specific dose scaling.
/// Amounts use the target's declared input unit; times use the run time unit.
/// `administered_amount` precedes bioavailability, while `event.amount()` is
/// the delivered amount. Keep both: zero bioavailability loses information.
#[derive(Clone, Debug)]
pub struct ExpandedAdministration {
    pub administration_index: usize,
    pub repetition_index: u32,
    pub scheduled_time: f64,
    pub administered_amount: f64,
    pub event: Event,
}
fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidInput, message)
}
fn convert(q: &Quantity, to: Unit) -> Result<f64, Error> {
    Unit::parse(&q.unit)?.convert(q.value, to)
}
/// Expand before solver dispatch. All expanded events must fit inside the run.
/// Infusion duration is derived from the administered amount/rate before F scales
/// delivery, so F changes exposure without changing administration timing.
pub fn expand(
    administrations: &[Administration],
    targets: &[Target],
    time_unit: Unit,
    end: f64,
    event_limit: usize,
) -> Result<Vec<Event>, Error> {
    expand_in_window(administrations, targets, time_unit, 0.0, end, event_limit)
}
/// Expand against an absolute-time window. Initial state is supplied at start.
pub fn expand_in_window(
    administrations: &[Administration],
    targets: &[Target],
    time_unit: Unit,
    start_time: f64,
    end: f64,
    event_limit: usize,
) -> Result<Vec<Event>, Error> {
    Ok(expand_administrations_in_window(
        administrations,
        targets,
        time_unit,
        start_time,
        end,
        event_limit,
    )?
    .into_iter()
    .map(|record| record.event)
    .collect())
}
/// Expand with original dose information retained for history-aware consumers.
/// Validation, event limits, unit conversion and ordering are shared with
/// `expand_in_window`; no caller needs to reconstruct an amount by dividing by F.
pub fn expand_administrations_in_window(
    administrations: &[Administration],
    targets: &[Target],
    time_unit: Unit,
    start_time: f64,
    end: f64,
    event_limit: usize,
) -> Result<Vec<ExpandedAdministration>, Error> {
    expand_administrations_internal(
        administrations,
        targets,
        time_unit,
        start_time,
        end,
        event_limit,
        false,
    )
}
/// Periodic operation only: retain infusion tails beyond the right boundary.
/// The periodic kernel validates and wraps these before normal execution.
pub fn expand_periodic_in_window(
    administrations: &[Administration],
    targets: &[Target],
    time_unit: Unit,
    start_time: f64,
    end: f64,
    event_limit: usize,
) -> Result<Vec<Event>, Error> {
    Ok(expand_administrations_internal(
        administrations,
        targets,
        time_unit,
        start_time,
        end,
        event_limit,
        true,
    )?
    .into_iter()
    .map(|r| r.event)
    .collect())
}
fn expand_administrations_internal(
    administrations: &[Administration],
    targets: &[Target],
    time_unit: Unit,
    start_time: f64,
    end: f64,
    event_limit: usize,
    periodic: bool,
) -> Result<Vec<ExpandedAdministration>, Error> {
    time_unit.conversion_to(Unit::parse("s")?)?;
    if !start_time.is_finite() || !end.is_finite() || end <= start_time {
        return Err(invalid("run window must be finite and increasing"));
    }
    // Count in a fixed width on native and wasm32, before any expansion allocation.
    let count = administrations.iter().try_fold(0u64, |count, a| {
        count
            .checked_add(a.repeat.as_ref().map_or(1, |r| u64::from(r.additional) + 1))
            .ok_or_else(|| Error::new(ErrorCode::WorkBudget, "event count overflow"))
    })?;
    if count > event_limit as u64 {
        return Err(Error::new(
            ErrorCode::WorkBudget,
            "expanded administration count exceeds event budget",
        ));
    }
    let count = count as usize; // bounded above by a usize event_limit
    let mut names = std::collections::BTreeSet::new();
    for t in targets {
        crate::model::validate_delivery_modes(&t.modes)?;
        if t.name.is_empty() || !names.insert(&t.name) {
            return Err(invalid("duplicate or empty regimen target"));
        }
    }
    let mut result = Vec::with_capacity(count);
    for (index, a) in administrations.iter().enumerate() {
        let one = || -> Result<Vec<ExpandedAdministration>, Error> {
            let target = targets
                .iter()
                .find(|t| t.name == a.target)
                .ok_or_else(|| invalid("unknown administration target"))?;
            let mode = match a.delivery {
                Delivery::Bolus => crate::model::DeliveryMode::Bolus,
                Delivery::Infusion { .. } => crate::model::DeliveryMode::Infusion,
            };
            if !target.modes.contains(&mode) {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "delivery mode is prohibited for this target",
                ));
            }
            let time = convert(&a.time, time_unit)?;
            let lag = a
                .lag
                .as_ref()
                .map(|q| convert(q, time_unit))
                .transpose()?
                .unwrap_or(0.0);
            let amount = convert(&a.amount, target.amount_unit)?;
            if lag < 0.0
                || amount < 0.0
                || !a.bioavailability.is_finite()
                || !(0.0..=1.0).contains(&a.bioavailability)
            {
                return Err(invalid(
                    "time, lag, amount or bioavailability is outside its domain",
                ));
            }
            let interval = a
                .repeat
                .as_ref()
                .map(|r| convert(&r.interval, time_unit))
                .transpose()?;
            if interval.is_some_and(|ii| ii <= 0.0) {
                return Err(invalid("repeat interval must be positive"));
            }
            let duration = match &a.delivery {
                Delivery::Bolus => None,
                Delivery::Infusion { span } => {
                    let duration = match span {
                        InfusionSpan::Duration { duration } => convert(duration, time_unit)?,
                        InfusionSpan::Rate { rate } => {
                            let rate = convert(rate, target.amount_unit.divide(time_unit)?)?;
                            if rate <= 0.0 || amount <= 0.0 {
                                return Err(invalid(
                                    "rate-defined infusion requires positive amount and rate",
                                ));
                            }
                            amount / rate
                        }
                    };
                    if !duration.is_finite() || duration <= 0.0 {
                        return Err(invalid("infusion duration must be finite and positive"));
                    }
                    Some(duration)
                }
            };
            let delivered = amount * a.bioavailability;
            if !delivered.is_finite() {
                return Err(invalid("delivered amount overflow"));
            }
            let repetitions = u64::from(a.repeat.as_ref().map_or(0, |r| r.additional)) + 1;
            let mut expanded = Vec::with_capacity(repetitions as usize);
            let mut previous = None;
            for i in 0..repetitions {
                let scheduled = time + (i as f64) * interval.unwrap_or(0.0);
                let start = scheduled + lag;
                if start < start_time
                    || !start.is_finite()
                    || start > end
                    || previous.is_some_and(|prev| start <= prev)
                    || (lag > 0.0 && start <= scheduled)
                {
                    return Err(invalid(
                        "expanded dose time overflows, collapses or exceeds the run window",
                    ));
                }
                previous = Some(start);
                let event = if let Some(duration) = duration {
                    let stop = start + duration;
                    if !stop.is_finite() || stop <= start || (!periodic && stop > end) {
                        return Err(invalid(
                            "infusion stop overflows, collapses or exceeds the run window",
                        ));
                    }
                    Event::Infusion {
                        time: start,
                        target: target.state,
                        amount: delivered,
                        duration,
                    }
                } else {
                    Event::Bolus {
                        time: start,
                        target: target.state,
                        amount: delivered,
                    }
                };
                expanded.push(ExpandedAdministration {
                    administration_index: index,
                    repetition_index: i as u32,
                    scheduled_time: scheduled,
                    administered_amount: amount,
                    event,
                });
            }
            Ok(expanded)
        };
        result.extend(one().map_err(|mut e| {
            e.expression = Some(format!("administrations[{index}]"));
            e
        })?);
    }
    // Stable ordering preserves explicitly simultaneous records; no epsilon merge.
    result.sort_by(|a, b| a.event.time().total_cmp(&b.event.time()));
    Ok(result)
}

/// Unit-bearing times and administrations with the existing scalar tolerance contract.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    #[serde(default)]
    pub start: Option<Quantity>,
    pub end: Quantity,
    /// Fixed solver restart times; these do not change state or parameters.
    #[serde(default)]
    pub checkpoints: Vec<Quantity>,
    pub samples: Vec<Quantity>,
    pub observations: Option<Vec<ObservationPoint>>,
    pub administrations: Vec<Administration>,
    #[serde(default)]
    pub resets: Vec<StateReset>,
    #[serde(default)]
    pub covariates: std::collections::BTreeMap<String, Quantity>,
    #[serde(default)]
    pub covariate_changes: Vec<CovariateChange>,
    pub rtol: f64,
    /// Legacy scalar in each state's numeric unit; exclusive with absolute_tolerances.
    pub atol: Option<f64>,
    /// Exactly one positive quantity for each state, identified by name.
    pub absolute_tolerances: Option<std::collections::BTreeMap<String, Quantity>>,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateReset {
    pub target: String,
    pub time: Quantity,
    pub value: Quantity,
    pub order: u32,
    pub active_inputs: crate::ActiveInputs,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSide {
    Pre,
    Post,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPoint {
    pub time: Quantity,
    pub side: ObservationSide,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CovariateChange {
    pub name: String,
    pub time: Quantity,
    pub value: Quantity,
}

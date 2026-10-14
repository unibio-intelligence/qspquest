//! Immutable per-target administration history, independent of solver stepping.
mod predicates;
use crate::{
    regimen::{ExpandedAdministration, ObservationSide},
    Error, ErrorCode, Event,
};
pub use predicates::{PredicateBoundary, PredicateSchedule};
use std::collections::BTreeMap;

/// Explicit policy for multiple administration records at exactly the same time.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimultaneousDose {
    First,
    Last,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoseHistoryValue {
    pub administered_amount: f64,
    pub delivery_time: f64,
    pub scheduled_time: f64,
    pub elapsed: f64,
    pub administration_index: usize,
    pub repetition_index: u32,
}

#[derive(Clone, Debug)]
pub struct DoseHistory {
    targets: BTreeMap<usize, Vec<ExpandedAdministration>>,
}
fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidInput, message)
}
impl DoseHistory {
    /// Records must already use common run-time and per-target input units.
    /// This owns its records: queries cannot mutate history or depend on the
    /// order in which an adaptive solver requests past and future times.
    pub fn new(
        records: Vec<ExpandedAdministration>,
        simultaneous: SimultaneousDose,
    ) -> Result<Self, Error> {
        let mut targets: BTreeMap<usize, Vec<ExpandedAdministration>> = BTreeMap::new();
        for record in records {
            let time = record.event.time();
            if !time.is_finite()
                || !record.scheduled_time.is_finite()
                || record.scheduled_time > time
                || !record.administered_amount.is_finite()
                || record.administered_amount < 0.0
            {
                return Err(invalid("invalid administration history metadata"));
            }
            match &record.event {
                Event::Reset { .. } => {
                    return Err(Error::new(
                        ErrorCode::Unsupported,
                        "reset history requires an explicit policy",
                    ));
                }
                Event::Bolus { amount, .. } | Event::Infusion { amount, .. }
                    if !amount.is_finite() || *amount < 0.0 =>
                {
                    return Err(invalid("invalid delivered history amount"));
                }
                Event::Infusion { duration, .. }
                    if !duration.is_finite()
                        || *duration <= 0.0
                        || !(time + duration).is_finite()
                        || time + duration <= time =>
                {
                    return Err(invalid("invalid history infusion span"));
                }
                _ => {}
            }
            targets
                .entry(record.event.target())
                .or_default()
                .push(record);
        }
        for records in targets.values_mut() {
            // Finite times were checked above. Numeric comparison keeps +0 and -0
            // in input order because they denote the same scientific time.
            records.sort_by(|a, b| a.event.time().partial_cmp(&b.event.time()).unwrap());
            let mut selected: Vec<ExpandedAdministration> = Vec::with_capacity(records.len());
            for record in records.drain(..) {
                if selected
                    .last()
                    .is_some_and(|last| last.event.time() == record.event.time())
                {
                    if simultaneous == SimultaneousDose::Last {
                        *selected.last_mut().unwrap() = record;
                    }
                } else {
                    selected.push(record);
                }
            }
            *records = selected;
        }
        Ok(Self { targets })
    }
    /// `Pre` excludes every administration at the query time; `Post` includes
    /// the selected simultaneous record. No prior administration returns None,
    /// never a fabricated zero dose/time or a non-finite numeric sentinel.
    pub fn at(
        &self,
        target: usize,
        time: f64,
        side: ObservationSide,
    ) -> Result<Option<DoseHistoryValue>, Error> {
        if !time.is_finite() {
            return Err(invalid("history query time must be finite"));
        }
        let Some(records) = self.targets.get(&target) else {
            return Ok(None);
        };
        let index = records.partition_point(|r| match side {
            ObservationSide::Pre => r.event.time() < time,
            ObservationSide::Post => r.event.time() <= time,
        });
        if index == 0 {
            return Ok(None);
        }
        let record = &records[index - 1];
        let elapsed = time - record.event.time();
        if !elapsed.is_finite() {
            return Err(invalid("elapsed dose time overflow"));
        }
        Ok(Some(DoseHistoryValue {
            administered_amount: record.administered_amount,
            delivery_time: record.event.time(),
            scheduled_time: record.scheduled_time,
            elapsed,
            administration_index: record.administration_index,
            repetition_index: record.repetition_index,
        }))
    }
}

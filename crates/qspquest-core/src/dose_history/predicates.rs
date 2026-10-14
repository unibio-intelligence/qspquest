//! Schedule elapsed-dose predicates without approximate floating-point nudges.
use super::*;
use crate::expression::Compare;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PredicateBoundary {
    pub time: f64,
    /// Exact numeric readouts, using elapsed-time subtraction and the stated side.
    pub pre: Option<bool>,
    pub post: Option<bool>,
    /// Right-hand integration phase. This is not the exact boundary readout.
    pub after: Option<bool>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PredicateSchedule {
    pub initial: PredicateBoundary,
    pub boundaries: Vec<PredicateBoundary>,
}
fn validate(offset: f64, op: Compare) -> Result<(), Error> {
    if !offset.is_finite() || offset < 0. {
        return Err(invalid(
            "dose-relative offset must be finite and nonnegative",
        ));
    }
    if matches!(op, Compare::Eq | Compare::Ne) {
        return Err(Error::new(
            ErrorCode::Unsupported,
            "dose-relative predicates support lt, le, gt and ge only",
        ));
    }
    Ok(())
}
fn crossing(time: f64, offset: f64) -> Result<f64, Error> {
    let result = time + offset;
    if !result.is_finite() || (offset > 0. && result <= time) {
        return Err(invalid("dose-relative time overflows or collapses"));
    }
    Ok(result)
}
impl DoseHistory {
    /// Exact pointwise source arithmetic, including floating-point subtraction.
    pub fn predicate_at(
        &self,
        target: usize,
        time: f64,
        side: ObservationSide,
        offset: f64,
        op: Compare,
    ) -> Result<Option<bool>, Error> {
        validate(offset, op)?;
        Ok(self.at(target, time, side)?.map(|value| match op {
            Compare::Lt => value.elapsed < offset,
            Compare::Le => value.elapsed <= offset,
            Compare::Gt => value.elapsed > offset,
            Compare::Ge => value.elapsed >= offset,
            _ => unreachable!("validated comparison"),
        }))
    }
    fn predicate_boundary(
        &self,
        target: usize,
        time: f64,
        offset: f64,
        op: Compare,
    ) -> Result<PredicateBoundary, Error> {
        let after = self
            .at(target, time, ObservationSide::Post)?
            .map(|value| {
                let boundary = crossing(value.delivery_time, offset)?;
                Ok::<_, Error>(match op {
                    Compare::Lt | Compare::Le => time < boundary,
                    Compare::Gt | Compare::Ge => time >= boundary,
                    _ => unreachable!("validated comparison"),
                })
            })
            .transpose()?;
        Ok(PredicateBoundary {
            time,
            pre: self.predicate_at(target, time, ObservationSide::Pre, offset, op)?,
            post: self.predicate_at(target, time, ObservationSide::Post, offset, op)?,
            after,
        })
    }
    /// Plan stops at deliveries and still-active elapsed-time thresholds.
    /// A newer delivery replaces the prior timer. Initial history may precede
    /// the run window; no dose is replayed by this history-only calculation.
    /// The budget counts candidate stops before coincident times are merged.
    pub fn predicate_schedule(
        &self,
        target: usize,
        start: f64,
        end: f64,
        offset: f64,
        op: Compare,
        event_limit: usize,
    ) -> Result<PredicateSchedule, Error> {
        validate(offset, op)?;
        if !start.is_finite() || !end.is_finite() || end <= start {
            return Err(invalid("predicate window must be finite and increasing"));
        }
        let mut times = Vec::new();
        let mut push = |time: f64| -> Result<(), Error> {
            if time > start && time <= end {
                if times.len() >= event_limit {
                    return Err(Error::new(
                        ErrorCode::WorkBudget,
                        "dose-relative candidate stops exceed event budget",
                    ));
                }
                times.push(time);
            }
            Ok(())
        };
        if let Some(records) = self.targets.get(&target) {
            for (i, record) in records.iter().enumerate() {
                let delivery = record.event.time();
                if delivery > end {
                    break;
                }
                push(delivery)?;
                let threshold = crossing(delivery, offset)?;
                // At a coincident next delivery that delivery already supplies
                // the stop, including the old history's pre-side readout.
                if records
                    .get(i + 1)
                    .is_none_or(|next| threshold < next.event.time())
                {
                    push(threshold)?;
                }
            }
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        times.dedup();
        Ok(PredicateSchedule {
            initial: self.predicate_boundary(target, start, offset, op)?,
            boundaries: times
                .into_iter()
                .map(|t| self.predicate_boundary(target, t, offset, op))
                .collect::<Result<_, _>>()?,
        })
    }
}

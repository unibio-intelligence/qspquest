//! Dose history lowers to private step bindings; missing values fail lazily.
use super::*;
use qspquest_core::{
    dose_history::{DoseHistory, SimultaneousDose},
    expression::{Binary, Expr},
    regimen::{self, ObservationSide},
};
#[derive(Debug)]
pub(super) struct Binding {
    declaration: DoseHistoryBinding,
    target: usize,
    unit: String,
    value: String,
    available: String,
}
impl Binding {
    pub(super) fn owns_covariate(&self, name: &str) -> bool {
        self.value == name || self.available == name
    }
}
impl CompiledDocument {
    pub(super) fn compile_history(document: &ModelDocument) -> Result<Arc<Self>, Error> {
        if document.dose_history.len() > 1024 {
            return Err(invalid("too many dose-history bindings"));
        }
        fn uses_history(
            expr: &Expr,
            names: &BTreeSet<&str>,
            defs: &BTreeMap<&str, &Expr>,
            depth: usize,
        ) -> bool {
            if depth > 128 {
                return true;
            }
            if let Expr::Symbol { name } = expr {
                return names.contains(name.as_str())
                    || defs
                        .get(name.as_str())
                        .is_some_and(|e| uses_history(e, names, defs, depth + 1));
            }
            expr.children()
                .iter()
                .any(|e| uses_history(e, names, defs, depth + 1))
        }
        let names: BTreeSet<_> = document
            .dose_history
            .iter()
            .map(|b| b.name.as_str())
            .collect();
        let defs: BTreeMap<_, _> = document
            .definitions
            .iter()
            .map(|d| (d.name.as_str(), &d.expression))
            .chain(
                document
                    .individual
                    .iter()
                    .map(|d| (d.name.as_str(), &d.expression)),
            )
            .collect();
        for state in &document.states {
            let initial = matches!(&state.initial,Initial::Expression{expression} if uses_history(expression,&names,&defs,0));
            let dosing = state
                .dosing
                .as_ref()
                .is_some_and(|d| uses_history(&d.scale, &names, &defs, 0));
            if initial || dosing {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "dose history in initial conditions or dose scales is not qualified",
                ));
            }
        }
        let mut lowered = document.clone();
        lowered.dose_history.clear();
        let mut history = Vec::new();
        for (i, item) in document.dose_history.iter().enumerate() {
            let target = document
                .states
                .iter()
                .position(|s| s.name == item.target)
                .ok_or_else(|| invalid("unknown dose-history target"))?;
            let dosing = document.states[target]
                .dosing
                .as_ref()
                .ok_or_else(|| invalid("history target does not accept dosing"))?;
            let unit = match item.quantity {
                HistoryQuantity::DoseAmount => dosing.amount_unit.clone(),
                HistoryQuantity::HasDose => "1".into(),
                _ => document.time_unit.clone(),
            };
            let value = format!("history_internal_{i}_value");
            let available = format!("history_internal_{i}_available");
            for (name, unit) in [(&value, &unit), (&available, &"1".to_string())] {
                lowered.covariates.push(Covariate {
                    name: name.clone(),
                    unit: unit.clone(),
                    interpolation: Interpolation::Step,
                    default: Some(Quantity {
                        value: 0.,
                        unit: unit.clone(),
                    }),
                });
            }
            let expression = match item.quantity {
                HistoryQuantity::HasDose => Expr::symbol(&available),
                HistoryQuantity::TimeSinceDose => Expr::binary(
                    Binary::Divide,
                    Expr::binary(Binary::Subtract, Expr::symbol("time"), Expr::symbol(&value)),
                    Expr::symbol(&available),
                ),
                _ => Expr::binary(
                    Binary::Divide,
                    Expr::symbol(&value),
                    Expr::symbol(&available),
                ),
            };
            lowered.definitions.push(Definition {
                name: item.name.clone(),
                expression,
                unit: Some(unit.clone()),
            });
            history.push(Binding {
                declaration: item.clone(),
                target,
                unit,
                value,
                available,
            });
        }
        let mut compiled = Self::compile(&lowered)?;
        let inner = Arc::get_mut(&mut compiled).expect("new history model is uniquely owned");
        inner.history = history;
        inner.model_content_hash = qspquest_core::identity::canonical_hash(
            &serde_json::to_value(document).map_err(|e| invalid(e.to_string()))?,
        )?;
        Ok(compiled)
    }
    pub(super) fn reject_history_overrides<'a>(
        &self,
        names: impl Iterator<Item = &'a str>,
    ) -> Result<(), Error> {
        for name in names {
            if self
                .history
                .iter()
                .any(|b| name == b.value || name == b.available)
                || self
                    .dose_switches
                    .iter()
                    .any(|b| name == b.value || name == b.available)
            {
                return Err(invalid("internal dose history cannot be overridden"));
            }
        }
        Ok(())
    }
    pub(super) fn expand_history(
        &self,
        request: &regimen::Request,
        budgets: crate::Budgets,
    ) -> Result<regimen::Request, Error> {
        if self.history.is_empty() {
            return Ok(request.clone());
        }
        if !request.resets.is_empty() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "dose-history reset semantics are not qualified",
            ));
        }
        let start = request
            .start
            .as_ref()
            .map(|q| quantity(q)?.in_unit(self.model.time_unit))
            .transpose()?
            .unwrap_or(0.);
        let end = quantity(&request.end)?.in_unit(self.model.time_unit)?;
        let records = regimen::expand_administrations_in_window(
            &request.administrations,
            &self.dose_targets,
            self.model.time_unit,
            start,
            end,
            budgets.events,
        )?;
        let upper = records
            .len()
            .checked_mul(self.history.len())
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(request.covariate_changes.len()))
            .ok_or_else(|| invalid("history event count overflow"))?;
        if upper > budgets.events {
            return Err(Error::new(
                ErrorCode::WorkBudget,
                "dose-history changes exceed event budget",
            ));
        }
        let first = DoseHistory::new(records.clone(), SimultaneousDose::First)?;
        let last = DoseHistory::new(records.clone(), SimultaneousDose::Last)?;
        let mut result = request.clone();
        for b in &self.history {
            let history = if b.declaration.simultaneous == SimultaneousDose::First {
                &first
            } else {
                &last
            };
            let mut previous = None;
            for record in records.iter().filter(|r| r.event.target() == b.target) {
                let time = record.event.time();
                if previous == Some(time) {
                    continue;
                }
                previous = Some(time);
                let snapshot = history
                    .at(b.target, time, ObservationSide::Post)?
                    .expect("record exists at delivery time");
                let value = match b.declaration.quantity {
                    HistoryQuantity::DoseAmount => snapshot.administered_amount,
                    HistoryQuantity::HasDose => 1.,
                    _ => snapshot.delivery_time,
                };
                for (name, value, unit) in [
                    (&b.value, value, &b.unit),
                    (&b.available, 1., &"1".to_string()),
                ] {
                    result.covariate_changes.push(regimen::CovariateChange {
                        name: name.clone(),
                        time: Quantity {
                            value: time,
                            unit: self.time_unit.clone(),
                        },
                        value: Quantity {
                            value,
                            unit: unit.clone(),
                        },
                    });
                }
            }
        }
        Ok(result)
    }
}

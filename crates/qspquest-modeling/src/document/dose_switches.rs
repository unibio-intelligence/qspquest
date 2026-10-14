//! Bind dose-relative integration phases separately from exact point readouts.
use super::*;
use qspquest_core::{
    dose_history::{DoseHistory, SimultaneousDose},
    expression::{Binary, Compare, Expr},
    regimen::{self, ObservationSide},
};
#[derive(Debug)]
pub(super) struct Binding {
    target: usize,
    offset: Program,
    comparison: Compare,
    simultaneous: SimultaneousDose,
    pub(super) value: String,
    pub(super) available: String,
}
pub(super) struct BoundPredicate {
    history: Arc<DoseHistory>,
    target: usize,
    offset: f64,
    comparison: Compare,
    value: String,
    available: String,
}
impl BoundPredicate {
    pub(super) fn readout(
        &self,
        time: f64,
        side: &str,
        values: &mut BTreeMap<String, Quantity>,
    ) -> Result<(), Error> {
        let side = if side == "pre" {
            ObservationSide::Pre
        } else {
            ObservationSide::Post
        };
        let result =
            self.history
                .predicate_at(self.target, time, side, self.offset, self.comparison)?;
        values.insert(
            self.value.clone(),
            Quantity {
                value: if result == Some(true) { 1. } else { 0. },
                unit: "1".into(),
            },
        );
        values.insert(
            self.available.clone(),
            Quantity {
                value: if result.is_some() { 1. } else { 0. },
                unit: "1".into(),
            },
        );
        Ok(())
    }
}
impl CompiledDocument {
    pub(super) fn compile_dose_switches(document: &ModelDocument) -> Result<Arc<Self>, Error> {
        if document.dose_switches.len() > 1024 {
            return Err(invalid("too many dose switches"));
        }
        let symbols = document
            .parameters
            .iter()
            .map(|p| {
                Ok((
                    Symbol {
                        name: p.name.clone(),
                        kind: SymbolKind::Parameter,
                    },
                    Unit::parse(&p.default.unit)?,
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let names: BTreeSet<_> = document
            .dose_switches
            .iter()
            .map(|s| s.name.as_str())
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
        fn uses(
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
                        .is_some_and(|e| uses(e, names, defs, depth + 1));
            }
            expr.children()
                .iter()
                .any(|e| uses(e, names, defs, depth + 1))
        }
        for s in &document.states {
            if matches!(&s.initial,Initial::Expression{expression} if uses(expression,&names,&defs,0))
                || s.dosing
                    .as_ref()
                    .is_some_and(|d| uses(&d.scale, &names, &defs, 0))
            {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "dose-relative predicates in initial conditions or dose scales are not qualified",
                ));
            }
        }
        let mut lowered = document.clone();
        lowered.dose_switches.clear();
        let mut bindings = Vec::new();
        for (i, s) in document.dose_switches.iter().enumerate() {
            if matches!(s.comparison, Compare::Eq | Compare::Ne) {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "dose switches support lt, le, gt and ge only",
                ));
            }
            let target = document
                .states
                .iter()
                .position(|v| v.name == s.target && v.dosing.is_some())
                .ok_or_else(|| invalid("dose switch requires a dose target"))?;
            let offset = Program::compile_with_units(
                &s.offset,
                &symbols,
                Unit::parse(&document.time_unit)?,
            )?;
            let value = format!("dose_switch_internal_{i}_value");
            let available = format!("dose_switch_internal_{i}_available");
            for name in [&value, &available] {
                lowered.covariates.push(Covariate {
                    name: name.clone(),
                    unit: "1".into(),
                    interpolation: Interpolation::Step,
                    default: Some(Quantity {
                        value: 0.,
                        unit: "1".into(),
                    }),
                });
            }
            lowered.definitions.push(Definition {
                name: s.name.clone(),
                unit: Some("1".into()),
                expression: Expr::binary(
                    Binary::Divide,
                    Expr::symbol(&value),
                    Expr::symbol(&available),
                ),
            });
            bindings.push(Binding {
                target,
                offset,
                comparison: s.comparison,
                simultaneous: s.simultaneous,
                value,
                available,
            });
        }
        let mut compiled = Self::compile(&lowered)?;
        let inner = Arc::get_mut(&mut compiled).expect("new dose-switch model is uniquely owned");
        inner.dose_switches = bindings;
        inner.model_content_hash = qspquest_core::identity::canonical_hash(
            &serde_json::to_value(document).map_err(|e| invalid(e.to_string()))?,
        )?;
        Ok(compiled)
    }
    pub(super) fn expand_dose_switches(
        &self,
        request: &regimen::Request,
        parameters: &BTreeMap<String, Quantity>,
        budgets: crate::Budgets,
    ) -> Result<(regimen::Request, Vec<BoundPredicate>), Error> {
        if self.dose_switches.is_empty() {
            return Ok((request.clone(), Vec::new()));
        }
        if !request.resets.is_empty() {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "dose-switch reset semantics are not qualified",
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
        let first = Arc::new(DoseHistory::new(records.clone(), SimultaneousDose::First)?);
        let last = Arc::new(DoseHistory::new(records, SimultaneousDose::Last)?);
        let values = self
            .parameters
            .iter()
            .map(|p| {
                quantity(parameters.get(&p.name).unwrap_or(&p.default))?
                    .in_unit(Unit::parse(&p.default.unit)?)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut result = request.clone();
        let mut points = Vec::new();
        for b in &self.dose_switches {
            let offset = b.offset.evaluate(&values)?;
            let history = if b.simultaneous == SimultaneousDose::First {
                first.clone()
            } else {
                last.clone()
            };
            let remaining = budgets
                .events
                .checked_sub(result.covariate_changes.len())
                .ok_or_else(|| {
                    Error::new(ErrorCode::WorkBudget, "dose switches exceed event budget")
                })?;
            let schedule = history.predicate_schedule(
                b.target,
                start,
                end,
                offset,
                b.comparison,
                remaining,
            )?;
            let count = schedule
                .boundaries
                .len()
                .checked_add(1)
                .and_then(|n| n.checked_mul(2))
                .ok_or_else(|| invalid("dose switch count overflow"))?;
            if count > remaining {
                return Err(Error::new(
                    ErrorCode::WorkBudget,
                    "dose switch bindings exceed event budget",
                ));
            }
            for boundary in std::iter::once(schedule.initial).chain(schedule.boundaries) {
                for (name, value) in [
                    (&b.value, if boundary.after == Some(true) { 1. } else { 0. }),
                    (&b.available, if boundary.after.is_some() { 1. } else { 0. }),
                ] {
                    result.covariate_changes.push(regimen::CovariateChange {
                        name: name.clone(),
                        time: Quantity {
                            value: boundary.time,
                            unit: self.time_unit.clone(),
                        },
                        value: Quantity {
                            value,
                            unit: "1".into(),
                        },
                    });
                }
            }
            points.push(BoundPredicate {
                history,
                target: b.target,
                offset,
                comparison: b.comparison,
                value: b.value.clone(),
                available: b.available.clone(),
            });
        }
        Ok((result, points))
    }
}

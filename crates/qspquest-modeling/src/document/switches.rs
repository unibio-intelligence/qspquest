//! Declared parameter-bound switch times lower to owned step bindings.
use super::*;
use qspquest_core::expression::{Compare, Expr};
pub(super) fn predicate_value(op: Compare, time: f64, boundary: f64) -> f64 {
    let truth = match op {
        Compare::Lt => time < boundary,
        Compare::Le => time <= boundary,
        Compare::Gt => time > boundary,
        Compare::Ge => time >= boundary,
        Compare::Eq => time == boundary,
        Compare::Ne => time != boundary,
    };
    if truth {
        1.0
    } else {
        0.0
    }
}

impl CompiledDocument {
    pub(super) fn compile_switches(document: &ModelDocument) -> Result<Arc<Self>, Error> {
        if document.switches.len() > 1024 {
            return Err(invalid("too many fixed switches"));
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
        let predicate_names: BTreeSet<_> = document
            .switches
            .iter()
            .filter(|s| s.comparison.is_some())
            .map(|s| s.name.as_str())
            .collect();
        let definitions: BTreeMap<_, _> = document
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
        fn uses_predicate(
            expr: &Expr,
            names: &BTreeSet<&str>,
            definitions: &BTreeMap<&str, &Expr>,
            depth: usize,
            seen: &mut BTreeSet<String>,
        ) -> bool {
            if depth > 128 {
                return true;
            }
            if let Expr::Symbol { name } = expr {
                return names.contains(name.as_str())
                    || (seen.insert(name.clone())
                        && definitions.get(name.as_str()).is_some_and(|e| {
                            uses_predicate(e, names, definitions, depth + 1, seen)
                        }));
            }
            expr.children()
                .iter()
                .any(|e| uses_predicate(e, names, definitions, depth + 1, seen))
        }
        for state in &document.states {
            if state.dosing.as_ref().is_some_and(|d| {
                uses_predicate(
                    &d.scale,
                    &predicate_names,
                    &definitions,
                    0,
                    &mut BTreeSet::new(),
                )
            }) {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "pointwise time predicates in dose scales are not qualified",
                ));
            }
        }
        let mut lowered = document.clone();
        lowered.switches.clear();
        let mut switches = Vec::new();
        for switch in &document.switches {
            if matches!(switch.comparison, Some(Compare::Eq | Compare::Ne)) {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "time predicates support lt, le, gt and ge only",
                ));
            }
            let program = Program::compile_with_units(
                &switch.time,
                &symbols,
                Unit::parse(&document.time_unit)?,
            )
            .map_err(|mut e| {
                e.expression = Some(format!("switches.{}.time", switch.name));
                e
            })?;
            switches.push((switch.name.clone(), program, switch.comparison));
            lowered.covariates.push(Covariate {
                name: switch.name.clone(),
                unit: "1".into(),
                interpolation: Interpolation::Step,
                default: Some(Quantity {
                    value: 0.0,
                    unit: "1".into(),
                }),
            });
        }
        let mut compiled = Self::compile(&lowered)?;
        let inner = Arc::get_mut(&mut compiled).expect("new compiled model is uniquely owned");
        inner.switches = switches;
        inner.model_content_hash = qspquest_core::identity::canonical_hash(
            &serde_json::to_value(document).map_err(|e| invalid(e.to_string()))?,
        )?;
        Ok(compiled)
    }
    pub(super) fn reject_switch_overrides<'a>(
        &self,
        names: impl Iterator<Item = &'a str>,
    ) -> Result<(), Error> {
        for name in names {
            if self.switches.iter().any(|(s, _, _)| s == name) {
                return Err(invalid(format!(
                    "fixed switch {name} cannot be overridden as a covariate"
                )));
            }
        }
        Ok(())
    }
    pub(super) fn expand_switches(
        &self,
        request: &qspquest_core::regimen::Request,
        parameters: &BTreeMap<String, Quantity>,
        budgets: crate::Budgets,
    ) -> Result<qspquest_core::regimen::Request, Error> {
        self.reject_switch_overrides(
            request
                .covariates
                .keys()
                .map(String::as_str)
                .chain(request.covariate_changes.iter().map(|c| c.name.as_str())),
        )?;
        let mut result = request.clone();
        let values = self
            .parameters
            .iter()
            .map(|p| {
                (|| {
                    quantity_in_declared_unit(
                        parameters.get(&p.name).unwrap_or(&p.default),
                        &p.default.unit,
                    )
                })()
                .map_err(|mut e| {
                    if e.expression.is_none() {
                        e.expression = Some(format!("parameters.{}", p.name));
                    }
                    e
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let unit = self.model.time_unit;
        let start = request
            .start
            .as_ref()
            .map(|q| quantity(q)?.in_unit(unit))
            .transpose()?
            .unwrap_or(0.0);
        let end = quantity(&request.end)?.in_unit(unit)?;
        for (name, program, comparison) in &self.switches {
            let time = program.evaluate(&values).map_err(|mut e| {
                e.expression = Some(format!("switches.{name}.time"));
                e
            })?;
            if !time.is_finite() {
                return Err(invalid("fixed switch time must be finite"));
            }
            result.covariates.insert(
                name.clone(),
                Quantity {
                    value: comparison
                        .map(|op| predicate_value(op, start, time))
                        .unwrap_or(if time <= start { 1.0 } else { 0.0 }),
                    unit: "1".into(),
                },
            );
            if (time > start || (time == start && comparison.is_some())) && time <= end {
                result
                    .covariate_changes
                    .push(qspquest_core::regimen::CovariateChange {
                        name: name.clone(),
                        time: Quantity {
                            value: time,
                            unit: self.time_unit.clone(),
                        },
                        value: Quantity {
                            value: if matches!(comparison, Some(Compare::Lt | Compare::Le)) {
                                0.0
                            } else {
                                1.0
                            },
                            unit: "1".into(),
                        },
                    });
            }
        }
        if result
            .covariate_changes
            .len()
            .checked_add(result.checkpoints.len())
            .and_then(|n| n.checked_add(result.resets.len()))
            .is_none_or(|n| n > budgets.events)
        {
            return Err(Error::new(
                ErrorCode::WorkBudget,
                "fixed switches exceed event budget",
            ));
        }
        Ok(result)
    }
}

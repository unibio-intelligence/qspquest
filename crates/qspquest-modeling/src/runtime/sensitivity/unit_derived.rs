//! Resolve constant derived quantities without losing declared-unit validation.
use super::*;
use crate::runtime::units::{
    BindingDefinition, InitialCondition, UnitCompiledModel, UnitModelDefinition,
};
use qspquest_core::units::Unit;
#[derive(Clone, Debug)]
pub struct UnitDerivedBinding {
    pub name: String,
    pub unit: Unit,
    pub expression: Expr,
}
/// Expand derived quantities in initial states, equations, dose scales and
/// observables. The returned model retains only independent bindings.
/// Each derived declaration is dimension-checked before substitution.
pub fn resolve_unit_bindings(
    definition: &UnitModelDefinition,
    derived: &[UnitDerivedBinding],
    readouts: &[UnitSensitivityReadout],
) -> Result<(UnitModelDefinition, Vec<UnitSensitivityReadout>), Error> {
    if readouts.len() > 1024 || definition.bindings.len().saturating_add(derived.len()) > 4096 {
        return Err(invalid("too many derived bindings or sensitivity readouts"));
    }
    let base = definition
        .bindings
        .iter()
        .map(|b| b.name.clone())
        .collect::<Vec<_>>();
    let raw = derived
        .iter()
        .map(|b| DerivedBinding {
            name: b.name.clone(),
            expression: b.expression.clone(),
        })
        .collect::<Vec<_>>();
    let resolver = ResolvedBindings::new(&base, &raw)?;
    let mut source = definition.clone();
    source
        .bindings
        .extend(derived.iter().map(|b| BindingDefinition {
            name: b.name.clone(),
            unit: b.unit,
            bounds: None,
        }));
    UnitCompiledModel::compile(&source)?;
    let mut symbols = source
        .bindings
        .iter()
        .map(|b| {
            (
                Symbol {
                    name: b.name.clone(),
                    kind: SymbolKind::Parameter,
                },
                b.unit,
            )
        })
        .collect::<Vec<_>>();
    for binding in derived {
        crate::compile::units::normalize(&binding.expression, &symbols, binding.unit)?;
    }
    symbols.extend(source.states.iter().map(|s| {
        (
            Symbol {
                name: s.name.clone(),
                kind: SymbolKind::State,
            },
            s.unit,
        )
    }));
    symbols.push((
        Symbol {
            name: "time".into(),
            kind: SymbolKind::Time,
        },
        source.time_unit,
    ));
    for state in &source.states {
        symbols.push((
            Symbol {
                name: format!("input_{}", state.name),
                kind: SymbolKind::Input,
            },
            state.unit.divide(source.time_unit)?,
        ));
    }
    for r in readouts {
        crate::compile::units::normalize(&r.expression, &symbols, r.unit)?;
    }
    let raw_model = ModelDefinition {
        bindings: base,
        states: definition
            .states
            .iter()
            .map(|s| StateDefinition {
                name: s.name.clone(),
                initial: match &s.initial {
                    InitialCondition::Quantity(q) => Expr::number(q.value),
                    InitialCondition::Expression(e) => e.clone(),
                },
                rhs: s.rhs.clone(),
                dose_scale: s.dosing.as_ref().map(|(_, e)| e.clone()),
            })
            .collect(),
    };
    let expanded = resolver.expand_model(&raw_model)?;
    let mut result = definition.clone();
    for (state, expanded) in result.states.iter_mut().zip(expanded.states) {
        if matches!(state.initial, InitialCondition::Expression(_)) {
            state.initial = InitialCondition::Expression(expanded.initial);
        }
        state.rhs = expanded.rhs;
        if let Some((_, expression)) = &mut state.dosing {
            *expression = expanded
                .dose_scale
                .ok_or_else(|| invalid("missing expanded dose scale"))?;
        }
    }
    let expressions = resolver.expand_readouts(
        &readouts
            .iter()
            .map(|r| r.expression.clone())
            .collect::<Vec<_>>(),
    )?;
    let readouts = readouts
        .iter()
        .zip(expressions)
        .map(|(r, expression)| UnitSensitivityReadout {
            expression,
            unit: r.unit,
        })
        .collect();
    UnitCompiledModel::compile(&result)?;
    Ok((result, readouts))
}

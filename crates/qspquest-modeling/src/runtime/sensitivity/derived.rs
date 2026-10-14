//! Bounded expansion of constant derived bindings before symbolic differentiation.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug)]
pub struct DerivedBinding {
    pub name: String,
    pub expression: Expr,
}
#[derive(Debug)]
pub struct ResolvedBindings {
    base: Vec<String>,
    resolved: BTreeMap<String, Expr>,
}
fn charge(expr: &Expr, remaining: &mut usize, depth: usize) -> Result<(), Error> {
    if depth > 128 || *remaining == 0 {
        return Err(Error::new(
            ErrorCode::ExpressionLimit,
            "derived sensitivity expansion limit exceeded",
        ));
    }
    *remaining -= 1;
    for child in expr.children() {
        charge(child, remaining, depth + 1)?;
    }
    Ok(())
}
fn resolve(
    name: &str,
    defs: &BTreeMap<String, Expr>,
    cache: &mut BTreeMap<String, Expr>,
    visiting: &mut BTreeSet<String>,
    remaining: &mut usize,
    depth: usize,
) -> Result<Expr, Error> {
    if let Some(expr) = cache.get(name) {
        charge(expr, remaining, depth)?;
        return Ok(expr.clone());
    }
    if visiting.len() >= 64 || !visiting.insert(name.into()) {
        return Err(invalid("cyclic or over-deep derived binding dependency"));
    }
    let expr = expand(&defs[name], defs, cache, visiting, remaining, depth)?;
    visiting.remove(name);
    cache.insert(name.into(), expr.clone());
    Ok(expr)
}
fn expand(
    expr: &Expr,
    defs: &BTreeMap<String, Expr>,
    cache: &mut BTreeMap<String, Expr>,
    visiting: &mut BTreeSet<String>,
    remaining: &mut usize,
    depth: usize,
) -> Result<Expr, Error> {
    if depth > 128 || *remaining == 0 {
        return Err(Error::new(
            ErrorCode::ExpressionLimit,
            "derived sensitivity expansion limit exceeded",
        ));
    }
    *remaining -= 1;
    if let Expr::Symbol { name } = expr {
        if defs.contains_key(name) || cache.contains_key(name) {
            return resolve(name, defs, cache, visiting, remaining, depth + 1);
        }
    }
    let mut next = |e: &Expr| expand(e, defs, cache, visiting, remaining, depth + 1);
    Ok(match expr {
        Expr::Literal { .. } | Expr::Quantity { .. } | Expr::Symbol { .. } => expr.clone(),
        Expr::Unary {
            operator,
            arguments,
        } => Expr::Unary {
            operator: *operator,
            arguments: Box::new([next(&arguments[0])?]),
        },
        Expr::Binary {
            operator,
            arguments,
        } => Expr::Binary {
            operator: *operator,
            arguments: Box::new([next(&arguments[0])?, next(&arguments[1])?]),
        },
        Expr::Compare {
            operator,
            arguments,
        } => Expr::Compare {
            operator: *operator,
            arguments: Box::new([next(&arguments[0])?, next(&arguments[1])?]),
        },
        Expr::Boolean {
            operator,
            arguments,
        } => Expr::Boolean {
            operator: *operator,
            arguments: arguments.iter().map(&mut next).collect::<Result<_, _>>()?,
        },
        Expr::Call { name, arguments } => Expr::Call {
            name: *name,
            arguments: arguments.iter().map(&mut next).collect::<Result<_, _>>()?,
        },
        Expr::Conditional { arguments } => Expr::Conditional {
            arguments: Box::new([
                next(&arguments[0])?,
                next(&arguments[1])?,
                next(&arguments[2])?,
            ]),
        },
    })
}
impl ResolvedBindings {
    /// Definitions may be unordered. Only independent bindings and other derived
    /// bindings may appear in their expressions; state/time/input dependencies fail.
    pub fn new(base: &[String], derived: &[DerivedBinding]) -> Result<Self, Error> {
        if base.len() + derived.len() > 4096 {
            return Err(invalid("too many sensitivity bindings"));
        }
        let mut names: BTreeSet<String> = base.iter().cloned().collect();
        if names.len() != base.len() {
            return Err(invalid("duplicate independent binding"));
        }
        for binding in derived {
            if !names.insert(binding.name.clone()) {
                return Err(invalid("duplicate derived binding"));
            }
        }
        let symbols: Vec<_> = names
            .iter()
            .map(|name| Symbol {
                name: name.clone(),
                kind: SymbolKind::Parameter,
            })
            .collect();
        Program::compile(&Expr::number(0.), &symbols)?;
        for binding in derived {
            numeric(Program::compile(&binding.expression, &symbols)?)?;
        }
        let defs: BTreeMap<_, _> = derived
            .iter()
            .map(|d| (d.name.clone(), d.expression.clone()))
            .collect();
        let mut cache = BTreeMap::new();
        let mut remaining = 1_000_000;
        for name in defs.keys() {
            resolve(
                name,
                &defs,
                &mut cache,
                &mut BTreeSet::new(),
                &mut remaining,
                0,
            )?;
        }
        Ok(Self {
            base: base.to_vec(),
            resolved: cache,
        })
    }
    pub fn expand_model(&self, definition: &ModelDefinition) -> Result<ModelDefinition, Error> {
        if definition.bindings != self.base {
            return Err(invalid("independent binding order differs"));
        }
        // Validate source expressions and names before recursive expansion.
        let mut source = definition.clone();
        source.bindings.extend(self.resolved.keys().cloned());
        CompiledModel::compile(&source)?;
        let mut result = definition.clone();
        let mut remaining = 1_000_000;
        let mut cache = self.resolved.clone();
        let defs = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        for state in &mut result.states {
            state.initial = expand(
                &state.initial,
                &defs,
                &mut cache,
                &mut visiting,
                &mut remaining,
                0,
            )?;
            state.rhs = expand(
                &state.rhs,
                &defs,
                &mut cache,
                &mut visiting,
                &mut remaining,
                0,
            )?;
            state.dose_scale = state
                .dose_scale
                .as_ref()
                .map(|e| expand(e, &defs, &mut cache, &mut visiting, &mut remaining, 0))
                .transpose()?;
        }
        CompiledModel::compile(&result)?;
        Ok(result)
    }
    /// Expand an observable using the same resolved dependency graph.
    pub fn expand_readout(&self, expression: &Expr) -> Result<Expr, Error> {
        Ok(self
            .expand_readouts(std::slice::from_ref(expression))?
            .remove(0))
    }
    /// All observables share one expansion budget to prevent repeated aliases
    /// from multiplying allocation before the readout compiler sees them.
    pub fn expand_readouts(&self, expressions: &[Expr]) -> Result<Vec<Expr>, Error> {
        if expressions.len() > 1024 {
            return Err(invalid("too many sensitivity readouts"));
        }
        let mut remaining = 1_000_000;
        let mut cache = self.resolved.clone();
        let defs = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        expressions
            .iter()
            .map(|e| expand(e, &defs, &mut cache, &mut visiting, &mut remaining, 0))
            .collect()
    }
}

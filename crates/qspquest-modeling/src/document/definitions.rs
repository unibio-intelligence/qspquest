//! Bounded algebraic substitution; preserves lazy branches and symbolic derivatives.
use crate::compile::{Program, Symbol, SymbolKind, SymbolTable};
use qspquest_core::{expression::Expr, model::Definition, units::Unit, Error, ErrorCode};
use std::collections::{BTreeMap, BTreeSet};
fn contextual(mut error: Error, path: &str) -> Error {
    error.expression = Some(format!("{path}.{}", error.expression.unwrap_or_default()));
    error
}
fn limit() -> Error {
    Error::new(
        ErrorCode::ExpressionLimit,
        "algebraic expansion exceeds node or depth budget",
    )
}
fn copy(
    expr: &Expr,
    aliases: &BTreeMap<String, Expr>,
    depth: usize,
    nodes: &mut usize,
) -> Result<Expr, Error> {
    *nodes += 1;
    if depth > 128 || *nodes > 16384 {
        return Err(limit());
    }
    if let Expr::Symbol { name } = expr {
        if let Some(value) = aliases.get(name) {
            return copy(value, &BTreeMap::new(), depth, nodes);
        }
    }
    let mut child = |e: &Expr| copy(e, aliases, depth + 1, nodes);
    Ok(match expr {
        Expr::Literal { value } => Expr::Literal {
            value: value.clone(),
        },
        Expr::Quantity { quantity } => Expr::Quantity {
            quantity: quantity.clone(),
        },
        Expr::Symbol { name } => Expr::symbol(name),
        Expr::Unary {
            operator,
            arguments,
        } => Expr::Unary {
            operator: *operator,
            arguments: Box::new([child(&arguments[0])?]),
        },
        Expr::Binary {
            operator,
            arguments,
        } => Expr::binary(*operator, child(&arguments[0])?, child(&arguments[1])?),
        Expr::Compare {
            operator,
            arguments,
        } => Expr::Compare {
            operator: *operator,
            arguments: Box::new([child(&arguments[0])?, child(&arguments[1])?]),
        },
        Expr::Boolean {
            operator,
            arguments,
        } => Expr::Boolean {
            operator: *operator,
            arguments: arguments.iter().map(child).collect::<Result<_, _>>()?,
        },
        Expr::Call { name, arguments } => Expr::call(
            *name,
            arguments.iter().map(child).collect::<Result<_, _>>()?,
        ),
        Expr::Conditional { arguments } => Expr::conditional(
            child(&arguments[0])?,
            child(&arguments[1])?,
            child(&arguments[2])?,
        ),
    })
}
fn ready(expr: &Expr, names: &BTreeSet<&str>, done: &BTreeMap<String, Expr>) -> bool {
    if let Expr::Symbol { name } = expr {
        return !names.contains(name.as_str()) || done.contains_key(name);
    }
    expr.children().iter().all(|e| ready(e, names, done))
}
pub(super) struct Expanded {
    aliases: BTreeMap<String, Expr>,
    source_symbols: Vec<Symbol>,
    total: usize,
}
impl Expanded {
    pub fn new(definitions: &[Definition], symbols: &[(Symbol, Unit)]) -> Result<Self, Error> {
        if definitions.len() > 4096 {
            return Err(limit());
        }
        // Definitions have no execution role until expanded into a consumer.
        // Validate shape/units here; dynamics re-check expanded expressions with
        // the actual state/time/input kinds before integration.
        let validation_symbols = crate::compile::readout_symbols(symbols);
        let mut source_symbols = validation_symbols
            .iter()
            .map(|(s, _)| s.clone())
            .collect::<Vec<_>>();
        source_symbols.extend(definitions.iter().map(|d| Symbol {
            name: d.name.clone(),
            kind: SymbolKind::Parameter,
        }));
        let mut out = Self {
            aliases: BTreeMap::new(),
            source_symbols,
            total: 0,
        };
        for d in definitions {
            Program::check_unit_shape(&d.expression, &SymbolTable::new(&out.source_symbols)?, 256)
                .map_err(|e| contextual(e, &format!("definitions.{}", d.name)))?;
        }
        let names = definitions
            .iter()
            .map(|d| d.name.as_str())
            .collect::<BTreeSet<_>>();
        let mut pending = definitions.iter().collect::<Vec<_>>();
        while !pending.is_empty() {
            let index = pending
                .iter()
                .position(|d| ready(&d.expression, &names, &out.aliases))
                .ok_or_else(|| {
                    Error::new(ErrorCode::InvalidInput, "cyclic algebraic definitions")
                })?;
            let d = pending.remove(index);
            let expr = out.expand(&d.expression, &format!("definitions.{}", d.name))?;
            let inferred = crate::compile::units::infer_expanded(&expr, &validation_symbols)
                .map_err(|e| contextual(e, &format!("definitions.{}", d.name)))?;
            if let Some(code) = &d.unit {
                inferred
                    .conversion_to(Unit::parse(code)?)
                    .map_err(|e| contextual(e, &format!("definitions.{}", d.name)))?;
            }
            out.aliases.insert(d.name.clone(), expr);
        }
        Ok(out)
    }
    pub fn expand(&mut self, expr: &Expr, path: &str) -> Result<Expr, Error> {
        Program::check_unit_shape(expr, &SymbolTable::new(&self.source_symbols)?, 256)
            .map_err(|e| contextual(e, path))?;
        let mut nodes = 0;
        let expr = copy(expr, &self.aliases, 1, &mut nodes).map_err(|e| contextual(e, path))?;
        self.total = self.total.checked_add(nodes).ok_or_else(limit)?;
        if self.total > 1_000_000 {
            return Err(contextual(limit(), path));
        }
        Ok(expr)
    }
}

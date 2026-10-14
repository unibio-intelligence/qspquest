pub mod sensitivity;
pub mod units;
// Lowered execution model. Unit-checked public model IR will lower to these types.
use crate::{
    compile::{Program, Symbol, SymbolKind, ValueKind},
    Error, ErrorCode, Model,
};
use qspquest_core::expression::Expr;
use std::{cell::RefCell, sync::Arc};
#[derive(Clone, Debug)]
pub struct StateDefinition {
    pub name: String,
    pub initial: Expr,
    pub rhs: Expr,
    pub dose_scale: Option<Expr>,
}
#[derive(Clone, Debug)]
pub struct ModelDefinition {
    /// Parameters and constant covariates share binding slots; values are never compiled.
    pub bindings: Vec<String>,
    pub states: Vec<StateDefinition>,
}
#[derive(Debug)]
pub struct CompiledModel {
    names: Vec<String>,
    initial: Vec<Program>,
    scales: Vec<Option<Program>>,
    rhs: Vec<Program>,
    jacobian: Vec<Vec<(usize, Program)>>,
    scratch_size: usize,
    time_dependent: bool,
}
#[derive(Debug)]
struct Workspace {
    inputs: Vec<f64>,
    scratch: Vec<f64>,
    jac_cache: Option<JacobianCache>,
}
#[derive(Debug)]
struct JacobianCache {
    time: f64,
    state: Vec<f64>,
    rates: Vec<f64>,
    entries: Vec<Vec<f64>>,
}
fn same_bits(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(left, right)| left.to_bits() == right.to_bits())
}
#[derive(Debug)]
pub struct BoundProblem {
    compiled: Arc<CompiledModel>,
    initial: Vec<f64>,
    scales: Vec<Option<f64>>,
    workspace: RefCell<Workspace>,
}
fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::InvalidInput, message)
}
fn numeric(program: Program) -> Result<Program, Error> {
    if program.kind != ValueKind::Number {
        Err(Error::new(
            ErrorCode::Type,
            "model equations must be numeric",
        ))
    } else {
        Ok(program)
    }
}
fn contains(expr: &Expr, name: &str) -> bool {
    matches!(expr,Expr::Symbol{name:n} if n==name)
        || expr.children().iter().any(|e| contains(e, name))
}
impl CompiledModel {
    pub(crate) fn binding_storage_values(&self) -> usize {
        2 * self.names.len() + 5 * self.initial.len() + 1 + self.scratch_size
    }
    pub fn compile(definition: &ModelDefinition) -> Result<Arc<Self>, Error> {
        Self::compile_internal(definition, false)
    }
    // Validated unit/sensitivity lowering may use the expanded expression budget.
    // Each caller validates source expressions before generating new nodes.
    fn compile_internal(
        definition: &ModelDefinition,
        normalized: bool,
    ) -> Result<Arc<Self>, Error> {
        let n = definition.states.len();
        let m = definition.bindings.len();
        if n == 0 || n > 1024 || m > 4096 {
            return Err(invalid("model exceeds supported state/binding bounds"));
        }
        let bindings: Vec<_> = definition
            .bindings
            .iter()
            .map(|name| Symbol {
                name: name.clone(),
                kind: SymbolKind::Parameter,
            })
            .collect();
        let mut symbols = bindings.clone();
        symbols.extend(definition.states.iter().map(|s| Symbol {
            name: s.name.clone(),
            kind: SymbolKind::State,
        }));
        symbols.push(Symbol {
            name: "time".into(),
            kind: SymbolKind::Time,
        });
        symbols.extend(definition.states.iter().map(|s| Symbol {
            name: format!("input_{}", s.name),
            kind: SymbolKind::Input,
        }));
        // Validate each environment once, including unused names.
        let binding_table = crate::compile::SymbolTable::new(&bindings)?;
        let state_table = crate::compile::SymbolTable::new(&symbols)?;
        let expression_limit = if normalized { 16384 } else { 256 };
        let state_columns: std::collections::BTreeMap<_, _> = definition
            .states
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.as_str(), i))
            .collect();
        let mut result = Self {
            time_dependent: definition.states.iter().any(|s| contains(&s.rhs, "time")),
            names: definition.bindings.clone(),
            initial: vec![],
            scales: vec![],
            rhs: vec![],
            jacobian: vec![],
            scratch_size: 0,
        };
        let mut instructions = 0usize;
        let mut account = |program: &Program| -> Result<(), Error> {
            instructions = instructions
                .checked_add(program.instruction_count())
                .ok_or_else(|| invalid("program size overflow"))?;
            if instructions > 1_000_000 {
                return Err(Error::new(
                    ErrorCode::ExpressionLimit,
                    "model exceeds one million instructions",
                ));
            }
            Ok(())
        };
        for state in &definition.states {
            let initial = numeric(Program::compile_prepared(
                &state.initial,
                &binding_table,
                normalized,
                expression_limit,
            )?)?;
            account(&initial)?;
            result.initial.push(initial);
            let scale = state
                .dose_scale
                .as_ref()
                .map(|expr| {
                    numeric(Program::compile_prepared(
                        expr,
                        &binding_table,
                        normalized,
                        expression_limit,
                    )?)
                })
                .transpose()?;
            if let Some(program) = &scale {
                account(program)?;
            }
            result.scales.push(scale);
            let rhs = numeric(Program::compile_prepared(
                &state.rhs,
                &state_table,
                normalized,
                expression_limit,
            )?)?;
            account(&rhs)?;
            result.scratch_size = result.scratch_size.max(rhs.workspace_size());
            result.rhs.push(rhs);
            let mut row = vec![];
            fn dependencies<'a>(expr: &'a Expr, names: &mut std::collections::BTreeSet<&'a str>) {
                if let Expr::Symbol { name } = expr {
                    names.insert(name.as_str());
                }
                for e in expr.children() {
                    dependencies(e, names);
                }
            }
            let mut names = std::collections::BTreeSet::new();
            dependencies(&state.rhs, &mut names);
            let mut columns = names
                .into_iter()
                .filter_map(|name| state_columns.get(name).copied())
                .collect::<Vec<_>>();
            columns.sort_unstable();
            for column in columns {
                let variable = &definition.states[column];
                let derivative =
                    Program::derivative_row_prechecked(&state.rhs, &state_table, &variable.name)?;
                account(&derivative)?;
                result.scratch_size = result.scratch_size.max(derivative.workspace_size());
                row.push((column, derivative));
            }
            result.jacobian.push(row);
        }
        Ok(Arc::new(result))
    }
    pub fn bind(self: &Arc<Self>, values: &[f64]) -> Result<BoundProblem, Error> {
        self.bind_continuing(values, None)
    }
    pub(crate) fn bind_continuing(
        self: &Arc<Self>,
        values: &[f64],
        continuing: Option<&[f64]>,
    ) -> Result<BoundProblem, Error> {
        let initial = continuing.map(|v| v.iter().copied().map(Some).collect::<Vec<_>>());
        self.bind_initial(values, initial.as_deref())
    }
    pub(crate) fn bind_initial(
        self: &Arc<Self>,
        values: &[f64],
        initial_values: Option<&[Option<f64>]>,
    ) -> Result<BoundProblem, Error> {
        if values.len() != self.names.len() || values.iter().any(|v| !v.is_finite()) {
            return Err(invalid("invalid binding vector"));
        }
        if initial_values.is_some_and(|v| {
            v.len() != self.initial.len() || v.iter().flatten().any(|x| !x.is_finite())
        }) {
            return Err(invalid("invalid initial state overrides"));
        }
        let initial = self
            .initial
            .iter()
            .enumerate()
            .map(|(i, p)| match initial_values.and_then(|v| v[i]) {
                Some(value) => Ok(value),
                None => p.evaluate(values),
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let scales = self
            .scales
            .iter()
            .map(|p| p.as_ref().map(|p| p.evaluate(values)).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        if scales.iter().flatten().any(|v| *v < 0.0) {
            return Err(invalid("dose scales must be finite and nonnegative"));
        }
        let mut inputs = vec![0.0; values.len() + 2 * self.rhs.len() + 1];
        inputs[..values.len()].copy_from_slice(values);
        Ok(BoundProblem {
            compiled: self.clone(),
            initial,
            scales,
            workspace: RefCell::new(Workspace {
                inputs,
                scratch: vec![0.0; self.scratch_size],
                jac_cache: None,
            }),
        })
    }
    pub fn jacobian_sparsity(&self) -> Vec<(usize, usize)> {
        self.jacobian
            .iter()
            .enumerate()
            .flat_map(|(row, entries)| entries.iter().map(move |(column, _)| (row, *column)))
            .collect()
    }
}
impl BoundProblem {
    /// Recompute every bound artifact atomically. A failed bind leaves this problem intact.
    pub fn rebind(&mut self, values: &[f64]) -> Result<(), Error> {
        *self = self.compiled.bind(values)?;
        Ok(())
    }
    fn inputs(
        &self,
        workspace: &mut Workspace,
        time: f64,
        x: &[f64],
        rates: &[f64],
        output_len: usize,
    ) -> Result<(), Error> {
        let n = self.compiled.rhs.len();
        let m = self.compiled.names.len();
        if x.len() != n || rates.len() != n || output_len != n {
            return Err(invalid("model vector dimensions differ"));
        }
        workspace.inputs[m..m + n].copy_from_slice(x);
        workspace.inputs[m + n] = time;
        workspace.inputs[m + n + 1..].copy_from_slice(rates);
        // Binding slots are immutable and were validated when this problem was bound.
        if workspace.inputs[m..].iter().any(|v| !v.is_finite()) {
            return Err(invalid("model inputs must be finite"));
        }
        Ok(())
    }
}
impl Model for BoundProblem {
    fn jacobian_sparsity(&self) -> Option<Vec<(usize, usize)>> {
        Some(self.compiled.jacobian_sparsity())
    }
    fn rhs_depends_on_time(&self) -> bool {
        self.compiled.time_dependent
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn initial(&self) -> Vec<f64> {
        self.initial.clone()
    }
    fn dose_scale(&self, state: usize) -> Option<f64> {
        self.scales.get(state).copied().flatten()
    }
    fn rhs(&self, time: f64, x: &[f64], rates: &[f64], out: &mut [f64]) -> Result<(), Error> {
        let mut workspace = self.workspace.borrow_mut();
        self.inputs(&mut workspace, time, x, rates, out.len())?;
        let Workspace {
            inputs, scratch, ..
        } = &mut *workspace;
        for (i, program) in self.compiled.rhs.iter().enumerate() {
            out[i] = program
                .evaluate_prechecked(inputs, scratch)
                .map_err(|mut e| {
                    e.expression = Some(format!(
                        "dynamics[{i}].{}",
                        e.expression.unwrap_or_default()
                    ));
                    e
                })?;
        }
        Ok(())
    }
    fn jac_mul(
        &self,
        time: f64,
        x: &[f64],
        rates: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        if out.len() != self.compiled.rhs.len() {
            return Err(invalid("invalid Jacobian output dimensions"));
        }
        if v.len() != self.compiled.rhs.len() || v.iter().any(|x| !x.is_finite()) {
            return Err(invalid("invalid Jacobian direction"));
        }
        let mut workspace = self.workspace.borrow_mut();
        // Diffsol requests several directions at the same state while assembling
        // one Jacobian. Evaluate symbolic entries once, then apply each direction.
        let hit = workspace.jac_cache.as_ref().is_some_and(|cached| {
            cached.time.to_bits() == time.to_bits()
                && same_bits(&cached.state, x)
                && same_bits(&cached.rates, rates)
        });
        if !hit {
            // Remove the cache before writing entries so a failed evaluation cannot
            // leave partial new entries under the previous state's cache key.
            let mut cached = workspace.jac_cache.take().unwrap_or_else(|| JacobianCache {
                time,
                state: Vec::with_capacity(x.len()),
                rates: Vec::with_capacity(rates.len()),
                entries: self
                    .compiled
                    .jacobian
                    .iter()
                    .map(|entries| vec![0.0; entries.len()])
                    .collect(),
            });
            self.inputs(&mut workspace, time, x, rates, out.len())?;
            let Workspace {
                inputs, scratch, ..
            } = &mut *workspace;
            for (row, entries) in self.compiled.jacobian.iter().enumerate() {
                // Validate the primal once per row, including structurally zero rows.
                // Each derivative retains its own domain checks; no primal work is duplicated.
                self.compiled.rhs[row].evaluate_prechecked(inputs, scratch)?;
                for ((column, program), value) in entries.iter().zip(&mut cached.entries[row]) {
                    *value = program
                        .evaluate_prechecked(inputs, scratch)
                        .map_err(|mut e| {
                            e.expression = Some(format!(
                                "jacobian[{row},{column}].{}",
                                e.expression.unwrap_or_default()
                            ));
                            e
                        })?;
                }
            }
            cached.time = time;
            cached.state.clear();
            cached.state.extend_from_slice(x);
            cached.rates.clear();
            cached.rates.extend_from_slice(rates);
            workspace.jac_cache = Some(cached);
        }
        let cached = workspace
            .jac_cache
            .as_ref()
            .ok_or_else(|| Error::new(ErrorCode::Solver, "Jacobian cache was not initialized"))?;
        for (row, entries) in self.compiled.jacobian.iter().enumerate() {
            out[row] = entries
                .iter()
                .zip(&cached.entries[row])
                .map(|((column, _), value)| value * v[*column])
                .sum();
            if !out[row].is_finite() {
                return Err(Error::new(ErrorCode::Domain, "non-finite Jacobian product"));
            }
        }
        Ok(())
    }
}

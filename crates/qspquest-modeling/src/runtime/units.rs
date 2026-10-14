//! Unit-checked lowering and quantity binding; not the serialized public ModelIR.
//! Solver times, states, tolerances and event amounts use the model's declared units.
use super::*;
use qspquest_core::units::Unit;

#[derive(Clone, Copy, Debug)]
pub struct Quantity {
    pub value: f64,
    pub unit: Unit,
}
impl Quantity {
    pub fn in_unit(self, unit: Unit) -> Result<f64, Error> {
        self.unit.convert(self.value, unit)
    }
}
#[derive(Clone, Debug)]
pub struct BindingDefinition {
    pub name: String,
    pub unit: Unit,
    /// Inclusive bounds, converted and validated once during compilation.
    pub bounds: Option<(Quantity, Quantity)>,
}
#[derive(Clone, Debug)]
pub enum InitialCondition {
    Quantity(Quantity),
    Expression(Expr),
}
#[derive(Clone, Debug)]
pub struct UnitStateDefinition {
    pub name: String,
    pub unit: Unit,
    pub initial: InitialCondition,
    pub rhs: Expr,
    /// Dose conversion expression must have state-unit / dose-unit dimensions.
    pub dosing: Option<(Unit, Expr)>,
}
#[derive(Clone, Debug)]
pub struct UnitModelDefinition {
    pub time_unit: Unit,
    pub bindings: Vec<BindingDefinition>,
    pub states: Vec<UnitStateDefinition>,
}
#[derive(Debug)]
pub struct UnitCompiledModel {
    compiled: Arc<CompiledModel>,
    bindings: Vec<(Unit, Option<(f64, f64)>)>,
    pub time_unit: Unit,
    pub state_units: Vec<Unit>,
    pub dose_units: Vec<Option<Unit>>,
}
#[derive(Debug)]
pub struct UnitBoundProblem {
    compiled: Arc<UnitCompiledModel>,
    problem: BoundProblem,
}
fn contextual(mut error: Error, field: &str) -> Error {
    error.expression = Some(match error.expression {
        Some(path) => format!("{field}.{path}"),
        None => field.into(),
    });
    error
}
impl UnitCompiledModel {
    pub fn compile(definition: &UnitModelDefinition) -> Result<Arc<Self>, Error> {
        Self::compile_mode(definition, false)
    }
    pub(crate) fn compile_expanded(definition: &UnitModelDefinition) -> Result<Arc<Self>, Error> {
        Self::compile_mode(definition, true)
    }
    pub(crate) fn normalized_definition(
        definition: &UnitModelDefinition,
    ) -> Result<ModelDefinition, Error> {
        Ok(Self::lower_definition(definition, false)?.0)
    }
    fn lower_definition(
        definition: &UnitModelDefinition,
        expanded: bool,
    ) -> Result<(ModelDefinition, Vec<(Unit, Option<(f64, f64)>)>), Error> {
        definition.time_unit.conversion_to(Unit::parse("s")?)?;
        if definition.states.is_empty()
            || definition.states.len() > 1024
            || definition.bindings.len() > 4096
        {
            return Err(invalid("model exceeds supported state/binding bounds"));
        }
        let bindings = definition
            .bindings
            .iter()
            .map(|binding| {
                let bounds = binding
                    .bounds
                    .map(|(lo, hi)| -> Result<_, Error> {
                        let lo = lo.in_unit(binding.unit)?;
                        let hi = hi.in_unit(binding.unit)?;
                        if lo > hi {
                            return Err(invalid("binding bounds are reversed"));
                        }
                        Ok((lo, hi))
                    })
                    .transpose()
                    .map_err(|e| contextual(e, &format!("bindings.{}.bounds", binding.name)))?;
                Ok((binding.unit, bounds))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let binding_symbols: Vec<_> = definition
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
            .collect();
        let mut symbols = binding_symbols.clone();
        symbols.extend(definition.states.iter().map(|s| {
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
            definition.time_unit,
        ));
        for state in &definition.states {
            symbols.push((
                Symbol {
                    name: format!("input_{}", state.name),
                    kind: SymbolKind::Input,
                },
                state.unit.divide(definition.time_unit)?,
            ));
        }
        let binding_context = crate::compile::units::CompileContext::new(&binding_symbols)?;
        let state_context = crate::compile::units::CompileContext::new(&symbols)?;
        let states = definition
            .states
            .iter()
            .map(|state| {
                let initial = match &state.initial {
                    InitialCondition::Quantity(q) => {
                        Expr::number(q.in_unit(state.unit).map_err(|e| {
                            contextual(e, &format!("states.{}.initial", state.name))
                        })?)
                    }
                    InitialCondition::Expression(expr) => binding_context
                        .normalize(expr, state.unit, expanded)
                        .map_err(|e| contextual(e, &format!("states.{}.initial", state.name)))?,
                };
                let rhs = state_context
                    .normalize(
                        &state.rhs,
                        state.unit.divide(definition.time_unit)?,
                        expanded,
                    )
                    .map_err(|e| contextual(e, &format!("states.{}.rhs", state.name)))?;
                let dose_scale = state
                    .dosing
                    .as_ref()
                    .map(|(unit, expr)| {
                        binding_context.normalize(expr, state.unit.divide(*unit)?, expanded)
                    })
                    .transpose()
                    .map_err(|e| contextual(e, &format!("states.{}.dosing", state.name)))?;
                Ok(StateDefinition {
                    name: state.name.clone(),
                    initial,
                    rhs,
                    dose_scale,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok((
            ModelDefinition {
                bindings: definition.bindings.iter().map(|b| b.name.clone()).collect(),
                states,
            },
            bindings,
        ))
    }
    fn compile_mode(definition: &UnitModelDefinition, expanded: bool) -> Result<Arc<Self>, Error> {
        let (lowered, bindings) = Self::lower_definition(definition, expanded)?;
        let compiled = CompiledModel::compile_internal(&lowered, true)?;
        Ok(Arc::new(Self {
            compiled,
            bindings,
            time_unit: definition.time_unit,
            state_units: definition.states.iter().map(|s| s.unit).collect(),
            dose_units: definition
                .states
                .iter()
                .map(|s| s.dosing.as_ref().map(|(unit, _)| *unit))
                .collect(),
        }))
    }
    pub fn bind(self: &Arc<Self>, values: &[Quantity]) -> Result<UnitBoundProblem, Error> {
        self.bind_continuing(values, None)
    }
    pub(crate) fn binding_storage_values(&self) -> usize {
        2 * self.bindings.len() + 4 * self.state_units.len() + 1 + self.compiled.scratch_size
    }
    pub(crate) fn bind_continuing(
        self: &Arc<Self>,
        values: &[Quantity],
        continuing: Option<&[f64]>,
    ) -> Result<UnitBoundProblem, Error> {
        let initial = continuing.map(|v| v.iter().copied().map(Some).collect::<Vec<_>>());
        self.bind_initial(values, initial.as_deref())
    }
    pub(crate) fn bind_initial(
        self: &Arc<Self>,
        values: &[Quantity],
        initial_values: Option<&[Option<f64>]>,
    ) -> Result<UnitBoundProblem, Error> {
        if values.len() != self.bindings.len() {
            return Err(invalid("invalid quantity binding vector"));
        }
        let values = values
            .iter()
            .zip(&self.bindings)
            .enumerate()
            .map(|(i, (q, (unit, bounds)))| {
                let value = q
                    .in_unit(*unit)
                    .map_err(|e| contextual(e, &format!("bindings[{i}]")))?;
                if bounds.is_some_and(|(lo, hi)| value < lo || value > hi) {
                    return Err(contextual(
                        invalid("binding is outside its declared bounds"),
                        &format!("bindings[{i}]"),
                    ));
                }
                Ok(value)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(UnitBoundProblem {
            compiled: self.clone(),
            problem: self.compiled.bind_initial(&values, initial_values)?,
        })
    }
}
impl UnitBoundProblem {
    /// A failed conversion, bound check or evaluation preserves the previous binding.
    pub fn rebind(&mut self, values: &[Quantity]) -> Result<(), Error> {
        *self = self.compiled.bind(values)?;
        Ok(())
    }
}
impl Model for UnitBoundProblem {
    fn jacobian_sparsity(&self) -> Option<Vec<(usize, usize)>> {
        self.problem.jacobian_sparsity()
    }
    fn rhs_depends_on_time(&self) -> bool {
        self.problem.rhs_depends_on_time()
    }
    fn validate(&self) -> Result<(), Error> {
        self.problem.validate()
    }
    fn initial(&self) -> Vec<f64> {
        self.problem.initial()
    }
    fn dose_scale(&self, state: usize) -> Option<f64> {
        self.problem.dose_scale(state)
    }
    fn rhs(&self, time: f64, x: &[f64], rates: &[f64], out: &mut [f64]) -> Result<(), Error> {
        self.problem.rhs(time, x, rates, out)
    }
    fn jac_mul(
        &self,
        time: f64,
        x: &[f64],
        rates: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        self.problem.jac_mul(time, x, rates, v, out)
    }
}

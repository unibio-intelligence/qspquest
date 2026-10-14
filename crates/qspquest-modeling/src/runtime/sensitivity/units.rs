//! Quantity binding and derivative-unit metadata around augmented integration.
use super::*;
use crate::runtime::units::{Quantity, UnitCompiledModel, UnitModelDefinition};
use qspquest_core::units::Unit;
#[derive(Debug)]
pub struct UnitCompiledForwardSensitivity {
    primal: Arc<UnitCompiledModel>,
    forward: Arc<CompiledForwardSensitivity>,
    binding_units: Vec<Unit>,
    pub time_unit: Unit,
    pub dose_units: Vec<Option<Unit>>,
    pub parameter_units: Vec<Unit>,
    /// Primal state units, then state-unit / selected-binding-unit blocks.
    pub augmented_state_units: Vec<Unit>,
}
#[derive(Debug)]
pub struct UnitBoundForwardSensitivity {
    compiled: Arc<UnitCompiledForwardSensitivity>,
    inner: BoundForwardSensitivity,
}
impl UnitCompiledForwardSensitivity {
    pub(crate) fn binding_storage_values(&self) -> usize {
        self.forward.binding_storage_values() + self.primal.binding_storage_values()
    }
    pub fn compile(
        definition: &UnitModelDefinition,
        parameters: &[String],
    ) -> Result<Arc<Self>, Error> {
        let primal = UnitCompiledModel::compile(definition)?;
        let normalized = UnitCompiledModel::normalized_definition(definition)?;
        let forward = CompiledForwardSensitivity::compile(&normalized, parameters)?;
        let parameter_units = parameters
            .iter()
            .map(|p| {
                definition
                    .bindings
                    .iter()
                    .find(|b| &b.name == p)
                    .map(|b| b.unit)
                    .ok_or_else(|| invalid("unknown sensitivity binding"))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut augmented_state_units = primal.state_units.clone();
        for parameter_unit in &parameter_units {
            for state_unit in &primal.state_units {
                augmented_state_units.push(state_unit.divide(*parameter_unit)?);
            }
        }
        Ok(Arc::new(Self {
            time_unit: primal.time_unit,
            dose_units: primal.dose_units.clone(),
            primal,
            forward,
            binding_units: definition.bindings.iter().map(|b| b.unit).collect(),
            parameter_units,
            augmented_state_units,
        }))
    }
    pub fn parameters(&self) -> &[String] {
        self.forward.parameters()
    }
    pub fn state_count(&self) -> usize {
        self.forward.state_count()
    }
    pub fn bind(
        self: &Arc<Self>,
        quantities: &[Quantity],
    ) -> Result<UnitBoundForwardSensitivity, Error> {
        self.bind_initial(quantities, None)
    }
    pub(crate) fn bind_initial(
        self: &Arc<Self>,
        quantities: &[Quantity],
        overrides: Option<&[Option<f64>]>,
    ) -> Result<UnitBoundForwardSensitivity, Error> {
        // Reuse ordinary unit, bound and primal-domain validation.
        self.primal.bind_initial(quantities, overrides)?;
        let values = quantities
            .iter()
            .zip(&self.binding_units)
            .map(|(q, u)| q.in_unit(*u))
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(UnitBoundForwardSensitivity {
            compiled: self.clone(),
            inner: self.forward.bind_initial(&values, overrides)?,
        })
    }
}
impl UnitBoundForwardSensitivity {
    pub fn rebind(&mut self, quantities: &[Quantity]) -> Result<(), Error> {
        *self = self.compiled.bind(quantities)?;
        Ok(())
    }
}
impl Model for UnitBoundForwardSensitivity {
    fn rhs_depends_on_time(&self) -> bool {
        self.inner.rhs_depends_on_time()
    }
    fn validate(&self) -> Result<(), Error> {
        self.inner.validate()
    }
    fn initial(&self) -> Vec<f64> {
        self.inner.initial()
    }
    fn dose_scale(&self, target: usize) -> Option<f64> {
        self.inner.dose_scale(target)
    }
    fn apply_bolus(
        &self,
        time: f64,
        target: usize,
        amount: f64,
        state: &mut [f64],
    ) -> Result<(), Error> {
        self.inner.apply_bolus(time, target, amount, state)
    }
    fn apply_reset(
        &self,
        time: f64,
        target: usize,
        value: f64,
        state: &mut [f64],
    ) -> Result<(), Error> {
        self.inner.apply_reset(time, target, value, state)
    }
    fn accumulate_infusion_rate(
        &self,
        time: f64,
        target: usize,
        rate: f64,
        rates: &mut [f64],
    ) -> Result<(), Error> {
        self.inner
            .accumulate_infusion_rate(time, target, rate, rates)
    }
    fn rhs(&self, time: f64, state: &[f64], rates: &[f64], out: &mut [f64]) -> Result<(), Error> {
        self.inner.rhs(time, state, rates, out)
    }
    fn jac_mul(
        &self,
        time: f64,
        state: &[f64],
        rates: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        self.inner.jac_mul(time, state, rates, v, out)
    }
}

/// A smooth observable with an explicit output unit.
#[derive(Clone, Debug)]
pub struct UnitSensitivityReadout {
    pub expression: Expr,
    pub unit: Unit,
}
#[derive(Debug)]
pub struct UnitCompiledSensitivityReadouts {
    primal: Arc<UnitCompiledModel>,
    readouts: Arc<CompiledSensitivityReadouts>,
    binding_units: Vec<Unit>,
    pub output_units: Vec<Unit>,
    /// Selected parameter-major output-unit / declared binding-unit metadata.
    pub derivative_units: Vec<Vec<Unit>>,
}
#[derive(Debug)]
pub struct UnitBoundSensitivityReadouts {
    compiled: Arc<UnitCompiledSensitivityReadouts>,
    inner: BoundSensitivityReadouts,
}
impl UnitCompiledSensitivityReadouts {
    pub(crate) fn binding_storage_values(&self) -> usize {
        self.readouts.binding_storage_values() + self.primal.binding_storage_values()
    }
    pub fn compile(
        definition: &UnitModelDefinition,
        parameters: &[String],
        readouts: &[UnitSensitivityReadout],
    ) -> Result<Arc<Self>, Error> {
        if readouts.is_empty() || readouts.len() > 1024 {
            return Err(invalid("select 1..1024 sensitivity readouts"));
        }
        let primal = UnitCompiledModel::compile(definition)?;
        let normalized = UnitCompiledModel::normalized_definition(definition)?;
        let mut symbols = definition
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
        let expressions = readouts
            .iter()
            .map(|r| crate::compile::units::normalize(&r.expression, &symbols, r.unit))
            .collect::<Result<Vec<_>, Error>>()?;
        let compiled = CompiledSensitivityReadouts::compile(&normalized, parameters, &expressions)?;
        let derivative_units = parameters
            .iter()
            .map(|p| {
                let unit = definition
                    .bindings
                    .iter()
                    .find(|b| &b.name == p)
                    .ok_or_else(|| invalid("unknown sensitivity binding"))?
                    .unit;
                readouts
                    .iter()
                    .map(|r| r.unit.divide(unit))
                    .collect::<Result<Vec<_>, Error>>()
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Arc::new(Self {
            primal,
            readouts: compiled,
            binding_units: definition.bindings.iter().map(|b| b.unit).collect(),
            output_units: readouts.iter().map(|r| r.unit).collect(),
            derivative_units,
        }))
    }
    pub fn parameters(&self) -> &[String] {
        self.readouts.parameters()
    }
    pub fn bind(
        self: &Arc<Self>,
        quantities: &[Quantity],
    ) -> Result<UnitBoundSensitivityReadouts, Error> {
        self.bind_initial(quantities, None)
    }
    pub(crate) fn bind_initial(
        self: &Arc<Self>,
        quantities: &[Quantity],
        overrides: Option<&[Option<f64>]>,
    ) -> Result<UnitBoundSensitivityReadouts, Error> {
        self.primal.bind_initial(quantities, overrides)?;
        let values = quantities
            .iter()
            .zip(&self.binding_units)
            .map(|(q, u)| q.in_unit(*u))
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(UnitBoundSensitivityReadouts {
            compiled: self.clone(),
            inner: self.readouts.bind_initial(&values, overrides)?,
        })
    }
}
impl UnitBoundSensitivityReadouts {
    pub fn rebind(&mut self, quantities: &[Quantity]) -> Result<(), Error> {
        *self = self.compiled.bind(quantities)?;
        Ok(())
    }
    /// Time, augmented state and raw rates use the declared model units and
    /// parameter order of UnitCompiledForwardSensitivity. Rates are dose/time.
    pub fn evaluate(
        &self,
        time: f64,
        augmented: &[f64],
        raw_rates: &[f64],
    ) -> Result<SensitivityReadoutValues, Error> {
        self.inner.evaluate(time, augmented, raw_rates)
    }
}

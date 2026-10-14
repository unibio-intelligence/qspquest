//! Symbolic partial derivatives with respect to independent runtime bindings.
//! Derived-parameter chain rules and trajectory integration are separate layers.
use super::*;
mod derived;
mod units;
pub use units::{
    UnitBoundForwardSensitivity, UnitBoundSensitivityReadouts, UnitCompiledForwardSensitivity,
    UnitCompiledSensitivityReadouts, UnitSensitivityReadout,
};
mod forward;
mod readouts;
pub use derived::{DerivedBinding, ResolvedBindings};
pub use forward::{BoundForwardSensitivity, CompiledForwardSensitivity};
pub use readouts::{
    BoundSensitivityReadouts, CompiledSensitivityReadouts, SensitivityReadoutValues,
};
#[derive(Debug)]
pub struct CompiledSensitivity {
    primal: Arc<CompiledModel>,
    parameters: Vec<String>,
    initial: Vec<Vec<Program>>,
    scales: Vec<Vec<Option<Program>>>,
    rhs: Vec<Vec<Program>>,
    inputs: Vec<Vec<(usize, Program)>>,
}
#[derive(Debug)]
pub struct BoundSensitivity {
    compiled: Arc<CompiledSensitivity>,
    primal: BoundProblem,
    values: Vec<f64>,
    initial: Vec<Vec<f64>>,
    scales: Vec<Vec<f64>>,
}
impl CompiledSensitivity {
    pub(crate) fn binding_storage_values(&self) -> usize {
        self.primal.binding_storage_values()
            + self.primal.names.len()
            + 2 * self.parameters.len() * self.primal.initial.len()
    }
    pub fn compile(
        definition: &ModelDefinition,
        parameters: &[String],
    ) -> Result<Arc<Self>, Error> {
        let primal = CompiledModel::compile(definition)?;
        let unique: std::collections::BTreeSet<_> = parameters.iter().collect();
        if parameters.is_empty()
            || parameters.len() > 32
            || unique.len() != parameters.len()
            || parameters.iter().any(|p| !definition.bindings.contains(p))
        {
            return Err(invalid(
                "select 1..32 distinct independent binding names for sensitivities",
            ));
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
        let mut instructions = 0usize;
        let mut compile = |expr: &Expr, symbols: &[Symbol], name: &str| -> Result<Program, Error> {
            let program = Program::derivative(expr, symbols, name)?;
            instructions = instructions
                .checked_add(program.instruction_count())
                .ok_or_else(|| invalid("sensitivity instruction count overflow"))?;
            if instructions > 1_000_000 {
                return Err(Error::new(
                    ErrorCode::ExpressionLimit,
                    "sensitivities exceed one million instructions",
                ));
            }
            Ok(program)
        };
        let mut initial = vec![];
        let mut scales = vec![];
        let mut rhs = vec![];
        for parameter in parameters {
            initial.push(
                definition
                    .states
                    .iter()
                    .map(|s| compile(&s.initial, &bindings, parameter))
                    .collect::<Result<Vec<_>, _>>()?,
            );
            scales.push(
                definition
                    .states
                    .iter()
                    .map(|s| {
                        s.dose_scale
                            .as_ref()
                            .map(|e| compile(e, &bindings, parameter))
                            .transpose()
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );
            rhs.push(
                definition
                    .states
                    .iter()
                    .map(|s| compile(&s.rhs, &symbols, parameter))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        let mut inputs = vec![];
        for state in &definition.states {
            let mut row = vec![];
            for (j, input) in definition.states.iter().enumerate() {
                let name = format!("input_{}", input.name);
                if contains(&state.rhs, &name) {
                    row.push((j, compile(&state.rhs, &symbols, &name)?));
                }
            }
            inputs.push(row);
        }
        Ok(Arc::new(Self {
            primal,
            parameters: parameters.to_vec(),
            initial,
            scales,
            rhs,
            inputs,
        }))
    }
    pub fn parameters(&self) -> &[String] {
        &self.parameters
    }
    pub fn bind(self: &Arc<Self>, values: &[f64]) -> Result<BoundSensitivity, Error> {
        self.bind_initial(values, None)
    }
    pub(crate) fn bind_initial(
        self: &Arc<Self>,
        values: &[f64],
        overrides: Option<&[Option<f64>]>,
    ) -> Result<BoundSensitivity, Error> {
        let primal = self.primal.bind_initial(values, overrides)?;
        let initial = self
            .initial
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(i, p)| {
                        if overrides.is_some_and(|v| v[i].is_some()) {
                            Ok(0.)
                        } else {
                            p.evaluate(values)
                        }
                    })
                    .collect()
            })
            .collect::<Result<Vec<Vec<_>>, Error>>()?;
        let scales = self
            .scales
            .iter()
            .map(|row| {
                row.iter()
                    .map(|p| {
                        p.as_ref()
                            .map(|p| p.evaluate(values))
                            .transpose()
                            .map(|v| v.unwrap_or(0.))
                    })
                    .collect()
            })
            .collect::<Result<Vec<Vec<_>>, Error>>()?;
        Ok(BoundSensitivity {
            compiled: self.clone(),
            primal,
            values: values.to_vec(),
            initial,
            scales,
        })
    }
}
impl BoundSensitivity {
    pub fn primal(&self) -> &BoundProblem {
        &self.primal
    }
    /// Layout [selected parameter][state], in raw model/binding units.
    pub fn initial_derivatives(&self) -> &[Vec<f64>] {
        &self.initial
    }
    /// Multiply by unscaled event amount/rate for bolus/infusion derivatives.
    pub fn dose_scale_derivatives(&self) -> &[Vec<f64>] {
        &self.scales
    }
    pub fn rebind(&mut self, values: &[f64]) -> Result<(), Error> {
        *self = self.compiled.bind(values)?;
        Ok(())
    }
    /// df/dp + (df/du)*(du/dp), excluding the state-Jacobian times sensitivity.
    /// Raw input rates precede dose scaling, including at zero bioavailability.
    pub fn rhs_forcing(
        &self,
        time: f64,
        state: &[f64],
        raw_rates: &[f64],
    ) -> Result<Vec<Vec<f64>>, Error> {
        let n = self.primal.initial.len();
        if state.len() != n
            || raw_rates.len() != n
            || !time.is_finite()
            || state.iter().any(|v| !v.is_finite())
            || raw_rates.iter().any(|v| !v.is_finite() || *v < 0.)
        {
            return Err(invalid("invalid sensitivity state or raw input rate"));
        }
        let rates = raw_rates
            .iter()
            .enumerate()
            .map(|(i, r)| match self.primal.dose_scale(i) {
                Some(s) => Ok(r * s),
                None if *r == 0. => Ok(0.),
                _ => Err(invalid("raw input targets a nondosing state")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if rates.iter().any(|v| !v.is_finite()) {
            return Err(Error::new(
                ErrorCode::Domain,
                "scaled sensitivity input overflow",
            ));
        }
        let mut primal = vec![0.; n];
        self.primal.rhs(time, state, &rates, &mut primal)?;
        let mut inputs = self.values.clone();
        inputs.extend_from_slice(state);
        inputs.push(time);
        inputs.extend_from_slice(&rates);
        let input_partials = self
            .compiled
            .inputs
            .iter()
            .map(|row| {
                row.iter()
                    .map(|(j, p)| Ok((*j, p.evaluate(&inputs)?)))
                    .collect()
            })
            .collect::<Result<Vec<Vec<(usize, f64)>>, Error>>()?;
        self.compiled
            .rhs
            .iter()
            .enumerate()
            .map(|(parameter, row)| {
                row.iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let mut value = p.evaluate(&inputs)?;
                        for (j, partial) in &input_partials[i] {
                            value += partial * raw_rates[*j] * self.scales[parameter][*j];
                        }
                        if !value.is_finite() {
                            return Err(Error::new(
                                ErrorCode::Domain,
                                "sensitivity forcing overflow",
                            ));
                        }
                        Ok(value)
                    })
                    .collect()
            })
            .collect()
    }
}

mod unit_derived;
pub use unit_derived::{resolve_unit_bindings, UnitDerivedBinding};

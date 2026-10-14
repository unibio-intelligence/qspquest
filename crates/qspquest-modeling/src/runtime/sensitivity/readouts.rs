//! Total observable derivatives: explicit parameter, state and scaled input terms.
use super::*;
#[derive(Debug)]
pub struct CompiledSensitivityReadouts {
    derivatives: Arc<CompiledSensitivity>,
    values: Vec<Program>,
    parameters: Vec<Vec<Program>>,
    states: Vec<Vec<(usize, Program)>>,
    inputs: Vec<Vec<(usize, Program)>>,
    state_count: usize,
}
#[derive(Debug)]
pub struct BoundSensitivityReadouts {
    compiled: Arc<CompiledSensitivityReadouts>,
    bound: BoundSensitivity,
}
#[derive(Clone, Debug)]
pub struct SensitivityReadoutValues {
    pub values: Vec<f64>,
    /// [selected parameter][readout], in raw output/binding units.
    pub derivatives: Vec<Vec<f64>>,
}
impl CompiledSensitivityReadouts {
    pub(crate) fn binding_storage_values(&self) -> usize {
        self.derivatives.binding_storage_values()
    }
    pub fn compile(
        definition: &ModelDefinition,
        parameters: &[String],
        readouts: &[Expr],
    ) -> Result<Arc<Self>, Error> {
        if readouts.is_empty() || readouts.len() > 1024 {
            return Err(invalid("select 1..1024 sensitivity readouts"));
        }
        let variables = forward::variables(definition, parameters);
        if readouts.iter().any(|e| !forward::smooth(e, &variables)) {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "sensitivity readouts require smooth expressions",
            ));
        }
        let derivatives = CompiledSensitivity::compile(definition, parameters)?;
        let mut symbols: Vec<_> = definition
            .bindings
            .iter()
            .map(|name| Symbol {
                name: name.clone(),
                kind: SymbolKind::Parameter,
            })
            .collect();
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
        let mut count = 0usize;
        let mut account = |p: Program| -> Result<Program, Error> {
            count = count
                .checked_add(p.instruction_count())
                .ok_or_else(|| invalid("readout instruction overflow"))?;
            if count > 1_000_000 {
                return Err(Error::new(
                    ErrorCode::ExpressionLimit,
                    "sensitivity readouts exceed one million instructions",
                ));
            }
            Ok(p)
        };
        let values = readouts
            .iter()
            .map(|e| account(numeric(Program::compile(e, &symbols)?)?))
            .collect::<Result<Vec<_>, _>>()?;
        let partials = parameters
            .iter()
            .map(|p| {
                readouts
                    .iter()
                    .map(|e| account(Program::derivative(e, &symbols, p)?))
                    .collect()
            })
            .collect::<Result<Vec<Vec<_>>, Error>>()?;
        let mut states = vec![];
        let mut inputs = vec![];
        for e in readouts {
            let mut dy = vec![];
            let mut du = vec![];
            for (i, s) in definition.states.iter().enumerate() {
                if contains(e, &s.name) {
                    dy.push((i, account(Program::derivative(e, &symbols, &s.name)?)?));
                }
                let name = format!("input_{}", s.name);
                if contains(e, &name) {
                    du.push((i, account(Program::derivative(e, &symbols, &name)?)?));
                }
            }
            states.push(dy);
            inputs.push(du);
        }
        Ok(Arc::new(Self {
            derivatives,
            values,
            parameters: partials,
            states,
            inputs,
            state_count: definition.states.len(),
        }))
    }
    pub fn parameters(&self) -> &[String] {
        self.derivatives.parameters()
    }
    pub fn bind(self: &Arc<Self>, values: &[f64]) -> Result<BoundSensitivityReadouts, Error> {
        self.bind_initial(values, None)
    }
    pub(crate) fn bind_initial(
        self: &Arc<Self>,
        values: &[f64],
        overrides: Option<&[Option<f64>]>,
    ) -> Result<BoundSensitivityReadouts, Error> {
        Ok(BoundSensitivityReadouts {
            compiled: self.clone(),
            bound: self.derivatives.bind_initial(values, overrides)?,
        })
    }
}
impl BoundSensitivityReadouts {
    pub fn rebind(&mut self, values: &[f64]) -> Result<(), Error> {
        *self = self.compiled.bind(values)?;
        Ok(())
    }
    /// Supply the same augmented row/order as CompiledForwardSensitivity and the
    /// raw input rates on the desired observation side of the event boundary.
    pub fn evaluate(
        &self,
        time: f64,
        augmented: &[f64],
        raw_rates: &[f64],
    ) -> Result<SensitivityReadoutValues, Error> {
        let n = self.compiled.state_count;
        let p = self.compiled.parameters.len();
        if augmented.len() != n * (p + 1)
            || raw_rates.len() != n
            || !time.is_finite()
            || augmented.iter().any(|x| !x.is_finite())
            || raw_rates.iter().any(|x| !x.is_finite() || *x < 0.)
        {
            return Err(invalid("invalid sensitivity readout inputs"));
        }
        let rates = raw_rates
            .iter()
            .enumerate()
            .map(|(i, r)| match self.bound.primal.dose_scale(i) {
                Some(scale) => Ok(r * scale),
                None if *r == 0. => Ok(0.),
                _ => Err(invalid("input targets a nondosing state")),
            })
            .collect::<Result<Vec<_>, Error>>()?;
        if rates.iter().any(|r| !r.is_finite()) {
            return Err(Error::new(ErrorCode::Domain, "readout input overflow"));
        }
        let mut inputs = self.bound.values.clone();
        inputs.extend_from_slice(&augmented[..n]);
        inputs.push(time);
        inputs.extend_from_slice(&rates);
        let values = self
            .compiled
            .values
            .iter()
            .map(|p| p.evaluate(&inputs))
            .collect::<Result<Vec<_>, _>>()?;
        let partials = |rows: &Vec<Vec<(usize, Program)>>| {
            rows.iter()
                .map(|row| {
                    row.iter()
                        .map(|(j, p)| Ok((*j, p.evaluate(&inputs)?)))
                        .collect()
                })
                .collect::<Result<Vec<Vec<(usize, f64)>>, Error>>()
        };
        let dy = partials(&self.compiled.states)?;
        let du = partials(&self.compiled.inputs)?;
        let derivatives = self
            .compiled
            .parameters
            .iter()
            .enumerate()
            .map(|(p, row)| {
                row.iter()
                    .enumerate()
                    .map(|(o, program)| {
                        let mut value = program.evaluate(&inputs)?;
                        for (j, partial) in &dy[o] {
                            value += partial * augmented[n + p * n + j];
                        }
                        for (j, partial) in &du[o] {
                            value += partial * raw_rates[*j] * self.bound.scales[p][*j];
                        }
                        if !value.is_finite() {
                            return Err(Error::new(
                                ErrorCode::Domain,
                                "readout derivative overflow",
                            ));
                        }
                        Ok(value)
                    })
                    .collect()
            })
            .collect::<Result<Vec<Vec<_>>, Error>>()?;
        Ok(SensitivityReadoutValues {
            values,
            derivatives,
        })
    }
}

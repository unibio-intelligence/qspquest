//! Smooth, fixed-boundary augmented ODE compilation for independent bindings.
use super::*;
use qspquest_core::expression::{Binary, Function};
#[derive(Debug)]
pub struct CompiledForwardSensitivity {
    augmented: Arc<CompiledModel>,
    derivatives: Arc<CompiledSensitivity>,
    states: usize,
}
#[derive(Debug)]
pub struct BoundForwardSensitivity {
    compiled: Arc<CompiledForwardSensitivity>,
    augmented: BoundProblem,
    derivatives: BoundSensitivity,
}
pub(super) fn variables(
    definition: &ModelDefinition,
    parameters: &[String],
) -> std::collections::BTreeSet<String> {
    parameters
        .iter()
        .cloned()
        .chain(definition.states.iter().map(|s| s.name.clone()))
        .chain(
            definition
                .states
                .iter()
                .map(|s| format!("input_{}", s.name)),
        )
        .chain(std::iter::once("time".into()))
        .collect()
}
pub(super) fn smooth(expr: &Expr, variables: &std::collections::BTreeSet<String>) -> bool {
    // A subtree independent of the differentiated bindings, states, inputs and
    // clock is a constant in this sensitivity problem, even if it has a clamp.
    fn depends(expr: &Expr, variables: &std::collections::BTreeSet<String>) -> bool {
        matches!(expr, Expr::Symbol { name } if variables.contains(name))
            || expr.children().iter().any(|e| depends(e, variables))
    }
    match expr {
        Expr::Conditional { .. } | Expr::Compare { .. } | Expr::Boolean { .. } => {
            return !depends(expr, variables)
        }
        Expr::Binary {
            operator: Binary::Modulo,
            ..
        } => return !depends(expr, variables),
        Expr::Call {
            name:
                Function::Abs
                | Function::Min
                | Function::Max
                | Function::Floor
                | Function::Ceil
                | Function::Ifelse,
            ..
        } => return !depends(expr, variables),
        _ => {}
    }
    expr.children().iter().all(|e| smooth(e, variables))
}
impl CompiledForwardSensitivity {
    pub(crate) fn binding_storage_values(&self) -> usize {
        self.augmented.binding_storage_values() + self.derivatives.binding_storage_values()
    }
    pub fn compile(
        definition: &ModelDefinition,
        parameters: &[String],
    ) -> Result<Arc<Self>, Error> {
        let derivatives = CompiledSensitivity::compile(definition, parameters)?;
        let n = definition.states.len();
        if n.checked_mul(parameters.len() + 1).is_none_or(|n| n > 1024) {
            return Err(invalid("augmented sensitivity model exceeds 1024 states"));
        }
        let variables = variables(definition, parameters);
        if definition.states.iter().any(|s| {
            !smooth(&s.initial, &variables)
                || !smooth(&s.rhs, &variables)
                || s.dose_scale
                    .as_ref()
                    .is_some_and(|s| !smooth(s, &variables))
        }) {
            return Err(Error::new(ErrorCode::Unsupported,"forward sensitivity trajectories require smooth expressions; nonsmooth crossings are unqualified"));
        }
        let name = |p: usize, i: usize| format!("qspquest_sensitivity_{p}_{i}");
        for p in 0..parameters.len() {
            for i in 0..n {
                let generated = name(p, i);
                if definition.bindings.contains(&generated)
                    || definition.states.iter().any(|s| s.name == generated)
                {
                    return Err(invalid("generated sensitivity name collision"));
                }
            }
        }
        let d = crate::compile::derivative_expression;
        let add = |a, b| Expr::binary(Binary::Add, a, b);
        let mul = |a, b| Expr::binary(Binary::Multiply, a, b);
        let mut augmented = definition.clone();
        for (p, parameter) in parameters.iter().enumerate() {
            for (i, state) in definition.states.iter().enumerate() {
                let mut rhs = d(&state.rhs, parameter)?;
                for (j, other) in definition.states.iter().enumerate() {
                    if contains(&state.rhs, &other.name) {
                        rhs = add(
                            rhs,
                            mul(d(&state.rhs, &other.name)?, Expr::symbol(name(p, j))),
                        );
                    }
                    let input = format!("input_{}", other.name);
                    if let Some(scale) = &other.dose_scale {
                        if contains(&state.rhs, &input) && contains(scale, parameter) {
                            rhs = add(
                                rhs,
                                mul(
                                    mul(d(&state.rhs, &input)?, d(scale, parameter)?),
                                    Expr::symbol(format!("input_{}", name(0, j))),
                                ),
                            );
                        }
                    }
                }
                augmented.states.push(StateDefinition {
                    name: name(p, i),
                    initial: d(&state.initial, parameter)?,
                    rhs,
                    dose_scale: None,
                });
            }
        }
        // Source expressions were validated above; generated derivative equations
        // use the existing bounded compiler-expansion budget.
        let augmented = CompiledModel::compile_internal(&augmented, true)?;
        Ok(Arc::new(Self {
            augmented,
            derivatives,
            states: n,
        }))
    }
    pub fn parameters(&self) -> &[String] {
        self.derivatives.parameters()
    }
    pub fn state_count(&self) -> usize {
        self.states
    }
    pub fn bind(self: &Arc<Self>, values: &[f64]) -> Result<BoundForwardSensitivity, Error> {
        self.bind_initial(values, None)
    }
    pub(crate) fn bind_initial(
        self: &Arc<Self>,
        values: &[f64],
        overrides: Option<&[Option<f64>]>,
    ) -> Result<BoundForwardSensitivity, Error> {
        let derivatives = self.derivatives.bind_initial(values, overrides)?;
        let augmented_overrides = overrides.map(|initial| {
            let mut result = initial.to_vec();
            for _ in self.parameters() {
                result.extend(initial.iter().map(|v| v.map(|_| 0.)));
            }
            result
        });
        Ok(BoundForwardSensitivity {
            compiled: self.clone(),
            derivatives,
            augmented: self
                .augmented
                .bind_initial(values, augmented_overrides.as_deref())?,
        })
    }
}
impl BoundForwardSensitivity {
    pub fn rebind(&mut self, values: &[f64]) -> Result<(), Error> {
        *self = self.compiled.bind(values)?;
        Ok(())
    }
    fn target(&self, target: usize) -> Result<(), Error> {
        if target >= self.compiled.states {
            Err(invalid("events must target primal states"))
        } else {
            Ok(())
        }
    }
}
impl Model for BoundForwardSensitivity {
    fn rhs_depends_on_time(&self) -> bool {
        self.augmented.rhs_depends_on_time()
    }
    fn validate(&self) -> Result<(), Error> {
        self.augmented.validate()
    }
    fn initial(&self) -> Vec<f64> {
        self.augmented.initial()
    }
    fn dose_scale(&self, target: usize) -> Option<f64> {
        if target < self.compiled.states {
            self.augmented.dose_scale(target)
        } else {
            None
        }
    }
    fn apply_bolus(
        &self,
        time: f64,
        target: usize,
        amount: f64,
        state: &mut [f64],
    ) -> Result<(), Error> {
        self.target(target)?;
        self.augmented.apply_bolus(time, target, amount, state)?;
        let n = self.compiled.states;
        for (p, row) in self.derivatives.scales.iter().enumerate() {
            state[n + p * n + target] += amount * row[target];
        }
        Ok(())
    }
    fn apply_reset(
        &self,
        time: f64,
        target: usize,
        value: f64,
        state: &mut [f64],
    ) -> Result<(), Error> {
        self.target(target)?;
        self.augmented.apply_reset(time, target, value, state)?;
        let n = self.compiled.states;
        for p in 0..self.compiled.parameters().len() {
            state[n + p * n + target] = 0.;
        }
        Ok(())
    }
    fn accumulate_infusion_rate(
        &self,
        time: f64,
        target: usize,
        rate: f64,
        rates: &mut [f64],
    ) -> Result<(), Error> {
        self.target(target)?;
        self.augmented
            .accumulate_infusion_rate(time, target, rate, rates)?;
        // First sensitivity input block carries raw rates; its ODE state remains S.
        rates[self.compiled.states + target] += rate;
        Ok(())
    }
    fn rhs(&self, time: f64, state: &[f64], rates: &[f64], out: &mut [f64]) -> Result<(), Error> {
        self.augmented.rhs(time, state, rates, out)
    }
    fn jac_mul(
        &self,
        time: f64,
        state: &[f64],
        rates: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        self.augmented.jac_mul(time, state, rates, v, out)
    }
}

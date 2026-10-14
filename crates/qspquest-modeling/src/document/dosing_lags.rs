//! Parameter-bound model lag is additive to explicit administration lag.
use super::*;
impl CompiledDocument {
    pub(super) fn expand_dose_lags(
        &self,
        request: &qspquest_core::regimen::Request,
        parameters: &BTreeMap<String, Quantity>,
        covariates: &BTreeMap<String, Quantity>,
    ) -> Result<qspquest_core::regimen::Request, Error> {
        let mut result = request.clone();
        if self.dose_lags.is_empty() {
            return Ok(result);
        }
        let mut values = self
            .parameters
            .iter()
            .map(|p| {
                quantity(parameters.get(&p.name).unwrap_or(&p.default))?
                    .in_unit(Unit::parse(&p.default.unit)?)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        for c in self
            .covariates
            .iter()
            .filter(|c| c.interpolation == Interpolation::Constant)
        {
            let q = covariates
                .get(&c.name)
                .or(c.default.as_ref())
                .ok_or_else(|| invalid(format!("missing constant covariate: {}", c.name)))?;
            values.push(quantity(q)?.in_unit(Unit::parse(&c.unit)?)?);
        }
        for (name, program) in &self.dose_lags {
            let lag = program.evaluate(&values).map_err(|mut e| {
                e.expression = Some(format!("states.{name}.dosing.lag"));
                e
            })?;
            if !lag.is_finite() || lag < 0.0 {
                return Err(invalid(format!(
                    "model lag for {name} must be finite and nonnegative"
                )));
            }
            for administration in &mut result.administrations {
                if administration.target != *name {
                    continue;
                }
                let explicit = administration
                    .lag
                    .as_ref()
                    .map(|q| quantity(q)?.in_unit(self.model.time_unit))
                    .transpose()?
                    .unwrap_or(0.0);
                if explicit < 0.0 || !explicit.is_finite() || !(lag + explicit).is_finite() {
                    return Err(invalid("administration lag must be finite and nonnegative"));
                }
                administration.lag = Some(Quantity {
                    value: lag + explicit,
                    unit: self.time_unit.clone(),
                });
            }
        }
        Ok(result)
    }
}

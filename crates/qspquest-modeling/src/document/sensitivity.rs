//! Typed document compilation for smooth, constant-binding sensitivities.
mod fit;
mod fit_planning;
mod objective;
mod outputs;
mod planning;
mod population_focei;
mod population_laplace;
mod population_saem;
mod regimen;
mod run;
use super::*;
use crate::runtime::sensitivity::*;
pub use outputs::SensitivitySample;
#[derive(Debug)]
pub struct CompiledSensitivityDocument {
    primal: Arc<CompiledDocument>,
    forward: Arc<UnitCompiledForwardSensitivity>,
    readouts: Arc<UnitCompiledSensitivityReadouts>,
    binding_units: Vec<Unit>,
}
#[derive(Debug)]
pub struct BoundSensitivityDocument {
    compiled: Arc<CompiledSensitivityDocument>,
    primal: BoundDocument,
    forward: UnitBoundForwardSensitivity,
    readouts: UnitBoundSensitivityReadouts,
    checkpoints: Vec<f64>,
}
impl CompiledSensitivityDocument {
    pub fn compile(document: &ModelDocument, parameters: &[String]) -> Result<Arc<Self>, Error> {
        let primal = CompiledDocument::compile(document)?;
        let lowered;
        let document = if document.linear_pk.is_some() {
            lowered = CompiledDocument::lower_linear_pk(document)?;
            &lowered
        } else {
            document
        };
        if !document.switches.is_empty()
            || !document.dose_switches.is_empty()
            || !document.dose_history.is_empty()
        {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "document sensitivities do not yet support history or moving event boundaries",
            ));
        }
        if parameters
            .iter()
            .any(|name| !document.parameters.iter().any(|p| &p.name == name))
        {
            return Err(invalid(
                "select declared model parameters for document sensitivities",
            ));
        }
        let mut definition = runtime::UnitModelDefinition {
            time_unit: Unit::parse(&document.time_unit)?,
            bindings: document
                .parameters
                .iter()
                .map(|p| {
                    Ok(runtime::BindingDefinition {
                        name: p.name.clone(),
                        unit: Unit::parse(&p.default.unit)?,
                        bounds: p
                            .bounds
                            .as_ref()
                            .map(|[a, b]| Ok::<_, Error>((quantity(a)?, quantity(b)?)))
                            .transpose()?,
                    })
                })
                .collect::<Result<_, Error>>()?,
            states: document
                .states
                .iter()
                .map(|s| {
                    Ok(runtime::UnitStateDefinition {
                        name: s.name.clone(),
                        unit: Unit::parse(&s.unit)?,
                        initial: match &s.initial {
                            Initial::Quantity { quantity: q } => {
                                runtime::InitialCondition::Quantity(quantity(q)?)
                            }
                            Initial::Expression { expression } => {
                                runtime::InitialCondition::Expression(expression.clone())
                            }
                        },
                        rhs: s.rhs.clone(),
                        dosing: s
                            .dosing
                            .as_ref()
                            .map(|d| {
                                Ok::<_, Error>((Unit::parse(&d.amount_unit)?, d.scale.clone()))
                            })
                            .transpose()?,
                    })
                })
                .collect::<Result<_, Error>>()?,
        };
        for c in &document.covariates {
            definition.bindings.push(runtime::BindingDefinition {
                name: c.name.clone(),
                unit: Unit::parse(&c.unit)?,
                bounds: None,
            });
        }
        let derived = document
            .individual
            .iter()
            .map(|p| {
                Ok(UnitDerivedBinding {
                    name: p.name.clone(),
                    unit: primal
                        .individual
                        .iter()
                        .find(|(n, _, _)| n == &p.name)
                        .ok_or_else(|| invalid("missing individual unit"))?
                        .1,
                    expression: p.expression.clone(),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut outputs = document
            .outputs
            .iter()
            .map(|o| {
                Ok(UnitSensitivityReadout {
                    expression: o.expression.clone(),
                    unit: Unit::parse(&o.unit)?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut lags = document
            .states
            .iter()
            .filter_map(|s| {
                s.dosing
                    .as_ref()
                    .and_then(|d| d.lag.as_ref())
                    .map(|e| (s.name.clone(), e.clone()))
            })
            .collect::<Vec<_>>();
        if !document.definitions.is_empty() {
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
            symbols.extend(derived.iter().map(|b| {
                (
                    Symbol {
                        name: b.name.clone(),
                        kind: SymbolKind::Parameter,
                    },
                    b.unit,
                )
            }));
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
            for s in &definition.states {
                symbols.push((
                    Symbol {
                        name: format!("input_{}", s.name),
                        kind: SymbolKind::Input,
                    },
                    s.unit.divide(definition.time_unit)?,
                ));
            }
            let mut expanded = definitions::Expanded::new(&document.definitions, &symbols)?;
            for state in &mut definition.states {
                state.rhs = expanded.expand(&state.rhs, "sensitivity.rhs")?;
                if let runtime::InitialCondition::Expression(e) = &mut state.initial {
                    *e = expanded.expand(e, "sensitivity.initial")?;
                }
                if let Some((_, e)) = &mut state.dosing {
                    *e = expanded.expand(e, "sensitivity.dosing")?;
                }
            }
            for o in &mut outputs {
                o.expression = expanded.expand(&o.expression, "sensitivity.output")?;
            }
            for (name, expression) in &mut lags {
                *expression = expanded.expand(expression, &format!("states.{name}.dosing.lag"))?;
            }
        }
        fn depends(expression: &qspquest_core::expression::Expr, parameter: &str) -> bool {
            matches!(expression,qspquest_core::expression::Expr::Symbol{name} if name==parameter)
                || expression.children().iter().any(|e| depends(e, parameter))
        }
        for (state, expression) in lags {
            if let Some(parameter) = parameters.iter().find(|p| depends(&expression, p)) {
                let mut error=Error::new(ErrorCode::Unsupported,format!("model lag depends on selected sensitivity parameter {parameter}; moving-event derivatives are not implemented"));
                error.expression = Some(format!("states.{state}.dosing.lag"));
                return Err(error);
            }
        }
        let (definition, outputs) = resolve_unit_bindings(&definition, &derived, &outputs)?;
        Ok(Arc::new(Self {
            forward: UnitCompiledForwardSensitivity::compile(&definition, parameters)?,
            readouts: UnitCompiledSensitivityReadouts::compile(&definition, parameters, &outputs)?,
            binding_units: definition.bindings.iter().map(|b| b.unit).collect(),
            primal,
        }))
    }
    pub fn parameters(&self) -> &[String] {
        self.forward.parameters()
    }
    pub fn output_names(&self) -> &[String] {
        &self.primal.output_names
    }
    pub fn output_units(&self) -> &[String] {
        &self.primal.output_units
    }
    pub fn derivative_units(&self) -> &[Vec<Unit>] {
        &self.readouts.derivative_units
    }
    pub fn augmented_state_units(&self) -> &[Unit] {
        &self.forward.augmented_state_units
    }
    pub fn bind(
        self: &Arc<Self>,
        parameters: &BTreeMap<String, Quantity>,
        covariates: &BTreeMap<String, Quantity>,
    ) -> Result<BoundSensitivityDocument, Error> {
        self.bind_with_initial_states(parameters, covariates, &BTreeMap::new())
    }
    pub fn bind_with_initial_states(
        self: &Arc<Self>,
        parameters: &BTreeMap<String, Quantity>,
        covariates: &BTreeMap<String, Quantity>,
        initial_states: &BTreeMap<String, Quantity>,
    ) -> Result<BoundSensitivityDocument, Error> {
        let primal =
            self.primal
                .bind_with_initial_states(parameters, covariates, initial_states)?;
        let overrides = self
            .primal
            .state_names
            .iter()
            .zip(&self.primal.model.state_units)
            .map(|(name, unit)| {
                initial_states
                    .get(name)
                    .map(|q| quantity(q)?.in_unit(*unit))
                    .transpose()
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let values = primal
            .values
            .iter()
            .zip(&self.binding_units)
            .map(|(v, u)| runtime::Quantity {
                value: *v,
                unit: *u,
            })
            .collect::<Vec<_>>();
        Ok(BoundSensitivityDocument {
            compiled: self.clone(),
            checkpoints: Vec::new(),
            forward: self.forward.bind_initial(&values, Some(&overrides))?,
            readouts: self.readouts.bind_initial(&values, Some(&overrides))?,
            primal,
        })
    }
}
impl BoundSensitivityDocument {
    pub fn rebind(
        &mut self,
        parameters: &BTreeMap<String, Quantity>,
        covariates: &BTreeMap<String, Quantity>,
    ) -> Result<(), Error> {
        *self = self.compiled.bind_with_initial_states(
            parameters,
            covariates,
            &self.primal.initial_overrides,
        )?;
        Ok(())
    }
    pub fn outputs(
        &self,
        time: f64,
        augmented: &[f64],
        raw_rates: &[f64],
    ) -> Result<SensitivityReadoutValues, Error> {
        self.readouts.evaluate(time, augmented, raw_rates)
    }
}
impl Model for BoundSensitivityDocument {
    fn rhs_depends_on_time(&self) -> bool {
        self.forward.rhs_depends_on_time()
    }
    fn discontinuities(&self) -> &[f64] {
        &self.checkpoints
    }
    fn validate(&self) -> Result<(), Error> {
        self.primal.validate()?;
        self.forward.validate()
    }
    fn initial(&self) -> Vec<f64> {
        self.forward.initial()
    }
    fn check_delivery(&self, event: &qspquest_core::Event) -> Result<(), Error> {
        self.primal.check_delivery(event)
    }
    fn check_state(&self, time: f64, state: &[f64]) -> Result<(), Error> {
        if state.len() != self.compiled.forward.augmented_state_units.len() {
            return Err(invalid("invalid augmented state dimension"));
        }
        self.primal
            .check_state(time, &state[..self.compiled.primal.state_count])
    }
    fn dose_scale(&self, target: usize) -> Option<f64> {
        self.forward.dose_scale(target)
    }
    fn apply_bolus(
        &self,
        time: f64,
        target: usize,
        amount: f64,
        state: &mut [f64],
    ) -> Result<(), Error> {
        self.forward.apply_bolus(time, target, amount, state)
    }
    fn apply_reset(
        &self,
        time: f64,
        target: usize,
        value: f64,
        state: &mut [f64],
    ) -> Result<(), Error> {
        self.forward.apply_reset(time, target, value, state)
    }
    fn accumulate_infusion_rate(
        &self,
        time: f64,
        target: usize,
        rate: f64,
        rates: &mut [f64],
    ) -> Result<(), Error> {
        self.forward
            .accumulate_infusion_rate(time, target, rate, rates)
    }
    fn rhs(&self, time: f64, state: &[f64], rates: &[f64], out: &mut [f64]) -> Result<(), Error> {
        self.forward.rhs(time, state, rates, out)
    }
    fn jac_mul(
        &self,
        time: f64,
        state: &[f64],
        rates: &[f64],
        v: &[f64],
        out: &mut [f64],
    ) -> Result<(), Error> {
        self.forward.jac_mul(time, state, rates, v, out)
    }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    use qspquest_core::{fit::*, run::*};
    use serde_json::json;

    pub(super) fn fixture(
        undefined_readout: bool,
    ) -> (Arc<CompiledSensitivityDocument>, SensitivityRequest) {
        let mut source: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../conformance/models/synthetic-one-compartment.json"
        ))
        .unwrap();
        if undefined_readout {
            source["outputs"].as_array_mut().unwrap().push(json!({
                "name":"undefined_readout", "unit":"1", "expression":{
                    "kind":"binary", "operator":"divide", "arguments":[
                        {"kind":"literal", "value":0}, {"kind":"literal", "value":0}
                    ]
                }
            }));
        }
        let request: SensitivityRequest = serde_json::from_str(include_str!(
            "../../../../conformance/requests/synthetic-sensitivity.json"
        ))
        .unwrap();
        let model = CompiledSensitivityDocument::compile(
            &serde_json::from_value(source).unwrap(),
            &request.with_respect_to,
        )
        .unwrap();
        (model, request)
    }
    pub(super) fn objective(simulation: SensitivityRequest) -> GaussianObjectiveRequest {
        serde_json::from_value(json!({"simulation":simulation,"priors":[],"observations":[{
            "row":1,"output":"cp","value":{"value":1,"unit":"mg/L"},
            "error":{"model":"normal","additive_sd":{"value":0.1,"unit":"mg/L"},"proportional_sd":0}
        }]}))
        .unwrap()
    }

    #[test]
    fn sensitivity_preparation_refuses_invalid_inputs_without_readouts() {
        let (model, request) = fixture(true);
        assert!(model.prepare_run(&request).is_ok());
        assert_eq!(model.run(&request).unwrap_err().code, ErrorCode::Domain);
        for case in 0..8 {
            let mut invalid = request.clone();
            match case {
                0 => invalid.with_respect_to.reverse(),
                1 => invalid.run.solver = Solver::DiffsolTsit45,
                2 => invalid.run.seed = Some(1),
                3 => {
                    invalid.derivative_absolute_tolerances.remove("cl");
                }
                4 => {
                    invalid
                        .derivative_absolute_tolerances
                        .get_mut("cl")
                        .unwrap()
                        .get_mut("central")
                        .unwrap()
                        .unit = "h".into()
                }
                5 => {
                    invalid.run.initial_states.insert(
                        "unknown".into(),
                        Quantity {
                            value: 0.,
                            unit: "mg".into(),
                        },
                    );
                }
                6 => invalid.run.regimen.samples[0].value = 5.,
                _ => invalid.run.budgets.output_values = 1,
            }
            let prepared = model.prepare_run(&invalid).err().unwrap();
            assert_eq!(prepared, model.run(&invalid).unwrap_err(), "case {case}");
        }
    }

    #[test]
    fn gaussian_rows_units_and_lognormal_data_are_validated_before_solving() {
        let (model, simulation) = fixture(true);
        let request = objective(simulation);
        assert!(model.prepare_gaussian_objective(&request).is_ok());
        assert_eq!(
            model.gaussian_objective(&request).unwrap_err().code,
            ErrorCode::Domain
        );
        for case in 0..3 {
            let mut invalid = request.clone();
            match case {
                0 => invalid.observations[0].row = 999,
                1 => invalid.observations[0].value.unit = "h".into(),
                _ => {
                    invalid.observations[0].value.value = 0.;
                    invalid.observations[0].error.model = GaussianErrorModel::Lognormal;
                    invalid.observations[0].error.additive_sd.value = 0.;
                    invalid.observations[0].error.proportional_sd = 0.1;
                }
            }
            let prepared = model.prepare_gaussian_objective(&invalid).err().unwrap();
            assert_eq!(prepared.expression.as_deref(), Some("observations[0]"));
            assert_eq!(prepared, model.gaussian_objective(&invalid).unwrap_err());
            assert_eq!(prepared, model.gaussian_primal_nll(&invalid).unwrap_err());
        }
    }

    #[test]
    fn pooled_preparation_checks_later_subjects_before_primary_readouts() {
        let (model, simulation) = fixture(true);
        let objective = objective(simulation);
        let fit: GaussianFitRequest = serde_json::from_value(json!({
            "objective":objective,"bounds":[
                {"parameter":"cl","lower":{"value":0.1,"unit":"L/h"},"upper":{"value":1,"unit":"L/h"}},
                {"parameter":"v","lower":{"value":1,"unit":"L"},"upper":{"value":5,"unit":"L"}}
            ],"max_evaluations":10,"max_iterations":10,"gradient_tolerance":0.00001
        })).unwrap();
        assert!(model.prepare_gaussian_fit(&fit, &[]).is_ok());
        assert_eq!(
            model.fit_gaussian(&fit).unwrap_err().code,
            ErrorCode::Domain
        );
        let mut later = objective;
        later.observations[0].row = 999;
        let prepared = model
            .prepare_gaussian_fit(&fit, &[later.clone()])
            .err()
            .unwrap();
        assert_eq!(
            prepared.expression.as_deref(),
            Some("subjects[1].observations[0]")
        );
        assert_eq!(
            prepared,
            model
                .fit_pooled_gaussian(&PooledGaussianFitRequest {
                    fit,
                    additional_subjects: vec![later]
                })
                .unwrap_err()
        );
    }

    #[test]
    fn sensitivity_projection_preserves_duplicate_observations_and_sides() {
        let (model, mut request) = fixture(false);
        request.run.regimen.samples.clear();
        request.run.regimen.checkpoints = vec![Quantity {
            value: 2.,
            unit: "h".into(),
        }];
        request.run.regimen.observations = Some(
            serde_json::from_value(json!([
                {"time":{"value":2,"unit":"h"},"side":"pre"},
                {"time":{"value":0,"unit":"h"},"side":"post"},
                {"time":{"value":0,"unit":"h"},"side":"pre"},
                {"time":{"value":1,"unit":"h"},"side":"pre"},
                {"time":{"value":1,"unit":"h"},"side":"post"},
                {"time":{"value":0,"unit":"h"},"side":"post"}
            ]))
            .unwrap(),
        );
        let result = model.run(&request).unwrap();
        assert_eq!(result.result.times, [2., 0., 0., 1., 1., 0.]);
        use qspquest_core::regimen::ObservationSide::{Post, Pre};
        assert_eq!(result.result.sides, [Pre, Post, Pre, Pre, Post, Post]);
        assert_eq!(result.result.outputs[0].values[2], 0.);
        for values in std::iter::once(&result.result.outputs[0].values)
            .chain(result.derivatives.iter().map(|column| &column.values))
        {
            assert_eq!(values[3], values[4]);
            assert_eq!(values[1], values[5]);
        }
    }
    pub(super) fn population_fixture(
        two_effects: bool,
    ) -> (Arc<CompiledSensitivityDocument>, PopulationFitRequest) {
        let (model, mut simulation) = fixture(true);
        let model = if two_effects {
            model
        } else {
            simulation.with_respect_to = vec!["cl".into()];
            simulation.derivative_absolute_tolerances.remove("v");
            let mut source: serde_json::Value = serde_json::from_str(include_str!(
                "../../../../conformance/models/synthetic-one-compartment.json"
            ))
            .unwrap();
            source["outputs"].as_array_mut().unwrap().push(json!({"name":"undefined_readout","unit":"1","expression":{"kind":"binary","operator":"divide","arguments":[{"kind":"literal","value":0},{"kind":"literal","value":0}]}}));
            CompiledSensitivityDocument::compile(
                &serde_json::from_value(source).unwrap(),
                &simulation.with_respect_to,
            )
            .unwrap()
        };
        let subject = objective(simulation);
        let mut value = json!({
            "subjects":[subject.clone(), subject],
            "fixed_effects":[{"parameter":"cl","lower":{"value":0.1,"unit":"L/h"},"upper":{"value":1,"unit":"L/h"}}],
            "random_effects":[{"parameter":"cl"}],"omega":[[0.1]],"omega_diagonal_bounds":[[0.1,0.1]],
            "eta_bound":1,"max_evaluations":40,"max_iterations":8,"gradient_tolerance":0.01,"seed":17
        });
        if two_effects {
            value["fixed_effects"].as_array_mut().unwrap().push(json!({"parameter":"v","lower":{"value":1,"unit":"L"},"upper":{"value":5,"unit":"L"}}));
            value["random_effects"]
                .as_array_mut()
                .unwrap()
                .push(json!({"parameter":"v"}));
            value["omega"] = json!([[0.1, 0], [0, 0.1]]);
            value["omega_diagonal_bounds"] = json!([[0.1, 0.1], [0.1, 0.1]]);
        }
        (model, serde_json::from_value(value).unwrap())
    }

    #[test]
    fn laplace_preparation_checks_all_subjects_before_readouts() {
        let (model, mut request) = population_fixture(false);
        assert!(model.prepare_population_laplace(&request).is_ok());
        request.subjects[1].observations[0].row = 999;
        let error = model.prepare_population_laplace(&request).err().unwrap();
        assert_eq!(
            error.expression.as_deref(),
            Some("subjects[1].observations[0]")
        );
        assert_eq!(error, model.fit_population_laplace(&request).unwrap_err());
    }
}

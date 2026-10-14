//! Browser bindings. Synthetic exports are explicitly gated for conformance checks.
use qspquest_core::Sample;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Trajectory {
    times: Vec<f64>,
    sides: Vec<u8>,
    columns: Vec<Vec<f64>>,
    metadata: Option<String>,
}
impl Trajectory {
    pub fn from_samples(samples: Vec<Sample>) -> Self {
        let count = samples.first().map_or(0, |row| row.values.len());
        Self {
            metadata: None,
            times: samples.iter().map(|row| row.time).collect(),
            sides: samples
                .iter()
                .map(|row| u8::from(row.side == "post"))
                .collect(),
            columns: (0..count)
                .map(|index| samples.iter().map(|row| row.values[index]).collect())
                .collect(),
        }
    }
}
#[wasm_bindgen]
impl Trajectory {
    pub fn metadata_json(&self) -> Option<String> {
        self.metadata.clone()
    }
    pub fn times(&self) -> Vec<f64> {
        self.times.clone()
    }
    /// 0 = before boundary events; 1 = after boundary events.
    pub fn sides(&self) -> Vec<u8> {
        self.sides.clone()
    }
    pub fn state_count(&self) -> usize {
        self.columns.len()
    }
    pub fn column(&self, index: usize) -> Result<Vec<f64>, JsValue> {
        self.columns
            .get(index)
            .cloned()
            .ok_or_else(|| JsValue::from_str("invalid state index"))
    }
}

#[cfg(feature = "m0-conformance")]
#[path = "../../../conformance/tmdd.rs"]
mod tmdd;

/// Feasibility harness only; the public authoring API will compile model IR.
#[cfg(feature = "m0-conformance")]
#[wasm_bindgen]
pub fn run_tmdd_case(case_json: &str, callback_budget: u32) -> Result<Trajectory, JsValue> {
    let case: tmdd::Case =
        serde_json::from_str(case_json).map_err(|error| JsValue::from_str(&error.to_string()))?;
    let rows = qspquest_modeling::simulate_with_budgets(
        &case.model,
        &case.protocol,
        qspquest_core::Budgets {
            solver_callbacks: callback_budget as u64,
            ..Default::default()
        },
    )
    .map_err(|error| {
        JsValue::from_str(&serde_json::to_string(&error).unwrap_or_else(|_| error.to_string()))
    })?;
    Ok(Trajectory::from_samples(rows))
}

/// No-dose oscillatory forcing keeps the worker busy without synthetic spin loops.
/// The domain variant fails after integration has started, preserving its diagnostic.
#[cfg(feature = "m0-conformance")]
#[wasm_bindgen]
pub fn run_failure_probe(domain_error: bool, callback_budget: u32) -> Result<Trajectory, JsValue> {
    use qspquest_modeling::{Budgets, Error, ErrorCode, Model, Protocol};
    struct Probe(bool);
    impl Model for Probe {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn initial(&self) -> Vec<f64> {
            vec![1.0]
        }
        fn dose_scale(&self, _: usize) -> Option<f64> {
            None
        }
        fn rhs(&self, t: f64, x: &[f64], _: &[f64], out: &mut [f64]) -> Result<(), Error> {
            if self.0 && t > 0.1 {
                let mut e = Error::new(ErrorCode::Domain, "probe log argument must be positive");
                e.expression = Some("dynamics.probe.log".into());
                return Err(e);
            }
            out[0] = -0.1 * x[0] + (100000.0 * t).sin();
            Ok(())
        }
        fn jac_mul(
            &self,
            _: f64,
            _: &[f64],
            _rates: &[f64],
            v: &[f64],
            out: &mut [f64],
        ) -> Result<(), Error> {
            out[0] = -0.1 * v[0];
            Ok(())
        }
    }
    let protocol = Protocol {
        start: 0.0,
        end: 1e9,
        samples: vec![],
        events: vec![],
        rtol: 1e-10,
        atol: 1e-12,
    };
    qspquest_modeling::simulate_with_budgets(
        &Probe(domain_error),
        &protocol,
        Budgets {
            solver_callbacks: callback_budget as u64,
            ..Default::default()
        },
    )
    .map(Trajectory::from_samples)
    .map_err(|e| JsValue::from_str(&serde_json::to_string(&e).unwrap_or_else(|_| e.to_string())))
}

/// Expression harness: runtime values are supplied on every evaluation.
#[cfg(feature = "m0-conformance")]
#[wasm_bindgen]
pub struct CompiledExpression {
    program: qspquest_modeling::compile::Program,
    scratch: Vec<f64>,
}
#[cfg(feature = "m0-conformance")]
#[wasm_bindgen]
impl CompiledExpression {
    #[wasm_bindgen(constructor)]
    pub fn new(
        expression_json: &str,
        names_json: &str,
        derivative_of: Option<String>,
    ) -> Result<CompiledExpression, JsValue> {
        use qspquest_modeling::compile::{Program, Symbol, SymbolKind};
        let expression: qspquest_core::expression::Expr =
            serde_json::from_str(expression_json).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let names: Vec<String> =
            serde_json::from_str(names_json).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let symbols: Vec<_> = names
            .into_iter()
            .map(|name| Symbol {
                name,
                kind: SymbolKind::Parameter,
            })
            .collect();
        let program = match derivative_of {
            Some(name) => Program::derivative(&expression, &symbols, &name),
            None => Program::compile(&expression, &symbols),
        }
        .map_err(|e| {
            JsValue::from_str(&serde_json::to_string(&e).unwrap_or_else(|_| e.to_string()))
        })?;
        let scratch = program.workspace();
        Ok(Self { program, scratch })
    }
    /// Conformance factory. Symbols are [name, UCUM-subset code] pairs.
    pub fn with_units(
        expression_json: &str,
        symbols_json: &str,
        output_unit: &str,
    ) -> Result<CompiledExpression, JsValue> {
        use qspquest_core::{expression::Expr, units::Unit, Error};
        use qspquest_modeling::compile::{Program, Symbol, SymbolKind};
        let expression: Expr =
            serde_json::from_str(expression_json).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let declarations: Vec<(String, String)> =
            serde_json::from_str(symbols_json).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let compile = || -> Result<Program, Error> {
            let symbols = declarations
                .into_iter()
                .map(|(name, code)| {
                    Ok((
                        Symbol {
                            name,
                            kind: SymbolKind::Parameter,
                        },
                        Unit::parse(&code)?,
                    ))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            Program::compile_with_units(&expression, &symbols, Unit::parse(output_unit)?)
        };
        let program = compile().map_err(|e| {
            JsValue::from_str(&serde_json::to_string(&e).unwrap_or_else(|_| e.to_string()))
        })?;
        let scratch = program.workspace();
        Ok(Self { program, scratch })
    }
    pub fn evaluate(&mut self, values: &[f64]) -> Result<f64, JsValue> {
        self.program
            .evaluate_into(values, &mut self.scratch)
            .map_err(|e| {
                JsValue::from_str(&serde_json::to_string(&e).unwrap_or_else(|_| e.to_string()))
            })
    }
}

#[cfg(feature = "m0-conformance")]
#[path = "../../../conformance/tmdd_vm.rs"]
mod tmdd_vm;
#[cfg(feature = "m0-conformance")]
#[wasm_bindgen]
pub struct CompiledTmdd {
    model: std::sync::Arc<qspquest_modeling::runtime::CompiledModel>,
}
#[cfg(feature = "m0-conformance")]
#[wasm_bindgen]
impl CompiledTmdd {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<CompiledTmdd, JsValue> {
        qspquest_modeling::runtime::CompiledModel::compile(&tmdd_vm::definition())
            .map(|model| Self { model })
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }
    pub fn simulate(&self, case_json: &str, callback_budget: u32) -> Result<Trajectory, JsValue> {
        let case: tmdd::Case =
            serde_json::from_str(case_json).map_err(|e| JsValue::from_str(&e.to_string()))?;
        case.model
            .validate()
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let bound = self
            .model
            .bind(&tmdd_vm::values(&case.model))
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        qspquest_modeling::simulate_with_budgets(
            &bound,
            &case.protocol,
            qspquest_modeling::Budgets {
                solver_callbacks: callback_budget as u64,
                ..Default::default()
            },
        )
        .map(Trajectory::from_samples)
        .map_err(|e| {
            JsValue::from_str(&serde_json::to_string(&e).unwrap_or_else(|_| e.to_string()))
        })
    }
}

#[cfg(feature = "m0-conformance")]
#[wasm_bindgen]
pub fn run_vm_domain_probe(callback_budget: u32) -> Result<Trajectory, JsValue> {
    use qspquest_core::expression::{Binary, Expr, Function};
    use qspquest_modeling::runtime::{CompiledModel, ModelDefinition, StateDefinition};
    let definition = ModelDefinition {
        bindings: vec![],
        states: vec![StateDefinition {
            name: "x".into(),
            initial: Expr::number(1.0),
            dose_scale: None,
            rhs: Expr::call(
                Function::Log,
                vec![Expr::binary(
                    Binary::Subtract,
                    Expr::number(1.0),
                    Expr::symbol("time"),
                )],
            ),
        }],
    };
    let model = CompiledModel::compile(&definition)
        .and_then(|model| model.bind(&[]))
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let protocol = qspquest_modeling::Protocol {
        start: 0.0,
        end: 2.0,
        samples: vec![0.1],
        events: vec![],
        rtol: 1e-8,
        atol: 1e-10,
    };
    qspquest_modeling::simulate_with_budgets(
        &model,
        &protocol,
        qspquest_modeling::Budgets {
            solver_callbacks: callback_budget as u64,
            ..Default::default()
        },
    )
    .map(Trajectory::from_samples)
    .map_err(|e| JsValue::from_str(&serde_json::to_string(&e).unwrap_or_else(|_| e.to_string())))
}

#[cfg(feature = "m0-conformance")]
enum KernelModel {
    Vm(qspquest_modeling::runtime::BoundProblem),
    Hand(tmdd::Tmdd),
}
#[cfg(feature = "m0-conformance")]
#[wasm_bindgen]
pub struct KernelProbe {
    model: KernelModel,
}
#[cfg(feature = "m0-conformance")]
#[wasm_bindgen]
impl KernelProbe {
    #[wasm_bindgen(constructor)]
    pub fn new(case_json: &str, vm: bool) -> Result<KernelProbe, JsValue> {
        let case: tmdd::Case =
            serde_json::from_str(case_json).map_err(|e| JsValue::from_str(&e.to_string()))?;
        case.model
            .validate()
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        let model = if vm {
            let model = qspquest_modeling::runtime::CompiledModel::compile(&tmdd_vm::definition())
                .and_then(|model| model.bind(&tmdd_vm::values(&case.model)))
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            KernelModel::Vm(model)
        } else {
            KernelModel::Hand(case.model)
        };
        Ok(Self { model })
    }
    pub fn run(&self, iterations: u32, jacobian: bool) -> Result<f64, JsValue> {
        fn execute<M: qspquest_modeling::Model>(
            model: &M,
            iterations: u32,
            jacobian: bool,
        ) -> Result<f64, qspquest_modeling::Error> {
            let state = [1.2, 0.7, 0.3, 0.9, 2.0, 1.0, 3.0];
            let rates = [0.1; 7];
            let direction = [1.0; 7];
            let mut out = [0.0; 7];
            let mut checksum = 0.0;
            for _ in 0..iterations {
                if jacobian {
                    model.jac_mul(
                        1.0,
                        std::hint::black_box(&state),
                        &rates,
                        &direction,
                        &mut out,
                    )?;
                } else {
                    model.rhs(1.0, std::hint::black_box(&state), &rates, &mut out)?;
                }
                checksum += std::hint::black_box(out)[0];
            }
            Ok(checksum)
        }
        if iterations == 0 || iterations > 100000 {
            return Err(JsValue::from_str("kernel iterations must be 1..100000"));
        }
        match &self.model {
            KernelModel::Vm(model) => execute(model, iterations, jacobian),
            KernelModel::Hand(model) => execute(model, iterations, jacobian),
        }
        .map_err(|e| JsValue::from_str(&e.to_string()))
    }
}

/// Versioned simulation-document API used by the browser engine package.
#[wasm_bindgen]
pub struct CompiledSimulation {
    model: std::sync::Arc<qspquest_modeling::document::CompiledDocument>,
}
fn diagnostic(error: qspquest_core::Error) -> JsValue {
    JsValue::from_str(&serde_json::to_string(&error).unwrap_or_else(|_| error.to_string()))
}
#[wasm_bindgen]
impl CompiledSimulation {
    #[wasm_bindgen(constructor)]
    pub fn new(document_json: &str) -> Result<CompiledSimulation, JsValue> {
        Ok(Self {
            model: qspquest_modeling::document::CompiledDocument::from_json(document_json)
                .map_err(diagnostic)?,
        })
    }
    pub fn from_text(source: &str) -> Result<CompiledSimulation, JsValue> {
        let language_error = |e: qspquest_modeling::language::Diagnostic| {
            JsValue::from_str(&serde_json::json!({"code":e.code,"message":e.message,"line":e.line,"column":e.column,"hint":e.hint}).to_string())
        };
        let parsed = qspquest_modeling::language::parse(source).map_err(language_error)?;
        Ok(Self {
            model: parsed.compile().map_err(language_error)?,
        })
    }
    pub fn format_text(source: &str) -> Result<String, JsValue> {
        let language_error = |e: qspquest_modeling::language::Diagnostic| {
            JsValue::from_str(&serde_json::json!({"code":e.code,"message":e.message,"line":e.line,"column":e.column,"hint":e.hint}).to_string())
        };
        let parsed = qspquest_modeling::language::parse(source).map_err(language_error)?;
        qspquest_modeling::language::format(&parsed.document).map_err(language_error)
    }
    pub fn simulate_regimen(
        &self,
        overrides_json: &str,
        request_json: &str,
        callback_budget: u32,
    ) -> Result<Trajectory, JsValue> {
        if overrides_json.len() > 1_000_000 || request_json.len() > 1_000_000 {
            return Err(diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                "run document exceeds input byte budget",
            )));
        }
        let parse_error = |e: serde_json::Error| {
            diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                e.to_string(),
            ))
        };
        let overrides: std::collections::BTreeMap<String, qspquest_core::model::Quantity> =
            serde_json::from_str(overrides_json).map_err(parse_error)?;
        let request: qspquest_core::regimen::Request =
            serde_json::from_str(request_json).map_err(parse_error)?;
        let budgets = qspquest_modeling::Budgets {
            solver_callbacks: callback_budget as u64,
            ..Default::default()
        };
        self.model
            .bind_with_covariates(&overrides, &request.covariates)
            .map_err(diagnostic)?
            .simulate_regimen(&request, budgets)
            .map(Trajectory::from_samples)
            .map_err(diagnostic)
    }
    pub fn periodic_steady_state(&self, request_json: &str) -> Result<String, JsValue> {
        if request_json.len() > 1_000_000 {
            return Err(diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                "steady-state request exceeds input byte budget",
            )));
        }
        let request: qspquest_core::run::SteadyStateRequest = serde_json::from_str(request_json)
            .map_err(|e| {
                diagnostic(qspquest_core::Error::new(
                    qspquest_core::ErrorCode::InvalidInput,
                    e.to_string(),
                ))
            })?;
        let result = self
            .model
            .periodic_steady_state(&request)
            .map_err(diagnostic)?;
        serde_json::to_string(&result).map_err(|e| {
            diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::Domain,
                e.to_string(),
            ))
        })
    }
    pub fn morris(&self, request_json: &str) -> Result<String, JsValue> {
        let invalid = |message: String| {
            diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                message,
            ))
        };
        if request_json.len() > 1_000_000 {
            return Err(invalid("Morris request exceeds input byte budget".into()));
        }
        let request = serde_json::from_str(request_json).map_err(|e| invalid(e.to_string()))?;
        let result = self.model.execute_morris(&request).map_err(diagnostic)?;
        serde_json::to_string(&result).map_err(|e| invalid(e.to_string()))
    }
    pub fn scan(&self, request_json: &str) -> Result<String, JsValue> {
        let invalid = |message: String| {
            diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                message,
            ))
        };
        if request_json.len() > 1_000_000 {
            return Err(invalid("scan request exceeds input byte budget".into()));
        }
        let request = serde_json::from_str(request_json).map_err(|e| invalid(e.to_string()))?;
        let result = self.model.scan(&request).map_err(diagnostic)?;
        serde_json::to_string(&result).map_err(|e| invalid(e.to_string()))
    }
    pub fn iterate_periodic_state(&self, request_json: &str) -> Result<String, JsValue> {
        if request_json.len() > 1_000_000 {
            return Err(diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                "cycle-iteration request exceeds input byte budget",
            )));
        }
        let request: qspquest_core::run::CycleIterationRequest = serde_json::from_str(request_json)
            .map_err(|e| {
                diagnostic(qspquest_core::Error::new(
                    qspquest_core::ErrorCode::InvalidInput,
                    e.to_string(),
                ))
            })?;
        let result = self
            .model
            .iterate_periodic_state(&request)
            .map_err(diagnostic)?;
        serde_json::to_string(&result).map_err(|e| {
            diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::Domain,
                e.to_string(),
            ))
        })
    }
    pub fn execute(&self, request_json: &str) -> Result<Trajectory, JsValue> {
        self.execute_selected(request_json, "null")
    }
    pub fn execute_selected(
        &self,
        request_json: &str,
        outputs_json: &str,
    ) -> Result<Trajectory, JsValue> {
        if request_json.len() > 1_000_000 || outputs_json.len() > 1_000_000 {
            return Err(diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                "run request exceeds input byte budget",
            )));
        }
        let request: qspquest_core::run::RunRequest =
            serde_json::from_str(request_json).map_err(|e| {
                diagnostic(qspquest_core::Error::new(
                    qspquest_core::ErrorCode::InvalidInput,
                    e.to_string(),
                ))
            })?;
        let selected: Option<Vec<String>> = serde_json::from_str(outputs_json).map_err(|e| {
            diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                e.to_string(),
            ))
        })?;
        let result = self
            .model
            .execute_selected(&request, selected.as_deref())
            .map_err(diagnostic)?;
        let metadata=serde_json::json!({"schema":result.schema,"request_id":result.request_id,"identity":result.identity,"model_id":result.model_id,"description":result.description,"time_unit":result.time_unit,"names":result.outputs.iter().map(|o|o.name.as_str()).collect::<Vec<_>>(),"units":result.outputs.iter().map(|o|o.unit.as_str()).collect::<Vec<_>>()}).to_string();
        Ok(Trajectory {
            times: result.times,
            sides: result
                .sides
                .iter()
                .map(|s| u8::from(*s == qspquest_core::regimen::ObservationSide::Post))
                .collect(),
            columns: result.outputs.into_iter().map(|o| o.values).collect(),
            metadata: Some(metadata),
        })
    }
    pub fn output_names(&self) -> Vec<String> {
        self.model.output_names().to_vec()
    }
    pub fn output_units(&self) -> Vec<String> {
        self.model.output_units().to_vec()
    }
    pub fn simulate(
        &self,
        overrides_json: &str,
        protocol_json: &str,
        callback_budget: u32,
    ) -> Result<Trajectory, JsValue> {
        if overrides_json.len() > 1_000_000 || protocol_json.len() > 1_000_000 {
            return Err(diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                "run document exceeds input byte budget",
            )));
        }
        let parse_error = |e: serde_json::Error| {
            diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                e.to_string(),
            ))
        };
        let overrides: std::collections::BTreeMap<String, qspquest_core::model::Quantity> =
            serde_json::from_str(overrides_json).map_err(parse_error)?;
        let protocol: qspquest_modeling::Protocol =
            serde_json::from_str(protocol_json).map_err(parse_error)?;
        let bound = self.model.bind(&overrides).map_err(diagnostic)?;
        bound
            .simulate_outputs(
                &protocol,
                qspquest_modeling::Budgets {
                    solver_callbacks: callback_budget as u64,
                    ..Default::default()
                },
            )
            .map(Trajectory::from_samples)
            .map_err(diagnostic)
    }
}

/// Compile once for an ordered selection of independent model parameters.
#[cfg(feature = "m0-conformance")]
#[wasm_bindgen]
pub struct CompiledSensitivities {
    model: std::sync::Arc<qspquest_modeling::document::sensitivity::CompiledSensitivityDocument>,
}
#[cfg(feature = "m0-conformance")]
#[wasm_bindgen]
impl CompiledSensitivities {
    #[wasm_bindgen(constructor)]
    pub fn new(source: &str, parameters_json: &str) -> Result<CompiledSensitivities, JsValue> {
        let invalid = |message: String| {
            diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                message,
            ))
        };
        if source.len() > 1_000_000 || parameters_json.len() > 1_000_000 {
            return Err(invalid(
                "sensitivity source exceeds input byte budget".into(),
            ));
        }
        let parameters: Vec<String> =
            serde_json::from_str(parameters_json).map_err(|e| invalid(e.to_string()))?;
        let document = if source.trim_start().starts_with('{') {
            serde_json::from_str(source).map_err(|e| invalid(e.to_string()))?
        } else {
            qspquest_modeling::language::parse(source).map_err(|e|JsValue::from_str(&serde_json::json!({"code":e.code,"message":e.message,"line":e.line,"column":e.column,"hint":e.hint}).to_string()))?.document
        };
        Ok(Self {
            model: qspquest_modeling::document::sensitivity::CompiledSensitivityDocument::compile(
                &document,
                &parameters,
            )
            .map_err(diagnostic)?,
        })
    }
    pub fn fit(&self, request_json: &str) -> Result<String, JsValue> {
        let invalid = |message: String| {
            diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                message,
            ))
        };
        if request_json.len() > 1_000_000 {
            return Err(invalid("fit request exceeds input byte budget".into()));
        }
        let request = serde_json::from_str(request_json).map_err(|e| invalid(e.to_string()))?;
        let result = self.model.execute_fit(&request).map_err(diagnostic)?;
        serde_json::to_string(&result).map_err(|e| invalid(e.to_string()))
    }
    pub fn execute(&self, request_json: &str) -> Result<String, JsValue> {
        let invalid = |message: String| {
            diagnostic(qspquest_core::Error::new(
                qspquest_core::ErrorCode::InvalidInput,
                message,
            ))
        };
        if request_json.len() > 1_000_000 {
            return Err(invalid(
                "sensitivity request exceeds input byte budget".into(),
            ));
        }
        let request = serde_json::from_str(request_json).map_err(|e| invalid(e.to_string()))?;
        let result = self.model.run(&request).map_err(diagnostic)?;
        serde_json::to_string(&result).map_err(|e| invalid(e.to_string()))
    }
}

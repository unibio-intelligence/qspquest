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
#[wasm_bindgen]
pub struct CompiledSensitivities {
    model: std::sync::Arc<qspquest_modeling::document::sensitivity::CompiledSensitivityDocument>,
}
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

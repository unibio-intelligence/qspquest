//! Python ownership and error boundary; no scientific implementation lives here.
use pyo3::{exceptions::PyException, prelude::*};
use qspquest_modeling::document::CompiledDocument;
use serde_json::{json, Value};
use std::sync::Arc;
pyo3::create_exception!(_native, NativeError, PyException);

fn scientific(error: qspquest_core::Error) -> Value {
    json!(error)
}
fn diagnostic(error: qspquest_modeling::language::Diagnostic) -> Value {
    json!({"code":error.code,"message":error.message,"line":error.line,"column":error.column,"hint":error.hint})
}
fn invalid(message: impl ToString) -> Value {
    json!({"code":"invalid_input","message":message.to_string()})
}
fn detached<T: Send>(py: Python<'_>, f: impl FnOnce() -> Result<T, Value> + Send) -> PyResult<T> {
    let result = py.detach(|| std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(NativeError::new_err(error.to_string())),
        Err(_) => Err(NativeError::new_err(
            json!({"code":"internal","message":"Rust execution panicked; no result returned"})
                .to_string(),
        )),
    }
}
#[pyclass(frozen, name = "CompiledModel", module = "qspquest._native")]
struct PythonModel {
    inner: Arc<CompiledDocument>,
}
#[pymethods]
impl PythonModel {
    #[new]
    fn new(py: Python<'_>, source: String) -> PyResult<Self> {
        detached(py, move || {
            let inner = if source.trim_start().starts_with('{') {
                CompiledDocument::from_json(&source).map_err(scientific)?
            } else {
                qspquest_modeling::language::parse(&source)
                    .map_err(diagnostic)?
                    .compile()
                    .map_err(diagnostic)?
            };
            Ok(Self { inner })
        })
    }
    fn morris_json(&self, py: Python<'_>, request: String) -> PyResult<String> {
        let inner = self.inner.clone();
        detached(py, move || {
            if request.len() > 1_000_000 {
                return Err(invalid("request exceeds one million bytes"));
            }
            let request = serde_json::from_str(&request).map_err(invalid)?;
            let result = inner.execute_morris(&request).map_err(scientific)?;
            serde_json::to_string(&result).map_err(invalid)
        })
    }
    fn scan_json(&self, py: Python<'_>, request: String) -> PyResult<String> {
        let inner = self.inner.clone();
        detached(py, move || {
            if request.len() > 1_000_000 {
                return Err(invalid("request exceeds one million bytes"));
            }
            let request = serde_json::from_str(&request).map_err(invalid)?;
            let result = inner.scan(&request).map_err(scientific)?;
            serde_json::to_string(&result).map_err(invalid)
        })
    }
    /// Admission only for the fixed-regimen interchange profile; no integration.
    fn validate_run_json(&self, py: Python<'_>, request: String) -> PyResult<String> {
        let inner = self.inner.clone();
        detached(py, move || {
            if request.len() > 1_000_000 {
                return Err(invalid("request exceeds one million bytes"));
            }
            let request: qspquest_core::run::RunRequest =
                serde_json::from_str(&request).map_err(invalid)?;
            if request.seed.is_some()
                || !request.initial_states.is_empty()
                || !matches!(request.solver, qspquest_core::run::Solver::DiffsolBdf)
            {
                return Err(invalid(
                    "interchange admission requires diffsol_bdf, no seed or initial overrides",
                ));
            }
            if request.budgets.solver_callbacks > u32::MAX as u64
                || request.budgets.events > u32::MAX as usize
                || request.budgets.output_values > u32::MAX as usize
            {
                return Err(invalid("budgets must fit the shared unsigned 32-bit range"));
            }
            inner.bind(&request.parameters).map_err(scientific)?;
            inner
                .regimen_protocol(&request.regimen, request.budgets)
                .map_err(scientific)?;
            Ok("{\"status\":\"valid\"}".to_string())
        })
    }
    fn execute_json(&self, py: Python<'_>, request: String) -> PyResult<String> {
        let inner = self.inner.clone();
        detached(py, move || {
            if request.len() > 1_000_000 {
                return Err(invalid("request exceeds one million bytes"));
            }
            let request = serde_json::from_str(&request).map_err(invalid)?;
            let result = inner.execute(&request).map_err(scientific)?;
            serde_json::to_string(&result).map_err(invalid)
        })
    }
    fn fit_scalar_json(&self, py: Python<'_>, request: String) -> PyResult<String> {
        let inner = self.inner.clone();
        detached(py, move || {
            if request.len() > 1_000_000 {
                return Err(invalid("request exceeds one million bytes"));
            }
            let request = serde_json::from_str(&request).map_err(invalid)?;
            let result = inner.fit_scalar(&request).map_err(scientific)?;
            serde_json::to_string(&result).map_err(invalid)
        })
    }
    fn periodic_steady_state_json(&self, py: Python<'_>, request: String) -> PyResult<String> {
        let inner = self.inner.clone();
        detached(py, move || {
            if request.len() > 1_000_000 {
                return Err(invalid("request exceeds one million bytes"));
            }
            let request = serde_json::from_str(&request).map_err(invalid)?;
            let result = inner.periodic_steady_state(&request).map_err(scientific)?;
            serde_json::to_string(&result).map_err(invalid)
        })
    }
    fn iterate_periodic_state_json(&self, py: Python<'_>, request: String) -> PyResult<String> {
        let inner = self.inner.clone();
        detached(py, move || {
            if request.len() > 1_000_000 {
                return Err(invalid("request exceeds one million bytes"));
            }
            let request = serde_json::from_str(&request).map_err(invalid)?;
            let result = inner.iterate_periodic_state(&request).map_err(scientific)?;
            serde_json::to_string(&result).map_err(invalid)
        })
    }
    #[getter]
    fn model_content_hash(&self) -> String {
        self.inner.model_content_hash().into()
    }
    #[getter]
    fn output_names(&self) -> Vec<String> {
        self.inner.output_names().to_vec()
    }
    #[getter]
    fn output_units(&self) -> Vec<String> {
        self.inner.output_units().to_vec()
    }
}
#[pyclass(frozen, name = "CompiledSensitivities", module = "qspquest._native")]
struct PythonSensitivities {
    inner: Arc<qspquest_modeling::document::sensitivity::CompiledSensitivityDocument>,
}
#[pymethods]
impl PythonSensitivities {
    #[new]
    fn new(py: Python<'_>, source: String, parameters: Vec<String>) -> PyResult<Self> {
        detached(py, move || {
            if source.len() > qspquest_modeling::document::MODEL_DOCUMENT_BYTE_LIMIT {
                return Err(invalid("model exceeds four million bytes"));
            }
            let document = if source.trim_start().starts_with('{') {
                serde_json::from_str(&source).map_err(invalid)?
            } else {
                qspquest_modeling::language::parse(&source)
                    .map_err(diagnostic)?
                    .document
            };
            let inner =
                qspquest_modeling::document::sensitivity::CompiledSensitivityDocument::compile(
                    &document,
                    &parameters,
                )
                .map_err(scientific)?;
            Ok(Self { inner })
        })
    }
    /// Bind observations, residual errors, bounds and work limits; no integration.
    fn plan_fit_json(&self, py: Python<'_>, request: String) -> PyResult<String> {
        let inner = self.inner.clone();
        detached(py, move || {
            if request.len() > 1_000_000 {
                return Err(invalid("request exceeds one million bytes"));
            }
            let request = serde_json::from_str(&request).map_err(invalid)?;
            let result = inner.plan_fit(&request).map_err(scientific)?;
            serde_json::to_string(&result).map_err(invalid)
        })
    }
    fn objective_json(&self, py: Python<'_>, request: String) -> PyResult<String> {
        let inner = self.inner.clone();
        detached(py, move || {
            if request.len() > 1_000_000 {
                return Err(invalid("request exceeds one million bytes"));
            }
            let request = serde_json::from_str(&request).map_err(invalid)?;
            let result = inner.gaussian_objective(&request).map_err(scientific)?;
            serde_json::to_string(&result).map_err(invalid)
        })
    }
    fn fit_json(&self, py: Python<'_>, request: String) -> PyResult<String> {
        let inner = self.inner.clone();
        detached(py, move || {
            if request.len() > 1_000_000 {
                return Err(invalid("request exceeds one million bytes"));
            }
            let request = serde_json::from_str(&request).map_err(invalid)?;
            let result = inner.execute_fit(&request).map_err(scientific)?;
            serde_json::to_string(&result).map_err(invalid)
        })
    }
    fn execute_json(&self, py: Python<'_>, request: String) -> PyResult<String> {
        let inner = self.inner.clone();
        detached(py, move || {
            if request.len() > 1_000_000 {
                return Err(invalid("request exceeds one million bytes"));
            }
            let request = serde_json::from_str(&request).map_err(invalid)?;
            let result = inner.run(&request).map_err(scientific)?;
            serde_json::to_string(&result).map_err(invalid)
        })
    }
}
#[pyfunction]
fn format_model(py: Python<'_>, source: String) -> PyResult<String> {
    detached(py, move || {
        if source.len() > qspquest_modeling::document::MODEL_DOCUMENT_BYTE_LIMIT {
            return Err(invalid("model exceeds four million bytes"));
        }
        let document = if source.trim_start().starts_with('{') {
            serde_json::from_str(&source).map_err(invalid)?
        } else {
            qspquest_modeling::language::parse(&source)
                .map_err(diagnostic)?
                .document
        };
        qspquest_modeling::language::format(&document).map_err(diagnostic)
    })
}
#[pyfunction]
fn fit_gamma_poisson_json(py: Python<'_>, request: String) -> PyResult<String> {
    detached(py, move || {
        if request.len() > 1_000_000 {
            return Err(invalid("request exceeds one million bytes"));
        }
        let request: qspquest_core::discrete_fit::GammaPoissonFitRequest =
            serde_json::from_str(&request).map_err(invalid)?;
        let result =
            qspquest_modeling::discrete_fit::fit_gamma_poisson(&request).map_err(scientific)?;
        serde_json::to_string(&result).map_err(invalid)
    })
}
#[pyfunction]
fn fit_probit_normal_binomial_json(py: Python<'_>, request: String) -> PyResult<String> {
    detached(py, move || {
        if request.len() > 1_000_000 {
            return Err(invalid("request exceeds one million bytes"));
        }
        let request: qspquest_core::probit_fit::ProbitNormalBinomialFitRequest =
            serde_json::from_str(&request).map_err(invalid)?;
        let result = qspquest_modeling::probit_normal::fit_probit_normal_binomial(&request)
            .map_err(scientific)?;
        serde_json::to_string(&result).map_err(invalid)
    })
}
/// Native serde normalization and JCS identity; no compilation or numerical execution.
#[pyfunction]
#[pyo3(signature = (source, request, operation, result=None))]
fn fitting_identity(
    py: Python<'_>,
    source: String,
    request: String,
    operation: String,
    result: Option<String>,
) -> PyResult<String> {
    detached(py, move || {
        use qspquest_core::{
            fit::{FitProblem, FitRequest, ScalarGaussianFitRequest},
            model::ModelDocument,
        };
        if source.len() > qspquest_modeling::document::MODEL_DOCUMENT_BYTE_LIMIT
            || request.len() > 1_000_000
        {
            return Err(invalid(
                "fitting identity exceeds model/request byte limits",
            ));
        }
        let model: ModelDocument = serde_json::from_str(&source).map_err(invalid)?;
        let model = serde_json::to_value(model).map_err(invalid)?;
        let request = match operation.as_str() {
            "fit_scalar" => serde_json::to_value(
                serde_json::from_str::<ScalarGaussianFitRequest>(&request).map_err(invalid)?,
            )
            .map_err(invalid)?,
            "fit_individual" => {
                let value: FitRequest = serde_json::from_str(&request).map_err(invalid)?;
                if !matches!(value.problem, FitProblem::Individual(_)) {
                    return Err(invalid(
                        "fit_individual identity requires problem.kind individual",
                    ));
                }
                serde_json::to_value(value).map_err(invalid)?
            }
            _ => {
                return Err(invalid(
                    "fitting identity supports fit_scalar and fit_individual",
                ));
            }
        };
        let model_hash = qspquest_core::identity::canonical_hash(&model).map_err(scientific)?;
        let request_hash = qspquest_core::identity::canonical_hash(&request).map_err(scientific)?;
        let mut identity = json!({"model_content_hash":model_hash,"native_request_sha256":request_hash.trim_start_matches("sha256:"),"native_request":request});
        if let Some(result) = result {
            if result.len() > 1_000_000 {
                return Err(invalid("fitting result exceeds one million bytes"));
            }
            identity["native_result"] = match operation.as_str() {
                "fit_scalar" => serde_json::to_value(
                    serde_json::from_str::<qspquest_core::fit::ScalarGaussianFitResult>(&result)
                        .map_err(invalid)?,
                )
                .map_err(invalid)?,
                _ => {
                    let result: qspquest_core::fit::FitResult =
                        serde_json::from_str(&result).map_err(invalid)?;
                    if result.population.is_some()
                        || result.fit.is_none()
                        || result.residual_scales.is_some()
                    {
                        return Err(invalid(
                            "individual evidence requires an individual fit result without profiled residual scales",
                        ));
                    }
                    serde_json::to_value(result).map_err(invalid)?
                }
            };
        }
        serde_json::to_string(&identity).map_err(invalid)
    })
}
/// Reuse the compiler's exact unit lowering for portable numeric coordinates.
#[pyfunction]
fn normalize_interchange_expression(
    py: Python<'_>,
    expression: String,
    symbol_units: String,
    output_unit: String,
) -> PyResult<String> {
    detached(py, move || {
        use qspquest_core::units::Unit;
        use qspquest_modeling::compile::{Symbol, SymbolKind};
        if expression.len() > 1_000_000 || symbol_units.len() > 1_000_000 {
            return Err(invalid("expression or symbols exceed one million bytes"));
        }
        let expression = serde_json::from_str(&expression).map_err(invalid)?;
        let units: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&symbol_units).map_err(invalid)?;
        let symbols = units
            .into_iter()
            .map(|(name, unit)| {
                Ok((
                    Symbol {
                        name,
                        kind: SymbolKind::Parameter,
                    },
                    Unit::parse(&unit)?,
                ))
            })
            .collect::<Result<Vec<_>, qspquest_core::Error>>()
            .map_err(scientific)?;
        let output = Unit::parse(&output_unit).map_err(scientific)?;
        let normalized =
            qspquest_modeling::compile::units::normalize(&expression, &symbols, output)
                .map_err(scientific)?;
        serde_json::to_string(&normalized).map_err(invalid)
    })
}
#[pyfunction]
fn convert_interchange_quantity(value: f64, from_unit: String, to_unit: String) -> PyResult<f64> {
    use qspquest_core::units::Unit;
    let result = Unit::parse(&from_unit)
        .and_then(|source| Unit::parse(&to_unit).and_then(|target| source.convert(value, target)));
    result.map_err(|error| NativeError::new_err(scientific(error).to_string()))
}
#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PythonModel>()?;
    m.add_class::<PythonSensitivities>()?;
    m.add_function(wrap_pyfunction!(format_model, m)?)?;
    m.add_function(wrap_pyfunction!(fit_gamma_poisson_json, m)?)?;
    m.add_function(wrap_pyfunction!(fit_probit_normal_binomial_json, m)?)?;
    m.add_function(wrap_pyfunction!(fitting_identity, m)?)?;
    m.add_function(wrap_pyfunction!(normalize_interchange_expression, m)?)?;
    m.add_function(wrap_pyfunction!(convert_interchange_quantity, m)?)?;
    m.add("NativeError", m.py().get_type::<NativeError>())?;
    m.add(
        "__version__",
        env!("CARGO_PKG_VERSION").replace("-rc.", "rc"),
    )?;
    Ok(())
}

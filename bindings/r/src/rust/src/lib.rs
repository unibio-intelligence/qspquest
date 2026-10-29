use extendr_api::prelude::*;
type NativeResult<T> = std::result::Result<T, String>;
use qspquest_core::model::ModelDocument;
use qspquest_modeling::document::{sensitivity::CompiledSensitivityDocument, CompiledDocument};
use std::sync::Arc;
struct RModel {
    document: ModelDocument,
    compiled: Arc<CompiledDocument>,
}
fn invalid(message: impl std::fmt::Display) -> String {
    serde_json::json!({"code":"invalid_input","message":message.to_string()}).to_string()
}
fn scientific(error: qspquest_core::Error) -> String {
    serde_json::to_string(&error).unwrap()
}
fn bounded(value: &str) -> NativeResult<()> {
    if value.len() > 1_000_000 {
        Err(invalid("input exceeds one million bytes"))
    } else {
        Ok(())
    }
}
#[extendr]
fn qspquest_compile(source: &str) -> NativeResult<ExternalPtr<RModel>> {
    if source.len() > qspquest_modeling::document::MODEL_DOCUMENT_BYTE_LIMIT {
        return Err(invalid("model exceeds four million bytes"));
    }
    let document: ModelDocument = if source.trim_start().starts_with('{') {
        serde_json::from_str(source).map_err(invalid)?
    } else {
        qspquest_modeling::language::parse(source)
            .map_err(|e| invalid(e.message))?
            .document
    };
    let compiled = CompiledDocument::compile(&document).map_err(scientific)?;
    Ok(ExternalPtr::new(RModel { document, compiled }))
}
#[extendr]
fn qspquest_execute(
    model: ExternalPtr<RModel>,
    operation: &str,
    request: &str,
) -> NativeResult<String> {
    bounded(request)?;
    let value = match operation {
        "run" => serde_json::to_value(
            model
                .compiled
                .execute(&serde_json::from_str(request).map_err(invalid)?)
                .map_err(scientific)?,
        ),
        "scan" => serde_json::to_value(
            model
                .compiled
                .scan(&serde_json::from_str(request).map_err(invalid)?)
                .map_err(scientific)?,
        ),
        "morris" => serde_json::to_value(
            model
                .compiled
                .execute_morris(&serde_json::from_str(request).map_err(invalid)?)
                .map_err(scientific)?,
        ),
        "fit" => {
            let request: qspquest_core::fit::FitRequest =
                serde_json::from_str(request).map_err(invalid)?;
            let compiled =
                CompiledSensitivityDocument::compile(&model.document, request.parameters())
                    .map_err(scientific)?;
            serde_json::to_value(compiled.execute_fit(&request).map_err(scientific)?)
        }
        _ => return Err(invalid("unknown operation")),
    }
    .map_err(invalid)?;
    serde_json::to_string(&value).map_err(invalid)
}
#[extendr]
fn qspquest_fit_gamma_poisson(request: &str) -> NativeResult<String> {
    bounded(request)?;
    let request: qspquest_core::discrete_fit::GammaPoissonFitRequest =
        serde_json::from_str(request).map_err(invalid)?;
    let result =
        qspquest_modeling::discrete_fit::fit_gamma_poisson(&request).map_err(scientific)?;
    serde_json::to_string(&result).map_err(invalid)
}
#[extendr]
fn qspquest_fit_probit_normal_binomial(request: &str) -> NativeResult<String> {
    bounded(request)?;
    let request: qspquest_core::probit_fit::ProbitNormalBinomialFitRequest =
        serde_json::from_str(request).map_err(invalid)?;
    let result = qspquest_modeling::probit_normal::fit_probit_normal_binomial(&request)
        .map_err(scientific)?;
    serde_json::to_string(&result).map_err(invalid)
}
extendr_module! {mod qspquest; fn qspquest_compile; fn qspquest_execute; fn qspquest_fit_gamma_poisson; fn qspquest_fit_probit_normal_binomial;}

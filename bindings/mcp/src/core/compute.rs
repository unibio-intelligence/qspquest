//! Target-independent execution used by the native server and the raw WASM ABI.
use qspquest_core::{
    fit::{FitRequest, FitResult, ScalarGaussianFitRequest, ScalarGaussianFitResult},
    initial_values::{InitialValuesPlan, InitialValuesRequest, InitialValuesResult},
    model::ModelDocument,
    morris::{MorrisExecutionRequest, MorrisExecutionResult},
    planning::{FitPlan, MorrisPlan, PeriodicPlan, RunPlan, ScalarFitPlan, ScanPlan},
    run::{RunRequest, RunResult, SteadyStateRequest, SteadyStateResult},
    scan::{ScanRequest, ScanResult},
};
use qspquest_modeling::{
    document::{sensitivity::CompiledSensitivityDocument, CompiledDocument},
    language,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
pub fn core_error(e: qspquest_modeling::Error) -> Value {
    let suggestion = match e.code {
        qspquest_modeling::ErrorCode::Unit | qspquest_modeling::ErrorCode::UnknownUnit => "Use a quantity with the unit declared by the model for this field; choose a compatible model when the required dimension is unavailable.",
        qspquest_modeling::ErrorCode::UnknownSymbol => "Use a declared native parameter, state, target or output name; inspect get_model for supported names.",
        qspquest_modeling::ErrorCode::Unsupported => "Inspect the operation's supported solver, estimator and observation contract; choose an explicitly supported alternative. Unsupported capability is not a scientific result.",
        qspquest_modeling::ErrorCode::WorkBudget | qspquest_modeling::ErrorCode::OutputBudget => "Reduce the requested grid, events, selected outputs or evaluation workload, or use an explicitly permitted larger budget.",
        _ => "Correct the named field and inspect the model's units, domain and operation requirements before retrying.",
    };
    json!({"code":format!("{:?}",e.code),"message":e.message,"time":e.time,"expression":e.expression,"suggestion":suggestion})
}
pub fn model(source: &str) -> Result<ModelDocument, Value> {
    if source.len() > 1_000_000 {
        return Err(json!({"code":"input_limit","message":"Model source exceeds 1 MB"}));
    }
    if source.trim_start().starts_with('{') {
        serde_json::from_str(source).map_err(|e|json!({"code":"invalid_json","message":e.to_string(),"line":e.line(),"column":e.column(),"suggestion":"Use model JSON or QSPQuest .qsp text"}))
    } else {
        language::parse(source).map(|p|p.document).map_err(|e|json!({"code":e.code,"message":e.message,"line":e.line,"column":e.column,"suggestion":e.hint}))
    }
}
pub fn execute(source: &str, operation: &str, request: Value) -> Result<Value, Value> {
    let d = model(source)?;
    if operation == "inspect_model" {
        return serde_json::to_value(d)
            .map_err(|e| json!({"code":"model","message":e.to_string()}));
    }
    execute_document(&d, operation, request)
}
// These types belong to the native boundary, not the public tool schemas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Plan,
    Execute,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FitKind {
    Individual,
    Population,
}
impl FitKind {
    fn operation(self) -> &'static str {
        match self {
            Self::Individual => "fit_individual",
            Self::Population => "fit_population",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Run(Mode),
    InitialValues(Mode),
    Scan(Mode),
    Periodic(Mode),
    Morris(Mode),
    ScalarFit(Mode),
    Fit(Mode, FitKind),
    Compile,
    Validate,
}
impl Operation {
    fn parse(name: &str) -> Result<Self, Value> {
        use Mode::{Execute, Plan};
        match name {
            "plan_simulate" => Ok(Self::Run(Plan)),
            "simulate" => Ok(Self::Run(Execute)),
            "plan_get_patient_parameters" => Ok(Self::InitialValues(Plan)),
            "get_patient_parameters" => Ok(Self::InitialValues(Execute)),
            "plan_scan_parameters" => Ok(Self::Scan(Plan)),
            "scan_parameters" => Ok(Self::Scan(Execute)),
            "plan_steady_state" => Ok(Self::Periodic(Plan)),
            "steady_state" => Ok(Self::Periodic(Execute)),
            "plan_morris_screening" => Ok(Self::Morris(Plan)),
            "morris_screening" => Ok(Self::Morris(Execute)),
            "plan_fit_scalar" => Ok(Self::ScalarFit(Plan)),
            "fit_scalar" => Ok(Self::ScalarFit(Execute)),
            "plan_fit_individual" => Ok(Self::Fit(Plan, FitKind::Individual)),
            "fit_individual" => Ok(Self::Fit(Execute, FitKind::Individual)),
            "plan_fit_population" => Ok(Self::Fit(Plan, FitKind::Population)),
            "fit_population" => Ok(Self::Fit(Execute, FitKind::Population)),
            "compile_model" => Ok(Self::Compile),
            "validate_model" => Ok(Self::Validate),
            _ => Err(json!({"code":"unsupported","message":"Unknown compute operation"})),
        }
    }
}

enum NativeRequest {
    Run(Mode, RunRequest, Option<Vec<String>>),
    InitialValues(Mode, InitialValuesRequest),
    Scan(Mode, ScanRequest),
    Periodic(Mode, SteadyStateRequest),
    Morris(Mode, MorrisExecutionRequest),
    ScalarFit(Mode, ScalarGaussianFitRequest),
    Fit(Mode, FitRequest),
    Compile,
    Validate,
}
fn invalid_request(error: serde_json::Error) -> Value {
    json!({"code":"invalid_request","message":error.to_string()})
}
fn decode<T: DeserializeOwned>(request: Value) -> Result<T, Value> {
    serde_json::from_value(request).map_err(invalid_request)
}
impl NativeRequest {
    fn parse(operation: Operation, mut request: Value) -> Result<Self, Value> {
        match operation {
            Operation::Run(mode) => {
                // A present null is invalid, just like any non-array selection.
                let selected = request
                    .as_object_mut()
                    .and_then(|o| o.remove("_selected_outputs"))
                    .map(decode::<Vec<String>>)
                    .transpose()?;
                Ok(Self::Run(mode, decode(request)?, selected))
            }
            Operation::InitialValues(mode) => Ok(Self::InitialValues(mode, decode(request)?)),
            Operation::Scan(mode) => Ok(Self::Scan(mode, decode(request)?)),
            Operation::Periodic(mode) => Ok(Self::Periodic(mode, decode(request)?)),
            Operation::Morris(mode) => Ok(Self::Morris(mode, decode(request)?)),
            Operation::ScalarFit(mode) => Ok(Self::ScalarFit(mode, decode(request)?)),
            Operation::Fit(mode, kind) => Ok(Self::Fit(mode, decode_fit(kind, request)?)),
            Operation::Compile => Ok(Self::Compile),
            Operation::Validate => Ok(Self::Validate),
        }
    }
}

// Untagged serialization preserves each authoritative core result verbatim.
#[derive(Serialize)]
#[serde(untagged)]
enum NativeOutput {
    RunPlan(RunPlan),
    Run(RunResult),
    InitialValuesPlan(InitialValuesPlan),
    InitialValues(InitialValuesResult),
    ScanPlan(ScanPlan),
    Scan(ScanResult),
    PeriodicPlan(PeriodicPlan),
    Periodic(SteadyStateResult),
    MorrisPlan(MorrisPlan),
    Morris(MorrisExecutionResult),
    ScalarFitPlan(ScalarFitPlan),
    ScalarFit(ScalarGaussianFitResult),
    FitPlan(FitPlan),
    Fit(FitResult),
    Compilation {
        valid: bool,
        model_content_hash: String,
    },
    Validation {
        schema: &'static str,
        valid: bool,
        model_content_hash: String,
        model: ModelDocument,
        canonical_source: String,
    },
}
impl NativeOutput {
    fn into_value(self) -> Result<Value, Value> {
        serde_json::to_value(self).map_err(invalid_request)
    }
}
fn decode_fit(kind: FitKind, request: Value) -> Result<FitRequest, Value> {
    let fit = decode(request)?;
    validate_fit_operation(kind.operation(), &fit)?;
    Ok(fit)
}
fn fit_output(d: &ModelDocument, mode: Mode, request: &FitRequest) -> Result<NativeOutput, Value> {
    let compiled =
        CompiledSensitivityDocument::compile(d, request.parameters()).map_err(core_error)?;
    match mode {
        Mode::Plan => compiled
            .plan_fit(request)
            .map(NativeOutput::FitPlan)
            .map_err(core_error),
        Mode::Execute => compiled
            .execute_fit(request)
            .map(NativeOutput::Fit)
            .map_err(core_error),
    }
}
pub fn execute_document(
    d: &ModelDocument,
    operation: &str,
    request: Value,
) -> Result<Value, Value> {
    // Fits admit their request before sensitivity compilation. All other
    // operations retain ordinary compilation before request admission.
    if let Ok(Operation::Fit(mode, kind)) = Operation::parse(operation) {
        let fit = decode_fit(kind, request)?;
        return fit_output(d, mode, &fit)?.into_value();
    }
    let compiled = CompiledDocument::compile(d).map_err(core_error)?;
    execute_compiled(&compiled, d, operation, request)
}
pub fn execute_compiled(
    compiled: &std::sync::Arc<CompiledDocument>,
    d: &ModelDocument,
    operation: &str,
    request: Value,
) -> Result<Value, Value> {
    let request = NativeRequest::parse(Operation::parse(operation)?, request)?;
    use Mode::{Execute, Plan};
    let output = match request {
        NativeRequest::Run(Plan, request, selected) => NativeOutput::RunPlan(
            compiled
                .plan_run_selected(&request, selected.as_deref())
                .map_err(core_error)?,
        ),
        NativeRequest::Run(Execute, request, selected) => NativeOutput::Run(
            compiled
                .execute_selected(&request, selected.as_deref())
                .map_err(core_error)?,
        ),
        NativeRequest::InitialValues(Plan, request) => NativeOutput::InitialValuesPlan(
            compiled.plan_initial_values(&request).map_err(core_error)?,
        ),
        NativeRequest::InitialValues(Execute, request) => {
            NativeOutput::InitialValues(compiled.initial_values(&request).map_err(core_error)?)
        }
        NativeRequest::Scan(Plan, request) => {
            NativeOutput::ScanPlan(compiled.plan_scan(&request).map_err(core_error)?)
        }
        NativeRequest::Scan(Execute, request) => {
            NativeOutput::Scan(compiled.scan(&request).map_err(core_error)?)
        }
        NativeRequest::Periodic(Plan, request) => NativeOutput::PeriodicPlan(
            compiled
                .plan_periodic_steady_state(&request)
                .map_err(core_error)?,
        ),
        NativeRequest::Periodic(Execute, request) => NativeOutput::Periodic(
            compiled
                .periodic_steady_state(&request)
                .map_err(core_error)?,
        ),
        NativeRequest::Morris(Plan, request) => {
            NativeOutput::MorrisPlan(compiled.plan_morris(&request).map_err(core_error)?)
        }
        NativeRequest::Morris(Execute, request) => {
            NativeOutput::Morris(compiled.execute_morris(&request).map_err(core_error)?)
        }
        NativeRequest::ScalarFit(Plan, request) => {
            NativeOutput::ScalarFitPlan(compiled.plan_scalar_fit(&request).map_err(core_error)?)
        }
        NativeRequest::ScalarFit(Execute, request) => {
            NativeOutput::ScalarFit(compiled.fit_scalar(&request).map_err(core_error)?)
        }
        NativeRequest::Fit(mode, request) => fit_output(d, mode, &request)?,
        NativeRequest::Compile => NativeOutput::Compilation {
            valid: true,
            model_content_hash: compiled.model_content_hash().to_owned(),
        },
        NativeRequest::Validate => NativeOutput::Validation {
            schema: "qspquest.validation/v0.1",
            valid: true,
            model_content_hash: compiled.model_content_hash().to_owned(),
            model: d.clone(),
            canonical_source: language::format(d)
                .map_err(|e| json!({"code":e.code,"message":e.message}))?,
        },
    };
    output.into_value()
}

fn validate_fit_operation(
    operation: &str,
    request: &qspquest_core::fit::FitRequest,
) -> Result<(), Value> {
    let individual = matches!(
        request.problem,
        qspquest_core::fit::FitProblem::Individual(_)
    );
    if (operation == "fit_individual") != individual {
        return Err(
            json!({"code":"unsupported","message":"Fit problem kind does not match the requested tool.","expression":"fit_request.problem.kind","suggestion":"Use fit_individual for an individual problem; use fit_population for pooled, Laplace, FOCEI or native SAEM."}),
        );
    }
    Ok(())
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    fn run() -> Value {
        json!({"schema":"qspquest.run/v0.1","solver":"analytic_linear_pk",
            "budgets":{"solver_callbacks":1000,"output_values":1000,"events":10},
            "regimen":{"end":{"value":1.0,"unit":"h"},"administrations":[],"rtol":1e-7,"atol":1e-9,
                "samples":[{"value":0.0,"unit":"h"},{"value":1.0,"unit":"h"}]}})
    }
    fn individual() -> Value {
        json!({"schema":"qspquest.fit/v0.1","problem":{"kind":"individual","request":{
            "objective":{"simulation":{"schema":"qspquest.sensitivity/v0.1",
                "run":run(),"with_respect_to":["cl"],"derivative_absolute_tolerances":{}},
                "observations":[],"priors":[]},"bounds":[],"max_evaluations":10,
                "max_iterations":5,"gradient_tolerance":1e-5}}})
    }
    #[test]
    fn exact_operation_denominator_rejects_aliases_and_repeated_prefixes() {
        let names = [
            "simulate",
            "get_patient_parameters",
            "scan_parameters",
            "steady_state",
            "morris_screening",
            "fit_scalar",
            "fit_individual",
            "fit_population",
        ];
        for name in names {
            assert!(Operation::parse(name).is_ok());
            assert!(Operation::parse(&format!("plan_{name}")).is_ok());
            assert!(Operation::parse(&format!("plan_plan_{name}")).is_err());
        }
        for name in ["compile_model", "validate_model"] {
            assert!(Operation::parse(name).is_ok());
        }
        for name in [
            "inspect_model",
            "run",
            "simulate ",
            "plan_compile_model",
            "",
        ] {
            assert_eq!(Operation::parse(name).unwrap_err()["code"], "unsupported");
        }
    }
    #[test]
    fn selections_preserve_absence_order_and_present_null_rejection() {
        for mode in [Mode::Plan, Mode::Execute] {
            for selection in [None, Some(json!([])), Some(json!(["cp", "central", "cp"]))] {
                let mut value = run();
                if let Some(selected) = &selection {
                    value["_selected_outputs"] = selected.clone();
                }
                let NativeRequest::Run(_, _, actual) =
                    NativeRequest::parse(Operation::Run(mode), value).unwrap()
                else {
                    panic!("expected run");
                };
                assert_eq!(actual.map(|v| json!(v)), selection);
            }
            for selected in [Value::Null, json!("cp"), json!([1]), json!({"cp":true})] {
                let mut value = run();
                value["_selected_outputs"] = selected;
                let error = NativeRequest::parse(Operation::Run(mode), value)
                    .err()
                    .unwrap();
                assert_eq!(error["code"], "invalid_request");
            }
        }
    }
    #[test]
    fn native_defaults_and_unknown_field_rejection_remain_authoritative() {
        let NativeRequest::Run(_, request, _) =
            NativeRequest::parse(Operation::Run(Mode::Plan), run()).unwrap()
        else {
            panic!("expected run");
        };
        assert!(request.parameters.is_empty());
        assert!(request.initial_states.is_empty());
        assert!(request.accuracy_control.is_local());
        let mut value = run();
        value["unexpected"] = json!(true);
        assert_eq!(
            NativeRequest::parse(Operation::Run(Mode::Execute), value)
                .err()
                .unwrap()["code"],
            "invalid_request"
        );
    }
    #[test]
    fn fit_kind_gate_precedes_model_compilation_for_both_modes() {
        for mode in [Mode::Plan, Mode::Execute] {
            assert!(
                NativeRequest::parse(Operation::Fit(mode, FitKind::Individual), individual())
                    .is_ok()
            );
            let error =
                NativeRequest::parse(Operation::Fit(mode, FitKind::Population), individual())
                    .err()
                    .unwrap();
            assert_eq!(error["code"], "unsupported");
            assert_eq!(error["expression"], "fit_request.problem.kind");
        }
    }
    #[test]
    fn inspection_and_error_order_are_preserved() {
        let source =
            include_str!("../../assets/templates/oral.qsp").replace("central / vc", "missing / vc");
        assert!(execute(&source, "inspect_model", Value::Null).is_ok());
        let document = model(&source).unwrap();
        for operation in ["simulate", "plan_simulate", "unknown", "inspect_model"] {
            assert_eq!(
                execute_document(&document, operation, Value::Null).unwrap_err()["code"],
                "UnknownSymbol"
            );
        }
        for operation in [
            "fit_individual",
            "plan_fit_individual",
            "fit_population",
            "plan_fit_population",
        ] {
            assert_eq!(
                execute_document(&document, operation, Value::Null).unwrap_err()["code"],
                "invalid_request"
            );
        }
    }
    #[test]
    fn metadata_serialization_keeps_untagged_wire_shape_and_ignores_request() {
        let source = include_str!("../../assets/templates/oral.qsp");
        let document = model(source).unwrap();
        let compiled = CompiledDocument::compile(&document).unwrap();
        let expected = json!({"valid":true,"model_content_hash":compiled.model_content_hash()});
        for input in [Value::Null, json!([]), json!({"unexpected":true})] {
            assert_eq!(
                execute_compiled(&compiled, &document, "compile_model", input.clone()).unwrap(),
                expected
            );
            let value = execute_compiled(&compiled, &document, "validate_model", input).unwrap();
            assert_eq!(
                value,
                json!({"schema":"qspquest.validation/v0.1","valid":true,
                "model_content_hash":compiled.model_content_hash(),"model":document,
                "canonical_source":language::format(&document).unwrap()})
            );
        }
        assert_eq!(
            execute_compiled(&compiled, &document, "inspect_model", Value::Null).unwrap_err()
                ["code"],
            "unsupported"
        );
    }
}

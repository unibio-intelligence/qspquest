//! Archive-bound synthetic fitting workflows; recorded recovery is not a current-run claim.
use crate::{contract::JsonValueSchema, library::hash};
use qspquest_core::fit::{FitProblem, FitRequest, ScalarGaussianFitRequest};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const SCHEMA: &str = "qspquest.library-fitting/v1";
const LEGACY: &str = "qspquest.mcp-fitting-contract/v0.1";

#[derive(Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "LibraryFitOperation")]
pub enum Operation {
    FitScalar,
    FitIndividual,
}
impl Operation {
    fn name(&self) -> &'static str {
        match self {
            Self::FitScalar => "fit_scalar",
            Self::FitIndividual => "fit_individual",
        }
    }
}
#[derive(Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "LibraryFitScope")]
pub enum Scope {
    SyntheticParameterRecoveryFixture,
}
#[derive(Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "LibraryFitQualificationStatus")]
pub enum Status {
    NotAssessed,
    Passed,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "LibraryFitMember")]
pub struct Member {
    pub path: String,
    pub sha256: String,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "LibraryFitParameter")]
pub struct Parameter {
    pub native_name: String,
    pub unit: String,
    pub lower: f64,
    pub upper: f64,
    pub starter: f64,
    pub synthetic_truth: f64,
    pub recovery_absolute_tolerance: f64,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "LibraryFitQualification")]
pub struct Qualification {
    pub status: Status,
    pub independent_method: String,
    pub evidence: Option<Member>,
    pub expected_status: String,
    pub interpretation: String,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "LibraryFitWorkflow")]
pub struct Workflow {
    pub id: String,
    pub operation: Operation,
    pub parameters: Vec<Parameter>,
    /// Complete native request fixes observation values, residual model, priors,
    /// regimen, estimator settings, bounds and starter values for this fixture.
    #[schemars(with = "JsonValueSchema")]
    pub native_request: Value,
    pub synthetic_observations: Member,
    pub qualification: Qualification,
    pub limitations: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "LibraryFittingDeclaration")]
pub struct Declaration {
    #[schemars(extend("const" = "qspquest.library-fitting/v1"))]
    pub schema: String,
    pub model_id: String,
    pub model_version: String,
    pub native_model_sha256: String,
    pub fit_ready: bool,
    pub fitting_supported: bool,
    pub scope: Scope,
    #[schemars(extend("const" = false))]
    pub source_population_reproduction: bool,
    pub workflows: Vec<Workflow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyContract {
    schema: String,
    scientific_status: String,
    operation: Operation,
    parameter: String,
    output: String,
    solver: String,
    lower: qspquest_core::model::Quantity,
    upper: qspquest_core::model::Quantity,
    max_observations: usize,
    qualification: Value,
    limitations: Vec<String>,
    fit_ready: Option<bool>,
    fitting_supported: Option<bool>,
}

fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn member(
    reference: &Member,
    index: &Value,
    read: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Value, String> {
    if !digest(&reference.sha256)
        || reference.path.contains(['\\', ':'])
        || reference
            .path
            .split('/')
            .any(|part| matches!(part, "" | "." | ".."))
        || index["members"][&reference.path].as_str() != Some(&reference.sha256)
    {
        return Err("Fitting data/evidence must be a safe hash-indexed archive member".into());
    }
    let bytes = read(&reference.path)?;
    if bytes.len() > 1_000_000 || hash(&bytes) != reference.sha256 {
        return Err("Fitting data/evidence size or SHA-256 differs".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("Fitting data/evidence JSON: {e}"))
}

/// Parse and normalize through the actual native request type. Unknown fields,
/// unsupported problem kinds and invalid JSON never become a contract match.
fn native(operation: &Operation, request: &Value) -> Result<Value, String> {
    match operation {
        Operation::FitScalar => serde_json::from_value::<ScalarGaussianFitRequest>(request.clone())
            .and_then(serde_json::to_value)
            .map_err(|e| format!("Scalar fitting request: {e}")),
        Operation::FitIndividual => {
            let fit: FitRequest = serde_json::from_value(request.clone())
                .map_err(|e| format!("Individual fitting request: {e}"))?;
            if !matches!(fit.problem, FitProblem::Individual(_)) {
                return Err("Individual fitting contract requires problem.kind individual".into());
            }
            serde_json::to_value(fit).map_err(|e| e.to_string())
        }
    }
}
fn pieces<'a>(operation: &Operation, request: &'a Value) -> (&'a Value, &'a Value) {
    match operation {
        Operation::FitScalar => (&request["run"], &request["observations"]),
        Operation::FitIndividual => (
            &request["problem"]["request"]["objective"]["simulation"]["run"],
            &request["problem"]["request"]["objective"]["observations"],
        ),
    }
}
fn quantity_matches(value: &Value, number: f64, unit: &str) -> bool {
    value["value"].as_f64() == Some(number) && value["unit"].as_str() == Some(unit)
}

impl Declaration {
    fn validate(
        &self,
        metadata: &Value,
        model: &Value,
        index: &Value,
        read: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<(), String> {
        let native_path = index["native_execution"]["model"]
            .as_str()
            .unwrap_or("native/model.json");
        if self.schema != SCHEMA
            || metadata["id"] != self.model_id
            || metadata["version"] != self.model_version
            || model["model_id"] != self.model_id
            || !digest(&self.native_model_sha256)
            || index["members"][native_path].as_str() != Some(&self.native_model_sha256)
            || metadata["fit_ready"].as_bool() != Some(self.fit_ready)
            || metadata["fitting_supported"].as_bool() != Some(self.fitting_supported)
            || self.source_population_reproduction
            || self.workflows.is_empty()
            || self.workflows.len() > 8
        {
            return Err(
                "Fitting identity, readiness, source scope or workflow count differs".into(),
            );
        }
        let document: qspquest_core::model::ModelDocument =
            serde_json::from_value(model.clone())
                .map_err(|e| format!("Native fitting model: {e}"))?;
        let model_content_hash = qspquest_core::identity::canonical_hash(
            &serde_json::to_value(document).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if self.fit_ready
            && (model.is_null()
                || metadata["availability"]["availability"] == "source_replay"
                || metadata["availability"]["availability"] == "inspection_only")
        {
            return Err("Source-replay and inspection-only models cannot be fit ready".into());
        }
        let mut ids = BTreeSet::new();
        let mut requests = BTreeSet::new();
        let mut ready = false;
        for workflow in &self.workflows {
            if workflow.id.is_empty()
                || !ids.insert(&workflow.id)
                || workflow.parameters.is_empty()
                || workflow.parameters.len() > 32
                || workflow.limitations.is_empty()
                || workflow.limitations.iter().any(String::is_empty)
                || workflow.qualification.independent_method.is_empty()
                || workflow.qualification.interpretation.is_empty()
                || workflow.qualification.expected_status != "converged"
            {
                return Err("Fitting workflow requires a unique ID, parameter roles and diagnostic interpretation".into());
            }
            let request = native(&workflow.operation, &workflow.native_request)?;
            let request_bytes =
                qspquest_core::identity::canonical_bytes(&request).map_err(|e| e.to_string())?;
            if !requests.insert((workflow.operation.name(), hash(&request_bytes))) {
                return Err("Duplicate native fitting workflow request".into());
            }
            let (run, observations) = pieces(&workflow.operation, &request);
            let observations = observations
                .as_array()
                .ok_or("Fitting observations must be an array")?;
            if observations.is_empty()
                || observations.len() > 4096
                || member(&workflow.synthetic_observations, index, read)? != json!(observations)
            {
                return Err("Synthetic observations must match every complete request row and residual contract".into());
            }
            for observation in observations {
                if observation
                    .get("censoring")
                    .is_some_and(|value| !value.is_null())
                    || observation["error"]
                        .get("model")
                        .is_some_and(|value| value != "normal")
                {
                    return Err("V1 synthetic recovery workflows require uncensored normal residual errors; another observation likelihood needs a separate supported contract".into());
                }
                let output = model["outputs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|output| output["name"] == observation["output"])
                    .ok_or("Unknown native observation endpoint")?;
                let additive = observation["error"]["additive_sd"]["value"]
                    .as_f64()
                    .ok_or("Additive SD must be finite")?;
                let proportional = observation["error"]["proportional_sd"]
                    .as_f64()
                    .ok_or("Proportional SD must be finite")?;
                if observation["value"]["unit"] != output["unit"]
                    || observation["error"]["additive_sd"]["unit"] != output["unit"]
                    || additive < 0.
                    || proportional < 0.
                    || (additive == 0. && proportional == 0.)
                {
                    return Err("Synthetic observations require the declared endpoint units and a positive residual-error component".into());
                }
            }
            let mut names = BTreeSet::new();
            for role in &workflow.parameters {
                let model_parameter = model["parameters"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|parameter| parameter["name"] == role.native_name)
                    .ok_or("Unknown native fitting parameter")?;
                if !names.insert(&role.native_name)
                    || model_parameter["fixed"] == true
                    || model_parameter["default"]["unit"] != role.unit
                    || [
                        role.lower,
                        role.upper,
                        role.starter,
                        role.synthetic_truth,
                        role.recovery_absolute_tolerance,
                    ]
                    .iter()
                    .any(|v| !v.is_finite())
                    || role.lower >= role.upper
                    || role.starter < role.lower
                    || role.starter > role.upper
                    || role.synthetic_truth < role.lower
                    || role.synthetic_truth > role.upper
                    || role.recovery_absolute_tolerance <= 0.
                    || model_parameter["transform"] == "log" && role.lower <= 0.
                    || matches!(
                        model_parameter["transform"].as_str(),
                        Some("logit" | "probit")
                    ) && (role.lower <= 0. || role.upper >= 1.)
                {
                    return Err("Fitting parameter role has inconsistent names, units, bounds, starter or truth".into());
                }
                let initial = run["parameters"]
                    .get(&role.native_name)
                    .unwrap_or(&model_parameter["default"]);
                if !quantity_matches(initial, role.starter, &role.unit) {
                    return Err(
                        "Fitting starter differs from the exact native request/model".into(),
                    );
                }
                match workflow.operation {
                    Operation::FitScalar => {
                        if workflow.parameters.len() != 1
                            || request["parameter"] != role.native_name
                            || !quantity_matches(&request["lower"], role.lower, &role.unit)
                            || !quantity_matches(&request["upper"], role.upper, &role.unit)
                        {
                            return Err(
                                "Scalar parameter/bounds differ from the declared role".into()
                            );
                        }
                    }
                    Operation::FitIndividual => {
                        let fit = &request["problem"]["request"];
                        let bound = fit["bounds"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|bound| bound["parameter"] == role.native_name)
                            .ok_or("Individual fitting bound missing")?;
                        if !quantity_matches(&bound["lower"], role.lower, &role.unit)
                            || !quantity_matches(&bound["upper"], role.upper, &role.unit)
                        {
                            return Err(
                                "Individual fitting bounds differ from the declared role".into()
                            );
                        }
                    }
                }
            }
            if let Operation::FitIndividual = workflow.operation {
                let fit = &request["problem"]["request"];
                let with_respect_to = fit["objective"]["simulation"]["with_respect_to"]
                    .as_array()
                    .ok_or("Sensitivity parameter list missing")?;
                let ordered = workflow
                    .parameters
                    .iter()
                    .map(|p| json!(p.native_name))
                    .collect::<Vec<_>>();
                if with_respect_to != &ordered
                    || fit["bounds"].as_array().map(Vec::len) != Some(ordered.len())
                    || !fit["objective"]["priors"]
                        .as_array()
                        .is_some_and(Vec::is_empty)
                {
                    return Err("Individual fixture requires exactly its ordered fitted roles and no priors".into());
                }
            }
            match workflow.qualification.status {
                Status::NotAssessed if workflow.qualification.evidence.is_some() => {
                    return Err("Unassessed fitting workflow must not carry passed evidence".into());
                }
                Status::Passed => {
                    let evidence = member(
                        workflow
                            .qualification
                            .evidence
                            .as_ref()
                            .ok_or("Passed fitting workflow requires evidence")?,
                        index,
                        read,
                    )?;
                    if evidence["schema"] != "qspquest.fitting-qualification/v1"
                        || evidence["status"] != "passed"
                        || evidence["model_id"] != self.model_id
                        || evidence["model_version"] != self.model_version
                        || evidence["native_model_sha256"] != self.native_model_sha256
                        || evidence["native_request_sha256"] != hash(&request_bytes)
                        || evidence["synthetic_observations_sha256"]
                            != workflow.synthetic_observations.sha256
                        || evidence["workflow_id"] != workflow.id
                        || evidence["operation"] != workflow.operation.name()
                        || evidence["source_population_reproduction"] != false
                        || !evidence["engine_identity"]["compiler_source_sha256"]
                            .as_str()
                            .is_some_and(|s| s.strip_prefix("sha256:").is_some_and(digest))
                        || !evidence["engine_identity"]["target"]
                            .as_str()
                            .is_some_and(|s| !s.is_empty())
                    {
                        return Err("Recorded fitting evidence differs from native source, request, observations or operation".into());
                    }
                    ready = true;
                    let result_descriptor: Member =
                        serde_json::from_value(evidence["native_result"].clone())
                            .map_err(|e| format!("Fitting evidence result descriptor: {e}"))?;
                    let raw_result = member(&result_descriptor, index, read)?;
                    let result = match workflow.operation {
                        Operation::FitScalar => serde_json::to_value(
                            serde_json::from_value::<qspquest_core::fit::ScalarGaussianFitResult>(
                                raw_result,
                            )
                            .map_err(|e| format!("Recorded scalar fitting result: {e}"))?,
                        )
                        .map_err(|e| e.to_string())?,
                        Operation::FitIndividual => {
                            let result: qspquest_core::fit::FitResult =
                                serde_json::from_value(raw_result).map_err(|e| {
                                    format!("Recorded individual fitting result: {e}")
                                })?;
                            if result.population.is_some() || result.fit.is_none() {
                                return Err(
                                    "Individual evidence must retain an individual fit result"
                                        .into(),
                                );
                            }
                            serde_json::to_value(result.fit).map_err(|e| e.to_string())?
                        }
                    };
                    if result["status"] != "converged"
                        || result["objective"]["identity"] != evidence["engine_identity"]
                        || result["objective"]["identity"]["model_content_hash"]
                            != model_content_hash
                        || !result["evaluations"].as_u64().is_some_and(|n| n > 0)
                        || result["objective"]["observations"].as_u64()
                            != Some(observations.len() as u64)
                        || result["fit_request_hash"] != evidence["fit_request_hash"]
                        || !result["fit_request_hash"]
                            .as_str()
                            .is_some_and(|s| s.strip_prefix("sha256:").is_some_and(digest))
                    {
                        return Err("Recorded fitting result status, observations, request hash or engine identity differs".into());
                    }
                    if let Operation::FitIndividual = workflow.operation {
                        let gradient = result["projected_gradient_norm"]
                            .as_f64()
                            .ok_or("Missing fitting gradient diagnostic")?;
                        let tolerance = request["problem"]["request"]["gradient_tolerance"]
                            .as_f64()
                            .ok_or("Missing gradient tolerance")?;
                        if gradient < 0. || tolerance <= 0. || gradient > tolerance {
                            return Err("Recorded individual fitting result exceeds its projected-gradient tolerance".into());
                        }
                    }
                    for role in &workflow.parameters {
                        let recovered = &result["parameters"][&role.native_name];
                        if recovered["unit"] != role.unit
                            || !recovered["value"].as_f64().is_some_and(|value| {
                                (value - role.synthetic_truth).abs()
                                    <= role.recovery_absolute_tolerance
                            })
                        {
                            return Err("Recorded fitting result fails its parameter recovery limit or units".into());
                        }
                    }
                }
                Status::NotAssessed => (),
            }
        }
        if self.fit_ready != ready || self.fitting_supported != ready {
            return Err(
                "Fitting readiness requires at least one passed, hash-bound fixture workflow"
                    .into(),
            );
        }
        Ok(())
    }
}

/// Called after archive member checks, before readiness enters discovery or execution.
pub fn validate_archive(
    contract: &Value,
    metadata: &Value,
    model: &Value,
    index: &Value,
    mut read: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    match contract["schema"].as_str() {
        Some(SCHEMA) => serde_json::from_value::<Declaration>(contract.clone())
            .map_err(|e| format!("Library fitting declaration: {e}"))?
            .validate(metadata, model, index, &mut read),
        Some(LEGACY) => {
            let legacy: LegacyContract = serde_json::from_value(contract.clone())
                .map_err(|e| format!("Legacy library fitting contract: {e}"))?;
            let parameter = model["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|p| p["name"] == legacy.parameter)
                .ok_or("Legacy fitting parameter is absent from the native model")?;
            if legacy.schema != LEGACY
                || !matches!(legacy.operation, Operation::FitScalar)
                || legacy.output.is_empty()
                || legacy.scientific_status.is_empty()
                || !matches!(
                    legacy.solver.as_str(),
                    "diffsol_bdf" | "diffsol_rodas5p" | "analytic_linear_pk"
                )
                || legacy.lower.unit != legacy.upper.unit
                || parameter["default"]["unit"] != legacy.lower.unit
                || !legacy.lower.value.is_finite()
                || !legacy.upper.value.is_finite()
                || legacy.lower.value >= legacy.upper.value
                || legacy.max_observations == 0
                || legacy.max_observations > 4096
                || !model["outputs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|o| o["name"] == legacy.output)
                || !legacy.qualification.is_object()
                || legacy.limitations.is_empty()
                || legacy.limitations.iter().any(String::is_empty)
                || legacy
                    .fit_ready
                    .is_some_and(|flag| metadata["fit_ready"].as_bool() != Some(flag))
                || legacy
                    .fitting_supported
                    .is_some_and(|flag| metadata["fitting_supported"].as_bool() != Some(flag))
            {
                return Err("Legacy library fitting contract supports only its scalar endpoint/bounds profile".into());
            }
            Ok(())
        }
        None if metadata["fit_ready"] != true
            && metadata["fitting_supported"] != true
            && contract["fit_ready"] != true
            && contract["fitting_supported"] != true
            && contract["supported"] != true =>
        {
            Ok(())
        }
        _ => Err("Unknown fitting contract schema or missing ready contract".into()),
    }
}

pub fn supports_operation(contract: &Value, operation: &str) -> bool {
    if contract["schema"] == SCHEMA {
        contract["workflows"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|workflow| {
                workflow["operation"] == operation
                    && workflow["qualification"]["status"] == "passed"
            })
    } else {
        contract["schema"] == LEGACY
            && operation == "fit_scalar"
            && contract["operation"] == operation
    }
}

/// Exact fixture membership is deliberate: no observed-value, residual, estimator,
/// prior, solver, starter, bound, event, budget or censoring edits inherit its claim.
pub fn enforce(
    contract: &Value,
    operation: &str,
    request: &Value,
) -> Result<Option<String>, String> {
    if contract["schema"] == SCHEMA {
        let declaration: Declaration =
            serde_json::from_value(contract.clone()).map_err(|e| e.to_string())?;
        for workflow in declaration.workflows {
            if workflow.operation.name() == operation
                && matches!(workflow.qualification.status, Status::Passed)
                && native(&workflow.operation, request)?
                    == native(&workflow.operation, &workflow.native_request)?
            {
                return Ok(Some(workflow.id));
            }
        }
        return Err("Use the exact published synthetic native_request. Changed observations, priors, residuals, bounds, starters, solvers or budgets need separate qualification; exploratory fitting requires an explicitly labeled user model.".into());
    }
    if !supports_operation(contract, operation) {
        return Err("This archive has no fitting contract for the requested operation".into());
    }
    if request["parameter"] != contract["parameter"]
        || request["run"]["solver"] != contract["solver"]
        || request["lower"] != contract["lower"]
        || request["upper"] != contract["upper"]
        || !request["observations"].as_array().is_some_and(|a| {
            !a.is_empty()
                && a.len() <= contract["max_observations"].as_u64().unwrap_or(0) as usize
                && a.iter().all(|o| o["output"] == contract["output"])
        })
    {
        return Err("Library scalar fitting requires the published parameter, solver, bounds and observation endpoint/count contract".into());
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    // Structural archive fixtures only. These records never establish numerical qualification.
    use super::*;
    use std::collections::BTreeMap;

    struct Fixture {
        metadata: Value,
        model: Value,
        index: Value,
        contract: Value,
        files: BTreeMap<String, Vec<u8>>,
    }
    fn fixture(operation: Operation, passed: bool) -> Fixture {
        let model: Value = serde_json::from_str(include_str!(
            "../../../conformance/models/synthetic-one-compartment.json"
        ))
        .unwrap();
        let document: qspquest_core::model::ModelDocument =
            serde_json::from_value(model.clone()).unwrap();
        let model_hash =
            qspquest_core::identity::canonical_hash(&serde_json::to_value(document).unwrap())
                .unwrap();
        let source_sha = hash(&serde_json::to_vec(&model).unwrap());
        let run: Value =
            serde_json::from_str(include_str!("../../../docs/examples/first-run.json")).unwrap();
        let observations = json!([{"row":0,"output":"cp","value":{"value":2.,"unit":"mg/L"},
            "error":{"additive_sd":{"value":0.1,"unit":"mg/L"},"proportional_sd":0.}}]);
        let request = match operation {
            Operation::FitScalar => {
                json!({"schema":"qspquest.scalar-fit/v0.1","run":run,"parameter":"cl",
                "lower":{"value":0.1,"unit":"L/h"},"upper":{"value":1.,"unit":"L/h"},"observations":observations,
                "max_iterations":100,"absolute_tolerance":1e-8})
            }
            Operation::FitIndividual => {
                json!({"schema":"qspquest.fit/v0.1","problem":{"kind":"individual","request":{
                "objective":{"simulation":{"schema":"qspquest.sensitivity/v0.1","run":run,"with_respect_to":["cl"],
                    "derivative_absolute_tolerances":{"cl":{"central":{"value":1e-10,"unit":"(mg)/(L/h)"}}}},
                    "observations":observations,"priors":[]},"bounds":[{"parameter":"cl","lower":{"value":0.1,"unit":"L/h"},"upper":{"value":1.,"unit":"L/h"}}],
                "max_evaluations":100,"max_iterations":50,"gradient_tolerance":1e-6}}})
            }
        };
        let request = native(&operation, &request).unwrap();
        let (_, observations) = pieces(&operation, &request);
        let data = serde_json::to_vec(observations).unwrap();
        let data_sha = hash(&data);
        let mut files = BTreeMap::from([("fitting/observations.json".into(), data)]);
        let mut contract = json!({"schema":SCHEMA,"model_id":"synthetic_one_compartment","model_version":"test",
            "native_model_sha256":source_sha,"fit_ready":passed,"fitting_supported":passed,
            "scope":"synthetic_parameter_recovery_fixture","source_population_reproduction":false,"workflows":[{
                "id":"recovery","operation":operation.name(),"parameters":[{"native_name":"cl","unit":"L/h","lower":0.1,"upper":1.,
                    "starter":0.3,"synthetic_truth":0.6,"recovery_absolute_tolerance":1e-5}],"native_request":request,
                "synthetic_observations":{"path":"fitting/observations.json","sha256":data_sha},
                "qualification":{"status":if passed {"passed"} else {"not_assessed"},"independent_method":"Structural fixture; no fit executed",
                    "evidence":null,"expected_status":"converged","interpretation":"Parser/binding checks only"},
                "limitations":["Structural archive fixture; no scientific evidence"]}]});
        if passed {
            let identity = json!({"model_content_hash":model_hash,"spec_version":"qspquest.spec/0.1",
                "compiler_version":"qspquest/test","compiler_source_sha256":format!("sha256:{}", "b".repeat(64)),
                "rustc_version":"test","target":"test-target","build_profile":"test","rustflags_sha256":"test",
                "bound_value_hash":"test","run_request_hash":"test","backend":"test","backend_version":"test","algorithm":"test"});
            let fit_hash = format!("sha256:{}", "c".repeat(64));
            let mut result = json!({"status":"converged","parameters":{"cl":{"value":0.6,"unit":"L/h"}},
                "objective":{"identity":identity,"negative_log_likelihood":1.,"objective":1.,"observations":1},
                "evaluations":1,"iterations":0,"fit_request_hash":fit_hash});
            let result = match operation {
                Operation::FitScalar => {
                    result["schema"] = json!("qspquest.scalar-fit-result/v0.1");
                    result
                }
                Operation::FitIndividual => {
                    result["objective"]["negative_log_prior"] = json!(0.);
                    result["objective"]["gradient"] =
                        json!([{"parameter":"cl","unit":"(h)/(L)","value":0.}]);
                    result["projected_gradient_norm"] = json!(1e-8);
                    json!({"schema":"qspquest.fit-result/v0.1","fit":result})
                }
            };
            let result_bytes = serde_json::to_vec(&result).unwrap();
            let result_sha = hash(&result_bytes);
            files.insert("fitting/result.json".into(), result_bytes);
            let proof = json!({"schema":"qspquest.fitting-qualification/v1","status":"passed","model_id":"synthetic_one_compartment",
                "model_version":"test","native_model_sha256":source_sha,"native_request_sha256":hash(&qspquest_core::identity::canonical_bytes(&request).unwrap()),
                "synthetic_observations_sha256":data_sha,"workflow_id":"recovery","operation":operation.name(),
                "source_population_reproduction":false,"engine_identity":identity,"fit_request_hash":fit_hash,
                "native_result":{"path":"fitting/result.json","sha256":result_sha}});
            let proof_bytes = serde_json::to_vec(&proof).unwrap();
            contract["workflows"][0]["qualification"]["evidence"] =
                json!({"path":"fitting/evidence.json","sha256":hash(&proof_bytes)});
            files.insert("fitting/evidence.json".into(), proof_bytes);
        }
        let mut index = json!({"native_execution":{"model":"native/model.json"},"members":{"native/model.json":source_sha}});
        for (name, bytes) in &files {
            index["members"][name] = json!(hash(bytes));
        }
        Fixture {
            metadata: json!({"id":"synthetic_one_compartment","version":"test","fit_ready":passed,"fitting_supported":passed,
                    "availability":{"availability":"dynamic"}}),
            model,
            index,
            contract,
            files,
        }
    }
    fn check(f: &Fixture) -> Result<(), String> {
        validate_archive(&f.contract, &f.metadata, &f.model, &f.index, |name| {
            f.files.get(name).cloned().ok_or("Missing member".into())
        })
    }
    #[test]
    fn unassessed_workflow_is_discoverable_but_not_ready() {
        let f = fixture(Operation::FitScalar, false);
        check(&f).unwrap();
        assert!(!supports_operation(&f.contract, "fit_scalar"));
        assert!(enforce(
            &f.contract,
            "fit_scalar",
            &f.contract["workflows"][0]["native_request"]
        )
        .is_err());
    }
    #[test]
    fn scalar_and_individual_contracts_match_only_their_complete_native_request() {
        for operation in [Operation::FitScalar, Operation::FitIndividual] {
            let f = fixture(operation, true);
            check(&f).unwrap();
            let mut request = f.contract["workflows"][0]["native_request"].clone();
            assert_eq!(
                enforce(&f.contract, operation.name(), &request)
                    .unwrap()
                    .as_deref(),
                Some("recovery")
            );
            let observations = match operation {
                Operation::FitScalar => &mut request["observations"],
                Operation::FitIndividual => {
                    &mut request["problem"]["request"]["objective"]["observations"]
                }
            };
            observations[0]["value"]["value"] = json!(2.01);
            assert!(enforce(&f.contract, operation.name(), &request).is_err());
            assert!(!supports_operation(&f.contract, "fit_population"));
        }
    }
    #[test]
    fn cannot_promote_a_draft_or_source_only_model_or_change_identity() {
        for variant in [
            "draft",
            "source_replay",
            "inspection_only",
            "source_hash",
            "population_claim",
            "unknown_schema",
            "unknown_field",
        ] {
            let mut f = fixture(Operation::FitScalar, variant != "draft");
            match variant {
                "draft" => {
                    f.contract["fit_ready"] = json!(true);
                    f.contract["fitting_supported"] = json!(true);
                    f.metadata["fit_ready"] = json!(true);
                    f.metadata["fitting_supported"] = json!(true);
                }
                "source_replay" | "inspection_only" => {
                    f.metadata["availability"]["availability"] = json!(variant)
                }
                "source_hash" => f.contract["native_model_sha256"] = json!("d".repeat(64)),
                "population_claim" => f.contract["source_population_reproduction"] = json!(true),
                "unknown_schema" => f.contract["schema"] = json!("qspquest.library-fitting/v9"),
                "unknown_field" => f.contract["ignore_this"] = json!(true),
                _ => unreachable!(),
            }
            assert!(check(&f).is_err(), "{variant}");
        }
    }
    #[test]
    fn request_roles_and_hashed_data_are_checked_before_readiness() {
        for variant in [
            "data_bytes",
            "data_path",
            "bounds",
            "starter",
            "unit",
            "endpoint",
            "duplicate_role",
            "problem_kind",
            "evidence_bytes",
        ] {
            let mut f = fixture(Operation::FitIndividual, true);
            match variant {
                "data_bytes" => f
                    .files
                    .get_mut("fitting/observations.json")
                    .unwrap()
                    .push(b' '),
                "data_path" => {
                    f.contract["workflows"][0]["synthetic_observations"]["path"] =
                        json!("../observations.json")
                }
                "bounds" => f.contract["workflows"][0]["parameters"][0]["lower"] = json!(0.2),
                "starter" => f.contract["workflows"][0]["parameters"][0]["starter"] = json!(0.4),
                "unit" => f.contract["workflows"][0]["parameters"][0]["unit"] = json!("mL/h"),
                "endpoint" => f.model["outputs"][0]["unit"] = json!("ug/L"),
                "duplicate_role" => {
                    let role = f.contract["workflows"][0]["parameters"][0].clone();
                    f.contract["workflows"][0]["parameters"]
                        .as_array_mut()
                        .unwrap()
                        .push(role);
                }
                "problem_kind" => {
                    f.contract["workflows"][0]["native_request"]["problem"]["kind"] = json!("saem")
                }
                "evidence_bytes" => f.files.get_mut("fitting/evidence.json").unwrap().push(b' '),
                _ => unreachable!(),
            }
            assert!(check(&f).is_err(), "{variant}");
        }
    }
}

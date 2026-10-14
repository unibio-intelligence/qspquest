//! Declared protocol meaning alongside the native clock, initials and binding phases.
use crate::{contract::EngineIdentity, library::hash, planning::DoseQuantity, policy};
use qspquest_core::{
    expression::{Binary, Expr},
    planning::RunPlan,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub path: String,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub enum DeclarationSchema {
    #[serde(rename = "qspquest.protocol-initialization/v1")]
    V1,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NormalizationBasis {
    DeclaredReferenceInput,
    ConditionSpecificBinding,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Normalization {
    pub canonical_output: String,
    pub numerator_state: String,
    pub denominator_input: String,
    pub multiplier: f64,
    pub basis: NormalizationBasis,
    /// This policy describes the declared input expression, never an inferred trajectory baseline.
    pub interpretation: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub native_request_sha256: String,
    pub integration_start: DoseQuantity,
    pub intervention_time: Option<DoseQuantity>,
    pub explicit_initial_override_count: usize,
    pub interpretation: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "ProtocolInitializationManifest")]
pub struct Manifest {
    pub schema: DeclarationSchema,
    pub model_id: String,
    pub model_version: String,
    pub archive_sha256: String,
    pub native_model_sha256: String,
    pub scenarios: BTreeMap<String, Scenario>,
    pub normalizations: Vec<Normalization>,
    pub scope: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "ProtocolInitializationDeclaration")]
pub struct Declaration {
    pub file_sha256: String,
    pub manifest: Manifest,
}
fn valid_text(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= 1024 && !text.chars().any(char::is_control)
}
fn valid_hash(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Declaration {
    pub fn load(
        raw: &[u8],
        descriptor: &Descriptor,
        metadata: &Value,
        model: &Value,
        index: &Value,
        scenarios: &BTreeMap<String, Value>,
        native_hash: &str,
    ) -> Result<Self, String> {
        if raw.len() > 1_000_000
            || !valid_hash(&descriptor.sha256)
            || hash(raw) != descriptor.sha256
        {
            return Err(
                "Protocol initialization file exceeds 1 MB or differs from its catalog SHA-256"
                    .into(),
            );
        }
        let manifest: Manifest = serde_json::from_slice(raw)
            .map_err(|e| format!("Protocol initialization declaration: {e}"))?;
        if manifest.model_id != metadata["id"]
            || manifest.model_version != metadata["version"]
            || manifest.archive_sha256 != metadata["omex_sha256"]
            || manifest.native_model_sha256 != native_hash
        {
            return Err(
                "Protocol initialization model/version/archive/native identity mismatch".into(),
            );
        }
        if !valid_text(&manifest.scope)
            || manifest.scenarios.keys().collect::<Vec<_>>() != scenarios.keys().collect::<Vec<_>>()
        {
            return Err("Protocol initialization must cover the exact executable scenario set and declare its scope".into());
        }
        for (id, declared) in &manifest.scenarios {
            let request = &scenarios[id];
            let path = format!("native/scenarios/{id}.json");
            let start = request["regimen"]
                .get("start")
                .filter(|s| !s.is_null())
                .cloned()
                .unwrap_or_else(|| json!({"value":0.,"unit":model["time_unit"]}));
            let start: DoseQuantity = serde_json::from_value(start)
                .map_err(|e| format!("Native scenario '{id}' start quantity: {e}"))?;
            if !valid_hash(&declared.native_request_sha256)
                || index["members"][&path] != declared.native_request_sha256
                || !declared.integration_start.value.is_finite()
                || start.value != declared.integration_start.value
                || start.unit != declared.integration_start.unit
                || !valid_text(&declared.interpretation)
                || declared.explicit_initial_override_count
                    != request["initial_states"].as_object().map_or(0, |s| s.len())
            {
                return Err(format!(
                    "Protocol initialization differs from hash-indexed scenario '{id}' or lacks a valid clock/initialization interpretation"
                ));
            }
            if let Some(intervention) = &declared.intervention_time {
                if !intervention.value.is_finite() || !valid_text(&intervention.unit) {
                    return Err(format!("Invalid declared intervention quantity for '{id}'"));
                }
                let unit = qspquest_core::units::Unit::parse(&intervention.unit)
                    .map_err(|e| e.to_string())?;
                unit.conversion_to(
                    qspquest_core::units::Unit::parse(
                        model["time_unit"]
                            .as_str()
                            .ok_or("Native time unit absent")?,
                    )
                    .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            }
        }
        let states = crate::library::display_states(model);
        let inputs: BTreeSet<&str> = ["parameters", "covariates", "individual"]
            .iter()
            .flat_map(|section| model[*section].as_array().into_iter().flatten())
            .filter_map(|p| p["name"].as_str())
            .collect();
        let mut seen = BTreeSet::new();
        if manifest.normalizations.len() > 128 {
            return Err("At most 128 normalization declarations are supported".into());
        }
        for n in &manifest.normalizations {
            if !seen.insert(&n.canonical_output)
                || !valid_text(&n.canonical_output)
                || !valid_text(&n.numerator_state)
                || !valid_text(&n.denominator_input)
                || !valid_text(&n.interpretation)
                || !n.multiplier.is_finite()
                || n.multiplier <= 0.
                || !states.iter().any(|s| s["name"] == n.numerator_state)
                || !inputs.contains(n.denominator_input.as_str())
            {
                return Err("Normalization requires unique outputs, declared state/input coordinates and a positive finite multiplier".into());
            }
            let expected = Expr::binary(
                Binary::Divide,
                Expr::binary(
                    Binary::Multiply,
                    Expr::number(n.multiplier),
                    Expr::symbol(&n.numerator_state),
                ),
                Expr::symbol(&n.denominator_input),
            );
            let output = model["outputs"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|o| o["name"] == n.canonical_output)
                .ok_or("Normalization refers to an absent native output")?;
            if output["expression"] != json!(expected) {
                return Err(format!(
                    "Normalization '{}' differs from its actual native readout expression",
                    n.canonical_output
                ));
            }
            let parameter = model["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|p| p["name"] == n.denominator_input);
            if matches!(n.basis, NormalizationBasis::DeclaredReferenceInput) && !parameter {
                return Err(
                    "Declared reference normalization requires a native parameter input".into(),
                );
            }
            let derived = model["individual"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|p| p["name"] == n.denominator_input);
            if matches!(n.basis, NormalizationBasis::ConditionSpecificBinding) && !derived {
                return Err(
                    "Condition-specific normalization requires an explicit native derived binding"
                        .into(),
                );
            }
        }
        Ok(Self {
            file_sha256: descriptor.sha256.clone(),
            manifest,
        })
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub enum ReceiptSchema {
    #[serde(rename = "qspquest.initialization-receipt/v1")]
    V1,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Prepared,
    Executed,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitialState {
    pub name: String,
    pub unit: String,
    pub value: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExactUnit {
    pub dimensions: [i16; 8],
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub entities: std::collections::BTreeMap<Entity, i16>,
    pub scale_numerator: String,
    pub scale_denominator: String,
    pub declared: Option<String>,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Entity {
    Cell,
    Genome,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    Parameter,
    Covariate,
    IndividualParameter,
    DerivedCovariate,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub name: String,
    pub kind: InputKind,
    pub value: f64,
    pub unit: ExactUnit,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    Initial,
    PostBoundary,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingPhase {
    pub time: f64,
    pub activation: Activation,
    pub inputs: Vec<Input>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DenominatorBinding {
    pub time: f64,
    pub activation: Activation,
    pub value: f64,
    pub unit: ExactUnit,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NormalizationReceipt {
    pub declaration: Normalization,
    pub denominator_bindings: Vec<DenominatorBinding>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub start: f64,
    pub end: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "InitializationReceipt")]
pub struct Receipt {
    pub schema: ReceiptSchema,
    pub stage: Stage,
    pub time_unit: String,
    pub integration_start: f64,
    pub integration_end: f64,
    pub first_dose_time: Option<f64>,
    /// An integration interval before the first dose, not an equilibrium/convergence assertion.
    pub predose_integration: Option<Window>,
    /// Native evaluated initial values, before boundary updates and dose events.
    pub initialized_states: Vec<InitialState>,
    /// Actual native input phases, including derived values and post-boundary activation.
    pub binding_phases: Vec<BindingPhase>,
    pub source_declaration: Option<Declaration>,
    pub declared_scenario: Option<String>,
    pub normalization_receipts: Vec<NormalizationReceipt>,
    pub planned_execution_identity: EngineIdentity,
    pub notice: String,
}
pub fn prepare(plan: &RunPlan, metadata: &Value, scenario: Option<&str>) -> Result<Receipt, Value> {
    let phases: Vec<BindingPhase> =
        serde_json::from_value(json!(plan.binding_phases)).map_err(|e| {
            policy::error(
                "initialization_contract",
                format!("Native binding phases differ from the receipt contract: {e}"),
            )
        })?;
    let source_declaration: Option<Declaration> = serde_json::from_value(
        metadata["protocol_initialization_contract"].clone(),
    )
    .map_err(|e| {
        policy::error(
            "initialization_contract",
            format!("Invalid source initialization contract: {e}"),
        )
    })?;
    let mut normalization_receipts = Vec::new();
    if let Some(source) = &source_declaration {
        for n in &source.manifest.normalizations {
            if plan
                .selected_outputs
                .as_ref()
                .is_some_and(|names| !names.contains(&n.canonical_output))
            {
                continue;
            }
            let mut bindings = Vec::new();
            for phase in &phases {
                let input=phase.inputs.iter().find(|input|input.name==n.denominator_input)
                    .ok_or_else(||policy::error("initialization_contract",format!("Normalization '{}' denominator '{}' is absent from a native input phase",n.canonical_output,n.denominator_input)))?;
                if !input.value.is_finite() || input.value <= 0. {
                    return Err(policy::error(
                        "initialization_contract",
                        format!(
                            "Normalization '{}' requires a positive finite '{}' denominator; native phase at {} {} binds {}. No normalized output was computed.",
                            n.canonical_output,
                            n.denominator_input,
                            phase.time,
                            plan.time_unit,
                            input.value
                        ),
                    ));
                }
                bindings.push(DenominatorBinding {
                    time: phase.time,
                    activation: phase.activation.clone(),
                    value: input.value,
                    unit: input.unit.clone(),
                });
            }
            normalization_receipts.push(NormalizationReceipt {
                declaration: n.clone(),
                denominator_bindings: bindings,
            });
        }
    }
    let declared_scenario = scenario
        .filter(|id| {
            source_declaration
                .as_ref()
                .is_some_and(|s| s.manifest.scenarios.contains_key(*id))
        })
        .map(str::to_owned);
    let first_dose_time = plan
        .events
        .iter()
        .filter(|e| !matches!(e.event, qspquest_core::Event::Reset { .. }))
        .map(|e| e.event.time())
        .min_by(f64::total_cmp);
    let predose_integration = first_dose_time
        .filter(|t| *t > plan.start)
        .map(|end| Window {
            start: plan.start,
            end,
        });
    Ok(Receipt {
        schema:ReceiptSchema::V1, stage:Stage::Prepared, time_unit:plan.time_unit.clone(),
        integration_start:plan.start,integration_end:plan.end, first_dose_time,predose_integration,
        initialized_states:plan.states.iter().map(|s|InitialState{name:s.name.clone(),unit:s.unit.clone(),value:s.initial}).collect(),
        binding_phases:phases,source_declaration,declared_scenario,normalization_receipts,
        planned_execution_identity:serde_json::from_value(json!(plan.prospective_identity)).map_err(|e|policy::error("initialization_contract",e.to_string()))?,
        notice:"The clock, initial states and input phases come from native preparation. Initials precede boundary updates/doses. A pre-dose integration interval does not establish equilibrium or convergence. Source declarations describe exact archived scenarios; modified requests follow the actual clock and bindings above. Denominator policies are explicitly declared inputs, never inferred from names or automatically rebaselined from a trajectory. Native units and source labels do not establish physical or biological qualification.".into(),
    })
}
pub fn complete(receipt: &mut Receipt, result: &Value) -> Result<(), Value> {
    if result["identity"] != json!(receipt.planned_execution_identity) {
        return Err(policy::error(
            "initialization_contract",
            "The initialization receipt differs from the actual native execution identity; no receipt was marked executed.",
        ));
    }
    receipt.stage = Stage::Executed;
    Ok(())
}

#[cfg(test)]
pub(crate) fn synthetic_library() -> crate::library::Library {
    let source = "model synthetic_protocol_initialization\ntime_unit = h\nparameters\n synthesis = 2 [mg/h] fixed\n clearance = 1 [1/h] estimated\n reference = 2 [mg] estimated\nindividual\n condition_reference = synthesis / clearance [mg]\nstates\n target = 0 [mg]\n marker = 0 [mg]\ndynamics\n d/dt(target) = synthesis - clearance * target\n d/dt(marker) = input_marker\ndosing\n marker [mg] = 1\noutputs\n fixed_pct = 100 * target / reference [1]\n condition_pct = 100 * target / condition_reference [1]\nend";
    let model = crate::core::compute::execute(source, "inspect_model", json!({})).unwrap();
    let scenario = json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf",
        "budgets":{"solver_callbacks":1000000,"events":100,"output_values":100000},
        "regimen":{"start":{"value":-20,"unit":"h"},"end":{"value":2,"unit":"h"},
        "samples":[],"rtol":1e-10,"atol":1e-12,"administrations":[{"target":"marker",
        "amount":{"value":1,"unit":"mg"},"time":{"value":0,"unit":"h"},
        "delivery":{"kind":"bolus"},"bioavailability":1}],
        "observations":[{"time":{"value":-10,"unit":"h"},"side":"post"},
            {"time":{"value":0,"unit":"h"},"side":"post"},{"time":{"value":2,"unit":"h"},"side":"post"}]}});
    let mut metadata = json!({"id":model["model_id"],"version":"1.0.0","omex_sha256":"a".repeat(64),
        "name":"Synthetic initialization","candidate":false,"execution_available":true,
        "fit_ready":false,"fitting_supported":false,"scenario_ids":["baseline"],"license":"user supplied"});
    let native_hash = hash(model.to_string().as_bytes());
    let request_hash = hash(scenario.to_string().as_bytes());
    let manifest = json!({"schema":"qspquest.protocol-initialization/v1","model_id":model["model_id"],
        "model_version":"1.0.0","archive_sha256":"a".repeat(64),"native_model_sha256":native_hash,
        "scenarios":{"baseline":{"native_request_sha256":request_hash,"integration_start":{"value":-20,"unit":"h"},
        "intervention_time":{"value":0,"unit":"h"},"explicit_initial_override_count":0,
        "interpretation":"Initialize at -20 h and dose at 0 h; convergence is not asserted."}},
        "normalizations":[{"canonical_output":"fixed_pct","numerator_state":"target","denominator_input":"reference",
        "multiplier":100,"basis":"declared_reference_input","interpretation":"Declared reference input; explicit changes remain visible."},
        {"canonical_output":"condition_pct","numerator_state":"target","denominator_input":"condition_reference",
        "multiplier":100,"basis":"condition_specific_binding","interpretation":"Derived synthesis/clearance binding in each input phase."}],
        "scope":"Synthetic native initialization mechanics only."});
    let raw = serde_json::to_vec(&manifest).unwrap();
    let descriptor = Descriptor {
        path: "PROTOCOL_INITIALIZATION.json".into(),
        sha256: hash(&raw),
    };
    let scenarios = BTreeMap::from([("baseline".into(), scenario)]);
    let index = json!({"native_execution":{},"members":{"native/model.json":native_hash,"native/scenarios/baseline.json":request_hash}});
    let declaration = Declaration::load(
        &raw,
        &descriptor,
        &metadata,
        &model,
        &index,
        &scenarios,
        &native_hash,
    )
    .unwrap();
    metadata["protocol_initialization_contract"] = json!(declaration);
    crate::library::Library {
        version: "synthetic_initialization_library".into(),
        records: vec![crate::library::Record {
            metadata,
            model,
            index,
            scenarios,
            archive_hash: "a".repeat(64),
            fitting: json!({"supported":false}),
            source: None,
            precision_profiles: None,
        }],
        availability_counts: Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initialization_declaration_rejects_changed_coordinates_and_unsealed_context() {
        let library = synthetic_library();
        let record = &library.records[0];
        let source = &record.metadata["protocol_initialization_contract"]["manifest"];
        let check = |value: &Value| {
            let raw = serde_json::to_vec(value).unwrap();
            Declaration::load(
                &raw,
                &Descriptor {
                    path: "declaration.json".into(),
                    sha256: hash(&raw),
                },
                &record.metadata,
                &record.model,
                &record.index,
                &record.scenarios,
                record.index["members"]["native/model.json"]
                    .as_str()
                    .unwrap(),
            )
        };
        assert!(check(source).is_ok());
        for case in 0..9 {
            let mut altered = source.clone();
            match case {
                0 => altered["native_model_sha256"] = json!("b".repeat(64)),
                1 => altered["archive_sha256"] = json!("b".repeat(64)),
                2 => {
                    altered["scenarios"]["baseline"]["native_request_sha256"] =
                        json!("b".repeat(64))
                }
                3 => altered["scenarios"]["baseline"]["integration_start"]["value"] = json!(0),
                4 => altered["scenarios"]["baseline"]["explicit_initial_override_count"] = json!(1),
                5 => altered["scenarios"]["invented"] = source["scenarios"]["baseline"].clone(),
                6 => altered["normalizations"][0]["denominator_input"] = json!("clearance"),
                7 => altered["normalizations"][1]["basis"] = json!("declared_reference_input"),
                _ => altered["normalizations"][0]["unknown"] = json!(true),
            }
            assert!(check(&altered).is_err(), "case {case} accepted");
        }
        let raw = serde_json::to_vec(source).unwrap();
        assert!(Declaration::load(
            &raw,
            &Descriptor {
                path: "declaration.json".into(),
                sha256: "b".repeat(64)
            },
            &record.metadata,
            &record.model,
            &record.index,
            &record.scenarios,
            record.index["members"]["native/model.json"]
                .as_str()
                .unwrap()
        )
        .is_err());
    }
    #[test]
    fn executed_initialization_must_match_native_execution_identity() {
        let library = synthetic_library();
        let record = &library.records[0];
        let source = record.source_text();
        let request = record.scenarios["baseline"].clone();
        let plan: RunPlan = serde_json::from_value(
            crate::core::compute::execute(&source, "plan_simulate", request.clone()).unwrap(),
        )
        .unwrap();
        let mut receipt = prepare(&plan, &record.metadata, Some("baseline")).unwrap();
        let mut result = crate::core::compute::execute(&source, "simulate", request).unwrap();
        let original = result["identity"].clone();
        result["identity"]["model_content_hash"] = json!("changed");
        assert!(complete(&mut receipt, &result).is_err());
        assert_eq!(json!(receipt.stage), "prepared");
        result["identity"] = original;
        complete(&mut receipt, &result).unwrap();
        assert_eq!(json!(receipt.stage), "executed");
    }
}

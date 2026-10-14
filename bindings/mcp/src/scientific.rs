//! Hash-bound archive assessments; no declaration promotes a current calculation.
use crate::{library::hash, planning::Assessment};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub path: String,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub enum ManifestSchema {
    #[serde(rename = "qspquest.scientific-assessments/v1")]
    V1,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub enum ReceiptSchema {
    #[serde(rename = "qspquest.mcp-scientific-assessment/v1")]
    V1,
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    NumericalParity,
    SourceFidelity,
    Conservation,
    AuthorCodeParity,
    ClinicalValidation,
    PhysicalDimensions,
}
impl Axis {
    const ALL: [Self; 6] = [
        Self::NumericalParity,
        Self::SourceFidelity,
        Self::Conservation,
        Self::AuthorCodeParity,
        Self::ClinicalValidation,
        Self::PhysicalDimensions,
    ];
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    NotAssessed,
    Passed,
    Failed,
    DiscrepancyObserved,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "ScientificEvidence")]
pub struct Evidence {
    pub artifact_sha256: String,
    pub method: String,
    pub scope: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "ScientificFinding")]
pub struct Finding {
    pub id: String,
    pub description: String,
    pub expression: String,
    pub value: f64,
    pub declared_unit: String,
    pub native_symbols: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "ScientificAssessmentAxis")]
pub struct AxisAssessment {
    pub axis: Axis,
    pub outcome: Outcome,
    pub scope: String,
    pub evidence: Option<Evidence>,
    pub findings: Vec<Finding>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "ScientificAssessmentManifest")]
pub struct Manifest {
    pub schema: ManifestSchema,
    pub model_id: String,
    pub model_version: String,
    pub archive_sha256: String,
    pub native_model_sha256: String,
    pub assessments: Vec<AxisAssessment>,
    pub scope: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "ScientificAssessmentDeclaration")]
pub struct Declaration {
    pub file_sha256: String,
    pub manifest: Manifest,
}
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "ScientificAssessmentReceipt")]
pub struct Receipt {
    pub schema: ReceiptSchema,
    pub declaration: Option<Declaration>,
    pub effective_source_matches_archive: Option<bool>,
    pub request_matches_stored_preset: Option<bool>,
    pub current_engine_and_request: Assessment,
    pub notice: String,
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_text(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control)
}
impl Declaration {
    pub fn load(
        raw: &[u8],
        descriptor: &Descriptor,
        metadata: &Value,
        model: &Value,
        native_hash: &str,
    ) -> Result<Self, String> {
        if raw.len() > 24_000 || !valid_hash(&descriptor.sha256) || hash(raw) != descriptor.sha256 {
            return Err(
                "Scientific assessment file exceeds 24 KB or differs from catalog SHA-256".into(),
            );
        }
        let manifest: Manifest =
            serde_json::from_slice(raw).map_err(|e| format!("Scientific assessments: {e}"))?;
        if manifest.model_id != metadata["id"]
            || manifest.model_version != metadata["version"]
            || manifest.archive_sha256 != metadata["omex_sha256"]
            || manifest.native_model_sha256 != native_hash
            || !valid_hash(native_hash)
            || !valid_hash(&manifest.archive_sha256)
        {
            return Err(
                "Scientific assessment model/version/archive/native identity mismatch".into(),
            );
        }
        let axes: BTreeSet<_> = manifest.assessments.iter().map(|a| a.axis).collect();
        if manifest.assessments.len() != Axis::ALL.len()
            || axes != Axis::ALL.into_iter().collect()
            || !valid_text(&manifest.scope)
        {
            return Err("Scientific assessments must declare each of the six distinct axes exactly once and a bounded scope".into());
        }
        let symbols: BTreeSet<_> = [
            "parameters",
            "covariates",
            "individual",
            "definitions",
            "states",
            "outputs",
        ]
        .iter()
        .flat_map(|key| model[*key].as_array().into_iter().flatten())
        .filter_map(|s| s["name"].as_str())
        .collect();
        for a in &manifest.assessments {
            if !valid_text(&a.scope) || a.findings.len() > 8 {
                return Err(
                    "Scientific assessment lacks a bounded scope or has too many findings".into(),
                );
            }
            if a.outcome == Outcome::NotAssessed {
                if a.evidence.is_some() || !a.findings.is_empty() {
                    return Err(
                        "Unassessed axes must not carry assessment evidence or findings".into(),
                    );
                }
            } else {
                let e = a
                    .evidence
                    .as_ref()
                    .ok_or("Assessed scientific axes require explicitly scoped evidence")?;
                if !valid_hash(&e.artifact_sha256)
                    || !valid_text(&e.method)
                    || !valid_text(&e.scope)
                {
                    return Err(
                        "Scientific assessment evidence needs a SHA-256, method and bounded scope"
                            .into(),
                    );
                }
            }
            let mut ids = BTreeSet::new();
            for f in &a.findings {
                if !ids.insert(&f.id)
                    || !valid_text(&f.id)
                    || !valid_text(&f.description)
                    || !valid_text(&f.expression)
                    || !valid_text(&f.declared_unit)
                    || !f.value.is_finite()
                    || f.native_symbols.is_empty()
                    || f.native_symbols.len() > 64
                    || f.native_symbols
                        .iter()
                        .any(|n| !symbols.contains(n.as_str()))
                    || f.native_symbols.iter().collect::<BTreeSet<_>>().len()
                        != f.native_symbols.len()
                {
                    return Err("Scientific finding has invalid text/value, duplicate identifiers or unknown native symbols".into());
                }
            }
            if matches!(a.outcome, Outcome::Failed | Outcome::DiscrepancyObserved)
                && a.findings.is_empty()
            {
                return Err(
                    "Failed or discrepant scientific axes require explicit findings".into(),
                );
            }
        }
        Ok(Self {
            file_sha256: descriptor.sha256.clone(),
            manifest,
        })
    }
}
impl Receipt {
    pub fn new(metadata: &Value, source_matches: bool, request_matches: Option<bool>) -> Self {
        // This derived field is removed from untrusted catalog input and populated
        // only by the library's file-and-model-bound declaration validator.
        let declaration = metadata
            .get("scientific_assessment_contract")
            .and_then(|v| serde_json::from_value::<Declaration>(v.clone()).ok());
        Self { schema:ReceiptSchema::V1,effective_source_matches_archive:declaration.as_ref().map(|_|source_matches),request_matches_stored_preset:request_matches,declaration,current_engine_and_request:Assessment::NotAssessed,
            notice:"Archive assessments are scoped declarations, not authentication or qualification of this engine and effective request. Numerical agreement does not establish conservation, source/author-code fidelity, physical dimensions or clinical validation. Inspect every axis and finding; changed inputs or source require fresh applicable evidence.".into() }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (Value, Value, Value) {
        let assessments=Axis::ALL.map(|axis|json!({"axis":axis,"outcome":"not_assessed","scope":"No assessment evidence declared.","evidence":null,"findings":[]}));
        (
            json!({"schema":"qspquest.scientific-assessments/v1","model_id":"example","model_version":"1","archive_sha256":"a".repeat(64),"native_model_sha256":"b".repeat(64),"assessments":assessments,"scope":"Synthetic declaration validation only."}),
            json!({"id":"example","version":"1","omex_sha256":"a".repeat(64)}),
            json!({"parameters":[{"name":"flow"}]}),
        )
    }
    fn load(v: &Value, m: &Value, model: &Value) -> Result<Declaration, String> {
        let raw = serde_json::to_vec(v).unwrap();
        Declaration::load(
            &raw,
            &Descriptor {
                path: "assessment.json".into(),
                sha256: hash(&raw),
            },
            m,
            model,
            &"b".repeat(64),
        )
    }
    #[test]
    fn axes_are_independent_and_bound_declarations_never_qualify_a_run() {
        let (mut v, mut m, model) = fixture();
        assert!(load(&v, &m, &model).is_ok());
        v["assessments"][0]["outcome"] = json!("passed");
        v["assessments"][0]["evidence"] = json!({"artifact_sha256":"c".repeat(64),"method":"Analytical parity","scope":"One synthetic value only."});
        v["assessments"][2]["outcome"] = json!("discrepancy_observed");
        v["assessments"][2]["evidence"] = v["assessments"][0]["evidence"].clone();
        v["assessments"][2]["findings"] = json!([{"id":"imbalance","description":"Declared transport imbalance.","expression":"flow","value":-1.,"declared_unit":"L/h","native_symbols":["flow"]}]);
        let d = load(&v, &m, &model).unwrap();
        m["scientific_assessment_contract"] = json!(d);
        let receipt = json!(Receipt::new(&m, false, Some(false)));
        assert_eq!(receipt["current_engine_and_request"], "not_assessed");
        assert_eq!(
            receipt["declaration"]["manifest"]["assessments"][4]["outcome"],
            "not_assessed"
        );
        assert_eq!(receipt["effective_source_matches_archive"], false);
        for (name, mutant) in [
            ("general validated", {
                let mut x = v.clone();
                x["assessments"][0]["outcome"] = json!("validated");
                x
            }),
            ("missing evidence", {
                let mut x = v.clone();
                x["assessments"][0]["evidence"] = Value::Null;
                x
            }),
            ("wrong symbol", {
                let mut x = v.clone();
                x["assessments"][2]["findings"][0]["native_symbols"] = json!(["missing"]);
                x
            }),
            ("duplicate axis", {
                let mut x = v.clone();
                x["assessments"][5] = x["assessments"][0].clone();
                x
            }),
            ("unknown field", {
                let mut x = v.clone();
                x["validated"] = json!(true);
                x
            }),
        ] {
            assert!(load(&mutant, &m, &model).is_err(), "{name}");
        }
        let mut stale = m.clone();
        stale["omex_sha256"] = json!("d".repeat(64));
        assert!(load(&v, &stale, &model).is_err());
        let mut unsupported = v.clone();
        unsupported["assessments"][2]["findings"] = json!([]);
        assert!(load(&unsupported, &m, &model).is_err());
    }
}

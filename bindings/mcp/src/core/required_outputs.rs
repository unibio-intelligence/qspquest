//! Explicit output requirements bind requested scientific labels to computed columns.
use crate::contract::OutputMapping;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequiredOutput {
    pub output: String,
    /// Exact analyte identifier declared by the model's output metadata; names are not inferred.
    pub analyte: Option<String>,
    /// Exact compartment identifier declared by the model's output metadata.
    pub compartment: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    SelectedForComputation,
    Computed,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct RequiredOutputReceipt {
    pub requirement: RequiredOutput,
    pub canonical_output: String,
    pub returned_column: String,
    pub returned_unit: String,
    pub declared_analyte: Option<String>,
    pub declared_compartment: Option<String>,
    pub status: RequirementStatus,
    pub sample_count: Option<usize>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    UnknownOutput,
    NotSelected,
    AnalyteUndeclared,
    AnalyteMismatch,
    CompartmentUndeclared,
    CompartmentMismatch,
    IncompleteColumn,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct OutputRequirementFailure {
    pub index: usize,
    pub requirement: RequiredOutput,
    pub canonical_output: Option<String>,
    pub declared_analyte: Option<String>,
    pub declared_compartment: Option<String>,
    pub reason: RefusalReason,
    /// Always false: a refused requirement is never represented by fabricated numerical data.
    pub fulfilled: bool,
}
fn refused(
    index: usize,
    requirement: &RequiredOutput,
    mapping: Option<&OutputMapping>,
    reason: RefusalReason,
    message: String,
) -> Value {
    let diagnostic = OutputRequirementFailure {
        index,
        requirement: requirement.clone(),
        canonical_output: mapping.map(|m| m.canonical_output.clone()),
        declared_analyte: mapping.and_then(|m| m.analyte.clone()),
        declared_compartment: mapping.and_then(|m| m.compartment.clone()),
        reason,
        fulfilled: false,
    };
    json!({"error":{"code":"required_output","message":message,"expression":format!("arguments.required_outputs[{index}]"),
        "output_requirement":diagnostic,"suggestion":"Inspect get_model.interface.output_contract and select outputs that satisfy the declared requirement. No missing output is filled with a number."}})
}
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
}
/// Read-only admission; requirements never silently extend an explicit output selection.
pub fn prepare(
    model: &Value,
    native: &Value,
    requirements: &Value,
    selected: Option<&[String]>,
) -> Result<Vec<RequiredOutputReceipt>, Value> {
    if requirements.is_null() {
        return Ok(Vec::new());
    }
    let required: Vec<RequiredOutput> =
        serde_json::from_value(requirements.clone()).map_err(|e| {
            crate::policy::error(
                "required_output",
                format!("required_outputs must be an array of output requirements: {e}"),
            )
        })?;
    if required.len() > 128
        || required.iter().any(|r| {
            !valid_name(&r.output)
                || r.analyte.as_ref().is_some_and(|n| !valid_name(n))
                || r.compartment.as_ref().is_some_and(|n| !valid_name(n))
        })
    {
        return Err(crate::policy::error("required_output","Use at most 128 requirements, with nonempty identifiers of at most 256 UTF-8 bytes and no control characters"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut receipts = Vec::new();
    for (i, r) in required.into_iter().enumerate() {
        if !seen.insert(r.output.clone()) {
            return Err(crate::policy::error(
                "required_output",
                format!("Duplicate required output '{}'", r.output),
            ));
        }
        let mut resolved = super::usability::output_mapping(model, native, &[r.output.clone()])
            .map_err(|e| refused(i, &r, None, RefusalReason::UnknownOutput, e))?;
        let m = resolved.remove(0);
        if selected.is_some_and(|names| !names.contains(&m.canonical_output)) {
            return Err(refused(i,&r,Some(&m),RefusalReason::NotSelected,format!("Required output '{}' resolves to '{}', which is absent from the explicit output selection. Include it in outputs; the request was not changed.",r.output,m.canonical_output)));
        }
        for (expected, declared, is_analyte) in [
            (&r.analyte, &m.analyte, true),
            (&r.compartment, &m.compartment, false),
        ] {
            if let Some(expected) = expected {
                let label = if is_analyte { "analyte" } else { "compartment" };
                let reason = match (is_analyte, declared.is_none()) {
                    (true, true) => RefusalReason::AnalyteUndeclared,
                    (true, false) => RefusalReason::AnalyteMismatch,
                    (false, true) => RefusalReason::CompartmentUndeclared,
                    (false, false) => RefusalReason::CompartmentMismatch,
                };
                if declared.as_ref() != Some(expected) {
                    let actual = declared.as_deref().unwrap_or("undeclared");
                    return Err(refused(i,&r,Some(&m),reason,format!("Required {label} '{expected}' does not match output '{}' declaration '{actual}'. Supply a model with the required declared {label}; scientific labels are not inferred from output names.",r.output)));
                }
            }
        }
        receipts.push(RequiredOutputReceipt {
            requirement: r,
            canonical_output: m.canonical_output,
            returned_column: m.returned_column,
            returned_unit: m.returned_unit,
            declared_analyte: m.analyte,
            declared_compartment: m.compartment,
            status: RequirementStatus::SelectedForComputation,
            sample_count: None,
        });
    }
    Ok(receipts)
}
/// Validate the complete numerical result before retention, CSV export or preview clipping.
pub fn complete(receipts: &mut [RequiredOutputReceipt], result: &Value) -> Result<(), Value> {
    let times = result["times"].as_array();
    for (i, r) in receipts.iter_mut().enumerate() {
        let matches: Vec<_> = result["outputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|o| o["name"] == r.returned_column && o["unit"] == r.returned_unit)
            .collect();
        let valid = times.is_some()
            && matches.len() == 1
            && matches[0]["values"].as_array().is_some_and(|values| {
                values.len() == times.unwrap().len()
                    && values
                        .iter()
                        .all(|v| v.as_f64().is_some_and(f64::is_finite))
            });
        if !valid {
            return Err(refused(i,&r.requirement,None,RefusalReason::IncompleteColumn,format!("Required output '{}' did not return one complete finite '{}' column in '{}' on the requested grid.",r.requirement.output,r.returned_column,r.returned_unit)));
        }
        r.status = RequirementStatus::Computed;
        r.sample_count = times.map(Vec::len);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Value {
        json!({"outputs":[{"name":"vector","unit":"1"},{"name":"antibody","unit":"mg/L"}]})
    }
    fn native() -> Value {
        json!({"outputs":{"aav":{"native_name":"vector","unit":"1","analyte":"aav-vector","compartment":"brain"},"mab":{"native_name":"antibody","unit":"mg/L","analyte":"antibody","compartment":"plasma"}}})
    }
    #[test]
    fn missing_and_mismatched_analytes_are_refused_without_numeric_substitution() {
        for (requirement, reason) in [
            (json!({"output":"mab"}), "not_selected"),
            (
                json!({"output":"aav","analyte":"antibody"}),
                "analyte_mismatch",
            ),
            (json!({"output":"missing"}), "unknown_output"),
            (
                json!({"output":"aav","compartment":"plasma"}),
                "compartment_mismatch",
            ),
        ] {
            let e = prepare(
                &model(),
                &native(),
                &json!([requirement]),
                Some(&["vector".into()]),
            )
            .unwrap_err();
            assert_eq!(e["error"]["output_requirement"]["reason"], reason);
            assert_eq!(e["error"]["output_requirement"]["fulfilled"], false);
            assert!(e["outputs"].is_null());
        }
        let e = prepare(
            &model(),
            &json!({}),
            &json!([{"output":"antibody","analyte":"antibody"}]),
            None,
        )
        .unwrap_err();
        assert_eq!(
            e["error"]["output_requirement"]["reason"],
            "analyte_undeclared"
        );
    }
    #[test]
    fn actual_zero_is_computed_but_missing_null_and_wrong_grid_columns_are_not() {
        let original = prepare(
            &model(),
            &native(),
            &json!([{"output":"mab","analyte":"antibody","compartment":"plasma"}]),
            Some(&["antibody".into()]),
        )
        .unwrap();
        let valid =
            json!({"times":[0,1],"outputs":[{"name":"antibody","unit":"mg/L","values":[0,0]}]});
        let mut receipts = original.clone();
        complete(&mut receipts, &valid).unwrap();
        assert_eq!(receipts[0].status, RequirementStatus::Computed);
        assert_eq!(receipts[0].sample_count, Some(2));
        for bad in [
            json!({"times":[0,1],"outputs":[]}),
            json!({"times":[0,1],"outputs":[{"name":"antibody","unit":"mg/L","values":[0,null]}]}),
            json!({"times":[0,1],"outputs":[{"name":"antibody","unit":"mg/L","values":[0]}]}),
        ] {
            assert_eq!(
                complete(&mut original.clone(), &bad).unwrap_err()["error"]["output_requirement"]
                    ["reason"],
                "incomplete_column"
            );
        }
    }
}

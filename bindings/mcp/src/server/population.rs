//! Paired population regimen comparisons through the native analysis core.
//!
//! Members are prepared exactly as deterministic comparison members, so doses,
//! shared and member overrides, units and scenarios keep their meaning. Their
//! effective native run requests then become the labelled regimens of one
//! native `compare_regimens` analysis, which gives the same virtual subjects
//! every regimen.
use super::*;

/// Deterministic comparison fields that a population comparison does not use.
const DETERMINISTIC_ONLY: [&str; 4] = [
    "save_csv",
    "required_outputs",
    "readouts",
    "parameter_changes",
];

fn refuse(message: impl Into<String>) -> Value {
    json!({"error":{"code":"population_comparison","message":message.into(),
        "suggestion":"A population comparison reports paired exposure percentiles for one output. Keep shared and member doses, parameters and initial states; vary subject parameters only through population.parameters and omega."}})
}

pub(super) type Member = (
    String,
    PreparedInvocation,
    String,
    crate::planning::ComparisonInputReceipt,
);

/// Refuse inputs whose meaning a population comparison would change, before preparation.
pub(super) fn admit_inputs(args: &Value, population: &Value) -> Result<(), Value> {
    for field in DETERMINISTIC_ONLY {
        if args.get(field).is_some_and(|v| !v.is_null()) {
            return Err(refuse(format!(
                "{field} applies to the deterministic comparison; remove it or remove population"
            )));
        }
    }
    if !args["output"].is_string() {
        return Err(refuse(
            "A population comparison needs output, the one output whose exposure is compared",
        ));
    }
    let names: Vec<&str> = population["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["name"].as_str())
        .collect();
    let mut overrides = vec![("parameters".to_owned(), &args["parameters"])];
    for (index, row) in args["regimens"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if row.get("parameter_changes").is_some_and(|v| !v.is_null()) {
            return Err(refuse(format!(
                "regimens[{index}].parameter_changes applies to the deterministic comparison; a population comparison uses one model source for every regimen"
            )));
        }
        overrides.push((format!("regimens[{index}].parameters"), &row["parameters"]));
    }
    for (field, values) in overrides {
        if let Some(name) = values
            .as_object()
            .into_iter()
            .flat_map(|map| map.keys())
            .find(|name| names.contains(&name.as_str()))
        {
            return Err(refuse(format!(
                "{field}.{name} also varies between virtual subjects; remove the override so the subject value is not replaced"
            )));
        }
    }
    Ok(())
}

/// Build the admitted native `compare_regimens` envelope from prepared members.
pub(super) fn envelope(
    population: &Value,
    members: &[Member],
    output: &str,
    hosted: bool,
) -> Result<Value, Value> {
    let first = &members[0].1;
    if members
        .iter()
        .any(|(_, m, _, _)| m.source != first.source || m.trusted != first.trusted)
    {
        return Err(refuse(
            "Every regimen of a population comparison must use the same effective model source",
        ));
    }
    if hosted && first.model_json["states"].as_array().map_or(0, Vec::len) >= 64 {
        return Err(policy::error(
            "async_required",
            "Population comparisons of models with 64 or more states run only through local stdio.",
        ));
    }
    let declared: Vec<&str> = first.model_json["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["name"].as_str())
        .collect();
    let mut parameters = Vec::new();
    for parameter in population["parameters"].as_array().into_iter().flatten() {
        let name = parameter["name"].as_str().unwrap_or("");
        if !declared.contains(&name) {
            return Err(refuse(format!(
                "population parameter {name} is not a native model parameter; use a name from get_model detail full: {}",
                declared.join(", ")
            )));
        }
        parameters.push(parameter.clone());
    }
    let entry = &first.native["outputs"][output];
    let output = entry
        .as_str()
        .or_else(|| entry["native_name"].as_str())
        .unwrap_or(output);
    let regimens: Vec<Value> = members
        .iter()
        .map(|(name, member, _, _)| {
            let mut run = member.compute_request.clone();
            if let Some(object) = run.as_object_mut() {
                object.remove("_selected_outputs");
            }
            json!({"label": name, "run": run})
        })
        .collect();
    let mut arguments = serde_json::Map::new();
    arguments.insert("regimens".into(), json!(regimens));
    arguments.insert("parameters".into(), json!(parameters));
    arguments.insert("output".into(), json!(output));
    for (key, value) in population.as_object().into_iter().flatten() {
        if key != "parameters" && !value.is_null() {
            arguments.insert(key.clone(), value.clone());
        }
    }
    let mut envelope = json!({"schema":"qspquest.analysis/v0.2","operation":"compare_regimens","arguments":arguments});
    crate::execution_policy::admit_analysis(
        &mut envelope,
        crate::execution_policy::Profile::from_hosted(hosted),
    )?;
    Ok(envelope)
}

impl Server {
    pub(super) async fn execute_population_comparison(
        &self,
        members: Vec<Member>,
        envelope: Value,
        plan_hash: String,
        effective_request_hash: String,
        library_version: String,
    ) -> Result<Value, Value> {
        let first = &members[0].1;
        let record = self
            .compute_versioned(
                first.source.clone(),
                "analyze",
                envelope,
                first.trusted,
                &first.library_version,
            )
            .await?;
        let scientific_assessment = self.invocation_qualification(first).scientific_assessment;
        let receipts: Vec<Value> = members
            .iter()
            .map(|(name, _, hash, receipt)| {
                json!({"name":name,"member_plan_hash":hash,"input_receipt":receipt})
            })
            .collect();
        let summary = comparison_brief(&record);
        let text = usability::ordered(&[("comparison_summary", &summary)]);
        let mut result = json!({
            "comparison_summary":summary,
            "population":{"members":receipts,"analysis":record},
            "identity":record["identity"],
            "scientific_assessment":scientific_assessment,
            "plan_hash":plan_hash,
            "effective_request_hash":effective_request_hash,
            "library_version":library_version,
            "_text":text,
        });
        let retained = self.persist_result("compare_regimens", &mut result)?;
        if !retained && !self.hosted && result.to_string().len() > policy::RESPONSE_LIMIT / 2 {
            let id = format!("population-{}", hash(plan_hash.as_bytes()));
            if self.cache.put(id.clone(), result.clone()) {
                result["full_result_resource"] = json!(format!("qspquest://runs/{id}"));
            }
        }
        Ok(result)
    }
}

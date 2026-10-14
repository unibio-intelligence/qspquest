//! Structural consumption of explicitly supplied inputs, before numerical execution.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Consumption {
    Dynamic,
    InitializationOnly,
    StaticOutputOnly,
    Unused,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct Consumers {
    /// Equations through algebraic definitions, without traversing other state equations.
    pub state_equations: Vec<String>,
    pub initial_states: Vec<String>,
    /// Initial expressions replaced by explicit request.initial_states quantities.
    pub replaced_initial_states: Vec<String>,
    pub dosing_states: Vec<String>,
    pub switches: Vec<String>,
    /// Outputs through algebraic definitions and potentially through state evolution.
    pub outputs: Vec<String>,
    /// Outputs that combine this input with state, time or covariate expressions.
    pub dynamic_output_expressions: Vec<String>,
    pub definitions: Vec<String>,
    pub linear_pk: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct EffectiveValue {
    pub request_path: String,
    #[schemars(with = "crate::contract::JsonValueSchema")]
    pub quantity: Value,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct OverrideEntry {
    pub requested_parameter: String,
    pub canonical_parameter: String,
    pub request_path: String,
    #[schemars(with = "crate::contract::JsonValueSchema")]
    pub supplied_value: Value,
    pub effective_values: Vec<EffectiveValue>,
    pub consumption: Consumption,
    pub consumers: Consumers,
    pub warning: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct OverrideReceipt {
    pub entries: Vec<OverrideEntry>,
    pub scope: String,
    pub source_reconstruction: String,
    pub literal_initial_states_retained: usize,
}

impl OverrideReceipt {
    pub fn summary(&self) -> Value {
        serde_json::json!({"entry_count": self.entries.len(),
            "warnings": self.entries.iter().filter_map(|entry| entry.warning.as_ref()).collect::<Vec<_>>(),
            "hint": "Inspect override_receipt in structuredContent or the complete result resource for native consumers and effective values."})
    }
}

type Graph = BTreeMap<String, BTreeSet<String>>;
fn symbols(value: &Value) -> BTreeSet<String> {
    fn walk(value: &Value, found: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                if value["kind"] == "symbol" {
                    if let Some(name) = value["name"].as_str() {
                        found.insert(name.into());
                    }
                }
                for inner in map.values() {
                    walk(inner, found);
                }
            }
            Value::Array(values) => {
                for inner in values {
                    walk(inner, found);
                }
            }
            _ => {}
        }
    }
    let mut found = BTreeSet::new();
    walk(value, &mut found);
    found
}
fn closure(mut found: BTreeSet<String>, graph: &Graph) -> BTreeSet<String> {
    let mut pending: Vec<_> = found.iter().cloned().collect();
    while let Some(name) = pending.pop() {
        for child in graph.get(&name).into_iter().flatten() {
            if found.insert(child.clone()) {
                pending.push(child.clone());
            }
        }
    }
    found
}
fn canonical(name: &str, native: &Value) -> String {
    native["parameters"][name]
        .as_str()
        .or_else(|| native["parameters"][name]["native_name"].as_str())
        .unwrap_or(name)
        .into()
}
fn quantities(value: &Value, path: &str, found: &mut Vec<(String, String, Value)>) {
    match value {
        Value::Object(map) => {
            for key in ["parameters", "covariates"] {
                for (name, quantity) in value[key].as_object().into_iter().flatten() {
                    found.push((
                        format!("{path}.{key}.{name}"),
                        name.clone(),
                        quantity.clone(),
                    ));
                }
            }
            for (key, inner) in map {
                if ["parameters", "covariates", "model"].contains(&key.as_str()) {
                    continue;
                }
                if key == "covariate_changes" {
                    for (i, change) in inner.as_array().into_iter().flatten().enumerate() {
                        if let Some(name) = change["name"].as_str() {
                            found.push((
                                format!("{path}.{key}[{i}].value"),
                                name.into(),
                                change["value"].clone(),
                            ));
                        }
                    }
                } else {
                    quantities(inner, &format!("{path}.{key}"), found);
                }
            }
        }
        Value::Array(array) => {
            for (i, inner) in array.iter().enumerate() {
                quantities(inner, &format!("{path}[{i}]"), found);
            }
        }
        _ => {}
    }
}

/// Receipts describe actual native expressions. Resolved source values are never reconstructed
/// from names, descriptions or source-unit labels. No request or scientific values are modified.
pub fn receipt(
    model: &Value,
    native: &Value,
    original: &Value,
    request: &Value,
) -> OverrideReceipt {
    let mut algebra = Graph::new();
    for key in ["definitions", "individual", "switches", "dose_switches"] {
        for entry in model[key].as_array().into_iter().flatten() {
            if let Some(name) = entry["name"].as_str() {
                algebra.insert(name.into(), symbols(entry));
            }
        }
    }
    let run = request.get("run").unwrap_or(request);
    let mut flow = algebra.clone();
    for state in model["states"].as_array().into_iter().flatten() {
        if let Some(name) = state["name"].as_str() {
            let mut dependencies = symbols(&state["rhs"]);
            if !run["initial_states"][name].is_object() {
                dependencies.extend(symbols(&state["initial"]));
            }
            dependencies.extend(symbols(&state["dosing"]));
            flow.insert(name.into(), dependencies);
        }
    }
    let pk = &model["linear_pk"];
    let coefficients: BTreeSet<String> = pk["clearance"]
        .as_str()
        .into_iter()
        .chain(
            pk["exchange_clearances"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        )
        .chain(
            pk["compartments"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["volume"].as_str()),
        )
        .map(str::to_owned)
        .collect();
    for compartment in pk["compartments"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(pk.get("depot"))
    {
        if let Some(name) = compartment["state"].as_str() {
            let dependencies = flow.entry(name.into()).or_default();
            dependencies.extend(coefficients.iter().cloned());
            if let Some(absorption) = pk["depot"]["absorption"].as_str() {
                dependencies.insert(absorption.into());
            }
            if !run["initial_states"][name].is_object() {
                dependencies.extend(symbols(&compartment["initial"]));
            }
            dependencies.extend(symbols(&compartment["dosing_override"]));
        }
    }
    let dynamic_symbols: BTreeSet<String> = model["states"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(model["covariates"].as_array().into_iter().flatten())
        .filter_map(|s| s["name"].as_str().map(str::to_owned))
        .chain(["time".into()])
        .collect();
    let mut supplied = Vec::new();
    quantities(original, "arguments", &mut supplied);
    // A scan requests a parameter at every point, in addition to any baseline overrides.
    if let Some(name) = original["parameter"]
        .as_str()
        .filter(|_| request["points"].is_array())
    {
        supplied.push((
            "arguments.parameter".into(),
            name.into(),
            original
                .get("values")
                .or_else(|| original.get("range"))
                .cloned()
                .unwrap_or(Value::Null),
        ));
    }
    for (i, change) in original["parameter_changes"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let Some(name) = change["name"]
            .as_str()
            .or_else(|| change["parameter"].as_str())
        {
            supplied.push((
                format!("arguments.parameter_changes[{i}]"),
                name.into(),
                change.clone(),
            ));
        }
    }
    let mut applied = Vec::new();
    quantities(request, "effective_request", &mut applied);
    let literal_initial_states_retained = model["states"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| {
            s["initial"]["kind"] == "quantity"
                && !run["initial_states"][s["name"].as_str().unwrap_or("")].is_object()
        })
        .count();
    if supplied.is_empty() {
        return complete_receipt(Vec::new(), literal_initial_states_retained);
    }
    let state_dependencies: Vec<_> = model["states"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|state| {
            (
                state,
                closure(symbols(&state["rhs"]), &algebra),
                closure(symbols(&state["initial"]), &algebra),
                closure(symbols(&state["dosing"]), &algebra),
            )
        })
        .collect();
    let algebra_dependencies: Vec<_> = algebra
        .iter()
        .map(|(name, dependencies)| (name, closure(dependencies.clone(), &algebra)))
        .collect();
    let switch_dependencies: Vec<_> = ["switches", "dose_switches"]
        .into_iter()
        .flat_map(|key| model[key].as_array().into_iter().flatten())
        .map(|switch| (switch, closure(symbols(switch), &algebra)))
        .collect();
    let output_dependencies: Vec<_> = model["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|output| {
            (
                output,
                closure(symbols(&output["expression"]), &flow),
                closure(symbols(&output["expression"]), &algebra),
            )
        })
        .collect();
    let pk_dependencies: Vec<_> = pk["compartments"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(pk.get("depot"))
        .map(|compartment| {
            (
                compartment,
                closure(symbols(&compartment["initial"]), &algebra),
                closure(symbols(&compartment["dosing_override"]), &algebra),
            )
        })
        .collect();
    let entries = supplied.into_iter().map(|(path, requested, value)| {
        let name = canonical(&requested, native);
        let mut consumers = Consumers::default();
        for (state, rhs, initial, dosing) in &state_dependencies {
            let Some(state_name) = state["name"].as_str() else { continue; };
            if rhs.contains(&name) { consumers.state_equations.push(state_name.into()); }
            if initial.contains(&name) {
                if run["initial_states"][state_name].is_object() { consumers.replaced_initial_states.push(state_name.into()); }
                else { consumers.initial_states.push(state_name.into()); }
            }
            if dosing.contains(&name) { consumers.dosing_states.push(state_name.into()); }
        }
        for (definition, dependencies) in &algebra_dependencies {
            if dependencies.contains(&name) { consumers.definitions.push((*definition).clone()); }
        }
        for (switch, dependencies) in &switch_dependencies {
            if dependencies.contains(&name) {
                if let Some(n) = switch["name"].as_str() { consumers.switches.push(n.into()); }
            }
        }
        for (output, dependencies, direct) in &output_dependencies {
            if dependencies.contains(&name) {
                if let Some(n) = output["name"].as_str() {
                    consumers.outputs.push(n.into());
                    if direct.contains(&name) && !direct.is_disjoint(&dynamic_symbols) { consumers.dynamic_output_expressions.push(n.into()); }
                }
            }
        }
        // Linear PK coefficients are declared as strings, rather than expression symbols.
        let pk = &model["linear_pk"];
        if pk["clearance"] == name { consumers.linear_pk.push("clearance".into()); }
        for (i, v) in pk["exchange_clearances"].as_array().into_iter().flatten().enumerate() {
            if *v == name { consumers.linear_pk.push(format!("exchange_clearances[{i}]")); }
        }
        for (i, c) in pk["compartments"].as_array().into_iter().flatten().enumerate() {
            if c["volume"] == name { consumers.linear_pk.push(format!("compartments[{i}].volume")); }
        }
        if pk["depot"]["absorption"] == name { consumers.linear_pk.push("depot.absorption".into()); }
        for (compartment, initial, dosing) in &pk_dependencies {
            let Some(state) = compartment["state"].as_str() else { continue; };
            if initial.contains(&name) {
                let target = if run["initial_states"][state].is_object() { &mut consumers.replaced_initial_states } else { &mut consumers.initial_states };
                if !target.iter().any(|n| n == state) { target.push(state.into()); }
            }
            if dosing.contains(&name) && !consumers.dosing_states.iter().any(|n| n == state) {
                consumers.dosing_states.push(state.into());
            }
        }
        let consumption = if !consumers.state_equations.is_empty() || !consumers.dosing_states.is_empty() || !consumers.switches.is_empty() || !consumers.linear_pk.is_empty() || !consumers.dynamic_output_expressions.is_empty() {
            Consumption::Dynamic
        } else if !consumers.initial_states.is_empty() { Consumption::InitializationOnly }
        else if !consumers.outputs.is_empty() { Consumption::StaticOutputOnly }
        else { Consumption::Unused };
        let warning = match consumption {
            Consumption::Unused => Some(format!("{requested} has no native equation, output, dosing or retained initial-expression consumer. This override cannot change the simulated trajectory. Resolved production coefficients and literal initial states are retained; source reconstruction is unsupported.")),
            Consumption::StaticOutputOnly => Some(format!("{requested} only changes stored scalar outputs. It does not change state dynamics or retained initial conditions. Resolved production coefficients and literal initial states are retained; source reconstruction is unsupported.")),
            Consumption::InitializationOnly => Some(format!("{requested} is consumed only by retained initial expressions; no state equation or dosing expression uses it.")),
            Consumption::Dynamic => None,
        };
        let mut effective_values: Vec<_> = applied.iter().filter(|(_, n, _)| n == &name)
            .map(|(path, _, q)| EffectiveValue { request_path: path.clone(), quantity: q.clone() }).collect();
        if run["covariates"][&name].is_null() && run["parameters"][&name].is_null() {
            if let Some((i, covariate)) = model["covariates"].as_array().into_iter().flatten().enumerate().find(|(_, c)| c["name"] == name && c["default"].is_object()) {
                effective_values.insert(0, EffectiveValue { request_path: format!("effective_source.covariates[{i}].default"), quantity: covariate["default"].clone() });
            }
        }
        OverrideEntry { requested_parameter: requested, canonical_parameter: name, request_path: path, supplied_value: value, effective_values, consumption, consumers, warning }
    }).collect();
    complete_receipt(entries, literal_initial_states_retained)
}

fn complete_receipt(
    entries: Vec<OverrideEntry>,
    literal_initial_states_retained: usize,
) -> OverrideReceipt {
    OverrideReceipt { entries, literal_initial_states_retained,
        scope: "Structural native dependencies, including all branches and potential state propagation. References do not establish a numerical effect, identifiability, physical units or scientific qualification. Scenario defaults are excluded unless explicitly overridden; effective values list baseline and scan/step values for each requested name.".into(),
        source_reconstruction: "unsupported; only native initial expressions and algebraic definitions are evaluated. Resolved literals are retained unless explicitly overridden.".into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn symbol(name: &str) -> Value {
        json!({"kind":"symbol","name":name})
    }
    #[test]
    fn consumers_distinguish_native_reinitialization_from_resolved_scalars() {
        let model = json!({"states":[
            {"name":"x","rhs":symbol("rate"),"initial":{"kind":"expression","expression":symbol("start")},"dosing":{"scale":symbol("dose_scale")}},
            {"name":"y","rhs":symbol("x"),"initial":{"kind":"quantity","quantity":{"value":4,"unit":"1"}}}
        ],"definitions":[{"name":"rate","expression":symbol("k")}],"outputs":[
            {"name":"readout","expression":symbol("y")},
            {"name":"stored_source_value","expression":symbol("anchor")},
            {"name":"effect","expression":{"kind":"binary","arguments":[symbol("effect_scale"),symbol("x")]}}
        ],"switches":[{"name":"phase","time":symbol("switch_time")}]});
        let original = json!({"parameters":{"k":{"value":2,"unit":"1"},"start":{"value":3,"unit":"1"},"anchor":{"value":5,"unit":"1"},"resolved_scaling":{"value":9,"unit":"1"},"dose_scale":{"value":6,"unit":"1"},"switch_time":{"value":7,"unit":"1"},"effect_scale":{"value":8,"unit":"1"}}});
        let receipt = receipt(&model, &json!({}), &original, &original);
        let entry = |name: &str| {
            receipt
                .entries
                .iter()
                .find(|e| e.canonical_parameter == name)
                .unwrap()
        };
        assert!(matches!(entry("k").consumption, Consumption::Dynamic));
        assert_eq!(entry("k").consumers.state_equations, ["x"]);
        assert!(entry("k").consumers.outputs.contains(&"readout".into()));
        assert_eq!(entry("k").consumers.definitions, ["rate"]);
        assert!(matches!(
            entry("start").consumption,
            Consumption::InitializationOnly
        ));
        assert_eq!(entry("start").consumers.initial_states, ["x"]);
        assert!(matches!(
            entry("anchor").consumption,
            Consumption::StaticOutputOnly
        ));
        assert!(entry("anchor")
            .warning
            .as_ref()
            .unwrap()
            .contains("Resolved production coefficients"));
        assert!(matches!(
            entry("resolved_scaling").consumption,
            Consumption::Unused
        ));
        assert!(entry("resolved_scaling")
            .warning
            .as_ref()
            .unwrap()
            .contains("cannot change"));
        assert_eq!(entry("dose_scale").consumers.dosing_states, ["x"]);
        assert_eq!(entry("switch_time").consumers.switches, ["phase"]);
        assert!(matches!(
            entry("effect_scale").consumption,
            Consumption::Dynamic
        ));
        assert_eq!(receipt.literal_initial_states_retained, 1);
        let mut replaced = original.clone();
        replaced["initial_states"] = json!({"x":{"value":0,"unit":"1"}});
        let replaced = super::receipt(&model, &json!({}), &original, &replaced);
        let start = replaced
            .entries
            .iter()
            .find(|e| e.canonical_parameter == "start")
            .unwrap();
        assert!(matches!(start.consumption, Consumption::Unused));
        assert_eq!(start.consumers.replaced_initial_states, ["x"]);
        assert!(start.consumers.initial_states.is_empty());
    }
    #[test]
    fn scan_and_alias_receipts_retain_exact_normalized_quantities_without_mutation() {
        let model = json!({"linear_pk":{"clearance":"CL","compartments":[{"state":"amount","volume":"V"}]},"states":[{"name":"amount","rhs":{"kind":"literal","value":0}}],"outputs":[{"name":"concentration","expression":symbol("amount")}]});
        let native = json!({"parameters":{"clearance":{"native_name":"CL"}}});
        let original = json!({"parameter":"clearance","values":[{"value":100,"unit":"mL/h"},{"value":200,"unit":"mL/h"}],"request":{"parameters":{"V":{"value":3,"unit":"L"}}}});
        let request = json!({"run":{"parameters":{"V":{"value":3,"unit":"L"}}},"points":[{"parameters":{"CL":{"value":0.1,"unit":"L/h"}}},{"parameters":{"CL":{"value":0.2,"unit":"L/h"}}}]});
        let before = request.clone();
        let receipt = receipt(&model, &native, &original, &request);
        assert_eq!(request, before);
        let cl = receipt
            .entries
            .iter()
            .find(|e| e.requested_parameter == "clearance")
            .unwrap();
        assert_eq!(cl.canonical_parameter, "CL");
        assert_eq!(cl.effective_values.len(), 2);
        assert_eq!(
            cl.effective_values[1].quantity,
            json!({"value":0.2,"unit":"L/h"})
        );
        assert_eq!(cl.consumers.linear_pk, ["clearance"]);
        assert_eq!(cl.consumers.outputs, ["concentration"]);
        assert!(matches!(cl.consumption, Consumption::Dynamic));
    }
    #[test]
    fn nested_requests_and_time_changes_record_the_actual_starting_binding() {
        let model = json!({"states":[{"name":"x","rhs":symbol("rate")}],"covariates":[{"name":"rate","default":{"value":0.1,"unit":"1/h"}}]});
        let original = json!({"request":{"problem":{"run":{"parameters":{"rate":{"value":0.1,"unit":"1/h"}}}}},"parameter_changes":[{"name":"rate","at":{"value":1,"unit":"h"},"value":{"value":0.2,"unit":"1/h"}}]});
        let effective = json!({"regimen":{"covariate_changes":[{"name":"rate","time":{"value":1,"unit":"h"},"value":{"value":0.2,"unit":"1/h"}}]}});
        let result = receipt(&model, &json!({}), &original, &effective);
        assert_eq!(result.entries.len(), 2);
        for entry in &result.entries {
            assert_eq!(entry.effective_values.len(), 2);
            assert!(entry.effective_values[0]
                .request_path
                .starts_with("effective_source."));
            assert_eq!(
                entry.effective_values[0].quantity,
                json!({"value":0.1,"unit":"1/h"})
            );
            assert_eq!(
                entry.effective_values[1].quantity,
                json!({"value":0.2,"unit":"1/h"})
            );
            assert!(matches!(entry.consumption, Consumption::Dynamic));
        }
    }
    #[test]
    fn every_explicit_override_is_receipted_even_when_many_are_unused() {
        let parameters: serde_json::Map<_, _> = (0..70)
            .map(|i| (format!("p{i}"), json!({"value":i,"unit":"1"})))
            .collect();
        let request = json!({"parameters":parameters});
        let result = receipt(&json!({}), &json!({}), &request, &request);
        assert_eq!(result.entries.len(), 70);
        assert!(result
            .entries
            .iter()
            .all(|e| e.warning.is_some() && e.effective_values.len() == 1));
    }
}

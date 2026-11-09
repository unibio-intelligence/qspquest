//! Structured, validated edits to a model document.
//!
//! Agents extend models by mechanism (a peripheral compartment, a covariate effect,
//! a dosing input) rather than by rewriting whole files. Each operation edits the
//! native model document; the result is validated by the ordinary compiler, so an
//! edit can never produce a model that `validate_model` would refuse.
use qspquest_modeling::language;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const MAX_OPERATIONS: usize = 64;
const MAX_EXPRESSION: usize = 4096;

/// Symbol namespaces declared by the native model document.
pub(crate) const SYMBOL_SECTIONS: [&str; 8] = [
    "parameters",
    "covariates",
    "individual",
    "definitions",
    "states",
    "switches",
    "dose_switches",
    "dose_history",
];
pub(crate) const MAX_IDENTIFIER: usize = 128;

/// The legal identifier closest to `name`: ASCII lowercase snake_case, at most
/// 128 bytes, not reserved, not starting with a digit.
pub fn legal_identifier(name: &str) -> String {
    let mut out = String::new();
    let mut previous_underscore = false;
    let mut previous_lower = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() && previous_lower && !previous_underscore {
            // camelCase boundary: aB -> a_b
            out.push('_');
        }
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            previous_underscore = false;
            previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        } else if !previous_underscore && !out.is_empty() {
            out.push('_');
            previous_underscore = true;
            previous_lower = false;
        }
    }
    let mut out = out.trim_end_matches('_').to_owned();
    if out.is_empty() {
        out = "symbol".into();
    }
    if out.as_bytes()[0].is_ascii_digit() {
        out = format!("p_{out}");
    }
    if matches!(out.as_str(), "time" | "true" | "false") || out.starts_with("input_") {
        out = format!("{out}_value");
    }
    out.truncate(MAX_IDENTIFIER);
    out
}

fn rename_expressions(value: &mut Value, symbols: &BTreeMap<String, String>) {
    match value {
        Value::Object(object) => {
            if object.get("kind").and_then(Value::as_str) == Some("symbol") {
                if let Some(Value::String(name)) = object.get_mut("name") {
                    if let Some(new) = symbols.get(name.as_str()) {
                        *name = new.clone();
                    } else if let Some(state) = name.strip_prefix("input_") {
                        if let Some(new) = symbols.get(state) {
                            *name = format!("input_{new}");
                        }
                    }
                }
                return;
            }
            for child in object.values_mut() {
                rename_expressions(child, symbols);
            }
        }
        Value::Array(items) => {
            for child in items {
                rename_expressions(child, symbols);
            }
        }
        _ => {}
    }
}

pub(crate) fn rename_string(value: &mut Value, symbols: &BTreeMap<String, String>) {
    if let Value::String(name) = value {
        if let Some(new) = symbols.get(name.as_str()) {
            *name = new.clone();
        }
    }
}

/// Rename symbols (shared namespace) and output labels everywhere they occur in a model
/// document: declarations, expressions, `input_<state>` references and the string fields
/// that name states or parameters (invariants, linear PK, dose switches and history).
pub fn apply_renames(
    copy: &mut Value,
    symbols: &BTreeMap<String, String>,
    outputs: &BTreeMap<String, String>,
) {
    for section in SYMBOL_SECTIONS {
        for declaration in copy
            .get_mut(section)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(name) = declaration.get_mut("name") {
                rename_string(name, symbols);
            }
        }
    }
    for output in copy
        .get_mut("outputs")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if let Some(name) = output.get_mut("name") {
            rename_string(name, outputs);
        }
    }
    for section in ["dose_switches", "dose_history"] {
        for declaration in copy
            .get_mut(section)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(target) = declaration.get_mut("target") {
                rename_string(target, symbols);
            }
        }
    }
    for invariant in copy
        .get_mut("invariants")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        for state in invariant
            .get_mut("states")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            rename_string(state, symbols);
        }
    }
    if let Some(pk) = copy.get_mut("linear_pk").filter(|pk| pk.is_object()) {
        if let Some(v) = pk.get_mut("clearance") {
            rename_string(v, symbols);
        }
        for name in pk
            .get_mut("exchange_clearances")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            rename_string(name, symbols);
        }
        for compartment in pk
            .get_mut("compartments")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            for key in ["state", "volume"] {
                if let Some(v) = compartment.get_mut(key) {
                    rename_string(v, symbols);
                }
            }
        }
        if let Some(depot) = pk.get_mut("depot").filter(|d| d.is_object()) {
            for key in ["state", "absorption"] {
                if let Some(v) = depot.get_mut(key) {
                    rename_string(v, symbols);
                }
            }
        }
    }
    rename_expressions(copy, symbols);
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
}
impl Quantity {
    fn json(&self) -> Value {
        json!({"value":self.value,"unit":self.unit})
    }
}

/// One edit. Expressions are .qsp expression text, for example `q / vc * (central - tissue)`.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    AddParameter {
        name: String,
        value: Quantity,
        description: String,
        transform: Option<String>,
        fixed: Option<bool>,
    },
    SetParameter {
        name: String,
        value: Option<Quantity>,
        description: Option<String>,
        fixed: Option<bool>,
    },
    AddState {
        name: String,
        unit: String,
        initial: Option<Quantity>,
        initial_expression: Option<String>,
    },
    SetInitial {
        state: String,
        initial: Option<Quantity>,
        initial_expression: Option<String>,
    },
    AddTerm {
        state: String,
        expression: String,
    },
    AddFlux {
        from: Option<String>,
        to: Option<String>,
        rate: String,
    },
    AddPeripheralCompartment {
        central: String,
        name: String,
        coordinates: String,
        central_volume: String,
        volume: Quantity,
        clearance: Quantity,
        volume_parameter: Option<String>,
        clearance_parameter: Option<String>,
        description: Option<String>,
    },
    AddCovariate {
        name: String,
        unit: String,
        default: Quantity,
        interpolation: Option<String>,
    },
    AddCovariateEffect {
        parameter: String,
        covariate: String,
        reference: Quantity,
        form: String,
        coefficient: f64,
        name: Option<String>,
        description: Option<String>,
    },
    AddDosing {
        state: String,
        amount_unit: String,
        scale: Option<String>,
        modes: Option<Vec<String>>,
        lag: Option<String>,
    },
    SetDosing {
        state: String,
        amount_unit: Option<String>,
        scale: Option<String>,
        modes: Option<Vec<String>>,
        /// Lag expression; an empty string removes the lag.
        lag: Option<String>,
    },
    SetRhs {
        state: String,
        expression: String,
    },
    AddOutput {
        name: String,
        unit: String,
        expression: String,
    },
    RenameSymbol {
        from: String,
        to: String,
    },
}

fn identifier(name: &str, field: &str) -> Result<(), String> {
    let legal = !name.is_empty()
        && !matches!(name, "true" | "false" | "time")
        && !name.starts_with("input_")
        && name.len() <= 128
        && name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_lowercase() || (i > 0 && b.is_ascii_digit()));
    if legal {
        Ok(())
    } else {
        Err(format!(
            "{field} '{name}' is not a legal identifier; use lowercase ASCII snake_case such as '{}'",
            legal_identifier(name)
        ))
    }
}

/// Parse one expression through the model language, without resolving its symbols.
pub fn expression(text: &str, field: &str) -> Result<Value, String> {
    if text.trim().is_empty() || text.len() > MAX_EXPRESSION || text.contains(['\n', '\r', '#']) {
        return Err(format!(
            "{field} must be one nonempty .qsp expression of at most {MAX_EXPRESSION} bytes without newlines or comments"
        ));
    }
    let source = format!(
        "model edit_expression\ntime_unit = h\ndefinitions\n  edit_expression_value = {text}\nend\n"
    );
    let parsed = language::parse(&source)
        .map_err(|e| format!("{field}: {} (expression text: {text})", e.message))?;
    let document = serde_json::to_value(parsed.document).map_err(|e| e.to_string())?;
    Ok(document["definitions"][0]["expression"].clone())
}

fn symbol(name: &str) -> Value {
    json!({"kind":"symbol","name":name})
}
fn binary(operator: &str, a: Value, b: Value) -> Value {
    json!({"kind":"binary","operator":operator,"arguments":[a,b]})
}

struct Editor {
    model: Value,
    summary: Vec<Value>,
    added: BTreeMap<&'static str, Vec<String>>,
    changed_equations: Vec<String>,
}

impl Editor {
    fn names(&self) -> Vec<String> {
        [
            "parameters",
            "covariates",
            "individual",
            "definitions",
            "states",
            "switches",
            "dose_switches",
            "dose_history",
        ]
        .iter()
        .flat_map(|s| self.model[*s].as_array().into_iter().flatten())
        .filter_map(|d| d["name"].as_str().map(str::to_owned))
        .collect()
    }
    fn unused(&self, name: &str, field: &str) -> Result<(), String> {
        identifier(name, field)?;
        if self.names().iter().any(|n| n == name) {
            return Err(format!(
                "{field} '{name}' is already declared; choose another name"
            ));
        }
        Ok(())
    }
    /// Outputs whose expressions reference no state, definition, individual parameter or time.
    fn constant_outputs(&self) -> Vec<String> {
        fn symbols(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Object(o) => {
                    if o.get("kind").and_then(Value::as_str) == Some("symbol") {
                        if let Some(n) = o.get("name").and_then(Value::as_str) {
                            out.push(n.to_owned());
                        }
                    }
                    o.values().for_each(|c| symbols(c, out));
                }
                Value::Array(a) => a.iter().for_each(|c| symbols(c, out)),
                _ => {}
            }
        }
        let dynamic: Vec<String> = ["states", "definitions", "individual"]
            .iter()
            .flat_map(|s| self.model[*s].as_array().into_iter().flatten())
            .filter_map(|d| d["name"].as_str().map(str::to_owned))
            .chain(std::iter::once("time".to_owned()))
            .collect();
        self.model["outputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|o| {
                let mut used = Vec::new();
                symbols(&o["expression"], &mut used);
                !used.is_empty() && !used.iter().any(|u| dynamic.contains(u))
            })
            .filter_map(|o| o["name"].as_str().map(str::to_owned))
            .collect()
    }
    fn section_mut(&mut self, section: &str) -> &mut Vec<Value> {
        if !self.model[section].is_array() {
            self.model[section] = json!([]);
        }
        self.model[section].as_array_mut().unwrap()
    }
    fn state_mut(&mut self, name: &str) -> Result<&mut Value, String> {
        self.model
            .get_mut("states")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .find(|s| s["name"] == name)
            .ok_or_else(|| format!("state '{name}' is not declared"))
    }
    fn state_unit(&self, name: &str) -> Result<String, String> {
        self.model["states"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s["name"] == name)
            .and_then(|s| s["unit"].as_str().map(str::to_owned))
            .ok_or_else(|| format!("state '{name}' is not declared"))
    }
    fn add_to_rhs(&mut self, state: &str, operator: &str, term: Value) -> Result<(), String> {
        let s = self.state_mut(state)?;
        let rhs = s["rhs"].take();
        // A state added with a zero rate takes its first term directly.
        let zero = rhs["kind"] == "quantity" && rhs["quantity"]["value"].as_f64() == Some(0.0);
        s["rhs"] = match (zero, operator) {
            (true, "add") => term,
            (true, _) => json!({"kind":"unary","operator":"negative","arguments":[term]}),
            _ => binary(operator, rhs, term),
        };
        if !self.changed_equations.iter().any(|n| n == state) {
            self.changed_equations.push(state.to_owned());
        }
        Ok(())
    }
    fn parameter(
        &mut self,
        name: &str,
        value: &Quantity,
        description: &str,
        transform: &str,
        fixed: bool,
    ) -> Result<(), String> {
        self.unused(name, "parameter")?;
        if !matches!(transform, "identity" | "log" | "logit" | "probit") {
            return Err(format!(
                "transform '{transform}' is not identity, log, logit or probit"
            ));
        }
        if transform == "log" && value.value <= 0.0 {
            return Err(format!(
                "parameter '{name}' uses a log transform and needs a positive value"
            ));
        }
        let description = description.trim();
        if description.is_empty() {
            return Err(format!("parameter '{name}' needs a description stating its source or that it is an assumption"));
        }
        self.section_mut("parameters")
            .push(json!({"name":name,"default":value.json(),"bounds":null,
            "transform":transform,"fixed":fixed,"description":description}));
        self.added
            .entry("parameters")
            .or_default()
            .push(name.to_owned());
        Ok(())
    }
    /// Replace symbol uses of `from` with `to` everywhere except inside `skip` (an individual name).
    fn substitute(&mut self, from: &str, to: &str, skip: &str) -> usize {
        fn walk(v: &mut Value, from: &str, to: &str, count: &mut usize) {
            match v {
                Value::Object(o) => {
                    if o.get("kind").and_then(Value::as_str) == Some("symbol")
                        && o.get("name").and_then(Value::as_str) == Some(from)
                    {
                        o.insert("name".into(), json!(to));
                        *count += 1;
                        return;
                    }
                    for child in o.values_mut() {
                        walk(child, from, to, count);
                    }
                }
                Value::Array(a) => a.iter_mut().for_each(|c| walk(c, from, to, count)),
                _ => {}
            }
        }
        let mut count = 0;
        let mut changed = Vec::new();
        for state in self
            .model
            .get_mut("states")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            let before = count;
            walk(&mut state["rhs"], from, to, &mut count);
            if count > before {
                changed.push(state["name"].as_str().unwrap_or("").to_owned());
            }
            for key in ["initial", "dosing"] {
                if let Some(v) = state.get_mut(key) {
                    walk(v, from, to, &mut count);
                }
            }
        }
        for name in changed {
            if !self.changed_equations.contains(&name) {
                self.changed_equations.push(name);
            }
        }
        for section in [
            "outputs",
            "definitions",
            "switches",
            "dose_switches",
            "linear_pk",
        ] {
            if let Some(v) = self.model.get_mut(section) {
                walk(v, from, to, &mut count);
            }
        }
        for entry in self
            .model
            .get_mut("individual")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if entry["name"] != skip {
                walk(&mut entry["expression"], from, to, &mut count);
            }
        }
        count
    }

    fn apply(&mut self, op: Operation) -> Result<Value, String> {
        match op {
            Operation::AddParameter {
                name,
                value,
                description,
                transform,
                fixed,
            } => {
                let fraction = matches!(value.unit.as_str(), "1" | "%")
                    && value.value > 0.0
                    && value.value < if value.unit == "%" { 100.0 } else { 1.0 }
                    && (name.starts_with("f_")
                        || name == "f"
                        || name.contains("fraction")
                        || name.contains("bioavail"));
                let default_transform = if fraction { "logit" } else { "log" };
                self.parameter(
                    &name,
                    &value,
                    &description,
                    transform.as_deref().unwrap_or(default_transform),
                    fixed.unwrap_or(true),
                )?;
                Ok(json!(format!(
                    "added parameter {name} = {} {}",
                    value.value, value.unit
                )))
            }
            Operation::SetParameter {
                name,
                value,
                description,
                fixed,
            } => {
                let p = self
                    .model
                    .get_mut("parameters")
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
                    .find(|p| p["name"] == name.as_str())
                    .ok_or_else(|| format!("parameter '{name}' is not declared"))?;
                if let Some(value) = value {
                    p["default"] = value.json();
                }
                if let Some(d) = description {
                    p["description"] = json!(d);
                }
                if let Some(f) = fixed {
                    p["fixed"] = json!(f);
                }
                Ok(json!(format!("changed parameter {name}")))
            }
            Operation::AddState {
                name,
                unit,
                initial,
                initial_expression,
            } => {
                self.unused(&name, "state")?;
                let initial = match (initial, initial_expression) {
                    (Some(q), None) => json!({"kind":"quantity","quantity":q.json()}),
                    (None, Some(text)) => {
                        json!({"kind":"expression","expression":expression(&text, "initial_expression")?})
                    }
                    (None, None) => json!({"kind":"quantity","quantity":{"value":0.0,"unit":unit}}),
                    _ => return Err("give initial or initial_expression, not both".into()),
                };
                let rate_unit = format!(
                    "({unit})/{}",
                    self.model["time_unit"].as_str().unwrap_or("h")
                );
                self.section_mut("states").push(json!({"name":name,"unit":unit,"initial":initial,
                    "rhs":{"kind":"quantity","quantity":{"value":0.0,"unit":rate_unit}},"dosing":null}));
                self.added.entry("states").or_default().push(name.clone());
                Ok(json!(format!(
                    "added state {name} [{unit}] with zero rate; add terms, fluxes or dosing next"
                )))
            }
            Operation::SetInitial {
                state,
                initial,
                initial_expression,
            } => {
                let initial = match (initial, initial_expression) {
                    (Some(q), None) => json!({"kind":"quantity","quantity":q.json()}),
                    (None, Some(text)) => {
                        json!({"kind":"expression","expression":expression(&text, "initial_expression")?})
                    }
                    _ => return Err("give exactly one of initial or initial_expression".into()),
                };
                let s = self.state_mut(&state)?;
                let previous = s["initial"].clone();
                s["initial"] = initial;
                let preloaded = previous["kind"] == "quantity"
                    && previous["quantity"]["value"]
                        .as_f64()
                        .is_some_and(|v| v != 0.0);
                let constant = if preloaded {
                    self.constant_outputs()
                } else {
                    Vec::new()
                };
                let note = if constant.is_empty() {
                    String::new()
                } else {
                    format!("; outputs that do not depend on any state or time and may encode the old starting value: {} (they report parameters, not the simulated state)", constant.join(", "))
                };
                Ok(json!(format!("changed the initial value of {state}{note}")))
            }
            Operation::AddTerm {
                state,
                expression: text,
            } => {
                let term = expression(&text, "expression")?;
                self.add_to_rhs(&state, "add", term)?;
                Ok(json!(format!("d/dt({state}) += {text}")))
            }
            Operation::AddFlux { from, to, rate } => {
                if from.is_none() && to.is_none() {
                    return Err("add_flux needs from, to or both".into());
                }
                if let (Some(a), Some(b)) = (&from, &to) {
                    let (ua, ub) = (self.state_unit(a)?, self.state_unit(b)?);
                    if ua != ub {
                        return Err(format!("states {a} [{ua}] and {b} [{ub}] have different units; a flux moves the same quantity, so use add_term on each state with its own scaling"));
                    }
                }
                let term = expression(&rate, "rate")?;
                if let Some(a) = &from {
                    self.add_to_rhs(a, "subtract", term.clone())?;
                }
                if let Some(b) = &to {
                    self.add_to_rhs(b, "add", term)?;
                }
                Ok(json!(format!(
                    "flux {} -> {}: {rate}",
                    from.as_deref().unwrap_or("(source)"),
                    to.as_deref().unwrap_or("(sink)")
                )))
            }
            Operation::AddPeripheralCompartment {
                central,
                name,
                coordinates,
                central_volume,
                volume,
                clearance,
                volume_parameter,
                clearance_parameter,
                description,
            } => {
                let unit = self.state_unit(&central)?;
                let vp = volume_parameter.unwrap_or_else(|| format!("{name}_volume"));
                let q = clearance_parameter.unwrap_or_else(|| format!("{name}_clearance"));
                let note = description.unwrap_or_else(|| {
                    "ADDED by edit_model; value is an assumption unless a source is cited here"
                        .into()
                });
                self.parameter(
                    &vp,
                    &volume,
                    &format!("Peripheral volume of {name}. {note}"),
                    "log",
                    true,
                )?;
                self.parameter(
                    &q,
                    &clearance,
                    &format!("Intercompartmental clearance between {central} and {name}. {note}"),
                    "log",
                    true,
                )?;
                self.unused(&name, "state")?;
                let vc = expression(&central_volume, "central_volume")?;
                let (to_central, to_peripheral) = match coordinates.as_str() {
                    // Amounts: net transfer q * (Cc - Cp) leaves central and enters the periphery.
                    "amount" => {
                        let flux = binary("multiply", symbol(&q), binary("subtract", binary("divide", symbol(&central), vc.clone()), binary("divide", symbol(&name), symbol(&vp))));
                        (flux.clone(), flux)
                    }
                    // Concentrations: each side divides the same transfer by its own volume.
                    "concentration" => {
                        let difference = binary("subtract", symbol(&central), symbol(&name));
                        (binary("multiply", binary("divide", symbol(&q), vc.clone()), difference.clone()),
                         binary("multiply", binary("divide", symbol(&q), symbol(&vp)), difference))
                    }
                    other => return Err(format!("coordinates '{other}' must be amount or concentration (the unit kind of state {central})")),
                };
                self.section_mut("states").push(json!({"name":name,"unit":unit,"initial":{"kind":"quantity","quantity":{"value":0.0,"unit":unit}},"rhs":to_peripheral,"dosing":null}));
                self.added.entry("states").or_default().push(name.clone());
                self.add_to_rhs(&central, "subtract", to_central)?;
                Ok(json!(format!("added peripheral compartment {name} [{unit}] exchanging with {central} ({coordinates} coordinates; parameters {q}, {vp})")))
            }
            Operation::AddCovariate {
                name,
                unit,
                default,
                interpolation,
            } => {
                self.unused(&name, "covariate")?;
                let interpolation = interpolation.unwrap_or_else(|| "constant".into());
                if !matches!(interpolation.as_str(), "constant" | "step") {
                    return Err("interpolation must be constant or step".into());
                }
                self.section_mut("covariates").push(json!({"name":name,"unit":unit,"interpolation":interpolation,"default":default.json()}));
                self.added
                    .entry("covariates")
                    .or_default()
                    .push(name.clone());
                Ok(json!(format!(
                    "added covariate {name} [{unit}], default {} {}",
                    default.value, default.unit
                )))
            }
            Operation::AddCovariateEffect {
                parameter,
                covariate,
                reference,
                form,
                coefficient,
                name,
                description,
            } => {
                if !self.model["parameters"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|p| p["name"] == parameter.as_str())
                {
                    return Err(format!("parameter '{parameter}' is not declared"));
                }
                if !self.model["covariates"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|c| c["name"] == covariate.as_str())
                {
                    return Err(format!(
                        "covariate '{covariate}' is not declared; add it first with add_covariate"
                    ));
                }
                if !coefficient.is_finite() {
                    return Err("coefficient must be finite".into());
                }
                let individual = name.unwrap_or_else(|| format!("{parameter}_i"));
                let coefficient_name = format!(
                    "{parameter}_{covariate}_{}",
                    if form == "power" { "exponent" } else { "slope" }
                );
                let note = description.unwrap_or_else(|| "ADDED by edit_model; coefficient is an assumption unless a source is cited here".into());
                self.parameter(
                    &coefficient_name,
                    &Quantity {
                        value: coefficient,
                        unit: "1".into(),
                    },
                    &format!("Effect of {covariate} on {parameter} ({form}). {note}"),
                    "identity",
                    true,
                )?;
                self.unused(&individual, "individual parameter")?;
                let ratio = binary(
                    "divide",
                    symbol(&covariate),
                    json!({"kind":"quantity","quantity":reference.json()}),
                );
                let factor = match form.as_str() {
                    "power" => binary("power", ratio, symbol(&coefficient_name)),
                    "linear" => binary(
                        "add",
                        json!({"kind":"literal","value":1.0}),
                        binary(
                            "multiply",
                            symbol(&coefficient_name),
                            binary("subtract", ratio, json!({"kind":"literal","value":1.0})),
                        ),
                    ),
                    other => return Err(format!("form '{other}' must be power or linear")),
                };
                let unit = self.model["parameters"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|p| p["name"] == parameter.as_str())
                    .map(|p| p["default"]["unit"].clone())
                    .unwrap_or(Value::Null);
                self.section_mut("individual").push(json!({"name":individual,"expression":binary("multiply", symbol(&parameter), factor),"unit":unit}));
                self.added
                    .entry("individual")
                    .or_default()
                    .push(individual.clone());
                let uses = self.substitute(&parameter, &individual, &individual);
                Ok(json!(format!("{individual} = {parameter} * f({covariate}/{} {}) [{form}, coefficient {coefficient_name}]; {uses} uses of {parameter} now read {individual}", reference.value, reference.unit)))
            }
            Operation::AddDosing {
                state,
                amount_unit,
                scale,
                modes,
                lag,
            } => {
                let scale_text = scale.unwrap_or_else(|| "1".into());
                let scale = expression(&scale_text, "scale")?;
                let lag_expression = match lag.as_deref() {
                    Some(text) => expression(text, "lag")?,
                    None => Value::Null,
                };
                let modes = modes.unwrap_or_else(|| vec!["bolus".into(), "infusion".into()]);
                if modes.is_empty()
                    || modes
                        .iter()
                        .any(|m| !matches!(m.as_str(), "bolus" | "infusion"))
                {
                    return Err("modes must list bolus and/or infusion".into());
                }
                let input = format!("input_{state}");
                let s = self.state_mut(&state)?;
                if !s["dosing"].is_null() {
                    return Err(format!("state '{state}' already declares dosing"));
                }
                // A source that encoded its dose as a starting value keeps that value; doses would
                // then add to an already dosed state, so say so.
                let preloaded = s["initial"]["kind"] == "quantity"
                    && s["initial"]["quantity"]["value"]
                        .as_f64()
                        .is_some_and(|v| v != 0.0);
                let initial_note = if preloaded {
                    format!(
                        "; WARNING: {state} starts at {} {}, which may encode a source dose; set it to zero with set_initial if doses should start from an empty state",
                        s["initial"]["quantity"]["value"], s["initial"]["quantity"]["unit"].as_str().unwrap_or("")
                    )
                } else {
                    String::new()
                };
                s["dosing"] = json!({"amount_unit":amount_unit,"scale":scale,"modes":modes,"lag":lag_expression});
                let mentions_input = s["rhs"].to_string().contains(&format!("\"{input}\""));
                if !mentions_input {
                    let rhs = s["rhs"].take();
                    s["rhs"] = binary("add", symbol(&input), rhs);
                }
                Ok(json!(format!(
                    "{state} accepts doses in {amount_unit} scaled by {scale_text}{}{initial_note}",
                    if mentions_input {
                        ""
                    } else {
                        "; input term added to its equation"
                    }
                )))
            }
            Operation::SetDosing {
                state,
                amount_unit,
                scale,
                modes,
                lag,
            } => {
                let scale = scale
                    .as_deref()
                    .map(|text| expression(text, "scale"))
                    .transpose()?;
                let lag = match lag.as_deref() {
                    Some("") => Some(Value::Null),
                    Some(text) => Some(expression(text, "lag")?),
                    None => None,
                };
                if let Some(modes) = &modes {
                    if modes.is_empty()
                        || modes
                            .iter()
                            .any(|m| !matches!(m.as_str(), "bolus" | "infusion"))
                    {
                        return Err("modes must list bolus and/or infusion".into());
                    }
                }
                let s = self.state_mut(&state)?;
                if s["dosing"].is_null() {
                    return Err(format!(
                        "state '{state}' declares no dosing; use add_dosing"
                    ));
                }
                let mut changed = Vec::new();
                if let Some(unit) = amount_unit {
                    s["dosing"]["amount_unit"] = json!(unit);
                    changed.push("amount_unit");
                }
                if let Some(scale) = scale {
                    s["dosing"]["scale"] = scale;
                    changed.push("scale");
                }
                if let Some(modes) = modes {
                    s["dosing"]["modes"] = json!(modes);
                    changed.push("modes");
                }
                if let Some(lag) = lag {
                    s["dosing"]["lag"] = lag;
                    changed.push("lag");
                }
                if changed.is_empty() {
                    return Err(
                        "set_dosing needs at least one of amount_unit, scale, modes or lag".into(),
                    );
                }
                Ok(json!(format!(
                    "changed dosing of {state}: {}",
                    changed.join(", ")
                )))
            }
            Operation::SetRhs {
                state,
                expression: text,
            } => {
                let mut rhs = expression(&text, "expression")?;
                let s = self.state_mut(&state)?;
                let input = format!("input_{state}");
                let note = if !s["dosing"].is_null()
                    && !rhs.to_string().contains(&format!("\"{input}\""))
                {
                    rhs = binary("add", symbol(&input), rhs);
                    "; input term kept because the state accepts doses"
                } else {
                    ""
                };
                s["rhs"] = rhs;
                if !self.changed_equations.contains(&state) {
                    self.changed_equations.push(state.clone());
                }
                Ok(json!(format!("d/dt({state}) = {text}{note}")))
            }
            Operation::AddOutput {
                name,
                unit,
                expression: text,
            } => {
                identifier(&name, "output")?;
                if self.model["outputs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|o| o["name"] == name.as_str())
                {
                    return Err(format!("output '{name}' already exists"));
                }
                let e = expression(&text, "expression")?;
                self.section_mut("outputs")
                    .push(json!({"name":name,"unit":unit,"expression":e}));
                self.added.entry("outputs").or_default().push(name.clone());
                Ok(json!(format!("added output {name} = {text} [{unit}]")))
            }
            Operation::RenameSymbol { from, to } => {
                let is_output = self.model["outputs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|o| o["name"] == from.as_str());
                let is_symbol = self.names().iter().any(|n| n == &from);
                if !is_symbol && !is_output {
                    return Err(format!("'{from}' is not a declared symbol or output"));
                }
                identifier(&to, "new name")?;
                let mut symbols = BTreeMap::new();
                let mut outputs = BTreeMap::new();
                if is_symbol {
                    self.unused(&to, "new name")?;
                    symbols.insert(from.clone(), to.clone());
                }
                if is_output {
                    outputs.insert(from.clone(), to.clone());
                }
                apply_renames(&mut self.model, &symbols, &outputs);
                Ok(json!(format!("renamed {from} to {to}")))
            }
        }
    }
}

/// Plausibility warnings for a valid model: patterns that compile but commonly encode a
/// transcription error. They never block validation.
pub fn lints(model: &Value) -> Vec<String> {
    fn literal(v: &Value) -> Option<f64> {
        match v["kind"].as_str()? {
            "literal" => v["value"].as_f64(),
            _ => None,
        }
    }
    let mut warnings = Vec::new();
    for (section, label) in [
        ("outputs", "output"),
        ("definitions", "definition"),
        ("individual", "individual parameter"),
    ] {
        for item in model[section].as_array().into_iter().flatten() {
            let unit = item["unit"].as_str().unwrap_or("");
            let e = &item["expression"];
            let times_hundred = e["kind"] == "binary"
                && e["operator"] == "multiply"
                && e["arguments"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| literal(x) == Some(100.0)));
            if unit == "%" && times_hundred {
                warnings.push(format!(
                    "{label} '{}' is declared in [%] and its expression multiplies by 100. The unit already scales by 100, so values will be 100 times too large; drop the factor 100 or declare the unit [1].",
                    item["name"].as_str().unwrap_or("")
                ));
            }
        }
    }
    for p in model["parameters"].as_array().into_iter().flatten() {
        let unit = p["default"]["unit"].as_str().unwrap_or("");
        let value = p["default"]["value"].as_f64().unwrap_or(0.0);
        if p["transform"] == "log" && matches!(unit, "1" | "%") && (0.0..=1.0).contains(&value) {
            let name = p["name"].as_str().unwrap_or("");
            if ["f", "f_oral", "bioavailability", "fraction"]
                .iter()
                .any(|k| {
                    name == *k
                        || name.starts_with("f_")
                        || name.contains("fraction")
                        || name.contains("bioavail")
                })
            {
                warnings.push(format!("parameter '{name}' looks like a fraction but uses a log transform, which does not bound it above 1; consider transform logit."));
            }
        }
    }
    warnings
}

/// Run the whole edit in the same admitted execution environment as compilation.
pub(super) fn execute(source: &str, request: &Value) -> Result<Value, Value> {
    let operations = request["operations"].as_array().ok_or_else(|| {
        json!({"code":"operations","message":"operations must be an array","expression":"/operations"})
    })?;
    let base = super::execute(source, "validate_model", json!({})).map_err(|error| {
        if error["code"] == "input_limit" {
            error
        } else {
            json!({"code":"source","message":"The model to edit does not validate; correct it first (see detail).","expression":"/source","detail":error})
        }
    })?;
    let (edited, changes) =
        apply(base["model"].clone(), operations).map_err(|mut error| error["error"].take())?;
    let valid = super::execute(&edited.to_string(), "validate_model", json!({})).map_err(|error| {
        json!({"code":"edit_validation","message":"The edited model does not compile; the operations are well formed but the resulting equations, units or references are inconsistent (see detail).","expression":"/operations","detail":error,
            "suggestion":"Check units of added terms against d/dt(state) [state unit per time], and that every referenced name is declared."})
    })?;
    Ok(json!({
        "canonical_source": valid["canonical_source"],
        "model_content_hash": valid["model_content_hash"],
        "base_model_content_hash": base["model_content_hash"],
        "valid": true,
        "changes": changes,
        "warnings": lints(&valid["model"]),
        "hint": "Check the change list and canonical_source, then inspect with get_model {source} and simulate with model {source}. Added values are assumptions unless their descriptions cite a source."
    }))
}

/// Apply operations in order to a validated model document and return the edited document,
/// a per-operation summary and the semantic changes.
pub fn apply(model: Value, operations: &[Value]) -> Result<(Value, Value), Value> {
    if operations.is_empty() || operations.len() > MAX_OPERATIONS {
        return Err(
            json!({"error":{"code":"operations","message":format!("operations must contain 1..{MAX_OPERATIONS} edits"),"expression":"/operations"}}),
        );
    }
    let mut editor = Editor {
        model,
        summary: Vec::new(),
        added: BTreeMap::new(),
        changed_equations: Vec::new(),
    };
    for (index, raw) in operations.iter().enumerate() {
        let op: Operation = serde_json::from_value(raw.clone()).map_err(|e| {
            json!({"error":{"code":"operation","message":format!("operations[{index}]: {e}"),"expression":format!("/operations/{index}"),
                "suggestion":"Use one of add_parameter, set_parameter, add_state, set_initial, set_rhs, set_dosing, add_term, add_flux, add_peripheral_compartment, add_covariate, add_covariate_effect, add_dosing, add_output or rename_symbol with its documented fields."}})
        })?;
        let summary = editor.apply(op).map_err(|message| {
            json!({"error":{"code":"operation","message":format!("operations[{index}]: {message}"),"expression":format!("/operations/{index}"),
                "suggestion":"Correct this operation; no edits were applied."}})
        })?;
        editor
            .summary
            .push(json!({"index":index,"op":raw["op"],"summary":summary}));
    }
    let changes = json!({"applied":editor.summary,"added":editor.added,"changed_equations":editor.changed_equations});
    Ok((editor.model, changes))
}

#[cfg(test)]
mod tests {
    use super::super as compute;
    use super::*;

    const BASE: &str = "model base\ntime_unit = d\nparameters\n  cl = 0.2 [L/d] log description \"clearance\"\n  v = 3 [L] log description \"volume\"\nstates\n  central = 0 [mg]\ndynamics\n  d/dt(central) = (-(cl / v) * central)\noutputs\n  cp = central / v [mg/L]\nend\n";

    fn base() -> Value {
        compute::execute(BASE, "validate_model", json!({})).unwrap()["model"].clone()
    }

    #[test]
    fn mechanism_edits_validate_and_simulate() {
        let ops = vec![
            json!({"op":"add_dosing","state":"central","amount_unit":"mg"}),
            json!({"op":"add_peripheral_compartment","central":"central","name":"tissue","coordinates":"amount","central_volume":"v","volume":{"value":4,"unit":"L"},"clearance":{"value":0.5,"unit":"L/d"}}),
            json!({"op":"add_covariate","name":"wt","unit":"kg","default":{"value":70,"unit":"kg"}}),
            json!({"op":"add_covariate_effect","parameter":"cl","covariate":"wt","reference":{"value":70,"unit":"kg"},"form":"power","coefficient":0.75}),
            json!({"op":"add_output","name":"tissue_amount","unit":"mg","expression":"tissue"}),
        ];
        let (model, changes) = apply(base(), &ops).unwrap();
        assert_eq!(changes["changed_equations"], json!(["central"]));
        let validated = compute::execute(&model.to_string(), "validate_model", json!({})).unwrap();
        let text = validated["canonical_source"].as_str().unwrap();
        assert!(text.contains("tissue_clearance"));
        assert!(text.contains("cl_i"));
        assert!(text.contains("input_central"));
        let request = json!({"schema":"qspquest.run/v0.1","solver":"diffsol_bdf",
            "budgets":{"solver_callbacks":1000000,"events":100,"output_values":10000},
            "regimen":{"end":{"value":10,"unit":"d"},"samples":[{"value":0,"unit":"d"},{"value":10,"unit":"d"}],
                "administrations":[{"target":"central","time":{"value":0,"unit":"d"},"amount":{"value":100,"unit":"mg"},"delivery":{"kind":"bolus"},"bioavailability":1.0}],
                "rtol":1e-8,"atol":1e-10}});
        let result = compute::execute(text, "simulate", request).unwrap();
        let tissue = result["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "tissue_amount")
            .unwrap();
        assert!(
            tissue["values"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .as_f64()
                .unwrap()
                > 0.0
        );
    }

    #[test]
    fn invalid_operations_are_located_and_refused() {
        let error = apply(
            base(),
            &[json!({"op":"add_term","state":"missing","expression":"1"})],
        )
        .unwrap_err();
        assert_eq!(error["error"]["expression"], "/operations/0");
        let error = apply(base(), &[json!({"op":"add_parameter","name":"Q","value":{"value":1,"unit":"L/d"},"description":"x"})]).unwrap_err();
        assert!(error["error"]["message"].as_str().unwrap().contains("'q'"));
        let error = apply(base(), &[json!({"op":"add_parameter","name":"q","value":{"value":1,"unit":"L/d"},"description":" "})]).unwrap_err();
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("description"));
        let error = apply(
            base(),
            &[json!({"op":"add_term","state":"central","expression":"1\nend"})],
        )
        .unwrap_err();
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("newlines"));
    }

    #[test]
    fn dosing_a_preloaded_state_warns_and_set_initial_clears_it() {
        let source = BASE.replace("central = 0 [mg]", "central = 100 [mg]");
        let model =
            compute::execute(&source, "validate_model", json!({})).unwrap()["model"].clone();
        let (_, changes) = apply(
            model.clone(),
            &[json!({"op":"add_dosing","state":"central","amount_unit":"mg"})],
        )
        .unwrap();
        assert!(changes["applied"][0]["summary"]
            .as_str()
            .unwrap()
            .contains("WARNING"));
        let (edited, _) = apply(
            model,
            &[
                json!({"op":"set_initial","state":"central","initial":{"value":0,"unit":"mg"}}),
                json!({"op":"add_dosing","state":"central","amount_unit":"mg"}),
            ],
        )
        .unwrap();
        assert_eq!(edited["states"][0]["initial"]["quantity"]["value"], 0.0);
    }

    #[test]
    fn dosing_and_equations_can_be_replaced() {
        let ops = vec![
            json!({"op":"add_state","name":"depot","unit":"mg"}),
            json!({"op":"add_parameter","name":"ka","value":{"value":1.0,"unit":"1/d"},"description":"assumed"}),
            json!({"op":"add_parameter","name":"tlag","value":{"value":0.1,"unit":"d"},"description":"assumed"}),
            json!({"op":"add_flux","from":"depot","to":"central","rate":"ka * depot"}),
            json!({"op":"add_dosing","state":"depot","amount_unit":"mg","lag":"tlag"}),
            json!({"op":"set_dosing","state":"depot","lag":""}),
            json!({"op":"set_rhs","state":"central","expression":"ka * depot - cl / v * central"}),
            json!({"op":"add_covariate","name":"wt","unit":"kg","default":{"value":70,"unit":"kg"}}),
            json!({"op":"add_covariate_effect","parameter":"cl","covariate":"wt","reference":{"value":70,"unit":"kg"},"form":"power","coefficient":0.75}),
        ];
        let (model, changes) = apply(base(), &ops).unwrap();
        let text = compute::execute(&model.to_string(), "validate_model", json!({})).unwrap()
            ["canonical_source"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(!text.contains("quantity(0"), "{text}");
        assert!(
            text.contains("d/dt(depot) = (input_depot - (ka * depot))")
                || text.contains("input_depot"),
            "{text}"
        );
        assert!(!text.contains("lag(depot)"), "{text}");
        assert!(changes["changed_equations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "central"));
    }

    #[test]
    fn percent_outputs_scaled_twice_are_flagged() {
        let source = BASE.replace(
            "cp = central / v [mg/L]",
            "cp = central / v [mg/L]\n  pct = 100 * (central / quantity(100, \"mg\")) [%]",
        );
        let model =
            compute::execute(&source, "validate_model", json!({})).unwrap()["model"].clone();
        let warnings = lints(&model);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("'pct'"));
    }

    #[test]
    fn rename_follows_references() {
        let (model, _) = apply(
            base(),
            &[json!({"op":"rename_symbol","from":"central","to":"a_central"})],
        )
        .unwrap();
        let text = compute::execute(&model.to_string(), "validate_model", json!({})).unwrap()
            ["canonical_source"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(text.contains("d/dt(a_central)"));
        assert!(!text.contains("d/dt(central)"));
    }
}

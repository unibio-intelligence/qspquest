//! Draft v2 block language for the implemented simulation document.
pub mod expression;
use crate::{document::CompiledDocument, Error};
use qspquest_core::{expression::Expr, model::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: usize,
    pub column: usize,
    pub code: String,
    pub message: String,
    pub hint: String,
}
impl Diagnostic {
    pub fn new(line: usize, column: usize, code: &str, message: &str, hint: &str) -> Self {
        Self {
            line,
            column,
            code: code.into(),
            message: message.into(),
            hint: hint.into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}
#[derive(Clone, Debug)]
pub struct ParsedModel {
    pub document: ModelDocument,
    pub locations: BTreeMap<String, SourceLocation>,
}
impl ParsedModel {
    pub fn compile(&self) -> Result<Arc<CompiledDocument>, Diagnostic> {
        CompiledDocument::compile(&self.document).map_err(|e| self.diagnostic(e))
    }
    pub fn diagnostic(&self, error: Error) -> Diagnostic {
        let key = error.expression.as_deref().unwrap_or("");
        let location = self
            .locations
            .iter()
            .filter(|(path, _)| key == path.as_str() || key.starts_with(&format!("{path}.")))
            .max_by_key(|(path, _)| path.len())
            .map(|(_, v)| v);
        Diagnostic::new(
            location.map_or(1, |s| s.line),
            location.map_or(1, |s| s.column),
            serde_json::to_string(&error.code)
                .unwrap_or_default()
                .trim_matches('"'),
            &error.message,
            "Check the named expression, units, and parameter domains.",
        )
    }
}
fn problem(line: usize, column: usize, message: &str) -> Diagnostic {
    Diagnostic::new(
        line,
        column,
        "language_syntax",
        message,
        "Use the documented block syntax; unsupported features must be declared in a later supported version.",
    )
}
fn identifier(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "true" | "false")
        && name.len() <= 128
        && name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_lowercase() || (i > 0 && b.is_ascii_digit()))
}
fn uncomment(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && quoted {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
        } else if c == '#' && !quoted {
            return &line[..i];
        }
    }
    line
}
fn assignment(line: &str, n: usize) -> Result<(&str, &str, usize), Diagnostic> {
    let (a, b) = line
        .split_once('=')
        .ok_or_else(|| problem(n, 1, "Expected an assignment"))?;
    let column = a.len() + 2 + (b.len() - b.trim_start().len());
    Ok((a.trim(), b.trim(), column))
}
fn unit_value(source: &str, n: usize, column: usize) -> Result<(&str, String, &str), Diagnostic> {
    let mut quoted = false;
    let mut escaped = false;
    let open = source
        .char_indices()
        .find_map(|(i, c)| {
            if escaped {
                escaped = false;
                return None;
            }
            if c == '\\' && quoted {
                escaped = true;
            } else if c == '"' {
                quoted = !quoted;
            } else if c == '[' && !quoted {
                return Some(i);
            }
            None
        })
        .ok_or_else(|| problem(n, column, "Expected a unit in brackets"))?;
    let mut depth = 1;
    let close = source[open + 1..]
        .char_indices()
        .find_map(|(i, c)| {
            if c == '[' {
                depth += 1;
            } else if c == ']' {
                depth -= 1;
                if depth == 0 {
                    return Some(i + open + 1);
                }
            }
            None
        })
        .ok_or_else(|| problem(n, column + open, "Unclosed unit"))?;
    let unit = source[open + 1..close].trim();
    qspquest_core::units::Unit::parse(unit).map_err(|e| {
        Diagnostic::new(
            n,
            column + open + 1,
            "unit",
            &e.message,
            "Use a supported case-sensitive UCUM code.",
        )
    })?;
    Ok((
        source[..open].trim(),
        unit.into(),
        source[close + 1..].trim(),
    ))
}
fn number(source: &str, n: usize, column: usize) -> Result<f64, Diagnostic> {
    source
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| problem(n, column, "Expected a finite numeric value"))
}
fn quantity(source: &str, n: usize, column: usize) -> Result<(Quantity, &str), Diagnostic> {
    let (value, unit, tail) = unit_value(source, n, column)?;
    Ok((
        Quantity {
            value: number(value, n, column)?,
            unit,
        },
        tail,
    ))
}
fn quoted(source: &str, n: usize) -> Result<String, Diagnostic> {
    serde_json::from_str::<String>(source)
        .map_err(|_| problem(n, 1, "Expected a double-quoted string"))
}
pub fn parse(source: &str) -> Result<ParsedModel, Diagnostic> {
    if source.len() > 1_000_000 {
        return Err(problem(1, 1, "Model text exceeds one million bytes"));
    }
    let mut document = ModelDocument {
        schema: Schema::V01,
        model_id: String::new(),
        description: String::new(),
        time_unit: String::new(),
        parameters: vec![],
        covariates: vec![],
        individual: vec![],
        definitions: vec![],
        states: vec![],
        outputs: vec![],
        requirements: Requirements::default(),
        invariants: vec![],
        switches: vec![],
        dose_history: vec![],
        dose_switches: vec![],
        linear_pk: None,
    };
    let mut locations = BTreeMap::new();
    let mut blocks = BTreeSet::new();
    let mut block = "";
    let mut ended = false;
    let mut equations = BTreeMap::new();
    let mut dosing = BTreeMap::new();
    let mut lags = BTreeMap::new();
    let mut description_seen = false;
    let mut requirement_keys = BTreeSet::new();
    let mut linear_pk_keys = BTreeSet::new();
    for (index, raw) in source.lines().enumerate() {
        let n = index + 1;
        let line = uncomment(raw).trim();
        let indent = raw.len() - raw.trim_start().len();
        if line.is_empty() {
            continue;
        }
        if ended {
            return Err(problem(n, 1, "Content after end"));
        }
        if document.model_id.is_empty() {
            let id = line
                .strip_prefix("model ")
                .ok_or_else(|| problem(n, 1, "Expected model <name>"))?
                .trim();
            if !identifier(id) {
                return Err(problem(n, 7, "Invalid model identifier"));
            }
            document.model_id = id.into();
            continue;
        }
        if line == "end" {
            ended = true;
            continue;
        }
        if [
            "parameters",
            "covariates",
            "individual",
            "definitions",
            "switches",
            "dose_history",
            "dose_switches",
            "linear_pk",
            "states",
            "dynamics",
            "outputs",
            "dosing",
            "requirements",
            "invariants",
        ]
        .contains(&line)
        {
            if !blocks.insert(line.to_string()) {
                return Err(problem(n, 1, "Duplicate block"));
            }
            block = line;
            continue;
        }
        if block.is_empty() {
            let (name, value, _) = assignment(line, n)?;
            match name {
                "time_unit" if document.time_unit.is_empty() => {
                    qspquest_core::units::Unit::parse(value)
                        .map_err(|e| problem(n, 1, &e.message))?;
                    document.time_unit = value.into();
                }
                "description" if !description_seen => {
                    document.description = quoted(value, n)?;
                    description_seen = true;
                }
                _ => return Err(problem(n, 1, "Unknown or duplicate model header")),
            };
            continue;
        }
        match block {
            "invariants" => {
                let args = line
                    .strip_prefix("nonnegative(")
                    .and_then(|s| s.strip_suffix(')'))
                    .ok_or_else(|| problem(n, indent + 1, "Use nonnegative(state, other_state)"))?;
                let states: Vec<String> = args.split(',').map(|s| s.trim().to_string()).collect();
                if states.iter().any(|s| !identifier(s)) {
                    return Err(problem(n, indent + 1, "Invalid invariant state name"));
                }
                for name in &states {
                    locations.insert(
                        format!("invariants.nonnegative.{name}"),
                        SourceLocation {
                            line: n,
                            column: indent + 1,
                        },
                    );
                }
                document.invariants.push(Invariant::Nonnegative { states });
            }
            "requirements" => {
                let (name, value, col) = assignment(line, n)?;
                if !requirement_keys.insert(name.to_string()) {
                    return Err(problem(n, col + indent, "Duplicate requirement"));
                }
                match name {
                    "stiff" | "sparse_jacobian" | "gradients" => {
                        let flag = match value {
                            "true" => true,
                            "false" => false,
                            _ => {
                                return Err(problem(
                                    n,
                                    col + indent,
                                    "Requirement must be true or false",
                                ));
                            }
                        };
                        match name {
                            "stiff" => document.requirements.stiff = flag,
                            "sparse_jacobian" => document.requirements.sparse_jacobian = flag,
                            _ => document.requirements.gradients = flag,
                        }
                    }
                    "events" => {
                        let list = value
                            .strip_prefix('[')
                            .and_then(|s| s.strip_suffix(']'))
                            .ok_or_else(|| {
                                problem(n, col + indent, "Use events = [bolus, infusion]")
                            })?;
                        if !list.trim().is_empty() {
                            for item in list.split(',') {
                                let event: EventRequirement = serde_json::from_value(
                                    serde_json::Value::String(item.trim().into()),
                                )
                                .map_err(|_| {
                                    problem(n, col + indent, "Unknown event requirement")
                                })?;
                                if document.requirements.events.contains(&event) {
                                    return Err(problem(
                                        n,
                                        col + indent,
                                        "Duplicate required event capability",
                                    ));
                                }
                                document.requirements.events.push(event);
                            }
                        }
                    }
                    _ => return Err(problem(n, col + indent, "Unknown requirement")),
                }
                locations.insert(
                    format!("requirements.{name}"),
                    SourceLocation {
                        line: n,
                        column: col + indent,
                    },
                );
            }
            "parameters" => {
                let (name, value, col) = assignment(line, n)?;
                let col = col + indent;
                if !identifier(name) {
                    return Err(problem(n, 1, "Invalid parameter name"));
                }
                let (default, mut tail) = quantity(value, n, col)?;
                let mut transform = Transform::Identity;
                let mut fixed = false;
                let mut bounds = None;
                let mut description = String::new();
                let mut options = BTreeSet::new();
                while !tail.is_empty() {
                    let option = tail
                        .split(|c: char| c.is_whitespace() || c == '(')
                        .next()
                        .unwrap();
                    let family = match option {
                        "identity" | "log" | "logit" | "probit" => "transform",
                        "fixed" | "estimated" => "estimation",
                        other => other,
                    };
                    if !options.insert(family) {
                        return Err(problem(n, col, "Duplicate parameter option"));
                    }
                    tail = tail[option.len()..].trim_start();
                    match option {
                        "identity" => transform = Transform::Identity,
                        "log" => transform = Transform::Log,
                        "logit" => transform = Transform::Logit,
                        "probit" => transform = Transform::Probit,
                        "fixed" => fixed = true,
                        "estimated" => fixed = false,
                        "description" => {
                            description = quoted(tail, n)?;
                            tail = "";
                        }
                        "bounds" => {
                            tail = tail.strip_prefix('(').ok_or_else(|| {
                                problem(n, col, "Expected bounds(lower [unit], upper [unit])")
                            })?;
                            let (lo, rest) = quantity(tail, n, col)?;
                            let rest = rest
                                .strip_prefix(',')
                                .ok_or_else(|| problem(n, col, "Expected comma in bounds"))?
                                .trim();
                            let (hi, rest) = quantity(rest, n, col)?;
                            tail = rest
                                .strip_prefix(')')
                                .ok_or_else(|| {
                                    problem(n, col, "Expected closing bounds parenthesis")
                                })?
                                .trim();
                            bounds = Some([lo, hi]);
                        }
                        _ => return Err(problem(n, col, "Unknown parameter option")),
                    }
                }
                locations.insert(
                    format!("parameters.{name}"),
                    SourceLocation {
                        line: n,
                        column: col,
                    },
                );
                document.parameters.push(Parameter {
                    name: name.into(),
                    default,
                    bounds,
                    transform,
                    fixed,
                    description,
                });
            }
            "covariates" => {
                let open = line
                    .find('[')
                    .ok_or_else(|| problem(n, 1, "Expected name [unit] constant or step"))?;
                let close = line[open..]
                    .find(']')
                    .map(|i| i + open)
                    .ok_or_else(|| problem(n, 1, "Missing covariate unit closing bracket"))?;
                let name = line[..open].trim();
                if !identifier(name) {
                    return Err(problem(n, 1, "Invalid covariate name"));
                }
                let unit = line[open + 1..close].trim();
                let tail = line[close + 1..].trim();
                let (mode, default) = match tail.split_once(" default ") {
                    Some((a, b)) => (a, Some(b)),
                    None => (tail, None),
                };
                let interpolation = match mode {
                    "constant" => Interpolation::Constant,
                    "step" => Interpolation::Step,
                    _ => return Err(problem(n, 1, "Expected constant or step interpolation")),
                };
                let default = default
                    .map(|text| {
                        let (q, rest) = quantity(text, n, indent + 1)?;
                        if !rest.is_empty() {
                            return Err(problem(n, 1, "Unexpected covariate suffix"));
                        }
                        Ok(q)
                    })
                    .transpose()?;
                document.covariates.push(Covariate {
                    name: name.into(),
                    unit: unit.into(),
                    interpolation,
                    default,
                });
                locations.insert(
                    format!("covariates.{name}"),
                    SourceLocation {
                        line: n,
                        column: indent + 1,
                    },
                );
            }
            "individual" => {
                let (name, value, col) = assignment(line, n)?;
                let col = col + indent;
                if !identifier(name) {
                    return Err(problem(n, 1, "Invalid individual parameter name"));
                }
                let (expr, unit) = if value.contains('[') {
                    let (expr, unit, tail) = unit_value(value, n, col)?;
                    if !tail.is_empty() {
                        return Err(problem(n, col, "Unexpected individual parameter suffix"));
                    }
                    (expr, Some(unit))
                } else {
                    (value, None)
                };
                document.individual.push(IndividualParameter {
                    name: name.into(),
                    expression: expression::parse(expr, n, col)?,
                    unit,
                });
                locations.insert(
                    format!("individual.{name}"),
                    SourceLocation {
                        line: n,
                        column: col,
                    },
                );
            }
            "linear_pk" => {
                let (key, value, col) = assignment(line, n)?;
                let col = col + indent;
                let pk = document
                    .linear_pk
                    .get_or_insert_with(|| LinearPkDefinition {
                        amount_unit: String::new(),
                        clearance: String::new(),
                        compartments: vec![],
                        exchange_clearances: vec![],
                        depot: None,
                    });
                if !linear_pk_keys.insert(key.to_string()) {
                    return Err(problem(n, col, "Duplicate linear PK declaration"));
                }
                match key {
                    "amount_unit" => {
                        qspquest_core::units::Unit::parse(value)
                            .map_err(|e| problem(n, col, &e.message))?;
                        pk.amount_unit = value.into();
                    }
                    "clearance" => {
                        if !identifier(value) {
                            return Err(problem(
                                n,
                                col,
                                "Clearance must name a declared parameter",
                            ));
                        }
                        pk.clearance = value.into();
                    }
                    "exchange_clearances" => {
                        pk.exchange_clearances = if value.is_empty() {
                            vec![]
                        } else {
                            value.split(',').map(|v| v.trim().to_string()).collect()
                        };
                        if pk.exchange_clearances.iter().any(|v| !identifier(v)) {
                            return Err(problem(
                                n,
                                col,
                                "Expected comma-separated clearance parameter names",
                            ));
                        }
                    }
                    _ => {
                        let words: Vec<_> = key.split_whitespace().collect();
                        let (kind,name,coefficient)=match words.as_slice(){
                            ["compartment",name,"volume",coefficient] => ("compartment",*name,*coefficient),
                            ["depot",name,"absorption",coefficient] => ("depot",*name,*coefficient),
                            _=>return Err(problem(n,col,"Expected compartment NAME volume PARAMETER or depot NAME absorption PARAMETER")),
                        };
                        if !identifier(name) || !identifier(coefficient) {
                            return Err(problem(
                                n,
                                col,
                                "Invalid linear PK state or parameter name",
                            ));
                        }
                        let initial = if value.contains('[') {
                            let (quantity, tail) = quantity(value, n, col)?;
                            if !tail.is_empty() {
                                return Err(problem(n, col, "Unexpected initial quantity suffix"));
                            }
                            Initial::Quantity { quantity }
                        } else {
                            Initial::Expression {
                                expression: expression::parse(value, n, col)?,
                            }
                        };
                        if kind == "compartment" {
                            if pk.compartments.len() == 3 {
                                return Err(problem(
                                    n,
                                    col,
                                    "At most three disposition compartments are supported",
                                ));
                            }
                            pk.compartments.push(LinearPkCompartment {
                                dosing_override: None,
                                state: name.into(),
                                volume: coefficient.into(),
                                initial,
                            });
                        } else {
                            if pk.depot.is_some() {
                                return Err(problem(
                                    n,
                                    col,
                                    "Only one absorption depot is supported",
                                ));
                            }
                            pk.depot = Some(LinearPkDepot {
                                dosing_override: None,
                                state: name.into(),
                                absorption: coefficient.into(),
                                initial,
                            });
                        }
                        locations.insert(
                            format!("states.{name}.initial"),
                            SourceLocation {
                                line: n,
                                column: col,
                            },
                        );
                        locations.insert(
                            format!("states.{name}.rhs"),
                            SourceLocation {
                                line: n,
                                column: col,
                            },
                        );
                    }
                }
                locations.insert(
                    format!("linear_pk.{key}"),
                    SourceLocation {
                        line: n,
                        column: col,
                    },
                );
            }
            "dose_switches" => {
                let (name, value, col) = assignment(line, n)?;
                let col = col + indent;
                if !identifier(name) {
                    return Err(problem(n, col, "Invalid dose-switch name"));
                }
                let (predicate, policy) = value.rsplit_once(" simultaneous ").ok_or_else(|| {
                    problem(n, col, "Expected explicit simultaneous first|last policy")
                })?;
                let simultaneous = match policy.trim() {
                    "first" => qspquest_core::dose_history::SimultaneousDose::First,
                    "last" => qspquest_core::dose_history::SimultaneousDose::Last,
                    _ => return Err(problem(n, col, "Expected simultaneous first|last")),
                };
                let rest = predicate.strip_prefix("time_since_dose(").ok_or_else(|| {
                    problem(n, col, "Expected time_since_dose(target) comparison offset")
                })?;
                let (target, comparison) = rest.split_once(')').ok_or_else(|| {
                    problem(n, col, "Expected closing dose-switch target parenthesis")
                })?;
                let target = target.trim();
                if !identifier(target) {
                    return Err(problem(n, col, "Invalid dose-switch target"));
                }
                let comparison = comparison.trim();
                use qspquest_core::expression::Compare;
                let (operator, offset) = [
                    ("<=", Compare::Le),
                    (">=", Compare::Ge),
                    ("<", Compare::Lt),
                    (">", Compare::Gt),
                ]
                .into_iter()
                .find_map(|(token, op)| {
                    comparison.strip_prefix(token).map(|rest| (op, rest.trim()))
                })
                .ok_or_else(|| problem(n, col, "Expected <, <=, >, or >= dose comparison"))?;
                let offset_col = col + predicate.len() - offset.len();
                document.dose_switches.push(DoseSwitch {
                    name: name.into(),
                    target: target.into(),
                    offset: expression::parse(offset, n, offset_col)?,
                    comparison: operator,
                    simultaneous,
                });
                locations.insert(
                    format!("dose_switches.{name}"),
                    SourceLocation {
                        line: n,
                        column: col,
                    },
                );
            }
            "dose_history" => {
                let (name, value, col) = assignment(line, n)?;
                let col = col + indent;
                if !identifier(name) {
                    return Err(problem(n, 1, "Invalid dose-history name"));
                }
                let (kind, rest) = value.split_once('(').ok_or_else(|| {
                    problem(n, col, "Expected quantity(target) simultaneous first|last")
                })?;
                let (target, suffix) = rest.split_once(')').ok_or_else(|| {
                    problem(n, col, "Expected closing history target parenthesis")
                })?;
                let target = target.trim();
                if !identifier(target) {
                    return Err(problem(n, col, "Invalid dose-history target"));
                }
                let quantity = match kind.trim() {
                    "dose_amount" => HistoryQuantity::DoseAmount,
                    "dose_time" => HistoryQuantity::DoseTime,
                    "time_since_dose" => HistoryQuantity::TimeSinceDose,
                    "has_dose" => HistoryQuantity::HasDose,
                    _ => return Err(problem(n, col, "Unknown dose-history quantity")),
                };
                let policy: Vec<_> = suffix.split_whitespace().collect();
                let simultaneous = match policy.as_slice() {
                    ["simultaneous", "first"] => {
                        qspquest_core::dose_history::SimultaneousDose::First
                    }
                    ["simultaneous", "last"] => qspquest_core::dose_history::SimultaneousDose::Last,
                    _ => {
                        return Err(problem(
                            n,
                            col,
                            "Expected explicit simultaneous first|last policy",
                        ));
                    }
                };
                document.dose_history.push(DoseHistoryBinding {
                    name: name.into(),
                    target: target.into(),
                    quantity,
                    simultaneous,
                });
                locations.insert(
                    format!("dose_history.{name}"),
                    SourceLocation {
                        line: n,
                        column: col,
                    },
                );
            }
            "switches" => {
                let (name, value, col) = assignment(line, n)?;
                if !identifier(name) {
                    return Err(problem(n, 1, "Invalid switch name"));
                }
                let parsed = expression::parse(value, n, col + indent)?;
                let (time, comparison) = match parsed {
                    qspquest_core::expression::Expr::Compare {
                        operator,
                        arguments,
                    } if matches!(&arguments[0], qspquest_core::expression::Expr::Symbol { name } if name == "time") => {
                        (arguments[1].clone(), Some(operator))
                    }
                    other => (other, None),
                };
                document.switches.push(FixedSwitch {
                    name: name.into(),
                    time,
                    comparison,
                });
                locations.insert(
                    format!("switches.{name}"),
                    SourceLocation {
                        line: n,
                        column: col + indent,
                    },
                );
            }
            "definitions" => {
                let (name, value, col) = assignment(line, n)?;
                let col = col + indent;
                if !identifier(name) {
                    return Err(problem(n, 1, "Invalid definition name"));
                }
                let (expr, unit) = if value.contains('[') {
                    let (expr, unit, tail) = unit_value(value, n, col)?;
                    if !tail.is_empty() {
                        return Err(problem(n, col, "Unexpected definition suffix"));
                    }
                    (expr, Some(unit))
                } else {
                    (value, None)
                };
                document.definitions.push(Definition {
                    name: name.into(),
                    expression: expression::parse(expr, n, col)?,
                    unit,
                });
                locations.insert(
                    format!("definitions.{name}"),
                    SourceLocation {
                        line: n,
                        column: col,
                    },
                );
            }
            "states" => {
                let (name, value, col) = assignment(line, n)?;
                let col = col + indent;
                if !identifier(name) {
                    return Err(problem(n, 1, "Invalid state name"));
                }
                let (initial, initial_unit, mut tail) = unit_value(value, n, col)?;
                let mut unit = initial_unit.clone();
                if let Some(rest) = tail.strip_prefix("unit ") {
                    let (prefix, state_unit, rest) = unit_value(rest, n, col)?;
                    if !prefix.is_empty() {
                        return Err(problem(n, col, "Expected unit [state_unit]"));
                    }
                    unit = state_unit;
                    tail = rest;
                }
                let initial = if let Ok(value) = initial.parse::<f64>() {
                    if !value.is_finite() {
                        return Err(problem(n, col, "Initial value must be finite"));
                    }
                    Initial::Quantity {
                        quantity: Quantity {
                            value,
                            unit: initial_unit.clone(),
                        },
                    }
                } else {
                    if initial_unit != unit {
                        return Err(problem(
                            n,
                            col,
                            "Expression initial conditions use the declared state unit",
                        ));
                    }
                    Initial::Expression {
                        expression: expression::parse(initial, n, col)?,
                    }
                };
                let input = match tail {
                    "" => None,
                    "input" => Some(Dosing {
                        lag: None,
                        modes: qspquest_core::model::default_delivery_modes(),
                        amount_unit: unit.clone(),
                        scale: Expr::number(1.0),
                    }),
                    _ => return Err(problem(n, col, "Unknown state option")),
                };
                locations.insert(
                    format!("states.{name}.initial"),
                    SourceLocation {
                        line: n,
                        column: col,
                    },
                );
                document.states.push(State {
                    name: name.into(),
                    unit,
                    initial,
                    rhs: Expr::number(0.0),
                    dosing: input,
                });
            }
            "dynamics" => {
                let (target, value, col) = assignment(line, n)?;
                let col = col + indent;
                let target = target
                    .strip_prefix("d/dt(")
                    .and_then(|s| s.strip_suffix(')'))
                    .ok_or_else(|| problem(n, 1, "Use d/dt(state) = expression"))?;
                if !identifier(target) || equations.contains_key(target) {
                    return Err(problem(n, 1, "Invalid or duplicate derivative target"));
                }
                equations.insert(target.to_string(), expression::parse(value, n, col)?);
                locations.insert(
                    format!("states.{target}.rhs"),
                    SourceLocation {
                        line: n,
                        column: col,
                    },
                );
            }
            "outputs" => {
                let (name, value, col) = assignment(line, n)?;
                let col = col + indent;
                if !identifier(name) {
                    return Err(problem(n, 1, "Invalid output name"));
                }
                let (expr, unit, tail) = unit_value(value, n, col)?;
                if !tail.is_empty() {
                    return Err(problem(n, col, "Unexpected output suffix"));
                }
                document.outputs.push(Output {
                    name: name.into(),
                    unit,
                    expression: expression::parse(expr, n, col)?,
                });
                locations.insert(
                    format!("outputs.{name}"),
                    SourceLocation {
                        line: n,
                        column: col,
                    },
                );
            }
            "dosing" => {
                let (target, value, col) = assignment(line, n)?;
                let col = col + indent;
                if let Some(name) = target
                    .strip_prefix("lag(")
                    .and_then(|v| v.strip_suffix(')'))
                {
                    if !identifier(name) || lags.contains_key(name) {
                        return Err(problem(n, 1, "Invalid or duplicate lag target"));
                    }
                    lags.insert(name.to_string(), expression::parse(value, n, col)?);
                    locations.insert(
                        format!("states.{name}.dosing.lag"),
                        SourceLocation {
                            line: n,
                            column: col,
                        },
                    );
                    continue;
                }
                let (name, unit, tail) = unit_value(target, n, 1)?;
                if !identifier(name) || dosing.contains_key(name) {
                    return Err(problem(n, 1, "Invalid or duplicate dosing target"));
                }
                let modes = if tail.is_empty() {
                    qspquest_core::model::default_delivery_modes()
                } else {
                    tail.split_whitespace()
                        .map(|word| match word {
                            "bolus" => Ok(qspquest_core::model::DeliveryMode::Bolus),
                            "infusion" => Ok(qspquest_core::model::DeliveryMode::Infusion),
                            _ => Err(problem(n, 1, "Unknown dosing mode")),
                        })
                        .collect::<Result<Vec<_>, _>>()?
                };
                qspquest_core::model::validate_delivery_modes(&modes)
                    .map_err(|e| problem(n, 1, &e.message))?;
                dosing.insert(
                    name.to_string(),
                    Dosing {
                        lag: None,
                        modes,
                        amount_unit: unit,
                        scale: expression::parse(value, n, col)?,
                    },
                );
                locations.insert(
                    format!("states.{name}.dosing"),
                    SourceLocation {
                        line: n,
                        column: col,
                    },
                );
            }
            _ => unreachable!(),
        }
    }
    if !ended || document.model_id.is_empty() || document.time_unit.is_empty() {
        return Err(problem(
            source.lines().count().max(1),
            1,
            "Model requires model, time_unit, and end",
        ));
    }
    if blocks.contains("linear_pk") {
        let pk = document
            .linear_pk
            .as_ref()
            .ok_or_else(|| problem(1, 1, "Empty linear_pk block"))?;
        if pk.amount_unit.is_empty()
            || pk.clearance.is_empty()
            || pk.compartments.is_empty()
            || pk.exchange_clearances.len() + 1 != pk.compartments.len()
        {
            return Err(problem(1,1,"Linear PK requires amount_unit, clearance, 1..3 compartments and matching exchange clearances"));
        }
        if !document.states.is_empty() {
            return Err(problem(
                1,
                1,
                "Linear PK cannot coexist with explicit ODE states",
            ));
        }
    }
    if let Some(pk) = &mut document.linear_pk {
        let amount_unit = pk.amount_unit.clone();
        let records = pk
            .compartments
            .iter_mut()
            .map(|c| (&c.state, &mut c.dosing_override))
            .chain(
                pk.depot
                    .iter_mut()
                    .map(|d| (&d.state, &mut d.dosing_override)),
            );
        for (name, override_) in records {
            if let Some(d) = dosing.remove(name) {
                *override_ = Some(d);
            }
            if let Some(lag) = lags.remove(name) {
                override_
                    .get_or_insert_with(|| Dosing {
                        amount_unit: amount_unit.clone(),
                        modes: default_delivery_modes(),
                        scale: Expr::number(1.),
                        lag: None,
                    })
                    .lag = Some(lag);
            }
        }
    }
    for state in &mut document.states {
        state.rhs = equations
            .remove(&state.name)
            .ok_or_else(|| problem(1, 1, &format!("Missing dynamics for {}", state.name)))?;
        if let Some(input) = dosing.remove(&state.name) {
            if state.dosing.is_some() {
                return Err(problem(
                    1,
                    1,
                    "Declare dosing once, using input or the dosing block",
                ));
            }
            state.dosing = Some(input);
        }
        if let Some(lag) = lags.remove(&state.name) {
            let dosing = state
                .dosing
                .as_mut()
                .ok_or_else(|| problem(1, 1, "Lag target must accept dosing"))?;
            dosing.lag = Some(lag);
        }
    }
    if !equations.is_empty() || !dosing.is_empty() || !lags.is_empty() {
        return Err(problem(
            1,
            1,
            "Equation or dosing target is not a declared state",
        ));
    }
    Ok(ParsedModel {
        document,
        locations,
    })
}
pub fn format(document: &ModelDocument) -> Result<String, Diagnostic> {
    CompiledDocument::compile(document).map_err(|e| {
        Diagnostic::new(
            1,
            1,
            "invalid_model",
            &e.message,
            "Correct model validation errors before formatting.",
        )
    })?;
    let q = |q: &Quantity| format!("{} [{}]", q.value, q.unit);
    let string = |s: &str| serde_json::to_string(s).expect("String serialization cannot fail");
    let mut out = format!(
        "model {}\ntime_unit = {}\n",
        document.model_id, document.time_unit
    );
    if !document.description.is_empty() {
        out += &format!("description = {}\n", string(&document.description));
    }
    if document.requirements != Requirements::default() {
        out += "\nrequirements\n";
        out += &format!(
            "  stiff = {}\n  sparse_jacobian = {}\n  gradients = {}\n",
            document.requirements.stiff,
            document.requirements.sparse_jacobian,
            document.requirements.gradients
        );
        out += &format!(
            "  events = [{}]\n",
            document
                .requirements
                .events
                .iter()
                .map(|e| e.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    out += "\nparameters\n";
    for p in &document.parameters {
        out += &format!(
            "  {} = {} {} {}",
            p.name,
            q(&p.default),
            match p.transform {
                Transform::Identity => "identity",
                Transform::Log => "log",
                Transform::Logit => "logit",
                Transform::Probit => "probit",
            },
            if p.fixed { "fixed" } else { "estimated" }
        );
        if let Some([lo, hi]) = &p.bounds {
            out += &format!(" bounds({}, {})", q(lo), q(hi));
        }
        if !p.description.is_empty() {
            out += &format!(" description {}", string(&p.description));
        }
        out += "\n";
    }
    if !document.covariates.is_empty() {
        out += "\ncovariates\n";
        for c in &document.covariates {
            let mode = match c.interpolation {
                Interpolation::Constant => "constant",
                Interpolation::Step => "step",
            };
            let default = c
                .default
                .as_ref()
                .map(|v| format!(" default {}", q(v)))
                .unwrap_or_default();
            out += &format!("  {} [{}] {}{}\n", c.name, c.unit, mode, default);
        }
    }
    if !document.individual.is_empty() {
        out += "\nindividual\n";
        for p in &document.individual {
            out += &format!(
                "  {} = {}{}\n",
                p.name,
                expression::format(&p.expression),
                p.unit
                    .as_ref()
                    .map(|u| format!(" [{u}]"))
                    .unwrap_or_default()
            );
        }
    }
    if !document.dose_history.is_empty() {
        out += "\ndose_history\n";
        for binding in &document.dose_history {
            let quantity = match binding.quantity {
                HistoryQuantity::DoseAmount => "dose_amount",
                HistoryQuantity::DoseTime => "dose_time",
                HistoryQuantity::TimeSinceDose => "time_since_dose",
                HistoryQuantity::HasDose => "has_dose",
            };
            let policy = match binding.simultaneous {
                qspquest_core::dose_history::SimultaneousDose::First => "first",
                qspquest_core::dose_history::SimultaneousDose::Last => "last",
            };
            out += &format!(
                "  {} = {}({}) simultaneous {}\n",
                binding.name, quantity, binding.target, policy
            );
        }
    }
    if !document.dose_switches.is_empty() {
        out += "\ndose_switches\n";
        for switch in &document.dose_switches {
            use qspquest_core::expression::Compare;
            let operator = match switch.comparison {
                Compare::Lt => "<",
                Compare::Le => "<=",
                Compare::Gt => ">",
                Compare::Ge => ">=",
                _ => return Err(problem(1, 1, "Unsupported dose comparison")),
            };
            let policy = match switch.simultaneous {
                qspquest_core::dose_history::SimultaneousDose::First => "first",
                qspquest_core::dose_history::SimultaneousDose::Last => "last",
            };
            out += &format!(
                "  {} = time_since_dose({}) {} {} simultaneous {}\n",
                switch.name,
                switch.target,
                operator,
                expression::format(&switch.offset),
                policy
            );
        }
    }
    if !document.switches.is_empty() {
        out += "\nswitches\n";
        for switch in &document.switches {
            out += &format!(
                "  {} = {}\n",
                switch.name,
                expression::format(&match switch.comparison {
                    Some(operator) => qspquest_core::expression::Expr::Compare {
                        operator,
                        arguments: Box::new([
                            qspquest_core::expression::Expr::symbol("time"),
                            switch.time.clone()
                        ]),
                    },
                    None => switch.time.clone(),
                })
            );
        }
    }
    if !document.definitions.is_empty() {
        out += "\ndefinitions\n";
        for p in &document.definitions {
            out += &format!(
                "  {} = {}{}\n",
                p.name,
                expression::format(&p.expression),
                p.unit
                    .as_ref()
                    .map(|u| format!(" [{u}]"))
                    .unwrap_or_default()
            );
        }
    }
    if let Some(pk) = &document.linear_pk {
        out += "\nlinear_pk\n";
        out += &format!(
            "  amount_unit = {}\n  clearance = {}\n  exchange_clearances = {}\n",
            pk.amount_unit,
            pk.clearance,
            pk.exchange_clearances.join(", ")
        );
        let initial = |value: &Initial| match value {
            Initial::Quantity { quantity } => q(quantity),
            Initial::Expression { expression: e } => expression::format(e),
        };
        for c in &pk.compartments {
            out += &format!(
                "  compartment {} volume {} = {}\n",
                c.state,
                c.volume,
                initial(&c.initial)
            );
        }
        if let Some(d) = &pk.depot {
            out += &format!(
                "  depot {} absorption {} = {}\n",
                d.state,
                d.absorption,
                initial(&d.initial)
            );
        }
    }
    if !document.states.is_empty() {
        out += "\nstates\n";
    }
    for s in &document.states {
        let initial = match &s.initial {
            Initial::Quantity { quantity } => {
                let suffix = if quantity.unit == s.unit {
                    String::new()
                } else {
                    format!(" unit [{}]", s.unit)
                };
                format!("{} [{}]{}", quantity.value, quantity.unit, suffix)
            }
            Initial::Expression { expression: e } => {
                format!("({}) [{}]", expression::format(e), s.unit)
            }
        };
        out += &format!("  {} = {}\n", s.name, initial);
    }
    if !document.states.is_empty() {
        out += "\ndynamics\n";
    }
    for s in &document.states {
        out += &format!("  d/dt({}) = {}\n", s.name, expression::format(&s.rhs));
    }
    out += "\noutputs\n";
    for o in &document.outputs {
        out += &format!(
            "  {} = {} [{}]\n",
            o.name,
            expression::format(&o.expression),
            o.unit
        );
    }
    let mut dosing_entries: Vec<_> = document
        .states
        .iter()
        .filter_map(|s| s.dosing.as_ref().map(|d| (s.name.as_str(), d)))
        .collect();
    if let Some(pk) = &document.linear_pk {
        dosing_entries.extend(
            pk.compartments
                .iter()
                .filter_map(|c| c.dosing_override.as_ref().map(|d| (c.state.as_str(), d))),
        );
        dosing_entries.extend(
            pk.depot
                .iter()
                .filter_map(|c| c.dosing_override.as_ref().map(|d| (c.state.as_str(), d))),
        );
    }
    if !dosing_entries.is_empty() {
        out += "\ndosing\n";
        for (name, d) in dosing_entries {
            out += &format!(
                "  {} [{}]{} = {}\n",
                name,
                d.amount_unit,
                if d.modes == qspquest_core::model::default_delivery_modes() {
                    String::new()
                } else {
                    d.modes
                        .iter()
                        .map(|m| match m {
                            qspquest_core::model::DeliveryMode::Bolus => " bolus",
                            qspquest_core::model::DeliveryMode::Infusion => " infusion",
                        })
                        .collect::<String>()
                },
                expression::format(&d.scale)
            );
            if let Some(lag) = &d.lag {
                out += &format!("  lag({}) = {}\n", name, expression::format(lag));
            }
        }
    }
    if !document.invariants.is_empty() {
        out += "\ninvariants\n";
        for invariant in &document.invariants {
            match invariant {
                Invariant::Nonnegative { states } => {
                    out += &format!("  nonnegative({})\n", states.join(", "))
                }
            }
        }
    }
    out += "end\n";
    parse(&out)?;
    Ok(out)
}

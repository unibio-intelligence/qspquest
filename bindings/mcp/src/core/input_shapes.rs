//! Read-only structural admission before friendly JSON normalization.
use serde_json::{json, Value};

#[derive(Debug)]
pub(crate) struct Failure {
    pointer: String,
    message: String,
    suggestion: String,
}
impl Failure {
    pub(crate) fn at(
        pointer: String,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            pointer,
            message: message.into(),
            suggestion: suggestion.into(),
        }
    }
    pub(crate) fn into_value(self, code: &str) -> Value {
        json!({"error":{"code":code,"message":self.message,"expression":self.pointer,"suggestion":self.suggestion}})
    }
    pub(crate) fn message(self) -> String {
        self.message
    }
}
pub(crate) fn child(path: &str, name: &str) -> String {
    format!("{path}/{}", name.replace('~', "~0").replace('/', "~1"))
}
const QUANTITY_HINT: &str = "Supply an object with a finite numeric value and a nonempty unit string compatible with the model's declared dimension.";
pub(crate) fn quantity(value: &Value, path: &str, display: &str) -> Result<(), Failure> {
    // Choose the location from structure, never from parsing an error message.
    let location = if let Some(fields) = value.as_object() {
        if let Some(key) = fields
            .keys()
            .find(|k| !["value", "unit"].contains(&k.as_str()))
        {
            child(path, key)
        } else if value["value"].as_f64().is_none_or(|n| !n.is_finite()) {
            child(path, "value")
        } else if value["unit"].as_str().is_none_or(str::is_empty) {
            child(path, "unit")
        } else {
            path.to_owned()
        }
    } else {
        path.to_owned()
    };
    super::requests::quantity(value, display)
        .map(|_| ())
        .map_err(|message| Failure::at(location, message, QUANTITY_HINT))
}

fn dose_decode_location(dose: &Value, path: &str) -> String {
    let Some(fields) = dose.as_object() else {
        return path.to_owned();
    };
    for (name, value) in fields {
        let pointer = child(path, name);
        match name.as_str() {
            "target" if !value.is_string() => return pointer,
            "amount" | "at" | "every" | "duration" | "lag" => {
                if value.is_null() && !["amount", "at"].contains(&name.as_str()) {
                    continue;
                }
                if serde_json::from_value::<crate::tool_api::Quantity>(value.clone()).is_err() {
                    return quantity(value, &pointer, name)
                        .err()
                        .map_or(pointer, |error| error.pointer);
                }
            }
            "count" if !value.is_null() && value.as_u64().is_none() => return pointer,
            "bioavailability" if !value.is_null() && value.as_f64().is_none() => return pointer,
            "target" | "count" | "bioavailability" => {}
            _ => return pointer,
        }
    }
    for name in ["target", "amount", "at"] {
        if !fields.contains_key(name) {
            return child(path, name);
        }
    }
    path.to_owned()
}

/// String compatibility interface for existing callers.
pub fn friendly(args: &Value) -> Result<(), String> {
    admit(args).map_err(Failure::message)
}
/// Validate adapter containers without changing submitted values.
/// Native binding remains responsible for dimensions, domains and work budgets.
pub(crate) fn admit(args: &Value) -> Result<(), Failure> {
    args.as_object().ok_or_else(|| {
        Failure::at(
            "".into(),
            "arguments must be an object",
            "Supply the tool arguments as a JSON object.",
        )
    })?;
    for key in ["parameters", "initial_states"] {
        if let Some(given) = args.get(key).filter(|v| !v.is_null()) {
            let path = child("", key);
            let map = given.as_object().ok_or_else(|| {
                Failure::at(
                    path.clone(),
                    format!("{key} must map names to quantities"),
                    "Supply an object mapping declared names to quantity objects.",
                )
            })?;
            for (name, value) in map {
                quantity(value, &child(&path, name), &format!("{key}.{name}"))?;
            }
        }
    }
    if let Some(request) = args.get("request").filter(|v| !v.is_null()) {
        request.as_object().ok_or_else(|| Failure::at("/request".into(), "request must be an object", "Supply the complete native request object, or omit request and use the documented friendly fields."))?;
        if let Some(regimen) = request.get("regimen") {
            regimen.as_object().ok_or_else(|| Failure::at("/request/regimen".into(), "request.regimen must be an object", "Supply the native regimen object; inspect the run request schema for required fields."))?;
        }
        for key in ["parameters", "initial_states"] {
            if let Some(values) = request.get(key) {
                let path = child("/request", key);
                let fields = values.as_object().ok_or_else(|| {
                    Failure::at(
                        path.clone(),
                        format!("request.{key} must map names to quantities"),
                        "Supply an object mapping native names to quantity objects.",
                    )
                })?;
                for (name, value) in fields {
                    quantity(value, &child(&path, name), &format!("request.{key}.{name}"))?;
                }
            }
        }
    }
    if let Some(doses) = args.get("doses").filter(|v| !v.is_null()) {
        let doses = doses.as_array().ok_or_else(|| Failure::at("/doses".into(), "doses must be an array of dose objects", "Supply an array of doses with target, amount and at; use an empty array for no administrations."))?;
        for (index, dose) in doses.iter().enumerate() {
            let path = child("/doses", &index.to_string());
            // The public Dose type remains authoritative for closed shape.
            serde_json::from_value::<crate::tool_api::Dose>(dose.clone())
                .map_err(|e| {
                    Failure::at(dose_decode_location(dose, &path), format!("doses[{index}]: {e}"), "Supply a complete dose object using only target, amount, at, every, count, duration, lag and bioavailability.")
                })?;
            super::requests::dose(dose).map_err(|message| {
                let mut pointer = child(&path, "count");
                let mut hint = "Set count to an integer from 1 through 10000; repeated doses require an every quantity.";
                for field in ["at", "amount", "duration", "lag"] {
                    if ["at", "amount"].contains(&field) || dose.get(field).is_some() {
                        if let Err(error) = quantity(&dose[field], &child(&path, field), field) {
                            pointer = error.pointer;
                            hint = QUANTITY_HINT;
                            break;
                        }
                    }
                }
                if pointer == child(&path, "count") && dose["count"].as_u64().is_some_and(|n| (2..=10000).contains(&n)) {
                    if let Err(error) = quantity(&dose["every"], &child(&path, "every"), "every") {
                        pointer = error.pointer;
                        hint = QUANTITY_HINT;
                    }
                }
                Failure::at(pointer, format!("doses[{index}]: {message}"), hint)
            })?;
        }
    }
    if let Some(times) = args.get("times").filter(|v| !v.is_null()) {
        for (index, time) in times
            .as_array()
            .ok_or_else(|| {
                Failure::at(
                    "/times".into(),
                    "times must be an array of quantities",
                    "Supply explicit time quantities in an array.",
                )
            })?
            .iter()
            .enumerate()
        {
            quantity(
                time,
                &child("/times", &index.to_string()),
                &format!("times[{index}]"),
            )?;
        }
    }
    if let Some(end) = args.get("end").filter(|v| !v.is_null()) {
        quantity(end, "/end", "end")?;
    }
    if let Some(readouts) = args.get("readouts").filter(|v| !v.is_null()) {
        for (index, readout) in readouts
            .as_array()
            .ok_or_else(|| {
                Failure::at(
                    "/readouts".into(),
                    "readouts must be an array",
                    "Supply an array of readout objects or supported readout text.",
                )
            })?
            .iter()
            .enumerate()
        {
            if !readout.is_object() && !readout.is_string() {
                return Err(Failure::at(
                    child("/readouts", &index.to_string()),
                    format!("readouts[{index}] must be an object or supported readout text"),
                    "Supply kind, output and a time or window for this readout.",
                ));
            }
        }
    }
    if let Some(changes) = args.get("parameter_changes").filter(|v| !v.is_null()) {
        for (index, change) in changes
            .as_array()
            .ok_or_else(|| {
                Failure::at(
                    "/parameter_changes".into(),
                    "parameter_changes must be an array",
                    "Supply an array of timed changes with name, at and value.",
                )
            })?
            .iter()
            .enumerate()
        {
            let path = child("/parameter_changes", &index.to_string());
            let fields = change.as_object().ok_or_else(|| {
                Failure::at(
                    path.clone(),
                    format!("parameter_changes[{index}] must be an object"),
                    "Supply name, at and value for the timed change.",
                )
            })?;
            if let Some(key) = fields
                .keys()
                .find(|k| !["name", "parameter", "at", "value"].contains(&k.as_str()))
            {
                return Err(Failure::at(
                    child(&path, key),
                    format!("parameter_changes[{index}].{key} is unknown; use name, at and value"),
                    "Remove the unknown field; supply name, at and value.",
                ));
            }
            if let (Some(name), Some(alias)) = (change.get("name"), change.get("parameter")) {
                if name != alias {
                    return Err(Failure::at(child(&path, "name"), format!("parameter_changes[{index}].name conflicts with parameter; supply one name"), "Supply one parameter name, or identical name and parameter aliases."));
                }
            }
            change
                .get("name")
                .or_else(|| change.get("parameter"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    Failure::at(
                        child(
                            &path,
                            if change.get("name").is_some() {
                                "name"
                            } else if change.get("parameter").is_some() {
                                "parameter"
                            } else {
                                "name"
                            },
                        ),
                        format!("parameter_changes[{index}].name is required"),
                        "Supply a nonempty declared parameter name.",
                    )
                })?;
            for key in ["at", "value"] {
                if key == "value" && change[key].as_f64().is_some_and(f64::is_finite) {
                    continue;
                }
                quantity(
                    &change[key],
                    &child(&path, key),
                    &format!("parameter_changes[{index}].{key}"),
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn malformed_containers_refuse_without_mutation() {
        for (args, field) in [
            (json!(false), "arguments"),
            (json!({"parameters":[]}), "parameters"),
            (json!({"initial_states":"empty"}), "initial_states"),
            (json!({"doses":{}}), "doses"),
            (json!({"doses":[false]}), "doses[0]"),
            (json!({"times":[1]}), "times[0]"),
            (json!({"readouts":[[]]}), "readouts[0]"),
            (json!({"parameter_changes":[false]}), "parameter_changes[0]"),
            (
                json!({"request":{"schema":"qspquest.run/v0.1","regimen":[]}}),
                "request",
            ),
        ] {
            let before = args.clone();
            assert!(friendly(&args).unwrap_err().contains(field));
            assert_eq!(args, before);
        }
    }

    #[test]
    fn parameter_changes_are_closed_and_alias_conflicts_refuse() {
        let valid = json!({"parameter_changes":[{"name":"cl","at":{"value":1,"unit":"d"},"value":{"value":2,"unit":"L/d"}}]});
        friendly(&valid).unwrap();
        let mut unknown = valid.clone();
        unknown["parameter_changes"][0]["event"] = json!("reset");
        assert!(friendly(&unknown).unwrap_err().contains(".event"));
        let mut conflict = valid;
        conflict["parameter_changes"][0]["parameter"] = json!("volume");
        assert!(friendly(&conflict).unwrap_err().contains("conflicts"));
    }
    #[test]
    fn native_defaults_and_quantities_are_preserved_for_later_preparation() {
        let args = json!({"request":{"schema":"qspquest.run/v0.1","regimen":{"end":{"value":2,"unit":"h"},"samples":[]}},"doses":[{"target":"central","amount":{"value":1,"unit":"mg"},"at":{"value":0,"unit":"h"}}]});
        let before = args.clone();
        friendly(&args).unwrap();
        assert_eq!(args, before);
        friendly(&json!({"parameter_changes":[{"parameter":"k","at":{"value":1,"unit":"h"},"value":2.0}]})).unwrap();
    }
    #[test]
    fn locations_escape_scientific_names_without_parsing_messages() {
        for (args, pointer) in [
            (
                json!({"parameters":{"cl/~.x":{"value":1}}}),
                "/parameters/cl~1~0.x/unit",
            ),
            (
                json!({"request":{"initial_states":{"a/b":{"value":"bad","unit":"mg"}}}}),
                "/request/initial_states/a~1b/value",
            ),
            (
                json!({"times":[{"value":1,"unit":"h","extra/~":0}]}),
                "/times/0/extra~1~0",
            ),
            (
                json!({"parameter_changes":[{"name":"cl","at":{"value":1,"unit":"h"},"value":{"value":2,"unit":""}}]}),
                "/parameter_changes/0/value/unit",
            ),
        ] {
            let before = args.clone();
            let error = admit(&args).unwrap_err().into_value("request");
            assert_eq!(error["error"]["expression"], pointer);
            assert!(error["error"]["suggestion"].as_str().unwrap().len() > 10);
            assert_eq!(args, before);
        }
    }
    #[test]
    fn repeated_dose_locations_and_string_compatibility_are_preserved() {
        let dose = json!({"target":"central","amount":{"value":1,"unit":"mg"},"at":{"value":0,"unit":"h"}});
        for (field, value, pointer) in [
            ("count", json!(0), "/doses/0/count"),
            ("duration", Value::Null, "/doses/0/duration"),
            ("unexpected", json!(true), "/doses/0/unexpected"),
            ("at", json!({"value":"bad","unit":"h"}), "/doses/0/at/value"),
            (
                "amount",
                json!({"value":1,"unit":7}),
                "/doses/0/amount/unit",
            ),
        ] {
            let mut args = json!({"doses":[dose.clone()]});
            args["doses"][0][field] = value;
            let message = friendly(&args).unwrap_err();
            let error = admit(&args).unwrap_err().into_value("request");
            assert_eq!(error["error"]["message"], message);
            assert_eq!(error["error"]["expression"], pointer);
        }
        let mut args = json!({"doses":[dose]});
        args["doses"][0]["count"] = json!(2);
        assert_eq!(
            admit(&args).unwrap_err().into_value("request")["error"]["expression"],
            "/doses/0/every"
        );
    }
}

use serde_json::{Map, Value};

/// Express Boolean schemas as equivalent object schemas for MCP clients that
/// accept object-only subschemas. Literal defaults, examples and flags retain
/// their original types; scientific request decoding remains unchanged.
pub(crate) fn object_subschemas(schema: &mut Value) {
    match schema {
        Value::Bool(true) => *schema = Value::Object(Map::new()),
        Value::Bool(false) => *schema = serde_json::json!({ "not": {} }),
        Value::Object(fields) => {
            for key in [
                "properties",
                "patternProperties",
                "$defs",
                "definitions",
                "dependentSchemas",
            ] {
                if let Some(Value::Object(entries)) = fields.get_mut(key) {
                    for value in entries.values_mut() {
                        object_subschemas(value);
                    }
                }
            }
            for key in [
                "items",
                "contains",
                "not",
                "if",
                "then",
                "else",
                "propertyNames",
                "additionalItems",
            ] {
                if let Some(value) = fields.get_mut(key) {
                    if let Value::Array(items) = value {
                        for item in items {
                            object_subschemas(item);
                        }
                    } else {
                        object_subschemas(value);
                    }
                }
            }
            for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
                if let Some(Value::Array(items)) = fields.get_mut(key) {
                    for item in items {
                        object_subschemas(item);
                    }
                }
            }
            // Boolean additionalProperties is supported by clients and carries
            // the request's closed-object constraint; preserve it verbatim.
            if let Some(value @ Value::Object(_)) = fields.get_mut("additionalProperties") {
                object_subschemas(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn arbitrary_json_fields_are_visible_to_object_only_clients() {
        let mut schema = json!({"type":"object", "properties":{"request":true},
            "additionalProperties":false});
        object_subschemas(&mut schema);
        assert_eq!(schema["properties"]["request"], json!({}));
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn schema_conversion_preserves_literal_boolean_values() {
        let mut schema = json!({"properties":{"flag":{"type":"boolean","default":true,
            "examples":[true,false],"enum":[true,false]},"nested":{"items":false}},
            "$defs":{"anything":true}, "anyOf":[false,{"type":"null"}]});
        object_subschemas(&mut schema);
        assert_eq!(schema["properties"]["flag"]["default"], true);
        assert_eq!(
            schema["properties"]["flag"]["examples"],
            json!([true, false])
        );
        assert_eq!(schema["properties"]["flag"]["enum"], json!([true, false]));
        assert_eq!(schema["properties"]["nested"]["items"], json!({"not":{}}));
        assert_eq!(schema["$defs"]["anything"], json!({}));
        assert_eq!(schema["anyOf"][0], json!({"not":{}}));
    }
}

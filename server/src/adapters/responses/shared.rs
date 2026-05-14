use serde_json::Value;

pub fn clean_tool_schema_for_gemini(value: &mut Value) {
    clean_tool_schema_with_case(value, true);
}

fn clean_tool_schema_with_case(value: &mut Value, uppercase_types: bool) {
    match value {
        Value::Object(map) => {
            map.remove("$schema");
            map.remove("definitions");
            map.remove("$defs");
            map.remove("format");

            let looks_like_schema = map.contains_key("type")
                || map.contains_key("properties")
                || map.contains_key("items")
                || map.contains_key("required")
                || map.contains_key("additionalProperties")
                || map.contains_key("enum")
                || map.contains_key("description");

            if let Some(type_value) = map.get_mut("type") {
                if let Value::String(type_name) = type_value {
                    if uppercase_types {
                        *type_name = type_name.to_uppercase();
                    }
                }
            } else if looks_like_schema {
                map.insert(
                    "type".to_string(),
                    Value::String(if uppercase_types { "OBJECT" } else { "object" }.to_string()),
                );
            }

            if let Some(properties) = map.get_mut("properties") {
                if let Value::Object(properties_map) = properties {
                    for value in properties_map.values_mut() {
                        clean_tool_schema_with_case(value, uppercase_types);
                    }
                }
            } else {
                for value in map.values_mut() {
                    clean_tool_schema_with_case(value, uppercase_types);
                }
            }

            if let Some(items) = map.get_mut("items") {
                clean_tool_schema_with_case(items, uppercase_types);
            }
        }
        Value::Array(values) => {
            for value in values {
                clean_tool_schema_with_case(value, uppercase_types);
            }
        }
        _ => {}
    }
}

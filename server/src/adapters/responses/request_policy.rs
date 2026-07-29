use crate::models::ProviderCompatibilityProfile;
use serde_json::Value;

const GENERIC_FILTERED_FUNCTION_TOOLS: &[&str] = &["list_available_plugins_to_install", "get_goal"];
const GENERIC_FILTERED_TOOL_TYPES: &[&str] = &["tool_search"];

pub fn apply_responses_request_policy(
    profile: &ProviderCompatibilityProfile,
    request_body: String,
) -> Result<String, String> {
    if !matches!(profile, ProviderCompatibilityProfile::GenericOpenAi) {
        return Ok(request_body);
    }

    let mut body: Value = serde_json::from_str(&request_body)
        .map_err(|err| format!("invalid request JSON: {err}"))?;
    filter_generic_openai_tools(&mut body);
    serde_json::to_string(&body).map_err(|err| err.to_string())
}

fn filter_generic_openai_tools(body: &mut Value) {
    let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) else {
        return;
    };

    tools.retain(|tool| !should_remove(tool));
}

fn should_remove(tool: &Value) -> bool {
    let Some(tool_type) = tool.get("type").and_then(Value::as_str) else {
        return false;
    };
    if GENERIC_FILTERED_TOOL_TYPES.contains(&tool_type) {
        return true;
    }

    tool_type == "function"
        && tool
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| GENERIC_FILTERED_FUNCTION_TOOLS.contains(&name))
}

#[cfg(test)]
mod tests {
    use super::apply_responses_request_policy;
    use crate::models::ProviderCompatibilityProfile;
    use serde_json::json;

    #[test]
    fn official_profile_preserves_raw_body() {
        let raw = "{ \"model\": \"gpt\", \"tools\": [] }".to_string();
        let transformed = apply_responses_request_policy(
            &ProviderCompatibilityProfile::OfficialOpenAi,
            raw.clone(),
        )
        .unwrap();

        assert_eq!(transformed, raw);
    }

    #[test]
    fn generic_profile_filters_unsupported_tools() {
        let transformed = apply_responses_request_policy(
            &ProviderCompatibilityProfile::GenericOpenAi,
            json!({
                "tools": [
                    { "type": "tool_search" },
                    { "type": "function", "name": "exec_command" },
                    { "type": "function", "name": "get_goal" }
                ]
            })
            .to_string(),
        )
        .unwrap();

        let body: serde_json::Value = serde_json::from_str(&transformed).unwrap();
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    }
}

use crate::models::ProviderCompatibilityProfile;
use serde::Serialize;
use serde_json::Value;

const GENERIC_FILTERED_FUNCTION_TOOLS: &[&str] = &["list_available_plugins_to_install", "get_goal"];
const GENERIC_FILTERED_TOOL_TYPES: &[&str] = &["tool_search"];

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RequestTransformChange {
    pub rule_id: &'static str,
    pub operation: &'static str,
    pub path: String,
    pub reason: &'static str,
}

#[derive(Debug)]
pub struct TransformedRequestBody {
    pub body: String,
    pub changes: Vec<RequestTransformChange>,
}

pub fn apply_responses_request_policy(
    profile: &ProviderCompatibilityProfile,
    request_body: String,
) -> Result<TransformedRequestBody, String> {
    if !matches!(profile, ProviderCompatibilityProfile::GenericOpenAi) {
        return Ok(TransformedRequestBody {
            body: request_body,
            changes: Vec::new(),
        });
    }

    let mut body: Value = serde_json::from_str(&request_body)
        .map_err(|err| format!("invalid request JSON: {err}"))?;
    let changes = filter_generic_openai_tools(&mut body);
    let body = serde_json::to_string(&body).map_err(|err| err.to_string())?;
    Ok(TransformedRequestBody { body, changes })
}

fn filter_generic_openai_tools(body: &mut Value) -> Vec<RequestTransformChange> {
    let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) else {
        return Vec::new();
    };

    let original = std::mem::take(tools);
    let mut changes = Vec::new();
    for (index, tool) in original.into_iter().enumerate() {
        let Some((rule_id, reason)) = removal_rule(&tool) else {
            tools.push(tool);
            continue;
        };
        changes.push(RequestTransformChange {
            rule_id,
            operation: "remove",
            path: format!("/tools/{index}"),
            reason,
        });
    }
    changes
}

fn removal_rule(tool: &Value) -> Option<(&'static str, &'static str)> {
    let tool_type = tool.get("type").and_then(Value::as_str)?;
    if GENERIC_FILTERED_TOOL_TYPES.contains(&tool_type) {
        return Some((
            "generic_openai.remove_unsupported_tool_type",
            "generic OpenAI-compatible Responses providers may reject Codex client-only tools",
        ));
    }

    let name = tool.get("name").and_then(Value::as_str)?;
    if tool_type == "function" && GENERIC_FILTERED_FUNCTION_TOOLS.contains(&name) {
        return Some((
            "generic_openai.remove_unsupported_function",
            "generic OpenAI-compatible Responses providers may reject gateway-only functions",
        ));
    }
    None
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

        assert_eq!(transformed.body, raw);
        assert!(transformed.changes.is_empty());
    }

    #[test]
    fn generic_profile_reports_removed_tool_paths() {
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

        let body: serde_json::Value = serde_json::from_str(&transformed.body).unwrap();
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(transformed.changes.len(), 2);
        assert_eq!(transformed.changes[0].path, "/tools/0");
        assert_eq!(transformed.changes[1].path, "/tools/2");
    }
}

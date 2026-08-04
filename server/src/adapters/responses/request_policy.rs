use crate::models::ProviderCompatibilityProfile;
use serde_json::Value;

const GENERIC_FILTERED_FUNCTION_TOOLS: &[&str] = &["list_available_plugins_to_install", "get_goal"];
const GENERIC_FILTERED_TOOL_TYPES: &[&str] = &["tool_search"];

pub fn apply_responses_request_policy(
    profile: &ProviderCompatibilityProfile,
    request_body: String,
) -> Result<String, String> {
    match profile {
        ProviderCompatibilityProfile::OfficialOpenAi => Ok(request_body),
        ProviderCompatibilityProfile::GenericOpenAi => {
            let mut body = parse_request_body(&request_body)?;
            filter_generic_openai_tools(&mut body);
            serialize_request_body(&body)
        }
        ProviderCompatibilityProfile::OpenAiCodex => {
            let mut body = parse_request_body(&request_body)?;
            if !strip_plaintext_reasoning_content(&mut body) {
                return Ok(request_body);
            }
            serialize_request_body(&body)
        }
    }
}

fn parse_request_body(request_body: &str) -> Result<Value, String> {
    serde_json::from_str(request_body).map_err(|err| format!("invalid request JSON: {err}"))
}

fn serialize_request_body(body: &Value) -> Result<String, String> {
    serde_json::to_string(body).map_err(|err| err.to_string())
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

fn strip_plaintext_reasoning_content(body: &mut Value) -> bool {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    for item in input {
        if item.get("type").and_then(Value::as_str) != Some("reasoning") {
            continue;
        }
        let Some(content) = item.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        if !content.is_empty() {
            content.clear();
            changed = true;
        }
    }
    changed
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

    #[test]
    fn codex_profile_only_empties_reasoning_content() {
        let body = json!({
            "input": [
                {
                    "type": "reasoning",
                    "content": [{"type": "reasoning_text", "text": "do not forward"}],
                    "encrypted_content": "opaque",
                    "summary": [{"type": "summary_text", "text": "summary"}]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "visible answer"}]
                }
            ]
        });
        let transformed = apply_responses_request_policy(
            &ProviderCompatibilityProfile::OpenAiCodex,
            body.to_string(),
        )
        .unwrap();
        let adapted: serde_json::Value = serde_json::from_str(&transformed).unwrap();

        assert_eq!(adapted["input"][0]["content"], json!([]));
        assert_eq!(adapted["input"][0]["encrypted_content"], "opaque");
        assert_eq!(adapted["input"][0]["summary"], body["input"][0]["summary"]);
        assert_eq!(adapted["input"][1], body["input"][1]);
    }

    #[test]
    fn codex_profile_preserves_raw_body_without_plaintext_reasoning() {
        let raw = "{ \"input\": [{\"type\":\"reasoning\",\"content\":[]}] }".to_string();
        let transformed =
            apply_responses_request_policy(&ProviderCompatibilityProfile::OpenAiCodex, raw.clone())
                .unwrap();

        assert_eq!(transformed, raw);
    }
}

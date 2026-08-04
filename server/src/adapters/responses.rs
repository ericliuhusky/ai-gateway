mod request_policy;

use crate::models::{ApiProviderRecord, ProviderAuthMode};
use request_policy::apply_responses_request_policy;

#[derive(Debug)]
pub enum ResponsesAdapterError {
    BadRequest(String),
}

#[derive(Clone, Debug)]
pub struct ResponsesAdapterProvider {
    pub name: String,
    pub auth_mode: ProviderAuthMode,
    pub record: Option<ApiProviderRecord>,
    pub uses_openai_account: bool,
}

#[derive(Debug)]
pub enum PreparedResponsesUpstream {
    OpenAiAccountResponsesPassthrough(PreparedResponsesPassthrough),
    ApiResponsesPassthrough(PreparedApiResponsesPassthrough),
}

#[derive(Debug)]
pub struct PreparedResponsesPassthrough {
    pub request_stream: bool,
    pub request_body: String,
}

#[derive(Debug)]
pub struct PreparedApiResponsesPassthrough {
    pub provider: ApiProviderRecord,
    pub request_stream: bool,
    pub request_body: String,
}

pub fn prepare_responses_upstream(
    provider: ResponsesAdapterProvider,
    request_body: String,
    request_stream: bool,
) -> Result<PreparedResponsesUpstream, ResponsesAdapterError> {
    if provider.auth_mode == ProviderAuthMode::Account && provider.uses_openai_account {
        let request_body = apply_responses_request_policy(
            &crate::models::ProviderCompatibilityProfile::OpenAiCodex,
            request_body,
        )
        .map_err(ResponsesAdapterError::BadRequest)?;
        return Ok(
            PreparedResponsesUpstream::OpenAiAccountResponsesPassthrough(
                PreparedResponsesPassthrough {
                    request_stream,
                    request_body,
                },
            ),
        );
    }

    if provider.auth_mode == ProviderAuthMode::Account {
        return Err(ResponsesAdapterError::BadRequest(format!(
            "account auth provider is not supported yet: {}",
            provider.name
        )));
    }

    let native_provider = provider.record.ok_or_else(|| {
        ResponsesAdapterError::BadRequest(format!("unknown provider: {}", provider.name))
    })?;

    let transformed =
        apply_responses_request_policy(&native_provider.compatibility_profile, request_body)
            .map_err(ResponsesAdapterError::BadRequest)?;
    Ok(PreparedResponsesUpstream::ApiResponsesPassthrough(
        PreparedApiResponsesPassthrough {
            provider: native_provider,
            request_stream,
            request_body: transformed,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::{PreparedResponsesUpstream, ResponsesAdapterProvider, prepare_responses_upstream};
    use crate::models::{
        ApiProviderRecord, ProviderAuthMode, ProviderCompatibilityProfile, ProviderUpstreamProtocol,
    };
    use serde_json::json;

    fn api_provider(
        base_url: &str,
        compatibility_profile: ProviderCompatibilityProfile,
    ) -> ApiProviderRecord {
        ApiProviderRecord {
            id: "provider-123".to_string(),
            name: "custom-compatible".to_string(),
            auth_mode: ProviderAuthMode::ApiKey,
            base_url: base_url.to_string(),
            api_key: "sk-test".to_string(),
            account_id: None,
            upstream_protocol: ProviderUpstreamProtocol::OpenAiResponses,
            compatibility_profile,
            owner_user_id: None,
        }
    }

    fn response_body(model: &str) -> serde_json::Value {
        json!({
            "model": model,
            "instructions": "",
            "input": [],
            "tools": [],
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "reasoning": null,
            "store": false,
            "stream": true,
            "include": []
        })
    }

    #[test]
    fn api_provider_uses_responses_by_default_even_for_compatible_provider_name() {
        let provider = api_provider(
            "https://example.com/v1",
            ProviderCompatibilityProfile::GenericOpenAi,
        );
        let prepared = prepare_responses_upstream(
            ResponsesAdapterProvider {
                name: provider.name.clone(),
                auth_mode: provider.auth_mode.clone(),
                record: Some(provider),
                uses_openai_account: false,
            },
            response_body("gpt-5.4").to_string(),
            true,
        )
        .unwrap();

        let PreparedResponsesUpstream::ApiResponsesPassthrough(_) = prepared else {
            panic!("expected responses passthrough");
        };
    }

    #[test]
    fn filters_known_incompatible_response_tools_for_non_openai_provider() {
        let provider = api_provider(
            "https://example.com/v1",
            ProviderCompatibilityProfile::GenericOpenAi,
        );
        let body = json!({
            "model": "external/gpt-5.5",
            "tools": [
                {
                    "type": "function",
                    "name": "list_available_plugins_to_install",
                    "description": "# List plugin/connector install candidates",
                    "parameters": {},
                    "strict": false
                },
                {
                    "type": "function",
                    "name": "exec_command",
                    "description": "Runs a command",
                    "parameters": { "type": "object", "properties": {} }
                },
                {
                    "type": "function",
                    "name": "get_goal",
                    "description": "Get the current goal",
                    "parameters": {},
                    "strict": false
                },
                {
                    "type": "namespace",
                    "name": "list_available_plugins_to_install",
                    "description": "Do not filter non-function tools",
                    "tools": []
                },
                {
                    "type": "tool_search",
                    "execution": "client",
                    "description": "Search deferred tools",
                    "parameters": { "type": "object", "properties": {} }
                }
            ]
        });

        let prepared = prepare_responses_upstream(
            ResponsesAdapterProvider {
                name: provider.name.clone(),
                auth_mode: provider.auth_mode.clone(),
                record: Some(provider),
                uses_openai_account: false,
            },
            body.to_string(),
            false,
        )
        .unwrap();

        let PreparedResponsesUpstream::ApiResponsesPassthrough(prepared) = prepared else {
            panic!("expected responses passthrough");
        };
        let adapted: serde_json::Value = serde_json::from_str(&prepared.request_body).unwrap();
        let tools = adapted["tools"].as_array().unwrap();

        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "exec_command");
        assert_eq!(tools[1]["type"], "namespace");
    }

    #[test]
    fn keeps_response_tools_for_official_openai_api_key_provider() {
        let provider = api_provider(
            "https://api.openai.com/v1",
            ProviderCompatibilityProfile::OfficialOpenAi,
        );
        let body = json!({
            "model": "gpt-5.4",
            "tools": [
                {
                    "type": "function",
                    "name": "list_available_plugins_to_install",
                    "description": "# List plugin/connector install candidates",
                    "parameters": {},
                    "strict": false
                }
            ]
        });

        let prepared = prepare_responses_upstream(
            ResponsesAdapterProvider {
                name: provider.name.clone(),
                auth_mode: provider.auth_mode.clone(),
                record: Some(provider),
                uses_openai_account: false,
            },
            body.to_string(),
            false,
        )
        .unwrap();

        let PreparedResponsesUpstream::ApiResponsesPassthrough(prepared) = prepared else {
            panic!("expected responses passthrough");
        };
        let adapted: serde_json::Value = serde_json::from_str(&prepared.request_body).unwrap();

        assert_eq!(adapted["tools"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn removes_non_replayable_reasoning_for_openai_account_requests() {
        let body = json!({
            "model": "gpt-test",
            "input": [
                {
                    "type": "message",
                    "role": "developer",
                    "content": [{"type": "input_text", "text": "context"}]
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "first"}]
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "second"}]
                },
                {
                    "type": "reasoning",
                    "content": [{"type": "reasoning_text", "text": "private reasoning"}],
                    "encrypted_content": null,
                    "summary": []
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "hi"}]
                }
            ]
        });

        let prepared = prepare_responses_upstream(
            ResponsesAdapterProvider {
                name: "GPT账户".to_string(),
                auth_mode: ProviderAuthMode::Account,
                record: None,
                uses_openai_account: true,
            },
            body.to_string(),
            true,
        )
        .unwrap();

        let PreparedResponsesUpstream::OpenAiAccountResponsesPassthrough(prepared) = prepared
        else {
            panic!("expected OpenAI account passthrough");
        };
        let adapted: serde_json::Value = serde_json::from_str(&prepared.request_body).unwrap();

        assert_eq!(adapted["input"].as_array().unwrap().len(), 4);
        assert_eq!(
            adapted["input"][0]["content"][0]["text"],
            body["input"][0]["content"][0]["text"]
        );
        assert_eq!(
            adapted["input"][3]["content"][0]["text"],
            body["input"][4]["content"][0]["text"]
        );
    }
}

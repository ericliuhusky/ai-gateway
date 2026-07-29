mod request_policy;

use crate::models::{
    ApiProviderRecord, ProviderAuthMode, ProviderUpstreamProtocol, ResponseCreateParams,
    responses_to_chat_completions,
};
use request_policy::apply_responses_request_policy;
use serde_json::Value;

#[derive(Debug)]
pub enum ResponsesAdapterError {
    BadRequest(String),
    Internal(String),
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
    ApiChatCompletions(PreparedApiChatCompletions),
    ApiResponsesPassthrough(PreparedApiResponsesPassthrough),
}

#[derive(Debug)]
pub struct PreparedResponsesPassthrough {
    pub request_stream: bool,
    pub request_body: String,
}

#[derive(Debug)]
pub struct PreparedApiChatCompletions {
    pub provider_name: String,
    pub provider: ApiProviderRecord,
    pub model: String,
    pub request_body: Value,
}

#[derive(Debug)]
pub struct PreparedApiResponsesPassthrough {
    pub provider: ApiProviderRecord,
    pub request_stream: bool,
    pub request_body: String,
}

#[derive(Clone, Debug)]
struct NativeTarget {
    upstream_model: String,
    uses_chat_completions: bool,
}

pub fn prepare_responses_upstream(
    provider: ResponsesAdapterProvider,
    request_json: Value,
    request_body: String,
    requested_model: String,
    request_stream: bool,
) -> Result<PreparedResponsesUpstream, ResponsesAdapterError> {
    if provider.auth_mode == ProviderAuthMode::Account && provider.uses_openai_account {
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

    let native_target = resolve_native_target(&native_provider, &requested_model);
    if native_target.uses_chat_completions {
        let request: ResponseCreateParams =
            serde_json::from_value(request_json).map_err(|err| {
                ResponsesAdapterError::BadRequest(format!("invalid request JSON: {err}"))
            })?;
        if !request.stream {
            return Err(ResponsesAdapterError::BadRequest(
                "responses 接口请求必须使用流式 (\"stream\": true)".to_string(),
            ));
        }
        let chat_request = responses_to_chat_completions(&request, &native_target.upstream_model)
            .map_err(ResponsesAdapterError::BadRequest)?;
        let request_body = serde_json::to_value(chat_request)
            .map_err(|err| ResponsesAdapterError::Internal(err.to_string()))?;

        return Ok(PreparedResponsesUpstream::ApiChatCompletions(
            PreparedApiChatCompletions {
                provider_name: provider.name,
                provider: native_provider.clone(),
                model: request.model,
                request_body,
            },
        ));
    }

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

fn resolve_native_target(provider: &ApiProviderRecord, requested_model: &str) -> NativeTarget {
    if provider.upstream_protocol == ProviderUpstreamProtocol::OpenAiChatCompletions {
        return NativeTarget {
            upstream_model: requested_model.to_string(),
            uses_chat_completions: true,
        };
    }

    NativeTarget {
        upstream_model: requested_model.to_string(),
        uses_chat_completions: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{PreparedResponsesUpstream, ResponsesAdapterProvider, prepare_responses_upstream};
    use crate::models::{
        ApiProviderBillingMode, ApiProviderRecord, ProviderAuthMode, ProviderCompatibilityProfile,
        ProviderUpstreamProtocol,
    };
    use serde_json::json;

    fn api_provider(
        base_url: &str,
        upstream_protocol: ProviderUpstreamProtocol,
        compatibility_profile: ProviderCompatibilityProfile,
    ) -> ApiProviderRecord {
        ApiProviderRecord {
            id: "provider-123".to_string(),
            name: "custom-compatible".to_string(),
            auth_mode: ProviderAuthMode::ApiKey,
            base_url: base_url.to_string(),
            api_key: "sk-test".to_string(),
            account_id: None,
            upstream_protocol,
            compatibility_profile,
            billing_mode: ApiProviderBillingMode::Metered,
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
            ProviderUpstreamProtocol::OpenAiResponses,
            ProviderCompatibilityProfile::GenericOpenAi,
        );
        let prepared = prepare_responses_upstream(
            ResponsesAdapterProvider {
                name: provider.name.clone(),
                auth_mode: provider.auth_mode.clone(),
                record: Some(provider),
                uses_openai_account: false,
            },
            response_body("gpt-5.4"),
            response_body("gpt-5.4").to_string(),
            "gpt-5.4".to_string(),
            true,
        )
        .unwrap();

        let PreparedResponsesUpstream::ApiResponsesPassthrough(_) = prepared else {
            panic!("expected responses passthrough");
        };
    }

    #[test]
    fn api_provider_uses_chat_completions_only_when_enabled() {
        let provider = api_provider(
            "https://example.com/v1",
            ProviderUpstreamProtocol::OpenAiChatCompletions,
            ProviderCompatibilityProfile::GenericOpenAi,
        );
        let prepared = prepare_responses_upstream(
            ResponsesAdapterProvider {
                name: provider.name.clone(),
                auth_mode: provider.auth_mode.clone(),
                record: Some(provider),
                uses_openai_account: false,
            },
            response_body("qwen3-32b"),
            response_body("qwen3-32b").to_string(),
            "qwen3-32b".to_string(),
            true,
        )
        .unwrap();

        let PreparedResponsesUpstream::ApiChatCompletions(prepared) = prepared else {
            panic!("expected chat completions");
        };
        assert_eq!(prepared.model, "qwen3-32b");
    }

    #[test]
    fn filters_known_incompatible_response_tools_for_non_openai_provider() {
        let provider = api_provider(
            "https://example.com/v1",
            ProviderUpstreamProtocol::OpenAiResponses,
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
            body.clone(),
            body.to_string(),
            "external/gpt-5.5".to_string(),
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
            ProviderUpstreamProtocol::OpenAiResponses,
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
            body.clone(),
            body.to_string(),
            "gpt-5.4".to_string(),
            false,
        )
        .unwrap();

        let PreparedResponsesUpstream::ApiResponsesPassthrough(prepared) = prepared else {
            panic!("expected responses passthrough");
        };
        let adapted: serde_json::Value = serde_json::from_str(&prepared.request_body).unwrap();

        assert_eq!(adapted["tools"].as_array().unwrap().len(), 1);
    }
}

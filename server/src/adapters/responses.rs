use crate::{
    config::Config,
    models::{
        ApiProviderRecord, ProviderAuthMode, ResponseCreateParams, UpstreamProtocol,
        responses_to_chat_completions,
    },
    upstream::{chat_completions_api_url, responses_api_url},
};
use reqwest::Url;
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
    pub provider_name: String,
    pub model: String,
    pub request_stream: bool,
    pub upstream_protocol: UpstreamProtocol,
    pub upstream_url: String,
    pub request_body: String,
}

#[derive(Debug)]
pub struct PreparedApiChatCompletions {
    pub provider_name: String,
    pub provider: ApiProviderRecord,
    pub model: String,
    pub stream: bool,
    pub upstream_protocol: UpstreamProtocol,
    pub upstream_url: String,
    pub request_body: Value,
}

#[derive(Debug)]
pub struct PreparedApiResponsesPassthrough {
    pub provider_name: String,
    pub provider: ApiProviderRecord,
    pub model: String,
    pub request_stream: bool,
    pub upstream_protocol: UpstreamProtocol,
    pub upstream_url: String,
    pub request_body: String,
}

#[derive(Clone, Debug)]
struct NativeTarget {
    upstream_model: String,
    upstream: UpstreamProtocol,
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
                    provider_name: provider.name,
                    model: requested_model,
                    request_stream,
                    upstream_protocol: UpstreamProtocol::OpenAiPrivateResponses,
                    upstream_url: Config::openai_private_responses_url().to_string(),
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
                stream: request.stream,
                upstream_protocol: native_target.upstream,
                upstream_url: chat_completions_api_url(&native_provider.base_url),
                request_body,
            },
        ));
    }

    let upstream_url = responses_api_url(&native_provider.base_url);
    let request_body = adapt_native_responses_passthrough_body(&native_provider, request_body)?;
    Ok(PreparedResponsesUpstream::ApiResponsesPassthrough(
        PreparedApiResponsesPassthrough {
            provider_name: provider.name,
            provider: native_provider,
            model: requested_model,
            request_stream,
            upstream_protocol: native_target.upstream,
            upstream_url,
            request_body,
        },
    ))
}

fn resolve_native_target(provider: &ApiProviderRecord, requested_model: &str) -> NativeTarget {
    if provider.uses_chat_completions {
        return NativeTarget {
            upstream_model: requested_model.to_string(),
            upstream: UpstreamProtocol::NativeChatCompletions,
            uses_chat_completions: true,
        };
    }

    NativeTarget {
        upstream_model: requested_model.to_string(),
        upstream: UpstreamProtocol::NativeResponses,
        uses_chat_completions: false,
    }
}

const NON_OPENAI_RESPONSES_FILTERED_FUNCTION_TOOLS: &[&str] =
    &["list_available_plugins_to_install"];

fn adapt_native_responses_passthrough_body(
    provider: &ApiProviderRecord,
    request_body: String,
) -> Result<String, ResponsesAdapterError> {
    if is_official_openai_api_key_provider(provider) {
        return Ok(request_body);
    }

    let mut body: Value = serde_json::from_str(&request_body)
        .map_err(|err| ResponsesAdapterError::BadRequest(format!("invalid request JSON: {err}")))?;
    filter_non_openai_responses_tools(&mut body);
    serde_json::to_string(&body).map_err(|err| ResponsesAdapterError::Internal(err.to_string()))
}

fn is_official_openai_api_key_provider(provider: &ApiProviderRecord) -> bool {
    Url::parse(provider.base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .as_deref()
        == Some("api.openai.com")
}

fn filter_non_openai_responses_tools(body: &mut Value) {
    let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) else {
        return;
    };

    tools.retain(|tool| {
        let should_filter = tool.get("type").and_then(Value::as_str) == Some("function")
            && tool
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| NON_OPENAI_RESPONSES_FILTERED_FUNCTION_TOOLS.contains(&name));
        !should_filter
    });
}

#[cfg(test)]
mod tests {
    use super::{PreparedResponsesUpstream, ResponsesAdapterProvider, prepare_responses_upstream};
    use crate::models::{
        ApiProviderBillingMode, ApiProviderRecord, ProviderAuthMode, UpstreamProtocol,
    };
    use serde_json::json;

    fn api_provider(base_url: &str, uses_chat_completions: bool) -> ApiProviderRecord {
        ApiProviderRecord {
            id: "provider-123".to_string(),
            name: "custom-compatible".to_string(),
            auth_mode: ProviderAuthMode::ApiKey,
            base_url: base_url.to_string(),
            api_key: "sk-test".to_string(),
            account_id: None,
            uses_chat_completions,
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
        let provider = api_provider("https://example.com/v1", false);
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

        let PreparedResponsesUpstream::ApiResponsesPassthrough(prepared) = prepared else {
            panic!("expected responses passthrough");
        };
        assert_eq!(
            prepared.upstream_protocol,
            UpstreamProtocol::NativeResponses
        );
    }

    #[test]
    fn api_provider_uses_chat_completions_only_when_enabled() {
        let provider = api_provider("https://example.com/v1", true);
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
        assert_eq!(
            prepared.upstream_protocol,
            UpstreamProtocol::NativeChatCompletions
        );
        assert_eq!(prepared.model, "qwen3-32b");
    }

    #[test]
    fn filters_known_incompatible_response_tool_for_non_openai_provider() {
        let provider = api_provider("https://example.com/v1", false);
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
                    "type": "namespace",
                    "name": "list_available_plugins_to_install",
                    "description": "Do not filter non-function tools",
                    "tools": []
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
        let provider = api_provider("https://api.openai.com/v1", false);
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

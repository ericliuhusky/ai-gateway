use super::{ContentItem, ResponseItem};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub type ResponseEvents = Vec<ResponseEvent>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ResponseEvent {
    Created,
    OutputItemDone(ResponseItem),
    OutputItemAdded(ResponseItem),
    /// Emitted when the server includes `OpenAI-Model` on the stream response.
    /// This can differ from the requested model when backend safety routing applies.
    ServerModel(String),
    /// Emitted when the server recommends additional account verification.
    ModelVerifications(Vec<ModelVerification>),
    /// Emitted when `X-Reasoning-Included: true` is present on the response,
    /// meaning the server already accounted for past reasoning tokens and the
    /// client should not re-estimate them.
    ServerReasoningIncluded(bool),
    Completed {
        response_id: String,
        token_usage: Option<TokenUsage>,
        /// Did the model affirmatively end its turn? Some providers do not set this,
        /// so we rely on fallback logic when this is `None`.
        end_turn: Option<bool>,
    },
    OutputTextDelta(String),
    ToolCallInputDelta {
        item_id: String,
        call_id: Option<String>,
        delta: String,
    },
    ReasoningSummaryDelta {
        delta: String,
        summary_index: i64,
    },
    ReasoningContentDelta {
        delta: String,
        content_index: i64,
    },
    ReasoningSummaryPartAdded {
        summary_index: i64,
    },
    RateLimits(RateLimitSnapshot),
    ModelsEtag(String),
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reasoning_output_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelVerification {
    TrustedAccessForCyber,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RateLimitSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<RateLimitWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<RateLimitWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<CreditsSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_reached_type: Option<RateLimitReachedType>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RateLimitWindow {
    pub used_percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CreditsSnapshot {
    pub has_credits: bool,
    pub unlimited: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitReachedType {
    RateLimitReached,
    WorkspaceOwnerCreditsDepleted,
    WorkspaceMemberCreditsDepleted,
    WorkspaceOwnerUsageLimitReached,
    WorkspaceMemberUsageLimitReached,
}

pub fn response_events_to_response_value(
    events: &[ResponseEvent],
    model: &str,
    created_at: u64,
) -> Value {
    let response_id = response_id_from_events(events);
    let output = completed_output_items(events);
    let usage = token_usage_from_events(events);

    json!({
        "id": response_id,
        "object": "response",
        "created_at": created_at,
        "status": "completed",
        "model": model,
        "output": output,
        "usage": usage,
    })
}

pub fn response_events_to_sse_lines(
    events: &[ResponseEvent],
    model: &str,
    created_at: u64,
) -> Result<Vec<String>, serde_json::Error> {
    let response_id = response_id_from_events(events);
    let output = completed_output_items(events);
    let mut lines = Vec::new();

    lines.push(encode_sse_value(&json!({
        "type": "response.created",
        "response": {
            "id": response_id,
            "object": "response",
            "status": "in_progress",
            "output": []
        }
    }))?);

    for (index, item) in output.iter().enumerate() {
        let item_id = response_item_id(item, index);
        lines.push(encode_sse_value(&json!({
            "type": "response.output_item.added",
            "output_index": index,
            "item": item
        }))?);

        if response_item_type(item) == Some("message") {
            for (content_index, part) in response_item_output_text_parts(item).iter().enumerate() {
                lines.push(encode_sse_value(&json!({
                    "type": "response.content_part.added",
                    "item_id": item_id,
                    "output_index": index,
                    "content_index": content_index,
                    "part": part
                }))?);
                lines.push(encode_sse_value(&json!({
                    "type": "response.output_text.delta",
                    "item_id": item_id,
                    "output_index": index,
                    "content_index": content_index,
                    "delta": part["text"]
                }))?);
                lines.push(encode_sse_value(&json!({
                    "type": "response.output_text.done",
                    "item_id": item_id,
                    "output_index": index,
                    "content_index": content_index,
                    "text": part["text"]
                }))?);
                lines.push(encode_sse_value(&json!({
                    "type": "response.content_part.done",
                    "item_id": item_id,
                    "output_index": index,
                    "content_index": content_index,
                    "part": part
                }))?);
            }
        }

        lines.push(encode_sse_value(&json!({
            "type": "response.output_item.done",
            "output_index": index,
            "item": item
        }))?);
    }

    lines.push(encode_sse_value(&json!({
        "type": "response.completed",
        "response": response_events_to_response_value(events, model, created_at)
    }))?);
    lines.push("data: [DONE]\n\n".to_string());

    Ok(lines)
}

fn response_id_from_events(events: &[ResponseEvent]) -> String {
    events
        .iter()
        .find_map(|event| match event {
            ResponseEvent::Completed { response_id, .. } => Some(response_id.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "resp_unknown".to_string())
}

fn completed_output_items(events: &[ResponseEvent]) -> Vec<ResponseItem> {
    events
        .iter()
        .filter_map(|event| match event {
            ResponseEvent::OutputItemDone(item) => Some(item.clone()),
            _ => None,
        })
        .collect()
}

fn token_usage_from_events(events: &[ResponseEvent]) -> Option<TokenUsage> {
    events.iter().find_map(|event| match event {
        ResponseEvent::Completed { token_usage, .. } => token_usage.clone(),
        _ => None,
    })
}

fn encode_sse_value(value: &Value) -> Result<String, serde_json::Error> {
    serde_json::to_string(value).map(|body| format!("data: {body}\n\n"))
}

fn response_item_type(item: &ResponseItem) -> Option<&'static str> {
    match item {
        ResponseItem::Message { .. } => Some("message"),
        ResponseItem::FunctionCall { .. } => Some("function_call"),
        ResponseItem::LocalShellCall { .. } => Some("local_shell_call"),
        ResponseItem::ToolSearchCall { .. } => Some("tool_search_call"),
        ResponseItem::FunctionCallOutput { .. } => Some("function_call_output"),
        ResponseItem::CustomToolCall { .. } => Some("custom_tool_call"),
        ResponseItem::CustomToolCallOutput { .. } => Some("custom_tool_call_output"),
        ResponseItem::ToolSearchOutput { .. } => Some("tool_search_output"),
        ResponseItem::WebSearchCall { .. } => Some("web_search_call"),
        ResponseItem::ImageGenerationCall { .. } => Some("image_generation_call"),
        ResponseItem::Reasoning { .. } => Some("reasoning"),
        ResponseItem::Compaction { .. } => Some("compaction"),
        ResponseItem::ContextCompaction { .. } => Some("context_compaction"),
        ResponseItem::Other => None,
    }
}

fn response_item_id(item: &ResponseItem, index: usize) -> String {
    match item {
        ResponseItem::Message { id, .. } => id.clone(),
        ResponseItem::FunctionCall { id, .. } => id.clone(),
        ResponseItem::LocalShellCall { id, .. } => id.clone(),
        ResponseItem::ToolSearchCall { id, .. } => id.clone(),
        ResponseItem::CustomToolCall { id, .. } => id.clone(),
        ResponseItem::WebSearchCall { id, .. } => id.clone(),
        ResponseItem::ImageGenerationCall { id, .. } => Some(id.clone()),
        ResponseItem::Reasoning { id, .. } => Some(id.clone()),
        ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::ContextCompaction { .. }
        | ResponseItem::Other => None,
    }
    .unwrap_or_else(|| format!("item_{index}"))
}

fn response_item_output_text_parts(item: &ResponseItem) -> Vec<Value> {
    let ResponseItem::Message { content, .. } = item else {
        return Vec::new();
    };

    content
        .iter()
        .filter_map(|content| match content {
            ContentItem::OutputText { text } => Some(json!({
                "type": "output_text",
                "text": text
            })),
            _ => None,
        })
        .collect()
}

fn is_zero(value: &i64) -> bool {
    *value == 0
}

#[cfg(test)]
mod public_responses_entry_compat_tests {
    use crate::models::request::{ResponsesRequest, merge_strict_responses_request_defaults};
    use serde_json::{Value, json};

    fn public_entry_roundtrip(value: Value) -> Value {
        let value = merge_strict_responses_request_defaults(value);
        let request: ResponsesRequest =
            serde_json::from_value(value).expect("public responses request should parse");
        serde_json::to_value(request).expect("public responses request should serialize")
    }

    fn tool_types(body: &Value) -> Vec<&str> {
        body["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .filter_map(|tool| tool["type"].as_str())
            .collect()
    }

    #[test]
    fn accepts_public_non_streaming_multimodal_and_tool_super_request() {
        let body = public_entry_roundtrip(json!({
            "model": "MODEL_ID",
            "instructions": "You are a helpful multimodal agent. Follow the user request, use tools when useful, and return structured output when requested.",
            "input": [
                {
                    "type": "message",
                    "role": "developer",
                    "content": [{
                        "type": "input_text",
                        "text": "Developer-level instruction for this request."
                    }]
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": "Analyze the text, images, and files. Use tools if needed." },
                        { "type": "input_image", "detail": "auto", "image_url": "https://example.com/image.png" },
                        { "type": "input_image", "detail": "high", "image_url": "data:image/png;base64,BASE64_IMAGE_DATA" },
                        { "type": "input_image", "detail": "original", "file_id": "file_image_123" },
                        {
                            "type": "input_file",
                            "filename": "report.pdf",
                            "file_data": "data:application/pdf;base64,BASE64_PDF_DATA",
                            "detail": "high"
                        },
                        { "type": "input_file", "file_id": "file_pdf_123", "detail": "low" },
                        { "type": "input_file", "filename": "spec.docx", "file_url": "https://example.com/spec.docx" }
                    ]
                }
            ],
            "tools": [
                {
                    "type": "function",
                    "name": "get_weather",
                    "description": "Get weather for a location.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "location": { "type": "string" },
                            "unit": { "type": "string", "enum": ["celsius", "fahrenheit"] }
                        },
                        "required": ["location"],
                        "additionalProperties": false
                    },
                    "strict": true,
                    "defer_loading": false
                },
                {
                    "type": "web_search",
                    "filters": { "allowed_domains": ["example.com"] },
                    "search_context_size": "medium",
                    "user_location": {
                        "type": "approximate",
                        "country": "US",
                        "region": "California",
                        "city": "San Francisco",
                        "timezone": "America/Los_Angeles"
                    }
                },
                {
                    "type": "image_generation",
                    "model": "gpt-image-1.5",
                    "action": "auto",
                    "background": "auto",
                    "input_fidelity": "high",
                    "input_image_mask": { "image_url": "data:image/png;base64,BASE64_MASK_IMAGE" },
                    "moderation": "auto",
                    "output_compression": 90,
                    "output_format": "png",
                    "partial_images": 0,
                    "quality": "auto",
                    "size": "auto"
                },
                {
                    "type": "custom",
                    "name": "freeform_transformer",
                    "description": "Accepts freeform text input.",
                    "format": { "type": "text" },
                    "defer_loading": false
                },
                { "type": "local_shell" },
                {
                    "type": "tool_search",
                    "execution": "search_tools",
                    "description": "Search available tools.",
                    "parameters": {
                        "type": "object",
                        "properties": { "query": { "type": "string" } },
                        "required": ["query"]
                    }
                },
                {
                    "type": "namespace",
                    "name": "example_namespace",
                    "description": "Grouped example tools.",
                    "tools": [{
                        "type": "function",
                        "name": "lookup",
                        "description": "Look up an item.",
                        "parameters": {
                            "type": "object",
                            "properties": { "id": { "type": "string" } },
                            "required": ["id"]
                        }
                    }]
                }
            ],
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "reasoning": { "effort": "medium", "summary": "auto" },
            "text": {
                "verbosity": "medium",
                "format": {
                    "type": "json_schema",
                    "name": "response_schema",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": {
                            "answer": { "type": "string" },
                            "used_tools": { "type": "array", "items": { "type": "string" } },
                            "files": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["answer", "used_tools", "files"],
                        "additionalProperties": false
                    }
                }
            },
            "include": [
                "web_search_call.action.sources",
                "code_interpreter_call.outputs",
                "computer_call_output.output.image_url",
                "file_search_call.results",
                "message.input_image.image_url",
                "message.output_text.logprobs",
                "reasoning.encrypted_content"
            ],
            "stream": false,
            "store": false,
            "prompt_cache_key": "stable-cache-key"
        }));

        assert_eq!(body["model"], "MODEL_ID");
        assert_eq!(body["stream"], false);
        assert_eq!(body["store"], false);
        assert_eq!(body["parallel_tool_calls"], true);
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        assert_eq!(body["include"].as_array().expect("include array").len(), 7);
        assert!(body.get("max_output_tokens").is_none());
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
        assert_eq!(body["prompt_cache_key"], "stable-cache-key");
        assert!(body.get("background").is_none());
        assert!(body.get("context_management").is_none());
        assert!(body.get("max_tool_calls").is_none());
        assert!(body.get("top_logprobs").is_none());
        assert!(body.get("truncation").is_none());
        assert!(body.get("service_tier").is_none());
        assert!(body.get("prompt_cache_retention").is_none());
        assert!(body.get("metadata").is_none());
        assert!(body.get("safety_identifier").is_none());
        assert!(body.get("user").is_none());

        let types = tool_types(&body);
        for expected in [
            "function",
            "web_search",
            "image_generation",
            "custom",
            "local_shell",
            "tool_search",
            "namespace",
        ] {
            assert!(types.contains(&expected), "missing tool type {expected}");
        }

        assert_eq!(
            body["input"][1]["content"][1]["image_url"],
            "https://example.com/image.png"
        );
        assert_eq!(body["input"][1]["content"][1]["detail"], "auto");
        assert_eq!(body["input"][1]["content"][3]["file_id"], "file_image_123");
        assert_eq!(
            body["input"][1]["content"][4]["file_data"],
            "data:application/pdf;base64,BASE64_PDF_DATA"
        );
        assert_eq!(
            body["input"][1]["content"][6]["file_url"],
            "https://example.com/spec.docx"
        );
    }

    #[test]
    fn accepts_public_streaming_conversation_and_prompt_requests() {
        let streaming = public_entry_roundtrip(json!({
            "model": "MODEL_ID",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "Stream the answer." }]
            }],
            "stream": true,
            "tools": [{ "type": "web_search", "search_context_size": "low" }],
            "tool_choice": "auto",
            "include": ["web_search_call.action.sources", "reasoning.encrypted_content"],
            "store": false
        }));
        assert_eq!(streaming["stream"], true);
        assert!(streaming.get("stream_options").is_none());
        assert_eq!(streaming["include"][1], "reasoning.encrypted_content");

        let conversation_object = public_entry_roundtrip(json!({
            "model": "MODEL_ID",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "Continue this conversation." }]
            }],
            "store": true
        }));
        assert!(conversation_object.get("conversation").is_none());
        assert_eq!(conversation_object["store"], true);

        let conversation_string = public_entry_roundtrip(json!({
            "model": "MODEL_ID",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "Continue this conversation." }]
            }]
        }));
        assert!(conversation_string.get("conversation").is_none());
        assert_eq!(conversation_string["input"][0]["type"], "message");
        assert_eq!(conversation_string["input"][0]["role"], "user");
        assert_eq!(
            conversation_string["input"][0]["content"][0]["text"],
            "Continue this conversation."
        );

        let prompt_template = public_entry_roundtrip(json!({
            "model": "MODEL_ID",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "Use the prompt template and complete the task." }]
            }],
            "store": false
        }));
        assert!(prompt_template.get("prompt").is_none());
        assert_eq!(prompt_template["store"], false);
    }

    #[test]
    fn accepts_public_agent_loop_tool_outputs_and_output_item_replay() {
        let body = public_entry_roundtrip(json!({
            "model": "MODEL_ID",
            "input": [
                {
                    "type": "function_call_output",
                    "call_id": "call_function_123",
                    "output": [
                        { "type": "output_text", "text": "Function result text." },
                        { "type": "input_image", "detail": "auto", "image_url": "data:image/png;base64,BASE64_TOOL_IMAGE" },
                        { "type": "input_file", "filename": "result.json", "file_data": "data:application/json;base64,BASE64_JSON" }
                    ],
                    "status": "completed"
                },
                {
                    "type": "computer_call_output",
                    "call_id": "call_computer_123",
                    "output": { "type": "input_image", "image_url": "data:image/png;base64,BASE64_SCREENSHOT" },
                    "acknowledged_safety_checks": [{
                        "id": "safety_check_123",
                        "code": "download",
                        "message": "User approved this action."
                    }],
                    "status": "completed"
                },
                {
                    "type": "shell_call_output",
                    "call_id": "call_shell_123",
                    "output": [{ "type": "output_text", "text": "stdout and stderr content" }],
                    "max_output_length": 20000,
                    "status": "completed"
                },
                {
                    "type": "apply_patch_call_output",
                    "call_id": "call_patch_123",
                    "status": "completed",
                    "output": "Patch applied successfully."
                },
                {
                    "type": "mcp_approval_response",
                    "approval_request_id": "approval_123",
                    "approve": true,
                    "reason": "Approved by user."
                },
                {
                    "type": "custom_tool_call_output",
                    "call_id": "call_custom_123",
                    "output": "Custom tool output text."
                },
                {
                    "type": "message",
                    "id": "msg_assistant_123",
                    "role": "assistant",
                    "status": "completed",
                    "phase": "final_answer",
                    "content": [{ "type": "output_text", "text": "Previous assistant answer." }]
                },
                {
                    "type": "function_call",
                    "id": "fc_123",
                    "call_id": "call_function_123",
                    "name": "get_weather",
                    "arguments": "{\"location\":\"San Francisco\"}",
                    "status": "completed"
                },
                {
                    "type": "image_generation_call",
                    "id": "ig_123",
                    "status": "completed",
                    "result": "BASE64_GENERATED_IMAGE"
                },
                {
                    "type": "code_interpreter_call",
                    "id": "ci_123",
                    "container_id": "cntr_123",
                    "code": "print('hello')",
                    "outputs": [
                        { "type": "logs", "logs": "hello\n" },
                        { "type": "image", "url": "https://example.com/chart.png" }
                    ],
                    "status": "completed"
                },
                {
                    "type": "mcp_call",
                    "id": "mcp_call_123",
                    "server_label": "github",
                    "name": "search_code",
                    "arguments": "{\"query\":\"Responses API\"}",
                    "output": "MCP result text.",
                    "status": "completed"
                },
                { "type": "item_reference", "id": "item_123" },
                {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "Continue from the replayed context." }]
                }
            ],
            "tools": [
                { "type": "function", "name": "get_weather", "description": "Get weather for a location.", "parameters": { "type": "object", "properties": { "location": { "type": "string" } }, "required": ["location"], "additionalProperties": false }, "strict": true },
                { "type": "local_shell" },
                { "type": "custom", "name": "freeform_transformer", "description": "Accepts freeform text input.", "format": { "type": "text" } }
            ],
            "tool_choice": "auto",
            "store": false
        }));

        assert_eq!(body["input"][0]["type"], "function_call_output");
        assert_eq!(body["input"][0]["output"][1]["type"], "input_image");
        assert_eq!(body["input"][1]["type"], "other");
        assert_eq!(body["input"][2]["type"], "other");
        assert_eq!(body["input"][3]["type"], "other");
        assert_eq!(body["input"][4]["type"], "other");
        assert_eq!(body["input"][5]["type"], "custom_tool_call_output");
        assert_eq!(body["input"][6]["phase"], "final_answer");
        assert_eq!(body["input"][8]["type"], "image_generation_call");
        assert_eq!(body["input"][9]["type"], "other");
        assert_eq!(body["input"][10]["type"], "other");
        assert_eq!(body["input"][11]["type"], "other");
    }

    #[test]
    fn accepts_public_tool_choice_union_and_text_format_variants() {
        for tool_choice in ["none", "auto", "required"] {
            let body = public_entry_roundtrip(json!({
                "model": "MODEL_ID",
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }],
                "tool_choice": tool_choice
            }));
            assert_eq!(body["tool_choice"], tool_choice);
        }

        for text in [
            json!({ "verbosity": "medium", "format": { "type": "text" } }),
            json!({ "verbosity": "medium", "format": { "type": "json_object" } }),
            json!({
                "verbosity": "medium",
                "format": {
                    "type": "json_schema",
                    "name": "result_schema",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": { "answer": { "type": "string" } },
                        "required": ["answer"],
                        "additionalProperties": false
                    }
                }
            }),
        ] {
            let body = public_entry_roundtrip(json!({
                "model": "MODEL_ID",
                "input": [{
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }],
                "text": text.clone()
            }));
            assert_eq!(body["text"], text);
        }
    }
}

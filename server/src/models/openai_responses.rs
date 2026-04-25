use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponsesRequest {
    pub model: String,
    #[serde(default)]
    pub input: Option<ResponsesInput>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub stream: bool,
    #[serde(rename = "max_output_tokens", alias = "max_tokens")]
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f64>,
    #[serde(rename = "top_p")]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub tools: Option<Vec<ResponseTool>>,
    #[serde(default)]
    #[allow(dead_code)]
    pub tool_choice: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResponsesInput {
    String(String),
    Array(Vec<ResponsesInputItem>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ResponsesInputItem {
    Message(ResponseMessageInput),
    Block(ResponsesInputBlock),
    FunctionCall(ResponseFunctionCallItem),
    LocalShellCall(ResponseLocalShellCallItem),
    WebSearchCall(ResponseWebSearchCallItem),
    FunctionCallOutput(ResponseFunctionCallOutputItem),
    CustomToolCallOutput(ResponseCustomToolCallOutputItem),
    Raw(Value),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseMessageInput {
    pub role: String,
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ResponseFunctionToolCall>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum ResponsesInputBlock {
    #[serde(rename = "input_text")]
    InputText {
        text: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
    #[serde(rename = "input_image")]
    InputImage {
        image_url: String,
        #[serde(flatten)]
        extra: Map<String, Value>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseFunctionToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseFunctionCallItem {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub item_type: String,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseLocalShellCallItem {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub item_type: String,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub action: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseWebSearchCallItem {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub item_type: String,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub action: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseFunctionCallOutputItem {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub item_type: String,
    pub call_id: String,
    pub output: Value,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseCustomToolCallOutputItem {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub item_type: String,
    pub call_id: String,
    pub output: Value,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Option<Value>,
    #[serde(default)]
    pub function: Option<Value>,
    #[serde(default)]
    pub tools: Option<Vec<ResponseTool>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponsesResponse {
    pub id: String,
    pub object: String,
    pub created_at: u64,
    pub status: String,
    pub model: String,
    pub output: Vec<ResponseOutputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponsesUsage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseOutputItem {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ResponseOutputContent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseOutputContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponsesUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}

#[cfg(test)]
mod public_responses_entry_compat_tests {
    use super::ResponsesRequest;
    use serde_json::{Value, json};

    fn public_entry_roundtrip(value: Value) -> Value {
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
                    "type": "file_search",
                    "vector_store_ids": ["vs_123"],
                    "filters": { "type": "eq", "key": "project", "value": "demo" },
                    "max_num_results": 20,
                    "ranking_options": {
                        "ranker": "auto",
                        "score_threshold": 0.1,
                        "hybrid_search": { "embedding_weight": 0.5, "text_weight": 0.5 }
                    }
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
                    "type": "web_search_preview",
                    "search_content_types": ["text", "image"],
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
                    "type": "code_interpreter",
                    "container": {
                        "type": "auto",
                        "file_ids": ["file_data_123"],
                        "memory_limit": "4g",
                        "network_policy": { "type": "disabled" }
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
                { "type": "computer" },
                { "type": "computer_use_preview", "display_width": 1024, "display_height": 768, "environment": "browser" },
                {
                    "type": "mcp",
                    "server_label": "github",
                    "server_url": "https://example.com/mcp",
                    "server_description": "Example MCP server",
                    "authorization": "Bearer OAUTH_TOKEN",
                    "headers": { "x-custom-header": "value" },
                    "allowed_tools": { "read_only": true, "tool_names": ["search_code", "read_file"] },
                    "require_approval": {
                        "always": { "read_only": false },
                        "never": { "tool_names": ["search_code", "read_file"] }
                    },
                    "defer_loading": false
                },
                {
                    "type": "mcp",
                    "server_label": "googledrive",
                    "connector_id": "connector_googledrive",
                    "allowed_tools": ["search", "fetch"],
                    "require_approval": "never"
                },
                {
                    "type": "custom",
                    "name": "freeform_transformer",
                    "description": "Accepts freeform text input.",
                    "format": { "type": "text" },
                    "defer_loading": false
                },
                { "type": "local_shell" },
                { "type": "shell", "environment": { "type": "local" } },
                { "type": "apply_patch" },
                { "type": "tool_search" },
                { "type": "namespace", "namespace": "example_namespace" }
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
            "background": false,
            "context_management": [{ "type": "compaction", "compact_threshold": 120000 }],
            "max_output_tokens": 8192,
            "max_tool_calls": 32,
            "temperature": 0.2,
            "top_p": 1,
            "top_logprobs": 5,
            "truncation": "auto",
            "service_tier": "auto",
            "prompt_cache_key": "stable-cache-key",
            "prompt_cache_retention": "24h",
            "metadata": { "app": "my-client", "feature": "responses-full-template" },
            "safety_identifier": "hashed_user_id_123",
            "user": "legacy_user_bucket"
        }));

        assert_eq!(body["model"], "MODEL_ID");
        assert_eq!(body["stream"], false);
        assert_eq!(body["store"], false);
        assert_eq!(body["parallel_tool_calls"], true);
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        assert_eq!(body["include"].as_array().expect("include array").len(), 7);
        assert_eq!(body["background"], false);
        assert_eq!(body["context_management"][0]["compact_threshold"], 120000);
        assert_eq!(body["max_output_tokens"], 8192);
        assert_eq!(body["max_tool_calls"], 32);
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["top_p"], 1.0);
        assert_eq!(body["top_logprobs"], 5);
        assert_eq!(body["truncation"], "auto");
        assert_eq!(body["service_tier"], "auto");
        assert_eq!(body["prompt_cache_key"], "stable-cache-key");
        assert_eq!(body["prompt_cache_retention"], "24h");
        assert_eq!(body["metadata"]["feature"], "responses-full-template");
        assert_eq!(body["safety_identifier"], "hashed_user_id_123");
        assert_eq!(body["user"], "legacy_user_bucket");

        let types = tool_types(&body);
        for expected in [
            "function",
            "file_search",
            "web_search",
            "web_search_preview",
            "code_interpreter",
            "image_generation",
            "computer",
            "computer_use_preview",
            "mcp",
            "custom",
            "local_shell",
            "shell",
            "apply_patch",
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
    fn accepts_public_streaming_previous_response_conversation_and_prompt_requests() {
        let streaming = public_entry_roundtrip(json!({
            "model": "MODEL_ID",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "Stream the answer." }]
            }],
            "stream": true,
            "stream_options": { "include_obfuscation": false },
            "tools": [{ "type": "web_search_preview", "search_context_size": "low" }],
            "tool_choice": "auto",
            "include": ["web_search_call.action.sources", "reasoning.encrypted_content"],
            "store": false
        }));
        assert_eq!(streaming["stream"], true);
        assert_eq!(streaming["stream_options"]["include_obfuscation"], false);
        assert_eq!(streaming["include"][1], "reasoning.encrypted_content");

        let previous_response = public_entry_roundtrip(json!({
            "model": "MODEL_ID",
            "previous_response_id": "resp_previous_123",
            "instructions": "You may replace or update prior system/developer instructions for this request.",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "Continue from the previous response and refine the answer." }]
            }],
            "include": ["reasoning.encrypted_content"],
            "store": false
        }));
        assert_eq!(
            previous_response["previous_response_id"],
            "resp_previous_123"
        );
        assert!(previous_response.get("conversation").is_none());

        let conversation_object = public_entry_roundtrip(json!({
            "model": "MODEL_ID",
            "conversation": { "id": "conv_123" },
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "Continue this conversation." }]
            }],
            "store": true
        }));
        assert_eq!(conversation_object["conversation"]["id"], "conv_123");
        assert_eq!(conversation_object["store"], true);

        let conversation_string = public_entry_roundtrip(json!({
            "model": "MODEL_ID",
            "conversation": "conv_123",
            "input": "Continue this conversation."
        }));
        assert_eq!(conversation_string["conversation"], "conv_123");
        assert_eq!(conversation_string["input"], "Continue this conversation.");

        let prompt_template = public_entry_roundtrip(json!({
            "model": "MODEL_ID",
            "prompt": {
                "id": "pmpt_123",
                "version": "1",
                "variables": {
                    "project": "codex",
                    "language": "zh-CN",
                    "task": "analyze repository"
                }
            },
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "Use the prompt template and complete the task." }]
            }],
            "store": false
        }));
        assert_eq!(prompt_template["prompt"]["id"], "pmpt_123");
        assert_eq!(prompt_template["prompt"]["variables"]["language"], "zh-CN");
    }

    #[test]
    fn accepts_public_agent_loop_tool_outputs_and_output_item_replay() {
        let body = public_entry_roundtrip(json!({
            "model": "MODEL_ID",
            "previous_response_id": "resp_previous_123",
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
                { "type": "function", "name": "get_weather", "parameters": { "type": "object", "properties": { "location": { "type": "string" } }, "required": ["location"], "additionalProperties": false }, "strict": true },
                { "type": "computer" },
                { "type": "shell", "environment": { "type": "local" } },
                { "type": "apply_patch" },
                { "type": "custom", "name": "freeform_transformer" }
            ],
            "tool_choice": "auto",
            "store": false
        }));

        assert_eq!(body["previous_response_id"], "resp_previous_123");
        assert_eq!(body["input"][0]["type"], "function_call_output");
        assert_eq!(body["input"][0]["status"], "completed");
        assert_eq!(body["input"][0]["output"][1]["type"], "input_image");
        assert_eq!(body["input"][1]["type"], "computer_call_output");
        assert_eq!(body["input"][2]["type"], "shell_call_output");
        assert_eq!(body["input"][3]["type"], "apply_patch_call_output");
        assert_eq!(body["input"][4]["type"], "mcp_approval_response");
        assert_eq!(body["input"][5]["type"], "custom_tool_call_output");
        assert_eq!(body["input"][6]["id"], "msg_assistant_123");
        assert_eq!(body["input"][6]["phase"], "final_answer");
        assert_eq!(body["input"][7]["id"], "fc_123");
        assert_eq!(body["input"][8]["type"], "image_generation_call");
        assert_eq!(body["input"][9]["type"], "code_interpreter_call");
        assert_eq!(body["input"][10]["type"], "mcp_call");
        assert_eq!(body["input"][11]["type"], "item_reference");
    }

    #[test]
    fn accepts_public_tool_choice_union_and_text_format_variants() {
        for tool_choice in [
            json!("none"),
            json!("auto"),
            json!("required"),
            json!({
                "type": "allowed_tools",
                "mode": "auto",
                "tools": [
                    { "type": "function", "name": "get_weather" },
                    { "type": "mcp", "server_label": "github" },
                    { "type": "image_generation" }
                ]
            }),
            json!({
                "type": "allowed_tools",
                "mode": "required",
                "tools": [
                    { "type": "function", "name": "get_weather" },
                    { "type": "file_search" }
                ]
            }),
            json!({ "type": "file_search" }),
            json!({ "type": "web_search_preview" }),
            json!({ "type": "computer" }),
            json!({ "type": "computer_use_preview" }),
            json!({ "type": "computer_use" }),
            json!({ "type": "web_search_preview_2025_03_11" }),
            json!({ "type": "image_generation" }),
            json!({ "type": "code_interpreter" }),
            json!({ "type": "function", "name": "get_weather" }),
            json!({ "type": "mcp", "server_label": "github", "name": "search_code" }),
            json!({ "type": "custom", "name": "freeform_transformer" }),
            json!({ "type": "shell" }),
            json!({ "type": "apply_patch" }),
        ] {
            let body = public_entry_roundtrip(json!({
                "model": "MODEL_ID",
                "input": "hello",
                "tool_choice": tool_choice.clone()
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
                "input": "hello",
                "text": text.clone()
            }));
            assert_eq!(body["text"], text);
        }
    }
}

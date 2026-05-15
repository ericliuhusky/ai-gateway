use serde::Serialize;

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
    pub r#type: String,
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

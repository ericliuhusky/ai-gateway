#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientProtocol {
    OpenAiResponses,
}

#[derive(Debug, Clone)]
pub struct CachedProviderModels {
    pub provider_id: String,
    pub models_json: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamProtocol {
    OpenAiPrivateResponses,
    NativeResponses,
    NativeChatCompletions,
}

impl ClientProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiResponses => "openai-responses",
        }
    }
}

impl UpstreamProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiPrivateResponses => "openai-private-responses",
            Self::NativeResponses => "native-responses",
            Self::NativeChatCompletions => "native-chat-completions",
        }
    }
}

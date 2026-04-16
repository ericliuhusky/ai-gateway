#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressProtocol {
    OpenAiResponses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressProtocol {
    OpenAiPrivateResponses,
    GoogleV1Internal,
    NativeResponses,
    NativeChatCompletions,
}

impl IngressProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiResponses => "openai-responses",
        }
    }
}

impl EgressProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiPrivateResponses => "openai-private-responses",
            Self::GoogleV1Internal => "google-v1internal",
            Self::NativeResponses => "native-responses",
            Self::NativeChatCompletions => "native-chat-completions",
        }
    }
}

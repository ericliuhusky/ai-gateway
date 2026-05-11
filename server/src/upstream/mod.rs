pub mod client;
mod google_v1internal;
mod openai;
mod shared;

pub use client::UpstreamClient;
pub use google_v1internal::GOOGLE_PROJECT_ID_FALLBACK;
pub use openai::{
    OPENAI_CODEX_BASE_URL, OpenAiEndpoint, PrivateOpenAiRequestBuilder, PublicOpenAiRequestBuilder,
    chat_completions_api_url, responses_api_url,
};

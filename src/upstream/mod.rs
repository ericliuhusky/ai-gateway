pub mod client;
mod google_v1internal;
mod openai_chat;
mod openai_private;
mod openai_responses;
mod shared;

pub use client::UpstreamClient;
pub use openai_chat::chat_completions_api_url;
pub use openai_responses::responses_api_url;

mod openai_chat;
mod openai_responses;

pub use openai_chat::responses_to_chat_completions;
pub use openai_responses::request_with_model;

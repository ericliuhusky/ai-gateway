use std::{env, net::SocketAddr, path::PathBuf};

const GOOGLE_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const GOOGLE_CLIENT_SECRET: &str = "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";
const BIND_ADDR: &str = "127.0.0.1:10100";
const OPENAI_CALLBACK_ADDR: &str = "127.0.0.1:1455";
const OPENAI_CALLBACK_URL: &str = "http://localhost:1455/auth/callback";

#[derive(Clone, Debug)]
pub struct Config;

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let _ = env::var("HOME").map_err(|_| "HOME environment variable is not set".to_string())?;
        Ok(Self)
    }

    pub fn bind_addr(&self) -> SocketAddr {
        BIND_ADDR
            .parse()
            .expect("hardcoded bind address must be valid")
    }

    pub fn data_dir(&self) -> PathBuf {
        PathBuf::from(env::var("HOME").expect("HOME environment variable is not set"))
            .join(".gemini-proxy")
    }

    pub fn openai_callback_addr(&self) -> SocketAddr {
        OPENAI_CALLBACK_ADDR
            .parse()
            .expect("hardcoded openai callback address must be valid")
    }

    pub fn openai_callback_url(&self) -> &'static str {
        OPENAI_CALLBACK_URL
    }

    pub fn google_client_id(&self) -> &'static str {
        GOOGLE_CLIENT_ID
    }

    pub fn google_client_secret(&self) -> &'static str {
        GOOGLE_CLIENT_SECRET
    }
}

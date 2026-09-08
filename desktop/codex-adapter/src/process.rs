use crate::config::{CodexConfigurationResult, start_default_codex, stop_default_codex};
#[cfg(target_os = "macos")]
use std::{path::Path, process::Command, thread, time::Duration};

pub fn start_codex_gateway(gateway_base_url: &str) -> Result<CodexConfigurationResult, String> {
    configure_codex(true, gateway_base_url)
}

pub fn stop_codex_gateway() -> Result<CodexConfigurationResult, String> {
    configure_codex(false, "")
}

fn configure_codex(
    start: bool,
    gateway_base_url: &str,
) -> Result<CodexConfigurationResult, String> {
    if start {
        start_default_codex(gateway_base_url)
    } else {
        let _ = gateway_base_url;
        stop_default_codex()
    }
}

pub fn restart_chatgpt() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if !Path::new("/Applications/ChatGPT.app").is_dir() {
            return Err("未找到 ChatGPT.app".to_string());
        }
        let running = Command::new("pgrep")
            .args(["-x", "ChatGPT"])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if running {
            if !Command::new("osascript")
                .args(["-e", "tell application \"ChatGPT\" to quit"])
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
            {
                return Err("无法退出 ChatGPT.app".to_string());
            }
            for _ in 0..10 {
                if !Command::new("pgrep")
                    .args(["-x", "ChatGPT"])
                    .status()
                    .map(|status| status.success())
                    .unwrap_or(false)
                {
                    break;
                }
                thread::sleep(Duration::from_secs(1));
            }
            if Command::new("pgrep")
                .args(["-x", "ChatGPT"])
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
            {
                return Err("ChatGPT.app 未在 10 秒内完全退出".to_string());
            }
        }
        if !Command::new("open")
            .args(["-a", "ChatGPT"])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return Err("无法打开 ChatGPT.app".to_string());
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("仅支持在 macOS 上重新打开 ChatGPT.app".to_string())
    }
}

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
    let mut result = if start {
        start_default_codex(gateway_base_url)?
    } else {
        let _ = gateway_base_url;
        stop_default_codex()?
    };
    if result.changed {
        if let Some(warning) = restart_codex() {
            result.warnings.push(warning);
        }
    }
    Ok(result)
}

fn restart_codex() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        if !Path::new("/Applications/ChatGPT.app").is_dir() {
            return Some("未找到 ChatGPT.app，请手动重新启动 Codex 以加载新配置。".to_string());
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
                return Some("无法自动退出 Codex，请手动完全退出后重新打开。".to_string());
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
                return Some("Codex 未在 10 秒内完全退出，请手动完全退出后重新打开。".to_string());
            }
        }
        if !Command::new("open")
            .args(["-a", "ChatGPT"])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return Some("无法自动打开 Codex，请手动重新打开 ChatGPT.app。".to_string());
        }
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        Some("请手动重新启动 Codex 以加载新配置。".to_string())
    }
}

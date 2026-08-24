use crate::config::{
    CodexConfigurationResult, CodexInstancePaths, prepare_codex_instance, start_default_codex,
    stop_default_codex,
};
#[cfg(target_os = "macos")]
use std::{path::Path, process::Command, thread, time::Duration};

pub fn start_codex_gateway(gateway_base_url: &str) -> Result<CodexConfigurationResult, String> {
    configure_codex(true, gateway_base_url)
}

pub fn stop_codex_gateway() -> Result<CodexConfigurationResult, String> {
    configure_codex(false, "")
}

pub fn start_codex_instance(instance_id: &str, api_root: &str) -> Result<(), String> {
    if instance_id == "default" {
        return Err("默认实例请使用 AI 网关播放按钮".to_string());
    }
    let api_root = api_root.trim().trim_end_matches('/');
    let gateway_url = format!("{api_root}/instances/{instance_id}/openai/v1");
    let paths = prepare_codex_instance(instance_id, &gateway_url)?;
    open_codex_instance(&paths)
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

fn open_codex_instance(paths: &CodexInstancePaths) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if !Path::new("/Applications/ChatGPT.app").is_dir() {
            return Err("未找到 /Applications/ChatGPT.app".to_string());
        }
        let codex_home = paths.codex_home.display().to_string();
        let electron_home = paths.electron_home.display().to_string();
        let codex_env = format!("CODEX_HOME={codex_home}");
        let electron_env = format!("CODEX_ELECTRON_USER_DATA_PATH={electron_home}");
        let user_data_dir = format!("--user-data-dir={electron_home}");
        let status = Command::new("open")
            .args([
                "-n",
                "-a",
                "ChatGPT",
                "--env",
                &codex_env,
                "--env",
                &electron_env,
                "--args",
                &user_data_dir,
            ])
            .status()
            .map_err(|error| format!("启动 Codex 实例失败：{error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("启动 Codex 实例失败".to_string())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = paths;
        Err("Codex 实例启动仅支持 macOS".to_string())
    }
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

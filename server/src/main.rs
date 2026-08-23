use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const SERVICE_LABEL: &str = "com.ai-gateway.server";
const SERVICE_BINARY: &str = "ai-gateway-server";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "serve".to_string());
    let web_dir_arg = parse_web_dir(arguments.collect())?;
    match command.as_str() {
        "serve" => {
            let web_dir = prepare_serve_web_dir(web_dir_arg)?;
            server::serve_gateway(web_dir).await.map_err(Into::into)
        }
        "install" => {
            let web_dir = match web_dir_arg {
                WebDirArg::FromCli(path) => Some(path),
                WebDirArg::FromEnv(_) | WebDirArg::Unspecified => None,
            };
            install_service(web_dir).map_err(Into::into)
        }
        "start" => start_service().map_err(Into::into),
        "stop" => stop_service().map_err(Into::into),
        "status" => service_status().map_err(Into::into),
        "uninstall" => uninstall_service().map_err(Into::into),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        other => Err(format!("未知命令：{other}").into()),
    }
}

#[derive(Debug)]
enum WebDirArg {
    Unspecified,
    FromEnv(PathBuf),
    FromCli(PathBuf),
}

fn parse_web_dir(arguments: Vec<String>) -> Result<WebDirArg, String> {
    if arguments.is_empty() {
        return Ok(match env::var_os("AI_GATEWAY_WEB_DIR") {
            Some(path) => WebDirArg::FromEnv(PathBuf::from(path)),
            None => WebDirArg::Unspecified,
        });
    }
    if arguments.len() == 2 && arguments[0] == "--web-dir" {
        let path = PathBuf::from(&arguments[1]);
        return path
            .is_dir()
            .then_some(WebDirArg::FromCli(path))
            .ok_or_else(|| "Web 目录不存在或不是目录".to_string());
    }
    Err("用法：ai-gateway-server <serve|install> [--web-dir <目录>]".to_string())
}

fn prepare_serve_web_dir(web_dir_arg: WebDirArg) -> Result<Option<PathBuf>, String> {
    match web_dir_arg {
        WebDirArg::Unspecified => Ok(None),
        WebDirArg::FromEnv(path) | WebDirArg::FromCli(path) => {
            if path.is_dir() {
                Ok(Some(path))
            } else {
                Err("Web 目录不存在或不是目录".to_string())
            }
        }
    }
}

fn install_web_assets(destination: &Path, source: &Path) -> Result<PathBuf, String> {
    if !source.is_dir() {
        return Err(format!("Web 构建产物不存在：{}", source.display()));
    }
    if destination.exists() {
        fs::remove_dir_all(destination).map_err(|error| format!("清理旧 Web 资源失败：{error}"))?;
    }
    copy_directory(source, destination)?;
    Ok(destination.to_path_buf())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| format!("创建 Web 资源目录失败：{error}"))?;
    for entry in fs::read_dir(source).map_err(|error| format!("读取 Web 资源目录失败：{error}"))?
    {
        let entry = entry.map_err(|error| format!("读取 Web 资源项失败：{error}"))?;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| format!("读取 Web 资源类型失败：{error}"))?
            .is_dir()
        {
            copy_directory(&path, &target)?;
        } else {
            fs::copy(&path, &target).map_err(|error| format!("复制 Web 资源失败：{error}"))?;
        }
    }
    Ok(())
}

fn resolve_install_web_source(web_dir: Option<PathBuf>) -> Result<Option<PathBuf>, String> {
    web_dir
        .map(|path| fs::canonicalize(&path).map_err(|error| format!("解析 Web 目录失败：{error}")))
        .transpose()
}

fn install_service(web_dir: Option<PathBuf>) -> Result<(), String> {
    let layout = ServiceLayout::load()?;
    fs::create_dir_all(&layout.bin_dir).map_err(|error| format!("创建服务目录失败：{error}"))?;
    fs::create_dir_all(&layout.log_dir).map_err(|error| format!("创建日志目录失败：{error}"))?;
    fs::create_dir_all(&layout.plist_dir)
        .map_err(|error| format!("创建 LaunchAgent 目录失败：{error}"))?;

    let current = env::current_exe().map_err(|error| format!("读取当前服务程序失败：{error}"))?;
    if current != layout.binary_path {
        fs::copy(&current, &layout.binary_path)
            .map_err(|error| format!("安装 Gateway Server 二进制失败：{error}"))?;
        make_executable(&layout.binary_path)?;
    }

    let resolved_web_dir = resolve_install_web_source(web_dir)?;
    let installed_web_dir = resolved_web_dir
        .as_deref()
        .map(|source| install_web_assets(&layout.web_dir(), source))
        .transpose()?;
    fs::write(
        &layout.plist_path,
        launchd_plist(&layout, installed_web_dir.as_deref()),
    )
    .map_err(|error| format!("写入 LaunchAgent 配置失败：{error}"))?;
    run_launchctl(&["bootout", &layout.target()], true)?;
    run_launchctl(
        &["bootstrap", &layout.domain(), path_str(&layout.plist_path)?],
        false,
    )?;
    run_launchctl(&["enable", &layout.target()], true)?;
    run_launchctl(&["kickstart", "-k", &layout.target()], false)?;
    println!("Gateway Server 已安装并启动：{}", layout.target());
    Ok(())
}

fn start_service() -> Result<(), String> {
    let layout = ServiceLayout::load()?;
    if !layout.plist_path.is_file() {
        return Err("Gateway Server 尚未安装；请先运行 ai-gateway-server install".to_string());
    }
    run_launchctl(
        &["bootstrap", &layout.domain(), path_str(&layout.plist_path)?],
        true,
    )?;
    run_launchctl(&["enable", &layout.target()], true)?;
    run_launchctl(&["kickstart", "-k", &layout.target()], false)?;
    println!("Gateway Server 已启动");
    Ok(())
}

fn stop_service() -> Result<(), String> {
    let layout = ServiceLayout::load()?;
    run_launchctl(&["bootout", &layout.target()], true)?;
    println!("Gateway Server 已停止");
    Ok(())
}

fn service_status() -> Result<(), String> {
    let layout = ServiceLayout::load()?;
    let output = run_launchctl(&["print", &layout.target()], true)?;
    if output.is_empty() {
        println!("Gateway Server 未运行");
    } else {
        println!("{output}");
    }
    Ok(())
}

fn uninstall_service() -> Result<(), String> {
    let layout = ServiceLayout::load()?;
    run_launchctl(&["bootout", &layout.target()], true)?;
    if layout.plist_path.exists() {
        fs::remove_file(&layout.plist_path)
            .map_err(|error| format!("删除 LaunchAgent 配置失败：{error}"))?;
    }
    println!("Gateway Server LaunchAgent 已卸载；数据文件未删除");
    Ok(())
}

struct ServiceLayout {
    root: PathBuf,
    bin_dir: PathBuf,
    log_dir: PathBuf,
    plist_dir: PathBuf,
    binary_path: PathBuf,
    plist_path: PathBuf,
}

impl ServiceLayout {
    fn load() -> Result<Self, String> {
        let home = env::var_os("HOME").ok_or_else(|| "未设置 HOME 环境变量".to_string())?;
        let root = PathBuf::from(home).join(".ai-gateway");
        let bin_dir = root.join("bin");
        let plist_dir = PathBuf::from(env::var_os("HOME").unwrap()).join("Library/LaunchAgents");
        Ok(Self {
            log_dir: root.join("log"),
            binary_path: bin_dir.join(SERVICE_BINARY),
            plist_path: plist_dir.join(format!("{SERVICE_LABEL}.plist")),
            root,
            bin_dir,
            plist_dir,
        })
    }

    fn web_dir(&self) -> PathBuf {
        self.root.join("web")
    }

    fn domain(&self) -> String {
        format!("gui/{}", libc_getuid())
    }
    fn target(&self) -> String {
        format!("{}/{}", self.domain(), SERVICE_LABEL)
    }
}

fn libc_getuid() -> u32 {
    // macOS-only binary; the shell command avoids an additional FFI dependency.
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

fn run_launchctl(arguments: &[&str], allow_failure: bool) -> Result<String, String> {
    let output = Command::new("/bin/launchctl")
        .args(arguments)
        .output()
        .map_err(|error| format!("执行 launchctl 失败：{error}"))?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .trim()
    .to_string();
    if !allow_failure && !output.status.success() {
        return Err(format!(
            "launchctl {} 失败：{}",
            arguments.join(" "),
            combined
        ));
    }
    Ok(if output.status.success() {
        combined
    } else {
        String::new()
    })
}

fn launchd_plist(layout: &ServiceLayout, web_dir: Option<&Path>) -> String {
    let web_environment = web_dir
        .map(|path| {
            format!(
                "\n    <key>AI_GATEWAY_WEB_DIR</key>\n    <string>{}</string>",
                xml_escape(&path.display().to_string())
            )
        })
        .unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{SERVICE_LABEL}</string>
  <key>ProgramArguments</key><array><string>{}</string><string>serve</string></array>
  <key>WorkingDirectory</key><string>{}</string>
  <key>EnvironmentVariables</key><dict>
    <key>HOME</key><string>{}</string>{web_environment}
  </dict>
  <key>StandardOutPath</key><string>{}/service.out.log</string>
  <key>StandardErrorPath</key><string>{}/service.err.log</string>
  <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
</dict></plist>"#,
        xml_escape(&layout.binary_path.display().to_string()),
        xml_escape(&layout.root.display().to_string()),
        xml_escape(&env::var("HOME").unwrap_or_default()),
        xml_escape(&layout.log_dir.display().to_string()),
        xml_escape(&layout.log_dir.display().to_string()),
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "路径不是有效 UTF-8".to_string())
}

fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .map_err(|error| format!("设置服务二进制权限失败：{error}"))
}

fn print_usage() {
    println!("ai-gateway-server <serve|install|start|stop|status|uninstall> [--web-dir <目录>]");
}

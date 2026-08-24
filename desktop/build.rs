use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const GATEWAY_DAEMON_BINARY: &str = "ai-gateway-daemon";

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing manifest dir"));
    let workspace_dir = manifest_dir
        .parent()
        .expect("desktop must be inside the workspace")
        .to_path_buf();
    build_gateway_sidecar(&workspace_dir, &manifest_dir);

    let web_dir = manifest_dir.join("web");
    for path in [
        "package.json",
        "bun.lock",
        "scripts",
        "src",
        "tsconfig.json",
    ] {
        println!("cargo::rerun-if-changed={}", web_dir.join(path).display());
    }
    let status = Command::new("bun")
        .args(["run", "build"])
        .current_dir(&web_dir)
        .status()
        .unwrap_or_else(|error| panic!("无法构建 Tauri Web 界面；请先安装 Bun：{error}"));
    assert!(status.success(), "Tauri Web 界面构建失败");
    tauri_build::build()
}

fn build_gateway_sidecar(workspace_dir: &Path, desktop_dir: &Path) {
    let target = env::var("TARGET").expect("missing Cargo target");
    let profile = env::var("PROFILE").expect("missing Cargo profile");
    let gateway_dir = workspace_dir.join("gateway");
    emit_rerun_if_changed(&gateway_dir);
    println!(
        "cargo::rerun-if-changed={}",
        workspace_dir.join("Cargo.lock").display()
    );

    // Use a separate target directory: invoking Cargo from a build script
    // must not contend with the parent Cargo process for the normal target
    // directory lock.
    let sidecar_target_dir = workspace_dir.join("target/gateway-sidecar");
    let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .current_dir(workspace_dir)
        .env("CARGO_TARGET_DIR", &sidecar_target_dir)
        .env_remove("CARGO_MAKEFLAGS")
        .args([
            "build",
            "--locked",
            "--package",
            "gateway",
            "--bin",
            GATEWAY_DAEMON_BINARY,
            "--target",
            &target,
        ]);
    if profile == "release" {
        command.arg("--release");
    }
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("无法构建内置 Gateway 服务程序：{error}"));
    assert!(
        output.status.success(),
        "构建内置 Gateway 服务程序失败：\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let executable = sidecar_target_dir
        .join(&target)
        .join(profile)
        .join(GATEWAY_DAEMON_BINARY);
    assert!(
        executable.is_file(),
        "未找到已构建的 Gateway 服务程序：{}",
        executable.display()
    );

    let output_dir = desktop_dir.join("binaries");
    fs::create_dir_all(&output_dir).expect("无法创建 Gateway sidecar 目录");
    let bundled_name = format!("{GATEWAY_DAEMON_BINARY}-{target}");
    let bundled_path = output_dir.join(bundled_name);
    fs::copy(&executable, &bundled_path).unwrap_or_else(|error| {
        panic!(
            "无法准备 Gateway sidecar（{} → {}）：{error}",
            executable.display(),
            bundled_path.display()
        )
    });
}

fn emit_rerun_if_changed(path: &Path) {
    if path.is_file() {
        println!("cargo::rerun-if-changed={}", path.display());
        return;
    }
    for entry in
        fs::read_dir(path).unwrap_or_else(|error| panic!("无法读取 {}：{error}", path.display()))
    {
        let path = entry
            .unwrap_or_else(|error| panic!("无法读取 Gateway 源码条目：{error}"))
            .path();
        emit_rerun_if_changed(&path);
    }
}

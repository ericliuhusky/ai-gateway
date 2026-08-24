use std::{env, path::PathBuf, process::Command};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing manifest dir"));
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

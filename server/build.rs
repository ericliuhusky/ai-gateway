use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("无法获取 Cargo 项目目录"));
    let web_dir = manifest_dir.join("../web");

    emit_rerun_if_changed(&web_dir);
    println!("cargo::rerun-if-env-changed=HOME");

    let status = Command::new("bun")
        .args(["run", "build"])
        .current_dir(&web_dir)
        .status()
        .unwrap_or_else(|error| panic!("无法运行 Bun；请先安装 Bun：{error}"));
    assert!(status.success(), "Web 管理端构建失败");

    let target_dir = web_install_dir();
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir).unwrap_or_else(|error| {
            panic!(
                "无法删除旧的 Web 静态文件（{}）：{error}",
                target_dir.display()
            )
        });
    }
    copy_dir(&web_dir.join("dist"), &target_dir);
}

fn web_install_dir() -> PathBuf {
    PathBuf::from(env::var_os("HOME").expect("未设置 HOME 环境变量")).join(".ai-gateway/web")
}

fn emit_rerun_if_changed(web_dir: &Path) {
    for path in [
        "package.json",
        "bun.lock",
        "components.json",
        "scripts",
        "src",
    ] {
        emit_paths(&web_dir.join(path));
    }
}

fn emit_paths(path: &Path) {
    if path.is_file() {
        println!("cargo::rerun-if-changed={}", path.display());
        return;
    }

    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            emit_paths(&entry.path());
        }
    }
}

fn copy_dir(source: &Path, destination: &Path) {
    fs::create_dir_all(destination)
        .unwrap_or_else(|error| panic!("无法创建目录 {}：{error}", destination.display()));

    for entry in fs::read_dir(source)
        .unwrap_or_else(|error| panic!("无法读取目录 {}：{error}", source.display()))
        .flatten()
    {
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &destination_path);
        } else {
            fs::copy(&source_path, &destination_path).unwrap_or_else(|error| {
                panic!(
                    "无法复制 {} 到 {}：{error}",
                    source_path.display(),
                    destination_path.display()
                )
            });
        }
    }
}

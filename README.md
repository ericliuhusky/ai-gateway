# AI Gateway

AI Gateway 是仅支持 macOS 的**本机桌面客户端**。用户只需安装并启动 Tauri Client；客户端会确保本机 Rust Gateway 后台服务（LaunchAgent）运行、加载内置 Web UI，并将本机 Codex 配置为使用 `http://127.0.0.1:42401/v1`。

不支持远程 Gateway 连接，也不提供通过浏览器托管的管理控制台。

```text
.
├── desktop/                # 唯一面向用户的 Tauri 桌面客户端
│   ├── web/                # 仅供桌面端使用的 UI 源码
│   └── codex-adapter/      # 桌面端管理本机 Codex 配置
└── gateway/                # 本机 Gateway 服务与共享 API crate
```

## 使用客户端

```bash
cargo run
cargo build
```

启动客户端后：

- Gateway 只监听 `127.0.0.1:42401`；
- UI 通过 Tauri `invoke` 调用 Rust，再经本机 HTTP 管理接口管理后台 Gateway；
- 运行数据保存在 `~/Library/Application Support/AI Gateway/db.sqlite`；
- 默认 Codex 的配置保留在当前 Mac；
- 退出客户端不会停止 Gateway；Codex 可继续使用本机网关。

客户端的 AI 网关播放键会将默认 Codex 指向本机 Gateway；停止键会恢复原先的 Codex 配置。

## 开发组件

`gateway/` 既提供共享 API crate，也构建为内部使用的 `ai-gateway-daemon` 二进制。开发时，执行 `cargo run` 会自动构建并携带该二进制；发布时，Tauri 会将它作为 sidecar 打包进客户端。客户端会将该二进制注册为 LaunchAgent 并自动启动，用户不需要手动运行任何 Gateway 命令。

Gateway 对 TCP 暴露 `/v1/*` 和 `/management/*`；前者用于 LLM 请求，后者用于本机管理操作。管理接口只绑定本机回环地址，并由 Tauri `invoke` 间接调用，不构建或托管 Web UI。

## 验证

```bash
cargo test --workspace -- --test-threads=1
cargo check --workspace
cd desktop/web && bun run typecheck && bun run build
```

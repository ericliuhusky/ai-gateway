# AI Gateway

AI Gateway 是仅支持 macOS 的**本机桌面客户端**。用户只需安装并启动 Tauri Client；客户端会确保本机 Rust Gateway 后台服务（LaunchAgent）运行、加载内置 Web UI，并将本机 Codex 配置为使用 `http://127.0.0.1:4242/openai/v1`。

不支持远程 Gateway 连接，也不提供通过浏览器托管的管理控制台。

```text
.
├── desktop/                # 唯一面向用户的 Tauri 桌面客户端
│   ├── web/                # 仅供桌面端使用的 UI 源码
│   └── codex-adapter/      # 桌面端管理本机 Codex 配置与实例
└── gateway/                # 本机 Gateway 服务与共享 API crate
```

## 使用客户端

```bash
cargo run
cargo build
```

启动客户端后：

- Gateway 只监听 `127.0.0.1:4242`；
- UI 通过 Tauri `invoke` 调用 Rust，再经私有 Unix Socket 管理后台 Gateway；
- 运行数据保存在 `~/Library/Application Support/AI Gateway/db.sqlite`；
- 私有控制 Socket 位于 `~/Library/Application Support/AI Gateway/control/gateway.sock`；
- 默认 Codex 和命名实例的配置、会话与 Electron 数据均保留在当前 Mac；
- 退出客户端不会停止 Gateway；Codex 可继续使用本机网关。

客户端的 AI 网关播放键会将默认 Codex 指向本机 Gateway；停止键会恢复原先的 Codex 配置。命名实例会在本机创建独立的 Codex 配置和数据目录；删除实例会同时删除其网关路由及本机实例文件。

## 开发组件

`gateway/` 既提供共享 API crate，也构建为内部使用的 `ai-gateway-daemon` 二进制。开发时，执行 `cargo run` 会自动构建并携带该二进制；发布时，Tauri 会将它作为 sidecar 打包进客户端。客户端会将该二进制注册为 LaunchAgent 并自动启动，用户不需要手动运行任何 Gateway 命令。

Gateway 对 TCP 只暴露 `/healthz`、`/openai/v1/*` 和实例对应的 OpenAI 兼容接口；供应商、路由、账号、用量等管理操作只在当前用户可访问的 Unix Socket 上提供，并由 Tauri `invoke` 间接调用，不构建或托管 Web UI。

## 验证

```bash
cargo test --workspace -- --test-threads=1
cargo check --workspace
cd desktop/web && bun run typecheck && bun run build
```

# AI Gateway

AI Gateway 是仅支持 macOS 的**本机桌面客户端**。用户只需安装并启动 Tauri Client；客户端会确保本机 Rust Gateway 后台服务（LaunchAgent）运行、加载内置 Web UI，并将本机 Codex 配置为使用 `http://127.0.0.1:4242/openai/v1`。

不支持远程 Gateway 连接，也不提供通过浏览器托管的管理控制台。

```text
.
├── client/                 # 唯一面向用户的 Tauri 桌面客户端
├── server/                 # 客户端内嵌使用的本机 Gateway API
├── codex-adapter/          # 客户端管理本机 Codex 配置与实例
├── web/                    # 打包进 Tauri Client 的 UI 源码
└── share-group/
    └── server/             # 可选的共享群组中心服务
```

## 使用客户端

```bash
cargo run -p ai-gateway-client
cargo build -p ai-gateway-client
```

启动客户端后：

- Gateway 只监听 `127.0.0.1:4242`；
- UI 通过 Tauri `invoke` 调用 Rust，再经私有 Unix Socket 管理后台 Gateway；
- 默认 Codex 和命名实例的配置、会话与 Electron 数据均保留在当前 Mac；
- 退出客户端不会停止 Gateway；Codex 可继续使用本机网关。

客户端的 AI 网关播放键会将默认 Codex 指向本机 Gateway；停止键会恢复原先的 Codex 配置。命名实例会在本机创建独立的 Codex 配置和数据目录；删除实例会同时删除其网关路由及本机实例文件。

## 开发组件

`server/` 是客户端使用的本机 API crate，也可在开发时单独运行 OpenAI 兼容网关：

```bash
cargo run -p server -- serve
```

它对 TCP 只暴露 `/healthz`、`/openai/v1/*` 和实例对应的 OpenAI 兼容接口；供应商、路由、账号、用量、群组等管理操作只在当前用户可访问的 Unix Socket 上提供，并由 Tauri `invoke` 间接调用，不构建或托管 Web UI。`share-group/server` 是独立的共享群组中心服务：

```bash
cargo run -p ai-gateway-share-group-server
```

## 验证

```bash
cargo test --workspace -- --test-threads=1
cargo check --workspace
cd web && bun run typecheck && bun run build
```

# AI Gateway

AI Gateway 由四个可独立部署的部分组成：

```text
.
├── server/                 # Gateway Server：本机常驻服务、供应商/路由/Codex 控制
├── web/                    # Web 控制台：可由 Gateway Server 托管
├── client/                 # Tauri 本机客户端：只负责显示 Web UI 并连接本机 Server
└── share-group/
    └── server/             # 共享群组中心服务：用户、群组、共享供应商授权
```

只支持 **macOS**。Gateway Server 使用 `launchd` LaunchAgent 常驻，不包含 Linux 或 `systemctl` 兼容逻辑。

## 部署方式

### 本机使用：`server + client`

在同一台 Mac 上安装 Gateway Server 和 Tauri Client。Client 启动时只会连接 `127.0.0.1:4242` 的 Server；如果尚未运行，会调用 `ai-gateway-server start` 启动已安装的 LaunchAgent。

### 远程控制：`server + web`

在装有 Gateway 的 Mac 上运行 Server，并将 Web 静态资源托管在 `~/.ai-gateway/web`。控制端通过浏览器访问该 Mac 的 Web 控制台即可管理网关；不需要安装 Tauri Client。

> 当前 Server 默认仅绑定 `127.0.0.1:4242`，避免未认证的管理接口被直接暴露。若要跨机器访问，请通过带认证与 TLS 的反向代理或安全隧道转发，而不是直接公开端口。

### 共享群组中心：`share-group/server`

共享群组服务与 Gateway Server 完全独立，可部署在中心 Mac 上。它只负责账号、群组、共享连接和短期租约，不转发模型请求，也不托管 Gateway Web。

## Gateway Server

开发运行（会自动在仓库 `web/` 执行 `bun install`、`bun run build`，并同步到 `~/.ai-gateway/web`）：

```bash
cargo run -p server -- serve
```

安装为当前 macOS 用户的 LaunchAgent（同样默认构建并安装到 `~/.ai-gateway/web`）：

```bash
cargo run -p server -- install
```

管理命令：

```bash
ai-gateway-server start
ai-gateway-server stop
ai-gateway-server status
ai-gateway-server uninstall
```

安装目录、日志和控制面凭据位于：

```text
~/.ai-gateway/
├── bin/ai-gateway-server
├── log/
├── web/
├── config/
│   ├── config.json
│   ├── credentials.json.enc
│   └── credentials.key
└── data/
    └── db.sqlite
```

LaunchAgent 文件为：

```text
~/Library/LaunchAgents/com.ai-gateway.server.plist
```

Server 首次启动会自动配置默认 Codex 使用 `http://127.0.0.1:4242/openai/v1`。界面的 AI 网关卡片播放键负责启动服务并配置默认 Codex；停止方块键负责恢复默认 Codex 配置后停止服务。命名实例只有播放键：首次播放创建隔离实例，后续播放直接打开已有实例；删除实例会同时删除路由与实例文件，并清理实例的 Codex 配置。

## Web

```bash
cd web
bun install
bun run build
```

默认情况下 Web 在浏览器中使用当前页面的 origin；在 Tauri WebView 中自动使用 `http://127.0.0.1:4242`。Server 默认托管 `~/.ai-gateway/web`；需要覆盖时再传 `--web-dir`。

## Client

```bash
cargo run -p ai-gateway-client
cargo build -p ai-gateway-client
```

Client 不再在进程内启动 Gateway，也不保存供应商或中心共享凭据；这些都归 Gateway Server 所有。

## 共享群组服务

开发运行：

```bash
cargo run -p ai-gateway-share-group-server
```

可用 `deploy.sh` 构建并部署共享群组服务。该脚本仅面向 macOS LaunchAgent 部署。

首次启动可用环境变量创建管理员：

```bash
export AI_GATEWAY_BOOTSTRAP_ADMIN_EMAIL="admin@example.com"
export AI_GATEWAY_BOOTSTRAP_ADMIN_PASSWORD="change-me"
export AI_GATEWAY_BOOTSTRAP_ADMIN_NAME="管理员"
cargo run -p ai-gateway-share-group-server
```

## 验证

```bash
cargo test --workspace -- --test-threads=1
cargo check --workspace
cd web && bun run typecheck && bun run build
```

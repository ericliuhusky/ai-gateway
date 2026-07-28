# ai-gateway

AI Gateway 现在拆分为两个进程：

- `server`：部署在服务器上的网关核心，负责 Provider、账号 Token、路由、协议适配、额度、日志和上游请求。
- `agent`：只运行在用户电脑上的本地集成进程，负责修改 `~/.codex` 配置和同步本地历史。

macOS App 只内置和管理 `agent`，不再内置完整网关 `server`。

## 架构

```text
Codex -> https://gateway.example.com/openai/v1 -> server -> upstream provider
  |
  +-> local agent (127.0.0.1:10101)
        - apply/restore ~/.codex/config.toml
        - sync ~/.codex/state_5.sqlite and rollout aliases
```

## Server

### 本地运行

```bash
cargo run -p ai-gateway
```

默认监听：

```text
0.0.0.0:10100
```

支持的环境变量：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `AI_GATEWAY_BIND_ADDR` | `0.0.0.0:10100` | HTTP 监听地址 |
| `AI_GATEWAY_DATA_DIR` | `$HOME/.ai-gateway` | SQLite 和日志目录 |
| `AI_GATEWAY_CODEX_CLIENT_VERSION` | `0.130.0` | 调用 ChatGPT Codex 私有模型接口时使用的客户端版本 |

Server 不再读取或修改服务器用户的 `~/.codex`。

当前网关接口还没有客户端鉴权。正式暴露到公网前，应放在带 TLS 和访问控制的反向代理后，或者先完成网关 API Key/多租户改造。

## Agent

### 运行

```bash
cargo run -p ai-gateway-agent
```

Agent 默认只监听：

```text
127.0.0.1:10101
```

可通过 `AI_GATEWAY_AGENT_BIND_ADDR` 修改，但不建议暴露到局域网或公网。

### 应用 Codex 配置

```bash
curl -X PUT http://127.0.0.1:10101/codex-config \
  -H 'Content-Type: application/json' \
  -d '{
    "gateway_base_url": "https://gateway.example.com/openai/v1"
  }'
```

Agent 会：

- 备份并修改 `~/.codex/config.toml`
- 将 `model_provider` 指向 `ai-gateway`
- 尝试为现有 OpenAI 历史创建 `ai-gateway` 别名

历史数据库不存在时不会阻止配置写入，响应中的 `history_warning` 会说明原因。

恢复：

```bash
curl -X DELETE http://127.0.0.1:10101/codex-config
```

## macOS App

App 启动时会：

1. 编译并内置 `ai-gateway-agent`
2. 安装到 `~/.ai-gateway/bin/ai-gateway-agent`
3. 使用 LaunchAgent `ericliu.husky.ai-gateway.agent` 管理本地 Agent
4. 将远程 Server URL 写入 Codex 配置

Server URL 当前按以下顺序确定：

1. macOS `UserDefaults` 的 `gatewayServerURL`
2. 环境变量 `AI_GATEWAY_SERVER_URL`
3. 默认 `http://127.0.0.1:10100`

配置远程 Server：

```bash
defaults write ericliu.husky.AIGateway gatewayServerURL https://gateway.example.com
```

## Token 导入

浏览器 OAuth 登录已经移除。OpenAI 账号只通过粘贴 Codex Token 导入：

```bash
curl -X POST http://127.0.0.1:10100/accounts/openai/import-token \
  -H 'Content-Type: application/json' \
  -d '{
    "tokens": {
      "access_token": "...",
      "refresh_token": "...",
      "id_token": "...",
      "account_id": "..."
    }
  }'
```

必填：

- `tokens.access_token`
- `tokens.refresh_token`

可选：

- `tokens.id_token`
- `tokens.account_id`

Server 会从 Token 中提取邮箱、过期时间和 ChatGPT Account ID，并在 access token 即将过期时使用 refresh token 自动刷新。

## 主要接口

```text
GET    /healthz
POST   /accounts/openai/import-token
GET    /providers
POST   /providers
DELETE /providers/:provider_id
GET    /providers/:provider_id/quota
GET    /selected-provider
PUT    /selected-provider
GET    /selected-model
PUT    /selected-model
DELETE /selected-model
GET    /openai/v1/models
POST   /openai/v1/responses
GET    /debug
```

所有推理请求仍统一使用 OpenAI Responses 客户端协议：

```bash
curl -X POST http://127.0.0.1:10100/openai/v1/responses \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "gpt-5.4",
    "input": "hello"
  }'
```

# ai-gateway

AI Gateway 现在由远程 Server 和 Web 管理端组成：

- `server`：部署在服务器上的网关核心，负责 Provider、账号 Token、路由、协议适配、额度和上游请求。
- `web`：由 `server` 同源托管的管理端，负责远程 Provider、模型、额度和接入配置。

浏览器不能直接修改本机文件，因此 Web 管理端会生成一次性 Shell 命令，由用户在本机终端执行。接入过程不需要安装客户端或启动后台服务。

## 架构

```text
Codex -> https://gateway.example.com/openai/v1 -> server -> upstream provider

Browser -> Web 管理端 -> 生成一次性 setup.sh / restore.sh 命令
                           -> 修改或恢复 ~/.codex/config.toml
```

## Server

### 本地运行

```bash
cargo run -p ai-gateway
```

默认监听：

```text
0.0.0.0:4242
```

支持的环境变量：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `AI_GATEWAY_BIND_ADDR` | `0.0.0.0:4242` | HTTP 监听地址 |
| `AI_GATEWAY_DATA_DIR` | `$HOME/.ai-gateway` | SQLite 数据目录 |
| `AI_GATEWAY_CODEX_CLIENT_VERSION` | `0.130.0` | 调用 ChatGPT Codex 私有模型接口时使用的客户端版本 |

Server 不再读取或修改服务器用户的 `~/.codex`。

当前网关接口还没有客户端鉴权。正式暴露到公网前，应放在带 TLS 和访问控制的反向代理后，或者先完成网关 API Key/多租户改造。

## Codex 接入脚本

无需安装本地 Agent。在本机终端执行：

```bash
curl -fsSL 'https://gateway.example.com/codex/setup.sh' |
  sh -s -- 'https://gateway.example.com/openai/v1'
```

脚本会：

- 首次运行时备份 `~/.codex/config.toml`
- 将 `model_provider` 指向 `ai-gateway`
- 写入当前 Gateway 的 `base_url`
- 使用临时文件原子替换配置
- 执行完成后立即退出，不安装程序或启动后台服务

恢复：

```bash
curl -fsSL 'https://gateway.example.com/codex/restore.sh' | sh
```

恢复前，脚本还会将当前配置额外备份到 `~/.ai-gateway`。

第一阶段脚本只处理 Provider 配置，不修改 `state_5.sqlite`，也不为旧历史创建 Provider 别名。

## Web 管理端

管理端已迁移为 Web，并由远程 Server 同源托管。技术栈为 React、TypeScript、Bun、Tailwind CSS。

```bash
cd web
bun install
bun run install:server
cd ..
cargo run -p ai-gateway
```

默认构建并安装到：

```text
$HOME/.ai-gateway/web
```

打开：

```text
http://127.0.0.1:4242/
```

如果静态文件部署在其他目录，可设置：

```text
AI_GATEWAY_WEB_DIR=/path/to/web
```

Web 页面与它所在的远程 Server 同源通信，不需要单独配置 Server URL。页面中的“Codex 接入”会根据当前 Server 地址生成一次性接入和恢复命令。

API Key 供应商使用显式的上游协议和兼容 Profile：

- `openai_responses`：上游原生支持 Responses。
- `openai_chat_completions`：网关负责 Responses 与 Chat Completions 的双向转换。
- `official_openai`：保持 Responses 请求 Body 原样。
- `generic_openai`：应用通用兼容清理规则。

适配架构和后续拆分计划见 `docs/adapter-architecture.md`。

## Token 导入

浏览器 OAuth 登录已经移除。OpenAI 账号只通过粘贴 Codex Token 导入：

```bash
curl -X POST http://127.0.0.1:4242/accounts/openai/import-token \
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
GET    /codex/setup.sh
GET    /codex/restore.sh
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
```

所有推理请求仍统一使用 OpenAI Responses 客户端协议：

```bash
curl -X POST http://127.0.0.1:4242/openai/v1/responses \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "gpt-5.4",
    "input": "hello"
  }'
```

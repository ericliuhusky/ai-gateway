# ai-gateway

AI Gateway 现在由远程 Server 和 Web 管理端组成：

- `server`：部署在服务器上的网关核心，负责 Provider、账号 Token、路由、协议适配、额度和上游请求。
- `web`：由 `server` 同源托管的管理端，负责远程 Provider、模型、额度和接入配置。

浏览器不能直接修改本机文件，因此 Web 管理端会生成一次性 Shell 命令，由用户在本机终端执行。接入过程不需要安装客户端或启动后台服务。

## 自动模型路由

网关可选地使用一个低成本路由分类模型，为每个普通文本请求选择轻量、标准、专业或极致模型。它默认关闭。新 turn 若配置了“已选模型”，会优先使用该模型；同一 turn 在后续工具回合中会粘住首回合选定的模型，不会中途切换。

管理端顶部还可为当前供应商设置“推理强度”覆盖。选择 `low`、`medium`、`high` 或 `xhigh` 后，网关会把每个 Responses 请求中的 `reasoning.effort` 改为该值（会保留 `reasoning` 下的其他字段）；选择“跟随请求”即可取消覆盖。模型和推理强度偏好均按供应商分别保存。

在 Web 管理端的“网关设置 → 自动模型路由”中配置：

- 路由分类模型：仅返回 `light`、`standard`、`pro` 或 `max` 和置信度。
- 轻量模型（`light`）、标准模型（`standard`）、专业模型（`pro`）、极致模型（`max`，兜底）：对应最终执行模型。
- 低置信度阈值：分类置信度低于阈值时，自动升级到极致模型。

图片、低置信度、分类失败、返回无效结果或配置不完整时会保守使用极致模型。工具**声明**本身仍会交给路由分类模型判断；带工具结果的后续请求则复用该 turn 的既定模型。

观测数据保存在同一 SQLite 数据库的 `turn_route_logs` 表中，不会写入响应头。每个 turn 一行，记录首条用户输入的最多 160 字预览、哈希后的 turn ID、模型、路由档位/原因、分类置信度、推理度、请求次数、工具回合数和时间。分类失败时保存最多 500 字的上游错误详情；分类模型返回文本时同样最多保存 500 字，便于判断 JSON 是否有效。不保存完整提示词、工具结果或最终回答。表最多保留 1000 条，按 `updated_at` 作为 LRU 淘汰最旧数据。管理端首页会显示“最近路由 Turn”。

网关优先使用 Codex 请求头 `x-codex-turn-metadata` 中的 `turn_id` 聚合同一 turn，并兼容请求 Body 中的 `client_metadata.turn_id` / `turnId`；原始 ID 不会落库，仅保存不可逆哈希。如果客户端未提供这些字段，网关会为该请求生成独立的匿名 turn，无法跨工具回合保持模型粘性。

分类器读取的是经长度限制的文本摘要，且业务输入会以不可信用户内容传给分类器，不会作为系统指令执行。

## 架构

```text
Codex (default) -> https://gateway.example.com/openai/v1 -> server -> default routing profile
Codex (account-a) -> https://gateway.example.com/instances/account-a/openai/v1 -> server -> account-a routing profile

Browser -> Web 管理端 -> 生成一次性 setup.sh / restore.sh 命令
                           -> 修改或恢复 ~/.codex/config.toml
                           -> 为原 Provider 历史创建 ai-gateway 别名

Browser -> Web 管理端 -> 生成一次性 instances.sh 命令（macOS）
                           -> 每个 Codex 窗口使用独立的 CODEX_HOME
                           -> 每个实例使用独立的 auth.json 和 Electron 数据目录
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
| `AI_GATEWAY_ENCRYPTION_KEY` | 无（必填） | 用于加密数据库凭据的 Base64 编码 32 字节密钥 |

Server 不再读取或修改服务器用户的 `~/.codex`。

Codex 模型接口的客户端版本默认使用代码内置值 `0.146.0`。可在 Web 管理端的“网关设置”中写入数据库覆盖值；恢复默认时会删除数据库覆盖值并重新使用代码内置版本。更新或恢复版本都会自动清理 OpenAI 模型缓存。

当前网关接口还没有客户端鉴权。正式暴露到公网前，应放在带 TLS 和访问控制的反向代理后，或者先完成网关 API Key/多租户改造。

## 数据库凭据加密

网关使用 AES-256-GCM 对 SQLite 中的以下凭据列进行应用层加密（每次写入使用随机 nonce，并带完整性校验）：

- `accounts.access_token`
- `accounts.refresh_token`
- `providers.api_key`

密钥不能放进数据库或 Git 仓库。为服务进程和迁移脚本设置同一个密钥，例如：

```bash
export AI_GATEWAY_ENCRYPTION_KEY="$(openssl rand -base64 32)"
```

现有数据库需要在**停止网关服务后**迁移一次。下面的脚本会先使用 SQLite 的 `.backup` 创建备份，再在单个事务中加密上述列；已加密的值会被校验但不会重复加密：

```bash
AI_GATEWAY_ENCRYPTION_KEY='同一个Base64密钥' \
  ./server/scripts/encrypt-existing-db.sh "$HOME/.ai-gateway/db.sqlite"
```

迁移成功后再以相同的 `AI_GATEWAY_ENCRYPTION_KEY` 启动服务。丢失或更换该密钥将无法恢复已有凭据，因此请将密钥保存在独立的密钥管理系统或受限的部署环境变量中。

## Codex 接入脚本

无需安装本地 Agent。在本机终端执行：

```bash
curl -fsSL 'https://gateway.example.com/codex/setup.sh' |
  sh -s -- 'https://gateway.example.com/openai/v1'
```

脚本会：

- 在 `config.toml` 注释中记录切换前的 `model_provider`
- 将 `model_provider` 指向 `ai-gateway`
- 写入当前 Gateway 的 `base_url`
- 为切换前 Provider 的现有任务创建 `ai-gateway` 历史别名
- 首次同步前备份 `state_5.sqlite`，重复执行时复用已有别名
- 使用临时文件原子替换配置
- 执行完成后立即退出，不安装程序或启动后台服务

恢复：

```bash
curl -fsSL 'https://gateway.example.com/codex/restore.sh' | sh
```

恢复脚本只将 `model_provider` 切回原值，并移除保存原值的注释。`model_providers.ai-gateway` 配置和已经创建的历史别名会继续保留，之后可以再次切换，不会恢复或覆盖整个配置文件。

历史同步只复制任务记录和对应 rollout，不修改原任务。映射保存在 `~/.codex/.ai-gateway-history/aliases.tsv`，首次同步前的数据库快照保存在同目录的 `state_5.before-first-sync.sqlite`。如果原 rollout 文件已经缺失，该任务会被跳过并输出警告，不影响 Provider 配置切换。

### Codex 多实例（macOS）

为两个账号分别创建隔离的 Codex 窗口：

```bash
curl -fsSL 'https://gateway.example.com/codex/instances.sh' |
  sh -s -- create account-a 'https://gateway.example.com/instances/account-a/openai/v1'

curl -fsSL 'https://gateway.example.com/codex/instances.sh' |
  sh -s -- create account-b 'https://gateway.example.com/instances/account-b/openai/v1'
```

实例保存在 `~/.ai-gateway/codex-instances/<name>/`。每个实例有独立的 `CODEX_HOME`、`config.toml`、`auth.json`、会话记录和 Electron 用户数据目录，因此可分别在窗口中登录不同的 Codex 账号。创建时不会复制默认实例的 `auth.json`。`skills`、`rules` 和 `AGENTS.md` 会从默认 `~/.codex` 共享，便于复用本地工作流。

### 按 URL path 隔离网关路由

`/openai/v1` 保持原有默认路由。每个 `/instances/<instance-id>/openai/v1` path 都有独立的已选供应商、固定模型、推理强度和自动路由配置；供应商凭据仍在同一个网关中共享。`instance-id` 只能使用字母、数字、`_` 和 `-`。 在 Web 管理端顶部点击“实例”即可新建、编辑实例配置并复制对应的 macOS 启动命令。

实例可先只创建名称，之后再配置默认供应商、固定模型或自动路由。固定模型需要默认供应商；启用自动路由时，可只配置各档位的路由目标而不设置默认供应商。例如 `account-a` 固定使用一个模型，`account-b` 启用自动路由：

```bash
curl -X PUT 'https://gateway.example.com/instances/account-a/config' \
  -H 'Content-Type: application/json' \
  -d '{
    "provider_id": "provider-a-id",
    "selected_model": "model-a",
    "selected_reasoning_effort": "high"
  }'

curl -X PUT 'https://gateway.example.com/instances/account-b/config' \
  -H 'Content-Type: application/json' \
  -d '{
    "provider_id": "provider-b-id",
    "automatic_routing": {
      "enabled": true,
      "classifier": { "provider_id": "router-provider-id", "model": "router-model" },
      "light": { "provider_id": "provider-b-id", "model": "small-model" },
      "standard": { "provider_id": "provider-b-id", "model": "standard-model" },
      "pro": { "provider_id": "provider-b-id", "model": "pro-model" },
      "max": { "provider_id": "provider-b-id", "model": "max-model" },
      "low_confidence_threshold": 0.7
    }
  }'
```

用 `GET /instances/<instance-id>/config` 查看某个实例的独立配置。实例路径同时提供 `/models` 和 `/responses`，因此 Codex 不会回落到默认实例。不同实例的同一 Codex turn ID 会被隔离记录，避免续轮请求串到别的实例。

查看或再次启动实例：

```bash
curl -fsSL 'https://gateway.example.com/codex/instances.sh' | sh -s -- list
curl -fsSL 'https://gateway.example.com/codex/instances.sh' | sh -s -- start account-a
```

## Web 管理端

管理端已迁移为 Web，并由远程 Server 同源托管。技术栈为 React、TypeScript、Bun、Tailwind CSS。

```bash
cd web
bun install
cd ..
cargo run -p ai-gateway
```

`cargo run` 会构建 Web 管理端，并安装到：

```text
$HOME/.ai-gateway/web
```

打开：

```text
http://127.0.0.1:4242/
```

Web 页面与它所在的远程 Server 同源通信，不需要单独配置 Server URL。页面中的“Codex 接入”会根据当前 Server 地址生成一次性接入和恢复命令。

API Key 供应商上游统一使用 OpenAI Responses 接口，并可选择兼容 Profile：

- `official_openai`：保持 Responses 请求 Body 原样。
- `generic_openai`：应用通用兼容清理规则。

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
GET    /settings/codex-client-version
PUT    /settings/codex-client-version
DELETE /settings/codex-client-version
GET    /settings/automatic-routing
PUT    /settings/automatic-routing
GET    /routing/turns?limit=50
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

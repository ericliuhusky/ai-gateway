# ai-gateway

AI Gateway 现在由远程 Server 和 Web 管理端组成：

- `server`：部署在服务器上的网关核心，负责 Provider、账号 Token、路由、协议适配、额度和上游请求。
- `web`：由 `server` 同源托管的管理端，负责远程 Provider、模型、额度和接入配置。

浏览器不能直接修改本机文件，因此 Web 管理端会生成一次性 Shell 命令，由用户在本机终端执行。接入过程不需要安装客户端或启动后台服务。

## 自动模型路由

网关可选地直接使用轻量（`light`）路由目标作为分类模型，为每个普通文本请求选择低、中、高或极高模型。它默认关闭。新 turn 若配置了“已选模型”，会优先使用该模型；同一 turn 在后续工具回合中会粘住首回合选定的模型，不会中途切换。

管理端顶部还可为当前供应商设置“推理强度”覆盖。选择 `low`、`medium`、`high` 或 `xhigh` 后，网关会把每个 Responses 请求中的 `reasoning.effort` 改为该值（会保留 `reasoning` 下的其他字段）；选择“跟随请求”即可取消覆盖。模型和推理强度偏好均按供应商分别保存。

在 Web 管理端的“网关设置 → 自动模型路由”中配置：

- 轻量（`light`）目标同时承担路由分类，仅返回 `low`、`medium`、`high` 或 `xhigh` 和置信度。
- 低级别（`low`，使用 `light` 路由目标并同时承担分类）、中级别（`medium`，使用 `standard` 路由目标）、高级别（`high`，使用 `pro` 路由目标）、极高级别（`xhigh`，使用 `max` 路由目标）：对应最终执行模型；低置信度与分类失败时回退到高级别（`high`）。
- 低置信度阈值固定为 `0.7`：分类置信度低于该值时，自动回退到高级别（`high`）模型。

图片、低置信度、分类失败、返回无效结果或配置不完整时会保守使用专业（`pro`）模型。工具**声明**本身仍会交给轻量路由目标判断；带工具结果的后续请求则复用该 turn 的既定模型。

观测数据保存在同一 SQLite 数据库的 `turn_route_logs` 表中，不会写入响应头。每个 turn 一行，记录首条用户输入的最多 160 字预览、哈希后的 turn ID、模型、路由档位/原因、分类置信度、推理度、请求次数、工具回合数和时间。分类失败时保存最多 500 字的上游错误详情；分类模型返回文本时同样最多保存 500 字，便于判断 JSON 是否有效。不保存完整提示词、工具结果或最终回答。表最多保留 1000 条，按 `updated_at` 作为 LRU 淘汰最旧数据。管理端首页会显示“最近路由 Turn”。

## 网关故障记录与修复提示词

网关只在无法连接上游、上游返回非 2xx、响应体读取失败或流式响应中途断开时，把实际发送给上游的请求和收到的响应写入 SQLite 的 `gateway_issues` 表。成功请求不会写入故障表。

记录按登录用户隔离，每位用户最多保留最近 200 条；每条请求体和响应体分别最多保存 128 KiB，超出后会在 UTF-8 字符边界截断。记录包含实例、供应商、模型、上游 URL、错误类型、HTTP 状态、错误信息和创建时间，但不保存 Authorization 请求头。

管理端首页的“网关问题”区域可查看原始请求/响应。点击“复制修复提示词”会生成一段包含故障证据、安全约束和测试要求的提示词并复制到剪贴板；网关不会执行修复，用户可将提示词粘贴到 Codex 或其他 Agent 的用户输入中。点击“一键清空”会删除当前用户的全部故障记录。

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
| 无 | — | 网关不再读取业务配置环境变量；固定监听 `0.0.0.0:4242`，数据目录固定为 `$HOME/.ai-gateway`。 |

Server 不再读取或修改服务器用户的 `~/.codex`。

Codex 模型接口的客户端版本默认使用代码内置值 `0.146.0`。可在 Web 管理端的“网关设置”中写入数据库覆盖值；恢复默认时会删除数据库覆盖值并重新使用代码内置版本。更新或恢复版本都会自动清理 OpenAI 模型缓存。

管理控制台 API 已要求登录；Codex 网关请求接口仍未提供客户端 API Key 鉴权。正式暴露到公网前，应将网关放在带 TLS 和访问控制的反向代理后，或继续完成网关 API Key/多租户改造。

## 数据库凭据加密

网关使用 AES-256-GCM 对 SQLite 中的以下凭据列进行应用层加密（每次写入使用随机 nonce，并带完整性校验）：

- `accounts.access_token`
- `accounts.refresh_token`
- `providers.api_key`
- `gateway_state.feishu_app_secret`
- `gateway_access_tokens.token_ciphertext`

首次创建数据库时，网关会自动生成并保存一个 Base64 编码、解码后为 32 字节的数据库加密密钥（等同于执行以下命令）：

```bash
openssl rand -base64 32
```

已有旧数据库且尚未设置密钥时，网关仍允许管理员完成初始配置，但会拒绝保存 Provider API Key、OpenAI 账户 Token 和用户网关 API Key 等需加密的内容。管理员设置页只提供系统生成密钥的操作，不显示或接受密钥内容。点击生成或重新生成后，经二次确认即立即生效。网关会在一个 SQLite 事务中先解密全部已加密凭据，再用新密钥重新加密并保存新密钥。任一条记录无法解密时，整个更换会回滚。

> 注意：按当前设计，加密密钥本身保存在 SQLite，因此数据库文件的访问控制仍然至关重要；请限制 `$HOME/.ai-gateway/db.sqlite` 的文件权限并定期备份。

## Codex 接入脚本

无需安装本地 Agent。在本机终端执行：

```bash
curl -fsSL 'https://gateway.example.com/codex/setup.sh' |
  sh -s -- 'https://gateway.example.com/openai/v1'
```

账户模式下，将管理端生成的 API Key 作为第二个参数传入：

```bash
curl -fsSL 'https://gateway.example.com/codex/setup.sh' |
  sh -s -- 'https://gateway.example.com/openai/v1' 'agw_...'
```

脚本会：

- 在 `config.toml` 注释中记录切换前的 `model_provider`
- 将 `model_provider` 指向 `ai-gateway`
- 写入当前 Gateway 的 `base_url`
- 为切换前 Provider 的现有任务创建 `ai-gateway` 历史别名
- 首次同步前备份 `state_5.sqlite`，重复执行时复用已有别名
- 使用临时文件原子替换配置
- 执行完成后立即退出，不安装程序或启动后台服务

清理默认 Codex 设置：

```bash
curl -fsSL 'https://gateway.example.com/codex/restore.sh' | sh
```

清理脚本会将 `model_provider` 切回原值，并移除保存原值的注释及 `[model_providers.ai-gateway]` TOML 配置。历史别名不会删除，以免误删已有任务记录。

历史同步只复制任务记录和对应 rollout，不修改原任务。映射保存在 `~/.codex/.ai-gateway-history/aliases.tsv`，首次同步前的数据库快照保存在同目录的 `state_5.before-first-sync.sqlite`。如果原 rollout 文件已经缺失，该任务会被跳过并输出警告，不影响 Provider 配置切换。

### Codex 多实例（macOS）

为两个账号分别创建隔离的 Codex 窗口：

```bash
curl -fsSL 'https://gateway.example.com/codex/instances.sh' |
  sh -s -- create account-a 'https://gateway.example.com/instances/account-a/openai/v1'

curl -fsSL 'https://gateway.example.com/codex/instances.sh' |
  sh -s -- create account-b 'https://gateway.example.com/instances/account-b/openai/v1'
```

账户模式下，`create` 命令可在实例 URL 后附加 API Key。

实例保存在 `~/.ai-gateway/codex-instances/<name>/`。每个实例有独立的 `CODEX_HOME`、`config.toml`、`auth.json`、会话记录和 Electron 用户数据目录，因此可分别在窗口中登录不同的 Codex 账号。创建时不会复制默认实例的 `auth.json`。`skills`、`rules` 和 `AGENTS.md` 会从默认 `~/.codex` 共享，便于复用本地工作流。

删除本机隔离实例及其配置、登录信息、会话和 Electron 数据：

```bash
curl -fsSL 'https://gateway.example.com/codex/instances.sh' |
  sh -s -- delete account-a
```

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
      "light": { "provider_id": "provider-b-id", "model": "small-model" },
      "standard": { "provider_id": "provider-b-id", "model": "standard-model" },
      "pro": { "provider_id": "provider-b-id", "model": "pro-model" },
      "max": { "provider_id": "provider-b-id", "model": "max-model" }
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

Web 页面与它所在的远程 Server 同源通信，不需要单独配置 Server URL。默认实例和每个普通实例卡片都提供“Codex 脚本”按钮：默认实例展示设置与清理命令，普通实例展示新建与删除隔离 Codex 实例的命令。新建普通实例后会自动显示对应的新建脚本；删除前也会展示本地实例清理命令。

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
POST   /benchmarks/models
GET    /settings/codex-client-version
PUT    /settings/codex-client-version
DELETE /settings/codex-client-version
GET    /settings/automatic-routing
PUT    /settings/automatic-routing
GET    /routing/turns?limit=50
GET    /gateway/issues?limit=50
GET    /gateway/issues/:issue_id/repair-prompt
DELETE /gateway/issues
GET    /selected-provider
PUT    /selected-provider
GET    /selected-model
PUT    /selected-model
DELETE /selected-model
GET    /openai/v1/models
POST   /openai/v1/responses
```

`POST /benchmarks/models` 会向指定供应商发送 1–5 次固定的 Rust 斐波那契代码生成请求，
返回真实的首 token 延迟（TTFT）、总耗时和生成 tokens/s 中位数；该接口会消耗上游
token，管理控制台默认执行 3 次。

所有推理请求仍统一使用 OpenAI Responses 客户端协议：

```bash
curl -X POST http://127.0.0.1:4242/openai/v1/responses \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "gpt-5.4",
    "input": "hello"
  }'
```

## 管理控制台账户与登录

认证通过管理端“管理员设置 → 安全与账户登录”中的“账户登录模式”开关配置：

- `disabled`（默认）：不启用账户系统，管理端和网关接口保持当前的无鉴权单人部署行为。
- `required`：管理端必须使用飞书 OAuth 登录。首个成功登录的用户自动成为 `admin`，后续用户为 `user`。飞书身份、角色和会话保存在 `$HOME/.ai-gateway/db.sqlite` 的 `gateway_users`、`gateway_feishu_identities`、`gateway_sessions` 表中。

开启前必须设置数据库加密密钥、飞书 App ID 和飞书 App Secret。App ID 和 App Secret 填写完成后自动保存；账户登录模式开关点击后自动保存并立即生效。管理员可在账户登录模式下再次进入该页面关闭它。

在 `required` 模式下，网关请求还必须使用用户自己的 API Key：

```http
Authorization: Bearer agw_...
```

登录管理端后，点击顶部的“API Key”生成并复制。该 Key 仅在生成时显示一次；Provider、OpenAI 账户、默认实例及命名实例均按该 Key 对应的用户隔离。

在飞书应用的 OAuth 重定向 URL 白名单中登记：

```text
https://gateway.example.com/auth/feishu/callback
```

网关会根据请求的 `Host` 与 `X-Forwarded-Proto` / `X-Forwarded-Host` 生成回调地址。

以下**管理 API**需要飞书登录：供应商和 OpenAI 账号导入、网关设置、路由设置、实例管理和路由日志。为兼容已部署的 Codex 客户端，请求转发接口、模型查询接口、健康检查和一次性 Codex 脚本仍保持公开；生产环境应继续在反向代理层为这些接口配置 TLS 与客户端访问控制。

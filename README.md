# ai-gateway

## 架构词汇

- `Ingress`: 网关对外暴露的入口协议。当前只有 `OpenAI Responses`
- `Egress`: 网关对上游发起请求时使用的出口协议。当前固定 4 种：
  - `OpenAI private responses`
  - `Google v1internal`
  - `Native responses`
  - `Native chat completions`
- `Provider`: 具体供应商实例，例如 `openai-proxy`、`google-proxy`、`bytedance`
- `Route`: 当前把入口请求转发到哪个 provider
- `Adapter`: 从统一 `Responses` 入口适配到具体出口协议的转换层

当前实现遵循“单一入口、多个出口”的结构：

- 所有推理请求统一从 `POST /openai/v1/responses` 进入
- 路由层根据当前选中的 provider 决定出口协议
- adapter 层负责把入口 `Responses` 适配到对应的出口协议
- upstream 层只负责调用真实上游接口

参考 [`Antigravity-Manager`](https://github.com/lbjlaq/Antigravity-Manager) 的代理实现：

- 用户通过 Google OAuth 登录
- 用户也可以仿照 Codex / OpenClaw 的方式，通过 ChatGPT OAuth + PKCE 登录
- 登录成功后账号会写入本地账号池 `~/.ai-gateway/accounts/*.json`
- provider 如果使用账号登录，会绑定本地账号池里的账号
- access token 过期前自动刷新
- project_id 缺失时通过私有 `v1internal:loadCodeAssist` 获取
- `POST /openai/v1/responses` 走私有 `cloudcode-pa.googleapis.com/v1internal`

## 运行

```bash
cargo run
```

默认固定监听 `127.0.0.1:10100`。

账号数据固定保存在 `~/.ai-gateway/accounts/*.json`。
provider 数据固定保存在 `~/.ai-gateway/providers/*.json`。

## 登录

浏览器打开：

```bash
open http://127.0.0.1:10100/auth/google/start
```

登录成功后，账号会被加入本地账号池。可用下面接口查看：

如果你想直接走浏览器 OAuth，而不是导入本地文件：

```bash
open http://127.0.0.1:10100/auth/openai/start
```

这会仿照 Codex / OpenClaw：

- 生成 PKCE `code_verifier` / `code_challenge`
- 打开 `https://auth.openai.com/oauth/authorize`
- 使用固定回调 `http://localhost:1455/auth/callback`
- 回调后向 `https://auth.openai.com/oauth/token` 交换 token
- 从 access token 提取 `accountId`

注意：服务除了 `127.0.0.1:10100` 外，还会额外监听 `127.0.0.1:1455` 以接收 OpenAI OAuth 回调。

注意：按 OpenAI 官方文档，Codex 的 ChatGPT 登录和 API key 登录是两条不同的访问路径。当前 ChatGPT/Codex OAuth 会话，实测可能不带公开 `POST /openai/v1/responses` 所需的 `api.responses.write` scope；如果你要稳定访问公开 OpenAI API，仍应优先使用 API key。

## 原生 API 供应商

除了 `openai-proxy` / `google-proxy` 这类 OAuth 代理供应商，现在也支持登记“原生 key 的 API 供应商”配置。`provider` 是统一入口：

- `api_key` 型 provider 通过 `POST /providers` 手动创建
- `account` 型 provider 只能通过 OAuth 登录自动创建或更新
- 用户不会手动绑定 account；登录成功后系统会自动维护 `provider <-> account` 这一对一关系

```bash
curl -X POST http://127.0.0.1:10100/providers \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "bytedance",
    "base_url": "https://ark.cn-beijing.volces.com/api/v3",
    "api_key": "sk-xxx",
    "billing_mode": "metered"
  }'
```

其中：

- `name`: 供应商名，例如 `openai`、`google`、`bytedance`、`bytedance-coding-plan`、`local-8080`
- `base_url`: 该供应商的 API 基础地址
- `api_key`: 上游 API key
- `billing_mode`: `metered` 或 `subscription`
  - `metered`: 按量计费，通常按 token、请求次数或实际用量扣费
  - `subscription`: 订阅制 / 套餐制，通常不是每次调用单独计费

`POST /providers` 不接受 `auth_mode` 或 `account_id`；这个接口默认创建的就是 `api_key` 型 provider。

查看已登记的供应商：

```bash
curl http://127.0.0.1:10100/providers
```

### 列出供应商模型

```bash
curl 'http://127.0.0.1:10100/openai/v1/models'
```

- 返回当前路由选中的供应商模型列表
- 必须先通过 `/selected-provider` 明确选择 provider

## 选择当前 Provider

所有转发统一都走：

```bash
curl -X POST http://127.0.0.1:10100/openai/v1/responses \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "gpt-5.4",
    "input": "hello"
  }'
```

如果你想查看当前选择的 provider，可以调用：

```bash
curl http://127.0.0.1:10100/selected-provider
```

返回当前路由状态，例如：

```json
{
  "selected_provider": {
    "provider": "bytedance"
  }
}
```

更新为某个指定供应商：

```bash
curl -X PUT http://127.0.0.1:10100/selected-provider \
  -H 'Content-Type: application/json' \
  -d '{
    "provider": "bytedance"
  }'
```

`provider` 可以是：

- `openai-proxy`
- `google-proxy`
- 通过 `/providers` 已登记的原生供应商名

## 路由行为

- 当前这两个 OAuth 供应商被视为“账号型 provider”：
  - `openai-proxy`: 使用 ChatGPT OAuth，会转发到 `https://chatgpt.com/backend-api/codex/responses`
  - `google-proxy`: 使用 Google OAuth，会转发到 Gemini 私有 `v1internal`
- OAuth 登录成功后，会自动创建或更新对应 provider，并绑定到刚登录的本地 account
- 当前设计要求 `account` 和 `provider` 一对一存在：要么同时存在，要么同时不存在
- 不再提供自动路由；所有 `/openai/v1/models` 和 `/openai/v1/responses` 调用都依赖用户显式选择的 provider
- `account` 不再对外暴露接口，只作为 provider 的内部认证信息存在

## 当前范围

- 已实现：Google 登录、OpenAI 浏览器 OAuth + PKCE 登录、账号持久化、账号轮询、token 刷新、`project_id` 获取、OpenAI Responses -> Gemini v1internal、GPT 请求直连 ChatGPT Codex backend-api、最小函数工具调用映射
- 暂未实现：复杂配额保护、设备指纹、官方客户端全部 Header 指纹、更多管理接口

# gemini-proxy

参考 [`Antigravity-Manager`](https://github.com/lbjlaq/Antigravity-Manager) 的gemini代理：

- 用户通过 Google OAuth 登录
- 用户也可以仿照 Codex / OpenClaw 的方式，通过 ChatGPT OAuth + PKCE 登录
- 登录成功后账号会写入本地账号池 `~/.gemini-proxy/accounts/*.json`
- 代理请求时从账号池轮询账号
- access token 过期前自动刷新
- project_id 缺失时通过私有 `v1internal:loadCodeAssist` 获取
- `POST /v1/responses` 走私有 `cloudcode-pa.googleapis.com/v1internal`

## 运行

```bash
cargo run
```

默认固定监听 `127.0.0.1:10100`。

账号数据固定保存在 `~/.gemini-proxy/accounts/*.json`。

## 登录

浏览器打开：

```bash
open http://127.0.0.1:10100/auth/google/start
```

登录成功后，账号会被加入本地账号池。可用下面接口查看：

```bash
curl http://127.0.0.1:10100/v1/accounts
```

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

注意：按 OpenAI 官方文档，Codex 的 ChatGPT 登录和 API key 登录是两条不同的访问路径。当前 ChatGPT/Codex OAuth 会话，实测可能不带公开 `POST /v1/responses` 所需的 `api.responses.write` scope；如果你要稳定访问公开 OpenAI API，仍应优先使用 API key。

## 路由行为

- `gpt-*`、`o1*`、`o3*`、`o4*`、`codex-*` 模型会直接转发到 ChatGPT Codex `POST https://chatgpt.com/backend-api/codex/responses`
- 其他模型继续走 Gemini 私有 `v1internal`

## 当前范围

- 已实现：Google 登录、OpenAI 浏览器 OAuth + PKCE 登录、账号持久化、账号轮询、token 刷新、`project_id` 获取、OpenAI Responses -> Gemini v1internal、GPT 请求直连 OpenAI Responses、最小函数工具调用映射
- 暂未实现：复杂配额保护、设备指纹、官方客户端全部 Header 指纹、更多管理接口

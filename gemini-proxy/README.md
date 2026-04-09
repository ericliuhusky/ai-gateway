# gemini-proxy

参考 [`Antigravity-Manager`](https://github.com/lbjlaq/Antigravity-Manager) 的gemini代理：

- 用户通过 Google OAuth 登录
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

## 当前范围

- 已实现：Google 登录、账号持久化、账号轮询、token 刷新、`project_id` 获取、OpenAI Responses -> Gemini v1internal、最小函数工具调用映射
- 暂未实现：复杂配额保护、设备指纹、官方客户端全部 Header 指纹、更多管理接口

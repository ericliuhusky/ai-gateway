use leptos::prelude::*;
use serde_json::Value;

const LONG_VALUE_PREVIEW_CHARS: usize = 500;

#[derive(Clone, Debug)]
pub struct DebugPageData {
    pub logging_enabled: bool,
    pub logs: Vec<DebugLogSummary>,
    pub selected_request_id: Option<String>,
    pub selected_detail: Option<DebugLogDetail>,
    pub limit: usize,
    pub notice: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DebugLogSummary {
    pub request_id: String,
    pub updated_at_label: String,
    pub provider_name: Option<String>,
    pub account_email: Option<String>,
    pub model: Option<String>,
    pub stream: bool,
    pub status_code: Option<u16>,
    pub has_error: bool,
    pub error_message: Option<String>,
    pub ingress_protocol: Option<String>,
    pub egress_protocol: Option<String>,
    pub event_count: usize,
}

#[derive(Clone, Debug)]
pub struct DebugLogEvent {
    pub id: i64,
    pub stage: String,
    pub created_at_label: String,
    pub status_code: Option<u16>,
    pub ingress_protocol: Option<String>,
    pub egress_protocol: Option<String>,
    pub provider_name: Option<String>,
    pub account_id: Option<String>,
    pub account_email: Option<String>,
    pub model: Option<String>,
    pub stream: bool,
    pub method: Option<String>,
    pub path: Option<String>,
    pub url: Option<String>,
    pub body: Option<String>,
    pub body_truncated: bool,
    pub error_message: Option<String>,
    pub error_truncated: bool,
    pub elapsed_ms: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct DebugLogDetail {
    pub request_id: String,
    pub events: Vec<DebugLogEvent>,
}

pub fn render_debug_page(data: DebugPageData) -> String {
    let body = view! { <DebugApp data=data/> }.to_html();

    format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"/><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"/><title>debug-web</title></head><body>{}</body></html>",
        body
    )
}

#[component]
fn DebugApp(data: DebugPageData) -> impl IntoView {
    let toggle_enabled_value = if data.logging_enabled { "false" } else { "true" };
    let toggle_label = if data.logging_enabled {
        "暂停日志"
    } else {
        "开启日志"
    };
    let selected_request_id = data.selected_request_id.clone();

    view! {
        <main class="debug-shell">
            <style>{STYLE}</style>
            <section class="hero">
                <div>
                    <p class="eyebrow">"debug-web"</p>
                    <h1>"网关日志调试台"</h1>
                    <p class="hero-copy">
                        "这里直接托管在 /debug，下钻查看 request_id 的完整入口、出口与错误链路。"
                    </p>
                </div>
                <div class="hero-actions">
                    <a class="button secondary" href=format!("/debug?limit={}", data.limit)>
                        "刷新"
                    </a>
                    <form method="post" action="/debug/logging">
                        <input type="hidden" name="enabled" value=toggle_enabled_value/>
                        <HiddenContextInputs request_id=selected_request_id.clone() limit=data.limit/>
                        <button class="button primary" type="submit">{toggle_label}</button>
                    </form>
                    <form method="post" action="/debug/clear">
                        <input type="hidden" name="limit" value=data.limit.to_string()/>
                        <button class="button danger" type="submit">"清空日志"</button>
                    </form>
                </div>
            </section>

            <section class="status-row">
                <span class=if data.logging_enabled { "pill live" } else { "pill paused" }>
                    {if data.logging_enabled { "记录中" } else { "已暂停" }}
                </span>
                <span class="pill neutral">{format!("{} 条请求", data.logs.len())}</span>
                <span class="pill neutral">{format!("limit {}", data.limit)}</span>
            </section>

            {data.notice.as_ref().map(|message| view! {
                <div class="banner notice">{message.clone()}</div>
            })}
            {data.error.as_ref().map(|message| view! {
                <div class="banner error">{message.clone()}</div>
            })}

            <section class="grid">
                <div class="panel">
                    <div class="panel-header">
                        <div>
                            <h2>"请求列表"</h2>
                            <p>"选择一个 request_id 查看完整明细。"</p>
                        </div>
                    </div>
                    <div class="log-list">
                        {if data.logs.is_empty() {
                            view! {
                                <div class="empty-state">
                                    <strong>"还没有日志"</strong>
                                    <p>
                                        {if data.logging_enabled {
                                            "等网关收到请求后，这里会出现最新摘要。"
                                        } else {
                                            "当前日志已暂停，恢复记录后才会继续写入。"
                                        }}
                                    </p>
                                </div>
                            }.into_any()
                        } else {
                            data.logs
                                .iter()
                                .map(|log| {
                                    let href = format!(
                                        "/debug?limit={}&request_id={}",
                                        data.limit, log.request_id
                                    );
                                    let is_selected = data
                                        .selected_request_id
                                        .as_deref()
                                        == Some(log.request_id.as_str());
                                    let row_class = if is_selected {
                                        "log-row selected"
                                    } else {
                                        "log-row"
                                    };

                                    view! {
                                        <a class=row_class href=href>
                                            <div class="row-top">
                                                <div>
                                                    <strong>{log.provider_name.clone().unwrap_or_else(|| "未知供应商".to_string())}</strong>
                                                    <span class="mono">{log.request_id.clone()}</span>
                                                </div>
                                                <span class=if log.has_error { "badge error" } else { "badge ok" }>
                                                    {match log.status_code {
                                                        Some(code) => code.to_string(),
                                                        None => "PENDING".to_string(),
                                                    }}
                                                </span>
                                            </div>
                                            <div class="tag-row">
                                                <span class="tag">{if log.stream { "SSE" } else { "JSON" }}</span>
                                                <span class="tag">{log.model.clone().unwrap_or_else(|| "未知模型".to_string())}</span>
                                                <span class="tag">{format!("{} 事件", log.event_count)}</span>
                                            </div>
                                            <div class="meta-grid">
                                                <span>{format!("入口 {}", log.ingress_protocol.clone().unwrap_or_else(|| "-".to_string()))}</span>
                                                <span>{format!("出口 {}", log.egress_protocol.clone().unwrap_or_else(|| "-".to_string()))}</span>
                                                <span>{log.account_email.clone().unwrap_or_default()}</span>
                                                <span>{log.updated_at_label.clone()}</span>
                                            </div>
                                            {log.error_message.as_ref().map(|message| view! {
                                                <p class="error-copy">{message.clone()}</p>
                                            })}
                                        </a>
                                    }
                                })
                                .collect_view()
                                .into_any()
                        }}
                    </div>
                </div>

                <div class="panel detail">
                    <div class="panel-header">
                        <div>
                            <h2>"日志详情"</h2>
                            <p>"按时间顺序检查每一步事件、请求体和响应体。"</p>
                        </div>
                    </div>
                    {match data.selected_detail.clone() {
                        Some(detail) => view! {
                            <div class="detail-header">
                                <span class="mono">{detail.request_id.clone()}</span>
                                <span class="pill neutral">{format!("{} 个事件", detail.events.len())}</span>
                            </div>
                            <div class="event-list">
                                {detail.events.iter().map(|event| view! {
                                    <article class="event-card">
                                        <div class="event-top">
                                            <div>
                                                <strong>{event.stage.clone()}</strong>
                                                <span class="subtle">{event.created_at_label.clone()}</span>
                                            </div>
                                            <div class="event-meta">
                                                {event.status_code.map(|code| view! {
                                                    <span class="badge ok">{code.to_string()}</span>
                                                })}
                                                {event.elapsed_ms.map(|elapsed| view! {
                                                    <span class="pill neutral">{format!("{} ms", elapsed)}</span>
                                                })}
                                            </div>
                                        </div>
                                        <dl class="kv-grid">
                                            <KeyValue label="Provider" value=event.provider_name.clone()/>
                                            <KeyValue label="Account" value=event.account_email.clone().or(event.account_id.clone())/>
                                            <KeyValue label="Model" value=event.model.clone()/>
                                            <KeyValue label="Stream" value=Some(if event.stream { "true".to_string() } else { "false".to_string() })/>
                                            <KeyValue label="Ingress" value=event.ingress_protocol.clone()/>
                                            <KeyValue label="Egress" value=event.egress_protocol.clone()/>
                                            <KeyValue label="Method" value=event.method.clone()/>
                                            <KeyValue label="Path" value=event.path.clone()/>
                                            <KeyValue label="URL" value=event.url.clone()/>
                                        </dl>
                                        {event.error_message.as_ref().map(|message| view! {
                                            <div class="block error-block">
                                                <div class="block-title">
                                                    "错误"
                                                    {if event.error_truncated { "（已截断）" } else { "" }}
                                                </div>
                                                <JsonBlock content=message.clone() root_label="error"/>
                                            </div>
                                        })}
                                        {event.body.as_ref().map(|body| view! {
                                            <div class="block">
                                                <div class="block-title">
                                                    "Body"
                                                    {if event.body_truncated { "（已截断）" } else { "" }}
                                                </div>
                                                <JsonBlock content=body.clone() root_label="body"/>
                                            </div>
                                        })}
                                    </article>
                                }).collect_view()}
                            </div>
                        }.into_any(),
                        None => view! {
                            <div class="empty-state detail-empty">
                                <strong>"没有选中的日志"</strong>
                                <p>"左侧选中一个 request_id 后，这里会显示完整事件链路。"</p>
                            </div>
                        }.into_any(),
                    }}
                </div>
            </section>
        </main>
    }
}

#[component]
fn HiddenContextInputs(request_id: Option<String>, limit: usize) -> impl IntoView {
    view! {
        <input type="hidden" name="limit" value=limit.to_string()/>
        {request_id.map(|request_id| view! {
            <input type="hidden" name="request_id" value=request_id/>
        })}
    }
}

#[component]
fn KeyValue(label: &'static str, value: Option<String>) -> impl IntoView {
    view! {
        <div class="kv-row">
            <dt>{label}</dt>
            <dd>{value.unwrap_or_else(|| "-".to_string())}</dd>
        </div>
    }
}

#[component]
fn JsonBlock(content: String, root_label: &'static str) -> impl IntoView {
    match serde_json::from_str::<Value>(&content) {
        Ok(value) => view! {
            <div class="json-tree-shell">
                <JsonNode label=Some(root_label.to_string()) value=value/>
            </div>
        }
        .into_any(),
        Err(_) => view! {
            <LongTextBlock text=content quoted=false class_name="plain-text-block"/>
        }
        .into_any(),
    }
}

#[component]
fn JsonNode(label: Option<String>, value: Value) -> impl IntoView {
    match value {
        Value::Object(map) => {
            let item_count = map.len();
            let summary = format!("{{{item_count}}}");
            view! {
                <details class="json-node">
                    <summary>
                        {label.clone().map(|label| view! { <span class="json-key">{label}</span> })}
                        <span class="json-punct">{": "}</span>
                        <span class="json-summary">{summary}</span>
                    </summary>
                    <div class="json-children">
                        {map
                            .into_iter()
                            .map(|(child_label, child_value)| view! {
                                <JsonNode label=Some(child_label) value=child_value/>
                            })
                            .collect_view()}
                    </div>
                </details>
            }
            .into_any()
        }
        Value::Array(items) => {
            let item_count = items.len();
            let summary = format!("[{item_count}]");
            view! {
                <details class="json-node">
                    <summary>
                        {label.clone().map(|label| view! { <span class="json-key">{label}</span> })}
                        <span class="json-punct">{": "}</span>
                        <span class="json-summary">{summary}</span>
                    </summary>
                    <div class="json-children">
                        {items
                            .into_iter()
                            .enumerate()
                            .map(|(index, child_value)| view! {
                                <JsonNode label=Some(index.to_string()) value=child_value/>
                            })
                            .collect_view()}
                    </div>
                </details>
            }
            .into_any()
        }
        Value::String(text) => view! {
            <div class="json-leaf">
                {label.map(|label| view! { <span class="json-key">{label}</span> })}
                <span class="json-punct">{": "}</span>
                <LongTextBlock text=text quoted=true class_name="json-string"/>
            </div>
        }
        .into_any(),
        Value::Number(number) => view! {
            <div class="json-leaf">
                {label.map(|label| view! { <span class="json-key">{label}</span> })}
                <span class="json-punct">{": "}</span>
                <span class="json-number">{number.to_string()}</span>
            </div>
        }
        .into_any(),
        Value::Bool(boolean) => view! {
            <div class="json-leaf">
                {label.map(|label| view! { <span class="json-key">{label}</span> })}
                <span class="json-punct">{": "}</span>
                <span class="json-bool">{boolean.to_string()}</span>
            </div>
        }
        .into_any(),
        Value::Null => view! {
            <div class="json-leaf">
                {label.map(|label| view! { <span class="json-key">{label}</span> })}
                <span class="json-punct">{": "}</span>
                <span class="json-null">"null"</span>
            </div>
        }
        .into_any(),
    }
}

#[component]
fn LongTextBlock(text: String, quoted: bool, class_name: &'static str) -> impl IntoView {
    if text.chars().count() <= LONG_VALUE_PREVIEW_CHARS {
        return view! {
            <span class=class_name>
                {if quoted { "\"".to_string() } else { String::new() }}
                {text}
                {if quoted { "\"".to_string() } else { String::new() }}
            </span>
        }
        .into_any();
    }

    let preview: String = text.chars().take(LONG_VALUE_PREVIEW_CHARS).collect();

    view! {
        <details class="long-text-toggle">
            <summary>
                <span class=class_name>
                    {if quoted { "\"".to_string() } else { String::new() }}
                    {preview}
                    "..."
                    {if quoted { "\"".to_string() } else { String::new() }}
                </span>
                <span class="long-text-hint">"展开"</span>
            </summary>
            <div class="long-text-expanded">
                <span class=class_name>
                    {if quoted { "\"".to_string() } else { String::new() }}
                    {text}
                    {if quoted { "\"".to_string() } else { String::new() }}
                </span>
            </div>
        </details>
    }
    .into_any()
}

const STYLE: &str = r#"
body {
  margin: 0;
  font-family: "Iowan Old Style", "Palatino Linotype", "Book Antiqua", serif;
  background:
    radial-gradient(circle at top left, rgba(255, 183, 77, 0.18), transparent 28%),
    radial-gradient(circle at top right, rgba(75, 192, 192, 0.15), transparent 24%),
    linear-gradient(180deg, #fcf7ef 0%, #f4efe5 100%);
  color: #1f2430;
}

.debug-shell {
  max-width: 1440px;
  margin: 0 auto;
  padding: 32px 24px 40px;
}

.hero, .panel, .banner {
  border: 1px solid rgba(48, 54, 61, 0.08);
  box-shadow: 0 18px 50px rgba(31, 36, 48, 0.08);
}

.hero {
  display: flex;
  justify-content: space-between;
  gap: 24px;
  padding: 28px;
  border-radius: 28px;
  background: rgba(255, 252, 247, 0.92);
}

.eyebrow {
  margin: 0 0 10px;
  text-transform: uppercase;
  letter-spacing: 0.18em;
  font-size: 12px;
  color: #9b5c22;
}

h1, h2, p {
  margin: 0;
}

h1 {
  font-size: clamp(32px, 4vw, 52px);
  line-height: 1;
}

.hero-copy, .panel-header p, .empty-state p, .subtle {
  color: #5d6472;
}

.hero-copy {
  margin-top: 12px;
  max-width: 52ch;
  line-height: 1.5;
}

.hero-actions, .status-row, .tag-row, .row-top, .event-top, .event-meta {
  display: flex;
  gap: 12px;
  flex-wrap: wrap;
}

.hero-actions {
  align-items: flex-start;
  justify-content: flex-end;
}

form {
  margin: 0;
}

.button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 42px;
  padding: 0 16px;
  border-radius: 999px;
  border: none;
  text-decoration: none;
  cursor: pointer;
  font: inherit;
  font-weight: 700;
}

.button.primary { background: #1f6f78; color: #fff; }
.button.secondary { background: rgba(31, 111, 120, 0.1); color: #1f6f78; }
.button.danger { background: #b43f32; color: #fff; }

.status-row {
  margin: 18px 0 14px;
}

.pill, .tag, .badge {
  display: inline-flex;
  align-items: center;
  border-radius: 999px;
  padding: 6px 10px;
  font-size: 12px;
  font-weight: 700;
}

.pill.live { background: rgba(33, 150, 83, 0.14); color: #20643a; }
.pill.paused { background: rgba(180, 63, 50, 0.12); color: #8f352b; }
.pill.neutral, .tag { background: rgba(31, 36, 48, 0.07); color: #4f5663; }
.badge.ok { background: rgba(31, 111, 120, 0.12); color: #14545b; }
.badge.error { background: rgba(180, 63, 50, 0.14); color: #8f352b; }

.banner {
  margin-bottom: 14px;
  padding: 14px 18px;
  border-radius: 18px;
  background: rgba(255, 252, 247, 0.92);
}

.banner.notice { border-color: rgba(33, 150, 83, 0.18); }
.banner.error { border-color: rgba(180, 63, 50, 0.22); color: #8f352b; }

.grid {
  display: grid;
  gap: 18px;
  grid-template-columns: minmax(320px, 420px) minmax(0, 1fr);
}

.panel {
  min-height: 68vh;
  padding: 22px;
  border-radius: 28px;
  background: rgba(255, 252, 247, 0.92);
}

.panel-header {
  margin-bottom: 16px;
}

.log-list, .event-list {
  display: grid;
  gap: 14px;
}

.log-row {
  display: grid;
  gap: 12px;
  padding: 16px;
  border: 1px solid rgba(48, 54, 61, 0.08);
  border-radius: 20px;
  background: rgba(250, 247, 241, 0.9);
  color: inherit;
  text-decoration: none;
}

.log-row.selected {
  border-color: rgba(31, 111, 120, 0.34);
  background: rgba(228, 244, 243, 0.92);
}

.row-top {
  justify-content: space-between;
  align-items: flex-start;
}

.row-top > div {
  display: grid;
  gap: 6px;
}

.mono {
  font-family: "SF Mono", "Menlo", monospace;
  font-size: 12px;
  overflow-wrap: anywhere;
}

.meta-grid, .kv-grid {
  display: grid;
  gap: 8px 14px;
}

.meta-grid {
  grid-template-columns: repeat(2, minmax(0, 1fr));
  color: #5d6472;
  font-size: 12px;
}

.error-copy {
  margin: 0;
  color: #8f352b;
  font-size: 13px;
}

.detail-header {
  display: flex;
  gap: 12px;
  align-items: center;
  margin-bottom: 16px;
}

.event-card {
  display: grid;
  gap: 14px;
  padding: 18px;
  border-radius: 22px;
  background: rgba(248, 244, 237, 0.96);
  border: 1px solid rgba(48, 54, 61, 0.08);
}

.event-top {
  justify-content: space-between;
  align-items: flex-start;
}

.event-top > div:first-child {
  display: grid;
  gap: 4px;
}

.kv-grid {
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.kv-row {
  display: grid;
  gap: 4px;
}

.kv-row dt {
  font-size: 11px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: #7a808d;
}

.kv-row dd {
  margin: 0;
  font-size: 13px;
  overflow-wrap: anywhere;
}

.block {
  display: grid;
  gap: 8px;
}

.block-title {
  font-size: 12px;
  font-weight: 700;
  color: #5d6472;
}

.error-block .block-title {
  color: #8f352b;
}

pre {
  margin: 0;
  padding: 14px;
  border-radius: 18px;
  background: #1e2430;
  color: #eef2f5;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: "SF Mono", "Menlo", monospace;
  font-size: 12px;
  line-height: 1.55;
}

.plain-text-block {
  display: block;
  padding: 14px;
  border-radius: 18px;
  background: #1e2430;
  color: #eef2f5;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: "SF Mono", "Menlo", monospace;
  font-size: 12px;
  line-height: 1.55;
}

.json-tree-shell {
  padding: 12px 14px;
  border-radius: 18px;
  background: #1e2430;
  color: #eef2f5;
  overflow-x: auto;
  font-family: "SF Mono", "Menlo", monospace;
  font-size: 12px;
  line-height: 1.6;
}

.json-node {
  margin-left: 0;
}

.json-node + .json-node,
.json-leaf + .json-node,
.json-node + .json-leaf,
.json-leaf + .json-leaf {
  margin-top: 4px;
}

.json-node summary {
  list-style: none;
  cursor: pointer;
}

.json-node summary::-webkit-details-marker {
  display: none;
}

.json-node summary::before {
  content: "▸";
  display: inline-block;
  width: 12px;
  margin-right: 6px;
  color: #8cb9ff;
}

.json-node[open] > summary::before {
  content: "▾";
}

.json-children {
  margin-left: 18px;
  margin-top: 4px;
  padding-left: 10px;
  border-left: 1px solid rgba(140, 185, 255, 0.22);
}

.json-leaf {
  padding-left: 18px;
  overflow-wrap: anywhere;
}

.long-text-toggle {
  display: inline;
}

.long-text-toggle summary {
  display: inline;
  list-style: none;
  cursor: pointer;
}

.long-text-toggle summary::-webkit-details-marker {
  display: none;
}

.long-text-toggle[open] > summary .long-text-hint::before {
  content: "收起";
}

.long-text-toggle:not([open]) > summary .long-text-hint::before {
  content: "展开";
}

.long-text-hint {
  margin-left: 8px;
  color: #8cb9ff;
  font-size: 11px;
  font-weight: 700;
}

.long-text-hint {
  color: transparent;
}

.long-text-expanded {
  display: inline;
}

.json-key { color: #ffcf8b; }
.json-punct { color: #7d8696; }
.json-summary { color: #8cb9ff; }
.json-string { color: #9be28f; }
.json-number { color: #ffd479; }
.json-bool { color: #ff9f7f; }
.json-null { color: #c6a7ff; }

.empty-state {
  display: grid;
  gap: 8px;
  place-items: center;
  min-height: 260px;
  text-align: center;
  padding: 24px;
}

.detail-empty {
  min-height: 420px;
}

@media (max-width: 980px) {
  .hero {
    flex-direction: column;
  }

  .grid {
    grid-template-columns: 1fr;
  }

  .kv-grid, .meta-grid {
    grid-template-columns: 1fr;
  }
}
"#;

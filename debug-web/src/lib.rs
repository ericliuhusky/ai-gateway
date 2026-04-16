use leptos::prelude::*;
use serde_json::{Map, Value};
use similar::{ChangeTag, TextDiff};
use std::collections::{BTreeMap, BTreeSet};

const LONG_VALUE_PREVIEW_CHARS: usize = 500;

#[derive(Clone, Debug)]
pub struct DebugPageData {
    pub logging_enabled: bool,
    pub logs: Vec<DebugLogSummary>,
    pub selected_id: Option<String>,
    pub selected_detail: Option<DebugLogDetail>,
    pub limit: usize,
    pub notice: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DebugLogSummary {
    pub id: String,
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
}

#[derive(Clone, Debug)]
pub struct DebugLogDetail {
    pub id: String,
    pub created_at_label: String,
    pub updated_at_label: String,
    pub provider_name: Option<String>,
    pub account_id: Option<String>,
    pub account_email: Option<String>,
    pub model: Option<String>,
    pub stream: bool,
    pub ingress_protocol: Option<String>,
    pub egress_protocol: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub egress_request_url: Option<String>,
    pub ingress_request_body: Option<String>,
    pub ingress_request_body_truncated: bool,
    pub egress_request_body: Option<String>,
    pub egress_request_body_truncated: bool,
    pub ingress_response_status_code: Option<u16>,
    pub ingress_response_body: Option<String>,
    pub ingress_response_body_truncated: bool,
    pub egress_response_status_code: Option<u16>,
    pub egress_response_body: Option<String>,
    pub egress_response_body_truncated: bool,
    pub error_message: Option<String>,
    pub error_truncated: bool,
    pub elapsed_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug)]
enum ComparisonKind {
    Request,
    Response,
}

#[derive(Clone, Copy, Debug)]
enum DiffSide {
    Ingress,
    Egress,
}

#[derive(Clone, Debug)]
enum JsonDiffNode {
    Object(Vec<JsonDiffField>),
    Array(Vec<JsonDiffIndex>),
    Scalar {
        before: Option<Value>,
        after: Option<Value>,
    },
    Summary {
        ingress_value: Option<Value>,
        egress_value: Option<Value>,
        count: usize,
    },
}

#[derive(Clone, Debug)]
struct JsonDiffField {
    key: String,
    change: JsonDiffChange,
}

#[derive(Clone, Debug)]
struct JsonDiffIndex {
    label: String,
    change: JsonDiffChange,
}

#[derive(Clone, Debug)]
enum JsonDiffChange {
    Added(Value),
    Removed(Value),
    Modified(JsonDiffNode),
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
    let toggle_enabled_value = if data.logging_enabled {
        "false"
    } else {
        "true"
    };
    let toggle_label = if data.logging_enabled {
        "暂停日志"
    } else {
        "开启日志"
    };
    let selected_id = data.selected_id.clone();

    view! {
        <main class="debug-shell">
            <style>{STYLE}</style>
            <section class="hero">
                <div>
                    <p class="eyebrow">"debug-web"</p>
                    <h1>"网关日志调试台"</h1>
                    <p class="hero-copy">
                        "这里直接托管在 /debug，下钻查看日志 id 的完整入口、出口与错误链路。"
                    </p>
                </div>
                <div class="hero-actions">
                    <a class="button secondary" href=format!("/debug?limit={}", data.limit)>
                        "刷新"
                    </a>
                    <form method="post" action="/debug/logging">
                        <input type="hidden" name="enabled" value=toggle_enabled_value/>
                        <HiddenContextInputs id=selected_id.clone() limit=data.limit/>
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
                            <p>"选择一个 id 查看完整明细。"</p>
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
                                        "/debug?limit={}&id={}",
                                        data.limit, log.id
                                    );
                                    let is_selected = data
                                        .selected_id
                                        .as_deref()
                                        == Some(log.id.as_str());
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
                                                    <span>{log.account_email.clone().unwrap_or_else(|| "未绑定账户".to_string())}</span>
                                                </div>
                                                <span class=if log.has_error { "badge error" } else { "badge ok" }>
                                                    {match log.status_code {
                                                        Some(code) => code.to_string(),
                                                        None => "PENDING".to_string(),
                                                    }}
                                                </span>
                                            </div>
                                            <div class="tag-row">
                                                <span class="tag">{if log.stream { "流式" } else { "非流式" }}</span>
                                                <span class="tag">{log.model.clone().unwrap_or_else(|| "未知模型".to_string())}</span>
                                            </div>
                                            <div class="meta-grid">
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
                            <p>"直接查看这次请求的入口、出口、响应和错误字段。"</p>
                        </div>
                    </div>
                    {match data.selected_detail.clone() {
                        Some(detail) => view! {
                            <div class="detail-tabs">
                                <input class="detail-tab-input" type="radio" id="tab-detail" name="detail-tab" checked/>
                                <input class="detail-tab-input" type="radio" id="tab-diff" name="detail-tab"/>
                                <div class="detail-tab-bar">
                                    <label class="detail-tab-label" for="tab-detail">"详情"</label>
                                    <label class="detail-tab-label" for="tab-diff">"对比"</label>
                                </div>
                                <div class="detail-tab-panel detail-panel-main">
                                    <div class="event-list">
                                        <article class="event-card">
                                            <div class="event-top">
                                                <div>
                                                    <strong class="mono">{detail.id.clone()}</strong>
                                                    <span class="subtle">{detail.created_at_label.clone()}</span>
                                                    <div class="detail-chip-row">
                                                        <span class="pill neutral">
                                                            {if detail.stream { "流式" } else { "非流式" }}
                                                        </span>
                                                        {detail.model.as_ref().map(|model| view! {
                                                            <span class="pill neutral">{model.clone()}</span>
                                                        })}
                                                    </div>
                                                </div>
                                                <div class="event-meta">
                                                    {detail.egress_response_status_code.or(detail.ingress_response_status_code).map(|code| view! {
                                                        <span class="badge ok">{code.to_string()}</span>
                                                    })}
                                                    {detail.elapsed_ms.map(|elapsed| view! {
                                                        <span class="pill neutral">{format!("{} ms", elapsed)}</span>
                                                    })}
                                                </div>
                                            </div>
                                            <dl class="kv-grid">
                                                <KeyValue label="供应商" value=detail.provider_name.clone()/>
                                                <KeyValue label="账户" value=detail.account_email.clone().or(detail.account_id.clone())/>
                                                <KeyValue label="入口协议" value=detail.ingress_protocol.clone()/>
                                                <KeyValue label="出口协议" value=detail.egress_protocol.clone()/>
                                                <KeyValue label="请求方法" value=detail.method.clone()/>
                                                <KeyValue label="路径" value=detail.path.clone()/>
                                                <KeyValue label="上游地址" value=detail.egress_request_url.clone()/>
                                            </dl>
                                            {detail.error_message.as_ref().map(|message| view! {
                                                <div class="block error-block">
                                                    <div class="block-title">
                                                        "错误"
                                                        {if detail.error_truncated { "（已截断）" } else { "" }}
                                                    </div>
                                                    <JsonBlock content=message.clone() root_label="error"/>
                                                </div>
                                            })}
                                            {detail.ingress_request_body.as_ref().map(|body| view! {
                                                <div class="block">
                                                    <div class="block-title">
                                                        "入口请求"
                                                        {if detail.ingress_request_body_truncated { "（已截断）" } else { "" }}
                                                    </div>
                                                    <JsonBlock content=body.clone() root_label="ingress_request"/>
                                                </div>
                                            })}
                                            {detail.egress_request_body.as_ref().map(|body| view! {
                                                <div class="block">
                                                    <div class="block-title">
                                                        "出口请求"
                                                        {if detail.egress_request_body_truncated { "（已截断）" } else { "" }}
                                                    </div>
                                                    <JsonBlock content=body.clone() root_label="egress_request"/>
                                                </div>
                                            })}
                                            {detail.ingress_response_body.as_ref().map(|body| view! {
                                                <div class="block">
                                                    <div class="block-title">
                                                        {format!("入口响应{}", detail.ingress_response_status_code.map(|code| format!(" ({code})")).unwrap_or_default())}
                                                        {if detail.ingress_response_body_truncated { "（已截断）" } else { "" }}
                                                    </div>
                                                    <JsonBlock content=body.clone() root_label="ingress_response"/>
                                                </div>
                                            })}
                                            {detail.egress_response_body.as_ref().map(|body| view! {
                                                <div class="block">
                                                    <div class="block-title">
                                                        {format!("出口响应{}", detail.egress_response_status_code.map(|code| format!(" ({code})")).unwrap_or_default())}
                                                        {if detail.egress_response_body_truncated { "（已截断）" } else { "" }}
                                                    </div>
                                                    <JsonBlock content=body.clone() root_label="egress_response"/>
                                                </div>
                                            })}
                                        </article>
                                    </div>
                                </div>
                                <div class="detail-tab-panel diff-panel-main">
                                    <ComparisonSection detail=detail.clone() kind=ComparisonKind::Request/>
                                    <ComparisonSection detail=detail.clone() kind=ComparisonKind::Response/>
                                </div>
                            </div>
                        }.into_any(),
                        None => view! {
                            <div class="empty-state detail-empty">
                                <strong>"没有选中的日志"</strong>
                                <p>"左侧选中一个 id 后，这里会显示完整事件链路。"</p>
                            </div>
                        }.into_any(),
                    }}
                </div>
            </section>
        </main>
    }
}

#[component]
fn ComparisonSection(detail: DebugLogDetail, kind: ComparisonKind) -> impl IntoView {
    let (title, left_title, right_title, left_body, right_body) = match kind {
        ComparisonKind::Request => (
            "请求对比",
            "入口请求",
            "出口请求",
            detail.ingress_request_body.clone(),
            detail.egress_request_body.clone(),
        ),
        ComparisonKind::Response => (
            "响应对比",
            "入口响应",
            "出口响应",
            detail.ingress_response_body.clone(),
            detail.egress_response_body.clone(),
        ),
    };

    if left_body.is_none() && right_body.is_none() {
        return ().into_any();
    }

    view! {
        <section class="compare-section">
            <div class="compare-header">
                <h3>{title}</h3>
            </div>
            <BodyDiffBlock
                title="Body Diff"
                left_label=left_title
                right_label=right_title
                left_content=left_body
                right_content=right_body
            />
        </section>
    }
    .into_any()
}

#[component]
fn DiffBlock(
    title: &'static str,
    left_label: &'static str,
    right_label: &'static str,
    left_content: Option<String>,
    right_content: Option<String>,
) -> impl IntoView {
    let left_prepared = left_content
        .as_deref()
        .map(prepare_diff_content)
        .unwrap_or_else(|| "(empty)".to_string());
    let right_prepared = right_content
        .as_deref()
        .map(prepare_diff_content)
        .unwrap_or_else(|| "(empty)".to_string());

    let diff = TextDiff::from_lines(&left_prepared, &right_prepared);
    let grouped_ops = diff.grouped_ops(0);
    let has_changes = grouped_ops.iter().any(|group| {
        group.iter().any(|op| {
            diff.iter_changes(op)
                .any(|change| change.tag() != ChangeTag::Equal)
        })
    });
    let summary = summarize_text_diff(&diff, &grouped_ops);

    view! {
        <div class="diff-block">
            <div class="diff-head">
                <strong>{title}</strong>
                <div class="diff-legend">
                    <span class="diff-side before">{left_label}</span>
                    <span class="diff-side after">{right_label}</span>
                </div>
            </div>
            <DiffSummaryText items=summary/>
            <details class="diff-detail-toggle">
                <summary>"详细 Diff"</summary>
                <div class="diff-shell">
                    {if has_changes {
                        grouped_ops
                            .iter()
                            .enumerate()
                            .map(|(group_index, group)| {
                                let lines = group
                                    .iter()
                                    .flat_map(|op| {
                                        diff.iter_changes(op)
                                            .filter(|change| change.tag() != ChangeTag::Equal)
                                            .map(|change| {
                                                let (prefix, class_name) = match change.tag() {
                                                    ChangeTag::Delete => ("-", "diff-line delete"),
                                                    ChangeTag::Insert => ("+", "diff-line insert"),
                                                    ChangeTag::Equal => (" ", "diff-line equal"),
                                                };
                                                let text = change.to_string();
                                                let text = text.strip_suffix('\n').unwrap_or(&text).to_string();

                                                view! {
                                                    <div class=class_name>
                                                        <span class="diff-prefix">{prefix}</span>
                                                        <span class="diff-text">{text}</span>
                                                    </div>
                                                }
                                            })
                                            .collect_view()
                                    })
                                    .collect_view();

                                view! {
                                    <div class="diff-group">
                                        {if group_index > 0 {
                                            view! { <div class="diff-hidden">"… unchanged lines hidden …"</div> }.into_any()
                                        } else {
                                            ().into_any()
                                        }}
                                        {lines}
                                    </div>
                                }
                            })
                            .collect_view()
                            .into_any()
                    } else {
                        view! {
                            <div class="diff-line equal">
                                <span class="diff-prefix">"="</span>
                                <span class="diff-text">"没有差异"</span>
                            </div>
                        }
                        .into_any()
                    }}
                </div>
            </details>
        </div>
    }
}

#[component]
fn BodyDiffBlock(
    title: &'static str,
    left_label: &'static str,
    right_label: &'static str,
    left_content: Option<String>,
    right_content: Option<String>,
) -> impl IntoView {
    match (
        left_content.as_deref().and_then(parse_json_content),
        right_content.as_deref().and_then(parse_json_content),
    ) {
        (Some(left_json), Some(right_json)) => view! {
            <JsonDiffBlock
                title=title
                left_label=left_label
                right_label=right_label
                left_value=left_json
                right_value=right_json
            />
        }
        .into_any(),
        _ => view! {
            <DiffBlock
                title=title
                left_label=left_label
                right_label=right_label
                left_content=left_content
                right_content=right_content
            />
        }
        .into_any(),
    }
}

#[component]
fn JsonDiffBlock(
    title: &'static str,
    left_label: &'static str,
    right_label: &'static str,
    left_value: Value,
    right_value: Value,
) -> impl IntoView {
    let diff = diff_json_values(&left_value, &right_value);
    let summary = diff.as_ref().and_then(build_json_diff_summary);

    view! {
        <div class="diff-block">
            <div class="diff-head">
                <strong>{title}</strong>
                <div class="diff-legend">
                    <span class="diff-side before"><span class="side-dot ingress"></span>{left_label}</span>
                    <span class="diff-side after"><span class="side-dot egress"></span>{right_label}</span>
                    <span class="diff-side present">"绿色=存在"</span>
                    <span class="diff-side missing">"红色=缺失"</span>
                </div>
            </div>
            <DiffSummaryJson node=summary/>
            <details class="diff-detail-toggle">
                <summary>"详细 Diff"</summary>
                <div class="json-diff-shell">
                    {match diff {
                        Some(node) => view! { <JsonDiffNodeView label=None node=node/> }.into_any(),
                        None => view! {
                            <div class="diff-line equal">
                                <span class="diff-prefix">"="</span>
                                <span class="diff-text">"没有差异"</span>
                            </div>
                        }
                        .into_any(),
                    }}
                </div>
            </details>
        </div>
    }
}

#[component]
fn DiffSummaryText(items: Vec<String>) -> impl IntoView {
    if items.is_empty() {
        return ().into_any();
    }
    view! {
        <div class="diff-summary">
            <div class="diff-summary-title">"差异总结"</div>
            <div class="diff-summary-list">
                {items.into_iter().map(|item| view! {
                    <div class="diff-summary-item">
                        <span>{item}</span>
                    </div>
                }).collect_view()}
            </div>
        </div>
    }
    .into_any()
}

#[component]
fn DiffSummaryJson(node: Option<JsonDiffNode>) -> impl IntoView {
    match node {
        Some(node) => view! {
            <div class="diff-summary">
                <div class="diff-summary-title">"差异总结"</div>
                <div class="json-diff-shell diff-summary-shell">
                    <JsonDiffNodeView label=None node=node/>
                </div>
            </div>
        }
        .into_any(),
        None => ().into_any(),
    }
}

#[component]
fn JsonDiffNodeView(label: Option<String>, node: JsonDiffNode) -> impl IntoView {
    match node {
        JsonDiffNode::Object(fields) => view! {
            <details class="json-diff-node" open>
                <summary>
                    {label.map(|label| view! { <span class="json-key">{label}</span> })}
                    <span class="json-punct">{": "}</span>
                    <span class="json-summary">{format!("{{{} changed}}", fields.len())}</span>
                </summary>
                <div class="json-diff-children">
                    {fields
                        .into_iter()
                        .map(|field| view! {
                            <JsonDiffChangeView label=field.key change=field.change/>
                        })
                        .collect_view()}
                </div>
            </details>
        }
        .into_any(),
        JsonDiffNode::Array(items) => view! {
            <details class="json-diff-node" open>
                <summary>
                    {label.map(|label| view! { <span class="json-key">{label}</span> })}
                    <span class="json-punct">{": "}</span>
                    <span class="json-summary">{format!("[{} changed]", items.len())}</span>
                </summary>
                <div class="json-diff-children">
                    {items
                        .into_iter()
                        .map(|item| view! {
                            <JsonDiffChangeView label=item.label change=item.change/>
                        })
                        .collect_view()}
                </div>
            </details>
        }
        .into_any(),
        JsonDiffNode::Scalar { before, after } => view! {
            <div class="json-diff-leaf">
                {label.map(|label| view! { <span class="json-key">{label}</span> })}
                <span class="json-punct">{": "}</span>
                <div class="json-diff-scalar">
                    {before.map(|value| view! {
                        <JsonSideValueRow side=DiffSide::Ingress state_class="compare" value=Some(value)/>
                    })}
                    {after.map(|value| view! {
                        <JsonSideValueRow side=DiffSide::Egress state_class="compare" value=Some(value)/>
                    })}
                </div>
            </div>
        }
        .into_any(),
        JsonDiffNode::Summary {
            ingress_value,
            egress_value,
            count,
        } => {
            let ingress_state_class = summary_value_state_class(ingress_value.as_ref(), egress_value.as_ref(), true);
            let egress_state_class = summary_value_state_class(ingress_value.as_ref(), egress_value.as_ref(), false);
            view! {
                <div class="json-diff-leaf">
                    {label.map(|label| view! { <span class="json-key">{label}</span> })}
                    <span class="json-punct">{": "}</span>
                    <div class="json-diff-scalar">
                        <JsonSideValueRow side=DiffSide::Ingress state_class=ingress_state_class value=ingress_value/>
                        <JsonSideValueRow side=DiffSide::Egress state_class=egress_state_class value=egress_value/>
                        {if count > 1 {
                            view! {
                                <div class="json-diff-summary-count">{format!("共 {} 处", count)}</div>
                            }.into_any()
                        } else {
                            ().into_any()
                        }}
                    </div>
                </div>
            }.into_any()
        }
    }
}

#[component]
fn JsonDiffChangeView(label: String, change: JsonDiffChange) -> impl IntoView {
    match change {
        JsonDiffChange::Added(value) => view! {
            <div class="json-diff-leaf">
                <span class="json-key">{label}</span>
                <span class="json-punct">{": "}</span>
                <div class="json-diff-scalar">
                    <JsonSideValueRow side=DiffSide::Ingress state_class="missing" value=None/>
                    <JsonSideValueRow side=DiffSide::Egress state_class="present" value=Some(value)/>
                </div>
            </div>
        }
        .into_any(),
        JsonDiffChange::Removed(value) => view! {
            <div class="json-diff-leaf">
                <span class="json-key">{label}</span>
                <span class="json-punct">{": "}</span>
                <div class="json-diff-scalar">
                    <JsonSideValueRow side=DiffSide::Ingress state_class="present" value=Some(value)/>
                    <JsonSideValueRow side=DiffSide::Egress state_class="missing" value=None/>
                </div>
            </div>
        }
        .into_any(),
        JsonDiffChange::Modified(node) => view! {
            <JsonDiffNodeView label=Some(label) node=node/>
        }
        .into_any(),
    }
}

#[component]
fn JsonSideValueRow(
    side: DiffSide,
    state_class: &'static str,
    value: Option<Value>,
) -> impl IntoView {
    let (label, dot_class, row_class) = match side {
        DiffSide::Ingress => (
            "入口",
            "side-dot ingress",
            format!("json-diff-value ingress {state_class}"),
        ),
        DiffSide::Egress => (
            "出口",
            "side-dot egress",
            format!("json-diff-value egress {state_class}"),
        ),
    };

    view! {
        <div class=row_class>
            <div class="json-diff-side-label">
                <span class=dot_class></span>
                <span>{label}</span>
            </div>
            <div class="json-diff-side-value">
                {match value {
                    Some(value) => view! { <JsonInlineValue value=value/> }.into_any(),
                    None => view! { <span class="json-missing-text">"没有"</span> }.into_any(),
                }}
            </div>
        </div>
    }
}

fn summary_value_state_class(
    ingress_value: Option<&Value>,
    egress_value: Option<&Value>,
    is_ingress: bool,
) -> &'static str {
    match (ingress_value.is_some(), egress_value.is_some(), is_ingress) {
        (true, true, _) => "compare",
        (true, false, true) => "present",
        (true, false, false) => "missing",
        (false, true, true) => "missing",
        (false, true, false) => "present",
        (false, false, _) => "compare",
    }
}

#[component]
fn JsonInlineValue(value: Value) -> impl IntoView {
    match value {
        Value::Object(_) | Value::Array(_) => {
            let pretty = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
            view! { <LongTextBlock text=pretty quoted=false class_name="json-inline-blob"/> }
                .into_any()
        }
        Value::String(text) => {
            view! { <LongTextBlock text=text quoted=true class_name="json-string"/> }.into_any()
        }
        Value::Number(number) => {
            view! { <span class="json-number">{number.to_string()}</span> }.into_any()
        }
        Value::Bool(boolean) => {
            view! { <span class="json-bool">{boolean.to_string()}</span> }.into_any()
        }
        Value::Null => view! { <span class="json-null">"null"</span> }.into_any(),
    }
}

#[component]
fn HiddenContextInputs(id: Option<String>, limit: usize) -> impl IntoView {
    view! {
        <input type="hidden" name="limit" value=limit.to_string()/>
        {id.map(|id| view! {
            <input type="hidden" name="id" value=id/>
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
    match parse_structured_content(&content) {
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
                <span class="long-text-preview">
                    <span class=class_name>
                        {if quoted { "\"".to_string() } else { String::new() }}
                        {preview}
                        "..."
                        {if quoted { "\"".to_string() } else { String::new() }}
                    </span>
                </span>
                <span class="long-text-full">
                    <span class=class_name>
                        {if quoted { "\"".to_string() } else { String::new() }}
                        {text}
                        {if quoted { "\"".to_string() } else { String::new() }}
                    </span>
                </span>
                <span class="long-text-hint"></span>
            </summary>
        </details>
    }
    .into_any()
}

fn prepare_diff_content(content: &str) -> String {
    match parse_structured_content(content) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| content.to_string()),
        Err(_) => content.to_string(),
    }
}

fn parse_json_content(content: &str) -> Option<Value> {
    parse_structured_content(content).ok()
}

fn parse_structured_content(content: &str) -> Result<Value, serde_json::Error> {
    match serde_json::from_str::<Value>(content) {
        Ok(value) => Ok(value),
        Err(json_error) => parse_sse_content(content).ok_or(json_error),
    }
}

fn parse_sse_content(content: &str) -> Option<Value> {
    let mut events = Vec::new();
    let mut event_name = None;
    let mut data_lines = Vec::new();
    let mut saw_sse_field = false;

    for line in content.lines() {
        if line.is_empty() {
            push_sse_event(&mut events, &mut event_name, &mut data_lines);
            continue;
        }

        if line.starts_with(':') {
            saw_sse_field = true;
            continue;
        }

        let Some((field, value)) = line.split_once(':') else {
            return None;
        };
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => {
                saw_sse_field = true;
                event_name = Some(value.to_string());
            }
            "data" => {
                saw_sse_field = true;
                data_lines.push(value.to_string());
            }
            "id" | "retry" => {
                saw_sse_field = true;
            }
            _ => return None,
        }
    }

    push_sse_event(&mut events, &mut event_name, &mut data_lines);

    if saw_sse_field && !events.is_empty() {
        Some(Value::Array(events))
    } else {
        None
    }
}

fn push_sse_event(
    events: &mut Vec<Value>,
    event_name: &mut Option<String>,
    data_lines: &mut Vec<String>,
) {
    if event_name.is_none() && data_lines.is_empty() {
        return;
    }

    let mut event = Map::new();
    if let Some(name) = event_name.take() {
        event.insert("event".to_string(), Value::String(name));
    }

    if !data_lines.is_empty() {
        let data = data_lines.join("\n");
        let data_value =
            serde_json::from_str::<Value>(&data).unwrap_or_else(|_| Value::String(data));
        event.insert("data".to_string(), data_value);
        data_lines.clear();
    }

    events.push(Value::Object(event));
}

fn summarize_text_diff(
    diff: &TextDiff<'_, '_, '_, str>,
    grouped_ops: &[Vec<similar::DiffOp>],
) -> Vec<String> {
    let mut removed = 0usize;
    let mut added = 0usize;
    let mut groups = 0usize;
    for group in grouped_ops {
        let mut group_has_change = false;
        for op in group {
            for change in diff.iter_changes(op) {
                match change.tag() {
                    ChangeTag::Delete => {
                        removed += 1;
                        group_has_change = true;
                    }
                    ChangeTag::Insert => {
                        added += 1;
                        group_has_change = true;
                    }
                    ChangeTag::Equal => {}
                }
            }
        }
        if group_has_change {
            groups += 1;
        }
    }
    let mut items = Vec::new();
    if groups > 0 {
        items.push(format!("{groups} 个变更块，已隐藏未变化的大段内容"));
    }
    if removed > 0 {
        items.push(format!("仅入口存在的文本行 x{removed}"));
    }
    if added > 0 {
        items.push(format!("仅出口存在的文本行 x{added}"));
    }
    items
}

fn build_json_diff_summary(node: &JsonDiffNode) -> Option<JsonDiffNode> {
    let mut summaries = BTreeMap::<String, SummaryLeaf>::new();
    collect_json_diff_summary(node, String::new(), &mut summaries);
    if summaries.is_empty() {
        return None;
    }

    Some(JsonDiffNode::Object(
        summaries
            .into_iter()
            .map(|(path, summary)| JsonDiffField {
                key: path,
                change: JsonDiffChange::Modified(JsonDiffNode::Summary {
                    ingress_value: summary.into_value(true),
                    egress_value: summary.into_value(false),
                    count: summary.count,
                }),
            })
            .collect(),
    ))
}

#[derive(Default)]
struct SummaryLeaf {
    ingress_values: Vec<Value>,
    egress_values: Vec<Value>,
    count: usize,
}

impl SummaryLeaf {
    fn push(&mut self, ingress_value: Option<Value>, egress_value: Option<Value>) {
        if let Some(value) = ingress_value {
            self.ingress_values.push(value);
        }
        if let Some(value) = egress_value {
            self.egress_values.push(value);
        }
        self.count += 1;
    }

    fn into_value(&self, is_ingress: bool) -> Option<Value> {
        let values = if is_ingress {
            &self.ingress_values
        } else {
            &self.egress_values
        };
        match values.len() {
            0 => None,
            1 => values.first().cloned(),
            _ => Some(Value::Array(values.clone())),
        }
    }
}

fn collect_json_diff_summary(
    node: &JsonDiffNode,
    path: String,
    summaries: &mut BTreeMap<String, SummaryLeaf>,
) {
    match node {
        JsonDiffNode::Object(fields) => {
            for field in fields {
                let next_path = if path.is_empty() {
                    field.key.clone()
                } else {
                    format!("{path}.{}", field.key)
                };
                collect_json_change_summary(&field.change, next_path, summaries);
            }
        }
        JsonDiffNode::Array(items) => {
            for item in items {
                let next_path = if path.is_empty() {
                    "[]".to_string()
                } else {
                    format!("{path}[]")
                };
                collect_json_change_summary(&item.change, next_path, summaries);
            }
        }
        JsonDiffNode::Scalar { before, after } => {
            summaries
                .entry(path)
                .or_default()
                .push(before.clone(), after.clone());
        }
        JsonDiffNode::Summary { .. } => {}
    }
}

fn collect_json_change_summary(
    change: &JsonDiffChange,
    path: String,
    summaries: &mut BTreeMap<String, SummaryLeaf>,
) {
    match change {
        JsonDiffChange::Added(_) => {
            if let JsonDiffChange::Added(value) = change {
                summaries
                    .entry(path)
                    .or_default()
                    .push(None, Some(value.clone()));
            }
        }
        JsonDiffChange::Removed(_) => {
            if let JsonDiffChange::Removed(value) = change {
                summaries
                    .entry(path)
                    .or_default()
                    .push(Some(value.clone()), None);
            }
        }
        JsonDiffChange::Modified(node) => collect_json_diff_summary(node, path, summaries),
    }
}

fn diff_json_values(left: &Value, right: &Value) -> Option<JsonDiffNode> {
    match (left, right) {
        (Value::Object(left_map), Value::Object(right_map)) => {
            let mut keys = BTreeSet::new();
            keys.extend(left_map.keys().cloned());
            keys.extend(right_map.keys().cloned());

            let mut fields = Vec::new();
            for key in keys {
                match (left_map.get(&key), right_map.get(&key)) {
                    (Some(left_value), Some(right_value)) => {
                        if let Some(node) = diff_json_values(left_value, right_value) {
                            fields.push(JsonDiffField {
                                key,
                                change: JsonDiffChange::Modified(node),
                            });
                        }
                    }
                    (Some(left_value), None) => fields.push(JsonDiffField {
                        key,
                        change: JsonDiffChange::Removed(left_value.clone()),
                    }),
                    (None, Some(right_value)) => fields.push(JsonDiffField {
                        key,
                        change: JsonDiffChange::Added(right_value.clone()),
                    }),
                    (None, None) => {}
                }
            }

            if fields.is_empty() {
                None
            } else {
                Some(JsonDiffNode::Object(fields))
            }
        }
        (Value::Array(left_items), Value::Array(right_items)) => {
            let max_len = left_items.len().max(right_items.len());
            let mut items = Vec::new();

            for index in 0..max_len {
                match (left_items.get(index), right_items.get(index)) {
                    (Some(left_value), Some(right_value)) => {
                        if let Some(node) = diff_json_values(left_value, right_value) {
                            items.push(JsonDiffIndex {
                                label: index.to_string(),
                                change: JsonDiffChange::Modified(node),
                            });
                        }
                    }
                    (Some(left_value), None) => items.push(JsonDiffIndex {
                        label: index.to_string(),
                        change: JsonDiffChange::Removed(left_value.clone()),
                    }),
                    (None, Some(right_value)) => items.push(JsonDiffIndex {
                        label: index.to_string(),
                        change: JsonDiffChange::Added(right_value.clone()),
                    }),
                    (None, None) => {}
                }
            }

            if items.is_empty() {
                None
            } else {
                Some(JsonDiffNode::Array(items))
            }
        }
        _ if left == right => None,
        _ => Some(JsonDiffNode::Scalar {
            before: Some(left.clone()),
            after: Some(right.clone()),
        }),
    }
}

const STYLE: &str = r#"
:root {
  --bg-0: #ffffff;
  --bg-1: #fbfbfc;
  --bg-2: #f3f4f6;
  --panel: rgba(255, 255, 255, 0.94);
  --panel-strong: rgba(255, 255, 255, 0.98);
  --panel-soft: rgba(249, 250, 251, 0.94);
  --line: rgba(15, 23, 42, 0.08);
  --line-strong: rgba(15, 23, 42, 0.14);
  --text: #111827;
  --muted: #6b7280;
  --muted-2: #9ca3af;
  --accent: #111827;
  --accent-soft: rgba(15, 23, 42, 0.06);
  --accent-strong: rgba(15, 23, 42, 0.10);
  --danger: #c2410c;
  --warn: #b45309;
  --cyan: #1f2937;
  --shadow: 0 1px 2px rgba(15, 23, 42, 0.04), 0 12px 36px rgba(15, 23, 42, 0.06);
}

* {
  box-sizing: border-box;
}

body {
  margin: 0;
  font-family: "Inter", "SF Pro Display", "Segoe UI", "Helvetica Neue", Arial, sans-serif;
  background:
    radial-gradient(circle at top left, rgba(15, 23, 42, 0.03), transparent 24%),
    radial-gradient(circle at top right, rgba(15, 23, 42, 0.02), transparent 20%),
    linear-gradient(180deg, #ffffff 0%, #f8fafc 100%);
  color: var(--text);
  min-height: 100vh;
}

.debug-shell {
  max-width: 1440px;
  margin: 0 auto;
  padding: 32px 24px 40px;
}

.hero, .panel, .banner {
  border: 1px solid var(--line);
  box-shadow: var(--shadow);
}

.hero {
  display: flex;
  justify-content: space-between;
  gap: 24px;
  padding: 28px;
  border-radius: 24px;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.98), rgba(249, 250, 251, 0.98)),
    linear-gradient(90deg, rgba(15, 23, 42, 0.02), transparent 36%);
}

.eyebrow {
  margin: 0 0 10px;
  text-transform: uppercase;
  letter-spacing: 0.18em;
  font-size: 12px;
  color: var(--muted);
  text-shadow: none;
}

h1, h2, p {
  margin: 0;
}

h1 {
  font-size: clamp(32px, 4vw, 52px);
  line-height: 1.02;
  letter-spacing: -0.03em;
  text-transform: none;
  text-shadow: none;
}

.hero-copy, .panel-header p, .empty-state p, .subtle {
  color: var(--muted);
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
  border: 1px solid var(--line);
  text-decoration: none;
  cursor: pointer;
  font: inherit;
  font-weight: 700;
  text-transform: none;
  letter-spacing: 0;
  box-shadow: none;
}

.button.primary { background: #111827; color: #ffffff; border-color: #111827; }
.button.secondary { background: #ffffff; color: #111827; }
.button.danger { background: #fff7ed; color: #c2410c; border-color: rgba(194, 65, 12, 0.14); }

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
  border: 1px solid var(--line);
  box-shadow: none;
}

.pill.live { background: #f3f4f6; color: #111827; }
.pill.paused { background: #fff7ed; color: #c2410c; border-color: rgba(194, 65, 12, 0.14); }
.pill.neutral, .tag { background: #f9fafb; color: var(--muted); }
.badge.ok { background: #f3f4f6; color: #111827; }
.badge.error { background: #fff7ed; color: #c2410c; border-color: rgba(194, 65, 12, 0.14); }

.banner {
  margin-bottom: 14px;
  padding: 14px 18px;
  border-radius: 18px;
  background: var(--panel-soft);
}

.banner.notice { border-color: var(--line-strong); }
.banner.error { border-color: rgba(194, 65, 12, 0.18); color: #c2410c; }

.grid {
  display: grid;
  gap: 18px;
  grid-template-columns: minmax(320px, 420px) minmax(0, 1fr);
}

.panel {
  min-height: 68vh;
  padding: 22px;
  border-radius: 24px;
  background: var(--panel);
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
  border: 1px solid var(--line);
  border-radius: 18px;
  background: rgba(255, 255, 255, 0.98);
  color: inherit;
  text-decoration: none;
  transition: transform 140ms ease, border-color 140ms ease, box-shadow 140ms ease;
}

.log-row.selected {
  border-color: rgba(29, 78, 216, 0.34);
  background:
    linear-gradient(180deg, rgba(239, 246, 255, 0.92), rgba(255, 255, 255, 0.98)),
    #ffffff;
  box-shadow:
    inset 4px 0 0 #2563eb,
    0 0 0 1px rgba(37, 99, 235, 0.06),
    0 12px 30px rgba(37, 99, 235, 0.12);
  transform: translateY(-1px);
}

.log-row:hover {
  transform: translateY(-1px);
  border-color: rgba(15, 23, 42, 0.14);
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
  color: #374151;
}

.meta-grid, .kv-grid {
  display: grid;
  gap: 8px 14px;
}

.meta-grid {
  grid-template-columns: repeat(2, minmax(0, 1fr));
  color: var(--muted);
  font-size: 12px;
}

.error-copy {
  margin: 0;
  color: #c2410c;
  font-size: 13px;
}

.detail-header {
  display: flex;
  gap: 12px;
  align-items: center;
  margin-bottom: 16px;
}

.detail-tabs {
  display: grid;
  gap: 14px;
}

.detail-tab-input {
  display: none;
}

.detail-tab-bar {
  display: inline-flex;
  gap: 8px;
  padding: 6px;
  border-radius: 999px;
  background: #f3f4f6;
  border: 1px solid var(--line);
  width: fit-content;
}

.detail-tab-label {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 88px;
  padding: 10px 16px;
  border-radius: 999px;
  cursor: pointer;
  font-size: 13px;
  font-weight: 700;
  color: var(--muted);
  text-transform: none;
  letter-spacing: 0;
}

.detail-tab-panel {
  display: none;
}

#tab-detail:checked ~ .detail-tab-bar label[for="tab-detail"],
#tab-diff:checked ~ .detail-tab-bar label[for="tab-diff"] {
  background: #ffffff;
  color: #111827;
  box-shadow:
    0 0 0 1px rgba(37, 99, 235, 0.16),
    0 8px 18px rgba(37, 99, 235, 0.10);
}

#tab-detail:checked ~ .detail-panel-main,
#tab-diff:checked ~ .diff-panel-main {
  display: block;
}

.compare-section {
  display: grid;
  gap: 12px;
  margin-bottom: 18px;
  padding: 16px;
  border-radius: 18px;
  background: rgba(249, 250, 251, 0.9);
  border: 1px solid var(--line);
}

.compare-header {
  display: flex;
  gap: 12px;
  align-items: center;
  justify-content: space-between;
}

.compare-header h3 {
  margin: 0;
  font-size: 18px;
}

.diff-block {
  display: grid;
  gap: 10px;
}

.diff-summary {
  display: grid;
  gap: 8px;
  padding: 12px 14px;
  border-radius: 16px;
  background: #ffffff;
  border: 1px solid var(--line);
}

.diff-summary-title {
  font-size: 12px;
  font-weight: 800;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--muted);
}

.diff-summary-list {
  display: grid;
  gap: 6px;
}

.diff-summary-item {
  display: block;
  padding: 10px 12px;
  border-radius: 14px;
  background: #f9fafb;
  border: 1px solid rgba(15, 23, 42, 0.06);
  font-size: 13px;
  color: var(--text);
  overflow-wrap: anywhere;
}

.diff-summary-count {
  display: inline-flex;
  align-items: center;
  border-radius: 999px;
  padding: 4px 8px;
  background: #f3f4f6;
  color: #374151;
  font-size: 11px;
  font-weight: 800;
}

.diff-head {
  display: flex;
  gap: 12px;
  align-items: center;
  justify-content: space-between;
}

.diff-legend {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.diff-side {
  display: inline-flex;
  align-items: center;
  border-radius: 999px;
  padding: 6px 10px;
  font-size: 11px;
  font-weight: 700;
}

.diff-side.before {
  background: rgba(31, 117, 220, 0.12);
  color: #1d5ea8;
  border: 1px solid rgba(31, 117, 220, 0.16);
}

.diff-side.after {
  background: rgba(224, 122, 47, 0.16);
  color: #9a4f17;
  border: 1px solid rgba(224, 122, 47, 0.18);
}

.diff-side.present {
  background: rgba(33, 150, 83, 0.14);
  color: #20643a;
  border: 1px solid rgba(33, 150, 83, 0.16);
}

.diff-side.missing {
  background: rgba(180, 63, 50, 0.14);
  color: #8f352b;
  border: 1px solid rgba(180, 63, 50, 0.18);
}

.side-dot {
  width: 8px;
  height: 8px;
  border-radius: 999px;
  display: inline-block;
  margin-right: 8px;
}

.side-dot.ingress {
  background: #2563eb;
}

.side-dot.egress {
  background: #e07a2f;
}

.diff-shell {
  border-radius: 16px;
  overflow: hidden;
  border: 1px solid var(--line);
  background: #1e2430;
  font-family: "SF Mono", "Menlo", monospace;
  font-size: 12px;
  line-height: 1.55;
  box-shadow: 0 8px 24px rgba(15, 23, 42, 0.10);
}

.json-diff-shell {
  border-radius: 16px;
  overflow: hidden;
  border: 1px solid var(--line);
  background: #1e2430;
  color: #eef2f5;
  padding: 12px 14px;
  font-family: "SF Mono", "Menlo", monospace;
  font-size: 12px;
  line-height: 1.6;
  box-shadow: 0 8px 24px rgba(15, 23, 42, 0.10);
}

.diff-summary-shell {
  padding: 12px 14px;
}

.diff-detail-toggle {
  display: grid;
  gap: 10px;
}

.diff-detail-toggle summary {
  list-style: none;
  cursor: pointer;
  font-size: 12px;
  font-weight: 800;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--muted);
}

.diff-detail-toggle summary::-webkit-details-marker {
  display: none;
}

.diff-detail-toggle summary::before {
  content: "▸";
  display: inline-block;
  width: 12px;
  margin-right: 6px;
  color: #8cb9ff;
}

.diff-detail-toggle[open] > summary::before {
  content: "▾";
}

.diff-line {
  display: grid;
  grid-template-columns: 22px minmax(0, 1fr);
  align-items: start;
}

.diff-group + .diff-group {
  border-top: 1px solid rgba(255, 255, 255, 0.08);
}

.diff-line.equal {
  background: rgba(255, 255, 255, 0.02);
  color: #d7dde5;
}

.diff-line.delete {
  background: rgba(180, 63, 50, 0.16);
  color: #ffd7d1;
}

.diff-line.insert {
  background: rgba(33, 150, 83, 0.16);
  color: #d7ffe1;
}

.diff-prefix,
.diff-text {
  padding: 4px 10px;
  white-space: pre-wrap;
  word-break: break-word;
}

.diff-prefix {
  text-align: center;
  color: rgba(255, 255, 255, 0.72);
  border-right: 1px solid rgba(255, 255, 255, 0.08);
}

.diff-hidden {
  padding: 8px 12px;
  color: #8f99aa;
  font-size: 11px;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.event-card {
  display: grid;
  gap: 14px;
  padding: 18px;
  border-radius: 18px;
  background: #ffffff;
  border: 1px solid var(--line);
}

.event-top {
  justify-content: space-between;
  align-items: flex-start;
}

.event-top > div:first-child {
  display: grid;
  gap: 4px;
}

.detail-chip-row {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  margin-top: 4px;
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
  color: var(--muted-2);
}

.kv-row dd {
  margin: 0;
  font-size: 13px;
  overflow-wrap: anywhere;
  color: var(--text);
}

.block {
  display: grid;
  gap: 8px;
}

.block-title {
  font-size: 12px;
  font-weight: 700;
  color: var(--muted);
}

.error-block .block-title {
  color: #c2410c;
}

pre {
  margin: 0;
  padding: 14px;
  border-radius: 16px;
  background: #1e2430;
  color: #eef2f5;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: "SF Mono", "Menlo", monospace;
  font-size: 12px;
  line-height: 1.55;
  border: 1px solid rgba(15, 23, 42, 0.12);
}

.plain-text-block {
  display: block;
  padding: 14px;
  border-radius: 16px;
  background: #1e2430;
  color: #eef2f5;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: "SF Mono", "Menlo", monospace;
  font-size: 12px;
  line-height: 1.55;
  border: 1px solid rgba(15, 23, 42, 0.12);
}

.json-tree-shell {
  padding: 12px 14px;
  border-radius: 16px;
  background: #1e2430;
  color: #eef2f5;
  overflow-x: auto;
  font-family: "SF Mono", "Menlo", monospace;
  font-size: 12px;
  line-height: 1.6;
  border: 1px solid rgba(15, 23, 42, 0.12);
  box-shadow: 0 8px 24px rgba(15, 23, 42, 0.10);
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

.json-diff-node {
  margin-left: 0;
}

.json-diff-node + .json-diff-node,
.json-diff-leaf + .json-diff-node,
.json-diff-node + .json-diff-leaf,
.json-diff-leaf + .json-diff-leaf {
  margin-top: 6px;
}

.json-diff-node summary {
  list-style: none;
  cursor: pointer;
}

.json-diff-node summary::-webkit-details-marker {
  display: none;
}

.json-diff-node summary::before {
  content: "▾";
  display: inline-block;
  width: 12px;
  margin-right: 6px;
  color: #8cb9ff;
}

.json-diff-node:not([open]) > summary::before {
  content: "▸";
}

.json-diff-children {
  margin-left: 18px;
  margin-top: 6px;
  padding-left: 10px;
  border-left: 1px solid rgba(140, 185, 255, 0.22);
}

.json-diff-leaf {
  padding-left: 18px;
  overflow-wrap: anywhere;
}

.json-diff-scalar {
  display: grid;
  gap: 4px;
  margin-top: 4px;
}

.json-diff-value {
  display: grid;
  grid-template-columns: 72px minmax(0, 1fr);
  align-items: start;
  border-radius: 12px;
  overflow: hidden;
}

.json-diff-value.present {
  background: rgba(33, 150, 83, 0.16);
  color: #d7ffe1;
}

.json-diff-value.missing {
  background: rgba(180, 63, 50, 0.16);
  color: #ffd7d1;
}

.json-diff-value.ingress.compare {
  background: rgba(31, 117, 220, 0.16);
  color: #d9ebff;
}

.json-diff-value.egress.compare {
  background: rgba(224, 122, 47, 0.18);
  color: #ffe6d5;
}

.json-diff-side-label,
.json-diff-side-value {
  padding: 6px 10px;
}

.json-diff-side-label {
  display: inline-flex;
  align-items: center;
  gap: 0;
  font-size: 11px;
  font-weight: 700;
  border-right: 1px solid rgba(255, 255, 255, 0.08);
}

.json-diff-side-value {
  white-space: pre-wrap;
  word-break: break-word;
}

.json-missing-text {
  font-weight: 700;
}

.json-diff-summary-count {
  padding-left: 4px;
  font-size: 11px;
  color: #8f99aa;
}

.json-diff-value.added {
  background: rgba(33, 150, 83, 0.16);
  color: #d7ffe1;
}

.json-diff-value.removed {
  background: rgba(180, 63, 50, 0.16);
  color: #ffd7d1;
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

.long-text-preview,
.long-text-full {
  display: none;
}

.long-text-toggle:not([open]) > summary .long-text-preview {
  display: inline;
}

.long-text-toggle[open] > summary .long-text-full {
  display: inline;
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

.json-key { color: #ffcf8b; }
.json-punct { color: #7d8696; }
.json-summary { color: #8cb9ff; }
.json-string { color: #9be28f; }
.json-inline-blob { color: #d7dde5; }
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

#[cfg(test)]
mod tests {
    use super::parse_structured_content;
    use serde_json::json;

    #[test]
    fn parses_plain_json_body() {
        let parsed = parse_structured_content(r#"{"model":"gpt-5.4","stream":false}"#)
            .expect("plain json should parse");

        assert_eq!(
            parsed,
            json!({
                "model": "gpt-5.4",
                "stream": false
            })
        );
    }

    #[test]
    fn parses_sse_json_data_as_structured_body() {
        let parsed = parse_structured_content(
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\ndata: [DONE]\n\n",
        )
        .expect("sse json data should parse");

        assert_eq!(
            parsed,
            json!([
                {
                    "event": "response.created",
                    "data": {
                        "type": "response.created",
                        "response": {
                            "id": "resp_1"
                        }
                    }
                },
                {
                    "data": "[DONE]"
                }
            ])
        );
    }
}

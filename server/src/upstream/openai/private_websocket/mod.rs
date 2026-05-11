use crate::upstream::shared::proxy_url_for;
use native_tls::{TlsConnector as NativeTlsConnector, TlsStream as NativeTlsStream};
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpStream as StdTcpStream,
    time::Duration as StdDuration,
};
use tokio::sync::mpsc;
use tungstenite::{
    Message,
    client::{ClientRequestBuilder, client},
    handshake::HandshakeError,
    http::Uri,
};
use url::Url;

const OPENAI_RESPONSES_WS_URL: &str = "wss://chatgpt.com/backend-api/codex/responses";
const OPENAI_RESPONSES_WS_BETA_HEADER_VALUE: &str = "responses_websockets=2026-02-06";
const MAX_WS_HEADER_BYTES: usize = 16 * 1024;

pub fn stream_responses_websocket_blocking(
    access_token: String,
    account_id: Option<String>,
    request_id: String,
    request_text: String,
    tx: mpsc::UnboundedSender<Result<String, String>>,
) -> Result<(), String> {
    let url = Url::parse(OPENAI_RESPONSES_WS_URL)
        .map_err(|err| format!("invalid OpenAI websocket url: {err}"))?;
    let uri: Uri = OPENAI_RESPONSES_WS_URL
        .parse()
        .map_err(|err| format!("invalid OpenAI websocket uri: {err}"))?;

    let mut builder = ClientRequestBuilder::new(uri)
        .with_header("Authorization", format!("Bearer {access_token}"))
        .with_header("Origin", "https://chatgpt.com")
        .with_header("User-Agent", "CodexBar")
        .with_header("OpenAI-Beta", OPENAI_RESPONSES_WS_BETA_HEADER_VALUE)
        .with_header("x-client-request-id", request_id.clone())
        .with_header("session_id", request_id.clone())
        .with_header("thread_id", request_id);
    if let Some(account_id) = account_id.filter(|value| !value.is_empty()) {
        builder = builder.with_header("ChatGPT-Account-Id", account_id);
    }

    let transport = connect_blocking_websocket_transport(&url)?;
    let stream = wrap_blocking_tls_stream(transport, &url)?;
    let (mut ws, _) = client(builder, stream).map_err(|err| match err {
        HandshakeError::Failure(e) => format!("openai websocket handshake failed: {e}"),
        HandshakeError::Interrupted(_) => "websocket handshake interrupted".to_string(),
    })?;

    ws.send(Message::Text(request_text))
        .map_err(|err| format!("send websocket message failed: {err}"))?;

    loop {
        match ws.read() {
            Ok(Message::Text(text)) => {
                let is_terminal =
                    is_terminal_ws_event_text(&text) || is_wrapped_error_event_text(&text);
                if tx.send(Ok(text)).is_err() {
                    return Err("response event consumer dropped".to_string());
                }
                if is_terminal {
                    break;
                }
            }
            Ok(Message::Ping(payload)) => {
                ws.send(Message::Pong(payload))
                    .map_err(|err| format!("websocket pong failed: {err}"))?;
            }
            Ok(Message::Pong(_)) => {}
            Ok(Message::Binary(_)) => {
                return Err("unexpected binary websocket event".to_string());
            }
            Ok(Message::Close(_)) => {
                return Err("websocket closed by server before response.completed".to_string());
            }
            Ok(Message::Frame(_)) => {}
            Err(err) => return Err(format!("read websocket message failed: {err}")),
        }
    }

    Ok(())
}

fn is_wrapped_error_event_text(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .is_some_and(|event_type| event_type == "error")
}

fn is_terminal_ws_event_text(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .is_some_and(|event_type| {
            matches!(
                event_type.as_str(),
                "response.completed" | "response.failed" | "response.incomplete"
            )
        })
}

fn connect_blocking_websocket_transport(url: &Url) -> Result<StdTcpStream, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "websocket url is missing host".to_string())?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "websocket url is missing port".to_string())?;

    let stream = if let Some(proxy_url) = proxy_url_for(url) {
        connect_blocking_via_http_proxy(&proxy_url, host, port)?
    } else {
        StdTcpStream::connect((host, port))
            .map_err(|err| format!("connect OpenAI websocket upstream failed: {err}"))?
    };
    stream
        .set_read_timeout(Some(StdDuration::from_secs(30)))
        .map_err(|err| format!("set websocket read timeout failed: {err}"))?;
    stream
        .set_write_timeout(Some(StdDuration::from_secs(30)))
        .map_err(|err| format!("set websocket write timeout failed: {err}"))?;
    Ok(stream)
}

fn connect_blocking_via_http_proxy(
    proxy_url: &str,
    host: &str,
    port: u16,
) -> Result<StdTcpStream, String> {
    let proxy = Url::parse(proxy_url).map_err(|err| format!("invalid proxy url: {err}"))?;
    let proxy_host = proxy
        .host_str()
        .ok_or_else(|| "proxy url is missing host".to_string())?;
    let proxy_port = proxy
        .port_or_known_default()
        .ok_or_else(|| "proxy url is missing port".to_string())?;
    let mut stream = StdTcpStream::connect((proxy_host, proxy_port))
        .map_err(|err| format!("connect proxy failed: {err}"))?;
    let request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("write proxy CONNECT failed: {err}"))?;
    stream
        .flush()
        .map_err(|err| format!("flush proxy CONNECT failed: {err}"))?;

    let mut response = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|err| format!("read proxy CONNECT response failed: {err}"))?;
        if read == 0 {
            return Err("proxy closed connection during CONNECT".to_string());
        }
        response.extend_from_slice(&chunk[..read]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if response.len() > MAX_WS_HEADER_BYTES {
            return Err("proxy CONNECT response headers exceeded 16 KiB".to_string());
        }
    }

    let head = String::from_utf8_lossy(&response);
    let status_line = head.lines().next().unwrap_or_default();
    if !status_line.contains(" 200 ") && !status_line.ends_with(" 200") {
        return Err(format!("proxy CONNECT failed: {status_line}"));
    }

    Ok(stream)
}

fn wrap_blocking_tls_stream(
    stream: StdTcpStream,
    url: &Url,
) -> Result<NativeTlsStream<StdTcpStream>, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "websocket url is missing host".to_string())?;
    let connector = NativeTlsConnector::new()
        .map_err(|err| format!("create websocket TLS connector failed: {err}"))?;
    connector
        .connect(host, stream)
        .map_err(|err| format!("connect websocket TLS failed: {err}"))
}

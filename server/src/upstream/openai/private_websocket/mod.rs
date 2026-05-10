use crate::upstream::shared::proxy_url_for;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use native_tls::{TlsConnector as NativeTlsConnector, TlsStream as NativeTlsStream};
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpStream as StdTcpStream,
    time::Duration as StdDuration,
};
use tokio::sync::mpsc;
use url::Url;
use uuid::Uuid;

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
    let transport = connect_blocking_websocket_transport(&url)?;
    let mut stream = wrap_blocking_tls_stream(transport, &url)?;
    let request =
        build_websocket_handshake_request(&url, &access_token, account_id.as_deref(), &request_id);
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("write websocket handshake failed: {err}"))?;
    stream
        .flush()
        .map_err(|err| format!("flush websocket handshake failed: {err}"))?;
    let response_head = read_blocking_http_head(&mut stream, MAX_WS_HEADER_BYTES)?;
    let status_line = response_head.lines().next().unwrap_or_default();
    if !status_line.contains(" 101 ") && !status_line.ends_with(" 101") {
        return Err(format!("openai websocket connect failed: {status_line}"));
    }

    write_blocking_websocket_frame(&mut stream, 0x1, request_text.as_bytes())?;

    loop {
        let (opcode, payload) = read_blocking_websocket_frame(&mut stream)?;
        match opcode {
            0x1 => {
                let text = String::from_utf8(payload)
                    .map_err(|err| format!("invalid UTF-8 websocket event: {err}"))?;
                let is_terminal =
                    is_terminal_ws_event_text(&text) || is_wrapped_error_event_text(&text);
                if tx.send(Ok(text)).is_err() {
                    return Err("response event consumer dropped".to_string());
                }
                if is_terminal {
                    break;
                }
            }
            0x9 => write_blocking_websocket_frame(&mut stream, 0xA, &payload)?,
            0xA => {}
            0x2 => return Err("unexpected binary websocket event".to_string()),
            0x8 => {
                return Err("websocket closed by server before response.completed".to_string());
            }
            0x0 => return Err("fragmented websocket frames are not supported".to_string()),
            other => return Err(format!("unexpected websocket opcode: {other}")),
        }
    }

    Ok(())
}

fn build_websocket_handshake_request(
    url: &Url,
    access_token: &str,
    account_id: Option<&str>,
    request_id: &str,
) -> String {
    let path = websocket_request_target(url);
    let host = url.host_str().unwrap_or("chatgpt.com");
    let handshake_key = websocket_handshake_key();
    let mut request = format!(
        "GET {path} HTTP/1.1\r\n\
Host: {host}\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Key: {handshake_key}\r\n\
Sec-WebSocket-Version: 13\r\n\
Authorization: Bearer {access_token}\r\n"
    );
    if let Some(account_id) = account_id.filter(|value| !value.is_empty()) {
        request.push_str(&format!("ChatGPT-Account-Id: {account_id}\r\n"));
    }
    request.push_str(&format!(
        "Origin: https://chatgpt.com\r\n\
User-Agent: CodexBar\r\n\
OpenAI-Beta: {OPENAI_RESPONSES_WS_BETA_HEADER_VALUE}\r\n\
x-client-request-id: {request_id}\r\n\
session_id: {request_id}\r\n\
thread_id: {request_id}\r\n\r\n"
    ));
    request
}

fn websocket_request_target(url: &Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    }
}

fn websocket_handshake_key() -> String {
    BASE64_STANDARD.encode(Uuid::new_v4().into_bytes())
}

fn websocket_mask_key() -> [u8; 4] {
    let bytes = Uuid::new_v4().into_bytes();
    [bytes[0], bytes[1], bytes[2], bytes[3]]
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

fn read_blocking_http_head(
    stream: &mut NativeTlsStream<StdTcpStream>,
    limit: usize,
) -> Result<String, String> {
    let mut response = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|err| format!("read websocket handshake response failed: {err}"))?;
        if read == 0 {
            return Err("websocket closed during handshake".to_string());
        }
        response.extend_from_slice(&chunk[..read]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            return String::from_utf8(response)
                .map_err(|err| format!("websocket handshake response was not UTF-8: {err}"));
        }
        if response.len() > limit {
            return Err(format!(
                "websocket handshake response headers exceeded {limit} bytes"
            ));
        }
    }
}

fn write_blocking_websocket_frame(
    stream: &mut NativeTlsStream<StdTcpStream>,
    opcode: u8,
    payload: &[u8],
) -> Result<(), String> {
    let mask = websocket_mask_key();
    let mut frame = Vec::with_capacity(payload.len() + 16);
    frame.push(0x80 | opcode);
    match payload.len() {
        0..=125 => frame.push(0x80 | payload.len() as u8),
        126..=65535 => {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        _ => {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(&mask);
    for (index, byte) in payload.iter().enumerate() {
        frame.push(byte ^ mask[index % mask.len()]);
    }
    stream
        .write_all(&frame)
        .map_err(|err| format!("write websocket frame failed: {err}"))?;
    stream
        .flush()
        .map_err(|err| format!("flush websocket frame failed: {err}"))
}

fn read_blocking_websocket_frame(
    stream: &mut NativeTlsStream<StdTcpStream>,
) -> Result<(u8, Vec<u8>), String> {
    let mut head = [0_u8; 2];
    stream
        .read_exact(&mut head)
        .map_err(|err| format!("read websocket frame head failed: {err}"))?;
    let fin = head[0] & 0x80 != 0;
    let opcode = head[0] & 0x0f;
    let masked = head[1] & 0x80 != 0;
    let mut payload_len = (head[1] & 0x7f) as u64;
    if payload_len == 126 {
        let mut extended = [0_u8; 2];
        stream
            .read_exact(&mut extended)
            .map_err(|err| format!("read websocket extended payload length failed: {err}"))?;
        payload_len = u16::from_be_bytes(extended) as u64;
    } else if payload_len == 127 {
        let mut extended = [0_u8; 8];
        stream
            .read_exact(&mut extended)
            .map_err(|err| format!("read websocket extended payload length failed: {err}"))?;
        payload_len = u64::from_be_bytes(extended);
    }
    if !fin {
        return Err("fragmented websocket frames are not supported".to_string());
    }
    let mask = if masked {
        let mut mask = [0_u8; 4];
        stream
            .read_exact(&mut mask)
            .map_err(|err| format!("read websocket mask failed: {err}"))?;
        Some(mask)
    } else {
        None
    };
    let payload_len: usize = payload_len
        .try_into()
        .map_err(|_| "websocket frame payload length overflow".to_string())?;
    let mut payload = vec![0_u8; payload_len];
    stream
        .read_exact(&mut payload)
        .map_err(|err| format!("read websocket payload failed: {err}"))?;
    if let Some(mask) = mask {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % mask.len()];
        }
    }
    Ok((opcode, payload))
}

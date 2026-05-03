use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Pool, Sqlite};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex};

use crate::vcp_modules::chat_manager::ChatMessage;
use crate::vcp_modules::db_manager::DbState;
use crate::vcp_modules::message_service;

const INJECTED_EVENT: &str = "diagnostic-chat-message-injected";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticInjectAssistantPayload {
    pub owner_id: String,
    pub owner_type: String,
    pub topic_id: String,
    pub content: String,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub timestamp: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticInjectedMessage {
    pub owner_id: String,
    pub owner_type: String,
    pub topic_id: String,
    pub message: ChatMessage,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticTestServerInfo {
    pub running: bool,
    pub bind_lan: bool,
    pub port: u16,
    pub token: String,
    pub url: String,
    pub local_url: String,
}

struct DiagnosticTestServerHandle {
    info: DiagnosticTestServerInfo,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

#[derive(Clone)]
pub struct DiagnosticTestServerState {
    server: Arc<Mutex<Option<DiagnosticTestServerHandle>>>,
}

impl DiagnosticTestServerState {
    pub fn new() -> Self {
        Self {
            server: Arc::new(Mutex::new(None)),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn random_token() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

fn random_suffix() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(6)
        .map(char::from)
        .collect()
}

fn detect_lan_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

fn sample_tool_result_content() -> &'static str {
    r#"Diagnostic AI reply for VCPMobile rendering.

<<<[TOOL_REQUEST]>>>
tool_name: 「始」ImageGen「末」
prompt: 「始」mobile diagnostic tool preview「末」
<<<[END_TOOL_REQUEST]>>>

[[VCP调用结果信息汇总:
- 工具名称: ImageGen
- 执行状态: success
- 返回内容: ![preview](https://picsum.photos/seed/vcp-mobile-tool/480/320)
- Result: **Markdown field**
  - item A
  - item B
VCP调用结果结束]]

Text after the tool block. This should remain attached to the same assistant message."#
}

fn validate_payload(
    payload: &DiagnosticInjectAssistantPayload,
) -> Result<(String, String, String, String), String> {
    let owner_id = payload.owner_id.trim().to_string();
    let owner_type = payload.owner_type.trim().to_ascii_lowercase();
    let topic_id = payload.topic_id.trim().to_string();
    let content = payload.content.trim_end().to_string();

    if owner_id.is_empty() {
        return Err("ownerId is required.".to_string());
    }
    if owner_type != "agent" && owner_type != "group" {
        return Err("ownerType must be agent or group.".to_string());
    }
    if topic_id.is_empty() {
        return Err("topicId is required.".to_string());
    }
    if content.trim().is_empty() {
        return Err("content is required.".to_string());
    }

    Ok((owner_id, owner_type, topic_id, content))
}

async fn inject_assistant_message_shared(
    app_handle: AppHandle,
    db_pool: Pool<Sqlite>,
    payload: DiagnosticInjectAssistantPayload,
) -> Result<DiagnosticInjectedMessage, String> {
    let (owner_id, owner_type, topic_id, content) = validate_payload(&payload)?;
    let timestamp = payload.timestamp.unwrap_or_else(now_ms);
    let message_id = payload
        .message_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("diag_ai_{}_{}", timestamp, random_suffix()));
    let agent_id = payload
        .agent_id
        .filter(|value| !value.trim().is_empty())
        .or_else(|| (owner_type == "agent").then(|| owner_id.clone()));

    let message = ChatMessage {
        id: message_id,
        role: "assistant".to_string(),
        name: payload
            .name
            .filter(|value| !value.trim().is_empty())
            .or_else(|| Some("Diagnostic AI".to_string())),
        content,
        timestamp,
        is_thinking: Some(false),
        agent_id,
        group_id: (owner_type == "group").then(|| owner_id.clone()),
        topic_id: Some(topic_id.clone()),
        is_group_message: Some(owner_type == "group"),
        finish_reason: Some("diagnostic".to_string()),
        attachments: None,
        blocks: None,
    };

    message_service::append_single_message(
        app_handle.clone(),
        &db_pool,
        &owner_id,
        &owner_type,
        topic_id.clone(),
        message.clone(),
    )
    .await?;

    let injected = DiagnosticInjectedMessage {
        owner_id,
        owner_type,
        topic_id,
        message,
    };
    let _ = app_handle.emit(INJECTED_EVENT, &injected);
    Ok(injected)
}

#[tauri::command]
pub async fn inject_diagnostic_assistant_message(
    app_handle: AppHandle,
    db_state: State<'_, DbState>,
    payload: DiagnosticInjectAssistantPayload,
) -> Result<DiagnosticInjectedMessage, String> {
    inject_assistant_message_shared(app_handle, db_state.pool.clone(), payload).await
}

#[tauri::command]
pub async fn start_diagnostic_test_server(
    app_handle: AppHandle,
    db_state: State<'_, DbState>,
    state: State<'_, DiagnosticTestServerState>,
    bind_lan: Option<bool>,
) -> Result<DiagnosticTestServerInfo, String> {
    start_test_server(
        app_handle,
        db_state.pool.clone(),
        &state,
        bind_lan.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn stop_diagnostic_test_server(
    state: State<'_, DiagnosticTestServerState>,
) -> Result<(), String> {
    stop_test_server(&state).await
}

pub async fn start_test_server(
    app_handle: AppHandle,
    db_pool: Pool<Sqlite>,
    state: &DiagnosticTestServerState,
    bind_lan: bool,
) -> Result<DiagnosticTestServerInfo, String> {
    let mut server = state.server.lock().await;
    if let Some(existing) = server.as_ref() {
        return Ok(existing.info.clone());
    }

    let bind_addr = if bind_lan { "0.0.0.0:0" } else { "127.0.0.1:0" };
    let listener = TcpListener::bind(bind_addr)
        .await
        .map_err(|error| format!("Failed to bind diagnostic test server: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let token = random_token();
    let host = if bind_lan {
        detect_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        "127.0.0.1".to_string()
    };
    let local_url = format!("http://127.0.0.1:{port}/?token={token}");
    let url = format!("http://{host}:{port}/?token={token}");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_token = token.clone();

    let task = tauri::async_runtime::spawn(async move {
        run_test_server(
            listener,
            app_handle,
            db_pool,
            server_token,
            shutdown_rx,
        )
        .await;
    });

    let info = DiagnosticTestServerInfo {
        running: true,
        bind_lan,
        port,
        token,
        url,
        local_url,
    };

    *server = Some(DiagnosticTestServerHandle {
        info: info.clone(),
        shutdown: shutdown_tx,
        task,
    });

    Ok(info)
}

pub async fn stop_test_server(state: &DiagnosticTestServerState) -> Result<(), String> {
    let mut server = state.server.lock().await;
    if let Some(handle) = server.take() {
        let _ = handle.shutdown.send(());
        handle.task.abort();
    }
    Ok(())
}

async fn run_test_server(
    listener: TcpListener,
    app_handle: AppHandle,
    db_pool: Pool<Sqlite>,
    token: String,
    mut shutdown: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, addr)) => {
                        let app_handle = app_handle.clone();
                        let db_pool = db_pool.clone();
                        let token = token.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = handle_test_connection(stream, addr, app_handle, db_pool, token).await {
                                log::warn!("[DiagnosticTestServer] request failed: {error}");
                            }
                        });
                    }
                    Err(error) => {
                        log::warn!("[DiagnosticTestServer] accept failed: {error}");
                        break;
                    }
                }
            }
        }
    }
}

async fn handle_test_connection(
    mut stream: TcpStream,
    _addr: SocketAddr,
    app_handle: AppHandle,
    db_pool: Pool<Sqlite>,
    token: String,
) -> Result<(), String> {
    let (headers, body_bytes) = read_http_request(&mut stream).await?;
    if headers.is_empty() {
        return Ok(());
    }

    let body = String::from_utf8_lossy(&body_bytes);
    let first_line = headers.lines().next().unwrap_or_default();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or("/");
    let (path, query) = split_target(target);

    if method == "OPTIONS" {
        write_response(&mut stream, 204, "text/plain; charset=utf-8", "").await?;
        return Ok(());
    }

    if extract_token(query) != Some(token.as_str()) {
        write_response(
            &mut stream,
            403,
            "text/plain; charset=utf-8",
            "Forbidden: invalid token.",
        )
        .await?;
        return Ok(());
    }

    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => {
            let page = render_test_page(&token);
            write_response(&mut stream, 200, "text/html; charset=utf-8", &page).await?;
        }
        ("GET", "/status") => {
            let response = json!({
                "status": "ok",
                "server": "vcp-mobile-diagnostic-test",
                "time": now_ms(),
                "endpoints": [
                    "POST /inject/assistant",
                    "GET /sample/assistant"
                ],
            })
            .to_string();
            write_response(&mut stream, 200, "application/json", &response).await?;
        }
        ("GET", "/sample/assistant") => {
            let response = json!({
                "ownerId": "agent id from current chat",
                "ownerType": "agent",
                "topicId": "topic id from current chat",
                "name": "Diagnostic AI",
                "content": sample_tool_result_content(),
            })
            .to_string();
            write_response(&mut stream, 200, "application/json", &response).await?;
        }
        ("POST", "/inject/assistant") => {
            let payload: DiagnosticInjectAssistantPayload = match serde_json::from_str(&body) {
                Ok(payload) => payload,
                Err(error) => {
                    let response = json!({
                        "status": "error",
                        "error": format!("Invalid injection JSON: {error}"),
                    })
                    .to_string();
                    write_response(&mut stream, 400, "application/json", &response).await?;
                    return Ok(());
                }
            };

            match inject_assistant_message_shared(app_handle, db_pool, payload).await {
                Ok(injected) => {
                    let response = serde_json::to_string(&json!({
                        "status": "ok",
                        "injected": injected,
                    }))
                    .map_err(|error| error.to_string())?;
                    write_response(&mut stream, 200, "application/json", &response).await?;
                }
                Err(error) => {
                    let response = json!({
                        "status": "error",
                        "error": error,
                    })
                    .to_string();
                    write_response(&mut stream, 400, "application/json", &response).await?;
                }
            }
        }
        _ => {
            write_response(&mut stream, 404, "text/plain; charset=utf-8", "Not found.").await?;
        }
    }

    Ok(())
}

async fn read_http_request(stream: &mut TcpStream) -> Result<(String, Vec<u8>), String> {
    const MAX_REQUEST_BYTES: usize = 1024 * 1024;

    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    let mut header_end = None;
    let mut content_length = 0usize;

    loop {
        let size = stream
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        if size == 0 {
            break;
        }

        bytes.extend_from_slice(&buffer[..size]);
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err("Diagnostic test request is too large.".to_string());
        }

        if header_end.is_none() {
            if let Some(end) = find_header_end(&bytes) {
                header_end = Some(end);
                let headers = String::from_utf8_lossy(&bytes[..end]);
                content_length = parse_content_length(&headers).unwrap_or(0);
            }
        }

        if let Some(end) = header_end {
            if bytes.len() >= end + 4 + content_length {
                break;
            }
        }
    }

    if bytes.is_empty() {
        return Ok((String::new(), Vec::new()));
    }

    let end = header_end.ok_or_else(|| "Malformed diagnostic HTTP request.".to_string())?;
    let body_start = end + 4;
    let body_end = (body_start + content_length).min(bytes.len());
    Ok((
        String::from_utf8_lossy(&bytes[..end]).to_string(),
        bytes[body_start..body_end].to_vec(),
    ))
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> Option<usize> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn split_target(target: &str) -> (&str, &str) {
    match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target, ""),
    }
}

fn extract_token(query: &str) -> Option<&str> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        if key == "token" {
            Some(value)
        } else {
            None
        }
    })
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: content-type\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\n\r\n{body}",
        body.as_bytes().len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| error.to_string())
}

fn render_test_page(token: &str) -> String {
    format!(
        r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>VCPMobile Diagnostic Test API</title>
  <style>
    body {{ margin: 0; font-family: system-ui, sans-serif; background: #10151f; color: #f5f7fb; }}
    main {{ max-width: 860px; margin: 0 auto; padding: 24px; }}
    code, pre {{ background: rgba(255,255,255,.08); border-radius: 8px; }}
    code {{ padding: 2px 6px; }}
    pre {{ padding: 12px; overflow: auto; white-space: pre-wrap; }}
  </style>
</head>
<body>
  <main>
    <h1>VCPMobile Diagnostic Test API</h1>
    <p>Use <code>POST /inject/assistant?token={token}</code> to inject an assistant message into an existing topic.</p>
    <pre>curl -X POST "http://127.0.0.1:PORT/inject/assistant?token={token}" \
  -H "Content-Type: application/json" \
  --data '{{"ownerId":"...","ownerType":"agent","topicId":"...","content":"hello"}}'</pre>
  </main>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcp_modules::content_parser::{parse_content, ContentBlock};

    #[test]
    fn sample_content_exercises_tool_blocks() {
        let blocks = parse_content(sample_tool_result_content());

        assert!(blocks.iter().any(|block| matches!(
            block,
            ContentBlock::ToolUse { tool_name, .. } if tool_name == "ImageGen"
        )));
        assert!(blocks.iter().any(|block| matches!(
            block,
            ContentBlock::ToolResult { tool_name, status, .. }
                if tool_name == "ImageGen" && status == "success"
        )));
    }

    #[test]
    fn validate_payload_rejects_missing_topic() {
        let payload = DiagnosticInjectAssistantPayload {
            owner_id: "agent_1".to_string(),
            owner_type: "agent".to_string(),
            topic_id: "".to_string(),
            content: "hello".to_string(),
            message_id: None,
            name: None,
            agent_id: None,
            timestamp: None,
        };

        assert!(validate_payload(&payload).is_err());
    }
}

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, State};
#[cfg(target_os = "android")]
use tauri::Manager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Mutex, RwLock};
use tauri::async_runtime::JoinHandle;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserLink {
    pub text: String,
    pub href: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserForm {
    pub id: Option<String>,
    pub name: Option<String>,
    pub action: Option<String>,
    pub method: Option<String>,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSnapshot {
    pub status: String,
    pub control_mode: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub text: Option<String>,
    pub links: Vec<BrowserLink>,
    pub forms: Vec<BrowserForm>,
    pub screenshot: Option<String>,
    pub last_action: Option<String>,
    pub waiting_reason: Option<String>,
    pub bridge_kind: String,
    pub updated_at: u64,
}

impl Default for BrowserSnapshot {
    fn default() -> Self {
        Self {
            status: "idle".to_string(),
            control_mode: "ai".to_string(),
            url: None,
            title: None,
            text: None,
            links: Vec::new(),
            forms: Vec::new(),
            screenshot: None,
            last_action: None,
            waiting_reason: None,
            bridge_kind: browser_bridge_kind().to_string(),
            updated_at: now_ms(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAction {
    #[serde(alias = "command")]
    pub action: String,
    pub url: Option<String>,
    pub selector: Option<String>,
    pub text: Option<String>,
    pub value: Option<String>,
    pub script: Option<String>,
    pub direction: Option<String>,
    pub amount: Option<i64>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAssistInfo {
    pub running: bool,
    pub bind_lan: bool,
    pub port: u16,
    pub token: String,
    pub url: String,
    pub local_url: String,
}

struct AssistServerHandle {
    info: BrowserAssistInfo,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

#[derive(Clone)]
pub struct BrowserRuntimeState {
    snapshot: Arc<RwLock<BrowserSnapshot>>,
    assist: Arc<Mutex<Option<AssistServerHandle>>>,
}

impl BrowserRuntimeState {
    fn new() -> Self {
        Self {
            snapshot: Arc::new(RwLock::new(BrowserSnapshot::default())),
            assist: Arc::new(Mutex::new(None)),
        }
    }

    pub fn global() -> Self {
        static INSTANCE: std::sync::LazyLock<BrowserRuntimeState> =
            std::sync::LazyLock::new(BrowserRuntimeState::new);
        INSTANCE.clone()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn browser_bridge_kind() -> &'static str {
    #[cfg(target_os = "android")]
    {
        "android-webview"
    }
    #[cfg(not(target_os = "android"))]
    {
        "rust-fallback"
    }
}

fn random_token() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

fn get_action_name(action: &BrowserAction) -> String {
    action.action.trim().to_ascii_lowercase()
}

async fn update_snapshot_from_value(
    state: &BrowserRuntimeState,
    action_name: &str,
    value: &Value,
) -> BrowserSnapshot {
    let mut current = state.snapshot.write().await;
    if let Ok(mut next) = serde_json::from_value::<BrowserSnapshot>(value.clone()) {
        next.last_action = Some(action_name.to_string());
        next.bridge_kind = browser_bridge_kind().to_string();
        next.updated_at = now_ms();
        *current = next;
    } else {
        current.last_action = Some(action_name.to_string());
        current.updated_at = now_ms();
    }
    current.clone()
}

async fn apply_local_action(
    state: &BrowserRuntimeState,
    action: &BrowserAction,
) -> Result<Value, String> {
    let action_name = get_action_name(action);
    let mut snapshot = state.snapshot.write().await;
    snapshot.bridge_kind = browser_bridge_kind().to_string();
    snapshot.last_action = Some(action_name.clone());
    snapshot.updated_at = now_ms();

    match action_name.as_str() {
        "open" | "navigate" => {
            let url = action
                .url
                .as_deref()
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .ok_or_else(|| "Missing url for browser navigate action.".to_string())?;
            snapshot.url = Some(url.to_string());
            snapshot.title = Some(url.to_string());
            snapshot.status = "loaded".to_string();
            snapshot.control_mode = "ai".to_string();
            snapshot.waiting_reason = None;
            snapshot.text = Some(
                "Native Android WebView control is available on device builds. This desktop fallback records the target URL only."
                    .to_string(),
            );
        }
        "handoff" | "handoff_to_user" | "wait_for_user" => {
            snapshot.status = "waiting_for_user".to_string();
            snapshot.control_mode = "user".to_string();
            snapshot.waiting_reason = action
                .reason
                .clone()
                .or_else(|| Some("AI requested manual browser assistance.".to_string()));
        }
        "resume" | "resume_ai" => {
            snapshot.status = "idle".to_string();
            snapshot.control_mode = "ai".to_string();
            snapshot.waiting_reason = None;
        }
        "snapshot" | "read" | "read_text" | "read_dom" => {}
        "back" | "forward" | "reload" | "click" | "type" | "scroll" | "screenshot" | "tap"
        | "select" | "eval" | "evaluate" | "wait_for" => {
            snapshot.status = "pending_native_webview".to_string();
        }
        other => return Err(format!("Unsupported browser action: {other}")),
    }

    Ok(json!({
        "status": "success",
        "bridgeKind": browser_bridge_kind(),
        "snapshot": snapshot.clone(),
    }))
}

#[cfg(target_os = "android")]
fn execute_android_browser_action(app: &AppHandle, action: &BrowserAction) -> Result<Value, String> {
    use std::sync::mpsc;
    use std::time::Duration;

    let action_json = serde_json::to_string(action).map_err(|error| error.to_string())?;
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Main webview window is not available.".to_string())?;
    let (tx, rx) = mpsc::channel();

    window
        .with_webview(move |platform_webview| {
            let jni_handle = platform_webview.jni_handle();
            jni_handle.exec(move |env, activity, _webview| {
                use jni::objects::{JClass, JObject, JValue};

                let result = (|| -> Result<String, String> {
                    let class_loader = env
                        .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
                        .map_err(|error| error.to_string())?
                        .l()
                        .map_err(|error| error.to_string())?;
                    let bridge_name = env
                        .new_string("com.vcp.avatar.BrowserBridge")
                        .map_err(|error| error.to_string())?;
                    let bridge_name = JObject::from(bridge_name);
                    let bridge_class = env
                        .call_method(
                            &class_loader,
                            "loadClass",
                            "(Ljava/lang/String;)Ljava/lang/Class;",
                            &[JValue::Object(&bridge_name)],
                        )
                        .map_err(|error| error.to_string())?
                        .l()
                        .map_err(|error| error.to_string())?;
                    let bridge_class = JClass::from(bridge_class);
                    let action = env
                        .new_string(action_json)
                        .map_err(|error| error.to_string())?;
                    let action = JObject::from(action);
                    let output = env
                        .call_static_method(
                            bridge_class,
                            "execute",
                            "(Landroid/app/Activity;Ljava/lang/String;)Ljava/lang/String;",
                            &[JValue::Object(activity), JValue::Object(&action)],
                        )
                        .map_err(|error| error.to_string())?
                        .l()
                        .map_err(|error| error.to_string())?;
                    let output = jni::objects::JString::from(output);
                    env.get_string(&output)
                        .map(|value| value.to_string_lossy().to_string())
                        .map_err(|error| error.to_string())
                })();

                let _ = tx.send(result);
            });
        })
        .map_err(|error| error.to_string())?;

    let result = rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "Android browser bridge timed out.".to_string())??;
    serde_json::from_str(&result).map_err(|error| format!("Invalid Android browser JSON: {error}"))
}

#[cfg(not(target_os = "android"))]
fn execute_android_browser_action(_app: &AppHandle, _action: &BrowserAction) -> Result<Value, String> {
    Err("Android browser bridge is not available on this platform.".to_string())
}

pub async fn execute_browser_action_shared(
    app: &AppHandle,
    state: &BrowserRuntimeState,
    action: BrowserAction,
) -> Result<Value, String> {
    let action_name = get_action_name(&action);
    if action_name.is_empty() {
        return Err("Missing browser action.".to_string());
    }

    let value = match action_name.as_str() {
        "handoff" | "handoff_to_user" | "wait_for_user" | "resume" | "resume_ai" => {
            apply_local_action(state, &action).await?
        }
        _ => match execute_android_browser_action(app, &action) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("[BrowserRuntime] Falling back to local action: {error}");
                apply_local_action(state, &action).await?
            }
        },
    };

    let snapshot = update_snapshot_from_value(state, &action_name, &value).await;
    Ok(json!({
        "status": "success",
        "action": action_name,
        "result": value,
        "snapshot": snapshot,
    }))
}

#[tauri::command]
pub async fn browser_execute_action(
    app: AppHandle,
    state: State<'_, BrowserRuntimeState>,
    action: BrowserAction,
) -> Result<Value, String> {
    execute_browser_action_shared(&app, &state, action).await
}

#[tauri::command]
pub async fn get_browser_snapshot(
    state: State<'_, BrowserRuntimeState>,
) -> Result<BrowserSnapshot, String> {
    Ok(state.snapshot.read().await.clone())
}

#[tauri::command]
pub async fn start_browser_assist_server(
    app: AppHandle,
    state: State<'_, BrowserRuntimeState>,
    bind_lan: Option<bool>,
) -> Result<BrowserAssistInfo, String> {
    start_assist_server(Some(app), &state, bind_lan.unwrap_or(false)).await
}

#[tauri::command]
pub async fn stop_browser_assist_server(
    state: State<'_, BrowserRuntimeState>,
) -> Result<(), String> {
    stop_assist_server(&state).await
}

pub async fn start_assist_server(
    app: Option<AppHandle>,
    state: &BrowserRuntimeState,
    bind_lan: bool,
) -> Result<BrowserAssistInfo, String> {
    let mut assist = state.assist.lock().await;
    if let Some(existing) = assist.as_ref() {
        return Ok(existing.info.clone());
    }

    let bind_addr = if bind_lan { "0.0.0.0:0" } else { "127.0.0.1:0" };
    let listener = TcpListener::bind(bind_addr)
        .await
        .map_err(|error| format!("Failed to bind browser assist server: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let token = random_token();
    let local_url = format!("http://127.0.0.1:{port}/?token={token}");
    let host = if bind_lan {
        detect_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        "127.0.0.1".to_string()
    };
    let url = format!("http://{host}:{port}/?token={token}");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_state = state.clone();
    let server_token = token.clone();

    let task = tauri::async_runtime::spawn(async move {
        run_assist_server(listener, server_state, app, server_token, shutdown_rx).await;
    });

    let info = BrowserAssistInfo {
        running: true,
        bind_lan,
        port,
        token,
        url,
        local_url,
    };

    *assist = Some(AssistServerHandle {
        info: info.clone(),
        shutdown: shutdown_tx,
        task,
    });

    Ok(info)
}

pub async fn stop_assist_server(state: &BrowserRuntimeState) -> Result<(), String> {
    let mut assist = state.assist.lock().await;
    if let Some(handle) = assist.take() {
        let _ = handle.shutdown.send(());
        handle.task.abort();
    }
    Ok(())
}

async fn run_assist_server(
    listener: TcpListener,
    state: BrowserRuntimeState,
    app: Option<AppHandle>,
    token: String,
    mut shutdown: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, addr)) => {
                        let state = state.clone();
                        let app = app.clone();
                        let token = token.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = handle_assist_connection(stream, addr, state, app, token).await {
                                log::warn!("[BrowserAssist] request failed: {error}");
                            }
                        });
                    }
                    Err(error) => {
                        log::warn!("[BrowserAssist] accept failed: {error}");
                        break;
                    }
                }
            }
        }
    }
}

async fn handle_assist_connection(
    mut stream: TcpStream,
    _addr: SocketAddr,
    state: BrowserRuntimeState,
    app: Option<AppHandle>,
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
            let body = render_assist_page(&token);
            write_response(&mut stream, 200, "text/html; charset=utf-8", &body).await?;
        }
        ("GET", "/status") => {
            let body = serde_json::to_string(&*state.snapshot.read().await)
                .map_err(|error| error.to_string())?;
            write_response(&mut stream, 200, "application/json", &body).await?;
        }
        ("POST", "/action") => {
            let app = app
                .as_ref()
                .ok_or_else(|| "Browser actions are not available for this assist server.".to_string())?;
            let action: BrowserAction = match serde_json::from_str(&body) {
                Ok(action) => action,
                Err(error) => {
                    let body = json!({
                        "status": "error",
                        "error": format!("Invalid action JSON: {error}"),
                    })
                    .to_string();
                    write_response(&mut stream, 400, "application/json", &body).await?;
                    return Ok(());
                }
            };
            let result = match execute_browser_action_shared(app, &state, action).await {
                Ok(result) => result,
                Err(error) => {
                    let body = json!({
                        "status": "error",
                        "error": error,
                    })
                    .to_string();
                    write_response(&mut stream, 500, "application/json", &body).await?;
                    return Ok(());
                }
            };
            let body = serde_json::to_string(&result).map_err(|error| error.to_string())?;
            write_response(&mut stream, 200, "application/json", &body).await?;
        }
        ("POST", "/resume") => {
            let mut current = state.snapshot.write().await;
            current.status = "idle".to_string();
            current.control_mode = "ai".to_string();
            current.waiting_reason = None;
            current.last_action = Some("manual_resume".to_string());
            current.updated_at = now_ms();
            write_response(&mut stream, 200, "application/json", "{\"status\":\"ok\"}").await?;
        }
        ("POST", "/handoff") => {
            let mut current = state.snapshot.write().await;
            current.status = "waiting_for_user".to_string();
            current.control_mode = "user".to_string();
            current.waiting_reason = Some("Manual assist page requested user control.".to_string());
            current.last_action = Some("manual_handoff".to_string());
            current.updated_at = now_ms();
            write_response(&mut stream, 200, "application/json", "{\"status\":\"ok\"}").await?;
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
            return Err("Browser assist request is too large.".to_string());
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

    let end = header_end.ok_or_else(|| "Malformed browser assist HTTP request.".to_string())?;
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
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.as_bytes().len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| error.to_string())
}

fn render_assist_page(token: &str) -> String {
    format!(
        r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>VCPMobile Browser Assist</title>
  <style>
    body {{ margin: 0; font-family: system-ui, sans-serif; background: #0f172a; color: #f8fafc; }}
    main {{ max-width: 860px; margin: 0 auto; padding: 24px; }}
    h1 {{ font-size: 22px; margin: 0 0 12px; }}
    button {{ border: 0; border-radius: 8px; padding: 10px 14px; font-weight: 700; margin: 0 8px 8px 0; }}
    input {{ border: 1px solid rgba(255,255,255,.16); border-radius: 8px; background: rgba(0,0,0,.2); color: #f8fafc; padding: 10px 12px; width: min(100%, 360px); margin: 0 8px 8px 0; }}
    .primary {{ background: #22c55e; color: #052e16; }}
    .secondary {{ background: #334155; color: #f8fafc; }}
    .panel {{ background: rgba(255,255,255,.08); border: 1px solid rgba(255,255,255,.12); border-radius: 8px; padding: 14px; margin-top: 14px; }}
    #shot {{ max-width: 100%; border-radius: 8px; border: 1px solid rgba(255,255,255,.14); margin-top: 10px; }}
    pre {{ white-space: pre-wrap; word-break: break-word; font-size: 12px; line-height: 1.5; }}
    a {{ color: #93c5fd; }}
  </style>
</head>
<body>
  <main>
    <h1>VCPMobile Browser Assist</h1>
    <p>Use the phone browser view to finish login, captcha, 2FA, or other checks. Press resume when the page is ready for AI control.</p>
    <button class="primary" onclick="post('/resume')">Resume AI</button>
    <button class="secondary" onclick="post('/handoff')">Keep User Control</button>
    <button class="secondary" onclick="action({{action:'screenshot'}})">Refresh Screenshot</button>
    <button class="secondary" onclick="action({{action:'scroll',direction:'down',amount:720}})">Scroll Down</button>
    <div class="panel">
      <input id="selector" placeholder="CSS selector">
      <input id="value" placeholder="value/text">
      <button class="secondary" onclick="action({{action:'click',selector:document.getElementById('selector').value}})">Click Selector</button>
      <button class="secondary" onclick="action({{action:'type',selector:document.getElementById('selector').value,value:document.getElementById('value').value}})">Type</button>
      <img id="shot" alt="">
    </div>
    <div class="panel"><strong>Current browser state</strong><pre id="state">Loading...</pre></div>
  </main>
  <script>
    const token = "{token}";
    async function load() {{
      const res = await fetch('/status?token=' + encodeURIComponent(token));
      const state = await res.json();
      render(state);
    }}
    function render(state) {{
      const image = state.screenshot;
      if (image) document.getElementById('shot').src = image;
      const clean = Object.assign({{}}, state);
      if (clean.screenshot) clean.screenshot = '[data-url omitted]';
      document.getElementById('state').textContent = JSON.stringify(clean, null, 2);
    }}
    async function action(payload) {{
      const res = await fetch('/action?token=' + encodeURIComponent(token), {{
        method: 'POST',
        headers: {{ 'content-type': 'application/json' }},
        body: JSON.stringify(payload)
      }});
      const data = await res.json();
      render(data.snapshot || data.result || data);
    }}
    async function post(path) {{
      await fetch(path + '?token=' + encodeURIComponent(token), {{ method: 'POST' }});
      await load();
    }}
    load();
    setInterval(load, 1500);
  </script>
</body>
</html>"#
    )
}

fn detect_lan_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, watch};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

const MOBILE_USER_AGENT: &str =
    "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36";
const MIN_VCP_LOG_HEARTBEAT_MS: u64 = 5_000;

static HEARTBEAT_INTERVAL_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(15000);

lazy_static::lazy_static! {
    static ref LOG_CONNECTION_ACTIVE: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    static ref LOG_SENDER: Arc<tokio::sync::Mutex<Option<mpsc::UnboundedSender<Value>>>> = Arc::new(tokio::sync::Mutex::new(None));
    // 关键修复：保持 Sender 和一个 Receiver 都在生命周期内，防止通道因无接收者而被视为关闭
    static ref WS_URL_CHANNEL: (watch::Sender<Option<Url>>, watch::Receiver<Option<Url>>) = watch::channel(None);
    static ref CURRENT_LOG_STATUS: Arc<tokio::sync::RwLock<String>> = Arc::new(tokio::sync::RwLock::new("closed".to_string()));
    static ref HEARTBEAT_RESET_TX: Arc<tokio::sync::Mutex<Option<mpsc::Sender<()>>>> = Arc::new(tokio::sync::Mutex::new(None));
    static ref BACKGROUND_LOG_CACHE: std::sync::Mutex<VecDeque<Value>> = std::sync::Mutex::new(VecDeque::new());
}

const MAX_BACKGROUND_LOG_EVENTS: usize = 500;

pub async fn handle_foreground_state_change<R: tauri::Runtime>(
    _app: &AppHandle<R>,
    is_foreground: bool,
) {
    let heartbeat_ms = if is_foreground { 15_000 } else { 120_000 };
    let _ = set_vcp_log_heartbeat(heartbeat_ms).await;
}

pub async fn disconnect_log_connections<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    let log_result =
        init_vcp_log_connection_internal(app.clone(), String::new(), String::new()).await;
    let info_result = super::vcp_info_service::init_vcp_info_connection_internal(
        app.clone(),
        String::new(),
        String::new(),
    )
    .await;

    combine_connection_results("disconnect", log_result, info_result)
}

pub async fn reconnect_log_connections<R: tauri::Runtime>(
    app: &AppHandle<R>,
    log_url: String,
    log_key: String,
) -> Result<(), String> {
    let log_result =
        init_vcp_log_connection_internal(app.clone(), log_url.clone(), log_key.clone()).await;
    let info_result =
        super::vcp_info_service::init_vcp_info_connection_internal(app.clone(), log_url, log_key)
            .await;

    combine_connection_results("reconnect", log_result, info_result)
}

fn combine_connection_results(
    action: &str,
    log_result: Result<(), String>,
    info_result: Result<(), String>,
) -> Result<(), String> {
    match (log_result, info_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(log_error), Ok(())) => Err(format!("Failed to {action} VCPLog: {log_error}")),
        (Ok(()), Err(info_error)) => Err(format!("Failed to {action} VCPInfo: {info_error}")),
        (Err(log_error), Err(info_error)) => Err(format!(
            "Failed to {action} VCPLog ({log_error}) and VCPInfo ({info_error})"
        )),
    }
}

fn emit_log_event<R: tauri::Runtime>(app: &AppHandle<R>, payload: Value) {
    if !crate::vcp_modules::infra::lifecycle_manager::is_app_in_foreground(app) {
        if let Ok(mut cache) = BACKGROUND_LOG_CACHE.lock() {
            if cache.len() >= MAX_BACKGROUND_LOG_EVENTS {
                cache.pop_front();
            }
            cache.push_back(payload);
        }
        return;
    }
    let _ = app.emit("vcp-system-event", payload);
}

pub fn flush_background_logs<R: tauri::Runtime>(app: &AppHandle<R>) {
    let events = BACKGROUND_LOG_CACHE
        .lock()
        .map(|mut cache| cache.drain(..).collect::<Vec<_>>())
        .unwrap_or_default();
    for payload in events {
        let _ = app.emit("vcp-system-event", payload);
    }
}

#[tauri::command]
pub async fn set_vcp_log_heartbeat(interval_ms: u64) -> Result<(), String> {
    HEARTBEAT_INTERVAL_MS.store(log_heartbeat_interval_ms(interval_ms), Ordering::SeqCst);
    {
        let tx_lock = HEARTBEAT_RESET_TX.lock().await;
        if let Some(tx) = tx_lock.as_ref() {
            let _ = tx.send(()).await;
        }
    }
    super::vcp_info_service::set_vcp_info_heartbeat(interval_ms).await;
    Ok(())
}

fn log_heartbeat_interval_ms(raw_ms: u64) -> u64 {
    raw_ms.max(MIN_VCP_LOG_HEARTBEAT_MS)
}

fn current_log_heartbeat_interval_ms() -> u64 {
    log_heartbeat_interval_ms(HEARTBEAT_INTERVAL_MS.load(Ordering::SeqCst))
}

pub async fn get_vcp_log_status_internal() -> String {
    CURRENT_LOG_STATUS.read().await.clone()
}

#[tauri::command]
pub async fn send_vcp_log_message(payload: serde_json::Value) -> Result<(), String> {
    let sender_lock = LOG_SENDER.lock().await;
    if let Some(sender) = sender_lock.as_ref() {
        sender
            .send(payload)
            .map_err(|e| format!("Failed to send message to VCPLog: {}", e))?;
        Ok(())
    } else {
        Err("VCPLog connection is not active".to_string())
    }
}

fn parse_log_url(url: &str, key: &str) -> Result<Url, String> {
    let mut base_url = url.trim_end_matches('/').to_string();
    if !base_url.contains("/VCPlog") {
        base_url.push_str("/VCPlog");
    }

    let url_with_key = if base_url.contains("VCP_Key=") {
        base_url
    } else {
        if !base_url.ends_with('/') {
            base_url.push('/');
        }
        format!("{}VCP_Key={}", base_url, key)
    };

    Url::parse(&url_with_key).map_err(|e| format!("Invalid URL: {}", e))
}

fn build_ws_request(url: &Url) -> Result<Request<()>, String> {
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| e.to_string())?;

    if let Some(host) = url.host_str() {
        let host_with_port = if let Some(port) = url.port() {
            format!("{}:{}", host, port)
        } else {
            host.to_string()
        };
        if let Ok(val) = host_with_port.parse() {
            request.headers_mut().insert("Host", val);
        }

        let origin_scheme = match url.scheme() {
            "wss" => "https",
            _ => "http",
        };
        let origin = if let Some(port) = url.port() {
            format!("{}://{}:{}", origin_scheme, host, port)
        } else {
            format!("{}://{}", origin_scheme, host)
        };
        if let Ok(val) = origin.parse() {
            request.headers_mut().insert("Origin", val);
        }
    }

    if let Ok(val) = MOBILE_USER_AGENT.parse() {
        request.headers_mut().insert("User-Agent", val);
    }

    Ok(request)
}

#[tauri::command]
pub async fn init_vcp_log_connection(
    app: AppHandle,
    url: String,
    key: String,
) -> Result<(), String> {
    init_vcp_log_connection_internal(app, url, key).await
}

pub async fn init_vcp_log_connection_internal<R: tauri::Runtime>(
    app: AppHandle<R>,
    url: String,
    key: String,
) -> Result<(), String> {
    // 如果 URL 或 Key 为空，发送 None 以停止现有连接并进入静默等待
    if url.trim().is_empty() || key.trim().is_empty() {
        WS_URL_CHANNEL
            .0
            .send(None)
            .map_err(|_| "VCPLog control channel is unavailable".to_string())?;
        {
            let mut sender_lock = LOG_SENDER.lock().await;
            *sender_lock = None;
        }
        return Ok(());
    }

    let ws_url = parse_log_url(&url, &key)?;

    // Always send the new URL to the watch channel
    WS_URL_CHANNEL
        .0
        .send(Some(ws_url.clone()))
        .map_err(|_| "VCPLog control channel is unavailable".to_string())?;

    if LOG_CONNECTION_ACTIVE.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        start_vcp_log_listener(h).await;
    });

    Ok(())
}

async fn start_vcp_log_listener<R: tauri::Runtime>(app_handle: AppHandle<R>) {
    let mut url_rx = WS_URL_CHANNEL.0.subscribe();

    // 创建 mpsc 通道用于回传消息；仅在 WebSocket 已连接时暴露 sender，
    // 避免断开期间 send_vcp_log_message 看似成功却无限积压内存。
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();

    let mut retry_delay = Duration::from_millis(1000);
    loop {
        // 获取当前 URL
        let ws_url = {
            let val = url_rx.borrow().clone();
            match val {
                Some(u) => u,
                None => {
                    if url_rx.changed().await.is_err() {
                        break;
                    }
                    continue;
                }
            }
        };

        let masked_url = if ws_url.as_str().contains("VCP_Key=") {
            let parts: Vec<&str> = ws_url.as_str().split("VCP_Key=").collect();
            format!("{}VCP_Key=********", parts[0])
        } else {
            ws_url.to_string()
        };
        log::info!("[VCPLog] Attempting to connect to {}...", masked_url);

        {
            *CURRENT_LOG_STATUS.write().await = "connecting".to_string();
        }

        let _ = app_handle.emit(
            "vcp-system-event",
            serde_json::json!({
                "type": "vcp-log-status",
                "status": "connecting",
                "message": "连接中...",
                "source": "VCPLog"
            }),
        );

        let request = match build_ws_request(&ws_url) {
            Ok(req) => req,
            Err(e) => {
                {
                    *CURRENT_LOG_STATUS.write().await = "error".to_string();
                }
                log::error!(
                    "[VCPLog] Failed to build request: {}. Retrying in 5 seconds...",
                    e
                );
                let _ = app_handle.emit(
                    "vcp-system-event",
                    serde_json::json!({
                        "type": "vcp-log-status",
                        "status": "error",
                        "message": "连接错误",
                        "source": "VCPLog"
                    }),
                );

                // 错误卡片 1：请求构建失败 (例如 URL 格式错误)
                let _ = app_handle.emit(
                    "vcp-system-event",
                    serde_json::json!({
                        "type": "vcp-log-message",
                        "data": {
                            "id": "vcp_log_connection_status",
                            "status": "error",
                            "tool_name": "VCPLog 请求异常",
                            "content": format!("❌ 无法构造请求: {}\n\n提示：请检查配置的 URL 格式是否正确。", e),
                            "source": "VCPLog"
                        }
                    }),
                );

                tokio::select! {
                    _ = url_rx.changed() => {},
                    _ = sleep(retry_delay) => {},
                }
                retry_delay = (retry_delay * 2).min(Duration::from_secs(60));
                continue;
            }
        };

        match tokio::time::timeout(Duration::from_secs(10), connect_async(request)).await {
            Ok(connection_result) => match connection_result {
                Ok((ws_stream, _)) => {
                    retry_delay = Duration::from_millis(1000);
                    {
                        *CURRENT_LOG_STATUS.write().await = "connected".to_string();
                    }
                    log::info!("[VCPLog] Connected successfully to {}", masked_url);

                    let (mut ws_write, mut ws_read) = ws_stream.split();
                    {
                        let mut sender_lock = LOG_SENDER.lock().await;
                        *sender_lock = Some(tx.clone());
                    }

                    let _ = app_handle.emit(
                        "vcp-system-event",
                        serde_json::json!({
                            "type": "vcp-log-status",
                            "status": "connected",
                            "message": "已连接",
                            "source": "VCPLog"
                        }),
                    );

                    // 额外发送一条连接成功的通知卡片
                    let _ = app_handle.emit(
                        "vcp-system-event",
                        serde_json::json!({
                            "type": "vcp-log-message",
                            "data": {
                                "id": "vcp_log_connection_status",
                                "status": "success",
                                "tool_name": "VCPLog",
                                "content": "✅ VCPLog 连接成功！已建立实时数据通道。",
                                "source": "VCPLog"
                            }
                        }),
                    );

                    let (reset_tx, mut reset_rx) = mpsc::channel::<()>(8);
                    {
                        let mut tx_lock = HEARTBEAT_RESET_TX.lock().await;
                        *tx_lock = Some(reset_tx);
                    }

                    let initial_ms = current_log_heartbeat_interval_ms();
                    let mut heartbeat_timer = Box::pin(sleep(Duration::from_millis(initial_ms)));

                    loop {
                        tokio::select! {
                            // 监听 URL 变更
                            _ = url_rx.changed() => {
                                log::info!("[VCPLog] URL changed, closing current connection.");
                                break;
                            }
                            // 监听心跳重置信号
                            Some(_) = reset_rx.recv() => {
                                let current_ms = current_log_heartbeat_interval_ms();
                                log::info!("[VCPLog] Heartbeat interval updated to {}ms, resetting timer.", current_ms);
                                heartbeat_timer.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(current_ms));
                            }
                            // 心跳周期触发
                            _ = &mut heartbeat_timer => {
                                if let Err(e) = ws_write.send(Message::Ping(vec![].into())).await {
                                    log::error!("[VCPLog] Failed to send Ping: {}", e);
                                    break;
                                }
                                let current_ms = current_log_heartbeat_interval_ms();
                                heartbeat_timer.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(current_ms));
                            }
                            // 处理接收到的消息
                            msg_result = ws_read.next() => {
                                match msg_result {
                                    Some(Ok(msg)) => {
                                        if msg.is_text() {
                                            let text = msg.to_text().unwrap_or_default();
                                            match serde_json::from_str::<Value>(text) {
                                                Ok(payload) => {
                                                    emit_log_event(&app_handle, payload);
                                                }
                                                Err(_) => {
                                                    emit_log_event(&app_handle, serde_json::json!({
                                                        "type": "raw_text",
                                                        "data": text
                                                    }));
                                                }
                                            }
                                        }
                                    }
                                    Some(Err(e)) => {
                                        log::error!("[VCPLog] WebSocket error during read: {}", e);
                                        break;
                                    }
                                    None => {
                                        log::warn!("[VCPLog] Connection closed by server.");
                                        break;
                                    }
                                }
                            }
                            // 处理待发送的消息
                            payload_opt = rx.recv() => {
                                if let Some(payload) = payload_opt {
                                    if let Ok(text) = serde_json::to_string(&payload) {
                                        if let Err(e) = ws_write.send(Message::Text(text.into())).await {
                                            log::error!("[VCPLog] Failed to send message: {}", e);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    {
                        let mut tx_lock = HEARTBEAT_RESET_TX.lock().await;
                        *tx_lock = None;
                    }
                    {
                        let mut sender_lock = LOG_SENDER.lock().await;
                        *sender_lock = None;
                    }

                    log::info!("[VCPLog] Disconnected from {}.", ws_url);
                    {
                        *CURRENT_LOG_STATUS.write().await = "closed".to_string();
                    }
                    let _ = app_handle.emit(
                        "vcp-system-event",
                        serde_json::json!({
                            "type": "vcp-log-status",
                            "status": "closed",
                            "message": "连接已断开",
                            "source": "VCPLog"
                        }),
                    );
                }
                Err(e) => {
                    {
                        *CURRENT_LOG_STATUS.write().await = "error".to_string();
                    }
                    log::error!("[VCPLog] Connection Error: {}. Status: {}", e, e);
                    let _ = app_handle.emit(
                        "vcp-system-event",
                        serde_json::json!({
                            "type": "vcp-log-status",
                            "status": "error",
                            "message": "连接错误",
                            "source": "VCPLog"
                        }),
                    );

                    // 额外发送一条连接错误的通知卡片，辅助排查 (错误卡片 2)
                    let _ = app_handle.emit(
                        "vcp-system-event",
                        serde_json::json!({
                            "type": "vcp-log-message",
                            "data": {
                                "id": "vcp_log_connection_status",
                                "status": "error",
                                "tool_name": "VCPLog 连接失败",
                                "content": format!("❌ 连接错误: {}\n\n提示：\n1. 请检查桌面端 VCP 是否已开启且 VCPLog 服务正常。\n2. 检查 VCP API 地址和 Key 配置是否正确。", e),
                                "source": "VCPLog"
                            }
                        }),
                    );
                }
            },
            Err(_) => {
                {
                    *CURRENT_LOG_STATUS.write().await = "error".to_string();
                }
                log::error!(
                    "[VCPLog] Connection timed out after 10 seconds. Retrying in 5 seconds..."
                );
                let _ = app_handle.emit(
                    "vcp-system-event",
                    serde_json::json!({
                        "type": "vcp-log-status",
                        "status": "error",
                        "message": "连接错误",
                        "source": "VCPLog"
                    }),
                );

                // 错误卡片 3：连接超时
                let _ = app_handle.emit(
                    "vcp-system-event",
                    serde_json::json!({
                        "type": "vcp-log-message",
                        "data": {
                            "id": "vcp_log_connection_status",
                            "status": "error",
                            "tool_name": "VCPLog 连接超时",
                            "content": "❌ 连接 VCPLog 超时 (10s)。\n\n提示：\n1. 请检查桌面端是否处于运行状态。\n2. 确认手机与电脑是否处于同一局域网。",
                            "source": "VCPLog"
                        }
                    }),
                );
            }
        }

        tokio::select! {
            _ = url_rx.changed() => log::info!("[VCPLog] URL changed during retry wait."),
            _ = sleep(retry_delay) => {},
        }
        retry_delay = (retry_delay * 2).min(Duration::from_secs(60));
    }
    LOG_CONNECTION_ACTIVE.store(false, Ordering::SeqCst);
    {
        let mut sender_lock = LOG_SENDER.lock().await;
        *sender_lock = None;
    }
    log::info!("[VCPLog] Listener task terminated, connection flag reset.");
}

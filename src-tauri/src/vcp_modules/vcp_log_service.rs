use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, sleep, Duration, Instant, MissedTickBehavior};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

lazy_static::lazy_static! {
    static ref LOG_CONNECTION_ACTIVE: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    static ref OUTBOUND_QUEUE_LEN: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    static ref LOG_SENDER: Arc<tokio::sync::Mutex<Option<mpsc::UnboundedSender<Value>>>> = Arc::new(tokio::sync::Mutex::new(None));
    // 关键修复：保持 Sender 和一个 Receiver 都在生命周期内，防止通道因无接收者而被视为关闭
    static ref WS_URL_CHANNEL: (watch::Sender<Option<Url>>, watch::Receiver<Option<Url>>) = watch::channel(None);
    static ref CURRENT_LOG_TARGET: Arc<tokio::sync::RwLock<Option<String>>> = Arc::new(tokio::sync::RwLock::new(None));
    static ref CURRENT_LOG_STATUS: Arc<tokio::sync::RwLock<String>> = Arc::new(tokio::sync::RwLock::new("disconnected".to_string()));
    static ref CURRENT_LOG_STATUS_MESSAGE: Arc<tokio::sync::RwLock<String>> = Arc::new(tokio::sync::RwLock::new("等待初始化...".to_string()));
}

const HEARTBEAT_INTERVAL_SECS: u64 = 25;
const CONNECTION_STALE_TIMEOUT_SECS: u64 = 90;
const RECONNECT_INITIAL_DELAY_SECS: u64 = 2;
const RECONNECT_MAX_DELAY_SECS: u64 = 60;
const MAX_OUTBOUND_QUEUE_LEN: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionEnd {
    UrlChanged,
    Network,
    Stale,
    SenderClosed,
}

#[tauri::command]
pub async fn get_vcp_log_status() -> Result<String, String> {
    Ok(CURRENT_LOG_STATUS.read().await.clone())
}

pub async fn get_vcp_log_status_internal() -> String {
    CURRENT_LOG_STATUS.read().await.clone()
}

#[tauri::command]
pub async fn send_vcp_log_message(payload: Value) -> Result<(), String> {
    if CURRENT_LOG_TARGET.read().await.is_none() {
        return Err("VCPLog connection is not configured".to_string());
    }

    if OUTBOUND_QUEUE_LEN.load(Ordering::SeqCst) >= MAX_OUTBOUND_QUEUE_LEN {
        return Err("VCPLog outgoing queue is full".to_string());
    }

    let sender_lock = LOG_SENDER.lock().await;
    if let Some(sender) = sender_lock.as_ref() {
        OUTBOUND_QUEUE_LEN.fetch_add(1, Ordering::SeqCst);
        sender.send(payload).map_err(|e| {
            OUTBOUND_QUEUE_LEN.fetch_sub(1, Ordering::SeqCst);
            format!("Failed to send message to VCPLog: {}", e)
        })?;
        Ok(())
    } else {
        Err("VCPLog connection is not active".to_string())
    }
}

async fn emit_log_status<R: Runtime>(app_handle: &AppHandle<R>, status: &str, message: String) {
    let current_status = CURRENT_LOG_STATUS.read().await.clone();
    let current_message = CURRENT_LOG_STATUS_MESSAGE.read().await.clone();
    if current_status == status && current_message == message {
        return;
    }

    {
        *CURRENT_LOG_STATUS.write().await = status.to_string();
        *CURRENT_LOG_STATUS_MESSAGE.write().await = message.clone();
    }

    let _ = app_handle.emit(
        "vcp-system-event",
        serde_json::json!({
            "type": "connection_status",
            "status": status,
            "message": message,
            "source": "VCPLog"
        }),
    );
}

fn next_reconnect_delay(current: Duration) -> Duration {
    let next_secs = current
        .as_secs()
        .saturating_mul(2)
        .clamp(RECONNECT_INITIAL_DELAY_SECS, RECONNECT_MAX_DELAY_SECS);
    Duration::from_secs(next_secs)
}

fn decrement_outbound_queue_len() {
    if OUTBOUND_QUEUE_LEN.load(Ordering::SeqCst) > 0 {
        OUTBOUND_QUEUE_LEN.fetch_sub(1, Ordering::SeqCst);
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
    let ws_url = if url.trim().is_empty() || key.trim().is_empty() {
        None
    } else {
        Some(parse_log_url(&url, &key)?)
    };

    let next_target = ws_url.as_ref().map(|u| u.as_str().to_string());
    let listener_active = LOG_CONNECTION_ACTIVE.load(Ordering::SeqCst);
    {
        let mut current_target = CURRENT_LOG_TARGET.write().await;
        if *current_target == next_target && listener_active {
            log::debug!("[VCPLog] init skipped because target is unchanged.");
            return Ok(());
        }
        *current_target = next_target;
    }

    // 如果 URL 或 Key 为空，发送 None 以停止现有连接并进入静默等待
    let _ = WS_URL_CHANNEL.0.send(ws_url.clone());
    if ws_url.is_none() {
        emit_log_status(&app, "disconnected", "VCPLog 未配置".to_string()).await;
        return Ok(());
    }

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
    let mut reconnect_delay = Duration::from_secs(RECONNECT_INITIAL_DELAY_SECS);

    // 创建 mpsc 通道用于回传消息
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();

    // 将发送端存储在全局静态变量中供 send_vcp_log_message 使用
    {
        let mut sender_lock = LOG_SENDER.lock().await;
        *sender_lock = Some(tx);
    }

    loop {
        // 获取当前 URL
        let ws_url = {
            let val = url_rx.borrow().clone();
            match val {
                Some(u) => u,
                None => {
                    OUTBOUND_QUEUE_LEN.store(0, Ordering::SeqCst);
                    emit_log_status(&app_handle, "disconnected", "VCPLog 未配置".to_string()).await;
                    if url_rx.changed().await.is_err() {
                        break;
                    }
                    reconnect_delay = Duration::from_secs(RECONNECT_INITIAL_DELAY_SECS);
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
        emit_log_status(&app_handle, "connecting", "正在连接 VCPLog...".to_string()).await;

        let mut request = match ws_url.as_str().into_client_request() {
            Ok(req) => req,
            Err(e) => {
                log::error!(
                    "[VCPLog] Failed to build request: {}. Retrying in 5 seconds...",
                    e
                );
                emit_log_status(&app_handle, "error", format!("VCPLog 请求构建失败: {}", e)).await;

                tokio::select! {
                    _ = url_rx.changed() => {},
                    _ = sleep(reconnect_delay) => {},
                }
                reconnect_delay = next_reconnect_delay(reconnect_delay);
                continue;
            }
        };

        if let Some(host) = ws_url.host_str() {
            let host_with_port = if let Some(port) = ws_url.port() {
                format!("{}:{}", host, port)
            } else {
                host.to_string()
            };
            if let Ok(val) = host_with_port.parse() {
                request.headers_mut().insert("Host", val);
            }

            let origin_scheme = match ws_url.scheme() {
                "wss" => "https",
                _ => "http",
            };
            let origin = if let Some(port) = ws_url.port() {
                format!("{}://{}:{}", origin_scheme, host, port)
            } else {
                format!("{}://{}", origin_scheme, host)
            };
            if let Ok(val) = origin.parse() {
                request.headers_mut().insert("Origin", val);
            }
        }

        request.headers_mut().insert(
            "User-Agent",
            "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36".parse().unwrap()
        );

        match tokio::time::timeout(Duration::from_secs(10), connect_async(request)).await {
            Ok(connection_result) => match connection_result {
                Ok((ws_stream, _)) => {
                    log::info!("[VCPLog] Connected successfully to {}", masked_url);
                    let (mut ws_write, mut ws_read) = ws_stream.split();
                    let mut heartbeat = interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
                    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
                    let mut last_seen_at = Instant::now();

                    reconnect_delay = Duration::from_secs(RECONNECT_INITIAL_DELAY_SECS);
                    emit_log_status(&app_handle, "connected", "已连接 VCPLog".to_string()).await;

                    let end_reason = loop {
                        tokio::select! {
                            // 监听 URL 变更
                            _ = url_rx.changed() => {
                                log::info!("[VCPLog] URL changed, closing current connection.");
                                break ConnectionEnd::UrlChanged;
                            }
                            _ = heartbeat.tick() => {
                                if last_seen_at.elapsed() >= Duration::from_secs(CONNECTION_STALE_TIMEOUT_SECS) {
                                    log::warn!("[VCPLog] Connection stale for {:?}, rebuilding socket.", last_seen_at.elapsed());
                                    break ConnectionEnd::Stale;
                                }

                                if let Err(e) = ws_write.send(Message::Ping(Vec::new().into())).await {
                                    log::error!("[VCPLog] Heartbeat ping failed: {}", e);
                                    break ConnectionEnd::Network;
                                }
                            }
                            // 处理接收到的消息
                            msg_result = ws_read.next() => {
                                match msg_result {
                                    Some(Ok(msg)) => {
                                        last_seen_at = Instant::now();
                                        if msg.is_text() {
                                            let text = msg.to_text().unwrap_or_default();
                                            match serde_json::from_str::<Value>(text) {
                                                Ok(payload) => {
                                                    if let Err(e) = app_handle.emit("vcp-system-event", payload) {
                                                        log::error!("[VCPLog] Failed to emit event to frontend: {}", e);
                                                    }
                                                }
                                                Err(_) => {
                                                    let _ = app_handle.emit("vcp-system-event", serde_json::json!({
                                                        "type": "raw_text",
                                                        "data": text
                                                    }));
                                                }
                                            }
                                        } else if msg.is_ping() {
                                            if let Err(e) = ws_write.send(Message::Pong(msg.into_data())).await {
                                                log::error!("[VCPLog] Failed to reply pong: {}", e);
                                                break ConnectionEnd::Network;
                                            }
                                        } else if msg.is_close() {
                                            log::warn!("[VCPLog] Close frame received from server.");
                                            break ConnectionEnd::Network;
                                        }
                                    }
                                    Some(Err(e)) => {
                                        log::error!("[VCPLog] WebSocket error during read: {}", e);
                                        break ConnectionEnd::Network;
                                    }
                                    None => {
                                        log::warn!("[VCPLog] Connection closed by server.");
                                        break ConnectionEnd::Network;
                                    }
                                }
                            }
                            // 处理待发送的消息
                            payload_opt = rx.recv() => {
                                if let Some(payload) = payload_opt {
                                    if let Ok(text) = serde_json::to_string(&payload) {
                                        if let Err(e) = ws_write.send(Message::Text(text.into())).await {
                                            log::error!("[VCPLog] Failed to send message: {}", e);
                                            decrement_outbound_queue_len();
                                            break ConnectionEnd::Network;
                                        }
                                    }
                                    decrement_outbound_queue_len();
                                } else {
                                    break ConnectionEnd::SenderClosed;
                                }
                            }
                        }
                    };

                    log::info!("[VCPLog] Disconnected from {}: {:?}", ws_url, end_reason);
                    if end_reason == ConnectionEnd::UrlChanged {
                        reconnect_delay = Duration::from_secs(RECONNECT_INITIAL_DELAY_SECS);
                        continue;
                    }

                    emit_log_status(
                        &app_handle,
                        "disconnected",
                        "VCPLog 连接已断开，准备重连".to_string(),
                    )
                    .await;
                }
                Err(e) => {
                    log::error!("[VCPLog] Connection Error: {}. Status: {}", e, e);
                    emit_log_status(&app_handle, "error", format!("VCPLog 连接失败: {}", e)).await;
                }
            },
            Err(_) => {
                log::error!(
                    "[VCPLog] Connection timed out after 10 seconds. Retrying in 5 seconds..."
                );
                emit_log_status(&app_handle, "error", "VCPLog 连接超时".to_string()).await;
            }
        }

        tokio::select! {
            _ = url_rx.changed() => {
                reconnect_delay = Duration::from_secs(RECONNECT_INITIAL_DELAY_SECS);
                log::info!("[VCPLog] URL changed during retry wait.");
            }
            _ = sleep(reconnect_delay) => {
                reconnect_delay = next_reconnect_delay(reconnect_delay);
            },
        }
    }

    LOG_CONNECTION_ACTIVE.store(false, Ordering::SeqCst);
    OUTBOUND_QUEUE_LEN.store(0, Ordering::SeqCst);
    {
        let mut sender_lock = LOG_SENDER.lock().await;
        *sender_lock = None;
    }
}

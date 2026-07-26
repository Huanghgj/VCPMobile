use crate::vcp_modules::media_processor::convert_local_image_for_multimodal;
use dashmap::{DashMap, DashSet};
use futures_util::StreamExt;
use futures_util::TryStreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Error as IoError;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tauri::{ipc::Channel, AppHandle, Emitter, Manager, Runtime};
use tokio::sync::watch;
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::io::StreamReader;
use url::Url;

use crate::vcp_modules::aurora_pipeline::{AuroraBuffer, AuroraUpdate};
use crate::vcp_modules::content_parser::ContentBlock;
use crate::vcp_modules::db_manager::DbState;
use crate::vcp_modules::settings_manager::{create_default_settings, Settings};

/// =================================================================
/// vcp_modules/vcp_client.rs - 统一的 VCP 请求处理模块 (Rust 重写版)
/// =================================================================
/// 该模块对应原项目的 modules/vcpClient.js，负责处理所有与 VCP 服务器的通信。
/// 包含动态路由、上下文注入（音乐、UI 规范）、流式 SSE 解析以及请求中止机制。
static IMAGE_HOST_UPLOAD_CACHE: LazyLock<DashMap<String, String>> = LazyLock::new(DashMap::new);

/// 请求参数结构体
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcpRequestPayload {
    pub vcp_url: String,        // VCP服务器URL
    pub vcp_api_key: String,    // API密钥
    pub messages: Vec<Value>,   // 消息数组
    pub model_config: Value,    // 模型配置 (包含 model, stream, temperature 等)
    pub message_id: String,     // 消息ID (用于跟踪和中止)
    pub context: Option<Value>, // 上下文信息 (agentId, topicId等)
}

/// 流式事件结构体，用于向前端发送数据
#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamEvent {
    pub r#type: String, // 事件类型: "data", "aurora", "end", "error", "reconnecting"
    pub chunk: Option<Value>, // 数据块 (仅 type="data" 时有效)
    pub message_id: String, // 消息ID
    pub context: Option<Value>, // 透传的上下文信息
    pub finish_reason: Option<String>, // 结束原因
    pub error: Option<String>, // 错误信息 (仅 type="error" 时有效)
    pub aurora: Option<AuroraUpdate>, // Aurora 语义沉淀更新 (type="aurora" 时有效)
    pub blocks: Option<Vec<ContentBlock>>, // 持久化后的预渲染块 (仅 type="end" 时有效)
    pub content: Option<String>, // 修复后的最终全文 (仅 type="end" 时有效)
    pub timestamp: Option<u64>, // ⚡ 新增物理落笔时间戳
}

// 将 data URL 拆成模型 API 需要的裸 base64 与格式字段。
fn split_audio_data_url(audio_url: &str) -> (String, &'static str) {
    if let Some((meta, data)) = audio_url.split_once(',') {
        let format = if meta.contains("audio/aac") {
            "aac"
        } else if meta.contains("audio/wav") {
            "wav"
        } else if meta.contains("audio/ogg") {
            "ogg"
        } else if meta.contains("audio/flac") {
            "flac"
        } else if meta.contains("audio/mp4") || meta.contains("audio/m4a") {
            "mp4"
        } else if meta.contains("audio/opus") {
            "opus"
        } else {
            "mp3"
        };
        return (data.to_string(), format);
    }

    (audio_url.to_string(), "mp3")
}

#[derive(Clone)]
struct ImageHostConfig {
    base_url: String,
    image_key: String,
    upload_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageUploadResponse {
    success: Option<bool>,
    url: Option<String>,
    key: Option<String>,
    error: Option<String>,
}

fn setting_string<'a>(settings: &'a Settings, key: &str) -> Option<&'a str> {
    settings
        .extra
        .as_object()
        .and_then(|extra| extra.get(key))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn origin_from_url(url_str: &str) -> Option<String> {
    let url = Url::parse(url_str).ok()?;
    let origin = url.origin().ascii_serialization();
    if origin == "null" {
        None
    } else {
        Some(origin.trim_end_matches('/').to_string())
    }
}

impl ImageHostConfig {
    fn from_settings(vcp_url: &str, settings: &Settings) -> Option<Self> {
        let image_key = setting_string(settings, "imageKey")
            .unwrap_or(settings.file_key.trim())
            .trim();
        if image_key.is_empty() {
            return None;
        }

        let base_url = setting_string(settings, "imageServerUrl")
            .or_else(|| setting_string(settings, "imagePublicBaseUrl"))
            .and_then(origin_from_url)
            .or_else(|| origin_from_url(vcp_url))?;

        Some(Self {
            base_url,
            image_key: image_key.to_string(),
            upload_key: setting_string(settings, "imageUploadKey")
                .unwrap_or(image_key)
                .to_string(),
        })
    }

    fn upload_url(&self) -> String {
        format!(
            "{}/pw={}/images/upload",
            self.base_url,
            urlencoding::encode(&self.upload_key)
        )
    }

    fn public_url_for_key(&self, key: &str) -> String {
        let encoded_key = key
            .split('/')
            .map(urlencoding::encode)
            .map(|part| part.into_owned())
            .collect::<Vec<_>>()
            .join("/");
        format!(
            "{}/pw={}/images/{}",
            self.base_url,
            urlencoding::encode(&self.image_key),
            encoded_key
        )
    }

    fn cache_scope(&self) -> String {
        format!("{}|{}", self.base_url, self.image_key)
    }
}

fn normalize_image_upload_location(config: &ImageHostConfig, value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if let Ok(parsed) = Url::parse(value) {
        if matches!(parsed.scheme(), "http" | "https") {
            return Some(value.to_string());
        }
    }

    if value.starts_with('/') {
        return Some(format!(
            "{}{}",
            config.base_url.trim_end_matches('/'),
            value
        ));
    }

    Some(config.public_url_for_key(value))
}

fn image_extension_for_mime(mime: &str) -> &'static str {
    match mime.to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/bmp" => "bmp",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "video/x-matroska" => "mkv",
        "video/x-msvideo" => "avi",
        "video/x-ms-wmv" => "wmv",
        "video/x-flv" => "flv",
        "video/3gpp" => "3gp",
        "video/3gpp2" => "3g2",
        "video/mp2t" => "ts",
        _ => "png",
    }
}

fn safe_upload_file_name(file_name: &str, mime: &str) -> String {
    let raw_name = std::path::Path::new(file_name)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("upload");

    if std::path::Path::new(raw_name).extension().is_some() {
        raw_name.to_string()
    } else {
        format!("{}.{}", raw_name, image_extension_for_mime(mime))
    }
}

fn image_mime_for_path(path: &std::path::Path, declared_mime: &str) -> String {
    if declared_mime.starts_with("image/") {
        return declared_mime.to_string();
    }
    let guessed = mime_guess::from_path(path).first_or_octet_stream();
    if guessed.type_().as_str() == "image" {
        guessed.to_string()
    } else {
        "image/png".to_string()
    }
}

fn trim_upload_error_body(body: &str) -> String {
    let compact = body.trim().replace('\n', " ");
    let mut chars = compact.chars();
    let clipped: String = chars.by_ref().take(240).collect();
    if chars.next().is_some() {
        format!("{}...", clipped)
    } else {
        compact
    }
}

fn mask_image_url_for_log(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut parsed) => {
            if let Some(segments) = parsed.path_segments() {
                let masked_segments = segments
                    .map(|segment| {
                        if segment.starts_with("pw=") {
                            "pw=***".to_string()
                        } else {
                            segment.to_string()
                        }
                    })
                    .collect::<Vec<_>>();
                parsed.set_path(&masked_segments.join("/"));
            }
            parsed.to_string()
        }
        Err(_) => url.replace("/pw=", "/pw=***"),
    }
}

async fn post_image_to_host(
    client: &Client,
    config: &ImageHostConfig,
    bytes: Vec<u8>,
    mime: &str,
    file_name: &str,
    trace_id: &str,
) -> Result<String, String> {
    if !mime.starts_with("image/") && !mime.starts_with("video/") {
        return Err(format!("unsupported media MIME: {}", mime));
    }

    let safe_name = safe_upload_file_name(file_name, mime);
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(safe_name)
        .mime_str(mime)
        .map_err(|e| format!("invalid upload MIME {}: {}", mime, e))?;
    let form = reqwest::multipart::Form::new()
        .part("image", part)
        .text("traceId", trace_id.to_string())
        .text("prefix", "vcp-mobile".to_string());

    let response = client
        .post(config.upload_url())
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("image host request failed: {}", e))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("image host response read failed: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "image host returned HTTP {}: {}",
            status.as_u16(),
            trim_upload_error_body(&body)
        ));
    }

    let upload: ImageUploadResponse = serde_json::from_str(&body).map_err(|e| {
        format!(
            "image host JSON parse failed: {} ({})",
            e,
            trim_upload_error_body(&body)
        )
    })?;
    if upload.success == Some(false) {
        return Err(upload
            .error
            .unwrap_or_else(|| "image host reported upload failure".to_string()));
    }

    upload
        .url
        .as_deref()
        .and_then(|url| normalize_image_upload_location(config, url))
        .or_else(|| {
            upload
                .key
                .as_deref()
                .and_then(|key| normalize_image_upload_location(config, key))
        })
        .ok_or_else(|| "image host upload response did not include url/key".to_string())
}

async fn upload_image_path_to_host(
    client: &Client,
    config: &ImageHostConfig,
    path: &std::path::Path,
    declared_mime: &str,
    file_name: &str,
    trace_id: &str,
) -> Result<String, String> {
    let cache_key = format!("{}|path|{}", config.cache_scope(), path.display());
    if let Some(cached) = IMAGE_HOST_UPLOAD_CACHE.get(&cache_key) {
        return Ok(cached.value().clone());
    }

    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|e| format!("image metadata read failed: {}", e))?;
    const IMAGE_HOST_MAX_BYTES: u64 = 50 * 1024 * 1024;
    if metadata.len() > IMAGE_HOST_MAX_BYTES {
        return Err(format!(
            "image file is too large for ImageServer upload: {} bytes",
            metadata.len()
        ));
    }

    let mime = image_mime_for_path(path, declared_mime);
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("image file read failed: {}", e))?;
    let url = post_image_to_host(client, config, bytes, &mime, file_name, trace_id).await?;
    IMAGE_HOST_UPLOAD_CACHE.insert(cache_key, url.clone());
    Ok(url)
}

impl StreamEvent {
    pub fn data(message_id: String, chunk: Value, context: Option<Value>) -> Self {
        Self {
            r#type: "data".into(),
            chunk: Some(chunk),
            message_id,
            context,
            ..Default::default()
        }
    }

    pub fn thinking(message_id: String, context: Option<Value>) -> Self {
        Self {
            r#type: "thinking".into(),
            message_id,
            context,
            ..Default::default()
        }
    }

    pub fn aurora(message_id: String, aurora: AuroraUpdate, context: Option<Value>) -> Self {
        Self {
            r#type: "aurora".into(),
            aurora: Some(aurora),
            message_id,
            context,
            ..Default::default()
        }
    }

    pub fn end(
        message_id: String,
        context: Option<Value>,
        finish_reason: Option<String>,
        blocks: Option<Vec<ContentBlock>>,
        timestamp: Option<u64>,
    ) -> Self {
        Self {
            r#type: "end".into(),
            message_id,
            context,
            finish_reason,
            blocks,
            timestamp,
            ..Default::default()
        }
    }

    pub fn error(message_id: String, context: Option<Value>, error: String) -> Self {
        Self {
            r#type: "error".into(),
            message_id,
            context,
            finish_reason: Some("error".to_string()),
            error: Some(error),
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug)]
struct RemoteInterruptContext {
    interrupt_url: String,
    api_key: String,
    request_id: String,
}

fn interrupt_url_for_request(url_str: &str) -> Option<String> {
    let mut url = Url::parse(url_str).ok()?;
    url.set_path("/v1/interrupt");
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

async fn post_remote_interrupt(ctx: RemoteInterruptContext) {
    let masked_url = mask_image_url_for_log(&ctx.interrupt_url);
    let client = match Client::builder()
        .timeout(Duration::from_millis(1500))
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            log::warn!(
                "[VCPClient] Failed to create remote interrupt client for {}: {}",
                ctx.request_id,
                e
            );
            return;
        }
    };

    match client
        .post(&ctx.interrupt_url)
        .header(AUTHORIZATION, format!("Bearer {}", ctx.api_key))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "requestId": ctx.request_id,
            "messageId": ctx.request_id
        }))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            log::info!(
                "[VCPClient] Remote VCP interrupt accepted for {} via {}",
                ctx.request_id,
                masked_url
            );
        }
        Ok(resp) => {
            log::warn!(
                "[VCPClient] Remote VCP interrupt returned {} for {} via {}",
                resp.status(),
                ctx.request_id,
                masked_url
            );
        }
        Err(e) => {
            log::warn!(
                "[VCPClient] Remote VCP interrupt failed for {} via {}: {}",
                ctx.request_id,
                masked_url,
                e
            );
        }
    }
}

/// 单个请求的取消状态。句柄可以在上下文和附件准备前登记，随后由网络层绑定远端中止信息。
pub struct ActiveRequestControl {
    cancelled: AtomicBool,
    remote_interrupt_sent: AtomicBool,
    cancel_tx: watch::Sender<bool>,
    remote_context: Mutex<Option<RemoteInterruptContext>>,
}

impl ActiveRequestControl {
    fn new() -> Self {
        let (cancel_tx, _) = watch::channel(false);
        Self {
            cancelled: AtomicBool::new(false),
            remote_interrupt_sent: AtomicBool::new(false),
            cancel_tx,
            remote_context: Mutex::new(None),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.cancel_tx.subscribe()
    }

    /// 返回 true 表示本次调用首次把请求切换为已取消。
    pub fn cancel(&self) -> bool {
        let first = !self.cancelled.swap(true, Ordering::AcqRel);
        self.cancel_tx.send_replace(true);
        self.try_spawn_remote_interrupt();
        first
    }

    pub fn bind_remote_interrupt(&self, message_id: &str, vcp_url: &str, api_key: &str) {
        let Some(interrupt_url) = interrupt_url_for_request(vcp_url) else {
            log::warn!(
                "[VCPClient] Failed to build remote interrupt URL for messageId: {}",
                message_id
            );
            return;
        };
        if let Ok(mut remote_context) = self.remote_context.lock() {
            *remote_context = Some(RemoteInterruptContext {
                interrupt_url,
                api_key: api_key.to_string(),
                request_id: message_id.to_string(),
            });
        }
        if self.is_cancelled() {
            self.try_spawn_remote_interrupt();
        }
    }

    fn try_spawn_remote_interrupt(&self) {
        let context = self
            .remote_context
            .lock()
            .ok()
            .and_then(|context| context.clone());
        let Some(context) = context else {
            return;
        };
        if self
            .remote_interrupt_sent
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            tauri::async_runtime::spawn(post_remote_interrupt(context));
        }
    }
}

async fn wait_for_cancellation(receiver: &mut watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    while receiver.changed().await.is_ok() {
        if *receiver.borrow() {
            return;
        }
    }
}

pub fn schedule_interrupt_request_with_retry(
    active_requests: Arc<DashMap<String, Arc<ActiveRequestControl>>>,
    message_id: String,
    reason: &'static str,
) {
    tauri::async_runtime::spawn(async move {
        for attempt in 0..20 {
            if let Some(control) = active_requests.get(&message_id).map(|entry| entry.clone()) {
                log::info!(
                    "[VCPClient] Interrupting request {} after {} attempt(s), reason={}",
                    message_id,
                    attempt + 1,
                    reason
                );
                control.cancel();
                return;
            }

            if attempt < 19 {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        log::warn!(
            "[VCPClient] Request {} was not found after retrying interrupt, reason={}",
            message_id,
            reason
        );
    });
}

/// 全局活跃请求管理器。请求准备阶段和网络阶段共享同一个幂等取消句柄。
pub struct ActiveRequests(pub Arc<DashMap<String, Arc<ActiveRequestControl>>>);

impl ActiveRequests {
    pub fn register(&self, message_id: &str) -> Arc<ActiveRequestControl> {
        let control = Arc::new(ActiveRequestControl::new());
        self.0.insert(message_id.to_string(), control.clone());
        control
    }
}

impl Default for ActiveRequests {
    fn default() -> Self {
        log::info!("[VCPClient] Initialized ActiveRequests successfully.");
        Self(Arc::new(DashMap::new()))
    }
}

/// RAII guard：在 Drop 时自动从 ActiveRequests 中移除对应条目，防止 panic 导致泄漏
pub struct ActiveRequestGuard {
    requests: Arc<DashMap<String, Arc<ActiveRequestControl>>>,
    message_id: String,
    control: Arc<ActiveRequestControl>,
}

impl ActiveRequestGuard {
    pub fn new(
        requests: Arc<DashMap<String, Arc<ActiveRequestControl>>>,
        message_id: String,
        control: Arc<ActiveRequestControl>,
    ) -> Self {
        Self {
            requests,
            message_id,
            control,
        }
    }
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        let should_remove = self
            .requests
            .get(&self.message_id)
            .is_some_and(|current| Arc::ptr_eq(current.value(), &self.control));
        if should_remove {
            self.requests.remove(&self.message_id);
        }
    }
}

/// 群组回合取消令牌，用于标记需要中断接力赛的话题
/// topicId -> true (存在即代表已取消)
pub struct CancelledGroupTurns(pub Arc<DashSet<String>>);

impl Default for CancelledGroupTurns {
    fn default() -> Self {
        log::info!("[VCPClient] Initialized CancelledGroupTurns successfully.");
        Self(Arc::new(DashSet::new()))
    }
}

/// 中止群组的整个接力赛回合
#[tauri::command]
#[allow(non_snake_case)]
pub fn interruptGroupTurn(
    state: tauri::State<'_, CancelledGroupTurns>,
    topic_id: String,
    turn_id: Option<String>,
) -> Result<Value, String> {
    let cancellation_key = turn_id.unwrap_or(topic_id);
    log::info!(
        "[VCPClient] interruptGroupTurn called for key: {}",
        cancellation_key
    );
    state.0.insert(cancellation_key.clone());
    Ok(json!({"status": "cancelled", "turnId": cancellation_key}))
}

/// 核心请求函数：sendToVCP
/// 对应 JS 版的 sendToVCP。处理逻辑：
/// 1. 数据验证与规范化 (通过 Rust 类型系统自动处理部分)
/// 2. 动态路由切换 (根据设置注入 /v1/chatvcp/completions)
/// 3. 上下文注入 (音乐信息、UI 规范要求)
/// 4. 发起 HTTP 请求 (支持流式和非流式)
/// 5. 注册 AbortController 实现中止机制
#[tauri::command]
#[allow(non_snake_case)]
pub async fn sendToVCP<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, ActiveRequests>,
    payload: VcpRequestPayload,
    stream_channel: Channel<StreamEvent>,
) -> Result<Value, String> {
    let message_id = payload.message_id.clone();
    let context = payload.context.clone();
    let request_control = state.register(&message_id);
    let _request_guard =
        ActiveRequestGuard::new(state.0.clone(), message_id.clone(), request_control.clone());

    let (res, is_aborted) =
        perform_vcp_request(&app, request_control, payload, Some(stream_channel.clone())).await?;

    if let Some(full_content) = res["fullContent"].as_str() {
        let finish_reason = if is_aborted {
            Some("cancelled_by_user".to_string())
        } else {
            res["finishReason"].as_str().map(|s| s.to_string())
        };

        // 从 context 解出 owner_id, owner_type, topic_id 并委派统一终结器
        let ctx = context.as_ref();
        let group_id = ctx.and_then(|c| c["groupId"].as_str());
        let agent_id = ctx.and_then(|c| c["agentId"].as_str());
        let agent_name = ctx.and_then(|c| c["agentName"].as_str());
        let topic_id = ctx
            .and_then(|c| c["topicId"].as_str())
            .unwrap_or("")
            .to_string();

        let (owner_id, owner_type) = if let Some(gid) = group_id {
            (gid, "group")
        } else if let Some(aid) = agent_id {
            (aid, "agent")
        } else {
            ("", "agent")
        };

        let pool = app
            .state::<crate::vcp_modules::db_manager::DbState>()
            .pool
            .clone();

        crate::vcp_modules::chat::message_service::finalize_stream_message(
            app.clone(),
            &pool,
            owner_id,
            owner_type,
            topic_id,
            message_id,
            full_content.to_string(),
            is_aborted,
            finish_reason,
            agent_id,
            agent_name,
            Some(stream_channel),
            agent_id.map(|s| s.to_string()),
        )
        .await?;
    }

    Ok(res)
}

fn extract_text_for_hash(content: &Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        let text_parts: Vec<String> = arr
            .iter()
            .filter(|part| matches!(part["type"].as_str(), Some("text") | Some("output_text")))
            .filter_map(|part| part["text"].as_str())
            .map(|s| s.to_string())
            .collect();
        return text_parts.join("\n");
    }
    if let Some(obj) = content.as_object() {
        if let Some(s) = obj.get("text").and_then(|t| t.as_str()) {
            return s.to_string();
        }
    }
    String::new()
}

fn extract_non_empty_text(value: Option<&Value>) -> Option<String> {
    let text = value.map(extract_text_for_hash)?;
    (!text.trim().is_empty()).then_some(text)
}

fn normalize_non_stream_completion(response: Value) -> Result<Value, String> {
    let (full_content, finish_reason) = {
        let choice = response
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .ok_or_else(|| {
                let detail = response
                    .get("error")
                    .map(Value::to_string)
                    .unwrap_or_else(|| "missing choices[0]".to_string());
                format!("VCP non-stream response did not contain a completion choice: {detail}")
            })?;
        let message = choice.get("message");
        let content = extract_non_empty_text(message.and_then(|value| value.get("content")))
            .or_else(|| extract_non_empty_text(message.and_then(|value| value.get("refusal"))))
            .or_else(|| {
                extract_non_empty_text(choice.get("delta").and_then(|value| value.get("content")))
            })
            .or_else(|| extract_non_empty_text(choice.get("text")))
            .unwrap_or_default();
        let reasoning = ["reasoning_content", "reasoningContent", "reasoning"]
            .into_iter()
            .find_map(|key| extract_non_empty_text(message.and_then(|value| value.get(key))))
            .or_else(|| {
                ["reasoning_content", "reasoningContent", "reasoning"]
                    .into_iter()
                    .find_map(|key| {
                        extract_non_empty_text(choice.get("delta").and_then(|value| value.get(key)))
                    })
            })
            .unwrap_or_default();

        let full_content = if reasoning.is_empty() {
            content
        } else if content.is_empty() {
            format!("<think>{reasoning}</think>")
        } else {
            format!("<think>{reasoning}</think>{content}")
        };
        let finish_reason = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(|reason| {
                if reason == "stop" {
                    "completed".to_string()
                } else {
                    reason.to_string()
                }
            });
        (full_content, finish_reason)
    };

    if full_content.trim().is_empty() {
        return Err(
            "VCP non-stream response contained no assistant content or reasoning".to_string(),
        );
    }

    Ok(json!({
        "response": response,
        "fullContent": full_content,
        "streamingStarted": false,
        "finishReason": finish_reason
    }))
}

fn non_stream_content_event(
    normalized: &Value,
    message_id: String,
    context: Option<Value>,
) -> Result<StreamEvent, String> {
    let content = normalized
        .get("fullContent")
        .and_then(Value::as_str)
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| "VCP non-stream response normalized to empty content".to_string())?;

    Ok(StreamEvent::data(
        message_id,
        Value::String(content.to_string()),
        context,
    ))
}

fn get_or_calculate_message_hash(content: &Value) -> String {
    use crate::vcp_modules::infra::utils::calculate_sha256;

    let text = extract_text_for_hash(content);
    let hash = calculate_sha256(text.as_bytes());
    format!("sha256:{}", hash)
}

/// 核心请求实现函数，可供 Tauri Command 或 内部 Rust 模块(如 GroupOrchestrator) 调用
/// 返回 Result<(全量内容/响应体, 是否被中止), 错误信息>
pub async fn perform_vcp_request<R: Runtime>(
    app: &AppHandle<R>,
    request_control: Arc<ActiveRequestControl>,
    payload: VcpRequestPayload,
    stream_channel: Option<Channel<StreamEvent>>,
) -> Result<(Value, bool), String> {
    log::info!(
        "[VCPClient] perform_vcp_request called for messageId: {}, context: {:?}",
        payload.message_id,
        payload.context
    );

    let mut abort_rx = request_control.subscribe();
    let cancelled_result = || {
        (
            json!({
                "fullContent": "",
                "streamingStarted": false,
                "finishReason": "cancelled_by_user"
            }),
            true,
        )
    };
    if request_control.is_cancelled() {
        return Ok(cancelled_result());
    }

    let send_stream_event = |event: StreamEvent| {
        if let Some(ref ch) = stream_channel {
            let _ = ch.send(event);
        }
    };

    let app_settings = load_app_settings(app).await.unwrap_or_else(|e| {
        log::warn!("[VCPClient] Failed to load app settings: {}", e);
        create_default_settings()
    });
    let image_host_config = ImageHostConfig::from_settings(&payload.vcp_url, &app_settings);
    let image_host_client = if image_host_config.is_some() {
        Some(
            Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let request_message_id = payload.message_id.clone();
    let latest_image_user_message_index = payload.messages.iter().rposition(|message| {
        message.get("role").and_then(|role| role.as_str()) == Some("user")
            && message
                .get("content")
                .and_then(Value::as_array)
                .is_some_and(|parts| {
                    parts.iter().any(|part| {
                        part.get("type").and_then(Value::as_str) == Some("local_file")
                            && part
                                .get("mime")
                                .and_then(Value::as_str)
                                .is_some_and(|mime| mime.starts_with("image/"))
                    })
                })
    });

    // === 0. 数据验证和规范化 ===
    let mut messages: Vec<Value> = Vec::new();
    for (msg_index, msg_val) in payload.messages.into_iter().enumerate() {
        if request_control.is_cancelled() {
            return Ok(cancelled_result());
        }
        if !msg_val.is_object() {
            messages.push(json!({"role": "system", "content": "[Invalid message]"}));
            continue;
        }

        let mut msg = msg_val.clone();
        let content = msg.get("content").cloned().unwrap_or(Value::Null);
        let is_latest_user_message = latest_image_user_message_index == Some(msg_index);

        // 处理多模态或复杂内容数组
        if let Some(content_array) = content.as_array() {
            let mut new_parts = Vec::new();
            for part in content_array {
                if request_control.is_cancelled() {
                    return Ok(cancelled_result());
                }
                if let Some(obj) = part.as_object() {
                    // 识别自定义的 local_file 类型并进行路径还原与编码
                    if obj.get("type").and_then(|t| t.as_str()) == Some("local_file") {
                        if let Some(path_str) = obj.get("path").and_then(|p| p.as_str()) {
                            let clean_path = path_str.replace("file://", "");
                            let display_name = obj
                                .get("name")
                                .and_then(|n| n.as_str())
                                .filter(|n| !n.is_empty())
                                .or_else(|| {
                                    std::path::Path::new(&clean_path)
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .filter(|n| !n.is_empty())
                                })
                                .unwrap_or("附件文件");
                            let path_buf = std::path::PathBuf::from(&clean_path);
                            let declared_is_image = obj
                                .get("mime")
                                .and_then(|mime| mime.as_str())
                                .is_some_and(|mime| mime.starts_with("image/"));

                            // 历史图片不在每一轮重复转码和重传。否则旧附件读取失败时会
                            // 降级成文件名文本，干扰当前轮多模态判断，并显著放大请求体。
                            if declared_is_image && !is_latest_user_message {
                                log::info!(
                                    "[VCPClient] Skipping historical image attachment in request replay"
                                );
                                continue;
                            }

                            let mut converted = false;
                            let mut fallback_text = format!("[附件文件: {}]", display_name);
                            if path_buf.exists() {
                                // 优先使用附件注册时记录的 MIME，后备才看扩展名。
                                // Android 相册/文件选择器有时给的是无后缀临时文件名，仅靠扩展名会误判。
                                let declared_mime = obj
                                    .get("mime")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let ext = path_buf
                                    .extension()
                                    .and_then(|e| e.to_str())
                                    .unwrap_or("")
                                    .to_lowercase();
                                let media_kind = if declared_mime.starts_with("image/") {
                                    "image"
                                } else if declared_mime.starts_with("audio/") {
                                    "audio"
                                } else if declared_mime.starts_with("video/") {
                                    "video"
                                } else {
                                    match ext.as_str() {
                                        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
                                        | "tiff" | "tif" | "heic" | "heif" | "avif" | "ico" => {
                                            "image"
                                        }
                                        "mp3" | "wav" | "ogg" | "flac" | "aac" | "m4a" | "opus"
                                        | "wma" | "amr" | "aiff" | "aif" => "audio",
                                        "mp4" | "mkv" | "webm" | "avi" | "mov" | "flv" | "m4v"
                                        | "3gp" | "3g2" | "wmv" | "ts" | "mts" | "m2ts" | "qt" => {
                                            "video"
                                        }
                                        _ => "application",
                                    }
                                };

                                if media_kind == "image" {
                                    if let (Some(config), Some(client)) =
                                        (image_host_config.as_ref(), image_host_client.as_ref())
                                    {
                                        match upload_image_path_to_host(
                                            client,
                                            config,
                                            &path_buf,
                                            &declared_mime,
                                            display_name,
                                            &format!(
                                                "{}-m{}-{}",
                                                request_message_id,
                                                msg_index,
                                                new_parts.len()
                                            ),
                                        )
                                        .await
                                        {
                                            Ok(url) => {
                                                log::info!(
                                                    "[VCPClient] Image uploaded to ImageServer for multimodal payload: {}",
                                                    mask_image_url_for_log(&url)
                                                );
                                                // 图床结果不写入模型文本，避免旧版服务端丢弃
                                                // image_url 后只剩文件名/URL 占位，让模型误判已收到图片。
                                            }
                                            Err(e) => {
                                                log::warn!(
                                                    "[VCPClient] ImageServer upload failed for {:?}: {}. Falling back to base64 multimodal payload.",
                                                    path_buf,
                                                    e
                                                );
                                            }
                                        }
                                    }

                                    // 模型多模态输入保持使用 base64 data URL，避免部分渠道拒绝外部 HTTP image_url。
                                    // 图床 URL 只作为文本上下文提供给 ComfyUI/工具调用使用。
                                    if !converted {
                                        let path_buf_clone = path_buf.clone();
                                        let app_clone = app.clone();
                                        let conversion_task =
                                            tokio::task::spawn_blocking(move || {
                                                convert_local_image_for_multimodal(
                                                    &app_clone,
                                                    &path_buf_clone,
                                                )
                                            });
                                        let conversion_result = tokio::select! {
                                            _ = wait_for_cancellation(&mut abort_rx) => {
                                                return Ok(cancelled_result());
                                            }
                                            result = conversion_task => result
                                        };
                                        match conversion_result {
                                            Ok(Ok(data_url)) => {
                                                let payload_chars = data_url.len();
                                                log::info!(
                                                    "[VCPClient] Base64 multimodal image ready: name={}, payload_chars={}",
                                                    display_name,
                                                    payload_chars
                                                );
                                                let _ = app.emit(
                                                    "vcp-multimodal-status",
                                                    json!({
                                                        "status": "base64_ready",
                                                        "payloadChars": payload_chars
                                                    }),
                                                );
                                                new_parts.push(json!({
                                                    "type": "image_url",
                                                    "image_url": { "url": data_url, "detail": "high" }
                                                }));
                                                converted = true;
                                            }
                                            Ok(Err(e)) => {
                                                log::warn!(
                                                    "[VCPClient] Image conversion failed for {:?}: {}",
                                                    path_buf,
                                                    e
                                                );
                                                if is_latest_user_message {
                                                    return Err(format!(
                                                        "图片附件“{}”转换为 Base64 失败，消息未发送：{}",
                                                        display_name, e
                                                    ));
                                                }
                                            }
                                            Err(e) => {
                                                log::warn!(
                                                    "[VCPClient] Image conversion task panicked: {}",
                                                    e
                                                );
                                                if is_latest_user_message {
                                                    return Err(format!(
                                                        "图片附件“{}”转换任务失败，消息未发送：{}",
                                                        display_name, e
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                } else if media_kind == "video" {
                                    let path_buf_clone = path_buf.clone();
                                    let declared_mime_clone = declared_mime.clone();
                                    match tokio::task::spawn_blocking(move || {
                                        crate::vcp_modules::media_processor::convert_local_video_for_multimodal(
                                            &path_buf_clone,
                                            &declared_mime_clone,
                                        )
                                    })
                                    .await
                                    {
                                        Ok(Ok(video_url)) => {
                                            new_parts.push(json!({
                                                "type": "image_url",
                                                "image_url": { "url": video_url }
                                            }));
                                            converted = true;
                                        }
                                        Ok(Err(e)) => {
                                            log::warn!("[VCPClient] Video conversion failed for {:?}: {}", path_buf, e);
                                            fallback_text = format!(
                                                "[视频附件发送失败: {}] (原因: {})",
                                                display_name, e
                                            );
                                        }
                                        Err(e) => {
                                            log::warn!("[VCPClient] Video conversion task panicked: {}", e);
                                            fallback_text = format!(
                                                "[视频附件发送失败: {}] (原因: {})",
                                                display_name, e
                                            );
                                        }
                                    }
                                } else if media_kind == "audio" {
                                    // 音频：原生转码为 AAC，失败时仅允许小文件安全兜底 -> input_audio
                                    let path_clone = path_buf.clone();
                                    let app_clone = app.clone();
                                    match tokio::task::spawn_blocking(move || {
                                        crate::vcp_modules::media_processor::process_audio_for_multimodal(&app_clone, &path_clone)
                                    }).await {
                                        Ok(Ok(audio_url)) => {
                                            let (audio_data, format_str) = split_audio_data_url(&audio_url);
                                            new_parts.push(json!({
                                                "type": "input_audio",
                                                "input_audio": {
                                                    "data": audio_data,
                                                    "format": format_str
                                                }
                                            }));
                                            converted = true;
                                        }
                                        Ok(Err(e)) => {
                                            log::warn!("[VCPClient] Audio extraction failed for {:?}: {}", path_buf, e);
                                        }
                                        Err(e) => {
                                            log::warn!("[VCPClient] Audio processing task panicked: {}", e);
                                        }
                                    }
                                }
                            }

                            // 修复：若文件不存在或读取失败，至少保留文本描述，避免内容静默丢失
                            if !converted {
                                if declared_is_image && is_latest_user_message {
                                    return Err(format!(
                                        "图片附件“{}”未能转换为 Base64，消息未发送；请重新选择图片后重试",
                                        display_name
                                    ));
                                }
                                new_parts.push(json!({
                                    "type": "text",
                                    "text": fallback_text
                                }));
                            }
                        }
                    } else {
                        new_parts.push(part.clone());
                    }
                } else {
                    new_parts.push(part.clone());
                }
            }
            msg["content"] = json!(new_parts);
        } else if content.is_object() {
            if let Some(text) = content.get("text") {
                msg["content"] = text.clone();
            } else {
                msg["content"] = json!(content.to_string());
            }
        } else if !content.is_string() && !content.is_null() {
            msg["content"] = json!(content.to_string());
        }

        messages.push(msg);
    }

    // === 1. 读取设置与动态路由切换 ===
    let mut enable_vcp_tool_injection = false;

    if let Some(extra) = app_settings.extra.as_object() {
        enable_vcp_tool_injection = extra
            .get("enableVcpToolInjection")
            .and_then(|v: &Value| v.as_bool())
            .unwrap_or(false);
    }

    let inline_image_stats = messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_array))
        .flatten()
        .filter_map(|part| {
            let url = part
                .get("image_url")
                .and_then(|image| image.get("url"))
                .and_then(Value::as_str)?;
            url.starts_with("data:image/").then_some(url.len())
        })
        .fold((0usize, 0usize), |(count, chars), len| {
            (count + 1, chars + len)
        });
    let has_inline_images = inline_image_stats.0 > 0;
    if has_inline_images {
        log::info!(
            "[VCPClient] Final request contains {} inline Base64 image(s), total_chars={}",
            inline_image_stats.0,
            inline_image_stats.1
        );
    }

    let mut final_url = payload.vcp_url.clone();
    if enable_vcp_tool_injection && !has_inline_images {
        if let Ok(mut url) = Url::parse(&final_url) {
            url.set_path("/v1/chatvcp/completions");
            final_url = url.to_string();
        }
    } else {
        if enable_vcp_tool_injection && has_inline_images {
            log::info!(
                "[VCPClient] Inline image detected; using standard chat completions route to preserve multimodal parts"
            );
        }
        final_url = normalize_vcp_url(&final_url);
    }

    // === 2. 上下文注入 ===
    let has_system = messages.iter().any(|m| m["role"] == "system");
    if !has_system {
        messages.insert(0, json!({"role": "system", "content": ""}));
    }

    // === 4. 准备请求体 ===
    let is_stream = payload.model_config["stream"].as_bool().unwrap_or(false);
    let mut message_timestamp_bindings = Vec::new();
    for (index, msg) in messages.iter_mut().enumerate() {
        let mut timestamp_meta = None;
        if let Some(obj) = msg.as_object_mut() {
            if let Some(meta) = obj.remove("__vcpchatTimestampMeta") {
                timestamp_meta = Some(meta);
            }
        }
        if let Some(meta) = timestamp_meta {
            if let (Some(message_id), Some(role), Some(timestamp)) = (
                meta.get("messageId").and_then(|id| id.as_str()),
                meta.get("role").and_then(|r| r.as_str()),
                meta.get("timestamp").and_then(|t| t.as_u64()),
            ) {
                use chrono::TimeZone;
                let timestamp_iso =
                    if let Some(dt) = chrono::Utc.timestamp_millis_opt(timestamp as i64).single() {
                        dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                    } else {
                        "".to_string()
                    };

                let final_content_val = msg.get("content").unwrap_or(&Value::Null);
                let sent_message_hash = get_or_calculate_message_hash(final_content_val);

                message_timestamp_bindings.push(json!({
                    "messageId": message_id,
                    "role": role,
                    "timestamp": timestamp,
                    "timestampIso": timestamp_iso,
                    "source": "client_history",
                    "sentMessageHash": sent_message_hash,
                    "sentMessageIndex": index
                }));
            }
        }
    }

    let mut request_body = payload.model_config.clone();
    if let Some(obj) = request_body.as_object_mut() {
        obj.insert("messages".to_string(), json!(messages));
        obj.insert("requestId".to_string(), json!(payload.message_id));
        obj.insert("stream".to_string(), json!(is_stream));
        if !message_timestamp_bindings.is_empty() {
            obj.insert(
                "vcpchatExtensions".to_string(),
                json!({
                    "schemaVersion": 1,
                    "messageMetadataMode": "hash_only",
                    "messageTimestampBindings": message_timestamp_bindings
                }),
            );
        }
    }

    // === 5. 配置网络请求 ===
    let client = Client::builder()
        // 不设 read_timeout：数小时自循环中，任何 read_timeout 都是定时炸弹
        // tcp_keepalive(20s) 维持 TCP 层活性，防止 NAT/防火墙静默丢弃空闲连接
        .tcp_keepalive(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    if request_control.is_cancelled() {
        return Ok(cancelled_result());
    }
    request_control.bind_remote_interrupt(
        &payload.message_id,
        &payload.vcp_url,
        &payload.vcp_api_key,
    );

    let message_id = payload.message_id.clone();
    let context = payload.context.clone();
    let api_key = payload.vcp_api_key.clone();

    if is_stream {
        // === 6. 流式处理模式 (同步等待，以便串行调用) ===
        let _app_handle = app.clone();
        let message_id_inner = message_id.clone();
        let context_inner = context.clone();
        let mut full_content = String::new();
        let mut last_finish_reason: Option<String> = None;
        let mut is_aborted = false;
        let mut aurora_buffer = AuroraBuffer::new();
        let mut pending_aurora_chunk = String::new();
        let mut last_aurora_parse = std::time::Instant::now() - Duration::from_millis(80);
        let mut last_aurora_content_len = 0usize;
        let mut last_aurora_stable_count = 0usize;
        let mut aurora_sequence = 0u64;
        let aurora_has_rendered = AtomicBool::new(false);
        let mut reasoning_block_open = false;

        // 移动端优先的提交节奏：活动 tail 只传原文，但每次 IPC 仍会唤醒 WebView。
        // 小块控制在 12.5Hz，中块约 8Hz，大块 5Hz；终帧始终立即刷新。
        fn adaptive_parse_interval_ms(tail_len: usize) -> u128 {
            match tail_len {
                0..=8_191 => 80,
                8_192..=24_575 => 120,
                _ => 200,
            }
        }
        // force-parse 字节阈值随档位放大，避免大 chunk 在降帧窗口内靠 byte 阈值反复击穿降帧
        fn adaptive_force_bytes(tail_len: usize) -> usize {
            match tail_len {
                0..=8_191 => 1024,
                8_192..=24_575 => 4096,
                _ => 8192,
            }
        }

        fn close_reasoning_block(
            pending_chunk: &mut String,
            full_content: &mut String,
            reasoning_block_open: &mut bool,
        ) {
            if *reasoning_block_open {
                full_content.push_str("</think>");
                pending_chunk.push_str("</think>");
                *reasoning_block_open = false;
            }
        }

        // 每帧只发送原文 delta、新闭合块和一个轻量活动 tail。
        let mut send_aurora_update = |buffer: &mut AuroraBuffer,
                                      stable_changed: bool,
                                      tail_changed: bool,
                                      finish_reason: Option<String>,
                                      error: Option<String>| {
            let is_final = finish_reason.is_some() || error.is_some();
            let content_delta = if is_final {
                last_aurora_content_len = buffer.full_text.len();
                None
            } else if buffer.full_text.len() > last_aurora_content_len {
                let delta = buffer.full_text[last_aurora_content_len..].to_string();
                last_aurora_content_len = buffer.full_text.len();
                Some(delta)
            } else {
                None
            };
            let stable_blocks_delta = if stable_changed {
                let start = last_aurora_stable_count.min(buffer.stable_blocks.len());
                let delta = buffer.stable_blocks[start..].to_vec();
                last_aurora_stable_count = buffer.stable_blocks.len();
                (!delta.is_empty()).then_some(delta)
            } else {
                None
            };
            aurora_sequence = aurora_sequence.saturating_add(1);

            let mut event = StreamEvent::aurora(
                message_id_inner.clone(),
                AuroraUpdate {
                    sequence: aurora_sequence,
                    stable_blocks_delta,
                    stable_changed,
                    tail_block: tail_changed.then(|| buffer.tail_block.clone()).flatten(),
                    tail_changed,
                    content_delta,
                    content: is_final.then(|| buffer.full_text.clone()),
                },
                context_inner.clone(),
            );
            event.finish_reason = finish_reason;
            event.error = error;
            send_stream_event(event);
            aurora_has_rendered.store(true, Ordering::Relaxed);
        };

        let flush_aurora_parse = |buffer: &mut AuroraBuffer,
                                  pending_chunk: &mut String,
                                  last_parse: &mut std::time::Instant,
                                  force: bool|
         -> (bool, bool) {
            if pending_chunk.is_empty() {
                return (false, false);
            }
            // 以「当前已沉淀 tail 长度 + 待并入 chunk」估算下一帧 tail 体量，据此选择降帧档位
            let projected_tail_len = buffer.tail_content.len() + pending_chunk.len();
            if !force
                && last_parse.elapsed().as_millis() < adaptive_parse_interval_ms(projected_tail_len)
                && pending_chunk.len() < adaptive_force_bytes(projected_tail_len)
            {
                return (false, false);
            }

            buffer.append_chunk(pending_chunk);
            pending_chunk.clear();
            *last_parse = std::time::Instant::now();
            buffer.process_queue()
        };

        let res_future = client
            .post(&final_url)
            .header(AUTHORIZATION, format!("Bearer {}", api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&request_body)
            .send();

        tokio::select! {
            _ = wait_for_cancellation(&mut abort_rx) => {
                log::warn!("[VCPClient] Request aborted before response for message: {}", message_id_inner);
                close_reasoning_block(&mut pending_aurora_chunk, &mut full_content, &mut reasoning_block_open);
                flush_aurora_parse(&mut aurora_buffer, &mut pending_aurora_chunk, &mut last_aurora_parse, true);
                aurora_buffer.finalize();
                send_aurora_update(&mut aurora_buffer, true, true, Some("cancelled_by_user".to_string()), Some("请求已中止".to_string()));
                return Ok((json!({ "fullContent": aurora_buffer.full_text, "streamingStarted": false }), true));
            }
            response_res = res_future => {
                match response_res {
                    Ok(resp) if resp.status().is_success() => {
                        let byte_stream = resp
                            .bytes_stream()
                            .map_err(IoError::other);
                        let stream_reader = StreamReader::new(byte_stream);
                        let mut lines = FramedRead::new(stream_reader, LinesCodec::new());

                        loop {
                            tokio::select! {
                                // 即使工具调用长时间不吐 token，也不能因 idle timeout 误杀连接；这里只响应用户中止和流自身结束。
                                _ = wait_for_cancellation(&mut abort_rx) => {
                                    is_aborted = true;
                                    log::warn!("[VCPClient] Stream deep-polling detected abort for message: {}", message_id_inner);
                                    close_reasoning_block(&mut pending_aurora_chunk, &mut full_content, &mut reasoning_block_open);
                                    flush_aurora_parse(&mut aurora_buffer, &mut pending_aurora_chunk, &mut last_aurora_parse, true);
                                    aurora_buffer.finalize();
                                    send_aurora_update(&mut aurora_buffer, true, true, Some("cancelled_by_user".to_string()), Some("请求已中止".to_string()));

                                    break;
                                }
                                line_res = lines.next() => {
                                    match line_res {
                                        Some(Ok(line)) => {
                                            if line.trim().is_empty() { continue; }
                                            if line.starts_with("data: ") {
                                                let data = line.trim_start_matches("data: ").trim();
                                                if data == "[DONE]" {
                                                    log::debug!("[VCPClient] Stream finished normally with [DONE] for message: {}", message_id_inner);
                                                    close_reasoning_block(&mut pending_aurora_chunk, &mut full_content, &mut reasoning_block_open);
                                                    flush_aurora_parse(&mut aurora_buffer, &mut pending_aurora_chunk, &mut last_aurora_parse, true);
                                                    aurora_buffer.finalize();
                                                    send_aurora_update(&mut aurora_buffer, true, true, last_finish_reason.clone(), None);
                                                    break;
                                                }
                                                if let Ok(chunk) = serde_json::from_str::<Value>(data) {
                                                    // 累加全量内容并驱动 Aurora 沉淀。
                                                    // 部分模型将思考过程放在 reasoning_content 中，不会出现在 delta.content。
                                                    // 将其包装成现有 <think> 语义块，交给前端折叠渲染。
                                                    let mut text_chunk = String::new();
                                                    if let Some(choice) = chunk["choices"].as_array().and_then(|a| a.first()) {
                                                        let delta = &choice["delta"];
                                                        let reasoning_text = delta["reasoning_content"]
                                                            .as_str()
                                                            .or_else(|| delta["reasoningContent"].as_str())
                                                            .or_else(|| delta["reasoning"].as_str())
                                                            .unwrap_or("");
                                                        if !reasoning_text.is_empty() {
                                                            if !reasoning_block_open {
                                                                full_content.push_str("<think>");
                                                                text_chunk.push_str("<think>");
                                                                reasoning_block_open = true;
                                                            }
                                                            full_content.push_str(reasoning_text);
                                                            text_chunk.push_str(reasoning_text);
                                                        }
                                                        if let Some(text) = delta["content"].as_str() {
                                                            if !text.is_empty() && reasoning_block_open {
                                                                full_content.push_str("</think>");
                                                                text_chunk.push_str("</think>");
                                                                reasoning_block_open = false;
                                                            }
                                                            full_content.push_str(text);
                                                            text_chunk.push_str(text);
                                                        }
                                                        if let Some(reason) = choice["finish_reason"].as_str() {
                                                            last_finish_reason = Some(
                                                                if reason == "stop" { "completed".to_string() } else { reason.to_string() }
                                                            );
                                                        }
                                                    }

                                                    if !text_chunk.is_empty() {
                                                        pending_aurora_chunk.push_str(&text_chunk);
                                                        let (stable_changed, tail_changed) = flush_aurora_parse(
                                                            &mut aurora_buffer,
                                                            &mut pending_aurora_chunk,
                                                            &mut last_aurora_parse,
                                                            false,
                                                        );
                                                        if stable_changed || tail_changed {
                                                            send_aurora_update(&mut aurora_buffer, stable_changed, tail_changed, None, None);
                                                        }
                                                    }

                                                    // Aurora 接管前才保留原始 data 事件作为兜底，避免每个 chunk 都跨 IPC 推送两套流式事件。
                                                    if !aurora_has_rendered.load(Ordering::Relaxed) {
                                                        send_stream_event(StreamEvent::data(
                                                            message_id_inner.clone(),
                                                            chunk,
                                                            context_inner.clone(),
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                        Some(Err(e)) => {
                                            log::error!("[VCPClient] Stream read error: {:?}", e);
                                            close_reasoning_block(&mut pending_aurora_chunk, &mut full_content, &mut reasoning_block_open);
                                            flush_aurora_parse(&mut aurora_buffer, &mut pending_aurora_chunk, &mut last_aurora_parse, true);
                                            aurora_buffer.finalize();
                                            send_aurora_update(&mut aurora_buffer, true, true, Some("error".to_string()), Some(format!("流读取错误: {}", e)));
                                            send_stream_event(StreamEvent::error(
                                                message_id_inner.clone(),
                                                context_inner.clone(),
                                                format!("流读取错误: {}", e),
                                            ));

                                            break;
                                        }
                                        None => {
                                            // 修复：若此前已收到有效 chunk，则视为正常结束（对齐桌面端行为）
                                            close_reasoning_block(&mut pending_aurora_chunk, &mut full_content, &mut reasoning_block_open);
                                            flush_aurora_parse(&mut aurora_buffer, &mut pending_aurora_chunk, &mut last_aurora_parse, true);
                                            aurora_buffer.finalize();
                                            if !full_content.is_empty() || last_finish_reason.is_some() {
                                                log::debug!("[VCPClient] Stream ended without [DONE] but content was received. Treating as normal end.");
                                                send_aurora_update(&mut aurora_buffer, true, true, last_finish_reason.clone(), None);
                                            } else {
                                                log::warn!("[VCPClient] Stream ended unexpectedly (None)");
                                                send_aurora_update(&mut aurora_buffer, true, true, Some("error".to_string()), Some("网络连接意外断开".to_string()));
                                                send_stream_event(StreamEvent::error(
                                                    message_id_inner.clone(),
                                                    context_inner.clone(),
                                                    "网络连接意外断开".to_string(),
                                                ));
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        send_stream_event(StreamEvent::error(
                            message_id_inner.clone(),
                            context_inner.clone(),
                            format!("VCP服务器错误: {} - {}", status, text),
                        ));

                        return Err(format!("VCP Error: {}", status));
                    }
                    Err(e) => {
                        send_stream_event(StreamEvent::error(
                            message_id_inner.clone(),
                            context_inner.clone(),
                            format!("网络请求异常: {}", e),
                        ));

                        return Err(e.to_string());
                    }
                }
            }
        }

        Ok((
            json!({
                "fullContent": aurora_buffer.full_text,
                "streamingStarted": true,
                "finishReason": last_finish_reason
            }),
            is_aborted,
        ))
    } else {
        // === 7. 非流式响应模式 ===
        let response_future = client
            .post(&final_url)
            .header(AUTHORIZATION, format!("Bearer {}", api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&request_body)
            .send();
        let response = tokio::select! {
            _ = wait_for_cancellation(&mut abort_rx) => {
                log::warn!("[VCPClient] Non-stream request aborted for message: {}", message_id);
                return Ok((json!({
                    "fullContent": "",
                    "streamingStarted": false,
                    "finishReason": "cancelled_by_user"
                }), true));
            }
            response = response_future => {
                match response {
                    Ok(response) => response,
                    Err(error) => {
                        let error = format!("VCP non-stream request failed: {error}");
                        send_stream_event(StreamEvent::error(
                            message_id.clone(),
                            context.clone(),
                            error.clone(),
                        ));
                        return Err(error);
                    }
                }
            }
        };

        let status = response.status();
        let response_text = match response.text().await {
            Ok(text) => text,
            Err(error) => {
                let error = format!("VCP non-stream response read failed: {error}");
                send_stream_event(StreamEvent::error(
                    message_id.clone(),
                    context.clone(),
                    error.clone(),
                ));
                return Err(error);
            }
        };

        if !status.is_success() {
            let detail: String = response_text.chars().take(512).collect();
            let error = if detail.trim().is_empty() {
                format!("VCP non-stream response failed with HTTP {status}")
            } else {
                format!("VCP non-stream response failed with HTTP {status}: {detail}")
            };
            send_stream_event(StreamEvent::error(
                message_id.clone(),
                context.clone(),
                error.clone(),
            ));
            return Err(error);
        }

        let vcp_response = match serde_json::from_str::<Value>(&response_text) {
            Ok(response) => response,
            Err(error) => {
                let error = format!("VCP non-stream JSON parse failed: {error}");
                send_stream_event(StreamEvent::error(
                    message_id.clone(),
                    context.clone(),
                    error.clone(),
                ));
                return Err(error);
            }
        };
        let mut normalized = match normalize_non_stream_completion(vcp_response) {
            Ok(normalized) => normalized,
            Err(error) => {
                send_stream_event(StreamEvent::error(
                    message_id.clone(),
                    context.clone(),
                    error.clone(),
                ));
                return Err(error);
            }
        };
        normalized["context"] = json!(context.clone());

        // Non-streaming responses still travel through the same message channel as SSE.
        // Publishing the complete text before the terminal event prevents an empty
        // placeholder when a caller or older frontend treats `end` as metadata only.
        let content_event = non_stream_content_event(&normalized, message_id, context)?;
        send_stream_event(content_event);
        Ok((normalized, false))
    }
}

async fn load_app_settings<R: Runtime>(app: &AppHandle<R>) -> Result<Settings, String> {
    let db_state = app.state::<DbState>();
    let pool = &db_state.pool;

    let row = sqlx::query("SELECT value FROM settings WHERE key = 'global'")
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(row) = row {
        use sqlx::Row;
        let content: String = row.get("value");
        let settings = serde_json::from_str::<Settings>(&content)
            .unwrap_or_else(|_| create_default_settings());
        Ok(settings)
    } else {
        Ok(create_default_settings())
    }
}

/// 中止请求 Command: interruptRequest
/// 通过 messageId 立即触发对应的 oneshot 信号
#[tauri::command]
#[allow(non_snake_case)]
pub fn interruptRequest(
    state: tauri::State<'_, ActiveRequests>,
    message_id: String,
) -> Result<Value, String> {
    log::info!(
        "[VCPClient] interruptRequest called for messageId: {}. Active requests: {}",
        message_id,
        state.0.len()
    );
    if let Some(control) = state.0.get(&message_id).map(|entry| entry.clone()) {
        let first = control.cancel();
        let status = if first {
            "interrupted"
        } else {
            "already_cancelled"
        };
        log::info!(
            "[VCPClient] Interrupt status for messageId {}: {}",
            message_id,
            status
        );
        Ok(json!({
            "success": true,
            "status": status,
            "messageId": message_id
        }))
    } else {
        log::warn!(
            "[VCPClient] No active request found for messageId: {}",
            message_id
        );
        Ok(json!({
            "success": false,
            "status": "not_found",
            "messageId": message_id
        }))
    }
}

/// 测试 VCP 后端连接状态并获取模型列表 (对齐桌面端 main.js fetchAndCacheModels 逻辑)
#[tauri::command]
pub async fn test_vcp_connection(vcp_url: String, vcp_api_key: String) -> Result<Value, String> {
    log::info!(
        "[VCPClient] test_vcp_connection called for URL: {}",
        vcp_url
    );

    // 对齐桌面端原汁原味的逻辑：
    // const urlObject = new URL(vcpServerUrl);
    // const baseUrl = `${urlObject.protocol}//${urlObject.host}`;
    // const modelsUrl = new URL('/v1/models', baseUrl).toString();

    let url_object = match Url::parse(&vcp_url) {
        Ok(url) => url,
        Err(e) => return Err(format!("URL 解析失败: {}", e)),
    };

    // 对齐 JS 的 urlObject.host (包含端口号)
    let port_str = match url_object.port() {
        Some(p) => format!(":{}", p),
        None => "".to_string(),
    };
    let host_with_port = format!("{}{}", url_object.host_str().unwrap_or(""), port_str);
    let base_url = format!("{}://{}", url_object.scheme(), host_with_port);

    let models_url = if base_url.ends_with('/') {
        format!("{}v1/models", base_url)
    } else {
        format!("{}/v1/models", base_url)
    };

    log::info!(
        "[VCPClient] Testing connection to (Original Logic): {}",
        models_url
    );

    let client = Client::builder()
        .timeout(Duration::from_secs(10)) // 测试连接 10s 超时即可
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .get(&models_url)
        .header(AUTHORIZATION, format!("Bearer {}", vcp_api_key))
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {}", e))?;

    let status = res.status();
    if status.is_success() {
        let json_res: Value = res
            .json()
            .await
            .map_err(|e| format!("JSON解析失败: {}", e))?;

        // 尝试提取模型数量，对齐桌面端 `cachedModels = data.data || []`
        let model_count = json_res
            .get("data")
            .and_then(|data| data.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0);

        Ok(json!({
            "success": true,
            "status": status.as_u16(),
            "modelCount": model_count,
            "models": json_res
        }))
    } else {
        let text = res.text().await.unwrap_or_default();
        Err(format!("验证失败 ({}): {}", status.as_u16(), text))
    }
}

/// Normalize a VCP server URL by appending `/v1/chat/completions` if missing.
/// Handles URLs with or without trailing slashes in the existing path.
pub fn normalize_vcp_url(url_str: &str) -> String {
    if let Ok(url) = Url::parse(url_str) {
        if url.path().ends_with("/chatvcp/completions") {
            let mut url = url;
            let new_path = url
                .path()
                .replace("/chatvcp/completions", "/chat/completions");
            url.set_path(&new_path);
            return url.to_string();
        }
        if !url.path().ends_with("/chat/completions") {
            let mut url = url;
            let new_path = if url.path().ends_with('/') {
                format!("{}v1/chat/completions", url.path())
            } else {
                format!("{}/v1/chat/completions", url.path())
            };
            url.set_path(&new_path);
            return url.to_string();
        }
    }
    url_str.to_string()
}

#[cfg(test)]
mod completion_response_tests {
    use super::*;

    #[test]
    fn normalizes_openai_non_stream_completion() {
        let normalized = normalize_non_stream_completion(json!({
            "choices": [{
                "message": { "role": "assistant", "content": "final answer" },
                "finish_reason": "stop"
            }]
        }))
        .unwrap();

        assert_eq!(normalized["fullContent"], "final answer");
        assert_eq!(normalized["finishReason"], "completed");
        assert_eq!(normalized["streamingStarted"], false);
    }

    #[test]
    fn preserves_reasoning_and_structured_text_content() {
        let normalized = normalize_non_stream_completion(json!({
            "choices": [{
                "message": {
                    "reasoning_content": "private reasoning",
                    "content": [
                        { "type": "text", "text": "first" },
                        { "type": "output_text", "text": "second" }
                    ]
                },
                "finish_reason": "length"
            }]
        }))
        .unwrap();

        assert_eq!(
            normalized["fullContent"],
            "<think>private reasoning</think>first\nsecond"
        );
        assert_eq!(normalized["finishReason"], "length");
    }

    #[test]
    fn rejects_non_completion_json() {
        let error = normalize_non_stream_completion(json!({ "error": "bad gateway" })).unwrap_err();

        assert!(error.contains("completion choice"));
        assert!(error.contains("bad gateway"));
    }

    #[test]
    fn rejects_empty_non_stream_completion() {
        let error = normalize_non_stream_completion(json!({
            "choices": [{
                "message": { "role": "assistant", "content": null },
                "finish_reason": "stop"
            }]
        }))
        .unwrap_err();

        assert!(error.contains("no assistant content"));
    }

    #[test]
    fn accepts_delta_and_refusal_fallbacks() {
        let delta = normalize_non_stream_completion(json!({
            "choices": [{
                "delta": { "content": "provider fallback" },
                "finish_reason": "stop"
            }]
        }))
        .unwrap();
        assert_eq!(delta["fullContent"], "provider fallback");

        let refusal = normalize_non_stream_completion(json!({
            "choices": [{
                "message": { "content": null, "refusal": "request refused" },
                "finish_reason": "stop"
            }]
        }))
        .unwrap();
        assert_eq!(refusal["fullContent"], "request refused");
    }

    #[test]
    fn builds_data_event_for_non_stream_content() {
        let normalized = normalize_non_stream_completion(json!({
            "choices": [{
                "message": { "content": "one-shot answer" },
                "finish_reason": "stop"
            }]
        }))
        .unwrap();
        let context = json!({ "agentId": "agent-1", "topicId": "topic-1" });

        let event =
            non_stream_content_event(&normalized, "message-1".to_string(), Some(context.clone()))
                .unwrap();

        assert_eq!(event.r#type, "data");
        assert_eq!(event.message_id, "message-1");
        assert_eq!(event.chunk, Some(json!("one-shot answer")));
        assert_eq!(event.context, Some(context));
    }

    #[test]
    fn cancellation_is_idempotent_and_visible_to_late_subscribers() {
        let control = ActiveRequestControl::new();
        assert!(control.cancel());
        assert!(!control.cancel());
        assert!(control.is_cancelled());
        assert!(*control.subscribe().borrow());
    }

    #[test]
    fn stale_guard_does_not_remove_replacement_request() {
        let requests = ActiveRequests::default();
        let old_control = requests.register("same-id");
        let guard = ActiveRequestGuard::new(requests.0.clone(), "same-id".to_string(), old_control);
        let replacement = requests.register("same-id");

        drop(guard);

        let current = requests.0.get("same-id").unwrap();
        assert!(Arc::ptr_eq(current.value(), &replacement));
    }
}

#[cfg(test)]
mod image_host_tests {
    use super::*;

    fn image_host_config() -> ImageHostConfig {
        ImageHostConfig {
            base_url: "https://images.example.com".to_string(),
            image_key: "read key".to_string(),
            upload_key: "upload key".to_string(),
        }
    }

    #[test]
    fn image_host_uses_dedicated_upload_key() {
        assert_eq!(
            image_host_config().upload_url(),
            "https://images.example.com/pw=upload%20key/images/upload"
        );
    }

    #[test]
    fn keeps_absolute_image_upload_url() {
        assert_eq!(
            normalize_image_upload_location(
                &image_host_config(),
                "https://cdn.example.com/folder/a.jpg"
            ),
            Some("https://cdn.example.com/folder/a.jpg".to_string())
        );
    }

    #[test]
    fn expands_relative_image_upload_file_name() {
        assert_eq!(
            normalize_image_upload_location(&image_host_config(), "8923.jpg"),
            Some("https://images.example.com/pw=read%20key/images/8923.jpg".to_string())
        );
    }

    #[test]
    fn expands_root_relative_image_upload_url() {
        assert_eq!(
            normalize_image_upload_location(&image_host_config(), "/pw=read%20key/images/8923.jpg"),
            Some("https://images.example.com/pw=read%20key/images/8923.jpg".to_string())
        );
    }

    #[test]
    fn encodes_nested_image_upload_key() {
        assert_eq!(
            normalize_image_upload_location(&image_host_config(), "folder/a b.jpg"),
            Some("https://images.example.com/pw=read%20key/images/folder/a%20b.jpg".to_string())
        );
    }
}

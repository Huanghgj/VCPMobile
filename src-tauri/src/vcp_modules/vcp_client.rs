use base64::{engine::general_purpose, Engine as _};
use dashmap::{DashMap, DashSet};
use futures_util::StreamExt;
use futures_util::TryStreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Error as IoError;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::oneshot;
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::io::StreamReader;
use url::Url;

use crate::vcp_modules::db_manager::DbState;
use crate::vcp_modules::settings_manager::{create_default_settings, Settings};

/// =================================================================
/// vcp_modules/vcp_client.rs - 统一的 VCP 请求处理模块 (Rust 重写版)
/// =================================================================
/// 该模块对应原项目的 modules/vcpClient.js，负责处理所有与 VCP 服务器的通信。
/// 包含动态路由、上下文注入（音乐、UI 规范）、流式 SSE 解析以及请求中止机制。
/// 请求参数结构体
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcpRequestPayload {
    pub vcp_url: String,                // VCP服务器URL
    pub vcp_api_key: String,            // API密钥
    pub messages: Vec<Value>,           // 消息数组
    pub model_config: Value,            // 模型配置 (包含 model, stream, temperature 等)
    pub message_id: String,             // 消息ID (用于跟踪和中止)
    pub context: Option<Value>,         // 上下文信息 (agentId, topicId等)
    pub stream_channel: Option<String>, // 流式数据频道名称 (默认为 vcp-stream-event)
}

/// 流式事件结构体，用于向前端发送数据
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StreamEvent {
    pub r#type: String,         // 事件类型: "data", "end", "error"
    pub chunk: Option<Value>,   // 数据块 (仅 type="data" 时有效)
    pub message_id: String,     // 消息ID
    pub context: Option<Value>, // 透传的上下文信息
    pub error: Option<String>,  // 错误信息 (仅 type="error" 时有效)
}

const STREAM_EMIT_MIN_CHARS: usize = 96;
const DEFAULT_THINKING_BUDGET: i64 = 4096;
const MIN_THINKING_BUDGET: i64 = 1024;
const MAX_THINKING_BUDGET: i64 = 32768;
const MAX_ORIGINAL_INLINE_IMAGE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ORIGINAL_INLINE_MEDIA_BYTES: u64 = 24 * 1024 * 1024;
const MAX_INLINE_IMAGE_DIMENSION: u32 = 2048;
const INLINE_IMAGE_JPEG_QUALITY: u8 = 85;
const MOBILE_SURFACE_PROMPT: &str = r#"移动端 Surface 挂件能力：
当用户需要临时看板、计时器、可视化、简单小工具、SVG/canvas 动画或交互式信息面板时，你可以输出一个完整的移动端 Surface 块。格式必须严格如下：
<<<[DESKTOP_PUSH]>>>
<div style="padding:16px;border-radius:14px;background:rgba(20,20,24,.86);color:#fff;">内容</div>
<<<[DESKTOP_PUSH_END]>>>

移动端会把块内 HTML/CSS/JS 渲染为可拖动浮层挂件。要求：
- HTML 必须自包含，优先使用内联 style 和原生 JavaScript。
- 适配手机屏幕，默认宽度约 320px，高度约 220px，避免桌面级大布局。
- 不要使用 DesktopRemote、Dock、Windows 快捷方式、系统壁纸或 Electron 专属能力。
- 可使用受限的 vcpAPI.weather()、vcpAPI.fetch(path, options)、vcpAPI.post(path, body) 访问移动端允许的 VCP 管理接口；musicAPI 仍可能不可用。
- 不要把 DESKTOP_PUSH 块包进 Markdown 代码块。"#;
const MOBILE_BROWSER_PROMPT: &str = r#"移动端浏览器能力：
你可以使用工具 MobileBrowser 访问和控制手机内置浏览器，用于打开网页、读取页面、点击、输入、滚动、截图、执行页面脚本，以及在必要时把控制权交给用户。

使用原则：
- 普通网页任务优先自己完成：navigate/open 打开 URL；snapshot/read_text 获取页面结构和文字；click/type/tap/scroll/back/forward/reload/screenshot/eval 操作页面并观察结果。
- 每次关键操作后用 snapshot、read_text 或 screenshot 确认页面状态，不要凭猜测继续。
- 遇到登录、验证码、2FA、人机验证、支付、授权确认或需要用户隐私凭据的步骤时，使用 handoff，并在 reason 中简要说明需要用户完成什么；不要索要或代填敏感信息。
- 如果用户需要从另一台设备或本机浏览器辅助，通过 start_assist_server 开启临时辅助网页。默认只绑定 127.0.0.1；需要局域网访问时传 bindLan=true。把工具返回的 url/localUrl/token 告诉用户，用完后可调用 stop_assist_server。
- 用户完成手动步骤后使用 resume 恢复 AI 控制，再继续 snapshot 检查当前页面。
- 只在完成用户请求所必需的范围内访问网页，不要绕过网站安全、付费或访问控制。"#;

#[derive(Default)]
struct StreamTextParts {
    reasoning: String,
    answer: String,
}

fn parse_i64_setting(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|n| i64::try_from(n).ok())),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn normalized_thinking_budget(value: Option<&Value>) -> i64 {
    parse_i64_setting(value)
        .unwrap_or(DEFAULT_THINKING_BUDGET)
        .clamp(MIN_THINKING_BUDGET, MAX_THINKING_BUDGET)
}

fn reasoning_effort_for_budget(budget: i64) -> &'static str {
    if budget >= 8192 {
        "high"
    } else if budget <= 2048 {
        "low"
    } else {
        "medium"
    }
}

fn is_claude_thinking_capable(model_lower: &str) -> bool {
    model_lower.contains("3-7")
        || model_lower.contains("3.7")
        || model_lower.contains("claude-4")
        || model_lower.contains("sonnet-4")
        || model_lower.contains("opus-4")
        || model_lower.contains("haiku-4")
        || model_lower.contains("4-")
        || model_lower.contains("4.")
        || model_lower.contains("mythos")
}

fn is_gemini_thinking_capable(model_lower: &str) -> bool {
    model_lower.contains("2.5")
        || model_lower.contains("gemini-3")
        || model_lower.contains("thinking")
        || model_lower.contains("think")
}

fn ensure_token_limit_above_budget(obj: &mut serde_json::Map<String, Value>, budget: i64) {
    let target_limit = budget + 1024;
    let has_known_limit =
        obj.contains_key("max_tokens") || obj.contains_key("max_completion_tokens");

    if has_known_limit {
        for key in ["max_tokens", "max_completion_tokens"] {
            let current = parse_i64_setting(obj.get(key));
            if current.map(|limit| limit <= budget).unwrap_or(false) {
                obj.insert(key.to_string(), json!(target_limit));
            }
        }
    } else {
        obj.insert("max_tokens".to_string(), json!(target_limit));
    }
}

fn set_nested_object<'a>(
    obj: &'a mut serde_json::Map<String, Value>,
    key: &str,
) -> &'a mut serde_json::Map<String, Value> {
    let value = obj.entry(key.to_string()).or_insert_with(|| json!({}));
    if !value.is_object() {
        *value = json!({});
    }
    value
        .as_object_mut()
        .expect("value was normalized to object")
}

fn apply_model_thinking_params(request_body: &mut Value, model: &str, budget: i64) {
    let Some(obj) = request_body.as_object_mut() else {
        return;
    };

    let model_lower = model.to_ascii_lowercase();
    let effort = reasoning_effort_for_budget(budget);

    if model_lower.contains("claude") && is_claude_thinking_capable(&model_lower) {
        let uses_adaptive_thinking = model_lower.contains("4-6")
            || model_lower.contains("4.6")
            || model_lower.contains("4-7")
            || model_lower.contains("4.7")
            || model_lower.contains("mythos");

        if uses_adaptive_thinking {
            obj.insert(
                "thinking".to_string(),
                json!({
                    "type": "adaptive",
                    "display": "summarized"
                }),
            );
            obj.insert("output_config".to_string(), json!({ "effort": effort }));
        } else {
            obj.insert(
                "thinking".to_string(),
                json!({
                    "type": "enabled",
                    "budget_tokens": budget
                }),
            );
            ensure_token_limit_above_budget(obj, budget);
        }

        obj.remove("temperature");
        return;
    }

    if model_lower.contains("gemini") && is_gemini_thinking_capable(&model_lower) {
        let extra_body = set_nested_object(obj, "extra_body");
        let google = set_nested_object(extra_body, "google");
        let thinking_config = set_nested_object(google, "thinking_config");
        thinking_config.insert("include_thoughts".to_string(), json!(true));

        if model_lower.contains("gemini-3") {
            thinking_config.insert("thinking_level".to_string(), json!(effort));
        } else {
            thinking_config.insert("thinking_budget".to_string(), json!(budget));
        }
        return;
    }

    // DeepSeek-compatible endpoints are inconsistent about reasoning fields:
    // some return final text in reasoning_content, and some expose raw <think>
    // tags in content. Do not proactively request reasoning here; otherwise
    // normal answers can be wrapped into the app's internal <think> transport.
    if model_lower.contains("deepseek") {
        return;
    }
}

fn append_text_value(target: &mut String, value: &Value) {
    match value {
        Value::String(text) => target.push_str(text),
        Value::Array(items) => {
            for item in items {
                append_text_value(target, item);
            }
        }
        Value::Object(map) => {
            for key in ["text", "content", "summary"] {
                if let Some(value) = map.get(key) {
                    let mut nested = String::new();
                    append_text_value(&mut nested, value);
                    if !nested.is_empty() {
                        target.push_str(&nested);
                        break;
                    }
                }
            }
        }
        _ => {}
    }
}

fn append_deduped_text(target: &mut String, text: &str) {
    if text.is_empty() || target.ends_with(text) {
        return;
    }
    target.push_str(text);
}

fn append_first_field(target: &mut String, source: &Value, keys: &[&str]) {
    for key in keys {
        if let Some(value) = source.get(*key) {
            let mut text = String::new();
            append_text_value(&mut text, value);
            if !text.is_empty() {
                append_deduped_text(target, &text);
                return;
            }
        }
    }
}

fn append_openai_choice_parts(parts: &mut StreamTextParts, source: &Value) {
    append_first_field(
        &mut parts.reasoning,
        source,
        &[
            "reasoning_content",
            "reasoning",
            "reasoning_text",
            "reasoning_details",
            "thinking",
            "thought",
        ],
    );
    append_first_field(&mut parts.answer, source, &["content", "text"]);
}

fn extract_stream_text_parts(chunk: &Value) -> StreamTextParts {
    let mut parts = StreamTextParts::default();

    if let Some(choices) = chunk.get("choices").and_then(Value::as_array) {
        for choice in choices {
            if let Some(delta) = choice.get("delta") {
                append_openai_choice_parts(&mut parts, delta);
            }
            if let Some(message) = choice.get("message") {
                append_openai_choice_parts(&mut parts, message);
            }
        }
    }

    if let Some(delta) = chunk.get("delta") {
        match delta.get("type").and_then(Value::as_str) {
            Some("thinking_delta") => {
                append_first_field(&mut parts.reasoning, delta, &["thinking", "text"])
            }
            Some("text_delta") => append_first_field(&mut parts.answer, delta, &["text"]),
            _ => append_openai_choice_parts(&mut parts, delta),
        }
    }

    if let Some(content_block) = chunk.get("content_block") {
        match content_block.get("type").and_then(Value::as_str) {
            Some("thinking") => {
                append_first_field(&mut parts.reasoning, content_block, &["thinking", "text"])
            }
            Some("text") => append_first_field(&mut parts.answer, content_block, &["text"]),
            _ => append_openai_choice_parts(&mut parts, content_block),
        }
    }

    if let Some(candidates) = chunk.get("candidates").and_then(Value::as_array) {
        for candidate in candidates {
            if let Some(gemini_parts) = candidate
                .pointer("/content/parts")
                .and_then(Value::as_array)
            {
                for part in gemini_parts {
                    if part
                        .get("thought")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        append_first_field(&mut parts.reasoning, part, &["text"]);
                    } else {
                        append_first_field(&mut parts.answer, part, &["text"]);
                    }
                }
            }
        }
    }

    if parts.reasoning.is_empty() && parts.answer.is_empty() {
        append_first_field(
            &mut parts.reasoning,
            chunk,
            &["reasoning_content", "reasoning", "thinking", "thought"],
        );
        append_first_field(&mut parts.answer, chunk, &["content", "text"]);
    }

    parts
}

fn push_stream_segment(
    full_content: &mut String,
    pending_emit_text: &mut String,
    reasoning_block_open: &mut bool,
    text: &str,
    is_reasoning: bool,
) {
    if text.is_empty() {
        return;
    }

    if is_reasoning {
        if !*reasoning_block_open {
            let opener = if full_content.is_empty() {
                "<think>\n"
            } else {
                "\n\n<think>\n"
            };
            full_content.push_str(opener);
            pending_emit_text.push_str(opener);
            *reasoning_block_open = true;
        }
    } else if *reasoning_block_open {
        let closer = "\n</think>\n\n";
        full_content.push_str(closer);
        pending_emit_text.push_str(closer);
        *reasoning_block_open = false;
    }

    full_content.push_str(text);
    pending_emit_text.push_str(text);
}

fn close_reasoning_block(
    full_content: &mut String,
    pending_emit_text: &mut String,
    reasoning_block_open: &mut bool,
) {
    if *reasoning_block_open {
        let closer = "\n</think>\n\n";
        full_content.push_str(closer);
        pending_emit_text.push_str(closer);
        *reasoning_block_open = false;
    }
}

fn should_emit_reasoning_output(model: &str) -> bool {
    !model.to_ascii_lowercase().contains("deepseek")
}

fn setting_value<'a>(settings: &'a Settings, key: &str) -> Option<&'a Value> {
    settings.extra.get(key).or_else(|| {
        settings
            .extra
            .get("extra")
            .and_then(Value::as_object)
            .and_then(|extra| extra.get(key))
    })
}

fn setting_bool(settings: &Settings, key: &str, default: bool) -> bool {
    setting_value(settings, key)
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

fn setting_thinking_budget(settings: &Settings, key: &str) -> i64 {
    normalized_thinking_budget(setting_value(settings, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_does_not_request_or_emit_reasoning_by_default() {
        let mut body = json!({
            "model": "deepseek-reasoner",
            "temperature": 0.7
        });

        apply_model_thinking_params(&mut body, "deepseek-reasoner", DEFAULT_THINKING_BUDGET);

        let obj = body.as_object().expect("request body should remain an object");
        assert!(!obj.contains_key("thinking"));
        assert!(!obj.contains_key("reasoning_effort"));
        assert!(!should_emit_reasoning_output("deepseek-reasoner"));
    }

    #[test]
    fn settings_reader_accepts_nested_legacy_extra() {
        let mut settings = create_default_settings();
        settings.extra = json!({
            "extra": {
                "enableVcpToolInjection": true,
                "modelThinkingBudget": 8192
            }
        });

        assert!(setting_bool(&settings, "enableVcpToolInjection", false));
        assert_eq!(setting_thinking_budget(&settings, "modelThinkingBudget"), 8192);
    }
}

fn emit_pending_stream_text<R: Runtime>(
    app_handle: &AppHandle<R>,
    stream_channel: &str,
    message_id: &str,
    context: &Option<Value>,
    pending_text: &mut String,
    last_emit_at: &mut Instant,
    force: bool,
) {
    if pending_text.is_empty() {
        return;
    }

    if !force
        && pending_text.len() < STREAM_EMIT_MIN_CHARS
        && last_emit_at.elapsed() < Duration::from_millis(120)
    {
        return;
    }

    let chunk = std::mem::take(pending_text);
    let _ = app_handle.emit(
        stream_channel,
        StreamEvent {
            r#type: "data".to_string(),
            chunk: Some(json!(chunk)),
            message_id: message_id.to_string(),
            context: context.clone(),
            error: None,
        },
    );
    *last_emit_at = Instant::now();
}

/// 单个活跃 VCP 请求的中断上下文。
pub struct ActiveRequestHandle {
    pub abort_sender: oneshot::Sender<()>,
    pub vcp_url: String,
    pub vcp_api_key: String,
}

/// 全局活跃请求管理器，使用 DashMap 存储中止信号发送端与 VCP 后端信息。
/// messageId -> ActiveRequestHandle
pub struct ActiveRequests(pub Arc<DashMap<String, ActiveRequestHandle>>);

impl Default for ActiveRequests {
    fn default() -> Self {
        println!("[VCPClient] Initialized ActiveRequests successfully.");
        Self(Arc::new(DashMap::new()))
    }
}

/// 群组回合取消令牌，用于标记需要中断接力赛的话题
/// topicId -> true (存在即代表已取消)
pub struct CancelledGroupTurns(pub Arc<DashSet<String>>);

impl Default for CancelledGroupTurns {
    fn default() -> Self {
        println!("[VCPClient] Initialized CancelledGroupTurns successfully.");
        Self(Arc::new(DashSet::new()))
    }
}

/// 内部辅助函数：获取应用程序数据目录
async fn get_app_data_path<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("AppData"))
}

fn image_mime_for_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

fn encode_original_data_url(path: &Path, mime_type: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取附件失败: {}", e))?;
    let b64 = general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime_type, b64))
}

fn encode_image_data_url(path: &Path, ext: &str) -> Result<String, String> {
    let mime_type = image_mime_for_ext(ext).unwrap_or("image/jpeg");
    let file_size = std::fs::metadata(path)
        .map_err(|e| format!("读取图片元数据失败: {}", e))?
        .len();
    let dimensions = image::image_dimensions(path).ok();

    if ext == "gif" {
        if file_size > MAX_ORIGINAL_INLINE_MEDIA_BYTES {
            return Err("GIF 图片过大，无法作为模型输入内联发送".to_string());
        }
        return encode_original_data_url(path, mime_type);
    }

    if file_size <= MAX_ORIGINAL_INLINE_IMAGE_BYTES {
        if let Some((width, height)) = dimensions {
            if width <= MAX_INLINE_IMAGE_DIMENSION && height <= MAX_INLINE_IMAGE_DIMENSION {
                return encode_original_data_url(path, mime_type);
            }
        }
    }

    let img = image::open(path).map_err(|e| format!("图片压缩前解码失败: {}", e))?;
    let resized = img.thumbnail(MAX_INLINE_IMAGE_DIMENSION, MAX_INLINE_IMAGE_DIMENSION);
    let rgb = resized.to_rgb8();
    let mut encoded = Vec::new();
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, INLINE_IMAGE_JPEG_QUALITY);
    encoder
        .encode_image(&rgb)
        .map_err(|e| format!("图片压缩编码失败: {}", e))?;

    let b64 = general_purpose::STANDARD.encode(&encoded);
    Ok(format!("data:image/jpeg;base64,{}", b64))
}

/// 中止群组的整个接力赛回合
#[tauri::command]
#[allow(non_snake_case)]
pub fn interruptGroupTurn(
    state: tauri::State<'_, CancelledGroupTurns>,
    topic_id: String,
) -> Result<Value, String> {
    println!(
        "[VCPClient] interruptGroupTurn called for topicId: {}",
        topic_id
    );
    state.0.insert(topic_id);
    Ok(json!({"status": "cancelled"}))
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
) -> Result<Value, String> {
    let (res, _is_aborted) = perform_vcp_request(&app, state.0.clone(), payload).await?;
    Ok(res)
}

/// 核心请求实现函数，可供 Tauri Command 或 内部 Rust 模块(如 GroupOrchestrator) 调用
/// 返回 Result<(全量内容/响应体, 是否被中止), 错误信息>
pub async fn perform_vcp_request<R: Runtime>(
    app: &AppHandle<R>,
    active_requests: Arc<DashMap<String, ActiveRequestHandle>>,
    payload: VcpRequestPayload,
) -> Result<(Value, bool), String> {
    println!(
        "[VCPClient] perform_vcp_request called for messageId: {}, context: {:?}",
        payload.message_id, payload.context
    );
    let app_data_path = get_app_data_path(app).await;
    let stream_channel = payload
        .stream_channel
        .clone()
        .unwrap_or_else(|| "vcp-stream".to_string());

    // === 0. 数据验证和规范化 ===
    let mut messages: Vec<Value> = Vec::new();
    for msg_val in payload.messages {
        if !msg_val.is_object() {
            messages.push(json!({"role": "system", "content": "[Invalid message]"}));
            continue;
        }

        let mut msg = msg_val.clone();
        let content = msg.get("content").cloned().unwrap_or(Value::Null);

        // 处理多模态或复杂内容数组
        if let Some(content_array) = content.as_array() {
            let mut new_parts = Vec::new();
            for part in content_array {
                if let Some(obj) = part.as_object() {
                    // 识别自定义的 local_file 类型并进行路径还原与编码
                    if obj.get("type").and_then(|t| t.as_str()) == Some("local_file") {
                        if let Some(path_str) = obj.get("path").and_then(|p| p.as_str()) {
                            let clean_path = path_str.replace("file://", "");
                            let path_buf = std::path::PathBuf::from(&clean_path);

                            if path_buf.exists() {
                                // 提取扩展名决定 mime_type
                                let ext = path_buf
                                    .extension()
                                    .and_then(|e| e.to_str())
                                    .unwrap_or("")
                                    .to_lowercase();
                                let encoded_part = match ext.as_str() {
                                    "png" | "jpg" | "jpeg" | "webp" | "gif" => {
                                        encode_image_data_url(&path_buf, &ext)
                                            .map(|data_url| ("image_url", data_url))
                                    }
                                    "mp3" => encode_original_data_url(&path_buf, "audio/mpeg")
                                        .map(|data_url| ("audio_url", data_url)),
                                    "wav" => encode_original_data_url(&path_buf, "audio/wav")
                                        .map(|data_url| ("audio_url", data_url)),
                                    "ogg" => encode_original_data_url(&path_buf, "audio/ogg")
                                        .map(|data_url| ("audio_url", data_url)),
                                    "mp4" => encode_original_data_url(&path_buf, "video/mp4")
                                        .map(|data_url| ("video_url", data_url)),
                                    "mkv" => {
                                        encode_original_data_url(&path_buf, "video/x-matroska")
                                            .map(|data_url| ("video_url", data_url))
                                    }
                                    "webm" => encode_original_data_url(&path_buf, "video/webm")
                                        .map(|data_url| ("video_url", data_url)),
                                    _ => encode_original_data_url(
                                        &path_buf,
                                        "application/octet-stream",
                                    )
                                    .map(|data_url| ("file_url", data_url)),
                                };

                                match encoded_part {
                                    Ok((part_type, data_url)) => {
                                        new_parts.push(json!({
                                            "type": part_type,
                                            part_type: { "url": data_url }
                                        }));
                                    }
                                    Err(e) => {
                                        new_parts.push(json!({
                                            "type": "text",
                                            "text": format!(
                                                "[附件处理失败: {} - {}]",
                                                path_buf.display(),
                                                e
                                            )
                                        }));
                                    }
                                }
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
    let mut agent_music_control = false;
    let mut enable_agent_bubble_theme = true;
    let mut enable_mobile_surface_injection = true;
    let mut enable_mobile_browser_injection = true;
    let mut enable_model_thinking = true;
    let mut model_thinking_budget = DEFAULT_THINKING_BUDGET;

    if let Ok(settings) = load_app_settings(app).await {
        enable_vcp_tool_injection = setting_bool(&settings, "enableVcpToolInjection", false);
        agent_music_control = setting_bool(&settings, "agentMusicControl", false);
        enable_agent_bubble_theme = setting_bool(&settings, "enableAgentBubbleTheme", true);
        enable_mobile_surface_injection =
            setting_bool(&settings, "enableMobileSurfaceInjection", true);
        enable_mobile_browser_injection =
            setting_bool(&settings, "enableMobileBrowserInjection", true);
        enable_model_thinking = setting_bool(&settings, "enableModelThinking", true);
        model_thinking_budget = setting_thinking_budget(&settings, "modelThinkingBudget");
    }

    let mut final_url = payload.vcp_url.clone();
    if enable_vcp_tool_injection {
        if let Ok(mut url) = Url::parse(&final_url) {
            url.set_path("/v1/chatvcp/completions");
            final_url = url.to_string();
        }
    } else {
        final_url = normalize_vcp_url(&final_url);
    }

    // === 2. 上下文注入 ===
    let has_system = messages.iter().any(|m| m["role"] == "system");
    if !has_system {
        messages.insert(0, json!({"role": "system", "content": ""}));
    }

    let mut top_parts = Vec::new();
    let mut bottom_parts = Vec::new();

    // 3.1 音乐状态注入
    let music_state_path = app_data_path.join("music_state.json");
    if let Ok(content) = tokio::fs::read_to_string(&music_state_path).await {
        if let Ok(m_state) = serde_json::from_str::<Value>(&content) {
            if let (Some(title), Some(artist)) =
                (m_state["title"].as_str(), m_state["artist"].as_str())
            {
                let album = m_state["album"].as_str().unwrap_or("未知专辑");
                bottom_parts.push(format!(
                    "[当前播放音乐：{} - {} ({})]",
                    title, artist, album
                ));
            }
        }
    }

    // 3.2 播放列表与点歌台注入
    if agent_music_control {
        let songlist_path = app_data_path.join("songlist.json");
        if let Ok(content) = tokio::fs::read_to_string(&songlist_path).await {
            if let Ok(songlist) = serde_json::from_str::<Value>(&content) {
                if let Some(songs) = songlist.as_array() {
                    let titles: Vec<&str> =
                        songs.iter().filter_map(|s| s["title"].as_str()).collect();
                    if !titles.is_empty() {
                        top_parts.push(format!("[播放列表——\n{}\n]", titles.join("\n")));
                    }
                }
            }
        }
        bottom_parts.push("点歌台{{VCPMusicController}}".to_string());
    }

    // 3.3 UI 规范要求注入
    if enable_agent_bubble_theme {
        bottom_parts.push("输出规范要求：{{VarDivRender}}".to_string());
    }

    // 3.4 移动端 Surface 挂件能力注入
    if enable_mobile_surface_injection {
        bottom_parts.push(MOBILE_SURFACE_PROMPT.to_string());
    }

    // 3.5 移动端浏览器控制能力注入
    if enable_mobile_browser_injection {
        bottom_parts.push(MOBILE_BROWSER_PROMPT.to_string());
    }

    // 应用注入到 System Message
    if !top_parts.is_empty() || !bottom_parts.is_empty() {
        for m in messages.iter_mut() {
            if m["role"] == "system" {
                let original_content = m["content"].as_str().unwrap_or("");
                let mut final_parts = Vec::new();
                if !top_parts.is_empty() {
                    final_parts.push(top_parts.join("\n"));
                }
                if !original_content.is_empty() {
                    final_parts.push(original_content.to_string());
                }
                if !bottom_parts.is_empty() {
                    final_parts.push(bottom_parts.join("\n"));
                }
                m["content"] = json!(final_parts.join("\n\n").trim());
                break;
            }
        }
    }

    // === 4. 准备请求体 ===
    let is_stream = payload.model_config["stream"].as_bool().unwrap_or(false);
    let mut request_body = payload.model_config.clone();
    let model_name = request_body
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default();
    let emit_reasoning_output = should_emit_reasoning_output(&model_name);
    if enable_model_thinking {
        apply_model_thinking_params(&mut request_body, &model_name, model_thinking_budget);
    }
    if let Some(obj) = request_body.as_object_mut() {
        obj.insert("messages".to_string(), json!(messages));
        obj.insert("requestId".to_string(), json!(payload.message_id));
        obj.insert("stream".to_string(), json!(is_stream));
    }

    // === 5. 配置网络请求 ===
    let client = Client::builder()
        // 移除硬超时限制，对齐桌面端，让长思考模型有充足时间生成，连接管理交给 VCP 服务器
        .build()
        .map_err(|e| e.to_string())?;

    // 创建并注册中止信号
    let (abort_tx, abort_rx) = oneshot::channel();
    active_requests.insert(
        payload.message_id.clone(),
        ActiveRequestHandle {
            abort_sender: abort_tx,
            vcp_url: payload.vcp_url.clone(),
            vcp_api_key: payload.vcp_api_key.clone(),
        },
    );

    let message_id = payload.message_id.clone();
    let context = payload.context.clone();
    let api_key = payload.vcp_api_key.clone();

    if is_stream {
        // === 6. 流式处理模式 (同步等待，以便串行调用) ===
        let app_handle = app.clone();
        let message_id_inner = message_id.clone();
        let context_inner = context.clone();
        let active_requests_inner = active_requests.clone();

        let mut full_content = String::new();
        let mut pending_emit_text = String::new();
        let mut reasoning_block_open = false;
        let mut last_emit_at = Instant::now();
        let mut is_aborted = false;
        let mut abort_rx = abort_rx; // 取得所有权进入循环

        let res_future = client
            .post(&final_url)
            .header(AUTHORIZATION, format!("Bearer {}", api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&request_body)
            .send();

        tokio::select! {
            _ = &mut abort_rx => {
                println!("[VCPClient] Request aborted before response for message: {}", message_id_inner);
                let _ = app_handle.emit(&stream_channel, StreamEvent {
                    r#type: "end".to_string(),
                    chunk: Some(json!({ "fullContent": full_content.clone() })),
                    message_id: message_id_inner.clone(),
                    context: context_inner.clone(),
                    error: Some("请求已中止".to_string()),
                });
                active_requests_inner.remove(&message_id_inner);
                return Ok((json!({ "fullContent": "", "streamingStarted": false }), true));
            }
            response_res = res_future => {
                match response_res {
                    Ok(resp) if resp.status().is_success() => {
                        let stream = resp.bytes_stream().map_err(IoError::other);
                        let reader = StreamReader::new(stream);
                        let mut lines = FramedRead::new(reader, LinesCodec::new());

                        loop {
                            tokio::select! {
                                // 核心修复：即使在等待数据的间隙，也能捕获中断信号
                                _ = &mut abort_rx => {
                                    is_aborted = true;
                                    println!("[VCPClient] Stream deep-polling detected abort for message: {}", message_id_inner);
                                    close_reasoning_block(
                                        &mut full_content,
                                        &mut pending_emit_text,
                                        &mut reasoning_block_open,
                                    );
                                    emit_pending_stream_text(
                                        &app_handle,
                                        &stream_channel,
                                        &message_id_inner,
                                        &context_inner,
                                        &mut pending_emit_text,
                                        &mut last_emit_at,
                                        true,
                                    );
                                    let _ = app_handle.emit(&stream_channel, StreamEvent {
                                        r#type: "end".to_string(),
                                        chunk: Some(json!({ "fullContent": full_content.clone() })),
                                        message_id: message_id_inner.clone(),
                                        context: context_inner.clone(),
                                        error: Some("请求已中止".to_string()),
                                    });
                                    // 显式清理，防止 race
                                    active_requests_inner.remove(&message_id_inner);
                                    break;
                                }
                                line_res = lines.next() => {
                                    match line_res {
                                        Some(Ok(line)) => {
                                            if line.trim().is_empty() { continue; }
                                            if line.starts_with("data: ") {
                                                let data = line.trim_start_matches("data: ").trim();
                                                if data == "[DONE]" {
                                                    close_reasoning_block(
                                                        &mut full_content,
                                                        &mut pending_emit_text,
                                                        &mut reasoning_block_open,
                                                    );
                                                    emit_pending_stream_text(
                                                        &app_handle,
                                                        &stream_channel,
                                                        &message_id_inner,
                                                        &context_inner,
                                                        &mut pending_emit_text,
                                                        &mut last_emit_at,
                                                        true,
                                                    );
                                                    let _ = app_handle.emit(&stream_channel, StreamEvent {
                                                        r#type: "end".to_string(),
                                                        chunk: Some(json!({ "fullContent": full_content.clone() })),
                                                        message_id: message_id_inner.clone(),
                                                        context: context_inner.clone(),
                                                        error: None,
                                                    });
                                                    break;
                                                }
                                                if let Ok(chunk) = serde_json::from_str::<Value>(data) {
                                                    let stream_parts = extract_stream_text_parts(&chunk);
                                                    if emit_reasoning_output && !stream_parts.reasoning.is_empty() {
                                                        push_stream_segment(
                                                            &mut full_content,
                                                            &mut pending_emit_text,
                                                            &mut reasoning_block_open,
                                                            &stream_parts.reasoning,
                                                            true,
                                                        );
                                                    }
                                                    if !stream_parts.answer.is_empty() {
                                                        push_stream_segment(
                                                            &mut full_content,
                                                            &mut pending_emit_text,
                                                            &mut reasoning_block_open,
                                                            &stream_parts.answer,
                                                            false,
                                                        );
                                                    }
                                                    if !stream_parts.reasoning.is_empty() || !stream_parts.answer.is_empty() {
                                                        emit_pending_stream_text(
                                                            &app_handle,
                                                            &stream_channel,
                                                            &message_id_inner,
                                                            &context_inner,
                                                            &mut pending_emit_text,
                                                            &mut last_emit_at,
                                                            false,
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        Some(Err(e)) => {
                                            println!("[VCPClient] Stream read error: {:?}", e);
                                            close_reasoning_block(
                                                &mut full_content,
                                                &mut pending_emit_text,
                                                &mut reasoning_block_open,
                                            );
                                            emit_pending_stream_text(
                                                &app_handle,
                                                &stream_channel,
                                                &message_id_inner,
                                                &context_inner,
                                                &mut pending_emit_text,
                                                &mut last_emit_at,
                                                true,
                                            );
                                            let _ = app_handle.emit(&stream_channel, StreamEvent {
                                                r#type: "error".to_string(),
                                                chunk: Some(json!({ "fullContent": full_content.clone() })),
                                                message_id: message_id_inner.clone(),
                                                context: context_inner.clone(),
                                                error: Some(format!("流读取错误: {}", e)),
                                            });
                                            break;
                                        }
                                        None => {
                                            println!("[VCPClient] Stream ended unexpectedly (None)");
                                            close_reasoning_block(
                                                &mut full_content,
                                                &mut pending_emit_text,
                                                &mut reasoning_block_open,
                                            );
                                            emit_pending_stream_text(
                                                &app_handle,
                                                &stream_channel,
                                                &message_id_inner,
                                                &context_inner,
                                                &mut pending_emit_text,
                                                &mut last_emit_at,
                                                true,
                                            );
                                            let _ = app_handle.emit(&stream_channel, StreamEvent {
                                                r#type: "error".to_string(),
                                                chunk: Some(json!({ "fullContent": full_content.clone() })),
                                                message_id: message_id_inner.clone(),
                                                context: context_inner.clone(),
                                                error: Some("网络连接意外断开".to_string()),
                                            });
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
                        let _ = app_handle.emit(&stream_channel, StreamEvent {
                            r#type: "error".to_string(),
                            chunk: Some(json!({ "fullContent": full_content.clone() })),
                            message_id: message_id_inner.clone(),
                            context: context_inner.clone(),
                            error: Some(format!("VCP服务器错误: {} - {}", status, text)),
                        });
                        active_requests_inner.remove(&message_id_inner);
                        return Err(format!("VCP Error: {}", status));
                    }
                    Err(e) => {
                        let _ = app_handle.emit(&stream_channel, StreamEvent {
                            r#type: "error".to_string(),
                            chunk: Some(json!({ "fullContent": full_content.clone() })),
                            message_id: message_id_inner.clone(),
                            context: context_inner.clone(),
                            error: Some(format!("网络请求异常: {}", e)),
                        });
                        active_requests_inner.remove(&message_id_inner);
                        return Err(e.to_string());
                    }
                }
            }
        }

        active_requests_inner.remove(&message_id_inner);
        Ok((
            json!({ "fullContent": full_content, "streamingStarted": true }),
            is_aborted,
        ))
    } else {
        // === 7. 非流式响应模式 ===
        let response = client
            .post(&final_url)
            .header(AUTHORIZATION, format!("Bearer {}", api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("VCP请求失败: {}", e))?;

        active_requests.remove(&message_id);

        if !response.status().is_success() {
            let status = response.status();
            return Err(format!("VCP响应错误: {}", status));
        }

        let vcp_response = response
            .json::<Value>()
            .await
            .map_err(|e| format!("JSON解析失败: {}", e))?;
        Ok((json!({"response": vcp_response, "context": context}), false))
    }
}

/// Normalize a VCP server URL by appending `/v1/chat/completions` if missing.
pub fn normalize_vcp_url(url_str: &str) -> String {
    if let Ok(url) = Url::parse(url_str) {
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
    println!(
        "[VCPClient] interruptRequest called for messageId: {}. Active requests: {}",
        message_id,
        state.0.len()
    );
    if let Some((_, handle)) = state.0.remove(&message_id) {
        println!(
            "[VCPClient] Found active request for messageId: {}, aborting locally and notifying VCP backend...",
            message_id
        );

        let interrupt_message_id = message_id.clone();
        let interrupt_url = handle.vcp_url.clone();
        let interrupt_api_key = handle.vcp_api_key.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) =
                notify_vcp_interrupt(&interrupt_url, &interrupt_api_key, &interrupt_message_id)
                    .await
            {
                eprintln!(
                    "[VCPClient] Failed to notify VCP backend interrupt for {}: {}",
                    interrupt_message_id, err
                );
            }
        });

        let _ = handle.abort_sender.send(());
        println!(
            "[VCPClient] Request interrupted for messageId: {}. Remaining active requests: {}",
            message_id,
            state.0.len()
        );
        Ok(json!({"success": true, "message": format!("Request {} interrupted", message_id)}))
    } else {
        println!(
            "[VCPClient] No active request found for messageId: {}",
            message_id
        );
        Err(format!("Request {} not found", message_id))
    }
}

async fn notify_vcp_interrupt(
    vcp_url: &str,
    vcp_api_key: &str,
    message_id: &str,
) -> Result<(), String> {
    let mut url = Url::parse(vcp_url).map_err(|e| format!("Invalid VCP URL: {}", e))?;
    url.set_path("/v1/interrupt");
    url.set_query(None);
    url.set_fragment(None);

    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .post(url)
        .header(AUTHORIZATION, format!("Bearer {}", vcp_api_key))
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({
            "requestId": message_id,
            "messageId": message_id,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("VCP interrupt returned HTTP {}", response.status()))
    }
}

/// 测试 VCP 后端连接状态并获取模型列表 (对齐桌面端 main.js fetchAndCacheModels 逻辑)
#[tauri::command]
pub async fn test_vcp_connection(vcp_url: String, vcp_api_key: String) -> Result<Value, String> {
    println!(
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

    println!(
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

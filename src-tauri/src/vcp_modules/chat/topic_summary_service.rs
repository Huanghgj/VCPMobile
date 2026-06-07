use crate::vcp_modules::message_repository::ContentCompressor;
use crate::vcp_modules::settings_manager::{read_settings, SettingsState};
use crate::vcp_modules::sync_hash::HashAggregator;
use crate::vcp_modules::sync_service::{SyncCommand, SyncState};
use crate::vcp_modules::sync_types::SyncDataType;
use crate::vcp_modules::vcp_client::normalize_vcp_url;
use regex::Regex;
use reqwest::Client;
use serde_json::{json, Value};
use sqlx::{Pool, Row, Sqlite};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

/// 话题总结的默认 Prompt
const DEFAULT_SUMMARY_PROMPT: &str = "请根据以上对话内容，仅返回一个简洁的话题标题。要求：1. 标题长度控制在10个汉字以内。2. 标题本身不能包含任何标点符号、数字编号 or 任何非标题文字。3. 直接给出标题文字，不要添加任何解释或前缀。";

/// 话题总结的默认模型
const DEFAULT_SUMMARY_MODEL: &str = "gemini-3.1-flash-lite";

/// 注：不主动发送 temperature 参数，以兼容 o1/Gemini thinking 等不支持该参数的模型
/// AI 请求超时时间 (秒)
const AI_REQUEST_TIMEOUT_SECS: u64 = 30;

/// AI 响应最大 Token 数（话题标题≤12汉字，64 token 绰绰有余）
const AI_MAX_TOKENS: u32 = 64;

const MIN_MESSAGES_FOR_AUTO_SUMMARY: i32 = 4;

lazy_static::lazy_static! {
    static ref AUTO_SUMMARY_IN_FLIGHT: dashmap::DashSet<String> = dashmap::DashSet::new();
}

struct AutoSummaryInFlightGuard {
    topic_id: String,
}

impl AutoSummaryInFlightGuard {
    fn acquire(topic_id: &str) -> Option<Self> {
        if AUTO_SUMMARY_IN_FLIGHT.insert(topic_id.to_string()) {
            Some(Self {
                topic_id: topic_id.to_string(),
            })
        } else {
            None
        }
    }
}

impl Drop for AutoSummaryInFlightGuard {
    fn drop(&mut self) {
        AUTO_SUMMARY_IN_FLIGHT.remove(&self.topic_id);
    }
}

fn is_default_topic_title(title: &str) -> bool {
    lazy_static::lazy_static! {
        static ref DEFAULT_TOPIC_TITLE_RE: Regex = Regex::new(
            r"^(新话题|新会话)(?:\s+(?:\d{1,2}:\d{2}:\d{2}(?:\s?(?:AM|PM|am|pm|上午|下午))?|(?:AM|PM|am|pm|上午|下午)\s*\d{1,2}:\d{2}:\d{2}))?$"
        )
        .unwrap();
    }

    DEFAULT_TOPIC_TITLE_RE.is_match(title.trim())
}

async fn load_recent_topic_messages(
    db_pool: &Pool<Sqlite>,
    topic_id: &str,
    limit: i64,
) -> Result<Vec<(String, String)>, String> {
    let rows = sqlx::query(
        "SELECT role, content FROM messages
         WHERE topic_id = ? AND deleted_at IS NULL
         ORDER BY timestamp DESC, rowid DESC
         LIMIT ?",
    )
    .bind(topic_id)
    .bind(limit)
    .fetch_all(db_pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut messages = Vec::with_capacity(rows.len());
    for row in rows.into_iter().rev() {
        let role: String = row.get("role");
        let content_bytes: Vec<u8> = row.get("content");
        let content = ContentCompressor::decompress(&content_bytes).unwrap_or_default();
        messages.push((role, content));
    }

    Ok(messages)
}

pub async fn summarize_topic_if_needed<R: Runtime>(
    app_handle: AppHandle<R>,
    db_pool: Pool<Sqlite>,
    owner_id: String,
    owner_type: String,
    topic_id: String,
    agent_name: String,
) {
    let Some(_in_flight_guard) = AutoSummaryInFlightGuard::acquire(&topic_id) else {
        return;
    };

    let topic_row = match sqlx::query(
        "SELECT title, msg_count FROM topics WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(&topic_id)
    .fetch_optional(&db_pool)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            log::warn!(
                "[TopicSummary] Failed to inspect topic {} before auto summary: {}",
                topic_id,
                e
            );
            return;
        }
    };

    let Some(row) = topic_row else {
        return;
    };

    let title: String = row.get("title");
    let msg_count: i32 = row.get("msg_count");
    if msg_count < MIN_MESSAGES_FOR_AUTO_SUMMARY || !is_default_topic_title(&title) {
        return;
    }

    let settings_state = app_handle.state::<SettingsState>();
    let summary_title = match summarize_topic(
        app_handle.clone(),
        settings_state,
        owner_id.clone(),
        owner_type.clone(),
        topic_id.clone(),
        agent_name,
    )
    .await
    {
        Ok(title) => title,
        Err(e) => {
            log::warn!(
                "[TopicSummary] Auto summary failed for topic {}: {}",
                topic_id,
                e
            );
            return;
        }
    };

    let mut tx = match db_pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::warn!(
                "[TopicSummary] Failed to begin hash update for topic {}: {}",
                topic_id,
                e
            );
            return;
        }
    };

    let latest_title = match sqlx::query_scalar::<_, String>(
        "SELECT title FROM topics WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(&topic_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(title) => title,
        Err(e) => {
            log::warn!(
                "[TopicSummary] Failed to re-check topic {} before persisting summary: {}",
                topic_id,
                e
            );
            return;
        }
    };

    let Some(latest_title) = latest_title else {
        return;
    };

    if !is_default_topic_title(&latest_title) {
        return;
    }

    let now = crate::vcp_modules::infra::utils::now_millis();
    let update_result = match sqlx::query(
        "UPDATE topics SET title = ?, updated_at = ?
         WHERE topic_id = ? AND deleted_at IS NULL AND title = ?",
    )
    .bind(&summary_title)
    .bind(now)
    .bind(&topic_id)
    .bind(&latest_title)
    .execute(&mut *tx)
    .await
    {
        Ok(result) => result,
        Err(e) => {
            log::warn!(
                "[TopicSummary] Failed to persist auto summary for topic {}: {}",
                topic_id,
                e
            );
            return;
        }
    };

    if update_result.rows_affected() == 0 {
        return;
    }

    if let Err(e) = HashAggregator::bubble_from_topic(&mut tx, &topic_id).await {
        log::warn!(
            "[TopicSummary] Failed to bubble summary hash for topic {}: {}",
            topic_id,
            e
        );
        return;
    }
    if let Err(e) = tx.commit().await {
        log::warn!(
            "[TopicSummary] Failed to commit summary hash for topic {}: {}",
            topic_id,
            e
        );
        return;
    }

    if let Some(sync_state) = app_handle.try_state::<SyncState>() {
        match sqlx::query(
            "SELECT config_hash, owner_type FROM topics WHERE topic_id = ? AND deleted_at IS NULL",
        )
        .bind(&topic_id)
        .fetch_optional(&db_pool)
        .await
        {
            Ok(Some(row)) => {
                let hash: String = row.get("config_hash");
                let db_owner_type: String = row.get("owner_type");
                let _ = sync_state.ws_sender.send(SyncCommand::NotifyLocalChange {
                    data_type: SyncDataType::Topic,
                    id: topic_id.clone(),
                    hash,
                    ts: now,
                    owner_type: Some(db_owner_type),
                });
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!(
                    "[TopicSummary] Failed to fetch summary hash for sync notification: {}",
                    e
                );
            }
        }
    }

    let _ = app_handle.emit(
        "topic-title-updated",
        json!({
            "topicId": topic_id,
            "ownerId": owner_id,
            "ownerType": owner_type,
            "title": summary_title,
            "updatedAt": now,
        }),
    );
}

pub async fn summarize_topic<R: Runtime>(
    app_handle: AppHandle<R>,
    settings_state: State<'_, SettingsState>,
    _owner_id: String,
    _owner_type: String,
    topic_id: String,
    agent_name: String,
) -> Result<String, String> {
    let settings = read_settings(app_handle.clone(), settings_state).await?;
    if settings.vcp_server_url.is_empty() || settings.vcp_api_key.is_empty() {
        return Err("VCP settings are missing".to_string());
    }

    // 1. 获取最近消息 (最近4条)。标题总结只需要纯文本，避免触发 UI 渲染/附件加载。
    let db_state = app_handle.state::<crate::vcp_modules::db_manager::DbState>();
    let messages = load_recent_topic_messages(&db_state.pool, &topic_id, 4).await?;

    if messages.len() < 2 {
        return Err("Not enough messages to summarize".to_string());
    }

    let mut recent_content = String::new();
    for (role, content) in messages {
        let role_name = if role == "user" {
            settings.user_name.as_str()
        } else {
            agent_name.as_str()
        };
        recent_content.push_str(&format!("{}: {}\n", role_name, content));
    }

    // 2. 构造 Prompt
    let summary_prompt = format!(
        "[待总结聊天记录: {}]\n{}",
        recent_content, DEFAULT_SUMMARY_PROMPT
    );

    // 3. 调用 AI
    let client = Client::builder()
        .timeout(Duration::from_secs(AI_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| e.to_string())?;

    let model = if settings.topic_summary_model.is_empty() {
        DEFAULT_SUMMARY_MODEL.to_string()
    } else {
        settings.topic_summary_model
    };

    let vcp_url = normalize_vcp_url(&settings.vcp_server_url);
    let response = client
        .post(&vcp_url)
        .header("Authorization", format!("Bearer {}", settings.vcp_api_key))
        .header("Content-Type", "application/json")
        .json(&json!({
            "messages": [{"role": "user", "content": summary_prompt}],
            "model": model,
            "max_tokens": AI_MAX_TOKENS,
            "stream": false
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!("AI request failed: {}", response.status()));
    }

    let res_json: Value = response.json().await.map_err(|e| e.to_string())?;
    let raw_title = res_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim();

    // 4. 清洗标题
    let clean_title = clean_summarized_title(raw_title);

    if clean_title.is_empty() {
        return Err("AI failed to generate a valid title".to_string());
    }

    Ok(clean_title)
}

pub fn clean_summarized_title(raw: &str) -> String {
    let first_line = raw.lines().next().unwrap_or("").trim();

    let mut cleaned = first_line
        .replace(|c: char| !c.is_alphanumeric() && !c.is_whitespace(), "")
        .replace("标题", "")
        .replace("总结", "")
        .replace("Topic", "")
        .replace(":", "")
        .replace("：", "")
        .trim()
        .to_string();

    cleaned = cleaned.replace(char::is_whitespace, "");

    let char_count = cleaned.chars().count();
    if char_count > 12 {
        cleaned.chars().take(12).collect()
    } else {
        cleaned
    }
}

use crate::vcp_modules::message_repository::ContentCompressor;
use crate::vcp_modules::settings_manager::{read_settings, SettingsState};
use crate::vcp_modules::sync_hash::HashAggregator;
use crate::vcp_modules::sync_service::{SyncCommand, SyncState};
use crate::vcp_modules::sync_types::SyncDataType;
use crate::vcp_modules::vcp_client::normalize_vcp_url;
use regex::Regex;
use reqwest::Client;
use serde_json::{json, Value};
use sqlx::{Pool, Row, Sqlite, Transaction};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

/// 话题总结的默认 Prompt
const DEFAULT_SUMMARY_PROMPT: &str = "请根据以上对话内容，仅返回一个简洁的话题标题。要求：1. 标题长度控制在10个汉字以内。2. 标题本身不能包含任何标点符号、数字编号 or 任何非标题文字。3. 直接给出标题文字，不要添加任何解释或前缀。";

/// 话题总结的默认模型
const DEFAULT_SUMMARY_MODEL: &str = "gemini-2.5-flash";

/// 注：不主动发送 temperature 参数，以兼容 o1/Gemini thinking 等不支持该参数的模型
/// AI 请求超时时间 (秒)
const AI_REQUEST_TIMEOUT_SECS: u64 = 30;

const MIN_MESSAGES_FOR_AUTO_SUMMARY: i32 = 2;
const MAX_SUMMARY_CONTEXT_CHARS: usize = 24_000;

#[derive(Debug, Clone)]
struct TopicSummaryTarget {
    title: String,
    owner_id: String,
    owner_type: String,
}

#[derive(Debug, Clone)]
struct PersistedTopicSummary {
    topic_id: String,
    owner_id: String,
    owner_type: String,
    title: String,
    updated_at: i64,
}

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

pub fn is_default_topic_title(title: &str) -> bool {
    lazy_static::lazy_static! {
        static ref DEFAULT_TOPIC_TITLE_RE: Regex = Regex::new(
            r"^(新话题|新会话)(?:\s+(?:(?:\d{4}[-/年]\d{1,2}[-/月]\d{1,2}日?\s+)?\d{1,2}:\d{2}(?::\d{2})?(?:\s?(?:AM|PM|am|pm|上午|下午))?|(?:AM|PM|am|pm|上午|下午)\s*\d{1,2}:\d{2}(?::\d{2})?|\d{1,2}[-/月]\d{1,2}日?\s+\d{1,2}:\d{2}(?::\d{2})?))?$"
        )
        .unwrap();
    }

    DEFAULT_TOPIC_TITLE_RE.is_match(title.trim())
}

async fn update_summarized_title_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    topic_id: &str,
    expected_current_title: Option<&str>,
    title: &str,
) -> Result<Option<PersistedTopicSummary>, String> {
    let target_row = sqlx::query(
        "SELECT title, owner_id, owner_type FROM topics
         WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;

    let Some(target_row) = target_row else {
        return Ok(None);
    };
    let target = TopicSummaryTarget {
        title: target_row.get("title"),
        owner_id: target_row.get("owner_id"),
        owner_type: target_row.get("owner_type"),
    };

    if let Some(expected_title) = expected_current_title {
        if target.title != expected_title {
            return Ok(None);
        }
    }

    let now = crate::vcp_modules::infra::utils::now_millis();
    let update_result = sqlx::query(
        "UPDATE topics SET title = ?, updated_at = ?
         WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(title)
    .bind(now)
    .bind(topic_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    if update_result.rows_affected() == 0 {
        return Ok(None);
    }

    HashAggregator::bubble_from_topic(tx, topic_id).await?;

    Ok(Some(PersistedTopicSummary {
        topic_id: topic_id.to_string(),
        owner_id: target.owner_id,
        owner_type: target.owner_type,
        title: title.to_string(),
        updated_at: now,
    }))
}

async fn emit_persisted_topic_summary<R: Runtime>(
    app_handle: AppHandle<R>,
    db_pool: &Pool<Sqlite>,
    persisted: &PersistedTopicSummary,
) {
    if let Some(sync_state) = app_handle.try_state::<SyncState>() {
        let row = sqlx::query(
            "SELECT config_hash, owner_type FROM topics WHERE topic_id = ? AND deleted_at IS NULL",
        )
        .bind(&persisted.topic_id)
        .fetch_optional(db_pool)
        .await
        .map_err(|e| e.to_string());

        match row {
            Ok(Some(row)) => {
                let hash: String = row.get("config_hash");
                let db_owner_type: String = row.get("owner_type");
                let _ = sync_state.ws_sender.send(SyncCommand::NotifyLocalChange {
                    data_type: SyncDataType::Topic,
                    id: persisted.topic_id.clone(),
                    hash,
                    ts: persisted.updated_at,
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
            "topicId": persisted.topic_id,
            "ownerId": persisted.owner_id,
            "ownerType": persisted.owner_type,
            "title": persisted.title,
            "updatedAt": persisted.updated_at,
        }),
    );
}

async fn load_topic_messages_for_summary(
    db_pool: &Pool<Sqlite>,
    topic_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let rows = sqlx::query(
        "SELECT role, content FROM messages
         WHERE topic_id = ? AND deleted_at IS NULL
         ORDER BY timestamp ASC, rowid ASC",
    )
    .bind(topic_id)
    .fetch_all(db_pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut messages = Vec::with_capacity(rows.len());
    for row in rows {
        let role: String = row.get("role");
        if role == "system" {
            continue;
        }
        let content_bytes: Vec<u8> = row.get("content");
        let content = ContentCompressor::decompress(&content_bytes).map_err(|error| {
            format!("Failed to decompress a message in topic {topic_id}: {error}")
        })?;
        let content = content.trim();
        if !content.is_empty() {
            messages.push((role, content.to_string()));
        }
    }

    Ok(messages)
}

fn build_summary_context(
    messages: Vec<(String, String)>,
    user_name: &str,
    agent_name: &str,
) -> String {
    let mut rendered: Vec<String> = messages
        .into_iter()
        .map(|(role, content)| {
            let role_name = if role == "user" {
                user_name
            } else {
                agent_name
            };
            format!("{}: {}", role_name, content)
        })
        .collect();

    let full = rendered.join("\n");
    if full.chars().count() <= MAX_SUMMARY_CONTEXT_CHARS {
        return full;
    }

    let mut kept = Vec::new();
    let mut total = 0usize;
    while let Some(item) = rendered.pop() {
        let item_len = item.chars().count() + 1;
        if total + item_len > MAX_SUMMARY_CONTEXT_CHARS {
            break;
        }
        total += item_len;
        kept.push(item);
    }
    kept.reverse();

    format!(
        "[前方较早聊天记录因长度限制已省略，以下为本话题最近的完整连续片段]\n{}",
        kept.join("\n")
    )
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

    let persisted = match update_summarized_title_in_tx(
        &mut tx,
        &topic_id,
        Some(&latest_title),
        &summary_title,
    )
    .await
    {
        Ok(Some(persisted)) => persisted,
        Ok(None) => return,
        Err(e) => {
            log::warn!(
                "[TopicSummary] Failed to persist auto summary for topic {}: {}",
                topic_id,
                e
            );
            return;
        }
    };

    if let Err(e) = tx.commit().await {
        log::warn!(
            "[TopicSummary] Failed to commit summary hash for topic {}: {}",
            topic_id,
            e
        );
        return;
    }

    emit_persisted_topic_summary(app_handle, &db_pool, &persisted).await;
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

    // 1. 获取该话题的完整纯文本消息。标题总结不依赖前端当前分页状态。
    let db_state = app_handle.state::<crate::vcp_modules::db_manager::DbState>();
    let messages = load_topic_messages_for_summary(&db_state.pool, &topic_id).await?;

    if messages.len() < 2 {
        return Err("Not enough messages to summarize".to_string());
    }

    let summary_context =
        build_summary_context(messages, settings.user_name.as_str(), agent_name.as_str());

    // 2. 构造 Prompt
    let summary_prompt = format!(
        "[待总结聊天记录: {}]\n{}",
        summary_context, DEFAULT_SUMMARY_PROMPT
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

pub async fn summarize_and_update_topic<R: Runtime>(
    app_handle: AppHandle<R>,
    settings_state: State<'_, SettingsState>,
    owner_id: String,
    owner_type: String,
    topic_id: String,
    agent_name: String,
) -> Result<String, String> {
    let title = summarize_topic(
        app_handle.clone(),
        settings_state,
        owner_id,
        owner_type,
        topic_id.clone(),
        agent_name,
    )
    .await?;

    let db_pool = app_handle
        .state::<crate::vcp_modules::db_manager::DbState>()
        .pool
        .clone();
    let mut tx = db_pool.begin().await.map_err(|e| e.to_string())?;
    let persisted = update_summarized_title_in_tx(&mut tx, &topic_id, None, &title).await?;
    tx.commit().await.map_err(|e| e.to_string())?;

    if let Some(persisted) = persisted {
        emit_persisted_topic_summary(app_handle, &db_pool, &persisted).await;
    }

    Ok(title)
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

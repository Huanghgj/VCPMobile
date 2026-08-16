use crate::vcp_modules::chat_manager::{Attachment, ChatMessage};
use crate::vcp_modules::content_parser::ContentBlock;
use crate::vcp_modules::file_manager::get_attachments_root_dir;
use crate::vcp_modules::message_repository::MessageRepository;
use crate::vcp_modules::message_repository::{ContentCompressor, MessageRenderCompiler};
use crate::vcp_modules::render_repair::repair_message_content_before_persist;
use crate::vcp_modules::settings_manager;
use crate::vcp_modules::sync_hash::HashAggregator;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::path::Path;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager};
use tokio::fs;
use tokio::io::AsyncWriteExt;

// =================================================================
// vcp_modules/message_service.rs - 消息业务逻辑中心 (含附件对齐)
// =================================================================

fn repair_assistant_render_content_before_persist(message: &mut ChatMessage) {
    if message.role != "assistant" || message.content.is_empty() {
        return;
    }

    let repaired = repair_message_content_before_persist(&message.content);
    if repaired != message.content {
        log::warn!(
            "[MessageService] Repaired unclosed render HTML before persisting message {}",
            message.id
        );
        message.content = repaired;
        message.blocks = None;
    }
}

fn compute_message_hash_from_content(content: &str, attachments: Option<&[Attachment]>) -> String {
    let attachment_hashes: Vec<String> = attachments
        .map(|atts| {
            atts.iter()
                .filter_map(|att| att.hash.as_deref())
                .filter(|hash| !hash.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    HashAggregator::compute_message_fingerprint(content, &attachment_hashes)
}

fn decompress_message_content(
    content_bytes: &[u8],
    topic_id: &str,
    message_id: &str,
) -> Result<String, String> {
    ContentCompressor::decompress(content_bytes).map_err(|error| {
        format!("Failed to decompress message {message_id} in topic {topic_id}: {error}")
    })
}

pub(crate) async fn ensure_active_topic_owner(
    pool: &sqlx::SqlitePool,
    topic_id: &str,
    owner_id: &str,
    owner_type: &str,
) -> Result<(), String> {
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM topics \
         WHERE topic_id = ? AND owner_id = ? AND owner_type = ? AND deleted_at IS NULL)",
    )
    .bind(topic_id)
    .bind(owner_id)
    .bind(owner_type)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    if active {
        Ok(())
    } else {
        Err(format!(
            "话题 {} 不存在、已删除或不属于 {} {}",
            topic_id, owner_type, owner_id
        ))
    }
}

async fn ensure_active_topic(pool: &sqlx::SqlitePool, topic_id: &str) -> Result<(), String> {
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM topics WHERE topic_id = ? AND deleted_at IS NULL)",
    )
    .bind(topic_id)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    if active {
        Ok(())
    } else {
        Err(format!("话题 {} 不存在或已删除", topic_id))
    }
}

/// Writes a lightweight assistant placeholder for an active stream.
///
/// This intentionally skips render_cache and attachment cleanup. The final stream
/// commit performs the full durable write, but the placeholder is still reflected
/// in topic counts/hash so interrupted streams remain consistent after restart.
pub async fn append_stream_skeleton_message(
    db_pool: &sqlx::Pool<sqlx::Sqlite>,
    topic_id: String,
    message: ChatMessage,
) -> Result<(), String> {
    if topic_id.trim().is_empty() || message.id.trim().is_empty() {
        return Err("stream skeleton requires topic_id and message.id".to_string());
    }

    let timestamp = if message.timestamp == 0 {
        chrono::Utc::now().timestamp_millis() as u64
    } else {
        message.timestamp
    };
    let content_hash = HashAggregator::compute_message_fingerprint("", &[]);
    let compressed_empty = ContentCompressor::compress("")?;

    let mut tx = db_pool.begin().await.map_err(|e| e.to_string())?;

    let result = sqlx::query(
        "INSERT INTO messages (
            msg_id, topic_id, role, name, agent_id, content, timestamp,
            is_group_message, group_id, finish_reason,
            content_hash,
            created_at, updated_at
        ) SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
          WHERE EXISTS (SELECT 1 FROM topics WHERE topic_id = ? AND deleted_at IS NULL)
         ON CONFLICT(topic_id, msg_id) DO NOTHING",
    )
    .bind(&message.id)
    .bind(&topic_id)
    .bind(if message.role.is_empty() {
        "assistant"
    } else {
        message.role.as_str()
    })
    .bind(&message.name)
    .bind(&message.agent_id)
    .bind(compressed_empty)
    .bind(timestamp as i64)
    .bind(message.is_group_message.unwrap_or(false))
    .bind(&message.group_id)
    .bind(&message.finish_reason)
    .bind(content_hash)
    .bind(timestamp as i64)
    .bind(timestamp as i64)
    .bind(&topic_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    if result.rows_affected() > 0 {
        let msg_count: i32 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE topic_id = ? AND deleted_at IS NULL",
        )
        .bind(&topic_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or(0);

        sqlx::query(
            "UPDATE topics SET updated_at = ?, msg_count = ? \
             WHERE topic_id = ? AND deleted_at IS NULL",
        )
        .bind(timestamp as i64)
        .bind(msg_count)
        .bind(&topic_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        HashAggregator::bubble_from_topic(&mut tx, &topic_id).await?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// 批量加载多个 topic 的全量消息 — 一次性 SQL 查询，按 topic_id 分组
/// 避免 push_messages_batch 场景下的 N+1 查询
pub async fn load_multi_topic_messages(
    pool: &sqlx::SqlitePool,
    topic_ids: &[String],
) -> Result<
    std::collections::HashMap<String, Vec<crate::vcp_modules::chat_manager::ChatMessage>>,
    String,
> {
    use sqlx::Row;
    let mut result: std::collections::HashMap<
        String,
        Vec<crate::vcp_modules::chat_manager::ChatMessage>,
    > = topic_ids
        .iter()
        .map(|id| (id.clone(), Vec::new()))
        .collect();

    if topic_ids.is_empty() {
        return Ok(result);
    }

    let placeholders = topic_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let query_str = format!(
        "SELECT m.msg_id, m.role, m.name, m.agent_id, m.content, m.timestamp, m.is_group_message, m.group_id, m.finish_reason, r.render_content, r.content_hash AS render_content_hash, m.topic_id, m.content_hash
         FROM messages m
         LEFT JOIN render_cache r ON m.topic_id = r.topic_id AND m.msg_id = r.msg_id
         WHERE m.topic_id IN ({}) AND m.deleted_at IS NULL
         ORDER BY m.topic_id, m.timestamp ASC, m.msg_id ASC",
        placeholders
    );

    let mut q = sqlx::query(&query_str);
    for id in topic_ids {
        q = q.bind(id);
    }
    let rows = q.fetch_all(pool).await.map_err(|e| e.to_string())?;

    for row in rows {
        let msg_id: String = row.get("msg_id");
        let role: String = row.get("role");
        let topic_id: String = row.get("topic_id");
        let timestamp: i64 = row.get("timestamp");
        let render_content: Option<Vec<u8>> = row.get("render_content");
        let render_content_hash: Option<String> = row.get("render_content_hash");

        let content_bytes: Vec<u8> = row.get("content");
        let content = decompress_message_content(&content_bytes, &topic_id, &msg_id)?;
        let content_hash_raw: String = row.get("content_hash");
        let content_hash = if content_hash_raw.is_empty() {
            None
        } else {
            Some(content_hash_raw)
        };
        let blocks = match (&render_content, &render_content_hash, &content_hash) {
            (Some(bytes), Some(render_hash), Some(message_hash))
                if !render_hash.is_empty()
                    && MessageRenderCompiler::cache_matches(render_hash, message_hash) =>
            {
                parse_render_bytes(Some(bytes.clone()))
                    .or_else(|| serde_json::to_value(MessageRenderCompiler::compile(&content)).ok())
            }
            _ if !content.is_empty() => {
                serde_json::to_value(MessageRenderCompiler::compile(&content)).ok()
            }
            _ => None,
        };

        let message = crate::vcp_modules::chat_manager::ChatMessage {
            id: msg_id,
            role,
            name: row.get("name"),
            content,
            timestamp: timestamp as u64,
            is_thinking: Some(false),
            agent_id: row.get("agent_id"),
            group_id: row.get("group_id"),
            topic_id: Some(topic_id.clone()),
            is_group_message: Some(row.get::<i64, _>("is_group_message") != 0),
            finish_reason: row.get("finish_reason"),
            attachments: None, // 批量 push 场景不需要附件回填
            blocks,
            shell: None, // 批量 push 场景不需要外壳预计算
            content_hash,
            transient_context: None,
            transient_system_prompt: None,
        };

        result.entry(topic_id).or_default().push(message);
    }

    // 批量加载附件 — 收集所有 (topic_id, msg_id)，一次 JOIN 查询
    let mut all_msg_refs: Vec<(String, String)> = Vec::new();
    for (tid, msgs) in result.iter() {
        for m in msgs {
            all_msg_refs.push((tid.clone(), m.id.clone()));
        }
    }

    if !all_msg_refs.is_empty() {
        let mut att_placeholders = Vec::new();
        att_placeholders.extend(std::iter::repeat_n("(?, ?)", all_msg_refs.len()));
        let att_query = format!(
            "SELECT a.hash, a.mime_type, a.size, a.internal_path, NULL as extracted_text, a.image_frames, a.thumbnail_path, a.created_at,
                    ma.topic_id, ma.msg_id, ma.display_name, ma.src, ma.status
             FROM message_attachments ma
             JOIN attachments a ON ma.hash = a.hash
             WHERE (ma.topic_id, ma.msg_id) IN ({}) AND ma.deleted_at IS NULL
             ORDER BY ma.topic_id, ma.msg_id, ma.attachment_order ASC",
            att_placeholders.join(",")
        );
        let mut q = sqlx::query(&att_query);
        for (tid, mid) in &all_msg_refs {
            q = q.bind(tid).bind(mid);
        }
        if let Ok(att_rows) = q.fetch_all(pool).await {
            let mut att_map: std::collections::HashMap<(String, String), Vec<Attachment>> =
                std::collections::HashMap::new();
            for ar in att_rows {
                let tid: String = ar.get("topic_id");
                let mid: String = ar.get("msg_id");
                let hash: String = ar.get("hash");
                let mime_type: String = ar.get("mime_type");
                let internal_path: String = ar.get("internal_path");
                let display_name: String = ar.get("display_name");
                let size_i64: i64 = ar.get("size");
                let created_at_i64: i64 = ar.get("created_at");

                att_map.entry((tid, mid)).or_default().push(Attachment {
                    r#type: mime_type,
                    src: ar.get("src"),
                    name: display_name,
                    size: size_i64 as u64,
                    hash: Some(hash),
                    status: Some(ar.get("status")),
                    internal_path,
                    extracted_text: ar.get("extracted_text"),
                    image_frames: ar
                        .get::<Option<String>, _>("image_frames")
                        .and_then(|s| serde_json::from_str(&s).ok()),
                    thumbnail_path: ar.get("thumbnail_path"),
                    created_at: Some(created_at_i64 as u64),
                });
            }
            // 回填附件到消息
            for (tid, msgs) in result.iter_mut() {
                for msg in msgs.iter_mut() {
                    let attachments = att_map.remove(&(tid.clone(), msg.id.clone()));
                    if let Some(atts) = attachments {
                        msg.attachments = Some(atts);
                    }
                }
            }
        }
    }

    for msgs in result.values_mut() {
        for msg in msgs {
            if msg.content_hash.is_none() {
                msg.content_hash = Some(compute_message_hash_from_content(
                    &msg.content,
                    msg.attachments.as_deref(),
                ));
            }
        }
    }

    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub async fn load_chat_history_internal(
    _app_handle: &AppHandle,
    owner_id: &str,
    owner_type: &str,
    topic_id: &str,
    limit: Option<usize>,
    offset: Option<usize>,
    include_content: bool,
    include_extracted_text: bool,
    include_ui_render_data: bool,
) -> Result<Vec<ChatMessage>, String> {
    let db_state = _app_handle.state::<crate::vcp_modules::db_manager::DbState>();
    let pool = &db_state.pool;
    ensure_active_topic_owner(pool, topic_id, owner_id, owner_type).await?;

    let offset = offset.unwrap_or(0);

    let render_select = if include_ui_render_data {
        ", r.render_content, r.content_hash AS render_content_hash"
    } else {
        ""
    };
    let render_join = if include_ui_render_data {
        " LEFT JOIN render_cache r ON m.topic_id = r.topic_id AND m.msg_id = r.msg_id"
    } else {
        ""
    };
    let query_str = if limit.is_some() {
        format!(
            "SELECT m.msg_id, m.role, m.name, m.agent_id, m.content, m.timestamp, m.is_group_message, m.group_id, m.finish_reason, m.content_hash{}
         FROM messages m{}
         WHERE m.topic_id = ? AND m.deleted_at IS NULL
         ORDER BY m.timestamp DESC, m.rowid DESC
         LIMIT ? OFFSET ?",
            render_select, render_join
        )
    } else {
        format!(
            "SELECT m.msg_id, m.role, m.name, m.agent_id, m.content, m.timestamp, m.is_group_message, m.group_id, m.finish_reason, m.content_hash{}
         FROM messages m{}
         WHERE m.topic_id = ? AND m.deleted_at IS NULL
         ORDER BY m.timestamp DESC, m.rowid DESC",
            render_select, render_join
        )
    };

    let mut q = sqlx::query(&query_str).bind(topic_id);
    if let Some(l) = limit {
        q = q.bind(l as i64);
        q = q.bind(offset as i64);
    }
    let rows = q.fetch_all(pool).await.map_err(|e| e.to_string())?;

    // 收集所有 msg_id，用于批量查询附件
    let mut msg_ids = Vec::new();
    for row in &rows {
        use sqlx::Row;
        let msg_id: String = row.get("msg_id");
        msg_ids.push(msg_id);
    }

    // 批量查询所有附件（利用 message_attachments 索引表）
    let mut att_map: std::collections::HashMap<String, Vec<Attachment>> =
        std::collections::HashMap::new();
    if !msg_ids.is_empty() {
        let placeholders = msg_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let extracted_text_column = if include_extracted_text {
            "a.extracted_text"
        } else {
            "NULL"
        };
        let att_query = format!(
            "SELECT a.hash, a.mime_type, a.size, a.internal_path, {} as extracted_text, a.image_frames, a.thumbnail_path, a.created_at,
                    ma.msg_id, ma.display_name, ma.src, ma.status
             FROM message_attachments ma
             JOIN attachments a ON ma.hash = a.hash
             WHERE ma.topic_id = ? AND ma.msg_id IN ({}) AND ma.deleted_at IS NULL
             ORDER BY ma.msg_id, ma.attachment_order ASC",
            extracted_text_column, placeholders
        );
        let mut q = sqlx::query(&att_query).bind(topic_id);
        for id in &msg_ids {
            q = q.bind(id);
        }
        let att_rows = q.fetch_all(pool).await.map_err(|e| e.to_string())?;

        for ar in att_rows {
            let msg_id: String = ar.get("msg_id");
            let hash: String = ar.get("hash");
            let mime_type: String = ar.get("mime_type");
            let internal_path: String = ar.get("internal_path");
            let display_name: String = ar.get("display_name");
            let size_i64: i64 = ar.get("size");
            let created_at_i64: i64 = ar.get("created_at");
            let mut extracted_text: Option<String> = ar.get("extracted_text");

            // ⚡ 极度优雅的消息-附件解耦调用：将物理文件判定、异步持久化完全委托给 file_manager
            if include_extracted_text && extracted_text.is_none() {
                extracted_text = crate::vcp_modules::infra::file_manager::ensure_extracted_text(
                    pool,
                    &hash,
                    &internal_path,
                    &mime_type,
                )
                .await;
            }

            att_map.entry(msg_id).or_default().push(Attachment {
                r#type: mime_type,
                src: ar.get("src"),
                name: display_name,
                size: size_i64 as u64,
                hash: Some(hash),
                status: Some(ar.get("status")),
                internal_path,
                extracted_text,
                image_frames: ar
                    .get::<Option<String>, _>("image_frames")
                    .and_then(|s| serde_json::from_str(&s).ok()),
                thumbnail_path: ar.get("thumbnail_path"),
                created_at: Some(created_at_i64 as u64),
            });
        }
    }

    // 预计算外壳属性所需的全局数据，仅 UI 历史加载需要，且避免 get_agents 的额外 topics 联表查询。
    let (agents, user_name, user_avatar_color) = if include_ui_render_data {
        let agents = match sqlx::query(
            "SELECT a.agent_id, a.name, av.dominant_color
             FROM agents a
             LEFT JOIN avatars av ON av.owner_id = a.agent_id AND av.owner_type = 'agent' AND av.deleted_at IS NULL
             WHERE a.deleted_at IS NULL",
        )
        .fetch_all(pool)
        .await
        {
            Ok(rows) => rows
                .into_iter()
                .map(|row| {
                    use sqlx::Row;
                    crate::vcp_modules::agent_types::AgentConfig {
                        id: row.get("agent_id"),
                        name: row.get("name"),
                        avatar_calculated_color: row.get("dominant_color"),
                        system_prompt: String::new(),
                        mobile_system_prompt: String::new(),
                        model: String::new(),
                        temperature: 0.0,
                        context_token_limit: 0,
                        max_output_tokens: 0,
                        stream_output: false,
                        use_temperature: false,
                        topics: vec![],
                    }
                })
                .collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };

        let settings = crate::vcp_modules::settings_manager::read_settings(
            _app_handle.clone(),
            _app_handle.state(),
        )
        .await
        .ok();
        let user_name = settings
            .map(|s| s.user_name)
            .unwrap_or_else(|| "User".to_string());

        let user_avatar_color: Option<String> = sqlx::query_scalar(
            "SELECT dominant_color FROM avatars WHERE owner_type = 'user' AND owner_id = 'user_avatar' AND deleted_at IS NULL",
        )
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

        (agents, user_name, user_avatar_color)
    } else {
        (Vec::new(), String::new(), None)
    };

    let mut history = Vec::new();
    for row in rows {
        use sqlx::Row;
        let msg_id: String = row.get("msg_id");
        let role: String = row.get("role");
        let name: Option<String> = row.get("name");

        let content_bytes: Vec<u8> = row.get("content");
        let render_content: Option<Vec<u8>> = if include_ui_render_data {
            row.get("render_content")
        } else {
            None
        };
        let render_content_hash: Option<String> = if include_ui_render_data {
            row.get("render_content_hash")
        } else {
            None
        };
        let content_hash_raw: String = row.get("content_hash");
        let timestamp: i64 = row.get("timestamp");
        let is_thinking: Option<bool> = Some(false);

        let attachments = att_map.remove(&msg_id);
        let content_hash_backfill = if content_hash_raw.is_empty() {
            let content = decompress_message_content(&content_bytes, topic_id, &msg_id)?;
            Some(compute_message_hash_from_content(
                &content,
                attachments.as_deref(),
            ))
        } else {
            None
        };
        let effective_content_hash = content_hash_backfill.clone().unwrap_or(content_hash_raw);
        let content_hash = Some(effective_content_hash.clone());
        let cached_blocks = match (&render_content, &render_content_hash, &content_hash) {
            (Some(bytes), Some(render_hash), Some(message_hash))
                if !render_hash.is_empty()
                    && MessageRenderCompiler::cache_matches(render_hash, message_hash) =>
            {
                parse_render_bytes(Some(bytes.clone()))
            }
            _ => None,
        };

        // 懒渲染策略：render_cache 指纹命中才直接用，避免旧 blocks 与新 content 串台
        let (blocks, content) = if !include_ui_render_data {
            let content = if include_content {
                decompress_message_content(&content_bytes, topic_id, &msg_id)?
            } else {
                String::new()
            };
            (None, content)
        } else if let Some(blocks) = cached_blocks {
            let content = if include_content {
                decompress_message_content(&content_bytes, topic_id, &msg_id)?
            } else {
                String::new()
            };
            (Some(blocks), content)
        } else {
            // 未命中：解压 content → 编译 blocks → 异步回写 cache
            let decompressed = decompress_message_content(&content_bytes, topic_id, &msg_id)?;
            if decompressed.is_empty() {
                (None, String::new())
            } else {
                let compiled = MessageRenderCompiler::compile(&decompressed);
                let blocks_json = serde_json::to_value(&compiled).ok();

                // 异步回写 render_cache (使用 tokio::spawn，不阻塞消息加载流)
                if let Ok(serialized) = MessageRenderCompiler::serialize(&compiled) {
                    let pool_c = pool.clone();
                    let tid = topic_id.to_string();
                    let mid = msg_id.clone();
                    let content_hash_for_cache =
                        MessageRenderCompiler::cache_key(&effective_content_hash);
                    let content_hash_backfill_for_message = content_hash_backfill.clone();
                    tokio::spawn(async move {
                        let now = chrono::Utc::now().timestamp_millis();
                        if let Some(hash) = content_hash_backfill_for_message {
                            let _ = sqlx::query(
                                "UPDATE messages SET content_hash = ?, updated_at = ? \
                                 WHERE topic_id = ? AND msg_id = ? AND content_hash = '' \
                                   AND deleted_at IS NULL",
                            )
                            .bind(&hash)
                            .bind(now)
                            .bind(&tid)
                            .bind(&mid)
                            .execute(&pool_c)
                            .await;
                        }
                        let _ = sqlx::query(
                            "INSERT INTO render_cache (topic_id, msg_id, content_hash, render_content, updated_at) \
                             SELECT ?, ?, ?, ?, ? \
                             WHERE EXISTS ( \
                                 SELECT 1 FROM messages m \
                                 JOIN topics t ON t.topic_id = m.topic_id \
                                 WHERE m.topic_id = ? AND m.msg_id = ? \
                                   AND m.deleted_at IS NULL AND t.deleted_at IS NULL \
                             ) \
                             ON CONFLICT(topic_id, msg_id) DO UPDATE SET \
                             content_hash = excluded.content_hash, \
                             render_content = excluded.render_content, \
                             updated_at = excluded.updated_at \
                             WHERE EXISTS ( \
                                 SELECT 1 FROM messages m \
                                 JOIN topics t ON t.topic_id = m.topic_id \
                                 WHERE m.topic_id = excluded.topic_id AND m.msg_id = excluded.msg_id \
                                   AND m.deleted_at IS NULL AND t.deleted_at IS NULL \
                             )"
                        )
                        .bind(&tid)
                        .bind(&mid)
                        .bind(&content_hash_for_cache)
                        .bind(&serialized)
                        .bind(now)
                        .bind(&tid)
                        .bind(&mid)
                        .execute(&pool_c)
                        .await;
                    });
                }

                let content = if include_content {
                    decompressed
                } else {
                    String::new()
                };
                (blocks_json, content)
            }
        };

        let mut message = ChatMessage {
            id: msg_id,
            role,
            name,
            content,
            timestamp: timestamp as u64,
            is_thinking,
            agent_id: row.get("agent_id"),
            group_id: row.get("group_id"),
            topic_id: Some(topic_id.to_string()),
            is_group_message: Some(row.get::<i64, _>("is_group_message") != 0),
            finish_reason: row.get("finish_reason"),
            attachments,
            blocks,
            shell: None,
            content_hash,
            transient_context: None,
            transient_system_prompt: None,
        };

        if include_ui_render_data {
            message.shell = Some(crate::vcp_modules::pre_renderer::precompute_shell(
                &message,
                &agents,
                &user_name,
                user_avatar_color.as_deref(),
            ));
        }
        history.push(message);
    }

    history.reverse();
    Ok(history)
}

/// 为 Agent 和 Group 组装大模型上下文提供专用的轻量历史查询。
/// 只查询消息纯文本和附件（在需要时提取文本），完全跳过 render_content 反序列化和 UI shell 预计算。
pub(crate) fn estimate_text_tokens(text: &str) -> usize {
    let mut tokens = 0usize;
    let mut ascii_run = 0usize;

    for character in text.chars() {
        if character.is_ascii() {
            ascii_run += 1;
        } else {
            tokens += ascii_run.div_ceil(4);
            ascii_run = 0;
            // CJK characters are commonly close to one token each. Counting every non-ASCII
            // scalar as one is intentionally conservative for mixed-language conversations.
            tokens += 1;
        }
    }

    tokens + ascii_run.div_ceil(4)
}

pub(crate) fn estimate_chat_message_tokens(message: &ChatMessage) -> usize {
    const MESSAGE_ENVELOPE_TOKENS: usize = 16;
    const MEDIA_REFERENCE_TOKENS: usize = 256;

    let mut tokens = MESSAGE_ENVELOPE_TOKENS + estimate_text_tokens(&message.content);
    if let Some(name) = message.name.as_deref() {
        tokens += estimate_text_tokens(name);
    }
    if let Some(attachments) = message.attachments.as_deref() {
        for attachment in attachments {
            tokens += estimate_text_tokens(&attachment.name);
            tokens += attachment
                .extracted_text
                .as_deref()
                .map(estimate_text_tokens)
                .unwrap_or(MEDIA_REFERENCE_TOKENS);
        }
    }
    tokens
}

pub(crate) fn context_input_token_budget(
    context_token_limit: i32,
    max_output_tokens: i32,
    additional_text: &[&str],
) -> usize {
    const DEFAULT_CONTEXT_TOKENS: usize = 128_000;
    const REQUEST_SAFETY_TOKENS: usize = 2_048;
    const MIN_HISTORY_TOKENS: usize = 2_048;

    let context_tokens = if context_token_limit > 0 {
        context_token_limit as usize
    } else {
        DEFAULT_CONTEXT_TOKENS
    };
    let prompt_tokens = additional_text
        .iter()
        .map(|text| estimate_text_tokens(text))
        .sum::<usize>();
    let reserved_tokens = (max_output_tokens.max(0) as usize)
        .saturating_add(prompt_tokens)
        .saturating_add(REQUEST_SAFETY_TOKENS);

    context_tokens
        .saturating_sub(reserved_tokens)
        .max(MIN_HISTORY_TOKENS)
}

pub(crate) fn select_recent_history_within_token_budget(
    history: &[ChatMessage],
    token_budget: usize,
) -> Vec<ChatMessage> {
    let mut selected_newest_first = Vec::new();
    let mut used_tokens = 0usize;

    for message in history.iter().rev() {
        let message_tokens = estimate_chat_message_tokens(message);
        if !selected_newest_first.is_empty()
            && used_tokens.saturating_add(message_tokens) > token_budget
        {
            break;
        }
        used_tokens = used_tokens.saturating_add(message_tokens);
        selected_newest_first.push(message.clone());
    }

    selected_newest_first.reverse();
    selected_newest_first
}

pub async fn load_chat_text_history_for_context(
    app_handle: &AppHandle,
    topic_id: &str,
    limit: Option<usize>,
    offset: Option<usize>,
    include_extracted_text: bool,
) -> Result<Vec<ChatMessage>, String> {
    let db_state = app_handle.state::<crate::vcp_modules::db_manager::DbState>();
    let pool = &db_state.pool;
    ensure_active_topic(pool, topic_id).await?;

    let offset = offset.unwrap_or(0);

    // 彻底剥离了对 render_cache 联表查询，仅拉取核心文本和配置字段
    let query_str = if limit.is_some() {
        "SELECT m.msg_id, m.role, m.name, m.agent_id, m.content, m.timestamp, m.is_group_message, m.group_id, m.finish_reason, m.content_hash
         FROM messages m
         WHERE m.topic_id = ? AND m.deleted_at IS NULL
         ORDER BY m.timestamp DESC, m.rowid DESC
         LIMIT ? OFFSET ?"
    } else {
        "SELECT m.msg_id, m.role, m.name, m.agent_id, m.content, m.timestamp, m.is_group_message, m.group_id, m.finish_reason, m.content_hash
         FROM messages m
         WHERE m.topic_id = ? AND m.deleted_at IS NULL
         ORDER BY m.timestamp DESC, m.rowid DESC"
    };

    let mut q = sqlx::query(query_str).bind(topic_id);
    if let Some(l) = limit {
        q = q.bind(l as i64);
        q = q.bind(offset as i64);
    }
    let rows = q.fetch_all(pool).await.map_err(|e| e.to_string())?;

    // 收集所有 msg_id，用于查询附件
    let mut msg_ids = Vec::new();
    for row in &rows {
        let msg_id: String = row.get("msg_id");
        msg_ids.push(msg_id);
    }

    let mut att_map: std::collections::HashMap<String, Vec<Attachment>> =
        std::collections::HashMap::new();
    if !msg_ids.is_empty() {
        let placeholders = msg_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let extracted_text_column = if include_extracted_text {
            "a.extracted_text"
        } else {
            "NULL"
        };
        let att_query = format!(
            "SELECT a.hash, a.mime_type, a.size, a.internal_path, {} as extracted_text, a.image_frames, a.thumbnail_path, a.created_at,
                    ma.msg_id, ma.display_name, ma.src, ma.status
             FROM message_attachments ma
             JOIN attachments a ON ma.hash = a.hash
             WHERE ma.topic_id = ? AND ma.msg_id IN ({}) AND ma.deleted_at IS NULL
             ORDER BY ma.msg_id, ma.attachment_order ASC",
            extracted_text_column, placeholders
        );
        let mut q = sqlx::query(&att_query).bind(topic_id);
        for id in &msg_ids {
            q = q.bind(id);
        }
        let att_rows = q.fetch_all(pool).await.map_err(|e| e.to_string())?;

        for ar in att_rows {
            let msg_id: String = ar.get("msg_id");
            let hash: String = ar.get("hash");
            let mime_type: String = ar.get("mime_type");
            let internal_path: String = ar.get("internal_path");
            let display_name: String = ar.get("display_name");
            let size_i64: i64 = ar.get("size");
            let created_at_i64: i64 = ar.get("created_at");
            let mut extracted_text: Option<String> = ar.get("extracted_text");

            if include_extracted_text && extracted_text.is_none() {
                extracted_text = crate::vcp_modules::infra::file_manager::ensure_extracted_text(
                    pool,
                    &hash,
                    &internal_path,
                    &mime_type,
                )
                .await;
            }

            att_map.entry(msg_id).or_default().push(Attachment {
                r#type: mime_type,
                src: ar.get("src"),
                name: display_name,
                size: size_i64 as u64,
                hash: Some(hash),
                status: Some(ar.get("status")),
                internal_path,
                extracted_text,
                image_frames: ar
                    .get::<Option<String>, _>("image_frames")
                    .and_then(|s| serde_json::from_str(&s).ok()),
                thumbnail_path: ar.get("thumbnail_path"),
                created_at: Some(created_at_i64 as u64),
            });
        }
    }

    let mut history = Vec::new();
    for row in rows {
        let msg_id: String = row.get("msg_id");
        let role: String = row.get("role");
        let name: Option<String> = row.get("name");

        let content_bytes: Vec<u8> = row.get("content");
        let content = decompress_message_content(&content_bytes, topic_id, &msg_id)?;

        let content_hash_raw: String = row.get("content_hash");
        let content_hash = if content_hash_raw.is_empty() {
            None
        } else {
            Some(content_hash_raw)
        };

        let timestamp: i64 = row.get("timestamp");
        let attachments = att_map.remove(&msg_id);

        let message = ChatMessage {
            id: msg_id,
            role,
            name,
            content,
            timestamp: timestamp as u64,
            is_thinking: Some(false),
            agent_id: row.get("agent_id"),
            group_id: row.get("group_id"),
            topic_id: Some(topic_id.to_string()),
            is_group_message: Some(row.get::<i64, _>("is_group_message") != 0),
            finish_reason: row.get("finish_reason"),
            attachments,
            blocks: None, // 彻底不加载和反序列化渲染 cache 块
            shell: None,  // 彻底不预计算 UI 头像、边框背景等外壳属性
            content_hash,
            transient_context: None,
            transient_system_prompt: None,
        };
        history.push(message);
    }

    history.reverse();
    Ok(history)
}

/// Loads recent context in bounded pages and stops at an estimated token budget.
/// This keeps large configured context windows usable without loading UI render caches or applying
/// a fixed message-count ceiling that silently discards otherwise valid conversation history.
pub async fn load_chat_text_history_for_context_window(
    app_handle: &AppHandle,
    topic_id: &str,
    token_budget: usize,
    include_extracted_text: bool,
) -> Result<Vec<ChatMessage>, String> {
    const PAGE_SIZE: usize = 64;

    let mut selected_newest_first = Vec::new();
    let mut used_tokens = 0usize;
    let mut offset = 0usize;
    let mut reached_budget = false;

    while !reached_budget {
        let page = load_chat_text_history_for_context(
            app_handle,
            topic_id,
            Some(PAGE_SIZE),
            Some(offset),
            include_extracted_text,
        )
        .await?;
        let page_len = page.len();
        if page_len == 0 {
            break;
        }

        for message in page.into_iter().rev() {
            let message_tokens = estimate_chat_message_tokens(&message);
            if !selected_newest_first.is_empty()
                && used_tokens.saturating_add(message_tokens) > token_budget
            {
                reached_budget = true;
                break;
            }
            used_tokens = used_tokens.saturating_add(message_tokens);
            selected_newest_first.push(message);
        }

        offset = offset.saturating_add(page_len);
        if page_len < PAGE_SIZE {
            break;
        }
    }

    selected_newest_first.reverse();
    log::info!(
        "[MessageService] Context history selected: topic_id={}, messages={}, estimated_tokens={}, budget_tokens={}, budget_reached={}",
        topic_id,
        selected_newest_first.len(),
        used_tokens,
        token_budget,
        reached_budget
    );
    Ok(selected_newest_first)
}

#[cfg(test)]
mod context_window_tests {
    use super::*;

    fn message(index: usize, content: String) -> ChatMessage {
        ChatMessage {
            id: format!("message-{index}"),
            role: if index % 2 == 0 { "user" } else { "assistant" }.to_string(),
            content,
            timestamp: index as u64,
            ..ChatMessage::default()
        }
    }

    #[test]
    fn configured_large_context_is_not_capped_at_240_messages() {
        let history = (0..400)
            .map(|index| message(index, format!("第 {index} 条短消息")))
            .collect::<Vec<_>>();

        let selected = select_recent_history_within_token_budget(&history, 1_000_000);

        assert_eq!(selected.len(), 400);
        assert_eq!(selected.first().unwrap().id, "message-0");
        assert_eq!(selected.last().unwrap().id, "message-399");
    }

    #[test]
    fn token_budget_discards_oldest_messages_and_keeps_latest_oversized_message() {
        let history = vec![
            message(0, "a".repeat(400)),
            message(1, "b".repeat(400)),
            message(2, "中".repeat(300)),
        ];

        let selected = select_recent_history_within_token_budget(&history, 200);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, "message-2");
        assert!(estimate_chat_message_tokens(&selected[0]) > 200);
    }

    #[test]
    fn input_budget_reserves_output_and_prompt_tokens() {
        let prompt = "中".repeat(1_000);
        let budget = context_input_token_budget(10_000, 2_000, &[&prompt]);

        assert_eq!(budget, 4_952);
    }
}

const MAX_SYNC_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;

async fn download_attachment_to_cas(
    response: reqwest::Response,
    temp_path: &Path,
    final_path: &Path,
    expected_hash: &str,
) -> Result<(), String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_SYNC_ATTACHMENT_BYTES)
    {
        return Err("同步附件超过 100MB 限制".to_string());
    }

    let result = async {
        let mut file = fs::File::create(temp_path)
            .await
            .map_err(|error| format!("创建附件临时文件失败: {error}"))?;
        let mut stream = response.bytes_stream();
        let mut total = 0u64;
        let mut hasher = Sha256::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| format!("读取同步附件失败: {error}"))?;
            total = total
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| "同步附件大小溢出".to_string())?;
            if total > MAX_SYNC_ATTACHMENT_BYTES {
                return Err("同步附件超过 100MB 限制".to_string());
            }
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|error| format!("写入附件临时文件失败: {error}"))?;
        }
        file.flush()
            .await
            .map_err(|error| format!("刷新附件临时文件失败: {error}"))?;
        drop(file);

        let actual_hash = hex::encode(hasher.finalize());
        if !actual_hash.eq_ignore_ascii_case(expected_hash) {
            return Err(format!(
                "同步附件哈希不匹配: expected={expected_hash}, actual={actual_hash}"
            ));
        }

        crate::vcp_modules::file_manager::safe_rename(temp_path, final_path)
            .map_err(|error| format!("提交同步附件失败: {error}"))
    }
    .await;

    if result.is_err() {
        let _ = fs::remove_file(temp_path).await;
    }
    result
}

/// 核心：确保消息中的附件在手机本地物理存在，否则从电脑同步下载
async fn ensure_attachments_locally<R: tauri::Runtime>(
    app: &AppHandle<R>,
    message: &mut ChatMessage,
) -> Result<(), String> {
    let attachments = match &mut message.attachments {
        Some(atts) => atts,
        None => return Ok(()),
    };

    let att_dir = get_attachments_root_dir(app)?;
    if !att_dir.exists() {
        fs::create_dir_all(&att_dir)
            .await
            .map_err(|e| e.to_string())?;
    }

    for att in attachments {
        let hash = match &att.hash {
            Some(h) => h.clone(),
            None => continue,
        };
        if !crate::vcp_modules::infra::utils::is_valid_cas_hash(&hash) {
            log::warn!("[MessageService] Ignoring attachment with invalid CAS hash");
            continue;
        }
        if att.size > MAX_SYNC_ATTACHMENT_BYTES {
            log::warn!(
                "[MessageService] Skipping oversized attachment {} ({} bytes)",
                hash,
                att.size
            );
            continue;
        }

        // 判定后缀 (对齐 file_manager.rs 逻辑)
        let ext = Path::new(&att.name)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let local_file_name = if ext.is_empty() {
            hash.clone()
        } else {
            format!("{}.{}", hash, ext)
        };

        let local_path = att_dir.join(&local_file_name);
        let local_path_str = local_path.to_string_lossy().into_owned();

        if !local_path.exists() {
            // 尝试下载
            let settings = settings_manager::read_settings(app.clone(), app.state()).await?;
            if !settings.sync_http_url.is_empty() {
                let client = reqwest::Client::new();
                let url = format!(
                    "{}/api/mobile-sync/download-attachment?hash={}",
                    settings.sync_http_url, hash
                );
                match client
                    .get(&url)
                    .header("x-sync-token", &settings.sync_token)
                    .header("Authorization", format!("Bearer {}", &settings.sync_token))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        let temp_path =
                            att_dir.join(format!(".{}.{}.part", hash, uuid::Uuid::new_v4()));
                        if let Err(error) =
                            download_attachment_to_cas(resp, &temp_path, &local_path, &hash).await
                        {
                            log::warn!(
                                "[MessageService] Failed to download attachment {}: {}",
                                hash,
                                error
                            );
                        }
                    }
                    Ok(resp) => log::warn!(
                        "[MessageService] Attachment {} download returned {}",
                        hash,
                        resp.status()
                    ),
                    Err(error) => log::warn!(
                        "[MessageService] Attachment {} download failed: {}",
                        hash,
                        error
                    ),
                }
            }
        }

        // 核心对齐：
        // 1. src 保持物理路径（用于超栈追踪），如果来自电脑端，它已经包含 file:// 路径
        // 2. internal_path 专门作为手机本地可访问路径，前端可通过 convertFileSrc 转换为 asset://
        if att.src.is_empty() {
            att.src = format!("file://{}", local_path_str);
        }
        att.internal_path = local_path_str;
    }
    Ok(())
}

pub async fn append_single_message<R: tauri::Runtime>(
    app_handle: AppHandle<R>,
    db_pool: &sqlx::Pool<sqlx::Sqlite>,
    owner_id: &str,
    owner_type: &str,
    topic_id: String,
    mut message: ChatMessage,
) -> Result<Vec<ContentBlock>, String> {
    ensure_active_topic_owner(db_pool, &topic_id, owner_id, owner_type).await?;
    ensure_attachments_locally(&app_handle, &mut message).await?;
    repair_assistant_render_content_before_persist(&mut message);

    let blocks: Vec<ContentBlock> = if let Some(blocks_val) = &message.blocks {
        serde_json::from_value(blocks_val.clone()).map_err(|e| e.to_string())?
    } else {
        MessageRenderCompiler::compile(&message.content)
    };
    let render_bytes = MessageRenderCompiler::serialize(&blocks)?;

    let mut tx = db_pool.begin().await.map_err(|e| e.to_string())?;
    MessageRepository::upsert_message(&mut tx, &message, &topic_id, &render_bytes, false).await?;

    let msg_count: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(&topic_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?
    .unwrap_or(0);

    sqlx::query(
        "UPDATE topics SET updated_at = ?, msg_count = ? \
         WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(message.timestamp as i64)
    .bind(msg_count)
    .bind(&topic_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(blocks)
}

#[tauri::command]
pub async fn fetch_raw_message_content(
    app_handle: tauri::AppHandle,
    message_id: String,
    topic_id: Option<String>,
) -> Result<String, String> {
    let db_state = app_handle.state::<crate::vcp_modules::db_manager::DbState>();
    let pool = &db_state.pool;

    let row = if let Some(topic_id) = topic_id {
        sqlx::query(
            "SELECT content FROM messages \
             WHERE topic_id = ? AND msg_id = ? AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .bind(&message_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
    } else {
        sqlx::query("SELECT content FROM messages WHERE msg_id = ? AND deleted_at IS NULL")
            .bind(&message_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
    };

    match row {
        Some(r) => {
            let bytes: Vec<u8> = r.get(0);
            let content = ContentCompressor::decompress(&bytes).map_err(|e| {
                format!(
                    "Failed to decompress content for message {}: {}",
                    message_id, e
                )
            })?;
            Ok(content)
        }
        None => Err(format!("Message {} not found", message_id)),
    }
}

#[tauri::command]
pub async fn re_render_message(
    app_handle: tauri::AppHandle,
    message_id: String,
    topic_id: String,
) -> Result<serde_json::Value, String> {
    let db_state = app_handle.state::<crate::vcp_modules::db_manager::DbState>();
    let pool = &db_state.pool;

    let row = sqlx::query(
        "SELECT m.content, m.content_hash FROM messages m \
                     JOIN topics t ON t.topic_id = m.topic_id \
                     WHERE m.msg_id = ? AND m.topic_id = ? \
                       AND m.deleted_at IS NULL AND t.deleted_at IS NULL",
    )
    .bind(&message_id)
    .bind(&topic_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    match row {
        Some(r) => {
            let bytes: Vec<u8> = r.get("content");
            let decompressed = ContentCompressor::decompress(&bytes).map_err(|e| {
                format!(
                    "Failed to decompress content for message {} in topic {}: {}",
                    message_id, topic_id, e
                )
            })?;

            let compiled = MessageRenderCompiler::compile(&decompressed);
            let serialized = MessageRenderCompiler::serialize(&compiled)?;
            let content_hash: String = r.get("content_hash");

            let now = chrono::Utc::now().timestamp_millis();
            let cache_result = sqlx::query(
                "INSERT INTO render_cache (topic_id, msg_id, content_hash, render_content, updated_at) \
                 SELECT ?, ?, ?, ?, ? \
                 WHERE EXISTS ( \
                     SELECT 1 FROM messages m \
                     JOIN topics t ON t.topic_id = m.topic_id \
                     WHERE m.topic_id = ? AND m.msg_id = ? \
                       AND m.deleted_at IS NULL AND t.deleted_at IS NULL \
                 ) \
                 ON CONFLICT(topic_id, msg_id) DO UPDATE SET \
                 content_hash = excluded.content_hash, \
                 render_content = excluded.render_content, \
                 updated_at = excluded.updated_at \
                 WHERE EXISTS ( \
                     SELECT 1 FROM messages m \
                     JOIN topics t ON t.topic_id = m.topic_id \
                     WHERE m.topic_id = excluded.topic_id AND m.msg_id = excluded.msg_id \
                       AND m.deleted_at IS NULL AND t.deleted_at IS NULL \
                 )",
            )
            .bind(&topic_id)
            .bind(&message_id)
            .bind(MessageRenderCompiler::cache_key(&content_hash))
            .bind(&serialized)
            .bind(now)
            .bind(&topic_id)
            .bind(&message_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
            if cache_result.rows_affected() == 0 {
                return Err(format!(
                    "Message {} in topic {} was deleted during re-render",
                    message_id, topic_id
                ));
            }

            serde_json::to_value(&compiled).map_err(|e| e.to_string())
        }
        None => Err(format!(
            "Message {} with topic {} not found",
            message_id, topic_id
        )),
    }
}

pub async fn patch_single_message<R: tauri::Runtime>(
    app_handle: AppHandle<R>,
    db_pool: &sqlx::Pool<sqlx::Sqlite>,
    owner_id: &str,
    owner_type: &str,
    topic_id: String,
    mut message: ChatMessage,
    skip_bubble: bool,
) -> Result<Vec<ContentBlock>, String> {
    ensure_active_topic_owner(db_pool, &topic_id, owner_id, owner_type).await?;
    ensure_attachments_locally(&app_handle, &mut message).await?;
    repair_assistant_render_content_before_persist(&mut message);

    // 优先使用传入的 blocks，如果缺失则实时编译
    let blocks: Vec<ContentBlock> = if let Some(blocks_val) = &message.blocks {
        serde_json::from_value(blocks_val.clone()).map_err(|e| e.to_string())?
    } else {
        MessageRenderCompiler::compile(&message.content)
    };
    let render_bytes = MessageRenderCompiler::serialize(&blocks)?;

    let mut tx = db_pool.begin().await.map_err(|e| e.to_string())?;
    MessageRepository::upsert_message(&mut tx, &message, &topic_id, &render_bytes, skip_bubble)
        .await?;

    let msg_count: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(&topic_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?
    .unwrap_or(0);

    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "UPDATE topics SET updated_at = ?, msg_count = ? \
         WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(now)
    .bind(msg_count)
    .bind(&topic_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(blocks)
}

async fn cancel_lifecycle_jobs_for_messages(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    topic_id: &str,
    message_ids: &[String],
    now: i64,
) -> Result<(), String> {
    if message_ids.is_empty() {
        return Ok(());
    }
    let placeholders = message_ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let query = format!(
        "UPDATE lifecycle_jobs SET status = 'cancelled', lease_until = NULL, \
         failure_reason = 'Source or response message was deleted', updated_at = ? \
         WHERE topic_id = ? AND status NOT IN ('completed', 'cancelled') \
         AND (source_message_id IN ({0}) OR response_message_id IN ({0}))",
        placeholders
    );
    let mut query = sqlx::query(&query).bind(now).bind(topic_id);
    for message_id in message_ids {
        query = query.bind(message_id);
    }
    for message_id in message_ids {
        query = query.bind(message_id);
    }
    query.execute(&mut **tx).await.map_err(|error| {
        format!("Failed to cancel lifecycle jobs for deleted messages: {error}")
    })?;
    Ok(())
}

pub async fn delete_messages(
    db_pool: &sqlx::Pool<sqlx::Sqlite>,
    topic_id: &str,
    msg_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    if msg_ids.is_empty() {
        return Ok(Vec::new());
    }
    ensure_active_topic(db_pool, topic_id).await?;
    let mut tx = db_pool.begin().await.map_err(|e| e.to_string())?;
    let select_query = format!(
        "SELECT msg_id FROM messages WHERE topic_id = ? AND deleted_at IS NULL AND msg_id IN ({})",
        msg_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ")
    );
    let mut select = sqlx::query_scalar::<_, String>(&select_query).bind(topic_id);
    for id in &msg_ids {
        select = select.bind(id);
    }
    let deleted_ids = select
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    if deleted_ids.is_empty() {
        tx.commit().await.map_err(|e| e.to_string())?;
        return Ok(Vec::new());
    }
    let delete_query = format!(
        "UPDATE messages SET deleted_at = ? WHERE topic_id = ? AND msg_id IN ({})",
        deleted_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ")
    );
    let now = chrono::Utc::now().timestamp_millis();
    let mut q = sqlx::query(&delete_query).bind(now).bind(topic_id);
    for id in &deleted_ids {
        q = q.bind(id);
    }
    q.execute(&mut *tx).await.map_err(|e| e.to_string())?;

    // 物理强清除 render_cache 缓存，杜绝幽灵缓存残留
    let delete_cache_query = format!(
        "DELETE FROM render_cache WHERE topic_id = ? AND msg_id IN ({})",
        deleted_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut q_cache = sqlx::query(&delete_cache_query).bind(topic_id);
    for id in &deleted_ids {
        q_cache = q_cache.bind(id);
    }
    q_cache.execute(&mut *tx).await.map_err(|e| e.to_string())?;

    // 物理强清除 message_attachments 关联，防止孤立关联残留
    let delete_attachments_query = format!(
        "DELETE FROM message_attachments WHERE topic_id = ? AND msg_id IN ({})",
        deleted_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut q_attachments = sqlx::query(&delete_attachments_query).bind(topic_id);
    for id in &deleted_ids {
        q_attachments = q_attachments.bind(id);
    }
    q_attachments
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    let delete_generations_query = format!(
        "DELETE FROM active_generations WHERE topic_id = ? AND msg_id IN ({})",
        deleted_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut q_generations = sqlx::query(&delete_generations_query).bind(topic_id);
    for id in &deleted_ids {
        q_generations = q_generations.bind(id);
    }
    q_generations
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    cancel_lifecycle_jobs_for_messages(&mut tx, topic_id, &deleted_ids, now).await?;

    let msg_count: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?
    .unwrap_or(0);

    sqlx::query(
        "UPDATE topics SET msg_count = ?, updated_at = ? \
         WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(msg_count)
    .bind(now)
    .bind(topic_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    HashAggregator::bubble_from_topic(&mut tx, topic_id).await?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(deleted_ids)
}

pub async fn truncate_history_after_timestamp(
    app_handle: AppHandle,
    db_pool: &sqlx::Pool<sqlx::Sqlite>,
    owner_id: &str,
    owner_type: &str,
    topic_id: &str,
    timestamp: i64,
) -> Result<(), String> {
    ensure_active_topic_owner(db_pool, topic_id, owner_id, owner_type).await?;

    let request_ids: Vec<String> = sqlx::query_scalar(
        "SELECT msg_id FROM messages \
         WHERE topic_id = ? AND timestamp > ? AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .bind(timestamp)
    .fetch_all(db_pool)
    .await
    .map_err(|e| e.to_string())?;
    if let Some(active_requests) =
        app_handle.try_state::<crate::vcp_modules::vcp_client::ActiveRequests>()
    {
        let group_turn_ids = active_requests.cancel_topic(topic_id);
        active_requests.cancel_ids(request_ids.iter().map(String::as_str));
        if let Some(cancelled_turns) =
            app_handle.try_state::<crate::vcp_modules::vcp_client::CancelledGroupTurns>()
        {
            for turn_id in group_turn_ids {
                cancelled_turns.0.insert(turn_id);
            }
        }
    }

    let mut tx = db_pool.begin().await.map_err(|e| e.to_string())?;

    // 物理强清除 render_cache，消灭幽灵缓存
    sqlx::query("DELETE FROM render_cache WHERE topic_id = ? AND msg_id IN (SELECT msg_id FROM messages WHERE topic_id = ? AND timestamp > ?)")
        .bind(topic_id).bind(topic_id).bind(timestamp).execute(&mut *tx).await.map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM message_attachments WHERE topic_id = ? AND msg_id IN (SELECT msg_id FROM messages WHERE topic_id = ? AND timestamp > ?)")
        .bind(topic_id).bind(topic_id).bind(timestamp).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM active_generations WHERE topic_id = ? AND msg_id IN (SELECT msg_id FROM messages WHERE topic_id = ? AND timestamp > ?)")
        .bind(topic_id).bind(topic_id).bind(timestamp).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE messages SET deleted_at = ? WHERE topic_id = ? AND timestamp > ?")
        .bind(now)
        .bind(topic_id)
        .bind(timestamp)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    cancel_lifecycle_jobs_for_messages(&mut tx, topic_id, &request_ids, now).await?;
    let msg_count: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?
    .unwrap_or(0);
    sqlx::query(
        "UPDATE topics SET msg_count = ?, updated_at = ? \
         WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(msg_count)
    .bind(now)
    .bind(topic_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    HashAggregator::bubble_from_topic(&mut tx, topic_id).await?;
    tx.commit().await.map_err(|e| e.to_string())?;

    if let Some(sync_state) =
        app_handle.try_state::<crate::vcp_modules::sync::sync_service::SyncState>()
    {
        for message_id in request_ids {
            let _ = sync_state.ws_sender.send(
                crate::vcp_modules::sync::sync_service::SyncCommand::NotifyDelete {
                    data_type: crate::vcp_modules::sync::sync_types::SyncDataType::Message,
                    id: message_id,
                },
            );
        }
    }
    Ok(())
}

/// Helper: Deserializes render_content bytes (JSON + zstd) into JSON blocks for frontend
fn parse_render_bytes(render_content: Option<Vec<u8>>) -> Option<serde_json::Value> {
    render_content.and_then(|bytes| {
        match crate::vcp_modules::message_repository::MessageRenderCompiler::deserialize(&bytes) {
            Ok(blocks) => serde_json::to_value(blocks).ok(),
            Err(error) => {
                log::warn!("[MessageService] Ignoring invalid render cache: {error}");
                None
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn finalize_stream_message<R: tauri::Runtime>(
    app_handle: AppHandle<R>,
    pool: &sqlx::Pool<sqlx::Sqlite>,
    owner_id: &str,
    owner_type: &str, // "agent" | "group"
    topic_id: String,
    message_id: String,
    full_content: String,
    is_aborted: bool,
    finish_reason: Option<String>,
    responder_agent_id: Option<&str>,
    responder_name: Option<&str>,
    stream_channel: Option<Channel<crate::vcp_modules::vcp_client::StreamEvent>>,
    agent_id: Option<String>,
) -> Result<(), String> {
    let final_ts = crate::vcp_modules::infra::utils::now_millis() as u64;

    let (mut final_content, scheduled_jobs) =
        crate::vcp_modules::lifecycle_scheduler::extract_and_schedule_directives(
            pool,
            &full_content,
            owner_id,
            owner_type,
            &topic_id,
            responder_agent_id,
            &message_id,
        )
        .await?;
    if !scheduled_jobs.is_empty() {
        log::info!(
            "[StreamFinalizer] Scheduled {} lifecycle job(s) from message {}",
            scheduled_jobs.len(),
            message_id
        );
        let _ = app_handle.emit("vcp-lifecycle-jobs-changed", scheduled_jobs.len());
    }
    if is_aborted {
        final_content.push_str("\n\n> VCP流式错误: 请求已中止");
    }

    final_content = repair_message_content_before_persist(&final_content);
    let repaired_final_content = final_content.clone();

    let is_group = owner_type == "group";

    let final_agent_id = if is_group {
        agent_id.or_else(|| responder_agent_id.map(ToString::to_string))
    } else {
        Some(owner_id.to_string())
    };

    let mut final_name = responder_name.map(ToString::to_string);
    if final_name.is_none() {
        if let Some(ref aid) = final_agent_id {
            if let Ok(Some(row)) =
                sqlx::query("SELECT name FROM agents WHERE agent_id = ? AND deleted_at IS NULL")
                    .bind(aid)
                    .fetch_optional(pool)
                    .await
            {
                use sqlx::Row;
                final_name = Some(row.get::<String, _>("name"));
            }
        }
    }

    let final_msg = ChatMessage {
        id: message_id.clone(),
        role: "assistant".to_string(),
        name: final_name.clone(),
        content: final_content,
        timestamp: final_ts,
        is_thinking: Some(false),
        agent_id: final_agent_id.clone(),
        group_id: if is_group {
            Some(owner_id.to_string())
        } else {
            None
        },
        topic_id: Some(topic_id.clone()),
        is_group_message: Some(is_group),
        finish_reason: finish_reason.clone(),
        attachments: None,
        blocks: None,
        shell: None,
        content_hash: None,
        transient_context: None,
        transient_system_prompt: None,
    };

    let end_blocks = if owner_id.is_empty() || topic_id.is_empty() {
        None
    } else if !is_group {
        match patch_single_message(
            app_handle.clone(),
            pool,
            owner_id,
            "agent",
            topic_id.clone(),
            final_msg,
            false,
        )
        .await
        {
            Ok(blocks) => Some(blocks),
            Err(e) => {
                let error = format!("[StreamFinalizer] Failed to persist agent message: {}", e);
                log::error!("{}", error);
                if let Some(chan) = stream_channel.as_ref() {
                    let _ = chan.send(crate::vcp_modules::vcp_client::StreamEvent::error(
                        message_id.clone(),
                        None,
                        error.clone(),
                    ));
                }
                return Err(error);
            }
        }
    } else {
        match append_single_message(
            app_handle.clone(),
            pool,
            owner_id,
            "group",
            topic_id.clone(),
            final_msg,
        )
        .await
        {
            Ok(blocks) => Some(blocks),
            Err(e) => {
                let error = format!("[StreamFinalizer] Failed to persist group message: {}", e);
                log::error!("{}", error);
                if let Some(chan) = stream_channel.as_ref() {
                    let _ = chan.send(crate::vcp_modules::vcp_client::StreamEvent::error(
                        message_id.clone(),
                        None,
                        error.clone(),
                    ));
                }
                return Err(error);
            }
        }
    };

    let should_auto_summarize = !owner_id.is_empty()
        && !topic_id.is_empty()
        && !is_aborted
        && !matches!(
            finish_reason.as_deref(),
            Some("cancelled_by_user") | Some("error")
        );

    if let Some(chan) = stream_channel {
        let context = if owner_id.is_empty() || topic_id.is_empty() {
            None
        } else if is_group {
            Some(serde_json::json!({
                "groupId": owner_id,
                "topicId": topic_id,
                "agentId": final_agent_id,
                "agentName": final_name,
                "isGroupMessage": true,
            }))
        } else {
            Some(serde_json::json!({
                "agentId": owner_id,
                "topicId": topic_id,
                "agentName": final_name,
            }))
        };

        let mut end_event = crate::vcp_modules::vcp_client::StreamEvent::end(
            message_id,
            context,
            Some(finish_reason.unwrap_or_else(|| "completed".to_string())),
            end_blocks,
            Some(final_ts),
        );
        end_event.content = Some(repaired_final_content);
        let _ = chan.send(end_event);
    }

    if should_auto_summarize {
        let summary_app = app_handle.clone();
        let summary_pool = pool.clone();
        let summary_owner_id = owner_id.to_string();
        let summary_owner_type = owner_type.to_string();
        let summary_topic_id = topic_id.clone();
        let summary_agent_name = final_name.unwrap_or_else(|| "AI".to_string());

        tauri::async_runtime::spawn(async move {
            crate::vcp_modules::chat::topic_summary_service::summarize_topic_if_needed(
                summary_app,
                summary_pool,
                summary_owner_id,
                summary_owner_type,
                summary_topic_id,
                summary_agent_name,
            )
            .await;
        });
    }

    Ok(())
}

async fn delete_message_attachment_in_pool(
    pool: &sqlx::SqlitePool,
    topic_id: &str,
    message_id: &str,
    hash: &str,
    now: i64,
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let result = sqlx::query(
        "UPDATE message_attachments SET deleted_at = ? \
         WHERE topic_id = ? AND msg_id = ? AND hash = ? AND deleted_at IS NULL",
    )
    .bind(now)
    .bind(topic_id)
    .bind(message_id)
    .bind(hash)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    if result.rows_affected() == 0 {
        return Err("附件不存在或已经移除".to_string());
    }

    let content_bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT content FROM messages \
         WHERE topic_id = ? AND msg_id = ? AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .bind(message_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let content = ContentCompressor::decompress(&content_bytes)?;
    let attachment_hashes: Vec<String> = sqlx::query_scalar(
        "SELECT hash FROM message_attachments \
         WHERE topic_id = ? AND msg_id = ? AND deleted_at IS NULL ORDER BY hash",
    )
    .bind(topic_id)
    .bind(message_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let content_hash = HashAggregator::compute_message_fingerprint(&content, &attachment_hashes);

    sqlx::query(
        "UPDATE messages SET content_hash = ?, updated_at = ? \
         WHERE topic_id = ? AND msg_id = ?",
    )
    .bind(content_hash)
    .bind(now)
    .bind(topic_id)
    .bind(message_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    HashAggregator::bubble_from_topic(&mut tx, topic_id).await?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_message_attachment(
    app_handle: tauri::AppHandle,
    topic_id: String,
    message_id: String,
    hash: String,
) -> Result<(), String> {
    let db_state = app_handle.state::<crate::vcp_modules::db_manager::DbState>();
    delete_message_attachment_in_pool(
        &db_state.pool,
        &topic_id,
        &message_id,
        &hash,
        crate::vcp_modules::infra::utils::now_millis(),
    )
    .await
}

#[cfg(test)]
mod attachment_deletion_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn deleting_attachment_hides_only_target_and_rehashes_message() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        for statement in [
            "CREATE TABLE agents (agent_id TEXT PRIMARY KEY, content_hash TEXT NOT NULL DEFAULT '')",
            "CREATE TABLE topics (
                topic_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, owner_type TEXT NOT NULL,
                title TEXT NOT NULL, created_at BIGINT NOT NULL, locked INTEGER NOT NULL DEFAULT 0,
                unread INTEGER NOT NULL DEFAULT 0, config_hash TEXT NOT NULL DEFAULT '',
                content_hash TEXT NOT NULL DEFAULT '', deleted_at BIGINT
            )",
            "CREATE TABLE messages (
                topic_id TEXT NOT NULL, msg_id TEXT NOT NULL, content BLOB NOT NULL,
                content_hash TEXT NOT NULL DEFAULT '', timestamp BIGINT NOT NULL,
                updated_at BIGINT NOT NULL, deleted_at BIGINT,
                PRIMARY KEY (topic_id, msg_id)
            )",
            "CREATE TABLE message_attachments (
                topic_id TEXT NOT NULL, msg_id TEXT NOT NULL, hash TEXT NOT NULL,
                attachment_order INTEGER NOT NULL, deleted_at BIGINT,
                PRIMARY KEY (topic_id, msg_id, attachment_order)
            )",
            "INSERT INTO agents (agent_id) VALUES ('agent-alpha')",
            "INSERT INTO topics (
                topic_id, owner_id, owner_type, title, created_at
             ) VALUES ('topic-alpha', 'agent-alpha', 'agent', 'Topic', 1)",
            "INSERT INTO message_attachments
                (topic_id, msg_id, hash, attachment_order)
             VALUES ('topic-alpha', 'message-alpha', 'hash-a', 0)",
            "INSERT INTO message_attachments
                (topic_id, msg_id, hash, attachment_order)
             VALUES ('topic-alpha', 'message-alpha', 'hash-b', 1)",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }

        let content = "hello";
        let compressed = ContentCompressor::compress(content).unwrap();
        sqlx::query(
            "INSERT INTO messages
                (topic_id, msg_id, content, timestamp, updated_at)
             VALUES ('topic-alpha', 'message-alpha', ?, 1, 1)",
        )
        .bind(compressed)
        .execute(&pool)
        .await
        .unwrap();

        delete_message_attachment_in_pool(&pool, "topic-alpha", "message-alpha", "hash-a", 42)
            .await
            .unwrap();

        let deleted_at: Option<i64> = sqlx::query_scalar(
            "SELECT deleted_at FROM message_attachments
             WHERE topic_id = 'topic-alpha' AND msg_id = 'message-alpha' AND hash = 'hash-a'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let visible_hashes: Vec<String> = sqlx::query_scalar(
            "SELECT hash FROM message_attachments
             WHERE topic_id = 'topic-alpha' AND msg_id = 'message-alpha'
               AND deleted_at IS NULL ORDER BY hash",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let message_hash: String = sqlx::query_scalar(
            "SELECT content_hash FROM messages
             WHERE topic_id = 'topic-alpha' AND msg_id = 'message-alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(deleted_at, Some(42));
        assert_eq!(visible_hashes, vec!["hash-b"]);
        assert_eq!(
            message_hash,
            HashAggregator::compute_message_fingerprint(content, &["hash-b".to_string()])
        );
    }

    #[tokio::test]
    async fn deleting_message_rehashes_topic_and_owner_in_the_same_transaction() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        for statement in [
            "CREATE TABLE agents (agent_id TEXT PRIMARY KEY, content_hash TEXT NOT NULL DEFAULT '')",
            "CREATE TABLE topics (
                topic_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, owner_type TEXT NOT NULL,
                title TEXT NOT NULL, created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL,
                locked INTEGER NOT NULL DEFAULT 0, unread INTEGER NOT NULL DEFAULT 0,
                msg_count INTEGER NOT NULL DEFAULT 0, config_hash TEXT NOT NULL DEFAULT '',
                content_hash TEXT NOT NULL DEFAULT '', deleted_at BIGINT
            )",
            "CREATE TABLE messages (
                topic_id TEXT NOT NULL, msg_id TEXT NOT NULL, content BLOB NOT NULL,
                content_hash TEXT NOT NULL DEFAULT '', timestamp BIGINT NOT NULL,
                updated_at BIGINT NOT NULL, deleted_at BIGINT,
                PRIMARY KEY (topic_id, msg_id)
            )",
            "CREATE TABLE render_cache (
                topic_id TEXT NOT NULL, msg_id TEXT NOT NULL,
                PRIMARY KEY (topic_id, msg_id)
            )",
            "CREATE TABLE message_attachments (
                topic_id TEXT NOT NULL, msg_id TEXT NOT NULL, hash TEXT NOT NULL
            )",
            "CREATE TABLE active_generations (
                msg_id TEXT PRIMARY KEY, topic_id TEXT NOT NULL
            )",
            "CREATE TABLE lifecycle_jobs (
                job_id TEXT PRIMARY KEY, topic_id TEXT NOT NULL,
                source_message_id TEXT, response_message_id TEXT,
                status TEXT NOT NULL, lease_until BIGINT,
                failure_reason TEXT, updated_at BIGINT NOT NULL
            )",
            "INSERT INTO agents (agent_id, content_hash) VALUES ('agent-alpha', 'stale')",
            "INSERT INTO topics (
                topic_id, owner_id, owner_type, title, created_at, updated_at, msg_count,
                config_hash, content_hash
             ) VALUES ('topic-alpha', 'agent-alpha', 'agent', 'Topic', 1, 1, 2, 'stale', 'stale')",
            "INSERT INTO messages (
                topic_id, msg_id, content, content_hash, timestamp, updated_at
             ) VALUES ('topic-alpha', 'message-a', X'00', 'hash-a', 1, 1)",
            "INSERT INTO messages (
                topic_id, msg_id, content, content_hash, timestamp, updated_at
             ) VALUES ('topic-alpha', 'message-b', X'00', 'hash-b', 2, 2)",
            "INSERT INTO active_generations (msg_id, topic_id) VALUES ('message-a', 'topic-alpha')",
            "INSERT INTO lifecycle_jobs (
                job_id, topic_id, source_message_id, status, lease_until, updated_at
             ) VALUES ('job-message-a', 'topic-alpha', 'message-a', 'running', 999, 1)",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }

        delete_messages(&pool, "topic-alpha", vec!["message-a".to_string()])
            .await
            .unwrap();

        let deleted_at: Option<i64> = sqlx::query_scalar(
            "SELECT deleted_at FROM messages WHERE topic_id = 'topic-alpha' AND msg_id = 'message-a'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(deleted_at.is_some());

        let (msg_count, topic_hash): (i64, String) = sqlx::query_as(
            "SELECT msg_count, content_hash FROM topics WHERE topic_id = 'topic-alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(msg_count, 1);
        assert_eq!(
            topic_hash,
            crate::vcp_modules::sync_types::compute_merkle_root(vec!["hash-b".to_string()])
        );

        let owner_hash: String =
            sqlx::query_scalar("SELECT content_hash FROM agents WHERE agent_id = 'agent-alpha'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_ne!(owner_hash, "stale");

        let active_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM active_generations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(active_count, 0);

        let lifecycle: (String, Option<i64>, Option<String>) = sqlx::query_as(
            "SELECT status, lease_until, failure_reason
             FROM lifecycle_jobs WHERE job_id = 'job-message-a'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(lifecycle.0, "cancelled");
        assert_eq!(lifecycle.1, None);
        assert_eq!(
            lifecycle.2.as_deref(),
            Some("Source or response message was deleted")
        );
    }
}

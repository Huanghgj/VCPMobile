use crate::vcp_modules::chat_manager::{Attachment, ChatMessage};
use crate::vcp_modules::content_parser::ContentBlock;
use crate::vcp_modules::file_manager::get_attachments_root_dir;
use crate::vcp_modules::message_repository::MessageRepository;
use crate::vcp_modules::message_repository::{ContentCompressor, MessageRenderCompiler};
use crate::vcp_modules::render_repair::repair_message_content_before_persist;
use crate::vcp_modules::settings_manager;
use crate::vcp_modules::sync_hash::HashAggregator;
use sqlx::Row;
use std::path::Path;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager};
use tokio::fs;

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
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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

        sqlx::query("UPDATE topics SET updated_at = ?, msg_count = ? WHERE topic_id = ?")
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
        let content = ContentCompressor::decompress(&content_bytes).unwrap_or_default();
        let content_hash_raw: String = row.get("content_hash");
        let content_hash = if content_hash_raw.is_empty() {
            None
        } else {
            Some(content_hash_raw)
        };
        let blocks = match (&render_content, &render_content_hash, &content_hash) {
            (Some(bytes), Some(render_hash), Some(message_hash))
                if !render_hash.is_empty() && render_hash == message_hash =>
            {
                parse_render_bytes(Some(bytes.clone()))
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
             WHERE (ma.topic_id, ma.msg_id) IN ({})
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
    _owner_id: &str,
    _owner_type: &str,
    topic_id: &str,
    limit: Option<usize>,
    offset: Option<usize>,
    include_content: bool,
    include_extracted_text: bool,
    include_ui_render_data: bool,
) -> Result<Vec<ChatMessage>, String> {
    let db_state = _app_handle.state::<crate::vcp_modules::db_manager::DbState>();
    let pool = &db_state.pool;

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
             WHERE ma.topic_id = ? AND ma.msg_id IN ({})
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
             LEFT JOIN avatars av ON av.owner_id = a.agent_id AND av.owner_type = 'agent'
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
            "SELECT dominant_color FROM avatars WHERE owner_type = 'user' AND owner_id = 'user_avatar'",
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
            let content = ContentCompressor::decompress(&content_bytes).unwrap_or_default();
            Some(compute_message_hash_from_content(
                &content,
                attachments.as_deref(),
            ))
        } else {
            None
        };
        let effective_content_hash = content_hash_backfill.clone().unwrap_or(content_hash_raw);
        let content_hash = Some(effective_content_hash.clone());
        let cache_matches_content = match (&render_content, &render_content_hash, &content_hash) {
            (Some(_), Some(render_hash), Some(message_hash)) if !render_hash.is_empty() => {
                render_hash == message_hash
            }
            _ => false,
        };

        // 懒渲染策略：render_cache 指纹命中才直接用，避免旧 blocks 与新 content 串台
        let (blocks, content) = if !include_ui_render_data {
            let content = if include_content {
                ContentCompressor::decompress(&content_bytes).unwrap_or_default()
            } else {
                String::new()
            };
            (None, content)
        } else if cache_matches_content {
            let rb = render_content
                .as_ref()
                .expect("checked by cache_matches_content");
            let blocks = parse_render_bytes(Some(rb.clone()));
            let content = if include_content {
                ContentCompressor::decompress(&content_bytes).unwrap_or_default()
            } else {
                String::new()
            };
            (blocks, content)
        } else {
            // 未命中：解压 content → 编译 blocks → 异步回写 cache
            let decompressed = ContentCompressor::decompress(&content_bytes).unwrap_or_default();
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
                    let content_hash_for_cache = effective_content_hash.clone();
                    let content_hash_backfill_for_message = content_hash_backfill.clone();
                    tokio::spawn(async move {
                        let now = chrono::Utc::now().timestamp_millis();
                        if let Some(hash) = content_hash_backfill_for_message {
                            let _ = sqlx::query(
                                "UPDATE messages SET content_hash = ?, updated_at = ? \
                                 WHERE topic_id = ? AND msg_id = ? AND content_hash = ''",
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
                             VALUES (?, ?, ?, ?, ?) \
                             ON CONFLICT(topic_id, msg_id) DO UPDATE SET \
                             content_hash = excluded.content_hash, \
                             render_content = excluded.render_content, \
                             updated_at = excluded.updated_at"
                        )
                        .bind(&tid)
                        .bind(&mid)
                        .bind(&content_hash_for_cache)
                        .bind(&serialized)
                        .bind(now)
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
pub async fn load_chat_text_history_for_context(
    app_handle: &AppHandle,
    topic_id: &str,
    limit: Option<usize>,
    offset: Option<usize>,
    include_extracted_text: bool,
) -> Result<Vec<ChatMessage>, String> {
    let db_state = app_handle.state::<crate::vcp_modules::db_manager::DbState>();
    let pool = &db_state.pool;

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
             WHERE ma.topic_id = ? AND ma.msg_id IN ({})
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
        let content = ContentCompressor::decompress(&content_bytes).unwrap_or_default();

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
        };
        history.push(message);
    }

    history.reverse();
    Ok(history)
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
                        if let Ok(bytes) = resp.bytes().await {
                            let _ = fs::write(&local_path, bytes).await;
                        }
                    }
                    _ => {} // 下载失败则跳过，UI 会显示裂图
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
    _owner_id: &str,
    _owner_type: &str,
    topic_id: String,
    mut message: ChatMessage,
) -> Result<Vec<ContentBlock>, String> {
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

    sqlx::query("UPDATE topics SET updated_at = ?, msg_count = ? WHERE topic_id = ?")
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
) -> Result<String, String> {
    let db_state = app_handle.state::<crate::vcp_modules::db_manager::DbState>();
    let pool = &db_state.pool;

    let row = sqlx::query("SELECT content FROM messages WHERE msg_id = ?")
        .bind(&message_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

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

    let row =
        sqlx::query("SELECT content, content_hash FROM messages WHERE msg_id = ? AND topic_id = ?")
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
            sqlx::query(
                "INSERT INTO render_cache (topic_id, msg_id, content_hash, render_content, updated_at) \
                 VALUES (?, ?, ?, ?, ?) \
                 ON CONFLICT(topic_id, msg_id) DO UPDATE SET \
                 content_hash = excluded.content_hash, \
                 render_content = excluded.render_content, \
                 updated_at = excluded.updated_at",
            )
            .bind(&topic_id)
            .bind(&message_id)
            .bind(&content_hash)
            .bind(&serialized)
            .bind(now)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;

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
    _owner_id: &str,
    _owner_type: &str,
    topic_id: String,
    mut message: ChatMessage,
    skip_bubble: bool,
) -> Result<Vec<ContentBlock>, String> {
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
    sqlx::query("UPDATE topics SET updated_at = ?, msg_count = ? WHERE topic_id = ?")
        .bind(now)
        .bind(msg_count)
        .bind(&topic_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(blocks)
}

pub async fn delete_messages(
    db_pool: &sqlx::Pool<sqlx::Sqlite>,
    topic_id: &str,
    msg_ids: Vec<String>,
) -> Result<(), String> {
    if msg_ids.is_empty() {
        return Ok(());
    }
    let mut tx = db_pool.begin().await.map_err(|e| e.to_string())?;
    let delete_query = format!(
        "UPDATE messages SET deleted_at = ? WHERE topic_id = ? AND msg_id IN ({})",
        msg_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ")
    );
    let now = chrono::Utc::now().timestamp_millis();
    let mut q = sqlx::query(&delete_query).bind(now).bind(topic_id);
    for id in &msg_ids {
        q = q.bind(id);
    }
    q.execute(&mut *tx).await.map_err(|e| e.to_string())?;

    // 物理强清除 render_cache 缓存，杜绝幽灵缓存残留
    let delete_cache_query = format!(
        "DELETE FROM render_cache WHERE topic_id = ? AND msg_id IN ({})",
        msg_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ")
    );
    let mut q_cache = sqlx::query(&delete_cache_query).bind(topic_id);
    for id in &msg_ids {
        q_cache = q_cache.bind(id);
    }
    q_cache.execute(&mut *tx).await.map_err(|e| e.to_string())?;

    // 物理强清除 message_attachments 关联，防止孤立关联残留
    let delete_attachments_query = format!(
        "DELETE FROM message_attachments WHERE topic_id = ? AND msg_id IN ({})",
        msg_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ")
    );
    let mut q_attachments = sqlx::query(&delete_attachments_query).bind(topic_id);
    for id in &msg_ids {
        q_attachments = q_attachments.bind(id);
    }
    q_attachments
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    let msg_count: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?
    .unwrap_or(0);

    sqlx::query("UPDATE topics SET msg_count = ?, updated_at = ? WHERE topic_id = ?")
        .bind(msg_count)
        .bind(now)
        .bind(topic_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn truncate_history_after_timestamp(
    _app_handle: AppHandle,
    db_pool: &sqlx::Pool<sqlx::Sqlite>,
    _owner_id: &str,
    _owner_type: &str,
    topic_id: &str,
    timestamp: i64,
) -> Result<(), String> {
    let mut tx = db_pool.begin().await.map_err(|e| e.to_string())?;

    // 物理强清除 render_cache，消灭幽灵缓存
    sqlx::query("DELETE FROM render_cache WHERE topic_id = ? AND msg_id IN (SELECT msg_id FROM messages WHERE topic_id = ? AND timestamp > ?)")
        .bind(topic_id).bind(topic_id).bind(timestamp).execute(&mut *tx).await.map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM message_attachments WHERE topic_id = ? AND msg_id IN (SELECT msg_id FROM messages WHERE topic_id = ? AND timestamp > ?)")
        .bind(topic_id).bind(topic_id).bind(timestamp).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("UPDATE messages SET deleted_at = ? WHERE topic_id = ? AND timestamp > ?")
        .bind(now)
        .bind(topic_id)
        .bind(timestamp)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let msg_count: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE topic_id = ? AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?
    .unwrap_or(0);
    sqlx::query("UPDATE topics SET msg_count = ?, updated_at = ? WHERE topic_id = ?")
        .bind(msg_count)
        .bind(timestamp)
        .bind(topic_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Helper: Deserializes render_content bytes (JSON + zstd) into JSON blocks for frontend
fn parse_render_bytes(render_content: Option<Vec<u8>>) -> Option<serde_json::Value> {
    render_content.and_then(|bytes| {
        crate::vcp_modules::message_repository::MessageRenderCompiler::deserialize(&bytes)
            .ok()
            .and_then(
                |blocks: Vec<crate::vcp_modules::content_parser::ContentBlock>| {
                    serde_json::to_value(blocks).ok()
                },
            )
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
            if let Ok(Some(row)) = sqlx::query("SELECT name FROM agents WHERE id = ?")
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

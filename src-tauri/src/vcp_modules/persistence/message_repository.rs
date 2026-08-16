use crate::vcp_modules::chat_manager::ChatMessage;
use crate::vcp_modules::content_parser::{parse_content, ContentBlock};
use crate::vcp_modules::sync_hash::HashAggregator;
use serde::Serialize;

use sqlx::Row;
use std::collections::HashSet;
use std::io::Read;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

type CachedMessageContent = (String, String, String, Vec<u8>);
type CachedMessageBatch = Vec<CachedMessageContent>;
const RENDER_CACHE_VERSION: &str = "render-v5";
const MAX_DECOMPRESSED_MESSAGE_BYTES: usize = 128 * 1024 * 1024;

fn decompress_zstd_bounded(bytes: &[u8], data_kind: &str) -> Result<Vec<u8>, String> {
    let decoder = zstd::stream::read::Decoder::new(bytes)
        .map_err(|e| format!("zstd {data_kind} decoder initialization failed: {e}"))?;
    let mut decompressed = Vec::new();
    decoder
        .take((MAX_DECOMPRESSED_MESSAGE_BYTES + 1) as u64)
        .read_to_end(&mut decompressed)
        .map_err(|e| format!("zstd {data_kind} decompression failed: {e}"))?;

    if decompressed.len() > MAX_DECOMPRESSED_MESSAGE_BYTES {
        return Err(format!(
            "zstd {data_kind} exceeds the {} MiB decompressed safety limit",
            MAX_DECOMPRESSED_MESSAGE_BYTES / (1024 * 1024)
        ));
    }

    Ok(decompressed)
}

pub struct MessageRenderCompiler;

impl MessageRenderCompiler {
    pub fn cache_key(content_hash: &str) -> String {
        format!("{RENDER_CACHE_VERSION}:{content_hash}")
    }

    pub fn cache_matches(cache_hash: &str, content_hash: &str) -> bool {
        cache_hash == Self::cache_key(content_hash)
    }

    /// Compiles raw message content into AST blocks (the "astbin" format base)
    pub fn compile(content: &str) -> Vec<ContentBlock> {
        // Core parse (now robust enough to handle HTML natively via content_parser)
        parse_content(content)
    }

    /// Serializes AST blocks to compressed binary (JSON + zstd)
    pub fn serialize(blocks: &[ContentBlock]) -> Result<Vec<u8>, String> {
        let json_bytes =
            serde_json::to_vec(blocks).map_err(|e| format!("json serialize failed: {}", e))?;
        let compressed = zstd::bulk::compress(&json_bytes, 3)
            .map_err(|e| format!("zstd compress failed: {}", e))?;
        Ok(compressed)
    }

    /// Deserializes compressed binary back to AST blocks (JSON + zstd)
    pub fn deserialize(bytes: &[u8]) -> Result<Vec<ContentBlock>, String> {
        let decompressed = decompress_zstd_bounded(bytes, "render cache")?;
        serde_json::from_slice(&decompressed).map_err(|e| format!("json deserialize failed: {}", e))
    }
}

/// Simple zstd compressor for raw text content.
/// Text compresses very well (often 3-10x) with low overhead.
pub struct ContentCompressor;

impl ContentCompressor {
    pub fn compress(text: &str) -> Result<Vec<u8>, String> {
        zstd::bulk::compress(text.as_bytes(), 3)
            .map_err(|e| format!("zstd compress content failed: {}", e))
    }

    pub fn decompress(bytes: &[u8]) -> Result<String, String> {
        let decompressed = decompress_zstd_bounded(bytes, "message content")?;
        String::from_utf8(decompressed)
            .map_err(|e| format!("content decompression not valid utf-8: {}", e))
    }
}

#[tauri::command]
pub async fn process_message_content(
    _app_handle: AppHandle,
    content: String,
) -> Result<Vec<ContentBlock>, String> {
    // 1. 全量预解析 (调用统一的渲染编译器)
    let blocks = MessageRenderCompiler::compile(&content);

    Ok(blocks)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildProgress {
    pub current: usize,
    pub total: usize,
}

// =================================================================
// 通用三段流水线基础设施（Reader → Processor → Writer）
// =================================================================

fn open_maintenance_rusqlite(db_path: &std::path::Path) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(db_path).map_err(|e| e.to_string())?;
    conn.execute("PRAGMA journal_mode = WAL", []).ok();
    conn.execute("PRAGMA synchronous = NORMAL", []).ok();
    conn.execute("PRAGMA busy_timeout = 30000", []).ok();
    Ok(conn)
}

/// 分页流式读取已有渲染缓存的消息的 (topic_id, msg_id, content_hash, content_bytes)，不做任何解压
async fn stream_cached_message_contents(
    pool: &sqlx::SqlitePool,
    tx: mpsc::Sender<CachedMessageContent>,
) -> Result<(), String> {
    let mut last_rowid = 0i64;
    const FETCH_SIZE: i64 = 500;

    loop {
        let rows = sqlx::query(
            "SELECT m.rowid, m.topic_id, m.msg_id, m.content_hash, m.content \
             FROM messages m \
             INNER JOIN render_cache r ON m.topic_id = r.topic_id AND m.msg_id = r.msg_id \
             INNER JOIN topics t ON t.topic_id = m.topic_id \
             WHERE m.rowid > ? AND m.deleted_at IS NULL AND t.deleted_at IS NULL \
             ORDER BY m.rowid \
             LIMIT ?",
        )
        .bind(last_rowid)
        .bind(FETCH_SIZE)
        .fetch_all(pool)
        .await;

        match rows {
            Ok(rows) if !rows.is_empty() => {
                if let Some(last) = rows.last() {
                    last_rowid = last.get::<i64, _>(0);
                }
                for row in rows {
                    let topic_id: String = row.get("topic_id");
                    let msg_id: String = row.get("msg_id");
                    let content_hash: String = row.get("content_hash");
                    let content_bytes: Vec<u8> = row.get("content");
                    if tx
                        .send((topic_id, msg_id, content_hash, content_bytes))
                        .await
                        .is_err()
                    {
                        return Ok(());
                    }
                }
            }
            _ => break,
        }
    }
    Ok(())
}

/// 通用批量 UPDATE Writer，带进度发射
fn run_batch_update_writer(
    db_path: &std::path::Path,
    mut rx: mpsc::Receiver<CachedMessageBatch>,
    update_sql: &str,
    progress_event: &str,
    app_handle: AppHandle,
    total: usize,
) -> tokio::task::JoinHandle<Result<(), String>> {
    let update_sql = update_sql.to_string();
    let progress_event = progress_event.to_string();
    let db_path = db_path.to_path_buf();

    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut conn = open_maintenance_rusqlite(&db_path)?;
        let mut processed = 0;
        let mut last_emit_time = std::time::Instant::now();
        let emit_interval = std::time::Duration::from_millis(32);

        while let Some(batch) = rx.blocking_recv() {
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            {
                let mut stmt = tx.prepare_cached(&update_sql).map_err(|e| e.to_string())?;
                let now = chrono::Utc::now().timestamp_millis();
                for (topic_id, msg_id, content_hash, bytes) in batch {
                    // 适配 render_cache 的 5 参数 SQL (topic_id, msg_id, content_hash, bytes, now)
                    // 或 content_compress 的 3 参数 SQL (bytes, topic_id, msg_id)
                    if update_sql.contains("render_cache") {
                        stmt.execute(rusqlite::params![
                            topic_id,
                            msg_id,
                            content_hash,
                            bytes,
                            now
                        ])
                        .map_err(|e| e.to_string())?;
                    } else {
                        stmt.execute(rusqlite::params![bytes, topic_id, msg_id])
                            .map_err(|e| e.to_string())?;
                    }
                    processed += 1;
                }
            }
            tx.commit().map_err(|e| e.to_string())?;

            if last_emit_time.elapsed() >= emit_interval || processed == total {
                let _ = app_handle.emit(
                    &progress_event,
                    RebuildProgress {
                        current: processed,
                        total,
                    },
                );
                last_emit_time = std::time::Instant::now();
            }
        }
        Ok(())
    })
}

// =================================================================
// 任务 1：全量预渲染重建
// =================================================================

#[tauri::command]
pub async fn rebuild_all_pre_renders(app_handle: AppHandle) -> Result<(), String> {
    let db_state = app_handle.state::<crate::vcp_modules::db_manager::DbState>();
    let pool = db_state.pool.clone();
    let db_path = db_state.path.clone();

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM render_cache r \
         JOIN messages m ON m.topic_id = r.topic_id AND m.msg_id = r.msg_id \
         JOIN topics t ON t.topic_id = m.topic_id \
         WHERE m.deleted_at IS NULL AND t.deleted_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;

    if total == 0 {
        return Ok(());
    }

    #[cfg(target_os = "android")]
    let _ = tauri_plugin_vcp_mobile::stream::start_stream_service_inner(
        &app_handle,
        "[预渲染重建] VCP Mobile",
    );

    let (tx_compiler, rx_compiler) = mpsc::channel::<(String, String, String, String)>(1000);
    let (tx_writer, rx_writer) = mpsc::channel::<CachedMessageBatch>(100);
    let total_count = total as usize;

    // --- Stage 3: Writer ---
    let writer_handle = run_batch_update_writer(
        &db_path,
        rx_writer,
        "INSERT INTO render_cache (topic_id, msg_id, content_hash, render_content, updated_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(topic_id, msg_id) DO UPDATE SET content_hash = excluded.content_hash, render_content = excluded.render_content, updated_at = excluded.updated_at",
        "render_rebuild_progress",
        app_handle.clone(),
        total_count,
    );

    // --- Stage 2: Parallel Compiler Workers ---
    let concurrency = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 12);

    let rx_compiler = std::sync::Arc::new(tokio::sync::Mutex::new(rx_compiler));
    let mut compiler_handles = Vec::new();

    for _ in 0..concurrency {
        let rx_clone = rx_compiler.clone();
        let tx_writer_clone = tx_writer.clone();

        let handle = tokio::task::spawn_blocking(move || {
            let mut batch = Vec::with_capacity(50);
            loop {
                let item = {
                    let mut rx = rx_clone.blocking_lock();
                    rx.blocking_recv()
                };

                match item {
                    Some((topic_id, msg_id, content_hash, content)) => {
                        let blocks = MessageRenderCompiler::compile(&content);
                        if let Ok(bytes) = MessageRenderCompiler::serialize(&blocks) {
                            batch.push((
                                topic_id,
                                msg_id,
                                MessageRenderCompiler::cache_key(&content_hash),
                                bytes,
                            ));
                        }

                        if batch.len() >= 50
                            && tx_writer_clone
                                .blocking_send(std::mem::take(&mut batch))
                                .is_err()
                        {
                            break;
                        }
                    }
                    None => {
                        if !batch.is_empty() {
                            let _ = tx_writer_clone.blocking_send(batch);
                        }
                        break;
                    }
                }
            }
        });
        compiler_handles.push(handle);
    }

    // --- Stage 1: Reader ---
    let reader_handle = tokio::spawn(async move {
        let (tx_inner, mut rx_inner) = mpsc::channel::<CachedMessageContent>(1000);

        let stream_handle = tokio::spawn(async move {
            let _ = stream_cached_message_contents(&pool, tx_inner).await;
        });

        while let Some((topic_id, msg_id, content_hash, content_bytes)) = rx_inner.recv().await {
            let content = ContentCompressor::decompress(&content_bytes)
                .unwrap_or_else(|_| String::from_utf8_lossy(&content_bytes).to_string());
            if tx_compiler
                .send((topic_id, msg_id, content_hash, content))
                .await
                .is_err()
            {
                break;
            }
        }
        drop(tx_compiler);
        let _ = stream_handle.await;
    });

    // 等待流水线排空
    let _ = reader_handle.await;
    let _ = futures_util::future::join_all(compiler_handles).await;
    drop(tx_writer);

    let write_res = writer_handle.await.map_err(|e| e.to_string());

    #[cfg(target_os = "android")]
    let _ = tauri_plugin_vcp_mobile::stream::stop_stream_service_inner(
        &app_handle,
        "[预渲染重建] VCP Mobile",
    );

    write_res??;

    // 补偿 100% 进度
    let _ = app_handle.emit(
        "render_rebuild_progress",
        RebuildProgress {
            current: total_count,
            total: total_count,
        },
    );
    Ok(())
}

/// Internal message repository for DB operations
pub struct MessageRepository;

impl MessageRepository {
    pub async fn upsert_message(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        message: &ChatMessage,
        topic_id: &str,
        render_content: &[u8],
        skip_bubble: bool,
    ) -> Result<(), String> {
        let (tombstoned_attachment_hashes, reserved_attachment_orders) =
            Self::attachment_tombstones(tx, topic_id, &message.id).await?;

        // 1. 计算核心内容指纹 (通过 HashAggregator)
        let attachment_hashes: Vec<String> = message
            .attachments
            .as_ref()
            .map(|atts| {
                atts.iter()
                    .map(Self::attachment_hash)
                    .filter(|hash| !tombstoned_attachment_hashes.contains(hash))
                    .collect()
            })
            .unwrap_or_default();

        let content_hash =
            HashAggregator::compute_message_fingerprint(&message.content, &attachment_hashes);

        // 2. 插入或更新消息 (不含 render_content)
        let upsert_result = sqlx::query(
            "INSERT INTO messages (
                msg_id, topic_id, role, name, agent_id, content, timestamp,
                is_group_message, group_id, finish_reason,
                content_hash,
                created_at, updated_at
            ) SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
              WHERE EXISTS (
                SELECT 1 FROM topics WHERE topic_id = ? AND deleted_at IS NULL
              )
             ON CONFLICT(topic_id, msg_id) DO UPDATE SET
                content = excluded.content,
                role = excluded.role,
                name = excluded.name,
                agent_id = excluded.agent_id,
                timestamp = excluded.timestamp,
                is_group_message = excluded.is_group_message,
                group_id = excluded.group_id,
                finish_reason = excluded.finish_reason,
                content_hash = excluded.content_hash,
                updated_at = excluded.updated_at
              WHERE messages.deleted_at IS NULL",
        )
        .bind(&message.id)
        .bind(topic_id)
        .bind(&message.role)
        .bind(&message.name)
        .bind(&message.agent_id)
        .bind(ContentCompressor::compress(&message.content)?)
        .bind(message.timestamp as i64)
        .bind(message.is_group_message.unwrap_or(false))
        .bind(&message.group_id)
        .bind(&message.finish_reason)
        .bind(&content_hash)
        .bind(message.timestamp as i64) // created_at
        .bind(message.timestamp as i64) // updated_at
        .bind(topic_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;

        // A late stream finalizer must not recreate a deleted message or write into a deleted topic.
        if upsert_result.rows_affected() == 0 {
            return Err(format!(
                "Message {} was deleted or topic {} is no longer active",
                message.id, topic_id
            ));
        }

        // 2.1 插入或更新渲染缓存 (独立表)
        sqlx::query(
            "INSERT INTO render_cache (topic_id, msg_id, content_hash, render_content, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(topic_id, msg_id) DO UPDATE SET
                content_hash = excluded.content_hash,
                render_content = excluded.render_content,
                updated_at = excluded.updated_at",
        )
        .bind(topic_id)
        .bind(&message.id)
        .bind(MessageRenderCompiler::cache_key(&content_hash))
        .bind(render_content)
        .bind(message.timestamp as i64)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;

        // Handle attachments
        if let Some(ref attachments) = message.attachments {
            Self::upsert_attachments_for_message(
                tx,
                topic_id,
                &message.id,
                message.timestamp as i64,
                attachments,
                &tombstoned_attachment_hashes,
                &reserved_attachment_orders,
            )
            .await?;
        } else {
            sqlx::query(
                "DELETE FROM message_attachments \
                 WHERE topic_id = ? AND msg_id = ? AND deleted_at IS NULL",
            )
            .bind(topic_id)
            .bind(&message.id)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        }

        // 3. 触发聚合哈希冒泡 (通过 HashAggregator 统一处理)
        if !skip_bubble {
            HashAggregator::bubble_from_topic(tx, topic_id).await?;
        }

        Ok(())
    }

    fn attachment_hash(attachment: &crate::vcp_modules::chat_manager::Attachment) -> String {
        attachment
            .hash
            .as_ref()
            .filter(|hash| !hash.is_empty())
            .cloned()
            .unwrap_or_else(|| {
                crate::vcp_modules::infra::utils::calculate_sha256(attachment.src.as_bytes())
            })
    }

    async fn attachment_tombstones(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        topic_id: &str,
        msg_id: &str,
    ) -> Result<(HashSet<String>, HashSet<i32>), String> {
        let rows = sqlx::query(
            "SELECT hash, attachment_order FROM message_attachments \
             WHERE topic_id = ? AND msg_id = ? AND deleted_at IS NOT NULL",
        )
        .bind(topic_id)
        .bind(msg_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;

        let hashes = rows
            .iter()
            .map(|row| row.get::<String, _>("hash"))
            .collect();
        let orders = rows
            .iter()
            .map(|row| row.get::<i32, _>("attachment_order"))
            .collect();
        Ok((hashes, orders))
    }

    async fn upsert_attachments_for_message(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        topic_id: &str,
        msg_id: &str,
        timestamp: i64,
        attachments: &[crate::vcp_modules::chat_manager::Attachment],
        tombstoned_hashes: &HashSet<String>,
        reserved_orders: &HashSet<i32>,
    ) -> Result<(), String> {
        sqlx::query(
            "DELETE FROM message_attachments \
             WHERE topic_id = ? AND msg_id = ? AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .bind(msg_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;

        let mut attachment_order = 0i32;
        for att in attachments {
            let hash = Self::attachment_hash(att);
            if tombstoned_hashes.contains(&hash) {
                continue;
            }
            while reserved_orders.contains(&attachment_order) {
                attachment_order += 1;
            }

            let image_frames = att
                .image_frames
                .as_ref()
                .and_then(|frames| serde_json::to_string(frames).ok());

            sqlx::query(
                "INSERT INTO attachments (
                    hash, mime_type, size, internal_path, extracted_text, image_frames, thumbnail_path,
                    created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(hash) DO UPDATE SET
                    mime_type = excluded.mime_type,
                    size = excluded.size,
                    internal_path = excluded.internal_path,
                    extracted_text = COALESCE(attachments.extracted_text, excluded.extracted_text),
                    image_frames = COALESCE(attachments.image_frames, excluded.image_frames),
                    thumbnail_path = COALESCE(attachments.thumbnail_path, excluded.thumbnail_path),
                    updated_at = excluded.updated_at"
            )
            .bind(&hash)
            .bind(&att.r#type)
            .bind(att.size as i64)
            .bind(&att.internal_path)
            .bind(&att.extracted_text)
            .bind(image_frames)
            .bind(&att.thumbnail_path)
            .bind(timestamp)
            .bind(timestamp)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;

            sqlx::query(
                "INSERT INTO message_attachments (
                    topic_id, msg_id, hash, attachment_order, display_name, src, status, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(topic_id)
            .bind(msg_id)
            .bind(&hash)
            .bind(attachment_order)
            .bind(&att.name)
            .bind(&att.src)
            .bind(&att.status)
            .bind(timestamp)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;

            attachment_order += 1;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ContentCompressor, MessageRenderCompiler, MessageRepository};
    use crate::vcp_modules::chat_manager::{Attachment, ChatMessage};
    use crate::vcp_modules::sync_hash::HashAggregator;
    use sqlx::{sqlite::SqlitePoolOptions, Row};

    async fn repository_test_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for statement in [
            "CREATE TABLE topics (
                topic_id TEXT PRIMARY KEY,
                content_hash TEXT NOT NULL DEFAULT '',
                deleted_at BIGINT
            )",
            "CREATE TABLE messages (
                msg_id TEXT NOT NULL,
                topic_id TEXT NOT NULL,
                role TEXT NOT NULL,
                name TEXT,
                agent_id TEXT,
                content BLOB NOT NULL,
                timestamp BIGINT NOT NULL,
                is_group_message INTEGER NOT NULL DEFAULT 0,
                group_id TEXT,
                finish_reason TEXT,
                content_hash TEXT NOT NULL DEFAULT '',
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL,
                deleted_at BIGINT,
                PRIMARY KEY (topic_id, msg_id)
            )",
            "CREATE TABLE render_cache (
                topic_id TEXT NOT NULL,
                msg_id TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                render_content BLOB NOT NULL,
                updated_at BIGINT NOT NULL,
                PRIMARY KEY (topic_id, msg_id)
            )",
            "CREATE TABLE message_attachments (
                topic_id TEXT NOT NULL,
                msg_id TEXT NOT NULL,
                hash TEXT NOT NULL,
                attachment_order INTEGER NOT NULL,
                display_name TEXT NOT NULL,
                src TEXT,
                status TEXT,
                created_at BIGINT NOT NULL,
                deleted_at BIGINT,
                PRIMARY KEY (topic_id, msg_id, attachment_order)
            )",
            "CREATE TABLE attachments (
                hash TEXT PRIMARY KEY,
                mime_type TEXT NOT NULL,
                size BIGINT NOT NULL,
                internal_path TEXT NOT NULL,
                extracted_text TEXT,
                image_frames TEXT,
                thumbnail_path TEXT,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL
            )",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        pool
    }

    fn test_message(content: &str, timestamp: u64) -> ChatMessage {
        ChatMessage {
            id: "message-1".to_string(),
            role: "assistant".to_string(),
            name: Some("Agent".to_string()),
            agent_id: Some("agent-1".to_string()),
            content: content.to_string(),
            timestamp,
            is_thinking: Some(false),
            group_id: None,
            topic_id: Some("topic-1".to_string()),
            is_group_message: Some(false),
            finish_reason: Some("completed".to_string()),
            attachments: None,
            blocks: None,
            shell: None,
            content_hash: None,
            transient_context: None,
            transient_system_prompt: None,
        }
    }

    fn test_attachment(hash: &str) -> Attachment {
        Attachment {
            r#type: "image/png".to_string(),
            src: format!("file://{hash}.png"),
            name: format!("{hash}.png"),
            size: 10,
            hash: Some(hash.to_string()),
            status: Some("ready".to_string()),
            internal_path: format!("{hash}.png"),
            ..Attachment::default()
        }
    }

    #[test]
    fn render_cache_key_requires_current_compiler_version() {
        let content_hash = "message-fingerprint";
        let current = MessageRenderCompiler::cache_key(content_hash);

        assert!(MessageRenderCompiler::cache_matches(&current, content_hash));
        assert!(!MessageRenderCompiler::cache_matches(
            content_hash,
            content_hash
        ));
        assert!(!MessageRenderCompiler::cache_matches(
            "render-v1:message-fingerprint",
            content_hash
        ));
        assert!(!MessageRenderCompiler::cache_matches(
            "render-v2:message-fingerprint",
            content_hash
        ));
        assert!(!MessageRenderCompiler::cache_matches(
            "render-v3:message-fingerprint",
            content_hash
        ));
        assert!(!MessageRenderCompiler::cache_matches(
            "render-v4:message-fingerprint",
            content_hash
        ));
    }

    #[test]
    fn compressed_content_round_trips_past_the_legacy_16_mib_limit() {
        let marker = "VCP_LARGE_MESSAGE_TAIL";
        let content = format!("{}{}", "x".repeat(17 * 1024 * 1024), marker);

        let compressed = ContentCompressor::compress(&content).unwrap();
        let restored = ContentCompressor::decompress(&compressed).unwrap();

        assert_eq!(restored.len(), content.len());
        assert!(restored.ends_with(marker));
    }

    #[test]
    fn render_cache_round_trips_large_vcp_document_tail() {
        let marker = "VCP_LARGE_RENDER_TAIL";
        let content = format!(
            "<div id=\"vcp-root\"><p>{}{}</p></div>",
            "x".repeat(17 * 1024 * 1024),
            marker
        );
        let blocks = MessageRenderCompiler::compile(&content);

        let compressed = MessageRenderCompiler::serialize(&blocks).unwrap();
        let restored = MessageRenderCompiler::deserialize(&compressed).unwrap();
        let restored_json = serde_json::to_string(&restored).unwrap();

        assert!(restored_json.contains(marker));
    }

    #[tokio::test]
    async fn upsert_writes_into_an_active_topic() {
        let pool = repository_test_pool().await;
        sqlx::query("INSERT INTO topics (topic_id) VALUES ('topic-1')")
            .execute(&pool)
            .await
            .unwrap();
        let mut tx = pool.begin().await.unwrap();
        MessageRepository::upsert_message(
            &mut tx,
            &test_message("hello", 20),
            "topic-1",
            b"[]",
            true,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn late_upsert_does_not_revive_a_tombstoned_message() {
        let pool = repository_test_pool().await;
        sqlx::query("INSERT INTO topics (topic_id) VALUES ('topic-1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO messages (
                msg_id, topic_id, role, content, timestamp, created_at, updated_at, deleted_at
             ) VALUES ('message-1', 'topic-1', 'assistant', X'00', 10, 10, 10, 99)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        let result = MessageRepository::upsert_message(
            &mut tx,
            &test_message("late final", 20),
            "topic-1",
            b"[]",
            true,
        )
        .await;
        assert!(result.is_err());
        tx.rollback().await.unwrap();

        let row = sqlx::query("SELECT timestamp, deleted_at FROM messages WHERE topic_id = 'topic-1' AND msg_id = 'message-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<i64, _>("timestamp"), 10);
        assert_eq!(row.get::<i64, _>("deleted_at"), 99);
        let cache_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM render_cache")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(cache_count, 0);
    }

    #[tokio::test]
    async fn late_upsert_does_not_insert_into_a_deleted_topic() {
        let pool = repository_test_pool().await;
        sqlx::query("INSERT INTO topics (topic_id, deleted_at) VALUES ('topic-1', 99)")
            .execute(&pool)
            .await
            .unwrap();

        let mut tx = pool.begin().await.unwrap();
        let result = MessageRepository::upsert_message(
            &mut tx,
            &test_message("late final", 20),
            "topic-1",
            b"[]",
            true,
        )
        .await;
        assert!(result.is_err());
        tx.rollback().await.unwrap();

        let message_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(message_count, 0);
    }

    #[tokio::test]
    async fn upsert_preserves_attachment_tombstones_and_uses_filtered_fingerprint() {
        let pool = repository_test_pool().await;
        sqlx::query("INSERT INTO topics (topic_id) VALUES ('topic-1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO message_attachments (
                topic_id, msg_id, hash, attachment_order, display_name, src, status,
                created_at, deleted_at
             ) VALUES ('topic-1', 'message-1', 'hash-a', 0, 'a.png', '', 'ready', 1, 99)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut message = test_message("hello", 20);
        message.attachments = Some(vec![test_attachment("hash-a"), test_attachment("hash-b")]);
        let mut tx = pool.begin().await.unwrap();
        MessageRepository::upsert_message(&mut tx, &message, "topic-1", b"[]", true)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let relations = sqlx::query(
            "SELECT hash, attachment_order, deleted_at FROM message_attachments \
             WHERE topic_id = 'topic-1' AND msg_id = 'message-1' ORDER BY attachment_order",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(relations.len(), 2);
        assert_eq!(relations[0].get::<String, _>("hash"), "hash-a");
        assert_eq!(relations[0].get::<Option<i64>, _>("deleted_at"), Some(99));
        assert_eq!(relations[1].get::<String, _>("hash"), "hash-b");
        assert_eq!(relations[1].get::<i32, _>("attachment_order"), 1);
        assert_eq!(relations[1].get::<Option<i64>, _>("deleted_at"), None);

        let expected_hash =
            HashAggregator::compute_message_fingerprint("hello", &["hash-b".to_string()]);
        let message_hash: String = sqlx::query_scalar(
            "SELECT content_hash FROM messages WHERE topic_id = 'topic-1' AND msg_id = 'message-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let render_hash: String = sqlx::query_scalar(
            "SELECT content_hash FROM render_cache WHERE topic_id = 'topic-1' AND msg_id = 'message-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(message_hash, expected_hash);
        assert_eq!(
            render_hash,
            MessageRenderCompiler::cache_key(&expected_hash)
        );
    }
}

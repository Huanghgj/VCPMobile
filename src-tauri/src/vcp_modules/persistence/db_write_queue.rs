use crate::vcp_modules::chat_manager::ChatMessage;
use crate::vcp_modules::message_repository::MessageRenderCompiler;
use crate::vcp_modules::sync_dto::{
    AgentSyncDTO, AgentTopicSyncDTO, GroupSyncDTO, GroupTopicSyncDTO,
};
use crate::vcp_modules::sync_hash::HashAggregator;
use crate::vcp_modules::sync_logger::SyncLogger;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug)]
pub enum DbWriteTask {
    Agent {
        id: String,
        dto: AgentSyncDTO,
    },
    Group {
        id: String,
        dto: GroupSyncDTO,
    },
    Avatar {
        owner_type: String,
        owner_id: String,
        bytes: Vec<u8>,
    },
    AgentTopic {
        topic_id: String,
        dto: AgentTopicSyncDTO,
    },
    AgentTopicBatch {
        topics: Vec<(String, AgentTopicSyncDTO)>,
    },
    GroupTopic {
        topic_id: String,
        dto: GroupTopicSyncDTO,
    },
    GroupTopicBatch {
        topics: Vec<(String, GroupTopicSyncDTO)>,
    },
    TopicMessages {
        topic_id: String,
        messages: Vec<crate::vcp_modules::chat_manager::ChatMessage>,
        compressed_contents: Vec<Vec<u8>>,
        render_bytes: Vec<Vec<u8>>,
        content_hashes: Vec<String>,
        skip_bubble: bool,
    },
    Flush {
        tx: oneshot::Sender<Result<(), String>>,
    },
}

pub struct DbWriteQueue {
    sender: mpsc::Sender<DbWriteTask>,
    logger: Option<Arc<Mutex<SyncLogger>>>,
    db_path: std::path::PathBuf,
    _worker: Option<tokio::task::JoinHandle<()>>,
}

impl Clone for DbWriteQueue {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            logger: self.logger.clone(),
            db_path: self.db_path.clone(),
            _worker: None,
        }
    }
}

impl DbWriteQueue {
    pub fn new(_pool: sqlx::SqlitePool, db_path: std::path::PathBuf) -> Self {
        let (tx, mut rx) = mpsc::channel(256);
        let db_path_for_worker = db_path.clone();

        // 核心优化：利用 Mutex 持有持久连接，确保 spawn_blocking 之间 prepare_cached 缓存不失效
        let conn_holder: Arc<Mutex<Option<rusqlite::Connection>>> = Arc::new(Mutex::new(None));

        let worker = tokio::spawn(async move {
            log::info!("[DbWriteQueue] Worker started (Turbo rusqlite Mode)");

            let mut success_count = 0u32;
            let mut error_count = 0u32;
            let mut pending_error: Option<String> = None;

            while let Some(first_task) = rx.recv().await {
                // 如果第一个任务就是 Flush，直接确认
                if let DbWriteTask::Flush { tx } = first_task {
                    let result = pending_error.take().map_or(Ok(()), Err);
                    let _ = tx.send(result);
                    continue;
                }

                let mut tasks_in_this_tx = vec![first_task];
                let mut total_msg_count = 0u32;

                if let DbWriteTask::TopicMessages { messages, .. } = &tasks_in_this_tx[0] {
                    total_msg_count += messages.len() as u32;
                }

                let mut flush_tx_opt: Option<oneshot::Sender<Result<(), String>>> = None;

                while tasks_in_this_tx.len() < 200 && total_msg_count < 5000 {
                    let next_res =
                        tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;

                    match next_res {
                        Ok(Some(DbWriteTask::Flush { tx })) => {
                            flush_tx_opt = Some(tx);
                            break;
                        }
                        Ok(Some(task)) => {
                            if let DbWriteTask::TopicMessages { messages, .. } = &task {
                                total_msg_count += messages.len() as u32;
                            }
                            tasks_in_this_tx.push(task);
                        }
                        _ => break,
                    }
                }

                let db_path = db_path_for_worker.clone();
                let ch = conn_holder.clone();

                // [Turbo Phase 3] 使用 spawn_blocking + rusqlite 进行极致写入
                let result = tokio::task::spawn_blocking(move || {
                    let mut guard = ch.lock().unwrap_or_else(|poisoned| {
                        log::warn!("[DbWriteQueue] Recovering from a poisoned connection lock");
                        poisoned.into_inner()
                    });
                    if guard.is_none() {
                        let conn = rusqlite::Connection::open(&db_path)?;
                        // 极致性能调优 (仅在初始化连接时执行一次)
                        conn.pragma_update(None, "journal_mode", "WAL")?;
                        conn.pragma_update(None, "synchronous", "NORMAL")?;
                        conn.busy_timeout(std::time::Duration::from_millis(30000))?;
                        *guard = Some(conn);
                    }
                    let Some(conn) = guard.as_mut() else {
                        return Err(rusqlite::Error::InvalidQuery);
                    };
                    let tx = conn.transaction()?;

                    let mut affected_owners = HashSet::new();
                    let mut affected_topics = HashSet::new();

                    for task in tasks_in_this_tx {
                        match task {
                            DbWriteTask::Agent { id, dto } => {
                                Self::rusqlite_upsert_agent(&tx, &id, &dto)?;
                                affected_owners.insert((id, "agent".to_string()));
                            }
                            DbWriteTask::Group { id, dto } => {
                                Self::rusqlite_upsert_group(&tx, &id, &dto)?;
                                affected_owners.insert((id, "group".to_string()));
                            }
                            DbWriteTask::Avatar { owner_type, owner_id, bytes } => {
                                Self::rusqlite_upsert_avatar(&tx, &owner_type, &owner_id, &bytes)?;
                            }
                            DbWriteTask::AgentTopic { topic_id, dto } => {
                                Self::rusqlite_upsert_agent_topic(&tx, &topic_id, &dto)?;
                                affected_owners.insert((dto.owner_id, "agent".to_string()));
                            }
                            DbWriteTask::AgentTopicBatch { topics } => {
                                for (tid, dto) in topics {
                                    affected_owners.insert((dto.owner_id.clone(), "agent".to_string()));
                                    Self::rusqlite_upsert_agent_topic(&tx, &tid, &dto)?;
                                }
                            }
                            DbWriteTask::GroupTopic { topic_id, dto } => {
                                Self::rusqlite_upsert_group_topic(&tx, &topic_id, &dto)?;
                                affected_owners.insert((dto.owner_id, "group".to_string()));
                            }
                            DbWriteTask::GroupTopicBatch { topics } => {
                                for (tid, dto) in topics {
                                    affected_owners.insert((dto.owner_id.clone(), "group".to_string()));
                                    Self::rusqlite_upsert_group_topic(&tx, &tid, &dto)?;
                                }
                            }
                            DbWriteTask::TopicMessages { topic_id, messages, compressed_contents, render_bytes, content_hashes, skip_bubble } => {
                                if !skip_bubble {
                                    affected_topics.insert(topic_id.clone());
                                }
                                Self::rusqlite_upsert_messages_batch(&tx, &topic_id, messages, compressed_contents, render_bytes, content_hashes)?;
                            }
                            DbWriteTask::Flush { .. } => unreachable!(),
                        }
                    }

                    // [Phase 5] 统一冒泡：分层去重，批量校验存在，确保最小化开销
                    for topic_id in affected_topics {
                        Self::rusqlite_bubble_topic_hash(&tx, &topic_id)?;
                    }

                    // 批量提取 Owner 并去重校验
                    let mut unique_agents = HashSet::new();
                    let mut unique_groups = HashSet::new();
                    for (id, owner_type) in affected_owners {
                        if owner_type == "agent" {
                            unique_agents.insert(id);
                        } else if owner_type == "group" {
                            unique_groups.insert(id);
                        }
                    }

                    if !unique_agents.is_empty() {
                        let placeholders = vec!["?"; unique_agents.len()].join(",");
                        let sql = format!("SELECT agent_id FROM agents WHERE agent_id IN ({}) AND deleted_at IS NULL", placeholders);
                        let mut stmt = tx.prepare(&sql)?;
                        let valid_ids: Vec<String> = stmt.query_map(rusqlite::params_from_iter(unique_agents.iter()), |r| r.get(0))?
                            .filter_map(|r| r.ok()).collect();
                        for aid in valid_ids {
                            Self::rusqlite_bubble_agent_hash(&tx, &aid)?;
                        }
                    }

                    if !unique_groups.is_empty() {
                        let placeholders = vec!["?"; unique_groups.len()].join(",");
                        let sql = format!("SELECT group_id FROM groups WHERE group_id IN ({}) AND deleted_at IS NULL", placeholders);
                        let mut stmt = tx.prepare(&sql)?;
                        let valid_ids: Vec<String> = stmt.query_map(rusqlite::params_from_iter(unique_groups.iter()), |r| r.get(0))?
                            .filter_map(|r| r.ok()).collect();
                        for gid in valid_ids {
                            Self::rusqlite_bubble_group_hash(&tx, &gid)?;
                        }
                    }

                    tx.commit()?;
                    Ok::<(), rusqlite::Error>(())
                }).await;

                let batch_error = match result {
                    Ok(Ok(_)) => {
                        success_count += 1;
                        None
                    }
                    Ok(Err(e)) => {
                        error_count += 1;
                        log::error!("[DbWriteQueue] rusqlite execution error: {}", e);
                        Some(format!("Database write failed: {e}"))
                    }
                    Err(e) => {
                        error_count += 1;
                        log::error!("[DbWriteQueue] spawn_blocking error: {}", e);
                        Some(format!("Database write worker failed: {e}"))
                    }
                };
                if pending_error.is_none() {
                    pending_error = batch_error;
                }

                if let Some(tx) = flush_tx_opt {
                    let result = pending_error.take().map_or(Ok(()), Err);
                    let _ = tx.send(result);
                }
            }

            log::info!(
                "[DbWriteQueue] Worker stopped. Total: success={}, errors={}",
                success_count,
                error_count
            );
        });

        Self {
            sender: tx,
            logger: None,
            db_path,
            _worker: Some(worker),
        }
    }

    pub fn set_logger(&mut self, logger: Arc<Mutex<SyncLogger>>) {
        self.logger = Some(logger);
    }

    pub async fn submit(&self, task: DbWriteTask) -> Result<(), String> {
        self.sender.send(task).await.map_err(|error| {
            let message = format!("Database write queue is unavailable: {error}");
            log::error!("[DbWriteQueue] Submit error: {}", message);
            message
        })
    }

    pub async fn flush(&self) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(DbWriteTask::Flush { tx })
            .await
            .map_err(|error| format!("Failed to submit database flush: {error}"))?;
        rx.await
            .map_err(|error| format!("Database flush acknowledgement failed: {error}"))??;
        log::debug!("[DbWriteQueue] Flush completed");
        Ok(())
    }

    // --- rusqlite 事务级方法 ---

    fn rusqlite_owner_is_active(
        tx: &rusqlite::Transaction,
        owner_type: &str,
        owner_id: &str,
    ) -> rusqlite::Result<bool> {
        match owner_type {
            "agent" => tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM agents WHERE agent_id = ? AND deleted_at IS NULL)",
                [owner_id],
                |row| row.get(0),
            ),
            "group" => tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM groups WHERE group_id = ? AND deleted_at IS NULL)",
                [owner_id],
                |row| row.get(0),
            ),
            "user" | "system" => Ok(true),
            _ => Ok(false),
        }
    }

    fn rusqlite_upsert_agent(
        tx: &rusqlite::Transaction,
        id: &str,
        dto: &AgentSyncDTO,
    ) -> rusqlite::Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let config_hash = HashAggregator::compute_agent_config_hash(dto);

        let changed = tx.execute(
            "INSERT INTO agents (
                agent_id, name, system_prompt, model, temperature, 
                context_token_limit, max_output_tokens, 
                stream_output, config_hash, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(agent_id) DO UPDATE SET
                name = excluded.name, 
                system_prompt = excluded.system_prompt, 
                model = excluded.model, 
                temperature = excluded.temperature, 
                context_token_limit = excluded.context_token_limit, 
                max_output_tokens = excluded.max_output_tokens, 
                stream_output = excluded.stream_output, 
                config_hash = excluded.config_hash,
                updated_at = excluded.updated_at
             WHERE agents.deleted_at IS NULL",
            rusqlite::params![
                id,
                &dto.name,
                &dto.system_prompt,
                &dto.model,
                dto.temperature,
                dto.context_token_limit,
                dto.max_output_tokens,
                if dto.stream_output { 1 } else { 0 },
                &config_hash,
                now
            ],
        )?;

        if changed == 0 {
            return Ok(());
        }

        Ok(())
    }

    fn rusqlite_upsert_group(
        tx: &rusqlite::Transaction,
        id: &str,
        dto: &GroupSyncDTO,
    ) -> rusqlite::Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let config_hash = HashAggregator::compute_group_config_hash(dto);

        let changed = tx.execute(
            "INSERT INTO groups (
                group_id, name, mode,
                group_prompt, invite_prompt, use_unified_model, unified_model,
                tag_match_mode, created_at, config_hash, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(group_id) DO UPDATE SET
                name = excluded.name,
                mode = excluded.mode,
                group_prompt = excluded.group_prompt,
                invite_prompt = excluded.invite_prompt,
                use_unified_model = excluded.use_unified_model,
                unified_model = excluded.unified_model,
                tag_match_mode = excluded.tag_match_mode,
                created_at = excluded.created_at,
                config_hash = excluded.config_hash,
                updated_at = excluded.updated_at
             WHERE groups.deleted_at IS NULL",
            rusqlite::params![
                id,
                &dto.name,
                &dto.mode,
                &dto.group_prompt,
                &dto.invite_prompt,
                if dto.use_unified_model { 1 } else { 0 },
                &dto.unified_model,
                &dto.tag_match_mode,
                dto.created_at,
                &config_hash,
                now
            ],
        )?;

        if changed == 0 {
            return Ok(());
        }

        tx.execute("DELETE FROM group_members WHERE group_id = ?", [id])?;

        let member_tags = dto.member_tags.as_ref().and_then(|v| v.as_object());

        for member in &dto.members {
            let tag = member_tags
                .and_then(|m| m.get(member))
                .and_then(|v| v.as_str());
            tx.execute(
                "INSERT INTO group_members (group_id, agent_id, member_tag, sort_order, updated_at) VALUES (?, ?, ?, 0, ?)",
                rusqlite::params![id, member, tag, now]
            )?;
        }

        Ok(())
    }

    fn rusqlite_upsert_avatar(
        tx: &rusqlite::Transaction,
        owner_type: &str,
        owner_id: &str,
        bytes: &[u8],
    ) -> rusqlite::Result<()> {
        if !Self::rusqlite_owner_is_active(tx, owner_type, owner_id)? {
            return Ok(());
        }

        let hash = HashAggregator::compute_avatar_hash(bytes);
        let dominant_color: Option<String> = None;
        let now = chrono::Utc::now().timestamp_millis();

        tx.execute(
            "INSERT INTO avatars (owner_type, owner_id, avatar_hash, mime_type, image_data, dominant_color, updated_at) 
             VALUES (?, ?, ?, 'image/png', ?, ?, ?) 
             ON CONFLICT(owner_type, owner_id) DO UPDATE SET 
             avatar_hash=excluded.avatar_hash, image_data=excluded.image_data, dominant_color=excluded.dominant_color, updated_at=excluded.updated_at
             WHERE avatars.deleted_at IS NULL",
            rusqlite::params![owner_type, owner_id, &hash, bytes, &dominant_color, now]
        )?;

        Ok(())
    }

    fn rusqlite_upsert_agent_topic(
        tx: &rusqlite::Transaction,
        topic_id: &str,
        dto: &AgentTopicSyncDTO,
    ) -> rusqlite::Result<()> {
        if !Self::rusqlite_owner_is_active(tx, "agent", &dto.owner_id)? {
            return Ok(());
        }

        let now = chrono::Utc::now().timestamp_millis();

        let changed = tx.execute(
            "INSERT INTO topics (topic_id, title, owner_id, owner_type, created_at, locked, unread, updated_at)
            VALUES (?, ?, ?, 'agent', ?, ?, ?, ?)
            ON CONFLICT(topic_id) DO UPDATE SET
            title=excluded.title, locked=excluded.locked, unread=excluded.unread, updated_at=excluded.updated_at
            WHERE topics.deleted_at IS NULL
              AND topics.owner_id = excluded.owner_id
              AND topics.owner_type = excluded.owner_type",
            rusqlite::params![
                topic_id, &dto.name, &dto.owner_id, dto.created_at,
                if dto.locked { 1 } else { 0 },
                if dto.unread { 1 } else { 0 },
                now
            ]
        )?;

        if changed == 0 {
            return Ok(());
        }

        Ok(())
    }

    fn rusqlite_upsert_group_topic(
        tx: &rusqlite::Transaction,
        topic_id: &str,
        dto: &GroupTopicSyncDTO,
    ) -> rusqlite::Result<()> {
        if !Self::rusqlite_owner_is_active(tx, "group", &dto.owner_id)? {
            return Ok(());
        }

        let now = chrono::Utc::now().timestamp_millis();

        let changed = tx.execute(
            "INSERT INTO topics (topic_id, title, owner_id, owner_type, created_at, locked, unread, updated_at)
            VALUES (?, ?, ?, 'group', ?, 1, 0, ?)
            ON CONFLICT(topic_id) DO UPDATE SET
            title=excluded.title, updated_at=excluded.updated_at
            WHERE topics.deleted_at IS NULL
              AND topics.owner_id = excluded.owner_id
              AND topics.owner_type = excluded.owner_type",
            rusqlite::params![topic_id, &dto.name, &dto.owner_id, dto.created_at, now]
        )?;

        if changed == 0 {
            return Ok(());
        }

        Ok(())
    }

    fn rusqlite_upsert_messages_batch(
        tx: &rusqlite::Transaction,
        topic_id: &str,
        messages: Vec<ChatMessage>,
        compressed_contents: Vec<Vec<u8>>,
        render_bytes: Vec<Vec<u8>>,
        supplied_content_hashes: Vec<String>,
    ) -> rusqlite::Result<()> {
        if messages.is_empty() {
            return Ok(());
        }
        if compressed_contents.len() != messages.len()
            || render_bytes.len() != messages.len()
            || supplied_content_hashes.len() != messages.len()
        {
            log::error!(
                "[DbWriteQueue] Rejecting inconsistent message batch: messages={}, compressed={}, render={}, hashes={}",
                messages.len(),
                compressed_contents.len(),
                render_bytes.len(),
                supplied_content_hashes.len()
            );
            return Err(rusqlite::Error::InvalidQuery);
        }

        let topic_is_active = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM topics WHERE topic_id = ? AND deleted_at IS NULL)",
            [topic_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !topic_is_active {
            return Ok(());
        }

        let mut tombstone_stmt = tx
            .prepare("SELECT msg_id FROM messages WHERE topic_id = ? AND deleted_at IS NOT NULL")?;
        let tombstoned_ids: HashSet<String> = tombstone_stmt
            .query_map([topic_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let eligible_messages: Vec<(usize, &ChatMessage)> = messages
            .iter()
            .enumerate()
            .filter(|(_, message)| !tombstoned_ids.contains(&message.id))
            .collect();
        if eligible_messages.is_empty() {
            return Ok(());
        }

        let mut tombstoned_hashes: HashMap<String, HashSet<String>> = HashMap::new();
        let mut reserved_orders: HashMap<String, HashSet<i32>> = HashMap::new();
        {
            let mut statement = tx.prepare(
                "SELECT msg_id, hash, attachment_order FROM message_attachments \
                 WHERE topic_id = ? AND deleted_at IS NOT NULL",
            )?;
            let rows = statement.query_map([topic_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i32>(2)?,
                ))
            })?;
            for row in rows {
                let (msg_id, hash, attachment_order) = row?;
                tombstoned_hashes
                    .entry(msg_id.clone())
                    .or_default()
                    .insert(hash);
                reserved_orders
                    .entry(msg_id)
                    .or_default()
                    .insert(attachment_order);
            }
        }

        let effective_content_hashes: Vec<String> = messages
            .iter()
            .map(|message| {
                let deleted_hashes = tombstoned_hashes.get(&message.id);
                let attachment_hashes: Vec<String> = message
                    .attachments
                    .as_ref()
                    .map(|attachments| {
                        attachments
                            .iter()
                            .map(|attachment| {
                                attachment
                                    .hash
                                    .as_ref()
                                    .filter(|hash| !hash.is_empty())
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        crate::vcp_modules::infra::utils::calculate_sha256(
                                            attachment.src.as_bytes(),
                                        )
                                    })
                            })
                            .filter(|hash| {
                                deleted_hashes.is_none_or(|deleted| !deleted.contains(hash))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                HashAggregator::compute_message_fingerprint(&message.content, &attachment_hashes)
            })
            .collect();

        let now = chrono::Utc::now().timestamp_millis();

        // Phase 3: Turbo Mode - Chunked Bulk Insert
        const MAX_PARAMS: usize = 999;
        const PARAMS_PER_MSG: usize = 13;
        let chunk_size = MAX_PARAMS / PARAMS_PER_MSG;

        for chunk_indices in eligible_messages.chunks(chunk_size) {
            // 1. 批量插入 messages 表 (不含 render_content)
            let mut sql_msgs = String::from(
                "INSERT INTO messages (
                    msg_id, topic_id, role, name, agent_id, content, timestamp,
                    is_group_message, group_id, finish_reason,
                    content_hash, created_at, updated_at
                ) VALUES ",
            );

            for i in 0..chunk_indices.len() {
                if i > 0 {
                    sql_msgs.push_str(", ");
                }
                sql_msgs.push_str("(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)");
            }

            sql_msgs.push_str(
                " ON CONFLICT(topic_id, msg_id) DO UPDATE SET
                    content = excluded.content,
                    role = excluded.role,
                    name = excluded.name,
                    agent_id = excluded.agent_id,
                    is_group_message = excluded.is_group_message,
                    group_id = excluded.group_id,
                    finish_reason = excluded.finish_reason,
                    content_hash = excluded.content_hash,
                    updated_at = excluded.updated_at",
            );

            let mut stmt_msgs = tx.prepare_cached(&sql_msgs)?;
            let mut params_msgs: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

            for (idx, msg) in chunk_indices {
                params_msgs.push(Box::new(msg.id.clone()));
                params_msgs.push(Box::new(topic_id.to_string()));
                params_msgs.push(Box::new(msg.role.clone()));
                params_msgs.push(Box::new(msg.name.clone()));
                params_msgs.push(Box::new(msg.agent_id.clone()));
                params_msgs.push(Box::new(compressed_contents[*idx].clone()));
                params_msgs.push(Box::new(msg.timestamp as i64));
                params_msgs.push(Box::new(msg.is_group_message.unwrap_or(false)));
                params_msgs.push(Box::new(msg.group_id.clone()));
                params_msgs.push(Box::new(msg.finish_reason.clone()));
                params_msgs.push(Box::new(effective_content_hashes[*idx].clone()));
                params_msgs.push(Box::new(msg.timestamp as i64));
                params_msgs.push(Box::new(msg.timestamp as i64));
            }

            let refs_msgs: Vec<&dyn rusqlite::ToSql> =
                params_msgs.iter().map(|p| p.as_ref()).collect();
            stmt_msgs.execute(&*refs_msgs)?;

            // 2. 批量插入 render_cache 表
            // 过滤出有实际预渲染内容的消息（当预渲染关闭时，所有 render_bytes 均为空）
            let render_chunk: Vec<_> = chunk_indices
                .iter()
                .map(|&(idx, msg)| (idx, msg))
                .filter(|(idx, _)| !render_bytes[*idx].is_empty())
                .collect();

            if !render_chunk.is_empty() {
                let mut sql_render = String::from(
                    "INSERT INTO render_cache (topic_id, msg_id, content_hash, render_content, updated_at) VALUES ",
                );

                for i in 0..render_chunk.len() {
                    if i > 0 {
                        sql_render.push_str(", ");
                    }
                    sql_render.push_str("(?, ?, ?, ?, ?)");
                }

                sql_render.push_str(
                    " ON CONFLICT(topic_id, msg_id) DO UPDATE SET
                        content_hash = excluded.content_hash,
                        render_content = excluded.render_content,
                        updated_at = excluded.updated_at",
                );

                let mut stmt_render = tx.prepare_cached(&sql_render)?;
                let mut params_render: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

                for (idx, msg) in render_chunk {
                    params_render.push(Box::new(topic_id.to_string()));
                    params_render.push(Box::new(msg.id.clone()));
                    params_render.push(Box::new(MessageRenderCompiler::cache_key(
                        &effective_content_hashes[idx],
                    )));
                    params_render.push(Box::new(render_bytes[idx].clone()));
                    params_render.push(Box::new(now));
                }

                let refs_render: Vec<&dyn rusqlite::ToSql> =
                    params_render.iter().map(|p| p.as_ref()).collect();
                stmt_render.execute(&*refs_render)?;
            }
        }

        // Phase 4: Attachment Optimization
        let mut msg_ids = Vec::new();
        let mut all_relations = Vec::new();

        for (_, msg) in &eligible_messages {
            msg_ids.push(msg.id.clone());
            let deleted_hashes = tombstoned_hashes.get(&msg.id);
            let message_reserved_orders = reserved_orders.get(&msg.id);
            let mut attachment_order = 0i32;
            if let Some(ref attachments) = msg.attachments {
                for att in attachments {
                    let hash = att
                        .hash
                        .as_ref()
                        .filter(|hash| !hash.is_empty())
                        .cloned()
                        .unwrap_or_else(|| {
                            crate::vcp_modules::infra::utils::calculate_sha256(att.src.as_bytes())
                        });
                    if deleted_hashes.is_some_and(|deleted| deleted.contains(&hash)) {
                        continue;
                    }
                    while message_reserved_orders
                        .is_some_and(|reserved| reserved.contains(&attachment_order))
                    {
                        attachment_order += 1;
                    }

                    Self::rusqlite_upsert_attachment_core(tx, &hash, att, msg.timestamp as i64)?;

                    all_relations.push((
                        msg.id.clone(),
                        hash,
                        attachment_order,
                        att.name.clone(),
                        att.src.clone(),
                        att.status.clone().unwrap_or_else(|| "ready".to_string()),
                        msg.timestamp as i64,
                    ));
                    attachment_order += 1;
                }
            }
        }

        // Chunked Delete
        for chunk in msg_ids.chunks(999) {
            let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
            let sql = format!(
                "DELETE FROM message_attachments \
                 WHERE topic_id = ? AND msg_id IN ({}) AND deleted_at IS NULL",
                placeholders
            );
            let mut stmt = tx.prepare_cached(&sql)?;
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            params.push(Box::new(topic_id.to_string()));
            for id in chunk {
                params.push(Box::new(id.clone()));
            }
            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            stmt.execute(&*params_refs)?;
        }

        // Chunked Relation Insert
        if !all_relations.is_empty() {
            const PARAMS_PER_REL: usize = 8;
            let rel_chunk_size = MAX_PARAMS / PARAMS_PER_REL;
            for chunk in all_relations.chunks(rel_chunk_size) {
                let mut sql = String::from(
                    "INSERT INTO message_attachments (
                    topic_id, msg_id, hash, attachment_order, display_name, src, status, created_at
                ) VALUES ",
                );
                for i in 0..chunk.len() {
                    if i > 0 {
                        sql.push_str(", ");
                    }
                    sql.push_str("(?, ?, ?, ?, ?, ?, ?, ?)");
                }
                let mut stmt = tx.prepare_cached(&sql)?;
                let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
                for rel in chunk {
                    params.push(Box::new(topic_id.to_string()));
                    params.push(Box::new(rel.0.clone()));
                    params.push(Box::new(rel.1.clone()));
                    params.push(Box::new(rel.2));
                    params.push(Box::new(rel.3.clone()));
                    params.push(Box::new(rel.4.clone()));
                    params.push(Box::new(rel.5.clone()));
                    params.push(Box::new(rel.6));
                }
                let params_refs: Vec<&dyn rusqlite::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();
                stmt.execute(&*params_refs)?;
            }
        }

        Ok(())
    }

    fn rusqlite_bubble_topic_hash(
        tx: &rusqlite::Transaction,
        topic_id: &str,
    ) -> rusqlite::Result<()> {
        // 1. 计算 content_hash (消息聚合)
        let mut stmt = tx.prepare("SELECT content_hash FROM messages WHERE topic_id = ? AND deleted_at IS NULL ORDER BY timestamp ASC, msg_id ASC")?;
        let hashes: Vec<String> = stmt
            .query_map([topic_id], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        let root_hash = crate::vcp_modules::sync_types::compute_merkle_root(hashes);

        // 2. 计算 config_hash (元数据)
        let owner_type: String = tx.query_row(
            "SELECT owner_type FROM topics WHERE topic_id = ?",
            [topic_id],
            |r| r.get(0),
        )?;

        let config_hash = if owner_type == "agent" {
            let dto = Self::rusqlite_load_agent_topic_dto(tx, topic_id)?;
            HashAggregator::compute_agent_topic_metadata_hash(&dto)
        } else {
            let dto = Self::rusqlite_load_group_topic_dto(tx, topic_id)?;
            HashAggregator::compute_group_topic_metadata_hash(&dto)
        };

        tx.execute(
            "UPDATE topics SET content_hash = ?, config_hash = ? WHERE topic_id = ?",
            rusqlite::params![root_hash, config_hash, topic_id],
        )?;
        Ok(())
    }

    fn rusqlite_bubble_agent_hash(
        tx: &rusqlite::Transaction,
        agent_id: &str,
    ) -> rusqlite::Result<()> {
        let mut stmt = tx.prepare("SELECT config_hash, content_hash FROM topics WHERE owner_id = ? AND owner_type = 'agent' AND deleted_at IS NULL ORDER BY topic_id ASC")?;
        let mut rows = stmt.query([agent_id])?;
        let mut hashes = Vec::new();
        while let Some(row) = rows.next()? {
            hashes.push(row.get::<_, String>(0)?);
            hashes.push(row.get::<_, String>(1)?);
        }
        let root_hash = crate::vcp_modules::sync_types::compute_merkle_root(hashes);
        tx.execute(
            "UPDATE agents SET content_hash = ? WHERE agent_id = ?",
            [root_hash, agent_id.to_string()],
        )?;
        Ok(())
    }

    fn rusqlite_bubble_group_hash(
        tx: &rusqlite::Transaction,
        group_id: &str,
    ) -> rusqlite::Result<()> {
        let mut stmt = tx.prepare("SELECT config_hash, content_hash FROM topics WHERE owner_id = ? AND owner_type = 'group' AND deleted_at IS NULL ORDER BY topic_id ASC")?;
        let mut rows = stmt.query([group_id])?;
        let mut hashes = Vec::new();
        while let Some(row) = rows.next()? {
            hashes.push(row.get::<_, String>(0)?);
            hashes.push(row.get::<_, String>(1)?);
        }
        let root_hash = crate::vcp_modules::sync_types::compute_merkle_root(hashes);
        tx.execute(
            "UPDATE groups SET content_hash = ? WHERE group_id = ?",
            [root_hash, group_id.to_string()],
        )?;
        Ok(())
    }

    fn rusqlite_load_agent_topic_dto(
        tx: &rusqlite::Transaction,
        topic_id: &str,
    ) -> rusqlite::Result<AgentTopicSyncDTO> {
        tx.query_row(
            "SELECT topic_id, title, created_at, locked, unread, owner_id FROM topics WHERE topic_id = ?",
            [topic_id],
            |row| Ok(AgentTopicSyncDTO {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                locked: row.get::<_, i64>(3)? != 0,
                unread: row.get::<_, i64>(4)? != 0,
                owner_id: row.get(5)?,
            })
        )
    }

    fn rusqlite_load_group_topic_dto(
        tx: &rusqlite::Transaction,
        topic_id: &str,
    ) -> rusqlite::Result<GroupTopicSyncDTO> {
        tx.query_row(
            "SELECT topic_id, title, created_at, owner_id FROM topics WHERE topic_id = ?",
            [topic_id],
            |row| {
                Ok(GroupTopicSyncDTO {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                    owner_id: row.get(3)?,
                })
            },
        )
    }

    fn rusqlite_upsert_attachment_core(
        tx: &rusqlite::Transaction,
        hash: &str,
        att: &crate::vcp_modules::chat_manager::Attachment,
        timestamp: i64,
    ) -> rusqlite::Result<()> {
        let image_frames = att
            .image_frames
            .as_ref()
            .and_then(|frames| serde_json::to_string(frames).ok());

        tx.execute(
            "INSERT INTO attachments (
                hash, mime_type, size, internal_path, extracted_text, image_frames, thumbnail_path,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(hash) DO UPDATE SET
                mime_type = excluded.mime_type,
                size = excluded.size,
                internal_path = excluded.internal_path,
                extracted_text = excluded.extracted_text,
                image_frames = excluded.image_frames,
                thumbnail_path = excluded.thumbnail_path,
                updated_at = excluded.updated_at",
            rusqlite::params![
                hash,
                &att.r#type,
                att.size as i64,
                &att.internal_path,
                &att.extracted_text,
                image_frames,
                &att.thumbnail_path,
                timestamp,
                timestamp
            ],
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{DbWriteQueue, DbWriteTask, MessageRenderCompiler};
    use crate::vcp_modules::chat_manager::{Attachment, ChatMessage};
    use crate::vcp_modules::message_repository::ContentCompressor;
    use crate::vcp_modules::sync_dto::{
        AgentSyncDTO, AgentTopicSyncDTO, GroupSyncDTO, GroupTopicSyncDTO,
    };
    use crate::vcp_modules::sync_hash::HashAggregator;

    fn test_connection() -> rusqlite::Connection {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        for statement in [
            "CREATE TABLE topics (topic_id TEXT PRIMARY KEY, deleted_at BIGINT)",
            "CREATE TABLE messages (
                msg_id TEXT NOT NULL, topic_id TEXT NOT NULL, role TEXT NOT NULL, name TEXT,
                agent_id TEXT, content BLOB NOT NULL, timestamp BIGINT NOT NULL,
                is_group_message INTEGER NOT NULL DEFAULT 0, group_id TEXT, finish_reason TEXT,
                content_hash TEXT NOT NULL DEFAULT '', created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL, deleted_at BIGINT,
                PRIMARY KEY (topic_id, msg_id)
            )",
            "CREATE TABLE render_cache (
                topic_id TEXT NOT NULL, msg_id TEXT NOT NULL, content_hash TEXT NOT NULL,
                render_content BLOB NOT NULL, updated_at BIGINT NOT NULL,
                PRIMARY KEY (topic_id, msg_id)
            )",
            "CREATE TABLE message_attachments (
                topic_id TEXT NOT NULL, msg_id TEXT NOT NULL, hash TEXT NOT NULL,
                attachment_order INTEGER NOT NULL DEFAULT 0, display_name TEXT NOT NULL DEFAULT '',
                src TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT '', created_at BIGINT NOT NULL,
                deleted_at BIGINT,
                PRIMARY KEY (topic_id, msg_id, attachment_order)
            )",
            "CREATE TABLE attachments (
                hash TEXT PRIMARY KEY, mime_type TEXT NOT NULL, size BIGINT NOT NULL,
                internal_path TEXT NOT NULL, extracted_text TEXT, image_frames TEXT,
                thumbnail_path TEXT, created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL
            )",
        ] {
            connection.execute_batch(statement).unwrap();
        }
        connection
    }

    fn metadata_test_connection() -> rusqlite::Connection {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE agents (
                    agent_id TEXT PRIMARY KEY, name TEXT NOT NULL, system_prompt TEXT NOT NULL,
                    model TEXT NOT NULL, temperature REAL NOT NULL, context_token_limit INTEGER NOT NULL,
                    max_output_tokens INTEGER NOT NULL, stream_output INTEGER NOT NULL,
                    config_hash TEXT NOT NULL DEFAULT '', updated_at BIGINT NOT NULL, deleted_at BIGINT
                );
                CREATE TABLE groups (
                    group_id TEXT PRIMARY KEY, name TEXT NOT NULL, mode TEXT NOT NULL,
                    group_prompt TEXT, invite_prompt TEXT, use_unified_model INTEGER NOT NULL,
                    unified_model TEXT, tag_match_mode TEXT, created_at BIGINT NOT NULL,
                    config_hash TEXT NOT NULL DEFAULT '', updated_at BIGINT NOT NULL, deleted_at BIGINT
                );
                CREATE TABLE group_members (
                    group_id TEXT NOT NULL, agent_id TEXT NOT NULL, member_tag TEXT,
                    sort_order INTEGER NOT NULL, updated_at BIGINT NOT NULL,
                    PRIMARY KEY (group_id, agent_id)
                );
                CREATE TABLE topics (
                    topic_id TEXT PRIMARY KEY, title TEXT NOT NULL, owner_id TEXT NOT NULL,
                    owner_type TEXT NOT NULL, created_at BIGINT NOT NULL, locked INTEGER NOT NULL,
                    unread INTEGER NOT NULL, updated_at BIGINT NOT NULL, deleted_at BIGINT
                );
                CREATE TABLE avatars (
                    owner_type TEXT NOT NULL, owner_id TEXT NOT NULL, avatar_hash TEXT NOT NULL,
                    mime_type TEXT NOT NULL, image_data BLOB NOT NULL, dominant_color TEXT,
                    updated_at BIGINT NOT NULL, deleted_at BIGINT,
                    PRIMARY KEY (owner_type, owner_id)
                );",
            )
            .unwrap();
        connection
    }

    fn agent_dto(name: &str) -> AgentSyncDTO {
        AgentSyncDTO {
            name: name.to_string(),
            system_prompt: "prompt".to_string(),
            model: "model".to_string(),
            temperature: 1.0,
            context_token_limit: 1024,
            max_output_tokens: 256,
            stream_output: true,
        }
    }

    fn group_dto(name: &str) -> GroupSyncDTO {
        GroupSyncDTO {
            name: name.to_string(),
            members: Vec::new(),
            mode: "sequential".to_string(),
            member_tags: None,
            group_prompt: None,
            invite_prompt: None,
            use_unified_model: false,
            unified_model: None,
            tag_match_mode: None,
            created_at: 1,
        }
    }

    fn test_message(timestamp: u64) -> ChatMessage {
        ChatMessage {
            id: "message-1".to_string(),
            role: "assistant".to_string(),
            name: None,
            agent_id: Some("agent-1".to_string()),
            content: "remote content".to_string(),
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

    #[tokio::test]
    async fn flush_reports_a_worker_transaction_failure() {
        let db_path = std::env::temp_dir().join(format!(
            "vcp-mobile-broken-sync-{}.db",
            uuid::Uuid::new_v4()
        ));
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let queue = DbWriteQueue::new(pool, db_path.clone());

        queue
            .submit(DbWriteTask::Agent {
                id: "agent-1".to_string(),
                dto: agent_dto("Agent"),
            })
            .await
            .unwrap();

        let error = queue.flush().await.unwrap_err();
        assert!(error.contains("Database write failed"));
        assert!(error.contains("agents"));

        drop(queue);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn sync_batch_does_not_revive_a_tombstoned_message() {
        let mut connection = test_connection();
        connection
            .execute("INSERT INTO topics (topic_id) VALUES ('topic-1')", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO messages (
                msg_id, topic_id, role, content, timestamp, created_at, updated_at, deleted_at
             ) VALUES ('message-1', 'topic-1', 'assistant', X'00', 10, 10, 10, 99)",
                [],
            )
            .unwrap();
        let transaction = connection.transaction().unwrap();
        DbWriteQueue::rusqlite_upsert_messages_batch(
            &transaction,
            "topic-1",
            vec![test_message(20)],
            vec![ContentCompressor::compress("remote content").unwrap()],
            vec![b"[]".to_vec()],
            vec!["remote-hash".to_string()],
        )
        .unwrap();
        transaction.commit().unwrap();

        let (timestamp, deleted_at): (i64, i64) = connection
            .query_row(
                "SELECT timestamp, deleted_at FROM messages WHERE topic_id = 'topic-1' AND msg_id = 'message-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(timestamp, 10);
        assert_eq!(deleted_at, 99);
        let cache_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM render_cache", [], |row| row.get(0))
            .unwrap();
        assert_eq!(cache_count, 0);
    }

    #[test]
    fn sync_batch_does_not_write_into_a_deleted_topic() {
        let mut connection = test_connection();
        connection
            .execute(
                "INSERT INTO topics (topic_id, deleted_at) VALUES ('topic-1', 99)",
                [],
            )
            .unwrap();
        let transaction = connection.transaction().unwrap();
        DbWriteQueue::rusqlite_upsert_messages_batch(
            &transaction,
            "topic-1",
            vec![test_message(20)],
            vec![ContentCompressor::compress("remote content").unwrap()],
            vec![b"[]".to_vec()],
            vec!["remote-hash".to_string()],
        )
        .unwrap();
        transaction.commit().unwrap();

        let message_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(message_count, 0);
    }

    #[test]
    fn sync_batch_preserves_attachment_tombstones_and_recomputes_cache_key() {
        let mut connection = test_connection();
        connection
            .execute("INSERT INTO topics (topic_id) VALUES ('topic-1')", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO message_attachments (
                    topic_id, msg_id, hash, attachment_order, display_name, src, status,
                    created_at, deleted_at
                 ) VALUES ('topic-1', 'message-1', 'hash-a', 0, 'a.png', '', 'ready', 1, 99)",
                [],
            )
            .unwrap();
        let mut message = test_message(20);
        message.attachments = Some(vec![test_attachment("hash-a"), test_attachment("hash-b")]);
        let expected_hash =
            HashAggregator::compute_message_fingerprint(&message.content, &["hash-b".to_string()]);

        let transaction = connection.transaction().unwrap();
        DbWriteQueue::rusqlite_upsert_messages_batch(
            &transaction,
            "topic-1",
            vec![message],
            vec![ContentCompressor::compress("remote content").unwrap()],
            vec![b"[]".to_vec()],
            vec!["stale-remote-hash".to_string()],
        )
        .unwrap();
        transaction.commit().unwrap();

        let mut statement = connection
            .prepare(
                "SELECT hash, attachment_order, deleted_at FROM message_attachments \
                 WHERE topic_id = 'topic-1' AND msg_id = 'message-1' ORDER BY attachment_order",
            )
            .unwrap();
        let relations: Vec<(String, i32, Option<i64>)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            relations,
            vec![
                ("hash-a".to_string(), 0, Some(99)),
                ("hash-b".to_string(), 1, None),
            ]
        );

        let (message_hash, render_hash): (String, String) = connection
            .query_row(
                "SELECT m.content_hash, r.content_hash FROM messages m \
                 JOIN render_cache r ON r.topic_id = m.topic_id AND r.msg_id = m.msg_id \
                 WHERE m.topic_id = 'topic-1' AND m.msg_id = 'message-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(message_hash, expected_hash);
        assert_eq!(
            render_hash,
            MessageRenderCompiler::cache_key(&expected_hash)
        );
    }

    #[test]
    fn sync_metadata_does_not_overwrite_owner_tombstones() {
        let mut connection = metadata_test_connection();
        connection
            .execute(
                "INSERT INTO agents VALUES ('agent-1', 'old-agent', '', 'old-model', 1, 1, 1, 1, '', 1, 99)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO groups VALUES ('group-1', 'old-group', 'sequential', NULL, NULL, 0, NULL, NULL, 1, '', 1, 99)",
                [],
            )
            .unwrap();

        let transaction = connection.transaction().unwrap();
        DbWriteQueue::rusqlite_upsert_agent(&transaction, "agent-1", &agent_dto("new-agent"))
            .unwrap();
        DbWriteQueue::rusqlite_upsert_group(&transaction, "group-1", &group_dto("new-group"))
            .unwrap();
        transaction.commit().unwrap();

        let agent: (String, i64) = connection
            .query_row(
                "SELECT name, deleted_at FROM agents WHERE agent_id = 'agent-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let group: (String, i64) = connection
            .query_row(
                "SELECT name, deleted_at FROM groups WHERE group_id = 'group-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(agent, ("old-agent".to_string(), 99));
        assert_eq!(group, ("old-group".to_string(), 99));
    }

    #[test]
    fn sync_topic_requires_an_active_matching_owner_and_preserves_tombstones() {
        let mut connection = metadata_test_connection();
        connection
            .execute(
                "INSERT INTO agents VALUES ('agent-active', 'Agent', '', 'model', 1, 1, 1, 1, '', 1, NULL)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO agents VALUES ('agent-deleted', 'Deleted', '', 'model', 1, 1, 1, 1, '', 1, 99)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO topics VALUES ('topic-tombstone', 'old-title', 'agent-active', 'agent', 1, 1, 0, 1, 99)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO topics VALUES ('topic-owned', 'owned-title', 'agent-active', 'agent', 1, 1, 0, 1, NULL)",
                [],
            )
            .unwrap();

        let tombstone_dto = AgentTopicSyncDTO {
            id: "topic-tombstone".to_string(),
            name: "new-title".to_string(),
            created_at: 2,
            locked: false,
            unread: true,
            owner_id: "agent-active".to_string(),
        };
        let mismatch_dto = AgentTopicSyncDTO {
            id: "topic-owned".to_string(),
            name: "wrong-owner-title".to_string(),
            created_at: 2,
            locked: false,
            unread: true,
            owner_id: "agent-deleted".to_string(),
        };
        let orphan_dto = AgentTopicSyncDTO {
            id: "topic-orphan".to_string(),
            name: "orphan".to_string(),
            created_at: 2,
            locked: true,
            unread: false,
            owner_id: "agent-deleted".to_string(),
        };

        let transaction = connection.transaction().unwrap();
        DbWriteQueue::rusqlite_upsert_agent_topic(&transaction, "topic-tombstone", &tombstone_dto)
            .unwrap();
        DbWriteQueue::rusqlite_upsert_agent_topic(&transaction, "topic-owned", &mismatch_dto)
            .unwrap();
        DbWriteQueue::rusqlite_upsert_agent_topic(&transaction, "topic-orphan", &orphan_dto)
            .unwrap();
        transaction.commit().unwrap();

        let tombstone: (String, i64) = connection
            .query_row(
                "SELECT title, deleted_at FROM topics WHERE topic_id = 'topic-tombstone'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let owned_title: String = connection
            .query_row(
                "SELECT title FROM topics WHERE topic_id = 'topic-owned'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let orphan_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM topics WHERE topic_id = 'topic-orphan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tombstone, ("old-title".to_string(), 99));
        assert_eq!(owned_title, "owned-title");
        assert_eq!(orphan_count, 0);
    }

    #[test]
    fn sync_avatar_does_not_recreate_deleted_owner_data() {
        let mut connection = metadata_test_connection();
        connection
            .execute(
                "INSERT INTO agents VALUES ('agent-deleted', 'Deleted', '', 'model', 1, 1, 1, 1, '', 1, 99)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO avatars VALUES ('agent', 'agent-deleted', 'old-hash', 'image/png', X'01', NULL, 1, 99)",
                [],
            )
            .unwrap();

        let transaction = connection.transaction().unwrap();
        DbWriteQueue::rusqlite_upsert_avatar(&transaction, "agent", "agent-deleted", &[2, 3])
            .unwrap();
        transaction.commit().unwrap();

        let avatar: (String, Vec<u8>, i64) = connection
            .query_row(
                "SELECT avatar_hash, image_data, deleted_at FROM avatars \
                 WHERE owner_type = 'agent' AND owner_id = 'agent-deleted'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(avatar.0, "old-hash");
        assert_eq!(avatar.1, vec![1]);
        assert_eq!(avatar.2, 99);
    }

    #[test]
    fn sync_group_topic_is_accepted_for_an_active_group() {
        let mut connection = metadata_test_connection();
        connection
            .execute(
                "INSERT INTO groups VALUES ('group-active', 'Group', 'sequential', NULL, NULL, 0, NULL, NULL, 1, '', 1, NULL)",
                [],
            )
            .unwrap();
        let dto = GroupTopicSyncDTO {
            id: "group-topic".to_string(),
            name: "Group topic".to_string(),
            created_at: 1,
            owner_id: "group-active".to_string(),
        };

        let transaction = connection.transaction().unwrap();
        DbWriteQueue::rusqlite_upsert_group_topic(&transaction, "group-topic", &dto).unwrap();
        transaction.commit().unwrap();

        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM topics WHERE topic_id = 'group-topic' \
                 AND owner_id = 'group-active' AND owner_type = 'group' AND deleted_at IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}

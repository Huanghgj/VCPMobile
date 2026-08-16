use crate::vcp_modules::db_manager::DbState;
use crate::vcp_modules::db_write_queue::DbWriteQueue;
use crate::vcp_modules::sync_hash::HashAggregator;
use crate::vcp_modules::sync_logger::{LogLevel, SyncLogger};
use crate::vcp_modules::sync_pipeline::SyncPipeline;
use crate::vcp_modules::sync_service::emit_sync_log;
use sqlx::Row;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use tauri::AppHandle;

pub struct SyncFinalizer;

async fn refresh_active_topic_counts(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    modified_topics: &HashSet<String>,
    updated_at: i64,
) -> Result<(), String> {
    if modified_topics.is_empty() {
        return Ok(());
    }
    let placeholders = modified_topics
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "UPDATE topics SET
            msg_count = (SELECT COUNT(*) FROM messages WHERE messages.topic_id = topics.topic_id AND deleted_at IS NULL),
            updated_at = ?
         WHERE topic_id IN ({}) AND deleted_at IS NULL",
        placeholders
    );
    let mut query = sqlx::query(&sql).bind(updated_at);
    for topic_id in modified_topics {
        query = query.bind(topic_id);
    }
    query
        .execute(&mut **tx)
        .await
        .map_err(|error| format!("[SyncFinalizer] Failed to refresh topic counts: {error}"))?;
    Ok(())
}

impl SyncFinalizer {
    pub async fn execute(
        app_handle: &AppHandle,
        db: &DbState,
        write_queue: &DbWriteQueue,
        pipeline: &SyncPipeline,
        logger: &Arc<Mutex<SyncLogger>>,
        modified_topics: HashSet<String>,
    ) -> Result<(), String> {
        // 1. 强制落盘数据库写队列
        write_queue.flush().await?;

        // 2. 全局 Hash 冒泡
        if !modified_topics.is_empty() {
            let start_instant = std::time::Instant::now();
            log::info!(
                "[SyncFinalizer] Finalizing {} modified topics (recalculating hashes)...",
                modified_topics.len()
            );
            emit_sync_log(
                app_handle,
                "info",
                &format!("正在校验 {} 个话题的一致性...", modified_topics.len()),
            );

            // [批量优化 Phase 1] 一次性批量预读取所有受影响话题的元数据到内存中，消灭循环内 N+1 读
            struct TopicBubbleMeta {
                owner_id: String,
                owner_type: String,
                title: String,
                created_at: i64,
                locked: bool,
                unread: bool,
            }

            let mut meta_map = std::collections::HashMap::new();
            let placeholders = modified_topics
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let query_sql = format!(
                "SELECT topic_id, owner_id, owner_type, title, created_at, locked, unread \
                 FROM topics WHERE topic_id IN ({}) AND deleted_at IS NULL",
                placeholders
            );
            let mut q = sqlx::query(&query_sql);
            for tid in &modified_topics {
                q = q.bind(tid);
            }

            let rows = q.fetch_all(&db.pool).await.map_err(|error| {
                format!("[SyncFinalizer] Failed to load modified topics: {error}")
            })?;
            for row in rows {
                let tid: String = row.get("topic_id");
                meta_map.insert(
                    tid,
                    TopicBubbleMeta {
                        owner_id: row.get("owner_id"),
                        owner_type: row.get("owner_type"),
                        title: row.get("title"),
                        created_at: row.get("created_at"),
                        locked: row.get::<i64, _>("locked") != 0,
                        unread: row.get::<i64, _>("unread") != 0,
                    },
                );
            }

            let mut bubbled_topics = 0usize;

            {
                let mut tx = db.pool.begin().await.map_err(|error| {
                    format!("[SyncFinalizer] Failed to begin transaction: {error}")
                })?;
                // 1. [Batch Optimization] 一条 SQL 更新所有受影响话题的消息计数和时间戳
                refresh_active_topic_counts(
                    &mut tx,
                    &modified_topics,
                    chrono::Utc::now().timestamp_millis(),
                )
                .await?;

                // 2. 逐话题计算指纹并向上冒泡（使用传参版接口，彻底避免折返 SELECT）
                let mut affected_agents: HashSet<String> = HashSet::new();
                let mut affected_groups: HashSet<String> = HashSet::new();

                for tid in &modified_topics {
                    if let Some(meta) = meta_map.get(tid) {
                        if let Err(error) = HashAggregator::bubble_topic_hash_with_meta(
                            &mut tx,
                            tid,
                            &meta.owner_type,
                            &meta.title,
                            meta.created_at,
                            meta.locked,
                            meta.unread,
                        )
                        .await
                        {
                            log::error!(
                                "[SyncFinalizer] bubble_topic_hash_with_meta failed for {}: {}",
                                tid,
                                error
                            );
                            if let Ok(mut l) = logger.lock() {
                                l.log(
                                    LogLevel::Error,
                                    "finalize",
                                    &format!("Bubble topic hash failed for {}: {}", tid, error),
                                );
                            }
                            return Err(format!(
                                "[SyncFinalizer] Failed to bubble topic {}: {}",
                                tid, error
                            ));
                        }
                        bubbled_topics += 1;

                        // 直接从内存提取 owner 归属，杜绝 N+1 读
                        if meta.owner_type == "agent" {
                            affected_agents.insert(meta.owner_id.clone());
                        } else if meta.owner_type == "group" {
                            affected_groups.insert(meta.owner_id.clone());
                        }
                    }
                }

                let agent_count = affected_agents.len();
                let group_count = affected_groups.len();

                for aid in affected_agents {
                    HashAggregator::bubble_agent_hash(&mut tx, &aid)
                        .await
                        .map_err(|error| {
                            format!("[SyncFinalizer] Failed to bubble agent {}: {}", aid, error)
                        })?;
                }
                for gid in affected_groups {
                    HashAggregator::bubble_group_hash(&mut tx, &gid)
                        .await
                        .map_err(|error| {
                            format!("[SyncFinalizer] Failed to bubble group {}: {}", gid, error)
                        })?;
                }

                match tx.commit().await {
                    Ok(_) => {
                        let elapsed = start_instant.elapsed();
                        let success_msg = format!(
                            "[SyncFinalizer] 一致性校验校验成功！耗时: {:?}. 冒泡话题: {}, 级联智能体: {}, 级联群组: {}.",
                            elapsed, bubbled_topics, agent_count, group_count
                        );
                        log::info!("{}", success_msg);
                        emit_sync_log(app_handle, "success", &success_msg);
                    }
                    Err(e) => {
                        let err_msg = format!("[SyncFinalizer] Transaction commit failed: {}", e);
                        log::error!("{}", err_msg);
                        emit_sync_log(app_handle, "error", &err_msg);
                        return Err(err_msg);
                    }
                }
            }
        }

        // 3. 推进 Pipeline 状态
        pipeline.on_messages_done().await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn refresh_counts_does_not_update_deleted_topics() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE topics (
                topic_id TEXT PRIMARY KEY,
                msg_count INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deleted_at INTEGER
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("CREATE TABLE messages (topic_id TEXT NOT NULL, deleted_at INTEGER)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO topics(topic_id, msg_count, updated_at, deleted_at)
             VALUES ('active', 0, 10, NULL), ('deleted', 7, 20, 100)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO messages(topic_id, deleted_at)
             VALUES ('active', NULL), ('active', 100), ('deleted', NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let modified_topics = HashSet::from(["active".to_string(), "deleted".to_string()]);
        let mut tx = pool.begin().await.unwrap();
        refresh_active_topic_counts(&mut tx, &modified_topics, 999)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let active: (i64, i64) =
            sqlx::query_as("SELECT msg_count, updated_at FROM topics WHERE topic_id = 'active'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let deleted: (i64, i64) =
            sqlx::query_as("SELECT msg_count, updated_at FROM topics WHERE topic_id = 'deleted'")
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(active, (1, 999));
        assert_eq!(deleted, (7, 20));
    }
}

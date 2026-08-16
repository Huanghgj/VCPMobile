use crate::vcp_modules::db_manager::DbState;
use crate::vcp_modules::message_repository::ContentCompressor;
use crate::vcp_modules::sync_hash::HashAggregator;
use sqlx::Row;
use tauri::{AppHandle, Manager, Runtime};

pub struct DeleteExecutor;

impl DeleteExecutor {
    async fn cancel_active_requests_for_topic<R: Runtime>(
        app: &AppHandle<R>,
        pool: &sqlx::SqlitePool,
        topic_id: &str,
    ) -> Result<(), String> {
        let message_ids: Vec<String> =
            sqlx::query_scalar("SELECT msg_id FROM active_generations WHERE topic_id = ?")
                .bind(topic_id)
                .fetch_all(pool)
                .await
                .map_err(|e| e.to_string())?;
        if let Some(active_requests) =
            app.try_state::<crate::vcp_modules::vcp_client::ActiveRequests>()
        {
            let group_turn_ids = active_requests.cancel_topic(topic_id);
            active_requests.cancel_ids(message_ids.iter().map(String::as_str));
            if let Some(cancelled_turns) =
                app.try_state::<crate::vcp_modules::vcp_client::CancelledGroupTurns>()
            {
                for turn_id in group_turn_ids {
                    cancelled_turns.0.insert(turn_id);
                }
            }
        }
        Ok(())
    }

    async fn cancel_active_requests_for_owner<R: Runtime>(
        app: &AppHandle<R>,
        pool: &sqlx::SqlitePool,
        owner_id: &str,
        owner_type: &str,
    ) -> Result<(), String> {
        let message_ids: Vec<String> = sqlx::query_scalar(
            "SELECT msg_id FROM active_generations WHERE owner_id = ? AND owner_type = ?",
        )
        .bind(owner_id)
        .bind(owner_type)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        if let Some(active_requests) =
            app.try_state::<crate::vcp_modules::vcp_client::ActiveRequests>()
        {
            let group_turn_ids = active_requests.cancel_owner(owner_id, owner_type);
            active_requests.cancel_ids(message_ids.iter().map(String::as_str));
            if let Some(cancelled_turns) =
                app.try_state::<crate::vcp_modules::vcp_client::CancelledGroupTurns>()
            {
                for turn_id in group_turn_ids {
                    cancelled_turns.0.insert(turn_id);
                }
            }
        }
        Ok(())
    }

    pub async fn soft_delete_agent<R: Runtime>(
        app: &AppHandle<R>,
        agent_id: &str,
    ) -> Result<(), String> {
        let db = app.state::<DbState>();
        Self::cancel_active_requests_for_owner(app, &db.pool, agent_id, "agent").await?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut tx = db.pool.begin().await.map_err(|e| e.to_string())?;

        sqlx::query("UPDATE agents SET deleted_at = ? WHERE agent_id = ?")
            .bind(now)
            .bind(agent_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        // 级联将该 Agent 下的所有话题标记为逻辑删除
        sqlx::query("UPDATE topics SET deleted_at = ? WHERE owner_id = ? AND owner_type = 'agent' AND deleted_at IS NULL")
            .bind(now)
            .bind(agent_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        // 级联将该 Agent 下所有话题的所有消息标记为逻辑删除
        sqlx::query("UPDATE messages SET deleted_at = ? WHERE topic_id IN (SELECT topic_id FROM topics WHERE owner_id = ? AND owner_type = 'agent') AND deleted_at IS NULL")
            .bind(now)
            .bind(agent_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query("UPDATE message_attachments SET deleted_at = ? WHERE topic_id IN (SELECT topic_id FROM topics WHERE owner_id = ? AND owner_type = 'agent') AND deleted_at IS NULL")
            .bind(now)
            .bind(agent_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query("DELETE FROM render_cache WHERE topic_id IN (SELECT topic_id FROM topics WHERE owner_id = ? AND owner_type = 'agent')")
            .bind(agent_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query("UPDATE avatars SET deleted_at = ? WHERE owner_type = 'agent' AND owner_id = ? AND deleted_at IS NULL")
            .bind(now)
            .bind(agent_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        // 级联清除该 Agent 下的所有活跃生成，杜绝已删除消息复活
        sqlx::query("DELETE FROM active_generations WHERE owner_id = ? AND owner_type = 'agent'")
            .bind(agent_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query(
            "UPDATE lifecycle_jobs SET status = 'cancelled', lease_until = NULL, \
             failure_reason = '所属 Agent 已删除', updated_at = ? \
             WHERE owner_id = ? AND owner_type = 'agent' \
             AND status NOT IN ('completed', 'cancelled')",
        )
        .bind(now)
        .bind(agent_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        HashAggregator::bubble_agent_hash(&mut tx, agent_id).await?;
        tx.commit().await.map_err(|e| e.to_string())?;

        if let Some(state) = app.try_state::<crate::vcp_modules::agent_service::AgentConfigState>()
        {
            state.caches.remove(agent_id);
        }

        Ok(())
    }

    pub async fn soft_delete_group<R: Runtime>(
        app: &AppHandle<R>,
        group_id: &str,
    ) -> Result<(), String> {
        let db = app.state::<DbState>();
        Self::cancel_active_requests_for_owner(app, &db.pool, group_id, "group").await?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut tx = db.pool.begin().await.map_err(|e| e.to_string())?;

        sqlx::query("UPDATE groups SET deleted_at = ? WHERE group_id = ?")
            .bind(now)
            .bind(group_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        // 级联将该 Group 下的所有话题标记为逻辑删除
        sqlx::query("UPDATE topics SET deleted_at = ? WHERE owner_id = ? AND owner_type = 'group' AND deleted_at IS NULL")
            .bind(now)
            .bind(group_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        // 级联将该 Group 下所有话题的所有消息标记为逻辑删除
        sqlx::query("UPDATE messages SET deleted_at = ? WHERE topic_id IN (SELECT topic_id FROM topics WHERE owner_id = ? AND owner_type = 'group') AND deleted_at IS NULL")
            .bind(now)
            .bind(group_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query("UPDATE message_attachments SET deleted_at = ? WHERE topic_id IN (SELECT topic_id FROM topics WHERE owner_id = ? AND owner_type = 'group') AND deleted_at IS NULL")
            .bind(now)
            .bind(group_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query("DELETE FROM render_cache WHERE topic_id IN (SELECT topic_id FROM topics WHERE owner_id = ? AND owner_type = 'group')")
            .bind(group_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query("UPDATE avatars SET deleted_at = ? WHERE owner_type = 'group' AND owner_id = ? AND deleted_at IS NULL")
            .bind(now)
            .bind(group_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        // 级联清除该 Group 下的所有活跃生成，杜绝已删除消息复活
        sqlx::query("DELETE FROM active_generations WHERE owner_id = ? AND owner_type = 'group'")
            .bind(group_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query(
            "UPDATE lifecycle_jobs SET status = 'cancelled', lease_until = NULL, \
             failure_reason = '所属群组已删除', updated_at = ? \
             WHERE owner_id = ? AND owner_type = 'group' \
             AND status NOT IN ('completed', 'cancelled')",
        )
        .bind(now)
        .bind(group_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        HashAggregator::bubble_group_hash(&mut tx, group_id).await?;
        tx.commit().await.map_err(|e| e.to_string())?;

        if let Some(state) = app.try_state::<crate::vcp_modules::group_service::GroupManagerState>()
        {
            state.caches.remove(group_id);
        }

        Ok(())
    }

    pub async fn soft_delete_topic<R: Runtime>(
        app: &AppHandle<R>,
        topic_id: &str,
    ) -> Result<(), String> {
        let db = app.state::<DbState>();
        Self::cancel_active_requests_for_topic(app, &db.pool, topic_id).await?;
        let now = chrono::Utc::now().timestamp_millis();
        let owner = sqlx::query(
            "SELECT owner_id, owner_type FROM topics WHERE topic_id = ? AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .fetch_optional(&db.pool)
        .await
        .map_err(|e| e.to_string())?
        .map(|row| {
            (
                row.get::<String, _>("owner_id"),
                row.get::<String, _>("owner_type"),
            )
        });

        Self::soft_delete_topic_in_pool(&db.pool, topic_id, now).await?;
        if let Some((owner_id, owner_type)) = owner {
            if owner_type == "agent" {
                if let Some(state) =
                    app.try_state::<crate::vcp_modules::agent_service::AgentConfigState>()
                {
                    state.caches.remove(&owner_id);
                }
            } else if owner_type == "group" {
                if let Some(state) =
                    app.try_state::<crate::vcp_modules::group_service::GroupManagerState>()
                {
                    state.caches.remove(&owner_id);
                }
            }
        }
        Ok(())
    }

    pub async fn soft_delete_message<R: Runtime>(
        app: &AppHandle<R>,
        message_id: &str,
    ) -> Result<(), String> {
        if message_id.is_empty() {
            return Err("Message id cannot be empty".to_string());
        }
        let db = app.state::<DbState>();
        if let Some(active_requests) =
            app.try_state::<crate::vcp_modules::vcp_client::ActiveRequests>()
        {
            active_requests.cancel_ids(std::iter::once(message_id));
        }
        Self::soft_delete_message_in_pool(&db.pool, message_id).await
    }

    async fn soft_delete_message_in_pool(
        pool: &sqlx::SqlitePool,
        message_id: &str,
    ) -> Result<(), String> {
        let topic_ids: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT m.topic_id FROM messages m \
             JOIN topics t ON t.topic_id = m.topic_id \
             WHERE m.msg_id = ? AND m.deleted_at IS NULL AND t.deleted_at IS NULL",
        )
        .bind(message_id)
        .fetch_all(pool)
        .await
        .map_err(|error| format!("Failed to locate message {message_id}: {error}"))?;

        for topic_id in topic_ids {
            crate::vcp_modules::message_service::delete_messages(
                pool,
                &topic_id,
                vec![message_id.to_string()],
            )
            .await?;
        }
        Ok(())
    }

    async fn soft_delete_topic_in_pool(
        pool: &sqlx::SqlitePool,
        topic_id: &str,
        now: i64,
    ) -> Result<(), String> {
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

        let parent_row = sqlx::query(
            "SELECT owner_id, owner_type FROM topics WHERE topic_id = ? AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        let Some(parent_row) = parent_row else {
            tx.commit().await.map_err(|e| e.to_string())?;
            return Ok(());
        };

        sqlx::query("UPDATE topics SET deleted_at = ? WHERE topic_id = ? AND deleted_at IS NULL")
            .bind(now)
            .bind(topic_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query(
            "UPDATE message_attachments SET deleted_at = ? \
             WHERE topic_id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(topic_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query("DELETE FROM render_cache WHERE topic_id = ?")
            .bind(topic_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        // 级联将该话题下的所有消息标记为逻辑删除
        sqlx::query("UPDATE messages SET deleted_at = ? WHERE topic_id = ? AND deleted_at IS NULL")
            .bind(now)
            .bind(topic_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        // 级联清除活跃生成注册表，杜绝已删除消息复活
        sqlx::query("DELETE FROM active_generations WHERE topic_id = ?")
            .bind(topic_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query(
            "UPDATE lifecycle_jobs SET status = 'cancelled', lease_until = NULL, \
             failure_reason = '所属话题已删除', updated_at = ? \
             WHERE topic_id = ? AND status NOT IN ('completed', 'cancelled')",
        )
        .bind(now)
        .bind(topic_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        let owner_id: String = parent_row.get("owner_id");
        let owner_type: String = parent_row.get("owner_type");

        if owner_type == "agent" {
            HashAggregator::bubble_agent_hash(&mut tx, &owner_id).await?;
        } else if owner_type == "group" {
            HashAggregator::bubble_group_hash(&mut tx, &owner_id).await?;
        }
        tx.commit().await.map_err(|e| e.to_string())?;

        Ok(())
    }

    pub async fn soft_delete_avatar<R: Runtime>(
        app: &AppHandle<R>,
        owner_type: &str,
        owner_id: &str,
    ) -> Result<(), String> {
        let db = app.state::<DbState>();
        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query("UPDATE avatars SET deleted_at = ? WHERE owner_type = ? AND owner_id = ?")
            .bind(now)
            .bind(owner_type)
            .bind(owner_id)
            .execute(&db.pool)
            .await
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    pub async fn cleanup_old_deleted_records<R: Runtime>(
        app: &AppHandle<R>,
        days: i64,
    ) -> Result<(), String> {
        let db = app.state::<DbState>();
        let threshold = chrono::Utc::now().timestamp_millis() - days * 24 * 60 * 60 * 1000;

        // 1. 物理强清除已删除超过安全期（30天）的消息的预渲染缓存
        let render_cache =
            sqlx::query("DELETE FROM render_cache WHERE (topic_id, msg_id) IN (SELECT topic_id, msg_id FROM messages WHERE deleted_at IS NOT NULL AND deleted_at < ?)")
                .bind(threshold)
                .execute(&db.pool)
                .await
                .map_err(|e| e.to_string())?;

        // 2. 仅清空已删除超过安全期（30天）的消息的正文内容，保留消息的主键、角色与墓碑时间戳（防止多端同步幽灵复活，并释放大文本空间）
        let cleared_content = ContentCompressor::compress("[已清空]")?;
        let messages =
            sqlx::query("UPDATE messages SET content = ? WHERE deleted_at IS NOT NULL AND deleted_at < ? AND content != ?")
                .bind(&cleared_content)
                .bind(threshold)
                .bind(&cleared_content)
                .execute(&db.pool)
                .await
                .map_err(|e| e.to_string())?;

        log::info!(
            "[DeleteExecutor] Completed safety-period cleanup (older than {} days): cleared_messages_content={}, deleted_render_caches={}",
            days,
            messages.rows_affected(),
            render_cache.rows_affected()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::DeleteExecutor;
    use sqlx::{sqlite::SqlitePoolOptions, Row};

    async fn deletion_test_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");

        for statement in [
            "CREATE TABLE agents (
                agent_id TEXT PRIMARY KEY,
                content_hash TEXT NOT NULL DEFAULT '',
                deleted_at BIGINT
            )",
            "CREATE TABLE topics (
                topic_id TEXT PRIMARY KEY,
                owner_id TEXT NOT NULL,
                owner_type TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                created_at BIGINT NOT NULL DEFAULT 0,
                locked INTEGER NOT NULL DEFAULT 0,
                unread INTEGER NOT NULL DEFAULT 0,
                msg_count INTEGER NOT NULL DEFAULT 0,
                updated_at BIGINT NOT NULL DEFAULT 0,
                config_hash TEXT NOT NULL DEFAULT '',
                content_hash TEXT NOT NULL DEFAULT '',
                deleted_at BIGINT
            )",
            "CREATE TABLE messages (
                msg_id TEXT NOT NULL,
                topic_id TEXT NOT NULL,
                content_hash TEXT NOT NULL DEFAULT '',
                timestamp BIGINT NOT NULL,
                deleted_at BIGINT,
                PRIMARY KEY (topic_id, msg_id)
            )",
            "CREATE TABLE active_generations (
                msg_id TEXT PRIMARY KEY,
                topic_id TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                owner_type TEXT NOT NULL,
                created_at BIGINT NOT NULL
            )",
            "CREATE TABLE message_attachments (
                topic_id TEXT NOT NULL,
                msg_id TEXT NOT NULL,
                hash TEXT NOT NULL,
                deleted_at BIGINT
            )",
            "CREATE TABLE render_cache (
                topic_id TEXT NOT NULL,
                msg_id TEXT NOT NULL
            )",
            "CREATE TABLE lifecycle_jobs (
                job_id TEXT PRIMARY KEY,
                owner_id TEXT NOT NULL,
                owner_type TEXT NOT NULL,
                topic_id TEXT NOT NULL,
                status TEXT NOT NULL,
                lease_until BIGINT,
                source_message_id TEXT,
                response_message_id TEXT,
                failure_reason TEXT,
                updated_at BIGINT NOT NULL
            )",
            "INSERT INTO agents (agent_id, content_hash) VALUES ('agent_alpha', 'old-agent-hash')",
            "INSERT INTO topics
                (topic_id, owner_id, owner_type, msg_count, config_hash, content_hash)
             VALUES ('topic_alpha', 'agent_alpha', 'agent', 1, 'topic-config', 'topic-content')",
            "INSERT INTO messages
                (msg_id, topic_id, content_hash, timestamp)
             VALUES ('message_alpha', 'topic_alpha', 'message-content', 1)",
            "INSERT INTO active_generations
                (msg_id, topic_id, owner_id, owner_type, created_at)
             VALUES ('message_alpha', 'topic_alpha', 'agent_alpha', 'agent', 1)",
            "INSERT INTO message_attachments
                (topic_id, msg_id, hash)
             VALUES ('topic_alpha', 'message_alpha', 'hash-alpha')",
            "INSERT INTO render_cache
                (topic_id, msg_id)
             VALUES ('topic_alpha', 'message_alpha')",
            "INSERT INTO lifecycle_jobs
                (job_id, owner_id, owner_type, topic_id, status, source_message_id, updated_at)
             VALUES ('job-alpha', 'agent_alpha', 'agent', 'topic_alpha', 'scheduled', 'message_alpha', 1)",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }

        pool
    }

    #[tokio::test]
    async fn topic_delete_marks_topic_and_messages_and_clears_active_generation() {
        let pool = deletion_test_pool().await;

        DeleteExecutor::soft_delete_topic_in_pool(&pool, "topic_alpha", 42)
            .await
            .unwrap();

        let topic_deleted_at: Option<i64> =
            sqlx::query_scalar("SELECT deleted_at FROM topics WHERE topic_id = 'topic_alpha'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let message_deleted_at: Option<i64> = sqlx::query_scalar(
            "SELECT deleted_at FROM messages WHERE topic_id = 'topic_alpha' AND msg_id = 'message_alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let active_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM active_generations WHERE topic_id = 'topic_alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let attachment_deleted_at: Option<i64> = sqlx::query_scalar(
            "SELECT deleted_at FROM message_attachments WHERE topic_id = 'topic_alpha' AND msg_id = 'message_alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let render_cache_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM render_cache WHERE topic_id = 'topic_alpha'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let lifecycle_status: String =
            sqlx::query_scalar("SELECT status FROM lifecycle_jobs WHERE job_id = 'job-alpha'")
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(topic_deleted_at, Some(42));
        assert_eq!(message_deleted_at, Some(42));
        assert_eq!(attachment_deleted_at, Some(42));
        assert_eq!(active_count, 0);
        assert_eq!(render_cache_count, 0);
        assert_eq!(lifecycle_status, "cancelled");
        let agent = sqlx::query("SELECT content_hash FROM agents WHERE agent_id = 'agent_alpha'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_ne!(agent.get::<String, _>("content_hash"), "old-agent-hash");
    }

    #[tokio::test]
    async fn topic_delete_rolls_back_when_active_generation_cleanup_fails() {
        let pool = deletion_test_pool().await;
        sqlx::query(
            "CREATE TRIGGER fail_active_generation_delete
             BEFORE DELETE ON active_generations
             BEGIN
                 SELECT RAISE(ABORT, 'forced active generation delete failure');
             END",
        )
        .execute(&pool)
        .await
        .unwrap();

        let result = DeleteExecutor::soft_delete_topic_in_pool(&pool, "topic_alpha", 42).await;
        assert!(result.is_err());

        let topic_deleted_at: Option<i64> =
            sqlx::query_scalar("SELECT deleted_at FROM topics WHERE topic_id = 'topic_alpha'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let message_deleted_at: Option<i64> = sqlx::query_scalar(
            "SELECT deleted_at FROM messages WHERE topic_id = 'topic_alpha' AND msg_id = 'message_alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let active_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM active_generations WHERE topic_id = 'topic_alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let attachment_deleted_at: Option<i64> = sqlx::query_scalar(
            "SELECT deleted_at FROM message_attachments WHERE topic_id = 'topic_alpha' AND msg_id = 'message_alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let render_cache_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM render_cache WHERE topic_id = 'topic_alpha'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let lifecycle_status: String =
            sqlx::query_scalar("SELECT status FROM lifecycle_jobs WHERE job_id = 'job-alpha'")
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(topic_deleted_at, None);
        assert_eq!(message_deleted_at, None);
        assert_eq!(attachment_deleted_at, None);
        assert_eq!(active_count, 1);
        assert_eq!(render_cache_count, 1);
        assert_eq!(lifecycle_status, "scheduled");
    }

    #[tokio::test]
    async fn message_delete_tombstones_message_and_cleans_dependent_state() {
        let pool = deletion_test_pool().await;

        DeleteExecutor::soft_delete_message_in_pool(&pool, "message_alpha")
            .await
            .unwrap();

        let message_deleted_at: Option<i64> = sqlx::query_scalar(
            "SELECT deleted_at FROM messages
             WHERE topic_id = 'topic_alpha' AND msg_id = 'message_alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let topic: (i64, String) = sqlx::query_as(
            "SELECT msg_count, content_hash FROM topics WHERE topic_id = 'topic_alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let active_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM active_generations WHERE msg_id = 'message_alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let attachment_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM message_attachments WHERE msg_id = 'message_alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let render_cache_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM render_cache WHERE msg_id = 'message_alpha'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let lifecycle: (String, Option<i64>) = sqlx::query_as(
            "SELECT status, lease_until FROM lifecycle_jobs WHERE job_id = 'job-alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let agent_hash: String =
            sqlx::query_scalar("SELECT content_hash FROM agents WHERE agent_id = 'agent_alpha'")
                .fetch_one(&pool)
                .await
                .unwrap();

        assert!(message_deleted_at.is_some());
        assert_eq!(topic.0, 0);
        assert_ne!(topic.1, "topic-content");
        assert_eq!(active_count, 0);
        assert_eq!(attachment_count, 0);
        assert_eq!(render_cache_count, 0);
        assert_eq!(lifecycle, ("cancelled".to_string(), None));
        assert_ne!(agent_hash, "old-agent-hash");
    }
}

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use tauri::State;
use uuid::Uuid;

use crate::vcp_modules::db_manager::DbState;

const DIRECTIVE_START: &str = "<<<[VCP_LIFECYCLE]>>>";
const DIRECTIVE_END: &str = "<<<[END_VCP_LIFECYCLE]>>>";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleJob {
    pub job_id: String,
    pub owner_id: String,
    pub owner_type: String,
    pub topic_id: String,
    pub responder_agent_id: Option<String>,
    pub action: String,
    pub intent: String,
    pub condition: Option<String>,
    pub status: String,
    pub scheduled_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub attempt_count: i64,
    pub max_attempts: i64,
    pub source_message_id: Option<String>,
    pub response_message_id: Option<String>,
    pub failure_reason: Option<String>,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateLifecycleJobInput {
    pub owner_id: String,
    pub owner_type: String,
    pub topic_id: String,
    pub responder_agent_id: Option<String>,
    pub action: String,
    pub intent: String,
    pub condition: Option<String>,
    pub scheduled_at: i64,
    pub source_message_id: Option<String>,
    pub max_attempts: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifecycleDirective {
    action: String,
    intent: String,
    delay_seconds: Option<i64>,
    scheduled_at: Option<String>,
    condition: Option<String>,
}

pub async fn setup_lifecycle_tables(pool: &Pool<Sqlite>) -> Result<(), String> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS lifecycle_jobs (
            job_id TEXT PRIMARY KEY,
            owner_id TEXT NOT NULL,
            owner_type TEXT NOT NULL,
            topic_id TEXT NOT NULL,
            responder_agent_id TEXT,
            action TEXT NOT NULL,
            intent TEXT NOT NULL,
            condition_json TEXT,
            status TEXT NOT NULL DEFAULT 'scheduled',
            scheduled_at BIGINT NOT NULL,
            lease_until BIGINT,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            max_attempts INTEGER NOT NULL DEFAULT 3,
            source_message_id TEXT,
            response_message_id TEXT,
            idempotency_key TEXT NOT NULL UNIQUE,
            failure_reason TEXT,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL,
            completed_at BIGINT
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let columns = sqlx::query("PRAGMA table_info(lifecycle_jobs)")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let has_response_message_id = columns
        .iter()
        .any(|row| row.get::<String, _>("name") == "response_message_id");
    if !has_response_message_id {
        sqlx::query("ALTER TABLE lifecycle_jobs ADD COLUMN response_message_id TEXT")
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_lifecycle_jobs_due
         ON lifecycle_jobs(status, scheduled_at)",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn validate_input(input: &CreateLifecycleJobInput) -> Result<(), String> {
    if input.owner_id.trim().is_empty() || input.topic_id.trim().is_empty() {
        return Err("生命周期任务缺少目标会话".to_string());
    }
    if input.owner_type != "agent" && input.owner_type != "group" {
        return Err("生命周期任务 ownerType 无效".to_string());
    }
    if input.action != "schedule_message" && input.action != "continue_message" {
        return Err("不支持的生命周期动作".to_string());
    }
    if input.intent.trim().is_empty() {
        return Err("生命周期任务 intent 不能为空".to_string());
    }
    Ok(())
}

fn row_to_job(row: &sqlx::sqlite::SqliteRow) -> Result<LifecycleJob, String> {
    Ok(LifecycleJob {
        job_id: row.try_get("job_id").map_err(|e| e.to_string())?,
        owner_id: row.try_get("owner_id").map_err(|e| e.to_string())?,
        owner_type: row.try_get("owner_type").map_err(|e| e.to_string())?,
        topic_id: row.try_get("topic_id").map_err(|e| e.to_string())?,
        responder_agent_id: row.try_get("responder_agent_id").ok(),
        action: row.try_get("action").map_err(|e| e.to_string())?,
        intent: row.try_get("intent").map_err(|e| e.to_string())?,
        condition: row.try_get("condition_json").ok(),
        status: row.try_get("status").map_err(|e| e.to_string())?,
        scheduled_at: row.try_get("scheduled_at").map_err(|e| e.to_string())?,
        created_at: row.try_get("created_at").map_err(|e| e.to_string())?,
        updated_at: row.try_get("updated_at").map_err(|e| e.to_string())?,
        attempt_count: row.try_get("attempt_count").map_err(|e| e.to_string())?,
        max_attempts: row.try_get("max_attempts").map_err(|e| e.to_string())?,
        source_message_id: row.try_get("source_message_id").ok(),
        response_message_id: row.try_get("response_message_id").ok(),
        failure_reason: row.try_get("failure_reason").ok(),
        completed_at: row.try_get("completed_at").ok(),
    })
}

pub async fn insert_job(
    pool: &Pool<Sqlite>,
    input: CreateLifecycleJobInput,
) -> Result<LifecycleJob, String> {
    validate_input(&input)?;
    let now = Utc::now().timestamp_millis();
    let scheduled_at = input.scheduled_at.max(now + 1_000);
    let job_id = format!("life_job_{}", Uuid::new_v4());
    let idempotency_key = format!(
        "{}:{}:{}:{}:{}",
        input.source_message_id.as_deref().unwrap_or("manual"),
        input.action,
        input.owner_id,
        input.topic_id,
        scheduled_at
    );
    let max_attempts = input.max_attempts.unwrap_or(3).clamp(1, 8);
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let active_topic: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM topics \
         WHERE topic_id = ? AND owner_id = ? AND owner_type = ? AND deleted_at IS NULL)",
    )
    .bind(&input.topic_id)
    .bind(&input.owner_id)
    .bind(&input.owner_type)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    if !active_topic {
        return Err(format!(
            "生命周期任务目标话题 {} 不存在、已删除或归属不匹配",
            input.topic_id
        ));
    }

    sqlx::query(
        "INSERT OR IGNORE INTO lifecycle_jobs (
            job_id, owner_id, owner_type, topic_id, responder_agent_id, action, intent,
            condition_json, status, scheduled_at, attempt_count, max_attempts,
            source_message_id, idempotency_key, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'scheduled', ?, 0, ?, ?, ?, ?, ?)",
    )
    .bind(&job_id)
    .bind(&input.owner_id)
    .bind(&input.owner_type)
    .bind(&input.topic_id)
    .bind(&input.responder_agent_id)
    .bind(&input.action)
    .bind(input.intent.trim())
    .bind(&input.condition)
    .bind(scheduled_at)
    .bind(max_attempts)
    .bind(&input.source_message_id)
    .bind(&idempotency_key)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let row = sqlx::query("SELECT * FROM lifecycle_jobs WHERE idempotency_key = ? LIMIT 1")
        .bind(idempotency_key)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let job = row_to_job(&row)?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(job)
}

#[tauri::command]
pub async fn create_lifecycle_job(
    state: State<'_, DbState>,
    input: CreateLifecycleJobInput,
) -> Result<LifecycleJob, String> {
    insert_job(&state.pool, input).await
}

#[tauri::command]
pub async fn list_lifecycle_jobs(
    state: State<'_, DbState>,
    include_finished: Option<bool>,
) -> Result<Vec<LifecycleJob>, String> {
    let rows = if include_finished.unwrap_or(false) {
        sqlx::query("SELECT * FROM lifecycle_jobs ORDER BY updated_at DESC LIMIT 200")
            .fetch_all(&state.pool)
            .await
    } else {
        sqlx::query(
            "SELECT * FROM lifecycle_jobs WHERE status IN ('scheduled', 'running', 'failed')
             ORDER BY scheduled_at ASC LIMIT 200",
        )
        .fetch_all(&state.pool)
        .await
    }
    .map_err(|e| e.to_string())?;
    rows.iter().map(row_to_job).collect()
}

#[tauri::command]
pub async fn claim_due_lifecycle_jobs(
    state: State<'_, DbState>,
    limit: Option<i64>,
) -> Result<Vec<LifecycleJob>, String> {
    claim_due_jobs(&state.pool, limit).await
}

async fn claim_due_jobs(
    pool: &Pool<Sqlite>,
    limit: Option<i64>,
) -> Result<Vec<LifecycleJob>, String> {
    let now = Utc::now().timestamp_millis();
    let lease_until = now + 10 * 60_000;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(
        "UPDATE lifecycle_jobs SET status = 'cancelled', lease_until = NULL,
         failure_reason = '所属话题不存在或已删除', updated_at = ?
         WHERE status IN ('scheduled', 'running', 'failed')
           AND NOT EXISTS (
             SELECT 1 FROM topics t
             WHERE t.topic_id = lifecycle_jobs.topic_id
               AND t.owner_id = lifecycle_jobs.owner_id
               AND t.owner_type = lifecycle_jobs.owner_type
               AND t.deleted_at IS NULL
           )",
    )
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query(
        "UPDATE lifecycle_jobs SET status = 'scheduled', lease_until = NULL, updated_at = ?
         WHERE status = 'running' AND lease_until IS NOT NULL AND lease_until < ?",
    )
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let rows = sqlx::query(
        "SELECT * FROM lifecycle_jobs WHERE status = 'scheduled' AND scheduled_at <= ?
         ORDER BY scheduled_at ASC LIMIT ?",
    )
    .bind(now)
    .bind(limit.unwrap_or(4).clamp(1, 20))
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let mut jobs = Vec::new();
    for row in rows {
        let job = row_to_job(&row)?;
        if job.condition.as_deref() == Some("user_has_not_replied") {
            let latest_user_timestamp: Option<i64> = sqlx::query_scalar(
                "SELECT MAX(timestamp) FROM messages
                 WHERE topic_id = ? AND role = 'user' AND deleted_at IS NULL",
            )
            .bind(&job.topic_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            if latest_user_timestamp.is_some_and(|timestamp| timestamp > job.created_at) {
                sqlx::query(
                    "UPDATE lifecycle_jobs SET status = 'cancelled', failure_reason = ?, updated_at = ?
                     WHERE job_id = ?",
                )
                .bind("用户已回复，条件式跟进自动取消")
                .bind(now)
                .bind(&job.job_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
                continue;
            }
        }
        let result = sqlx::query(
            "UPDATE lifecycle_jobs SET status = 'running', lease_until = ?,
             attempt_count = attempt_count + 1, updated_at = ?
             WHERE job_id = ? AND status = 'scheduled'",
        )
        .bind(lease_until)
        .bind(now)
        .bind(&job.job_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        if result.rows_affected() == 1 {
            jobs.push(LifecycleJob {
                status: "running".to_string(),
                attempt_count: job.attempt_count + 1,
                updated_at: now,
                ..job
            });
        }
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(jobs)
}

#[tauri::command]
pub async fn complete_lifecycle_job(
    state: State<'_, DbState>,
    job_id: String,
    response_message_id: String,
) -> Result<(), String> {
    complete_job(&state.pool, &job_id, &response_message_id).await
}

async fn complete_job(
    pool: &Pool<Sqlite>,
    job_id: &str,
    response_message_id: &str,
) -> Result<(), String> {
    let response_message_id = response_message_id.trim();
    if response_message_id.is_empty() {
        return Err("生命周期任务缺少回复消息 ID".to_string());
    }

    let now = Utc::now().timestamp_millis();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let topic_id: Option<String> = sqlx::query_scalar(
        "SELECT topic_id FROM lifecycle_jobs WHERE job_id = ? AND status = 'running' LIMIT 1",
    )
    .bind(job_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let topic_id = topic_id.ok_or_else(|| "生命周期任务不存在或不在执行中".to_string())?;

    let response_exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM messages
         WHERE msg_id = ? AND topic_id = ? AND role = 'assistant' AND deleted_at IS NULL
         LIMIT 1",
    )
    .bind(response_message_id)
    .bind(&topic_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    if response_exists.is_none() {
        return Err(format!(
            "生命周期回复尚未持久化: topic_id={}, message_id={}",
            topic_id, response_message_id
        ));
    }

    let result = sqlx::query(
        "UPDATE lifecycle_jobs SET status = 'completed', completed_at = ?, lease_until = NULL,
         response_message_id = ?, failure_reason = NULL, updated_at = ?
         WHERE job_id = ? AND status = 'running'",
    )
    .bind(now)
    .bind(response_message_id)
    .bind(now)
    .bind(job_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    if result.rows_affected() != 1 {
        return Err("生命周期任务完成状态更新失败".to_string());
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn fail_lifecycle_job(
    state: State<'_, DbState>,
    job_id: String,
    reason: String,
    retry_delay_seconds: Option<i64>,
) -> Result<(), String> {
    let now = Utc::now().timestamp_millis();
    let retry_at = now + retry_delay_seconds.unwrap_or(300).clamp(10, 86_400) * 1_000;
    sqlx::query(
        "UPDATE lifecycle_jobs SET
           status = CASE WHEN attempt_count >= max_attempts THEN 'failed' ELSE 'scheduled' END,
           scheduled_at = CASE WHEN attempt_count >= max_attempts THEN scheduled_at ELSE ? END,
           lease_until = NULL, failure_reason = ?, updated_at = ?
         WHERE job_id = ? AND status = 'running'",
    )
    .bind(retry_at)
    .bind(reason)
    .bind(now)
    .bind(job_id)
    .execute(&state.pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn cancel_lifecycle_job(state: State<'_, DbState>, job_id: String) -> Result<(), String> {
    let now = Utc::now().timestamp_millis();
    sqlx::query(
        "UPDATE lifecycle_jobs SET status = 'cancelled', lease_until = NULL, updated_at = ?
         WHERE job_id = ? AND status NOT IN ('completed', 'cancelled')",
    )
    .bind(now)
    .bind(job_id)
    .execute(&state.pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn extract_and_schedule_directives(
    pool: &Pool<Sqlite>,
    content: &str,
    owner_id: &str,
    owner_type: &str,
    topic_id: &str,
    responder_agent_id: Option<&str>,
    source_message_id: &str,
) -> Result<(String, Vec<LifecycleJob>), String> {
    if !content.contains(DIRECTIVE_START) {
        return Ok((content.to_string(), Vec::new()));
    }
    let pattern = format!(
        r"(?s){}\s*(.*?)\s*{}",
        regex::escape(DIRECTIVE_START),
        regex::escape(DIRECTIVE_END)
    );
    let re = Regex::new(&pattern).map_err(|e| e.to_string())?;
    let now = Utc::now().timestamp_millis();
    let mut jobs = Vec::new();
    for captures in re.captures_iter(content) {
        let Some(raw) = captures.get(1).map(|value| value.as_str().trim()) else {
            continue;
        };
        let directive: LifecycleDirective = match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("[LifecycleScheduler] Ignoring invalid directive: {}", error);
                continue;
            }
        };
        let scheduled_at = if let Some(raw_time) = directive.scheduled_at.as_deref() {
            DateTime::parse_from_rfc3339(raw_time)
                .map(|value| value.timestamp_millis())
                .unwrap_or(now + 60_000)
        } else {
            let default_delay = if directive.action == "continue_message" {
                3
            } else {
                60
            };
            now + directive
                .delay_seconds
                .unwrap_or(default_delay)
                .clamp(1, 31_536_000)
                * 1_000
        };
        jobs.push(
            insert_job(
                pool,
                CreateLifecycleJobInput {
                    owner_id: owner_id.to_string(),
                    owner_type: owner_type.to_string(),
                    topic_id: topic_id.to_string(),
                    responder_agent_id: responder_agent_id.map(ToString::to_string),
                    action: directive.action,
                    intent: directive.intent,
                    condition: directive.condition,
                    scheduled_at,
                    source_message_id: Some(source_message_id.to_string()),
                    max_attempts: Some(3),
                },
            )
            .await?,
        );
    }
    Ok((re.replace_all(content, "").trim().to_string(), jobs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn setup_active_topic(pool: &Pool<Sqlite>) {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS topics (
                topic_id TEXT PRIMARY KEY,
                owner_id TEXT NOT NULL,
                owner_type TEXT NOT NULL,
                deleted_at BIGINT
            )",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT OR REPLACE INTO topics (topic_id, owner_id, owner_type, deleted_at)
             VALUES ('topic-1', 'agent-1', 'agent', NULL)",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn lifecycle_directive_is_hidden_and_persisted() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        setup_lifecycle_tables(&pool).await.unwrap();
        setup_active_topic(&pool).await;
        let before = Utc::now().timestamp_millis();
        let content = "我稍后再来看看。\n<<<[VCP_LIFECYCLE]>>>\n{\"action\":\"schedule_message\",\"delaySeconds\":60,\"intent\":\"询问用户是否完成\",\"condition\":\"user_has_not_replied\"}\n<<<[END_VCP_LIFECYCLE]>>>";
        let (visible, jobs) = extract_and_schedule_directives(
            &pool,
            content,
            "agent-1",
            "agent",
            "topic-1",
            Some("agent-1"),
            "message-1",
        )
        .await
        .unwrap();
        assert_eq!(visible, "我稍后再来看看。");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].condition.as_deref(), Some("user_has_not_replied"));
        assert!(jobs[0].scheduled_at >= before + 59_000);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM lifecycle_jobs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn setup_migrates_existing_lifecycle_table() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE lifecycle_jobs (
                job_id TEXT PRIMARY KEY,
                owner_id TEXT NOT NULL,
                owner_type TEXT NOT NULL,
                topic_id TEXT NOT NULL,
                responder_agent_id TEXT,
                action TEXT NOT NULL,
                intent TEXT NOT NULL,
                condition_json TEXT,
                status TEXT NOT NULL DEFAULT 'scheduled',
                scheduled_at BIGINT NOT NULL,
                lease_until BIGINT,
                attempt_count INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 3,
                source_message_id TEXT,
                idempotency_key TEXT NOT NULL UNIQUE,
                failure_reason TEXT,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL,
                completed_at BIGINT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        setup_lifecycle_tables(&pool).await.unwrap();

        let columns = sqlx::query("PRAGMA table_info(lifecycle_jobs)")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(columns
            .iter()
            .any(|row| row.get::<String, _>("name") == "response_message_id"));
        setup_lifecycle_tables(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn completed_job_records_persisted_assistant_message() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        setup_lifecycle_tables(&pool).await.unwrap();
        setup_active_topic(&pool).await;
        sqlx::query(
            "CREATE TABLE messages (
                msg_id TEXT PRIMARY KEY,
                topic_id TEXT NOT NULL,
                role TEXT NOT NULL,
                deleted_at BIGINT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        let job = insert_job(
            &pool,
            CreateLifecycleJobInput {
                owner_id: "agent-1".to_string(),
                owner_type: "agent".to_string(),
                topic_id: "topic-1".to_string(),
                responder_agent_id: Some("agent-1".to_string()),
                action: "continue_message".to_string(),
                intent: "继续对话".to_string(),
                condition: None,
                scheduled_at: Utc::now().timestamp_millis() + 1_000,
                source_message_id: Some("source-1".to_string()),
                max_attempts: Some(3),
            },
        )
        .await
        .unwrap();
        sqlx::query("UPDATE lifecycle_jobs SET status = 'running' WHERE job_id = ?")
            .bind(&job.job_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO messages (msg_id, topic_id, role) VALUES ('reply-1', 'topic-1', 'assistant')",
        )
        .execute(&pool)
        .await
        .unwrap();

        complete_job(&pool, &job.job_id, "reply-1").await.unwrap();

        let row = sqlx::query(
            "SELECT status, response_message_id, completed_at FROM lifecycle_jobs WHERE job_id = ?",
        )
        .bind(&job.job_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("status"), "completed");
        assert_eq!(row.get::<String, _>("response_message_id"), "reply-1");
        assert!(row.get::<Option<i64>, _>("completed_at").is_some());
    }

    #[tokio::test]
    async fn job_cannot_complete_before_assistant_message_is_persisted() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        setup_lifecycle_tables(&pool).await.unwrap();
        setup_active_topic(&pool).await;
        sqlx::query(
            "CREATE TABLE messages (
                msg_id TEXT PRIMARY KEY,
                topic_id TEXT NOT NULL,
                role TEXT NOT NULL,
                deleted_at BIGINT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        let job = insert_job(
            &pool,
            CreateLifecycleJobInput {
                owner_id: "agent-1".to_string(),
                owner_type: "agent".to_string(),
                topic_id: "topic-1".to_string(),
                responder_agent_id: Some("agent-1".to_string()),
                action: "continue_message".to_string(),
                intent: "继续对话".to_string(),
                condition: None,
                scheduled_at: Utc::now().timestamp_millis() + 1_000,
                source_message_id: Some("source-1".to_string()),
                max_attempts: Some(3),
            },
        )
        .await
        .unwrap();
        sqlx::query("UPDATE lifecycle_jobs SET status = 'running' WHERE job_id = ?")
            .bind(&job.job_id)
            .execute(&pool)
            .await
            .unwrap();

        let error = complete_job(&pool, &job.job_id, "missing-reply")
            .await
            .unwrap_err();
        assert!(error.contains("尚未持久化"));
        let status: String =
            sqlx::query_scalar("SELECT status FROM lifecycle_jobs WHERE job_id = ?")
                .bind(&job.job_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "running");
    }

    #[tokio::test]
    async fn lifecycle_job_cannot_target_a_deleted_topic() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        setup_lifecycle_tables(&pool).await.unwrap();
        setup_active_topic(&pool).await;
        sqlx::query("UPDATE topics SET deleted_at = 42 WHERE topic_id = 'topic-1'")
            .execute(&pool)
            .await
            .unwrap();

        let error = insert_job(
            &pool,
            CreateLifecycleJobInput {
                owner_id: "agent-1".to_string(),
                owner_type: "agent".to_string(),
                topic_id: "topic-1".to_string(),
                responder_agent_id: Some("agent-1".to_string()),
                action: "continue_message".to_string(),
                intent: "不应创建".to_string(),
                condition: None,
                scheduled_at: Utc::now().timestamp_millis() + 1_000,
                source_message_id: Some("source-deleted".to_string()),
                max_attempts: Some(3),
            },
        )
        .await
        .unwrap_err();

        assert!(error.contains("已删除"));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM lifecycle_jobs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn due_job_is_cancelled_when_its_topic_was_deleted() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        setup_lifecycle_tables(&pool).await.unwrap();
        setup_active_topic(&pool).await;
        let job = insert_job(
            &pool,
            CreateLifecycleJobInput {
                owner_id: "agent-1".to_string(),
                owner_type: "agent".to_string(),
                topic_id: "topic-1".to_string(),
                responder_agent_id: Some("agent-1".to_string()),
                action: "continue_message".to_string(),
                intent: "稍后继续".to_string(),
                condition: None,
                scheduled_at: Utc::now().timestamp_millis() + 1_000,
                source_message_id: Some("source-due".to_string()),
                max_attempts: Some(3),
            },
        )
        .await
        .unwrap();
        sqlx::query("UPDATE lifecycle_jobs SET scheduled_at = 0 WHERE job_id = ?")
            .bind(&job.job_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE topics SET deleted_at = 42 WHERE topic_id = 'topic-1'")
            .execute(&pool)
            .await
            .unwrap();

        let claimed = claim_due_jobs(&pool, Some(4)).await.unwrap();

        assert!(claimed.is_empty());
        let status: String =
            sqlx::query_scalar("SELECT status FROM lifecycle_jobs WHERE job_id = ?")
                .bind(&job.job_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "cancelled");
    }
}

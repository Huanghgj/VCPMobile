use super::message_repository::ContentCompressor;
use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};
use std::collections::{HashMap, HashSet};
use std::fs;
use tauri::AppHandle;
use tauri::Manager;

pub struct DbState {
    pub pool: Pool<Sqlite>,
    pub path: std::path::PathBuf,
}

impl DbState {
    /// 执行 SQLite 物理页面碎片分批回收与查询规划器索引优化
    pub async fn run_incremental_vacuum_optimize(
        &self,
        pages_to_vacuum: i32,
    ) -> Result<(), sqlx::Error> {
        // 1. 分批页整理碎片，防堵大面积 I/O 阻塞
        sqlx::query(&format!("PRAGMA incremental_vacuum({})", pages_to_vacuum))
            .execute(&self.pool)
            .await?;
        // 2. 重构索引规划器
        sqlx::query("PRAGMA optimize").execute(&self.pool).await?;
        Ok(())
    }
}

pub async fn init_db(app_handle: &AppHandle) -> Result<(Pool<Sqlite>, std::path::PathBuf), String> {
    // 获取应用配置目录 (Android 下通常为 /data/user/0/com.vcp.avatar/files)
    let config_dir = app_handle
        .path()
        .app_config_dir()
        .map_err(|e| format!("Config dir failed: {}", e))?;

    // 确保父目录存在
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir).map_err(|e| format!("Create dir failed: {}", e))?;
    }

    let mut db_path = config_dir.clone();
    db_path.push("vcp_avatar.db");

    log::info!("[DBManager] Initializing SQLite at: {:?}", db_path);

    // 配置连接选项
    let mut connect_options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true);

    // 深度性能优化：
    // 1. WAL 模式：允许读写并发，极大提升 UI 相应速度
    // 2. Normal 同步：在 WAL 模式下兼顾安全性与速度
    // 3. mmap_size: 开启内存映射 I/O (256MB)，将磁盘读取变为内存访问
    // 4. temp_store: 将临时表、排序操作强制放在内存中
    // 5. page_size: 提升至 16KB，优化现代闪存 I/O 效率
    // 6. auto_vacuum: 开启增量清理逻辑，配合维护任务物理回收空间
    // 7. foreign_keys: 开启外键约束，以支持级联删除
    connect_options = connect_options
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(30))
        .pragma("mmap_size", "268435456")
        .pragma("temp_store", "2")
        .pragma("page_size", "16384")
        .pragma("cache_size", "-8000")
        .pragma("auto_vacuum", "2")
        .pragma("foreign_keys", "1");

    let mut retry_count = 0;
    let pool = loop {
        match SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(connect_options.clone())
            .await
        {
            Ok(p) => break p,
            Err(e) => {
                retry_count += 1;
                if retry_count >= 3 {
                    return Err(format!(
                        "数据库连接重试失败 (已重试 {} 次): {}",
                        retry_count, e
                    ));
                }
                log::warn!(
                    "[DBManager] Connection failed: {}. Retrying in {}ms... (Attempt {})",
                    e,
                    retry_count * 50,
                    retry_count
                );
                tokio::time::sleep(std::time::Duration::from_millis(retry_count * 50)).await;
            }
        }
    };

    // 运行初始化建表
    setup_tables(&pool).await?;

    // 挂载到 App State (注意：由于 init_db 返回 pool，我们需要在外部构建 DbState)
    Ok((pool, db_path))
}

fn quote_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn legacy_backup_name(table: &str) -> String {
    format!(
        "{}_legacy_backup_{}_{}",
        table,
        chrono::Utc::now().timestamp_millis(),
        std::process::id()
    )
}

async fn table_exists(pool: &Pool<Sqlite>, table: &str) -> Result<bool, String> {
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ? LIMIT 1")
            .bind(table)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
    Ok(exists.is_some())
}

async fn table_columns(pool: &Pool<Sqlite>, table: &str) -> Result<HashSet<String>, String> {
    let query = format!("PRAGMA table_info({})", quote_ident(table));
    let rows = sqlx::query(&query)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect())
}

#[derive(Default)]
struct LegacyMessageMigration {
    message_attachments_backup: Option<String>,
}

async fn backup_table(
    pool: &Pool<Sqlite>,
    table: &str,
    reason: &str,
) -> Result<Option<String>, String> {
    if !table_exists(pool, table).await? {
        return Ok(None);
    }

    let backup = legacy_backup_name(table);
    log::warn!(
        "[DBManager] Legacy table {} requires rebuild ({}); renaming to {}",
        table,
        reason,
        backup
    );
    let sql = format!(
        "ALTER TABLE {} RENAME TO {}",
        quote_ident(table),
        quote_ident(&backup)
    );
    sqlx::query(&sql)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(backup))
}

async fn backup_table_if_lacks_column(
    pool: &Pool<Sqlite>,
    table: &str,
    column: &str,
) -> Result<Option<String>, String> {
    if !table_exists(pool, table).await? {
        return Ok(None);
    }

    let columns = table_columns(pool, table).await?;
    if columns.contains(column) {
        return Ok(None);
    }

    backup_table(pool, table, &format!("missing column {column}")).await
}

async fn add_column_if_missing(
    pool: &Pool<Sqlite>,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let columns = table_columns(pool, table).await?;
    if columns.contains(column) {
        return Ok(());
    }

    log::warn!(
        "[DBManager] Adding missing column {}.{} ({})",
        table,
        column,
        definition
    );
    let sql = format!(
        "ALTER TABLE {} ADD COLUMN {} {}",
        quote_ident(table),
        quote_ident(column),
        definition
    );
    sqlx::query(&sql)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn get_optional_string(row: &sqlx::sqlite::SqliteRow, column: &str) -> Option<String> {
    row.try_get::<String, _>(column)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn get_optional_i64(row: &sqlx::sqlite::SqliteRow, column: &str) -> Option<i64> {
    if let Ok(value) = row.try_get::<Option<i64>, _>(column) {
        return value;
    }
    row.try_get::<i64, _>(column).ok()
}

fn get_legacy_content(row: &sqlx::sqlite::SqliteRow) -> Result<Vec<u8>, String> {
    if let Ok(text) = row.try_get::<String, _>("content") {
        return ContentCompressor::compress(&text);
    }

    if let Ok(bytes) = row.try_get::<Vec<u8>, _>("content") {
        if ContentCompressor::decompress(&bytes).is_ok() {
            return Ok(bytes);
        }
        if let Ok(text) = String::from_utf8(bytes) {
            return ContentCompressor::compress(&text);
        }
    }

    ContentCompressor::compress("")
}

fn stable_hex(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn legacy_topic_for_message(
    agent_id: &Option<String>,
    group_id: &Option<String>,
    is_group_message: bool,
) -> (String, String, String) {
    let owner_type = if is_group_message || group_id.is_some() {
        "group"
    } else {
        "agent"
    };
    let owner_id = if owner_type == "group" {
        group_id
            .clone()
            .unwrap_or_else(|| "legacy_group".to_string())
    } else {
        agent_id
            .clone()
            .unwrap_or_else(|| "legacy_agent".to_string())
    };
    let topic_id = format!("legacy_migrated_{}_{}", owner_type, stable_hex(&owner_id));
    (topic_id, owner_type.to_string(), owner_id)
}

async fn create_messages_table(pool: &Pool<Sqlite>) -> Result<(), String> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS messages (
            msg_id TEXT NOT NULL,
            topic_id TEXT NOT NULL,
            role TEXT NOT NULL,
            name TEXT,
            agent_id TEXT,
            content TEXT NOT NULL,
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
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn messages_table_needs_rebuild(pool: &Pool<Sqlite>) -> Result<bool, String> {
    if !table_exists(pool, "messages").await? {
        return Ok(false);
    }

    let rows = sqlx::query("PRAGMA table_info(messages)")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let columns: HashSet<String> = rows
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    if !columns.contains("topic_id") {
        return Ok(true);
    }

    let mut pk_columns = rows
        .iter()
        .filter_map(|row| {
            let pk: i64 = row.get("pk");
            if pk > 0 {
                Some((pk, row.get::<String, _>("name")))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    pk_columns.sort_by_key(|(pk, _)| *pk);
    let pk_names = pk_columns
        .into_iter()
        .map(|(_, name)| name)
        .collect::<Vec<_>>();
    Ok(pk_names != ["topic_id".to_string(), "msg_id".to_string()])
}

async fn legacy_backup_tables(pool: &Pool<Sqlite>, table: &str) -> Result<Vec<String>, String> {
    let pattern = format!("{}_legacy_backup_%", table);
    let rows = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE ? ORDER BY name ASC",
    )
    .bind(pattern)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn setup_tables_migrates_legacy_messages_without_topic_id() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");

        sqlx::query(
            "CREATE TABLE messages (
                msg_id TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                name TEXT,
                agent_id TEXT,
                content TEXT NOT NULL,
                timestamp BIGINT NOT NULL,
                is_group_message INTEGER NOT NULL DEFAULT 0,
                group_id TEXT,
                finish_reason TEXT,
                content_hash TEXT NOT NULL DEFAULT '',
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL,
                deleted_at BIGINT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO messages (
                msg_id, role, name, agent_id, content, timestamp, created_at, updated_at
            ) VALUES ('legacy_1', 'assistant', 'AI', 'agent_alpha', 'hello legacy', 10, 10, 10)",
        )
        .execute(&pool)
        .await
        .unwrap();

        setup_tables(&pool).await.unwrap();

        let has_topic_id = table_columns(&pool, "messages")
            .await
            .unwrap()
            .contains("topic_id");
        assert!(has_topic_id);

        let row = sqlx::query(
            "SELECT m.topic_id, m.content, t.owner_type, t.owner_id, t.msg_count
             FROM messages m
             INNER JOIN topics t ON t.topic_id = m.topic_id
             WHERE m.msg_id = 'legacy_1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let topic_id: String = row.get("topic_id");
        let owner_type: String = row.get("owner_type");
        let owner_id: String = row.get("owner_id");
        let msg_count: i64 = row.get("msg_count");
        let content_bytes: Vec<u8> = row.get("content");

        assert!(topic_id.starts_with("legacy_migrated_agent_"));
        assert_eq!(owner_type, "agent");
        assert_eq!(owner_id, "agent_alpha");
        assert_eq!(msg_count, 1);
        assert_eq!(
            ContentCompressor::decompress(&content_bytes).unwrap(),
            "hello legacy"
        );
    }

    #[tokio::test]
    async fn setup_tables_rebuilds_messages_with_legacy_primary_key() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");

        sqlx::query(
            "CREATE TABLE messages (
                msg_id TEXT PRIMARY KEY,
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
                deleted_at BIGINT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE message_attachments (
                msg_id TEXT NOT NULL,
                hash TEXT NOT NULL,
                attachment_order INTEGER NOT NULL,
                display_name TEXT NOT NULL,
                created_at BIGINT NOT NULL,
                PRIMARY KEY (msg_id, attachment_order)
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO messages (
                msg_id, topic_id, role, name, agent_id, content, timestamp, created_at, updated_at
            ) VALUES (?, 'topic_existing', 'assistant', 'AI', 'agent_alpha', ?, 10, 10, 10)",
        )
        .bind("legacy_1")
        .bind(ContentCompressor::compress("hello existing topic").unwrap())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO message_attachments (
                msg_id, hash, attachment_order, display_name, created_at
            ) VALUES ('legacy_1', 'hash_1', 0, 'a.png', 10)",
        )
        .execute(&pool)
        .await
        .unwrap();

        setup_tables(&pool).await.unwrap();

        let pk_rows = sqlx::query("PRAGMA table_info(messages)")
            .fetch_all(&pool)
            .await
            .unwrap();
        let mut pk_columns = pk_rows
            .iter()
            .filter_map(|row| {
                let pk: i64 = row.get("pk");
                if pk > 0 {
                    Some((pk, row.get::<String, _>("name")))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        pk_columns.sort_by_key(|(pk, _)| *pk);
        assert_eq!(
            pk_columns
                .into_iter()
                .map(|(_, name)| name)
                .collect::<Vec<_>>(),
            vec!["topic_id".to_string(), "msg_id".to_string()]
        );

        let content_bytes: Vec<u8> =
            sqlx::query_scalar("SELECT content FROM messages WHERE topic_id = 'topic_existing' AND msg_id = 'legacy_1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            ContentCompressor::decompress(&content_bytes).unwrap(),
            "hello existing topic"
        );

        let attachment_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM message_attachments WHERE topic_id = 'topic_existing' AND msg_id = 'legacy_1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(attachment_count, 1);
    }

    #[tokio::test]
    async fn setup_tables_backfills_legacy_agents_columns() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");

        sqlx::query(
            "CREATE TABLE agents (
                agent_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                system_prompt TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL,
                updated_at BIGINT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO agents (
                agent_id, name, system_prompt, model, updated_at
            ) VALUES ('agent_alpha', 'Alpha', 'desktop prompt', 'gpt-test', 10)",
        )
        .execute(&pool)
        .await
        .unwrap();

        setup_tables(&pool).await.unwrap();

        let columns = table_columns(&pool, "agents").await.unwrap();
        for column in [
            "mobile_system_prompt",
            "temperature",
            "context_token_limit",
            "max_output_tokens",
            "stream_output",
            "use_temperature",
            "config_hash",
            "content_hash",
            "deleted_at",
        ] {
            assert!(
                columns.contains(column),
                "expected legacy agents table to gain column {column}"
            );
        }

        let row = sqlx::query(
            "SELECT name, system_prompt, mobile_system_prompt, temperature, context_token_limit,
                    max_output_tokens, stream_output, use_temperature, config_hash, content_hash,
                    deleted_at
             FROM agents WHERE agent_id = 'agent_alpha'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("name"), "Alpha");
        assert_eq!(row.get::<String, _>("system_prompt"), "desktop prompt");
        assert_eq!(row.get::<String, _>("mobile_system_prompt"), "");
        assert_eq!(row.get::<f64, _>("temperature"), 1.0);
        assert_eq!(row.get::<i64, _>("context_token_limit"), 0);
        assert_eq!(row.get::<i64, _>("max_output_tokens"), 0);
        assert_eq!(row.get::<i64, _>("stream_output"), 1);
        assert_eq!(row.get::<i64, _>("use_temperature"), 0);
        assert_eq!(row.get::<String, _>("config_hash"), "");
        assert_eq!(row.get::<String, _>("content_hash"), "");
        assert_eq!(row.try_get::<Option<i64>, _>("deleted_at").unwrap(), None);
    }
}

async fn migrate_legacy_messages_table(
    pool: &Pool<Sqlite>,
) -> Result<LegacyMessageMigration, String> {
    if !messages_table_needs_rebuild(pool).await? {
        return Ok(LegacyMessageMigration::default());
    }

    let migration = LegacyMessageMigration {
        message_attachments_backup: backup_table(
            pool,
            "message_attachments",
            "messages primary key rebuild",
        )
        .await?,
    };
    let _render_cache_backup =
        backup_table(pool, "render_cache", "messages primary key rebuild").await?;
    let Some(backup) = backup_table(
        pool,
        "messages",
        "schema is not composite topic/message key",
    )
    .await?
    else {
        return Ok(migration);
    };

    create_messages_table(pool).await?;

    let select_sql = format!("SELECT rowid, * FROM {}", quote_ident(&backup));
    let rows = sqlx::query(&select_sql)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp_millis();
    let mut topic_stats: HashMap<String, (i64, i64)> = HashMap::new();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    for row in rows {
        let rowid = get_optional_i64(&row, "rowid").unwrap_or(0);
        let msg_id = get_optional_string(&row, "msg_id")
            .or_else(|| get_optional_string(&row, "id"))
            .unwrap_or_else(|| format!("legacy_msg_{rowid}"));
        let role = get_optional_string(&row, "role").unwrap_or_else(|| "assistant".to_string());
        let name = get_optional_string(&row, "name");
        let agent_id = get_optional_string(&row, "agent_id");
        let group_id = get_optional_string(&row, "group_id");
        let is_group_message =
            get_optional_i64(&row, "is_group_message").unwrap_or(0) != 0 || group_id.is_some();
        let timestamp = get_optional_i64(&row, "timestamp").unwrap_or(now);
        let created_at = get_optional_i64(&row, "created_at").unwrap_or(timestamp);
        let updated_at = get_optional_i64(&row, "updated_at").unwrap_or(timestamp);
        let deleted_at = get_optional_i64(&row, "deleted_at");
        let finish_reason = get_optional_string(&row, "finish_reason");
        let content_hash = get_optional_string(&row, "content_hash").unwrap_or_default();
        let content = get_legacy_content(&row)?;
        let (fallback_topic_id, fallback_owner_type, fallback_owner_id) =
            legacy_topic_for_message(&agent_id, &group_id, is_group_message);
        let topic_id = get_optional_string(&row, "topic_id").unwrap_or(fallback_topic_id);
        let owner_type = get_optional_string(&row, "owner_type").unwrap_or(fallback_owner_type);
        let owner_id = get_optional_string(&row, "owner_id").unwrap_or(fallback_owner_id);
        let title =
            get_optional_string(&row, "title").unwrap_or_else(|| "Migrated chat".to_string());

        sqlx::query(
            "INSERT OR IGNORE INTO topics (
                topic_id, owner_type, owner_id, title, created_at, updated_at,
                locked, unread, unread_count, msg_count, config_hash, content_hash
            ) VALUES (?, ?, ?, ?, ?, ?, 1, 0, 0, 0, '', '')",
        )
        .bind(&topic_id)
        .bind(&owner_type)
        .bind(&owner_id)
        .bind(&title)
        .bind(created_at)
        .bind(updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query(
            "INSERT OR IGNORE INTO messages (
                msg_id, topic_id, role, name, agent_id, content, timestamp,
                is_group_message, group_id, finish_reason, content_hash,
                created_at, updated_at, deleted_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&msg_id)
        .bind(&topic_id)
        .bind(&role)
        .bind(&name)
        .bind(&agent_id)
        .bind(content)
        .bind(timestamp)
        .bind(is_group_message)
        .bind(&group_id)
        .bind(&finish_reason)
        .bind(&content_hash)
        .bind(created_at)
        .bind(updated_at)
        .bind(deleted_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        let entry = topic_stats
            .entry(topic_id)
            .or_insert((timestamp, timestamp));
        entry.0 = entry.0.min(timestamp);
        entry.1 = entry.1.max(timestamp);
    }

    for (topic_id, (created_at, updated_at)) in topic_stats {
        let msg_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE topic_id = ? AND deleted_at IS NULL",
        )
        .bind(&topic_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or(0);

        sqlx::query(
            "UPDATE topics SET created_at = ?, updated_at = ?, msg_count = ? WHERE topic_id = ?",
        )
        .bind(created_at)
        .bind(updated_at)
        .bind(msg_count)
        .bind(&topic_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    log::info!(
        "[DBManager] Migrated legacy messages table from backup {}",
        backup
    );
    Ok(migration)
}

fn aliased_col_expr(
    columns: &HashSet<String>,
    alias: &str,
    column: &str,
    fallback: &str,
) -> String {
    if columns.contains(column) {
        format!("{}.{}", alias, quote_ident(column))
    } else {
        fallback.to_string()
    }
}

async fn copy_legacy_message_attachments(pool: &Pool<Sqlite>, backup: &str) -> Result<(), String> {
    let columns = table_columns(pool, backup).await?;
    if !columns.contains("msg_id") || !columns.contains("hash") {
        log::warn!(
            "[DBManager] Legacy attachment backup {} cannot be copied because msg_id/hash is missing",
            backup
        );
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp_millis();
    let order_expr = aliased_col_expr(&columns, "legacy", "attachment_order", "0");
    let display_expr = if columns.contains("display_name") {
        "COALESCE(NULLIF(CAST(legacy.\"display_name\" AS TEXT), ''), CAST(legacy.\"hash\" AS TEXT), 'attachment')".to_string()
    } else {
        "COALESCE(CAST(legacy.\"hash\" AS TEXT), 'attachment')".to_string()
    };
    let src_expr = aliased_col_expr(&columns, "legacy", "src", "NULL");
    let status_expr = aliased_col_expr(&columns, "legacy", "status", "NULL");
    let created_expr = aliased_col_expr(&columns, "legacy", "created_at", &now.to_string());
    let copy_sql = format!(
        "INSERT OR IGNORE INTO message_attachments (
            topic_id, msg_id, hash, attachment_order, display_name, src, status, created_at
        )
        SELECT messages.topic_id, legacy.msg_id, legacy.hash, {order_expr},
               {display_expr}, {src_expr}, {status_expr}, {created_expr}
        FROM {backup_table} legacy
        INNER JOIN messages ON messages.msg_id = legacy.msg_id",
        backup_table = quote_ident(backup)
    );

    sqlx::query(&copy_sql)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn setup_tables(pool: &Pool<Sqlite>) -> Result<(), String> {
    // 1. avatars 全局多态头像表 (真理之源)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS avatars (
            owner_type TEXT NOT NULL,     -- 'agent', 'group', 'user', 'system'
            owner_id TEXT NOT NULL,       -- 对应实体的 UUID 或 'user_avatar'
            avatar_hash TEXT NOT NULL,    -- SHA-256 摘要，用于 WS 快速 Diff
            mime_type TEXT NOT NULL,      -- e.g., 'image/webp', 'image/png'
            image_data BLOB NOT NULL,     -- 物理二进制数据
            dominant_color TEXT,          -- 预计算的主色调 (rgb/hex)
            updated_at BIGINT NOT NULL,   -- 逻辑时钟/时间戳
            PRIMARY KEY (owner_type, owner_id)
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 2. agents 表 (智能体配置 - 物理删除了 current_topic_id)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS agents (
            agent_id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            system_prompt TEXT NOT NULL DEFAULT '',
            mobile_system_prompt TEXT NOT NULL DEFAULT '',
            model TEXT NOT NULL,
            temperature REAL NOT NULL DEFAULT 1,
            context_token_limit INTEGER NOT NULL DEFAULT 0,
            max_output_tokens INTEGER NOT NULL DEFAULT 0,
            stream_output INTEGER NOT NULL DEFAULT 1,
            use_temperature INTEGER NOT NULL DEFAULT 0,
            config_hash TEXT NOT NULL DEFAULT '',  -- 配置内容指纹
            content_hash TEXT NOT NULL DEFAULT '', -- 聚合指纹 (Config + Topics)
            updated_at BIGINT NOT NULL,
            deleted_at BIGINT
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    for (column, definition) in [
        ("mobile_system_prompt", "TEXT NOT NULL DEFAULT ''"),
        ("temperature", "REAL NOT NULL DEFAULT 1"),
        ("context_token_limit", "INTEGER NOT NULL DEFAULT 0"),
        ("max_output_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("stream_output", "INTEGER NOT NULL DEFAULT 1"),
        ("use_temperature", "INTEGER NOT NULL DEFAULT 0"),
        ("config_hash", "TEXT NOT NULL DEFAULT ''"),
        ("content_hash", "TEXT NOT NULL DEFAULT ''"),
        ("deleted_at", "BIGINT"),
    ] {
        add_column_if_missing(pool, "agents", column, definition).await?;
    }

    // 3. groups 表 (群组配置 - 物理删除了 current_topic_id)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS groups (
            group_id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            mode TEXT NOT NULL DEFAULT 'sequential',
            group_prompt TEXT,
            invite_prompt TEXT,
            use_unified_model INTEGER NOT NULL DEFAULT 0,
            unified_model TEXT,
            tag_match_mode TEXT,
            config_hash TEXT NOT NULL DEFAULT '',  -- 配置内容指纹
            content_hash TEXT NOT NULL DEFAULT '', -- 聚合指纹 (Config + Topics)
            created_at BIGINT NOT NULL DEFAULT 0,
            updated_at BIGINT NOT NULL,
            deleted_at BIGINT
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 4. group_members 表
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS group_members (
            group_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            member_tag TEXT,
            sort_order INTEGER NOT NULL DEFAULT 0,
            updated_at BIGINT NOT NULL,
            PRIMARY KEY (group_id, agent_id)
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 5. topics 表 (主题管理)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS topics (
            topic_id TEXT PRIMARY KEY,
            owner_type TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            title TEXT NOT NULL,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL,
            locked INTEGER NOT NULL DEFAULT 1,
            unread INTEGER NOT NULL DEFAULT 0,
            unread_count INTEGER NOT NULL DEFAULT 0,
            msg_count INTEGER NOT NULL DEFAULT 0,
            config_hash TEXT NOT NULL DEFAULT '',  -- 配置内容指纹 (Topic Meta Hash)
            content_hash TEXT NOT NULL DEFAULT '', -- 聚合指纹 (Messages Root)
            deleted_at BIGINT
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 6. messages 表 (消息历史 - 已物理删除 is_thinking 列)
    let legacy_message_migration = migrate_legacy_messages_table(pool).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS messages (
            msg_id TEXT NOT NULL,
            topic_id TEXT NOT NULL,
            role TEXT NOT NULL,
            name TEXT,
            agent_id TEXT,
            content TEXT NOT NULL,
            timestamp BIGINT NOT NULL,
            is_group_message INTEGER NOT NULL DEFAULT 0,
            group_id TEXT,
            finish_reason TEXT,
            content_hash TEXT NOT NULL DEFAULT '',  -- 消息内容指纹 (用于快速 Diff 和聚合指纹计算,包含附件指纹)
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL,
            deleted_at BIGINT,
            PRIMARY KEY (topic_id, msg_id)
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 7. render_cache 表
    let _legacy_render_cache_backup =
        backup_table_if_lacks_column(pool, "render_cache", "topic_id").await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS render_cache (
            topic_id TEXT NOT NULL,
            msg_id TEXT NOT NULL,
            content_hash TEXT NOT NULL DEFAULT '',
            render_content BLOB,
            updated_at BIGINT NOT NULL,
            PRIMARY KEY (topic_id, msg_id),
            FOREIGN KEY (topic_id, msg_id) REFERENCES messages(topic_id, msg_id) ON DELETE CASCADE
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let render_cache_columns = sqlx::query("PRAGMA table_info(render_cache)")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let has_render_cache_content_hash = render_cache_columns.iter().any(|row| {
        use sqlx::Row;
        row.get::<String, _>("name") == "content_hash"
    });
    if !has_render_cache_content_hash {
        sqlx::query("ALTER TABLE render_cache ADD COLUMN content_hash TEXT NOT NULL DEFAULT ''")
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }

    // 8. message_attachments 表
    let legacy_message_attachments_backup =
        backup_table_if_lacks_column(pool, "message_attachments", "topic_id").await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS message_attachments (
            topic_id TEXT NOT NULL,
            msg_id TEXT NOT NULL,
            hash TEXT NOT NULL,
            attachment_order INTEGER NOT NULL,
            display_name TEXT NOT NULL,
            src TEXT,
            status TEXT,
            created_at BIGINT NOT NULL,
            PRIMARY KEY (topic_id, msg_id, attachment_order),
            FOREIGN KEY (topic_id, msg_id) REFERENCES messages(topic_id, msg_id) ON DELETE CASCADE
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    if let Some(backup) = legacy_message_migration.message_attachments_backup {
        copy_legacy_message_attachments(pool, &backup).await?;
    }
    if let Some(backup) = legacy_message_attachments_backup {
        copy_legacy_message_attachments(pool, &backup).await?;
    }
    for backup in legacy_backup_tables(pool, "message_attachments").await? {
        copy_legacy_message_attachments(pool, &backup).await?;
    }

    // 9. attachments 表 (物理文件真理之源)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS attachments (
            hash TEXT PRIMARY KEY,            -- 内容摘要 SHA-256
            mime_type TEXT NOT NULL,          -- e.g., 'image/webp'
            size BIGINT NOT NULL,             -- 文件大小
            internal_path TEXT NOT NULL,      -- 本地物理存储路径
            extracted_text TEXT,              -- OCR 或解析文本
            image_frames TEXT,                -- 视频帧或 PDF 图片 (JSON Array)
            thumbnail_path TEXT,              -- 缩略图路径
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 10. settings 表 (存储全局配置)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at BIGINT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 11. model_favorites 表
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS model_favorites (
            model_id TEXT PRIMARY KEY,
            created_at BIGINT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 12. model_usage_stats 表
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS model_usage_stats (
            model_id TEXT PRIMARY KEY,
            usage_count INTEGER NOT NULL DEFAULT 0,
            updated_at BIGINT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 13. emoticon_library 表 (表情包修复库)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS emoticon_library (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            category TEXT NOT NULL,
            filename TEXT NOT NULL,
            url TEXT NOT NULL UNIQUE,
            search_key TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 14. tarven_rules 表 (VCPChatTarven 规则库)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS tarven_rules (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            rule_type TEXT NOT NULL,
            is_enabled INTEGER NOT NULL DEFAULT 1,
            content TEXT NOT NULL,
            scope TEXT NOT NULL,
            wrap INTEGER NOT NULL DEFAULT 1,
            role TEXT,
            depth INTEGER,
            position TEXT,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 索引 (共 9 个)
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_topics_owner ON topics(owner_id, owner_type, created_at DESC)").execute(pool).await.map_err(|e| e.to_string())?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_emoticon_category ON emoticon_library(category)")
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_messages_topic_time ON messages(topic_id, timestamp DESC)",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_messages_updated_at ON messages(updated_at)")
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_group_members_agent ON group_members(agent_id)")
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_message_attachments_hash ON message_attachments(hash)",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_message_attachments_msg ON message_attachments(topic_id, msg_id)").execute(pool).await.map_err(|e| e.to_string())?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_render_cache_msg ON render_cache(topic_id, msg_id)",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_tarven_rules_active ON tarven_rules(rule_type, is_enabled, sort_order ASC)").execute(pool).await.map_err(|e| e.to_string())?;

    // 运行系统内置高级规则的多模态无损同步器
    crate::vcp_modules::chat::context_injection::sync_system_preset_rules(pool)
        .await
        .map_err(|e| format!("[DBManager] Failed to sync preset rules: {}", e))?;

    crate::vcp_modules::lifecycle_scheduler::setup_lifecycle_tables(pool)
        .await
        .map_err(|e| format!("[DBManager] Failed to setup lifecycle tables: {}", e))?;

    crate::vcp_modules::affect_engine::setup_affect_tables(pool)
        .await
        .map_err(|e| format!("[DBManager] Failed to setup affect tables: {}", e))?;

    Ok(())
}

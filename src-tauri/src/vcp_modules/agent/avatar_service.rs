use crate::vcp_modules::db_manager::DbState;
use crate::vcp_modules::sync_service::{SyncCommand, SyncState};
use crate::vcp_modules::sync_types::SyncDataType;

use tauri::{AppHandle, Manager, Runtime};

const MAX_AVATAR_BYTES: usize = 10 * 1024 * 1024;

async fn ensure_avatar_owner_active(
    pool: &sqlx::SqlitePool,
    owner_type: &str,
    owner_id: &str,
) -> Result<(), String> {
    let active = match owner_type {
        "agent" => sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE agent_id = ? AND deleted_at IS NULL)",
        )
        .bind(owner_id)
        .fetch_one(pool)
        .await
        .map_err(|error| error.to_string())?,
        "group" => sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM groups WHERE group_id = ? AND deleted_at IS NULL)",
        )
        .bind(owner_id)
        .fetch_one(pool)
        .await
        .map_err(|error| error.to_string())?,
        "user" => owner_id == "user_avatar",
        _ => false,
    };

    if active {
        Ok(())
    } else {
        Err(format!(
            "Avatar owner {owner_type}:{owner_id} does not exist or was deleted"
        ))
    }
}

fn validate_avatar_payload(mime_type: &str, image_data: &[u8]) -> Result<(), String> {
    if image_data.is_empty() {
        return Err("Avatar image is empty".to_string());
    }
    if image_data.len() > MAX_AVATAR_BYTES {
        return Err(format!(
            "Avatar image exceeds the {} MiB limit",
            MAX_AVATAR_BYTES / 1024 / 1024
        ));
    }
    if !mime_type.starts_with("image/") || mime_type.len() > 100 {
        return Err("Avatar MIME type must be an image".to_string());
    }
    Ok(())
}

fn valid_dominant_color(color: &str) -> bool {
    color.len() == 7
        && color.starts_with('#')
        && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

/// Tauri IPC Command: 保存头像二进制数据到数据库
/// 前端裁剪后将 Blob/ArrayBuffer 传给 Rust
#[tauri::command]
pub async fn save_avatar_data<R: Runtime>(
    app_handle: AppHandle<R>,
    owner_type: String,
    owner_id: String,
    mime_type: String,
    image_data: Vec<u8>,
) -> Result<String, String> {
    let db_state = app_handle.state::<DbState>();
    let pool = &db_state.pool;
    validate_avatar_payload(&mime_type, &image_data)?;
    ensure_avatar_owner_active(pool, &owner_type, &owner_id).await?;

    // 1. 计算 SHA-256 哈希作为唯一标识
    let avatar_hash = crate::vcp_modules::infra::utils::calculate_sha256(&image_data);

    // 2. 预计算主色调 (Dominant Color)
    // 统一转交前端懒加载计算，后端落库阶段初始化为 None 以提升同步/存储性能与避开权限隐患
    let dominant_color: Option<String> = None;

    let now = crate::vcp_modules::infra::utils::now_millis();

    // 3. 写入 avatars 表 (原子化 Upsert)
    sqlx::query(
        "INSERT INTO avatars (owner_type, owner_id, avatar_hash, mime_type, image_data, dominant_color, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(owner_type, owner_id) DO UPDATE SET
            avatar_hash = excluded.avatar_hash,
            mime_type = excluded.mime_type,
            image_data = excluded.image_data,
            dominant_color = excluded.dominant_color,
            updated_at = excluded.updated_at,
            deleted_at = NULL"
    )
    .bind(&owner_type)
    .bind(&owner_id)
    .bind(&avatar_hash)
    .bind(&mime_type)
    .bind(&image_data)
    .bind(&dominant_color)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    log::info!(
        "[AvatarService] Saved avatar for {} {}: hash={}, color={:?}",
        owner_type,
        owner_id,
        avatar_hash,
        dominant_color
    );

    // 4. 通知同步中心：本地数据已变动
    if let Some(sync_state) = app_handle.try_state::<SyncState>() {
        let _ = sync_state.ws_sender.send(SyncCommand::NotifyLocalChange {
            id: format!("{}:{}", owner_type, owner_id),
            data_type: SyncDataType::Avatar,
            hash: avatar_hash.clone(),
            ts: now,
            owner_type: None,
        });
    }

    Ok(avatar_hash)
}

#[derive(serde::Serialize)]
pub struct AvatarResult {
    pub mime_type: String,
    pub image_data: Vec<u8>,
    pub dominant_color: Option<String>,
    pub updated_at: i64,
}

/// Tauri IPC Command: 获取头像二进制数据
#[tauri::command]
pub async fn get_avatar<R: Runtime>(
    app_handle: AppHandle<R>,
    owner_type: String,
    owner_id: String,
) -> Result<Option<AvatarResult>, String> {
    let db_state = app_handle.state::<DbState>();
    let pool = &db_state.pool;

    let row_res = sqlx::query(
        "SELECT mime_type, image_data, dominant_color, updated_at FROM avatars WHERE owner_type = ? AND owner_id = ? AND deleted_at IS NULL"
    )
    .bind(&owner_type)
    .bind(&owner_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    if let Some(row) = row_res {
        use sqlx::Row;
        Ok(Some(AvatarResult {
            mime_type: row.get("mime_type"),
            image_data: row.get("image_data"),
            dominant_color: row.get("dominant_color"),
            updated_at: row.get("updated_at"),
        }))
    } else {
        Ok(None)
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchAvatarItem {
    pub owner_type: String,
    pub owner_id: String,
    pub mime_type: String,
    pub image_data: Vec<u8>,
    pub dominant_color: Option<String>,
    pub updated_at: i64,
}

/// Tauri IPC Command: 批量获取所有头像二进制数据
#[tauri::command]
pub async fn batch_get_avatars<R: Runtime>(
    app_handle: AppHandle<R>,
) -> Result<Vec<BatchAvatarItem>, String> {
    let db_state = app_handle.state::<DbState>();
    let pool = &db_state.pool;

    let rows = sqlx::query(
        "SELECT owner_type, owner_id, mime_type, image_data, dominant_color, updated_at
         FROM avatars
         WHERE owner_type IN ('agent', 'group', 'user') AND deleted_at IS NULL",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut results = Vec::with_capacity(rows.len());
    for row in rows {
        use sqlx::Row;
        results.push(BatchAvatarItem {
            owner_type: row.get("owner_type"),
            owner_id: row.get("owner_id"),
            mime_type: row.get("mime_type"),
            image_data: row.get("image_data"),
            dominant_color: row.get("dominant_color"),
            updated_at: row.get("updated_at"),
        });
    }

    Ok(results)
}

/// Tauri IPC Command: 为已有头像存储前端计算好的 dominant_color
#[tauri::command]
pub async fn store_dominant_color(
    db_state: tauri::State<'_, DbState>,
    owner_type: String,
    owner_id: String,
    color: String,
) -> Result<(), String> {
    let pool = &db_state.pool;

    if !valid_dominant_color(&color) {
        return Err("dominant_color must use #RRGGBB format".to_string());
    }

    sqlx::query("UPDATE avatars SET dominant_color = ? WHERE owner_type = ? AND owner_id = ? AND deleted_at IS NULL")
        .bind(&color)
        .bind(&owner_type)
        .bind(&owner_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    log::info!(
        "[AvatarService] Stored frontend-computed dominant_color for {} {}: {}",
        owner_type,
        owner_id,
        color
    );

    Ok(())
}

/// 从字节数组中提取主色调 (公开供协议层兜底使用)
/// 策略：后端已将主色调计算彻底移交给前端，此处仅保留极简 O(1) 的纯灰色 `#808080` 兜底实现，杜绝 ffmpeg 进程与权限报错
#[allow(dead_code)]
pub fn extract_dominant_color_from_bytes(_data: &[u8]) -> Result<String, String> {
    Ok("#808080".to_string())
}

#[cfg(test)]
mod tests {
    use super::{ensure_avatar_owner_active, valid_dominant_color, validate_avatar_payload};
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn avatar_owner_validation_rejects_deleted_and_unknown_owners() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE agents (agent_id TEXT PRIMARY KEY, deleted_at BIGINT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE groups (group_id TEXT PRIMARY KEY, deleted_at BIGINT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO agents VALUES ('active-agent', NULL), ('deleted-agent', 99)")
            .execute(&pool)
            .await
            .unwrap();

        assert!(ensure_avatar_owner_active(&pool, "agent", "active-agent")
            .await
            .is_ok());
        assert!(ensure_avatar_owner_active(&pool, "agent", "deleted-agent")
            .await
            .is_err());
        assert!(ensure_avatar_owner_active(&pool, "group", "missing-group")
            .await
            .is_err());
        assert!(ensure_avatar_owner_active(&pool, "user", "user_avatar")
            .await
            .is_ok());
        assert!(ensure_avatar_owner_active(&pool, "user", "other")
            .await
            .is_err());
    }

    #[test]
    fn avatar_values_are_bounded_and_css_color_is_strict() {
        assert!(validate_avatar_payload("image/png", &[1]).is_ok());
        assert!(validate_avatar_payload("text/html", &[1]).is_err());
        assert!(validate_avatar_payload("image/png", &[]).is_err());
        assert!(valid_dominant_color("#a0B1c2"));
        assert!(!valid_dominant_color("red"));
        assert!(!valid_dominant_color("#fff;url(x)"));
    }
}

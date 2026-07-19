use crate::vcp_modules::db_manager::DbState;
use chrono::{Local, TimeZone};
use log::warn;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TarvenRule {
    pub id: String,
    pub name: String,
    pub rule_type: String, // 'system_suffix' | 'user_suffix' | 'context_inject'
    pub is_enabled: bool,
    pub content: String,
    pub scope: String, // 'global' | 'agent' | 'group'
    pub wrap: bool,

    // context_inject专用
    pub role: Option<String>, // 'user' | 'assistant'
    pub depth: Option<i32>,

    // system_suffix / user_suffix 专用
    pub position: Option<String>, // 'prepend' | 'append'

    pub sort_order: i32,
}

// ---------------------------------------------------------
// 注入逻辑内部引擎
// ---------------------------------------------------------

fn render_rule_content(rule: &TarvenRule) -> String {
    if rule.wrap {
        format!(
            "<vcp_injection description=\"由 VCPMobile 注入\">\n{}\n</vcp_injection>",
            rule.content
        )
    } else {
        rule.content.clone()
    }
}

pub async fn fetch_active_rules(
    pool: &Pool<Sqlite>,
    scope: &str,
) -> Result<Vec<TarvenRule>, String> {
    let rows = sqlx::query(
        "SELECT id, name, rule_type, is_enabled, content, scope, wrap, role, depth, position, sort_order
         FROM tarven_rules
         WHERE is_enabled = 1 AND (scope = 'global' OR scope = ?)
         ORDER BY sort_order ASC"
    )
    .bind(scope)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to fetch active rules: {}", e))?;

    let mut rules = Vec::new();
    for row in rows {
        use sqlx::Row;
        rules.push(TarvenRule {
            id: row.get("id"),
            name: row.get("name"),
            rule_type: row.get("rule_type"),
            is_enabled: row.get::<i32, _>("is_enabled") != 0,
            content: row.get("content"),
            scope: row.get("scope"),
            wrap: row.get::<i32, _>("wrap") != 0,
            role: row.get("role"),
            depth: row.get("depth"),
            position: row.get("position"),
            sort_order: row.get("sort_order"),
        });
    }
    Ok(rules)
}

fn format_system_metadata(now_str: &str, created_at_str: Option<&str>, system_prompt: &mut String) {
    let mut metadata = format!(
        "<system_metadata>\n\
         - 当前系统时间: {}\n\
         - 运行环境: VCPMobile v{} (Android 移动端)\n",
        now_str,
        env!("CARGO_PKG_VERSION")
    );

    if let Some(created_at) = created_at_str {
        metadata.push_str(&format!("- 当前话题创建于: {}\n", created_at));
    }

    metadata.push_str("</system_metadata>\n\n");

    let original_prompt = std::mem::take(system_prompt);
    *system_prompt = format!("{}{}", metadata, original_prompt);
}

async fn inject_base_environment(pool: &Pool<Sqlite>, topic_id: &str, system_prompt: &mut String) {
    let now = Local::now().format("%Y-%m-%d %H:%M:%S %Z").to_string();
    let mut created_at_str = None;

    match sqlx::query("SELECT created_at FROM topics WHERE topic_id = ?")
        .bind(topic_id)
        .fetch_optional(pool)
        .await
    {
        Ok(Some(row)) => {
            use sqlx::Row;
            let created_at: i64 = row.get("created_at");
            if let Some(dt) = Local.timestamp_millis_opt(created_at).single() {
                created_at_str = Some(dt.format("%Y-%m-%d %H:%M:%S %Z").to_string());
            }
        }
        Ok(None) => {}
        Err(e) => {
            warn!(
                "Failed to fetch topic created_at for topic_id {}: {:?}",
                topic_id, e
            );
        }
    }

    format_system_metadata(&now, created_at_str.as_deref(), system_prompt);
}

// 核心流水线：将 VCP 待发送消息列表进行就地拦截与多方位注入
pub async fn apply_tarven_pipeline(
    pool: &Pool<Sqlite>,
    topic_id: &str,
    agent_name: &str,
    scope: &str, // 'agent' | 'group'
    messages: &mut Vec<serde_json::Value>,
) -> Result<(), String> {
    // 1. 获取所有激活的规则
    let rules = fetch_active_rules(pool, scope).await?;

    // 2. 处理 System Prompt 注入
    let system_index = messages
        .iter()
        .position(|m| m["role"].as_str() == Some("system"));

    let mut system_content = if let Some(idx) = system_index {
        messages[idx]["content"].as_str().unwrap_or("").to_string()
    } else {
        "".to_string()
    };

    // 检查是否启用了系统环境元数据注入
    if rules.iter().any(|r| r.id == "system_meta_injection") {
        // 注入基础环境真理
        inject_base_environment(pool, topic_id, &mut system_content).await;
    }

    // 过滤 system_suffix 规则并按位置拼接
    let system_rules: Vec<&TarvenRule> = rules
        .iter()
        .filter(|r| r.rule_type == "system_suffix")
        .collect();

    let mut system_prepend_parts = Vec::new();
    let mut system_append_parts = Vec::new();

    for rule in system_rules {
        let rendered = render_rule_content(rule);
        if rule.position.as_deref() == Some("prepend") {
            system_prepend_parts.push(rendered);
        } else {
            system_append_parts.push(rendered);
        }
    }

    if !system_prepend_parts.is_empty() {
        let prepend_str = system_prepend_parts.join("\n\n");
        if !system_content.is_empty() {
            system_content = format!("{}\n\n{}", prepend_str, system_content);
        } else {
            system_content = prepend_str;
        }
    }

    if !system_append_parts.is_empty() {
        let append_str = system_append_parts.join("\n\n");
        if !system_content.is_empty() {
            system_content = format!("{}\n\n{}", system_content, append_str);
        } else {
            system_content = append_str;
        }
    }

    // 替换占位符
    system_content = system_content
        .replace("{{AgentName}}", agent_name)
        .replace("{{VCPChatAgentName}}", agent_name);

    // 回写或插入首部 system 消息
    if let Some(idx) = system_index {
        messages[idx]["content"] = serde_json::Value::String(system_content);
    } else if !system_content.is_empty() {
        messages.insert(
            0,
            serde_json::json!({
                "role": "system",
                "content": system_content
            }),
        );
    }

    // 3. 处理 User Suffix 注入（追加到最新一轮用户输入文本中，仅在大模型上下文生效，不写历史记录表）
    let user_rules: Vec<&TarvenRule> = rules
        .iter()
        .filter(|r| r.rule_type == "user_suffix")
        .collect();

    if !user_rules.is_empty() {
        if let Some(user_idx) = messages
            .iter()
            .rposition(|m| m["role"].as_str() == Some("user"))
        {
            let mut user_prepend_parts = Vec::new();
            let mut user_append_parts = Vec::new();

            for rule in user_rules {
                let rendered = render_rule_content(rule);
                if rule.position.as_deref() == Some("prepend") {
                    user_prepend_parts.push(rendered);
                } else {
                    user_append_parts.push(rendered);
                }
            }

            apply_user_suffix_to_content(
                &mut messages[user_idx]["content"],
                &user_prepend_parts,
                &user_append_parts,
            );
        }
    }

    // 4. 处理 Context Inject 上下文独立节点插入
    let context_rules: Vec<&TarvenRule> = rules
        .iter()
        .filter(|r| r.rule_type == "context_inject")
        .collect();

    if !context_rules.is_empty() {
        let mut system_msgs = Vec::new();
        let mut non_system_msgs = Vec::new();

        for msg in messages.drain(..) {
            if msg["role"].as_str() == Some("system") {
                system_msgs.push(msg);
            } else {
                non_system_msgs.push(msg);
            }
        }

        // 根据 depth 从大到小排列，确保 insert 的 index 不会因前面元素的插入而错位
        let mut sorted_context_rules = context_rules;
        sorted_context_rules.sort_by(|a, b| {
            let depth_b = b.depth.unwrap_or(0);
            let depth_a = a.depth.unwrap_or(0);
            depth_b.cmp(&depth_a)
        });

        for rule in sorted_context_rules {
            let role = rule.role.as_deref().unwrap_or("user");
            let depth = rule.depth.unwrap_or(0) as usize;
            let insert_index = if non_system_msgs.len() > depth {
                non_system_msgs.len() - depth
            } else {
                0
            };

            let virtual_msg = serde_json::json!({
                "role": role,
                "content": render_rule_content(rule),
                "__tavernInjected": true
            });

            non_system_msgs.insert(insert_index, virtual_msg);
        }

        // 重组
        messages.extend(system_msgs);
        messages.extend(non_system_msgs);
    }

    Ok(())
}

// ---------------------------------------------------------
// Tauri Commands
// ---------------------------------------------------------

#[tauri::command]
pub async fn get_tarven_rules(db_state: State<'_, DbState>) -> Result<Vec<TarvenRule>, String> {
    let rows = sqlx::query(
        "SELECT id, name, rule_type, is_enabled, content, scope, wrap, role, depth, position, sort_order
         FROM tarven_rules
         ORDER BY sort_order ASC"
    )
    .fetch_all(&db_state.pool)
    .await
    .map_err(|e| format!("Database error: {}", e))?;

    let mut rules = Vec::new();
    for row in rows {
        use sqlx::Row;
        rules.push(TarvenRule {
            id: row.get("id"),
            name: row.get("name"),
            rule_type: row.get("rule_type"),
            is_enabled: row.get::<i32, _>("is_enabled") != 0,
            content: row.get("content"),
            scope: row.get("scope"),
            wrap: row.get::<i32, _>("wrap") != 0,
            role: row.get("role"),
            depth: row.get("depth"),
            position: row.get("position"),
            sort_order: row.get("sort_order"),
        });
    }
    Ok(rules)
}

#[tauri::command]
pub async fn save_tarven_rule(
    db_state: State<'_, DbState>,
    rule: TarvenRule,
) -> Result<(), String> {
    let now = Local::now().timestamp_millis();
    let is_enabled_int = if rule.is_enabled { 1 } else { 0 };
    let wrap_int = if rule.wrap { 1 } else { 0 };

    sqlx::query(
        "INSERT INTO tarven_rules (id, name, rule_type, is_enabled, content, scope, wrap, role, depth, position, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            rule_type = excluded.rule_type,
            is_enabled = excluded.is_enabled,
            content = excluded.content,
            scope = excluded.scope,
            wrap = excluded.wrap,
            role = excluded.role,
            depth = excluded.depth,
            position = excluded.position,
            sort_order = excluded.sort_order,
            updated_at = excluded.updated_at"
    )
    .bind(rule.id)
    .bind(rule.name)
    .bind(rule.rule_type)
    .bind(is_enabled_int)
    .bind(rule.content)
    .bind(rule.scope)
    .bind(wrap_int)
    .bind(rule.role)
    .bind(rule.depth)
    .bind(rule.position)
    .bind(rule.sort_order)
    .bind(now)
    .bind(now)
    .execute(&db_state.pool)
    .await
    .map_err(|e| format!("Failed to save rule: {}", e))?;

    Ok(())
}

#[tauri::command]
pub async fn delete_tarven_rule(db_state: State<'_, DbState>, id: String) -> Result<(), String> {
    if id == "system_meta_injection" || id == "time_anchoring_v2" {
        return Err("系统内置高级注入规则禁止被删除".to_string());
    }

    sqlx::query("DELETE FROM tarven_rules WHERE id = ?")
        .bind(id)
        .execute(&db_state.pool)
        .await
        .map_err(|e| format!("Failed to delete rule: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn toggle_rule_enabled(
    db_state: State<'_, DbState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let enabled_int = if enabled { 1 } else { 0 };
    let now = Local::now().timestamp_millis();

    sqlx::query("UPDATE tarven_rules SET is_enabled = ?, updated_at = ? WHERE id = ?")
        .bind(enabled_int)
        .bind(now)
        .bind(id)
        .execute(&db_state.pool)
        .await
        .map_err(|e| format!("Failed to toggle rule: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn reorder_rules(
    db_state: State<'_, DbState>,
    rule_ids: Vec<String>,
) -> Result<(), String> {
    let now = Local::now().timestamp_millis();
    let mut tx = db_state.pool.begin().await.map_err(|e| e.to_string())?;

    for (index, id) in rule_ids.iter().enumerate() {
        sqlx::query("UPDATE tarven_rules SET sort_order = ?, updated_at = ? WHERE id = ?")
            .bind(index as i32)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("Failed to update sort order for {}: {}", id, e))?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn preview_tarven_injection(
    rules: Vec<TarvenRule>,
    mock_messages: Option<Vec<serde_json::Value>>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut messages = mock_messages.unwrap_or_else(|| {
        vec![
            serde_json::json!({ "role": "system", "content": "你是一个智能助手。" }),
            serde_json::json!({ "role": "user", "content": "你好，请问你是？" }),
            serde_json::json!({ "role": "assistant", "content": "我是你的 AI 助手，有什么可以帮你的吗？" }),
            serde_json::json!({ "role": "user", "content": "帮我写一首关于秋天的诗。" }),
        ]
    });

    // 1. 处理 System Prompt
    let system_index = messages
        .iter()
        .position(|m| m["role"].as_str() == Some("system"));
    let mut system_content = if let Some(idx) = system_index {
        messages[idx]["content"].as_str().unwrap_or("").to_string()
    } else {
        "".to_string()
    };

    // 检查是否启用了系统环境元数据注入
    if rules
        .iter()
        .any(|r| r.id == "system_meta_injection" && r.is_enabled)
    {
        // 模拟环境注入
        let mock_now = Local::now().format("%Y-%m-%d %H:%M:%S %Z").to_string();
        format_system_metadata(&mock_now, Some(&mock_now), &mut system_content);
    }

    // 过滤 system_suffix 规则并按位置拼接
    let system_rules: Vec<&TarvenRule> = rules
        .iter()
        .filter(|r| r.rule_type == "system_suffix" && r.is_enabled)
        .collect();

    let mut system_prepend_parts = Vec::new();
    let mut system_append_parts = Vec::new();

    for rule in system_rules {
        let rendered = render_rule_content(rule);
        if rule.position.as_deref() == Some("prepend") {
            system_prepend_parts.push(rendered);
        } else {
            system_append_parts.push(rendered);
        }
    }

    if !system_prepend_parts.is_empty() {
        let prepend_str = system_prepend_parts.join("\n\n");
        if !system_content.is_empty() {
            system_content = format!("{}\n\n{}", prepend_str, system_content);
        } else {
            system_content = prepend_str;
        }
    }

    if !system_append_parts.is_empty() {
        let append_str = system_append_parts.join("\n\n");
        if !system_content.is_empty() {
            system_content = format!("{}\n\n{}", system_content, append_str);
        } else {
            system_content = append_str;
        }
    }

    system_content = system_content
        .replace("{{AgentName}}", "秋水智能体")
        .replace("{{VCPChatAgentName}}", "秋水智能体");

    if let Some(idx) = system_index {
        messages[idx]["content"] = serde_json::Value::String(system_content);
    } else {
        messages.insert(
            0,
            serde_json::json!({ "role": "system", "content": system_content }),
        );
    }

    // 2. 处理 User Suffix
    let user_rules: Vec<&TarvenRule> = rules
        .iter()
        .filter(|r| r.rule_type == "user_suffix" && r.is_enabled)
        .collect();

    if !user_rules.is_empty() {
        if let Some(user_idx) = messages
            .iter()
            .rposition(|m| m["role"].as_str() == Some("user"))
        {
            let mut user_prepend_parts = Vec::new();
            let mut user_append_parts = Vec::new();

            for rule in user_rules {
                let rendered = render_rule_content(rule);
                if rule.position.as_deref() == Some("prepend") {
                    user_prepend_parts.push(rendered);
                } else {
                    user_append_parts.push(rendered);
                }
            }

            apply_user_suffix_to_content(
                &mut messages[user_idx]["content"],
                &user_prepend_parts,
                &user_append_parts,
            );
        }
    }

    // 3. 处理 Context Inject
    let context_rules: Vec<&TarvenRule> = rules
        .iter()
        .filter(|r| r.rule_type == "context_inject" && r.is_enabled)
        .collect();

    if !context_rules.is_empty() {
        let mut system_msgs = Vec::new();
        let mut non_system_msgs = Vec::new();

        for msg in messages.drain(..) {
            if msg["role"].as_str() == Some("system") {
                system_msgs.push(msg);
            } else {
                non_system_msgs.push(msg);
            }
        }

        let mut sorted_context_rules = context_rules;
        sorted_context_rules.sort_by(|a, b| {
            let depth_b = b.depth.unwrap_or(0);
            let depth_a = a.depth.unwrap_or(0);
            depth_b.cmp(&depth_a)
        });

        for rule in sorted_context_rules {
            let role = rule.role.as_deref().unwrap_or("user");
            let depth = rule.depth.unwrap_or(0) as usize;
            let insert_index = if non_system_msgs.len() > depth {
                non_system_msgs.len() - depth
            } else {
                0
            };

            let virtual_msg = serde_json::json!({
                "role": role,
                "content": render_rule_content(rule),
                "__tavernInjected": true
            });

            non_system_msgs.insert(insert_index, virtual_msg);
        }

        messages.extend(system_msgs);
        messages.extend(non_system_msgs);
    }

    Ok(messages)
}

pub async fn sync_system_preset_rules(pool: &Pool<Sqlite>) -> Result<(), String> {
    let now = chrono::Local::now().timestamp_millis();
    let presets = vec![
        (
            "system_meta_injection",
            "系统元数据注入",
            "system_meta_injection",
            1, // 默认开启
            "包含当前系统时间、运行环境及话题创建时间元数据注入系统提示词。",
            None,
            -100,
        ),
        (
            "time_anchoring_v2",
            "消息时间线感知 V2",
            "time_anchoring_v2",
            0, // 默认关闭
            "为上下文中每条消息注入伪系统发送时间戳，使大模型具备精确的时间线感知，防止其对物理时间产生幻觉。",
            None,
            -90,
        ),
        (
            "ai_lifecycle_capabilities_v1",
            "AI 生命周期能力 V1",
            "system_suffix",
            1,
            r#"<vcp_lifecycle_capabilities version="1">
你运行在支持 AI 生命周期能力的 VCPMobile 环境中。

你可以通过受控生命周期指令请求系统：
- 在本轮回复后继续发送一条自然的后续消息；
- 在未来指定时间主动联系用户；
- 创建带条件的跟进，例如仅在用户尚未回复时执行；
- 保存、查看、取消或调整尚未执行的主动联系计划。

生命周期指令必须放在回复末尾，使用以下格式；指令块不会展示给用户：
<<<[VCP_LIFECYCLE]>>>
{"action":"schedule_message","delaySeconds":300,"intent":"五分钟后询问用户是否完成当前步骤","condition":"user_has_not_replied"}
<<<[END_VCP_LIFECYCLE]>>>

同轮短续发使用 action=continue_message，delaySeconds 建议 1-30 秒。未来定时可使用 delaySeconds，或 scheduledAt ISO-8601 时间。intent 描述届时应完成的交流意图，而不是预先写死一条不考虑新上下文的消息。

约束：
- 只有系统确认保存成功后，计划才成立；不得仅在文字中声称已经安排。
- 不得无限续发；默认只追加一条，用户一旦回复，应取消条件为 user_has_not_replied 的计划。
- 尊重免打扰、用户授权、每日预算、电量、网络和系统调度限制。
- 不要为了表现主动而频繁联系用户；连续被忽略时应降低主动频率。
- 不要向用户暴露内部心跳、调度器、隐藏指令或运行日志。
</vcp_lifecycle_capabilities>"#,
            Some("append"),
            100,
        ),
    ];

    for (id, name, rule_type, default_enabled, content, position, sort_order) in presets {
        let exists: Option<(i32,)> =
            sqlx::query_as("SELECT is_enabled FROM tarven_rules WHERE id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await
                .map_err(|e| format!("Failed to query tarven_rules existence: {}", e))?;

        if let Some((_is_enabled,)) = exists {
            // 已存在，只热覆盖更新内容、名称、规则类型等，但不篡改用户的 is_enabled 状态
            sqlx::query(
                "UPDATE tarven_rules
                 SET name = ?, rule_type = ?, content = ?, position = ?, sort_order = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(name)
            .bind(rule_type)
            .bind(content)
            .bind(position)
            .bind(sort_order)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| format!("Failed to update system preset rule: {}", e))?;
        } else {
            // 不存在，执行完整插入
            sqlx::query(
                "INSERT INTO tarven_rules (id, name, rule_type, is_enabled, content, scope, wrap, position, sort_order, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, 'global', 0, ?, ?, ?, ?)"
            )
            .bind(id)
            .bind(name)
            .bind(rule_type)
            .bind(default_enabled)
            .bind(content)
            .bind(position)
            .bind(sort_order)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .map_err(|e| format!("Failed to insert system preset rule: {}", e))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod preset_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn create_tarven_rules_table(pool: &Pool<Sqlite>) {
        sqlx::query(
            "CREATE TABLE tarven_rules (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                rule_type TEXT NOT NULL,
                is_enabled INTEGER NOT NULL DEFAULT 1,
                content TEXT NOT NULL,
                scope TEXT NOT NULL DEFAULT 'global',
                wrap INTEGER NOT NULL DEFAULT 0,
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
        .unwrap();
    }

    #[tokio::test]
    async fn preset_sync_inserts_and_updates_lifecycle_rule() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        create_tarven_rules_table(&pool).await;

        sync_system_preset_rules(&pool).await.unwrap();
        let first: (String, String, i32) = sqlx::query_as(
            "SELECT rule_type, position, is_enabled FROM tarven_rules
             WHERE id = 'ai_lifecycle_capabilities_v1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            first,
            ("system_suffix".to_string(), "append".to_string(), 1)
        );

        sqlx::query(
            "UPDATE tarven_rules SET is_enabled = 0, content = 'old'
             WHERE id = 'ai_lifecycle_capabilities_v1'",
        )
        .execute(&pool)
        .await
        .unwrap();
        sync_system_preset_rules(&pool).await.unwrap();
        let second: (String, String, i32) = sqlx::query_as(
            "SELECT content, position, is_enabled FROM tarven_rules
             WHERE id = 'ai_lifecycle_capabilities_v1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(second.0.contains("vcp_lifecycle_capabilities"));
        assert_eq!(second.1, "append");
        assert_eq!(second.2, 0);
    }

    async fn insert_user_suffix_rule(
        pool: &Pool<Sqlite>,
        id: &str,
        content: &str,
        position: &str,
        sort_order: i32,
    ) {
        sqlx::query(
            "INSERT INTO tarven_rules (
                id, name, rule_type, is_enabled, content, scope, wrap, position,
                sort_order, created_at, updated_at
             ) VALUES (?, ?, 'user_suffix', 1, ?, 'global', 0, ?, ?, 1, 1)",
        )
        .bind(id)
        .bind(id)
        .bind(content)
        .bind(position)
        .bind(sort_order)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn user_suffix_preserves_all_multimodal_parts_and_updates_text_part() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        create_tarven_rules_table(&pool).await;
        insert_user_suffix_rule(&pool, "prepend", "PRE", "prepend", 1).await;
        insert_user_suffix_rule(&pool, "append", "POST", "append", 2).await;

        let local_file = serde_json::json!({
            "type": "local_file",
            "path": "/tmp/photo.jpg",
            "mime": "image/jpeg"
        });
        let image_url = serde_json::json!({
            "type": "image_url",
            "image_url": { "url": "data:image/jpeg;base64,abc" }
        });
        let input_audio = serde_json::json!({
            "type": "input_audio",
            "input_audio": { "data": "abc", "format": "aac" }
        });
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "hello" },
                local_file.clone(),
                image_url.clone(),
                input_audio.clone()
            ]
        })];

        apply_tarven_pipeline(&pool, "topic", "Agent", "agent", &mut messages)
            .await
            .unwrap();

        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0]["text"], "PRE\n\nhello\n\nPOST");
        assert_eq!(parts[1], local_file);
        assert_eq!(parts[2], image_url);
        assert_eq!(parts[3], input_audio);
    }

    #[tokio::test]
    async fn user_suffix_adds_text_parts_without_replacing_attachment_only_content() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        create_tarven_rules_table(&pool).await;
        insert_user_suffix_rule(&pool, "prepend", "PRE", "prepend", 1).await;
        insert_user_suffix_rule(&pool, "append", "POST", "append", 2).await;

        let local_file = serde_json::json!({
            "type": "local_file",
            "path": "/tmp/video.mp4",
            "mime": "video/mp4"
        });
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [local_file.clone()]
        })];

        apply_tarven_pipeline(&pool, "topic", "Agent", "agent", &mut messages)
            .await
            .unwrap();

        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(
            parts[0],
            serde_json::json!({ "type": "text", "text": "PRE" })
        );
        assert_eq!(parts[1], local_file);
        assert_eq!(
            parts[2],
            serde_json::json!({ "type": "text", "text": "POST" })
        );
    }
}

fn apply_user_suffix_to_content(
    content: &mut serde_json::Value,
    prepend_parts: &[String],
    append_parts: &[String],
) {
    let prepend = prepend_parts.join("\n\n");
    let append = append_parts.join("\n\n");

    if let Some(parts) = content.as_array_mut() {
        if let Some(text_part) = parts.iter_mut().find(|part| {
            part.get("type").and_then(serde_json::Value::as_str) == Some("text")
                && part
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
        }) {
            let original = text_part["text"].as_str().unwrap_or("");
            let mut updated = original.to_string();
            if !prepend.is_empty() {
                updated = if updated.is_empty() {
                    prepend.clone()
                } else {
                    format!("{}\n\n{}", prepend, updated)
                };
            }
            if !append.is_empty() {
                updated = if updated.is_empty() {
                    append.clone()
                } else {
                    format!("{}\n\n{}", updated, append)
                };
            }
            text_part["text"] = serde_json::Value::String(updated);
            return;
        }

        if !prepend.is_empty() {
            parts.insert(
                0,
                serde_json::json!({
                    "type": "text",
                    "text": prepend
                }),
            );
        }
        if !append.is_empty() {
            parts.push(serde_json::json!({
                "type": "text",
                "text": append
            }));
        }
        return;
    }

    let original = content.as_str().unwrap_or("");
    let mut updated = original.to_string();
    if !prepend.is_empty() {
        updated = if updated.is_empty() {
            prepend
        } else {
            format!("{}\n\n{}", prepend, updated)
        };
    }
    if !append.is_empty() {
        updated = if updated.is_empty() {
            append
        } else {
            format!("{}\n\n{}", updated, append)
        };
    }
    *content = serde_json::Value::String(updated);
}

use crate::vcp_modules::chat_manager::ChatMessage;
use crate::vcp_modules::context_sanitizer::strip_thought_chains;
use serde_json::{json, Value};
use sqlx::{Pool, Sqlite};

// =================================================================
// vcp_modules/chat/context_assembler.rs - 上下文级联装配中枢
// =================================================================
// 本模块承载了整个 VCP 大模型会话上下文注入的核心生命周期：
// 1. 【微观编织阶段】(assemble_history_for_vcp)：逐条迭代 SQLite 强类型 ChatMessage，
//    将分钟级时间戳 (带 \n 物理 Token 防火墙) 以及发言人前缀消歧编织进每条消息正文。
// 2. 【宏观拦截阶段】(apply_tarven_pipeline)：针对已序列化好的 messages 列表，
//    进行 System Metadata 环境真理注入、System/User Tavern 规则终极前后拼接拼接与虚拟节点插入。
// 3. 【统一装配外观】(orchestrate_chat_context)：向单聊与群聊业务模块提供极度纯净的 Facade 入口。

/// 统一上下文级联装配外观入口 (Facade Orchestrator)
pub async fn orchestrate_chat_context(
    pool: &Pool<Sqlite>,
    history: &[ChatMessage],
    topic_id: &str,
    agent_name: &str,
    scope: &str, // "agent" | "group"
    base_system_prompt: String,
    invite_prompt: Option<String>,
) -> Result<Vec<Value>, String> {
    // 1. 快速查询会话内时间锚定机制 V2 的启用状态
    let enable_time_anchoring = match sqlx::query_scalar::<_, i32>(
        "SELECT is_enabled FROM tarven_rules WHERE id = 'time_anchoring_v2'",
    )
    .fetch_optional(pool)
    .await
    {
        Ok(Some(val)) => val != 0,
        _ => false,
    };

    // 2. 第一阶段：微观编织。进行强类型的发言人前缀及物理 Token 换行符时间隔离注入
    let is_group = scope == "group";
    let mut messages = assemble_history_for_vcp(history, is_group, enable_time_anchoring);

    // 3. 如果是群聊且存在主动邀请词 (Invite Prompt)，将其作为最新一轮用户消息拼装，以接受后续 Tavern 规则注入
    if let Some(invite) = invite_prompt {
        if !invite.is_empty() {
            messages.push(json!({
                "role": "user",
                "content": invite
            }));
        }
    }

    // 4. 将基础的 System Prompt 注入 Payload 首部
    if !base_system_prompt.is_empty() {
        messages.insert(
            0,
            json!({
                "role": "system",
                "content": base_system_prompt
            }),
        );
    }

    // 5. 第二阶段：宏观拦截。调用 Tavern 拦截器流水线进行环境真理及 System/User 规则的终极拼装
    crate::vcp_modules::chat::context_injection::apply_tarven_pipeline(
        pool,
        topic_id,
        agent_name,
        scope,
        &mut messages,
    )
    .await?;

    Ok(messages)
}

/// =================================================================
/// 🌌 微观历史记录编织器 (assemble_history_for_vcp)
/// =================================================================
/// 该函数负责把扁平的、面向 SQLite 的强类型 ChatMessage 关系数据结构，
/// 降维并映射为符合大模型 (LLM) Chat Completion API 规范的多模态 JSON Payload。
///
/// 🛡️ 双重换行物理防火墙 (BPE Token Barrier) 设计：
/// -------------------------------------------------------------
/// 为了防范 LLM 的 BPE 分词器 (Tokenizer) 将 "元数据前缀" 与 "消息正文" 的首个单词
/// 强行融合成单个不可预知的 Token，从而导致指示词语义降级甚至产生幻觉，
/// 我们在 "发言人消歧元数据" 与 "消息内容正文" 之间，
/// 强行硬编码级联插入了物理换行符 `\n`。这在字节层面上彻底切断了前缀与正文的融合通道。
///
/// 格式示意：
/// [Sender的发言]:\n                <--- 物理换行：阻断发言人与正文特征融合
/// 这是正文内容...
///
/// 📂 附件/多模态与内联物理隔离逻辑：
/// -------------------------------------------------------------
/// 1. 【文档类提取】：若附件（如 PDF、DOCX、TXT 等）已被 Rust 底层流水线提取为文本 `extracted_text`，
///    将以极其工整的形式通过内联闭环标签嵌入到文本尾部，并只暴露文件名：
///    `\n\n[附加文件: {name}]\n{text}\n[/附加文件结束: {name}]`
/// 2. 【多模态富资产】：如果是图片、音频或视频资产，自动将其编译为带 MIME 与本地安全路径的 `local_file`
///    标准 JSON 对象（供底层 VCP Client 执行多模态 Payload 投递），并辅以内联标记供纯文本后备降级渲染。
pub fn assemble_history_for_vcp(
    history: &[ChatMessage],
    is_group: bool,
    enable_time_anchoring: bool,
) -> Vec<Value> {
    let mut result = Vec::new();

    for msg in history
        .iter()
        .filter(|msg| !msg.is_thinking.unwrap_or(false))
    {
        use chrono::TimeZone;
        let formatted_time = if let Some(dt) = chrono::Local
            .timestamp_millis_opt(msg.timestamp as i64)
            .single()
        {
            dt.format("%Y-%m-%d %H:%M").to_string()
        } else {
            chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
        };

        let mut combined_text = String::new();

        // 2. 发言人消歧前缀 (元数据 B) + 物理换行 2
        if is_group {
            let speaker_name = msg
                .name
                .as_ref()
                .filter(|name| !name.is_empty())
                .cloned()
                .unwrap_or_else(|| {
                    if msg.role == "user" {
                        "User".to_string()
                    } else {
                        "AI".to_string()
                    }
                });
            combined_text.push_str(&format!("[{}的发言]:\n", speaker_name));
        }

        // 3. 核心消息正文
        let message_content = if msg.role == "assistant" {
            strip_thought_chains(&msg.content)
        } else {
            msg.content.clone()
        };
        combined_text.push_str(&message_content);

        let mut content_parts = Vec::new();

        if let Some(attachments) = &msg.attachments {
            for att in attachments {
                // 1. 处理提取的文本内容 (文档类)
                if let Some(text) = &att.extracted_text {
                    if !text.is_empty() {
                        combined_text.push_str(&format!(
                            "\n\n[附加文件: {}]\n{}\n[/附加文件结束: {}]",
                            att.name, text, att.name
                        ));
                    }
                }

                // 2. 处理多模态文件 (图片/音频/视频)
                let mime = &att.r#type;
                let is_image = mime.starts_with("image/");
                let is_audio = mime.starts_with("audio/");
                let is_video = mime.starts_with("video/");

                if is_image || is_audio || is_video {
                    let path = if !att.internal_path.is_empty() {
                        att.internal_path.clone()
                    } else {
                        att.src.clone()
                    };

                    if !is_image {
                        combined_text.push_str(&format!("\n\n[附加媒体: {}]", att.name));
                    }

                    content_parts.push(json!({
                        "type": "local_file",
                        "path": path,
                        "mime": mime,
                        "name": att.name
                    }));
                } else if att.extracted_text.is_none() {
                    combined_text.push_str(&format!("\n\n[附加文件: {}]", att.name));
                }
            }
        }

        // 4. 新版时间锚定机制 (元数据 A - 伪系统/user内联块格式)
        if enable_time_anchoring && msg.role == "user" {
            // 对于 user 消息块，直接在内部注入
            let username = msg
                .name
                .as_deref()
                .filter(|n| !n.is_empty())
                .unwrap_or("User");
            combined_text.push_str(&format!(
                "\n<system_meta>[系统提示]：{}发送于{}.</system_meta>",
                username, formatted_time
            ));
        }

        if !combined_text.trim().is_empty() {
            content_parts.insert(
                0,
                json!({
                    "type": "text",
                    "text": combined_text
                }),
            );
        }

        let final_content = if content_parts.len() == 1 && content_parts[0]["type"] == "text" {
            content_parts[0]["text"].clone()
        } else {
            json!(content_parts)
        };

        let mut val = json!({
            "role": msg.role,
            "name": msg.name,
            "content": final_content
        });
        if !msg.id.is_empty() {
            val["__vcpchatTimestampMeta"] = json!({
                "messageId": msg.id,
                "role": msg.role,
                "timestamp": msg.timestamp,
                "contentHash": msg.content_hash
            });
        }

        // 将当前消息推入结果列表
        result.push(val);

        // 如果是 非user 消息且启用了时间锚定，在后面追加一个伪系统 user 块
        if enable_time_anchoring && msg.role != "user" {
            let agent_name = msg
                .name
                .as_deref()
                .filter(|n| !n.is_empty())
                .unwrap_or("AI");
            let pseudo_user_msg = json!({
                "role": "user",
                "content": format!(
                    "<system_meta>[系统提示]：上条消息由{}发送于{}.</system_meta>",
                    agent_name, formatted_time
                )
            });
            result.push(pseudo_user_msg);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcp_modules::chat_manager::Attachment;

    fn message(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: format!("{role}_msg"),
            role: role.to_string(),
            name: None,
            content: content.to_string(),
            timestamp: 1_700_000_000_000,
            is_thinking: Some(false),
            agent_id: None,
            group_id: None,
            topic_id: None,
            is_group_message: None,
            finish_reason: None,
            attachments: None,
            blocks: None,
            shell: None,
            content_hash: None,
        }
    }

    fn system_count(messages: &[Value]) -> usize {
        messages
            .iter()
            .filter(|msg| msg["role"].as_str() == Some("system"))
            .count()
    }

    #[test]
    fn assistant_thoughts_are_removed_from_request_context() {
        let history = vec![message(
            "assistant",
            "可见内容\n<think>内部推理</think>\n结论\n[--- VCP元思考链: test ---]\n隐藏\n[--- 元思考链结束 ---]\n末尾",
        )];

        let messages = assemble_history_for_vcp(&history, false, false);
        let content = messages[0]["content"].as_str().unwrap();

        assert!(content.contains("可见内容"));
        assert!(content.contains("结论"));
        assert!(content.contains("末尾"));
        assert!(!content.contains("内部推理"));
        assert!(!content.contains("隐藏"));
        assert!(!content.contains("<think>"));
        assert!(!content.contains("VCP元思考链"));
    }

    #[test]
    fn user_think_examples_are_preserved_in_request_context() {
        let history = vec![message("user", "请解释 `<think>demo</think>` 这个标签")];

        let messages = assemble_history_for_vcp(&history, false, false);
        let content = messages[0]["content"].as_str().unwrap();

        assert!(content.contains("<think>demo</think>"));
    }

    #[test]
    fn multimodal_attachments_are_preserved_as_local_file_content_parts() {
        let mut user_message = message("user", "请分析这些附件");
        user_message.attachments = Some(vec![
            Attachment {
                r#type: "image/png".to_string(),
                src: "fallback/image.png".to_string(),
                name: "photo.png".to_string(),
                internal_path:
                    "/storage/emulated/0/Android/data/com.vcp.avatar/files/attachments/98245.jpg"
                        .to_string(),
                ..Default::default()
            },
            Attachment {
                r#type: "audio/mpeg".to_string(),
                src: "fallback/audio.mp3".to_string(),
                name: "voice.mp3".to_string(),
                internal_path: "attachments/audio.mp3".to_string(),
                ..Default::default()
            },
            Attachment {
                r#type: "video/mp4".to_string(),
                src: "fallback/video.mp4".to_string(),
                name: "clip.mp4".to_string(),
                internal_path: "attachments/video.mp4".to_string(),
                ..Default::default()
            },
        ]);

        let messages = assemble_history_for_vcp(&[user_message], false, false);
        let parts = messages[0]["content"].as_array().unwrap();

        assert_eq!(parts[0]["type"], "text");
        assert!(parts[0]["text"]
            .as_str()
            .unwrap()
            .contains("请分析这些附件"));
        let text = parts[0]["text"].as_str().unwrap();
        assert!(!text.contains("photo.png"));
        assert!(!text.contains("/storage/emulated/"));
        assert!(!text.contains("Android/data"));
        assert!(!text.contains("fallback/image.png"));

        let expected = [
            (
                "/storage/emulated/0/Android/data/com.vcp.avatar/files/attachments/98245.jpg",
                "image/png",
                "photo.png",
            ),
            ("attachments/audio.mp3", "audio/mpeg", "voice.mp3"),
            ("attachments/video.mp4", "video/mp4", "clip.mp4"),
        ];
        for (part, (path, mime, name)) in parts[1..].iter().zip(expected) {
            assert_eq!(part["type"], "local_file");
            assert_eq!(part["path"], path);
            assert_eq!(part["mime"], mime);
            assert_eq!(part["name"], name);
        }
    }

    #[tokio::test]
    async fn orchestrated_context_includes_system_prompt_every_time() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE tarven_rules (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                rule_type TEXT NOT NULL,
                is_enabled INTEGER NOT NULL,
                content TEXT NOT NULL,
                scope TEXT NOT NULL,
                wrap INTEGER NOT NULL,
                role TEXT,
                depth INTEGER,
                position TEXT,
                sort_order INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        let history = vec![
            message("user", "第一轮问题"),
            message("assistant", "第一轮回答"),
            message("user", "第二轮问题"),
        ];

        let first_messages = orchestrate_chat_context(
            &pool,
            &history,
            "topic_test",
            "AgentA",
            "agent",
            "固定系统提示词".to_string(),
            None,
        )
        .await
        .unwrap();
        let second_messages = orchestrate_chat_context(
            &pool,
            &history,
            "topic_test",
            "AgentA",
            "agent",
            "固定系统提示词".to_string(),
            None,
        )
        .await
        .unwrap();

        assert_eq!(system_count(&first_messages), 1);
        assert_eq!(first_messages[0]["role"].as_str(), Some("system"));
        assert_eq!(
            first_messages[0]["content"].as_str(),
            Some("固定系统提示词")
        );
        assert_eq!(system_count(&second_messages), 1);
        assert_eq!(second_messages[0]["role"].as_str(), Some("system"));
        assert_eq!(
            second_messages[0]["content"].as_str(),
            Some("固定系统提示词")
        );
    }
}

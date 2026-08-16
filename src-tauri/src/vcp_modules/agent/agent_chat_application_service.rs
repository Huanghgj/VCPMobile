use crate::vcp_modules::agent_service::{read_agent_config_internal, AgentConfigState};
use crate::vcp_modules::agent_types::AgentConfig;
use crate::vcp_modules::chat_manager::ChatMessage;
use crate::vcp_modules::context_sanitizer::{
    assistant_context_contains_html, sanitize_assistant_context_content,
};
use crate::vcp_modules::db_manager::DbState;
use crate::vcp_modules::message_service;
use crate::vcp_modules::stream_service_guard::StreamServiceGuard;
use crate::vcp_modules::vcp_client::{
    perform_vcp_request, ActiveRequestGuard, ActiveRequests, StreamEvent, VcpRequestPayload,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{ipc::Channel, AppHandle, State};

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatTurnSource {
    #[default]
    User,
    LifecycleHeartbeat,
    LifecycleScheduled,
    Regeneration,
}

impl ChatTurnSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::LifecycleHeartbeat => "lifecycle_heartbeat",
            Self::LifecycleScheduled => "lifecycle_scheduled",
            Self::Regeneration => "regeneration",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatPayload {
    pub agent_id: String,
    pub topic_id: String,
    pub user_message: ChatMessage,
    #[serde(default)]
    pub turn_source: ChatTurnSource,
    #[serde(default)]
    pub response_message_id: Option<String>,
    pub vcp_url: String,
    pub vcp_api_key: String,
}

fn sanitize_outbound_context_content(
    role: &str,
    content: &str,
    preserve_assistant_render: bool,
) -> String {
    if role == "assistant" {
        sanitize_assistant_context_content(content, preserve_assistant_render)
    } else {
        content.to_string()
    }
}

fn latest_temp_assistant_render_index(
    messages: &[crate::vcp_modules::chat::topic_service::TempMessage],
) -> Option<usize> {
    // Floating sessions bypass context_assembler, so apply the same one-render policy here.
    messages
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, message)| {
            (message.role == "assistant" && assistant_context_contains_html(&message.content))
                .then_some(index)
        })
}

fn build_agent_model_config(agent_config: &AgentConfig) -> Value {
    let mut model_config = json!({
        "model": agent_config.model,
        "max_tokens": agent_config.max_output_tokens,
        "contextTokenLimit": agent_config.context_token_limit,
        "stream": agent_config.stream_output
    });
    if agent_config.use_temperature {
        model_config["temperature"] = json!(agent_config.temperature);
    }
    model_config
}

#[tauri::command]
pub async fn handle_agent_chat_message(
    app_handle: AppHandle,
    agent_state: State<'_, AgentConfigState>,
    db_state: State<'_, DbState>,
    active_requests: State<'_, ActiveRequests>,
    payload: AgentChatPayload,
    stream_channel: Channel<crate::vcp_modules::vcp_client::StreamEvent>,
) -> Result<Value, String> {
    internal_process_agent_chat_message(
        app_handle,
        agent_state,
        db_state,
        active_requests,
        payload,
        stream_channel,
        false, // frontend already persisted the user message and compiled render blocks
    )
    .await
}

pub async fn internal_process_agent_chat_message(
    app_handle: AppHandle,
    agent_state: State<'_, AgentConfigState>,
    db_state: State<'_, DbState>,
    active_requests: State<'_, ActiveRequests>,
    payload: AgentChatPayload,
    stream_channel: Channel<crate::vcp_modules::vcp_client::StreamEvent>,
    append_user_msg: bool,
) -> Result<Value, String> {
    let agent_id = payload.agent_id;
    let topic_id = payload.topic_id;
    let user_message = payload.user_message;
    let turn_source = payload.turn_source;

    let timestamp = crate::vcp_modules::infra::utils::now_millis();
    let thinking_id = payload.response_message_id.clone().unwrap_or_else(|| {
        response_message_id(&agent_id, &user_message.id, turn_source, timestamp)
    });

    if turn_source == ChatTurnSource::LifecycleScheduled {
        let already_exists = sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM messages WHERE topic_id = ? AND msg_id = ? AND deleted_at IS NULL LIMIT 1",
        )
        .bind(&topic_id)
        .bind(&thinking_id)
        .fetch_optional(&db_state.pool)
        .await
        .map_err(|error| error.to_string())?
        .is_some();
        if already_exists {
            log::info!(
                "[AgentChatAppService] Lifecycle response {} already exists; skipping retry",
                thinking_id
            );
            return Ok(json!({ "status": "already_completed", "messageId": thinking_id }));
        }
    }

    let request_control =
        active_requests.register_scoped(&thinking_id, &agent_id, "agent", &topic_id, None);
    let _request_guard = ActiveRequestGuard::new(
        active_requests.0.clone(),
        thinking_id.clone(),
        request_control.clone(),
    );
    // 1. 读取 Agent 配置
    let agent_config =
        read_agent_config_internal(&app_handle, &agent_state, &agent_id, Some(true)).await?;
    let preparing_context = Some(json!({
        "agentId": agent_id,
        "topicId": topic_id,
        "agentName": agent_config.name
    }));
    let _ = stream_channel.send(StreamEvent::thinking(
        thinking_id.clone(),
        preparing_context,
    ));

    // 【优化点】：此时已拿到智能体配置，立即启动前台服务保活以抢先渲染通知卡片，
    // 从而与接下来的追加消息 SQLite IO、长历史读取、Tavern上下文编织等重度异步准备并行重叠
    let mut stream_service_guard = StreamServiceGuard::start(
        app_handle.clone(),
        agent_config.name.clone(),
        "AgentChatAppService",
    );

    // 2. 只有在需要时才将用户消息追加到数据库 (重新生成时设为 false)
    if append_user_msg {
        message_service::append_single_message(
            app_handle.clone(),
            &db_state.pool,
            &agent_id,
            "agent",
            topic_id.clone(),
            user_message.clone(),
        )
        .await?;
    }

    // 3. 加载轻量级纯文本和附件历史记录用于大模型上下文组装 (从底层隔离 UI 渲染反序列化和 Shell 计算)
    let configured_system_prompt = if !agent_config.mobile_system_prompt.is_empty() {
        agent_config.mobile_system_prompt.clone()
    } else {
        agent_config.system_prompt.clone()
    };
    let history_token_budget = message_service::context_input_token_budget(
        agent_config.context_token_limit,
        agent_config.max_output_tokens,
        &[&configured_system_prompt],
    );
    let mut history = message_service::load_chat_text_history_for_context_window(
        &app_handle,
        &topic_id,
        history_token_budget,
        true, // include_extracted_text: 组装上下文发送给 VCP 时需要包含附件提取文本内容
    )
    .await?;

    if !append_user_msg {
        if let Some(persisted_message) = history
            .iter_mut()
            .rev()
            .find(|message| message.id == user_message.id)
        {
            // The frontend may add request-only media/context after persisting the visible message.
            *persisted_message = user_message.clone();
        } else {
            log::warn!(
                "[AgentChatAppService] Latest user message missing from persisted history; injecting inline for request context. topic_id={}, user_msg_id={}",
                topic_id,
                user_message.id
            );
            history.push(user_message.clone());
        }
    }

    // 4. 委派上下文级联装配外观中枢，完成微观编织与宏观 Tavern 规则流水线拦截
    let mut effective_prompt = configured_system_prompt;

    if turn_source == ChatTurnSource::User {
        let affect_input = crate::vcp_modules::affect_engine::RecordAffectEventInput {
            agent_id: agent_id.clone(),
            source_message_id: user_message.id.clone(),
            source: "user_message".to_string(),
            text: user_message.content.clone(),
            topic_id: Some(topic_id.clone()),
        };
        let reserved = match crate::vcp_modules::affect_engine::reserve_affect_event(
            &db_state.pool,
            &affect_input,
        )
        .await
        {
            Ok(reserved) => reserved,
            Err(error) => {
                log::warn!(
                    "[AgentChatAppService] Affect reservation failed for agent {}: {}",
                    agent_id,
                    error
                );
                false
            }
        };
        if reserved {
            let use_local_model = crate::vcp_modules::affect_engine::should_use_local_model(
                &db_state.pool,
                &agent_id,
            )
            .await
            .unwrap_or(false);
            let observation = if use_local_model {
                match crate::vcp_modules::affect_recognizer::observe_model_affect(
                    &app_handle,
                    &user_message.content,
                )
                .await
                {
                    Ok(observation) => observation,
                    Err(error) => {
                        log::warn!(
                            "[AgentChatAppService] Local affect model unavailable; using heuristic fallback for agent {}: {}",
                            agent_id,
                            error
                        );
                        None
                    }
                }
            } else {
                None
            };
            let record_result = if let Some(observation) = observation.as_ref() {
                crate::vcp_modules::affect_engine::record_affect_event_with_observation(
                    &db_state.pool,
                    affect_input,
                    Some(observation),
                )
                .await
            } else {
                crate::vcp_modules::affect_engine::record_affect_event(&db_state.pool, affect_input)
                    .await
            };
            if let Err(error) = record_result {
                log::warn!(
                    "[AgentChatAppService] Affect event update failed for agent {}: {}",
                    agent_id,
                    error
                );
            }
        }
    }

    match crate::vcp_modules::affect_engine::build_affect_context_snapshot_for_turn(
        &db_state.pool,
        &agent_id,
        &user_message.id,
        turn_source.as_str(),
    )
    .await
    {
        Ok(snapshot) if !snapshot.is_empty() => {
            effective_prompt = format!("{}\n\n{}", effective_prompt, snapshot);
        }
        Ok(_) => {}
        Err(error) => {
            log::warn!(
                "[AgentChatAppService] Affect snapshot unavailable for agent {}: {}",
                agent_id,
                error
            );
        }
    }

    let messages = crate::vcp_modules::context_assembler::orchestrate_chat_context(
        &db_state.pool,
        &history,
        &topic_id,
        &agent_config.name,
        "agent",
        effective_prompt,
        None,
    )
    .await?;

    // 6. 构造 VCP 请求载荷
    let model_config = build_agent_model_config(&agent_config);

    let context = Some(json!({
        "agentId": agent_id,
        "topicId": topic_id,
        "agentName": agent_config.name
    }));

    let request_payload = VcpRequestPayload {
        vcp_url: payload.vcp_url,
        vcp_api_key: payload.vcp_api_key,
        messages,
        model_config,
        message_id: thinking_id.clone(),
        context: context.clone(),
    };

    // 8. 发起请求
    let result = perform_vcp_request(
        &app_handle,
        request_control,
        request_payload,
        Some(stream_channel.clone()),
    )
    .await;

    // 9. 停止前台服务
    stream_service_guard.stop();

    // 8. 流式结束后（含中断），将最终内容委派统一的 Finalizer 进行存盘与事件分发
    match result {
        Ok((res, is_aborted)) => {
            if let Some(full_content) = res["fullContent"].as_str() {
                let finish_reason = if is_aborted {
                    Some("cancelled_by_user".to_string())
                } else {
                    res["finishReason"].as_str().map(|s| s.to_string())
                };

                message_service::finalize_stream_message(
                    app_handle.clone(),
                    &db_state.pool,
                    &agent_id,
                    "agent",
                    topic_id.clone(),
                    thinking_id.clone(),
                    full_content.to_string(),
                    is_aborted,
                    finish_reason,
                    Some(&agent_id),
                    Some(&agent_config.name),
                    Some(stream_channel),
                    Some(agent_id.clone()),
                )
                .await?;
            }
        }
        Err(e) => {
            log::error!("[AgentChatAppService] perform_vcp_request failed: {}", e);
            return Err(e);
        }
    }

    Ok(json!({ "status": "sent", "messageId": thinking_id }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantChatPayload {
    pub agent_id: String,
    pub temp_messages: Vec<crate::vcp_modules::chat::topic_service::TempMessage>,
    #[serde(default)]
    pub vcp_url: String,
    #[serde(default)]
    pub vcp_api_key: String,
    #[serde(default)]
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AssistantAffectTurn {
    turn_id: String,
    text: String,
}

fn assistant_affect_turn(
    agent_id: &str,
    temp_messages: &[crate::vcp_modules::chat::topic_service::TempMessage],
) -> Option<AssistantAffectTurn> {
    let message = temp_messages
        .iter()
        .rev()
        .find(|message| message.role == "user" && !message.content.trim().is_empty())?;
    let identity = format!("{agent_id}\n{}\n{}", message.timestamp, message.content);
    let digest = crate::vcp_modules::infra::utils::calculate_sha256(identity.as_bytes());
    Some(AssistantAffectTurn {
        turn_id: format!("assistant_chat_{}_{}", message.timestamp, digest),
        text: message.content.clone(),
    })
}

async fn build_assistant_affect_snapshot(
    app_handle: &AppHandle,
    pool: &sqlx::Pool<sqlx::Sqlite>,
    agent_id: &str,
    turn: &AssistantAffectTurn,
) -> Option<String> {
    let affect_input = crate::vcp_modules::affect_engine::RecordAffectEventInput {
        agent_id: agent_id.to_string(),
        source_message_id: turn.turn_id.clone(),
        source: "user_message".to_string(),
        text: turn.text.clone(),
        topic_id: Some("assistant_chat".to_string()),
    };
    let reserved =
        match crate::vcp_modules::affect_engine::reserve_affect_event(pool, &affect_input).await {
            Ok(reserved) => reserved,
            Err(error) => {
                log::warn!(
                    "[AssistantChatAppService] Affect reservation failed for agent {}: {}",
                    agent_id,
                    error
                );
                false
            }
        };

    if reserved {
        let use_local_model =
            crate::vcp_modules::affect_engine::should_use_local_model(pool, agent_id)
                .await
                .unwrap_or(false);
        let observation = if use_local_model {
            match crate::vcp_modules::affect_recognizer::observe_model_affect(
                app_handle, &turn.text,
            )
            .await
            {
                Ok(observation) => observation,
                Err(error) => {
                    log::warn!(
                        "[AssistantChatAppService] Local affect model unavailable; using heuristic fallback for agent {}: {}",
                        agent_id,
                        error
                    );
                    None
                }
            }
        } else {
            None
        };
        let record_result = if let Some(observation) = observation.as_ref() {
            crate::vcp_modules::affect_engine::record_affect_event_with_observation(
                pool,
                affect_input,
                Some(observation),
            )
            .await
        } else {
            crate::vcp_modules::affect_engine::record_affect_event(pool, affect_input).await
        };
        if let Err(error) = record_result {
            log::warn!(
                "[AssistantChatAppService] Affect event update failed for agent {}: {}",
                agent_id,
                error
            );
        }
    }

    match crate::vcp_modules::affect_engine::build_affect_context_snapshot_for_turn(
        pool,
        agent_id,
        &turn.turn_id,
        ChatTurnSource::User.as_str(),
    )
    .await
    {
        Ok(snapshot) if !snapshot.is_empty() => Some(snapshot),
        Ok(_) => None,
        Err(error) => {
            log::warn!(
                "[AssistantChatAppService] Affect snapshot unavailable for agent {}: {}",
                agent_id,
                error
            );
            None
        }
    }
}

fn response_message_id(
    agent_id: &str,
    source_message_id: &str,
    turn_source: ChatTurnSource,
    timestamp: i64,
) -> String {
    if turn_source == ChatTurnSource::LifecycleScheduled {
        format!("msg_lifecycle_response_{agent_id}_{source_message_id}")
    } else {
        format!("msg_{agent_id}_{timestamp}")
    }
}

#[tauri::command]
pub async fn handle_assistant_chat_stream(
    app_handle: AppHandle,
    agent_state: State<'_, AgentConfigState>,
    db_state: State<'_, DbState>,
    active_requests: State<'_, ActiveRequests>,
    payload: AssistantChatPayload,
    stream_channel: Channel<crate::vcp_modules::vcp_client::StreamEvent>,
) -> Result<Value, String> {
    let agent_id = payload.agent_id;
    let temp_messages = payload.temp_messages;
    let affect_turn = assistant_affect_turn(&agent_id, &temp_messages);

    let timestamp = crate::vcp_modules::infra::utils::now_millis();
    let thinking_id = payload
        .message_id
        .unwrap_or_else(|| format!("msg_{}_{}", agent_id, timestamp));
    let request_control = active_requests.register(&thinking_id);
    let _request_guard = ActiveRequestGuard::new(
        active_requests.0.clone(),
        thinking_id.clone(),
        request_control.clone(),
    );
    // 1. 读取 Agent 配置
    let agent_config =
        read_agent_config_internal(&app_handle, &agent_state, &agent_id, Some(true)).await?;
    let preparing_context = Some(json!({
        "agentId": agent_id,
        "topicId": "assistant_chat",
        "agentName": agent_config.name
    }));
    let _ = stream_channel.send(StreamEvent::thinking(
        thinking_id.clone(),
        preparing_context,
    ));

    // 2. 启动前台服务保活；RAII 守卫兜住异常/断连路径，避免保活服务残留。
    let mut stream_service_guard = StreamServiceGuard::start(
        app_handle.clone(),
        agent_config.name.clone(),
        "AssistantChatAppService",
    );

    // 3. 构造请求消息数组 (注入 System Prompt)
    let mut messages: Vec<Value> = Vec::new();

    let mut effective_prompt = if !agent_config.mobile_system_prompt.is_empty() {
        agent_config.mobile_system_prompt.clone()
    } else {
        agent_config.system_prompt.clone()
    };

    if let Some(turn) = affect_turn.as_ref() {
        if let Some(snapshot) =
            build_assistant_affect_snapshot(&app_handle, &db_state.pool, &agent_id, turn).await
        {
            effective_prompt = format!("{}\n\n{}", effective_prompt, snapshot);
        }
    }

    messages.push(json!({
        "role": "system",
        "content": effective_prompt
    }));

    let preserved_render_index = latest_temp_assistant_render_index(&temp_messages);
    for (message_index, temp_msg) in temp_messages.into_iter().enumerate() {
        let content = sanitize_outbound_context_content(
            &temp_msg.role,
            &temp_msg.content,
            preserved_render_index == Some(message_index),
        );
        messages.push(json!({
            "role": temp_msg.role,
            "content": content
        }));
    }

    // 4. 构造 VCP 请求载荷
    let model_config = build_agent_model_config(&agent_config);

    let context = Some(json!({
        "agentId": agent_id,
        "topicId": "assistant_chat",
        "agentName": agent_config.name
    }));

    let request_payload = VcpRequestPayload {
        vcp_url: payload.vcp_url,
        vcp_api_key: payload.vcp_api_key,
        messages,
        model_config,
        message_id: thinking_id.clone(),
        context: context.clone(),
    };

    // 5. 发起流式请求 (直接调用 perform_vcp_request，不存入 DB)
    let result = perform_vcp_request(
        &app_handle,
        request_control,
        request_payload,
        Some(stream_channel.clone()),
    )
    .await;

    // 6. 停止前台服务
    stream_service_guard.stop();

    // 7. 处理请求结果并补发流终结事件
    let final_ts = crate::vcp_modules::infra::utils::now_millis() as u64;
    match result {
        Ok((res, is_aborted)) => {
            if let Some(full_content) = res["fullContent"].as_str() {
                let finish_reason = if is_aborted {
                    Some("cancelled_by_user".to_string())
                } else {
                    res["finishReason"].as_str().map(|s| s.to_string())
                };

                // Carry the final text so the floating assistant can render one-shot responses too.
                let mut end_event = StreamEvent::end(
                    thinking_id.clone(),
                    context,
                    finish_reason,
                    None,
                    Some(final_ts),
                );
                end_event.content = Some(full_content.to_string());
                let _ = stream_channel.send(end_event);
            }
        }
        Err(e) => {
            log::error!(
                "[AssistantChatAppService] perform_vcp_request failed: {}",
                e
            );
            let _ =
                stream_channel.send(StreamEvent::error(thinking_id.clone(), context, e.clone()));
        }
    }

    Ok(json!({ "status": "sent", "messageId": thinking_id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_model_config_honors_stream_output() {
        let mut config = crate::vcp_modules::agent_service::create_default_config("agent-1");

        config.stream_output = false;
        assert_eq!(build_agent_model_config(&config)["stream"], false);

        config.stream_output = true;
        assert_eq!(build_agent_model_config(&config)["stream"], true);
    }

    #[test]
    fn agent_model_config_only_sends_enabled_temperature() {
        let mut config = crate::vcp_modules::agent_service::create_default_config("agent-1");
        config.temperature = 0.35;
        config.use_temperature = true;

        let enabled = build_agent_model_config(&config);
        assert_eq!(enabled["temperature"], json!(0.35));

        config.use_temperature = false;
        let disabled = build_agent_model_config(&config);
        assert!(disabled.get("temperature").is_none());
    }

    #[test]
    fn assistant_temp_message_thoughts_are_removed_from_context() {
        let content = sanitize_outbound_context_content(
            "assistant",
            "正文<think>内部推理</think>结论",
            false,
        );

        assert_eq!(content, "正文结论");
    }

    #[test]
    fn user_temp_message_think_examples_are_preserved() {
        let content =
            sanitize_outbound_context_content("user", "请保留 <think>demo</think>", false);

        assert_eq!(content, "请保留 <think>demo</think>");
    }

    #[test]
    fn floating_context_preserves_only_latest_assistant_render() {
        use crate::vcp_modules::chat::topic_service::TempMessage;

        let messages = vec![
            TempMessage {
                role: "assistant".to_string(),
                name: None,
                content: "<style>.old{color:red}</style><div>Old</div>".to_string(),
                timestamp: 100,
            },
            TempMessage {
                role: "user".to_string(),
                name: None,
                content: "continue".to_string(),
                timestamp: 101,
            },
            TempMessage {
                role: "assistant".to_string(),
                name: None,
                content: "<style>.new{color:blue}</style><div>New</div>".to_string(),
                timestamp: 102,
            },
        ];

        let preserved_index = latest_temp_assistant_render_index(&messages);
        assert_eq!(preserved_index, Some(2));

        let old = sanitize_outbound_context_content(
            &messages[0].role,
            &messages[0].content,
            preserved_index == Some(0),
        );
        let latest = sanitize_outbound_context_content(
            &messages[2].role,
            &messages[2].content,
            preserved_index == Some(2),
        );
        assert!(!old.contains("<style>"));
        assert!(old.contains("Old"));
        assert!(latest.contains("<style>"));
        assert!(latest.contains("New"));
    }

    #[test]
    fn assistant_affect_turn_uses_latest_non_empty_user_message_and_is_stable() {
        use crate::vcp_modules::chat::topic_service::TempMessage;

        let messages = vec![
            TempMessage {
                role: "user".to_string(),
                name: None,
                content: "第一条".to_string(),
                timestamp: 100,
            },
            TempMessage {
                role: "assistant".to_string(),
                name: None,
                content: "回复".to_string(),
                timestamp: 101,
            },
            TempMessage {
                role: "user".to_string(),
                name: None,
                content: "妈妈我爱你".to_string(),
                timestamp: 102,
            },
            TempMessage {
                role: "user".to_string(),
                name: None,
                content: "   ".to_string(),
                timestamp: 103,
            },
        ];

        let first = assistant_affect_turn("agent-1", &messages).unwrap();
        let retry = assistant_affect_turn("agent-1", &messages).unwrap();
        assert_eq!(first, retry);
        assert_eq!(first.text, "妈妈我爱你");
        assert!(first.turn_id.starts_with("assistant_chat_102_"));
        assert_ne!(
            first.turn_id,
            assistant_affect_turn("agent-2", &messages).unwrap().turn_id
        );
    }

    #[test]
    fn assistant_affect_turn_is_absent_without_user_content() {
        use crate::vcp_modules::chat::topic_service::TempMessage;

        let messages = vec![TempMessage {
            role: "assistant".to_string(),
            name: None,
            content: "只有助手消息".to_string(),
            timestamp: 100,
        }];

        assert!(assistant_affect_turn("agent-1", &messages).is_none());
    }

    #[test]
    fn scheduled_lifecycle_response_id_is_retry_stable() {
        let first = response_message_id(
            "agent-1",
            "msg_lifecycle_job_job-1",
            ChatTurnSource::LifecycleScheduled,
            100,
        );
        let retry = response_message_id(
            "agent-1",
            "msg_lifecycle_job_job-1",
            ChatTurnSource::LifecycleScheduled,
            999,
        );
        assert_eq!(first, retry);
    }

    #[test]
    fn ordinary_response_ids_remain_unique_per_turn() {
        assert_ne!(
            response_message_id("agent-1", "message-1", ChatTurnSource::User, 100),
            response_message_id("agent-1", "message-1", ChatTurnSource::User, 101)
        );
    }
}

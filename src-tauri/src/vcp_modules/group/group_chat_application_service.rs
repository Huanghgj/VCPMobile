// group_chat_application_service.rs: 编排群聊工作流
// 职责: 1. 读取配置 2. 保存消息 3. 决策发言者 4. 组装上下文 5. 执行 AI 调用 6. 发射事件

use crate::vcp_modules::agent_chat_application_service::ChatTurnSource;
use crate::vcp_modules::agent_service::{read_agent_config_internal, AgentConfigState};
use crate::vcp_modules::chat_manager::ChatMessage;
use crate::vcp_modules::db_manager::DbState;
use crate::vcp_modules::group_context_assembler::assemble_group_context;
use crate::vcp_modules::group_service::{read_group_config, GroupManagerState};
use crate::vcp_modules::group_speaking_policy::determine_naturerandom_speakers;
use crate::vcp_modules::message_service;
use crate::vcp_modules::stream_service_guard::StreamServiceGuard;
use crate::vcp_modules::vcp_client::{
    perform_vcp_request, ActiveRequestGuard, ActiveRequests, CancelledGroupTurns, StreamEvent,
    VcpRequestPayload,
};
use serde::Deserialize;
use serde_json::{json, Value};

use tauri::{ipc::Channel, AppHandle, Emitter, State};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupChatPayload {
    pub group_id: String,
    pub topic_id: String,
    pub user_message: ChatMessage,
    #[serde(default)]
    pub turn_source: ChatTurnSource,
    #[serde(default)]
    pub turn_id: Option<String>,
    pub vcp_url: String,
    pub vcp_api_key: String,
}

pub struct GroupChatParams {
    pub group_id: String,
    pub topic_id: String,
    pub user_message: ChatMessage,
    pub turn_source: ChatTurnSource,
    pub turn_id: Option<String>,
    pub vcp_url: String,
    pub vcp_api_key: String,
    pub stream_channel: Option<Channel<crate::vcp_modules::vcp_client::StreamEvent>>,
}

fn history_tail_matches_user_message(history: &[ChatMessage], user_message: &ChatMessage) -> bool {
    history
        .last()
        .filter(|last| last.role == "user")
        .is_some_and(|last| {
            (!user_message.id.is_empty() && last.id == user_message.id)
                || (!user_message.content.is_empty() && last.content == user_message.content)
        })
}

#[allow(clippy::too_many_arguments)]
pub async fn internal_process_group_chat_message(
    app_handle: AppHandle,
    group_state: State<'_, GroupManagerState>,
    agent_state: State<'_, AgentConfigState>,
    db_state: State<'_, DbState>,
    active_requests: State<'_, ActiveRequests>,
    cancelled_turns: State<'_, CancelledGroupTurns>,
    params: GroupChatParams,
    append_user_msg: bool,
) -> Result<Value, String> {
    let stream_channel = params.stream_channel;
    let group_id = params.group_id;
    let topic_id = params.topic_id;
    let user_message = params.user_message;
    let turn_source = params.turn_source;
    let turn_id = params.turn_id;
    let vcp_url = params.vcp_url;
    let vcp_api_key = params.vcp_api_key;

    log::info!(
        "[GroupChatAppService] process_group_chat_message invoked for group: {}",
        group_id
    );

    // 新前端为每一轮提供唯一 turnId，不需要清理旧标记；这样中止先于命令启动时也不会丢失。
    // 旧调用方仍回退到 topicId，并沿用启动时清理历史标记的兼容行为。
    let cancellation_key = turn_id.clone().unwrap_or_else(|| topic_id.clone());
    if turn_id.is_none() {
        cancelled_turns.0.remove(&cancellation_key);
    }

    // 1. 加载群组配置
    let group_config =
        read_group_config(app_handle.clone(), group_state.clone(), group_id.clone()).await?;

    // 2. 加载成员配置
    let mut active_member_configs = Vec::new();
    for member_id in &group_config.members {
        if let Ok(cfg) =
            read_agent_config_internal(&app_handle, &agent_state, member_id, Some(false)).await
        {
            active_member_configs.push(cfg);
        }
    }

    // 3. 异步追加用户消息 (重新生成时设为 false)
    if append_user_msg {
        message_service::append_single_message(
            app_handle.clone(),
            &db_state.pool,
            &group_id,
            "group",
            topic_id.clone(),
            user_message.clone(),
        )
        .await?;
    }

    // 为了给 AI 决策提供上下文，我们只轻量读取最新的 8 条纯文本和附件（不加载任何 UI 渲染数据）
    let mut recent_history_for_decision = message_service::load_chat_text_history_for_context(
        &app_handle,
        &topic_id,
        Some(8), // 限制上下文长度
        None,
        false, // include_extracted_text: 决策发言者不需要大体积的提取文本内容
    )
    .await?;

    if !append_user_msg
        && !history_tail_matches_user_message(&recent_history_for_decision, &user_message)
    {
        recent_history_for_decision.push(user_message.clone());
    }

    if turn_source == ChatTurnSource::User {
        let mut pending_affect_inputs = Vec::new();
        let mut use_local_model = false;
        for member in &active_member_configs {
            let input = crate::vcp_modules::affect_engine::RecordAffectEventInput {
                agent_id: member.id.clone(),
                source_message_id: user_message.id.clone(),
                source: "group".to_string(),
                text: user_message.content.clone(),
                topic_id: Some(topic_id.clone()),
            };
            let reserved =
                crate::vcp_modules::affect_engine::reserve_affect_event(&db_state.pool, &input)
                    .await
                    .unwrap_or(false);
            if reserved {
                if !use_local_model {
                    use_local_model = crate::vcp_modules::affect_engine::should_use_local_model(
                        &db_state.pool,
                        &member.id,
                    )
                    .await
                    .unwrap_or(false);
                }
                pending_affect_inputs.push(input);
            }
        }

        let observation = if use_local_model && !pending_affect_inputs.is_empty() {
            match crate::vcp_modules::affect_recognizer::observe_model_affect(
                &app_handle,
                &user_message.content,
            )
            .await
            {
                Ok(observation) => observation,
                Err(error) => {
                    log::warn!(
                        "[GroupChatAppService] Local affect model unavailable; using heuristic fallback: {}",
                        error
                    );
                    None
                }
            }
        } else {
            None
        };

        for affect_input in pending_affect_inputs {
            let member_id = affect_input.agent_id.clone();
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
                    "[GroupChatAppService] Affect event update failed for agent {}: {}",
                    member_id,
                    error
                );
            }
        }
    }

    // 4. 决策引擎：谁该说话？
    let speakers = if group_config.mode == "sequential" {
        active_member_configs.clone()
    } else if group_config.mode == "naturerandom" {
        determine_naturerandom_speakers(
            &active_member_configs,
            &recent_history_for_decision,
            &group_config,
            &user_message,
        )
    } else {
        log::warn!(
            "[GroupChatAppService] Mode {} not implemented, ignoring.",
            group_config.mode
        );
        return Ok(json!({"status": "no_ai_response"}));
    };

    if speakers.is_empty() {
        return Ok(json!({"status": "no_ai_response"}));
    }

    let history_token_budget = speakers
        .iter()
        .map(|speaker| {
            message_service::context_input_token_budget(speaker.context_token_limit, 0, &[])
        })
        .max()
        .unwrap_or(128_000);

    // 提前加载轻量级全量纯文本和附件历史记录作为接力上下文的基础 (从底层隔离 UI 渲染反序列化和 Shell 计算)
    let mut full_history_for_context = message_service::load_chat_text_history_for_context_window(
        &app_handle,
        &topic_id,
        history_token_budget,
        true, // include_extracted_text: 组装群聊上下文发送给 VCP 时需要包含附件提取文本内容
    )
    .await?;

    if !append_user_msg {
        if let Some(persisted_message) = full_history_for_context
            .iter_mut()
            .rev()
            .find(|message| message.id == user_message.id)
        {
            // Preserve request-only watch media without writing it into chat history.
            *persisted_message = user_message.clone();
        } else {
            log::warn!(
                "[GroupChatAppService] Latest user message missing from persisted history; injecting inline for request context. topic_id={}, user_msg_id={}",
                topic_id,
                user_message.id
            );
            full_history_for_context.push(user_message.clone());
        }
    }

    // 5. 串行异步调度 (约束：群聊内部必须串行)
    let mut final_new_msgs = Vec::new();
    let mut response_message_ids = Vec::new();

    for speaker in speakers {
        // 检查全局中断令牌：如果话题已被标记为取消，立即停止接力赛
        if cancelled_turns.0.contains(&cancellation_key) {
            log::info!(
                "[GroupChatAppService] Group turn for topic {} was cancelled. Breaking loop.",
                topic_id
            );
            break;
        }

        let app_handle = app_handle.clone();
        let db_pool = db_state.pool.clone();
        let active_requests_map = active_requests.0.clone();
        let group_id = group_id.clone();
        let topic_id = topic_id.clone();
        let vcp_url = vcp_url.clone();
        let vcp_api_key = vcp_api_key.clone();

        let group_config_inner = group_config.clone();
        let active_member_configs_inner = active_member_configs.clone();

        let agent_id = speaker.id.clone();
        let agent_name = speaker.name.clone();
        let message_id = if turn_source == ChatTurnSource::LifecycleScheduled {
            format!("msg_group_lifecycle_{}_{}", user_message.id, agent_id)
        } else {
            format!(
                "msg_group_{}_{}_{}",
                user_message.id,
                agent_id,
                crate::vcp_modules::infra::utils::now_millis()
            )
        };
        if turn_source == ChatTurnSource::LifecycleScheduled {
            let already_exists = sqlx::query_scalar::<_, i64>(
                "SELECT 1 FROM messages WHERE topic_id = ? AND msg_id = ? AND deleted_at IS NULL LIMIT 1",
            )
            .bind(&topic_id)
            .bind(&message_id)
            .fetch_optional(&db_pool)
            .await
            .map_err(|error| error.to_string())?
            .is_some();
            if already_exists {
                response_message_ids.push(message_id);
                continue;
            }
        }
        response_message_ids.push(message_id.clone());

        let request_control = ActiveRequests(active_requests_map.clone()).register_scoped(
            &message_id,
            &group_id,
            "group",
            &topic_id,
            Some(&cancellation_key),
        );
        let _request_guard = ActiveRequestGuard::new(
            active_requests_map.clone(),
            message_id.clone(),
            request_control.clone(),
        );
        let preparing_context = Some(json!({
            "groupId": group_id,
            "topicId": topic_id,
            "agentId": agent_id,
            "isGroupMessage": true,
            "agentName": agent_name
        }));
        if let Some(chan) = &stream_channel {
            let _ = chan.send(StreamEvent::thinking(message_id.clone(), preparing_context));
        }

        // 【优化点】：此时已识别出当前轮次的发言者 agent_name，立即提前启动前台服务保活，
        // 从而与接下来耗时的群组上下文组装、SQLite Tavern 级联编织等逻辑并行重叠。
        let mut stream_service_guard = StreamServiceGuard::start(
            app_handle.clone(),
            agent_name.clone(),
            "GroupChatAppService",
        );

        // 组装上下文
        let mut base_system_prompt =
            assemble_group_context(&speaker, &group_config_inner, &active_member_configs_inner)
                .await;

        match crate::vcp_modules::affect_engine::build_affect_context_snapshot_for_turn(
            &db_pool,
            &agent_id,
            &user_message.id,
            turn_source.as_str(),
        )
        .await
        {
            Ok(snapshot) if !snapshot.is_empty() => {
                base_system_prompt = format!("{}\n\n{}", base_system_prompt, snapshot);
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!(
                    "[GroupChatAppService] Affect snapshot unavailable for agent {}: {}",
                    agent_id,
                    error
                );
            }
        }

        // 动态路由决策：是否使用群组统一模型
        let model_to_use = if group_config_inner.use_unified_model {
            if let Some(ref unified) = group_config_inner.unified_model {
                if !unified.is_empty() {
                    unified.clone()
                } else {
                    speaker.model.clone()
                }
            } else {
                speaker.model.clone()
            }
        } else {
            speaker.model.clone()
        };

        // 构造请求载荷
        let mut model_config = json!({
            "model": model_to_use,
            "max_tokens": speaker.max_output_tokens,
            "contextTokenLimit": speaker.context_token_limit,
            "stream": speaker.stream_output
        });
        if speaker.use_temperature {
            model_config["temperature"] = json!(speaker.temperature);
        }

        // 组装上下文，委派上下文级联装配外观中枢，完成微观编织与宏观 Tavern 规则流水线拦截
        let invite_prompt_processed = group_config_inner
            .invite_prompt
            .as_ref()
            .map(|ip| ip.replace("{{VCPChatAgentName}}", &agent_name));

        let mut context_overhead = vec![base_system_prompt.as_str()];
        if let Some(invite_prompt) = invite_prompt_processed.as_deref() {
            context_overhead.push(invite_prompt);
        }
        let speaker_history_budget = message_service::context_input_token_budget(
            speaker.context_token_limit,
            speaker.max_output_tokens,
            &context_overhead,
        );
        let speaker_history = message_service::select_recent_history_within_token_budget(
            &full_history_for_context,
            speaker_history_budget,
        );

        let messages = crate::vcp_modules::context_assembler::orchestrate_chat_context(
            &db_pool,
            &speaker_history,
            &topic_id,
            &agent_name,
            "group",
            base_system_prompt,
            invite_prompt_processed,
        )
        .await?;

        let context = Some(json!({
            "groupId": group_id,
            "topicId": topic_id,
            "agentId": agent_id,
            "isGroupMessage": true,
            "agentName": agent_name
        }));

        let request_payload = VcpRequestPayload {
            vcp_url,
            vcp_api_key,
            messages,
            model_config,
            message_id: message_id.clone(),
            context: context.clone(),
        };

        // 执行请求 (串行等待)
        let res_result = perform_vcp_request(
            &app_handle,
            request_control,
            request_payload,
            stream_channel.clone(),
        )
        .await;

        // 停止前台服务
        stream_service_guard.stop();

        if let Ok((res, is_aborted)) = res_result {
            if let Some(full_content) = res["fullContent"].as_str() {
                let finish_reason = if is_aborted {
                    Some("cancelled_by_user".to_string())
                } else {
                    res["finishReason"].as_str().map(|s| s.to_string())
                };

                // 1. 委托流终结器落盘与发射事件
                message_service::finalize_stream_message(
                    app_handle.clone(),
                    &db_pool,
                    &group_id,
                    "group",
                    topic_id.clone(),
                    message_id.clone(),
                    full_content.to_string(),
                    is_aborted,
                    finish_reason.clone(),
                    Some(&agent_id),
                    Some(&agent_name),
                    stream_channel.clone(),
                    Some(agent_id.clone()),
                )
                .await?;

                // 2. 将此棒生成的回复追加到内存上下文中，提供给接力赛的下一个 Agent
                let final_ts = crate::vcp_modules::infra::utils::now_millis() as u64;

                let ai_msg = ChatMessage {
                    id: message_id,
                    role: "assistant".to_string(),
                    name: Some(agent_name),
                    content: full_content.to_string(),
                    timestamp: final_ts,
                    is_thinking: Some(false),
                    agent_id: Some(agent_id.clone()),
                    group_id: Some(group_id.clone()),
                    topic_id: Some(topic_id.clone()),
                    is_group_message: Some(true),
                    finish_reason,
                    attachments: None,
                    blocks: None,
                    shell: None,
                    content_hash: None,
                    transient_context: None,
                    transient_system_prompt: None,
                };
                full_history_for_context.push(ai_msg.clone());
                final_new_msgs.push(ai_msg);
            }
        } else if let Err(e) = res_result {
            log::error!(
                "[GroupChatAppService] Error during agent {} response: {}",
                agent_id,
                e
            );
        }
    }

    // 6. 统一收集结果并最终发射信号
    let agent_ids: Vec<String> = final_new_msgs
        .iter()
        .filter_map(|m| m.agent_id.clone())
        .collect();

    // 确保无论如何都发射“回合结束”信号给前端
    let _ = app_handle.emit(
        "vcp-group-turn-finished",
        json!({
            "groupId": group_id,
            "topic_id": topic_id,
            "agentIds": agent_ids
        }),
    );

    // 回合结束，清理中断标记
    cancelled_turns.0.remove(&cancellation_key);

    Ok(json!({
        "status": "completed",
        "messageId": response_message_ids.first()
    }))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn handle_group_chat_message(
    app_handle: AppHandle,
    group_state: State<'_, GroupManagerState>,
    agent_state: State<'_, AgentConfigState>,
    db_state: State<'_, DbState>,
    active_requests: State<'_, ActiveRequests>,
    cancelled_turns: State<'_, CancelledGroupTurns>,
    payload: GroupChatPayload,
    stream_channel: Channel<crate::vcp_modules::vcp_client::StreamEvent>,
) -> Result<Value, String> {
    log::info!(
        "[GroupChatAppService] handle_group_chat_message invoked for group: {}",
        payload.group_id
    );

    internal_process_group_chat_message(
        app_handle,
        group_state,
        agent_state,
        db_state,
        active_requests,
        cancelled_turns,
        GroupChatParams {
            group_id: payload.group_id,
            topic_id: payload.topic_id,
            user_message: payload.user_message,
            turn_source: payload.turn_source,
            turn_id: payload.turn_id,
            vcp_url: payload.vcp_url,
            vcp_api_key: payload.vcp_api_key,
            stream_channel: Some(stream_channel),
        },
        false, // frontend already persisted the user message and compiled render blocks
    )
    .await
}

use crate::vcp_modules::db_write_queue::{DbWriteQueue, DbWriteTask};
use crate::vcp_modules::message_repository::MessageRenderCompiler;
use crate::vcp_modules::sync_dto::{
    AgentSyncDTO, AgentTopicSyncDTO, GroupSyncDTO, GroupTopicSyncDTO,
};
use crate::vcp_modules::sync_hash::HashAggregator;
use std::collections::HashSet;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::{mpsc, Semaphore};

#[derive(Debug, serde::Deserialize)]
struct TopicNDJSONFrame {
    #[serde(rename = "topicId")]
    topic_id: String,
    messages: Vec<crate::vcp_modules::sync_dto::MessagePullSyncDTO>,
    #[serde(rename = "_error")]
    error: Option<String>,
}

#[derive(Debug)]
enum EntityBatchItem {
    Agent(String, AgentSyncDTO),
    Group(String, GroupSyncDTO),
    AgentTopic(String, AgentTopicSyncDTO),
    GroupTopic(String, GroupTopicSyncDTO),
    IgnoredDefaultTopic,
}

const MAX_AVATAR_BYTES: u64 = 16 * 1024 * 1024;

async fn read_avatar_bytes_limited(response: reqwest::Response) -> Result<Vec<u8>, String> {
    use futures_util::StreamExt;

    if response
        .content_length()
        .is_some_and(|length| length > MAX_AVATAR_BYTES)
    {
        return Err("Avatar exceeds the 16MB limit".to_string());
    }

    let mut output =
        Vec::with_capacity(response.content_length().unwrap_or(0).min(MAX_AVATAR_BYTES) as usize);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        if output.len().saturating_add(chunk.len()) > MAX_AVATAR_BYTES as usize {
            return Err("Avatar exceeds the 16MB limit".to_string());
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

fn parse_entity_request_keys(
    requests: &[serde_json::Value],
) -> Result<HashSet<(String, String)>, String> {
    let mut expected = HashSet::with_capacity(requests.len());
    for request in requests {
        let id = request
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "Batch entity request contains an empty id".to_string())?;
        let entity_type = request
            .get("type")
            .and_then(serde_json::Value::as_str)
            .filter(|entity_type| {
                matches!(
                    *entity_type,
                    "agent" | "group" | "agent_topic" | "group_topic"
                )
            })
            .ok_or_else(|| format!("Unsupported batch entity request type for {id}"))?;
        if !expected.insert((id.to_string(), entity_type.to_string())) {
            return Err(format!(
                "Duplicate batch entity request: {entity_type}:{id}"
            ));
        }
    }
    Ok(expected)
}

fn parse_entity_batch_results(
    results: Vec<serde_json::Value>,
    expected: &HashSet<(String, String)>,
) -> Result<Vec<EntityBatchItem>, String> {
    let mut received = HashSet::with_capacity(results.len());
    let mut parsed = Vec::with_capacity(results.len());

    for item in results {
        let id = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "Batch entity response contains an empty id".to_string())?;
        let entity_type = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .filter(|entity_type| {
                matches!(
                    *entity_type,
                    "agent" | "group" | "agent_topic" | "group_topic"
                )
            })
            .ok_or_else(|| format!("Unsupported batch entity response type for {id}"))?;
        let key = (id.to_string(), entity_type.to_string());
        if !expected.contains(&key) {
            return Err(format!(
                "Unexpected batch entity response: {entity_type}:{id}"
            ));
        }
        if !received.insert(key) {
            return Err(format!(
                "Duplicate batch entity response: {entity_type}:{id}"
            ));
        }
        let data = item.get("data").cloned().ok_or_else(|| {
            format!("Batch entity response is missing data for {entity_type}:{id}")
        })?;

        let parsed_item = match entity_type {
            "agent" => EntityBatchItem::Agent(
                id.to_string(),
                serde_json::from_value(data)
                    .map_err(|error| format!("Invalid agent DTO for {id}: {error}"))?,
            ),
            "group" => EntityBatchItem::Group(
                id.to_string(),
                serde_json::from_value(data)
                    .map_err(|error| format!("Invalid group DTO for {id}: {error}"))?,
            ),
            "agent_topic" => {
                if id == "default" {
                    EntityBatchItem::IgnoredDefaultTopic
                } else {
                    let dto: AgentTopicSyncDTO = serde_json::from_value(data)
                        .map_err(|error| format!("Invalid agent topic DTO for {id}: {error}"))?;
                    if dto.id != id {
                        return Err(format!(
                            "Agent topic DTO id mismatch: response={id}, data={}",
                            dto.id
                        ));
                    }
                    EntityBatchItem::AgentTopic(id.to_string(), dto)
                }
            }
            "group_topic" => {
                if id == "default" {
                    EntityBatchItem::IgnoredDefaultTopic
                } else {
                    let dto: GroupTopicSyncDTO = serde_json::from_value(data)
                        .map_err(|error| format!("Invalid group topic DTO for {id}: {error}"))?;
                    if dto.id != id {
                        return Err(format!(
                            "Group topic DTO id mismatch: response={id}, data={}",
                            dto.id
                        ));
                    }
                    EntityBatchItem::GroupTopic(id.to_string(), dto)
                }
            }
            _ => unreachable!(),
        };
        parsed.push(parsed_item);
    }

    let missing: Vec<String> = expected
        .difference(&received)
        .map(|(id, entity_type)| format!("{entity_type}:{id}"))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "Batch entity response omitted requested entities: {}",
            missing.join(", ")
        ));
    }

    Ok(parsed)
}

fn parse_topic_ndjson_frame(bytes: &[u8]) -> Result<TopicNDJSONFrame, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Malformed NDJSON frame: {error}"))?;
    if let Some(error) = value.get("_stream_error") {
        return Err(format!(
            "Desktop stream error: {}",
            error.as_str().unwrap_or("invalid stream error payload")
        ));
    }
    let frame: TopicNDJSONFrame = serde_json::from_value(value)
        .map_err(|error| format!("Malformed topic NDJSON frame: {error}"))?;
    if frame.topic_id.is_empty() {
        return Err("Batch pull response contains an empty topicId".to_string());
    }
    Ok(frame)
}

fn validate_batch_pull_results(
    requests: &[(String, Vec<String>)],
    results: &[BatchPullResult],
) -> Result<(), String> {
    let mut expected = HashSet::with_capacity(requests.len());
    for (topic_id, _) in requests {
        if topic_id.is_empty() {
            return Err("Batch pull request contains an empty topic id".to_string());
        }
        if !expected.insert(topic_id.as_str()) {
            return Err(format!("Duplicate batch pull request for topic {topic_id}"));
        }
    }

    let mut received = HashSet::with_capacity(results.len());
    for result in results {
        if result.topic_id.is_empty() {
            return Err(result
                .error
                .clone()
                .unwrap_or_else(|| "Batch pull protocol error".to_string()));
        }
        if !expected.contains(result.topic_id.as_str()) {
            return Err(format!(
                "Unexpected topic in batch pull response: {}",
                result.topic_id
            ));
        }
        if !received.insert(result.topic_id.as_str()) {
            return Err(format!(
                "Duplicate topic in batch pull response: {}",
                result.topic_id
            ));
        }
    }

    let missing: Vec<&str> = expected.difference(&received).copied().collect();
    if !missing.is_empty() {
        return Err(format!(
            "Batch pull response omitted requested topics: {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

/// 共享消息处理管线：附件路径批量查询 → 填充 → 预渲染并文本压缩(通过Rayon并行化) → 写入队列
/// 被 `pull_messages_batch` 内各并发任务复用。
/// 返回 `(parsed_count, failed_count)`。
async fn process_topic_messages<R: Runtime>(
    app: &AppHandle<R>,
    topic_id: &str,
    mut parsed_messages: Vec<crate::vcp_modules::chat_manager::ChatMessage>,
    write_queue: &DbWriteQueue,
    prerender_enabled: bool,
) -> Result<(usize, usize), String> {
    let t_start = std::time::Instant::now();
    use crate::vcp_modules::db_manager::DbState;
    use sqlx::Row;
    let db = app.state::<DbState>();

    // 1. 批量收集所有附件 hash，一次性查询本地路径（替代 N+1 查询）
    let t_att_start = std::time::Instant::now();
    let mut all_hashes = Vec::new();
    for msg in &parsed_messages {
        if let Some(ref atts) = msg.attachments {
            for att in atts {
                if let Some(ref hash) = att.hash {
                    if !hash.is_empty() {
                        all_hashes.push(hash.to_string());
                    }
                }
            }
        }
    }

    let mut path_map = std::collections::HashMap::new();
    if !all_hashes.is_empty() {
        let placeholders = all_hashes
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ");
        let query = format!(
            "SELECT hash, internal_path FROM attachments WHERE hash IN ({})",
            placeholders
        );
        let mut q = sqlx::query(&query);
        for h in &all_hashes {
            q = q.bind(h);
        }
        let rows = q
            .fetch_all(&db.pool)
            .await
            .map_err(|error| format!("Failed to resolve attachment paths: {error}"))?;
        for row in rows {
            let hash = row
                .try_get::<String, _>("hash")
                .map_err(|error| format!("Invalid attachment hash row: {error}"))?;
            let path = row
                .try_get::<String, _>("internal_path")
                .map_err(|error| format!("Invalid attachment path row: {error}"))?;
            path_map.insert(hash, path);
        }
    }
    let t_att = t_att_start.elapsed();

    // 2. 用缓存的 path_map 填充附件路径与状态
    for msg in &mut parsed_messages {
        if let Some(ref mut atts) = msg.attachments {
            for att in atts {
                if let Some(ref hash) = att.hash {
                    if !hash.is_empty() {
                        if let Some(path) = path_map.get(hash) {
                            att.internal_path = path.clone();
                            att.src = format!("file://{}", path);
                        } else {
                            let default_path = format!("file://attachments/{}", hash);
                            att.internal_path =
                                default_path.trim_start_matches("file://").to_string();
                            att.src = default_path;
                        }
                    }
                }
                att.status = Some("ready".to_string());
            }
        }
    }

    let parsed_count = parsed_messages.len();
    let mut t_block = std::time::Duration::from_secs(0);
    let mut t_submit = std::time::Duration::from_secs(0);

    if !parsed_messages.is_empty() {
        // 3. 将预渲染和 Zstd 压缩等 CPU 密集型任务完美剥离至 spawn_blocking 线程池，解除 Tokio Worker 线程阻塞
        let t_block_start = std::time::Instant::now();
        let topic_id_clone = topic_id.to_string();
        let (parsed_messages_back, content_hashes, render_bytes_list, compressed_contents) =
            tokio::task::spawn_blocking(move || {
                let count = parsed_messages.len();
                let mut content_hashes = Vec::with_capacity(count);
                let mut render_bytes_list = Vec::with_capacity(count);
                let mut compressed_contents = Vec::with_capacity(count);

                for msg in &parsed_messages {
                    // A. 计算/直读指纹
                    let attachment_hashes: Vec<String> = msg
                        .attachments
                        .as_ref()
                        .map(|atts| {
                            atts.iter()
                                .map(|a| a.hash.clone().unwrap_or_default())
                                .filter(|h| !h.is_empty())
                                .collect()
                        })
                        .unwrap_or_default();

                    // ⚡ 优化：如果桌面端下发了 content_hash，则直接秒级复用，免去重算开销
                    let content_hash = match msg.content_hash {
                        Some(ref h) if !h.is_empty() => h.clone(),
                        _ => HashAggregator::compute_message_fingerprint(&msg.content, &attachment_hashes),
                    };

                    // B. 文本压缩（始终执行）+ 预渲染（按开关控制）
                    let content = &msg.content;
                    let topic_id_log = topic_id_clone.clone();
                    let msg_id_log = msg.id.clone();

                    let cc = crate::vcp_modules::persistence::message_repository::ContentCompressor::compress(content)
                        .map_err(|error| format!(
                            "Failed to compress message {} for topic {}: {}",
                            msg_id_log, topic_id_log, error
                        ))?;
                    let rb = if prerender_enabled {
                        let comp_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            let blocks = MessageRenderCompiler::compile(content);
                            MessageRenderCompiler::serialize(&blocks).unwrap_or_default()
                        }));
                        match comp_res {
                            Ok(val) => val,
                            Err(_) => {
                                log::warn!(
                                    "[PullExecutor] Compile panicked for msg {} (topic {})",
                                    msg_id_log, topic_id_log
                                );
                                Vec::new()
                            }
                        }
                    } else {
                        Vec::new()
                    };

                    content_hashes.push(content_hash);
                    render_bytes_list.push(rb);
                    compressed_contents.push(cc);
                }

                Ok::<_, String>((
                    parsed_messages,
                    content_hashes,
                    render_bytes_list,
                    compressed_contents,
                ))
            })
            .await
            .map_err(|e| format!("Spawn blocking failed: {}", e))??;
        t_block = t_block_start.elapsed();

        // 4. 提交落盘
        let t_submit_start = std::time::Instant::now();
        write_queue
            .submit(DbWriteTask::TopicMessages {
                topic_id: topic_id.to_string(),
                messages: parsed_messages_back,
                compressed_contents,
                render_bytes: render_bytes_list,
                content_hashes,
                skip_bubble: true,
            })
            .await?;
        t_submit = t_submit_start.elapsed();
    }

    let t_total = t_start.elapsed();
    if parsed_count > 0 {
        log::debug!(
            "[PullExecutor] [ProfileDetail] topic={} msgs={} | sql_att={:?} spawn_blocking={:?} submit_queue={:?} | total_proc={:?}",
            topic_id, parsed_count, t_att, t_block, t_submit, t_total
        );
    }

    Ok((parsed_count, 0))
}

/// 批量 Pull 单 topic 处理结果
#[allow(dead_code)]
pub struct BatchPullResult {
    pub topic_id: String,
    pub success: bool,
    pub parsed_count: usize,
    pub failed_count: usize,
    pub error: Option<String>,
}

pub struct PullExecutor;

impl PullExecutor {
    pub async fn pull_agent<R: Runtime>(
        _app: &AppHandle<R>,
        client: &reqwest::Client,
        http_url: &str,
        sync_token: &str,
        agent_id: &str,
        write_queue: &DbWriteQueue,
    ) -> Result<(), String> {
        let url = format!(
            "{}/api/mobile-sync/download-entity?id={}&type=agent",
            http_url, agent_id
        );
        let res = client
            .get(&url)
            .header("x-sync-token", sync_token)
            .header("Authorization", format!("Bearer {}", sync_token))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !res.status().is_success() {
            return Err(format!("Pull agent failed: {}", res.status()));
        }

        let dto: AgentSyncDTO = res.json().await.map_err(|e| e.to_string())?;
        write_queue
            .submit(DbWriteTask::Agent {
                id: agent_id.to_string(),
                dto,
            })
            .await?;

        Ok(())
    }

    pub async fn pull_group<R: Runtime>(
        _app: &AppHandle<R>,
        client: &reqwest::Client,
        http_url: &str,
        sync_token: &str,
        group_id: &str,
        write_queue: &DbWriteQueue,
    ) -> Result<(), String> {
        let url = format!(
            "{}/api/mobile-sync/download-entity?id={}&type=group",
            http_url, group_id
        );
        let res = client
            .get(&url)
            .header("x-sync-token", sync_token)
            .header("Authorization", format!("Bearer {}", sync_token))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !res.status().is_success() {
            return Err(format!("Pull group failed: {}", res.status()));
        }

        let dto: GroupSyncDTO = res.json().await.map_err(|e| e.to_string())?;
        write_queue
            .submit(DbWriteTask::Group {
                id: group_id.to_string(),
                dto,
            })
            .await?;

        Ok(())
    }

    pub async fn pull_entities_batch<R: Runtime>(
        app: &AppHandle<R>,
        client: &reqwest::Client,
        http_url: &str,
        sync_token: &str,
        requests: Vec<serde_json::Value>,
        write_queue: &DbWriteQueue,
    ) -> Result<(), String> {
        let expected = parse_entity_request_keys(&requests)?;
        let url = format!("{}/api/mobile-sync/download-entities", http_url);
        let res = client
            .post(&url)
            .header("x-sync-token", sync_token)
            .header("Authorization", format!("Bearer {}", sync_token))
            .json(&serde_json::json!({ "requests": requests }))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !res.status().is_success() {
            return Err(format!("Pull entities batch failed: {}", res.status()));
        }

        let results: Vec<serde_json::Value> = res.json().await.map_err(|e| e.to_string())?;
        log::info!(
            "[PullExecutor] Received {} entities from server",
            results.len()
        );

        let parsed_items = parse_entity_batch_results(results, &expected)?;
        let mut agent_topics = Vec::new();
        let mut group_topics = Vec::new();

        for item in parsed_items {
            match item {
                EntityBatchItem::Agent(id, dto) => {
                    write_queue.submit(DbWriteTask::Agent { id, dto }).await?;
                }
                EntityBatchItem::Group(id, dto) => {
                    write_queue.submit(DbWriteTask::Group { id, dto }).await?;
                }
                EntityBatchItem::AgentTopic(id, dto) => agent_topics.push((id, dto)),
                EntityBatchItem::GroupTopic(id, dto) => group_topics.push((id, dto)),
                EntityBatchItem::IgnoredDefaultTopic => {}
            }
        }

        if !agent_topics.is_empty() {
            log::debug!(
                "[PullExecutor] Submitting {} agent topics to write queue",
                agent_topics.len()
            );
            write_queue
                .submit(DbWriteTask::AgentTopicBatch {
                    topics: agent_topics,
                })
                .await?;
        }
        if !group_topics.is_empty() {
            log::debug!(
                "[PullExecutor] Submitting {} group topics to write queue",
                group_topics.len()
            );
            write_queue
                .submit(DbWriteTask::GroupTopicBatch {
                    topics: group_topics,
                })
                .await?;
        }

        crate::vcp_modules::sync::sync_service::emit_sync_log(
            app,
            "info",
            "[PullExecutor] Batch pull completed",
        );
        Ok(())
    }

    pub async fn pull_avatar<R: Runtime>(
        _app: &AppHandle<R>,
        client: &reqwest::Client,
        http_url: &str,
        sync_token: &str,
        owner_type: &str,
        owner_id: &str,
        write_queue: &DbWriteQueue,
    ) -> Result<(), String> {
        let url = format!(
            "{}/api/mobile-sync/download-avatar?id={}&type={}",
            http_url, owner_id, owner_type
        );

        // 指数退避重试：avatar 下载受网络波动影响较大
        let mut retries = 0;
        let max_retries = 3;
        let mut delay_ms = 200u64;
        loop {
            match client
                .get(&url)
                .header("x-sync-token", sync_token)
                .header("Authorization", format!("Bearer {}", sync_token))
                .send()
                .await
            {
                Ok(res) => {
                    if !res.status().is_success() {
                        return Err(format!("Pull avatar failed: {}", res.status()));
                    }
                    match read_avatar_bytes_limited(res).await {
                        Ok(bytes) => {
                            write_queue
                                .submit(DbWriteTask::Avatar {
                                    owner_type: owner_type.to_string(),
                                    owner_id: owner_id.to_string(),
                                    bytes,
                                })
                                .await?;
                            if retries > 0 {
                                log::info!(
                                    "[PullExecutor] Avatar {} {} succeeded after {} retries",
                                    owner_type,
                                    owner_id,
                                    retries
                                );
                            }
                            return Ok(());
                        }
                        Err(e) if retries < max_retries => {
                            retries += 1;
                            log::warn!("[PullExecutor] Avatar {} {} decode failed (retry {}/{}): {}. Waiting {}ms", owner_type, owner_id, retries, max_retries, e, delay_ms);
                            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                            delay_ms *= 2;
                        }
                        Err(e) => {
                            return Err(format!(
                                "Pull avatar decode failed after {} retries: {}",
                                max_retries, e
                            ));
                        }
                    }
                }
                Err(e) if retries < max_retries => {
                    retries += 1;
                    log::warn!("[PullExecutor] Avatar {} {} request failed (retry {}/{}): {}. Waiting {}ms", owner_type, owner_id, retries, max_retries, e, delay_ms);
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    delay_ms *= 2;
                }
                Err(e) => {
                    return Err(format!(
                        "Pull avatar request failed after {} retries: {}",
                        max_retries, e
                    ));
                }
            }
        }
    }

    pub async fn pull_agent_topic<R: Runtime>(
        _app: &AppHandle<R>,
        client: &reqwest::Client,
        http_url: &str,
        sync_token: &str,
        topic_id: &str,
        write_queue: &DbWriteQueue,
    ) -> Result<(), String> {
        let url = format!(
            "{}/api/mobile-sync/download-entity?id={}&type=agent_topic",
            http_url, topic_id
        );
        let res = client
            .get(&url)
            .header("x-sync-token", sync_token)
            .header("Authorization", format!("Bearer {}", sync_token))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if res.status() == reqwest::StatusCode::NOT_FOUND {
            // Topic not found on desktop, skip silently
            return Ok(());
        }

        if !res.status().is_success() {
            return Err(format!("Pull agent_topic failed: {}", res.status()));
        }

        let dto: AgentTopicSyncDTO = res.json().await.map_err(|e| e.to_string())?;
        write_queue
            .submit(DbWriteTask::AgentTopic {
                topic_id: topic_id.to_string(),
                dto,
            })
            .await?;

        Ok(())
    }

    pub async fn pull_group_topic<R: Runtime>(
        _app: &AppHandle<R>,
        client: &reqwest::Client,
        http_url: &str,
        sync_token: &str,
        topic_id: &str,
        write_queue: &DbWriteQueue,
    ) -> Result<(), String> {
        let url = format!(
            "{}/api/mobile-sync/download-entity?id={}&type=group_topic",
            http_url, topic_id
        );
        let res = client
            .get(&url)
            .header("x-sync-token", sync_token)
            .header("Authorization", format!("Bearer {}", sync_token))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if res.status() == reqwest::StatusCode::NOT_FOUND {
            // Topic not found on desktop, skip silently
            return Ok(());
        }

        if !res.status().is_success() {
            return Err(format!("Pull group_topic failed: {}", res.status()));
        }

        let dto: GroupTopicSyncDTO = res.json().await.map_err(|e| e.to_string())?;
        write_queue
            .submit(DbWriteTask::GroupTopic {
                topic_id: topic_id.to_string(),
                dto,
            })
            .await?;

        Ok(())
    }

    /// 流式批量 Pull — 一次 HTTP 请求拉取多个 topic 的消息
    ///
    /// 桌面端以 NDJSON 逐 topic 分帧返回，手机端逐行消费，
    /// 不等待整个响应结束。单 topic 失败不中断流。
    ///
    /// **并发控制**: Semaphore(20) + tokio spawn 并行处理 topic 消息，
    /// mpsc channel 实时推送进度日志。NDJSON 解析与并发处理完全分离。
    ///
    /// 返回每个 topic 的处理结果。
    pub async fn pull_messages_batch<R: Runtime>(
        app: &AppHandle<R>,
        client: &reqwest::Client,
        http_url: &str,
        sync_token: &str,
        requests: &[(String, Vec<String>)], // (topic_id, msg_ids), 空 vec = 拉全部消息
        write_queue: &DbWriteQueue,
        prerender_enabled: bool,
    ) -> Result<Vec<BatchPullResult>, String> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }

        let url = format!("{}/api/mobile-sync/download-messages-stream", http_url);
        let req_body: Vec<serde_json::Value> = requests
            .iter()
            .map(|(tid, ids)| serde_json::json!({ "topicId": tid, "msgIds": ids }))
            .collect();

        let res = client
            .post(&url)
            .header("x-sync-token", sync_token)
            .header("Authorization", format!("Bearer {}", sync_token))
            .json(&serde_json::json!({ "requests": req_body }))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !res.status().is_success() {
            let status = res.status();
            let err_body = res.text().await.unwrap_or_default();
            return Err(format!(
                "Batch pull messages failed: HTTP {} body={}",
                status, err_body
            ));
        }

        // ── 并发基础设施 ──
        let sem = Arc::new(Semaphore::new(20));
        let (tx, mut rx) = mpsc::unbounded_channel::<BatchPullResult>();
        let mut spawn_handles = Vec::new();
        let total = requests.len();

        // 启动接收协程：实时消费 channel 输出进度日志
        let app_receiver = app.clone();
        let receiver_handle = tokio::spawn(async move {
            let mut results = Vec::new();
            let mut completed = 0usize;
            while let Some(result) = rx.recv().await {
                completed += 1;
                if result.success {
                    let msg = format!(
                        "[PullExecutor] Batch pull: topic {} completed ({}/{})",
                        result.topic_id, completed, total
                    );
                    crate::vcp_modules::sync::sync_service::emit_sync_log(
                        &app_receiver,
                        "info",
                        &msg,
                    );
                } else {
                    let err = result.error.as_deref().unwrap_or("unknown");
                    let msg = format!(
                        "[PullExecutor] Batch pull: topic {} FAILED ({}/{}): {}",
                        result.topic_id, completed, total, err
                    );
                    crate::vcp_modules::sync::sync_service::emit_sync_log(
                        &app_receiver,
                        "error",
                        &msg,
                    );
                }
                results.push(result);
            }
            results
        });

        // ── NDJSON 解析协程 ──
        use futures_util::StreamExt;
        let mut stream = res.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();
        let mut search_start = 0; // 核心优化：新增扫描游标，避免 O(N^2) 重复扫描

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| format!("Stream read error: {}", e))?;

            buffer.extend_from_slice(&chunk);

            // 逐行解析 NDJSON（优化为从游标处开始扫描，实现 O(N) 性能）
            while let Some(pos) = buffer[search_start..].iter().position(|&b| b == b'\n') {
                let line_end = search_start + pos;
                let line = buffer.drain(..=line_end).collect::<Vec<_>>();
                search_start = 0; // 成功切分一行后，后续扫描从头开始（因为 buffer 已被 drain）

                if line.len() <= 1 {
                    continue;
                }

                // ⚡ 异步重构：流主协程不等待解析，立即把 line 抛进后台多核协程
                let app_clone = app.clone();
                let sem_clone = sem.clone();
                let wq_clone = write_queue.clone();
                let tx_clone = tx.clone();

                let handle = tokio::spawn(async move {
                    let start_t = std::time::Instant::now();
                    // 1. 在后台多核并发解码标准 DTO JSON
                    let frame = match parse_topic_ndjson_frame(&line) {
                        Ok(f) => f,
                        Err(e) => {
                            let err_msg = format!("[PullExecutor] {}", e);
                            crate::vcp_modules::sync::sync_service::emit_sync_log(
                                &app_clone, "error", &err_msg,
                            );
                            let _ = tx_clone.send(BatchPullResult {
                                topic_id: String::new(),
                                success: false,
                                parsed_count: 0,
                                failed_count: 0,
                                error: Some(e),
                            });
                            return;
                        }
                    };

                    let topic_id = frame.topic_id;

                    // 2. 检查单 topic 错误帧
                    if let Some(topic_err) = frame.error {
                        let _ = tx_clone.send(BatchPullResult {
                            topic_id,
                            success: false,
                            parsed_count: 0,
                            failed_count: 0,
                            error: Some(format!("Desktop error: {}", topic_err)),
                        });
                        return;
                    }

                    let pull_dtos = frame.messages;
                    if pull_dtos.is_empty() {
                        let _ = tx_clone.send(BatchPullResult {
                            topic_id,
                            success: true,
                            parsed_count: 0,
                            failed_count: 0,
                            error: None,
                        });
                        return;
                    }

                    // ⚡ 核心转换：通过 DTO From 实现三层完全隔离，净化核心 ChatMessage
                    let messages: Vec<crate::vcp_modules::chat_manager::ChatMessage> = pull_dtos
                        .into_iter()
                        .map(crate::vcp_modules::chat_manager::ChatMessage::from)
                        .collect();

                    let decode_t = start_t.elapsed();

                    // 3. 抢占信号量，控制写入并发度
                    let sem_start = std::time::Instant::now();
                    match sem_clone.acquire_owned().await {
                        Ok(permit) => {
                            let sem_t = sem_start.elapsed();
                            let _permit = permit; // 持有 permit 直到任务完成
                            let proc_start = std::time::Instant::now();
                            match process_topic_messages(
                                &app_clone,
                                &topic_id,
                                messages,
                                &wq_clone,
                                prerender_enabled,
                            )
                            .await
                            {
                                Ok((parsed, failed)) => {
                                    let proc_t = proc_start.elapsed();
                                    let total_t = start_t.elapsed();
                                    log::debug!(
                                        "[PullExecutor] [ProfileSummary] topic={} msgs={} | decode={:?} sem_wait={:?} process={:?} | total={:?}",
                                        topic_id, parsed, decode_t, sem_t, proc_t, total_t
                                    );
                                    let _ = tx_clone.send(BatchPullResult {
                                        topic_id,
                                        success: true,
                                        parsed_count: parsed,
                                        failed_count: failed,
                                        error: None,
                                    });
                                }
                                Err(e) => {
                                    let _ = tx_clone.send(BatchPullResult {
                                        topic_id,
                                        success: false,
                                        parsed_count: 0,
                                        failed_count: 0,
                                        error: Some(e),
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx_clone.send(BatchPullResult {
                                topic_id,
                                success: false,
                                parsed_count: 0,
                                failed_count: 0,
                                error: Some(e.to_string()),
                            });
                        }
                    }
                });
                spawn_handles.push(handle);
            }

            // 循环结束后，游标指向 buffer 末尾，下一轮 chunk 进来时只需扫描新增部分
            search_start = buffer.len();
        }

        // 处理流结束后 buffer 中残留的非换行数据（兜底）
        if !buffer.is_empty() {
            match parse_topic_ndjson_frame(&buffer) {
                Ok(frame) => {
                    let topic_id = frame.topic_id;
                    if let Some(topic_err) = frame.error {
                        let _ = tx.send(BatchPullResult {
                            topic_id,
                            success: false,
                            parsed_count: 0,
                            failed_count: 0,
                            error: Some(format!("Desktop error: {}", topic_err)),
                        });
                    } else {
                        let pull_dtos = frame.messages;
                        if !pull_dtos.is_empty() {
                            let app_clone = app.clone();
                            let sem_clone = sem.clone();
                            let wq_clone = write_queue.clone();
                            let tx_clone = tx.clone();
                            let handle = tokio::spawn(async move {
                                match sem_clone.acquire_owned().await {
                                    Ok(permit) => {
                                        let _permit = permit;
                                        // 转换 DTO 到 ChatMessage 核心实体
                                        let messages: Vec<
                                            crate::vcp_modules::chat_manager::ChatMessage,
                                        > = pull_dtos
                                            .into_iter()
                                            .map(
                                                crate::vcp_modules::chat_manager::ChatMessage::from,
                                            )
                                            .collect();
                                        match process_topic_messages(
                                            &app_clone,
                                            &topic_id,
                                            messages,
                                            &wq_clone,
                                            prerender_enabled,
                                        )
                                        .await
                                        {
                                            Ok((parsed, failed)) => {
                                                let _ = tx_clone.send(BatchPullResult {
                                                    topic_id,
                                                    success: true,
                                                    parsed_count: parsed,
                                                    failed_count: failed,
                                                    error: None,
                                                });
                                            }
                                            Err(e) => {
                                                let _ = tx_clone.send(BatchPullResult {
                                                    topic_id,
                                                    success: false,
                                                    parsed_count: 0,
                                                    failed_count: 0,
                                                    error: Some(e),
                                                });
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx_clone.send(BatchPullResult {
                                            topic_id,
                                            success: false,
                                            parsed_count: 0,
                                            failed_count: 0,
                                            error: Some(e.to_string()),
                                        });
                                    }
                                }
                            });
                            spawn_handles.push(handle);
                        } else {
                            let _ = tx.send(BatchPullResult {
                                topic_id,
                                success: true,
                                parsed_count: 0,
                                failed_count: 0,
                                error: None,
                            });
                        }
                    }
                }
                Err(error) => {
                    let _ = tx.send(BatchPullResult {
                        topic_id: String::new(),
                        success: false,
                        parsed_count: 0,
                        failed_count: 0,
                        error: Some(error),
                    });
                }
            }
        }

        // ── 等待所有任务完成 ──
        drop(tx); // 关闭 channel，通知 receiver 不再有新消息
        let task_results = futures_util::future::join_all(spawn_handles).await;
        if let Some(error) = task_results.into_iter().find_map(Result::err) {
            return Err(format!("Batch pull worker failed: {error}"));
        }
        let results = receiver_handle
            .await
            .map_err(|error| format!("Batch pull result collector failed: {error}"))?;
        validate_batch_pull_results(requests, &results)?;

        let ok_count = results.iter().filter(|r| r.success).count();
        let err_count = results.iter().filter(|r| !r.success).count();
        let msg = format!(
            "[PullExecutor] Batch pull completed: {}/{} topics processed, {} errors",
            ok_count, total, err_count
        );
        crate::vcp_modules::sync::sync_service::emit_sync_log(app, "info", &msg);
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn successful_result(topic_id: &str) -> BatchPullResult {
        BatchPullResult {
            topic_id: topic_id.to_string(),
            success: true,
            parsed_count: 0,
            failed_count: 0,
            error: None,
        }
    }

    #[test]
    fn entity_batch_rejects_missing_and_malformed_responses_before_writes() {
        let requests = vec![
            json!({ "id": "agent-1", "type": "agent" }),
            json!({ "id": "group-1", "type": "group" }),
        ];
        let expected = parse_entity_request_keys(&requests).unwrap();

        let missing = parse_entity_batch_results(
            vec![json!({
                "id": "agent-1",
                "type": "agent",
                "data": {
                    "name": "Agent",
                    "systemPrompt": "",
                    "model": "model",
                    "temperature": 0.7,
                    "contextTokenLimit": 4096,
                    "maxOutputTokens": 1024,
                    "streamOutput": true
                }
            })],
            &expected,
        )
        .unwrap_err();
        assert!(missing.contains("group:group-1"));

        let malformed = parse_entity_batch_results(
            vec![
                json!({ "id": "agent-1", "type": "agent", "data": {} }),
                json!({ "id": "group-1", "type": "group", "data": {} }),
            ],
            &expected,
        )
        .unwrap_err();
        assert!(malformed.contains("Invalid agent DTO"));
    }

    #[test]
    fn entity_batch_rejects_topic_id_mismatch() {
        let requests = vec![json!({ "id": "topic-1", "type": "agent_topic" })];
        let expected = parse_entity_request_keys(&requests).unwrap();
        let error = parse_entity_batch_results(
            vec![json!({
                "id": "topic-1",
                "type": "agent_topic",
                "data": {
                    "id": "topic-other",
                    "name": "Topic",
                    "createdAt": 1,
                    "locked": true,
                    "unread": false,
                    "ownerId": "agent-1"
                }
            })],
            &expected,
        )
        .unwrap_err();
        assert!(error.contains("DTO id mismatch"));
    }

    #[test]
    fn ndjson_parser_recognizes_stream_errors_after_reassembly() {
        let mut reassembled = br#"{"_stream_error":"desktop failed"}"#.to_vec();
        reassembled.push(b'\n');
        let error = parse_topic_ndjson_frame(&reassembled).unwrap_err();
        assert_eq!(error, "Desktop stream error: desktop failed");
    }

    #[test]
    fn batch_pull_validation_rejects_missing_duplicate_and_protocol_results() {
        let requests = vec![
            ("topic-1".to_string(), Vec::new()),
            ("topic-2".to_string(), Vec::new()),
        ];

        let missing = vec![successful_result("topic-1")];
        assert!(validate_batch_pull_results(&requests, &missing)
            .unwrap_err()
            .contains("topic-2"));

        let duplicate = vec![
            successful_result("topic-1"),
            successful_result("topic-1"),
            successful_result("topic-2"),
        ];
        assert!(validate_batch_pull_results(&requests, &duplicate)
            .unwrap_err()
            .contains("Duplicate topic"));

        let protocol = vec![
            successful_result("topic-1"),
            BatchPullResult {
                topic_id: String::new(),
                success: false,
                parsed_count: 0,
                failed_count: 0,
                error: Some("Malformed NDJSON frame".to_string()),
            },
        ];
        assert_eq!(
            validate_batch_pull_results(&requests, &protocol).unwrap_err(),
            "Malformed NDJSON frame"
        );
    }
}

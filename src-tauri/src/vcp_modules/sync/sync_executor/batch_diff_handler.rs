use crate::vcp_modules::db_write_queue::DbWriteQueue;
use crate::vcp_modules::sync_executor::{BatchPullResult, PullExecutor, PushExecutor};
use crate::vcp_modules::sync_logger::{LogLevel, SyncLogger};
use crate::vcp_modules::sync_service::{
    emit_sync_log, fail_sync_session, Phase3Tracker, SyncCommand,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;

pub struct BatchDiffHandler;

type PendingDiffBatches =
    Arc<tokio::sync::Mutex<std::collections::VecDeque<serde_json::Map<String, serde_json::Value>>>>;

async fn send_next_diff_batch(
    pending_diff_batches: &PendingDiffBatches,
    tx_internal: &mpsc::UnboundedSender<SyncCommand>,
) -> Result<(), String> {
    let mut pending = pending_diff_batches.lock().await;
    if let Some(next_batch) = pending.pop_front() {
        log::debug!(
            "[SyncService] Sending next diff batch, {} remaining",
            pending.len()
        );
        tx_internal
            .send(SyncCommand::SendWsMessage(json!({
                "type": "SYNC_MESSAGE_DIFF_BATCH",
                "topics": next_batch,
            })))
            .map_err(|error| format!("Failed to queue next message diff batch: {error}"))?;
    }
    Ok(())
}

fn parse_topic_actions(topic_id: &str, result: &Value) -> Result<(bool, Vec<String>), String> {
    if topic_id.is_empty() || !result.is_object() {
        return Err("Malformed topic entry in SYNC_DIFF_RESULTS_BATCH".to_string());
    }
    let to_pull_ids = match result.get("toPull") {
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("Invalid toPull id for topic {topic_id}"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(Value::Null) | None => Vec::new(),
        Some(_) => return Err(format!("toPull must be an array for topic {topic_id}")),
    };
    let to_push = match result.get("toPush") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => false,
        Some(_) => return Err(format!("toPush must be a boolean for topic {topic_id}")),
    };
    Ok((to_push, to_pull_ids))
}

impl BatchDiffHandler {
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_diff_batch(
        app_handle: &AppHandle,
        payload: &Value,
        http_client: &reqwest::Client,
        base_url: &str,
        token: &str,
        tracker: &Arc<Phase3Tracker>,
        tx_internal: &mpsc::UnboundedSender<SyncCommand>,
        logger: &Arc<Mutex<SyncLogger>>,
        write_queue: &Arc<DbWriteQueue>,
        pending_diff_batches: &PendingDiffBatches,
        prerender_enabled: bool,
    ) -> Result<(), String> {
        let results = payload["results"]
            .as_object()
            .ok_or_else(|| "SYNC_DIFF_RESULTS_BATCH.results must be an object".to_string())?;
        // 分类 topics: push_only, push_pull, pull_only
        let mut push_topic_ids: Vec<String> = Vec::new();
        let mut pull_batch: Vec<(String, Vec<String>)> = Vec::new();

        for (topic_id, result) in results {
            let (to_push, to_pull_ids) = parse_topic_actions(topic_id, result)?;

            if !to_push && to_pull_ids.is_empty() {
                // 无需操作，直接标记完成
                tracker
                    .mark_completed(topic_id, logger, tx_internal, app_handle, true)
                    .await?;
                continue;
            }

            if to_push {
                push_topic_ids.push(topic_id.clone());
            }
            if !to_pull_ids.is_empty() {
                pull_batch.push((topic_id.clone(), to_pull_ids));
            }
        }

        let has_push = !push_topic_ids.is_empty();
        let has_pull = !pull_batch.is_empty();

        if has_push || has_pull {
            let h_in = app_handle.clone();
            let c_in = http_client.clone();
            let b_in = base_url.to_string();
            let token = token.to_string();
            let tracker_clone = tracker.clone();
            let tx_internal_msg = tx_internal.clone();
            let sync_logger_msg = logger.clone();
            let wq_in = write_queue.clone();
            let pending_batches = pending_diff_batches.clone();

            let sync_state =
                app_handle.state::<crate::vcp_modules::sync::sync_service::SyncState>();
            let uploaded_hashes = sync_state.uploaded_hashes.clone();

            // 收集所有涉及的 topic ID（去重）
            let mut all_topic_ids: HashSet<String> = HashSet::new();
            for tid in &push_topic_ids {
                all_topic_ids.insert(tid.clone());
            }
            for (tid, _) in &pull_batch {
                all_topic_ids.insert(tid.clone());
            }

            tauri::async_runtime::spawn(async move {
                let mut failed = false;
                // 1. Push 批量（先执行，确保 push_pull 的 topic 推送完再拉取）
                if has_push {
                    match PushExecutor::push_messages_batch(
                        &h_in,
                        &c_in,
                        &b_in,
                        &token,
                        &push_topic_ids,
                        uploaded_hashes.clone(),
                    )
                    .await
                    {
                        Ok(results) => {
                            for r in &results {
                                if r.success {
                                    tracker_clone.mark_modified(&r.topic_id).await;
                                } else {
                                    let err = r.error.as_deref().unwrap_or("unknown");
                                    if let Ok(mut l) = sync_logger_msg.lock() {
                                        l.log_operation(
                                            "messages",
                                            "topic",
                                            &r.topic_id,
                                            false,
                                            Some(err),
                                        );
                                    }
                                    emit_sync_log(
                                        &h_in,
                                        "error",
                                        &format!("Push failed for {}: {}", r.topic_id, err),
                                    );
                                    failed = true;
                                }
                            }
                        }
                        Err(e) => {
                            let err_msg = format!("Batch push messages failed: {}", e);
                            if let Ok(mut l) = sync_logger_msg.lock() {
                                l.log(LogLevel::Error, "messages", &err_msg);
                            }
                            emit_sync_log(&h_in, "error", &err_msg);
                            failed = true;
                        }
                    }
                }

                if failed {
                    let message = "Message batch push failed; sync was cancelled";
                    fail_sync_session(&h_in, message).await;
                    let _ = tx_internal_msg.send(SyncCommand::Cancel);
                    return;
                }

                // 2. Pull 批量（push 完成后再 pull，确保 push_pull 的 topic 数据已合并）
                if has_pull {
                    match PullExecutor::pull_messages_batch(
                        &h_in,
                        &c_in,
                        &b_in,
                        &token,
                        &pull_batch,
                        &wq_in,
                        prerender_enabled,
                    )
                    .await
                    {
                        Ok(results) => {
                            let result_map: std::collections::HashMap<&str, &BatchPullResult> =
                                results.iter().map(|r| (r.topic_id.as_str(), r)).collect();
                            for (tid, _) in &pull_batch {
                                if let Some(r) = result_map.get(tid.as_str()) {
                                    if r.success {
                                        tracker_clone.mark_modified(tid).await;
                                    } else {
                                        let err = r.error.as_deref().unwrap_or("unknown");
                                        if let Ok(mut l) = sync_logger_msg.lock() {
                                            l.log_operation(
                                                "messages",
                                                "topic",
                                                tid,
                                                false,
                                                Some(err),
                                            );
                                        }
                                        emit_sync_log(
                                            &h_in,
                                            "error",
                                            &format!("Pull failed for {}: {}", tid, err),
                                        );
                                        failed = true;
                                    }
                                } else {
                                    if let Ok(mut l) = sync_logger_msg.lock() {
                                        l.log_operation(
                                            "messages",
                                            "topic",
                                            tid,
                                            false,
                                            Some("not in batch response"),
                                        );
                                    }
                                    emit_sync_log(
                                        &h_in,
                                        "error",
                                        &format!("Pull result missing for {}", tid),
                                    );
                                    failed = true;
                                }
                            }
                        }
                        Err(e) => {
                            let err_msg = format!("Batch pull messages failed: {}", e);
                            if let Ok(mut l) = sync_logger_msg.lock() {
                                l.log(LogLevel::Error, "messages", &err_msg);
                            }
                            emit_sync_log(&h_in, "error", &err_msg);
                            failed = true;
                        }
                    }
                }

                if failed {
                    let message = "Message batch pull failed; sync was cancelled";
                    fail_sync_session(&h_in, message).await;
                    let _ = tx_internal_msg.send(SyncCommand::Cancel);
                    return;
                }

                // 3. 所有 topic 标记完成
                for tid in &all_topic_ids {
                    if let Err(error) = tracker_clone
                        .mark_completed(tid, &sync_logger_msg, &tx_internal_msg, &h_in, false)
                        .await
                    {
                        log::error!("[SyncService] {}", error);
                        fail_sync_session(&h_in, &error).await;
                        let _ = tx_internal_msg.send(SyncCommand::Cancel);
                        return;
                    }
                }

                log::info!(
                    "[SyncService] Phase 3 batch done: push={} pull={}",
                    push_topic_ids.len(),
                    pull_batch.len()
                );

                if let Err(error) = send_next_diff_batch(&pending_batches, &tx_internal_msg).await {
                    log::error!("[SyncService] {}", error);
                    fail_sync_session(&h_in, &error).await;
                    let _ = tx_internal_msg.send(SyncCommand::Cancel);
                }
            });
        } else {
            send_next_diff_batch(pending_diff_batches, tx_internal).await?;
        }

        // 当前批次处理完毕，发送下一批（如果还有）
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_batch_topic_actions() {
        let wrong_pull_shape = json!({ "toPull": "message-1", "toPush": false });
        assert!(parse_topic_actions("topic-1", &wrong_pull_shape).is_err());

        let wrong_push_shape = json!({ "toPull": [], "toPush": "yes" });
        assert!(parse_topic_actions("topic-1", &wrong_push_shape).is_err());

        let non_string_message_id = json!({ "toPull": [42], "toPush": false });
        assert!(parse_topic_actions("topic-1", &non_string_message_id).is_err());
    }

    #[test]
    fn accepts_missing_optional_batch_topic_actions() {
        assert_eq!(
            parse_topic_actions("topic-1", &json!({})).unwrap(),
            (false, Vec::<String>::new())
        );
    }
}

use crate::vcp_modules::infra::lifecycle_state::LifecycleState;
use crate::vcp_modules::settings_manager::{read_settings, Settings, SettingsState};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub fn is_app_in_foreground<R: tauri::Runtime>(app: &AppHandle<R>) -> bool {
    if let Some(state) = app.try_state::<LifecycleState>() {
        state.is_foreground.load(Ordering::SeqCst)
    } else {
        true
    }
}

fn transition_matches(state: &LifecycleState, epoch: u64, is_foreground: bool) -> bool {
    state.transition_epoch.load(Ordering::SeqCst) == epoch
        && state.is_foreground.load(Ordering::SeqCst) == is_foreground
}

fn app_transition_matches<R: tauri::Runtime>(
    app: &AppHandle<R>,
    epoch: u64,
    is_foreground: bool,
) -> bool {
    app.try_state::<LifecycleState>()
        .is_some_and(|state| transition_matches(&state, epoch, is_foreground))
}

#[tauri::command]
pub async fn set_app_foreground_state(app: AppHandle, is_foreground: bool) {
    set_app_foreground_state_internal(app, is_foreground).await;
}

pub async fn set_app_foreground_state_internal(app: AppHandle, is_foreground: bool) {
    let state = match app.try_state::<LifecycleState>() {
        Some(s) => s,
        None => {
            log::warn!("[Lifecycle] LifecycleState not registered, skipping foreground transition");
            return;
        }
    };

    let was_foreground = state.is_foreground.swap(is_foreground, Ordering::SeqCst);
    if was_foreground == is_foreground {
        return;
    }
    let transition_epoch = state.transition_epoch.fetch_add(1, Ordering::SeqCst) + 1;
    log::info!(
        "[Lifecycle] App foreground state transitioned: {} -> {}",
        was_foreground,
        is_foreground
    );

    // 1. 调整心跳频率
    crate::vcp_modules::infra::vcp_log_service::handle_foreground_state_change(&app, is_foreground)
        .await;
    if !transition_matches(&state, transition_epoch, is_foreground) {
        return;
    }

    // 向前端广播最新的前台状态（Tauri 官方单通道）
    let _ = app.emit(
        "vcp-lifecycle-changed",
        serde_json::json!({
            "state": if is_foreground { "resume" } else { "pause" }
        }),
    );

    if !is_foreground {
        // --- 进入后台 ---
        // 1.1 取消旧倒计时任务
        {
            let mut cancel_lock = state.linger.log_cancel.lock().await;
            if let Some(token) = cancel_lock.take() {
                token.cancel();
            }
        }
        {
            let mut cancel_lock = state.linger.dist_cancel.lock().await;
            if let Some(token) = cancel_lock.take() {
                token.cancel();
            }
        }
        state
            .linger
            .is_log_disconnected
            .store(false, Ordering::SeqCst);
        state
            .linger
            .is_dist_disconnected
            .store(false, Ordering::SeqCst);

        if !transition_matches(&state, transition_epoch, false) {
            return;
        }

        let settings_state = app.state::<SettingsState>();
        let settings = match read_settings(app.clone(), settings_state).await {
            Ok(settings) => Some(settings),
            Err(error) => {
                log::error!(
                    "[Lifecycle] Failed to read settings on background transition: {error}"
                );
                None
            }
        };
        if !transition_matches(&state, transition_epoch, false) {
            return;
        }
        let log_configured = settings.as_ref().is_some_and(has_log_configuration);
        let log_status =
            crate::vcp_modules::infra::vcp_log_service::get_vcp_log_status_internal().await;
        let info_status =
            crate::vcp_modules::infra::vcp_info_service::get_vcp_info_connection_status()
                .await
                .unwrap_or_else(|_| "closed".to_string());
        if !transition_matches(&state, transition_epoch, false) {
            return;
        }
        let should_linger_log = log_configured || log_status != "closed" || info_status != "closed";

        // 1.1b 申请持有 vcp_log 对应的原生进程级前台锁，以保证 10 分钟内后台存活
        if should_linger_log {
            let _ = tauri_plugin_vcp_mobile::stream::acquire_foreground_inner(
                &app,
                "vcp_log",
                10,
                "VCP Log Linger",
                false,
            );
            if !transition_matches(&state, transition_epoch, false) {
                let _ = tauri_plugin_vcp_mobile::stream::release_foreground_inner(&app, "vcp_log");
                return;
            }
        }

        // 1.2 开启 VCPLog/Info (10分钟延迟断连任务)
        if should_linger_log {
            let log_token = tokio_util::sync::CancellationToken::new();
            {
                let mut cancel_lock = state.linger.log_cancel.lock().await;
                if !transition_matches(&state, transition_epoch, false) {
                    log_token.cancel();
                    let _ =
                        tauri_plugin_vcp_mobile::stream::release_foreground_inner(&app, "vcp_log");
                    return;
                }
                *cancel_lock = Some(log_token.clone());
            }
            let app_clone = app.clone();
            crate::vcp_modules::infra::utils::spawn_linger_task(
                Duration::from_secs(600),
                log_token,
                move || async move {
                    if !app_transition_matches(&app_clone, transition_epoch, false) {
                        return;
                    }
                    log::info!(
                        "[Lifecycle] Background linger expired (10m). Disconnecting VCPLog/Info."
                    );
                    if let Err(error) =
                        crate::vcp_modules::infra::vcp_log_service::disconnect_log_connections(
                            &app_clone,
                        )
                        .await
                    {
                        log::error!("[Lifecycle] Failed to disconnect VCPLog/Info: {error}");
                    }
                    if let Some(s) = app_clone.try_state::<LifecycleState>() {
                        s.linger.is_log_disconnected.store(true, Ordering::SeqCst);
                        if is_app_in_foreground(&app_clone) {
                            let current_epoch = s.transition_epoch.load(Ordering::SeqCst);
                            restore_log_connections(
                                &app_clone,
                                &s.linger.is_log_disconnected,
                                current_epoch,
                            )
                            .await;
                        } else if transition_matches(&s, transition_epoch, false) {
                            // 10 分钟到期，释放 vcp_log 的前台锁。
                            let _ = tauri_plugin_vcp_mobile::stream::release_foreground_inner(
                                &app_clone, "vcp_log",
                            );
                        }
                    }
                },
            );
        }

        if !transition_matches(&state, transition_epoch, false) {
            return;
        }

        // 1.3 开启 Distributed (5分钟保活冷却任务)
        let distributed_is_running =
            if let Some(dist_state) = app.try_state::<crate::distributed::DistributedState>() {
                let client = dist_state.client.read().await;
                client.is_running().await
            } else {
                false
            };
        if settings
            .as_ref()
            .is_some_and(|settings| settings.distributed_enabled)
            || distributed_is_running
        {
            log::info!("[Lifecycle] Distributed enabled. Active FGS lock is already managed by distributed client.");

            let dist_token = tokio_util::sync::CancellationToken::new();
            {
                let mut cancel_lock = state.linger.dist_cancel.lock().await;
                if !transition_matches(&state, transition_epoch, false) {
                    dist_token.cancel();
                    return;
                }
                *cancel_lock = Some(dist_token.clone());
            }
            let app_clone = app.clone();
            crate::vcp_modules::infra::utils::spawn_linger_task(
                Duration::from_secs(300),
                dist_token,
                move || async move {
                    if !app_transition_matches(&app_clone, transition_epoch, false) {
                        return;
                    }
                    log::info!("[Lifecycle] Background distributed linger expired (5m). Stopping distributed client cleanly.");
                    if let Some(dist_state) =
                        app_clone.try_state::<crate::distributed::DistributedState>()
                    {
                        let client = dist_state.client.read().await;
                        client.stop(&app_clone).await;
                    }
                    if let Some(s) = app_clone.try_state::<LifecycleState>() {
                        s.linger.is_dist_disconnected.store(true, Ordering::SeqCst);
                        if is_app_in_foreground(&app_clone) {
                            let current_epoch = s.transition_epoch.load(Ordering::SeqCst);
                            restore_distributed_connection(
                                &app_clone,
                                &s.linger.is_dist_disconnected,
                                current_epoch,
                            )
                            .await;
                        }
                    }
                },
            );
        }
    } else {
        // --- 返回前台 ---
        // 2.1 立即取消所有倒计时并释放 vcp_log 锁
        {
            let mut cancel_lock = state.linger.log_cancel.lock().await;
            if let Some(token) = cancel_lock.take() {
                token.cancel();
            }
        }
        {
            let mut cancel_lock = state.linger.dist_cancel.lock().await;
            if let Some(token) = cancel_lock.take() {
                token.cancel();
            }
        }
        let _ = tauri_plugin_vcp_mobile::stream::release_foreground_inner(&app, "vcp_log");
        if !transition_matches(&state, transition_epoch, true) {
            return;
        }

        // 2.2 恢复分布式保活状态 (前台关闭保活通知 - 如果仍然运行的话)
        // 2.3 若此前已冷断开，一键拉起恢复
        let was_log_disconnected = state.linger.is_log_disconnected.load(Ordering::SeqCst);
        let was_dist_disconnected = state.linger.is_dist_disconnected.load(Ordering::SeqCst);

        let settings_state = app.state::<SettingsState>();
        match read_settings(app.clone(), settings_state).await {
            Ok(settings) => {
                if !transition_matches(&state, transition_epoch, true) {
                    return;
                }
                if settings.distributed_enabled {
                    if let Err(error) =
                        tauri_plugin_vcp_mobile::stream::set_keepalive_mode_inner(&app, false)
                    {
                        log::error!(
                            "[Lifecycle] Failed to disable foreground keepalive mode: {error}"
                        );
                    }
                }
                if was_log_disconnected {
                    restore_log_connections_with_settings(
                        &app,
                        &state.linger.is_log_disconnected,
                        &settings,
                        transition_epoch,
                    )
                    .await;
                } else {
                    // 如果连接没有断开，则冲刷后台缓存的日志消息到前端 WebView
                    crate::vcp_modules::infra::vcp_log_service::flush_background_logs(&app);
                }

                if !transition_matches(&state, transition_epoch, true) {
                    return;
                }
                if was_dist_disconnected {
                    restore_distributed_connection_with_settings(
                        &app,
                        &state.linger.is_dist_disconnected,
                        &settings,
                        transition_epoch,
                    )
                    .await;
                }
            }
            Err(error) => {
                log::error!(
                    "[Lifecycle] Failed to read settings on foreground transition; reconnect flags retained: {error}"
                );
            }
        }
    }
}

fn has_log_configuration(settings: &Settings) -> bool {
    !settings.vcp_log_url.trim().is_empty() && !settings.vcp_log_key.trim().is_empty()
}

fn finish_restore_attempt(
    retry_flag: &AtomicBool,
    service: &str,
    result: Result<(), String>,
) -> bool {
    match result {
        Ok(()) => {
            retry_flag.store(false, Ordering::SeqCst);
            true
        }
        Err(error) => {
            log::error!("[Lifecycle] Failed to restore {service}; retry flag retained: {error}");
            false
        }
    }
}

async fn restore_log_connections(app: &AppHandle, retry_flag: &AtomicBool, epoch: u64) {
    if !app_transition_matches(app, epoch, true) {
        return;
    }
    let settings_state = app.state::<SettingsState>();
    match read_settings(app.clone(), settings_state).await {
        Ok(settings) => {
            restore_log_connections_with_settings(app, retry_flag, &settings, epoch).await;
        }
        Err(error) => {
            log::error!(
                "[Lifecycle] Failed to read settings while restoring VCPLog/Info; retry flag retained: {error}"
            );
        }
    }
}

async fn restore_log_connections_with_settings(
    app: &AppHandle,
    retry_flag: &AtomicBool,
    settings: &Settings,
    epoch: u64,
) {
    if !app_transition_matches(app, epoch, true) {
        return;
    }
    if !has_log_configuration(settings) {
        retry_flag.store(false, Ordering::SeqCst);
        crate::vcp_modules::infra::vcp_log_service::flush_background_logs(app);
        return;
    }

    log::info!("[Lifecycle] App returned to foreground. Reconnecting VCPLog/Info.");
    let result = crate::vcp_modules::infra::vcp_log_service::reconnect_log_connections(
        app,
        settings.vcp_log_url.clone(),
        settings.vcp_log_key.clone(),
    )
    .await;
    if !app_transition_matches(app, epoch, true) {
        return;
    }
    if finish_restore_attempt(retry_flag, "VCPLog/Info", result) {
        crate::vcp_modules::infra::vcp_log_service::flush_background_logs(app);
    }
}

async fn restore_distributed_connection(app: &AppHandle, retry_flag: &AtomicBool, epoch: u64) {
    if !app_transition_matches(app, epoch, true) {
        return;
    }
    let settings_state = app.state::<SettingsState>();
    match read_settings(app.clone(), settings_state).await {
        Ok(settings) => {
            restore_distributed_connection_with_settings(app, retry_flag, &settings, epoch).await;
        }
        Err(error) => {
            log::error!(
                "[Lifecycle] Failed to read settings while restoring distributed client; retry flag retained: {error}"
            );
        }
    }
}

async fn restore_distributed_connection_with_settings(
    app: &AppHandle,
    retry_flag: &AtomicBool,
    settings: &Settings,
    epoch: u64,
) {
    if !app_transition_matches(app, epoch, true) {
        return;
    }
    if !settings.distributed_enabled {
        retry_flag.store(false, Ordering::SeqCst);
        return;
    }

    log::info!("[Lifecycle] App returned to foreground. Reconnecting distributed client.");
    let result = crate::vcp_modules::infra::lifecycle_reconciler::reconcile_distributed_node(
        app, true, false,
    )
    .await;
    if !app_transition_matches(app, epoch, true) {
        return;
    }
    finish_restore_attempt(retry_flag, "distributed client", result);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_epoch_rejects_stale_async_work() {
        let state = LifecycleState::new();
        assert!(transition_matches(&state, 0, true));

        state.is_foreground.store(false, Ordering::SeqCst);
        state.transition_epoch.store(1, Ordering::SeqCst);
        assert!(transition_matches(&state, 1, false));
        assert!(!transition_matches(&state, 0, true));

        state.is_foreground.store(true, Ordering::SeqCst);
        state.transition_epoch.store(2, Ordering::SeqCst);
        assert!(transition_matches(&state, 2, true));
        assert!(!transition_matches(&state, 1, false));
    }

    #[test]
    fn restore_flag_is_cleared_only_after_success() {
        let flag = AtomicBool::new(true);

        assert!(!finish_restore_attempt(
            &flag,
            "test service",
            Err("offline".to_string())
        ));
        assert!(flag.load(Ordering::SeqCst));

        assert!(finish_restore_attempt(&flag, "test service", Ok(())));
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[test]
    fn log_configuration_requires_both_values() {
        let mut settings = Settings {
            vcp_log_url: "ws://localhost:6005".to_string(),
            ..Settings::default()
        };
        assert!(!has_log_configuration(&settings));

        settings.vcp_log_key = "key".to_string();
        assert!(has_log_configuration(&settings));
    }
}

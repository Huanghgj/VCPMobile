use tauri::{AppHandle, Runtime};

pub struct StreamServiceGuard<R: Runtime> {
    app_handle: AppHandle<R>,
    agent_name: String,
    active: bool,
    log_prefix: &'static str,
}

impl<R: Runtime> StreamServiceGuard<R> {
    pub fn start(app_handle: AppHandle<R>, agent_name: String, log_prefix: &'static str) -> Self {
        let active = match tauri_plugin_vcp_mobile::stream::start_stream_service_inner(
            &app_handle,
            &agent_name,
        ) {
            Ok(()) => true,
            Err(e) => {
                log::warn!(
                    "[{}] Failed to start streaming service early: {}",
                    log_prefix,
                    e
                );
                false
            }
        };

        Self {
            app_handle,
            agent_name,
            active,
            log_prefix,
        }
    }

    pub fn stop(&mut self) {
        if !self.active {
            return;
        }

        if let Err(e) = tauri_plugin_vcp_mobile::stream::stop_stream_service_inner(
            &self.app_handle,
            &self.agent_name,
        ) {
            log::warn!(
                "[{}] Failed to stop streaming service: {}",
                self.log_prefix,
                e
            );
            return;
        }
        self.active = false;
    }
}

impl<R: Runtime> Drop for StreamServiceGuard<R> {
    fn drop(&mut self) {
        self.stop();
    }
}

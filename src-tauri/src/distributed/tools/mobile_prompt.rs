// distributed/tools/mobile_prompt.rs
// [Interactive] MobileWaitingForUrReply — VCPChat WaitingForUrReply mobile equivalent.

use async_trait::async_trait;
use dashmap::DashMap;
use lazy_static::lazy_static;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use crate::distributed::tool_registry::OneShotTool;
use crate::distributed::types::ToolManifest;

lazy_static! {
    static ref PENDING_PROMPTS: DashMap<String, oneshot::Sender<Value>> = DashMap::new();
}

pub struct MobilePromptTool;

fn collect_options(args: &Value) -> Vec<String> {
    (1..=9)
        .filter_map(|idx| {
            let key = format!("option{idx:02}");
            args.get(&key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
        })
        .collect()
}

fn timeout_secs(args: &Value) -> u64 {
    args.get("timeout")
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(1200)
        .clamp(10, 3600)
}

pub fn submit_prompt_response(
    request_id: String,
    response: Option<String>,
    cancelled: Option<bool>,
) -> Result<(), String> {
    let Some((_, sender)) = PENDING_PROMPTS.remove(&request_id) else {
        return Err("Prompt request is not pending or already timed out.".to_string());
    };

    let is_cancelled = cancelled.unwrap_or(false);
    let payload = if is_cancelled {
        json!({
            "status": "cancelled",
            "response": "",
            "message": "User cancelled the mobile prompt."
        })
    } else {
        json!({
            "status": "success",
            "response": response.unwrap_or_default(),
        })
    };

    sender
        .send(payload)
        .map_err(|_| "Failed to deliver prompt response.".to_string())
}

#[async_trait]
impl OneShotTool for MobilePromptTool {
    fn manifest(&self) -> ToolManifest {
        ToolManifest {
            name: "MobileWaitingForUrReply".to_string(),
            description: "移动端等待用户回复工具。兼容 VCPChat WaitingForUrReply 的 title、prompt、placeholder、option01-option09、timeout 参数。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "prompt": { "type": "string" },
                    "placeholder": { "type": "string" },
                    "timeout": { "type": "integer" },
                    "option01": { "type": "string" },
                    "option02": { "type": "string" },
                    "option03": { "type": "string" },
                    "option04": { "type": "string" },
                    "option05": { "type": "string" },
                    "option06": { "type": "string" },
                    "option07": { "type": "string" },
                    "option08": { "type": "string" },
                    "option09": { "type": "string" }
                }
            }),
            tool_type: "mobile".to_string(),
        }
    }

    async fn execute(&self, args: Value, app: &AppHandle) -> Result<Value, String> {
        let request_id = Uuid::new_v4().to_string();
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("等待用户回复")
            .to_string();
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("请提供你的回复。")
            .to_string();
        let placeholder = args
            .get("placeholder")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let options = collect_options(&args);
        let wait_secs = timeout_secs(&args);

        let (tx, rx) = oneshot::channel();
        PENDING_PROMPTS.insert(request_id.clone(), tx);

        app.emit(
            "distributed-mobile-prompt",
            json!({
                "requestId": request_id,
                "title": title,
                "prompt": prompt,
                "placeholder": placeholder,
                "options": options,
                "timeout": wait_secs,
            }),
        )
        .map_err(|e| format!("Failed to show mobile prompt: {e}"))?;

        match timeout(Duration::from_secs(wait_secs), rx).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err("Mobile prompt response channel closed.".to_string()),
            Err(_) => {
                PENDING_PROMPTS.remove(&request_id);
                Ok(json!({
                    "status": "timeout",
                    "response": "",
                    "message": "User did not respond before timeout."
                }))
            }
        }
    }
}

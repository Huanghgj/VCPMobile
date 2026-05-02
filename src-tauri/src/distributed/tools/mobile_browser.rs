use async_trait::async_trait;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::distributed::tool_registry::OneShotTool;
use crate::distributed::types::ToolManifest;
use crate::vcp_modules::browser_service::{
    execute_browser_action_shared, start_assist_server, stop_assist_server, BrowserAction,
    BrowserRuntimeState,
};

pub struct MobileBrowserTool;

fn action_name(args: &Value) -> String {
    args.get("action")
        .or_else(|| args.get("command"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

#[async_trait]
impl OneShotTool for MobileBrowserTool {
    fn manifest(&self) -> ToolManifest {
        ToolManifest {
            name: "MobileBrowser".to_string(),
            description: "AI-controllable mobile browser runtime. Supports navigation, page snapshots, clicks, typing, scrolling, script evaluation, user handoff, and local assist server management.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "navigate",
                            "open",
                            "snapshot",
                            "read_text",
                            "click",
                            "type",
                            "scroll",
                            "screenshot",
                            "tap",
                            "back",
                            "forward",
                            "reload",
                            "eval",
                            "handoff",
                            "resume",
                            "start_assist_server",
                            "stop_assist_server"
                        ]
                    },
                    "url": { "type": "string" },
                    "selector": { "type": "string" },
                    "text": { "type": "string" },
                    "value": { "type": "string" },
                    "script": { "type": "string" },
                    "direction": { "type": "string", "enum": ["up", "down", "left", "right"] },
                    "amount": { "type": "number" },
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "reason": { "type": "string" },
                    "bindLan": { "type": "boolean" }
                },
                "required": ["action"]
            }),
            tool_type: "mobile".to_string(),
        }
    }

    async fn execute(&self, args: Value, app: &AppHandle) -> Result<Value, String> {
        let state = BrowserRuntimeState::global();
        match action_name(&args).as_str() {
            "start_assist_server" | "startassistserver" => {
                let bind_lan = args
                    .get("bindLan")
                    .or_else(|| args.get("bind_lan"))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let info = start_assist_server(Some(app.clone()), &state, bind_lan).await?;
                Ok(json!({ "status": "success", "assist": info }))
            }
            "stop_assist_server" | "stopassistserver" => {
                stop_assist_server(&state).await?;
                Ok(json!({ "status": "success" }))
            }
            _ => {
                let action: BrowserAction = serde_json::from_value(args)
                    .map_err(|error| format!("Invalid MobileBrowser args: {error}"))?;
                execute_browser_action_shared(app, &state, action).await
            }
        }
    }
}

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use url::Url;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcpToolboxRequest {
    pub vcp_url: String,
    pub vcp_api_key: String,
    pub method: Option<String>,
    pub path: String,
    pub body: Option<Value>,
    pub admin_username: Option<String>,
    pub admin_password: Option<String>,
}

fn build_url(base: &str, path: &str) -> Result<Url, String> {
    let mut url = Url::parse(base).map_err(|e| format!("Invalid VCP URL: {}", e))?;
    let (raw_path, raw_query) = path.split_once('?').unwrap_or((path, ""));
    let normalized_path = if raw_path.starts_with('/') {
        raw_path.to_string()
    } else {
        format!("/{}", raw_path)
    };
    url.set_path(&normalized_path);
    url.set_query(if raw_query.is_empty() {
        None
    } else {
        Some(raw_query)
    });
    url.set_fragment(None);
    Ok(url)
}

fn is_allowed_path(path: &str) -> bool {
    const ALLOWED_PREFIXES: &[&str] = &[
        "/v1/models",
        "/v1/interrupt",
        "/v1/schedule_task",
        "/admin_api/server/lifecycle",
        "/admin_api/system-monitor",
        "/admin_api/newapi-monitor",
        "/admin_api/server-log",
        "/admin_api/user-auth-code",
        "/admin_api/weather",
        "/admin_api/plugins",
        "/admin_api/dynamic-tools",
        "/admin_api/vectordb-status",
        "/admin_api/rag-tags",
        "/admin_api/rag-params",
        "/admin_api/available-clusters",
        "/admin_api/task-assistant",
        "/admin_api/schedules",
        "/admin_api/dailynotes",
    ];

    ALLOWED_PREFIXES
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{}/", prefix)))
}

#[tauri::command]
pub async fn call_vcp_toolbox_api(request: VcpToolboxRequest) -> Result<Value, String> {
    if !is_allowed_path(&request.path) {
        return Err(format!(
            "VCPToolBox API path is not allowed: {}",
            request.path
        ));
    }

    let method = request
        .method
        .unwrap_or_else(|| "GET".to_string())
        .to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "DELETE") {
        return Err(format!("Unsupported VCPToolBox API method: {}", method));
    }

    let url = build_url(&request.vcp_url, &request.path)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let mut builder = match method.as_str() {
        "POST" => client.post(url),
        "DELETE" => client.delete(url),
        _ => client.get(url),
    }
    .header(ACCEPT, "application/json")
    .header(CONTENT_TYPE, "application/json");

    if request.path.starts_with("/admin_api") {
        if let (Some(username), Some(password)) = (
            request.admin_username.filter(|v| !v.is_empty()),
            request.admin_password.filter(|v| !v.is_empty()),
        ) {
            builder = builder.basic_auth(username, Some(password));
        }
    } else if !request.vcp_api_key.is_empty() {
        builder = builder.header(AUTHORIZATION, format!("Bearer {}", request.vcp_api_key));
    }

    if let Some(body) = request.body {
        builder = builder.json(&body);
    }

    let response = builder.send().await.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let data = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({ "raw": text }))
    };

    Ok(json!({
        "ok": (200..300).contains(&status),
        "status": status,
        "data": data,
    }))
}

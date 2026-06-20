// diary_manager.rs —— 日记本(DailyNote)远程管理
// 复用 VCPToolBox 的管理员鉴权接口 /admin_api/dailynotes/*，让 app 端可以
// 浏览/搜索/查看/编辑/新建/移动/批量删除日记，并做向量「联想追溯」。
// 鉴权与 base_url 派生方式与 emoticon_manager 保持一致（adminUsername/adminPassword + Basic Auth）。

use crate::vcp_modules::settings_manager::{read_settings, SettingsState};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use url::Url;

// 路径段编码集：编码除字母数字与 - _ . 外的所有字符（含 / \ 空格 与中文 UTF-8 字节），
// 保证含中文/空格的本子名、形如 2026.03.02.txt 的文件名都能安全拼进 URL。
const PATH_SEG: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.');

fn enc(seg: &str) -> String {
    utf8_percent_encode(seg, PATH_SEG).to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiaryNote {
    pub name: String,
    #[serde(default)]
    pub last_modified: String,
    #[serde(default)]
    pub preview: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DiaryNoteRef {
    pub folder: String,
    pub file: String,
}

struct AdminCtx {
    base: String,
    user: String,
    pass: String,
    client: reqwest::Client,
}

/// 读取设置、校验管理员凭据、派生 origin。失败返回中文提示。
async fn admin_ctx(app_handle: &AppHandle) -> Result<AdminCtx, String> {
    let settings_state = app_handle.state::<SettingsState>();
    let settings = read_settings(app_handle.clone(), settings_state).await?;

    if settings.vcp_server_url.is_empty() {
        return Err("VCP 服务器地址未配置，请在 设置 → 服务器连接 中填写".to_string());
    }
    if settings.admin_username.is_empty() || settings.admin_password.is_empty() {
        return Err(
            "管理员账号或密码未配置，请在 设置 → 数据同步 中填写管理员账号和密码".to_string(),
        );
    }

    let base = match Url::parse(&settings.vcp_server_url) {
        Ok(u) => {
            let scheme = u.scheme();
            let host = u.host_str().unwrap_or("");
            match u.port() {
                Some(port) => format!("{}://{}:{}", scheme, host, port),
                None => format!("{}://{}", scheme, host),
            }
        }
        Err(_) => return Err("VCP 服务器地址无效".to_string()),
    };

    Ok(AdminCtx {
        base,
        user: settings.admin_username,
        pass: settings.admin_password,
        client: reqwest::Client::new(),
    })
}

/// 统一发送并解析 JSON；非 2xx 时优先回传服务端 message/error 文案。
async fn send_json(req: reqwest::RequestBuilder, url: &str) -> Result<Value, String> {
    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        let server_msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| {
            v.get("message")
                .and_then(|m| m.as_str())
                .or_else(|| v.get("error").and_then(|m| m.as_str()))
                .map(|s| s.to_string())
        });
        return Err(server_msg.unwrap_or_else(|| format!("接口返回 {} — {}", status, url)));
    }

    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str::<Value>(&text).map_err(|e| format!("解析响应失败: {}", e))
}

fn diary_url(ctx: &AdminCtx, suffix: &str) -> String {
    format!("{}/admin_api/dailynotes{}", ctx.base, suffix)
}

async fn admin_get(ctx: &AdminCtx, suffix: &str) -> Result<Value, String> {
    let url = diary_url(ctx, suffix);
    let req = ctx.client.get(&url).basic_auth(&ctx.user, Some(&ctx.pass));
    send_json(req, &url).await
}

async fn admin_post(ctx: &AdminCtx, suffix: &str, body: &Value) -> Result<Value, String> {
    let url = diary_url(ctx, suffix);
    let req = ctx
        .client
        .post(&url)
        .basic_auth(&ctx.user, Some(&ctx.pass))
        .json(body);
    send_json(req, &url).await
}

// ════════════════════════════ Tauri Commands ════════════════════════════

/// 列出所有日记本(文件夹)。
#[tauri::command]
pub async fn diary_list_folders(app_handle: AppHandle) -> Result<Vec<String>, String> {
    let ctx = admin_ctx(&app_handle).await?;
    let v = admin_get(&ctx, "/folders").await?;
    let folders = v
        .get("folders")
        .and_then(|f| f.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    Ok(folders)
}

/// 列出某个本子内的所有日记(按修改时间倒序)。
#[tauri::command]
pub async fn diary_list_notes(
    app_handle: AppHandle,
    folder: String,
) -> Result<Vec<DiaryNote>, String> {
    let ctx = admin_ctx(&app_handle).await?;
    let v = admin_get(&ctx, &format!("/folder/{}", enc(&folder))).await?;
    let notes = v.get("notes").cloned().unwrap_or(Value::Null);
    serde_json::from_value::<Vec<DiaryNote>>(notes).map_err(|e| format!("解析日记列表失败: {}", e))
}

/// 读取单篇日记内容。
#[tauri::command]
pub async fn diary_read_note(
    app_handle: AppHandle,
    folder: String,
    file: String,
) -> Result<String, String> {
    let ctx = admin_ctx(&app_handle).await?;
    let v = admin_get(&ctx, &format!("/note/{}/{}", enc(&folder), enc(&file))).await?;
    Ok(v.get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string())
}

/// 保存日记（新建/更新合一；目标本子不存在会自动创建）。重命名通过「存新名+删旧名」组合实现。
#[tauri::command]
pub async fn diary_save_note(
    app_handle: AppHandle,
    folder: String,
    file: String,
    content: String,
) -> Result<(), String> {
    let ctx = admin_ctx(&app_handle).await?;
    admin_post(
        &ctx,
        &format!("/note/{}/{}", enc(&folder), enc(&file)),
        &json!({ "content": content }),
    )
    .await?;
    Ok(())
}

/// 将若干日记移动到目标本子。透传 { message, moved[], errors[] }。
#[tauri::command]
pub async fn diary_move_notes(
    app_handle: AppHandle,
    source_notes: Vec<DiaryNoteRef>,
    target_folder: String,
) -> Result<Value, String> {
    let ctx = admin_ctx(&app_handle).await?;
    let source: Vec<Value> = source_notes
        .iter()
        .map(|n| json!({ "folder": n.folder, "file": n.file }))
        .collect();
    admin_post(
        &ctx,
        "/move",
        &json!({ "sourceNotes": source, "targetFolder": target_folder }),
    )
    .await
}

/// 批量删除日记。透传 { deleted[], errors[] }。
#[tauri::command]
pub async fn diary_delete_notes(
    app_handle: AppHandle,
    notes: Vec<DiaryNoteRef>,
) -> Result<Value, String> {
    let ctx = admin_ctx(&app_handle).await?;
    let list: Vec<Value> = notes
        .iter()
        .map(|n| json!({ "folder": n.folder, "file": n.file }))
        .collect();
    admin_post(&ctx, "/delete-batch", &json!({ "notesToDelete": list })).await
}

/// 删除空本子（服务端拒绝删除非空文件夹，会带回中文提示）。
#[tauri::command]
pub async fn diary_delete_folder(app_handle: AppHandle, folder: String) -> Result<(), String> {
    let ctx = admin_ctx(&app_handle).await?;
    admin_post(&ctx, "/folder/delete", &json!({ "folderName": folder })).await?;
    Ok(())
}

/// 关键词搜索（可限定本子）。透传 { notes[], total, limited }。
#[tauri::command]
pub async fn diary_search(
    app_handle: AppHandle,
    term: String,
    folder: Option<String>,
    limit: Option<u32>,
) -> Result<Value, String> {
    let ctx = admin_ctx(&app_handle).await?;
    let url = diary_url(&ctx, "/search");
    let mut query: Vec<(&str, String)> = vec![("term", term)];
    if let Some(f) = folder {
        if !f.is_empty() {
            query.push(("folder", f));
        }
    }
    query.push(("limit", limit.unwrap_or(200).to_string()));
    let req = ctx
        .client
        .get(&url)
        .basic_auth(&ctx.user, Some(&ctx.pass))
        .query(&query);
    send_json(req, &url).await
}

/// 联想追溯：基于某篇日记做向量语义关联发现。透传服务端结果。
#[tauri::command]
pub async fn diary_associative_discovery(
    app_handle: AppHandle,
    source_file_path: String,
    k: Option<u32>,
    range: Option<Value>,
    tag_boost: Option<Value>,
) -> Result<Value, String> {
    let ctx = admin_ctx(&app_handle).await?;
    let mut body = json!({ "sourceFilePath": source_file_path });
    if let Some(k) = k {
        body["k"] = json!(k);
    }
    if let Some(r) = range {
        body["range"] = r;
    }
    if let Some(tb) = tag_boost {
        body["tagBoost"] = tb;
    }
    admin_post(&ctx, "/associative-discovery", &body).await
}

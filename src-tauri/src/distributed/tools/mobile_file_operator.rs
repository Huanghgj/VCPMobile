// distributed/tools/mobile_file_operator.rs
// [OneShot] MobileFileOperator — VCPChat FileOperator-compatible subset.
// All paths are constrained to the app-owned distributed-files directory.

use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::fs;
use walkdir::WalkDir;

use crate::distributed::tool_registry::OneShotTool;
use crate::distributed::types::ToolManifest;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
const MAX_DIRECTORY_ITEMS: usize = 1000;
const MAX_SEARCH_RESULTS: usize = 100;
const VIRTUAL_ROOT: &str = "mobile://distributed-files";

pub struct MobileFileOperatorTool;

fn root_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let mut dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    dir.push("distributed-files");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create root directory: {e}"))?;
    Ok(dir)
}

fn arg_str<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn arg_bool(args: &Value, key: &str, default: bool) -> bool {
    args.get(key)
        .and_then(|v| {
            v.as_bool().or_else(|| {
                v.as_str()
                    .map(|s| matches!(s.to_lowercase().as_str(), "true" | "1" | "yes"))
            })
        })
        .unwrap_or(default)
}

fn resolve_path(app: &AppHandle, raw: &str) -> Result<PathBuf, String> {
    let root = root_dir(app)?;
    let root_str = root.to_string_lossy();
    let mut input = raw.trim();

    if input.is_empty() || input == VIRTUAL_ROOT {
        return Ok(root);
    }

    if let Some(stripped) = input.strip_prefix(VIRTUAL_ROOT) {
        input = stripped.trim_start_matches('/');
    } else if let Some(stripped) = input.strip_prefix("distributed-files://") {
        input = stripped.trim_start_matches('/');
    } else if input.starts_with(root_str.as_ref()) {
        input = input
            .trim_start_matches(root_str.as_ref())
            .trim_start_matches(std::path::MAIN_SEPARATOR);
    } else if Path::new(input).is_absolute() {
        return Err(format!(
            "Absolute path is outside the mobile sandbox. Use {VIRTUAL_ROOT}/..."
        ));
    }

    let mut resolved = root;
    for component in Path::new(input).components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("Path traversal is not allowed.".to_string());
            }
        }
    }
    Ok(resolved)
}

fn virtualize(app: &AppHandle, path: &Path) -> String {
    match root_dir(app).ok().and_then(|root| {
        path.strip_prefix(root)
            .ok()
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
    }) {
        Some(relative) if relative.is_empty() => VIRTUAL_ROOT.to_string(),
        Some(relative) => format!("{VIRTUAL_ROOT}/{relative}"),
        None => path.to_string_lossy().to_string(),
    }
}

fn metadata_json(app: &AppHandle, path: &Path) -> Result<Value, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);

    Ok(json!({
        "path": virtualize(app, path),
        "isDirectory": meta.is_dir(),
        "isFile": meta.is_file(),
        "size": meta.len(),
        "modifiedAt": modified,
    }))
}

fn wildcard_match(pattern: &str, text: &str, case_sensitive: bool) -> bool {
    let pattern = if case_sensitive {
        pattern.to_string()
    } else {
        pattern.to_lowercase()
    };
    let text = if case_sensitive {
        text.to_string()
    } else {
        text.to_lowercase()
    };

    if pattern == "*" {
        return true;
    }

    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return text.contains(&pattern);
    }

    let mut cursor = 0usize;
    for (idx, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let haystack = &text[cursor..];
        let Some(found) = haystack.find(part) else {
            return false;
        };
        if idx == 0 && !pattern.starts_with('*') && found != 0 {
            return false;
        }
        cursor += found + part.len();
    }

    pattern.ends_with('*') || parts.last().is_some_and(|last| text.ends_with(last))
}

async fn read_file(app: &AppHandle, path_arg: &str, encoding: &str) -> Result<Value, String> {
    let path = resolve_path(app, path_arg)?;
    let meta = fs::metadata(&path).await.map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Target is not a file.".to_string());
    }
    if meta.len() > MAX_FILE_SIZE {
        return Err(format!("File exceeds {} bytes.", MAX_FILE_SIZE));
    }
    let bytes = fs::read(&path).await.map_err(|e| e.to_string())?;
    let content = if encoding.eq_ignore_ascii_case("base64") {
        general_purpose::STANDARD.encode(bytes)
    } else {
        String::from_utf8(bytes).map_err(|e| format!("File is not valid UTF-8: {e}"))?
    };
    Ok(json!({
        "status": "success",
        "filePath": virtualize(app, &path),
        "encoding": encoding,
        "content": content,
    }))
}

async fn write_file(
    app: &AppHandle,
    path_arg: &str,
    content: &str,
    encoding: &str,
    append: bool,
    require_exists: bool,
    avoid_overwrite: bool,
) -> Result<Value, String> {
    let mut path = resolve_path(app, path_arg)?;
    if require_exists && !path.exists() {
        return Err("Target file does not exist.".to_string());
    }
    if avoid_overwrite {
        path = unique_path(&path);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let bytes = if encoding.eq_ignore_ascii_case("base64") {
        general_purpose::STANDARD
            .decode(content)
            .map_err(|e| format!("Invalid base64 content: {e}"))?
    } else {
        content.as_bytes().to_vec()
    };
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(format!("Content exceeds {} bytes.", MAX_FILE_SIZE));
    }
    if append {
        use tokio::io::AsyncWriteExt;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).await.map_err(|e| e.to_string())?;
    } else {
        fs::write(&path, bytes).await.map_err(|e| e.to_string())?;
    }
    Ok(json!({
        "status": "success",
        "filePath": virtualize(app, &path),
    }))
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    for i in 1..1000 {
        let file_name = if ext.is_empty() {
            format!("{stem}({i})")
        } else {
            format!("{stem}({i}).{ext}")
        };
        let candidate = parent.join(file_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

async fn list_directory(
    app: &AppHandle,
    path_arg: &str,
    show_hidden: bool,
) -> Result<Value, String> {
    let path = resolve_path(app, path_arg)?;
    let mut entries = fs::read_dir(&path).await.map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let name = entry.file_name().to_string_lossy().to_string();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let meta = entry.metadata().await.map_err(|e| e.to_string())?;
        items.push(json!({
            "name": name,
            "path": virtualize(app, &entry.path()),
            "type": if meta.is_dir() { "directory" } else { "file" },
            "size": meta.len(),
        }));
        if items.len() >= MAX_DIRECTORY_ITEMS {
            break;
        }
    }
    Ok(json!({
        "status": "success",
        "directoryPath": virtualize(app, &path),
        "items": items,
    }))
}

#[async_trait]
impl OneShotTool for MobileFileOperatorTool {
    fn manifest(&self) -> ToolManifest {
        ToolManifest {
            name: "MobileFileOperator".to_string(),
            description: "移动端安全文件操作器。兼容 VCPChat FileOperator 的常用 command：ListAllowedDirectories、ReadFile、WriteFile、AppendFile、EditFile、ListDirectory、FileInfo、CopyFile、MoveFile、RenameFile、DeleteFile、CreateDirectory、SearchFiles、DownloadFile。所有路径限制在 mobile://distributed-files。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "path": { "type": "string" },
                    "filePath": { "type": "string" },
                    "directoryPath": { "type": "string" },
                    "content": { "type": "string" },
                    "encoding": { "type": "string", "enum": ["utf8", "base64"] }
                },
                "required": ["command"]
            }),
            tool_type: "mobile".to_string(),
        }
    }

    async fn execute(&self, args: Value, app: &AppHandle) -> Result<Value, String> {
        let command = arg_str(&args, &["command", "action"]).unwrap_or("ListAllowedDirectories");
        let encoding = arg_str(&args, &["encoding"]).unwrap_or("utf8");

        match command {
            "ListAllowedDirectories" | "roots" => {
                let root = root_dir(app)?;
                Ok(json!({
                    "status": "success",
                    "directories": [{
                        "path": VIRTUAL_ROOT,
                        "realPath": root.to_string_lossy(),
                        "description": "VCPMobile app-owned distributed file sandbox"
                    }]
                }))
            }
            "ReadFile" | "read" => {
                let path = arg_str(&args, &["filePath", "path"]).ok_or("Missing filePath.")?;
                read_file(app, path, encoding).await
            }
            "WriteFile" | "write" => {
                let path = arg_str(&args, &["filePath", "path"]).ok_or("Missing filePath.")?;
                let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                write_file(app, path, content, encoding, false, false, true).await
            }
            "AppendFile" | "append" => {
                let path = arg_str(&args, &["filePath", "path"]).ok_or("Missing filePath.")?;
                let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                write_file(app, path, content, encoding, true, false, false).await
            }
            "EditFile" | "edit" => {
                let path = arg_str(&args, &["filePath", "path"]).ok_or("Missing filePath.")?;
                let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                write_file(app, path, content, encoding, false, true, false).await
            }
            "ListDirectory" | "list" => {
                let path = arg_str(&args, &["directoryPath", "path"]).unwrap_or("");
                list_directory(app, path, arg_bool(&args, "showHidden", false)).await
            }
            "FileInfo" | "stat" => {
                let path = arg_str(&args, &["filePath", "path"]).ok_or("Missing filePath.")?;
                let resolved = resolve_path(app, path)?;
                Ok(json!({ "status": "success", "info": metadata_json(app, &resolved)? }))
            }
            "CreateDirectory" | "mkdir" => {
                let path =
                    arg_str(&args, &["directoryPath", "path"]).ok_or("Missing directoryPath.")?;
                let resolved = resolve_path(app, path)?;
                fs::create_dir_all(&resolved)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(json!({ "status": "success", "directoryPath": virtualize(app, &resolved) }))
            }
            "DeleteFile" | "delete" => {
                let path = arg_str(&args, &["filePath", "path"]).ok_or("Missing filePath.")?;
                let resolved = resolve_path(app, path)?;
                let meta = fs::metadata(&resolved).await.map_err(|e| e.to_string())?;
                if meta.is_dir() {
                    fs::remove_dir(&resolved).await.map_err(|e| e.to_string())?;
                } else {
                    fs::remove_file(&resolved)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                Ok(json!({ "status": "success", "deletedPath": virtualize(app, &resolved) }))
            }
            "CopyFile" | "copy" => {
                let source = arg_str(&args, &["sourcePath"]).ok_or("Missing sourcePath.")?;
                let destination =
                    arg_str(&args, &["destinationPath"]).ok_or("Missing destinationPath.")?;
                let src = resolve_path(app, source)?;
                let mut dst = resolve_path(app, destination)?;
                dst = unique_path(&dst);
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                fs::copy(&src, &dst).await.map_err(|e| e.to_string())?;
                Ok(
                    json!({ "status": "success", "sourcePath": virtualize(app, &src), "destinationPath": virtualize(app, &dst) }),
                )
            }
            "MoveFile" | "RenameFile" | "move" | "rename" => {
                let source = arg_str(&args, &["sourcePath"]).ok_or("Missing sourcePath.")?;
                let destination =
                    arg_str(&args, &["destinationPath"]).ok_or("Missing destinationPath.")?;
                let src = resolve_path(app, source)?;
                let dst = resolve_path(app, destination)?;
                if dst.exists() {
                    return Err("Destination already exists.".to_string());
                }
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                fs::rename(&src, &dst).await.map_err(|e| e.to_string())?;
                Ok(
                    json!({ "status": "success", "sourcePath": virtualize(app, &src), "destinationPath": virtualize(app, &dst) }),
                )
            }
            "SearchFiles" | "search" => {
                let search_path =
                    arg_str(&args, &["searchPath", "directoryPath", "path"]).unwrap_or("");
                let pattern = arg_str(&args, &["pattern"]).unwrap_or("*");
                let base = resolve_path(app, search_path)?;
                let case_sensitive = args
                    .get("options")
                    .and_then(|v| v.as_object())
                    .and_then(|o| o.get("caseSensitive"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let mut results = Vec::new();
                for entry in WalkDir::new(&base).max_depth(8).into_iter().flatten() {
                    let name = entry.file_name().to_string_lossy();
                    if wildcard_match(pattern, &name, case_sensitive) {
                        results.push(metadata_json(app, entry.path())?);
                        if results.len() >= MAX_SEARCH_RESULTS {
                            break;
                        }
                    }
                }
                Ok(json!({ "status": "success", "results": results, "total": results.len() }))
            }
            "DownloadFile" | "download" => {
                let url = arg_str(&args, &["url"]).ok_or("Missing url.")?;
                let file_name = arg_str(&args, &["fileName"]).unwrap_or_else(|| {
                    url.rsplit('/')
                        .next()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("download.bin")
                });
                let dir = arg_str(&args, &["downloadDir", "directoryPath"]).unwrap_or("downloads");
                let mut path = resolve_path(app, dir)?;
                path.push(file_name);
                path = unique_path(&path);
                let bytes = reqwest::get(url)
                    .await
                    .map_err(|e| e.to_string())?
                    .bytes()
                    .await
                    .map_err(|e| e.to_string())?;
                if bytes.len() as u64 > MAX_FILE_SIZE {
                    return Err(format!("Download exceeds {} bytes.", MAX_FILE_SIZE));
                }
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                fs::write(&path, bytes).await.map_err(|e| e.to_string())?;
                Ok(json!({ "status": "success", "filePath": virtualize(app, &path) }))
            }
            other => Err(format!("Unsupported MobileFileOperator command: {other}")),
        }
    }
}

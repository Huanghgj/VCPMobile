use crate::vcp_modules::db_manager::DbState;
use sha2::{Digest, Sha256};
use std::fs;
use tauri::{AppHandle, Manager, Runtime, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(serde::Deserialize)]
pub struct UploadMetadata {
    pub name: String,
    pub mime: String,
    pub size: u64,
}

#[derive(serde::Serialize)]
pub struct UploadEndpoint {
    pub url: String,
    pub token: String,
}

#[derive(Debug, PartialEq, Eq)]
struct UploadRequestHeaders {
    method: String,
    upload_token: Option<String>,
}

fn parse_upload_request_headers(header_str: &str) -> UploadRequestHeaders {
    let mut lines = header_str.lines();
    let method = lines
        .next()
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or_default()
        .to_ascii_uppercase();

    let upload_token = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("X-Upload-Token") {
            Some(value.trim().to_string())
        } else {
            None
        }
    });

    UploadRequestHeaders {
        method,
        upload_token,
    }
}

fn plain_http_response(status: u16, reason: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {} {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: X-Upload-Token, Content-Type\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        reason,
        body.len(),
        body
    )
}

/// 准备高速上传链路：启动临时本地服务器并返回端口
///
/// 【适用场景】非 Android 端的大文件 (≥2MB) 上传。前端通过 XHR 向本地临时 TCP
/// 端口发送流式数据，Rust 直接写入磁盘，绕过 Tauri IPC 的内存限制。
///
/// Android 端不走此链路：Android 通过原生插件 `pick_file` 在 Kotlin 层完成流式
/// 拷贝与哈希，直接调用 `register_local_file` 零拷贝注册，无需 WebView 参与传输。
#[tauri::command]
pub async fn prepare_vcp_upload<R: Runtime>(
    app_handle: AppHandle<R>,
    db_state: State<'_, DbState>,
    metadata: UploadMetadata,
) -> Result<UploadEndpoint, String> {
    // 1. 监听本地随机端口
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = uuid::Uuid::new_v4().to_string();

    let url = format!("http://127.0.0.1:{}", port);
    let token_clone = token.clone();
    let pool = db_state.pool.clone();

    tauri::async_runtime::spawn(async move {
        let mut upload_finished = false;
        let timeout = std::time::Duration::from_secs(20);
        let start_time = std::time::Instant::now();

        let mut temp_dir = match app_handle.path().app_cache_dir() {
            Ok(path) => path,
            Err(e) => {
                log::error!("[HighSpeedUpload] Failed to resolve app cache dir: {}", e);
                return;
            }
        };
        temp_dir.push("uploads");
        if let Err(e) = fs::create_dir_all(&temp_dir) {
            log::error!("[HighSpeedUpload] Failed to create upload cache dir: {}", e);
            return;
        }

        while !upload_finished && start_time.elapsed() < timeout {
            let accept_res =
                tokio::time::timeout(std::time::Duration::from_millis(500), listener.accept())
                    .await;

            let (mut socket, _addr) = match accept_res {
                Ok(Ok(conn)) => conn,
                _ => continue,
            };

            let mut buffer = [0u8; 65536];
            let mut body_started = false;
            let mut header_data = Vec::with_capacity(4096);

            let session_id = uuid::Uuid::new_v4().to_string();
            let temp_file_path = temp_dir.join(format!("{}.tmp", session_id));

            let mut bytes_count = 0u64;
            let mut hasher = Sha256::new();
            let mut is_options = false;
            let mut file: Option<tokio::fs::File> = None;

            loop {
                let n = match socket.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };

                let data = if !body_started {
                    header_data.extend_from_slice(&buffer[..n]);
                    if header_data.len() > 16 * 1024 {
                        let response = plain_http_response(
                            431,
                            "Request Header Fields Too Large",
                            "Request headers too large",
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        break;
                    }

                    if let Some(pos) = header_data.windows(4).position(|w| w == b"\r\n\r\n") {
                        body_started = true;
                        let header_str = String::from_utf8_lossy(&header_data[..pos]);
                        let request_headers = parse_upload_request_headers(&header_str);

                        if request_headers.method == "OPTIONS" {
                            is_options = true;
                            let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: X-Upload-Token, Content-Type\r\nAccess-Control-Max-Age: 86400\r\nConnection: close\r\n\r\n";
                            socket.write_all(resp.as_bytes()).await.ok();
                            break;
                        }

                        if request_headers.method != "POST" {
                            let response = plain_http_response(
                                405,
                                "Method Not Allowed",
                                "Only POST uploads are allowed",
                            );
                            let _ = socket.write_all(response.as_bytes()).await;
                            break;
                        }

                        if request_headers.upload_token.as_deref() != Some(token.as_str()) {
                            let response =
                                plain_http_response(401, "Unauthorized", "Invalid upload token");
                            let _ = socket.write_all(response.as_bytes()).await;
                            break;
                        }

                        file = match tokio::fs::File::create(&temp_file_path).await {
                            Ok(file) => Some(file),
                            Err(e) => {
                                let response = plain_http_response(
                                    500,
                                    "Internal Server Error",
                                    &format!("Failed to create upload file: {}", e),
                                );
                                let _ = socket.write_all(response.as_bytes()).await;
                                break;
                            }
                        };

                        let header_len_in_current_buffer = if header_data.len() > n {
                            let consumed_before = header_data.len() - n;
                            (pos + 4).saturating_sub(consumed_before)
                        } else {
                            0
                        };

                        if header_len_in_current_buffer < n {
                            &buffer[header_len_in_current_buffer..n]
                        } else {
                            &[]
                        }
                    } else {
                        &[]
                    }
                } else {
                    &buffer[..n]
                };

                if !data.is_empty() && !is_options {
                    if let Some(ref mut f) = file {
                        if f.write_all(data).await.is_err() {
                            let _ = fs::remove_file(&temp_file_path);
                            break;
                        }
                    }
                    hasher.update(data);
                    bytes_count += data.len() as u64;

                    if bytes_count >= metadata.size {
                        break;
                    }
                }
            }

            if let Some(mut f) = file.take() {
                let _ = f.flush().await;
                drop(f);
            }

            if !is_options && bytes_count > 0 {
                if bytes_count < metadata.size {
                    let _ = fs::remove_file(&temp_file_path);
                    let response = plain_http_response(400, "Bad Request", "Incomplete Data");
                    let _ = socket.write_all(response.as_bytes()).await;
                } else {
                    let hash = hex::encode(hasher.finalize());
                    let final_data_res = finalize_high_speed_upload(
                        &app_handle,
                        &pool,
                        &temp_file_path,
                        &metadata,
                        hash,
                        bytes_count,
                    )
                    .await;

                    let (status, reason, body) = match final_data_res {
                        Ok(data) => (200, "OK", serde_json::to_vec(&data).unwrap_or_default()),
                        Err(e) => (500, "Internal Server Error", e.into_bytes()),
                    };

                    let response = format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        status, reason, body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.write_all(&body).await;

                    upload_finished = true;
                }
            }
        }
    });

    Ok(UploadEndpoint {
        url,
        token: token_clone,
    })
}

async fn finalize_high_speed_upload<R: Runtime>(
    app_handle: &AppHandle<R>,
    pool: &sqlx::SqlitePool,
    temp_path: &std::path::PathBuf,
    metadata: &UploadMetadata,
    hash: String,
    size: u64,
) -> Result<crate::vcp_modules::file_manager::AttachmentData, String> {
    let ext = std::path::Path::new(&metadata.name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let internal_name = if ext.is_empty() {
        hash.clone()
    } else {
        format!("{}.{}", hash, ext)
    };

    let dest = crate::vcp_modules::file_manager::get_attachments_root_dir(app_handle)?;
    if !dest.exists() {
        fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    }
    let dest_path = dest.join(internal_name);
    let dest_path_str = dest_path.to_str().ok_or("无效的目标路径字符")?.to_string();

    crate::vcp_modules::file_manager::safe_rename(temp_path, &dest_path)
        .map_err(|e| e.to_string())?;

    crate::vcp_modules::file_manager::register_attachment_internal(
        app_handle,
        pool,
        hash,
        metadata.name.clone(),
        metadata.mime.clone(),
        size,
        dest_path_str,
    )
    .await
}

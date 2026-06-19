use base64::Engine as _;
use std::path::Path;

fn video_mime_for_path(path: &Path, declared_mime: &str) -> String {
    if declared_mime.starts_with("video/") {
        return declared_mime.to_string();
    }

    let guessed = mime_guess::from_path(path).first_or_octet_stream();
    if guessed.type_().as_str() == "video" {
        return guessed.to_string();
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" | "qt" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "wmv" => "video/x-ms-wmv",
        "flv" => "video/x-flv",
        "3gp" => "video/3gpp",
        "3g2" => "video/3gpp2",
        "ts" | "mts" | "m2ts" => "video/mp2t",
        _ => "video/mp4",
    }
    .to_string()
}

/// 处理视频：完整读取本地视频并转成 VCPToolBox 可识别的 data:video/*;base64 URL。
/// VCPToolBox 的 JSON 请求体默认上限是 300MB，Base64 会有约 4/3 膨胀，因此这里按
/// 220MiB 原文件大小做硬保护，避免构造一个服务端必定拒绝或移动端容易 OOM 的请求体。
pub fn convert_local_video_for_multimodal(
    path: &Path,
    declared_mime: &str,
) -> Result<String, String> {
    let metadata =
        std::fs::metadata(path).map_err(|e| format!("Failed to read raw video metadata: {}", e))?;

    const MAX_INLINE_VIDEO_BYTES: u64 = 220 * 1024 * 1024;
    if metadata.len() > MAX_INLINE_VIDEO_BYTES {
        return Err(format!(
            "Video is too large for inline VCP multimodal payload: {} bytes > {} bytes",
            metadata.len(),
            MAX_INLINE_VIDEO_BYTES
        ));
    }

    let bytes =
        std::fs::read(path).map_err(|e| format!("Failed to read raw video bytes: {}", e))?;
    let mime = video_mime_for_path(path, declared_mime);
    let prefix = format!("data:{};base64,", mime);
    let b64_len = (bytes.len() * 4).div_ceil(3);
    let mut result = String::with_capacity(prefix.len() + b64_len);
    result.push_str(&prefix);
    base64::engine::general_purpose::STANDARD.encode_string(&bytes, &mut result);
    Ok(result)
}

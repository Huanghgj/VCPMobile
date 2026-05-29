use super::ffmpeg_cli::run_ffmpeg;
use base64::Engine as _;
use std::path::Path;

pub struct ProcessedAudio {
    pub data: String,
    pub format: &'static str,
}

/// 处理音频：提取为 16kHz 单声道 WAV，返回 OpenAI/VCP input_audio 需要的裸 base64。
/// WAV 约 32KB/s，硬截断 420 秒可将 Base64 后的请求体控制在约 18MB 内。
pub fn process_audio_for_multimodal(path: &Path) -> Result<ProcessedAudio, String> {
    let wav_bytes = run_ffmpeg(&[
        "-t",
        "420",
        "-i",
        path.to_str().ok_or("Invalid audio path")?,
        "-vn",
        "-c:a",
        "pcm_s16le",
        "-ar",
        "16000",
        "-ac",
        "1",
        "-f",
        "wav",
        "pipe:1",
    ])?;

    // 优化 Base64 拼接：预分配内存并直接编码到 String
    let b64_len = (wav_bytes.len() * 4).div_ceil(3);
    let mut result = String::with_capacity(b64_len);
    base64::engine::general_purpose::STANDARD.encode_string(&wav_bytes, &mut result);

    Ok(ProcessedAudio {
        data: result,
        format: "wav",
    })
}

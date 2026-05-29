use base64::Engine as _;
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, GenericImageView, ImageReader};
use std::path::Path;

fn convert_with_image_crate(path: &Path) -> Result<Vec<u8>, String> {
    let image = ImageReader::open(path)
        .map_err(|e| format!("Failed to open image: {}", e))?
        .with_guessed_format()
        .map_err(|e| format!("Failed to detect image format: {}", e))?
        .decode()
        .map_err(|e| format!("Failed to decode image: {}", e))?;

    let (width, height) = image.dimensions();
    let resized = if width > 1120 || height > 1120 {
        image.resize(1120, 1120, FilterType::Lanczos3)
    } else {
        image
    };

    let rgba = resized.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut flattened = image::RgbImage::new(width, height);
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let alpha = pixel[3] as u16;
        let inverse_alpha = 255 - alpha;
        let r = ((pixel[0] as u16 * alpha + 255 * inverse_alpha) / 255) as u8;
        let g = ((pixel[1] as u16 * alpha + 255 * inverse_alpha) / 255) as u8;
        let b = ((pixel[2] as u16 * alpha + 255 * inverse_alpha) / 255) as u8;
        flattened.put_pixel(x, y, image::Rgb([r, g, b]));
    }

    let mut jpeg_bytes = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg_bytes, 85);
    encoder
        .encode_image(&flattened)
        .map_err(|e| format!("Failed to encode JPEG: {}", e))?;

    Ok(jpeg_bytes)
}

fn convert_with_ffmpeg(path: &Path) -> Result<Vec<u8>, String> {
    use super::ffmpeg_cli::run_ffmpeg;

    run_ffmpeg(&[
        "-i",
        path.to_str().ok_or("Invalid image path")?,
        "-vf",
        "scale='min(1120,iw)':'min(1120,ih)':force_original_aspect_ratio=decrease:flags=lanczos",
        "-c:v",
        "mjpeg",
        "-q:v",
        "3",
        "-f",
        "image2pipe",
        "pipe:1",
    ])
}

/// 将本地图片转换为多模态 Base64 data URL。
/// 常见格式走纯 Rust 解码/缩放；HEIC/AVIF 等格式再降级到 ffmpeg。
pub fn convert_local_image_for_multimodal(path: &Path) -> Result<String, String> {
    let jpeg_bytes = convert_with_image_crate(path).or_else(|image_err| {
        log::warn!(
            "[MediaProcessor] Rust image conversion failed for {:?}: {}. Falling back to ffmpeg.",
            path,
            image_err
        );
        convert_with_ffmpeg(path).map_err(|ffmpeg_err| {
            format!(
                "Rust image conversion failed: {}; ffmpeg fallback failed: {}",
                image_err, ffmpeg_err
            )
        })
    })?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg_bytes);
    Ok(format!("data:image/jpeg;base64,{}", b64))
}

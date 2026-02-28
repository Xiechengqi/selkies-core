//! Frame capture for MCP screenshot tool.
//!
//! Bridges the calloop compositor thread (which renders frames) with
//! tokio-based MCP tool handlers via oneshot channels.

use base64::Engine;
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::web::SharedState;

/// Request a frame capture from the compositor main loop.
/// Returns (width, height, xrgb8888_pixels).
pub async fn capture_frame(
    state: &Arc<SharedState>,
) -> Result<(u32, u32, Vec<u8>), String> {
    let (tx, rx) = oneshot::channel();
    state
        .frame_capture_tx
        .send(tx)
        .map_err(|_| "compositor not running")?;

    tokio::time::timeout(std::time::Duration::from_secs(2), rx)
        .await
        .map_err(|_| "frame capture timed out (2s)")?
        .map_err(|_| "compositor dropped frame capture request".to_string())
}

/// Convert XRGB8888 pixel buffer to JPEG, returning base64-encoded string.
/// If the result exceeds `max_bytes`, downscale and re-encode.
pub fn xrgb_to_jpeg_base64(
    width: u32,
    height: u32,
    xrgb: &[u8],
    quality: u8,
    max_bytes: usize,
) -> Result<String, String> {
    use image::{ImageBuffer, RgbImage};

    // Convert XRGB8888 → RGB
    let mut rgb_buf: Vec<u8> = Vec::with_capacity((width * height * 3) as usize);
    for pixel in xrgb.chunks_exact(4) {
        rgb_buf.push(pixel[2]); // R  (XRGB8888 LE memory: [B, G, R, X])
        rgb_buf.push(pixel[1]); // G
        rgb_buf.push(pixel[0]); // B
    }

    let img: RgbImage = ImageBuffer::from_raw(width, height, rgb_buf)
        .ok_or("failed to create image buffer")?;

    // First attempt at original resolution
    let jpeg = encode_jpeg(&img, quality)?;
    if jpeg.len() <= max_bytes {
        return Ok(base64::engine::general_purpose::STANDARD.encode(&jpeg));
    }

    // Downscale if too large
    let scale = (max_bytes as f64 / jpeg.len() as f64).sqrt().max(0.25);
    let new_w = ((width as f64 * scale) as u32).max(1);
    let new_h = ((height as f64 * scale) as u32).max(1);

    let resized = image::imageops::resize(
        &img,
        new_w,
        new_h,
        image::imageops::FilterType::Triangle,
    );
    let jpeg = encode_jpeg(&resized, quality.min(75))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&jpeg))
}

fn encode_jpeg<P, C>(img: &image::ImageBuffer<P, C>, quality: u8) -> Result<Vec<u8>, String>
where
    P: image::Pixel<Subpixel = u8> + image::PixelWithColorType + 'static,
    C: std::ops::Deref<Target = [u8]>,
{
    use image::codecs::jpeg::JpegEncoder;
    use std::io::Cursor;

    let mut buf = Cursor::new(Vec::new());
    let encoder = JpegEncoder::new_with_quality(&mut buf, quality);
    img.write_with_encoder(encoder)
        .map_err(|e| format!("JPEG encode failed: {}", e))?;
    Ok(buf.into_inner())
}

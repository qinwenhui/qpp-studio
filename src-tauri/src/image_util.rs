//! EXIF 方向感知的图像解码。
//!
//! 相机竖拍照片带 Orientation 标签:浏览器渲染 <img> 时会自动旋转显示,
//! 而引擎解码原始像素不转 → 框坐标与显示坐标系对不上(框全错位)。
//! 这里在喂引擎前把像素转到"显示方向",画布/引擎/缩略图三方一致。

use image::metadata::Orientation;
use image::{ImageDecoder, ImageReader};
use std::path::Path;

/// 解码并应用 EXIF Orientation(无标签/解码失败回退原图)。
pub fn decode_file_oriented(path: &Path) -> Result<qppocr::Image, qppocr::Error> {
    let bytes = std::fs::read(path)?;
    decode_bytes_oriented(&bytes)
}

pub fn decode_bytes_oriented(bytes: &[u8]) -> Result<qppocr::Image, qppocr::Error> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| qppocr::Error::Image(format!("解码失败: {e}")))?;
    let mut img = img;
    if let Some(o) = exif_orientation(bytes) {
        img.apply_orientation(o); // 原地应用(无方向/180/翻转免拷贝)
    }
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    qppocr::rgb_from_bytes(w, h, rgb.into_raw())
}

/// 读 JPEG/PNG 的 EXIF Orientation(image 0.25 的 decoder API)。
fn exif_orientation(bytes: &[u8]) -> Option<Orientation> {
    // with_guessed_format(self) -> io::Result<Self>:消费并返回 reader,再取 decoder
    let mut reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    decoder.orientation().ok()
}

/// 探测"显示方向"的宽高(EXIF 5-8 需要交换宽高)。
pub fn probe_dims_oriented(path: &Path) -> Result<(u32, u32), qppocr::Error> {
    let bytes = std::fs::read(path)?;
    let mut reader = ImageReader::new(std::io::Cursor::new(&bytes));
    let (w, h) = reader
        .with_guessed_format()
        .map_err(|e| qppocr::Error::Image(format!("读头部失败: {e}")))?
        .into_dimensions()
        .map_err(|e| qppocr::Error::Image(format!("读头部失败: {e}")))?;
    let transpose = matches!(
        exif_orientation(&bytes),
        Some(
            Orientation::Rotate90
                | Orientation::Rotate270
                | Orientation::Rotate90FlipH
                | Orientation::Rotate270FlipH
        )
    );
    Ok(if transpose { (h, w) } else { (w, h) })
}

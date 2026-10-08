//! EXIF 方向感知的图像解码 + 超大图防护。
//!
//! 相机竖拍照片带 Orientation 标签:浏览器渲染 <img> 时会自动旋转显示,
//! 而引擎解码原始像素不转 → 框坐标与显示坐标系对不上(框全错位)。
//! 这里在喂引擎前把像素转到"显示方向",画布/引擎/缩略图三方一致。
//!
//! 超大图(亿级像素拼接/扫描图)两道闸:
//! 1. 探测超 [`DECODE_MAX_PIXELS`] 直接拒——解码本身就要 w*h*3 字节;
//! 2. 超 [`ENGINE_MAX_PIXELS`] 解码后等比降采样再喂引擎。det 输入本就被
//!    长边帽压住、rec 裁剪在降采样分辨率下仍远超 48px 画布,精度无损;
//!    而喂整图会让引擎再复制一份 GB 级缓冲,小内存机器直接崩。
//!    引擎输出的框在「喂入图」坐标系,调用方用 [`DecodedImage::rescale_result`]
//!    放大回真原图(画布/纸张按原图铺)。

use image::metadata::Orientation;
use image::{ImageDecoder, ImageReader};
use std::path::Path;

/// 解码硬上限(像素):约 20 亿,RGB 字节 ≈ 6GB。超过拒绝并提示。
const DECODE_MAX_PIXELS: u64 = 2_000_000_000;
/// 引擎喂图上限(像素):约 3200 万(≈8K×4K),超过等比降采样。
const ENGINE_MAX_PIXELS: u64 = 32_000_000;

/// 解码产物:引擎可直接吃的图 + 框坐标回真原图的缩放系数。
pub struct DecodedImage {
    pub img: qppocr::Image,
    /// 喂入图 → 真原图的 x 缩放(未降采样 = 1.0)
    pub scale_x: f32,
    /// 喂入图 → 真原图的 y 缩放
    pub scale_y: f32,
}

impl DecodedImage {
    /// 把引擎输出的框(喂入图坐标)放大回真原图坐标。幂等安全(系数 1 时零开销)。
    /// 关联函数而非方法:`img` 已 move 进引擎后仍可用事先拷出的系数调用。
    pub fn rescale_result(res: &mut qppocr::OcrResult, sx: f32, sy: f32) {
        if sx == 1.0 && sy == 1.0 {
            return;
        }
        for line in &mut res.lines {
            for p in &mut line.pts {
                p[0] *= sx;
                p[1] *= sy;
            }
            for ch in &mut line.chars {
                for p in &mut ch.pts {
                    p[0] *= sx;
                    p[1] *= sy;
                }
            }
        }
    }
}

/// HEIF/HEIC 探测(苹果相机默认格式;iPhone 直传的照片扩展名常常还是 .jpg)。
/// `image` crate 无 HEIF 解码器:macOS 用系统 sips 转 JPEG,其余平台明确报错。
fn is_heic(bytes: &[u8]) -> bool {
    bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && matches!(
            &bytes[8..12],
            b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1" | b"hevc" | b"hevx"
        )
}

const HEIC_HINT: &str = "此文件是 HEIC/HEIF 格式(苹果相机默认,扩展名常伪装成 .jpg)。\
请导出或转换为 JPEG 后导入;macOS 版会自动转换";

/// macOS:系统自带 sips 把 HEIC 转成 JPEG(临时文件,识别完即弃)。
#[cfg(target_os = "macos")]
fn heic_to_jpeg(path: &Path) -> Result<std::path::PathBuf, qppocr::Error> {
    use std::process::Command;
    let out =
        std::env::temp_dir().join(format!("qpp-heic-{}.jpg", uuid::Uuid::new_v4().simple()));
    let st = Command::new("sips")
        .args(["-s", "format", "jpeg", "-s", "formatOptions", "95"])
        .arg(path)
        .arg("--out")
        .arg(&out)
        .output()
        .map_err(|e| qppocr::Error::Image(format!("调用 sips 转换 HEIC 失败: {e}")))?;
    if !st.status.success() || !out.is_file() {
        return Err(qppocr::Error::Image(format!(
            "HEIC 转换失败:{}",
            String::from_utf8_lossy(&st.stderr).trim()
        )));
    }
    Ok(out)
}

/// 按需把 HEIC 换成可解码的 JPEG 路径(仅 macOS);其余平台返回指引错误。
fn resolve_heic(path: &Path) -> Result<std::path::PathBuf, qppocr::Error> {
    // 只读 12 字节判型,大文件不整读
    let mut head = [0u8; 12];
    let readable = std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
        .is_ok();
    if !readable || !is_heic(&head) {
        return Ok(path.to_path_buf());
    }
    #[cfg(target_os = "macos")]
    return heic_to_jpeg(path);
    #[cfg(not(target_os = "macos"))]
    return Err(qppocr::Error::Image(HEIC_HINT.into()));
}

/// 解码并应用 EXIF Orientation(无标签/解码失败回退原图)。
pub fn decode_file_oriented(path: &Path) -> Result<DecodedImage, qppocr::Error> {
    let path = resolve_heic(path)?;
    // 整文件只读一次:超大图闸在字节上探测(曾为探测再读一遍全文件,
    // 100 张批量基准实测多耗 3s 级墙钟)
    let bytes = std::fs::read(&path)?;
    let (pw, ph) = probe_dims_from_bytes(&bytes)?;
    if (pw as u64) * (ph as u64) > DECODE_MAX_PIXELS {
        return Err(qppocr::Error::Image(format!(
            "图片过大:{pw}×{ph}(约 {:.1} 亿像素),超过 {:.0} 亿像素上限,请裁切或压缩后重试",
            pw as f64 * ph as f64 / 1e8,
            DECODE_MAX_PIXELS as f64 / 1e8
        )));
    }
    decode_bytes_oriented(&bytes)
}

pub fn decode_bytes_oriented(bytes: &[u8]) -> Result<DecodedImage, qppocr::Error> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| qppocr::Error::Image(format!("解码失败: {e}")))?;
    let mut img = img;
    if let Some(o) = exif_orientation(bytes) {
        img.apply_orientation(o); // 原地应用(无方向/180/翻转免拷贝)
    }
    // Rgb8 变体免拷贝取出,其余转 Rgb8(亿级像素下省一半峰值内存)
    let rgb = match img {
        image::DynamicImage::ImageRgb8(buf) => buf,
        other => other.to_rgb8(),
    };
    let (w, h) = rgb.dimensions();
    let (scale_x, scale_y, rgb) = if (w as u64) * (h as u64) > ENGINE_MAX_PIXELS {
        let s = (ENGINE_MAX_PIXELS as f64 / (w as f64 * h as f64)).sqrt();
        let nw = ((w as f64 * s).round() as u32).max(1);
        let nh = ((h as f64 * s).round() as u32).max(1);
        let small = image::imageops::resize(&rgb, nw, nh, image::imageops::FilterType::Triangle);
        eprintln!(
            "[ocr] 超大图 {w}×{h} 降采样喂引擎 → {nw}×{nh}(框坐标已自动还原)"
        );
        (w as f32 / nw as f32, h as f32 / nh as f32, small)
    } else {
        (1.0, 1.0, rgb)
    };
    let (nw, nh) = rgb.dimensions();
    Ok(DecodedImage {
        img: qppocr::rgb_from_bytes(nw, nh, rgb.into_raw())?,
        scale_x,
        scale_y,
    })
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
    let path = resolve_heic(path)?;
    let bytes = std::fs::read(&path)?;
    probe_dims_from_bytes(&bytes)
}

/// 在已读字节上探测"显示方向"宽高(EXIF 5-8 需要交换宽高)。
fn probe_dims_from_bytes(bytes: &[u8]) -> Result<(u32, u32), qppocr::Error> {
    let mut reader = ImageReader::new(std::io::Cursor::new(bytes));
    let (w, h) = reader
        .with_guessed_format()
        .map_err(|e| qppocr::Error::Image(format!("读头部失败: {e}")))?
        .into_dimensions()
        .map_err(|e| qppocr::Error::Image(format!("读头部失败: {e}")))?;
    let transpose = matches!(
        exif_orientation(bytes),
        Some(
            Orientation::Rotate90
                | Orientation::Rotate270
                | Orientation::Rotate90FlipH
                | Orientation::Rotate270FlipH
        )
    );
    Ok(if transpose { (h, w) } else { (w, h) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成一张超上限 JPEG,走完整 decode+降采样+框还原路径。
    #[test]
    fn oversize_downscales_and_rescales() {
        // 9000×4000 = 36MP > 32MP 上限
        let (w, h) = (9000u32, 4000u32);
        let dyn_img =
            image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
        let mut jpg = Vec::new();
        dyn_img
            .write_to(&mut std::io::Cursor::new(&mut jpg), image::ImageFormat::Jpeg)
            .unwrap();
        let d = decode_bytes_oriented(&jpg).expect("解码失败");
        let fed = d.img.w as u64 * d.img.h as u64;
        assert!(fed <= ENGINE_MAX_PIXELS, "喂入 {}px 超上限", fed);
        assert!(d.scale_x > 1.0 && d.scale_y > 1.0);

        // 框还原:喂入图坐标 × 系数 = 真原图坐标
        let mut res = qppocr::OcrResult::default();
        res.lines.push(qppocr::TextLine {
            text: "x".into(),
            chars: vec![], // CharSpan 未从门面 re-export,pts 循环与 line.pts 同构
            confidence: 1.0,
            rotation: 0,
            retried: false,
            pts: [[0.0, 0.0], [100.0, 0.0], [100.0, 40.0], [0.0, 40.0]],
        });
        DecodedImage::rescale_result(&mut res, d.scale_x, d.scale_y);
        let tr = res.lines[0].pts[1]; // TR,构造值 (100,0)
        assert!((tr[0] - 100.0 * d.scale_x).abs() < 0.5);
        assert!(tr[0] <= w as f32 + 1.0 && tr[1] <= h as f32 + 1.0);
    }

    /// HEIC(伪装 .jpg 的苹果格式)必须给出指引错误而不是"无法确定格式"。
    /// 用仓库根的 sfz-rx.jpg(真实 HEIC);CI 无此文件时跳过。
    #[test]
    fn heic_reports_hint() {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sfz-rx.jpg");
        if !p.is_file() {
            eprintln!("跳过(无样本):{}", p.display());
            return;
        }
        let mut head = [0u8; 12];
        std::io::Read::read_exact(
            &mut std::fs::File::open(&p).unwrap(),
            &mut head,
        )
        .unwrap();
        assert!(is_heic(&head), "样本应为 HEIC");
        let msg = match decode_file_oriented(&p) {
            Ok(_) => panic!("HEIC 在本平台不该解码成功"),
            Err(e) => e.to_string(),
        };
        assert!(msg.contains("HEIC"), "实际报错:{msg}");
    }
}

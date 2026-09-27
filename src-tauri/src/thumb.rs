//! 缩略图：256px JPEG，落盘 cache/thumbs/<id>.jpg，经 media:// 协议服务。
//! 生成搭识别解码的便车(一次解码两用)，不单独解码源文件；
//! 降采样用单趟块均值(不整幅拷贝、不走通用重采样管线——那要慢一个量级)。

use std::path::{Path, PathBuf};

/// 从已解码的 RGB 像素快速生成缩略图：整数步长块均值，单趟 O(像素数)。
pub fn make_thumb_from_rgb(
    w: u32,
    h: u32,
    rgb: &[u8],
    id: &str,
    thumbs_dir: &Path,
) -> Result<PathBuf, String> {
    let (tw, th, small) = downscale_box(w, h, rgb, 256);
    let img = image::RgbImage::from_raw(tw, th, small)
        .ok_or_else(|| "像素长度与尺寸不符".to_string())?;
    std::fs::create_dir_all(thumbs_dir).map_err(|e| e.to_string())?;
    let out = thumbs_dir.join(format!("{id}.jpg"));
    let file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
    let mut writer = std::io::BufWriter::new(file);
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, 82);
    img.write_with_encoder(encoder)
        .map_err(|e| format!("缩略图编码失败：{e}"))?;
    Ok(out)
}

/// 整数步长块均值降采样(保边保笔画,无整幅分配)。
fn downscale_box(w: u32, h: u32, rgb: &[u8], max_side: u32) -> (u32, u32, Vec<u8>) {
    if w == 0 || h == 0 || rgb.len() < (w as usize) * (h as usize) * 3 {
        return (1, 1, vec![0, 0, 0]);
    }
    let step = ((w.max(h) + max_side - 1) / max_side).max(1);
    let tw = (w / step).max(1);
    let th = (h / step).max(1);
    let mut small = vec![0u8; (tw as usize) * (th as usize) * 3];
    for ty in 0..th {
        let y0 = ty * step;
        let y1 = ((ty + 1) * step).min(h);
        for tx in 0..tw {
            let x0 = tx * step;
            let x1 = ((tx + 1) * step).min(w);
            let (mut ar, mut ag, mut ab, mut cnt) = (0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                let row = (y as usize) * (w as usize) * 3;
                for x in x0..x1 {
                    let i = row + (x as usize) * 3;
                    ar += rgb[i] as u32;
                    ag += rgb[i + 1] as u32;
                    ab += rgb[i + 2] as u32;
                    cnt += 1;
                }
            }
            let o = ((ty as usize) * (tw as usize) + tx as usize) * 3;
            small[o] = (ar / cnt) as u8;
            small[o + 1] = (ag / cnt) as u8;
            small[o + 2] = (ab / cnt) as u8;
        }
    }
    (tw, th, small)
}

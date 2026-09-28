//! PDF 支持:光栅化 + 页面探测。
//!
//! 使用 pdfium-render(封装 Google PDFium,Chrome 同款引擎)。
//! static feature 编译期链接,无需外部 DLL。
//! 每次渲染时从字节流重新打开——PDFium 内部走内存映射,开销可忽略。

use std::path::Path;

use pdfium_render::prelude::*;

/// PDF 渲染分辨率(pt → px 按 DPI/72 换算)
pub const DPI_THUMB: u16 = 96;
pub const DPI_VIEW: u16 = 150;
pub const DPI_OCR: u16 = 200;

lazy_static::lazy_static! {
    static ref PDFIUM: Pdfium = Pdfium::new(Pdfium::bind_to_system_library().expect("PDFium 库未找到"));
}

/// 探测 PDF 页数(只读元数据,不解码像素)。
pub fn page_count(bytes: &[u8]) -> Result<u32, String> {
    let doc = PDFIUM
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| format!("PDF 加载失败: {e}"))?;
    Ok(doc.pages().len() as u32)
}

/// 渲染指定页,返回 (width, height, RGB 字节)。
/// PDFium 输出 BGRA,这里转 RGB 供引擎使用。
pub fn render_page(
    bytes: &[u8],
    page_index: u32,
    dpi: u16,
) -> Result<(u32, u32, Vec<u8>), String> {
    let doc = PDFIUM
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| format!("PDF 加载失败: {e}"))?;
    let page = doc
        .pages()
        .get(page_index as i32)
        .map_err(|e| format!("第 {} 页不存在: {e}", page_index + 1))?;

    let w_pt = page.width().value;
    let h_pt = page.height().value;
    let target_w = ((w_pt * dpi as f32 / 72.0) as i32).max(100);
    let max_h = ((h_pt * dpi as f32 / 72.0) as i32).max(100);

    let render = page
        .render_with_config(
            &PdfRenderConfig::new()
                .set_target_width(target_w)
                .set_maximum_height(max_h),
        )
        .map_err(|e| format!("第 {} 页渲染失败: {e}", page_index + 1))?;
    let bitmap = render;
    let w = bitmap.width() as u32;
    let h = bitmap.height() as u32;

    // BGRA → RGB
    let bgra = bitmap.as_raw_bytes();
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for px in bgra.chunks_exact(4) {
        rgb.push(px[2]);
        rgb.push(px[1]);
        rgb.push(px[0]);
    }
    Ok((w, h, rgb))
}

/// 判断 PDF 是否包含文本层(数字原生,非扫描件)。
pub fn has_text_layer(bytes: &[u8]) -> bool {
    let Ok(doc) = PDFIUM.load_pdf_from_byte_slice(bytes, None) else {
        return false;
    };
    let pages_to_check = doc.pages().len().min(3);
    for i in 0..pages_to_check {
        if let Ok(page) = doc.pages().get(i) {
            if let Ok(text) = page.text() {
                let content = text.all();
                if content.chars().filter(|c| !c.is_whitespace()).count() > 10 {
                    return true;
                }
            }
        }
    }
    false
}

/// 从 PDF 文件读字节。
pub fn read_pdf(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("读取 PDF 失败: {e}"))
}

/// 从 PDF 文本层直接提取(跳过 OCR,毫秒级)。
/// 返回 (text, confidence, [四点坐标]) — 和 OCR TextLine 对齐。
/// 坐标为全宽近似框(精确定位需要字符矩阵,此处取可用性优先)。
pub fn extract_text_lines(
    bytes: &[u8],
    page_index: u32,
    dpi: u16,
) -> Result<Vec<(String, f32, [[f32; 2]; 4])>, String> {
    let doc = PDFIUM
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| format!("PDF 加载失败: {e}"))?;
    let page = doc
        .pages()
        .get(page_index as i32)
        .map_err(|e| format!("第 {} 页不存在: {e}", page_index + 1))?;

    let page_w = page.width().value * dpi as f32 / 72.0;
    let page_h = page.height().value * dpi as f32 / 72.0;

    let text = page
        .text()
        .map_err(|e| format!("文本提取失败: {e}"))?;

    let all = text.all();
    let mut lines = Vec::new();
    let mut y = 0f32;
    for line in all.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            let line_h = 16f32; // 近似行高,不参与精确命中
            lines.push((
                trimmed.to_string(),
                1.0,
                [
                    [0.0, y + line_h],
                    [page_w, y + line_h],
                    [page_w, y],
                    [0.0, y],
                ],
            ));
            y += line_h + 4.0;
        } else {
            y += 8.0;
        }
    }
    let _ = page_h;
    Ok(lines)
}

//! 统一入库：图片 / PDF / 剪贴板 / 截图 → ImageItem。
//!
//! PDF 策略:拖入时后台渲染全部页为 PNG,每页创建普通条目,
//! 全走现有批量识别管线。用户翻页 = 切换条目,内容即时可见。

use crate::dto::ImageItemDto;
use crate::media::mime_for;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

lazy_static::lazy_static! {
    /// PDF 原始字节(元信息查询用)
    pub static ref PDF_STORE: Mutex<HashMap<String, Vec<u8>>> =
        Mutex::new(HashMap::new());
    /// PDF 元信息:id → (总页数, 当前页)
    pub static ref PDF_PAGES: Mutex<HashMap<String, (u32, u32)>> =
        Mutex::new(HashMap::new());
}

#[derive(Clone)]
pub struct ImageItem {
    pub id: String,
    pub name: String,
    /// 可显示/可解码的图片路径
    pub path: PathBuf,
    pub w: u32,
    pub h: u32,
    pub origin: String,
    pub added_at: u64,
    pub media_token: String,
    pub thumb_token: String,
    pub can_extract: bool,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn valid_ext(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "bmp" | "pdf")
    )
}

/// 渲染 PDF 某页 → PNG 落盘 → 返回 (路径, 宽, 高, 媒体令牌, 缩略图令牌)
fn render_pdf_page_to_item(
    app: &AppHandle,
    bytes: &[u8],
    page: u32,
    item_id: &str,
) -> Result<(PathBuf, u32, u32, String, String), String> {
    let state = app.state::<crate::AppCtx>();
    let (w, h, rgb) = crate::pdf::render_page(bytes, page, crate::pdf::DPI_VIEW)?;
    let png_path = state.dirs.inbox.join(format!("{item_id}_p{page}.png"));
    let img = image::RgbImage::from_raw(w, h, rgb)
        .ok_or("渲染数据无效")?;
    img.save_with_format(&png_path, image::ImageFormat::Png)
        .map_err(|e| format!("保存页面失败: {e}"))?;
    let media_token = state.media.register(png_path.clone(), "image/png");

    // 缩略图
    let thumb_token = match crate::pdf::render_page(bytes, page, crate::pdf::DPI_THUMB) {
        Ok((tw, th, trgb)) => {
            let tp = state.dirs.thumbs.join(format!("{item_id}.jpg"));
            if let Some(ti) = image::RgbImage::from_raw(tw, th, trgb) {
                if ti.save_with_format(&tp, image::ImageFormat::Jpeg).is_ok() {
                    state.media.register(tp, "image/jpeg")
                } else { String::new() }
            } else { String::new() }
        }
        Err(_) => String::new(),
    };
    Ok((png_path, w, h, media_token, thumb_token))
}

pub fn ingest_file(app: &AppHandle, path: &Path, origin: &str) -> Result<ImageItemDto, String> {
    if !path.is_file() {
        return Err(format!("文件不存在: {}", path.display()));
    }
    if !valid_ext(path) {
        return Err(format!("不支持的格式: {}", path.display()));
    }

    let state = app.state::<crate::AppCtx>();
    let is_pdf = path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false);

    if is_pdf {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let bytes = std::fs::read(path).map_err(|e| format!("读取 PDF 失败: {e}"))?;
        let count = crate::pdf::page_count(&bytes)?;
        let can_extract = crate::pdf::has_text_layer(&bytes);
        let file_name = path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "PDF".into());

        // 存 PDF 元信息(后台 OCR 由 pdf_ocr_range 命令负责,不在入库时触发)
        let bytes_ref = PDF_STORE.lock().unwrap();
        let bytes_for_render = bytes_ref.get(&id).cloned().unwrap_or_default();
        drop(bytes_ref);
        PDF_PAGES.lock().unwrap().insert(id.clone(), (count, 0));

        // 同步渲染第 1 页(用户立刻看到内容)
        let (png_path, w, h, media_token, thumb_token) =
            render_pdf_page_to_item(app, &bytes_for_render, 0, &id)?;
        let item = ImageItem {
            id: id.clone(),
            name: file_name,
            path: png_path,
            w, h,
            origin: "pdf".into(),
            added_at: now_ms(),
            media_token,
            thumb_token,
            can_extract,
        };
        state.register_item(item);

        Ok(state.item_dto(&id).expect("刚插入的条目"))
    } else {
        // ---- 普通图片 ----
        let id = uuid::Uuid::new_v4().simple().to_string();
        let (w, h) = crate::image_util::probe_dims_oriented(path)
            .map_err(|e| format!("读取图片尺寸失败: {e}"))?;
        let media_token = state.media.register(path.to_path_buf(), mime_for(path));
        let item = ImageItem {
            id: id.clone(),
            name: path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "未命名".into()),
            path: path.to_path_buf(),
            w, h,
            origin: origin.into(),
            added_at: now_ms(),
            media_token,
            thumb_token: String::new(),
            can_extract: false,
        };
        state.register_item(item);
        Ok(state.item_dto(&id).expect("刚插入的条目"))
    }
}

/// 位图入库(剪贴板/截图)
pub fn ingest_bitmap(
    app: &AppHandle,
    name: &str,
    img: image::RgbaImage,
    origin: &str,
) -> Result<ImageItemDto, String> {
    let state = app.state::<crate::AppCtx>();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let png_path = state.dirs.inbox.join(format!("{id}.png"));
    img.save_with_format(&png_path, image::ImageFormat::Png)
        .map_err(|e| format!("保存图片失败: {e}"))?;
    let mut dto = ingest_file(app, &png_path, origin)?;
    dto.name = name.to_string();
    if let Some(item) = state.items.write().unwrap().get_mut(&dto.id) {
        item.name = name.to_string();
    }
    Ok(dto)
}

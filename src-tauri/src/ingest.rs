//! 统一入库：文件 / 剪贴板位图 / 截图裁剪 / PDF → ImageItem。
//!
//! 设计原则:**条目的 path 和 media_token 永远指向可在画布显示的图片文件**。
//! PDF 在入库时渲染第 1 页为 PNG 落盘,条目指向 PNG;后续翻页同样渲染→落盘→更新令牌。

use crate::dto::ImageItemDto;
use crate::media::mime_for;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

lazy_static::lazy_static! {
    /// PDF 原始字节(翻页渲染用)
    pub static ref PDF_STORE: Mutex<HashMap<String, Vec<u8>>> =
        Mutex::new(HashMap::new());
    /// PDF 元信息:id → (总页数, 当前显示页)
    pub static ref PDF_PAGES: Mutex<HashMap<String, (u32, u32)>> =
        Mutex::new(HashMap::new());
}

#[derive(Clone)]
pub struct ImageItem {
    pub id: String,
    pub name: String,
    /// 可显示/可解码的图片路径(PDF 条目指向渲染后的 PNG)
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

fn valid_image_ext(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "bmp" | "pdf")
    )
}

/// PDF 渲染当前页 → PNG 落盘 → 更新条目路径和令牌。
/// 翻页和入库共用此函数。
pub fn pdf_update_page(app: &AppHandle, id: &str, page: u32) -> Result<(u32, u32), String> {
    // 先取字节(释放锁再做重活,避免长时间持锁)
    let bytes = {
        let store = PDF_STORE.lock().unwrap();
        store.get(id).cloned().ok_or("PDF 条目不存在")?
    };
    let total = {
        let pages = PDF_PAGES.lock().unwrap();
        pages.get(id).map(|p| p.0).unwrap_or(1)
    };
    // 渲染(重活)
    let (w, h, rgb) = crate::pdf::render_page(&bytes, page, crate::pdf::DPI_VIEW)?;
    let state = app.state::<crate::AppCtx>();
    let png_path = state.dirs.inbox.join(format!("{id}_p{page}.png"));
    let img = image::RgbImage::from_raw(w, h, rgb)
        .ok_or("渲染数据无效")?;
    img.save_with_format(&png_path, image::ImageFormat::Png)
        .map_err(|e| format!("保存失败: {e}"))?;

    // 更新条目(短锁)
    {
        let token = state.media.register(png_path.clone(), "image/png");
        let mut items = state.items.write().unwrap();
        if let Some(item) = items.get_mut(id) {
            item.path = png_path;
            item.media_token = token;
            item.w = w;
            item.h = h;
        }
    }
    // 更新页码(短锁,避免嵌套)
    PDF_PAGES.lock().unwrap().insert(id.to_string(), (total, page));
    Ok((w, h))
}

pub fn ingest_file(app: &AppHandle, path: &Path, origin: &str) -> Result<ImageItemDto, String> {
    if !path.is_file() {
        return Err(format!("文件不存在: {}", path.display()));
    }
    if !valid_image_ext(path) {
        return Err(format!("不支持的格式: {}", path.display()));
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let state = app.state::<crate::AppCtx>();

    let is_pdf = path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false);

    if is_pdf {
        // ---- PDF:渲染第 1 页成 PNG,path/media_token 指向 PNG ----
        let bytes = std::fs::read(path).map_err(|e| format!("读取 PDF 失败: {e}"))?;
        let count = crate::pdf::page_count(&bytes)?;
        let can_extract = crate::pdf::has_text_layer(&bytes);
        PDF_STORE.lock().unwrap().insert(id.clone(), bytes);
        PDF_PAGES.lock().unwrap().insert(id.clone(), (count, 0));

        // 渲染第 1 页
        let (w, h, rgb) = crate::pdf::render_page(
            &PDF_STORE.lock().unwrap().get(&id).unwrap(), 0, crate::pdf::DPI_VIEW,
        )?;
        let png_path = state.dirs.inbox.join(format!("{id}_p0.png"));
        let img = image::RgbImage::from_raw(w, h, rgb)
            .ok_or_else(|| "渲染数据无效".to_string())?;
        img.save_with_format(&png_path, image::ImageFormat::Png)
            .map_err(|e| format!("保存页面图片失败: {e}"))?;
        let media_token = state.media.register(png_path.clone(), "image/png");

        let item = ImageItem {
            id: id.clone(),
            name: path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "PDF".into()),
            path: png_path,
            w, h,
            origin: "pdf".into(),
            added_at: now_ms(),
            media_token,
            thumb_token: String::new(),
            can_extract,
        };
        state.register_item(item);
        Ok(state.item_dto(&id).expect("刚插入的条目"))
    } else {
        // ---- 普通图片:零解码,只读头部 ----
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

/// 位图入库(剪贴板/截图):保存 PNG 到 inbox 再走文件路径。
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

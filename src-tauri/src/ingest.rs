//! 统一入库：文件 / 剪贴板位图 / 截图裁剪 → ImageItem（+缩略图+media 令牌）。
//!
//! 文件类不拷贝（引用原路径，源删除时优雅降级）；无源文件的位图
//! （剪贴板/截图）保存 PNG 到 cache/inbox 以便历史重开。

use crate::dto::ImageItemDto;
use crate::media::mime_for;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Clone)]
pub struct ImageItem {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub w: u32,
    pub h: u32,
    pub origin: String,
    pub added_at: u64,
    pub media_token: String,
    pub thumb_token: String,
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
        Some("png" | "jpg" | "jpeg" | "bmp")
    )
}

/// 文件入库:**零解码**——只读头部探测宽高(毫秒级),列表立即可见、识别立刻开始。
/// 缩略图在识别解码时搭车生成(见 batch/thumb),不单独花一遍解码。
pub fn ingest_file(app: &AppHandle, path: &Path, origin: &str) -> Result<ImageItemDto, String> {
    if !path.is_file() {
        return Err(format!("文件不存在: {}", path.display()));
    }
    if !valid_image_ext(path) {
        return Err(format!("不支持的图片格式: {}", path.display()));
    }
    let (w, h) = crate::image_util::probe_dims_oriented(path)
        .map_err(|e| format!("读取图片尺寸失败: {e}"))?;

    let state = app.state::<crate::AppCtx>();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let media_token = state.media.register(path.to_path_buf(), mime_for(path));

    let item = ImageItem {
        id: id.clone(),
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "未命名".into()),
        path: path.to_path_buf(),
        w,
        h,
        origin: origin.into(),
        added_at: now_ms(),
        media_token,
        thumb_token: String::new(), // 识别后由 item-done 事件带回
    };
    state.register_item(item);
    Ok(state.item_dto(&id).expect("刚插入的条目"))
}

/// 位图入库（剪贴板 / 截图裁剪）：先落盘 PNG 再走文件路径，历史可重开。
/// `name` 用作显示名（文件名保持 uuid 以保证唯一）。
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

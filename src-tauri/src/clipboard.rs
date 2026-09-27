//! 剪贴板：arboard 读图（规范化为 RGBA、top-down）+ 写文本。
//! 用 arboard 直连而不引 clipboard-manager 插件，省一层体积。

pub fn read_image() -> Result<image::RgbaImage, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("打开剪贴板失败: {e}"))?;
    let img = cb
        .get_image()
        .map_err(|e| format!("剪贴板里没有图片: {e}"))?;
    image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
        .ok_or_else(|| "剪贴板图片尺寸无效".to_string())
}

pub fn set_text(text: &str) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("打开剪贴板失败: {e}"))?;
    cb.set_text(text.to_string())
        .map_err(|e| format!("写入剪贴板失败: {e}"))
}

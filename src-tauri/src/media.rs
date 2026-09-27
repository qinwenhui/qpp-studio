//! `media://` 自定义协议：图片字节永不走 IPC/base64。
//!
//! 命令只返回 uuid 令牌，前端用令牌拼 URL 由 webview 直接加载；
//! 协议端只认令牌查表 → 读盘 → 按 mime 响应。零路径穿越风险、零 scope 配置。
//!
//! 平台差异（wry 已知行为）：Windows/WebView2 上自定义协议映射为
//! `http://media.localhost/<token>`，macOS/Linux 为 `media://localhost/<token>`。
//! 前端 `mediaUrl()` 负责分支，CSP 两种写法都放行。

use std::{borrow::Cow, collections::HashMap, path::PathBuf, sync::Mutex};
use tauri::{http, Manager, Runtime, UriSchemeContext, UriSchemeResponder};

#[derive(Clone)]
pub struct MediaEntry {
    pub path: PathBuf,
    pub mime: &'static str,
}

#[derive(Default)]
pub struct MediaRegistry {
    entries: Mutex<HashMap<String, MediaEntry>>,
    /// 自测/诊断:协议实际命中的次数
    hit_count: std::sync::atomic::AtomicUsize,
}

impl MediaRegistry {
    pub fn hits(&self) -> usize {
        self.hit_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl MediaRegistry {
    pub fn register(&self, path: PathBuf, mime: &'static str) -> String {
        let token = uuid::Uuid::new_v4().simple().to_string();
        self.entries
            .lock()
            .unwrap()
            .insert(token.clone(), MediaEntry { path, mime });
        token
    }

    pub fn lookup(&self, token: &str) -> Option<MediaEntry> {
        self.entries.lock().unwrap().get(token).cloned()
    }
}

pub fn mime_for(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") | Some("bmp") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}

/// 协议 handler。token 唯一且不可猜，可放心长缓存。
pub fn handle<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let token = request.uri().path().trim_start_matches('/');
    let registry = &ctx.app_handle().state::<crate::AppCtx>().media;
    let response = match registry.lookup(token) {
        Some(entry) => match std::fs::read(&entry.path) {
            Ok(bytes) => {
                registry
                    .hit_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                http::Response::builder()
                    .header(http::header::CONTENT_TYPE, entry.mime)
                    .header(http::header::CACHE_CONTROL, "public, max-age=31536000, immutable")
                    .body(Cow::Owned(bytes))
                    .unwrap()
            }
            Err(e) => {
                eprintln!("[media] 读文件失败 {}: {e}", entry.path.display());
                not_found()
            }
        },
        None => {
            eprintln!("[media] 未知 token: {token}");
            not_found()
        }
    };
    responder.respond(response);
}

fn not_found() -> http::Response<Cow<'static, [u8]>> {
    http::Response::builder()
        .status(404)
        .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Cow::from(&b"media token not found\n"[..]))
        .unwrap()
}

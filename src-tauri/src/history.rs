//! 历史记录:JSONL 追加式持久化(上限 200 条)。
//! 曾经的"整体重写"是 O(N²):批量中途每 2s 重序列化全部历史(含完整结果,
//! 可达数 MB),批量中途反复重写会烧掉可观的 CPU。
//! 现在常规路径只追加新行;删除/清空/截断才整体重写一次。

use crate::dto::{text_preview, HistoryEntryDto, OcrOutcomeDto};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

const MAX_ENTRIES: usize = 200;

pub struct HistoryStore {
    inner: Mutex<Vec<HistoryEntryDto>>,
    /// 待追加的行(增量落盘)
    pending: Mutex<Vec<String>>,
    /// 需要整体重写(删除/清空/截断后)
    needs_rewrite: AtomicBool,
    path: PathBuf,
}

impl HistoryStore {
    pub fn load(path: PathBuf) -> Self {
        let raw = std::fs::read_to_string(&path).ok();
        // 兼容两种格式:旧 JSON 数组 / JSONL
        let mut entries: Vec<HistoryEntryDto> = raw
            .as_deref()
            .and_then(|s| {
                serde_json::from_str::<Vec<HistoryEntryDto>>(s)
                    .ok()
                    .or_else(|| parse_jsonl(s))
            })
            .unwrap_or_default();
        if raw.is_some() && entries.is_empty() {
            let _ = std::fs::rename(&path, path.with_extension("old.json"));
        }
        // 按 id 去重(保留最新)
        let mut seen = std::collections::HashSet::new();
        entries.retain(|e| seen.insert(e.id.clone()));
        Self {
            inner: Mutex::new(entries),
            pending: Mutex::new(Vec::new()),
            needs_rewrite: AtomicBool::new(false),
            path,
        }
    }

    /// 按 id 去重:同一张图重新识别 = 更新原条目并置顶,而不是追加重复。
    pub fn append(&self, entry: HistoryEntryDto) {
        let mut list = self.inner.lock().unwrap();
        let existed = list.iter().any(|e| e.id == entry.id);
        list.retain(|e| e.id != entry.id);
        list.insert(0, entry.clone());
        let truncated = list.len() > MAX_ENTRIES;
        list.truncate(MAX_ENTRIES);
        drop(list);

        let mut pend = self.pending.lock().unwrap();
        // 更新已存在条目也走重写(简单起见;重新识别是低频操作)
        if existed || truncated {
            self.needs_rewrite.store(true, Ordering::Release);
            pend.clear();
        } else {
            if let Ok(line) = serde_json::to_string(&entry) {
                pend.push(line);
            }
        }
    }

    pub fn list(&self, offset: usize, limit: usize) -> Vec<HistoryEntryDto> {
        let list = self.inner.lock().unwrap();
        list.iter().skip(offset).take(limit).cloned().collect()
    }

    pub fn delete(&self, id: &str) {
        self.inner.lock().unwrap().retain(|e| e.id != id);
        self.needs_rewrite.store(true, Ordering::Release);
        self.pending.lock().unwrap().clear();
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
        self.needs_rewrite.store(true, Ordering::Release);
        self.pending.lock().unwrap().clear();
    }

    pub fn get(&self, id: &str) -> Option<HistoryEntryDto> {
        self.inner.lock().unwrap().iter().find(|e| e.id == id).cloned()
    }

    pub fn flush_if_dirty(&self) {
        if self.needs_rewrite.swap(false, Ordering::AcqRel) {
            let jsonl = {
                let list = self.inner.lock().unwrap();
                list.iter()
                    .filter_map(|e| serde_json::to_string(e).ok())
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            write_atomic(&self.path, jsonl);
            self.pending.lock().unwrap().clear();
            return;
        }
        let lines = {
            let mut pend = self.pending.lock().unwrap();
            if pend.is_empty() {
                return;
            }
            std::mem::take(&mut *pend)
        };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            for line in lines {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

fn parse_jsonl(s: &str) -> Option<Vec<HistoryEntryDto>> {
    let mut out = Vec::new();
    for line in s.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let e: HistoryEntryDto = serde_json::from_str(line).ok()?;
        out.push(e);
    }
    Some(out)
}

fn write_atomic(path: &std::path::Path, content: String) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, content).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// 由识别结果构造历史条目。
pub fn entry_from(
    item: &crate::ingest::ImageItem,
    outcome: &OcrOutcomeDto,
) -> HistoryEntryDto {
    let line_count = outcome
        .result
        .as_ref()
        .map(|r| r.lines.len() as u32)
        .unwrap_or(0);
    let total_ms = outcome
        .result
        .as_ref()
        .map(|r| r.timings.total_ms)
        .unwrap_or(0.0);
    HistoryEntryDto {
        id: item.id.clone(),
        name: item.name.clone(),
        path: item.path.to_string_lossy().into_owned(),
        w: item.w,
        h: item.h,
        origin: item.origin.clone(),
        at: item.added_at,
        line_count,
        total_ms,
        text_preview: text_preview(outcome, 200),
        outcome: outcome.clone(),
        // 令牌是进程内的,重启即失效,不落盘;列表时按 thumbs/<id>.jpg 现注册。
        // 注意别写成 Some(item.thumb_token):空串会被前端 `{#if e.thumbToken}`
        // 判为假值,整列缩略图都会退化成占位图标。
        thumb_token: None,
    }
}

/// 每 2 秒检查一次脏标记的落盘线程。
pub fn spawn_flush_thread(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let state = app.state::<crate::AppCtx>();
        state.history.flush_if_dirty();
    });
}

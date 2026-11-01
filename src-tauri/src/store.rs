//! 历史记录与设置的内存模型及持久化（exe 同目录 data/）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ImgMeta {
    pub w: u32,
    pub h: u32,
    pub file: String,
    pub thumb: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Item {
    pub id: u64,
    /// text | image | files
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub img: Option<ImgMeta>,
    pub hash: String,
    #[serde(default)]
    pub pinned: bool,
    pub created: i64,
    pub used: i64,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub truncated: bool,
}

/// 发给前端列表的精简版本：文本截断，不带 HTML。
#[derive(Serialize)]
pub struct ItemDto {
    pub id: u64,
    pub kind: String,
    pub text: String,
    pub len: usize,
    pub has_html: bool,
    pub files: Vec<String>,
    pub img: Option<ImgMeta>,
    pub pinned: bool,
    pub created: i64,
    pub used: i64,
    pub source: String,
    pub truncated: bool,
}

const LIST_TEXT_CHARS: usize = 4000;

impl Item {
    pub fn dto(&self, full: bool) -> ItemDto {
        let len = self.text.chars().count();
        let text = if full || len <= LIST_TEXT_CHARS {
            self.text.clone()
        } else {
            self.text.chars().take(LIST_TEXT_CHARS).collect()
        };
        ItemDto {
            id: self.id,
            kind: self.kind.clone(),
            text,
            len,
            has_html: self.html.is_some(),
            files: self.files.clone(),
            img: self.img.clone(),
            pinned: self.pinned,
            created: self.created,
            used: self.used,
            source: self.source.clone(),
            truncated: self.truncated,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub max_items: usize,
    pub hotkey: String,
    /// system | light | dark
    pub theme: String,
    pub accent: String,
    pub reduce_motion: bool,
    pub autostart: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_items: 100,
            hotkey: "Alt+V".into(),
            theme: "system".into(),
            accent: "violet".into(),
            reduce_motion: false,
            autostart: false,
        }
    }
}

pub struct Store {
    pub items: Vec<Item>,
    next_id: u64,
    dir: PathBuf,
    pub settings: Settings,
    pub paused: bool,
}

/// 优先使用 exe 同目录的 data/（便携）；不可写时退回 %APPDATA%\Clipora。
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("data")))
    {
        if fs::create_dir_all(&d).is_ok() {
            let probe = d.join(".probe");
            if fs::write(&probe, b"1").is_ok() {
                let _ = fs::remove_file(probe);
                return d;
            }
        }
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join("Clipora");
    let _ = fs::create_dir_all(&d);
    d
}

fn write_atomic(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, bytes).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

impl Store {
    pub fn load(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(dir.join("images"));
        let items: Vec<Item> = fs::read(dir.join("history.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let settings: Settings = fs::read(dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let next_id = items.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        let mut s = Self { items, next_id, dir, settings, paused: false };
        s.settings.max_items = s.settings.max_items.clamp(20, 500);
        s.enforce_limit();
        s
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn img_dir(&self) -> PathBuf {
        self.dir.join("images")
    }

    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn move_to_front(&mut self, pos: usize) {
        let it = self.items.remove(pos);
        self.items.insert(0, it);
    }

    /// 已存在相同内容时移到最前，返回其 id。
    pub fn touch_hash(&mut self, hash: &str, source: &str) -> Option<u64> {
        let pos = self.items.iter().position(|i| i.hash == hash)?;
        let it = &mut self.items[pos];
        it.used = now_ms();
        if !source.is_empty() {
            it.source = source.to_string();
        }
        let id = it.id;
        self.move_to_front(pos);
        Some(id)
    }

    pub fn touch(&mut self, id: u64) -> Option<Item> {
        let pos = self.items.iter().position(|i| i.id == id)?;
        self.items[pos].used = now_ms();
        self.move_to_front(pos);
        Some(self.items[0].clone())
    }

    pub fn get(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn insert(&mut self, item: Item) {
        self.items.insert(0, item);
        self.enforce_limit();
    }

    /// 未固定条目超过上限时，从最旧的开始删除。
    pub fn enforce_limit(&mut self) {
        let max = self.settings.max_items;
        let mut unpinned = self.items.iter().filter(|i| !i.pinned).count();
        let mut i = self.items.len();
        while unpinned > max && i > 0 {
            i -= 1;
            if !self.items[i].pinned {
                let it = self.items.remove(i);
                self.delete_files(&it);
                unpinned -= 1;
            }
        }
    }

    fn delete_files(&self, it: &Item) {
        if let Some(img) = &it.img {
            let d = self.img_dir();
            let _ = fs::remove_file(d.join(&img.file));
            let _ = fs::remove_file(d.join(&img.thumb));
        }
    }

    pub fn remove(&mut self, id: u64) -> bool {
        match self.items.iter().position(|i| i.id == id) {
            Some(pos) => {
                let it = self.items.remove(pos);
                self.delete_files(&it);
                true
            }
            None => false,
        }
    }

    pub fn clear_unpinned(&mut self) {
        let (keep, drop): (Vec<Item>, Vec<Item>) =
            std::mem::take(&mut self.items).into_iter().partition(|i| i.pinned);
        for it in &drop {
            self.delete_files(it);
        }
        self.items = keep;
    }

    pub fn toggle_pin(&mut self, id: u64) -> Option<bool> {
        let it = self.items.iter_mut().find(|i| i.id == id)?;
        it.pinned = !it.pinned;
        let pinned = it.pinned;
        self.enforce_limit();
        Some(pinned)
    }

    pub fn save(&self) {
        if let Ok(b) = serde_json::to_vec(&self.items) {
            write_atomic(&self.dir.join("history.json"), &b);
        }
    }

    pub fn save_settings(&self) {
        if let Ok(b) = serde_json::to_vec_pretty(&self.settings) {
            write_atomic(&self.dir.join("settings.json"), &b);
        }
    }
}

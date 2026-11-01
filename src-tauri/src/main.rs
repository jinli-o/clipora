#![windows_subsystem = "windows"]

mod store;
mod win;

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde_json::json;
use store::{now_ms, ImgMeta, Item, ItemDto, Settings, Store};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// 卡片列表宽度 400 + 两侧阴影留白 16。
const BASE_W: f64 = 432.0;
const PREVIEW_W: f64 = 832.0;
const WIN_H: f64 = 600.0;
const MAX_TEXT_BYTES: usize = 1_000_000;
const MAX_PIXELS: u64 = 50_000_000;
const THUMB_W: u32 = 360;

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
static APP: OnceLock<AppHandle> = OnceLock::new();
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static PAUSE_ITEM: OnceLock<CheckMenuItem<Wry>> = OnceLock::new();

fn store() -> MutexGuard<'static, Store> {
    STORE.get().expect("store").lock().unwrap_or_else(|e| e.into_inner())
}

fn fnv(parts: &[&[u8]]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in parts {
        for b in p.iter() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn emit_changed(new_id: Option<u64>) {
    if let Some(app) = APP.get() {
        let _ = app.emit("items-changed", json!({ "new": new_id }));
    }
}

// ---------- 剪贴板采集 ----------

pub fn on_clipboard_update() {
    if win::is_self_write() || store().paused {
        return;
    }
    let source = win::clipboard_source();
    let id = match win::capture() {
        win::Captured::Skip => None,
        win::Captured::Text { mut text, html } => {
            let mut truncated = false;
            if text.len() > MAX_TEXT_BYTES {
                let mut cut = MAX_TEXT_BYTES;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
                truncated = true;
            }
            let hash = fnv(&[b"t", text.as_bytes()]);
            add(hash, &source, |id| Item {
                id,
                kind: "text".into(),
                text,
                html: if truncated { None } else { html },
                files: vec![],
                img: None,
                hash: String::new(),
                pinned: false,
                created: now_ms(),
                used: now_ms(),
                source: source.clone(),
                truncated,
            })
        }
        win::Captured::Files(files) => match image_file(&files).and_then(|p| image::open(p).ok()) {
            // 单个图片文件（QQ、微信、资源管理器复制图片时就是这种格式）按图片记录
            Some(img) => save_image(img.to_rgba8(), &source, files),
            None => add_files(files, &source),
        },
        win::Captured::Image => capture_image(&source),
    };
    if let Some(id) = id {
        store().save();
        emit_changed(Some(id));
    }
}

const IMG_EXTS: [&str; 7] = ["png", "jpg", "jpeg", "webp", "gif", "bmp", "jfif"];
const MAX_IMG_FILE: u64 = 40 * 1024 * 1024;

/// 剪贴板中恰好是一个图片文件时返回其路径。
fn image_file(files: &[String]) -> Option<&str> {
    let [f] = files else { return None };
    let p = std::path::Path::new(f);
    let ext = p.extension()?.to_string_lossy().to_lowercase();
    let size = std::fs::metadata(p).ok()?.len();
    (IMG_EXTS.contains(&ext.as_str()) && size <= MAX_IMG_FILE).then_some(f.as_str())
}

fn add_files(files: Vec<String>, source: &str) -> Option<u64> {
    let joined = files.join("\n");
    let hash = fnv(&[b"f", joined.as_bytes()]);
    add(hash, source, |id| Item {
        id,
        kind: "files".into(),
        text: joined,
        html: None,
        files,
        img: None,
        hash: String::new(),
        pinned: false,
        created: now_ms(),
        used: now_ms(),
        source: source.to_string(),
        truncated: false,
    })
}

/// 去重后插入：已存在则移到最前，否则用 make 构造新条目。
fn add(hash: String, source: &str, make: impl FnOnce(u64) -> Item) -> Option<u64> {
    let mut s = store();
    if let Some(id) = s.touch_hash(&hash, source) {
        return Some(id);
    }
    let id = s.alloc_id();
    let mut it = make(id);
    it.hash = hash;
    s.insert(it);
    Some(id)
}

fn capture_image(source: &str) -> Option<u64> {
    let mut cb = arboard::Clipboard::new().ok()?;
    let data = cb.get_image().ok()?;
    let img = image::RgbaImage::from_raw(data.width as u32, data.height as u32, data.bytes.into_owned())?;
    save_image(img, source, vec![])
}

fn image_hash(img: &image::RgbaImage) -> String {
    fnv(&[b"i", &img.width().to_le_bytes(), &img.height().to_le_bytes(), img.as_raw()])
}

/// 把原图和缩略图存到 data/images，返回图片信息。
fn store_image(img: &image::RgbaImage, hash: &str) -> Option<ImgMeta> {
    let (w, h) = img.dimensions();
    let dir = store().img_dir();
    let file = format!("{hash}.png");
    let thumb = format!("{hash}_t.png");
    img.save(dir.join(&file)).ok()?;
    if w > THUMB_W {
        let th = ((h as f64) * THUMB_W as f64 / w as f64).round().max(1.0) as u32;
        image::imageops::resize(img, THUMB_W, th, image::imageops::FilterType::Triangle)
            .save(dir.join(&thumb))
            .ok()?;
    } else {
        img.save(dir.join(&thumb)).ok()?;
    }
    Some(ImgMeta { w, h, file, thumb })
}

fn save_image(img: image::RgbaImage, source: &str, files: Vec<String>) -> Option<u64> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 || (w as u64 * h as u64) > MAX_PIXELS {
        return None;
    }
    let hash = image_hash(&img);
    if let Some(id) = store().touch_hash(&hash, source) {
        return Some(id);
    }
    let meta = store_image(&img, &hash)?;
    add(hash, source, |id| Item {
        id,
        kind: "image".into(),
        text: format!("图片 {w}×{h}"),
        html: None,
        files,
        img: Some(meta),
        hash: String::new(),
        pinned: false,
        created: now_ms(),
        used: now_ms(),
        source: source.to_string(),
        truncated: false,
    })
}

/// 把旧版本按"文件"记录的单个图片转换成图片条目（保留固定状态、时间和来源）。
fn migrate_image_files() {
    let todo: Vec<(u64, String)> = store()
        .items
        .iter()
        .filter(|i| i.kind == "files")
        .filter_map(|i| image_file(&i.files).map(|p| (i.id, p.to_string())))
        .collect();
    let mut changed = false;
    for (id, path) in todo {
        let Ok(img) = image::open(&path) else { continue };
        let img = img.to_rgba8();
        let (w, h) = img.dimensions();
        if (w as u64 * h as u64) > MAX_PIXELS {
            continue;
        }
        let hash = image_hash(&img);
        let Some(meta) = store_image(&img, &hash) else { continue };
        let mut s = store();
        if let Some(it) = s.items.iter_mut().find(|i| i.id == id) {
            it.kind = "image".into();
            it.text = format!("图片 {w}×{h}");
            it.img = Some(meta);
            it.hash = hash;
            changed = true;
        }
    }
    if changed {
        store().save();
        emit_changed(None);
    }
}

// ---------- 窗口 ----------

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

fn hwnd_of(w: &WebviewWindow) -> isize {
    w.hwnd().map(|h| h.0 as isize).unwrap_or(0)
}

fn show_panel(app: &AppHandle) {
    let Some(w) = main_window(app) else { return };
    win::remember_foreground(hwnd_of(&w));
    let scale = w.scale_factor().unwrap_or(1.0);
    let (cx, cy, wa) = win::cursor_and_work_area();
    let pw = (BASE_W * scale) as i32;
    let ph = (WIN_H * scale) as i32;
    let _ = w.set_size(PhysicalSize::new(pw as u32, ph as u32));
    let x = (cx - pw / 2).clamp(wa.left, (wa.right - pw).max(wa.left));
    let y = (cy + 12).clamp(wa.top, (wa.bottom - ph).max(wa.top));
    let _ = w.set_position(PhysicalPosition::new(x, y));
    let _ = w.show();
    let _ = w.set_focus();
    let _ = app.emit("panel-show", ());
}

fn toggle_panel(app: &AppHandle) {
    let Some(w) = main_window(app) else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = app.emit("panel-hide-request", ());
    } else {
        show_panel(app);
    }
}

fn register_hotkey(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let sc: Shortcut = hotkey.parse().map_err(|_| format!("无法识别的快捷键：{hotkey}"))?;
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    gs.on_shortcut(sc, |app, _, ev| {
        if ev.state == ShortcutState::Pressed {
            toggle_panel(app);
        }
    })
    .map_err(|e| format!("快捷键 {hotkey} 注册失败，可能已被其他程序占用（{e}）"))
}

fn set_paused(app: &AppHandle, paused: bool) {
    store().paused = paused;
    if let Some(m) = PAUSE_ITEM.get() {
        let _ = m.set_checked(paused);
    }
    let _ = app.emit("paused-changed", paused);
}

// ---------- 前端命令 ----------

#[tauri::command]
fn get_items() -> Vec<ItemDto> {
    store().items.iter().map(|i| i.dto(false)).collect()
}

#[tauri::command]
fn get_item(id: u64) -> Option<ItemDto> {
    store().get(id).map(|i| i.dto(true))
}

/// mode: paste | plain | copy
#[tauri::command]
fn paste(app: AppHandle, id: u64, mode: String) -> Result<(), String> {
    let it = store().get(id).cloned().ok_or("条目不存在")?;
    let plain = mode == "plain";
    match it.kind.as_str() {
        "files" if !plain => win::write_files(&it.files)?,
        "image" => {
            let meta = it.img.as_ref().ok_or("图片信息缺失")?;
            let path = store().img_dir().join(&meta.file);
            let png = std::fs::read(&path).map_err(|_| "图片文件已丢失")?;
            let rgba = image::load_from_memory(&png).map_err(|e| e.to_string())?.to_rgba8();
            let files: Vec<String> =
                it.files.iter().filter(|f| std::path::Path::new(f).exists()).cloned().collect();
            win::write_image(rgba.as_raw(), rgba.width(), rgba.height(), &png, &files)?;
        }
        _ => win::write_text(&it.text, if plain { None } else { it.html.as_deref() })?,
    }
    {
        let mut s = store();
        s.touch(id);
        s.save();
    }
    emit_changed(None);
    if let Some(w) = main_window(&app) {
        let _ = w.hide();
    }
    win::restore_and_paste(mode != "copy");
    Ok(())
}

#[tauri::command]
fn delete_item(id: u64) {
    let mut s = store();
    if s.remove(id) {
        s.save();
    }
}

#[tauri::command]
fn toggle_pin(id: u64) -> Option<bool> {
    let mut s = store();
    let r = s.toggle_pin(id);
    s.save();
    r
}

#[tauri::command]
fn clear_history() {
    let mut s = store();
    s.clear_unpinned();
    s.save();
}

#[tauri::command]
fn get_state() -> serde_json::Value {
    let s = store();
    json!({ "settings": s.settings, "paused": s.paused, "dataDir": s.dir().display().to_string() })
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    let old = store().settings.clone();
    let mut new = settings;
    new.max_items = new.max_items.clamp(20, 500);
    if new.hotkey != old.hotkey {
        if let Err(e) = register_hotkey(&app, &new.hotkey) {
            let _ = register_hotkey(&app, &old.hotkey);
            return Err(e);
        }
    }
    if new.autostart != old.autostart && !win::set_autostart(new.autostart) {
        new.autostart = old.autostart;
    }
    let mut s = store();
    s.settings = new.clone();
    s.enforce_limit();
    s.save_settings();
    s.save();
    drop(s);
    emit_changed(None);
    Ok(new)
}

#[tauri::command]
fn set_pause(app: AppHandle, paused: bool) {
    set_paused(&app, paused);
}

#[tauri::command]
fn hide_panel(app: AppHandle) {
    if let Some(w) = main_window(&app) {
        let _ = w.hide();
    }
}

/// 打开 / 关闭预览：窗口向右扩展，右侧放不下时整体左移。
#[tauri::command]
fn set_preview(app: AppHandle, open: bool) {
    let Some(w) = main_window(&app) else { return };
    let scale = w.scale_factor().unwrap_or(1.0);
    let nw = ((if open { PREVIEW_W } else { BASE_W }) * scale) as i32;
    let ph = (WIN_H * scale) as u32;
    let Ok(pos) = w.outer_position() else { return };
    let wa = win::window_work_area(hwnd_of(&w));
    let x = if pos.x + nw > wa.right { (wa.right - nw).max(wa.left) } else { pos.x };
    let _ = w.set_size(PhysicalSize::new(nw as u32, ph));
    if x != pos.x {
        let _ = w.set_position(PhysicalPosition::new(x, pos.y));
    }
}

#[tauri::command]
fn open_data_dir() {
    let d = store().dir().to_path_buf();
    let _ = std::process::Command::new("explorer").arg(d).spawn();
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

// ---------- 界面协议：http://ui.localhost/<文件> ----------
// 优先读取 exe 同目录 ui\ 文件夹（方便直接修改界面），缺失的文件回退到 exe 内置的那份。

static UI_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

fn ui_dir() -> Option<PathBuf> {
    let d = std::env::current_exe().ok()?.parent()?.join("ui");
    d.is_dir().then_some(d)
}

fn mime_of(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

fn serve_ui(app: &AppHandle, path: &str) -> tauri::http::Response<Cow<'static, [u8]>> {
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let safe = !rel.split('/').any(|c| c.is_empty() || c == ".." || c.contains('\\') || c.contains(':'));
    let mut body: Option<Vec<u8>> = None;
    if safe {
        if let Some(dir) = UI_DIR.get().and_then(|d| d.as_ref()) {
            body = std::fs::read(dir.join(rel)).ok();
        }
        if body.is_none() {
            body = app.asset_resolver().get(rel.to_string()).map(|a| a.bytes);
        }
    }
    match body {
        Some(b) => tauri::http::Response::builder()
            .header("Content-Type", mime_of(rel))
            .header("Cache-Control", "no-store")
            .body(Cow::Owned(b))
            .unwrap(),
        None => tauri::http::Response::builder().status(404).body(Cow::Borrowed(&[][..])).unwrap(),
    }
}

// ---------- 图片协议：http://clip.localhost/<文件名> ----------

fn serve_image(path: &str) -> tauri::http::Response<Cow<'static, [u8]>> {
    let name = path.trim_start_matches('/');
    let safe = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && !name.contains("..");
    let bytes = if safe {
        DATA_DIR.get().and_then(|d| std::fs::read(d.join("images").join(name)).ok())
    } else {
        None
    };
    match bytes {
        Some(b) => tauri::http::Response::builder()
            .header("Content-Type", "image/png")
            .header("Cache-Control", "max-age=31536000")
            .body(Cow::Owned(b))
            .unwrap(),
        None => tauri::http::Response::builder().status(404).body(Cow::Borrowed(&[][..])).unwrap(),
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "打开面板", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "暂停记录", true, false, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &pause, &settings, &sep, &quit])?;
    let _ = PAUSE_ITEM.set(pause);
    let mut tb = TrayIconBuilder::with_id("tray")
        .tooltip("Clipora 剪贴板")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "show" => show_panel(app),
            "pause" => {
                let p = !store().paused;
                set_paused(app, p);
            }
            "settings" => {
                show_panel(app);
                let _ = app.emit("open-settings", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                toggle_panel(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tb = tb.icon(icon.clone());
    }
    tb.build(app)?;
    Ok(())
}

fn main() {
    let dir = store::data_dir();
    let _ = DATA_DIR.set(dir.clone());
    let _ = UI_DIR.set(ui_dir());
    let _ = STORE.set(Mutex::new(Store::load(dir)));

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show_panel(app)))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .register_uri_scheme_protocol("clip", |_ctx, req| serve_image(req.uri().path()))
        .register_uri_scheme_protocol("ui", |ctx, req| serve_ui(ctx.app_handle(), req.uri().path()))
        .invoke_handler(tauri::generate_handler![
            get_items,
            get_item,
            paste,
            delete_item,
            toggle_pin,
            clear_history,
            get_state,
            save_settings,
            set_pause,
            hide_panel,
            set_preview,
            open_data_dir,
            quit
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let _ = APP.set(handle.clone());
            build_tray(&handle)?;
            let hotkey = store().settings.hotkey.clone();
            if register_hotkey(&handle, &hotkey).is_err() {
                let _ = register_hotkey(&handle, "Alt+V");
            }
            if let Some(w) = main_window(&handle) {
                let h2 = handle.clone();
                let own = hwnd_of(&w);
                w.on_window_event(move |e| {
                    if let WindowEvent::Focused(false) = e {
                        // 拖动窗口时也会短暂失焦。轮询最多 1.5 秒：前台回到自己就不隐藏；
                        // 前台换成别的程序、且鼠标已松开（点击结束）就隐藏
                        let h3 = h2.clone();
                        std::thread::spawn(move || {
                            for _ in 0..30 {
                                std::thread::sleep(std::time::Duration::from_millis(50));
                                if win::is_own_foreground(own) {
                                    return;
                                }
                                if !win::left_button_down() {
                                    break;
                                }
                            }
                            if !win::is_own_foreground(own) {
                                let _ = h3.emit("panel-hide-request", ());
                            }
                        });
                    }
                });
            }
            win::spawn_listener();
            std::thread::spawn(migrate_image_files);
            if !std::env::args().any(|a| a == "--silent") {
                let h3 = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    let h4 = h3.clone();
                    let _ = h3.run_on_main_thread(move || show_panel(&h4));
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("启动失败");
}

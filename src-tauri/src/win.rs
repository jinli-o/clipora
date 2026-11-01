//! Win32 相关：剪贴板监听 / 读写、来源程序、模拟粘贴、显示器区域、开机自启。

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::DragQueryFileW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CF_BITMAP: u32 = 2;
const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const CF_DIBV5: u32 = 17;
const WM_CLIPBOARDUPDATE_ID: u32 = 0x031D;

static LISTENER_HWND: AtomicIsize = AtomicIsize::new(0);
static PREV_FG: AtomicIsize = AtomicIsize::new(0);
static SKIP_SEQ: AtomicU32 = AtomicU32::new(0);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn fmt(name: &str) -> u32 {
    let w = wide(name);
    unsafe { RegisterClipboardFormatW(w.as_ptr()) }
}

fn has(format: u32) -> bool {
    format != 0 && unsafe { IsClipboardFormatAvailable(format) } != 0
}

struct ClipGuard;
impl Drop for ClipGuard {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

fn open_clipboard() -> Option<ClipGuard> {
    let hwnd = LISTENER_HWND.load(Ordering::SeqCst) as HWND;
    for _ in 0..20 {
        if unsafe { OpenClipboard(hwnd) } != 0 {
            return Some(ClipGuard);
        }
        thread::sleep(Duration::from_millis(15));
    }
    None
}

/// 读取剪贴板中某格式的原始字节（需已打开剪贴板）。
unsafe fn read_bytes(format: u32) -> Option<Vec<u8>> {
    let h = GetClipboardData(format);
    if h.is_null() {
        return None;
    }
    let size = GlobalSize(h as _);
    let p = GlobalLock(h as _) as *const u8;
    if p.is_null() {
        return None;
    }
    let v = std::slice::from_raw_parts(p, size).to_vec();
    GlobalUnlock(h as _);
    Some(v)
}

unsafe fn read_text() -> Option<String> {
    let b = read_bytes(CF_UNICODETEXT)?;
    let u: Vec<u16> = b
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    Some(String::from_utf16_lossy(&u))
}

unsafe fn read_files() -> Vec<String> {
    let h = GetClipboardData(CF_HDROP);
    if h.is_null() {
        return vec![];
    }
    let n = DragQueryFileW(h as _, u32::MAX, null_mut(), 0);
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let len = DragQueryFileW(h as _, i, null_mut(), 0);
        let mut buf = vec![0u16; len as usize + 1];
        DragQueryFileW(h as _, i, buf.as_mut_ptr(), len + 1);
        out.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    out
}

pub enum Captured {
    Text { text: String, html: Option<String> },
    Files(Vec<String>),
    Image,
    Skip,
}

pub const MAX_HTML: usize = 2 * 1024 * 1024;

/// 读取当前剪贴板内容。优先级：文件 > 文本 > 图片。
pub fn capture() -> Captured {
    // 密码管理器等会设置这些标记，表示不希望被剪贴板工具记录
    if has(fmt("ExcludeClipboardContentFromMonitorProcessing")) || has(fmt("Clipboard Viewer Ignore")) {
        return Captured::Skip;
    }
    let Some(_g) = open_clipboard() else { return Captured::Skip };
    unsafe {
        let cih = fmt("CanIncludeInClipboardHistory");
        if has(cih) {
            if let Some(b) = read_bytes(cih) {
                if b.len() >= 4 && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) == 0 {
                    return Captured::Skip;
                }
            }
        }
        if has(CF_HDROP) {
            let files = read_files();
            if !files.is_empty() {
                return Captured::Files(files);
            }
        }
        if has(CF_UNICODETEXT) {
            if let Some(text) = read_text() {
                if !text.trim().is_empty() {
                    let hf = fmt("HTML Format");
                    let html = if has(hf) {
                        read_bytes(hf).and_then(|mut b| {
                            while b.last() == Some(&0) {
                                b.pop();
                            }
                            (b.len() <= MAX_HTML).then(|| String::from_utf8_lossy(&b).into_owned())
                        })
                    } else {
                        None
                    };
                    return Captured::Text { text, html };
                }
            }
        }
        if has(CF_DIB) || has(CF_DIBV5) || has(CF_BITMAP) || has(fmt("PNG")) {
            return Captured::Image;
        }
    }
    Captured::Skip
}

/// 当前剪贴板变化是否由本程序自己写入。
pub fn is_self_write() -> bool {
    unsafe { GetClipboardSequenceNumber() == SKIP_SEQ.load(Ordering::SeqCst) }
}

fn exe_stem_of_window(hwnd: HWND) -> String {
    if hwnd.is_null() {
        return String::new();
    }
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 || pid == std::process::id() {
            return String::new();
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// 复制内容的来源程序（exe 名，不含扩展名）。
pub fn clipboard_source() -> String {
    unsafe {
        let owner = GetClipboardOwner();
        let s = exe_stem_of_window(owner);
        if !s.is_empty() {
            return s;
        }
        exe_stem_of_window(GetForegroundWindow())
    }
}

// ---------- 写入剪贴板 ----------

unsafe fn set_bytes(format: u32, bytes: &[u8]) -> bool {
    let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
    if h.is_null() {
        return false;
    }
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        GlobalFree(h);
        return false;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    GlobalUnlock(h);
    if SetClipboardData(format, h as HANDLE).is_null() {
        GlobalFree(h);
        return false;
    }
    true
}

fn utf16_bytes(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(std::iter::once(0)).flat_map(|c| c.to_le_bytes()).collect()
}

fn with_clipboard(f: impl FnOnce() -> bool) -> Result<(), String> {
    let g = open_clipboard().ok_or("剪贴板被其他程序占用")?;
    unsafe { EmptyClipboard() };
    let ok = f();
    drop(g);
    SKIP_SEQ.store(unsafe { GetClipboardSequenceNumber() }, Ordering::SeqCst);
    if ok { Ok(()) } else { Err("写入剪贴板失败".into()) }
}

pub fn write_text(text: &str, html: Option<&str>) -> Result<(), String> {
    with_clipboard(|| unsafe {
        let ok = set_bytes(CF_UNICODETEXT, &utf16_bytes(text));
        if let Some(h) = html {
            let mut b = h.as_bytes().to_vec();
            b.push(0);
            set_bytes(fmt("HTML Format"), &b);
        }
        ok
    })
}

fn dropfiles(files: &[String]) -> Vec<u8> {
    // DROPFILES 头：pFiles, pt.x, pt.y, fNC, fWide
    let mut b: Vec<u8> = Vec::new();
    for v in [20u32, 0, 0, 0, 1] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for f in files {
        b.extend(utf16_bytes(f));
    }
    b.extend_from_slice(&[0, 0]);
    b
}

unsafe fn set_files(files: &[String]) -> bool {
    let ok = set_bytes(CF_HDROP, &dropfiles(files));
    set_bytes(fmt("Preferred DropEffect"), &1u32.to_le_bytes());
    ok
}

pub fn write_files(files: &[String]) -> Result<(), String> {
    with_clipboard(|| unsafe { set_files(files) })
}

/// 写入图片：CF_DIB（兼容所有程序）+ PNG（保留透明度）；
/// 来自图片文件时同时写入文件路径，与 QQ、资源管理器复制时的格式一致。
pub fn write_image(rgba: &[u8], w: u32, h: u32, png: &[u8], files: &[String]) -> Result<(), String> {
    let mut b = Vec::with_capacity(40 + rgba.len());
    let header: [i32; 3] = [40, w as i32, h as i32];
    for v in header {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&1u16.to_le_bytes()); // planes
    b.extend_from_slice(&32u16.to_le_bytes()); // bpp
    b.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    b.extend_from_slice(&((w * h * 4) as u32).to_le_bytes());
    b.extend_from_slice(&[0u8; 16]); // 分辨率与调色板
    let stride = (w * 4) as usize;
    for row in (0..h as usize).rev() {
        for px in rgba[row * stride..(row + 1) * stride].chunks_exact(4) {
            b.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    with_clipboard(|| unsafe {
        let ok = set_bytes(CF_DIB, &b);
        set_bytes(fmt("PNG"), png);
        if !files.is_empty() {
            set_files(files);
        }
        ok
    })
}

// ---------- 焦点与粘贴 ----------

/// 呼出面板前记住当前前台窗口，粘贴时切回。
pub fn remember_foreground(own: isize) {
    let fg = unsafe { GetForegroundWindow() } as isize;
    if fg != 0 && fg != own {
        PREV_FG.store(fg, Ordering::SeqCst);
    }
}

fn key(vk: u16, up: bool) -> INPUT {
    let mut i: INPUT = unsafe { zeroed() };
    i.r#type = INPUT_KEYBOARD;
    i.Anonymous.ki = KEYBDINPUT {
        wVk: vk,
        wScan: 0,
        dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
        time: 0,
        dwExtraInfo: 0,
    };
    i
}

pub fn restore_and_paste(do_paste: bool) {
    let prev = PREV_FG.load(Ordering::SeqCst);
    thread::spawn(move || unsafe {
        let hwnd = prev as HWND;
        if !hwnd.is_null() && IsWindow(hwnd) != 0 {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
        if !do_paste {
            return;
        }
        thread::sleep(Duration::from_millis(90));
        let mut seq: Vec<INPUT> = Vec::new();
        // 先松开仍被按住的修饰键（例如 Shift+Enter 中的 Shift），避免变成 Ctrl+Shift+V
        for vk in [VK_SHIFT, VK_LSHIFT, VK_RSHIFT, VK_MENU, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN, VK_CONTROL] {
            if (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 {
                seq.push(key(vk, true));
            }
        }
        seq.push(key(VK_CONTROL, false));
        seq.push(key(0x56, false));
        seq.push(key(0x56, true));
        seq.push(key(VK_CONTROL, true));
        SendInput(seq.len() as u32, seq.as_ptr(), size_of::<INPUT>() as i32);
    });
}

/// 前台窗口是否是本程序的面板（含其 WebView 子窗口）。
pub fn is_own_foreground(own: isize) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() {
            return true;
        }
        if fg as isize == own {
            return true;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, &mut pid);
        pid == std::process::id()
    }
}

pub fn left_button_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON as i32) as u16 & 0x8000) != 0 }
}

// ---------- 显示器 ----------

pub struct WorkArea {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

fn work_of(m: HMONITOR) -> WorkArea {
    unsafe {
        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(m, &mut mi);
        let r = mi.rcWork;
        WorkArea { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

pub fn cursor_and_work_area() -> (i32, i32, WorkArea) {
    unsafe {
        let mut pt: POINT = zeroed();
        GetCursorPos(&mut pt);
        (pt.x, pt.y, work_of(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST)))
    }
}

pub fn window_work_area(hwnd: isize) -> WorkArea {
    unsafe { work_of(MonitorFromWindow(hwnd as HWND, MONITOR_DEFAULTTONEAREST)) }
}

// ---------- 开机自启 ----------

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn set_autostart(on: bool) -> bool {
    let key = wide(RUN_KEY);
    let name = wide("Clipora");
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return false };
            let data = wide(&format!("\"{}\" --silent", exe.display()));
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                data.as_ptr() as *const c_void,
                (data.len() * 2) as u32,
            ) == 0
        } else {
            let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr());
            r == 0 || r == ERROR_FILE_NOT_FOUND
        }
    }
}

// ---------- 剪贴板监听线程 ----------

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE_ID {
        crate::on_clipboard_update();
        return 0;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

pub fn spawn_listener() {
    thread::spawn(|| unsafe {
        let hinst = GetModuleHandleW(null());
        let cls = wide("CliporaClipboardListener");
        let mut wc: WNDCLASSW = zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = hinst;
        wc.lpszClassName = cls.as_ptr();
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            cls.as_ptr(),
            cls.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            hinst,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        LISTENER_HWND.store(hwnd as isize, Ordering::SeqCst);
        AddClipboardFormatListener(hwnd);
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

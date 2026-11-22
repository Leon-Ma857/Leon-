#![cfg(target_os = "macos")]

use crate::selection_capture::{
    is_own_injected_key, parse_smart_selection_key, SelectionAnchorRect, SelectionCaptureResult,
    SmartSelectionKey, SmartSelectionTracker, WorkAreaRect, LUMA_INJECTED_INPUT_MARKER,
};
use crate::win_ocr::{group_ocr_lines_into_blocks, guess_lang_from_text, OcrLayoutBlock, OcrLineBox};
use serde::Deserialize;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

extern "C" {
    fn luma_mac_free_string(s: *mut c_char);
    fn luma_mac_ocr_png(
        bytes: *const u8,
        len: usize,
        out_json: *mut *mut c_char,
        out_err: *mut *mut c_char,
    ) -> bool;
    fn luma_mac_get_cursor_point(out_x: *mut i32, out_y: *mut i32) -> bool;
    fn luma_mac_work_area_for_point(
        phys_x: i32,
        phys_y: i32,
        left: *mut i32,
        top: *mut i32,
        right: *mut i32,
        bottom: *mut i32,
        scale_out: *mut f32,
    ) -> bool;
    fn luma_mac_capture_selected_text(
        out_x: *mut i32,
        out_y: *mut i32,
        out_text: *mut *mut c_char,
        out_err: *mut *mut c_char,
    ) -> bool;
    fn luma_mac_smart_hook_start(
        mouse_cb: Option<extern "C" fn(i32, i32, i32, i32, bool)>,
        key_cb: Option<extern "C" fn(u32, bool, bool, i32, bool) -> bool>,
        out_err: *mut *mut c_char,
    ) -> bool;
    fn luma_mac_smart_hook_stop();
    fn luma_mac_configure_floating_window(ns_window: *mut c_void);
    fn luma_mac_configure_overlay_window(ns_window: *mut c_void);
    fn luma_mac_force_window_foreground(ns_window: *mut c_void);
    fn luma_mac_frontmost_external_pid() -> i32;
    fn luma_mac_popover_activate(
        ns_window: *mut c_void,
        source_pid: i32,
        on_dismiss: Option<extern "C" fn()>,
    );
    fn luma_mac_popover_deactivate();
    fn luma_mac_popover_shutdown();
    fn luma_mac_copy_files_to_pasteboard(
        paths: *const *const c_char,
        count: usize,
        out_err: *mut *mut c_char,
    ) -> bool;
    fn luma_mac_send_scroll_wheel_at(phys_x: i32, phys_y: i32, lines: i32);
    fn luma_mac_is_escape_pressed() -> bool;
}

unsafe fn take_c_string(ptr: *mut c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
    unsafe { luma_mac_free_string(ptr) };
    Some(s)
}

#[derive(Deserialize)]
struct VisionLineDto {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: String,
}

pub fn ocr_png_with_layout_sync(
    png_bytes: &[u8],
) -> Result<(String, String, Vec<OcrLayoutBlock>), String> {
    let mut out_json: *mut c_char = std::ptr::null_mut();
    let mut out_err: *mut c_char = std::ptr::null_mut();
    let ok = unsafe { luma_mac_ocr_png(png_bytes.as_ptr(), png_bytes.len(), &mut out_json, &mut out_err) };
    if !ok {
        let err = unsafe { take_c_string(out_err) }
            .unwrap_or_else(|| "Apple Vision OCR 识别失败".to_string());
        return Err(err);
    }
    let json_str = unsafe { take_c_string(out_json) }.unwrap_or_else(|| "[]".to_string());
    let mut items: Vec<VisionLineDto> =
        serde_json::from_str(&json_str).map_err(|e| format!("解析 Vision OCR 结果失败: {e}"))?;
    // Vision sometimes emits bottom-to-top depending on observation ordering; normalize top-to-bottom.
    items.sort_by(|a, b| a.y.total_cmp(&b.y).then_with(|| a.x.total_cmp(&b.x)));

    let lines: Vec<OcrLineBox> = items
        .into_iter()
        .map(|it| OcrLineBox {
            x: it.x,
            y: it.y,
            w: it.w,
            h: it.h,
            text: it.text,
            had_leading_icon: false,
        })
        .collect();
    let full_text = lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let detected = guess_lang_from_text(&full_text);
    let blocks = group_ocr_lines_into_blocks(lines);
    Ok((detected, full_text, blocks))
}

pub fn capture_cursor_point() -> Option<(i32, i32)> {
    let mut x = 0i32;
    let mut y = 0i32;
    if unsafe { luma_mac_get_cursor_point(&mut x, &mut y) } {
        Some((x, y))
    } else {
        None
    }
}

pub fn work_area_for_point(x: i32, y: i32) -> Option<WorkAreaRect> {
    let mut left = 0i32;
    let mut top = 0i32;
    let mut right = 0i32;
    let mut bottom = 0i32;
    let mut scale_factor = 1.0f32;
    let ok = unsafe {
        luma_mac_work_area_for_point(x, y, &mut left, &mut top, &mut right, &mut bottom, &mut scale_factor)
    };
    if ok {
        Some(WorkAreaRect {
            left,
            top,
            right,
            bottom,
            scale_factor: scale_factor.max(1.0),
        })
    } else {
        None
    }
}

pub fn capture_selected_text_without_polluting_clipboard() -> Result<SelectionCaptureResult, String> {
    let mut cx = 100i32;
    let mut cy = 100i32;
    let mut out_text: *mut c_char = std::ptr::null_mut();
    let mut out_err: *mut c_char = std::ptr::null_mut();
    let ok = unsafe {
        luma_mac_capture_selected_text(&mut cx, &mut cy, &mut out_text, &mut out_err)
    };
    if !ok {
        let err = unsafe { take_c_string(out_err) }
            .unwrap_or_else(|| "未能获取当前选中的文字".to_string());
        return Err(err);
    }
    let text = unsafe { take_c_string(out_text) }.unwrap_or_default();
    if text.trim().is_empty() {
        return Err("选中的内容不包含可翻译文本".to_string());
    }
    Ok(SelectionCaptureResult {
        text,
        cursor_x: cx,
        cursor_y: cy,
    })
}

type SmartCallback = Arc<dyn Fn(SelectionAnchorRect) + Send + Sync>;

struct SmartHookState {
    target_vk: std::sync::atomic::AtomicU32,
    tracker: Mutex<SmartSelectionTracker>,
    started_at: std::time::Instant,
    trigger_tx: Mutex<Option<mpsc::SyncSender<SelectionAnchorRect>>>,
    worker_join: Mutex<Option<std::thread::JoinHandle<()>>>,
}

static SMART_STATE: OnceLock<SmartHookState> = OnceLock::new();

fn smart_state() -> &'static SmartHookState {
    SMART_STATE.get_or_init(|| SmartHookState {
        target_vk: std::sync::atomic::AtomicU32::new(0),
        tracker: Mutex::new(SmartSelectionTracker::default()),
        started_at: std::time::Instant::now(),
        trigger_tx: Mutex::new(None),
        worker_join: Mutex::new(None),
    })
}

fn smart_now_ms() -> u64 {
    smart_state().started_at.elapsed().as_millis() as u64
}

extern "C" fn mac_mouse_cb(kind: i32, x: i32, y: i32, fg_pid: i32, is_self: bool) {
    let st = smart_state();
    if st.target_vk.load(std::sync::atomic::Ordering::Acquire) == 0 {
        return;
    }
    if let Ok(mut tracker) = st.tracker.try_lock() {
        let now = smart_now_ms();
        match kind {
            0 => tracker.on_lbutton_down(x, y, now),
            1 => tracker.on_lbutton_up(x, y, now, fg_pid as isize, is_self),
            _ => tracker.on_other_mouse_button(),
        }
    }
}

extern "C" fn mac_key_cb(
    vk: u32,
    has_mods: bool,
    is_own_injected: bool,
    fg_pid: i32,
    is_self: bool,
) -> bool {
    let st = smart_state();
    let target_vk = st.target_vk.load(std::sync::atomic::Ordering::Acquire);
    if target_vk == 0 || is_own_injected {
        return false;
    }
    // Reuses `is_own_injected_key` contract consistently with Windows.
    if is_own_injected_key(0x10, if is_own_injected { LUMA_INJECTED_INPUT_MARKER } else { 0 }) {
        return false;
    }
    let triggered = if let Ok(mut tracker) = st.tracker.try_lock() {
        tracker.on_key_down(
            vk,
            target_vk,
            has_mods,
            false,
            smart_now_ms(),
            fg_pid as isize,
            is_self,
        )
    } else {
        None
    };
    if let Some(rect) = triggered {
        if let Ok(guard) = st.trigger_tx.try_lock() {
            if let Some(ref tx) = *guard {
                let _ = tx.try_send(rect);
            }
        }
        return true;
    }
    false
}

pub fn configure_smart_selection_hook(
    target: Option<SmartSelectionKey>,
    on_trigger: Option<SmartCallback>,
) -> Result<(), String> {
    let st = smart_state();
    st.target_vk.store(0, std::sync::atomic::Ordering::Release);
    unsafe { luma_mac_smart_hook_stop() };
    if let Ok(mut tx_g) = st.trigger_tx.lock() {
        *tx_g = None;
    }
    if let Ok(mut wj) = st.worker_join.lock() {
        if let Some(h) = wj.take() {
            let _ = h.join();
        }
    }
    if let Ok(mut tr) = st.tracker.lock() {
        *tr = SmartSelectionTracker::default();
    }

    let Some(key) = target else {
        return Ok(());
    };
    let callback = on_trigger.ok_or_else(|| "缺少划词触发回调".to_string())?;
    let target_vk = match key {
        SmartSelectionKey::Space => 0x20u32,
        SmartSelectionKey::Tab => 0x09u32,
    };
    let _ = parse_smart_selection_key; // keep module cross-referenced cleanly

    let (tx, rx) = mpsc::sync_channel::<SelectionAnchorRect>(2);
    if let Ok(mut tx_g) = st.trigger_tx.lock() {
        *tx_g = Some(tx);
    }
    let handle = std::thread::Builder::new()
        .name("luma-mac-smart-sel-worker".to_string())
        .spawn(move || {
            while let Ok(rect) = rx.recv() {
                callback(rect);
            }
        })
        .map_err(|e| format!("启动划词监听工作线程失败: {e}"))?;
    if let Ok(mut wj) = st.worker_join.lock() {
        *wj = Some(handle);
    }

    st.target_vk
        .store(target_vk, std::sync::atomic::Ordering::Release);
    let mut out_err: *mut c_char = std::ptr::null_mut();
    let ok = unsafe { luma_mac_smart_hook_start(Some(mac_mouse_cb), Some(mac_key_cb), &mut out_err) };
    if !ok {
        st.target_vk.store(0, std::sync::atomic::Ordering::Release);
        if let Ok(mut tx_g) = st.trigger_tx.lock() {
            *tx_g = None;
        }
        if let Ok(mut wj) = st.worker_join.lock() {
            if let Some(h) = wj.take() {
                let _ = h.join();
            }
        }
        let err = unsafe { take_c_string(out_err) }
            .unwrap_or_else(|| "启动 macOS 单键划词监听失败".to_string());
        return Err(err);
    }
    Ok(())
}

pub fn configure_floating_window(ns_window: *mut c_void) {
    unsafe { luma_mac_configure_floating_window(ns_window) };
}

pub fn configure_overlay_window(ns_window: *mut c_void) {
    unsafe { luma_mac_configure_overlay_window(ns_window) };
}

pub fn force_window_foreground(raw_ns_window: isize) {
    if raw_ns_window != 0 {
        unsafe { luma_mac_force_window_foreground(raw_ns_window as *mut c_void) };
    }
}

pub fn capture_external_foreground_pid() -> Option<isize> {
    let pid = unsafe { luma_mac_frontmost_external_pid() };
    if pid > 0 {
        Some(pid as isize)
    } else {
        None
    }
}

type PopoverDismissCb = Arc<dyn Fn() + Send + Sync>;
static POPOVER_DISMISS_CB: OnceLock<Mutex<Option<PopoverDismissCb>>> = OnceLock::new();

extern "C" fn mac_popover_dismiss_trampoline() {
    if let Some(lock) = POPOVER_DISMISS_CB.get() {
        if let Ok(guard) = lock.lock() {
            if let Some(ref cb) = *guard {
                let cloned = Arc::clone(cb);
                drop(guard);
                cloned();
            }
        }
    }
}

pub fn popover_activate(raw_ns_window: isize, source_pid: Option<isize>, on_dismiss: PopoverDismissCb) {
    let lock = POPOVER_DISMISS_CB.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(on_dismiss);
    }
    let src = source_pid.map(|p| p as i32).unwrap_or(0);
    unsafe {
        luma_mac_popover_activate(
            raw_ns_window as *mut c_void,
            src,
            Some(mac_popover_dismiss_trampoline),
        );
    }
}

pub fn popover_deactivate() {
    unsafe { luma_mac_popover_deactivate() };
}

pub fn popover_shutdown() {
    if let Some(lock) = POPOVER_DISMISS_CB.get() {
        if let Ok(mut guard) = lock.lock() {
            *guard = None;
        }
    }
    unsafe { luma_mac_popover_shutdown() };
}

pub fn copy_files_to_pasteboard(paths: &[String]) -> Result<(), String> {
    let cstrings: Vec<CString> = paths
        .iter()
        .filter_map(|p| CString::new(p.as_str()).ok())
        .collect();
    if cstrings.is_empty() {
        return Err("没有可复制的图片文件".to_string());
    }
    let ptrs: Vec<*const c_char> = cstrings.iter().map(|s| s.as_ptr()).collect();
    let mut out_err: *mut c_char = std::ptr::null_mut();
    let ok = unsafe { luma_mac_copy_files_to_pasteboard(ptrs.as_ptr(), ptrs.len(), &mut out_err) };
    if !ok {
        let err = unsafe { take_c_string(out_err) }
            .unwrap_or_else(|| "写入 macOS 文件剪贴板失败".to_string());
        return Err(err);
    }
    Ok(())
}

pub fn send_scroll_wheel_at(phys_x: i32, phys_y: i32, delta_lines: i32) {
    unsafe { luma_mac_send_scroll_wheel_at(phys_x, phys_y, delta_lines) };
}

pub fn is_escape_pressed() -> bool {
    unsafe { luma_mac_is_escape_pressed() }
}

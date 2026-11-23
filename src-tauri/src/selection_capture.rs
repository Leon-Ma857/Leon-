//! Non-polluting global text selection capture and non-occluding popup placement.

use std::sync::Arc;

/// Bounding rectangle (physical screen coordinates) of the recent mouse selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionAnchorRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Single-key triggers supported after a confirmed mouse text selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartSelectionKey {
    Space,
    Tab,
}

pub fn parse_smart_selection_key(raw: &str) -> Option<SmartSelectionKey> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "space" => Some(SmartSelectionKey::Space),
        "tab" => Some(SmartSelectionKey::Tab),
        _ => None,
    }
}

/// Text captured from the currently focused application alongside the cursor coordinates
/// recorded when the selection shortcut was triggered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionCaptureResult {
    pub text: String,
    pub cursor_x: i32,
    pub cursor_y: i32,
}

/// Monitor work-area rectangle in physical screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkAreaRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub scale_factor: f32,
}

const CF_BITMAP: u32 = 2;
const CF_METAFILEPICT: u32 = 3;
const CF_PALETTE: u32 = 9;
const CF_ENHMETAFILE: u32 = 14;
const CF_OWNERDISPLAY: u32 = 0x0080;
const CF_DSPTEXT: u32 = 0x0081;
const CF_DSPMETAFILEPICT: u32 = 0x0083;
const CF_DSPENHMETAFILE: u32 = 0x008E;

/// Returns `true` if `fmt` is backed by a copyable `HGLOBAL` memory block on the Windows
/// clipboard, excluding GDI handles, owner-display, private/GDI ranges, and `0`.
#[cfg(any(windows, test))]
pub(crate) fn is_copyable_hglobal_format(fmt: u32) -> bool {
    if fmt == 0 {
        return false;
    }
    if matches!(
        fmt,
        CF_BITMAP | CF_METAFILEPICT | CF_PALETTE | CF_ENHMETAFILE | CF_OWNERDISPLAY | CF_DSPENHMETAFILE
    ) {
        return false;
    }
    if (CF_DSPTEXT..=CF_DSPMETAFILEPICT).contains(&fmt) {
        return false;
    }
    if (0x0200..=0x03FF).contains(&fmt) {
        return false;
    }
    true
}

/// Compute a physical-pixel popup `(x, y, placed_below)` anchored cleanly outside either a
/// mouse-drag `selection_rect` or `(cursor_x, cursor_y)` inside `work`.
pub fn compute_selection_popup_position(
    cursor_x: i32,
    cursor_y: i32,
    selection_rect: Option<SelectionAnchorRect>,
    work: WorkAreaRect,
    popup_w_logical: f64,
    popup_h_logical: f64,
) -> (i32, i32, bool) {
    let scale = f64::from(work.scale_factor.max(1.0));
    let w_phys = (popup_w_logical.max(1.0) * scale).round() as i32;
    let h_phys = (popup_h_logical.max(1.0) * scale).round() as i32;
    let margin = (12.0 * scale).round() as i32;

    let min_x = work.left + margin;
    let max_x = (work.right - w_phys - margin).max(min_x);
    let min_y = work.top + margin;
    let max_y = (work.bottom - h_phys - margin).max(min_y);

    if let Some(rect) = selection_rect {
        let r_left = rect.left.min(rect.right);
        let r_right = rect.left.max(rect.right);
        let r_top = rect.top.min(rect.bottom);
        let r_bottom = rect.top.max(rect.bottom);
        let gap = (18.0 * scale).round() as i32;
        let aligned_x = r_left.clamp(min_x, max_x);

        let below_y = r_bottom + gap;
        if below_y + h_phys <= work.bottom - margin {
            return (aligned_x, below_y.clamp(min_y, max_y), true);
        }

        let above_y = r_top - gap - h_phys;
        if above_y >= min_y {
            return (aligned_x, above_y.clamp(min_y, max_y), false);
        }

        let right_x = r_right + gap;
        if right_x + w_phys <= work.right - margin {
            return (right_x.clamp(min_x, max_x), r_top.clamp(min_y, max_y), true);
        }

        let left_x = r_left - gap - w_phys;
        if left_x >= min_x {
            return (left_x.clamp(min_x, max_x), r_top.clamp(min_y, max_y), true);
        }

        return (aligned_x, below_y.clamp(min_y, max_y), true);
    }

    let horiz_gap = (10.0 * scale).round() as i32;
    let below_offset = (28.0 * scale).round() as i32;
    let above_offset = (24.0 * scale).round() as i32;

    let right_x = cursor_x + horiz_gap;
    let raw_x = if right_x + w_phys <= work.right - margin {
        right_x
    } else {
        cursor_x - w_phys - horiz_gap
    };

    let below_y = cursor_y + below_offset;
    if below_y + h_phys <= work.bottom - margin {
        return (raw_x.clamp(min_x, max_x), below_y.clamp(min_y, max_y), true);
    }

    let above_y = cursor_y - h_phys - above_offset;
    (raw_x.clamp(min_x, max_x), above_y.clamp(min_y, max_y), false)
}

/// Compute a physical-pixel popup `(x, y)` anchored near `(cursor_x, cursor_y)` inside `work`.
#[allow(dead_code)]
pub fn compute_cursor_popup_position(
    cursor_x: i32,
    cursor_y: i32,
    work: WorkAreaRect,
    popup_w_logical: f64,
    popup_h_logical: f64,
) -> (i32, i32) {
    let (x, y, _) =
        compute_selection_popup_position(cursor_x, cursor_y, None, work, popup_w_logical, popup_h_logical);
    (x, y)
}

/// Deterministic state machine tracking drag/double-click text selections for single-key triggers.
#[derive(Debug, Default)]
pub(crate) struct SmartSelectionTracker {
    mouse_down: Option<(i32, i32, u64)>,
    last_up: Option<(i32, i32, u64)>,
    pending: Option<PendingSelection>,
}

#[derive(Debug, Clone, Copy)]
struct PendingSelection {
    rect: SelectionAnchorRect,
    hwnd: isize,
    armed_at_ms: u64,
}

/// `dwExtraInfo` tag ("LUMA") attached to every keystroke Luma synthesizes via `SendInput`.
pub(crate) const LUMA_INJECTED_INPUT_MARKER: usize = 0x4C55_4D41;
const LLKHF_INJECTED: u32 = 0x10;

/// Returns `true` only for keystrokes Luma injected itself, so the smart-key hook never reacts
/// to its own Ctrl+C while still honoring injected input from other tools.
pub(crate) fn is_own_injected_key(hook_flags: u32, extra_info: usize) -> bool {
    (hook_flags & LLKHF_INJECTED) != 0 && extra_info == LUMA_INJECTED_INPUT_MARKER
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RectI32 {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Returns `true` when the active foreground context looks like a game, exclusive/borderless
/// fullscreen media window, hidden-cursor 3D app, or Windows shell desktop where single-key
/// selection hooks must stay passive.
#[cfg(any(windows, test))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn should_suppress_smart_selection(
    user_notification_busy: bool,
    cursor_hidden: bool,
    is_shell_desktop_window: bool,
    win_rect: Option<RectI32>,
    monitor_rect: Option<RectI32>,
    work_rect: Option<RectI32>,
    has_caption: bool,
    is_zoomed: bool,
) -> bool {
    if user_notification_busy || cursor_hidden || is_shell_desktop_window {
        return true;
    }

    if let (Some(win), Some(mon)) = (win_rect, monitor_rect) {
        if mon.right > mon.left && mon.bottom > mon.top {
            let covers_monitor = win.left <= mon.left
                && win.top <= mon.top
                && win.right >= mon.right
                && win.bottom >= mon.bottom;
            if covers_monitor {
                let taskbar_visible = work_rect.is_some_and(|w| {
                    (w.left - mon.left) >= 16
                        || (w.top - mon.top) >= 16
                        || (mon.right - w.right) >= 16
                        || (mon.bottom - w.bottom) >= 16
                });
                if taskbar_visible || !(is_zoomed && has_caption) {
                    return true;
                }
            }
        }
    }

    false
}

const DRAG_THRESHOLD_SQ: i64 = 100; // 10px minimum drag distance
const DOUBLE_CLICK_MS: u64 = 500;
const DOUBLE_CLICK_DIST_SQ: i64 = 64; // 8px max jitter between clicks
const ARM_TTL_MS: u64 = 4000;

impl SmartSelectionTracker {
    pub fn on_lbutton_down(&mut self, x: i32, y: i32, now_ms: u64) {
        self.pending = None;
        self.mouse_down = Some((x, y, now_ms));
    }

    #[allow(dead_code)]
    pub fn on_lbutton_up(
        &mut self,
        x: i32,
        y: i32,
        now_ms: u64,
        fg_hwnd: isize,
        is_self_window: bool,
    ) {
        self.on_lbutton_up_with_guard(x, y, now_ms, fg_hwnd, is_self_window, || false);
    }

    pub fn on_lbutton_up_with_guard(
        &mut self,
        x: i32,
        y: i32,
        now_ms: u64,
        fg_hwnd: isize,
        is_self_window: bool,
        is_suppressed: impl FnOnce() -> bool,
    ) {
        let down = self.mouse_down.take();
        if is_self_window || fg_hwnd == 0 {
            self.pending = None;
            self.last_up = Some((x, y, now_ms));
            return;
        }

        let mut armed_rect: Option<SelectionAnchorRect> = None;
        if let Some((dx0, dy0, _)) = down {
            let dx = i64::from(x) - i64::from(dx0);
            let dy = i64::from(y) - i64::from(dy0);
            if dx * dx + dy * dy >= DRAG_THRESHOLD_SQ {
                armed_rect = Some(SelectionAnchorRect {
                    left: dx0.min(x) - 4,
                    top: dy0.min(y) - 10,
                    right: dx0.max(x) + 4,
                    bottom: dy0.max(y) + 12,
                });
            }
        }

        if armed_rect.is_none() {
            if let Some((lx, ly, lt)) = self.last_up {
                let dt = now_ms.saturating_sub(lt);
                let dx = i64::from(x) - i64::from(lx);
                let dy = i64::from(y) - i64::from(ly);
                if dt <= DOUBLE_CLICK_MS && dx * dx + dy * dy <= DOUBLE_CLICK_DIST_SQ {
                    armed_rect = Some(SelectionAnchorRect {
                        left: x - 24,
                        top: y - 16,
                        right: x + 24,
                        bottom: y + 16,
                    });
                }
            }
        }

        if armed_rect.is_some() && is_suppressed() {
            armed_rect = None;
        }

        self.pending = armed_rect.map(|rect| PendingSelection {
            rect,
            hwnd: fg_hwnd,
            armed_at_ms: now_ms,
        });
        self.last_up = Some((x, y, now_ms));
    }

    pub fn on_other_mouse_button(&mut self) {
        self.pending = None;
        self.mouse_down = None;
    }

    pub fn on_key_down(
        &mut self,
        vk: u32,
        target_vk: u32,
        has_modifiers: bool,
        is_modifier_only: bool,
        now_ms: u64,
        fg_hwnd: isize,
        is_self_window: bool,
    ) -> Option<SelectionAnchorRect> {
        if is_modifier_only {
            return None;
        }
        let pending = self.pending.take();
        if vk == target_vk && !has_modifiers {
            if let Some(p) = pending {
                if !is_self_window
                    && fg_hwnd != 0
                    && fg_hwnd == p.hwnd
                    && now_ms.saturating_sub(p.armed_at_ms) <= ARM_TTL_MS
                {
                    return Some(p.rect);
                }
            }
        }
        None
    }
}

#[cfg(windows)]
mod imp {
    use super::{
        is_copyable_hglobal_format, should_suppress_smart_selection, RectI32,
        SelectionAnchorRect, SelectionCaptureResult, SmartSelectionKey, SmartSelectionTracker,
        WorkAreaRect,
    };
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{mpsc, Arc, Mutex, OnceLock};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};
    use windows::core::w;
    use windows::Win32::Foundation::{
        GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
    };
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO,
        MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
        GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
        KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_C, VK_CONTROL, VK_LCONTROL,
        VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_NONAME, VK_RCONTROL, VK_RMENU, VK_RSHIFT,
        VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB,
    };
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_NOT_PRESENT, QUNS_PRESENTATION_MODE,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetClassNameW, GetCursorInfo, GetCursorPos,
        GetForegroundWindow, GetMessageW, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
        IsZoomed, PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
        CURSORINFO, CURSOR_SHOWING, GWL_STYLE, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT,
        WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN,
        WM_QUIT, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WS_CAPTION,
    };

    pub(super) const CF_UNICODETEXT: u32 = 13;
    const MAX_FORMAT_BYTES: usize = 32 * 1024 * 1024;
    const MAX_TOTAL_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
    const CLIPBOARD_OPEN_RETRIES: usize = 10;
    const CLIPBOARD_OPEN_SLEEP_MS: u64 = 12;
    const MODIFIER_WAIT_STEPS: usize = 20;
    const MODIFIER_POLL_MS: u64 = 12;
    const SEQUENCE_POLL_STEPS: usize = 24;
    const SEQUENCE_POLL_MS: u64 = 15;

    static SELECTION_CAPTURE_BUSY: AtomicBool = AtomicBool::new(false);

    const MODIFIER_VKS: &[VIRTUAL_KEY] = &[
        VK_LMENU,
        VK_RMENU,
        VK_MENU,
        VK_LSHIFT,
        VK_RSHIFT,
        VK_SHIFT,
        VK_LWIN,
        VK_RWIN,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_CONTROL,
    ];

    struct ReentryGuard;
    impl Drop for ReentryGuard {
        fn drop(&mut self) {
            SELECTION_CAPTURE_BUSY.store(false, Ordering::Release);
        }
    }

    pub(super) struct ClipboardGuard;
    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            let _ = unsafe { CloseClipboard() };
        }
    }

    pub(super) struct GlobalLockGuard(pub HGLOBAL);
    impl Drop for GlobalLockGuard {
        fn drop(&mut self) {
            let _ = unsafe { GlobalUnlock(self.0) };
        }
    }

    struct OwnedHGlobal(Option<HGLOBAL>);
    impl Drop for OwnedHGlobal {
        fn drop(&mut self) {
            if let Some(hmem) = self.0.take() {
                let _ = unsafe { GlobalFree(Some(hmem)) };
            }
        }
    }

    pub(super) fn register_control_formats() -> [u32; 3] {
        unsafe {
            [
                RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")),
                RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")),
                RegisterClipboardFormatW(w!("CanUploadToCloudClipboard")),
            ]
        }
    }

    pub(super) fn open_clipboard_with_retry() -> Result<ClipboardGuard, String> {
        for attempt in 0..CLIPBOARD_OPEN_RETRIES {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(ClipboardGuard);
            }
            if attempt + 1 < CLIPBOARD_OPEN_RETRIES {
                std::thread::sleep(Duration::from_millis(CLIPBOARD_OPEN_SLEEP_MS));
            }
        }
        Err("无法打开系统剪贴板，可能正被其他程序占用".to_string())
    }

    pub(super) fn write_hglobal_format(fmt: u32, bytes: &[u8]) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        let hmem = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }
            .map_err(|e| format!("分配剪贴板内存失败: {e}"))?;
        let mut owned = OwnedHGlobal(Some(hmem));

        let ptr = unsafe { GlobalLock(hmem) };
        if ptr.is_null() {
            return Err("锁定剪贴板内存失败".to_string());
        }
        {
            let _unlock = GlobalLockGuard(hmem);
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
            }
        }

        unsafe { SetClipboardData(fmt, Some(HANDLE(hmem.0))) }
            .map_err(|e| format!("写入剪贴板格式 {fmt} 失败: {e}"))?;
        // Ownership transferred to Windows upon SetClipboardData success.
        owned.0 = None;
        Ok(())
    }

    pub(super) fn read_unicode_text_from_open_clipboard() -> Option<String> {
        let handle = unsafe { GetClipboardData(CF_UNICODETEXT) }.ok()?;
        let hglobal = HGLOBAL(handle.0);
        if hglobal.0.is_null() {
            return None;
        }
        let size = unsafe { GlobalSize(hglobal) };
        if size < 2 {
            return None;
        }
        let ptr = unsafe { GlobalLock(hglobal) };
        if ptr.is_null() {
            return None;
        }
        let _unlock = GlobalLockGuard(hglobal);
        let byte_len = size.min(MAX_FORMAT_BYTES);
        let raw = unsafe { std::slice::from_raw_parts(ptr as *const u8, byte_len) };
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    }

    pub(super) struct ClipboardSnapshot {
        pub entries: Vec<(u32, Vec<u8>)>,
    }

    impl ClipboardSnapshot {
        pub fn capture() -> Result<Self, String> {
            let _clip = open_clipboard_with_retry()?;
            let control_formats = register_control_formats();
            let mut entries = Vec::new();
            let mut total_bytes = 0usize;

            let mut fmt = unsafe { EnumClipboardFormats(0) };
            while fmt != 0 {
                if is_copyable_hglobal_format(fmt) && !control_formats.contains(&fmt) {
                    if let Ok(handle) = unsafe { GetClipboardData(fmt) } {
                        let hglobal = HGLOBAL(handle.0);
                        if !hglobal.0.is_null() {
                            let size = unsafe { GlobalSize(hglobal) };
                            if size > 0
                                && size <= MAX_FORMAT_BYTES
                                && total_bytes.saturating_add(size) <= MAX_TOTAL_SNAPSHOT_BYTES
                            {
                                let ptr = unsafe { GlobalLock(hglobal) };
                                if !ptr.is_null() {
                                    let _unlock = GlobalLockGuard(hglobal);
                                    let bytes = unsafe {
                                        std::slice::from_raw_parts(ptr as *const u8, size)
                                    };
                                    entries.push((fmt, bytes.to_vec()));
                                    total_bytes += size;
                                }
                            }
                        }
                    }
                }
                fmt = unsafe { EnumClipboardFormats(fmt) };
            }

            Ok(Self { entries })
        }

        pub fn restore(self) -> Result<(), String> {
            let _clip = open_clipboard_with_retry()?;
            unsafe { EmptyClipboard() }.map_err(|e| format!("恢复前清空剪贴板失败: {e}"))?;

            for (fmt, bytes) in &self.entries {
                if let Err(err) = write_hglobal_format(*fmt, bytes) {
                    eprintln!("[Luma] 恢复剪贴板格式 {fmt} 失败: {err}");
                }
            }

            let zero_dword = 0u32.to_le_bytes();
            for fmt in register_control_formats() {
                if fmt != 0 {
                    let _ = write_hglobal_format(fmt, &zero_dword);
                }
            }

            Ok(())
        }
    }

    struct ClipboardRestoreGuard {
        snapshot: Option<ClipboardSnapshot>,
        should_restore: bool,
    }

    impl ClipboardRestoreGuard {
        fn restore_now(&mut self) {
            if self.should_restore {
                self.should_restore = false;
                if let Some(snap) = self.snapshot.take() {
                    if let Err(err) = snap.restore() {
                        eprintln!("[Luma] 恢复原剪贴板失败: {err}");
                    }
                }
            }
        }
    }

    impl Drop for ClipboardRestoreGuard {
        fn drop(&mut self) {
            self.restore_now();
        }
    }

    fn is_key_down(vk: VIRTUAL_KEY) -> bool {
        unsafe { (GetAsyncKeyState(i32::from(vk.0)) as u16 & 0x8000) != 0 }
    }

    fn keyboard_input(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: super::LUMA_INJECTED_INPUT_MARKER,
                },
            },
        }
    }

    fn release_modifiers_and_send_copy() -> Result<(), String> {
        for step in 0..MODIFIER_WAIT_STEPS {
            if !MODIFIER_VKS.iter().any(|&vk| is_key_down(vk)) {
                break;
            }
            if step + 1 < MODIFIER_WAIT_STEPS {
                std::thread::sleep(Duration::from_millis(MODIFIER_POLL_MS));
            }
        }

        let pressed: Vec<VIRTUAL_KEY> = MODIFIER_VKS
            .iter()
            .copied()
            .filter(|&vk| is_key_down(vk))
            .collect();

        if !pressed.is_empty() {
            let mut release_inputs = Vec::with_capacity(pressed.len() + 2);
            let alt_down = pressed
                .iter()
                .any(|&vk| vk == VK_MENU || vk == VK_LMENU || vk == VK_RMENU);
            if alt_down {
                // Suppress Win32 menu bar activation when Alt is released programmatically.
                release_inputs.push(keyboard_input(VK_NONAME, KEYBD_EVENT_FLAGS(0)));
                release_inputs.push(keyboard_input(VK_NONAME, KEYEVENTF_KEYUP));
            }
            for vk in pressed {
                release_inputs.push(keyboard_input(vk, KEYEVENTF_KEYUP));
            }
            unsafe {
                let _ = SendInput(&release_inputs, std::mem::size_of::<INPUT>() as i32);
            }
            std::thread::sleep(Duration::from_millis(12));
        }

        let copy_inputs = [
            keyboard_input(VK_CONTROL, KEYBD_EVENT_FLAGS(0)),
            keyboard_input(VK_C, KEYBD_EVENT_FLAGS(0)),
            keyboard_input(VK_C, KEYEVENTF_KEYUP),
            keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
        ];
        let sent = unsafe { SendInput(&copy_inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent == 0 {
            return Err("无法向当前窗口发送复制快捷键（目标窗口可能具有管理员权限）".to_string());
        }
        Ok(())
    }

    pub fn capture_cursor_point() -> Option<(i32, i32)> {
        let mut pt = POINT::default();
        unsafe { GetCursorPos(&mut pt) }.ok()?;
        Some((pt.x, pt.y))
    }

    pub fn work_area_for_point(x: i32, y: i32) -> Option<WorkAreaRect> {
        let hmon = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
        if hmon.0.is_null() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(hmon, &mut info) }.as_bool() {
            return None;
        }
        let work = info.rcWork;
        Some(WorkAreaRect {
            left: work.left,
            top: work.top,
            right: work.right,
            bottom: work.bottom,
            scale_factor: 1.0,
        })
    }

    pub fn capture_selected_text_without_polluting_clipboard(
    ) -> Result<SelectionCaptureResult, String> {
        if SELECTION_CAPTURE_BUSY
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("正在获取划词内容，请稍候".to_string());
        }
        let _reentry = ReentryGuard;

        let (cursor_x, cursor_y) = capture_cursor_point().unwrap_or((100, 100));
        let snapshot = ClipboardSnapshot::capture()?;
        let mut restore_guard = ClipboardRestoreGuard {
            snapshot: Some(snapshot),
            should_restore: false,
        };

        let seq_before = unsafe { GetClipboardSequenceNumber() };
        release_modifiers_and_send_copy()?;

        let mut seq_changed = false;
        for _ in 0..SEQUENCE_POLL_STEPS {
            std::thread::sleep(Duration::from_millis(SEQUENCE_POLL_MS));
            if unsafe { GetClipboardSequenceNumber() } != seq_before {
                seq_changed = true;
                break;
            }
        }

        if !seq_changed {
            return Err("未检测到选中的文字，请先用鼠标划选文字后再按快捷键".to_string());
        }

        // Clipboard changed: unconditionally restore the original snapshot even if reading fails.
        restore_guard.should_restore = true;

        let copied_text = {
            let _clip = open_clipboard_with_retry()?;
            read_unicode_text_from_open_clipboard()
        };

        restore_guard.restore_now();

        let trimmed = copied_text.unwrap_or_default().trim().to_string();
        if trimmed.is_empty() {
            return Err("选中的内容不包含可翻译文本".to_string());
        }

        Ok(SelectionCaptureResult {
            text: trimmed,
            cursor_x,
            cursor_y,
        })
    }

    type SmartCallback = Arc<dyn Fn(SelectionAnchorRect) + Send + Sync>;

    struct HookShared {
        target_vk: AtomicU32,
        tracker: Mutex<SmartSelectionTracker>,
        started_at: Instant,
        trigger_tx: Mutex<Option<mpsc::SyncSender<SelectionAnchorRect>>>,
        self_pid: u32,
    }

    static HOOK_SHARED: OnceLock<HookShared> = OnceLock::new();

    struct HookThreadController {
        hook_tid: u32,
        hook_join: JoinHandle<()>,
        worker_join: JoinHandle<()>,
    }

    static HOOK_CONTROLLER: OnceLock<Mutex<Option<HookThreadController>>> = OnceLock::new();

    fn shared_state() -> &'static HookShared {
        HOOK_SHARED.get_or_init(|| HookShared {
            target_vk: AtomicU32::new(0),
            tracker: Mutex::new(SmartSelectionTracker::default()),
            started_at: Instant::now(),
            trigger_tx: Mutex::new(None),
            self_pid: std::process::id(),
        })
    }

    fn key_to_vk(key: SmartSelectionKey) -> u32 {
        match key {
            SmartSelectionKey::Space => u32::from(VK_SPACE.0),
            SmartSelectionKey::Tab => u32::from(VK_TAB.0),
        }
    }

    fn foreground_info(self_pid: u32) -> (isize, bool) {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return (0, false);
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        (hwnd.0 as isize, pid == self_pid)
    }

    fn now_ms_from(shared: &HookShared) -> u64 {
        shared.started_at.elapsed().as_millis() as u64
    }

    fn is_modifier_vk(vk: u32) -> bool {
        matches!(
            VIRTUAL_KEY(vk as u16),
            VK_CONTROL
                | VK_LCONTROL
                | VK_RCONTROL
                | VK_MENU
                | VK_LMENU
                | VK_RMENU
                | VK_SHIFT
                | VK_LSHIFT
                | VK_RSHIFT
                | VK_LWIN
                | VK_RWIN
        )
    }

    fn rect_i32(r: RECT) -> RectI32 {
        RectI32 {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }

    fn is_smart_selection_suppressed_for_hwnd(fg_hwnd: isize) -> bool {
        if let Ok(quns) = unsafe { SHQueryUserNotificationState() } {
            if matches!(
                quns,
                QUNS_NOT_PRESENT
                    | QUNS_BUSY
                    | QUNS_RUNNING_D3D_FULL_SCREEN
                    | QUNS_PRESENTATION_MODE
            ) {
                return true;
            }
        }

        let mut ci = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetCursorInfo(&mut ci) }.is_ok() && (ci.flags.0 & CURSOR_SHOWING.0) == 0 {
            return true;
        }

        if fg_hwnd == 0 {
            return true;
        }
        let hwnd = HWND(fg_hwnd as *mut core::ffi::c_void);

        let mut cls_buf = [0u16; 128];
        let cls_len = unsafe { GetClassNameW(hwnd, &mut cls_buf) };
        if cls_len > 0 {
            let cls = String::from_utf16_lossy(&cls_buf[..cls_len as usize]);
            if matches!(cls.as_str(), "Progman" | "WorkerW") {
                return true;
            }
        }

        let mut dwm_rect = RECT::default();
        let win_rect = if unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                (&mut dwm_rect as *mut RECT).cast(),
                std::mem::size_of::<RECT>() as u32,
            )
        }
        .is_ok()
        {
            Some(rect_i32(dwm_rect))
        } else {
            let mut raw_rect = RECT::default();
            unsafe { GetWindowRect(hwnd, &mut raw_rect) }
                .ok()
                .map(|()| rect_i32(raw_rect))
        };

        let hmon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let (monitor_rect, work_rect) = if !hmon.0.is_null() {
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if unsafe { GetMonitorInfoW(hmon, &mut mi) }.as_bool() {
                (Some(rect_i32(mi.rcMonitor)), Some(rect_i32(mi.rcWork)))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
        let has_caption = (style & WS_CAPTION.0) == WS_CAPTION.0;
        let is_zoomed = unsafe { IsZoomed(hwnd) }.as_bool();

        should_suppress_smart_selection(
            false,
            false,
            false,
            win_rect,
            monitor_rect,
            work_rect,
            has_caption,
            is_zoomed,
        )
    }

    unsafe extern "system" fn mouse_hook_proc(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if code >= 0 && lparam.0 != 0 {
            let msg = wparam.0 as u32;
            if matches!(
                msg,
                WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_MBUTTONDOWN
            ) {
                let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
                let shared = shared_state();
                if shared.target_vk.load(Ordering::Acquire) != 0 {
                    if msg == WM_LBUTTONDOWN {
                        if let Ok(mut tracker) = shared.tracker.try_lock() {
                            tracker.on_lbutton_down(info.pt.x, info.pt.y, now_ms_from(shared));
                        }
                    } else if msg == WM_LBUTTONUP {
                        let (fg_hwnd, is_self) = foreground_info(shared.self_pid);
                        if let Ok(mut tracker) = shared.tracker.try_lock() {
                            tracker.on_lbutton_up_with_guard(
                                info.pt.x,
                                info.pt.y,
                                now_ms_from(shared),
                                fg_hwnd,
                                is_self,
                                || is_smart_selection_suppressed_for_hwnd(fg_hwnd),
                            );
                        }
                    } else if let Ok(mut tracker) = shared.tracker.try_lock() {
                        tracker.on_other_mouse_button();
                    }
                }
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    unsafe extern "system" fn keyboard_hook_proc(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if code >= 0 && lparam.0 != 0 {
            let msg = wparam.0 as u32;
            if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
                let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
                // Ignore only Luma's own synthetic keystrokes (e.g. the Ctrl+C it injects to read
                // the selection). Injected keys from the On-Screen Keyboard, key remappers or
                // accessibility tools must still be able to trigger the smart single key.
                let is_own_injection =
                    super::is_own_injected_key(kb.flags.0, kb.dwExtraInfo);
                let shared = shared_state();
                let target_vk = shared.target_vk.load(Ordering::Acquire);
                if target_vk != 0 && !is_own_injection {
                    let mod_only = is_modifier_vk(kb.vkCode);
                    let has_mods = is_key_down(VK_CONTROL)
                        || is_key_down(VK_MENU)
                        || is_key_down(VK_SHIFT)
                        || is_key_down(VK_LWIN)
                        || is_key_down(VK_RWIN);
                    let (fg_hwnd, is_self) = foreground_info(shared.self_pid);
                    let triggered = if let Ok(mut tracker) = shared.tracker.try_lock() {
                        tracker.on_key_down(
                            kb.vkCode,
                            target_vk,
                            has_mods,
                            mod_only,
                            now_ms_from(shared),
                            fg_hwnd,
                            is_self,
                        )
                    } else {
                        None
                    };
                    if let Some(rect) = triggered {
                        if !is_smart_selection_suppressed_for_hwnd(fg_hwnd) {
                            if let Ok(guard) = shared.trigger_tx.try_lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.try_send(rect);
                                }
                            }
                            return LRESULT(1);
                        }
                    }
                }
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    fn stop_hook_locked(slot: &mut Option<HookThreadController>) {
        shared_state().target_vk.store(0, Ordering::Release);
        if let Ok(mut tx_guard) = shared_state().trigger_tx.lock() {
            *tx_guard = None;
        }
        if let Ok(mut tracker) = shared_state().tracker.lock() {
            *tracker = SmartSelectionTracker::default();
        }
        if let Some(ctrl) = slot.take() {
            unsafe {
                let _ = PostThreadMessageW(ctrl.hook_tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            let _ = ctrl.hook_join.join();
            let _ = ctrl.worker_join.join();
        }
    }

    pub fn configure_smart_selection_hook(
        target: Option<SmartSelectionKey>,
        on_trigger: Option<SmartCallback>,
    ) -> Result<(), String> {
        let controller_lock = HOOK_CONTROLLER.get_or_init(|| Mutex::new(None));
        let mut slot = controller_lock
            .lock()
            .map_err(|_| "智能划词监听锁异常".to_string())?;

        let Some(key) = target else {
            stop_hook_locked(&mut slot);
            return Ok(());
        };

        let callback = on_trigger.ok_or_else(|| "缺少划词触发回调".to_string())?;

        // Restart cleanly so worker holds latest callback without stale closures.
        stop_hook_locked(&mut slot);

        let (tx, rx) = mpsc::sync_channel::<SelectionAnchorRect>(2);
        if let Ok(mut tx_guard) = shared_state().trigger_tx.lock() {
            *tx_guard = Some(tx);
        }
        shared_state().target_vk.store(key_to_vk(key), Ordering::Release);

        let worker_join = std::thread::Builder::new()
            .name("luma-smart-sel-worker".to_string())
            .spawn(move || {
                while let Ok(rect) = rx.recv() {
                    callback(rect);
                }
            })
            .map_err(|e| format!("启动划词触发工作线程失败: {e}"))?;

        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<u32, String>>(1);
        let hook_join = std::thread::Builder::new()
            .name("luma-smart-sel-hook".to_string())
            .spawn(move || {
                let tid = unsafe { GetCurrentThreadId() };
                let mouse_hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), None, 0) };
                let Ok(m_hook) = mouse_hook else {
                    let _ = ready_tx.send(Err("注册鼠标低级钩子失败".to_string()));
                    return;
                };
                let kb_hook =
                    unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), None, 0) };
                let Ok(k_hook) = kb_hook else {
                    unsafe {
                        let _ = UnhookWindowsHookEx(m_hook);
                    }
                    let _ = ready_tx.send(Err("注册键盘低级钩子失败".to_string()));
                    return;
                };
                let _ = ready_tx.send(Ok(tid));
                let mut msg = MSG::default();
                while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                unsafe {
                    let _ = UnhookWindowsHookEx(k_hook);
                    let _ = UnhookWindowsHookEx(m_hook);
                }
            })
            .map_err(|e| format!("启动划词监听线程失败: {e}"))?;

        match ready_rx.recv() {
            Ok(Ok(hook_tid)) => {
                *slot = Some(HookThreadController {
                    hook_tid,
                    hook_join,
                    worker_join,
                });
                Ok(())
            }
            Ok(Err(err)) => {
                shared_state().target_vk.store(0, Ordering::Release);
                if let Ok(mut tx_guard) = shared_state().trigger_tx.lock() {
                    *tx_guard = None;
                }
                let _ = hook_join.join();
                let _ = worker_join.join();
                Err(err)
            }
            Err(_) => {
                shared_state().target_vk.store(0, Ordering::Release);
                if let Ok(mut tx_guard) = shared_state().trigger_tx.lock() {
                    *tx_guard = None;
                }
                let _ = hook_join.join();
                let _ = worker_join.join();
                Err("划词监听线程启动异常".to_string())
            }
        }
    }
}

/// Capture the current cursor position in physical screen coordinates.
#[allow(dead_code)]
pub fn capture_cursor_point() -> Option<(i32, i32)> {
    #[cfg(windows)]
    {
        imp::capture_cursor_point()
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos_bridge::capture_cursor_point()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// Query the monitor work-area rectangle (excluding taskbar) containing physical point `(x, y)`.
pub fn work_area_for_point(x: i32, y: i32) -> Option<WorkAreaRect> {
    #[cfg(windows)]
    {
        imp::work_area_for_point(x, y)
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos_bridge::work_area_for_point(x, y)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (x, y);
        None
    }
}

/// Temporarily copy the current selection via `Ctrl+C` without polluting the user's clipboard
/// or Windows clipboard history, returning the trimmed text and initial cursor coordinates.
pub fn capture_selected_text_without_polluting_clipboard() -> Result<SelectionCaptureResult, String>
{
    #[cfg(windows)]
    {
        imp::capture_selected_text_without_polluting_clipboard()
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos_bridge::capture_selected_text_without_polluting_clipboard()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err("当前系统不支持全局划词取词".to_string())
    }
}

/// Configure or tear down the OS-level smart single-key (`Space` / `Tab`) selection listener.
pub fn configure_smart_selection_hook(
    target: Option<SmartSelectionKey>,
    on_trigger: Option<Arc<dyn Fn(SelectionAnchorRect) + Send + Sync>>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        imp::configure_smart_selection_hook(target, on_trigger)
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos_bridge::configure_smart_selection_hook(target, on_trigger)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (target, on_trigger);
        if target.is_some() {
            Err("单键智能划词仅支持 Windows 和 macOS".to_string())
        } else {
            Ok(())
        }
    }
}

/// Tear down any active smart selection listener cleanly on shutdown.
pub fn stop_smart_selection_hook() {
    let _ = configure_smart_selection_hook(None, None);
}

#[cfg(test)]
mod tests {
    use super::{
        compute_cursor_popup_position, compute_selection_popup_position, is_copyable_hglobal_format,
        parse_smart_selection_key, SelectionAnchorRect, SmartSelectionKey, SmartSelectionTracker,
        WorkAreaRect,
    };

    #[test]
    fn filters_copyable_hglobal_formats() {
        assert!(is_copyable_hglobal_format(1)); // CF_TEXT
        assert!(is_copyable_hglobal_format(13)); // CF_UNICODETEXT
        assert!(is_copyable_hglobal_format(15)); // CF_HDROP
        assert!(is_copyable_hglobal_format(0xC001)); // custom registered format

        assert!(!is_copyable_hglobal_format(0));
        assert!(!is_copyable_hglobal_format(2)); // CF_BITMAP
        assert!(!is_copyable_hglobal_format(3)); // CF_METAFILEPICT
        assert!(!is_copyable_hglobal_format(9)); // CF_PALETTE
        assert!(!is_copyable_hglobal_format(14)); // CF_ENHMETAFILE
        assert!(!is_copyable_hglobal_format(0x0080)); // CF_OWNERDISPLAY
        assert!(!is_copyable_hglobal_format(0x0081)); // CF_DSPTEXT
        assert!(!is_copyable_hglobal_format(0x0082)); // CF_DSPBITMAP
        assert!(!is_copyable_hglobal_format(0x0083)); // CF_DSPMETAFILEPICT
        assert!(!is_copyable_hglobal_format(0x008E)); // CF_DSPENHMETAFILE
        assert!(!is_copyable_hglobal_format(0x0200)); // CF_PRIVATEFIRST
        assert!(!is_copyable_hglobal_format(0x02FF)); // CF_PRIVATELAST
        assert!(!is_copyable_hglobal_format(0x0300)); // CF_GDIOBJFIRST
        assert!(!is_copyable_hglobal_format(0x03FF)); // CF_GDIOBJLAST
    }

    #[test]
    fn computes_cursor_popup_position_with_flip_and_clamping() {
        let primary = WorkAreaRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
            scale_factor: 1.0,
        };

        // Normal case: places below cursor and right of cursor horizontally.
        let (x, y) = compute_cursor_popup_position(600, 260, primary, 392.0, 210.0);
        assert_eq!(y, 260 + 28);
        assert!(x >= 600 && x <= 1920 - 392 - 12);

        // Bottom edge: flips above cursor (cursor_y - h_phys - 24 * scale).
        let (fx, fy) = compute_cursor_popup_position(600, 900, primary, 392.0, 210.0);
        assert_eq!(fy, 900 - 210 - 24);
        assert!(fx >= 12 && fx <= 1920 - 392 - 12);

        // Multi-monitor negative coordinates with HiDPI scaling.
        let secondary = WorkAreaRect {
            left: -1920,
            top: -120,
            right: 0,
            bottom: 920,
            scale_factor: 1.5,
        };
        let (sx, sy) = compute_cursor_popup_position(-1915, 860, secondary, 392.0, 210.0);
        let w_phys = (392.0 * 1.5f64).round() as i32;
        let h_phys = (210.0 * 1.5f64).round() as i32;
        let margin = (12.0 * 1.5f64).round() as i32;
        assert_eq!(sx, -1900);
        assert!(sx >= secondary.left + margin && sx <= secondary.right - w_phys - margin);
        assert!(sy >= secondary.top + margin && sy <= secondary.bottom - h_phys - margin);

        // Small monitor clamping: max < min falls back safely to left+margin / top+margin.
        let tiny = WorkAreaRect {
            left: 40,
            top: 20,
            right: 340,
            bottom: 240,
            scale_factor: 1.0,
        };
        let (tx, ty) = compute_cursor_popup_position(200, 150, tiny, 392.0, 210.0);
        assert_eq!((tx, ty), (40 + 12, 20 + 12));
    }

    #[test]
    fn selection_popup_never_covers_selected_text_rect() {
        let primary = WorkAreaRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
            scale_factor: 1.0,
        };
        let sel = SelectionAnchorRect {
            left: 200,
            top: 180,
            right: 520,
            bottom: 220,
        };
        let (bx, by, below) =
            compute_selection_popup_position(520, 220, Some(sel), primary, 392.0, 210.0);
        assert!(below);
        assert_eq!(bx, 200);
        assert_eq!(by, 220 + 18);
        assert!(by > sel.bottom);

        let bottom_sel = SelectionAnchorRect {
            left: 200,
            top: 860,
            right: 520,
            bottom: 900,
        };
        let (ax, ay, is_below) =
            compute_selection_popup_position(520, 900, Some(bottom_sel), primary, 392.0, 210.0);
        assert!(!is_below);
        assert_eq!(ax, 200);
        assert_eq!(ay, 860 - 18 - 210);
        assert!(ay + 210 < bottom_sel.top);
    }

    #[test]
    fn only_luma_own_injected_keys_are_ignored_by_smart_hook() {
        use super::{is_own_injected_key, LUMA_INJECTED_INPUT_MARKER};
        // Physical keystroke.
        assert!(!is_own_injected_key(0, 0));
        // Injected by another tool (On-Screen Keyboard, remapper, automation).
        assert!(!is_own_injected_key(0x10, 0));
        assert!(!is_own_injected_key(0x10, 0x1234));
        // Luma's own synthetic Ctrl+C.
        assert!(is_own_injected_key(0x10, LUMA_INJECTED_INPUT_MARKER));
        // Marker without the injected flag cannot be ours.
        assert!(!is_own_injected_key(0, LUMA_INJECTED_INPUT_MARKER));
    }

    #[test]
    fn parses_and_tracks_smart_single_key_selection() {
        assert_eq!(parse_smart_selection_key("Space"), Some(SmartSelectionKey::Space));
        assert_eq!(parse_smart_selection_key(" tab "), Some(SmartSelectionKey::Tab));
        assert_eq!(parse_smart_selection_key("Alt+E"), None);

        let mut tracker = SmartSelectionTracker::default();
        const VK_SP: u32 = 0x20;
        const VK_A: u32 = 0x41;

        // Normal Space without selection does not trigger.
        assert_eq!(
            tracker.on_key_down(VK_SP, VK_SP, false, false, 100, 1001, false),
            None
        );

        // Simple click or tiny 8px slip (< 10px threshold) does not arm.
        tracker.on_lbutton_down(300, 300, 200);
        tracker.on_lbutton_up(308, 300, 240, 1001, false);
        assert_eq!(
            tracker.on_key_down(VK_SP, VK_SP, false, false, 280, 1001, false),
            None
        );

        // Guarded suppression prevents arming even after a valid drag.
        tracker.on_lbutton_down(120, 200, 500);
        tracker.on_lbutton_up_with_guard(280, 220, 620, 1001, false, || true);
        assert_eq!(
            tracker.on_key_down(VK_SP, VK_SP, false, false, 700, 1001, false),
            None
        );

        // Drag selection arms and triggers once on Space, then disarms.
        tracker.on_lbutton_down(120, 200, 1000);
        tracker.on_lbutton_up(280, 220, 1200, 1001, false);
        let armed = tracker.on_key_down(VK_SP, VK_SP, false, false, 1500, 1001, false);
        assert!(armed.is_some());
        assert_eq!(
            tracker.on_key_down(VK_SP, VK_SP, false, false, 1600, 1001, false),
            None
        );

        // Double click arms too, but typing another key disarms it.
        tracker.on_lbutton_down(400, 400, 3000);
        tracker.on_lbutton_up(400, 400, 3050, 1001, false);
        tracker.on_lbutton_down(401, 401, 3150);
        tracker.on_lbutton_up(401, 401, 3200, 1001, false);
        assert_eq!(
            tracker.on_key_down(VK_A, VK_SP, false, false, 3300, 1001, false),
            None
        );
        assert_eq!(
            tracker.on_key_down(VK_SP, VK_SP, false, false, 3400, 1001, false),
            None
        );

        // Expired selection (> 4000ms) does not trigger.
        tracker.on_lbutton_down(100, 100, 4000);
        tracker.on_lbutton_up(220, 110, 4100, 1001, false);
        assert_eq!(
            tracker.on_key_down(VK_SP, VK_SP, false, false, 8200, 1001, false),
            None
        );
    }

    #[test]
    fn suppresses_smart_selection_in_games_and_fullscreen_only() {
        use super::{should_suppress_smart_selection, RectI32};

        let mon = RectI32 {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1440,
        };
        let work_with_taskbar = RectI32 {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1382,
        };
        let windowed = RectI32 {
            left: 160,
            top: 90,
            right: 1800,
            bottom: 1100,
        };

        // Direct game / fullscreen / desktop indicators suppress immediately.
        assert!(should_suppress_smart_selection(
            true,
            false,
            false,
            Some(windowed),
            Some(mon),
            Some(work_with_taskbar),
            true,
            false,
        ));
        assert!(should_suppress_smart_selection(
            false,
            true,
            false,
            Some(windowed),
            Some(mon),
            Some(work_with_taskbar),
            true,
            false,
        ));
        assert!(should_suppress_smart_selection(
            false,
            false,
            true,
            Some(mon),
            Some(mon),
            Some(work_with_taskbar),
            false,
            false,
        ));

        // Borderless fullscreen game over visible taskbar.
        assert!(should_suppress_smart_selection(
            false,
            false,
            false,
            Some(mon),
            Some(mon),
            Some(work_with_taskbar),
            false,
            false,
        ));

        // Borderless fullscreen game even with auto-hide taskbar (work == mon).
        assert!(should_suppress_smart_selection(
            false,
            false,
            false,
            Some(mon),
            Some(mon),
            Some(mon),
            false,
            false,
        ));

        // Maximized browser / editor with visible taskbar does NOT suppress.
        assert!(!should_suppress_smart_selection(
            false,
            false,
            false,
            Some(work_with_taskbar),
            Some(mon),
            Some(work_with_taskbar),
            true,
            true,
        ));

        // Maximized browser / editor with auto-hide taskbar does NOT suppress.
        assert!(!should_suppress_smart_selection(
            false,
            false,
            false,
            Some(mon),
            Some(mon),
            Some(mon),
            true,
            true,
        ));

        // Normal windowed app does NOT suppress.
        assert!(!should_suppress_smart_selection(
            false,
            false,
            false,
            Some(windowed),
            Some(mon),
            Some(work_with_taskbar),
            true,
            false,
        ));
    }

    #[cfg(windows)]
    #[test]
    fn snapshots_overwrites_and_restores_multiformat_clipboard() {
        use super::imp::{
            open_clipboard_with_retry, read_unicode_text_from_open_clipboard,
            register_control_formats, write_hglobal_format, ClipboardSnapshot, GlobalLockGuard,
            CF_UNICODETEXT,
        };
        use windows::core::w;
        use windows::Win32::Foundation::HGLOBAL;
        use windows::Win32::System::DataExchange::{
            EmptyClipboard, GetClipboardData, RegisterClipboardFormatW,
        };
        use windows::Win32::System::Memory::{GlobalLock, GlobalSize};

        let Ok(pre_test) = ClipboardSnapshot::capture() else {
            return;
        };
        struct PreTestRestore(Option<ClipboardSnapshot>);
        impl Drop for PreTestRestore {
            fn drop(&mut self) {
                if let Some(snap) = self.0.take() {
                    let _ = snap.restore();
                }
            }
        }
        let _pre_test_guard = PreTestRestore(Some(pre_test));

        let custom_fmt = unsafe { RegisterClipboardFormatW(w!("LumaSelectionCaptureTestFmt")) };
        assert_ne!(custom_fmt, 0);

        let unicode_sample = "Hello 世界 🚀";
        let unicode_bytes: Vec<u8> = unicode_sample
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        let custom_bytes = b"luma-custom-payload-01";

        {
            let Ok(_clip) = open_clipboard_with_retry() else {
                return;
            };
            unsafe { EmptyClipboard() }.expect("empty clipboard for test setup");
            write_hglobal_format(CF_UNICODETEXT, &unicode_bytes).expect("write test unicode");
            write_hglobal_format(custom_fmt, custom_bytes).expect("write test custom format");
        }

        let snapshot = ClipboardSnapshot::capture().expect("capture test snapshot");
        assert!(snapshot.entries.iter().any(|(f, _)| *f == CF_UNICODETEXT));
        assert!(snapshot
            .entries
            .iter()
            .any(|(f, b)| *f == custom_fmt && b.starts_with(custom_bytes)));

        {
            let _clip = open_clipboard_with_retry().expect("open clipboard to overwrite");
            unsafe { EmptyClipboard() }.expect("empty clipboard before overwrite");
            let overwrite_bytes: Vec<u8> = "OVERWRITTEN"
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(u16::to_le_bytes)
                .collect();
            write_hglobal_format(CF_UNICODETEXT, &overwrite_bytes).expect("write overwrite text");
        }

        {
            let _clip = open_clipboard_with_retry().expect("open clipboard to verify overwrite");
            assert_eq!(
                read_unicode_text_from_open_clipboard().as_deref(),
                Some("OVERWRITTEN")
            );
            assert!(unsafe { GetClipboardData(custom_fmt) }.is_err());
        }

        snapshot.restore().expect("restore test snapshot");

        {
            let _clip = open_clipboard_with_retry().expect("open clipboard to verify restore");
            assert_eq!(
                read_unicode_text_from_open_clipboard().as_deref(),
                Some(unicode_sample)
            );
            let handle = unsafe { GetClipboardData(custom_fmt) }.expect("restored custom format");
            let hglobal = HGLOBAL(handle.0);
            let size = unsafe { GlobalSize(hglobal) };
            assert!(size >= custom_bytes.len());
            let ptr = unsafe { GlobalLock(hglobal) };
            assert!(!ptr.is_null());
            {
                let _unlock = GlobalLockGuard(hglobal);
                let restored = unsafe {
                    std::slice::from_raw_parts(ptr as *const u8, custom_bytes.len())
                };
                assert_eq!(restored, custom_bytes);
            }

            for fmt in register_control_formats() {
                if fmt != 0 {
                    let h = unsafe { GetClipboardData(fmt) }.expect("control format present");
                    let hg = HGLOBAL(h.0);
                    let csize = unsafe { GlobalSize(hg) };
                    assert!(csize >= 4);
                    let cptr = unsafe { GlobalLock(hg) };
                    assert!(!cptr.is_null());
                    let _unlock = GlobalLockGuard(hg);
                    let cbytes = unsafe { std::slice::from_raw_parts(cptr as *const u8, 4) };
                    assert_eq!(cbytes, &[0, 0, 0, 0]);
                }
            }
        }
    }
}

//! Source-window-scoped floating popover behavior for `Luma Result`.
//!
//! When the user keeps the popup unpinned (`WS_EX_TOPMOST` off):
//! - Switching to another application (`Alt+Tab`, taskbar, or another window) preserves the
//!   result window underneath without covering the newly focused application.
//! - Switching back to the originating application re-raises `Luma Result` directly above that
//!   source window without stealing focus (`SWP_NOACTIVATE`).
//! - Clicking blank space inside the originating window or on the desktop wallpaper while in the
//!   source/popup layer automatically dismisses the popup without swallowing the click.

use std::sync::Arc;

/// Grace period after popup activation to ignore the tail of a shortcut/drag gesture.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const CLICK_DISMISS_GRACE_MS: u64 = 250;

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutsideClickDecision {
    Keep,
    Dismiss,
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_desktop_class(class_name: &str) -> bool {
    matches!(class_name, "Progman" | "WorkerW")
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_taskbar_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "NotifyIconOverflowWindow"
            | "TaskListThumbnailWnd"
            | "TopLevelWindowForOverflowXamlIsland"
    )
}

/// Decide whether a global mouse down outside `result_hwnd` should dismiss an unpinned popup.
#[cfg_attr(not(windows), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn decide_outside_click(
    elapsed_since_activate_ms: u64,
    is_pinned: bool,
    in_result_rect: bool,
    clicked_root: isize,
    clicked_root_owner: isize,
    clicked_root_class: &str,
    result_hwnd: isize,
    source_hwnd: Option<isize>,
    fg_root: isize,
) -> OutsideClickDecision {
    if elapsed_since_activate_ms < CLICK_DISMISS_GRACE_MS
        || is_pinned
        || in_result_rect
        || result_hwnd == 0
    {
        return OutsideClickDecision::Keep;
    }

    if clicked_root == result_hwnd || clicked_root_owner == result_hwnd {
        return OutsideClickDecision::Keep;
    }

    if is_taskbar_class(clicked_root_class) {
        return OutsideClickDecision::Keep;
    }

    let fg_is_popup_or_source =
        fg_root == result_hwnd || source_hwnd.is_some_and(|s| s != 0 && fg_root == s);

    if is_desktop_class(clicked_root_class) && fg_is_popup_or_source {
        return OutsideClickDecision::Dismiss;
    }

    if let Some(src) = source_hwnd.filter(|&h| h != 0) {
        if clicked_root == src && fg_is_popup_or_source {
            return OutsideClickDecision::Dismiss;
        }
    }

    OutsideClickDecision::Keep
}

/// Decide whether `EVENT_SYSTEM_FOREGROUND` moving onto `fg_root` should re-raise `Luma Result`.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn should_raise_for_foreground(
    active: bool,
    is_visible: bool,
    is_pinned: bool,
    fg_root: isize,
    source_hwnd: Option<isize>,
) -> bool {
    active
        && is_visible
        && !is_pinned
        && source_hwnd.is_some_and(|s| s != 0 && fg_root == s)
}

#[cfg(windows)]
mod imp {
    use super::{
        decide_outside_click, is_desktop_class, is_taskbar_class, should_raise_for_foreground,
        OutsideClickDecision,
    };
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
    use std::sync::{mpsc, Arc, Mutex, OnceLock};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetAncestor, GetClassNameW, GetForegroundWindow,
        GetMessageW, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
        PostThreadMessageW, SetWindowPos, SetWindowsHookExW, TranslateMessage,
        UnhookWindowsHookEx, WindowFromPoint, EVENT_SYSTEM_FOREGROUND, GA_ROOT, GA_ROOTOWNER,
        GWL_EXSTYLE, HWND_TOP, MSG, MSLLHOOKSTRUCT, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        WH_MOUSE_LL, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_LBUTTONDOWN, WM_QUIT,
        WM_RBUTTONDOWN, WS_EX_TOPMOST,
    };

    type DismissCallback = Arc<dyn Fn() + Send + Sync>;

    enum PopoverAction {
        Dismiss,
        RaiseAboveSource { result_hwnd: isize, source_hwnd: isize },
    }

    struct PopoverShared {
        active: AtomicBool,
        result_hwnd: AtomicIsize,
        source_hwnd: AtomicIsize,
        activated_at_ms: AtomicU64,
        started_at: Instant,
        action_tx: Mutex<Option<mpsc::SyncSender<PopoverAction>>>,
        on_dismiss: Mutex<Option<DismissCallback>>,
        self_pid: u32,
    }

    struct WatcherController {
        hook_tid: u32,
        hook_join: JoinHandle<()>,
        worker_join: JoinHandle<()>,
    }

    static SHARED: OnceLock<PopoverShared> = OnceLock::new();
    static CONTROLLER: OnceLock<Mutex<Option<WatcherController>>> = OnceLock::new();

    fn shared() -> &'static PopoverShared {
        SHARED.get_or_init(|| PopoverShared {
            active: AtomicBool::new(false),
            result_hwnd: AtomicIsize::new(0),
            source_hwnd: AtomicIsize::new(0),
            activated_at_ms: AtomicU64::new(0),
            started_at: Instant::now(),
            action_tx: Mutex::new(None),
            on_dismiss: Mutex::new(None),
            self_pid: std::process::id(),
        })
    }

    fn now_ms() -> u64 {
        shared().started_at.elapsed().as_millis() as u64
    }

    fn window_class_name(hwnd: HWND) -> String {
        if hwnd.0.is_null() {
            return String::new();
        }
        let mut buf = [0u16; 128];
        let len = unsafe { GetClassNameW(hwnd, &mut buf) };
        if len <= 0 {
            return String::new();
        }
        let slice_len = (len as usize).min(buf.len());
        String::from_utf16_lossy(&buf[..slice_len])
    }

    fn root_hwnd(hwnd: HWND) -> isize {
        if hwnd.0.is_null() {
            return 0;
        }
        let r = unsafe { GetAncestor(hwnd, GA_ROOT) };
        if r.0.is_null() {
            hwnd.0 as isize
        } else {
            r.0 as isize
        }
    }

    fn root_owner_hwnd(hwnd: HWND) -> isize {
        if hwnd.0.is_null() {
            return 0;
        }
        let r = unsafe { GetAncestor(hwnd, GA_ROOTOWNER) };
        r.0 as isize
    }

    fn is_window_visible(raw_hwnd: isize) -> bool {
        if raw_hwnd == 0 {
            return false;
        }
        unsafe { IsWindowVisible(HWND(raw_hwnd as *mut std::ffi::c_void)) }.as_bool()
    }

    fn is_window_topmost(raw_hwnd: isize) -> bool {
        if raw_hwnd == 0 {
            return false;
        }
        let ex_style = unsafe {
            GetWindowLongPtrW(HWND(raw_hwnd as *mut std::ffi::c_void), GWL_EXSTYLE)
        };
        (ex_style & WS_EX_TOPMOST.0 as isize) != 0
    }

    fn point_in_hwnd_rect(raw_hwnd: isize, pt: POINT) -> bool {
        if raw_hwnd == 0 {
            return false;
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(HWND(raw_hwnd as *mut std::ffi::c_void), &mut rect) }.is_err() {
            return false;
        }
        pt.x >= rect.left && pt.x < rect.right && pt.y >= rect.top && pt.y < rect.bottom
    }

    fn send_action(action: PopoverAction) {
        if let Ok(guard) = shared().action_tx.try_lock() {
            if let Some(ref tx) = *guard {
                let _ = tx.try_send(action);
            }
        }
    }

    pub fn capture_external_foreground_root_hwnd() -> Option<isize> {
        let fg = unsafe { GetForegroundWindow() };
        if fg.0.is_null() {
            return None;
        }
        let root_raw = root_hwnd(fg);
        if root_raw == 0 {
            return None;
        }
        let root = HWND(root_raw as *mut std::ffi::c_void);
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(root, Some(&mut pid)) };
        if pid == 0 || pid == std::process::id() {
            return None;
        }
        let class_name = window_class_name(root);
        if is_desktop_class(&class_name) || is_taskbar_class(&class_name) {
            return None;
        }
        Some(root_raw)
    }

    unsafe extern "system" fn mouse_hook_proc(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if code >= 0 && lparam.0 != 0 {
            let msg = wparam.0 as u32;
            if msg == WM_LBUTTONDOWN || msg == WM_RBUTTONDOWN {
                let st = shared();
                if st.active.load(Ordering::Acquire) {
                    let result_hwnd = st.result_hwnd.load(Ordering::Acquire);
                    if result_hwnd != 0 && is_window_visible(result_hwnd) {
                        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
                        let pt = info.pt;
                        let is_pinned = is_window_topmost(result_hwnd);
                        let in_rect = point_in_hwnd_rect(result_hwnd, pt);
                        let elapsed = now_ms().saturating_sub(
                            st.activated_at_ms.load(Ordering::Acquire),
                        );
                        let hit_hwnd = unsafe { WindowFromPoint(pt) };
                        let clicked_root = root_hwnd(hit_hwnd);
                        let clicked_owner = root_owner_hwnd(hit_hwnd);
                        let clicked_class = if clicked_root != 0 {
                            window_class_name(HWND(clicked_root as *mut std::ffi::c_void))
                        } else {
                            String::new()
                        };
                        let fg_root = root_hwnd(unsafe { GetForegroundWindow() });
                        let src_raw = st.source_hwnd.load(Ordering::Acquire);
                        let src_opt = if src_raw != 0 { Some(src_raw) } else { None };

                        if decide_outside_click(
                            elapsed,
                            is_pinned,
                            in_rect,
                            clicked_root,
                            clicked_owner,
                            &clicked_class,
                            result_hwnd,
                            src_opt,
                            fg_root,
                        ) == OutsideClickDecision::Dismiss
                        {
                            // Disarm before posting so trailing foreground events do not race.
                            st.active.store(false, Ordering::Release);
                            send_action(PopoverAction::Dismiss);
                        }
                    }
                }
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    unsafe extern "system" fn win_event_proc(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        id_object: i32,
        _id_child: i32,
        _id_event_thread: u32,
        _dwms_event_time: u32,
    ) {
        if event != EVENT_SYSTEM_FOREGROUND || id_object != 0 || hwnd.0.is_null() {
            return;
        }
        let st = shared();
        if !st.active.load(Ordering::Acquire) {
            return;
        }
        let result_hwnd = st.result_hwnd.load(Ordering::Acquire);
        let src_raw = st.source_hwnd.load(Ordering::Acquire);
        let src_opt = if src_raw != 0 { Some(src_raw) } else { None };
        let fg_root = root_hwnd(hwnd);

        let is_vis = is_window_visible(result_hwnd);
        let is_pin = is_window_topmost(result_hwnd);
        if should_raise_for_foreground(true, is_vis, is_pin, fg_root, src_opt) {
            send_action(PopoverAction::RaiseAboveSource {
                result_hwnd,
                source_hwnd: src_raw,
            });
        }
    }

    fn handle_worker_action(action: PopoverAction) {
        let st = shared();
        match action {
            PopoverAction::Dismiss => {
                let cb = st.on_dismiss.lock().ok().and_then(|g| g.clone());
                if let Some(callback) = cb {
                    callback();
                }
            }
            PopoverAction::RaiseAboveSource {
                result_hwnd,
                source_hwnd,
            } => {
                // Allow the target window switch / Alt+Tab settle before restoring Z-order.
                std::thread::sleep(Duration::from_millis(15));
                if !st.active.load(Ordering::Acquire)
                    || st.result_hwnd.load(Ordering::Acquire) != result_hwnd
                    || st.source_hwnd.load(Ordering::Acquire) != source_hwnd
                    || !is_window_visible(result_hwnd)
                    || is_window_topmost(result_hwnd)
                {
                    return;
                }
                let fg_root = root_hwnd(unsafe { GetForegroundWindow() });
                if fg_root != source_hwnd {
                    return;
                }
                // Check again right before raising so a self-window focus isn't overwritten.
                let mut fg_pid = 0u32;
                unsafe {
                    GetWindowThreadProcessId(
                        HWND(fg_root as *mut std::ffi::c_void),
                        Some(&mut fg_pid),
                    )
                };
                if fg_pid == st.self_pid {
                    return;
                }
                unsafe {
                    let _ = SetWindowPos(
                        HWND(result_hwnd as *mut std::ffi::c_void),
                        Some(HWND_TOP),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
            }
        }
    }

    fn ensure_watcher_started() {
        let lock = CONTROLLER.get_or_init(|| Mutex::new(None));
        let Ok(mut slot) = lock.lock() else {
            return;
        };
        if slot.is_some() {
            return;
        }

        let (tx, rx) = mpsc::sync_channel::<PopoverAction>(8);
        if let Ok(mut tx_guard) = shared().action_tx.lock() {
            *tx_guard = Some(tx);
        }

        let Ok(worker_join) = std::thread::Builder::new()
            .name("luma-popover-worker".to_string())
            .spawn(move || {
                while let Ok(action) = rx.recv() {
                    handle_worker_action(action);
                }
            })
        else {
            if let Ok(mut tx_guard) = shared().action_tx.lock() {
                *tx_guard = None;
            }
            return;
        };

        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<u32, ()>>(1);
        let Ok(hook_join) = std::thread::Builder::new()
            .name("luma-popover-watcher".to_string())
            .spawn(move || {
                let tid = unsafe { GetCurrentThreadId() };
                let event_hook = unsafe {
                    SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        None,
                        Some(win_event_proc),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                };
                let mouse_hook =
                    unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), None, 0) };
                if event_hook.is_invalid() && mouse_hook.is_err() {
                    let _ = ready_tx.send(Err(()));
                    return;
                }
                let _ = ready_tx.send(Ok(tid));
                let mut msg = MSG::default();
                while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                if let Ok(mh) = mouse_hook {
                    let _ = unsafe { UnhookWindowsHookEx(mh) };
                }
                if !event_hook.is_invalid() {
                    let _ = unsafe { UnhookWinEvent(event_hook) };
                }
            })
        else {
            if let Ok(mut tx_guard) = shared().action_tx.lock() {
                *tx_guard = None;
            }
            let _ = worker_join.join();
            return;
        };

        match ready_rx.recv() {
            Ok(Ok(hook_tid)) => {
                *slot = Some(WatcherController {
                    hook_tid,
                    hook_join,
                    worker_join,
                });
            }
            _ => {
                if let Ok(mut tx_guard) = shared().action_tx.lock() {
                    *tx_guard = None;
                }
                let _ = hook_join.join();
                let _ = worker_join.join();
            }
        }
    }

    pub fn activate(result_hwnd: isize, source_hwnd: Option<isize>, on_dismiss: DismissCallback) {
        if result_hwnd == 0 {
            deactivate();
            return;
        }
        ensure_watcher_started();
        let st = shared();
        if let Ok(mut cb_guard) = st.on_dismiss.lock() {
            *cb_guard = Some(on_dismiss);
        }
        st.result_hwnd.store(result_hwnd, Ordering::Release);
        st.source_hwnd
            .store(source_hwnd.unwrap_or(0), Ordering::Release);
        st.activated_at_ms.store(now_ms(), Ordering::Release);
        st.active.store(true, Ordering::Release);
    }

    pub fn deactivate() {
        let st = shared();
        st.active.store(false, Ordering::Release);
        st.source_hwnd.store(0, Ordering::Release);
    }

    pub fn shutdown() {
        deactivate();
        if let Ok(mut cb_guard) = shared().on_dismiss.lock() {
            *cb_guard = None;
        }
        if let Ok(mut tx_guard) = shared().action_tx.lock() {
            *tx_guard = None;
        }
        if let Some(lock) = CONTROLLER.get() {
            if let Ok(mut slot) = lock.lock() {
                if let Some(ctrl) = slot.take() {
                    unsafe {
                        let _ = PostThreadMessageW(ctrl.hook_tid, WM_QUIT, WPARAM(0), LPARAM(0));
                    }
                    let _ = ctrl.hook_join.join();
                    let _ = ctrl.worker_join.join();
                }
            }
        }
    }
}

/// Record the root `HWND` of the current external foreground window (if it is an application window,
/// excluding Luma itself, the Windows desktop, and taskbar surfaces).
pub fn capture_external_foreground_root_hwnd() -> Option<isize> {
    #[cfg(windows)]
    {
        imp::capture_external_foreground_root_hwnd()
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos_bridge::capture_external_foreground_pid()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// Begin monitoring outside blank-space clicks and source-window foreground switches for `result_hwnd`.
pub fn activate(
    result_hwnd: isize,
    source_hwnd: Option<isize>,
    on_dismiss: Arc<dyn Fn() + Send + Sync>,
) {
    #[cfg(windows)]
    imp::activate(result_hwnd, source_hwnd, on_dismiss);
    #[cfg(target_os = "macos")]
    crate::macos_bridge::popover_activate(result_hwnd, source_hwnd, on_dismiss);
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = (result_hwnd, source_hwnd, on_dismiss);
}

/// Pause popover outside-click / foreground tracking without tearing down the background watcher thread.
pub fn deactivate() {
    #[cfg(windows)]
    imp::deactivate();
    #[cfg(target_os = "macos")]
    crate::macos_bridge::popover_deactivate();
}

/// Cleanly tear down popover hooks and background threads on application exit.
pub fn shutdown() {
    #[cfg(windows)]
    imp::shutdown();
    #[cfg(target_os = "macos")]
    crate::macos_bridge::popover_shutdown();
}

#[cfg(test)]
mod tests {
    use super::{
        decide_outside_click, should_raise_for_foreground, CLICK_DISMISS_GRACE_MS,
        OutsideClickDecision,
    };

    const RES: isize = 0x1000;
    const SRC: isize = 0x2000;
    const APP_B: isize = 0x3000;
    const DESKTOP: isize = 0x4000;
    const TASKBAR: isize = 0x5000;
    const DROPDOWN: isize = 0x6000;

    #[test]
    fn keeps_popup_inside_rect_owned_or_system_chrome() {
        // Click inside the popup itself (buttons, text, drag bar).
        assert_eq!(
            decide_outside_click(500, false, true, RES, 0, "Chrome_WidgetWin_1", RES, Some(SRC), RES),
            OutsideClickDecision::Keep
        );
        // Owned native popup (e.g., dropdown owned by result).
        assert_eq!(
            decide_outside_click(500, false, false, DROPDOWN, RES, "#32768", RES, Some(SRC), RES),
            OutsideClickDecision::Keep
        );
        // Unowned browser select/IME popup while source or result has focus: never dismiss.
        assert_eq!(
            decide_outside_click(500, false, false, DROPDOWN, 0, "Chrome_WidgetWin_2", RES, Some(SRC), RES),
            OutsideClickDecision::Keep
        );
        // Click during the 250ms activation grace window.
        assert_eq!(
            decide_outside_click(
                CLICK_DISMISS_GRACE_MS - 1,
                false,
                false,
                SRC,
                0,
                "Chrome_WidgetWin_1",
                RES,
                Some(SRC),
                RES
            ),
            OutsideClickDecision::Keep
        );
        // Manual pin protects against all outside clicks.
        assert_eq!(
            decide_outside_click(500, true, false, SRC, 0, "Chrome_WidgetWin_1", RES, Some(SRC), RES),
            OutsideClickDecision::Keep
        );
        // Taskbar clicks switch windows without discarding translation.
        assert_eq!(
            decide_outside_click(500, false, false, TASKBAR, 0, "Shell_TrayWnd", RES, Some(SRC), RES),
            OutsideClickDecision::Keep
        );
        assert_eq!(
            decide_outside_click(
                500,
                false,
                false,
                TASKBAR,
                0,
                "Shell_SecondaryTrayWnd",
                RES,
                Some(SRC),
                SRC
            ),
            OutsideClickDecision::Keep
        );
    }

    #[test]
    fn switching_apps_keeps_while_clicking_source_or_desktop_dismisses() {
        // Clicking into another app keeps the popup alive behind App B.
        assert_eq!(
            decide_outside_click(500, false, false, APP_B, 0, "ApplicationFrameWindow", RES, Some(SRC), RES),
            OutsideClickDecision::Keep
        );
        // Clicking source blank area while result is active dismisses.
        assert_eq!(
            decide_outside_click(500, false, false, SRC, 0, "Chrome_WidgetWin_1", RES, Some(SRC), RES),
            OutsideClickDecision::Dismiss
        );
        // Clicking source blank area while source is already foreground dismisses too.
        assert_eq!(
            decide_outside_click(500, false, false, SRC, 0, "Chrome_WidgetWin_1", RES, Some(SRC), SRC),
            OutsideClickDecision::Dismiss
        );
        // Switching BACK to source from App B must keep the popup so user can still see it!
        assert_eq!(
            decide_outside_click(500, false, false, SRC, 0, "Chrome_WidgetWin_1", RES, Some(SRC), APP_B),
            OutsideClickDecision::Keep
        );
        // Clicking desktop wallpaper while in source/result layer dismisses.
        assert_eq!(
            decide_outside_click(500, false, false, DESKTOP, 0, "WorkerW", RES, Some(SRC), RES),
            OutsideClickDecision::Dismiss
        );
        assert_eq!(
            decide_outside_click(500, false, false, DESKTOP, 0, "Progman", RES, None, RES),
            OutsideClickDecision::Dismiss
        );
        // Clicking desktop wallpaper while App B was active keeps popup intact.
        assert_eq!(
            decide_outside_click(500, false, false, DESKTOP, 0, "WorkerW", RES, Some(SRC), APP_B),
            OutsideClickDecision::Keep
        );
    }

    #[test]
    fn raises_only_when_unpinned_active_visible_and_source_gains_foreground() {
        assert!(should_raise_for_foreground(true, true, false, SRC, Some(SRC)));
        assert!(!should_raise_for_foreground(true, true, false, APP_B, Some(SRC)));
        assert!(!should_raise_for_foreground(true, true, true, SRC, Some(SRC)));
        assert!(!should_raise_for_foreground(false, true, false, SRC, Some(SRC)));
        assert!(!should_raise_for_foreground(true, false, false, SRC, Some(SRC)));
        assert!(!should_raise_for_foreground(true, true, false, SRC, None));
    }
}

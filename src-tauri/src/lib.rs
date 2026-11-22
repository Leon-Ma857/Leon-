mod capture;
mod config;
mod local_ocr;
#[cfg(target_os = "macos")]
mod macos_bridge;
mod qr;
mod result_popover;
mod scroll_capture;
mod selection_capture;
mod translate;
mod updates;
mod win_ocr;

use capture::{
    capture_active_monitor, compute_popup_position, crop_normalized, cropped_region_from_rgba,
    decode_crop_from_base64, decode_png_to_rgba, encode_vision_optimized_base64, stitch_regions,
    CapturedScreen, CroppedRegion, OverlayPayload,
};
use config::{
    build_api_client, detect_system_proxy_info, load_config, load_config_from_dir, load_history,
    load_history_from_dir, normalize_ocr_engine, normalize_ocr_lang, normalize_proxy_url,
    normalize_theme, pre_setup_config_dir, proxy_has_credentials, sanitize_ask_quick_actions,
    save_config, save_history, AppConfig, AskQuickAction, SystemProxyInfo, MAX_HISTORY_ITEMS,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, State, Theme,
    WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use translate::{
    ask_ai_about_image, ask_ai_about_images, ask_ai_about_text, ocr_image_only, ocr_images_batch,
    test_connection, translate_image_full, translate_images_batch, translate_text_only,
    AiChatMessage, BatchStreamSink, BatchTextItem, StreamPreview, StreamSink, TranslationResult,
};
use win_ocr::{force_window_foreground, set_windows_autostart};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ResultViewState {
    Idle,
    Loading {
        job_id: u64,
        mode: String,
        engine: String,
        image_data_url: String,
        target_lang: String,
        pixel_width: u32,
        pixel_height: u32,
    },
    Done {
        result: TranslationResult,
        has_source_image: bool,
    },
    Pin {
        id: u64,
        image_data_url: String,
        pixel_width: u32,
        pixel_height: u32,
        scale_factor: f32,
    },
    Ask {
        job_id: u64,
        image_data_url: String,
        pixel_width: u32,
        pixel_height: u32,
        messages: Vec<AiChatMessage>,
        loading: bool,
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_text_preview: Option<String>,
    },
    Error {
        job_id: u64,
        mode: String,
        image_data_url: String,
        message: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionInput {
    pub rx: f64,
    pub ry: f64,
    pub rw: f64,
    pub rh: f64,
    pub annotated_base64: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayThumb {
    pub index: usize,
    pub label: String,
    pub image_data_url: String,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum OverlayAnalysisState {
    Idle,
    Ask {
        job_id: u64,
        thumbs: Vec<OverlayThumb>,
        messages: Vec<AiChatMessage>,
        loading: bool,
        error: Option<String>,
    },
    Translate {
        job_id: u64,
        thumbs: Vec<OverlayThumb>,
        engine: String,
        target_lang: String,
        loading: bool,
        items: Vec<BatchTextItem>,
        elapsed_ms: u64,
        model: String,
        error: Option<String>,
    },
    Ocr {
        job_id: u64,
        thumbs: Vec<OverlayThumb>,
        loading: bool,
        items: Vec<BatchTextItem>,
        elapsed_ms: u64,
        model: String,
        error: Option<String>,
    },
}

#[derive(Clone)]
pub struct StagedImage {
    pub id: u64,
    pub title: String,
    pub crop: CroppedRegion,
    pub thumb_data_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackItemDto {
    pub id: u64,
    pub title: String,
    pub thumb_data_url: String,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackItemImageDto {
    pub id: u64,
    pub title: String,
    pub image_data_url: String,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub scale_factor: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackViewState {
    pub items: Vec<StackItemDto>,
    pub max_stack_size: usize,
    pub clear_after_output: bool,
    pub ask_job_id: u64,
    pub ask_messages: Vec<AiChatMessage>,
    pub ask_loading: bool,
    pub ask_error: Option<String>,
    pub batch_job_id: u64,
    pub batch_mode: String,
    pub batch_loading: bool,
    pub batch_error: Option<String>,
    pub batch_items: Vec<BatchTextItem>,
    pub batch_elapsed_ms: u64,
    pub target_lang: String,
    pub engine: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub landing_capture_id: Option<u64>,
}

#[derive(Clone)]
pub struct PinItem {
    pub id: u64,
    pub crop: CroppedRegion,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinItemDto {
    pub id: u64,
    pub image_data_url: String,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub scale_factor: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutStatus {
    pub active: bool,
    pub shortcut: String,
    pub error: Option<String>,
    pub clipboard_shortcut: String,
    pub clipboard_active: bool,
    pub clipboard_error: Option<String>,
    pub selection_shortcut: String,
    pub selection_active: bool,
    pub selection_error: Option<String>,
}

pub struct AppState {
    pub config: AppConfig,
    pub client: Client,
    pub last_capture: Option<Arc<CapturedScreen>>,
    pub last_capture_source_hwnd: Option<isize>,
    pub last_monitor_rect: Option<(i32, i32, u32, u32, f32)>,
    pub last_crop: Option<CroppedRegion>,
    pub last_result: Option<TranslationResult>,
    pub last_input_text: Option<String>,
    pub ask_messages: Vec<AiChatMessage>,
    pub current_job_id: u64,
    pub current_capture_id: u64,
    pub overlay_presented: bool,
    pub latest_result_state: ResultViewState,
    pub history: Vec<TranslationResult>,
    pub next_history_id: u64,
    pub is_capturing: bool,
    pub registered_shortcut: String,
    pub shortcut_error: Option<String>,
    pub registered_clipboard_shortcut: String,
    pub clipboard_shortcut_id: Option<u32>,
    pub clipboard_shortcut_error: Option<String>,
    pub registered_selection_shortcut: String,
    pub selection_shortcut_id: Option<u32>,
    pub selection_shortcut_error: Option<String>,
    pub result_anchor_below: bool,
    pub result_anchor_y: Option<i32>,
    // In-place overlay side analysis
    pub overlay_crops: Vec<(String, CroppedRegion)>,
    pub overlay_ask_messages: Vec<AiChatMessage>,
    pub overlay_job_id: u64,
    pub overlay_analysis_state: OverlayAnalysisState,
    // Apple-style floating stack
    pub staged_stack: Vec<StagedImage>,
    pub next_stack_id: u64,
    stack_extent: (f64, f64),
    stack_frame_waiter: Option<(u64, tokio::sync::oneshot::Sender<()>)>,
    pub stack_ask_messages: Vec<AiChatMessage>,
    pub stack_ask_loading: bool,
    pub stack_ask_error: Option<String>,
    pub stack_batch_mode: String,
    pub stack_batch_loading: bool,
    pub stack_batch_error: Option<String>,
    pub stack_batch_items: Vec<BatchTextItem>,
    pub stack_batch_elapsed_ms: u64,
    pub stack_ask_job_id: u64,
    pub stack_batch_job_id: u64,
    // Multi-pin windows
    pub pins: HashMap<u64, PinItem>,
    pub next_pin_id: u64,
}

fn push_history_locked(
    guard: &mut AppState,
    mut res: TranslationResult,
) -> (TranslationResult, Vec<TranslationResult>) {
    guard.next_history_id += 1;
    res.id = guard.next_history_id;
    guard.history.insert(0, res.clone());
    if guard.history.len() > MAX_HISTORY_ITEMS {
        guard.history.truncate(MAX_HISTORY_ITEMS);
    }
    (res, guard.history.clone())
}

static HISTORY_WRITE_LOCK: Mutex<()> = Mutex::new(());

fn persist_history_and_notify(app: &AppHandle) -> Result<(), String> {
    let _writer = HISTORY_WRITE_LOCK.lock().map_err(|_| "历史记录写入锁异常".to_string())?;
    let snapshot = app.state::<Mutex<AppState>>().lock()
        .map_err(|_| "状态锁异常".to_string())?.history.clone();
    let result = save_history(app, &snapshot);
    let app_ui = app.clone();
    post_to_ui(app, move || {
        if let Some(main_win) = app_ui.get_webview_window("main") {
            let _ = main_win.emit("history:updated", ());
        }
    });
    result
}

fn format_history_markdown(items: &[TranslationResult]) -> String {
    let mut out = String::from("# Luma 翻译与识字历史\n\n");
    out.push_str(&format!("共 {} 条记录\n\n", items.len()));
    for (idx, item) in items.iter().enumerate() {
        let kind = if item.mode == "ocr" { "识字 OCR" } else { "翻译" };
        out.push_str(&format!(
            "## {}. [{}] {} → {} ({} · {}ms)\n\n",
            idx + 1,
            kind,
            item.source_lang,
            item.target_lang,
            item.model,
            item.elapsed_ms
        ));
        if !item.original_text.trim().is_empty() {
            out.push_str("**原文：**\n\n");
            out.push_str(&item.original_text);
            out.push_str("\n\n");
        }
        out.push_str("**结果：**\n\n");
        out.push_str(&item.translated_text);
        out.push_str("\n\n---\n\n");
    }
    out
}

fn copy_png_bytes_to_clipboard(png_bytes: &[u8]) -> Result<(), String> {
    let (width, height, bytes) = decode_png_to_rgba(png_bytes)?;
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("无法访问系统剪贴板: {e}"))?;
    clipboard
        .set_image(arboard::ImageData {
            width,
            height,
            bytes: Cow::Owned(bytes),
        })
        .map_err(|e| format!("写入剪贴板图像失败: {e}"))?;
    Ok(())
}

fn read_clipboard_cropped_region() -> Result<CroppedRegion, String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("无法访问系统剪贴板: {e}"))?;
    let img = clipboard
        .get_image()
        .map_err(|_| "剪贴板中当前没有可用的图片（请先复制截图或图像后再试）".to_string())?;
    cropped_region_from_rgba(
        img.width as u32,
        img.height as u32,
        img.bytes.into_owned(),
        1.0,
    )
}

fn resolve_crop_from_input(
    captured: Option<&CapturedScreen>,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
    annotated_base64: Option<String>,
) -> Result<CroppedRegion, String> {
    if let Some(b64) = annotated_base64 {
        let scale = captured.map(|c| c.scale_factor).unwrap_or(1.0);
        return decode_crop_from_base64(&b64, scale);
    }
    let cap = captured.ok_or_else(|| "当前没有可用的屏幕截图".to_string())?;
    crop_normalized(cap, rx, ry, rw, rh)
}

fn resolve_regions_from_inputs(
    captured: Option<&CapturedScreen>,
    inputs: &[RegionInput],
) -> Result<Vec<(String, CroppedRegion)>, String> {
    if inputs.is_empty() {
        return Err("请先框选需要处理的区域".to_string());
    }
    let mut out = Vec::with_capacity(inputs.len());
    for (idx, item) in inputs.iter().enumerate() {
        let crop = resolve_crop_from_input(
            captured,
            item.rx,
            item.ry,
            item.rw,
            item.rh,
            item.annotated_base64.clone(),
        )?;
        let label = item
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("选区 {}", idx + 1));
        out.push((label, crop));
    }
    Ok(out)
}

fn build_overlay_thumbs(crops: &[(String, CroppedRegion)]) -> Vec<OverlayThumb> {
    crops
        .iter()
        .enumerate()
        .map(|(idx, (label, crop))| OverlayThumb {
            index: idx,
            label: label.clone(),
            image_data_url: crop.make_thumbnail_data_url(),
            pixel_width: crop.pixel_width,
            pixel_height: crop.pixel_height,
        })
        .collect()
}

fn build_stack_view_state(guard: &AppState) -> StackViewState {
    StackViewState {
        items: guard
            .staged_stack
            .iter()
            .map(|item| StackItemDto {
                id: item.id,
                title: item.title.clone(),
                thumb_data_url: item.thumb_data_url.clone(),
                pixel_width: item.crop.pixel_width,
                pixel_height: item.crop.pixel_height,
            })
            .collect(),
        max_stack_size: guard.config.max_stack_size.clamp(2, 12),
        clear_after_output: guard.config.clear_stack_after_output,
        ask_job_id: guard.stack_ask_job_id,
        ask_messages: guard.stack_ask_messages.clone(),
        ask_loading: guard.stack_ask_loading,
        ask_error: guard.stack_ask_error.clone(),
        batch_job_id: guard.stack_batch_job_id,
        batch_mode: guard.stack_batch_mode.clone(),
        batch_loading: guard.stack_batch_loading,
        batch_error: guard.stack_batch_error.clone(),
        batch_items: guard.stack_batch_items.clone(),
        batch_elapsed_ms: guard.stack_batch_elapsed_ms,
        target_lang: guard.config.target_lang.clone(),
        engine: guard.config.translate_engine.clone(),
        landing_capture_id: None,
    }
}

fn dispatch_window_event<T: Serialize>(window: &tauri::WebviewWindow, name: &str, payload: &T) {
    let event = serde_json::to_string(&(name, payload))
        .and_then(|json| serde_json::to_string(&json));
    match event {
        Ok(json) => {
            let script = format!("(() => {{ const [name, detail] = JSON.parse({json}); window.dispatchEvent(new CustomEvent(name, {{ detail }})); }})()");
            if let Err(err) = window.eval(script) {
                eprintln!("[Luma] dispatch {name}: {err}");
            }
        }
        Err(err) => eprintln!("[Luma] serialize {name}: {err}"),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamPreviewEvent {
    job_id: u64,
    kind: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_index: Option<usize>,
}

const STREAM_EMIT_INTERVAL: Duration = Duration::from_millis(45);

fn emit_stream_preview(app: &AppHandle, window_label: &'static str, event_name: &'static str, payload: StreamPreviewEvent) {
    let app_ui = app.clone();
    post_to_ui(app, move || {
        if let Some(win) = app_ui.get_webview_window(window_label) {
            dispatch_window_event(&win, event_name, &payload);
        }
    });
}

fn make_throttled_stream_sink(
    app: AppHandle,
    window_label: &'static str,
    event_name: &'static str,
    job_id: u64,
    kind: &'static str,
    is_active: impl Fn(&AppState) -> bool + Send + Sync + 'static,
) -> StreamSink {
    let last_emit = Mutex::new(None::<Instant>);
    let is_active = Arc::new(is_active);
    Arc::new(move |preview: StreamPreview| {
        if preview.text.is_empty() && preview.original_text.as_deref().unwrap_or("").is_empty() {
            return;
        }
        {
            let Ok(mut last) = last_emit.lock() else {
                return;
            };
            let now = Instant::now();
            if last.is_some_and(|prev| now.duration_since(prev) < STREAM_EMIT_INTERVAL) {
                return;
            }
            *last = Some(now);
        }
        let still_active = app
            .state::<Mutex<AppState>>()
            .lock()
            .map(|guard| is_active(&guard))
            .unwrap_or(false);
        if !still_active {
            return;
        }
        emit_stream_preview(
            &app,
            window_label,
            event_name,
            StreamPreviewEvent {
                job_id,
                kind: kind.to_string(),
                text: preview.text,
                original_text: preview.original_text,
                item_index: None,
            },
        );
    })
}

fn make_throttled_batch_stream_sink(
    app: AppHandle,
    window_label: &'static str,
    event_name: &'static str,
    job_id: u64,
    kind: &'static str,
    is_active: impl Fn(&AppState) -> bool + Send + Sync + 'static,
) -> BatchStreamSink {
    let last_emits = Mutex::new(HashMap::<usize, Instant>::new());
    let is_active = Arc::new(is_active);
    Arc::new(move |index: usize, preview: StreamPreview| {
        if preview.text.is_empty() && preview.original_text.as_deref().unwrap_or("").is_empty() {
            return;
        }
        {
            let Ok(mut map) = last_emits.lock() else {
                return;
            };
            let now = Instant::now();
            if map
                .get(&index)
                .is_some_and(|prev| now.duration_since(*prev) < STREAM_EMIT_INTERVAL)
            {
                return;
            }
            map.insert(index, now);
        }
        let still_active = app
            .state::<Mutex<AppState>>()
            .lock()
            .map(|guard| is_active(&guard))
            .unwrap_or(false);
        if !still_active {
            return;
        }
        emit_stream_preview(
            &app,
            window_label,
            event_name,
            StreamPreviewEvent {
                job_id,
                kind: kind.to_string(),
                text: preview.text,
                original_text: preview.original_text,
                item_index: Some(index),
            },
        );
    })
}

fn emit_stack_update(app: &AppHandle, state: &StackViewState) {
    let state = state.clone();
    let app_ui = app.clone();
    post_to_ui(app, move || {
        if let Some(stack_win) = app_ui.get_webview_window("stack") {
            dispatch_window_event(&stack_win, "stack:updated", &state);
        }
    });
}

fn emit_overlay_analysis(app: &AppHandle, state: &OverlayAnalysisState) {
    let state = state.clone();
    let app_ui = app.clone();
    post_to_ui(app, move || {
        if let Some(overlay) = app_ui.get_webview_window("overlay") {
            dispatch_window_event(&overlay, "overlay:analysis", &state);
        }
    });
}

/// Window show/hide/resize/focus and `emit` all go through tao's UI thread and, on Windows,
/// `apply_diff` finishes the style change with a synchronous `SendMessageW` back to that same
/// queue. A Tauri command runs on that UI thread (the IPC custom-protocol handler), so doing any
/// of those calls inline deadlocks the message loop: the command never returns, the overlay stays
/// up, and Esc never arrives. Post the work and return; the queue runs it once this handler ends.
fn post_to_ui(app: &AppHandle, f: impl FnOnce() + Send + 'static) {
    let _ = app.run_on_main_thread(f);
}

/// Move + resize a window in one native call. `set_size` followed by `set_position`
/// produces an intermediate frame at the old corner with the new size, which reads
/// as a flicker/jump; a single SetWindowPos updates both atomically.
fn set_window_bounds(win: &tauri::WebviewWindow, x: i32, y: i32, w_phys: i32, h_phys: i32) -> tauri::Result<()> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};
        if let Ok(hwnd) = win.hwnd() {
            let ok = unsafe {
                SetWindowPos(
                    HWND(hwnd.0 as *mut std::ffi::c_void),
                    None,
                    x,
                    y,
                    w_phys,
                    h_phys,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            }
            .is_ok();
            if ok {
                return Ok(());
            }
        }
    }
    win.set_size(PhysicalSize::new(w_phys.max(1) as u32, h_phys.max(1) as u32))?;
    win.set_position(PhysicalPosition::new(x, y))
}

fn show_result_window_foreground(
    app: &AppHandle,
    result_win: &tauri::WebviewWindow,
    source_hwnd: Option<isize>,
) {
    #[cfg(target_os = "macos")]
    if let Ok(ns_win) = result_win.ns_window() {
        macos_bridge::configure_floating_window(ns_win);
    }
    let _ = result_win.show();
    let _ = result_win.set_focus();
    #[cfg(windows)]
    if let Ok(hwnd) = result_win.hwnd() {
        let raw_hwnd = hwnd.0 as isize;
        force_window_foreground(raw_hwnd);
        let app_dismiss = app.clone();
        result_popover::activate(
            raw_hwnd,
            source_hwnd,
            Arc::new(move || hide_result_window_internal(&app_dismiss)),
        );
    }
    #[cfg(target_os = "macos")]
    if let Ok(ns_win) = result_win.ns_window() {
        let raw_ns = ns_win as isize;
        force_window_foreground(raw_ns);
        let app_dismiss = app.clone();
        result_popover::activate(
            raw_ns,
            source_hwnd,
            Arc::new(move || hide_result_window_internal(&app_dismiss)),
        );
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (app, source_hwnd);
    }
}

fn hide_result_window_internal(app: &AppHandle) {
    result_popover::deactivate();
    let state = app.state::<Mutex<AppState>>();
    if let Ok(mut guard) = state.lock() {
        guard.current_job_id += 1;
        guard.last_crop = None;
        guard.last_result = None;
        guard.last_input_text = None;
        guard.ask_messages.clear();
        guard.latest_result_state = ResultViewState::Idle;
    }
    emit_result_update(app, &ResultViewState::Idle);
    let app_ui = app.clone();
    post_to_ui(app, move || {
        if let Some(result_win) = app_ui.get_webview_window("result") {
            let _ = result_win.set_always_on_top(false);
            let _ = result_win.hide();
        }
    });
}

fn report_capture_error(app: &AppHandle, err: String) {
    let app_ui = app.clone();
    post_to_ui(app, move || {
        if let Some(main_win) = app_ui.get_webview_window("main") {
            show_main_window(&app_ui);
            let _ = main_win.emit("capture:error", &err);
        }
    });
}

fn get_anchor_monitor_rect(app: &AppHandle, fallback_rect: Option<(i32, i32, u32, u32, f32)>) -> (i32, i32, u32, u32, f32) {
    if let Some(r) = fallback_rect {
        return r;
    }
    if let Ok(Some(mon)) = app.primary_monitor() {
        let pos = mon.position();
        let size = mon.size();
        return (pos.x, pos.y, size.width, size.height, mon.scale_factor() as f32);
    }
    (0, 0, 1920, 1080, 1.0)
}

// Win32 clips both painting and hit testing without reallocating the WebView2 surface.
#[cfg(windows)]
fn set_stack_region(win: &tauri::WebviewWindow, extent: (f64, f64)) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, SetWindowRgn};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let hwnd = HWND(win.hwnd().map_err(|err| err.to_string())?.0 as _);
    let scale = win.scale_factor().map_err(|err| err.to_string())?;
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.map_err(|err| err.to_string())?;
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let left = (width - (extent.0 * scale).round() as i32).max(0);
    let top = (height - (extent.1 * scale).round() as i32).max(0);
    let region = unsafe { CreateRectRgn(left, top, width, height) };
    if region.is_invalid() {
        return Err("无法创建叠堆窗口区域".into());
    }
    // The system takes ownership of the region on success.
    if unsafe { SetWindowRgn(hwnd, Some(region), true) } == 0 {
        let _ = unsafe { DeleteObject(region.into()) };
        return Err("无法调整叠堆窗口区域".into());
    }
    Ok(())
}

#[cfg(windows)]
fn keep_stack_region_on_screen(win: &tauri::WebviewWindow, extent: (f64, f64)) -> tauri::Result<()> {
    let pos = win.outer_position()?;
    let size = win.outer_size()?;
    let scale = win.scale_factor()?;
    let right = pos.x + size.width as i32;
    let bottom = pos.y + size.height as i32;
    let Some(monitor) = win.monitor_from_point(f64::from(right - 20), f64::from(bottom - 20))?
        .or(win.current_monitor()?) else { return Ok(()); };
    let work = monitor.work_area();
    let max_right = work.position.x + work.size.width as i32;
    let max_bottom = work.position.y + work.size.height as i32;
    let min_right = (work.position.x + (extent.0 * scale).round() as i32).min(max_right);
    let min_bottom = (work.position.y + (extent.1 * scale).round() as i32).min(max_bottom);
    let x = pos.x + right.clamp(min_right, max_right) - right;
    let y = pos.y + bottom.clamp(min_bottom, max_bottom) - bottom;
    if x != pos.x || y != pos.y {
        win.set_position(PhysicalPosition::new(x, y))?;
    }
    Ok(())
}

fn position_stack_bottom_right(app: &AppHandle, stack: &tauri::WebviewWindow) {
    #[cfg(windows)]
    let (width_logical, height_logical) = (820.0, 560.0);
    #[cfg(not(windows))]
    let (width_logical, height_logical) = (236.0, 168.0);
    let last_rect = app
        .state::<Mutex<AppState>>()
        .lock()
        .ok()
        .and_then(|g| g.last_monitor_rect);
    let (mon_x, mon_y, mon_w, mon_h, scale) = get_anchor_monitor_rect(app, last_rect);
    let sf = f64::from(scale.max(1.0));
    let w_phys = (width_logical * sf).round() as i32;
    let h_phys = (height_logical * sf).round() as i32;
    let margin = (20.0 * sf).round() as i32;
    let bottom_margin = (56.0 * sf).round() as i32;
    // Place the compact visible region, not the invisible top-left of the Windows surface.
    let x = mon_x + mon_w as i32 - w_phys - margin;
    let y = mon_y + mon_h as i32 - h_phys - bottom_margin;

    let _ = set_window_bounds(stack, x, y, w_phys, h_phys);
    #[cfg(windows)]
    {
        let extent = (236.0, 168.0);
        if let Ok(mut state) = app.state::<Mutex<AppState>>().lock() {
            state.stack_extent = extent;
        }
        if let Err(err) = set_stack_region(stack, extent) {
            eprintln!("{err}");
        }
    }
}

fn show_stack_behind_capture(app: &AppHandle, stack: &tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
        };
        if let (Ok(stack_hwnd), Some(overlay_hwnd)) = (
            stack.hwnd(),
            app.get_webview_window("overlay").and_then(|win| win.hwnd().ok()),
        ) {
            // Keep the frozen capture over the receiving deck until it has painted.
            if unsafe {
                SetWindowPos(
                    HWND(stack_hwnd.0 as _), Some(HWND(overlay_hwnd.0 as _)),
                    0, 0, 0, 0, SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                )
            }.is_ok() {
                return;
            }
        }
    }
    #[cfg(not(windows))]
    let _ = app;
    let _ = stack.show();
}

fn restore_stack_if_nonempty(app: &AppHandle) {
    let should_show = app
        .state::<Mutex<AppState>>()
        .lock()
        .map(|g| !g.staged_stack.is_empty())
        .unwrap_or(false);
    if should_show {
        let app_ui = app.clone();
        post_to_ui(app, move || {
            if let Some(stack_win) = app_ui.get_webview_window("stack") {
                let _ = stack_win.show();
            }
        });
    }
}

#[cfg(windows)]
fn disable_capture_window_transitions(overlay: &tauri::WebviewWindow) -> Result<(), String> {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};

    let hwnd = overlay.hwnd().map_err(|err| err.to_string())?;
    // A DWM setting on the capture HWND only. This does not affect WebView animations.
    let disabled = 1i32; // Win32 BOOL TRUE
    unsafe {
        DwmSetWindowAttribute(
            windows::Win32::Foundation::HWND(hwnd.0 as _),
            DWMWA_TRANSITIONS_FORCEDISABLED,
            std::ptr::from_ref(&disabled).cast(),
            std::mem::size_of_val(&disabled) as u32,
        )
    }
    .map_err(|err| err.to_string())
}

fn close_overlay_window(app: &AppHandle) {
    let app_ui = app.clone();
    post_to_ui(app, move || {
        if let Some(overlay) = app_ui.get_webview_window("overlay") {
            let _ = overlay.hide();
            dispatch_window_event(&overlay, "capture:clear", &());
        }
    });
}

fn present_overlay_window(app: &AppHandle, capture_id: u64) {
    let app_ui = app.clone();
    post_to_ui(app, move || {
        let state = app_ui.state::<Mutex<AppState>>();
        if active_capture(&state, capture_id).is_err() {
            return;
        }
        if let Some(overlay) = app_ui.get_webview_window("overlay") {
            if overlay.show().is_err() {
                return;
            }
            if let Ok(mut guard) = state.lock() {
                guard.overlay_presented = true;
            }
            let _ = overlay.set_focus();
            #[cfg(windows)]
            if let Ok(hwnd) = overlay.hwnd() {
                force_window_foreground(hwnd.0 as isize);
            }
            #[cfg(target_os = "macos")]
            if let Ok(ns_win) = overlay.ns_window() {
                macos_bridge::configure_overlay_window(ns_win);
                force_window_foreground(ns_win as isize);
            }
        }
    });
}

fn create_pin_window(
    app: &AppHandle,
    crop: CroppedRegion,
    pos_override: Option<(i32, i32)>,
    cascade_index: usize,
) -> Result<u64, String> {
    let (pin_id, monitor_rect) = {
        let state = app.state::<Mutex<AppState>>();
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard.next_pin_id += 1;
        let id = guard.next_pin_id;
        guard.pins.insert(id, PinItem { id, crop: crop.clone() });
        (id, guard.last_monitor_rect)
    };

    let scale = f64::from(crop.scale_factor.max(1.0));
    let lw = (crop.pixel_width as f64 / scale).clamp(140.0, 920.0) + 12.0;
    let lh = (crop.pixel_height as f64 / scale).clamp(48.0, 720.0) + 12.0;

    let (mon_x, mon_y, mon_w, mon_h, mon_scale) = get_anchor_monitor_rect(app, monitor_rect);
    let offset = (cascade_index as f64 * 26.0 * f64::from(mon_scale.max(1.0))).round() as i32;
    let (px, py) = if let Some((x, y)) = pos_override {
        (x + offset, y + offset)
    } else {
        let sf = f64::from(mon_scale.max(1.0));
        let w_phys = (lw * sf).round() as i32;
        let h_phys = (lh * sf).round() as i32;
        let cx = mon_x + ((mon_w as i32 - w_phys) / 2).max(24) + offset;
        let cy = mon_y + ((mon_h as i32 - h_phys) / 2).max(24) + offset;
        (cx, cy)
    };

    let label = format!("pin-{pin_id}");
    let url = WebviewUrl::App(format!("index.html?view=pin&pinId={pin_id}").into());
    let app_ui = app.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    // Callers run off the UI thread. Report build errors before they close the source window.
    let dispatch = app.run_on_main_thread(move || {
        let built = WebviewWindowBuilder::new(&app_ui, &label, url)
            .title("Luma 贴图")
            .visible(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .resizable(true)
            .inner_size(lw, lh)
            .min_inner_size(140.0, 56.0)
            .build();
        let result = built.and_then(|win| {
            #[cfg(target_os = "macos")]
            if let Ok(ns_win) = win.ns_window() {
                macos_bridge::configure_floating_window(ns_win);
            }
            win.set_position(PhysicalPosition::new(px, py))?;
            win.show()?;
            Ok(())
        });
        let _ = tx.send(result);
    });
    let result = dispatch.map_err(|e| e.to_string()).and_then(|()| {
        rx.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())
    });
    if let Err(err) = result {
        if let Ok(mut guard) = app.state::<Mutex<AppState>>().lock() {
            guard.pins.remove(&pin_id);
        }
        return Err(format!("创建贴图失败: {err}"));
    }
    Ok(pin_id)
}

async fn save_png_with_dialog(png_bytes: Vec<u8>, prefix: &str) -> Result<bool, String> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let default_name = format!("{prefix}_{ts}.png");

    let file = rfd::AsyncFileDialog::new()
        .add_filter("PNG 图片", &["png"])
        .set_file_name(default_name)
        .save_file()
        .await;

    if let Some(handle) = file {
        handle
            .write(&png_bytes)
            .await
            .map_err(|e| format!("保存图片失败: {e}"))?;
        return Ok(true);
    }
    Ok(false)
}

fn clear_stack_state_locked(guard: &mut AppState) {
    guard.staged_stack.clear();
    reset_stack_analysis(guard);
}

fn reset_stack_analysis(guard: &mut AppState) {
    guard.stack_ask_messages.clear();
    guard.stack_ask_loading = false;
    guard.stack_ask_error = None;
    guard.stack_batch_items.clear();
    guard.stack_batch_loading = false;
    guard.stack_batch_error = None;
    guard.stack_batch_elapsed_ms = 0;
    guard.stack_ask_job_id += 1;
    guard.stack_batch_job_id += 1;
}

struct CaptureResetGuard<'a>(&'a AppHandle);

impl Drop for CaptureResetGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.0.state::<Mutex<AppState>>().lock() {
            guard.is_capturing = false;
        }
    }
}

fn wait_for_window_hide() {
    #[cfg(windows)]
    if unsafe { windows::Win32::Graphics::Dwm::DwmFlush() }.is_ok() {
        return;
    }
    std::thread::sleep(Duration::from_millis(40));
}

fn trigger_capture(app: &AppHandle, hide_main: bool) -> Result<(), String> {
    {
        let state = app.state::<Mutex<AppState>>();
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if guard.is_capturing || guard.last_capture.is_some() {
            return Ok(());
        }
        guard.is_capturing = true;
    }
    let _reset_guard = CaptureResetGuard(app);
    let capture_source_hwnd = result_popover::capture_external_foreground_root_hwnd();
    result_popover::deactivate();

    // Hide every window that would show up in the screenshot, and wait until the UI thread has
    // actually applied it. A fixed sleep raced the posted hide and still captured our own windows.
    let (hide_tx, hide_rx) = std::sync::mpsc::channel();
    let app_hide = app.clone();
    let hide_cancelled = Arc::new(AtomicBool::new(false));
    let cancelled = hide_cancelled.clone();
    post_to_ui(app, move || {
        if cancelled.load(Ordering::Acquire) {
            return;
        }
        let mut hidden = false;
        for label in ["overlay", "result", "stack", "main"] {
            if label == "main" && !hide_main {
                continue;
            }
            if let Some(window) = app_hide.get_webview_window(label) {
                if window.is_visible().unwrap_or(true) {
                    hidden |= window.hide().is_ok();
                }
            }
        }
        let _ = hide_tx.send(hidden);
    });
    let hidden = hide_rx.recv_timeout(Duration::from_millis(700)).map_err(|_| {
        hide_cancelled.store(true, Ordering::Release);
        restore_stack_if_nonempty(app);
        "截图窗口暂时无响应，请重试".to_string()
    })?;
    if hidden {
        wait_for_window_hide();
    }

    let cursor = app
        .cursor_position()
        .ok()
        .map(|p| (p.x.round() as i32, p.y.round() as i32));

    let captured = match capture_active_monitor(cursor) {
        Ok(c) => c,
        Err(err) => {
            restore_stack_if_nonempty(app);
            return Err(err);
        }
    };

    let (mon_x, mon_y, mon_w, mon_h, capture_id, payload) = {
        let state = app.state::<Mutex<AppState>>();
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard.current_capture_id += 1;
        guard.overlay_presented = false;
        let capture_id = guard.current_capture_id;
        let payload = OverlayPayload {
            capture_id,
            monitor_width: captured.monitor_width,
            monitor_height: captured.monitor_height,
            scale_factor: captured.scale_factor,
            auto_translate_on_release: guard.config.auto_translate_on_release,
            translate_engine: guard.config.translate_engine.clone(),
            target_lang: guard.config.target_lang.clone(),
            staged_count: guard.staged_stack.len(),
            max_stack_size: guard.config.max_stack_size.clamp(2, 12),
            latest_staged_thumb: guard
                .staged_stack
                .last()
                .map(|s| s.thumb_data_url.clone())
                .unwrap_or_default(),
        };
        let pos = (
            captured.monitor_x,
            captured.monitor_y,
            captured.monitor_width,
            captured.monitor_height,
        );
        guard.last_monitor_rect = Some((
            captured.monitor_x,
            captured.monitor_y,
            captured.monitor_width,
            captured.monitor_height,
            captured.scale_factor,
        ));
        guard.last_capture = Some(Arc::new(captured));
        guard.last_capture_source_hwnd = capture_source_hwnd;
        guard.overlay_crops.clear();
        guard.overlay_ask_messages.clear();
        guard.overlay_job_id += 1;
        guard.overlay_analysis_state = OverlayAnalysisState::Idle;
        (pos.0, pos.1, pos.2, pos.3, capture_id, payload)
    };

    let app_ui = app.clone();
    post_to_ui(app, move || {
        if let Some(overlay) = app_ui.get_webview_window("overlay") {
            let _ = set_window_bounds(&overlay, mon_x, mon_y, mon_w as i32, mon_h as i32);
            dispatch_window_event(&overlay, "capture:ready", &payload);
        }
    });

    // Recover if the WebView never decodes the image without showing a blank fullscreen window.
    let app_timeout = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        let app_ui = app_timeout.clone();
        post_to_ui(&app_timeout, move || {
            let state = app_ui.state::<Mutex<AppState>>();
            let timed_out = state.lock().is_ok_and(|guard| {
                guard.current_capture_id == capture_id
                    && guard.last_capture.is_some()
                    && !guard.overlay_presented
            });
            if timed_out {
                let _ = cancel_capture(app_ui.clone(), state);
                report_capture_error(&app_ui, "截图画面未能显示，请重试".into());
            }
        });
    });

    Ok(())
}

fn emit_result_update(app: &AppHandle, _: &ResultViewState) {
    let app_ui = app.clone();
    post_to_ui(app, move || {
        let latest = app_ui.state::<Mutex<AppState>>().lock().ok().map(|g| g.latest_result_state.clone());
        if let (Some(result_win), Some(latest)) = (app_ui.get_webview_window("result"), latest) {
            dispatch_window_event(&result_win, "result:update", &latest);
        }
    });
}

fn spawn_image_job(
    app: AppHandle,
    job_id: u64,
    mode: String,
    engine: String,
    target_lang: String,
    crop: CroppedRegion,
    config: AppConfig,
    client: Client,
) {
    tauri::async_runtime::spawn(async move {
        let thumb_url = crop.make_thumbnail_data_url();
        let stream_kind: &'static str = if mode == "ocr" { "ocr" } else { "translate" };
        let stream_sink = make_throttled_stream_sink(
            app.clone(),
            "result",
            "result:stream",
            job_id,
            stream_kind,
            move |guard| guard.current_job_id == job_id,
        );

        let outcome = if mode == "ocr" {
            ocr_image_only(
                &client,
                &config,
                job_id,
                &crop.raw_base64,
                &thumb_url,
                Some(&stream_sink),
            )
            .await
        } else {
            let image = if engine == "ai" {
                encode_vision_optimized_base64(&crop)
            } else {
                crop.raw_base64
            };
            translate_image_full(
                &client,
                &config,
                job_id,
                &image,
                &thumb_url,
                &target_lang,
                &engine,
                if engine == "ai" {
                    Some(&stream_sink)
                } else {
                    None
                },
            )
            .await
        };

        let (view_state, history_snapshot) = {
            let state = app.state::<Mutex<AppState>>();
            let Ok(mut guard) = state.lock() else {
                return;
            };
            if guard.current_job_id != job_id {
                return;
            }
            match outcome {
                Ok(res) => {
                    let (stored, snapshot) = push_history_locked(&mut guard, res);
                    guard.last_result = Some(stored.clone());
                    let vs = ResultViewState::Done { result: stored, has_source_image: guard.last_crop.is_some() };
                    guard.latest_result_state = vs.clone();
                    (vs, Some(snapshot))
                }
                Err(msg) => {
                    let vs = ResultViewState::Error {
                        job_id,
                        mode,
                        image_data_url: thumb_url,
                        message: msg,
                    };
                    guard.latest_result_state = vs.clone();
                    (vs, None)
                }
            }
        };

        emit_result_update(&app, &view_state);
        if history_snapshot.is_some() {
            if let Err(err) = persist_history_and_notify(&app) { eprintln!("[Luma] {err}"); }
        }
    });
}

fn spawn_text_retranslate_job(
    app: AppHandle,
    job_id: u64,
    original_text: String,
    source_lang: String,
    thumb_url: String,
    target_lang: String,
    engine: String,
    config: AppConfig,
    client: Client,
) {
    tauri::async_runtime::spawn(async move {
        let stream_sink = make_throttled_stream_sink(
            app.clone(),
            "result",
            "result:stream",
            job_id,
            "translate",
            move |guard| guard.current_job_id == job_id,
        );
        let outcome = translate_text_only(
            &client,
            &config,
            job_id,
            &original_text,
            &source_lang,
            &thumb_url,
            &target_lang,
            &engine,
            if engine == "ai" {
                Some(&stream_sink)
            } else {
                None
            },
        )
        .await;

        let (view_state, history_snapshot) = {
            let state = app.state::<Mutex<AppState>>();
            let Ok(mut guard) = state.lock() else {
                return;
            };
            if guard.current_job_id != job_id {
                return;
            }
            match outcome {
                Ok(res) => {
                    let (stored, snapshot) = push_history_locked(&mut guard, res);
                    guard.last_result = Some(stored.clone());
                    let vs = ResultViewState::Done { result: stored, has_source_image: guard.last_crop.is_some() };
                    guard.latest_result_state = vs.clone();
                    (vs, Some(snapshot))
                }
                Err(msg) => {
                    let vs = ResultViewState::Error {
                        job_id,
                        mode: "translate".to_string(),
                        image_data_url: thumb_url,
                        message: msg,
                    };
                    guard.latest_result_state = vs.clone();
                    (vs, None)
                }
            }
        };

        emit_result_update(&app, &view_state);
        if history_snapshot.is_some() {
            if let Err(err) = persist_history_and_notify(&app) { eprintln!("[Luma] {err}"); }
        }
    });
}

fn spawn_ask_ai_job(
    app: AppHandle,
    job_id: u64,
    crop: CroppedRegion,
    messages: Vec<AiChatMessage>,
    config: AppConfig,
    client: Client,
) {
    tauri::async_runtime::spawn(async move {
        let vision_b64 = encode_vision_optimized_base64(&crop);
        let thumb_url = crop.make_thumbnail_data_url();
        let stream_sink = make_throttled_stream_sink(
            app.clone(),
            "result",
            "result:stream",
            job_id,
            "ask",
            move |guard| guard.current_job_id == job_id,
        );
        let outcome =
            ask_ai_about_image(&client, &config, &vision_b64, &messages, Some(&stream_sink)).await;

        let view_state = {
            let state = app.state::<Mutex<AppState>>();
            let Ok(mut guard) = state.lock() else {
                return;
            };
            if guard.current_job_id != job_id {
                return;
            }
            match outcome {
                Ok(answer) => {
                    guard.ask_messages.push(AiChatMessage {
                        role: "assistant".to_string(),
                        content: answer,
                    });
                    let vs = ResultViewState::Ask {
                        job_id,
                        image_data_url: thumb_url,
                        pixel_width: crop.pixel_width,
                        pixel_height: crop.pixel_height,
                        messages: guard.ask_messages.clone(),
                        loading: false,
                        error: None,
                        source_text_preview: None,
                    };
                    guard.latest_result_state = vs.clone();
                    vs
                }
                Err(err_msg) => {
                    if guard
                        .ask_messages
                        .last()
                        .map(|m| m.role == "user")
                        .unwrap_or(false)
                    {
                        guard.ask_messages.pop();
                    }
                    let vs = ResultViewState::Ask {
                        job_id,
                        image_data_url: thumb_url,
                        pixel_width: crop.pixel_width,
                        pixel_height: crop.pixel_height,
                        messages: guard.ask_messages.clone(),
                        loading: false,
                        error: Some(err_msg),
                        source_text_preview: None,
                    };
                    guard.latest_result_state = vs.clone();
                    vs
                }
            }
        };

        emit_result_update(&app, &view_state);
    });
}

fn build_source_text_preview(text: &str) -> String {
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if count > 120 {
        let head: String = trimmed.chars().take(120).collect();
        format!("{head}…")
    } else {
        trimmed.to_string()
    }
}

fn spawn_ask_ai_text_job(
    app: AppHandle,
    job_id: u64,
    source_text: String,
    messages: Vec<AiChatMessage>,
    config: AppConfig,
    client: Client,
) {
    let preview = build_source_text_preview(&source_text);
    tauri::async_runtime::spawn(async move {
        let stream_sink = make_throttled_stream_sink(
            app.clone(),
            "result",
            "result:stream",
            job_id,
            "ask",
            move |guard| guard.current_job_id == job_id,
        );
        let outcome =
            ask_ai_about_text(&client, &config, &source_text, &messages, Some(&stream_sink)).await;

        let view_state = {
            let state = app.state::<Mutex<AppState>>();
            let Ok(mut guard) = state.lock() else {
                return;
            };
            if guard.current_job_id != job_id {
                return;
            }
            match outcome {
                Ok(answer) => {
                    guard.ask_messages.push(AiChatMessage {
                        role: "assistant".to_string(),
                        content: answer,
                    });
                    let vs = ResultViewState::Ask {
                        job_id,
                        image_data_url: String::new(),
                        pixel_width: 0,
                        pixel_height: 0,
                        messages: guard.ask_messages.clone(),
                        loading: false,
                        error: None,
                        source_text_preview: Some(preview.clone()),
                    };
                    guard.latest_result_state = vs.clone();
                    vs
                }
                Err(err_msg) => {
                    if guard
                        .ask_messages
                        .last()
                        .map(|m| m.role == "user")
                        .unwrap_or(false)
                    {
                        guard.ask_messages.pop();
                    }
                    let vs = ResultViewState::Ask {
                        job_id,
                        image_data_url: String::new(),
                        pixel_width: 0,
                        pixel_height: 0,
                        messages: guard.ask_messages.clone(),
                        loading: false,
                        error: Some(err_msg),
                        source_text_preview: Some(preview.clone()),
                    };
                    guard.latest_result_state = vs.clone();
                    vs
                }
            }
        };

        emit_result_update(&app, &view_state);
    });
}

#[derive(Debug, Clone, Default)]
struct ShortcutRegOutcome {
    clipboard_id: Option<u32>,
    clipboard_error: Option<String>,
    selection_id: Option<u32>,
    selection_error: Option<String>,
}

fn register_shortcuts(
    app: &AppHandle,
    capture_acc: &str,
    clipboard_acc: &str,
    selection_acc: &str,
) -> Result<ShortcutRegOutcome, String> {
    let app_ui = app.clone();
    let capture_acc = capture_acc.to_owned();
    let clipboard_acc = clipboard_acc.to_owned();
    let selection_acc = selection_acc.to_owned();
    let (tx, rx) = std::sync::mpsc::channel();
    // The plugin holds its shortcut mutex during unregister_all. Run the operation
    // on the event loop so a hotkey event cannot block that loop on the same mutex.
    app.run_on_main_thread(move || {
        let _ = tx.send(register_shortcuts_on_ui(
            &app_ui,
            &capture_acc,
            &clipboard_acc,
            &selection_acc,
        ));
    })
    .map_err(|e| format!("无法更新快捷键: {e}"))?;
    rx.recv().map_err(|e| format!("无法获取快捷键注册结果: {e}"))?
}

fn register_shortcuts_on_ui(
    app: &AppHandle,
    capture_acc: &str,
    clipboard_acc: &str,
    selection_acc: &str,
) -> Result<ShortcutRegOutcome, String> {
    let cap_trimmed = capture_acc.trim();
    let cap_sc = Shortcut::from_str(cap_trimmed)
        .map_err(|e| format!("截图快捷键格式无效 ({cap_trimmed}): {e}"))?;

    let clip_trimmed = clipboard_acc.trim();
    let clip_sc = if clip_trimmed.is_empty() {
        None
    } else {
        let sc = Shortcut::from_str(clip_trimmed)
            .map_err(|e| format!("剪贴板翻译快捷键格式无效 ({clip_trimmed}): {e}"))?;
        if sc.id() == cap_sc.id() {
            return Err("剪贴板翻译快捷键不能与截图快捷键相同".to_string());
        }
        Some(sc)
    };

    let sel_trimmed = selection_acc.trim();
    let smart_sel_key = selection_capture::parse_smart_selection_key(sel_trimmed);
    let sel_sc = if sel_trimmed.is_empty() || smart_sel_key.is_some() {
        None
    } else {
        let sc = Shortcut::from_str(sel_trimmed)
            .map_err(|e| format!("划词翻译快捷键格式无效 ({sel_trimmed}): {e}"))?;
        if sc.id() == cap_sc.id() {
            return Err("划词翻译快捷键不能与截图快捷键相同".to_string());
        }
        if let Some(ref c_sc) = clip_sc {
            if sc.id() == c_sc.id() {
                return Err("划词翻译快捷键不能与剪贴板翻译快捷键相同".to_string());
            }
        }
        Some(sc)
    };

    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    gs.register(cap_sc).map_err(|e| {
        format!("注册全局快捷键 {cap_trimmed} 失败（可能被其他软件或已运行的实例占用）: {e}")
    })?;

    let mut outcome = ShortcutRegOutcome::default();

    if let Some(sc) = clip_sc {
        let clip_id = sc.id();
        if let Err(e) = gs.register(sc) {
            outcome.clipboard_error = Some(format!(
                "注册剪贴板翻译快捷键 {clip_trimmed} 失败（可能被其他软件占用）: {e}"
            ));
        } else {
            outcome.clipboard_id = Some(clip_id);
        }
    }

    if let Some(smart_key) = smart_sel_key {
        let cb_app = app.clone();
        let cb: Arc<dyn Fn(selection_capture::SelectionAnchorRect) + Send + Sync> =
            Arc::new(move |rect| {
                let app_spawn = cb_app.clone();
                std::thread::spawn(move || {
                    if let Err(err) = present_selected_text_in_result_with_rect(&app_spawn, Some(rect))
                    {
                        eprintln!("[Luma] 智能单键划词翻译失败: {err}");
                        report_capture_error(&app_spawn, err);
                    }
                });
            });
        if let Err(err) = selection_capture::configure_smart_selection_hook(Some(smart_key), Some(cb))
        {
            outcome.selection_error = Some(err);
        }
    } else {
        selection_capture::stop_smart_selection_hook();
        if let Some(sc) = sel_sc {
            let sel_id = sc.id();
            if let Err(e) = gs.register(sc) {
                outcome.selection_error = Some(format!(
                    "注册划词翻译快捷键 {sel_trimmed} 失败（可能被其他软件占用）: {e}"
                ));
            } else {
                outcome.selection_id = Some(sel_id);
            }
        }
    }

    Ok(outcome)
}

fn apply_native_window_theme(app: &AppHandle, theme: &str) {
    let target = match normalize_theme(theme).as_str() {
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        _ => None,
    };
    if let Some(main_win) = app.get_webview_window("main") {
        let _ = main_win.set_theme(target);
    }
}

fn build_shortcut_status(guard: &AppState) -> ShortcutStatus {
    ShortcutStatus {
        active: guard.shortcut_error.is_none(),
        shortcut: guard.registered_shortcut.clone(),
        error: guard.shortcut_error.clone(),
        clipboard_shortcut: guard.registered_clipboard_shortcut.clone(),
        clipboard_active: !guard.registered_clipboard_shortcut.is_empty()
            && guard.clipboard_shortcut_error.is_none(),
        clipboard_error: guard.clipboard_shortcut_error.clone(),
        selection_shortcut: guard.registered_selection_shortcut.clone(),
        selection_active: !guard.registered_selection_shortcut.is_empty()
            && guard.selection_shortcut_error.is_none(),
        selection_error: guard.selection_shortcut_error.clone(),
    }
}

#[tauri::command]
fn get_shortcut_status(state: State<'_, Mutex<AppState>>) -> Result<ShortcutStatus, String> {
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    Ok(build_shortcut_status(&guard))
}

#[tauri::command]
fn get_system_proxy_info() -> SystemProxyInfo {
    detect_system_proxy_info()
}

#[tauri::command]
fn get_theme(state: State<'_, Mutex<AppState>>) -> Result<String, String> {
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    Ok(guard.config.theme.clone())
}

#[tauri::command]
fn get_ask_quick_actions(state: State<'_, Mutex<AppState>>) -> Result<Vec<AskQuickAction>, String> {
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    Ok(guard.config.ask_quick_actions.clone())
}

#[tauri::command(async)]
fn save_ask_quick_actions(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    actions: Vec<AskQuickAction>,
) -> Result<Vec<AskQuickAction>, String> {
    let sanitized = sanitize_ask_quick_actions(actions);
    let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    let mut config = guard.config.clone();
    config.ask_quick_actions = sanitized.clone();
    save_config(&app, &config)?;
    guard.config = config;
    drop(guard);

    let emit_payload = sanitized.clone();
    let app_ui = app.clone();
    post_to_ui(&app, move || {
        let _ = app_ui.emit("ask-actions:updated", &emit_payload);
    });
    Ok(sanitized)
}

#[tauri::command]
fn get_stack_prompts(state: State<'_, Mutex<AppState>>) -> Result<Vec<String>, String> {
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    Ok(guard.config.custom_prompt_presets.clone())
}

#[tauri::command(async)]
fn save_stack_prompts(app: AppHandle, state: State<'_, Mutex<AppState>>, prompts: Vec<String>) -> Result<(), String> {
    let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    let mut config = guard.config.clone();
    config.custom_prompt_presets = prompts;
    save_config(&app, &config)?;
    guard.config = config;
    Ok(())
}

#[tauri::command]
fn get_config(state: State<'_, Mutex<AppState>>) -> Result<AppConfig, String> {
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    Ok(guard.config.clone())
}

fn require_key_for_origin(previous: &AppConfig, next: &AppConfig) -> Result<(), String> {
    if !next.api_key.trim().is_empty() && previous.api_key.trim() == next.api_key.trim()
        && !translate::same_ai_origin(&previous.api_base_url, &next.api_base_url)
    {
        return Err("AI 服务地址已变更，请为新服务填写 API Key。".into());
    }
    Ok(())
}

fn apply_and_persist_config(
    app: &AppHandle,
    state: &Mutex<AppState>,
    mut new_config: AppConfig,
) -> Result<AppConfig, String> {
    new_config.max_stack_size = new_config.max_stack_size.clamp(2, 12);
    new_config.shortcut = new_config.shortcut.trim().to_string();
    new_config.clipboard_shortcut = new_config.clipboard_shortcut.trim().to_string();
    new_config.selection_shortcut = new_config.selection_shortcut.trim().to_string();
    new_config.theme = normalize_theme(&new_config.theme);
    new_config.ask_quick_actions = sanitize_ask_quick_actions(new_config.ask_quick_actions);
    new_config.ocr_engine = normalize_ocr_engine(&new_config.ocr_engine);
    new_config.ocr_lang = normalize_ocr_lang(&new_config.ocr_lang);
    new_config.ocr_download_url = new_config.ocr_download_url.trim().to_string();
    new_config.proxy_url = normalize_proxy_url(&new_config.proxy_url)?.unwrap_or_default();
    new_config.update_feed_url = new_config.update_feed_url.trim().to_string();
    if !new_config.update_feed_url.is_empty() {
        updates::parse_feed(&new_config.update_feed_url)?;
    }
    let next_client = build_api_client(&new_config.proxy_url)?;

    let (
        old_shortcut,
        old_clip_shortcut,
        old_sel_shortcut,
        had_error,
        had_clip_error,
        had_sel_error,
        old_ocr_lang,
    ) = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        require_key_for_origin(&guard.config, &new_config)?;
        (
            guard.registered_shortcut.clone(),
            guard.registered_clipboard_shortcut.clone(),
            guard.registered_selection_shortcut.clone(),
            guard.shortcut_error.is_some(),
            guard.clipboard_shortcut_error.is_some(),
            guard.selection_shortcut_error.is_some(),
            guard.config.ocr_lang.clone(),
        )
    };

    let need_rebind = new_config.shortcut != old_shortcut
        || new_config.clipboard_shortcut != old_clip_shortcut
        || new_config.selection_shortcut != old_sel_shortcut
        || had_error
        || had_clip_error
        || had_sel_error;

    let reg_outcome = if need_rebind {
        match register_shortcuts(
            app,
            &new_config.shortcut,
            &new_config.clipboard_shortcut,
            &new_config.selection_shortcut,
        ) {
            Ok(outcome) => {
                if let Some(ref clip_err) = outcome.clipboard_error {
                    if !new_config.clipboard_shortcut.is_empty() {
                        let _ = register_shortcuts(
                            app,
                            &old_shortcut,
                            &old_clip_shortcut,
                            &old_sel_shortcut,
                        );
                        return Err(clip_err.clone());
                    }
                }
                if let Some(ref sel_err) = outcome.selection_error {
                    if !new_config.selection_shortcut.is_empty() {
                        let _ = register_shortcuts(
                            app,
                            &old_shortcut,
                            &old_clip_shortcut,
                            &old_sel_shortcut,
                        );
                        return Err(sel_err.clone());
                    }
                }
                outcome
            }
            Err(err) => {
                let rolled_back = !had_error
                    && !old_shortcut.is_empty()
                    && register_shortcuts(
                        app,
                        &old_shortcut,
                        &old_clip_shortcut,
                        &old_sel_shortcut,
                    )
                    .is_ok();

                let shortcut_status = if let Ok(mut guard) = state.lock() {
                    if rolled_back {
                        guard.shortcut_error = None;
                    } else {
                        guard.shortcut_error = Some(err.clone());
                    }
                    build_shortcut_status(&guard)
                } else {
                    ShortcutStatus {
                        active: rolled_back,
                        shortcut: old_shortcut,
                        error: if rolled_back { None } else { Some(err.clone()) },
                        clipboard_shortcut: old_clip_shortcut,
                        clipboard_active: false,
                        clipboard_error: None,
                        selection_shortcut: old_sel_shortcut,
                        selection_active: false,
                        selection_error: None,
                    }
                };

                let app_ui = app.clone();
                post_to_ui(app, move || {
                    if let Some(main_win) = app_ui.get_webview_window("main") {
                        let _ = main_win.emit("shortcut:status", &shortcut_status);
                    }
                });
                return Err(err);
            }
        }
    } else {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        ShortcutRegOutcome {
            clipboard_id: guard.clipboard_shortcut_id,
            clipboard_error: guard.clipboard_shortcut_error.clone(),
            selection_id: guard.selection_shortcut_id,
            selection_error: guard.selection_shortcut_error.clone(),
        }
    };

    save_config(app, &new_config)?;
    set_windows_autostart(new_config.launch_at_startup)?;
    if normalize_ocr_lang(&old_ocr_lang) != new_config.ocr_lang {
        local_ocr::shutdown_worker();
    }

    let (stack_state, shortcut_status) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard.config = new_config.clone();
        guard.client = next_client;
        guard.registered_shortcut = new_config.shortcut.clone();
        guard.shortcut_error = None;
        guard.registered_clipboard_shortcut = new_config.clipboard_shortcut.clone();
        guard.clipboard_shortcut_id = reg_outcome.clipboard_id;
        guard.clipboard_shortcut_error = reg_outcome.clipboard_error;
        guard.registered_selection_shortcut = new_config.selection_shortcut.clone();
        guard.selection_shortcut_id = reg_outcome.selection_id;
        guard.selection_shortcut_error = reg_outcome.selection_error;
        let ss = if !guard.staged_stack.is_empty() {
            Some(build_stack_view_state(&guard))
        } else {
            None
        };
        let st = build_shortcut_status(&guard);
        (ss, st)
    };
    let theme = new_config.theme.clone();
    let ask_actions_emit = new_config.ask_quick_actions.clone();
    let app_ui = app.clone();
    post_to_ui(app, move || {
        if let Some(main_win) = app_ui.get_webview_window("main") {
            let _ = main_win.emit("shortcut:status", &shortcut_status);
        }
        apply_native_window_theme(&app_ui, &theme);
        let _ = app_ui.emit("theme:updated", &theme);
        let _ = app_ui.emit("ask-actions:updated", &ask_actions_emit);
    });
    if let Some(ss) = stack_state {
        emit_stack_update(app, &ss);
    }
    Ok(new_config)
}

#[tauri::command]
fn preview_theme(app: AppHandle, state: State<'_, Mutex<AppState>>, theme: String) -> Result<String, String> {
    let normalized = normalize_theme(&theme);
    if let Ok(mut guard) = state.lock() {
        guard.config.theme = normalized.clone();
    }
    let theme_emit = normalized.clone();
    let app_ui = app.clone();
    post_to_ui(&app, move || {
        apply_native_window_theme(&app_ui, &theme_emit);
        let _ = app_ui.emit("theme:updated", &theme_emit);
    });
    Ok(normalized)
}

#[tauri::command(async)]
fn update_config(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    new_config: AppConfig,
) -> Result<AppConfig, String> {
    apply_and_persist_config(&app, &state, new_config)
}

#[tauri::command]
async fn test_api_connection(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    config: AppConfig,
    target: Option<String>,
) -> Result<String, String> {
    let target_name = target.as_deref().unwrap_or("ai");
    if target_name == "update" {
        return updates::test_update_connection(&app, &config.proxy_url, &config.update_feed_url)
            .await;
    }
    {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if target_name == "ai" {
            require_key_for_origin(&guard.config, &config)?;
        }
    }
    let client = build_api_client(&config.proxy_url)?;
    test_connection(&client, &config, target_name).await
}

#[tauri::command(async)]
fn start_capture(app: AppHandle, hide_main: Option<bool>) -> Result<(), String> {
    let hide_main = hide_main.unwrap_or(false);
    let result = trigger_capture(&app, hide_main);
    if hide_main && result.is_err() {
        let app_ui = app.clone();
        post_to_ui(&app, move || show_main_window(&app_ui));
    }
    result
}

#[tauri::command]
fn get_current_capture(state: State<'_, Mutex<AppState>>) -> Result<Option<OverlayPayload>, String> {
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    Ok(guard.last_capture.as_deref().map(|c| OverlayPayload {
        capture_id: guard.current_capture_id,
        monitor_width: c.monitor_width,
        monitor_height: c.monitor_height,
        scale_factor: c.scale_factor,
        auto_translate_on_release: guard.config.auto_translate_on_release,
        translate_engine: guard.config.translate_engine.clone(),
        target_lang: guard.config.target_lang.clone(),
        staged_count: guard.staged_stack.len(),
        max_stack_size: guard.config.max_stack_size.clamp(2, 12),
        latest_staged_thumb: guard
            .staged_stack
            .last()
            .map(|s| s.thumb_data_url.clone())
            .unwrap_or_default(),
    }))
}

#[tauri::command(async)]
fn get_capture_image(
    window: tauri::WebviewWindow,
    state: State<'_, Mutex<AppState>>,
    capture_id: u64,
) -> Result<tauri::ipc::Response, String> {
    if window.label() != "overlay" {
        return Err("仅截图窗口可读取当前画面".into());
    }
    let capture = active_capture(&state, capture_id)?;
    Ok(tauri::ipc::Response::new(capture.png_bytes.clone()))
}

#[tauri::command]
fn capture_has_staged_items(
    state: State<'_, Mutex<AppState>>,
    capture_id: u64,
) -> Result<bool, String> {
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    if guard.current_capture_id != capture_id || guard.last_capture.is_none() {
        return Err("截图已经关闭".into());
    }
    Ok(!guard.staged_stack.is_empty())
}

#[tauri::command(async)]
fn overlay_image_ready(app: AppHandle, capture_id: u64) {
    present_overlay_window(&app, capture_id);
}

#[tauri::command]
fn cancel_capture(app: AppHandle, state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    if let Ok(mut guard) = state.lock() {
        guard.is_capturing = false;
        guard.last_capture = None;
        guard.overlay_crops.clear();
        guard.overlay_ask_messages.clear();
        guard.overlay_job_id += 1;
        guard.overlay_analysis_state = OverlayAnalysisState::Idle;
    }
    close_overlay_window(&app);
    restore_stack_if_nonempty(&app);
    Ok(())
}

fn active_capture(state: &Mutex<AppState>, capture_id: u64) -> Result<Arc<CapturedScreen>, String> {
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    if guard.current_capture_id != capture_id {
        return Err("截图已经关闭".to_string());
    }
    guard.last_capture.clone().ok_or_else(|| "截图已经关闭".to_string())
}

fn finish_selection_output(app: &AppHandle, state: &Mutex<AppState>, capture_id: u64) -> Result<(), String> {
    let closed = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if guard.current_capture_id != capture_id || guard.last_capture.is_none() {
            false
        } else {
            guard.last_capture = None;
            guard.overlay_crops.clear();
            guard.overlay_ask_messages.clear();
            guard.overlay_job_id += 1;
            guard.overlay_analysis_state = OverlayAnalysisState::Idle;
            guard.is_capturing = false;
            true
        }
    };
    if closed {
        close_overlay_window(app);
        restore_stack_if_nonempty(app);
    }
    Ok(())
}

#[tauri::command(async)]
fn copy_selection_image(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    capture_id: u64,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
    annotated_base64: Option<String>,
) -> Result<(), String> {
    let capture = active_capture(&state, capture_id)?;
    let crop = resolve_crop_from_input(Some(&capture), rx, ry, rw, rh, annotated_base64)?;
    active_capture(&state, capture_id)?;
    copy_png_bytes_to_clipboard(&crop.png_bytes)?;
    finish_selection_output(&app, &state, capture_id)
}

#[tauri::command]
async fn save_selection_image(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    capture_id: u64,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
    annotated_base64: Option<String>,
) -> Result<bool, String> {
    let capture = active_capture(&state, capture_id)?;
    let crop = resolve_crop_from_input(Some(&capture), rx, ry, rw, rh, annotated_base64)?;
    active_capture(&state, capture_id)?;
    let app_ui = app.clone();
    post_to_ui(&app, move || {
        let state = app_ui.state::<Mutex<AppState>>();
        if active_capture(&state, capture_id).is_ok() {
            if let Some(overlay) = app_ui.get_webview_window("overlay") {
                let _ = overlay.hide();
            }
        }
    });
    let saved = save_png_with_dialog(crop.png_bytes, "Luma").await;
    if matches!(saved, Ok(true)) {
        finish_selection_output(&app, &state, capture_id)?;
    } else {
        present_overlay_window(&app, capture_id);
    }
    saved
}

#[tauri::command(async)]
fn submit_selection(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
    mode: Option<String>,
    annotated_base64: Option<String>,
    initial_prompt: Option<String>,
) -> Result<(), String> {
    let chosen_mode = mode.unwrap_or_else(|| "translate".to_string());

    if chosen_mode == "pin" {
        let (crop, tx, ty, capture_id) = {
            let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
            let crop = resolve_crop_from_input(
                guard.last_capture.as_deref(),
                rx,
                ry,
                rw,
                rh,
                annotated_base64,
            )?;
            let scale = f64::from(crop.scale_factor.max(1.0));
            let lw = (crop.pixel_width as f64 / scale).clamp(140.0, 920.0) + 12.0;
            let lh = (crop.pixel_height as f64 / scale).clamp(48.0, 720.0) + 12.0;
            let (tx, ty) = if let Some(captured) = guard.last_capture.as_deref() {
                compute_popup_position(captured, rx, ry, rw, rh, lw, lh, true)
            } else {
                (120, 120)
            };
            (crop, tx, ty, guard.current_capture_id)
        };
        create_pin_window(&app, crop, Some((tx, ty)), 0)?;
        return finish_selection_output(&app, &state, capture_id);
    }

    let (
        crop,
        target_x,
        target_y,
        win_w,
        win_h,
        job_id,
        config,
        client,
        view_state,
        ask_msgs_to_run,
        source_hwnd,
    ) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let source_hwnd = guard.last_capture_source_hwnd;
        let crop = resolve_crop_from_input(
            guard.last_capture.as_deref(),
            rx,
            ry,
            rw,
            rh,
            annotated_base64,
        )?;

        let (win_w, win_h, over_selection) = if chosen_mode == "ask" {
            (456.0, 470.0, false)
        } else {
            (456.0, 210.0, false)
        };

        let (tx, ty) = if let Some(captured) = guard.last_capture.as_deref() {
            compute_popup_position(captured, rx, ry, rw, rh, win_w, win_h, over_selection)
        } else {
            (120, 120)
        };

        guard.current_job_id += 1;
        let job_id = guard.current_job_id;
        guard.last_result = None;
        guard.last_input_text = None;
        guard.ask_messages.clear();
        guard.result_anchor_below = true;
        guard.result_anchor_y = None;

        let (view_state, ask_msgs_to_run) = if chosen_mode == "ask" {
            let thumb = crop.make_thumbnail_data_url();
            let prompt_text = initial_prompt
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if let Some(q) = prompt_text {
                let msgs = vec![AiChatMessage {
                    role: "user".to_string(),
                    content: q,
                }];
                guard.ask_messages = msgs.clone();
                let vs = ResultViewState::Ask {
                    job_id,
                    image_data_url: thumb,
                    pixel_width: crop.pixel_width,
                    pixel_height: crop.pixel_height,
                    messages: msgs.clone(),
                    loading: true,
                    error: None,
                    source_text_preview: None,
                };
                (vs, Some(msgs))
            } else {
                let vs = ResultViewState::Ask {
                    job_id,
                    image_data_url: thumb,
                    pixel_width: crop.pixel_width,
                    pixel_height: crop.pixel_height,
                    messages: Vec::new(),
                    loading: false,
                    error: None,
                    source_text_preview: None,
                };
                (vs, None)
            }
        } else {
            let vs = ResultViewState::Loading {
                job_id,
                mode: chosen_mode.clone(),
                engine: if chosen_mode == "ocr" {
                    "ai".to_string()
                } else {
                    guard.config.translate_engine.clone()
                },
                image_data_url: crop.make_thumbnail_data_url(),
                target_lang: guard.config.target_lang.clone(),
                pixel_width: crop.pixel_width,
                pixel_height: crop.pixel_height,
            };
            (vs, None)
        };

        guard.last_crop = Some(crop.clone());
        guard.latest_result_state = view_state.clone();
        // Release full-screen capture immediately to keep RAM footprint tiny
        guard.last_capture = None;
        guard.is_capturing = false;

        (
            crop,
            tx,
            ty,
            win_w,
            win_h,
            job_id,
            guard.config.clone(),
            guard.client.clone(),
            view_state,
            ask_msgs_to_run,
            source_hwnd,
        )
    };

    let app_ui = app.clone();
    post_to_ui(&app, move || {
        if let Some(overlay) = app_ui.get_webview_window("overlay") {
            let _ = overlay.hide();
            dispatch_window_event(&overlay, "capture:clear", &());
        }
        if let Some(result_win) = app_ui.get_webview_window("result") {
            let _ = result_win.set_size(LogicalSize::new(win_w, win_h));
            let _ = result_win.set_position(PhysicalPosition::new(target_x, target_y));
            dispatch_window_event(&result_win, "result:update", &view_state);
            show_result_window_foreground(&app_ui, &result_win, source_hwnd);
        }
    });

    if chosen_mode == "translate" || chosen_mode == "ocr" {
        let engine = if chosen_mode == "ocr" {
            "ai".to_string()
        } else {
            config.translate_engine.clone()
        };
        let target_lang = config.target_lang.clone();
        spawn_image_job(
            app.clone(),
            job_id,
            chosen_mode,
            engine,
            target_lang,
            crop,
            config,
            client,
        );
    } else if let Some(msgs) = ask_msgs_to_run {
        spawn_ask_ai_job(app.clone(), job_id, crop, msgs, config, client);
    }

    restore_stack_if_nonempty(&app);
    Ok(())
}

#[tauri::command(async)]
fn confirm_selection(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
) -> Result<(), String> {
    submit_selection(app, state, rx, ry, rw, rh, Some("translate".to_string()), None, None)
}

#[tauri::command(async)]
fn switch_result_mode(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    mode: String,
    engine_override: Option<String>,
    target_lang_override: Option<String>,
) -> Result<(), String> {
    enum NextAction {
        None,
        ImageJob {
            job_id: u64,
            mode: String,
            engine: String,
            target_lang: String,
            crop: CroppedRegion,
            config: AppConfig,
            client: Client,
        },
        TextRetranslate {
            job_id: u64,
            original_text: String,
            source_lang: String,
            thumb_url: String,
            target_lang: String,
            engine: String,
            config: AppConfig,
            client: Client,
        },
    }

    if mode == "pin" {
        let (crop, job_id) = {
            let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
            let crop = guard.last_crop.clone().ok_or("当前没有可用的选区截图")?;
            (crop, guard.current_job_id)
        };
        let pos_override = app
            .get_webview_window("result")
            .and_then(|w| w.outer_position().ok().map(|p| (p.x, p.y)));
        create_pin_window(&app, crop, pos_override, 0)?;
        {
            let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
            if guard.current_job_id != job_id { return Ok(()); }
            guard.current_job_id += 1;
            guard.latest_result_state = ResultViewState::Idle;
        }
        result_popover::deactivate();
        let app_ui = app.clone();
        post_to_ui(&app, move || {
            if let Some(result_win) = app_ui.get_webview_window("result") {
                dispatch_window_event(&result_win, "result:update", &ResultViewState::Idle);
                let _ = result_win.set_always_on_top(false);
                let _ = result_win.hide();
            }
        });
        return Ok(());
    }

    let (view_state, resize_to, action) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let crop_opt = guard.last_crop.clone();

        if mode == "ask" {
            guard.current_job_id += 1;
            let job_id = guard.current_job_id;
            if let Some(crop) = crop_opt {
                let thumb = crop.make_thumbnail_data_url();
                let vs = ResultViewState::Ask {
                    job_id,
                    image_data_url: thumb,
                    pixel_width: crop.pixel_width,
                    pixel_height: crop.pixel_height,
                    messages: guard.ask_messages.clone(),
                    loading: false,
                    error: None,
                    source_text_preview: None,
                };
                guard.latest_result_state = vs.clone();
                (vs, Some((456.0, 470.0)), NextAction::None)
            } else {
                let source_text = guard
                    .last_input_text
                    .as_ref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        guard
                            .last_result
                            .as_ref()
                            .map(|r| r.original_text.trim().to_string())
                            .filter(|s| !s.is_empty())
                    })
                    .ok_or_else(|| "当前没有可用的选区截图或文本".to_string())?;
                guard.last_input_text = Some(source_text.clone());
                let vs = ResultViewState::Ask {
                    job_id,
                    image_data_url: String::new(),
                    pixel_width: 0,
                    pixel_height: 0,
                    messages: guard.ask_messages.clone(),
                    loading: false,
                    error: None,
                    source_text_preview: Some(build_source_text_preview(&source_text)),
                };
                guard.latest_result_state = vs.clone();
                (vs, Some((456.0, 470.0)), NextAction::None)
            }
        } else {
            let fallback_text = guard
                .last_result
                .as_ref()
                .map(|r| r.original_text.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    guard
                        .last_input_text
                        .as_ref()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                });
            let can_reuse_text = mode == "translate" && fallback_text.is_some();

            if crop_opt.is_none() && !can_reuse_text {
                return Err("当前没有可用的选区截图".to_string());
            }

            guard.current_job_id += 1;
            let job_id = guard.current_job_id;
            let engine = engine_override.unwrap_or_else(|| {
                if mode == "ocr" {
                    "ai".to_string()
                } else {
                    guard.config.translate_engine.clone()
                }
            });
            let target_lang =
                target_lang_override.unwrap_or_else(|| guard.config.target_lang.clone());
            let thumb = crop_opt
                .as_ref()
                .map(|c| c.make_thumbnail_data_url())
                .unwrap_or_else(|| {
                    guard
                        .last_result
                        .as_ref()
                        .map(|r| r.image_data_url.clone())
                        .unwrap_or_default()
                });
            let (pw, ph) = crop_opt
                .as_ref()
                .map(|c| (c.pixel_width, c.pixel_height))
                .unwrap_or((320, 180));

            let vs = ResultViewState::Loading {
                job_id,
                mode: mode.clone(),
                engine: engine.clone(),
                image_data_url: thumb.clone(),
                target_lang: target_lang.clone(),
                pixel_width: pw,
                pixel_height: ph,
            };
            guard.latest_result_state = vs.clone();

            let action = if can_reuse_text {
                let orig_text = fallback_text.unwrap();
                let source_lang = guard
                    .last_result
                    .as_ref()
                    .map(|r| r.source_lang.clone())
                    .unwrap_or_else(|| "自动识别".to_string());
                NextAction::TextRetranslate {
                    job_id,
                    original_text: orig_text,
                    source_lang,
                    thumb_url: thumb,
                    target_lang,
                    engine,
                    config: guard.config.clone(),
                    client: guard.client.clone(),
                }
            } else {
                NextAction::ImageJob {
                    job_id,
                    mode,
                    engine,
                    target_lang,
                    crop: crop_opt.unwrap(),
                    config: guard.config.clone(),
                    client: guard.client.clone(),
                }
            };

            (vs, Some((456.0, 210.0)), action)
        }
    };

    if let Some((w, h)) = resize_to {
        let app_ui = app.clone();
        post_to_ui(&app, move || {
            if let Some(result_win) = app_ui.get_webview_window("result") {
                let _ = result_win.set_size(LogicalSize::new(w, h));
            }
        });
    }
    emit_result_update(&app, &view_state);

    match action {
        NextAction::None => {}
        NextAction::ImageJob {
            job_id,
            mode,
            engine,
            target_lang,
            crop,
            config,
            client,
        } => {
            spawn_image_job(app, job_id, mode, engine, target_lang, crop, config, client);
        }
        NextAction::TextRetranslate {
            job_id,
            original_text,
            source_lang,
            thumb_url,
            target_lang,
            engine,
            config,
            client,
        } => {
            spawn_text_retranslate_job(
                app,
                job_id,
                original_text,
                source_lang,
                thumb_url,
                target_lang,
                engine,
                config,
                client,
            );
        }
    }

    Ok(())
}

#[tauri::command(async)]
fn ask_ai_follow_up(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    question: String,
) -> Result<(), String> {
    let trimmed = question.trim().to_string();
    if trimmed.is_empty() {
        return Err("请输入想要询问的问题".to_string());
    }

    enum AskSource {
        Crop(CroppedRegion),
        Text(String),
    }

    let (source, job_id, messages, config, client, view_state) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let source = if let Some(crop) = guard.last_crop.clone() {
            AskSource::Crop(crop)
        } else if let Some(text) = guard
            .last_input_text
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                guard
                    .last_result
                    .as_ref()
                    .map(|r| r.original_text.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
        {
            guard.last_input_text = Some(text.clone());
            AskSource::Text(text)
        } else {
            return Err("当前没有可用的选区截图或文本".to_string());
        };

        guard.current_job_id += 1;
        let job_id = guard.current_job_id;
        guard.ask_messages.push(AiChatMessage {
            role: "user".to_string(),
            content: trimmed,
        });
        let msgs = guard.ask_messages.clone();
        let vs = match &source {
            AskSource::Crop(crop) => ResultViewState::Ask {
                job_id,
                image_data_url: crop.make_thumbnail_data_url(),
                pixel_width: crop.pixel_width,
                pixel_height: crop.pixel_height,
                messages: msgs.clone(),
                loading: true,
                error: None,
                source_text_preview: None,
            },
            AskSource::Text(text) => ResultViewState::Ask {
                job_id,
                image_data_url: String::new(),
                pixel_width: 0,
                pixel_height: 0,
                messages: msgs.clone(),
                loading: true,
                error: None,
                source_text_preview: Some(build_source_text_preview(text)),
            },
        };
        guard.latest_result_state = vs.clone();
        (
            source,
            job_id,
            msgs,
            guard.config.clone(),
            guard.client.clone(),
            vs,
        )
    };

    emit_result_update(&app, &view_state);
    match source {
        AskSource::Crop(crop) => spawn_ask_ai_job(app, job_id, crop, messages, config, client),
        AskSource::Text(text) => {
            spawn_ask_ai_text_job(app, job_id, text, messages, config, client)
        }
    }
    Ok(())
}

#[tauri::command(async)]
fn copy_last_crop_image(state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    let png_bytes = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard
            .last_crop
            .as_ref()
            .ok_or_else(|| "当前没有可复制的截图".to_string())?
            .png_bytes
            .clone()
    };
    copy_png_bytes_to_clipboard(&png_bytes)
}

#[tauri::command]
async fn save_last_crop_image(state: State<'_, Mutex<AppState>>) -> Result<bool, String> {
    let png_bytes = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard
            .last_crop
            .as_ref()
            .ok_or_else(|| "当前没有可保存的截图".to_string())?
            .png_bytes
            .clone()
    };

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let default_name = format!("Luma_{ts}.png");

    let file = rfd::AsyncFileDialog::new()
        .add_filter("PNG 图片", &["png"])
        .set_file_name(default_name)
        .save_file()
        .await;

    if let Some(handle) = file {
        handle
            .write(&png_bytes)
            .await
            .map_err(|e| format!("保存图片失败: {e}"))?;
        return Ok(true);
    }
    Ok(false)
}

#[tauri::command(async)]
fn retry_translation(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    engine_override: Option<String>,
    target_lang_override: Option<String>,
) -> Result<(), String> {
    enum RetryTarget {
        Image(CroppedRegion),
        Text {
            original_text: String,
            source_lang: String,
            thumb_url: String,
        },
    }

    let (mode, engine, target_lang, target, job_id, config, client, loading_state) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let crop_opt = guard.last_crop.clone();
        let mode = match &guard.latest_result_state {
            ResultViewState::Loading { mode, .. } | ResultViewState::Error { mode, .. } => {
                mode.clone()
            }
            ResultViewState::Done { result, .. } => result.mode.clone(),
            _ => "translate".to_string(),
        };

        let edited_or_clipboard_text = guard.last_input_text.clone().filter(|text| !text.trim().is_empty());
        let target = if let Some(text) = edited_or_clipboard_text.filter(|_| mode == "translate") {
            RetryTarget::Text {
                original_text: text,
                source_lang: guard.last_result.as_ref().map(|r| r.source_lang.clone())
                    .unwrap_or_else(|| "自动识别".to_string()),
                thumb_url: guard.last_result.as_ref().map(|r| r.image_data_url.clone())
                    .or_else(|| crop_opt.as_ref().map(CroppedRegion::make_thumbnail_data_url))
                    .unwrap_or_default(),
            }
        } else if let Some(crop) = crop_opt {
            RetryTarget::Image(crop)
        } else {
            return Err("没有可重试的内容".to_string());
        };

        guard.current_job_id += 1;
        let job_id = guard.current_job_id;
        let engine = engine_override.unwrap_or_else(|| {
            if mode == "ocr" {
                "ai".to_string()
            } else {
                guard.config.translate_engine.clone()
            }
        });
        let target_lang = target_lang_override.unwrap_or_else(|| guard.config.target_lang.clone());
        let (thumb, pw, ph) = match &target {
            RetryTarget::Image(crop) => (
                crop.make_thumbnail_data_url(),
                crop.pixel_width,
                crop.pixel_height,
            ),
            RetryTarget::Text { thumb_url, .. } => (thumb_url.clone(), 320, 180),
        };
        let loading_state = ResultViewState::Loading {
            job_id,
            mode: mode.clone(),
            engine: engine.clone(),
            image_data_url: thumb,
            target_lang: target_lang.clone(),
            pixel_width: pw,
            pixel_height: ph,
        };
        guard.latest_result_state = loading_state.clone();
        (
            mode,
            engine,
            target_lang,
            target,
            job_id,
            guard.config.clone(),
            guard.client.clone(),
            loading_state,
        )
    };

    emit_result_update(&app, &loading_state);
    match target {
        RetryTarget::Image(crop) => {
            spawn_image_job(app, job_id, mode, engine, target_lang, crop, config, client);
        }
        RetryTarget::Text {
            original_text,
            source_lang,
            thumb_url,
        } => {
            spawn_text_retranslate_job(
                app,
                job_id,
                original_text,
                source_lang,
                thumb_url,
                target_lang,
                engine,
                config,
                client,
            );
        }
    }
    Ok(())
}

fn present_external_crop_in_result(
    app: &AppHandle,
    crop: CroppedRegion,
    mode: &str,
) -> Result<(), String> {
    let source_hwnd = result_popover::capture_external_foreground_root_hwnd();
    let chosen_mode = match mode {
        "ocr" => "ocr".to_string(),
        "ask" => "ask".to_string(),
        _ => "translate".to_string(),
    };

    let last_rect = app.state::<Mutex<AppState>>().lock()
        .map_err(|_| "状态锁异常".to_string())?.last_monitor_rect;
    let (mon_x, mon_y, mon_w, mon_h, mon_scale) = get_anchor_monitor_rect(app, last_rect);

    let (job_id, config, client, view_state, win_w, win_h, target_x, target_y) = {
        let state = app.state::<Mutex<AppState>>();
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard.current_job_id += 1;
        let job_id = guard.current_job_id;
        guard.last_crop = Some(crop.clone());
        guard.last_result = None;
        guard.last_input_text = None;
        guard.ask_messages.clear();
        guard.result_anchor_below = true;
        guard.result_anchor_y = None;

        let sf = f64::from(mon_scale.max(1.0));
        let (win_w, win_h) = if chosen_mode == "ask" {
            (456.0, 470.0)
        } else {
            (456.0, 210.0)
        };
        let w_phys = (win_w * sf).round() as i32;
        let h_phys = (win_h * sf).round() as i32;
        let tx = mon_x + ((mon_w as i32 - w_phys) / 2).max(24);
        let ty = mon_y + ((mon_h as i32 - h_phys) / 2).max(24);

        let thumb = crop.make_thumbnail_data_url();
        let vs = if chosen_mode == "ask" {
            ResultViewState::Ask {
                job_id,
                image_data_url: thumb,
                pixel_width: crop.pixel_width,
                pixel_height: crop.pixel_height,
                messages: Vec::new(),
                loading: false,
                error: None,
                source_text_preview: None,
            }
        } else {
            ResultViewState::Loading {
                job_id,
                mode: chosen_mode.clone(),
                engine: if chosen_mode == "ocr" {
                    "ai".to_string()
                } else {
                    guard.config.translate_engine.clone()
                },
                image_data_url: thumb,
                target_lang: guard.config.target_lang.clone(),
                pixel_width: crop.pixel_width,
                pixel_height: crop.pixel_height,
            }
        };
        guard.latest_result_state = vs.clone();
        (
            job_id,
            guard.config.clone(),
            guard.client.clone(),
            vs,
            win_w,
            win_h,
            tx,
            ty,
        )
    };

    let app_ui = app.clone();
    post_to_ui(&app, move || {
        if let Some(result_win) = app_ui.get_webview_window("result") {
            let _ = result_win.set_size(LogicalSize::new(win_w, win_h));
            let _ = result_win.set_position(PhysicalPosition::new(target_x, target_y));
            dispatch_window_event(&result_win, "result:update", &view_state);
            show_result_window_foreground(&app_ui, &result_win, source_hwnd);
        }
    });

    if chosen_mode == "translate" || chosen_mode == "ocr" {
        let engine = if chosen_mode == "ocr" {
            "ai".to_string()
        } else {
            config.translate_engine.clone()
        };
        let target_lang = config.target_lang.clone();
        spawn_image_job(
            app.clone(),
            job_id,
            chosen_mode,
            engine,
            target_lang,
            crop,
            config,
            client,
        );
    }

    Ok(())
}

#[tauri::command(async)]
fn analyze_clipboard_image(app: AppHandle, mode: Option<String>) -> Result<(), String> {
    let crop = read_clipboard_cropped_region()?;
    present_external_crop_in_result(&app, crop, mode.as_deref().unwrap_or("translate"))
}

#[tauri::command(async)]
fn retranslate_custom_text(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    text: String,
    engine_override: Option<String>,
    target_lang_override: Option<String>,
) -> Result<(), String> {
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Err("待翻译文本不能为空".to_string());
    }

    let (job_id, source_lang, thumb_url, target_lang, engine, config, client, loading_state) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard.current_job_id += 1;
        let job_id = guard.current_job_id;
        guard.last_input_text = Some(trimmed.clone());
        let engine = engine_override.unwrap_or_else(|| guard.config.translate_engine.clone());
        let target_lang = target_lang_override.unwrap_or_else(|| guard.config.target_lang.clone());
        let thumb_url = guard
            .last_crop
            .as_ref()
            .map(|c| c.make_thumbnail_data_url())
            .unwrap_or_else(|| {
                guard
                    .last_result
                    .as_ref()
                    .map(|r| r.image_data_url.clone())
                    .unwrap_or_default()
            });
        let source_lang = guard
            .last_result
            .as_ref()
            .map(|r| r.source_lang.clone())
            .unwrap_or_else(|| "自动识别".to_string());
        let (pw, ph) = guard
            .last_crop
            .as_ref()
            .map(|c| (c.pixel_width, c.pixel_height))
            .unwrap_or((320, 180));

        let vs = ResultViewState::Loading {
            job_id,
            mode: "translate".to_string(),
            engine: engine.clone(),
            image_data_url: thumb_url.clone(),
            target_lang: target_lang.clone(),
            pixel_width: pw,
            pixel_height: ph,
        };
        guard.latest_result_state = vs.clone();
        (
            job_id,
            source_lang,
            thumb_url,
            target_lang,
            engine,
            guard.config.clone(),
            guard.client.clone(),
            vs,
        )
    };

    emit_result_update(&app, &loading_state);
    spawn_text_retranslate_job(
        app,
        job_id,
        trimmed,
        source_lang,
        thumb_url,
        target_lang,
        engine,
        config,
        client,
    );
    Ok(())
}

#[tauri::command]
fn get_latest_result_state(state: State<'_, Mutex<AppState>>) -> Result<ResultViewState, String> {
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    Ok(guard.latest_result_state.clone())
}

#[tauri::command]
fn hide_result_window(app: AppHandle, _state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    hide_result_window_internal(&app);
    Ok(())
}

#[tauri::command]
fn get_app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tauri::command]
fn get_history(state: State<'_, Mutex<AppState>>) -> Result<Vec<TranslationResult>, String> {
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    Ok(guard.history.clone())
}

#[tauri::command(async)]
fn clear_history(app: AppHandle, state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard.history.clear();
    }
    persist_history_and_notify(&app)
}

#[tauri::command(async)]
fn delete_history_item(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    id: u64,
) -> Result<Vec<TranslationResult>, String> {
    let snapshot = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard.history.retain(|item| item.id != id);
        guard.history.clone()
    };
    persist_history_and_notify(&app)?;
    Ok(snapshot)
}

#[tauri::command]
async fn export_history(
    state: State<'_, Mutex<AppState>>,
    format: String,
) -> Result<bool, String> {
    let items = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard.history.clone()
    };
    if items.is_empty() {
        return Err("暂无可导出的历史记录".to_string());
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let is_json = format.trim().eq_ignore_ascii_case("json");
    let (ext, filter_name, content) = if is_json {
        let json = serde_json::to_string_pretty(&items)
            .map_err(|e| format!("序列化历史记录失败: {e}"))?;
        ("json", "JSON 文件", json)
    } else {
        ("md", "Markdown 文档", format_history_markdown(&items))
    };

    let file = rfd::AsyncFileDialog::new()
        .add_filter(filter_name, &[ext])
        .set_file_name(format!("Luma_History_{ts}.{ext}"))
        .save_file()
        .await;

    if let Some(handle) = file {
        handle
            .write(content.as_bytes())
            .await
            .map_err(|e| format!("导出历史记录失败: {e}"))?;
        return Ok(true);
    }
    Ok(false)
}

#[tauri::command]
async fn translate_text_direct(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    text: String,
    engine_override: Option<String>,
    target_lang_override: Option<String>,
    request_id: Option<u64>,
) -> Result<TranslationResult, String> {
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Err("请输入需要翻译的文本".to_string());
    }
    let (config, client, engine, target_lang) = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let eng = engine_override.unwrap_or_else(|| guard.config.translate_engine.clone());
        let lang = target_lang_override.unwrap_or_else(|| guard.config.target_lang.clone());
        (guard.config.clone(), guard.client.clone(), eng, lang)
    };

    let req_id = request_id.unwrap_or(0);
    let stream_sink = make_throttled_stream_sink(
        app.clone(),
        "main",
        "main:text-stream",
        req_id,
        "text",
        |_| true,
    );

    let res = translate_text_only(
        &client,
        &config,
        0,
        &trimmed,
        "自动识别",
        "",
        &target_lang,
        &engine,
        if engine == "ai" {
            Some(&stream_sink)
        } else {
            None
        },
    )
    .await?;

    let (stored, _) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        push_history_locked(&mut guard, res)
    };
    persist_history_and_notify(&app)?;
    Ok(stored)
}

fn present_clipboard_text_in_result(app: &AppHandle) -> Result<(), String> {
    let source_hwnd = result_popover::capture_external_foreground_root_hwnd();
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("无法访问系统剪贴板: {e}"))?;
    let text = clipboard
        .get_text()
        .map_err(|_| "剪贴板中当前没有可用的纯文本".to_string())?;
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Err("剪贴板文本为空".to_string());
    }

    let last_rect = app.state::<Mutex<AppState>>().lock()
        .map_err(|_| "状态锁异常".to_string())?.last_monitor_rect;
    let (mon_x, mon_y, mon_w, mon_h, mon_scale) = get_anchor_monitor_rect(app, last_rect);

    let (job_id, target_lang, engine, config, client, loading_state, tx, ty) = {
        let state = app.state::<Mutex<AppState>>();
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard.current_job_id += 1;
        let job_id = guard.current_job_id;
        guard.last_crop = None;
        guard.last_result = None;
        guard.last_input_text = Some(trimmed.clone());
        guard.ask_messages.clear();
        guard.result_anchor_below = true;
        guard.result_anchor_y = None;

        let sf = f64::from(mon_scale.max(1.0));
        let w_phys = (456.0 * sf).round() as i32;
        let h_phys = (210.0 * sf).round() as i32;
        let tx = mon_x + ((mon_w as i32 - w_phys) / 2).max(24);
        let ty = mon_y + ((mon_h as i32 - h_phys) / 2).max(24);

        let engine = guard.config.translate_engine.clone();
        let target_lang = guard.config.target_lang.clone();
        let vs = ResultViewState::Loading {
            job_id,
            mode: "translate".to_string(),
            engine: engine.clone(),
            image_data_url: String::new(),
            target_lang: target_lang.clone(),
            pixel_width: 320,
            pixel_height: 180,
        };
        guard.latest_result_state = vs.clone();
        (
            job_id,
            target_lang,
            engine,
            guard.config.clone(),
            guard.client.clone(),
            vs,
            tx,
            ty,
        )
    };

    let app_ui = app.clone();
    post_to_ui(app, move || {
        if let Some(result_win) = app_ui.get_webview_window("result") {
            let _ = result_win.set_size(LogicalSize::new(456.0, 210.0));
            let _ = result_win.set_position(PhysicalPosition::new(tx, ty));
            dispatch_window_event(&result_win, "result:update", &loading_state);
            show_result_window_foreground(&app_ui, &result_win, source_hwnd);
        }
    });

    spawn_text_retranslate_job(
        app.clone(),
        job_id,
        trimmed,
        "自动识别".to_string(),
        String::new(),
        target_lang,
        engine,
        config,
        client,
    );
    Ok(())
}

#[tauri::command(async)]
fn analyze_clipboard_text(app: AppHandle) -> Result<(), String> {
    present_clipboard_text_in_result(&app)
}

fn present_selected_text_in_result(app: &AppHandle) -> Result<(), String> {
    present_selected_text_in_result_with_rect(app, None)
}

fn present_selected_text_in_result_with_rect(
    app: &AppHandle,
    selection_rect: Option<selection_capture::SelectionAnchorRect>,
) -> Result<(), String> {
    let source_hwnd = result_popover::capture_external_foreground_root_hwnd();
    let capture_res = selection_capture::capture_selected_text_without_polluting_clipboard()?;
    let trimmed = capture_res.text.trim().to_string();
    if trimmed.is_empty() {
        return Err("未获取到选中的文字".to_string());
    }

    let last_rect = app
        .state::<Mutex<AppState>>()
        .lock()
        .map_err(|_| "状态锁异常".to_string())?
        .last_monitor_rect;
    let (mon_x, mon_y, mon_w, mon_h, mon_scale) = get_anchor_monitor_rect(app, last_rect);

    let (job_id, target_lang, engine, config, client, loading_state, fallback_tx, fallback_ty) = {
        let state = app.state::<Mutex<AppState>>();
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard.current_job_id += 1;
        let job_id = guard.current_job_id;
        guard.last_crop = None;
        guard.last_result = None;
        guard.last_input_text = Some(trimmed.clone());
        guard.ask_messages.clear();
        guard.result_anchor_below = true;
        guard.result_anchor_y = None;

        let sf = f64::from(mon_scale.max(1.0));
        let w_phys = (456.0 * sf).round() as i32;
        let h_phys = (210.0 * sf).round() as i32;
        let tx = mon_x + ((mon_w as i32 - w_phys) / 2).max(24);
        let ty = mon_y + ((mon_h as i32 - h_phys) / 2).max(24);

        let engine = guard.config.translate_engine.clone();
        let target_lang = guard.config.target_lang.clone();
        let vs = ResultViewState::Loading {
            job_id,
            mode: "translate".to_string(),
            engine: engine.clone(),
            image_data_url: String::new(),
            target_lang: target_lang.clone(),
            pixel_width: 320,
            pixel_height: 180,
        };
        guard.latest_result_state = vs.clone();
        (
            job_id,
            target_lang,
            engine,
            guard.config.clone(),
            guard.client.clone(),
            vs,
            tx,
            ty,
        )
    };

    let cursor_x = capture_res.cursor_x;
    let cursor_y = capture_res.cursor_y;
    let app_ui = app.clone();
    post_to_ui(app, move || {
        if let Some(result_win) = app_ui.get_webview_window("result") {
            let mut work_rect = result_win
                .monitor_from_point(f64::from(cursor_x), f64::from(cursor_y))
                .ok()
                .flatten()
                .map(|mon| {
                    let wa = mon.work_area();
                    selection_capture::WorkAreaRect {
                        left: wa.position.x,
                        top: wa.position.y,
                        right: wa.position.x + wa.size.width as i32,
                        bottom: wa.position.y + wa.size.height as i32,
                        scale_factor: mon.scale_factor() as f32,
                    }
                })
                .or_else(|| selection_capture::work_area_for_point(cursor_x, cursor_y));

            let (tx, ty, placed_below, scale_val) = if let Some(ref mut wr) = work_rect {
                if wr.scale_factor <= 0.0 {
                    wr.scale_factor = mon_scale.max(1.0);
                }
                let (px, py, below) = selection_capture::compute_selection_popup_position(
                    cursor_x,
                    cursor_y,
                    selection_rect,
                    *wr,
                    456.0,
                    210.0,
                );
                (px, py, below, f64::from(wr.scale_factor))
            } else {
                (fallback_tx, fallback_ty, true, f64::from(mon_scale.max(1.0)))
            };

            let init_w = (456.0 * scale_val).round() as i32;
            let init_h = (210.0 * scale_val).round() as i32;
            if let Ok(mut guard) = app_ui.state::<Mutex<AppState>>().lock() {
                guard.result_anchor_below = placed_below;
                guard.result_anchor_y = Some(if placed_below { ty } else { ty + init_h });
            }

            let _ = set_window_bounds(&result_win, tx, ty, init_w, init_h);
            dispatch_window_event(&result_win, "result:update", &loading_state);
            show_result_window_foreground(&app_ui, &result_win, source_hwnd);
        }
    });

    spawn_text_retranslate_job(
        app.clone(),
        job_id,
        trimmed,
        "自动识别".to_string(),
        String::new(),
        target_lang,
        engine,
        config,
        client,
    );
    Ok(())
}

#[tauri::command(async)]
fn analyze_selected_text(app: AppHandle) -> Result<(), String> {
    present_selected_text_in_result(&app)
}

#[tauri::command(async)]
fn resize_result_window(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let target_w = width.clamp(320.0, 560.0);
    let target_h = height.clamp(140.0, 560.0);
    let (anchor_below, anchor_y) = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        (guard.result_anchor_below, guard.result_anchor_y)
    };
    let app_ui = app.clone();
    post_to_ui(&app, move || {
        let Some(win) = app_ui.get_webview_window("result") else {
            return;
        };
        let scale = win
            .current_monitor()
            .ok()
            .flatten()
            .map(|m| m.scale_factor())
            .unwrap_or(1.0)
            .max(1.0);
        let new_w = (target_w * scale).round() as i32;
        let new_h = (target_h * scale).round() as i32;
        let cur_pos = win.outer_position().unwrap_or(PhysicalPosition::new(100, 100));

        let (min_x, max_x, min_y, max_y) = if let Ok(Some(mon)) = win.current_monitor() {
            let wa = mon.work_area();
            let pad = (10.0 * scale).round() as i32;
            let lx = wa.position.x + pad;
            let rx = (wa.position.x + wa.size.width as i32 - new_w - pad).max(lx);
            let ty = wa.position.y + pad;
            let by = (wa.position.y + wa.size.height as i32 - new_h - pad).max(ty);
            (lx, rx, ty, by)
        } else {
            (cur_pos.x, cur_pos.x, cur_pos.y, cur_pos.y)
        };

        let next_x = cur_pos.x.clamp(min_x, max_x);
        let raw_y = match (anchor_below, anchor_y) {
            (false, Some(bottom_anchor)) => bottom_anchor - new_h,
            _ => cur_pos.y,
        };
        let next_y = raw_y.clamp(min_y, max_y);
        let _ = set_window_bounds(&win, next_x, next_y, new_w, new_h);
    });
    Ok(())
}

#[tauri::command(async)]
fn reopen_history_item(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    id: u64,
) -> Result<(), String> {
    let last_rect = app.state::<Mutex<AppState>>().lock()
        .map_err(|_| "状态锁异常".to_string())?.last_monitor_rect;
    let (mon_x, mon_y, mon_w, mon_h, mon_scale) = get_anchor_monitor_rect(&app, last_rect);

    let (vs, tx, ty) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let item = guard
            .history
            .iter()
            .find(|h| h.id == id)
            .cloned()
            .ok_or_else(|| "未找到该历史记录".to_string())?;

        guard.current_job_id += 1;
        guard.last_crop = None;
        guard.ask_messages.clear();
        guard.result_anchor_below = true;
        guard.result_anchor_y = None;
        guard.last_input_text = if item.original_text.trim().is_empty() {
            None
        } else {
            Some(item.original_text.clone())
        };
        guard.last_result = Some(item.clone());
        let vs = ResultViewState::Done { result: item, has_source_image: false };
        guard.latest_result_state = vs.clone();

        let sf = f64::from(mon_scale.max(1.0));
        let w_phys = (456.0 * sf).round() as i32;
        let h_phys = (210.0 * sf).round() as i32;
        let tx = mon_x + ((mon_w as i32 - w_phys) / 2).max(24);
        let ty = mon_y + ((mon_h as i32 - h_phys) / 2).max(24);
        (vs, tx, ty)
    };

    let app_ui = app.clone();
    post_to_ui(&app, move || {
        if let Some(result_win) = app_ui.get_webview_window("result") {
            let _ = result_win.set_size(LogicalSize::new(456.0, 210.0));
            let _ = result_win.set_position(PhysicalPosition::new(tx, ty));
            dispatch_window_event(&result_win, "result:update", &vs);
            show_result_window_foreground(&app_ui, &result_win, None);
        }
    });
    Ok(())
}

#[tauri::command]
async fn export_config_json(state: State<'_, Mutex<AppState>>) -> Result<bool, String> {
    let cfg = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard.config.clone()
    };
    let mut backup = serde_json::to_value(&cfg).map_err(|e| format!("序列化配置失败: {e}"))?;
    if let Some(fields) = backup.as_object_mut() {
        for secret in ["api_key", "google_api_key", "deepl_api_key"] {
            fields.remove(secret);
        }
        if proxy_has_credentials(&cfg.proxy_url) {
            fields.remove("proxy_url");
        }
    }
    let json = serde_json::to_string_pretty(&backup).map_err(|e| format!("序列化配置失败: {e}"))?;
    let file = rfd::AsyncFileDialog::new()
        .add_filter("JSON 配置文件", &["json"])
        .set_file_name("Luma_Config_Backup.json")
        .save_file()
        .await;
    if let Some(handle) = file {
        handle
            .write(json.as_bytes())
            .await
            .map_err(|e| format!("导出配置失败: {e}"))?;
        return Ok(true);
    }
    Ok(false)
}

#[tauri::command]
async fn import_config_json(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
) -> Result<Option<AppConfig>, String> {
    let file = rfd::AsyncFileDialog::new()
        .add_filter("JSON 配置文件", &["json"])
        .pick_file()
        .await;
    let Some(handle) = file else {
        return Ok(None);
    };
    let bytes = handle.read().await;
    let backup: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("配置文件格式无效，无法解析: {e}"))?;
    let mut parsed: AppConfig = serde_json::from_value(backup.clone())
        .map_err(|e| format!("配置文件格式无效，无法解析: {e}"))?;
    {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let current = &guard.config;
        if backup.get("api_key").is_none() && translate::same_ai_origin(&current.api_base_url, &parsed.api_base_url) {
            parsed.api_key = current.api_key.clone();
        }
        if backup.get("google_api_key").is_none() {
            parsed.google_api_key = current.google_api_key.clone();
        }
        if backup.get("deepl_api_key").is_none() {
            parsed.deepl_api_key = current.deepl_api_key.clone();
        }
        if backup.get("proxy_url").is_none() {
            parsed.proxy_url = current.proxy_url.clone();
        }
    }
    let applied = apply_and_persist_config(&app, &state, parsed)?;
    Ok(Some(applied))
}

// ==================== In-Place Overlay Analysis Commands ====================

fn spawn_overlay_ask_job(
    app: AppHandle,
    job_id: u64,
    crops: Vec<(String, CroppedRegion)>,
    thumbs: Vec<OverlayThumb>,
    messages: Vec<AiChatMessage>,
    config: AppConfig,
    client: Client,
) {
    tauri::async_runtime::spawn(async move {
        let images_b64: Vec<String> = crops
            .iter()
            .map(|(_, c)| encode_vision_optimized_base64(c))
            .collect();
        let stream_sink = make_throttled_stream_sink(
            app.clone(),
            "overlay",
            "overlay:stream",
            job_id,
            "ask",
            move |guard| guard.overlay_job_id == job_id,
        );
        let outcome =
            ask_ai_about_images(&client, &config, &images_b64, &messages, Some(&stream_sink)).await;

        let next_state = {
            let state = app.state::<Mutex<AppState>>();
            let Ok(mut guard) = state.lock() else {
                return;
            };
            if guard.overlay_job_id != job_id {
                return;
            }
            match outcome {
                Ok(answer) => {
                    guard.overlay_ask_messages.push(AiChatMessage {
                        role: "assistant".to_string(),
                        content: answer,
                    });
                    let st = OverlayAnalysisState::Ask {
                        job_id,
                        thumbs,
                        messages: guard.overlay_ask_messages.clone(),
                        loading: false,
                        error: None,
                    };
                    guard.overlay_analysis_state = st.clone();
                    st
                }
                Err(err_msg) => {
                    if guard
                        .overlay_ask_messages
                        .last()
                        .map(|m| m.role == "user")
                        .unwrap_or(false)
                    {
                        guard.overlay_ask_messages.pop();
                    }
                    let st = OverlayAnalysisState::Ask {
                        job_id,
                        thumbs,
                        messages: guard.overlay_ask_messages.clone(),
                        loading: false,
                        error: Some(err_msg),
                    };
                    guard.overlay_analysis_state = st.clone();
                    st
                }
            }
        };

        emit_overlay_analysis(&app, &next_state);
    });
}

fn spawn_overlay_batch_job(
    app: AppHandle,
    job_id: u64,
    mode: String,
    engine: String,
    target_lang: String,
    crops: Vec<(String, CroppedRegion)>,
    thumbs: Vec<OverlayThumb>,
    config: AppConfig,
    client: Client,
    emit_events: bool,
) -> tauri::async_runtime::JoinHandle<Option<OverlayAnalysisState>> {
    tauri::async_runtime::spawn(async move {
        let pairs: Vec<(String, String)> = crops
            .into_iter()
            .map(|(label, crop)| {
                let image = if mode == "ocr" || engine != "ai" {
                    crop.raw_base64
                } else {
                    encode_vision_optimized_base64(&crop)
                };
                (label, image)
            })
            .collect();

        if mode == "ocr" {
            let batch_sink = emit_events.then(|| {
                make_throttled_batch_stream_sink(
                    app.clone(),
                    "overlay",
                    "overlay:stream",
                    job_id,
                    "ocr",
                    move |guard| guard.overlay_job_id == job_id,
                )
            });
            let outcome = ocr_images_batch(&client, &config, &pairs, batch_sink.as_ref()).await;
            let (next_state, history_snapshot) = {
                let state = app.state::<Mutex<AppState>>();
                let Ok(mut guard) = state.lock() else {
                    return None;
                };
                if guard.overlay_job_id != job_id {
                    return None;
                }
                match outcome {
                    Ok((items, elapsed_ms, used_model)) => {
                        let mut snapshot = None;
                        if items.len() == 1 && !items[0].original_text.trim().is_empty() {
                            let res = TranslationResult {
                                id: 0,
                                mode: "ocr".to_string(),
                                engine: "ai".to_string(),
                                source_lang: items[0].source_lang.clone(),
                                target_lang: "纯文本识别".to_string(),
                                original_text: items[0].original_text.clone(),
                                translated_text: items[0].translated_text.clone(),
                                blocks: items[0].blocks.clone(),
                                elapsed_ms,
                                model: used_model.clone(),
                                image_data_url: thumbs
                                    .first()
                                    .map(|t| t.image_data_url.clone())
                                    .unwrap_or_default(),
                                created_at_ms: SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .map(|d| d.as_millis() as u64)
                                    .unwrap_or(0),
                            };
                            let (_, snap) = push_history_locked(&mut guard, res);
                            snapshot = Some(snap);
                        }
                        let st = OverlayAnalysisState::Ocr {
                            job_id,
                            thumbs,
                            loading: false,
                            items,
                            elapsed_ms,
                            model: used_model,
                            error: None,
                        };
                        guard.overlay_analysis_state = st.clone();
                        (st, snapshot)
                    }
                    Err(msg) => {
                        let st = OverlayAnalysisState::Ocr {
                            job_id,
                            thumbs,
                            loading: false,
                            items: Vec::new(),
                            elapsed_ms: 0,
                            model: config.model.clone(),
                            error: Some(msg),
                        };
                        guard.overlay_analysis_state = st.clone();
                        (st, None)
                    }
                }
            };
            if emit_events {
                emit_overlay_analysis(&app, &next_state);
            }
            if history_snapshot.is_some() {
                if let Err(err) = persist_history_and_notify(&app) { eprintln!("[Luma] {err}"); }
            }
            Some(next_state)
        } else {
            let batch_sink = (emit_events && engine == "ai").then(|| {
                make_throttled_batch_stream_sink(
                    app.clone(),
                    "overlay",
                    "overlay:stream",
                    job_id,
                    "translate",
                    move |guard| guard.overlay_job_id == job_id,
                )
            });
            let outcome = translate_images_batch(
                &client,
                &config,
                &pairs,
                &target_lang,
                &engine,
                batch_sink.as_ref(),
            )
            .await;
            let (next_state, history_snapshot) = {
                let state = app.state::<Mutex<AppState>>();
                let Ok(mut guard) = state.lock() else {
                    return None;
                };
                if guard.overlay_job_id != job_id {
                    return None;
                }
                match outcome {
                    Ok((items, elapsed_ms, model_name)) => {
                        let mut snapshot = None;
                        if items.len() == 1 {
                            let res = TranslationResult {
                                id: 0,
                                mode: "translate".to_string(),
                                engine: engine.clone(),
                                source_lang: items[0].source_lang.clone(),
                                target_lang: target_lang.clone(),
                                original_text: items[0].original_text.clone(),
                                translated_text: items[0].translated_text.clone(),
                                blocks: items[0].blocks.clone(),
                                elapsed_ms,
                                model: model_name.clone(),
                                image_data_url: thumbs
                                    .first()
                                    .map(|t| t.image_data_url.clone())
                                    .unwrap_or_default(),
                                created_at_ms: SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .map(|d| d.as_millis() as u64)
                                    .unwrap_or(0),
                            };
                            let (_, snap) = push_history_locked(&mut guard, res);
                            snapshot = Some(snap);
                        }
                        let st = OverlayAnalysisState::Translate {
                            job_id,
                            thumbs,
                            engine,
                            target_lang,
                            loading: false,
                            items,
                            elapsed_ms,
                            model: model_name,
                            error: None,
                        };
                        guard.overlay_analysis_state = st.clone();
                        (st, snapshot)
                    }
                    Err(msg) => {
                        let st = OverlayAnalysisState::Translate {
                            job_id,
                            thumbs,
                            engine,
                            target_lang,
                            loading: false,
                            items: Vec::new(),
                            elapsed_ms: 0,
                            model: config.model.clone(),
                            error: Some(msg),
                        };
                        guard.overlay_analysis_state = st.clone();
                        (st, None)
                    }
                }
            };
            if emit_events {
                emit_overlay_analysis(&app, &next_state);
            }
            if history_snapshot.is_some() {
                if let Err(err) = persist_history_and_notify(&app) { eprintln!("[Luma] {err}"); }
            }
            Some(next_state)
        }
    })
}

#[tauri::command]
async fn overlay_analyze_regions(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    regions: Vec<RegionInput>,
    mode: String,
    engine_override: Option<String>,
    target_lang_override: Option<String>,
    initial_prompt: Option<String>,
    capture_id: Option<u64>,
    wait_for_result: Option<bool>,
) -> Result<Option<(u64, Option<OverlayAnalysisState>)>, String> {
    let wait_for_result = wait_for_result.unwrap_or(false);
    if wait_for_result && (mode != "translate" || capture_id.is_none()) {
        return Err("原位翻译请求无效".into());
    }
    enum JobKind {
        None,
        Ask(Vec<AiChatMessage>),
        Batch { engine: String, target_lang: String },
    }

    let (capture, capture_id, expected_job_id) = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if capture_id.is_some_and(|id| id != guard.current_capture_id) {
            return Err("截图已经关闭".into());
        }
        let capture = guard.last_capture.clone().ok_or("截图已经关闭")?;
        (capture, guard.current_capture_id, guard.overlay_job_id)
    };
    let (crops, thumbs) = tauri::async_runtime::spawn_blocking(move || {
        let crops = resolve_regions_from_inputs(Some(&capture), &regions)?;
        let thumbs = build_overlay_thumbs(&crops);
        Ok::<_, String>((crops, thumbs))
    })
    .await
    .map_err(|err| format!("处理选区失败：{err}"))??;

    let (job_id, crops, thumbs, config, client, view_state, job_kind) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if guard.current_capture_id != capture_id || guard.last_capture.is_none()
            || guard.overlay_job_id != expected_job_id
        {
            return Err("截图或分析任务已切换，请重试。".into());
        }
        guard.overlay_crops = crops.clone();
        guard.overlay_ask_messages.clear();
        guard.overlay_job_id += 1;
        let job_id = guard.overlay_job_id;

        if mode == "ask" {
            let prompt = initial_prompt
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty());
            if let Some(q) = prompt {
                guard.overlay_ask_messages.push(AiChatMessage {
                    role: "user".to_string(),
                    content: q,
                });
                let msgs = guard.overlay_ask_messages.clone();
                let vs = OverlayAnalysisState::Ask {
                    job_id,
                    thumbs: thumbs.clone(),
                    messages: msgs.clone(),
                    loading: true,
                    error: None,
                };
                guard.overlay_analysis_state = vs.clone();
                (
                    job_id,
                    crops,
                    thumbs,
                    guard.config.clone(),
                    guard.client.clone(),
                    vs,
                    JobKind::Ask(msgs),
                )
            } else {
                let vs = OverlayAnalysisState::Ask {
                    job_id,
                    thumbs: thumbs.clone(),
                    messages: Vec::new(),
                    loading: false,
                    error: None,
                };
                guard.overlay_analysis_state = vs.clone();
                (
                    job_id,
                    crops,
                    thumbs,
                    guard.config.clone(),
                    guard.client.clone(),
                    vs,
                    JobKind::None,
                )
            }
        } else if mode == "ocr" {
            let vs = OverlayAnalysisState::Ocr {
                job_id,
                thumbs: thumbs.clone(),
                loading: true,
                items: Vec::new(),
                elapsed_ms: 0,
                model: guard.config.model.clone(),
                error: None,
            };
            guard.overlay_analysis_state = vs.clone();
            (
                job_id,
                crops,
                thumbs,
                guard.config.clone(),
                guard.client.clone(),
                vs,
                JobKind::Batch {
                    engine: "ai".to_string(),
                    target_lang: guard.config.target_lang.clone(),
                },
            )
        } else {
            let engine = engine_override.unwrap_or_else(|| guard.config.translate_engine.clone());
            let target_lang =
                target_lang_override.unwrap_or_else(|| guard.config.target_lang.clone());
            let vs = OverlayAnalysisState::Translate {
                job_id,
                thumbs: thumbs.clone(),
                engine: engine.clone(),
                target_lang: target_lang.clone(),
                loading: true,
                items: Vec::new(),
                elapsed_ms: 0,
                model: guard.config.model.clone(),
                error: None,
            };
            guard.overlay_analysis_state = vs.clone();
            (
                job_id,
                crops,
                thumbs,
                guard.config.clone(),
                guard.client.clone(),
                vs,
                JobKind::Batch {
                    engine,
                    target_lang,
                },
            )
        }
    };

    if !wait_for_result {
        emit_overlay_analysis(&app, &view_state);
    }

    match job_kind {
        JobKind::None => {}
        JobKind::Ask(msgs) => {
            spawn_overlay_ask_job(app, job_id, crops, thumbs, msgs, config, client);
        }
        JobKind::Batch {
            engine,
            target_lang,
        } => {
            let job = spawn_overlay_batch_job(
                app,
                job_id,
                mode,
                engine,
                target_lang,
                crops,
                thumbs,
                config,
                client,
                !wait_for_result,
            );
            if wait_for_result {
                let result = job.await.map_err(|err| format!("翻译任务失败：{err}"))?;
                return Ok(Some((job_id, result)));
            }
        }
    }

    Ok(None)
}

#[tauri::command(async)]
fn overlay_switch_mode(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    mode: String,
    engine_override: Option<String>,
    target_lang_override: Option<String>,
    regions: Option<Vec<RegionInput>>,
) -> Result<(), String> {
    let (job_id, crops, thumbs, config, client, view_state, run_batch) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if let Some(latest_regions) = regions.filter(|r| !r.is_empty()) {
            let resolved = resolve_regions_from_inputs(guard.last_capture.as_deref(), &latest_regions)?;
            guard.overlay_crops = resolved;
        }
        if guard.overlay_crops.is_empty() {
            return Err("当前没有选中的截图区域".to_string());
        }
        let crops = guard.overlay_crops.clone();
        let thumbs = build_overlay_thumbs(&crops);
        guard.overlay_job_id += 1;
        let job_id = guard.overlay_job_id;

        if mode == "ask" {
            let vs = OverlayAnalysisState::Ask {
                job_id,
                thumbs: thumbs.clone(),
                messages: guard.overlay_ask_messages.clone(),
                loading: false,
                error: None,
            };
            guard.overlay_analysis_state = vs.clone();
            (
                job_id,
                crops,
                thumbs,
                guard.config.clone(),
                guard.client.clone(),
                vs,
                None,
            )
        } else if mode == "ocr" {
            let vs = OverlayAnalysisState::Ocr {
                job_id,
                thumbs: thumbs.clone(),
                loading: true,
                items: Vec::new(),
                elapsed_ms: 0,
                model: guard.config.model.clone(),
                error: None,
            };
            guard.overlay_analysis_state = vs.clone();
            (
                job_id,
                crops,
                thumbs,
                guard.config.clone(),
                guard.client.clone(),
                vs,
                Some(("ai".to_string(), guard.config.target_lang.clone())),
            )
        } else {
            let engine = engine_override.unwrap_or_else(|| guard.config.translate_engine.clone());
            let target_lang =
                target_lang_override.unwrap_or_else(|| guard.config.target_lang.clone());
            let vs = OverlayAnalysisState::Translate {
                job_id,
                thumbs: thumbs.clone(),
                engine: engine.clone(),
                target_lang: target_lang.clone(),
                loading: true,
                items: Vec::new(),
                elapsed_ms: 0,
                model: guard.config.model.clone(),
                error: None,
            };
            guard.overlay_analysis_state = vs.clone();
            (
                job_id,
                crops,
                thumbs,
                guard.config.clone(),
                guard.client.clone(),
                vs,
                Some((engine, target_lang)),
            )
        }
    };

    emit_overlay_analysis(&app, &view_state);
    if let Some((engine, target_lang)) = run_batch {
        spawn_overlay_batch_job(
            app,
            job_id,
            mode,
            engine,
            target_lang,
            crops,
            thumbs,
            config,
            client,
            true,
        );
    }
    Ok(())
}

#[tauri::command(async)]
fn overlay_ask_follow_up(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    question: String,
    capture_id: u64,
    regions: Vec<RegionInput>,
) -> Result<(), String> {
    let trimmed = question.trim().to_string();
    if trimmed.is_empty() {
        return Err("请输入想要询问的问题".to_string());
    }
    let (capture, expected_job) = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if guard.current_capture_id != capture_id {
            return Err("截图已经关闭".to_string());
        }
        (guard.last_capture.clone().ok_or("截图已经关闭")?, guard.overlay_job_id)
    };
    let crops = resolve_regions_from_inputs(Some(&capture), &regions)?;
    let thumbs = build_overlay_thumbs(&crops);
    let (job_id, msgs, config, client, vs) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if guard.current_capture_id != capture_id || guard.last_capture.is_none()
            || guard.overlay_job_id != expected_job {
            return Err("选区已变更，请重新发送问题".to_string());
        }
        let same_images = crops.len() == guard.overlay_crops.len() && crops.iter()
            .zip(&guard.overlay_crops)
            .all(|((_, crop), (_, previous))| crop.png_bytes == previous.png_bytes);
        if !same_images {
            guard.overlay_ask_messages.clear();
        }
        guard.overlay_crops = crops.clone();
        guard.overlay_job_id += 1;
        let job_id = guard.overlay_job_id;
        guard.overlay_ask_messages.push(AiChatMessage {
            role: "user".to_string(),
            content: trimmed,
        });
        let msgs = guard.overlay_ask_messages.clone();
        let vs = OverlayAnalysisState::Ask {
            job_id,
            thumbs: thumbs.clone(),
            messages: msgs.clone(),
            loading: true,
            error: None,
        };
        guard.overlay_analysis_state = vs.clone();
        (
            job_id,
            msgs,
            guard.config.clone(),
            guard.client.clone(),
            vs,
        )
    };

    emit_overlay_analysis(&app, &vs);
    spawn_overlay_ask_job(app, job_id, crops, thumbs, msgs, config, client);
    Ok(())
}

#[tauri::command]
fn overlay_clear_analysis(app: AppHandle, state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    if let Ok(mut guard) = state.lock() {
        guard.overlay_job_id += 1;
        guard.overlay_crops.clear();
        guard.overlay_ask_messages.clear();
        guard.overlay_analysis_state = OverlayAnalysisState::Idle;
    }
    emit_overlay_analysis(&app, &OverlayAnalysisState::Idle);
    Ok(())
}

#[tauri::command(async)]
fn overlay_pin_current(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    regions: Vec<RegionInput>,
) -> Result<(), String> {
    let (crops_with_pos, capture_id) = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let resolved = if !regions.is_empty() {
            resolve_regions_from_inputs(guard.last_capture.as_deref(), &regions)?
        } else if !guard.overlay_crops.is_empty() {
            guard.overlay_crops.clone()
        } else {
            return Err("没有可贴图的选区".to_string());
        };

        let mut list = Vec::with_capacity(resolved.len());
        for (idx, (_label, crop)) in resolved.into_iter().enumerate() {
            let pos = if let (Some(cap), Some(reg)) = (guard.last_capture.as_deref(), regions.get(idx))
            {
                let scale = f64::from(crop.scale_factor.max(1.0));
                let lw = (crop.pixel_width as f64 / scale).clamp(180.0, 920.0) + 12.0;
                let lh = (crop.pixel_height as f64 / scale).clamp(120.0, 720.0) + 12.0;
                Some(compute_popup_position(
                    cap, reg.rx, reg.ry, reg.rw, reg.rh, lw, lh, true,
                ))
            } else {
                None
            };
            list.push((crop, pos));
        }

        (list, guard.current_capture_id)
    };
    for (idx, (crop, pos)) in crops_with_pos.into_iter().enumerate() {
        create_pin_window(&app, crop, pos, idx)?;
    }
    finish_selection_output(&app, &state, capture_id)
}

// ==================== Apple-Style Floating Stack Commands ====================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StageRegionsOutcome {
    added: usize,
    count: usize,
    max_size: usize,
    latest_thumb: String,
}

#[tauri::command(async)]
async fn stage_regions(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    capture_id: u64,
    regions: Vec<RegionInput>,
) -> Result<StageRegionsOutcome, String> {
    if regions.is_empty() {
        return Err("没有可加入叠堆的选区".into());
    }
    let check_capacity = |guard: &AppState| {
        let max_size = guard.config.max_stack_size.clamp(2, 12);
        if guard.staged_stack.len().saturating_add(regions.len()) > max_size {
            return Err(format!(
                "叠堆空间不足（已有 {}/{} 张，无法加入 {} 张）",
                guard.staged_stack.len(), max_size, regions.len()
            ));
        }
        Ok(())
    };
    let capture = active_capture(&state, capture_id)?;
    check_capacity(&*state.lock().map_err(|_| "状态锁异常".to_string())?)?;
    let crops = tauri::async_runtime::spawn_blocking({
        let regions = regions.clone();
        move || -> Result<_, String> {
            Ok(resolve_regions_from_inputs(Some(&capture), &regions)?
                .into_iter()
                .map(|(label, crop)| {
                    let thumb = crop.make_thumbnail_data_url();
                    (label, crop, thumb)
                })
                .collect::<Vec<_>>())
        }
    }).await.map_err(|err| err.to_string())??;

    let added = crops.len();
    let (stack_state, was_empty, outcome) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if guard.current_capture_id != capture_id || guard.last_capture.is_none() {
            return Err("截图已经关闭".to_string());
        }
        check_capacity(&guard)?;
        let was_empty = guard.staged_stack.is_empty();
        for (label, crop, thumb_data_url) in crops {
            guard.next_stack_id += 1;
            let id = guard.next_stack_id;
            guard.staged_stack.push(StagedImage {
                id,
                title: label,
                crop,
                thumb_data_url,
            });
        }

        reset_stack_analysis(&mut guard);
        let stack_state = build_stack_view_state(&guard);
        let max_size = guard.config.max_stack_size.clamp(2, 12);
        let latest_thumb = guard
            .staged_stack
            .last()
            .map(|s| s.thumb_data_url.clone())
            .unwrap_or_default();
        let outcome = StageRegionsOutcome {
            added,
            count: guard.staged_stack.len(),
            max_size,
            latest_thumb,
        };
        (stack_state, was_empty, outcome)
    };

    // Pre-render the floating stack window behind the capture overlay so closing capture reveals it smoothly.
    let ui = app.clone();
    post_to_ui(&app, move || {
        if let Some(stack) = ui.get_webview_window("stack") {
            if was_empty {
                position_stack_bottom_right(&ui, &stack);
            }
            show_stack_behind_capture(&ui, &stack);
            dispatch_window_event(&stack, "stack:updated", &stack_state);
        }
    });
    Ok(outcome)
}

fn signal_stack_frame(state: &Mutex<AppState>, capture_id: u64) {
    if let Ok(mut guard) = state.lock() {
        if guard.stack_frame_waiter.as_ref().is_some_and(|(id, _)| *id == capture_id) {
            if let Some((_, ready)) = guard.stack_frame_waiter.take() {
                let _ = ready.send(());
            }
        }
    }
}

#[tauri::command]
fn stack_frame_ready(window: tauri::WebviewWindow, state: State<'_, Mutex<AppState>>, capture_id: u64) {
    if window.label() == "stack" {
        signal_stack_frame(&state, capture_id);
    }
}

#[tauri::command]
fn finish_stack_handoff(app: AppHandle, state: State<'_, Mutex<AppState>>, capture_id: u64) -> Result<(), String> {
    finish_selection_output(&app, &state, capture_id)
}

#[tauri::command]
fn get_stack_state(state: State<'_, Mutex<AppState>>) -> Result<StackViewState, String> {
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    Ok(build_stack_view_state(&guard))
}

fn publish_stack_change(app: &AppHandle, stack_state: &StackViewState) {
    emit_stack_update(app, stack_state);
    if stack_state.items.is_empty() {
        let app_ui = app.clone();
        post_to_ui(app, move || {
            let still_empty = app_ui.state::<Mutex<AppState>>().lock()
                .map(|guard| guard.staged_stack.is_empty()).unwrap_or(false);
            if still_empty {
                if let Some(win) = app_ui.get_webview_window("stack") {
                    let _ = win.hide();
                }
            }
        });
    }
}

fn remove_exported_items(app: &AppHandle, state: &Mutex<AppState>, ids: &[u64]) -> Result<(), String> {
    let stack_state = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let previous_count = guard.staged_stack.len();
        guard.staged_stack.retain(|item| !ids.contains(&item.id));
        if guard.staged_stack.len() != previous_count {
            reset_stack_analysis(&mut guard);
        }
        build_stack_view_state(&guard)
    };
    publish_stack_change(app, &stack_state);
    Ok(())
}

#[tauri::command]
fn remove_stack_item(app: AppHandle, state: State<'_, Mutex<AppState>>, id: u64) -> Result<(), String> {
    remove_exported_items(&app, &state, &[id])
}

#[tauri::command]
fn reorder_stack_items(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    from_index: usize,
    to_index: usize,
) -> Result<(), String> {
    let stack_state = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let len = guard.staged_stack.len();
        if from_index >= len || to_index >= len {
            return Err("排序索引超出范围".to_string());
        }
        if from_index != to_index {
            let item = guard.staged_stack.remove(from_index);
            guard.staged_stack.insert(to_index, item);
            if guard.stack_batch_items.len() == len {
                let b = guard.stack_batch_items.remove(from_index);
                guard.stack_batch_items.insert(to_index, b);
                for (i, it) in guard.stack_batch_items.iter_mut().enumerate() {
                    it.index = i;
                }
            } else {
                reset_stack_analysis(&mut guard);
            }
        }
        build_stack_view_state(&guard)
    };
    publish_stack_change(&app, &stack_state);
    Ok(())
}

#[tauri::command(async)]
fn scan_qr_selection(
    state: State<'_, Mutex<AppState>>,
    capture_id: u64,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
    annotated_base64: Option<String>,
) -> Result<Vec<String>, String> {
    let capture = active_capture(&state, capture_id)?;
    let crop = resolve_crop_from_input(Some(&capture), rx, ry, rw, rh, annotated_base64)?;
    qr::decode_qr_from_png(&crop.png_bytes)
}

#[tauri::command(async)]
fn scan_qr_base64(image_base64: String) -> Result<Vec<String>, String> {
    let b64 = image_base64
        .split_once(',')
        .map(|(_, rest)| rest)
        .unwrap_or(image_base64.as_str());
    use base64::Engine as _;
    let png_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("解码图片数据失败: {e}"))?;
    qr::decode_qr_from_png(&png_bytes)
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err("仅支持打开 http:// 或 https:// 链接".to_string());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", trimmed])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("打开链接失败: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("/usr/bin/open")
            .arg(trimmed)
            .spawn()
            .map_err(|e| format!("打开链接失败: {e}"))?;
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = trimmed;
        Err("当前平台不支持打开外部链接".to_string())
    }
}

#[tauri::command]
async fn save_markdown_note(
    app: AppHandle,
    content: String,
    default_name: String,
) -> Result<bool, String> {
    let app_ui = app.clone();
    let filename = if default_name.trim().is_empty() {
        "Luma_Note.md".to_string()
    } else if default_name.ends_with(".md") {
        default_name
    } else {
        format!("{default_name}.md")
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    post_to_ui(&app, move || {
        let mut dialog = rfd::FileDialog::new()
            .set_file_name(&filename)
            .add_filter("Markdown 笔记", &["md"]);
        if let Some(win) = app_ui.get_webview_window("stack").or_else(|| app_ui.get_webview_window("main")) {
            dialog = dialog.set_parent(&win);
        }
        let picked = dialog.save_file();
        let _ = tx.send(picked);
    });
    let Some(path) = rx.await.map_err(|_| "保存对话框已取消".to_string())? else {
        return Ok(false);
    };
    std::fs::write(&path, content.as_bytes()).map_err(|e| format!("写入 Markdown 文件失败: {e}"))?;
    Ok(true)
}

#[tauri::command]
async fn start_scroll_capture(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    capture_id: u64,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
) -> Result<(), String> {
    let (mon_x, mon_y, mon_w, mon_h, scale_factor) = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if guard.current_capture_id != capture_id || guard.last_capture.is_none() {
            return Err("截图已关闭".to_string());
        }
        guard
            .last_monitor_rect
            .ok_or_else(|| "缺少显示器坐标信息".to_string())?
    };

    // Hide overlay so the underlying scrollable window is exposed and interactive
    let _ = finish_selection_output(&app, &state, capture_id);
    tokio::time::sleep(Duration::from_millis(160)).await;

    let crop_x = ((rx.clamp(0.0, 1.0) * mon_w as f64).round() as u32).min(mon_w.saturating_sub(1));
    let crop_y = ((ry.clamp(0.0, 1.0) * mon_h as f64).round() as u32).min(mon_h.saturating_sub(1));
    let crop_w = ((rw.clamp(0.0, 1.0) * mon_w as f64).round() as u32).max(24).min(mon_w - crop_x);
    let crop_h = ((rh.clamp(0.0, 1.0) * mon_h as f64).round() as u32).max(24).min(mon_h - crop_y);
    let center_phys_x = mon_x + (crop_x + crop_w / 2) as i32;
    let center_phys_y = mon_y + (crop_y + crop_h / 2) as i32;

    let (stitched_img, png_bytes) = tauri::async_runtime::spawn_blocking(move || -> Result<(image::RgbaImage, Vec<u8>), String> {
        let monitors = xcap::Monitor::all().map_err(|e| format!("枚举显示器失败: {e}"))?;
        let target_mon = monitors
            .into_iter()
            .find(|m| {
                #[cfg(target_os = "macos")]
                let sf = f64::from(m.scale_factor().unwrap_or(1.0).max(1.0));
                #[cfg(not(target_os = "macos"))]
                let sf = 1.0_f64;
                let mx = (f64::from(m.x().unwrap_or(0)) * sf).round() as i32;
                let my = (f64::from(m.y().unwrap_or(0)) * sf).round() as i32;
                mx == mon_x && my == mon_y
            })
            .ok_or_else(|| "未找到目标显示器".to_string())?;

        let mut frames: Vec<image::RgbaImage> = Vec::new();
        for step in 0..20 {
            #[cfg(any(windows, target_os = "macos"))]
            if step > 0 && scroll_capture::is_escape_pressed() {
                break;
            }
            let full = target_mon.capture_image().map_err(|e| format!("抓取长截图帧失败: {e}"))?;
            let img_w = full.width().max(1);
            let img_h = full.height().max(1);
            let sx = ((rx.clamp(0.0, 1.0) * img_w as f64).round() as u32).min(img_w.saturating_sub(1));
            let sy = ((ry.clamp(0.0, 1.0) * img_h as f64).round() as u32).min(img_h.saturating_sub(1));
            let sw = ((rw.clamp(0.0, 1.0) * img_w as f64).round() as u32)
                .max(24)
                .min(img_w.saturating_sub(sx).max(1));
            let sh = ((rh.clamp(0.0, 1.0) * img_h as f64).round() as u32)
                .max(24)
                .min(img_h.saturating_sub(sy).max(1));
            let sub = image::imageops::crop_imm(&full, sx, sy, sw, sh).to_image();
            let done = frames
                .last()
                .map(|prev| scroll_capture::frames_nearly_identical(prev, &sub))
                .unwrap_or(false);
            if done {
                break;
            }
            frames.push(sub);
            if step == 19 {
                break;
            }
            #[cfg(any(windows, target_os = "macos"))]
            {
                scroll_capture::send_scroll_wheel_at(center_phys_x, center_phys_y, -360);
            }
            std::thread::sleep(Duration::from_millis(260));
        }

        scroll_capture::stitch_vertical_frames(&frames)
    })
    .await
    .map_err(|e| e.to_string())??;

    let _ = copy_png_bytes_to_clipboard(&png_bytes);
    let (sw, sh) = stitched_img.dimensions();
    let crop = cropped_region_from_rgba(sw, sh, stitched_img.into_raw(), scale_factor)?;
    let thumb_data_url = crop.make_thumbnail_data_url();

    let (stack_state, was_empty) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let was_empty = guard.staged_stack.is_empty();
        let max_size = guard.config.max_stack_size.clamp(2, 12);
        if guard.staged_stack.len() >= max_size {
            guard.staged_stack.remove(0);
        }
        guard.next_stack_id += 1;
        let id = guard.next_stack_id;
        guard.staged_stack.push(StagedImage {
            id,
            title: "长截图".to_string(),
            crop,
            thumb_data_url,
        });
        reset_stack_analysis(&mut guard);
        (build_stack_view_state(&guard), was_empty)
    };

    let ui = app.clone();
    post_to_ui(&app, move || {
        if let Some(stack) = ui.get_webview_window("stack") {
            if was_empty {
                position_stack_bottom_right(&ui, &stack);
            }
            let _ = stack.show();
            let _ = stack.set_focus();
            dispatch_window_event(&stack, "stack:updated", &stack_state);
        }
    });

    Ok(())
}

#[tauri::command]
fn clear_stack(app: AppHandle, state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    let stack_state = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        clear_stack_state_locked(&mut guard);
        build_stack_view_state(&guard)
    };
    publish_stack_change(&app, &stack_state);
    Ok(())
}

#[tauri::command(async)]
fn pin_stack_item(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    id: u64,
) -> Result<(), String> {
    let crop = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard
            .staged_stack
            .iter()
            .find(|item| item.id == id)
            .map(|item| item.crop.clone())
            .ok_or_else(|| "未找到对应的暂存截图".to_string())?
    };
    create_pin_window(&app, crop, None, 0)?;
    Ok(())
}

#[tauri::command]
fn get_stack_item_image(
    state: State<'_, Mutex<AppState>>,
    id: u64,
) -> Result<StackItemImageDto, String> {
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    let item = guard
        .staged_stack
        .iter()
        .find(|it| it.id == id)
        .ok_or_else(|| "未找到对应的暂存截图".to_string())?;
    Ok(StackItemImageDto {
        id: item.id,
        title: item.title.clone(),
        image_data_url: item.crop.data_url(),
        pixel_width: item.crop.pixel_width,
        pixel_height: item.crop.pixel_height,
        scale_factor: item.crop.scale_factor,
    })
}

#[tauri::command(async)]
fn update_stack_item_image(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    id: u64,
    image_base64: String,
) -> Result<StackItemImageDto, String> {
    let existing_scale = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard
            .staged_stack
            .iter()
            .find(|it| it.id == id)
            .map(|it| it.crop.scale_factor)
            .ok_or_else(|| "未找到对应的暂存截图".to_string())?
    };

    let updated_crop = decode_crop_from_base64(&image_base64, existing_scale)?;
    let thumb = updated_crop.make_thumbnail_data_url();

    let (dto, stack_state) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let item = guard
            .staged_stack
            .iter_mut()
            .find(|it| it.id == id)
            .ok_or_else(|| "未找到对应的暂存截图".to_string())?;
        item.crop = updated_crop;
        item.thumb_data_url = thumb;
        let dto = StackItemImageDto {
            id: item.id,
            title: item.title.clone(),
            image_data_url: item.crop.data_url(),
            pixel_width: item.crop.pixel_width,
            pixel_height: item.crop.pixel_height,
            scale_factor: item.crop.scale_factor,
        };
        reset_stack_analysis(&mut guard);
        let ss = build_stack_view_state(&guard);
        (dto, ss)
    };

    emit_stack_update(&app, &stack_state);
    Ok(dto)
}

#[tauri::command(async)]
fn copy_stack_item_image(
    state: State<'_, Mutex<AppState>>,
    id: u64,
) -> Result<(), String> {
    let png_bytes = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard
            .staged_stack
            .iter()
            .find(|it| it.id == id)
            .map(|it| it.crop.png_bytes.clone())
            .ok_or_else(|| "未找到对应的暂存截图".to_string())?
    };
    copy_png_bytes_to_clipboard(&png_bytes)
}

#[tauri::command]
async fn save_stack_item_image(
    state: State<'_, Mutex<AppState>>,
    id: u64,
) -> Result<bool, String> {
    let png_bytes = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard
            .staged_stack
            .iter()
            .find(|it| it.id == id)
            .map(|it| it.crop.png_bytes.clone())
            .ok_or_else(|| "未找到对应的暂存截图".to_string())?
    };
    save_png_with_dialog(png_bytes, "Luma_Stack").await
}

#[tauri::command(async)]
fn copy_stitched_stack(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    direction: String,
) -> Result<usize, String> {
    let (crops, ids, clear_after) = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if guard.staged_stack.is_empty() {
            return Err("叠堆中没有可拼接的图片".to_string());
        }
        let list: Vec<CroppedRegion> = guard
            .staged_stack
            .iter()
            .map(|item| item.crop.clone())
            .collect();
        (list, guard.staged_stack.iter().map(|item| item.id).collect::<Vec<_>>(), guard.config.clear_stack_after_output)
    };

    let count = crops.len();
    let stitched = stitch_regions(&crops, &direction)?;
    copy_png_bytes_to_clipboard(&stitched.png_bytes)?;

    if clear_after {
        remove_exported_items(&app, &state, &ids)?;
    }

    Ok(count)
}

#[tauri::command]
async fn save_stitched_stack(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    direction: String,
) -> Result<bool, String> {
    let (crops, ids, clear_after) = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if guard.staged_stack.is_empty() {
            return Err("叠堆中没有可拼接的图片".to_string());
        }
        let list: Vec<CroppedRegion> = guard
            .staged_stack
            .iter()
            .map(|item| item.crop.clone())
            .collect();
        (list, guard.staged_stack.iter().map(|item| item.id).collect::<Vec<_>>(), guard.config.clear_stack_after_output)
    };

    let stitched = stitch_regions(&crops, &direction)?;
    let saved = save_png_with_dialog(stitched.png_bytes, "Luma_Stitch").await?;

    if saved && clear_after {
        remove_exported_items(&app, &state, &ids)?;
    }

    Ok(saved)
}

fn write_temp_stack_files(items: &[Vec<u8>], prefix: &str) -> Result<Vec<std::path::PathBuf>, String> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir()
        .join("LumaStack")
        .join(format!("{prefix}-{}-{ts}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时导出目录失败: {e}"))?;

    let mut paths = Vec::with_capacity(items.len());
    for (idx, png_bytes) in items.iter().enumerate() {
        let file_path = dir.join(format!("Luma_{ts}_{:02}.png", idx + 1));
        std::fs::write(&file_path, png_bytes)
            .map_err(|e| format!("写入临时图片文件失败: {e}"))?;
        paths.push(file_path);
    }
    Ok(paths)
}

#[tauri::command(async)]
fn copy_stack_as_files(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
) -> Result<usize, String> {
    let (items, ids, clear_after) = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if guard.staged_stack.is_empty() {
            return Err("叠堆中没有可复制的图片".to_string());
        }
        let list: Vec<Vec<u8>> = guard
            .staged_stack
            .iter()
            .map(|item| item.crop.png_bytes.clone())
            .collect();
        (list, guard.staged_stack.iter().map(|item| item.id).collect::<Vec<_>>(), guard.config.clear_stack_after_output)
    };

    let paths = write_temp_stack_files(&items, "clip")?;
    let path_strings: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    #[cfg(windows)]
    {
        use clipboard_win::Setter;
        let _clip = clipboard_win::Clipboard::new_attempts(10)
            .map_err(|e| format!("打开系统剪贴板失败: {e}"))?;
        // FileList::write_clipboard uses NoClear internally, so empty the clipboard first
        // to ensure stale CF_BITMAP/CF_UNICODETEXT formats don't block CF_HDROP paste in WeChat/Explorer.
        clipboard_win::raw::empty()
            .map_err(|e| format!("清空系统剪贴板失败: {e}"))?;
        clipboard_win::formats::FileList
            .write_clipboard(path_strings.as_slice())
            .map_err(|e| format!("写入系统多文件剪贴板 (CF_HDROP) 失败: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        macos_bridge::copy_files_to_pasteboard(&path_strings)?;
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        return Err("多文件剪贴板仅支持 Windows / macOS".to_string());
    }

    let count = path_strings.len();
    if clear_after {
        remove_exported_items(&app, &state, &ids)?;
    }

    Ok(count)
}

#[tauri::command(async)]
fn stack_ask_ai(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    question: String,
) -> Result<(), String> {
    let trimmed = question.trim().to_string();
    if trimmed.is_empty() {
        return Err("请输入想要询问的问题".to_string());
    }

    let (job_id, crops, msgs, config, client, stack_state) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if guard.staged_stack.is_empty() {
            return Err("叠堆中没有可分析的截图".to_string());
        }
        if guard.stack_ask_loading {
            return Err("上一条问题仍在处理中".to_string());
        }
        guard.stack_ask_job_id += 1;
        let job_id = guard.stack_ask_job_id;
        guard.stack_ask_messages.push(AiChatMessage {
            role: "user".to_string(),
            content: trimmed,
        });
        guard.stack_ask_loading = true;
        guard.stack_ask_error = None;
        let crops: Vec<CroppedRegion> = guard
            .staged_stack
            .iter()
            .map(|item| item.crop.clone())
            .collect();
        let msgs = guard.stack_ask_messages.clone();
        let ss = build_stack_view_state(&guard);
        (
            job_id,
            crops,
            msgs,
            guard.config.clone(),
            guard.client.clone(),
            ss,
        )
    };

    emit_stack_update(&app, &stack_state);

    tauri::async_runtime::spawn(async move {
        let images_b64: Vec<String> = crops.iter().map(encode_vision_optimized_base64).collect();
        let stream_sink = make_throttled_stream_sink(
            app.clone(),
            "stack",
            "stack:stream",
            job_id,
            "ask",
            move |guard| guard.stack_ask_job_id == job_id,
        );
        let outcome =
            ask_ai_about_images(&client, &config, &images_b64, &msgs, Some(&stream_sink)).await;

        let next_state = {
            let state = app.state::<Mutex<AppState>>();
            let Ok(mut guard) = state.lock() else {
                return;
            };
            if guard.stack_ask_job_id != job_id {
                return;
            }
            guard.stack_ask_loading = false;
            match outcome {
                Ok(answer) => {
                    guard.stack_ask_messages.push(AiChatMessage {
                        role: "assistant".to_string(),
                        content: answer,
                    });
                    guard.stack_ask_error = None;
                }
                Err(msg) => {
                    if guard
                        .stack_ask_messages
                        .last()
                        .map(|m| m.role == "user")
                        .unwrap_or(false)
                    {
                        guard.stack_ask_messages.pop();
                    }
                    guard.stack_ask_error = Some(msg);
                }
            }
            build_stack_view_state(&guard)
        };
        emit_stack_update(&app, &next_state);
    });

    Ok(())
}

#[tauri::command(async)]
fn stack_run_batch(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    mode: String,
    engine_override: Option<String>,
    target_lang_override: Option<String>,
) -> Result<(), String> {
    let (job_id, pairs, engine, target_lang, config, client, stack_state) = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        if guard.staged_stack.is_empty() {
            return Err("叠堆中没有可处理的截图".to_string());
        }
        guard.stack_batch_job_id += 1;
        let job_id = guard.stack_batch_job_id;
        guard.stack_batch_mode = mode.clone();
        guard.stack_batch_loading = true;
        guard.stack_batch_error = None;
        guard.stack_batch_items.clear();

        let engine = engine_override.unwrap_or_else(|| guard.config.translate_engine.clone());
        let target_lang =
            target_lang_override.unwrap_or_else(|| guard.config.target_lang.clone());
        let pairs: Vec<(String, CroppedRegion)> = guard
            .staged_stack
            .iter()
            .enumerate()
            .map(|(i, item)| (format!("图 {} · {}", i + 1, item.title), item.crop.clone()))
            .collect();
        let ss = build_stack_view_state(&guard);
        (
            job_id,
            pairs,
            engine,
            target_lang,
            guard.config.clone(),
            guard.client.clone(),
            ss,
        )
    };

    emit_stack_update(&app, &stack_state);

    tauri::async_runtime::spawn(async move {
        let encoded: Vec<(String, String)> = pairs
            .into_iter()
            .map(|(title, crop)| {
                let image = if mode == "ocr" || engine != "ai" {
                    crop.raw_base64
                } else {
                    encode_vision_optimized_base64(&crop)
                };
                (title, image)
            })
            .collect();

        let outcome = if mode == "ocr" {
            let batch_sink = make_throttled_batch_stream_sink(
                app.clone(),
                "stack",
                "stack:stream",
                job_id,
                "ocr",
                move |guard| guard.stack_batch_job_id == job_id,
            );
            ocr_images_batch(&client, &config, &encoded, Some(&batch_sink))
                .await
                .map(|(items, elapsed, _)| (items, elapsed))
        } else {
            let batch_sink = (engine == "ai").then(|| {
                make_throttled_batch_stream_sink(
                    app.clone(),
                    "stack",
                    "stack:stream",
                    job_id,
                    "translate",
                    move |guard| guard.stack_batch_job_id == job_id,
                )
            });
            translate_images_batch(
                &client,
                &config,
                &encoded,
                &target_lang,
                &engine,
                batch_sink.as_ref(),
            )
            .await
            .map(|(items, elapsed, _)| (items, elapsed))
        };

        let next_state = {
            let state = app.state::<Mutex<AppState>>();
            let Ok(mut guard) = state.lock() else {
                return;
            };
            if guard.stack_batch_job_id != job_id {
                return;
            }
            guard.stack_batch_loading = false;
            match outcome {
                Ok((items, elapsed)) => {
                    guard.stack_batch_items = items;
                    guard.stack_batch_elapsed_ms = elapsed;
                    guard.stack_batch_error = None;
                }
                Err(msg) => {
                    guard.stack_batch_error = Some(msg);
                }
            }
            build_stack_view_state(&guard)
        };
        emit_stack_update(&app, &next_state);
    });

    Ok(())
}

#[tauri::command]
fn stack_clear_chat(app: AppHandle, state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    let stack_state = {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard.stack_ask_job_id += 1;
        guard.stack_ask_messages.clear();
        guard.stack_ask_loading = false;
        guard.stack_ask_error = None;
        build_stack_view_state(&guard)
    };
    emit_stack_update(&app, &stack_state);
    Ok(())
}

#[tauri::command(async)]
fn resize_stack_window(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    let Some(stack_win) = app.get_webview_window("stack") else {
        return Ok(());
    };

    #[cfg(not(windows))]
    let bounds = stack_resize_bounds(&app, &stack_win, width, height);
    #[cfg(windows)]
    let ui = app.clone();
    // Acknowledge the region change before mounting the incoming view.
    let timeout = Duration::from_millis(500);
    let deadline = std::time::Instant::now() + timeout;
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        if std::time::Instant::now() < deadline {
            #[cfg(windows)]
            let result = keep_stack_region_on_screen(&stack_win, (width, height))
                .map_err(|err| err.to_string())
                .and_then(|()| set_stack_region(&stack_win, (width, height)))
                .map(|()| {
                    if let Ok(mut state) = ui.state::<Mutex<AppState>>().lock() {
                        state.stack_extent = (width, height);
                    }
                });
            #[cfg(not(windows))]
            let result = set_window_bounds(&stack_win, bounds.0, bounds.1, bounds.2, bounds.3)
                .map_err(|err| err.to_string());
            let _ = tx.send(result);
        }
    })
    .map_err(|err| err.to_string())?;
    rx.recv_timeout(timeout)
        .map_err(|_| "叠堆窗口调整超时".to_string())?
}

#[cfg(not(windows))]
fn stack_resize_bounds(app: &AppHandle, stack_win: &tauri::WebviewWindow, width: f64, height: f64) -> (i32, i32, i32, i32) {
    let last_rect = app
        .state::<Mutex<AppState>>()
        .lock()
        .ok()
        .and_then(|g| g.last_monitor_rect);

    let (mon_x, mon_y, mon_w, mon_h, mon_scale) = if let Ok(Some(mon)) = stack_win.current_monitor()
    {
        let p = mon.position();
        let s = mon.size();
        (p.x, p.y, s.width, s.height, mon.scale_factor() as f32)
    } else {
        get_anchor_monitor_rect(app, last_rect)
    };

    let sf = f64::from(mon_scale.max(1.0));
    let new_w_phys = (width * sf).round() as i32;
    let new_h_phys = (height * sf).round() as i32;

    let (anchor_right, anchor_bottom) = match (stack_win.outer_position(), stack_win.outer_size()) {
        (Ok(pos), Ok(size)) => (pos.x + size.width as i32, pos.y + size.height as i32),
        _ => (
            mon_x + mon_w as i32 - (20.0 * sf).round() as i32,
            mon_y + mon_h as i32 - (56.0 * sf).round() as i32,
        ),
    };

    let mon_right = mon_x + mon_w as i32;
    let mon_bottom = mon_y + mon_h as i32;
    let max_x = (mon_right - new_w_phys - 12).max(mon_x + 12);
    let max_y = (mon_bottom - new_h_phys - 12).max(mon_y + 12);
    let new_x = (anchor_right - new_w_phys).clamp(mon_x + 12, max_x);
    let new_y = (anchor_bottom - new_h_phys).clamp(mon_y + 12, max_y);

    (new_x, new_y, new_w_phys, new_h_phys)
}

// ==================== Multi-Pin Window Commands ====================

#[tauri::command]
fn get_pin_item(window: tauri::WebviewWindow, state: State<'_, Mutex<AppState>>, pin_id: u64) -> Result<PinItemDto, String> {
    if window.label() != format!("pin-{pin_id}") {
        return Err("贴图窗口与截图不匹配".into());
    }
    let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
    let item = guard
        .pins
        .get(&pin_id)
        .ok_or_else(|| "未找到该贴图或已关闭".to_string())?;
    Ok(PinItemDto {
        id: item.id,
        image_data_url: item.crop.data_url(),
        pixel_width: item.crop.pixel_width,
        pixel_height: item.crop.pixel_height,
        scale_factor: item.crop.scale_factor,
    })
}

#[tauri::command]
fn close_pin_window(
    window: tauri::WebviewWindow,
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    pin_id: u64,
) -> Result<(), String> {
    if window.label() != format!("pin-{pin_id}") {
        return Err("贴图窗口与截图不匹配".into());
    }
    if let Ok(mut guard) = state.lock() {
        guard.pins.remove(&pin_id);
    }
    let label = format!("pin-{pin_id}");
    let app_ui = app.clone();
    post_to_ui(&app, move || {
        if let Some(win) = app_ui.get_webview_window(&label) {
            let _ = win.close();
        }
    });
    Ok(())
}

#[tauri::command(async)]
fn copy_pin_image(window: tauri::WebviewWindow, state: State<'_, Mutex<AppState>>, pin_id: u64) -> Result<(), String> {
    if window.label() != format!("pin-{pin_id}") {
        return Err("贴图窗口与截图不匹配".into());
    }
    let png_bytes = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard
            .pins
            .get(&pin_id)
            .ok_or_else(|| "贴图不存在".to_string())?
            .crop
            .png_bytes
            .clone()
    };
    copy_png_bytes_to_clipboard(&png_bytes)
}

#[tauri::command]
async fn save_pin_image(window: tauri::WebviewWindow, state: State<'_, Mutex<AppState>>, pin_id: u64) -> Result<bool, String> {
    if window.label() != format!("pin-{pin_id}") {
        return Err("贴图窗口与截图不匹配".into());
    }
    let png_bytes = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        guard
            .pins
            .get(&pin_id)
            .ok_or_else(|| "贴图不存在".to_string())?
            .crop
            .png_bytes
            .clone()
    };
    save_png_with_dialog(png_bytes, "Luma_Pin").await
}

#[tauri::command(async)]
fn open_pin_in_result(
    window: tauri::WebviewWindow,
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    pin_id: u64,
    mode: String,
) -> Result<(), String> {
    if window.label() != format!("pin-{pin_id}") {
        return Err("贴图窗口与截图不匹配".into());
    }
    {
        let mut guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        let crop = guard.pins.get(&pin_id).ok_or("贴图不存在")?.crop.clone();
        guard.last_crop = Some(crop);
        guard.last_result = None;
        guard.last_input_text = None;
        guard.ask_messages.clear();
        guard.current_job_id += 1;
    }
    let (popup_width, popup_height) = if mode == "ask" { (456.0, 470.0) } else { (456.0, 210.0) };
    switch_result_mode(app.clone(), state, mode, None, None)?;

    let pin_label = format!("pin-{pin_id}");
    let app_ui = app.clone();
    post_to_ui(&app, move || {
        let placement = app_ui.get_webview_window(&pin_label).and_then(|pin| {
            let pin_pos = pin.outer_position().ok()?;
            let pin_size = pin.outer_size().ok()?;
            let monitor = pin.current_monitor().ok()??;
            let work = monitor.work_area();
            let origin = work.position;
            let size = work.size;
            let scale = monitor.scale_factor();
            let width = (popup_width * scale).round() as i32;
            let height = (popup_height * scale).round() as i32;
            let right = (origin.x + size.width as i32 - width).max(origin.x);
            let bottom = (origin.y + size.height as i32 - height).max(origin.y);
            let after_pin = pin_pos.x + pin_size.width as i32 + 10;
            let x = if after_pin <= right { after_pin } else { pin_pos.x - width - 10 };
            Some(PhysicalPosition::new(x.clamp(origin.x, right), pin_pos.y.clamp(origin.y, bottom)))
        });
        if let Some(result_win) = app_ui.get_webview_window("result") {
            if let Some(position) = placement {
                let _ = result_win.set_position(position);
            } else {
                let _ = result_win.center();
            }
            show_result_window_foreground(&app_ui, &result_win, None);
        }
    });
    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let pre_config = pre_setup_config_dir()
        .map(|d| load_config_from_dir(&d))
        .unwrap_or_default();
    let client = build_api_client(&pre_config.proxy_url)
        .unwrap_or_else(|_| build_api_client("").expect("failed to build reqwest client"));
    let pre_history = pre_setup_config_dir()
        .map(|d| load_history_from_dir(&d))
        .unwrap_or_default();
    let pre_next_history_id = pre_history.iter().map(|h| h.id).max().unwrap_or(0);
    let pre_shortcut = pre_config.shortcut.clone();
    let pre_clipboard_shortcut = pre_config.clipboard_shortcut.clone();
    let pre_selection_shortcut = pre_config.selection_shortcut.clone();

    let activation_pending = Arc::new(AtomicBool::new(false));
    let builder = tauri::Builder::default();
    #[cfg(any(windows, target_os = "macos"))]
    let builder = {
        let activation_pending = activation_pending.clone();
        builder.plugin(tauri_plugin_single_instance::init(move |app, _, _| {
            if app.get_webview_window("main").is_some() {
                show_main_window(app);
            } else {
                activation_pending.store(true, Ordering::Release);
            }
        }))
        .plugin(tauri_plugin_notification::init())
    };

    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::UpdateState::default())
        .manage(Mutex::new(AppState {
            config: pre_config,
            client,
            last_capture: None,
            last_capture_source_hwnd: None,
            last_monitor_rect: None,
            last_crop: None,
            last_result: None,
            last_input_text: None,
            ask_messages: Vec::new(),
            current_job_id: 0,
            current_capture_id: 0,
            overlay_presented: false,
            latest_result_state: ResultViewState::Idle,
            history: pre_history,
            next_history_id: pre_next_history_id,
            is_capturing: false,
            registered_shortcut: pre_shortcut,
            shortcut_error: None,
            registered_clipboard_shortcut: pre_clipboard_shortcut,
            clipboard_shortcut_id: None,
            clipboard_shortcut_error: None,
            registered_selection_shortcut: pre_selection_shortcut,
            selection_shortcut_id: None,
            selection_shortcut_error: None,
            result_anchor_below: true,
            result_anchor_y: None,
            overlay_crops: Vec::new(),
            overlay_ask_messages: Vec::new(),
            overlay_job_id: 0,
            overlay_analysis_state: OverlayAnalysisState::Idle,
            staged_stack: Vec::new(),
            next_stack_id: 0,
            stack_extent: (236.0, 168.0),
            stack_frame_waiter: None,
            stack_ask_messages: Vec::new(),
            stack_ask_loading: false,
            stack_ask_error: None,
            stack_batch_mode: "translate".to_string(),
            stack_batch_loading: false,
            stack_batch_error: None,
            stack_batch_items: Vec::new(),
            stack_batch_elapsed_ms: 0,
            stack_ask_job_id: 0,
            stack_batch_job_id: 0,
            pins: HashMap::new(),
            next_pin_id: 0,
        }))
        .setup(move |app| {
            let handle = app.handle().clone();
            #[cfg(windows)]
            if let Some(overlay) = handle.get_webview_window("overlay") {
                if let Err(err) = disable_capture_window_transitions(&overlay) {
                    eprintln!("[Luma] Could not disable native capture transitions: {err}");
                }
            }
            #[cfg(target_os = "macos")]
            {
                if let Some(overlay) = handle.get_webview_window("overlay") {
                    if let Ok(ns_win) = overlay.ns_window() {
                        macos_bridge::configure_overlay_window(ns_win);
                    }
                }
                for label in ["result", "stack"] {
                    if let Some(win) = handle.get_webview_window(label) {
                        if let Ok(ns_win) = win.ns_window() {
                            macos_bridge::configure_floating_window(ns_win);
                        }
                    }
                }
            }
            let config = load_config(&handle);
            // Launching from another path must not rewrite the existing startup entry.
            let history = load_history(&handle);
            let next_history_id = history.iter().map(|h| h.id).max().unwrap_or(0);
            let initial_shortcut = config.shortcut.clone();
            let initial_clipboard_shortcut = config.clipboard_shortcut.clone();
            let initial_selection_shortcut = config.selection_shortcut.clone();
            let should_show_main = !config.start_minimized;
            let initial_theme = normalize_theme(&config.theme);

            let initial_client = build_api_client(&config.proxy_url).ok();
            if let Ok(mut guard) = handle.state::<Mutex<AppState>>().lock() {
                if let Some(next_client) = initial_client {
                    guard.client = next_client;
                }
                guard.config = config;
                guard.history = history;
                guard.next_history_id = next_history_id;
                guard.registered_shortcut = initial_shortcut.clone();
                guard.registered_clipboard_shortcut = initial_clipboard_shortcut.clone();
                guard.registered_selection_shortcut = initial_selection_shortcut.clone();
            }

            let handle_theme = handle.clone();
            post_to_ui(&handle, move || {
                apply_native_window_theme(&handle_theme, &initial_theme);
                let _ = handle_theme.emit("theme:updated", &initial_theme);
            });

            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(|app, shortcut, event| {
                        if event.state() == ShortcutState::Pressed {
                            let (is_clipboard_shortcut, is_selection_shortcut) = app
                                .state::<Mutex<AppState>>()
                                .lock()
                                .map(|g| {
                                    (
                                        g.clipboard_shortcut_id == Some(shortcut.id()),
                                        g.selection_shortcut_id == Some(shortcut.id()),
                                    )
                                })
                                .unwrap_or((false, false));
                            let app_clone = app.clone();
                            std::thread::spawn(move || {
                                if is_selection_shortcut {
                                    if let Err(err) = present_selected_text_in_result(&app_clone) {
                                        eprintln!("[Luma] 快捷键划词翻译失败: {err}");
                                        report_capture_error(&app_clone, err);
                                    }
                                } else if is_clipboard_shortcut {
                                    if let Err(err) = present_clipboard_text_in_result(&app_clone) {
                                        eprintln!("[Luma] 快捷键翻译剪贴板失败: {err}");
                                        report_capture_error(&app_clone, err);
                                    }
                                } else if let Err(err) = trigger_capture(&app_clone, false) {
                                    eprintln!("[Luma] 快捷键截图失败: {err}");
                                    report_capture_error(&app_clone, err);
                                }
                            });
                        }
                    })
                    .build(),
            )?;

            let mut shortcut_failed = false;
            match register_shortcuts(
                &handle,
                &initial_shortcut,
                &initial_clipboard_shortcut,
                &initial_selection_shortcut,
            ) {
                Ok(outcome) => {
                    if let Ok(mut guard) = handle.state::<Mutex<AppState>>().lock() {
                        guard.clipboard_shortcut_id = outcome.clipboard_id;
                        guard.clipboard_shortcut_error = outcome.clipboard_error.clone();
                        guard.selection_shortcut_id = outcome.selection_id;
                        guard.selection_shortcut_error = outcome.selection_error.clone();
                        let st = build_shortcut_status(&guard);
                        let handle_ui = handle.clone();
                        post_to_ui(&handle, move || {
                            if let Some(main_win) = handle_ui.get_webview_window("main") {
                                let _ = main_win.emit("shortcut:status", &st);
                            }
                        });
                    }
                }
                Err(err) => {
                    eprintln!("[Luma] {err}");
                    shortcut_failed = true;
                    if let Ok(mut guard) = handle.state::<Mutex<AppState>>().lock() {
                        guard.shortcut_error = Some(err);
                        let st = build_shortcut_status(&guard);
                        let handle_ui = handle.clone();
                        post_to_ui(&handle, move || {
                            if let Some(main_win) = handle_ui.get_webview_window("main") {
                                let _ = main_win.emit("shortcut:status", &st);
                            }
                        });
                    }
                }
            }

            let capture_item = MenuItemBuilder::with_id("capture", "立即截图").build(app)?;
            let clip_translate_item =
                MenuItemBuilder::with_id("clipboard_translate", "翻译剪贴板图片").build(app)?;
            let clip_ocr_item =
                MenuItemBuilder::with_id("clipboard_ocr", "识别剪贴板图片").build(app)?;
            let clip_text_item =
                MenuItemBuilder::with_id("clipboard_text", "翻译剪贴板文字").build(app)?;
            let settings_item = MenuItemBuilder::with_id("settings", "打开设置与历史").build(app)?;
            let update_item = MenuItemBuilder::with_id("update", "检查更新").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;

            let tray_menu = MenuBuilder::new(app)
                .item(&capture_item)
                .item(&clip_translate_item)
                .item(&clip_ocr_item)
                .item(&clip_text_item)
                .item(&settings_item)
                .item(&update_item)
                .separator()
                .item(&quit_item)
                .build()?;

            let mut tray_builder = TrayIconBuilder::new()
                .menu(&tray_menu)
                .tooltip("Luma · 快捷截图与 AI 翻译")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "capture" => {
                        let app_clone = app.clone();
                        std::thread::spawn(move || {
                            if let Err(err) = trigger_capture(&app_clone, false) {
                                eprintln!("[Luma] 托盘截图失败: {err}");
                                report_capture_error(&app_clone, err);
                            }
                        });
                    }
                    "clipboard_translate" | "clipboard_ocr" => {
                        let mode = if event.id().as_ref() == "clipboard_ocr" {
                            "ocr"
                        } else {
                            "translate"
                        };
                        let app_clone = app.clone();
                        std::thread::spawn(move || {
                            if let Err(err) = read_clipboard_cropped_region().and_then(|crop| {
                                present_external_crop_in_result(&app_clone, crop, mode)
                            }) {
                                report_capture_error(&app_clone, err);
                            }
                        });
                    }
                    "clipboard_text" => {
                        let app_clone = app.clone();
                        std::thread::spawn(move || {
                            if let Err(err) = present_clipboard_text_in_result(&app_clone) {
                                report_capture_error(&app_clone, err);
                            }
                        });
                    }
                    "settings" => show_main_window(app),
                    "update" => {
                        let app_clone = app.clone();
                        std::thread::spawn(move || {
                            if let Some(window) = app_clone.get_webview_window("main") {
                                show_main_window(&app_clone);
                                let _ = window.emit("open-update", ());
                            }
                        });
                    }
                    "quit" => {
                        local_ocr::shutdown_worker();
                        selection_capture::stop_smart_selection_hook();
                        result_popover::shutdown();
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                });

            if let Some(icon) = app.default_window_icon().cloned() {
                tray_builder = tray_builder.icon(icon);
            }
            tray_builder.build(app)?;
            #[cfg(any(windows, target_os = "macos"))]
            {
                use tauri_plugin_notification::NotificationExt;
                let notification_app = handle.clone();
                if let Err(err) = handle.notification().on_action(move |action| {
                    if action.action_id() == "tap" {
                        let ui = notification_app.clone();
                        post_to_ui(&notification_app, move || {
                            show_main_window(&ui);
                            if let Some(window) = ui.get_webview_window("main") {
                                let _ = window.emit("open-update", ());
                            }
                        });
                    }
                }) {
                    eprintln!("[Luma] 注册更新通知点击处理失败: {err}");
                }
            }
            updates::start_automatic_checks(handle.clone(), update_item);

            if should_show_main || shortcut_failed || activation_pending.load(Ordering::Acquire) {
                show_main_window(&handle);
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(windows)]
            if window.label() == "stack" && matches!(event, WindowEvent::Resized(_)) {
                let app = window.app_handle().clone();
                let ui = app.clone();
                post_to_ui(&app, move || {
                    if let Some(stack) = ui.get_webview_window("stack") {
                        let extent = ui.state::<Mutex<AppState>>().lock().ok().map(|state| state.stack_extent);
                        if let Some(extent) = extent {
                            if let Err(err) = set_stack_region(&stack, extent) {
                                eprintln!("{err}");
                            }
                        }
                    }
                });
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    "main" => {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    "overlay" => {
                        api.prevent_close();
                        if let Ok(mut guard) = window.app_handle().state::<Mutex<AppState>>().lock()
                        {
                            guard.is_capturing = false;
                            guard.last_capture = None;
                            guard.overlay_crops.clear();
                            guard.overlay_ask_messages.clear();
                            guard.overlay_job_id += 1;
                            guard.overlay_analysis_state = OverlayAnalysisState::Idle;
                        }
                        let _ = window.hide();
                        if let Some(overlay) = window.app_handle().get_webview_window("overlay") {
                            dispatch_window_event(&overlay, "capture:clear", &());
                        }
                        restore_stack_if_nonempty(window.app_handle());
                    }
                    "result" => {
                        api.prevent_close();
                        result_popover::deactivate();
                        if let Ok(mut guard) = window.app_handle().state::<Mutex<AppState>>().lock()
                        {
                            guard.current_job_id += 1;
                            guard.last_crop = None;
                            guard.last_result = None;
                            guard.last_input_text = None;
                            guard.ask_messages.clear();
                            guard.latest_result_state = ResultViewState::Idle;
                        }
                        if let Some(result) = window.app_handle().get_webview_window("result") {
                            dispatch_window_event(&result, "result:update", &ResultViewState::Idle);
                        }
                        let _ = window.set_always_on_top(false);
                        let _ = window.hide();
                    }
                    "stack" => {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    label if label.starts_with("pin-") => {
                        if let Ok(pin_id) = label.trim_start_matches("pin-").parse::<u64>() {
                            if let Ok(mut guard) =
                                window.app_handle().state::<Mutex<AppState>>().lock()
                            {
                                guard.pins.remove(&pin_id);
                            }
                        }
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_app_version,
            updates::check_app_update,
            updates::get_available_app_update,
            updates::install_app_update,
            get_config,
            get_theme,
            get_ask_quick_actions,
            save_ask_quick_actions,
            get_stack_prompts,
            save_stack_prompts,
            preview_theme,
            update_config,
            get_shortcut_status,
            get_system_proxy_info,
            local_ocr::get_local_ocr_status,
            local_ocr::download_local_ocr,
            local_ocr::cancel_local_ocr_download,
            local_ocr::import_local_ocr_archive,
            local_ocr::uninstall_local_ocr,
            local_ocr::test_local_ocr,
            test_api_connection,
            start_capture,
            get_current_capture,
            get_capture_image,
            capture_has_staged_items,
            overlay_image_ready,
            cancel_capture,
            copy_selection_image,
            save_selection_image,
            submit_selection,
            confirm_selection,
            switch_result_mode,
            ask_ai_follow_up,
            copy_last_crop_image,
            save_last_crop_image,
            retry_translation,
            analyze_clipboard_image,
            analyze_clipboard_text,
            analyze_selected_text,
            retranslate_custom_text,
            translate_text_direct,
            reopen_history_item,
            export_config_json,
            import_config_json,
            get_latest_result_state,
            hide_result_window,
            resize_result_window,
            get_history,
            clear_history,
            delete_history_item,
            export_history,
            overlay_analyze_regions,
            overlay_switch_mode,
            overlay_ask_follow_up,
            overlay_clear_analysis,
            overlay_pin_current,
            stage_regions,
            stack_frame_ready,
            finish_stack_handoff,
            get_stack_state,
            remove_stack_item,
            reorder_stack_items,
            clear_stack,
            pin_stack_item,
            get_stack_item_image,
            update_stack_item_image,
            copy_stack_item_image,
            save_stack_item_image,
            copy_stitched_stack,
            save_stitched_stack,
            copy_stack_as_files,
            stack_ask_ai,
            stack_run_batch,
            stack_clear_chat,
            resize_stack_window,
            scan_qr_selection,
            scan_qr_base64,
            open_external_url,
            save_markdown_note,
            start_scroll_capture,
            get_pin_item,
            close_pin_window,
            copy_pin_image,
            save_pin_image,
            open_pin_in_result,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Luma application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                show_main_window(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = app;
            if let tauri::RunEvent::Exit = event {
                local_ocr::shutdown_worker();
                selection_capture::stop_smart_selection_hook();
                result_popover::shutdown();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::format_history_markdown;
    use crate::translate::TranslationResult;

    #[test]
    fn formats_history_markdown_cleanly() {
        let items = vec![
            TranslationResult {
                id: 1,
                mode: "translate".to_string(),
                engine: "ai".to_string(),
                source_lang: "English".to_string(),
                target_lang: "简体中文".to_string(),
                original_text: "Hello world".to_string(),
                translated_text: "你好，世界".to_string(),
                blocks: Vec::new(),
                elapsed_ms: 420,
                model: "gpt-4o-mini".to_string(),
                image_data_url: String::new(),
                created_at_ms: 1700000000000,
            },
            TranslationResult {
                id: 2,
                mode: "ocr".to_string(),
                engine: "ai".to_string(),
                source_lang: "English".to_string(),
                target_lang: "纯文本识别".to_string(),
                original_text: "SnapTrans".to_string(),
                translated_text: "SnapTrans".to_string(),
                blocks: Vec::new(),
                elapsed_ms: 85,
                model: "Windows OCR".to_string(),
                image_data_url: String::new(),
                created_at_ms: 1700000005000,
            },
        ];
        let md = format_history_markdown(&items);
        assert!(md.contains("共 2 条记录"));
        assert!(md.contains("[翻译] English → 简体中文"));
        assert!(md.contains("你好，世界"));
        assert!(md.contains("[识字 OCR]"));
    }
}

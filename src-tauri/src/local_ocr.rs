#![cfg_attr(target_os = "macos", allow(dead_code))]

use crate::config::{apply_proxy, normalize_ocr_lang};
use crate::win_ocr::{
    group_ocr_lines_into_blocks, guess_lang_from_text, OcrLayoutBlock, OcrLineBox,
};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, Once, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::Manager;

pub const RAPIDOCR_VERSION: &str = "v0.2.0";
pub const RAPIDOCR_OFFICIAL_URL: &str =
    "https://github.com/hiroi-sora/RapidOCR-json/releases/download/v0.2.0/RapidOCR-json_v0.2.0.7z";
pub const RAPIDOCR_OFFICIAL_SHA256: &str =
    "7ad9b283d03436c6cd0296723188699299cb4e5cf9140b410c59543aa5793c40";

const MAX_ARCHIVE_BYTES: u64 = 200 * 1024 * 1024;
const WORKER_INIT_TIMEOUT: Duration = Duration::from_secs(15);
const WORKER_INFER_TIMEOUT: Duration = Duration::from_secs(30);
const WORKER_IDLE_TIMEOUT_MS: u64 = 5 * 60 * 1000;
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(120);
const CANONICAL_LANGS: [&str; 6] = ["ch", "cht", "en", "jp", "kr", "ru"];

static DOWNLOAD_BUSY: AtomicBool = AtomicBool::new(false);
static DOWNLOAD_CANCELLED: AtomicBool = AtomicBool::new(false);
static LAST_ERROR: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static WORKER: OnceLock<Mutex<Option<RapidOcrWorker>>> = OnceLock::new();
static IDLE_WATCHER_STARTED: Once = Once::new();
static MONO_BASE: OnceLock<Instant> = OnceLock::new();
static LAST_USED_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOcrStatus {
    pub installed: bool,
    pub version: String,
    pub install_dir: String,
    pub disk_bytes: u64,
    pub available_langs: Vec<String>,
    pub active_lang: String,
    pub busy: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOcrProgress {
    pub stage: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub percent: Option<u8>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RapidOcrModelSpec {
    pub det: String,
    pub cls: Option<String>,
    pub rec: String,
    pub keys: String,
}

#[derive(Debug, Clone)]
pub struct InstalledRapidOcr {
    pub exe_path: PathBuf,
    pub exe_dir: PathBuf,
    #[allow(dead_code)]
    pub models_dir: PathBuf,
    pub models_rel_arg: String,
    pub lang_models: HashMap<String, RapidOcrModelSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RapidOcrItem {
    #[serde(rename = "box")]
    pub box_pts: [[f32; 2]; 4],
    pub score: f32,
    pub text: String,
}

#[derive(Debug, Deserialize)]
struct RapidOcrRawResponse {
    code: i32,
    #[serde(default)]
    data: serde_json::Value,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

struct RapidOcrWorker {
    lang: String,
    exe_path: PathBuf,
    child: Child,
    stdin: Option<ChildStdin>,
    stdout_rx: Receiver<Option<String>>,
    stderr_tail: std::sync::Arc<Mutex<Vec<String>>>,
}

impl Drop for RapidOcrWorker {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct BusyGuard;

impl BusyGuard {
    fn try_acquire() -> Result<Self, String> {
        if DOWNLOAD_BUSY
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("正在下载或安装离线 OCR 引擎，请稍候。".to_string());
        }
        DOWNLOAD_CANCELLED.store(false, Ordering::Release);
        Ok(Self)
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        DOWNLOAD_BUSY.store(false, Ordering::Release);
    }
}

struct TempCleanupGuard {
    paths: Vec<PathBuf>,
}

impl TempCleanupGuard {
    fn new() -> Self {
        Self { paths: Vec::new() }
    }

    fn push(&mut self, path: PathBuf) {
        self.paths.push(path);
    }
}

impl Drop for TempCleanupGuard {
    fn drop(&mut self) {
        for p in &self.paths {
            if p.is_dir() {
                let _ = fs::remove_dir_all(p);
            } else if p.exists() {
                let _ = fs::remove_file(p);
            }
        }
    }
}

fn last_error_slot() -> &'static Mutex<Option<String>> {
    LAST_ERROR.get_or_init(|| Mutex::new(None))
}

fn set_last_error(err: Option<String>) {
    if let Ok(mut guard) = last_error_slot().lock() {
        *guard = err;
    }
}

fn get_last_error() -> Option<String> {
    last_error_slot()
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
}

fn worker_slot() -> &'static Mutex<Option<RapidOcrWorker>> {
    WORKER.get_or_init(|| Mutex::new(None))
}

fn mono_ms() -> u64 {
    let base = MONO_BASE.get_or_init(Instant::now);
    base.elapsed().as_millis().max(1) as u64
}

fn ensure_idle_watcher() {
    IDLE_WATCHER_STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("luma-rapidocr-idle".to_string())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_secs(30));
                let last = LAST_USED_MS.load(Ordering::Acquire);
                if last == 0 {
                    continue;
                }
                let now = mono_ms();
                if now.saturating_sub(last) >= WORKER_IDLE_TIMEOUT_MS {
                    if let Ok(mut guard) = worker_slot().try_lock() {
                        if guard.is_some()
                            && mono_ms()
                                .saturating_sub(LAST_USED_MS.load(Ordering::Acquire))
                                >= WORKER_IDLE_TIMEOUT_MS
                        {
                            let _ = guard.take();
                        }
                    }
                }
            });
    });
}

/// Shut down any running RapidOCR background worker process and release file locks.
pub fn shutdown_worker() {
    if let Ok(mut guard) = worker_slot().lock() {
        let _ = guard.take();
    }
}

/// Return the versioned installation directory for RapidOCR without requiring `AppHandle`.
pub fn ocr_engine_install_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
            return Ok(PathBuf::from(local_appdata)
                .join("Luma")
                .join("OcrEngine")
                .join(format!("rapidocr-{RAPIDOCR_VERSION}")));
        }
        if let Some(userprofile) = std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()) {
            return Ok(PathBuf::from(userprofile)
                .join("AppData")
                .join("Local")
                .join("Luma")
                .join("OcrEngine")
                .join(format!("rapidocr-{RAPIDOCR_VERSION}")));
        }
        Err("无法获取本地应用数据目录 (LOCALAPPDATA)".to_string())
    }
    #[cfg(not(windows))]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
            return Ok(PathBuf::from(xdg)
                .join("Luma")
                .join("OcrEngine")
                .join(format!("rapidocr-{RAPIDOCR_VERSION}")));
        }
        if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
            return Ok(PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("Luma")
                .join("OcrEngine")
                .join(format!("rapidocr-{RAPIDOCR_VERSION}")));
        }
        Err("无法获取本地数据目录 (XDG_DATA_HOME / HOME)".to_string())
    }
}

/// Check whether a usable RapidOCR-json executable and at least one language model are installed.
pub fn is_installed() -> bool {
    #[cfg(target_os = "macos")]
    return false;
    #[cfg(not(target_os = "macos"))]
    ocr_engine_install_dir()
        .ok()
        .and_then(|dir| discover_installed_engine(&dir))
        .is_some()
}

fn dir_size_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        total = total.saturating_add(dir_size_bytes(&entry.path()));
    }
    total
}

fn relative_path_string(from_dir: &Path, target: &Path) -> String {
    if let Ok(rel) = target.strip_prefix(from_dir) {
        let s = rel.to_string_lossy().replace('\\', "/");
        if s.is_empty() {
            return ".".to_string();
        }
        return s;
    }
    let from_comps: Vec<Component<'_>> = from_dir.components().collect();
    let to_comps: Vec<Component<'_>> = target.components().collect();
    let mut common = 0usize;
    while common < from_comps.len()
        && common < to_comps.len()
        && from_comps[common] == to_comps[common]
    {
        common += 1;
    }
    if common == 0 {
        return "models".to_string();
    }
    let mut rel = PathBuf::new();
    for _ in common..from_comps.len() {
        rel.push("..");
    }
    for comp in &to_comps[common..] {
        rel.push(comp.as_os_str());
    }
    let s = rel.to_string_lossy().replace('\\', "/");
    if s.is_empty() {
        ".".to_string()
    } else {
        s
    }
}

fn find_file_bfs<F>(root: &Path, max_depth: usize, matcher: F) -> Option<PathBuf>
where
    F: Fn(&Path, &str) -> bool,
{
    let mut queue = std::collections::VecDeque::new();
    queue.push_back((root.to_path_buf(), 0usize));
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if ft.is_file() && matcher(&path, &name) {
                return Some(path);
            }
            if ft.is_dir() && depth < max_depth {
                subdirs.push(path);
            }
        }
        subdirs.sort();
        for sub in subdirs {
            queue.push_back((sub, depth + 1));
        }
    }
    None
}

fn find_models_dir(exe_dir: &Path, root: &Path) -> Option<PathBuf> {
    let direct = exe_dir.join("models");
    if direct.is_dir() {
        return Some(direct);
    }
    if let Ok(entries) = fs::read_dir(exe_dir) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir())
                && entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("models")
            {
                return Some(entry.path());
            }
        }
    }
    let mut queue = std::collections::VecDeque::new();
    queue.push_back((root.to_path_buf(), 0usize));
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() || !ft.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if name.eq_ignore_ascii_case("models") {
                return Some(path);
            }
            if depth < 4 {
                queue.push_back((path, depth + 1));
            }
        }
    }
    None
}

fn collect_model_files(models_dir: &Path) -> (Vec<String>, Vec<String>) {
    let mut onnx_files = Vec::new();
    let mut txt_files = Vec::new();
    let mut stack = vec![(models_dir.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() && depth < 2 {
                stack.push((path, depth + 1));
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let Ok(rel) = path.strip_prefix(models_dir) else {
                continue;
            };
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let lower = rel_str.to_ascii_lowercase();
            if lower.ends_with(".onnx") {
                onnx_files.push(rel_str);
            } else if lower.ends_with(".txt") {
                txt_files.push(rel_str);
            }
        }
    }
    onnx_files.sort();
    txt_files.sort();
    (onnx_files, txt_files)
}

fn model_version_rank(name_lower: &str) -> i32 {
    if name_lower.contains("v4") || name_lower.contains("pp-ocrv4") {
        400
    } else if name_lower.contains("v3") || name_lower.contains("pp-ocrv3") {
        300
    } else if name_lower.contains("v2") {
        200
    } else {
        100
    }
}

fn infer_lang_from_hint(raw: &str) -> Option<&'static str> {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("cht")
        || raw.contains("繁")
        || lower.contains("traditional")
    {
        return Some("cht");
    }
    if lower.contains("japan")
        || lower.starts_with("jp_")
        || lower.contains("_jp_")
        || lower.contains("jpn")
        || raw.contains("日")
    {
        return Some("jp");
    }
    if lower.contains("korean")
        || lower.starts_with("kr_")
        || lower.contains("_kr_")
        || lower.contains("kor")
        || raw.contains("韩")
        || raw.contains("韓")
    {
        return Some("kr");
    }
    if lower.contains("cyrillic")
        || lower.starts_with("ru_")
        || lower.contains("_ru_")
        || lower.contains("russian")
        || raw.contains("俄")
        || raw.contains("Рус")
    {
        return Some("ru");
    }
    if lower.starts_with("en_")
        || lower.contains("_en_")
        || lower.contains("english")
        || lower.contains("latin")
        || raw.contains("英文")
        || raw.contains("英语")
    {
        return Some("en");
    }
    if lower.starts_with("ch_")
        || lower.contains("_ch_")
        || lower.contains("chinese")
        || lower.contains("ppocr_keys")
        || raw.contains("简")
        || raw.contains("中英")
        || raw.contains("中文")
    {
        return Some("ch");
    }
    None
}

fn tokenize_cmd_args(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in line.chars() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                cur.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn resolve_existing_model_rel(models_dir: &Path, raw: &str, all_files: &[String]) -> Option<String> {
    let cleaned = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .replace('\\', "/");
    let stripped = cleaned.strip_prefix("models/").unwrap_or(&cleaned);
    if models_dir.join(stripped).is_file() {
        return Some(stripped.to_string());
    }
    let file_only = Path::new(stripped)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())?;
    all_files
        .iter()
        .find(|f| {
            Path::new(f.as_str())
                .file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&file_only))
        })
        .cloned()
}

fn parse_cmd_txt(
    exe_dir: &Path,
    models_dir: &Path,
    onnx_files: &[String],
    txt_files: &[String],
) -> HashMap<String, RapidOcrModelSpec> {
    let mut out: HashMap<String, RapidOcrModelSpec> = HashMap::new();
    let candidates = [models_dir.join("cmd.txt"), exe_dir.join("cmd.txt")];
    let mut all_files = Vec::with_capacity(onnx_files.len() + txt_files.len());
    all_files.extend_from_slice(onnx_files);
    all_files.extend_from_slice(txt_files);

    for path in candidates {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let mut prev_header = String::new();
        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if !line.contains("--rec") && !line.contains("--det") {
                prev_header = line.to_string();
                continue;
            }
            let prefix_hint = line.split("--").next().unwrap_or("").trim();
            let tokens = tokenize_cmd_args(line);
            let mut det: Option<String> = None;
            let mut cls: Option<String> = None;
            let mut rec: Option<String> = None;
            let mut keys: Option<String> = None;
            let mut idx = 0usize;
            while idx < tokens.len() {
                let tok = &tokens[idx];
                let (flag, val_opt) = if let Some((k, v)) = tok.split_once('=') {
                    (k, Some(v.to_string()))
                } else if tok.starts_with("--") && idx + 1 < tokens.len() {
                    let next = &tokens[idx + 1];
                    if !next.starts_with("--") {
                        idx += 1;
                        (tok.as_str(), Some(next.clone()))
                    } else {
                        (tok.as_str(), None)
                    }
                } else {
                    (tok.as_str(), None)
                };
                if let Some(val) = val_opt {
                    match flag {
                        "--det" => det = resolve_existing_model_rel(models_dir, &val, &all_files),
                        "--cls" => cls = resolve_existing_model_rel(models_dir, &val, &all_files),
                        "--rec" => rec = resolve_existing_model_rel(models_dir, &val, &all_files),
                        "--keys" => keys = resolve_existing_model_rel(models_dir, &val, &all_files),
                        _ => {}
                    }
                }
                idx += 1;
            }
            if let (Some(det_f), Some(rec_f), Some(keys_f)) = (det, rec, keys) {
                let lang = infer_lang_from_hint(&rec_f)
                    .or_else(|| infer_lang_from_hint(&keys_f))
                    .or_else(|| infer_lang_from_hint(prefix_hint))
                    .or_else(|| infer_lang_from_hint(&prev_header));
                if let Some(lang_code) = lang {
                    let replace = match out.get(lang_code) {
                        None => true,
                        Some(existing) => {
                            let new_score = model_version_rank(&rec_f.to_ascii_lowercase())
                                + model_version_rank(&det_f.to_ascii_lowercase());
                            let old_score = model_version_rank(&existing.rec.to_ascii_lowercase())
                                + model_version_rank(&existing.det.to_ascii_lowercase());
                            new_score > old_score
                        }
                    };
                    if replace {
                        out.insert(
                            lang_code.to_string(),
                            RapidOcrModelSpec {
                                det: det_f,
                                cls,
                                rec: rec_f,
                                keys: keys_f,
                            },
                        );
                    }
                }
            }
            prev_header.clear();
        }
    }
    out
}

fn select_best_cls(onnx_files: &[String]) -> Option<String> {
    onnx_files
        .iter()
        .filter(|f| f.to_ascii_lowercase().contains("cls"))
        .max_by_key(|f| {
            let lower = f.to_ascii_lowercase();
            let mut score = model_version_rank(&lower);
            if lower.contains("mobile") {
                score += 10;
            }
            score
        })
        .cloned()
}

fn select_best_det(onnx_files: &[String], lang: &str) -> Option<String> {
    onnx_files
        .iter()
        .filter(|f| f.to_ascii_lowercase().contains("det"))
        .max_by_key(|f| {
            let lower = f.to_ascii_lowercase();
            let mut score = model_version_rank(&lower);
            if lang == "ch" || lang == "cht" {
                if lower.starts_with("ch_") || lower.contains("_ch_") {
                    score += 80;
                } else if lower.contains("multilingual") {
                    score += 40;
                }
            } else if lang == "en" {
                if lower.starts_with("en_") || lower.contains("_en_") {
                    score += 50;
                } else if lower.starts_with("ch_") || lower.contains("_ch_") {
                    score += 45;
                } else if lower.contains("multilingual") {
                    score += 40;
                }
            } else if lower.contains("multilingual") {
                score += 70;
            } else if lower.starts_with("ch_") || lower.contains("_ch_") {
                score += 60;
            }
            if lower.contains("infer") {
                score += 5;
            }
            score
        })
        .cloned()
}

fn select_best_rec(onnx_files: &[String], lang: &str) -> Option<String> {
    onnx_files
        .iter()
        .filter(|f| {
            let lower = f.to_ascii_lowercase();
            lower.contains("rec") && infer_lang_from_hint(&lower) == Some(lang)
        })
        .max_by_key(|f| {
            let lower = f.to_ascii_lowercase();
            let mut score = model_version_rank(&lower);
            if lower.contains("infer") {
                score += 10;
            }
            if lower.contains("server") {
                score += 5;
            }
            score
        })
        .cloned()
}

fn select_best_keys(txt_files: &[String], lang: &str) -> Option<String> {
    txt_files
        .iter()
        .filter(|f| {
            let lower = f.to_ascii_lowercase();
            let base = Path::new(&lower)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            !matches!(
                base.as_str(),
                "cmd.txt" | "readme.txt" | "license.txt" | "config.txt" | "version.txt"
            )
        })
        .filter_map(|f| {
            let lower = f.to_ascii_lowercase();
            let mut score: i32 = 0;
            match lang {
                "ch" => {
                    if lower.contains("ppocr_keys_v1") {
                        score = 300;
                    } else if lower.contains("ppocr_keys") {
                        score = 250;
                    } else if infer_lang_from_hint(&lower) == Some("ch")
                        && (lower.contains("dict") || lower.contains("keys"))
                    {
                        score = 200;
                    }
                }
                "cht" => {
                    if infer_lang_from_hint(&lower) == Some("cht")
                        && (lower.contains("dict") || lower.contains("keys"))
                    {
                        score = 300;
                    } else if lower.contains("cht") {
                        score = 250;
                    }
                }
                "en" => {
                    if (lower.contains("en_dict") || lower.contains("en_keys") || lower.contains("english"))
                        && (lower.contains("dict") || lower.contains("keys"))
                    {
                        score = 300;
                    } else if infer_lang_from_hint(&lower) == Some("en") {
                        score = 200;
                    }
                }
                "jp" => {
                    if (lower.contains("japan") || lower.contains("jp"))
                        && (lower.contains("dict") || lower.contains("keys"))
                    {
                        score = 300;
                    } else if infer_lang_from_hint(&lower) == Some("jp") {
                        score = 200;
                    }
                }
                "kr" => {
                    if (lower.contains("korean") || lower.contains("kr"))
                        && (lower.contains("dict") || lower.contains("keys"))
                    {
                        score = 300;
                    } else if infer_lang_from_hint(&lower) == Some("kr") {
                        score = 200;
                    }
                }
                "ru" => {
                    if (lower.contains("cyrillic") || lower.contains("ru_dict") || lower.contains("russian"))
                        && (lower.contains("dict") || lower.contains("keys"))
                    {
                        score = 300;
                    } else if infer_lang_from_hint(&lower) == Some("ru") {
                        score = 200;
                    }
                }
                _ => {}
            }
            (score > 0).then_some((score, f.clone()))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, path)| path)
}

/// Recursively discover an installed RapidOCR-json engine and its available language models.
pub fn discover_installed_engine(root: &Path) -> Option<InstalledRapidOcr> {
    if !root.is_dir() {
        return None;
    }
    let exe_path = find_file_bfs(root, 6, |_path, name| {
        name.eq_ignore_ascii_case("RapidOCR-json.exe")
    })?;
    let exe_dir = exe_path.parent()?.to_path_buf();
    let models_dir = find_models_dir(&exe_dir, root)?;
    let models_rel_arg = relative_path_string(&exe_dir, &models_dir);

    let (onnx_files, txt_files) = collect_model_files(&models_dir);
    if onnx_files.is_empty() || txt_files.is_empty() {
        return None;
    }

    let mut lang_models = parse_cmd_txt(&exe_dir, &models_dir, &onnx_files, &txt_files);
    let default_cls = select_best_cls(&onnx_files);

    for lang in CANONICAL_LANGS {
        let Some(rec) = select_best_rec(&onnx_files, lang) else {
            continue;
        };
        let Some(det) = select_best_det(&onnx_files, lang) else {
            continue;
        };
        let Some(keys) = select_best_keys(&txt_files, lang) else {
            continue;
        };
        let scanned_spec = RapidOcrModelSpec {
            det,
            cls: default_cls.clone(),
            rec,
            keys,
        };
        match lang_models.get(lang) {
            None => {
                lang_models.insert(lang.to_string(), scanned_spec);
            }
            Some(existing) => {
                let existing_score = model_version_rank(&existing.rec.to_ascii_lowercase())
                    + model_version_rank(&existing.det.to_ascii_lowercase());
                let scanned_score = model_version_rank(&scanned_spec.rec.to_ascii_lowercase())
                    + model_version_rank(&scanned_spec.det.to_ascii_lowercase());
                if scanned_score > existing_score {
                    lang_models.insert(lang.to_string(), scanned_spec);
                }
            }
        }
    }

    if lang_models.is_empty() {
        return None;
    }

    Some(InstalledRapidOcr {
        exe_path,
        exe_dir,
        models_dir,
        models_rel_arg,
        lang_models,
    })
}

fn ordered_available_langs(lang_models: &HashMap<String, RapidOcrModelSpec>) -> Vec<String> {
    CANONICAL_LANGS
        .iter()
        .filter(|&&lang| lang_models.contains_key(lang))
        .map(|&lang| lang.to_string())
        .collect()
}

fn build_status(active_lang: String) -> Result<LocalOcrStatus, String> {
    #[cfg(target_os = "macos")]
    {
        return Ok(LocalOcrStatus {
            installed: true,
            version: "Apple Vision".to_string(),
            install_dir: "macOS 系统内置 Apple Vision 框架".to_string(),
            disk_bytes: 0,
            available_langs: vec![
                "auto".to_string(),
                "ch".to_string(),
                "en".to_string(),
                "jp".to_string(),
                "ko".to_string(),
            ],
            active_lang,
            busy: false,
            last_error: None,
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let install_dir = ocr_engine_install_dir()?;
        let discovered = discover_installed_engine(&install_dir);
        let (installed, available_langs, disk_bytes) = match discovered {
            Some(ref info) => (
                true,
                ordered_available_langs(&info.lang_models),
                dir_size_bytes(&install_dir),
            ),
            None => (false, Vec::new(), 0),
        };
        Ok(LocalOcrStatus {
            installed,
            version: RAPIDOCR_VERSION.to_string(),
            install_dir: install_dir.to_string_lossy().to_string(),
            disk_bytes,
            available_langs,
            active_lang,
            busy: DOWNLOAD_BUSY.load(Ordering::Acquire),
            last_error: get_last_error(),
        })
    }
}

fn emit_ocr_progress(app: &tauri::AppHandle, progress: &LocalOcrProgress) {
    let serialized = serde_json::to_string(&("ocr:progress", progress))
        .and_then(|json| serde_json::to_string(&json));
    let Ok(json) = serialized else {
        return;
    };
    let app_ui = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = app_ui.get_webview_window("main") {
            let script = format!(
                "(() => {{ const [name, detail] = JSON.parse({json}); window.dispatchEvent(new CustomEvent(name, {{ detail }})); }})()"
            );
            if let Err(err) = win.eval(script) {
                eprintln!("[Luma] dispatch ocr:progress: {err}");
            }
        }
    });
}

fn to_hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

#[allow(dead_code)]
pub(crate) fn verify_sha256_bytes(bytes: &[u8], expected_hex: &str) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = to_hex_lower(&hasher.finalize());
    let expected = expected_hex.trim().to_ascii_lowercase();
    if actual != expected {
        return Err(format!(
            "离线 OCR 引擎包 SHA-256 校验失败（期望 {expected}，实际 {actual}）"
        ));
    }
    Ok(actual)
}

#[allow(dead_code)]
pub(crate) fn verify_sha256_file(path: &Path, expected_hex: &str) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("无法打开引擎包进行校验: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("读取引擎包计算校验和失败: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = to_hex_lower(&hasher.finalize());
    let expected = expected_hex.trim().to_ascii_lowercase();
    if actual != expected {
        return Err(format!(
            "离线 OCR 引擎包 SHA-256 校验失败（期望 {expected}，实际 {actual}）"
        ));
    }
    Ok(actual)
}

fn resolve_download_url(custom_url: Option<&str>) -> Result<(Url, bool), String> {
    let trimmed = custom_url.unwrap_or("").trim();
    if trimmed.is_empty() {
        let official = Url::parse(RAPIDOCR_OFFICIAL_URL)
            .map_err(|e| format!("内置官方下载地址无效: {e}"))?;
        return Ok((official, true));
    }
    let url = Url::parse(trimmed).map_err(|_| "自定义 RapidOCR 下载地址格式无效。".to_string())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "自定义下载地址必须是有效的 HTTPS 地址，且不能包含用户名、密码或片段 (#)。".to_string(),
        );
    }
    let is_official = url.as_str() == RAPIDOCR_OFFICIAL_URL;
    Ok((url, is_official))
}

fn unique_stamp() -> String {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{ns}", std::process::id())
}

fn rename_with_retry(src: &Path, dst: &Path) -> std::io::Result<()> {
    let mut last_err = None;
    for attempt in 0..8 {
        match fs::rename(src, dst) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                if attempt < 7 {
                    std::thread::sleep(Duration::from_millis(60));
                }
            }
        }
    }
    Err(last_err.unwrap())
}

fn remove_dir_with_retry(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let mut last_err = None;
    for attempt in 0..8 {
        match fs::remove_dir_all(dir) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                if attempt < 7 {
                    std::thread::sleep(Duration::from_millis(60));
                }
            }
        }
    }
    Err(last_err.unwrap())
}

fn extract_and_install_archive(
    archive_path: &Path,
    install_dir: &Path,
    staging_dir: &Path,
    backup_dir: &Path,
) -> Result<(), String> {
    if staging_dir.exists() {
        let _ = fs::remove_dir_all(staging_dir);
    }
    fs::create_dir_all(staging_dir).map_err(|e| format!("创建临时解压目录失败: {e}"))?;

    sevenz_rust2::decompress_file(archive_path, staging_dir)
        .map_err(|e| format!("解压 RapidOCR 7z 压缩包失败: {e}"))?;

    if discover_installed_engine(staging_dir).is_none() {
        return Err("压缩包中未检测到有效的 RapidOCR-json.exe 或 models 模型目录".to_string());
    }

    shutdown_worker();

    if install_dir.exists() {
        if backup_dir.exists() {
            let _ = remove_dir_with_retry(backup_dir);
        }
        rename_with_retry(install_dir, backup_dir)
            .map_err(|e| format!("替换旧版离线引擎目录失败（请关闭占用进程后重试）: {e}"))?;
    }

    if let Err(err) = rename_with_retry(staging_dir, install_dir) {
        if backup_dir.exists() && !install_dir.exists() {
            let _ = rename_with_retry(backup_dir, install_dir);
        }
        return Err(format!("安装离线 OCR 引擎到目标目录失败: {err}"));
    }

    let _ = remove_dir_with_retry(backup_dir);
    Ok(())
}

/// Return current RapidOCR installation status and supported languages.
#[tauri::command(async)]
pub fn get_local_ocr_status(
    state: tauri::State<'_, std::sync::Mutex<crate::AppState>>,
) -> Result<LocalOcrStatus, String> {
    let active_lang = state
        .lock()
        .map(|g| normalize_ocr_lang(&g.config.ocr_lang))
        .unwrap_or_else(|_| "ch".to_string());
    build_status(active_lang)
}

/// Request cancellation of an active RapidOCR download.
#[tauri::command]
pub fn cancel_local_ocr_download() -> Result<(), String> {
    DOWNLOAD_CANCELLED.store(true, Ordering::Release);
    Ok(())
}

/// Download, verify, extract, and activate RapidOCR-json v0.2.0.
#[tauri::command]
pub async fn download_local_ocr(
    app: tauri::AppHandle,
    state: tauri::State<'_, std::sync::Mutex<crate::AppState>>,
    custom_url: Option<String>,
    proxy_url: Option<String>,
) -> Result<LocalOcrStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = (app, state, custom_url, proxy_url);
        return Err("macOS 已内置 Apple Vision 系统 OCR，无需额外下载 RapidOCR 引擎包。".to_string());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _busy = BusyGuard::try_acquire()?;
    set_last_error(None);

    let (cfg_download_url, cfg_proxy_url, active_lang) = {
        let guard = state.lock().map_err(|_| "状态锁异常".to_string())?;
        (
            guard.config.ocr_download_url.clone(),
            guard.config.proxy_url.clone(),
            normalize_ocr_lang(&guard.config.ocr_lang),
        )
    };

    let effective_url_raw = match custom_url.as_deref() {
        Some(s) => s,
        None => cfg_download_url.as_str(),
    };
    let effective_proxy = match proxy_url.as_deref() {
        Some(p) => p.trim(),
        None => cfg_proxy_url.trim(),
    };

    let result: Result<(), String> = async {
        let (url, is_official) = resolve_download_url(Some(effective_url_raw))?;
        let install_dir = ocr_engine_install_dir()?;
        let parent_dir = install_dir
            .parent()
            .ok_or_else(|| "离线引擎安装路径无效".to_string())?
            .to_path_buf();
        fs::create_dir_all(&parent_dir).map_err(|e| format!("创建引擎安装目录失败: {e}"))?;

        let stamp = unique_stamp();
        let part_path = parent_dir.join(format!(".rapidocr-{stamp}.7z.part"));
        let staging_dir = parent_dir.join(format!(".rapidocr-staging-{stamp}"));
        let backup_dir = parent_dir.join(format!(".rapidocr-old-{stamp}"));

        let mut cleanup = TempCleanupGuard::new();
        cleanup.push(part_path.clone());
        cleanup.push(staging_dir.clone());
        cleanup.push(backup_dir.clone());

        let builder = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(45))
            .timeout(Duration::from_secs(600));
        let builder = apply_proxy(builder, effective_proxy)?;
        let client = builder
            .build()
            .map_err(|e| format!("无法初始化下载网络客户端: {e}"))?;

        if DOWNLOAD_CANCELLED.load(Ordering::Acquire) {
            emit_ocr_progress(
                &app,
                &LocalOcrProgress {
                    stage: "cancelled".into(),
                    downloaded: 0,
                    total: None,
                    percent: None,
                    message: Some("已取消下载离线 OCR 引擎".into()),
                },
            );
            return Err("已取消下载离线 OCR 引擎".to_string());
        }

        emit_ocr_progress(
            &app,
            &LocalOcrProgress {
                stage: "downloading".into(),
                downloaded: 0,
                total: None,
                percent: None,
                message: Some("正在连接下载服务器…".into()),
            },
        );

        let mut response = client
            .get(url)
            .header(reqwest::header::CACHE_CONTROL, "no-cache")
            .send()
            .await
            .map_err(|err| {
                if err.is_timeout() {
                    "下载 RapidOCR 连接超时，请检查网络或代理设置后重试。".to_string()
                } else if err.is_redirect() {
                    "下载服务器重定向无效。".to_string()
                } else {
                    format!("无法连接 RapidOCR 下载服务器：{err}")
                }
            })?;

        if !response.status().is_success() {
            return Err(format!(
                "下载服务器返回 HTTP {}。",
                response.status().as_u16()
            ));
        }

        if DOWNLOAD_CANCELLED.load(Ordering::Acquire) {
            emit_ocr_progress(
                &app,
                &LocalOcrProgress {
                    stage: "cancelled".into(),
                    downloaded: 0,
                    total: response.content_length(),
                    percent: None,
                    message: Some("已取消下载离线 OCR 引擎".into()),
                },
            );
            return Err("已取消下载离线 OCR 引擎".to_string());
        }

        let total = response.content_length();
        if total.is_some_and(|len| len > MAX_ARCHIVE_BYTES) {
            return Err(format!(
                "引擎包大小超过上限（最大 {} MB）。",
                MAX_ARCHIVE_BYTES / (1024 * 1024)
            ));
        }

        let file = File::create(&part_path).map_err(|e| format!("创建临时下载文件失败: {e}"))?;
        let mut writer = BufWriter::with_capacity(64 * 1024, file);
        let mut hasher = Sha256::new();
        let mut downloaded: u64 = 0;
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        let mut last_percent: Option<u8> = None;

        emit_ocr_progress(
            &app,
            &LocalOcrProgress {
                stage: "downloading".into(),
                downloaded: 0,
                total,
                percent: total.map(|_| 0),
                message: Some("正在下载 RapidOCR ONNX 引擎包…".into()),
            },
        );

        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("下载中断，请检查网络或代理后重试：{e}"))?
        {
            if DOWNLOAD_CANCELLED.load(Ordering::Acquire) {
                emit_ocr_progress(
                    &app,
                    &LocalOcrProgress {
                        stage: "cancelled".into(),
                        downloaded,
                        total,
                        percent: None,
                        message: Some("已取消下载离线 OCR 引擎".into()),
                    },
                );
                return Err("已取消下载离线 OCR 引擎".to_string());
            }

            let next_len = downloaded.saturating_add(chunk.len() as u64);
            if next_len > MAX_ARCHIVE_BYTES {
                return Err(format!(
                    "引擎包大小超过上限（最大 {} MB）。",
                    MAX_ARCHIVE_BYTES / (1024 * 1024)
                ));
            }

            writer
                .write_all(&chunk)
                .map_err(|e| format!("写入下载缓存文件失败: {e}"))?;
            hasher.update(&chunk);
            downloaded = next_len;

            let percent = total.and_then(|len| {
                if len == 0 {
                    None
                } else {
                    Some(((downloaded.saturating_mul(100)) / len).min(100) as u8)
                }
            });
            let is_finished = total.is_some_and(|len| downloaded >= len);
            let should_emit = is_finished
                || percent != last_percent
                || last_emit.elapsed() >= PROGRESS_EMIT_INTERVAL;
            if should_emit {
                last_emit = Instant::now();
                last_percent = percent;
                emit_ocr_progress(
                    &app,
                    &LocalOcrProgress {
                        stage: "downloading".into(),
                        downloaded,
                        total,
                        percent,
                        message: Some("正在下载 RapidOCR ONNX 引擎包…".into()),
                    },
                );
            }
        }

        writer
            .flush()
            .map_err(|e| format!("刷新下载缓存文件失败: {e}"))?;
        drop(writer);

        if DOWNLOAD_CANCELLED.load(Ordering::Acquire) {
            emit_ocr_progress(
                &app,
                &LocalOcrProgress {
                    stage: "cancelled".into(),
                    downloaded,
                    total,
                    percent: None,
                    message: Some("已取消下载离线 OCR 引擎".into()),
                },
            );
            return Err("已取消下载离线 OCR 引擎".to_string());
        }

        emit_ocr_progress(
            &app,
            &LocalOcrProgress {
                stage: "verifying".into(),
                downloaded,
                total: Some(downloaded),
                percent: Some(100),
                message: Some("正在校验 SHA-256 完整性…".into()),
            },
        );

        let actual_sha = to_hex_lower(&hasher.finalize());
        if is_official {
            let expected = RAPIDOCR_OFFICIAL_SHA256.to_ascii_lowercase();
            if actual_sha != expected {
                return Err(format!(
                    "官方引擎包 SHA-256 校验不匹配（期望 {expected}，实际 {actual_sha}）"
                ));
            }
        }

        emit_ocr_progress(
            &app,
            &LocalOcrProgress {
                stage: "extracting".into(),
                downloaded,
                total: Some(downloaded),
                percent: Some(100),
                message: Some("正在解压并部署 RapidOCR ONNX 引擎…".into()),
            },
        );

        let part_clone = part_path.clone();
        let install_clone = install_dir.clone();
        let staging_clone = staging_dir.clone();
        let backup_clone = backup_dir.clone();
        tauri::async_runtime::spawn_blocking(move || {
            extract_and_install_archive(
                &part_clone,
                &install_clone,
                &staging_clone,
                &backup_clone,
            )
        })
        .await
        .map_err(|e| format!("解压线程异常: {e}"))??;

        emit_ocr_progress(
            &app,
            &LocalOcrProgress {
                stage: "ready".into(),
                downloaded,
                total: Some(downloaded),
                percent: Some(100),
                message: Some("离线 RapidOCR 引擎已就绪".into()),
            },
        );
        Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            set_last_error(None);
            // Release the busy flag before snapshotting status so the UI
            // receives `busy: false` and leaves the progress view.
            drop(_busy);
            build_status(active_lang)
        }
        Err(err) => {
            if !DOWNLOAD_CANCELLED.load(Ordering::Acquire) {
                set_last_error(Some(err.clone()));
                emit_ocr_progress(
                    &app,
                    &LocalOcrProgress {
                        stage: "error".into(),
                        downloaded: 0,
                        total: None,
                        percent: None,
                        message: Some(err.clone()),
                    },
                );
            } else {
                set_last_error(None);
            }
            Err(err)
        }
    }
    }
}

/// Import a local `.7z` RapidOCR-json archive chosen via system file picker.
#[tauri::command]
pub async fn import_local_ocr_archive(
    app: tauri::AppHandle,
    state: tauri::State<'_, std::sync::Mutex<crate::AppState>>,
) -> Result<LocalOcrStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = (app, state);
        return Err("macOS 已内置 Apple Vision 系统 OCR，无需导入 RapidOCR 引擎包。".to_string());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _busy = BusyGuard::try_acquire()?;
    let active_lang = state
        .lock()
        .map(|g| normalize_ocr_lang(&g.config.ocr_lang))
        .unwrap_or_else(|_| "ch".to_string());

    let picked = rfd::AsyncFileDialog::new()
        .add_filter("7z 压缩包", &["7z"])
        .pick_file()
        .await;
    let Some(handle) = picked else {
        drop(_busy);
        return build_status(active_lang);
    };

    set_last_error(None);
    let archive_path = handle.path().to_path_buf();

    let result: Result<(), String> = async {
        let meta =
            fs::metadata(&archive_path).map_err(|e| format!("无法读取所选 7z 文件信息: {e}"))?;
        let file_size = meta.len();
        if file_size > MAX_ARCHIVE_BYTES {
            return Err(format!(
                "所选压缩包超过大小上限（最大 {} MB）。",
                MAX_ARCHIVE_BYTES / (1024 * 1024)
            ));
        }

        let install_dir = ocr_engine_install_dir()?;
        let parent_dir = install_dir
            .parent()
            .ok_or_else(|| "离线引擎安装路径无效".to_string())?
            .to_path_buf();
        fs::create_dir_all(&parent_dir).map_err(|e| format!("创建引擎安装目录失败: {e}"))?;

        let stamp = unique_stamp();
        let staging_dir = parent_dir.join(format!(".rapidocr-staging-{stamp}"));
        let backup_dir = parent_dir.join(format!(".rapidocr-old-{stamp}"));

        let mut cleanup = TempCleanupGuard::new();
        cleanup.push(staging_dir.clone());
        cleanup.push(backup_dir.clone());

        emit_ocr_progress(
            &app,
            &LocalOcrProgress {
                stage: "extracting".into(),
                downloaded: file_size,
                total: Some(file_size),
                percent: Some(100),
                message: Some("正在校验并解压本地 7z 引擎包…".into()),
            },
        );

        let archive_clone = archive_path.clone();
        let install_clone = install_dir.clone();
        let staging_clone = staging_dir.clone();
        let backup_clone = backup_dir.clone();
        tauri::async_runtime::spawn_blocking(move || {
            extract_and_install_archive(
                &archive_clone,
                &install_clone,
                &staging_clone,
                &backup_clone,
            )
        })
        .await
        .map_err(|e| format!("解压线程异常: {e}"))??;

        emit_ocr_progress(
            &app,
            &LocalOcrProgress {
                stage: "ready".into(),
                downloaded: file_size,
                total: Some(file_size),
                percent: Some(100),
                message: Some("本地离线 RapidOCR 引擎包已导入并就绪".into()),
            },
        );
        Ok(())
    }
    .await;

    match result {
        Ok(()) => {
            set_last_error(None);
            drop(_busy);
            build_status(active_lang)
        }
        Err(err) => {
            set_last_error(Some(err.clone()));
            emit_ocr_progress(
                &app,
                &LocalOcrProgress {
                    stage: "error".into(),
                    downloaded: 0,
                    total: None,
                    percent: None,
                    message: Some(err.clone()),
                },
            );
            Err(err)
        }
    }
    }
}

/// Shut down any running RapidOCR worker and delete the installed engine directory.
#[tauri::command(async)]
pub fn uninstall_local_ocr(
    state: tauri::State<'_, std::sync::Mutex<crate::AppState>>,
) -> Result<LocalOcrStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = state;
        return Err("macOS 使用系统内置 Apple Vision OCR，不支持卸载。".to_string());
    }
    #[cfg(not(target_os = "macos"))]
    {
        if DOWNLOAD_BUSY.load(Ordering::Acquire) {
            return Err("正在下载或安装离线 OCR 引擎，请先取消后再卸载。".to_string());
        }
        let active_lang = state
            .lock()
            .map(|g| normalize_ocr_lang(&g.config.ocr_lang))
            .unwrap_or_else(|_| "ch".to_string());

        shutdown_worker();
        let install_dir = ocr_engine_install_dir()?;
        remove_dir_with_retry(&install_dir)
            .map_err(|e| format!("卸载离线 OCR 引擎目录失败: {e}"))?;
        set_last_error(None);
        build_status(active_lang)
    }
}

fn resolve_lang_spec<'a>(
    installed: &'a InstalledRapidOcr,
    requested_lang: &str,
) -> Result<(String, &'a RapidOcrModelSpec), String> {
    let norm = normalize_ocr_lang(requested_lang);
    if let Some(spec) = installed.lang_models.get(&norm) {
        return Ok((norm, spec));
    }
    if let Some(spec) = installed.lang_models.get("ch") {
        return Ok(("ch".to_string(), spec));
    }
    for lang in CANONICAL_LANGS {
        if let Some(spec) = installed.lang_models.get(lang) {
            return Ok((lang.to_string(), spec));
        }
    }
    Err("未找到可用的 RapidOCR 语言模型".to_string())
}

/// Attach the RapidOCR worker to a process-wide Job Object configured with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. The job handle is intentionally never
/// closed: when Luma exits for any reason (including a crash or a forced
/// termination that skips `Drop`), the OS closes the handle and kills the
/// worker, so no orphaned `RapidOCR-json.exe` is left behind.
#[cfg(windows)]
fn bind_child_to_kill_on_close_job(child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    static JOB: OnceLock<Option<usize>> = OnceLock::new();
    let job = JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(None, windows::core::PCWSTR::null()).ok()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .ok()?;
        Some(job.0 as usize)
    });
    if let Some(raw) = *job {
        let process = HANDLE(child.as_raw_handle() as _);
        if let Err(err) = unsafe { AssignProcessToJobObject(HANDLE(raw as _), process) } {
            eprintln!("RapidOCR worker job binding failed: {err}");
        }
    }
}

fn spawn_rapidocr_worker(
    installed: &InstalledRapidOcr,
    requested_lang: &str,
) -> Result<RapidOcrWorker, String> {
    let (resolved_lang, spec) = resolve_lang_spec(installed, requested_lang)?;
    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 6);

    let mut cmd = Command::new(&installed.exe_path);
    cmd.current_dir(&installed.exe_dir)
        .arg(format!("--models={}", installed.models_rel_arg))
        .arg(format!("--det={}", spec.det));
    if let Some(cls) = &spec.cls {
        cmd.arg(format!("--cls={cls}"));
    }
    cmd.arg(format!("--rec={}", spec.rec))
        .arg(format!("--keys={}", spec.keys))
        .arg("--ensureAscii=1")
        .arg(format!("--numThread={num_threads}"))
        .arg("--doAngle=1")
        .arg("--mostAngle=1")
        .arg("--padding=50")
        .arg("--maxSideLen=1600")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 RapidOCR 进程失败: {e}"))?;
    #[cfg(windows)]
    bind_child_to_kill_on_close_job(&child);
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "无法获取 RapidOCR 标准输入管道".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法获取 RapidOCR 标准输出管道".to_string())?;
    let stderr = child.stderr.take();

    let (stdout_tx, stdout_rx) = mpsc::channel::<Option<String>>();
    std::thread::Builder::new()
        .name("luma-rapidocr-stdout".to_string())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = stdout_tx.send(None);
                        break;
                    }
                    Ok(_) => {
                        if stdout_tx.send(Some(line)).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = stdout_tx.send(None);
                        break;
                    }
                }
            }
        })
        .map_err(|e| format!("创建 RapidOCR 输出读取线程失败: {e}"))?;

    let stderr_tail = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
    if let Some(err_pipe) = stderr {
        let tail_ref = stderr_tail.clone();
        let _ = std::thread::Builder::new()
            .name("luma-rapidocr-stderr".to_string())
            .spawn(move || {
                let reader = BufReader::new(err_pipe);
                for line in reader.lines().map_while(Result::ok) {
                    let trimmed = line.trim().to_string();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(mut buf) = tail_ref.lock() {
                        if buf.len() >= 12 {
                            buf.remove(0);
                        }
                        buf.push(trimmed);
                    }
                }
            });
    }

    let worker = RapidOcrWorker {
        lang: resolved_lang,
        exe_path: installed.exe_path.clone(),
        child,
        stdin: Some(stdin),
        stdout_rx,
        stderr_tail,
    };

    let deadline = Instant::now() + WORKER_INIT_TIMEOUT;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err("RapidOCR 引擎初始化超时（15 秒未就绪）".to_string());
        }
        let remain = deadline - now;
        match worker.stdout_rx.recv_timeout(remain) {
            Ok(Some(line)) => {
                if line.contains("OCR init completed.") {
                    LAST_USED_MS.store(mono_ms(), Ordering::Release);
                    ensure_idle_watcher();
                    return Ok(worker);
                }
            }
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                let tail = worker
                    .stderr_tail
                    .lock()
                    .ok()
                    .map(|v| v.join(" | "))
                    .unwrap_or_default();
                let detail = if tail.is_empty() {
                    "进程在初始化完成前意外退出".to_string()
                } else {
                    tail
                };
                return Err(format!("RapidOCR 引擎初始化失败：{detail}"));
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err("RapidOCR 引擎初始化超时（15 秒未就绪）".to_string());
            }
        }
    }
}

enum WorkerInferError {
    Broken(String),
    Ocr(String),
}

fn infer_with_worker(
    worker: &mut RapidOcrWorker,
    b64_png: &str,
) -> Result<Vec<RapidOcrItem>, WorkerInferError> {
    if let Ok(Some(status)) = worker.child.try_wait() {
        return Err(WorkerInferError::Broken(format!(
            "RapidOCR 进程已退出 ({status})"
        )));
    }
    let stdin = worker
        .stdin
        .as_mut()
        .ok_or_else(|| WorkerInferError::Broken("RapidOCR 标准输入已关闭".to_string()))?;

    let payload = format!("{{\"image_base64\":\"{b64_png}\"}}\n");
    stdin
        .write_all(payload.as_bytes())
        .and_then(|()| stdin.flush())
        .map_err(|e| WorkerInferError::Broken(format!("写入 RapidOCR 请求失败: {e}")))?;

    let deadline = Instant::now() + WORKER_INFER_TIMEOUT;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(WorkerInferError::Broken(
                "RapidOCR 识别超时（30 秒无响应）".to_string(),
            ));
        }
        let remain = deadline - now;
        match worker.stdout_rx.recv_timeout(remain) {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() || !trimmed.starts_with('{') {
                    continue;
                }
                LAST_USED_MS.store(mono_ms(), Ordering::Release);
                return parse_rapidocr_response(trimmed).map_err(WorkerInferError::Ocr);
            }
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                return Err(WorkerInferError::Broken(
                    "RapidOCR 进程输出流意外关闭".to_string(),
                ));
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(WorkerInferError::Broken(
                    "RapidOCR 识别超时（30 秒无响应）".to_string(),
                ));
            }
        }
    }
}

fn run_worker_ocr(
    installed: &InstalledRapidOcr,
    requested_lang: &str,
    png_bytes: &[u8],
) -> Result<Vec<RapidOcrItem>, String> {
    let (resolved_lang, _) = resolve_lang_spec(installed, requested_lang)?;
    let b64 = BASE64.encode(png_bytes);
    let mut guard = worker_slot()
        .lock()
        .map_err(|_| "RapidOCR 进程锁异常".to_string())?;

    for attempt in 0..2 {
        let needs_spawn = match guard.as_mut() {
            Some(w) => {
                w.lang != resolved_lang
                    || w.exe_path != installed.exe_path
                    || w.child.try_wait().ok().flatten().is_some()
            }
            None => true,
        };
        if needs_spawn {
            let _ = guard.take();
            let worker = spawn_rapidocr_worker(installed, &resolved_lang)?;
            *guard = Some(worker);
        }

        let worker = guard.as_mut().expect("worker initialized");
        match infer_with_worker(worker, &b64) {
            Ok(items) => {
                LAST_USED_MS.store(mono_ms(), Ordering::Release);
                return Ok(items);
            }
            Err(WorkerInferError::Ocr(err)) => return Err(err),
            Err(WorkerInferError::Broken(err)) => {
                let _ = guard.take();
                if attempt == 1 {
                    return Err(err);
                }
            }
        }
    }
    Err("RapidOCR 引擎执行失败".to_string())
}

/// Parse a single JSON output line emitted by RapidOCR-json.
pub fn parse_rapidocr_response(raw_json: &str) -> Result<Vec<RapidOcrItem>, String> {
    let parsed: RapidOcrRawResponse = serde_json::from_str(raw_json.trim())
        .map_err(|e| format!("解析 RapidOCR 响应 JSON 失败: {e}"))?;
    match parsed.code {
        100 => {
            if parsed.data.is_null() {
                return Ok(Vec::new());
            }
            serde_json::from_value::<Vec<RapidOcrItem>>(parsed.data)
                .map_err(|e| format!("解析 RapidOCR 识别框数据失败: {e}"))
        }
        101 => Ok(Vec::new()),
        code => {
            let detail = parsed
                .data
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| parsed.msg.filter(|s| !s.trim().is_empty()))
                .or_else(|| parsed.message.filter(|s| !s.trim().is_empty()))
                .unwrap_or_else(|| parsed.data.to_string());
            Err(format!("RapidOCR 识别返回错误 (code {code}): {detail}"))
        }
    }
}

fn is_cjk_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x4E00..=0x9FFF
            | 0x3400..=0x4DBF
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
            | 0xFF00..=0xFFEF
    )
}

#[derive(Debug, Clone)]
struct NormToken {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: String,
}

impl NormToken {
    fn right(&self) -> f32 {
        self.x + self.w
    }

    fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

#[derive(Debug, Clone)]
struct RowCell {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: String,
}

impl RowCell {
    fn right(&self) -> f32 {
        self.x + self.w
    }

    fn center_x(&self) -> f32 {
        self.x + self.w * 0.5
    }
}

fn join_tokens_smart(tokens: &[&NormToken]) -> String {
    let mut out = String::new();
    for tok in tokens {
        let piece = tok.text.trim();
        if piece.is_empty() {
            continue;
        }
        if out.is_empty() {
            out.push_str(piece);
            continue;
        }
        let prev_ch = out.chars().last();
        let next_ch = piece.chars().next();
        let prev_cjk = prev_ch.is_some_and(is_cjk_char);
        let next_cjk = next_ch.is_some_and(is_cjk_char);
        let no_space_before = matches!(
            next_ch,
            Some(
                ',' | '.'
                    | ';'
                    | ':'
                    | '!'
                    | '?'
                    | ')'
                    | ']'
                    | '}'
                    | '%'
                    | '/'
                    | '_'
                    | '，'
                    | '。'
                    | '；'
                    | '：'
                    | '！'
                    | '？'
                    | '）'
                    | '】'
                    | '、'
                    | '．'
                    | '％'
            )
        );
        let no_space_after = matches!(
            prev_ch,
            Some('(' | '[' | '{' | '/' | '_' | '（' | '【' | '“' | '‘')
        );
        if (prev_cjk && next_cjk) || no_space_before || no_space_after {
            out.push_str(piece);
        } else {
            out.push(' ');
            out.push_str(piece);
        }
    }
    out
}

fn build_cell_from_tokens(tokens: &[&NormToken]) -> RowCell {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_r = f32::NEG_INFINITY;
    let mut max_b = f32::NEG_INFINITY;
    for t in tokens {
        min_x = min_x.min(t.x);
        min_y = min_y.min(t.y);
        max_r = max_r.max(t.right());
        max_b = max_b.max(t.bottom());
    }
    RowCell {
        x: min_x.clamp(0.0, 1.0),
        y: min_y.clamp(0.0, 1.0),
        w: (max_r - min_x).max(0.005),
        h: (max_b - min_y).max(0.005),
        text: join_tokens_smart(tokens),
    }
}

fn cells_align_as_table_column(a: &RowCell, b: &RowCell, tol: f32) -> bool {
    let left_aligned = (a.x - b.x).abs() <= tol;
    let right_aligned = (a.right() - b.right()).abs() <= tol;
    let center_aligned = (a.center_x() - b.center_x()).abs() <= tol;
    let overlap = a.right().min(b.right()) - a.x.max(b.x);
    left_aligned || right_aligned || center_aligned || overlap > 0.0
}

fn rows_have_aligned_columns(a: &[RowCell], b: &[RowCell], avg_h: f32) -> bool {
    if a.len() < 2 || b.len() < 2 {
        return false;
    }
    let tol = (avg_h * 2.2).max(0.03);
    // Reject cases where one wide cell straddles >= 2 cells of the other row.
    for cell_a in a {
        let overlapped_b = b
            .iter()
            .filter(|cb| cell_a.right().min(cb.right()) - cell_a.x.max(cb.x) > 0.002)
            .count();
        if overlapped_b >= 2 {
            return false;
        }
    }
    for cell_b in b {
        let overlapped_a = a
            .iter()
            .filter(|ca| cell_b.right().min(ca.right()) - cell_b.x.max(ca.x) > 0.002)
            .count();
        if overlapped_a >= 2 {
            return false;
        }
    }

    let mut matched = 0usize;
    let mut b_idx = 0usize;
    for cell_a in a {
        while b_idx < b.len() && b[b_idx].right() + tol < cell_a.x {
            b_idx += 1;
        }
        if b_idx < b.len() && cells_align_as_table_column(cell_a, &b[b_idx], tol) {
            matched += 1;
            b_idx += 1;
        }
    }
    matched >= 2
}

/// Convert RapidOCR detection/recognition items into layout-aware text and `OcrLayoutBlock`s.
pub fn parse_and_layout_rapidocr_result(
    items: Vec<RapidOcrItem>,
    img_w: u32,
    img_h: u32,
) -> (String, String, Vec<OcrLayoutBlock>) {
    let w_f = img_w.max(1) as f32;
    let h_f = img_h.max(1) as f32;

    let mut tokens: Vec<NormToken> = Vec::with_capacity(items.len());
    for item in items {
        if item.score < 0.45 {
            continue;
        }
        let trimmed = item.text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for pt in item.box_pts {
            if pt[0].is_finite() && pt[1].is_finite() {
                min_x = min_x.min(pt[0]);
                min_y = min_y.min(pt[1]);
                max_x = max_x.max(pt[0]);
                max_y = max_y.max(pt[1]);
            }
        }
        if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
            continue;
        }
        let nx = (min_x / w_f).clamp(0.0, 1.0);
        let ny = (min_y / h_f).clamp(0.0, 1.0);
        let nr = (max_x / w_f).clamp(nx, 1.0);
        let nb = (max_y / h_f).clamp(ny, 1.0);
        let nw = (nr - nx).max(0.005);
        let nh = (nb - ny).max(0.005);
        tokens.push(NormToken {
            x: nx,
            y: ny,
            w: nw,
            h: nh,
            text: trimmed.to_string(),
        });
    }

    if tokens.is_empty() {
        return (guess_lang_from_text(""), String::new(), Vec::new());
    }

    tokens.sort_by(|a, b| a.y.total_cmp(&b.y).then_with(|| a.x.total_cmp(&b.x)));

    // Cluster tokens into visual rows (>45% vertical overlap).
    struct VisualRow {
        tokens: Vec<NormToken>,
        sum_y: f32,
        sum_b: f32,
        sum_h: f32,
    }
    let mut rows: Vec<VisualRow> = Vec::new();
    for tok in tokens {
        let mut target_row: Option<usize> = None;
        for (ri, row) in rows.iter().enumerate().rev() {
            let cnt = row.tokens.len() as f32;
            let row_y = row.sum_y / cnt;
            let row_b = row.sum_b / cnt;
            let row_h = (row.sum_h / cnt).max(0.001);
            let overlap = tok.bottom().min(row_b) - tok.y.max(row_y);
            let min_h = tok.h.min(row_h).max(0.001);
            if overlap > min_h * 0.45 {
                target_row = Some(ri);
                break;
            }
        }
        if let Some(ri) = target_row {
            let row = &mut rows[ri];
            row.sum_y += tok.y;
            row.sum_b += tok.bottom();
            row.sum_h += tok.h;
            row.tokens.push(tok);
        } else {
            rows.push(VisualRow {
                sum_y: tok.y,
                sum_b: tok.bottom(),
                sum_h: tok.h,
                tokens: vec![tok],
            });
        }
    }

    rows.sort_by(|a, b| {
        let ay = a.sum_y / a.tokens.len() as f32;
        let by = b.sum_y / b.tokens.len() as f32;
        ay.total_cmp(&by)
    });

    struct RowInfo {
        avg_y: f32,
        avg_bottom: f32,
        avg_h: f32,
        tokens: Vec<NormToken>,
        cells: Vec<RowCell>,
    }

    let mut row_infos: Vec<RowInfo> = Vec::with_capacity(rows.len());
    for mut row in rows {
        row.tokens.sort_by(|a, b| a.x.total_cmp(&b.x));
        let cnt = row.tokens.len() as f32;
        let avg_y = row.sum_y / cnt;
        let avg_bottom = row.sum_b / cnt;
        let avg_h = (row.sum_h / cnt).max(0.001);

        let mut cells = Vec::new();
        let mut cell_tokens: Vec<&NormToken> = Vec::new();
        for tok in &row.tokens {
            if let Some(prev) = cell_tokens.last().copied() {
                let gap = tok.x - prev.right();
                if gap > avg_h * 1.4 {
                    cells.push(build_cell_from_tokens(&cell_tokens));
                    cell_tokens.clear();
                }
            }
            cell_tokens.push(tok);
        }
        if !cell_tokens.is_empty() {
            cells.push(build_cell_from_tokens(&cell_tokens));
        }

        row_infos.push(RowInfo {
            avg_y,
            avg_bottom,
            avg_h,
            tokens: row.tokens,
            cells,
        });
    }

    let mut is_table_row = vec![false; row_infos.len()];
    let mut i = 0usize;
    while i + 1 < row_infos.len() {
        let row_gap = row_infos[i + 1].avg_y - row_infos[i].avg_bottom;
        let pair_avg_h = (row_infos[i].avg_h + row_infos[i + 1].avg_h) * 0.5;
        if row_gap <= pair_avg_h * 4.0
            && rows_have_aligned_columns(&row_infos[i].cells, &row_infos[i + 1].cells, pair_avg_h)
        {
            is_table_row[i] = true;
            is_table_row[i + 1] = true;
            let mut j = i + 1;
            while j + 1 < row_infos.len() {
                let next_gap = row_infos[j + 1].avg_y - row_infos[j].avg_bottom;
                let next_avg_h = (row_infos[j].avg_h + row_infos[j + 1].avg_h) * 0.5;
                if next_gap <= next_avg_h * 4.0
                    && rows_have_aligned_columns(
                        &row_infos[j].cells,
                        &row_infos[j + 1].cells,
                        next_avg_h,
                    )
                {
                    is_table_row[j + 1] = true;
                    j += 1;
                } else {
                    break;
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }

    let mut out_lines: Vec<String> = Vec::with_capacity(row_infos.len());
    let mut line_boxes: Vec<OcrLineBox> = Vec::with_capacity(row_infos.len());

    for (idx, row) in row_infos.iter().enumerate() {
        if is_table_row[idx] {
            let joined = row
                .cells
                .iter()
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
                .join("\t");
            if !joined.trim().is_empty() {
                out_lines.push(joined);
            }
            for cell in &row.cells {
                if !cell.text.trim().is_empty() {
                    line_boxes.push(OcrLineBox {
                        x: cell.x,
                        y: cell.y,
                        w: cell.w,
                        h: cell.h,
                        text: cell.text.clone(),
                        had_leading_icon: false,
                    });
                }
            }
        } else {
            let refs: Vec<&NormToken> = row.tokens.iter().collect();
            let row_cell = build_cell_from_tokens(&refs);
            if !row_cell.text.trim().is_empty() {
                out_lines.push(row_cell.text.clone());
                line_boxes.push(OcrLineBox {
                    x: row_cell.x,
                    y: row_cell.y,
                    w: row_cell.w,
                    h: row_cell.h,
                    text: row_cell.text,
                    had_leading_icon: false,
                });
            }
        }
    }

    let text = out_lines.join("\n");
    let blocks = group_ocr_lines_into_blocks(line_boxes);
    let detected = guess_lang_from_text(&text);
    (detected, text, blocks)
}

fn ocr_png_sync(
    png_bytes: &[u8],
    lang: &str,
) -> Result<(String, String, Vec<OcrLayoutBlock>), String> {
    let img =
        image::load_from_memory(png_bytes).map_err(|e| format!("解析 PNG 图像失败: {e}"))?;
    let img_w = img.width();
    let img_h = img.height();

    let install_dir = ocr_engine_install_dir()?;
    let installed = discover_installed_engine(&install_dir)
        .ok_or_else(|| "离线 RapidOCR 引擎尚未安装，请先在设置中下载或导入".to_string())?;

    let items = run_worker_ocr(&installed, lang, png_bytes)?;
    Ok(parse_and_layout_rapidocr_result(items, img_w, img_h))
}

/// Run offline RapidOCR on raw PNG bytes and reconstruct layout blocks.
pub async fn ocr_png_bytes_with_layout(
    png_bytes: Vec<u8>,
    lang: String,
) -> Result<(String, String, Vec<OcrLayoutBlock>), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = lang;
        return crate::win_ocr::ocr_png_bytes_with_layout(png_bytes).await;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let norm_lang = normalize_ocr_lang(&lang);
        tauri::async_runtime::spawn_blocking(move || ocr_png_sync(&png_bytes, &norm_lang))
            .await
            .map_err(|e| format!("RapidOCR 执行线程异常: {e}"))?
    }
}

fn bitmap_glyph_5x7(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '0' => [0b01110, 0b10011, 0b10101, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100],
        ':' => [0b00000, 0b01100, 0b01100, 0b00000, 0b01100, 0b01100, 0b00000],
        _ => [0; 7],
    }
}

/// Render crisp synthetic white-background black-text PNG bytes for offline OCR self-testing.
pub(crate) fn render_synthetic_test_png(text: &str) -> Result<Vec<u8>, String> {
    const IMG_W: u32 = 360;
    const IMG_H: u32 = 90;
    const SCALE: u32 = 4;
    const GLYPH_W: u32 = 5 * SCALE;
    const GLYPH_H: u32 = 7 * SCALE;
    const CHAR_GAP: u32 = SCALE;
    const SPACE_ADV: u32 = 4 * SCALE;

    let chars: Vec<char> = text.chars().collect();
    let mut total_w = 0u32;
    for (idx, &ch) in chars.iter().enumerate() {
        if ch == ' ' {
            total_w += SPACE_ADV;
        } else {
            total_w += GLYPH_W;
        }
        if idx + 1 < chars.len() {
            total_w += CHAR_GAP;
        }
    }

    let mut img = RgbaImage::from_pixel(IMG_W, IMG_H, Rgba([255, 255, 255, 255]));
    let mut cursor_x = IMG_W.saturating_sub(total_w) / 2;
    let origin_y = IMG_H.saturating_sub(GLYPH_H) / 2;

    for (idx, &ch) in chars.iter().enumerate() {
        if ch == ' ' {
            cursor_x += SPACE_ADV;
        } else {
            let rows = bitmap_glyph_5x7(ch);
            for (ry, &bits) in rows.iter().enumerate() {
                for rx in 0..5u32 {
                    if (bits >> (4 - rx)) & 1 == 1 {
                        for sy in 0..SCALE {
                            for sx in 0..SCALE {
                                let px = cursor_x + rx * SCALE + sx;
                                let py = origin_y + (ry as u32) * SCALE + sy;
                                if px < IMG_W && py < IMG_H {
                                    img.put_pixel(px, py, Rgba([0, 0, 0, 255]));
                                }
                            }
                        }
                    }
                }
            }
            cursor_x += GLYPH_W;
        }
        if idx + 1 < chars.len() {
            cursor_x += CHAR_GAP;
        }
    }

    let mut buf = Vec::with_capacity(4096);
    let encoder =
        PngEncoder::new_with_quality(&mut buf, CompressionType::Fast, FilterType::NoFilter);
    encoder
        .write_image(img.as_raw(), IMG_W, IMG_H, ExtendedColorType::Rgba8)
        .map_err(|e| format!("生成测试 PNG 失败: {e}"))?;
    Ok(buf)
}

/// Generate a synthetic test PNG and run offline RapidOCR to verify installation health.
#[tauri::command]
pub async fn test_local_ocr(
    state: tauri::State<'_, std::sync::Mutex<crate::AppState>>,
    lang: Option<String>,
) -> Result<String, String> {
    let chosen_lang = match lang {
        Some(l) if !l.trim().is_empty() => normalize_ocr_lang(&l),
        _ => state
            .lock()
            .map(|g| normalize_ocr_lang(&g.config.ocr_lang))
            .unwrap_or_else(|_| "ch".to_string()),
    };

    let png_bytes = render_synthetic_test_png("LUMA OCR 2026")?;
    let started = Instant::now();
    let (detected_lang, text, _blocks) =
        ocr_png_bytes_with_layout(png_bytes, chosen_lang.clone()).await?;
    let elapsed_ms = started.elapsed().as_millis();
    let recognized = text.trim();
    if recognized.is_empty() {
        return Err(format!(
            "离线 OCR ({chosen_lang}) 未识别到测试文字（耗时 {elapsed_ms}ms）"
        ));
    }
    #[cfg(target_os = "macos")]
    return Ok(format!(
        "Apple Vision 系统识字正常 · 耗时 {elapsed_ms}ms · 识别语种 {detected_lang} · 结果：「{recognized}」"
    ));
    #[cfg(not(target_os = "macos"))]
    Ok(format!(
        "RapidOCR 离线识字正常 · 模型 {chosen_lang} · 耗时 {elapsed_ms}ms · 识别语种 {detected_lang} · 结果：「{recognized}」"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("luma-local-ocr-test-{name}-{}", unique_stamp()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_rapidocr_json_responses() {
        let ok_json = r#"{"code":100,"data":[{"box":[[10,20],[110,20],[110,50],[10,50]],"score":0.97,"text":"Hello"},{"box":[[10,60],[120,60],[120,90],[10,90]],"score":0.30,"text":"Noise"}]}"#;
        let items = parse_rapidocr_response(ok_json).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "Hello");
        assert!((items[0].score - 0.97).abs() < 1e-4);

        let empty_json = r#"{"code":101,"data":"No text found in image."}"#;
        let empty = parse_rapidocr_response(empty_json).unwrap();
        assert!(empty.is_empty());

        let err_json = r#"{"code":203,"data":"Invalid base64 image"}"#;
        let err = parse_rapidocr_response(err_json).unwrap_err();
        assert!(err.contains("203"));
        assert!(err.contains("Invalid base64 image"));
    }

    #[test]
    fn normalizes_boxes_and_reconstructs_aligned_tables() {
        let items = vec![
            // Noise item filtered by score < 0.45
            RapidOcrItem {
                box_pts: [[10.0, 10.0], [40.0, 10.0], [40.0, 20.0], [10.0, 20.0]],
                score: 0.25,
                text: "BAD".into(),
            },
            // Regular title line
            RapidOcrItem {
                box_pts: [[40.0, 20.0], [140.0, 20.0], [140.0, 44.0], [40.0, 44.0]],
                score: 0.98,
                text: "成绩".into(),
            },
            RapidOcrItem {
                box_pts: [[142.0, 20.0], [240.0, 20.0], [240.0, 44.0], [142.0, 44.0]],
                score: 0.99,
                text: "汇总".into(),
            },
            // Table row 1 (y = 100..124)
            RapidOcrItem {
                box_pts: [[40.0, 100.0], [120.0, 100.0], [120.0, 124.0], [40.0, 124.0]],
                score: 0.96,
                text: "Name".into(),
            },
            RapidOcrItem {
                box_pts: [[300.0, 100.0], [380.0, 100.0], [380.0, 124.0], [300.0, 124.0]],
                score: 0.95,
                text: "Score".into(),
            },
            RapidOcrItem {
                box_pts: [[560.0, 100.0], [640.0, 100.0], [640.0, 124.0], [560.0, 124.0]],
                score: 0.97,
                text: "Grade".into(),
            },
            // Table row 2 (y = 148..172)
            RapidOcrItem {
                box_pts: [[40.0, 148.0], [110.0, 148.0], [110.0, 172.0], [40.0, 172.0]],
                score: 0.97,
                text: "Alice".into(),
            },
            RapidOcrItem {
                box_pts: [[315.0, 148.0], [365.0, 148.0], [365.0, 172.0], [315.0, 172.0]],
                score: 0.98,
                text: "98".into(),
            },
            RapidOcrItem {
                box_pts: [[580.0, 148.0], [620.0, 148.0], [620.0, 172.0], [580.0, 172.0]],
                score: 0.98,
                text: "A+".into(),
            },
        ];

        let (_lang, text, blocks) = parse_and_layout_rapidocr_result(items, 1000, 500);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "成绩汇总");
        assert_eq!(lines[1], "Name\tScore\tGrade");
        assert_eq!(lines[2], "Alice\t98\tA+");
        assert!(!text.contains("BAD"));
        assert!(blocks.len() >= 7);
        assert!(blocks.iter().all(|b| b.x >= 0.0 && b.x <= 1.0 && b.w > 0.0 && b.h > 0.0));
    }

    #[test]
    fn discovers_multilingual_models_and_prefers_v4_for_ch() {
        let root = make_temp_dir("discover");
        let _cleanup = {
            let mut g = TempCleanupGuard::new();
            g.push(root.clone());
            g
        };

        let app_dir = root.join("RapidOCR-json_v0.2.0");
        let models_dir = app_dir.join("models");
        fs::create_dir_all(&models_dir).unwrap();
        fs::write(app_dir.join("RapidOCR-json.exe"), b"MZ").unwrap();

        let files = [
            "ch_PP-OCRv3_det_infer.onnx",
            "ch_PP-OCRv4_det_infer.onnx",
            "ch_ppocr_mobile_v2.0_cls_infer.onnx",
            "ch_PP-OCRv3_rec_infer.onnx",
            "ch_PP-OCRv4_rec_infer.onnx",
            "ppocr_keys_v1.txt",
            "chinese_cht_PP-OCRv3_rec_infer.onnx",
            "chinese_cht_dict.txt",
            "en_PP-OCRv3_rec_infer.onnx",
            "en_dict.txt",
            "japan_PP-OCRv3_rec_infer.onnx",
            "japan_dict.txt",
            "korean_PP-OCRv3_rec_infer.onnx",
            "korean_dict.txt",
            "cyrillic_PP-OCRv3_rec_infer.onnx",
            "cyrillic_dict.txt",
        ];
        for f in files {
            fs::write(models_dir.join(f), b"stub").unwrap();
        }
        fs::write(
            models_dir.join("cmd.txt"),
            "ch: --det=ch_PP-OCRv3_det_infer.onnx --cls=ch_ppocr_mobile_v2.0_cls_infer.onnx --rec=ch_PP-OCRv3_rec_infer.onnx --keys=ppocr_keys_v1.txt\n",
        )
        .unwrap();

        let discovered = discover_installed_engine(&root).expect("should discover engine");
        assert_eq!(discovered.models_rel_arg, "models");
        let ch = discovered.lang_models.get("ch").expect("ch spec");
        assert_eq!(ch.det, "ch_PP-OCRv4_det_infer.onnx");
        assert_eq!(ch.rec, "ch_PP-OCRv4_rec_infer.onnx");
        assert_eq!(ch.cls.as_deref(), Some("ch_ppocr_mobile_v2.0_cls_infer.onnx"));
        assert_eq!(ch.keys, "ppocr_keys_v1.txt");

        let langs = ordered_available_langs(&discovered.lang_models);
        assert_eq!(langs, vec!["ch", "cht", "en", "jp", "kr", "ru"]);
    }

    #[test]
    fn verifies_sha256_bytes_and_file() {
        let data = b"Luma RapidOCR checksum payload";
        let mut hasher = Sha256::new();
        hasher.update(data);
        let expected = to_hex_lower(&hasher.finalize());

        assert!(verify_sha256_bytes(data, &expected.to_uppercase()).is_ok());
        assert!(verify_sha256_bytes(
            data,
            "0000000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err());

        let dir = make_temp_dir("sha");
        let mut g = TempCleanupGuard::new();
        g.push(dir.clone());
        let file_path = dir.join("payload.bin");
        fs::write(&file_path, data).unwrap();
        assert!(verify_sha256_file(&file_path, &expected).is_ok());
        assert!(verify_sha256_file(
            &file_path,
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
        )
        .is_err());
    }

    #[test]
    fn generates_decodable_synthetic_test_png() {
        let png = render_synthetic_test_png("LUMA OCR 2026").unwrap();
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!(img.width(), 360);
        assert_eq!(img.height(), 90);
        let rgba = img.to_rgba8();
        let dark_pixels = rgba
            .pixels()
            .filter(|p| p[0] < 32 && p[1] < 32 && p[2] < 32)
            .count();
        assert!(dark_pixels > 400, "expected crisp glyph pixels, got {dark_pixels}");
    }

    #[test]
    #[ignore = "downloads upstream RapidOCR-json archive and runs real ONNX worker inference"]
    fn live_rapidocr_download_extract_and_infer_smoke() {
        let root = make_temp_dir("live-rapidocr");
        let mut cleanup = TempCleanupGuard::new();
        cleanup.push(root.clone());

        let archive_path = root.join("RapidOCR-json_v0.2.0.7z");
        let install_dir = root.join("install");
        let staging_dir = root.join("staging");
        let backup_dir = root.join("backup");

        if let Some(cached) = std::env::var_os("LUMA_RAPIDOCR_ARCHIVE").map(PathBuf::from) {
            if cached.is_file() {
                fs::copy(&cached, &archive_path).expect("copy cached RapidOCR archive");
            }
        }
        if !archive_path.is_file() {
            tauri::async_runtime::block_on(async {
                let client = apply_proxy(
                    reqwest::Client::builder()
                        .https_only(true)
                        .connect_timeout(Duration::from_secs(20))
                        .timeout(Duration::from_secs(300)),
                    "",
                )
                .unwrap()
                .build()
                .unwrap();
                let resp = client.get(RAPIDOCR_OFFICIAL_URL).send().await.unwrap();
                assert!(
                    resp.status().is_success(),
                    "HTTP status {} downloading RapidOCR",
                    resp.status()
                );
                let bytes = resp.bytes().await.unwrap();
                println!("Downloaded RapidOCR archive: {} bytes", bytes.len());
                fs::write(&archive_path, &bytes).unwrap();
            });
        }

        let sha = verify_sha256_file(&archive_path, RAPIDOCR_OFFICIAL_SHA256)
            .expect("verify upstream RapidOCR SHA-256");
        println!("Verified SHA-256: {sha}");

        extract_and_install_archive(&archive_path, &install_dir, &staging_dir, &backup_dir)
            .expect("extract RapidOCR archive");

        let installed =
            discover_installed_engine(&install_dir).expect("discover installed RapidOCR engine");
        let langs = ordered_available_langs(&installed.lang_models);
        println!(
            "Discovered exe={} models_rel={} langs={:?}",
            installed.exe_path.display(),
            installed.models_rel_arg,
            langs
        );

        let png = render_synthetic_test_png("LUMA OCR 2026").unwrap();
        let items = run_worker_ocr(&installed, "ch", &png).expect("real RapidOCR ONNX inference");
        let (lang, text, blocks) = parse_and_layout_rapidocr_result(items, 360, 90);
        println!(
            "RapidOCR live output: lang={lang} blocks={} text={text:?}",
            blocks.len()
        );

        shutdown_worker();

        assert!(!text.trim().is_empty(), "expected non-empty OCR output");
        let upper = text.to_ascii_uppercase();
        assert!(
            upper.contains("LUMA") || upper.contains("OCR") || upper.contains("2026"),
            "unexpected OCR text: {text}"
        );
    }
}

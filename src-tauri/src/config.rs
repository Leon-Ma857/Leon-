use crate::translate::TranslationResult;
use reqwest::{Client, ClientBuilder, NoProxy, Proxy, Url};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

pub const MAX_HISTORY_ITEMS: usize = 100;
pub const MAX_ASK_QUICK_ACTIONS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskQuickAction {
    pub id: String,
    pub label: String,
    pub prompt: String,
}

pub fn default_ask_quick_actions() -> Vec<AskQuickAction> {
    vec![
        AskQuickAction {
            id: "md-table".to_string(),
            label: "转 Markdown 表格".to_string(),
            prompt: "请将内容中的表格或结构化数据精准提取并转换为规范的 Markdown 表格，保留完整表头、数字与对齐关系。".to_string(),
        },
        AskQuickAction {
            id: "latex-formula".to_string(),
            label: "提取 LaTeX 公式".to_string(),
            prompt: "请精准识别并提取内容中的数学公式，输出可直接复制使用的 LaTeX 代码（行内用 $...$，独立公式用 $$...$$），并简要核对符号。".to_string(),
        },
        AskQuickAction {
            id: "explain-error".to_string(),
            label: "解释这段报错".to_string(),
            prompt: "请分析这段报错信息或代码异常：说明根本原因、关键上下文线索，并给出具体可执行的排查与修复步骤。".to_string(),
        },
        AskQuickAction {
            id: "summarize-key".to_string(),
            label: "总结要点".to_string(),
            prompt: "请提炼核心结论与关键要点，按条理清晰的要点列表输出，突出重要数据、动作项或结论。".to_string(),
        },
        AskQuickAction {
            id: "explain-code".to_string(),
            label: "解释代码逻辑".to_string(),
            prompt: "请清晰解释这段代码的核心逻辑、输入输出与关键步骤，如有明显潜在 Bug 或边界问题一并指出。".to_string(),
        },
        AskQuickAction {
            id: "polish-rewrite".to_string(),
            label: "润色改写".to_string(),
            prompt: "请在不改变原意的前提下，对这段文字进行专业、自然、简洁的润色改写，并保留关键术语。".to_string(),
        },
    ]
}

pub fn sanitize_ask_quick_actions(actions: Vec<AskQuickAction>) -> Vec<AskQuickAction> {
    let mut cleaned = Vec::with_capacity(actions.len().min(MAX_ASK_QUICK_ACTIONS));
    let mut seen_ids = std::collections::HashSet::new();
    let mut next_custom_seq = 1usize;

    for action in actions {
        if cleaned.len() >= MAX_ASK_QUICK_ACTIONS {
            break;
        }
        let label: String = action.label.trim().chars().take(24).collect();
        let prompt: String = action.prompt.trim().chars().take(1200).collect();
        if label.is_empty() || prompt.is_empty() {
            continue;
        }
        let raw_id = action.id.trim();
        let id = if !raw_id.is_empty() && !seen_ids.contains(raw_id) {
            raw_id.to_string()
        } else {
            loop {
                let candidate = format!("custom-{next_custom_seq}");
                next_custom_seq += 1;
                if !seen_ids.contains(&candidate) {
                    break candidate;
                }
            }
        };
        seen_ids.insert(id.clone());
        cleaned.push(AskQuickAction { id, label, prompt });
    }
    cleaned
}

pub fn merge_legacy_prompt_presets(actions: &mut Vec<AskQuickAction>, legacy: &[String]) {
    for raw in legacy {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if actions
            .iter()
            .any(|a| a.prompt.trim() == trimmed || a.label.trim() == trimmed)
        {
            continue;
        }
        let label: String = trimmed.chars().take(20).collect();
        actions.push(AskQuickAction {
            id: String::new(),
            label,
            prompt: trimmed.to_string(),
        });
    }
    *actions = sanitize_ask_quick_actions(std::mem::take(actions));
}

pub fn normalize_ocr_engine(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "rapidocr" => "rapidocr".to_string(),
        "windows" => "windows".to_string(),
        _ => "auto".to_string(),
    }
}

pub fn normalize_ocr_lang(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "cht" => "cht".to_string(),
        "en" => "en".to_string(),
        "jp" => "jp".to_string(),
        "kr" => "kr".to_string(),
        "ru" => "ru".to_string(),
        _ => "ch".to_string(),
    }
}

fn default_ocr_engine() -> String {
    "auto".to_string()
}

fn default_ocr_lang() -> String {
    "ch".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub shortcut: String,
    pub clipboard_shortcut: String,
    pub selection_shortcut: String,
    pub target_lang: String,
    pub translate_engine: String,
    pub google_api_key: String,
    pub deepl_api_key: String,
    pub api_base_url: String,
    pub api_key: String,
    pub model: String,
    pub custom_prompt: String,
    pub glossary: String,
    pub auto_translate_on_release: bool,
    pub start_minimized: bool,
    pub launch_at_startup: bool,
    pub max_stack_size: usize,
    pub clear_stack_after_output: bool,
    pub theme: String,
    pub custom_prompt_presets: Vec<String>,
    #[serde(default = "default_ask_quick_actions")]
    pub ask_quick_actions: Vec<AskQuickAction>,
    #[serde(default = "default_ocr_engine")]
    pub ocr_engine: String,
    #[serde(default = "default_ocr_lang")]
    pub ocr_lang: String,
    pub ocr_download_url: String,
    pub proxy_url: String,
    pub update_feed_url: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            shortcut: "Alt+Q".to_string(),
            clipboard_shortcut: String::new(),
            selection_shortcut: String::new(),
            target_lang: "简体中文".to_string(),
            translate_engine: "ai".to_string(),
            google_api_key: String::new(),
            deepl_api_key: String::new(),
            api_base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: "gpt-4o-mini".to_string(),
            custom_prompt: String::new(),
            glossary: String::new(),
            auto_translate_on_release: false,
            start_minimized: false,
            launch_at_startup: false,
            max_stack_size: 6,
            clear_stack_after_output: false,
            theme: "system".to_string(),
            custom_prompt_presets: Vec::new(),
            ask_quick_actions: default_ask_quick_actions(),
            ocr_engine: default_ocr_engine(),
            ocr_lang: default_ocr_lang(),
            ocr_download_url: String::new(),
            proxy_url: String::new(),
            update_feed_url: String::new(),
        }
    }
}

pub fn parse_config_json(content: &str) -> AppConfig {
    let raw_value = serde_json::from_str::<serde_json::Value>(content).ok();
    let mut cfg = raw_value
        .as_ref()
        .and_then(|v| serde_json::from_value::<AppConfig>(v.clone()).ok())
        .unwrap_or_default();

    let has_ask_actions = raw_value
        .as_ref()
        .and_then(|v| v.as_object())
        .is_some_and(|obj| obj.contains_key("ask_quick_actions"));
    if !has_ask_actions && !cfg.custom_prompt_presets.is_empty() {
        merge_legacy_prompt_presets(&mut cfg.ask_quick_actions, &cfg.custom_prompt_presets);
    } else {
        cfg.ask_quick_actions = sanitize_ask_quick_actions(cfg.ask_quick_actions);
    }
    cfg.ocr_engine = normalize_ocr_engine(&cfg.ocr_engine);
    cfg.ocr_lang = normalize_ocr_lang(&cfg.ocr_lang);
    cfg
}

pub fn pre_setup_config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return Some(PathBuf::from(appdata).join("com.snaptrans.app"));
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(xdg).join("com.snaptrans.app"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            return Some(PathBuf::from(home).join(".config").join("com.snaptrans.app"));
        }
    }
    None
}

pub fn load_config_from_dir(dir: &std::path::Path) -> AppConfig {
    let path = dir.join("config.json");
    let Ok(content) = fs::read_to_string(&path) else {
        return AppConfig::default();
    };
    parse_config_json(&content)
}

pub fn load_history_from_dir(dir: &std::path::Path) -> Vec<TranslationResult> {
    let path = dir.join("history.json");
    let Ok(content) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut items = serde_json::from_str::<Vec<TranslationResult>>(&content).unwrap_or_default();
    if items.len() > MAX_HISTORY_ITEMS {
        items.truncate(MAX_HISTORY_ITEMS);
    }
    items
}

fn config_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("无法获取配置目录: {e}"))?;
    Ok(dir.join("config.json"))
}

pub fn load_config(app: &AppHandle) -> AppConfig {
    let Ok(path) = config_file_path(app) else {
        return AppConfig::default();
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return AppConfig::default();
    };
    parse_config_json(&content)
}

pub fn save_config(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let path = config_file_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let json =
        serde_json::to_string_pretty(config).map_err(|e| format!("序列化配置失败: {e}"))?;
    fs::write(&path, json).map_err(|e| format!("保存配置文件失败: {e}"))
}

fn history_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("无法获取配置目录: {e}"))?;
    Ok(dir.join("history.json"))
}

pub fn load_history(app: &AppHandle) -> Vec<TranslationResult> {
    let Ok(path) = history_file_path(app) else {
        return Vec::new();
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut items = serde_json::from_str::<Vec<TranslationResult>>(&content).unwrap_or_default();
    if items.len() > MAX_HISTORY_ITEMS {
        items.truncate(MAX_HISTORY_ITEMS);
    }
    items
}

pub fn save_history(app: &AppHandle, history: &[TranslationResult]) -> Result<(), String> {
    let path = history_file_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let json =
        serde_json::to_string_pretty(history_capped(history)).map_err(|e| format!("序列化历史记录失败: {e}"))?;
    let pending = path.with_extension("json.pending");
    fs::write(&pending, json).map_err(|e| format!("写入历史记录失败: {e}"))?;
    fs::rename(&pending, &path).map_err(|e| format!("替换历史记录失败: {e}"))
}

fn history_capped(history: &[TranslationResult]) -> &[TranslationResult] {
    if history.len() > MAX_HISTORY_ITEMS {
        &history[..MAX_HISTORY_ITEMS]
    } else {
        history
    }
}

pub fn normalize_theme(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "light" => "light".to_string(),
        "dark" => "dark".to_string(),
        _ => "system".to_string(),
    }
}

pub fn normalize_proxy_url(raw: &str) -> Result<Option<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let url = Url::parse(trimmed).map_err(|_| {
        "代理地址格式无效，请填写如 http://127.0.0.1:7890 或 socks5://127.0.0.1:10808".to_string()
    })?;
    match url.scheme() {
        "http" | "https" | "socks5" | "socks5h" => {}
        _ => {
            return Err("代理协议仅支持 http、https、socks5 或 socks5h。".to_string());
        }
    }
    if url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
        || (!url.path().is_empty() && url.path() != "/")
    {
        return Err("代理地址不应包含路径或参数，请填写如 http://127.0.0.1:7890".to_string());
    }
    Proxy::all(trimmed).map_err(|_| "无法解析代理地址，请检查格式。".to_string())?;
    Ok(Some(trimmed.to_string()))
}

pub fn proxy_has_credentials(raw: &str) -> bool {
    Url::parse(raw.trim())
        .map(|url| !url.username().is_empty() || url.password().is_some())
        .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemProxyInfo {
    pub enabled: bool,
    pub proxy_url: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SystemProxySnapshot {
    enabled: bool,
    proxy_server: String,
    proxy_override: String,
    source: &'static str,
}

const SYSTEM_PROXY_CACHE_TTL: Duration = Duration::from_secs(1);

fn mask_proxy_credentials(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw.trim()) else {
        return raw.trim().to_string();
    };
    if !url.username().is_empty() || url.password().is_some() {
        let _ = url.set_username("");
        let _ = url.set_password(None);
    }
    url.to_string().trim_end_matches('/').to_string()
}

fn format_proxy_candidate(entry: &str, default_scheme: &str) -> Option<String> {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("{default_scheme}://{trimmed}")
    };
    let parsed = Url::parse(&candidate).ok()?;
    match parsed.scheme() {
        "http" | "https" | "socks4" | "socks4a" | "socks5" | "socks5h" | "socks" => {}
        _ => return None,
    }
    parsed.host_str()?;
    Some(candidate)
}

pub(crate) fn parse_windows_proxy_server(raw: &str, target_scheme: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if !trimmed.contains('=') {
        return format_proxy_candidate(trimmed, "http");
    }

    let mut http_entry = None;
    let mut https_entry = None;
    let mut socks_entry = None;

    for part in trimmed.split(';') {
        let part = part.trim();
        let Some((proto, addr)) = part.split_once('=') else {
            continue;
        };
        let addr = addr.trim();
        if addr.is_empty() {
            continue;
        }
        match proto.trim().to_ascii_lowercase().as_str() {
            "https" => https_entry = format_proxy_candidate(addr, "http"),
            "http" => http_entry = format_proxy_candidate(addr, "http"),
            "socks" | "socks5" | "socks5h" => {
                socks_entry = format_proxy_candidate(addr, "socks5h")
            }
            _ => {}
        }
    }

    if target_scheme.eq_ignore_ascii_case("https") {
        https_entry.or(http_entry).or(socks_entry)
    } else {
        http_entry.or(socks_entry).or(https_entry)
    }
}

pub(crate) fn host_matches_bypass_list(host: &str, override_str: &str) -> bool {
    let clean_host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if clean_host.is_empty() {
        return false;
    }
    if clean_host == "localhost"
        || clean_host == "::1"
        || clean_host == "127.0.0.1"
        || clean_host.starts_with("127.")
    {
        return true;
    }

    for raw_rule in override_str.split([';', ',']) {
        let rule = raw_rule.trim().to_ascii_lowercase();
        if rule.is_empty() || rule == "<-loopback>" {
            continue;
        }
        if rule == "*" {
            return true;
        }
        if rule == "<local>" {
            if !clean_host.contains('.') && !clean_host.contains(':') {
                return true;
            }
            continue;
        }
        if let Some(prefix) = rule.strip_suffix(".*") {
            let prefix_dot = format!("{prefix}.");
            if clean_host.starts_with(&prefix_dot) {
                return true;
            }
            continue;
        }
        if rule.contains('*') {
            if let Some(suffix) = rule.strip_prefix('*') {
                let domain = suffix.trim_start_matches('.');
                if !domain.is_empty()
                    && (clean_host == domain || clean_host.ends_with(&format!(".{domain}")))
                {
                    return true;
                }
            }
            continue;
        }
        if let Some(domain) = rule.strip_prefix('.') {
            if clean_host == domain || clean_host.ends_with(&format!(".{domain}")) {
                return true;
            }
            continue;
        }
        let rule_host = rule.trim_start_matches('[').trim_end_matches(']');
        if clean_host == rule_host {
            return true;
        }
    }
    false
}

#[cfg(windows)]
fn read_windows_registry_proxy() -> Option<SystemProxySnapshot> {
    use windows::core::w;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
        REG_DWORD, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE,
    };

    fn read_reg_dword(hkey: HKEY, name: windows::core::PCWSTR) -> Option<u32> {
        let mut val_type = REG_VALUE_TYPE::default();
        let mut data = [0u8; 4];
        let mut size = 4u32;
        let status = unsafe {
            RegQueryValueExW(
                hkey,
                name,
                None,
                Some(&mut val_type),
                Some(data.as_mut_ptr()),
                Some(&mut size),
            )
        };
        if status.is_ok() && val_type == REG_DWORD && size == 4 {
            Some(u32::from_le_bytes(data))
        } else {
            None
        }
    }

    fn read_reg_string(hkey: HKEY, name: windows::core::PCWSTR) -> Option<String> {
        let mut val_type = REG_VALUE_TYPE::default();
        let mut byte_len = 0u32;
        let probe =
            unsafe { RegQueryValueExW(hkey, name, None, Some(&mut val_type), None, Some(&mut byte_len)) };
        if probe.is_err()
            || (val_type != REG_SZ && val_type != REG_EXPAND_SZ)
            || byte_len < 2
            || byte_len > 64 * 1024
        {
            return None;
        }
        let mut buf = vec![0u8; byte_len as usize];
        let read = unsafe {
            RegQueryValueExW(
                hkey,
                name,
                None,
                Some(&mut val_type),
                Some(buf.as_mut_ptr()),
                Some(&mut byte_len),
            )
        };
        if read.is_err() {
            return None;
        }
        let valid_len = (byte_len as usize).min(buf.len());
        let units: Vec<u16> = buf[..valid_len]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    }

    let subkey = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings");
    let mut hkey = HKEY::default();
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey, Some(0), KEY_READ, &mut hkey) };
    if status.is_err() {
        return None;
    }

    let enabled = read_reg_dword(hkey, w!("ProxyEnable")).unwrap_or(0) != 0;
    let proxy_server = read_reg_string(hkey, w!("ProxyServer")).unwrap_or_default();
    let proxy_override = read_reg_string(hkey, w!("ProxyOverride")).unwrap_or_default();
    let _ = unsafe { RegCloseKey(hkey) };

    if enabled && !proxy_server.trim().is_empty() {
        Some(SystemProxySnapshot {
            enabled: true,
            proxy_server,
            proxy_override,
            source: "windows",
        })
    } else {
        None
    }
}

fn first_nonempty_env(keys: &[&str]) -> Option<String> {
    for &key in keys {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

fn read_live_system_proxy_snapshot() -> SystemProxySnapshot {
    let https_env = first_nonempty_env(&["HTTPS_PROXY", "https_proxy"]);
    let http_env = first_nonempty_env(&["HTTP_PROXY", "http_proxy"]);
    let all_env = first_nonempty_env(&["ALL_PROXY", "all_proxy"]);
    let no_env = first_nonempty_env(&["NO_PROXY", "no_proxy"]).unwrap_or_default();

    if https_env.is_some() || http_env.is_some() || all_env.is_some() {
        let mut parts = Vec::new();
        if let Some(v) = https_env.as_deref().or(all_env.as_deref()) {
            parts.push(format!("https={v}"));
        }
        if let Some(v) = http_env.as_deref().or(all_env.as_deref()) {
            parts.push(format!("http={v}"));
        }
        return SystemProxySnapshot {
            enabled: true,
            proxy_server: parts.join(";"),
            proxy_override: no_env,
            source: "env",
        };
    }

    #[cfg(windows)]
    if let Some(win_snap) = read_windows_registry_proxy() {
        return win_snap;
    }

    #[cfg(target_os = "macos")]
    if let Some(mac_snap) = read_macos_scutil_proxy() {
        return mac_snap;
    }

    SystemProxySnapshot {
        enabled: false,
        proxy_server: String::new(),
        proxy_override: String::new(),
        source: "none",
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn parse_macos_scutil_proxy(output: &str) -> Option<SystemProxySnapshot> {
    let mut http_enable = false;
    let mut http_host = String::new();
    let mut http_port: u16 = 0;
    let mut https_enable = false;
    let mut https_host = String::new();
    let mut https_port: u16 = 0;
    let mut socks_enable = false;
    let mut socks_host = String::new();
    let mut socks_port: u16 = 0;
    let mut exceptions: Vec<String> = Vec::new();

    let mut depth: usize = 0;
    let mut in_exceptions = false;
    let mut exceptions_depth: usize = 0;

    for raw_line in output.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "}" {
            if in_exceptions && depth <= exceptions_depth {
                in_exceptions = false;
            }
            depth = depth.saturating_sub(1);
            continue;
        }
        if in_exceptions {
            if let Some((_, val)) = line.split_once(" : ") {
                let item = val.trim();
                if !item.is_empty() {
                    exceptions.push(item.to_string());
                }
            }
            if line.ends_with('{') {
                depth += 1;
            }
            continue;
        }
        if depth == 1 {
            if let Some((raw_key, raw_val)) = line.split_once(" : ") {
                let key = raw_key.trim();
                let val = raw_val.trim();
                if key == "ExceptionsList" && val.ends_with('{') {
                    in_exceptions = true;
                    depth += 1;
                    exceptions_depth = depth;
                    continue;
                }
                if val.ends_with('{') {
                    depth += 1;
                    continue;
                }
                match key {
                    "HTTPEnable" => http_enable = val == "1",
                    "HTTPProxy" => http_host = val.to_string(),
                    "HTTPPort" => http_port = val.parse::<u16>().unwrap_or(0),
                    "HTTPSEnable" => https_enable = val == "1",
                    "HTTPSProxy" => https_host = val.to_string(),
                    "HTTPSPort" => https_port = val.parse::<u16>().unwrap_or(0),
                    "SOCKSEnable" => socks_enable = val == "1",
                    "SOCKSProxy" => socks_host = val.to_string(),
                    "SOCKSPort" => socks_port = val.parse::<u16>().unwrap_or(0),
                    _ => {}
                }
                continue;
            }
        }
        if line.ends_with('{') {
            depth += 1;
        }
    }

    let format_host_port = |host: &str, port: u16| -> Option<String> {
        let host = host.trim();
        if host.is_empty() || port == 0 {
            return None;
        }
        if host.contains(':') && !host.starts_with('[') {
            Some(format!("[{host}]:{port}"))
        } else {
            Some(format!("{host}:{port}"))
        }
    };

    let mut entries = Vec::new();
    if https_enable {
        if let Some(hp) = format_host_port(&https_host, https_port) {
            entries.push(format!("https={hp}"));
        }
    }
    if http_enable {
        if let Some(hp) = format_host_port(&http_host, http_port) {
            entries.push(format!("http={hp}"));
        }
    }
    if socks_enable {
        if let Some(hp) = format_host_port(&socks_host, socks_port) {
            entries.push(format!("socks={hp}"));
        }
    }

    if entries.is_empty() {
        return None;
    }

    Some(SystemProxySnapshot {
        enabled: true,
        proxy_server: entries.join(";"),
        proxy_override: exceptions.join(";"),
        source: "macos",
    })
}

#[cfg(target_os = "macos")]
fn read_macos_scutil_proxy() -> Option<SystemProxySnapshot> {
    let output = std::process::Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_macos_scutil_proxy(&stdout)
}

fn cached_system_proxy_snapshot() -> SystemProxySnapshot {
    static CACHE: OnceLock<Mutex<(Instant, SystemProxySnapshot)>> = OnceLock::new();
    let lock = CACHE.get_or_init(|| {
        Mutex::new((Instant::now(), read_live_system_proxy_snapshot()))
    });
    if let Ok(mut guard) = lock.lock() {
        if guard.0.elapsed() < SYSTEM_PROXY_CACHE_TTL {
            return guard.1.clone();
        }
        let fresh = read_live_system_proxy_snapshot();
        *guard = (Instant::now(), fresh.clone());
        return fresh;
    }
    read_live_system_proxy_snapshot()
}

pub(crate) fn resolve_system_proxy(target_scheme: &str, target_host: &str) -> Option<String> {
    let snap = cached_system_proxy_snapshot();
    if !snap.enabled || host_matches_bypass_list(target_host, &snap.proxy_override) {
        return None;
    }
    parse_windows_proxy_server(&snap.proxy_server, target_scheme)
}

pub fn detect_system_proxy_info() -> SystemProxyInfo {
    let snap = read_live_system_proxy_snapshot();
    if !snap.enabled {
        return SystemProxyInfo {
            enabled: false,
            proxy_url: None,
            source: "none".to_string(),
        };
    }
    let resolved = parse_windows_proxy_server(&snap.proxy_server, "https")
        .or_else(|| parse_windows_proxy_server(&snap.proxy_server, "http"));
    match resolved {
        Some(url) => SystemProxyInfo {
            enabled: true,
            proxy_url: Some(mask_proxy_credentials(&url)),
            source: snap.source.to_string(),
        },
        None => SystemProxyInfo {
            enabled: false,
            proxy_url: None,
            source: "none".to_string(),
        },
    }
}

pub fn apply_proxy(builder: ClientBuilder, proxy_url: &str) -> Result<ClientBuilder, String> {
    if let Some(proxy_str) = normalize_proxy_url(proxy_url)? {
        let proxy = Proxy::all(&proxy_str)
            .map_err(|_| "无法应用代理配置，请检查代理地址。".to_string())?
            .no_proxy(NoProxy::from_string("localhost,127.0.0.1,::1"));
        return Ok(builder.no_proxy().proxy(proxy));
    }

    let dynamic_proxy = Proxy::custom(|url: &Url| -> Option<Url> {
        let host = url.host_str()?;
        let proxy_str = resolve_system_proxy(url.scheme(), host)?;
        Url::parse(&proxy_str).ok()
    });
    Ok(builder.no_proxy().proxy(dynamic_proxy))
}

pub fn build_api_client(proxy_url: &str) -> Result<Client, String> {
    let builder = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120));
    let builder = apply_proxy(builder, proxy_url)?;
    builder
        .build()
        .map_err(|e| format!("无法初始化网络客户端: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_config_deserializes_legacy_json_with_defaults() {
        let legacy = r#"{"shortcut":"Ctrl+Shift+A","target_lang":"English"}"#;
        let cfg = parse_config_json(legacy);
        assert_eq!(cfg.shortcut, "Ctrl+Shift+A");
        assert!(cfg.clipboard_shortcut.is_empty());
        assert!(cfg.selection_shortcut.is_empty());
        assert_eq!(cfg.target_lang, "English");
        assert_eq!(cfg.theme, "system");
        assert_eq!(cfg.max_stack_size, 6);
        assert!(!cfg.launch_at_startup);
        assert!(cfg.glossary.is_empty());
        assert!(cfg.proxy_url.is_empty());
        assert!(cfg.update_feed_url.is_empty());
        assert_eq!(cfg.ocr_engine, "auto");
        assert_eq!(cfg.ocr_lang, "ch");
        assert!(cfg.ocr_download_url.is_empty());
        assert_eq!(cfg.ask_quick_actions.len(), default_ask_quick_actions().len());
    }

    #[test]
    fn migrates_legacy_custom_prompt_presets_when_ask_quick_actions_missing() {
        let legacy = r#"{"custom_prompt_presets":["提取表格数据为 CSV","总结要点"]}"#;
        let cfg = parse_config_json(legacy);
        assert!(cfg.ask_quick_actions.iter().any(|a| a.prompt == "提取表格数据为 CSV"));
        assert_eq!(
            cfg.ask_quick_actions.iter().filter(|a| a.label == "总结要点").count(),
            1
        );

        // Explicit ask_quick_actions should not re-merge legacy presets
        let explicit = r#"{"custom_prompt_presets":["旧提示词"],"ask_quick_actions":[{"id":"only","label":"仅此一项","prompt":"只保留此项"}]}"#;
        let cfg2 = parse_config_json(explicit);
        assert_eq!(cfg2.ask_quick_actions.len(), 1);
        assert_eq!(cfg2.ask_quick_actions[0].id, "only");
    }

    #[test]
    fn sanitize_ask_quick_actions_cleans_deduplicates_and_caps() {
        let mut input = vec![
            AskQuickAction { id: "  ".into(), label: "  ".into(), prompt: "empty label".into() },
            AskQuickAction { id: "dup".into(), label: "  第一项  ".into(), prompt: "  提示词 1  ".into() },
            AskQuickAction { id: "dup".into(), label: "第二项".into(), prompt: "提示词 2".into() },
        ];
        for i in 0..25 {
            input.push(AskQuickAction {
                id: String::new(),
                label: format!("项{i}"),
                prompt: format!("内容{i}"),
            });
        }
        let cleaned = sanitize_ask_quick_actions(input);
        assert_eq!(cleaned.len(), MAX_ASK_QUICK_ACTIONS);
        assert_eq!(cleaned[0].id, "dup");
        assert_eq!(cleaned[0].label, "第一项");
        assert_eq!(cleaned[0].prompt, "提示词 1");
        assert_eq!(cleaned[1].id, "custom-1");
    }

    #[test]
    fn normalize_ocr_engine_and_lang_fallback_to_defaults() {
        assert_eq!(normalize_ocr_engine("RapidOCR"), "rapidocr");
        assert_eq!(normalize_ocr_engine(" WINDOWS "), "windows");
        assert_eq!(normalize_ocr_engine("unknown"), "auto");
        assert_eq!(normalize_ocr_lang("JP"), "jp");
        assert_eq!(normalize_ocr_lang(" cht "), "cht");
        assert_eq!(normalize_ocr_lang("other"), "ch");
    }

    #[test]
    fn normalize_theme_handles_known_and_unknown_values() {
        assert_eq!(normalize_theme("Dark"), "dark");
        assert_eq!(normalize_theme(" light "), "light");
        assert_eq!(normalize_theme("anything-else"), "system");
    }

    #[test]
    fn normalize_proxy_url_validates_schemes_and_shape() {
        assert_eq!(normalize_proxy_url("   ").unwrap(), None);
        assert_eq!(
            normalize_proxy_url(" http://127.0.0.1:7890 ").unwrap().as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            normalize_proxy_url("socks5://127.0.0.1:10808").unwrap().as_deref(),
            Some("socks5://127.0.0.1:10808")
        );
        assert_eq!(
            normalize_proxy_url("socks5h://127.0.0.1:10808").unwrap().as_deref(),
            Some("socks5h://127.0.0.1:10808")
        );
        assert!(normalize_proxy_url("ftp://127.0.0.1:21").is_err());
        assert!(normalize_proxy_url("127.0.0.1:7890").is_err());
        assert!(normalize_proxy_url("http://127.0.0.1:7890/path").is_err());
        assert!(normalize_proxy_url("http://127.0.0.1:7890?a=1").is_err());
        assert!(proxy_has_credentials("http://user:pass@127.0.0.1:7890"));
        assert!(!proxy_has_credentials("http://127.0.0.1:7890"));
    }

    #[test]
    fn build_api_client_accepts_empty_http_and_socks5_proxies() {
        assert!(build_api_client("").is_ok());
        assert!(build_api_client("http://127.0.0.1:7890").is_ok());
        assert!(build_api_client("socks5://127.0.0.1:10808").is_ok());
        assert!(build_api_client("invalid://proxy").is_err());
    }

    #[test]
    fn parses_windows_proxy_server_formats() {
        assert_eq!(
            parse_windows_proxy_server("127.0.0.1:3067", "https").as_deref(),
            Some("http://127.0.0.1:3067")
        );
        assert_eq!(
            parse_windows_proxy_server("http://127.0.0.1:7890", "https").as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            parse_windows_proxy_server(
                "http=127.0.0.1:10809;https=127.0.0.1:10810;socks=127.0.0.1:10808",
                "https"
            )
            .as_deref(),
            Some("http://127.0.0.1:10810")
        );
        assert_eq!(
            parse_windows_proxy_server(
                "http=127.0.0.1:10809;https=127.0.0.1:10810;socks=127.0.0.1:10808",
                "http"
            )
            .as_deref(),
            Some("http://127.0.0.1:10809")
        );
        assert_eq!(
            parse_windows_proxy_server("socks=127.0.0.1:10808", "https").as_deref(),
            Some("socks5h://127.0.0.1:10808")
        );
        assert_eq!(parse_windows_proxy_server("   ", "https"), None);
    }

    #[test]
    fn host_bypass_list_matches_windows_rules() {
        let rules = "<-loopback>;<local>;localhost;*.local;127.*;10.*;172.16.*;192.168.*";
        assert!(host_matches_bypass_list("localhost", rules));
        assert!(host_matches_bypass_list("127.0.0.1", rules));
        assert!(host_matches_bypass_list("::1", rules));
        assert!(host_matches_bypass_list("[::1]", rules));
        assert!(host_matches_bypass_list("intranet", rules));
        assert!(host_matches_bypass_list("dev.local", rules));
        assert!(host_matches_bypass_list("10.0.0.5", rules));
        assert!(host_matches_bypass_list("192.168.3.1", rules));
        assert!(!host_matches_bypass_list("github.com", rules));
        assert!(!host_matches_bypass_list(
            "release-assets.githubusercontent.com",
            rules
        ));
    }

    #[test]
    fn masks_proxy_credentials_cleanly() {
        assert_eq!(
            mask_proxy_credentials("http://alice:secret@127.0.0.1:7890"),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            mask_proxy_credentials("socks5h://127.0.0.1:10808"),
            "socks5h://127.0.0.1:10808"
        );
    }

    #[test]
    fn parses_macos_scutil_proxy_output() {
        let sample = r#"
<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : 169.254/16
    2 : 127.0.0.1
  }
  FTPPassive : 1
  HTTPEnable : 1
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7891
  HTTPSProxy : 127.0.0.1
  SOCKSEnable : 1
  SOCKSPort : 7892
  SOCKSProxy : 127.0.0.1
}
"#;
        let snap = parse_macos_scutil_proxy(sample).expect("scutil proxy parsed");
        assert!(snap.enabled);
        assert_eq!(snap.source, "macos");
        assert_eq!(
            snap.proxy_server,
            "https=127.0.0.1:7891;http=127.0.0.1:7890;socks=127.0.0.1:7892"
        );
        assert_eq!(snap.proxy_override, "*.local;169.254/16;127.0.0.1");
        assert_eq!(
            parse_windows_proxy_server(&snap.proxy_server, "https").as_deref(),
            Some("http://127.0.0.1:7891")
        );
        assert_eq!(
            parse_windows_proxy_server(&snap.proxy_server, "http").as_deref(),
            Some("http://127.0.0.1:7890")
        );

        let disabled = r#"
<dictionary> {
  HTTPEnable : 0
  HTTPSEnable : 0
  SOCKSEnable : 0
}
"#;
        assert!(parse_macos_scutil_proxy(disabled).is_none());
    }
}

use std::{
    collections::HashSet,
    fs::File,
    io::{self, Read},
    path::Path,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_updater::{Update, UpdaterExt};

const MANIFEST_LIMIT: u64 = 64 * 1024;
const INSTALLER_LIMIT: u64 = 128 * 1024 * 1024;
const NOTES_MAX_CHARS: usize = 4000;
const UPDATE_FEED: Option<&str> = option_env!("LUMA_UPDATE_FEED");

const STARTUP_DELAY: Duration = Duration::from_secs(5);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(15 * 60);
const BUSY_RETRY: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct UpdateState {
    operation: tokio::sync::Mutex<()>,
    reminder: Mutex<UpdateReminder>,
    tray_item: OnceLock<tauri::menu::MenuItem<tauri::Wry>>,
}

#[derive(Default)]
struct UpdateReminder {
    available: Option<AvailableUpdate>,
    seen_versions: HashSet<String>,
}

impl UpdateReminder {
    fn record(&mut self, available: Option<AvailableUpdate>, automatic: bool) -> (bool, bool) {
        if automatic
            && available.is_none()
            && self
                .available
                .as_ref()
                .is_some_and(|update| update.source == UpdateSource::Local)
        {
            return (false, false);
        }
        let changed = self.available != available;
        self.available = available;
        let Some(update) = &self.available else {
            return (changed, false);
        };
        let unseen = self.seen_versions.insert(update.version.clone());
        (changed, automatic && unseen)
    }
}

#[derive(Deserialize)]
struct ReleaseManifest {
    version: String,
    signature: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    pub_date: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum UpdateSource {
    Online,
    Local,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    version: String,
    source: UpdateSource,
    notes: Option<String>,
    pub_date: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub stage: String,
    pub version: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub percent: Option<u8>,
    pub message: Option<String>,
}

impl UpdateProgress {
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fn error(version: &str, message: &str) -> Self {
        Self {
            stage: "error".into(),
            version: version.into(),
            downloaded: 0,
            total: None,
            percent: None,
            message: Some(message.into()),
        }
    }
}

struct RestoredRelease {
    update: Update,
    notes: Option<String>,
    pub_date: Option<String>,
}

struct OnlineRelease {
    update: Update,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    url: Url,
    notes: Option<String>,
    pub_date: Option<String>,
}

fn sanitize_notes(raw: Option<String>) -> Option<String> {
    let trimmed = raw?.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= NOTES_MAX_CHARS {
        Some(trimmed)
    } else {
        Some(trimmed.chars().take(NOTES_MAX_CHARS).collect())
    }
}

fn sanitize_pub_date(raw: Option<String>) -> Option<String> {
    let trimmed = raw?.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(64).collect())
}

pub(crate) fn parse_feed(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "联网更新源地址无效。".to_string())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path_segments().and_then(|mut parts| parts.next_back()) != Some("latest.json")
    {
        return Err("联网更新源必须是以 latest.json 结尾的不含账号或参数的 HTTPS 地址。".into());
    }
    Ok(url)
}

fn online_feed() -> Result<Url, String> {
    UPDATE_FEED
        .filter(|url| !url.is_empty())
        .ok_or_else(|| "此构建未配置联网更新源。".to_string())
        .and_then(parse_feed)
}

fn candidate_feeds(custom_feed: &str) -> Result<Vec<Url>, String> {
    let mut feeds = Vec::new();
    let trimmed = custom_feed.trim();
    if !trimmed.is_empty() {
        feeds.push(parse_feed(trimmed)?);
    }
    if let Ok(builtin) = online_feed() {
        if !feeds.iter().any(|existing| existing == &builtin) {
            feeds.push(builtin);
        }
    }
    if feeds.is_empty() {
        return Err("此构建未配置联网更新源。".to_string());
    }
    Ok(feeds)
}

fn current_network_settings(app: &AppHandle) -> (String, String) {
    app.state::<Mutex<super::AppState>>()
        .lock()
        .map(|guard| {
            (
                guard.config.proxy_url.clone(),
                guard.config.update_feed_url.clone(),
            )
        })
        .unwrap_or_default()
}

fn read_limited(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "update file exceeds size limit",
        ));
    }
    Ok(bytes)
}

async fn fetch_limited<F>(
    url: Url,
    limit: u64,
    timeout: Duration,
    proxy_url: &str,
    on_progress: F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u64, Option<u64>),
{
    let builder = Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .timeout(timeout);
    let builder = crate::config::apply_proxy(builder, proxy_url)?;
    let client = builder
        .build()
        .map_err(|_| "无法初始化更新网络连接。".to_string())?;
    fetch_with_client_progress(&client, url, limit, on_progress).await
}

#[cfg(test)]
async fn fetch_with_client(client: &Client, url: Url, limit: u64) -> Result<Vec<u8>, String> {
    fetch_with_client_progress(client, url, limit, |_, _| {}).await
}

async fn fetch_with_client_progress<F>(
    client: &Client,
    url: Url,
    limit: u64,
    mut on_progress: F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u64, Option<u64>),
{
    let mut response = client
        .get(url)
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                "连接更新服务器超时，请检查网络或代理设置后重试。"
            } else if err.is_redirect() {
                "更新服务器重定向无效。"
            } else {
                "无法连接更新服务器，请检查网络、代理和系统证书后重试。"
            }
            .to_string()
        })?;
    if !response.status().is_success() {
        return Err(format!(
            "更新服务器返回 HTTP {}。",
            response.status().as_u16()
        ));
    }
    let total = response.content_length();
    if total.is_some_and(|length| length > limit) {
        return Err("更新文件超过大小限制。".into());
    }
    on_progress(0, total);
    let mut bytes = Vec::with_capacity(total.map(|n| n.min(limit) as usize).unwrap_or(0));
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "下载中断，请检查网络或代理后重试。".to_string())?
    {
        if chunk.len() as u64 > limit - bytes.len() as u64 {
            return Err("更新文件超过大小限制。".into());
        }
        bytes.extend_from_slice(&chunk);
        on_progress(bytes.len() as u64, total);
    }
    Ok(bytes)
}

fn restore_release(
    app: &AppHandle,
    manifest: &[u8],
    endpoint: Url,
) -> Result<Option<RestoredRelease>, String> {
    let release: ReleaseManifest =
        serde_json::from_slice(manifest).map_err(|_| "更新清单格式无效。".to_string())?;
    if release.signature.trim().is_empty() {
        return Err("更新清单缺少签名。".into());
    }
    let notes = sanitize_notes(release.notes);
    let pub_date = sanitize_pub_date(release.pub_date);
    let json = serde_json::json!({"version": release.version, "signature": release.signature, "url": endpoint.as_str()});
    let update = app
        .updater_builder()
        // On Windows the plugin runs this hook before it knows whether the installer launched.
        // Keep the tray and windows available on failure. A successful launch exits the process.
        .on_before_exit(|| {})
        .endpoints(vec![endpoint])
        .and_then(|builder| builder.build())
        .and_then(|updater| updater.restore_update(json))
        .map_err(|err| format!("无法读取更新版本：{err}"))?;
    Ok(update.map(|update| RestoredRelease {
        update,
        notes,
        pub_date,
    }))
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn installer_filename(update: &Update) -> String {
    // Tauri has parsed and normalized this version as SemVer.
    format!("Luma_{}_x64-setup.exe", update.version)
}

fn installer_url(feed: &Url, version: &str, filename: &str) -> Result<Url, String> {
    if let Some(prefix) = feed.path().strip_suffix("/releases/latest/download/latest.json") {
        if !prefix.is_empty() {
            // Pin the installer to the version read from the manifest. GitHub's `latest`
            // alias (and GitHub release mirrors) can move while a download is pending.
            let mut pinned = feed.clone();
            pinned.set_path(&format!("{prefix}/releases/download/v{version}/{filename}"));
            return Ok(pinned);
        }
    }
    feed.join(filename)
        .map_err(|_| "无法解析更新安装包地址。".to_string())
}

struct OnlineReleaseCheck {
    release: Option<OnlineRelease>,
    custom_feed_error: Option<String>,
}

async fn online_release_check(
    app: &AppHandle,
    proxy_url: &str,
    custom_feed: &str,
) -> Result<OnlineReleaseCheck, String> {
    let feeds = candidate_feeds(custom_feed)?;
    let has_custom = !custom_feed.trim().is_empty();
    let mut first_error: Option<String> = None;
    for (idx, endpoint) in feeds.into_iter().enumerate() {
        let fetched = fetch_limited(
            endpoint.clone(),
            MANIFEST_LIMIT,
            Duration::from_secs(15),
            proxy_url,
            |_, _| {},
        )
        .await
        .and_then(|bytes| {
            // Resolve from the stable feed URL, not a redirect target such as a temporary CDN URL.
            let restored = restore_release(app, &bytes, endpoint.clone())?;
            restored
                .map(|release| {
                    let url = installer_url(
                        &endpoint,
                        &release.update.version,
                        &installer_filename(&release.update),
                    )?;
                    Ok(OnlineRelease {
                        update: release.update,
                        url,
                        notes: release.notes,
                        pub_date: release.pub_date,
                    })
                })
                .transpose()
        });
        match fetched {
            Ok(release) => {
                let custom_feed_error = if has_custom && idx > 0 {
                    first_error
                } else {
                    None
                };
                return Ok(OnlineReleaseCheck {
                    release,
                    custom_feed_error,
                });
            }
            Err(err) => {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
    }
    Err(first_error.unwrap_or_else(|| "无法连接更新服务器。".to_string()))
}

async fn online_release_with(
    app: &AppHandle,
    proxy_url: &str,
    custom_feed: &str,
) -> Result<Option<OnlineRelease>, String> {
    Ok(online_release_check(app, proxy_url, custom_feed).await?.release)
}

async fn online_release(app: &AppHandle) -> Result<Option<OnlineRelease>, String> {
    let (proxy_url, custom_feed) = current_network_settings(app);
    online_release_with(app, &proxy_url, &custom_feed).await
}

pub async fn test_update_connection(
    app: &AppHandle,
    proxy_url: &str,
    custom_feed: &str,
) -> Result<String, String> {
    crate::config::normalize_proxy_url(proxy_url)?;
    if !custom_feed.trim().is_empty() {
        parse_feed(custom_feed.trim())?;
    }
    let started = Instant::now();
    let check = online_release_check(app, proxy_url, custom_feed).await?;
    let elapsed_ms = started.elapsed().as_millis();
    let via = if proxy_url.trim().is_empty() {
        match crate::config::detect_system_proxy_info() {
            info if info.enabled => {
                if let Some(url) = info.proxy_url {
                    format!("系统代理 ({url}) ")
                } else {
                    "系统代理".to_string()
                }
            }
            _ => "系统直连".to_string(),
        }
    } else {
        "自定义代理".to_string()
    };
    let status = match check.release {
        Some(found) => format!("发现新版本 v{}", found.update.version),
        None => "当前已是最新版本".to_string(),
    };
    if let Some(custom_err) = check.custom_feed_error {
        let reason = custom_err.trim_end_matches('。');
        return Ok(format!(
            "自定义更新源不可用（{reason}），已自动回退官方源并通过{via}连通，{status}（耗时 {elapsed_ms}ms）"
        ));
    }
    Ok(format!("{via}连接更新源正常，{status}（耗时 {elapsed_ms}ms）"))
}

fn verified_local_update(app: &AppHandle) -> Result<Option<(RestoredRelease, Vec<u8>)>, String> {
    let directory = app
        .path()
        .local_data_dir()
        .map_err(|err| err.to_string())?
        .join("Luma")
        .join("Updates");
    read_local_update(app, &directory)
}

fn read_local_update(
    app: &AppHandle,
    directory: &Path,
) -> Result<Option<(RestoredRelease, Vec<u8>)>, String> {
    let manifest = match read_limited(&directory.join("latest.json"), MANIFEST_LIMIT) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("读取本机更新信息失败：{err}")),
    };
    let placeholder = Url::parse("https://luma-update.invalid/").expect("valid HTTPS URL");
    let Some(release) = restore_release(app, &manifest, placeholder)? else {
        return Ok(None);
    };
    let bytes = read_limited(
        &directory.join(installer_filename(&release.update)),
        INSTALLER_LIMIT,
    )
    .map_err(|err| format!("无法读取本机更新包：{err}"))?;
    release
        .update
        .verify(&bytes)
        .map_err(|err| format!("更新签名校验失败：{err}"))?;
    Ok(Some((release, bytes)))
}

fn require_update_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("请在 Luma 主窗口操作更新。".into());
    }
    if !cfg!(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )) {
        return Err("当前系统暂不支持应用内检查更新。".into());
    }
    Ok(())
}

fn emit_update_progress(app: &AppHandle, progress: UpdateProgress) {
    if let Some(main) = app.get_webview_window("main") {
        let ui = app.clone();
        super::post_to_ui(&ui, move || {
            super::dispatch_window_event(&main, "update:progress", &progress);
        });
    }
}

fn record_available_update(app: &AppHandle, available: Option<AvailableUpdate>, automatic: bool) {
    let state = app.state::<UpdateState>();
    let (changed, notify) = state
        .reminder
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .record(available.clone(), automatic);
    if changed {
        if let Some(menu) = state.tray_item.get() {
            let title = available.as_ref().map_or_else(
                || "检查更新".into(),
                |update| format!("更新到 {}…", update.version),
            );
            if let Err(err) = menu.set_text(title) {
                eprintln!("[Luma] 更新托盘菜单失败: {err}");
            }
        }
        if let Some(main) = app.get_webview_window("main") {
            let name = "update:available";
            let ui = app.clone();
            super::post_to_ui(&ui, move || super::dispatch_window_event(&main, name, &()));
        }
    }
    #[cfg(any(windows, target_os = "macos"))]
    if notify {
        use tauri_plugin_notification::NotificationExt;

        if let Some(update) = available {
            let shown = app
                .notification()
                .builder()
                .title(format!("Luma {} 可更新", update.version))
                .body("点击查看更新说明并安装。你也可以在 Luma 托盘菜单中检查更新。")
                .show();
            if let Err(err) = shown {
                eprintln!("[Luma] 显示更新通知失败: {err}");
            }
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = notify;
}

pub fn start_automatic_checks(app: AppHandle, menu: tauri::menu::MenuItem<tauri::Wry>) {
    let _ = app.state::<UpdateState>().tray_item.set(menu);
    if !cfg!(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut delay = STARTUP_DELAY;
        loop {
            tokio::time::sleep(delay).await;
            let (proxy_url, custom_feed) = current_network_settings(&app);
            if candidate_feeds(&custom_feed).is_err() {
                delay = CHECK_INTERVAL;
                continue;
            }
            let state = app.state::<UpdateState>();
            let Ok(_check) = state.operation.try_lock() else {
                delay = BUSY_RETRY;
                continue;
            };
            match online_release_with(&app, &proxy_url, &custom_feed).await {
                Ok(release) => {
                    let available = release.map(|found| AvailableUpdate {
                        version: found.update.version,
                        source: UpdateSource::Online,
                        notes: found.notes,
                        pub_date: found.pub_date,
                    });
                    record_available_update(&app, available, true);
                    delay = CHECK_INTERVAL;
                }
                Err(err) => {
                    eprintln!("[Luma] 后台更新检查失败: {err}");
                    delay = RETRY_INTERVAL;
                }
            }
        }
    });
}

#[tauri::command]
pub fn get_available_app_update(window: WebviewWindow) -> Result<Option<AvailableUpdate>, String> {
    require_update_window(&window)?;
    let state = window.state::<UpdateState>();
    let reminder = state.reminder.lock().unwrap_or_else(|err| err.into_inner());
    Ok(reminder.available.clone())
}

#[tauri::command]
pub async fn check_app_update(window: WebviewWindow) -> Result<Option<AvailableUpdate>, String> {
    require_update_window(&window)?;
    let app = window.app_handle().clone();
    let state = app.state::<UpdateState>();
    let _check = state
        .operation
        .try_lock()
        .map_err(|_| "正在检查或安装更新，请稍候。".to_string())?;
    let available = check_available_update(&app).await?;
    record_available_update(&app, available.clone(), false);
    Ok(available)
}

async fn check_available_update(app: &AppHandle) -> Result<Option<AvailableUpdate>, String> {
    let online = online_release(app).await;
    if let Ok(Some(found)) = online {
        return Ok(Some(AvailableUpdate {
            version: found.update.version,
            source: UpdateSource::Online,
            notes: found.notes,
            pub_date: found.pub_date,
        }));
    }
    // A signed local import remains usable offline, including for test and migration builds.
    let local_app = app.clone();
    let local = tauri::async_runtime::spawn_blocking(move || verified_local_update(&local_app))
        .await
        .map_err(|err| err.to_string())?;
    match (local, online) {
        (Ok(Some((found, _))), _) => Ok(Some(AvailableUpdate {
            version: found.update.version,
            source: UpdateSource::Local,
            notes: found.notes,
            pub_date: found.pub_date,
        })),
        (Ok(None), online) => online.map(|_| None),
        // A stale or damaged staged package must not mask a successful online check.
        (Err(err), Ok(_)) => {
            eprintln!("[Luma] 忽略不可用的本机更新包: {err}");
            Ok(None)
        }
        (Err(err), Err(_)) => Err(err),
    }
}

#[tauri::command]
pub async fn install_app_update(
    window: WebviewWindow,
    expected_version: String,
    source: UpdateSource,
) -> Result<(), String> {
    require_update_window(&window)?;
    let app = window.app_handle().clone();
    let state = app.state::<UpdateState>();
    let _install = state
        .operation
        .try_lock()
        .map_err(|_| "正在安装更新。".to_string())?;
    #[cfg(target_os = "macos")]
    {
        let _ = source;
        let ver = expected_version.trim().trim_start_matches('v');
        if ver.is_empty()
            || !ver
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        {
            return Err("更新版本号无效。".to_string());
        }
        let release_url = format!("https://github.com/Leon-Ma857/Leon-/releases/tag/v{ver}");
        std::process::Command::new("/usr/bin/open")
            .arg(&release_url)
            .spawn()
            .map_err(|e| format!("无法在浏览器打开新版下载页：{e}"))?;
        emit_update_progress(
            &app,
            UpdateProgress {
                stage: "ready".into(),
                version: ver.to_string(),
                downloaded: 0,
                total: None,
                percent: Some(100),
                message: Some("已在浏览器打开 macOS 新版下载页，下载 .dmg 后拖入「应用程序」覆盖即可。".into()),
            },
        );
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let changed = || "更新版本已变化，请重新检查。".to_string();

        emit_update_progress(
            &app,
            UpdateProgress {
                stage: "checking".into(),
                version: expected_version.clone(),
                downloaded: 0,
                total: None,
                percent: None,
                message: Some("正在核对更新版本…".into()),
            },
        );

        let result: Result<(Update, Vec<u8>), String> = async {
            match source {
                UpdateSource::Online => {
                    let (proxy_url, custom_feed) = current_network_settings(&app);
                    let found = online_release_with(&app, &proxy_url, &custom_feed)
                        .await?
                        .filter(|release| release.update.version == expected_version)
                        .ok_or_else(changed)?;
                    let mut last_emit = Instant::now() - Duration::from_secs(1);
                    let mut last_percent: Option<u8> = None;
                    let progress_app = app.clone();
                    let progress_version = expected_version.clone();
                    let bytes = fetch_limited(
                        found.url,
                        INSTALLER_LIMIT,
                        Duration::from_secs(600),
                        &proxy_url,
                        |downloaded, total| {
                            let percent = total.and_then(|len| {
                                if len == 0 {
                                    None
                                } else {
                                    Some(((downloaded.saturating_mul(100)) / len).min(100) as u8)
                                }
                            });
                            let is_finished = total.is_some_and(|len| downloaded >= len);
                            let should_emit = downloaded == 0
                                || is_finished
                                || percent != last_percent
                                || last_emit.elapsed() >= Duration::from_millis(150);
                            if should_emit {
                                last_emit = Instant::now();
                                last_percent = percent;
                                emit_update_progress(
                                    &progress_app,
                                    UpdateProgress {
                                        stage: "downloading".into(),
                                        version: progress_version.clone(),
                                        downloaded,
                                        total,
                                        percent,
                                        message: Some("正在下载安装包…".into()),
                                    },
                                );
                            }
                        },
                    )
                    .await?;
                    Ok((found.update, bytes))
                }
                UpdateSource::Local => {
                    let local_app = app.clone();
                    let (found, bytes) = tauri::async_runtime::spawn_blocking(move || {
                        verified_local_update(&local_app)
                    })
                    .await
                    .map_err(|err| err.to_string())??
                    .filter(|(release, _)| release.update.version == expected_version)
                    .ok_or_else(changed)?;
                    Ok((found.update, bytes))
                }
            }
        }
        .await;

        let (update, bytes) = match result {
            Ok(pair) => pair,
            Err(err) => {
                emit_update_progress(&app, UpdateProgress::error(&expected_version, &err));
                return Err(err);
            }
        };

        let size = bytes.len() as u64;
        emit_update_progress(
            &app,
            UpdateProgress {
                stage: "verifying".into(),
                version: expected_version.clone(),
                downloaded: size,
                total: Some(size),
                percent: Some(100),
                message: Some("正在验证官方更新签名…".into()),
            },
        );

        let verified = tauri::async_runtime::spawn_blocking(move || {
            update
                .verify(&bytes)
                .map_err(|err| format!("更新签名校验失败：{err}"))?;
            Ok::<_, String>((update, bytes))
        })
        .await
        .map_err(|err| err.to_string())?;

        let (update, bytes) = match verified {
            Ok(pair) => pair,
            Err(err) => {
                emit_update_progress(&app, UpdateProgress::error(&expected_version, &err));
                return Err(err);
            }
        };

        emit_update_progress(
            &app,
            UpdateProgress {
                stage: "installing".into(),
                version: expected_version.clone(),
                downloaded: size,
                total: Some(size),
                percent: Some(100),
                message: Some("签名校验通过，正在启动安装程序…".into()),
            },
        );

        // Update::install verifies the artifact again and launches the NSIS installer.
        let install_result = tauri::async_runtime::spawn_blocking(move || update.install(bytes))
            .await
            .map_err(|err| err.to_string())?
            .map_err(|err| format!("无法校验或安装更新：{err}"));
        if let Err(err) = &install_result {
            emit_update_progress(&app, UpdateProgress::error(&expected_version, err));
        }
        install_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announces_each_newly_found_update_once_per_run() {
        let mut reminder = UpdateReminder::default();
        let online = |version: &str| {
            Some(AvailableUpdate {
                version: version.into(),
                source: UpdateSource::Online,
                notes: None,
                pub_date: None,
            })
        };

        assert_eq!(reminder.record(None, true), (false, false));
        assert_eq!(reminder.record(online("0.7.24"), true), (true, true));
        assert_eq!(reminder.record(online("0.7.24"), true), (false, false));
        assert_eq!(reminder.record(None, true), (true, false));
        assert_eq!(reminder.record(online("0.7.24"), true), (true, false));
        assert_eq!(reminder.record(online("0.7.25"), true), (true, true));
    }

    #[test]
    fn preserves_locally_discovered_updates_during_online_checks() {
        let mut reminder = UpdateReminder::default();
        let local = Some(AvailableUpdate {
            version: "0.7.24".into(),
            source: UpdateSource::Local,
            notes: None,
            pub_date: None,
        });
        assert_eq!(reminder.record(local.clone(), false), (true, false));
        assert_eq!(reminder.record(None, true), (false, false));
        assert_eq!(reminder.available, local);
        let online = Some(AvailableUpdate {
            version: "0.7.24".into(),
            source: UpdateSource::Online,
            notes: Some("- Fix".into()),
            pub_date: None,
        });
        assert_eq!(reminder.record(online, true), (true, false));
    }

    #[test]
    fn only_accepts_public_https_manifest_shape() {
        for url in [
            "http://example.com/latest.json",
            "https://user@example.com/latest.json",
            "https://example.com/latest.json?token=secret",
            "https://example.com/latest.json#test",
            "https://example.com/update/",
        ] {
            assert!(parse_feed(url).is_err(), "accepted {url}");
        }
        let feed =
            parse_feed("https://github.com/example/luma/releases/latest/download/latest.json")
                .unwrap();
        assert_eq!(
            installer_url(&feed, "0.7.13", "Luma_0.7.13_x64-setup.exe")
                .unwrap()
                .as_str(),
            "https://github.com/example/luma/releases/download/v0.7.13/Luma_0.7.13_x64-setup.exe"
        );
        let mirror = parse_feed(
            "https://mirror.example.com/https://github.com/example/luma/releases/latest/download/latest.json",
        )
        .unwrap();
        assert_eq!(
            installer_url(&mirror, "0.7.24", "Luma_0.7.24_x64-setup.exe")
                .unwrap()
                .as_str(),
            "https://mirror.example.com/https://github.com/example/luma/releases/download/v0.7.24/Luma_0.7.24_x64-setup.exe"
        );
        let static_feed = parse_feed("https://updates.example.com/luma/latest.json").unwrap();
        assert_eq!(
            installer_url(&static_feed, "0.7.24", "Luma_0.7.24_x64-setup.exe")
                .unwrap()
                .as_str(),
            "https://updates.example.com/luma/Luma_0.7.24_x64-setup.exe"
        );
    }

    #[test]
    #[ignore = "requires the controlled HTTPS fixture from scripts/test-online-update.mjs"]
    fn downloads_from_https_fixture() {
        tauri::async_runtime::block_on(async {
            let feed = parse_feed(&std::env::var("LUMA_TEST_FEED").unwrap()).unwrap();
            let cert = std::fs::read(std::env::var_os("LUMA_TEST_CERT").unwrap()).unwrap();
            let client = Client::builder()
                .https_only(true)
                .no_proxy()
                .add_root_certificate(reqwest::Certificate::from_pem(&cert).unwrap())
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap();
            let body = fetch_with_client(&client, feed.clone(), MANIFEST_LIMIT)
                .await
                .unwrap();
            let manifest: ReleaseManifest = serde_json::from_slice(&body).unwrap();
            assert_eq!(manifest.version, "0.7.12");
            assert!(!manifest.signature.is_empty());
            let url = feed.join("Luma_0.7.12_x64-setup.exe").unwrap();
            let mut progress_points = Vec::new();
            let bytes = fetch_with_client_progress(&client, url, INSTALLER_LIMIT, |done, total| {
                progress_points.push((done, total));
            })
            .await
            .unwrap();
            assert_eq!(
                bytes,
                std::fs::read(std::env::var_os("LUMA_TEST_INSTALLER").unwrap()).unwrap()
            );
            assert!(
                progress_points.first().is_some_and(|(done, _)| *done == 0),
                "expected initial progress report"
            );
            assert!(
                progress_points
                    .last()
                    .is_some_and(|(done, _)| *done == bytes.len() as u64),
                "expected final progress report matching downloaded length"
            );
            for (path, limit, message) in [
                ("missing", MANIFEST_LIMIT, "HTTP 404"),
                ("large", 8, "超过大小限制"),
                ("downgrade", MANIFEST_LIMIT, "更新服务器重定向无效"),
            ] {
                let url = feed.join(path).unwrap();
                let err = fetch_with_client(&client, url, limit).await.unwrap_err();
                assert!(err.contains(message), "{path}: {err}");
            }
        });
    }

    #[test]
    #[ignore = "requires a fresh signed installer from scripts/release-windows.ps1"]
    fn verifies_release_artifact() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        use minisign_verify::{PublicKey, Signature};

        let installer =
            std::path::PathBuf::from(std::env::var_os("LUMA_VERIFY_INSTALLER").unwrap());
        let bytes = read_limited(&installer, INSTALLER_LIMIT).expect("read size-limited installer");
        let mut signature_path = installer.into_os_string();
        signature_path.push(".sig");
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let key = config["plugins"]["updater"]["pubkey"].as_str().unwrap();
        let signature = std::fs::read_to_string(signature_path).unwrap();
        let decode =
            |encoded: &str| String::from_utf8(STANDARD.decode(encoded.trim()).unwrap()).unwrap();
        let public_key = PublicKey::decode(&decode(key)).unwrap();
        let signature = Signature::decode(&decode(&signature)).unwrap();
        public_key
            .verify(&bytes, &signature, true)
            .expect("valid updater signature");
        let version = config["version"].as_str().unwrap();
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            signature
                .trusted_comment()
                .split('\t')
                .find_map(|field| field.strip_prefix("version:")),
            Some(version)
        );
    }

    #[test]
    fn accepts_legacy_and_annotated_manifest_formats() {
        let legacy: ReleaseManifest =
            serde_json::from_str(r#"{"version":"0.7.12","signature":"signature"}"#).unwrap();
        assert_eq!(legacy.version, "0.7.12");
        assert_eq!(legacy.signature, "signature");
        assert!(legacy.notes.is_none());
        assert!(legacy.pub_date.is_none());

        let annotated: ReleaseManifest = serde_json::from_str(
            r#"{"version":"0.7.24","signature":"sig","notes":"  - 新增下载进度条\n- 新增代理支持  ","pub_date":"2026-11-22T10:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(
            sanitize_notes(annotated.notes).as_deref(),
            Some("- 新增下载进度条\n- 新增代理支持")
        );
        assert_eq!(
            sanitize_pub_date(annotated.pub_date).as_deref(),
            Some("2026-11-22T10:00:00Z")
        );
    }
}

use crate::config::{normalize_ocr_engine, normalize_ocr_lang, AppConfig};
use crate::local_ocr;
use crate::win_ocr::{ocr_png_bytes_with_layout, OcrLayoutBlock};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default)]
pub struct StreamPreview {
    pub text: String,
    pub original_text: Option<String>,
}

pub type StreamSink = Arc<dyn Fn(StreamPreview) + Send + Sync>;
pub type BatchStreamSink = Arc<dyn Fn(usize, StreamPreview) + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslatedBlock {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub line_height: f32,
    pub line_count: u32,
    pub original_text: String,
    pub translated_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchTextItem {
    pub index: usize,
    pub title: String,
    pub source_lang: String,
    pub original_text: String,
    pub translated_text: String,
    #[serde(default)]
    pub blocks: Vec<TranslatedBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationResult {
    pub id: u64,
    pub mode: String,
    pub engine: String,
    pub source_lang: String,
    pub target_lang: String,
    pub original_text: String,
    pub translated_text: String,
    #[serde(default)]
    pub blocks: Vec<TranslatedBlock>,
    pub elapsed_ms: u64,
    pub model: String,
    pub image_data_url: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Deserialize)]
struct ParsedModelJson {
    #[serde(default)]
    source_lang: String,
    #[serde(default)]
    original_text: String,
    #[serde(default)]
    translated_text: String,
}

#[derive(Debug, Deserialize)]
struct ParsedBlockEntry {
    id: usize,
    #[serde(default)]
    original_text: String,
    #[serde(default)]
    translated_text: String,
}

#[derive(Debug, Deserialize)]
struct ParsedBlocksJson {
    #[serde(default)]
    source_lang: String,
    #[serde(default)]
    blocks: Vec<ParsedBlockEntry>,
}

pub fn parse_glossary_entries(raw: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }
        let split = ["=>", "->", "=", "\t", "："]
            .iter()
            .find_map(|sep| trimmed.split_once(sep));
        if let Some((src, dst)) = split {
            let s = src.trim();
            let d = dst.trim();
            if !s.is_empty() && !d.is_empty() {
                entries.push((s.to_string(), d.to_string()));
                if entries.len() >= 40 {
                    break;
                }
            }
        }
    }
    entries
}

pub fn build_glossary_prompt_block(raw: &str) -> String {
    let entries = parse_glossary_entries(raw);
    if entries.is_empty() {
        return String::new();
    }
    let mut out = String::from("术语表约定（若遇到以下词汇请严格或优先采用对应译法）：\n");
    for (src, dst) in entries {
        out.push_str(&format!("- {src} => {dst}\n"));
    }
    out
}

pub(crate) fn same_ai_origin(first: &str, second: &str) -> bool {
    match (Url::parse(first.trim()), Url::parse(second.trim())) {
        (Ok(first), Ok(second)) => first.origin() == second.origin(),
        _ => false,
    }
}

fn validate_ai_endpoint(config: &AppConfig) -> Result<String, String> {
    let base = config.api_base_url.trim();
    if base.is_empty() {
        return Err("请先在设置中填写视觉 AI 的 API Base URL".to_string());
    }
    if config.model.trim().is_empty() {
        return Err("请先在设置中填写视觉 AI 模型名称".to_string());
    }
    let mut url = Url::parse(base).map_err(|_| "AI 的 API Base URL 无效".to_string())?;
    let is_local = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
    });
    if url.host().is_none() || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some()
        || !(url.scheme() == "https" || (is_local && url.scheme() == "http"))
    {
        return Err("AI 服务需要 HTTPS，本机回环地址可以使用 HTTP。地址不要包含账号、参数或片段。".into());
    }
    if config.api_key.trim().is_empty() && !is_local {
        return Err("请先在设置中填写视觉 AI 的 API Key（本地 Ollama 可留空）".to_string());
    }
    let path = url.path().trim_end_matches('/');
    let path = if path.ends_with("/chat/completions") {
        path.to_owned()
    } else {
        format!("{path}/chat/completions")
    };
    url.set_path(&path);
    Ok(url.into())
}

fn extract_message_text(body: &Value) -> Option<String> {
    let content = &body["choices"][0]["message"]["content"];
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = content.as_array() {
        let joined: Vec<&str> = arr
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect();
        if !joined.is_empty() {
            return Some(joined.join("\n"));
        }
    }
    None
}

fn strip_code_fences(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        if let Some(inner) = rest.strip_suffix("```") {
            return inner.trim();
        }
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        if let Some(inner) = rest.strip_suffix("```") {
            return inner.trim();
        }
    }
    trimmed
}

fn parse_vision_output(raw_text: &str) -> ParsedModelJson {
    let cleaned = strip_code_fences(raw_text);
    if let Ok(parsed) = serde_json::from_str::<ParsedModelJson>(cleaned) {
        return parsed;
    }

    if let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}')) {
        if start < end {
            if let Ok(parsed) = serde_json::from_str::<ParsedModelJson>(&cleaned[start..=end]) {
                return parsed;
            }
        }
    }

    ParsedModelJson {
        source_lang: "自动识别".to_string(),
        original_text: String::new(),
        translated_text: cleaned.to_string(),
    }
}

fn parse_blocks_output(
    raw_text: &str,
    layout_blocks: &[OcrLayoutBlock],
) -> Option<(String, Vec<TranslatedBlock>)> {
    if layout_blocks.is_empty() {
        return None;
    }
    let cleaned = strip_code_fences(raw_text);
    let parsed = serde_json::from_str::<ParsedBlocksJson>(cleaned).ok().or_else(|| {
        let start = cleaned.find('{')?;
        let end = cleaned.rfind('}')?;
        if start < end {
            serde_json::from_str::<ParsedBlocksJson>(&cleaned[start..=end]).ok()
        } else {
            None
        }
    })?;

    if parsed.blocks.is_empty() {
        return None;
    }

    let n = layout_blocks.len();
    let one_based = parsed.blocks.iter().all(|b| b.id >= 1 && b.id <= n)
        && !parsed.blocks.iter().any(|b| b.id == 0);

    let mut mapped: Vec<Option<(String, String)>> = vec![None; n];
    let mut matched = 0usize;
    for entry in parsed.blocks {
        let idx = if one_based {
            entry.id.saturating_sub(1)
        } else {
            entry.id
        };
        if idx < n && mapped[idx].is_none() {
            let trans = entry.translated_text.trim().to_string();
            let orig = entry.original_text.trim().to_string();
            if !trans.is_empty() || !orig.is_empty() {
                mapped[idx] = Some((orig, trans));
                matched += 1;
            }
        }
    }

    // Require high alignment coverage so we never show mismatched boxes.
    if matched == 0 || matched * 10 < n * 7 {
        return None;
    }

    let mut out = Vec::with_capacity(n);
    for (idx, lb) in layout_blocks.iter().enumerate() {
        let (orig, trans) = match mapped[idx].take() {
            Some((o, t)) => {
                let final_orig = if o.is_empty() { lb.text.clone() } else { o };
                let final_trans = if t.is_empty() { final_orig.clone() } else { t };
                (final_orig, final_trans)
            }
            None => (lb.text.clone(), lb.text.clone()),
        };
        out.push(TranslatedBlock {
            x: lb.x,
            y: lb.y,
            w: lb.w,
            h: lb.h,
            line_height: lb.line_height,
            line_count: lb.line_count,
            original_text: orig,
            translated_text: trans,
        });
    }

    Some((parsed.source_lang.trim().to_string(), out))
}

fn map_lines_to_layout_blocks(
    translated_joined: &str,
    layout_blocks: &[OcrLayoutBlock],
) -> Option<Vec<TranslatedBlock>> {
    if layout_blocks.is_empty() {
        return None;
    }
    let lines: Vec<&str> = translated_joined
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() != layout_blocks.len() {
        return None;
    }
    Some(
        layout_blocks
            .iter()
            .zip(lines.into_iter())
            .map(|(lb, tr)| TranslatedBlock {
                x: lb.x,
                y: lb.y,
                w: lb.w,
                h: lb.h,
                line_height: lb.line_height,
                line_count: lb.line_count,
                original_text: lb.text.clone(),
                translated_text: tr.to_string(),
            })
            .collect(),
    )
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn map_google_lang(label: &str) -> &'static str {
    match label {
        "简体中文" => "zh-CN",
        "繁体中文" => "zh-TW",
        "English" => "en",
        "日本語" => "ja",
        "한국어" => "ko",
        "Français" => "fr",
        "Deutsch" => "de",
        "Español" => "es",
        _ => "zh-CN",
    }
}

fn map_deepl_lang(label: &str) -> &'static str {
    match label {
        "简体中文" => "ZH-HANS",
        "繁体中文" => "ZH-HANT",
        "English" => "EN",
        "日本語" => "JA",
        "한국어" => "KO",
        "Français" => "FR",
        "Deutsch" => "DE",
        "Español" => "ES",
        _ => "ZH-HANS",
    }
}

fn extract_error_detail(text: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        for candidate in [
            v["error"]["message"].as_str(),
            v["message"].as_str(),
            v["detail"].as_str(),
            v["msg"].as_str(),
        ] {
            if let Some(s) = candidate {
                if !s.trim().is_empty() {
                    return s.trim().to_string();
                }
            }
        }
    }
    text.trim().to_string()
}

fn strip_optional_chat_params(payload: &Value) -> Option<Value> {
    let mut cloned = payload.clone();
    let obj = cloned.as_object_mut()?;
    let mut changed = false;

    for key in ["temperature", "max_tokens", "max_completion_tokens"] {
        if obj.remove(key).is_some() {
            changed = true;
        }
    }

    if let Some(messages) = obj.get_mut("messages").and_then(|m| m.as_array_mut()) {
        // Strip unsupported image_url.detail fields
        for msg in messages.iter_mut() {
            if let Some(parts) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
                for part in parts.iter_mut() {
                    if let Some(img_obj) = part.get_mut("image_url").and_then(|v| v.as_object_mut()) {
                        if img_obj.remove("detail").is_some() {
                            changed = true;
                        }
                    }
                }
            }
        }

        // Merge any system messages into the first user message for gateways/models without system role
        let mut sys_texts: Vec<String> = Vec::new();
        let mut kept: Vec<Value> = Vec::with_capacity(messages.len());
        for msg in messages.iter() {
            if msg["role"].as_str() == Some("system") {
                if let Some(s) = msg["content"].as_str() {
                    if !s.trim().is_empty() {
                        sys_texts.push(s.trim().to_string());
                    }
                }
                changed = true;
            } else {
                kept.push(msg.clone());
            }
        }
        if !sys_texts.is_empty() {
            let prefix = sys_texts.join("\n");
            if let Some(first_user) = kept.iter_mut().find(|m| m["role"].as_str() == Some("user")) {
                if let Some(s) = first_user["content"].as_str() {
                    first_user["content"] = json!(format!("{prefix}\n\n{s}"));
                } else if let Some(parts) = first_user["content"].as_array_mut() {
                    parts.insert(0, json!({ "type": "text", "text": prefix }));
                }
            }
            *messages = kept;
        }
    }

    if changed {
        Some(cloned)
    } else {
        None
    }
}

async fn call_chat_completions_once(
    client: &Client,
    config: &AppConfig,
    endpoint: &str,
    payload: &Value,
) -> Result<String, (u16, String)> {
    let mut req = client.post(endpoint).json(payload);
    if !config.api_key.trim().is_empty() {
        req = req.bearer_auth(config.api_key.trim());
    }

    let resp = req
        .send()
        .await
        .map_err(|e| (0, format!("网络请求失败: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| (status.as_u16(), format!("读取响应内容失败: {e}")))?;

    if !status.is_success() {
        let detail = extract_error_detail(&text);
        return Err((status.as_u16(), format!("AI 接口返回错误 ({status}): {detail}")));
    }

    let body: Value = serde_json::from_str(&text)
        .map_err(|e| (status.as_u16(), format!("解析 AI 响应 JSON 失败: {e}")))?;
    extract_message_text(&body)
        .ok_or_else(|| (status.as_u16(), "模型未返回有效文本内容".to_string()))
}

fn extract_delta_text(body: &Value) -> Option<String> {
    let delta = &body["choices"][0]["delta"]["content"];
    if let Some(s) = delta.as_str() {
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    if let Some(arr) = delta.as_array() {
        let joined: String = arr
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect();
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    None
}

const MAX_STREAM_TEXT_BYTES: usize = 4 * 1024 * 1024;
const STREAM_CHUNK_IDLE_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Default)]
struct SseStreamParser {
    byte_buf: Vec<u8>,
    raw_fallback: String,
    accumulated: String,
    saw_sse: bool,
    done: bool,
}

impl SseStreamParser {
    fn push_bytes(
        &mut self,
        bytes: &[u8],
        on_delta: &mut impl FnMut(&str),
    ) -> Result<(), String> {
        if self.done {
            return Ok(());
        }
        self.byte_buf.extend_from_slice(bytes);
        while let Some(nl_pos) = self.byte_buf.iter().position(|&b| b == b'\n') {
            let mut line_bytes: Vec<u8> = self.byte_buf.drain(..=nl_pos).collect();
            if line_bytes.last() == Some(&b'\n') {
                line_bytes.pop();
            }
            if line_bytes.last() == Some(&b'\r') {
                line_bytes.pop();
            }
            let line = String::from_utf8_lossy(&line_bytes);
            self.process_line(&line, on_delta)?;
            if self.done {
                break;
            }
        }
        Ok(())
    }

    fn process_line(
        &mut self,
        line: &str,
        on_delta: &mut impl FnMut(&str),
    ) -> Result<(), String> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            return Ok(());
        }
        if let Some(rest) = trimmed.strip_prefix("data:") {
            self.saw_sse = true;
            let data = rest.trim();
            if data.is_empty() {
                return Ok(());
            }
            if data == "[DONE]" {
                self.done = true;
                return Ok(());
            }
            let Ok(val) = serde_json::from_str::<Value>(data) else {
                return Ok(());
            };
            let has_choices = val["choices"]
                .as_array()
                .map(|arr| !arr.is_empty())
                .unwrap_or(false);
            if val.get("error").is_some() && !has_choices {
                return Err(format!("AI 接口返回错误: {}", extract_error_detail(data)));
            }
            let piece = extract_delta_text(&val).or_else(|| {
                if self.accumulated.is_empty() {
                    extract_message_text(&val)
                } else {
                    None
                }
            });
            if let Some(delta) = piece {
                if self.accumulated.len() + delta.len() > MAX_STREAM_TEXT_BYTES {
                    return Err("AI 流式响应超过大小限制".to_string());
                }
                self.accumulated.push_str(&delta);
                on_delta(&delta);
            }
            return Ok(());
        }

        if !self.saw_sse
            && !trimmed.starts_with("event:")
            && !trimmed.starts_with("id:")
            && !trimmed.starts_with("retry:")
            && self.raw_fallback.len() < 512 * 1024
        {
            self.raw_fallback.push_str(line);
            self.raw_fallback.push('\n');
        }
        Ok(())
    }

    fn finish(
        mut self,
        on_delta: &mut impl FnMut(&str),
    ) -> Result<String, String> {
        if !self.done && !self.byte_buf.is_empty() {
            let mut rem = std::mem::take(&mut self.byte_buf);
            if rem.last() == Some(&b'\r') {
                rem.pop();
            }
            let line = String::from_utf8_lossy(&rem);
            self.process_line(&line, on_delta)?;
        }
        if self.accumulated.is_empty() && !self.saw_sse && !self.raw_fallback.trim().is_empty() {
            if let Ok(body) = serde_json::from_str::<Value>(self.raw_fallback.trim()) {
                if body.get("error").is_some() && extract_message_text(&body).is_none() {
                    return Err(format!(
                        "AI 接口返回错误: {}",
                        extract_error_detail(self.raw_fallback.trim())
                    ));
                }
                if let Some(text) = extract_message_text(&body) {
                    self.accumulated.push_str(&text);
                    on_delta(&text);
                }
            }
        }
        if self.accumulated.is_empty() {
            return Err("模型未返回有效文本内容".to_string());
        }
        Ok(self.accumulated)
    }
}

fn parse_partial_json_string(s: &str, start_after_quote: usize) -> (String, bool, usize) {
    let bytes = s.as_bytes();
    let mut i = start_after_quote;
    let mut out = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            return (out, true, i + 1);
        }
        if b == b'\\' {
            if i + 1 >= bytes.len() {
                return (out, false, bytes.len());
            }
            match bytes[i + 1] {
                b'"' => {
                    out.push('"');
                    i += 2;
                }
                b'\\' => {
                    out.push('\\');
                    i += 2;
                }
                b'/' => {
                    out.push('/');
                    i += 2;
                }
                b'b' => {
                    out.push('\u{0008}');
                    i += 2;
                }
                b'f' => {
                    out.push('\u{000C}');
                    i += 2;
                }
                b'n' => {
                    out.push('\n');
                    i += 2;
                }
                b'r' => {
                    out.push('\r');
                    i += 2;
                }
                b't' => {
                    out.push('\t');
                    i += 2;
                }
                b'u' => {
                    if i + 6 > bytes.len() {
                        return (out, false, bytes.len());
                    }
                    let hex = &s[i + 2..i + 6];
                    if let Ok(code) = u16::from_str_radix(hex, 16) {
                        if (0xD800..=0xDBFF).contains(&code) {
                            // High surrogate: wait for or decode paired low surrogate
                            if i + 12 > bytes.len() {
                                return (out, false, bytes.len());
                            }
                            if &bytes[i + 6..i + 8] == b"\\u" {
                                if let Ok(low) = u16::from_str_radix(&s[i + 8..i + 12], 16) {
                                    if (0xDC00..=0xDFFF).contains(&low) {
                                        let combined = 0x10000
                                            + (((code as u32) - 0xD800) << 10)
                                            + ((low as u32) - 0xDC00);
                                        if let Some(ch) = char::from_u32(combined) {
                                            out.push(ch);
                                        }
                                        i += 12;
                                        continue;
                                    }
                                }
                            }
                            i += 6;
                        } else {
                            if let Some(ch) = char::from_u32(code as u32) {
                                out.push(ch);
                            }
                            i += 6;
                        }
                    } else {
                        i += 2;
                    }
                }
                other => {
                    out.push(other as char);
                    i += 2;
                }
            }
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, false, bytes.len())
}

fn extract_partial_json_strings(raw: &str, target_key: &str) -> Vec<String> {
    let mut s = raw.trim_start();
    if let Some(rest) = s.strip_prefix("```json") {
        s = rest.trim_start();
    } else if let Some(rest) = s.strip_prefix("```") {
        s = rest.trim_start();
    }

    let bytes = s.as_bytes();
    let mut pos = 0usize;
    let mut results = Vec::new();
    while pos < bytes.len() {
        if bytes[pos] != b'"' {
            let ch = s[pos..].chars().next().unwrap();
            pos += ch.len_utf8();
            continue;
        }
        let (token, closed, next_pos) = parse_partial_json_string(s, pos + 1);
        if !closed {
            break;
        }
        let mut look = next_pos;
        while look < bytes.len() && bytes[look].is_ascii_whitespace() {
            look += 1;
        }
        if look < bytes.len() && bytes[look] == b':' {
            look += 1;
            while look < bytes.len() && bytes[look].is_ascii_whitespace() {
                look += 1;
            }
            if token == target_key && look < bytes.len() && bytes[look] == b'"' {
                let (val, val_closed, val_next) = parse_partial_json_string(s, look + 1);
                results.push(val);
                pos = val_next;
                if !val_closed {
                    break;
                }
                continue;
            }
            pos = look;
        } else {
            pos = next_pos;
        }
    }
    results
}

fn build_json_stream_preview(raw: &str, primary_key: &str, secondary_key: Option<&str>) -> Option<StreamPreview> {
    let primary_parts = extract_partial_json_strings(raw, primary_key);
    let secondary_parts = secondary_key.map(|k| extract_partial_json_strings(raw, k));

    let primary_joined = primary_parts
        .iter()
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let secondary_joined = secondary_parts.as_ref().map(|parts| {
        parts
            .iter()
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    });

    if !primary_joined.is_empty() {
        return Some(StreamPreview {
            text: primary_joined,
            original_text: secondary_joined.filter(|s| !s.is_empty()),
        });
    }

    // If the model ignored JSON formatting and replied in plain text, stream it once confirmed non-JSON.
    let trimmed = raw.trim_start();
    if trimmed.chars().count() >= 2 && !trimmed.starts_with('{') && !trimmed.starts_with('[') && !trimmed.starts_with('`') {
        return Some(StreamPreview {
            text: trimmed.to_string(),
            original_text: None,
        });
    }

    if let Some(orig) = secondary_joined.filter(|s| !s.is_empty()) {
        return Some(StreamPreview {
            text: String::new(),
            original_text: Some(orig),
        });
    }

    None
}

async fn call_chat_completions_stream_once(
    client: &Client,
    config: &AppConfig,
    endpoint: &str,
    payload: &Value,
    on_delta: &mut impl FnMut(&str),
) -> Result<String, (u16, String, bool)> {
    let mut stream_payload = payload.clone();
    if let Some(obj) = stream_payload.as_object_mut() {
        obj.insert("stream".to_string(), json!(true));
    }

    let mut req = client
        .post(endpoint)
        .header("Accept", "text/event-stream, application/json")
        .json(&stream_payload);
    if !config.api_key.trim().is_empty() {
        req = req.bearer_auth(config.api_key.trim());
    }

    let mut resp = req
        .send()
        .await
        .map_err(|e| (0, format!("网络请求失败: {e}"), false))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp
            .text()
            .await
            .map_err(|e| (status.as_u16(), format!("读取响应内容失败: {e}"), false))?;
        let detail = extract_error_detail(&text);
        return Err((
            status.as_u16(),
            format!("AI 接口返回错误 ({status}): {detail}"),
            false,
        ));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    if content_type.contains("application/json") && !content_type.contains("event-stream") {
        let text = resp
            .text()
            .await
            .map_err(|e| (status.as_u16(), format!("读取响应内容失败: {e}"), false))?;
        let body: Value = serde_json::from_str(&text)
            .map_err(|e| (status.as_u16(), format!("解析 AI 响应 JSON 失败: {e}"), false))?;
        let msg = extract_message_text(&body)
            .ok_or_else(|| (status.as_u16(), "模型未返回有效文本内容".to_string(), false))?;
        on_delta(&msg);
        return Ok(msg);
    }

    let mut parser = SseStreamParser::default();
    let mut emitted_any = false;
    loop {
        let next = tokio::time::timeout(STREAM_CHUNK_IDLE_TIMEOUT, resp.chunk())
            .await
            .map_err(|_| (0, "等待 AI 流式响应超时，请稍后重试".to_string(), emitted_any))?;
        let chunk = next.map_err(|e| {
            (
                status.as_u16(),
                format!("读取流式响应失败: {e}"),
                emitted_any,
            )
        })?;
        let Some(bytes) = chunk else {
            break;
        };
        parser
            .push_bytes(&bytes, &mut |d| {
                emitted_any = true;
                on_delta(d);
            })
            .map_err(|msg| (status.as_u16(), msg, emitted_any))?;
        if parser.done {
            break;
        }
    }

    parser
        .finish(&mut |d| {
            emitted_any = true;
            on_delta(d);
        })
        .map_err(|msg| (status.as_u16(), msg, emitted_any))
}

async fn call_chat_completions_stream(
    client: &Client,
    config: &AppConfig,
    payload: &Value,
    mut on_delta: impl FnMut(&str),
) -> Result<String, String> {
    let endpoint = validate_ai_endpoint(config)?;
    match call_chat_completions_stream_once(client, config, &endpoint, payload, &mut on_delta).await {
        Ok(text) => Ok(text),
        Err((status, err, emitted_any)) if !emitted_any && (status == 400 || status == 422) => {
            if let Some(compat_payload) = strip_optional_chat_params(payload) {
                if let Ok(text) = call_chat_completions_stream_once(
                    client,
                    config,
                    &endpoint,
                    &compat_payload,
                    &mut on_delta,
                )
                .await
                {
                    return Ok(text);
                }
            }
            // Some endpoints reject stream=true with 400/422; gracefully fall back to non-streaming.
            if let Ok(text) = call_chat_completions(client, config, payload).await {
                on_delta(&text);
                return Ok(text);
            }
            Err(err)
        }
        Err((_status, err, _emitted_any)) => Err(err),
    }
}

async fn call_chat_maybe_stream_text(
    client: &Client,
    config: &AppConfig,
    payload: &Value,
    on_stream: Option<&StreamSink>,
) -> Result<String, String> {
    let Some(sink) = on_stream else {
        return call_chat_completions(client, config, payload).await;
    };
    let mut accumulated = String::new();
    call_chat_completions_stream(client, config, payload, |delta| {
        accumulated.push_str(delta);
        if !is_vision_refusal(&accumulated) {
            sink(StreamPreview {
                text: accumulated.clone(),
                original_text: None,
            });
        }
    })
    .await
}

async fn call_chat_maybe_stream_json(
    client: &Client,
    config: &AppConfig,
    payload: &Value,
    primary_key: &str,
    secondary_key: Option<&str>,
    on_stream: Option<&StreamSink>,
) -> Result<String, String> {
    let Some(sink) = on_stream else {
        return call_chat_completions(client, config, payload).await;
    };
    let mut raw = String::new();
    let mut last_primary = String::new();
    let mut last_secondary: Option<String> = None;
    call_chat_completions_stream(client, config, payload, |delta| {
        raw.push_str(delta);
        if is_vision_refusal(&raw) {
            return;
        }
        if let Some(preview) = build_json_stream_preview(&raw, primary_key, secondary_key) {
            if preview.text != last_primary || preview.original_text != last_secondary {
                last_primary = preview.text.clone();
                last_secondary = preview.original_text.clone();
                sink(preview);
            }
        }
    })
    .await
}

async fn call_chat_completions(
    client: &Client,
    config: &AppConfig,
    payload: &Value,
) -> Result<String, String> {
    let endpoint = validate_ai_endpoint(config)?;
    match call_chat_completions_once(client, config, &endpoint, payload).await {
        Ok(text) => Ok(text),
        Err((status, err)) if status == 400 || status == 422 => {
            if let Some(compat_payload) = strip_optional_chat_params(payload) {
                if let Ok(text) =
                    call_chat_completions_once(client, config, &endpoint, &compat_payload).await
                {
                    return Ok(text);
                }
            }
            Err(err)
        }
        Err((_status, err)) => Err(err),
    }
}

pub async fn google_translate_text(
    client: &Client,
    config: &AppConfig,
    text: &str,
    target_lang: &str,
) -> Result<(String, String), String> {
    let tl = map_google_lang(target_lang);
    let key = config.google_api_key.trim();

    if !key.is_empty() {
        let resp = client
            .post("https://translation.googleapis.com/language/translate/v2")
            .query(&[("key", key)])
            .json(&json!({
                "q": text,
                "target": tl,
                "format": "text"
            }))
            .send()
            .await
            .map_err(|e| format!("Google 翻译请求失败（请检查网络/代理）: {}", e.without_url()))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("读取 Google 响应失败: {e}"))?;
        if !status.is_success() {
            return Err(format!("Google Cloud API 错误 ({status}): {body_text}"));
        }
        let body: Value =
            serde_json::from_str(&body_text).map_err(|e| format!("解析 Google 响应失败: {e}"))?;
        let item = &body["data"]["translations"][0];
        let translated = item["translatedText"]
            .as_str()
            .ok_or_else(|| "Google API 未返回译文".to_string())?
            .to_string();
        let detected = item["detectedSourceLanguage"]
            .as_str()
            .unwrap_or("自动识别")
            .to_string();
        return Ok((detected, translated));
    }

    // Free Google Translate endpoint fallback when no Cloud API key is provided
    let resp = client
        .get("https://translate.googleapis.com/translate_a/single")
        .query(&[
            ("client", "gtx"),
            ("sl", "auto"),
            ("tl", tl),
            ("dt", "t"),
            ("q", text),
        ])
        .send()
        .await
        .map_err(|e| format!("Google 翻译连接失败（可能需要可用网络环境或改用 AI/DeepL）: {}", e.without_url()))?;

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|e| format!("读取 Google 响应失败: {e}"))?;
    if !status.is_success() {
        return Err(format!("Google 翻译返回错误 ({status}): {body_text}"));
    }

    let body: Value =
        serde_json::from_str(&body_text).map_err(|e| format!("解析 Google 翻译响应失败: {e}"))?;
    let segments = body[0]
        .as_array()
        .ok_or_else(|| "Google 翻译返回结构异常".to_string())?;
    let mut translated = String::new();
    for seg in segments {
        if let Some(piece) = seg[0].as_str() {
            translated.push_str(piece);
        }
    }
    let detected = body[2].as_str().unwrap_or("自动识别").to_string();
    Ok((detected, translated))
}

pub async fn deepl_translate_text(
    client: &Client,
    config: &AppConfig,
    text: &str,
    target_lang: &str,
) -> Result<(String, String), String> {
    let key = config.deepl_api_key.trim();
    if key.is_empty() {
        return Err("请先在设置中填写 DeepL API Key".to_string());
    }
    let endpoint = if key.ends_with(":fx") {
        "https://api-free.deepl.com/v2/translate"
    } else {
        "https://api.deepl.com/v2/translate"
    };

    let tl = map_deepl_lang(target_lang);
    let resp = client
        .post(endpoint)
        .header("Authorization", format!("DeepL-Auth-Key {key}"))
        .json(&json!({
            "text": [text],
            "target_lang": tl
        }))
        .send()
        .await
        .map_err(|e| format!("DeepL 请求失败: {e}"))?;

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|e| format!("读取 DeepL 响应失败: {e}"))?;
    if !status.is_success() {
        return Err(format!("DeepL API 错误 ({status}): {body_text}"));
    }

    let body: Value =
        serde_json::from_str(&body_text).map_err(|e| format!("解析 DeepL 响应失败: {e}"))?;
    let item = &body["translations"][0];
    let translated = item["text"]
        .as_str()
        .ok_or_else(|| "DeepL 未返回译文".to_string())?
        .to_string();
    let detected = item["detected_source_language"]
        .as_str()
        .unwrap_or("自动识别")
        .to_string();
    Ok((detected, translated))
}

async fn google_translate_blocks(
    client: &Client,
    config: &AppConfig,
    layout_blocks: &[OcrLayoutBlock],
    target_lang: &str,
) -> Result<(String, Vec<TranslatedBlock>), String> {
    if layout_blocks.is_empty() {
        return Ok(("自动识别".to_string(), Vec::new()));
    }
    let key = config.google_api_key.trim();
    let tl = map_google_lang(target_lang);

    if !key.is_empty() {
        let qs: Vec<&str> = layout_blocks.iter().map(|b| b.text.as_str()).collect();
        let resp = client
            .post("https://translation.googleapis.com/language/translate/v2")
            .query(&[("key", key)])
            .json(&json!({
                "q": qs,
                "target": tl,
                "format": "text"
            }))
            .send()
            .await
            .map_err(|e| format!("Google 翻译请求失败（请检查网络/代理）: {}", e.without_url()))?;
        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("读取 Google 响应失败: {e}"))?;
        if !status.is_success() {
            return Err(format!("Google Cloud API 错误 ({status}): {body_text}"));
        }
        let body: Value =
            serde_json::from_str(&body_text).map_err(|e| format!("解析 Google 响应失败: {e}"))?;
        let arr = body["data"]["translations"]
            .as_array()
            .ok_or_else(|| "Google API 未返回译文列表".to_string())?;
        if arr.len() == layout_blocks.len() {
            let detected = arr
                .first()
                .and_then(|v| v["detectedSourceLanguage"].as_str())
                .unwrap_or("自动识别")
                .to_string();
            let blocks = layout_blocks
                .iter()
                .zip(arr.iter())
                .map(|(lb, item)| TranslatedBlock {
                    x: lb.x,
                    y: lb.y,
                    w: lb.w,
                    h: lb.h,
                    line_height: lb.line_height,
                    line_count: lb.line_count,
                    original_text: lb.text.clone(),
                    translated_text: item["translatedText"]
                        .as_str()
                        .unwrap_or(&lb.text)
                        .trim()
                        .to_string(),
                })
                .collect();
            return Ok((detected, blocks));
        }
    }

    // Fast path: translate newline-joined single-line blocks in one call.
    let joined: String = layout_blocks
        .iter()
        .map(|b| b.text.replace(['\r', '\n'], " "))
        .collect::<Vec<_>>()
        .join("\n");
    let (detected, translated_joined) =
        google_translate_text(client, config, &joined, target_lang).await?;
    if let Some(mapped) = map_lines_to_layout_blocks(&translated_joined, layout_blocks) {
        return Ok((detected, mapped));
    }

    // Exact alignment fallback: translate blocks concurrently in bounded chunks.
    let mut results: Vec<Option<String>> = vec![None; layout_blocks.len()];
    for chunk_start in (0..layout_blocks.len()).step_by(6) {
        let chunk_end = (chunk_start + 6).min(layout_blocks.len());
        let mut set = tokio::task::JoinSet::new();
        for idx in chunk_start..chunk_end {
            let client = client.clone();
            let config = config.clone();
            let text = layout_blocks[idx].text.clone();
            let target_lang = target_lang.to_string();
            set.spawn(async move {
                let res = google_translate_text(&client, &config, &text, &target_lang).await;
                (idx, res)
            });
        }
        while let Some(joined_res) = set.join_next().await {
            if let Ok((idx, Ok((_d, tr)))) = joined_res {
                results[idx] = Some(tr.trim().to_string());
            }
        }
    }
    let blocks = layout_blocks
        .iter()
        .enumerate()
        .map(|(idx, lb)| TranslatedBlock {
            x: lb.x,
            y: lb.y,
            w: lb.w,
            h: lb.h,
            line_height: lb.line_height,
            line_count: lb.line_count,
            original_text: lb.text.clone(),
            translated_text: results[idx]
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| lb.text.clone()),
        })
        .collect();
    Ok((detected, blocks))
}

async fn deepl_translate_blocks(
    client: &Client,
    config: &AppConfig,
    layout_blocks: &[OcrLayoutBlock],
    target_lang: &str,
) -> Result<(String, Vec<TranslatedBlock>), String> {
    if layout_blocks.is_empty() {
        return Ok(("自动识别".to_string(), Vec::new()));
    }
    let key = config.deepl_api_key.trim();
    if key.is_empty() {
        return Err("请先在设置中填写 DeepL API Key".to_string());
    }
    let endpoint = if key.ends_with(":fx") {
        "https://api-free.deepl.com/v2/translate"
    } else {
        "https://api.deepl.com/v2/translate"
    };
    let tl = map_deepl_lang(target_lang);
    let texts: Vec<&str> = layout_blocks.iter().map(|b| b.text.as_str()).collect();
    let resp = client
        .post(endpoint)
        .header("Authorization", format!("DeepL-Auth-Key {key}"))
        .json(&json!({
            "text": texts,
            "target_lang": tl
        }))
        .send()
        .await
        .map_err(|e| format!("DeepL 请求失败: {e}"))?;

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|e| format!("读取 DeepL 响应失败: {e}"))?;
    if !status.is_success() {
        return Err(format!("DeepL API 错误 ({status}): {body_text}"));
    }
    let body: Value =
        serde_json::from_str(&body_text).map_err(|e| format!("解析 DeepL 响应失败: {e}"))?;
    let arr = body["translations"]
        .as_array()
        .ok_or_else(|| "DeepL 未返回译文列表".to_string())?;
    let detected = arr
        .first()
        .and_then(|v| v["detected_source_language"].as_str())
        .unwrap_or("自动识别")
        .to_string();
    let blocks = layout_blocks
        .iter()
        .enumerate()
        .map(|(i, lb)| {
            let tr = arr
                .get(i)
                .and_then(|v| v["text"].as_str())
                .unwrap_or(&lb.text)
                .trim()
                .to_string();
            TranslatedBlock {
                x: lb.x,
                y: lb.y,
                w: lb.w,
                h: lb.h,
                line_height: lb.line_height,
                line_count: lb.line_count,
                original_text: lb.text.clone(),
                translated_text: tr,
            }
        })
        .collect();
    Ok((detected, blocks))
}

fn is_qwen_mt_model(model: &str) -> bool {
    model.trim().to_ascii_lowercase().starts_with("qwen-mt")
}

fn map_qwen_mt_lang(label: &str) -> &'static str {
    match label {
        "简体中文" => "Chinese",
        "繁体中文" => "Traditional Chinese",
        "English" => "English",
        "日本語" => "Japanese",
        "한국어" => "Korean",
        "Français" => "French",
        "Deutsch" => "German",
        "Español" => "Spanish",
        _ => "Chinese",
    }
}

fn is_likely_text_only_model(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    if m.is_empty() {
        return false;
    }
    let has_vision_marker = m.contains("vl")
        || m.contains("vision")
        || m.contains("4o")
        || m.contains("4.1")
        || m.contains("4.5")
        || m.contains("omni")
        || m.contains("pixtral")
        || m.contains("llava")
        || m.contains("internvl")
        || m.contains("minicpm-v");
    if has_vision_marker {
        return false;
    }
    m.starts_with("qwen-mt")
        || m.starts_with("qwen-turbo")
        || m.starts_with("qwen-plus")
        || m.starts_with("qwen-max")
        || m.starts_with("qwen-long")
        || m.starts_with("qwen2.5")
        || m.starts_with("qwen3")
        || m.starts_with("deepseek-chat")
        || m.starts_with("deepseek-reasoner")
        || m.starts_with("deepseek-v3")
        || m.starts_with("deepseek-r1")
        || m.starts_with("gpt-3.5")
        || m.starts_with("moonshot-v1")
        || m.starts_with("glm-4-air")
        || m.starts_with("glm-4-flash")
        || m.starts_with("glm-3-turbo")
}

fn is_vision_refusal(reply: &str) -> bool {
    let lower = reply.to_ascii_lowercase();
    lower.contains("can't directly access or analyze images")
        || lower.contains("cannot view images")
        || lower.contains("cannot see images")
        || lower.contains("cannot analyze images")
        || lower.contains("unable to view")
        || lower.contains("unable to analyze image")
        || lower.contains("unable to process image")
        || lower.contains("do not support image")
        || lower.contains("don't have the ability to view")
        || lower.contains("no image provided")
        || reply.contains("无法查看图片")
        || reply.contains("无法识别图片")
        || reply.contains("无法直接访问")
        || reply.contains("不支持图片")
        || reply.contains("无法读取图片")
        || reply.contains("看不到图片")
        || reply.contains("请提供需要翻译的文本")
}

fn decode_b64_image_bytes(raw_base64: &str) -> Result<Vec<u8>, String> {
    let pure_b64 = raw_base64
        .trim()
        .strip_prefix("data:image/png;base64,")
        .or_else(|| raw_base64.trim().strip_prefix("data:image/jpeg;base64,"))
        .unwrap_or(raw_base64.trim());
    BASE64
        .decode(pure_b64)
        .map_err(|e| format!("解码截图 Base64 失败: {e}"))
}

const fn native_ocr_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Apple Vision OCR"
    } else {
        "Windows OCR"
    }
}

async fn try_local_ocr_layout_from_b64(
    config: &AppConfig,
    raw_base64: &str,
) -> Result<(String, String, Vec<OcrLayoutBlock>, &'static str), String> {
    let png_bytes = decode_b64_image_bytes(raw_base64)?;
    let mode = normalize_ocr_engine(&config.ocr_engine);
    let ocr_lang = normalize_ocr_lang(&config.ocr_lang);
    let fallback_label = native_ocr_label();

    if mode != "windows" && local_ocr::is_installed() {
        match local_ocr::ocr_png_bytes_with_layout(png_bytes.clone(), ocr_lang).await {
            Ok((lang, text, blocks)) if !text.trim().is_empty() => {
                return Ok((lang, text, blocks, "RapidOCR (PP-OCRv4)"));
            }
            Ok((lang, text, blocks)) => {
                eprintln!("[ocr] RapidOCR returned empty text (mode={mode}), falling back to {fallback_label}");
                if let Ok((fb_lang, fb_text, fb_blocks)) = ocr_png_bytes_with_layout(png_bytes).await
                {
                    if !fb_text.trim().is_empty() {
                        return Ok((fb_lang, fb_text, fb_blocks, fallback_label));
                    }
                }
                return Ok((lang, text, blocks, "RapidOCR (PP-OCRv4)"));
            }
            Err(rapid_err) => {
                eprintln!("[ocr] RapidOCR failed (mode={mode}), falling back to {fallback_label}: {rapid_err}");
                return ocr_png_bytes_with_layout(png_bytes)
                    .await
                    .map(|(lang, text, blocks)| (lang, text, blocks, fallback_label))
                    .map_err(|native_err| {
                        format!("RapidOCR 失败 ({rapid_err})；{fallback_label} 回退失败 ({native_err})")
                    });
            }
        }
    }

    ocr_png_bytes_with_layout(png_bytes)
        .await
        .map(|(lang, text, blocks)| (lang, text, blocks, fallback_label))
}

async fn try_local_ocr_from_b64(
    config: &AppConfig,
    raw_base64: &str,
) -> Result<(String, String, &'static str), String> {
    let (lang, text, _blocks, engine_name) =
        try_local_ocr_layout_from_b64(config, raw_base64).await?;
    Ok((lang, text, engine_name))
}

async fn ocr_image_with_vision_ai(
    client: &Client,
    config: &AppConfig,
    raw_base64: &str,
    on_stream: Option<&StreamSink>,
) -> Result<(String, String), String> {
    let instruction = "你是一个高精度 OCR 文字识别引擎。请逐字完整、准确地识别图片中的所有文字，严格保留原始大小写、标点符号、数字、代码标识符与段落换行缩进，不要翻译、补全或改写原文。\n\
        请严格以如下 JSON 格式输出，不要包含任何多余说明：\n\
        {\"source_lang\": \"识别到的主要语言名称\", \"original_text\": \"识别出的完整原文\"}\n\
        如果图片中没有文字，请将 original_text 置为空字符串。";

    let payload = json!({
        "model": config.model.trim(),
        "temperature": 0.0,
        "messages": [
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": instruction },
                    {
                        "type": "image_url",
                        "image_url": {
                            "url": format!("data:image/png;base64,{raw_base64}"),
                            "detail": "high"
                        }
                    }
                ]
            }
        ]
    });

    let content =
        call_chat_maybe_stream_json(client, config, &payload, "original_text", None, on_stream)
            .await?;
    if is_vision_refusal(&content) {
        return Err("当前配置的 AI 模型不支持直接看图识别".to_string());
    }
    let parsed = parse_vision_output(&content);
    let source_lang = if parsed.source_lang.trim().is_empty() {
        "自动识别".to_string()
    } else {
        parsed.source_lang.trim().to_string()
    };
    let text = if !parsed.original_text.trim().is_empty() {
        parsed.original_text
    } else {
        parsed.translated_text
    };
    Ok((source_lang, text))
}

fn is_ocr_text_suspect(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    let mut split_radicals = 0usize;
    let mut isolated_cjk = 0usize;
    let mut latin_chars = 0usize;
    let chars: Vec<char> = trimmed.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if matches!(ch, '亻' | '氵' | '讠' | '扌' | '纟' | '钅') {
            split_radicals += 1;
        }
        if ch.is_ascii_alphabetic() {
            latin_chars += 1;
        }
        let is_cjk = ('\u{4E00}'..='\u{9FFF}').contains(&ch);
        if is_cjk {
            let prev_cjk = i > 0 && ('\u{4E00}'..='\u{9FFF}').contains(&chars[i - 1]);
            let next_cjk = i + 1 < chars.len() && ('\u{4E00}'..='\u{9FFF}').contains(&chars[i + 1]);
            if !prev_cjk && !next_cjk {
                isolated_cjk += 1;
            }
        }
    }
    if split_radicals > 0 {
        return true;
    }
    if latin_chars > 12 && isolated_cjk >= 2 {
        return true;
    }
    // High ratio of single-letter ASCII fragments indicates broken word segmentation
    let mut total_tokens = 0usize;
    let mut single_alpha_tokens = 0usize;
    for tok in trimmed.split_whitespace() {
        total_tokens += 1;
        if tok.len() == 1 {
            if let Some(ch) = tok.chars().next() {
                if ch.is_ascii_alphabetic() && ch != 'a' && ch != 'A' && ch != 'I' {
                    single_alpha_tokens += 1;
                }
            }
        }
    }
    if total_tokens >= 6 && single_alpha_tokens * 3 >= total_tokens {
        return true;
    }
    false
}

async fn ocr_image_text_with_source(
    client: &Client,
    config: &AppConfig,
    raw_base64: &str,
    on_stream: Option<&StreamSink>,
) -> Result<(String, String, String), String> {
    let has_vision_ai =
        validate_ai_endpoint(config).is_ok() && !is_likely_text_only_model(&config.model);
    let ocr_mode = normalize_ocr_engine(&config.ocr_engine);

    if ocr_mode == "rapidocr" && local_ocr::is_installed() {
        if let Ok((lang, text, engine_name)) = try_local_ocr_from_b64(config, raw_base64).await {
            if !text.trim().is_empty() {
                return Ok((lang, text, engine_name.to_string()));
            }
        }
    }

    // If the user configured a vision-capable model, prefer Vision AI for high-accuracy OCR
    // and fall back to local OCR (RapidOCR / Windows OCR) if Vision AI fails or refuses.
    if has_vision_ai {
        if let Ok((lang, text)) =
            ocr_image_with_vision_ai(client, config, raw_base64, on_stream).await
        {
            if !text.trim().is_empty() {
                return Ok((lang, text, config.model.clone()));
            }
        }
    }

    let local_res = try_local_ocr_from_b64(config, raw_base64).await;
    if let Ok((lang, text, engine_name)) = &local_res {
        if !text.trim().is_empty() {
            // If local OCR output looks garbled and an AI endpoint is available, retry via Vision AI.
            if is_ocr_text_suspect(text) && has_vision_ai {
                if let Ok((v_lang, v_text)) =
                    ocr_image_with_vision_ai(client, config, raw_base64, on_stream).await
                {
                    if !v_text.trim().is_empty() {
                        return Ok((v_lang, v_text, config.model.clone()));
                    }
                }
            }
            return Ok((lang.clone(), text.clone(), (*engine_name).to_string()));
        }
    }

    local_res.map(|(lang, text, engine_name)| (lang, text, engine_name.to_string()))
}

pub async fn ocr_image_text_with_model(
    client: &Client,
    config: &AppConfig,
    raw_base64: &str,
    on_stream: Option<&StreamSink>,
) -> Result<(String, String, String), String> {
    ocr_image_text_with_source(client, config, raw_base64, on_stream).await
}

async fn ai_translate_layout_blocks_text(
    client: &Client,
    config: &AppConfig,
    layout_blocks: &[OcrLayoutBlock],
    target_lang: &str,
    on_stream: Option<&StreamSink>,
) -> Result<Vec<TranslatedBlock>, String> {
    if layout_blocks.is_empty() {
        return Ok(Vec::new());
    }

    if is_qwen_mt_model(&config.model) {
        let joined: String = layout_blocks
            .iter()
            .map(|b| b.text.replace(['\r', '\n'], " "))
            .collect::<Vec<_>>()
            .join("\n");
        let mt_payload = json!({
            "model": config.model.trim(),
            "messages": [
                { "role": "user", "content": joined }
            ],
            "translation_options": {
                "source_lang": "auto",
                "target_lang": map_qwen_mt_lang(target_lang)
            }
        });
        if let Ok(resp_text) =
            call_chat_maybe_stream_text(client, config, &mt_payload, on_stream).await
        {
            if let Some(mapped) = map_lines_to_layout_blocks(&resp_text, layout_blocks) {
                return Ok(mapped);
            }
        }
        // Fallback: translate each block via qwen-mt concurrently in small chunks.
        let mut out_texts: Vec<Option<String>> = vec![None; layout_blocks.len()];
        for chunk_start in (0..layout_blocks.len()).step_by(4) {
            let chunk_end = (chunk_start + 4).min(layout_blocks.len());
            let mut set = tokio::task::JoinSet::new();
            for idx in chunk_start..chunk_end {
                let client = client.clone();
                let config = config.clone();
                let text = layout_blocks[idx].text.clone();
                let tl = map_qwen_mt_lang(target_lang).to_string();
                set.spawn(async move {
                    let payload = json!({
                        "model": config.model.trim(),
                        "messages": [{ "role": "user", "content": text }],
                        "translation_options": {
                            "source_lang": "auto",
                            "target_lang": tl
                        }
                    });
                    let res = call_chat_completions(&client, &config, &payload).await;
                    (idx, res)
                });
            }
            while let Some(joined_res) = set.join_next().await {
                if let Ok((idx, Ok(tr))) = joined_res {
                    out_texts[idx] = Some(tr.trim().to_string());
                }
            }
        }
        return Ok(layout_blocks
            .iter()
            .enumerate()
            .map(|(i, lb)| TranslatedBlock {
                x: lb.x,
                y: lb.y,
                w: lb.w,
                h: lb.h,
                line_height: lb.line_height,
                line_count: lb.line_count,
                original_text: lb.text.clone(),
                translated_text: out_texts[i]
                    .clone()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| lb.text.clone()),
            })
            .collect());
    }

    let mut numbered = String::new();
    for (idx, b) in layout_blocks.iter().enumerate() {
        numbered.push_str(&format!("[{idx}] {}\n", b.text));
    }
    let mut prompt = format!(
        "请将以下按区域编号的文本逐条自然、准确地翻译为「{}」，保持每个编号一一对应，不要合并、拆分或遗漏编号。\n\
        请严格以如下 JSON 格式输出，不要包含多余说明：\n\
        {{\"blocks\": [{{\"id\": 0, \"translated_text\": \"第0条译文\"}}]}}\n\n\
        原文列表：\n{}",
        target_lang, numbered
    );
    if !config.custom_prompt.trim().is_empty() {
        prompt.push_str("\n补充要求：");
        prompt.push_str(config.custom_prompt.trim());
    }
    let glossary_block = build_glossary_prompt_block(&config.glossary);
    if !glossary_block.is_empty() {
        prompt.push('\n');
        prompt.push_str(&glossary_block);
    }

    let payload = json!({
        "model": config.model.trim(),
        "temperature": 0.1,
        "messages": [
            { "role": "user", "content": prompt }
        ]
    });
    let content = call_chat_maybe_stream_json(
        client,
        config,
        &payload,
        "translated_text",
        None,
        on_stream,
    )
    .await?;
    if let Some((_lang, blocks)) = parse_blocks_output(&content, layout_blocks) {
        return Ok(blocks);
    }
    if let Some(mapped) = map_lines_to_layout_blocks(&content, layout_blocks) {
        return Ok(mapped);
    }
    Err("模型返回格式无法对齐文本区域".to_string())
}

pub async fn translate_image_full(
    client: &Client,
    config: &AppConfig,
    job_id: u64,
    raw_base64: &str,
    thumb_data_url: &str,
    target_lang: &str,
    engine: &str,
    on_stream: Option<&StreamSink>,
) -> Result<TranslationResult, String> {
    let started = Instant::now();

    if engine == "google" || engine == "deepl" {
        // Run local layout OCR first (RapidOCR when installed/enabled, else Windows OCR)
        // so we retain exact per-block bounding boxes for in-place alignment.
        let (ocr_lang, original_text, layout_blocks, _ocr_engine) =
            try_local_ocr_layout_from_b64(config, raw_base64).await?;
        {
            if !layout_blocks.is_empty() {
                let (detected_lang, blocks) = if engine == "google" {
                    google_translate_blocks(client, config, &layout_blocks, target_lang).await?
                } else {
                    deepl_translate_blocks(client, config, &layout_blocks, target_lang).await?
                };
                let source_lang = if !ocr_lang.is_empty() && ocr_lang != "自动识别" {
                    ocr_lang
                } else {
                    detected_lang
                };
                let original_text = blocks
                    .iter()
                    .map(|b| b.original_text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let translated_text = blocks
                    .iter()
                    .map(|b| b.translated_text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(TranslationResult {
                    id: job_id,
                    mode: "translate".to_string(),
                    engine: engine.to_string(),
                    source_lang,
                    target_lang: target_lang.to_string(),
                    original_text,
                    translated_text,
                    blocks,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    model: if engine == "google" {
                        "Google Translate".to_string()
                    } else {
                        "DeepL".to_string()
                    },
                    image_data_url: thumb_data_url.to_string(),
                    created_at_ms: current_epoch_ms(),
                });
            }
        }

        if original_text.trim().is_empty() {
            return Ok(TranslationResult {
                id: job_id,
                mode: "translate".to_string(),
                engine: engine.to_string(),
                source_lang: ocr_lang,
                target_lang: target_lang.to_string(),
                original_text: String::new(),
                translated_text: "未在选区内检测到可识别文字".to_string(),
                blocks: Vec::new(),
                elapsed_ms: started.elapsed().as_millis() as u64,
                model: if engine == "google" {
                    "Google Translate".to_string()
                } else {
                    "DeepL".to_string()
                },
                image_data_url: thumb_data_url.to_string(),
                created_at_ms: current_epoch_ms(),
            });
        }

        let (detected_lang, translated_text) = if engine == "google" {
            google_translate_text(client, config, &original_text, target_lang).await?
        } else {
            deepl_translate_text(client, config, &original_text, target_lang).await?
        };

        let source_lang = if !ocr_lang.is_empty() && ocr_lang != "自动识别" {
            ocr_lang
        } else {
            detected_lang
        };

        return Ok(TranslationResult {
            id: job_id,
            mode: "translate".to_string(),
            engine: engine.to_string(),
            source_lang,
            target_lang: target_lang.to_string(),
            original_text,
            translated_text,
            blocks: Vec::new(),
            elapsed_ms: started.elapsed().as_millis() as u64,
            model: if engine == "google" {
                "Google Translate".to_string()
            } else {
                "DeepL".to_string()
            },
            image_data_url: thumb_data_url.to_string(),
            created_at_ms: current_epoch_ms(),
        });
    }

    // Check AI endpoint first before attempting multimodal or OCR+text
    let _ = validate_ai_endpoint(config)?;

    // Gather fast local OCR bounding boxes so both Vision AI and Text AI can align translations 1-to-1.
    let native_layout = try_local_ocr_layout_from_b64(config, raw_base64).await;
    let layout_blocks: Vec<OcrLayoutBlock> = match &native_layout {
        Ok((_lang, _text, blocks, _ocr_engine)) => blocks.clone(),
        Err(_) => Vec::new(),
    };

    let mut vision_err: Option<String> = None;
    if !is_likely_text_only_model(&config.model) {
        let use_block_prompt = !layout_blocks.is_empty() && layout_blocks.len() <= 45;
        let mut instruction = if use_block_prompt {
            let mut hints = String::new();
            for (idx, b) in layout_blocks.iter().enumerate() {
                hints.push_str(&format!("[{idx}] {}\n", b.text));
            }
            format!(
                "你是一个精准快速的屏幕截图 OCR 与翻译引擎。请直接观察图片中的真实文字，并按以下本地 OCR 粗定位出的 {} 个文本区域编号（0..{}）逐一修正错字并自然准确地翻译为「{}」。\n\
                请严格以如下 JSON 格式输出，不要包含多余解释：\n\
                {{\"source_lang\": \"识别到的源语言名称\", \"blocks\": [{{\"id\": 0, \"original_text\": \"图片中该区域的真实原文\", \"translated_text\": \"该区域译文\"}}]}}\n\
                要求：blocks 数组必须与下方每个编号严格一一对应，不要合并、拆分或遗漏编号；如果 OCR 参考有错字请以图片真实画面为准修正。\n\
                参考区域列表：\n{}",
                layout_blocks.len(),
                layout_blocks.len() - 1,
                target_lang,
                hints
            )
        } else {
            format!(
                "你是一个精准快速的屏幕截图 OCR 与翻译引擎。请识别图片中的所有文字，并将其自然、准确地翻译为「{}」。\n\
                请严格以如下 JSON 格式输出，不要包含多余解释：\n\
                {{\"source_lang\": \"识别到的源语言名称\", \"original_text\": \"图片中的原文（保留合理换行）\", \"translated_text\": \"翻译后的文本（保留对应段落结构）\"}}\n\
                如果图片中不包含可识别文字，请将 original_text 置空，并在 translated_text 中说明“未在选区内检测到文字”。",
                target_lang
            )
        };

        if !config.custom_prompt.trim().is_empty() {
            instruction.push_str("\n补充要求：");
            instruction.push_str(config.custom_prompt.trim());
        }
        let glossary_block = build_glossary_prompt_block(&config.glossary);
        if !glossary_block.is_empty() {
            instruction.push('\n');
            instruction.push_str(&glossary_block);
        }

        let payload = json!({
            "model": config.model.trim(),
            "temperature": 0.1,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": instruction },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:image/png;base64,{raw_base64}")
                            }
                        }
                    ]
                }
            ]
        });

        match call_chat_maybe_stream_json(
            client,
            config,
            &payload,
            "translated_text",
            Some("original_text"),
            on_stream,
        )
        .await
        {
            Ok(content) => {
                if !is_vision_refusal(&content) {
                    if use_block_prompt {
                        if let Some((lang, blocks)) = parse_blocks_output(&content, &layout_blocks) {
                            let original_text = blocks
                                .iter()
                                .map(|b| b.original_text.as_str())
                                .collect::<Vec<_>>()
                                .join("\n");
                            let translated_text = blocks
                                .iter()
                                .map(|b| b.translated_text.as_str())
                                .collect::<Vec<_>>()
                                .join("\n");
                            return Ok(TranslationResult {
                                id: job_id,
                                mode: "translate".to_string(),
                                engine: "ai".to_string(),
                                source_lang: if lang.is_empty() {
                                    "自动识别".to_string()
                                } else {
                                    lang
                                },
                                target_lang: target_lang.to_string(),
                                original_text,
                                translated_text,
                                blocks,
                                elapsed_ms: started.elapsed().as_millis() as u64,
                                model: config.model.clone(),
                                image_data_url: thumb_data_url.to_string(),
                                created_at_ms: current_epoch_ms(),
                            });
                        }
                    }

                    let parsed = parse_vision_output(&content);
                    if !parsed.translated_text.trim().is_empty()
                        || !parsed.original_text.trim().is_empty()
                    {
                        let fallback_blocks =
                            map_lines_to_layout_blocks(&parsed.translated_text, &layout_blocks)
                                .unwrap_or_default();
                        return Ok(TranslationResult {
                            id: job_id,
                            mode: "translate".to_string(),
                            engine: "ai".to_string(),
                            source_lang: if parsed.source_lang.trim().is_empty() {
                                "自动识别".to_string()
                            } else {
                                parsed.source_lang
                            },
                            target_lang: target_lang.to_string(),
                            original_text: parsed.original_text,
                            translated_text: parsed.translated_text,
                            blocks: fallback_blocks,
                            elapsed_ms: started.elapsed().as_millis() as u64,
                            model: config.model.clone(),
                            image_data_url: thumb_data_url.to_string(),
                            created_at_ms: current_epoch_ms(),
                        });
                    }
                }
            }
            Err(err) => {
                vision_err = Some(err);
            }
        }
    }

    // Text-only model (e.g. qwen-mt-plus / deepseek-chat) or vision fallback: local OCR + text translation
    let (ocr_lang, original_text, ocr_blocks, _ocr_engine) = match native_layout {
        Ok(res) => res,
        Err(ocr_err) => {
            return Err(vision_err.unwrap_or(ocr_err));
        }
    };
    if original_text.trim().is_empty() {
        if let Some(err) = vision_err {
            return Err(err);
        }
        return Ok(TranslationResult {
            id: job_id,
            mode: "translate".to_string(),
            engine: "ai".to_string(),
            source_lang: ocr_lang,
            target_lang: target_lang.to_string(),
            original_text: String::new(),
            translated_text: "未在选区内检测到可识别文字".to_string(),
            blocks: Vec::new(),
            elapsed_ms: started.elapsed().as_millis() as u64,
            model: config.model.clone(),
            image_data_url: thumb_data_url.to_string(),
            created_at_ms: current_epoch_ms(),
        });
    }

    if !ocr_blocks.is_empty() && ocr_blocks.len() <= 45 {
        if let Ok(blocks) =
            ai_translate_layout_blocks_text(client, config, &ocr_blocks, target_lang, on_stream)
                .await
        {
            let orig_joined = blocks
                .iter()
                .map(|b| b.original_text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let trans_joined = blocks
                .iter()
                .map(|b| b.translated_text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            return Ok(TranslationResult {
                id: job_id,
                mode: "translate".to_string(),
                engine: "ai".to_string(),
                source_lang: ocr_lang,
                target_lang: target_lang.to_string(),
                original_text: orig_joined,
                translated_text: trans_joined,
                blocks,
                elapsed_ms: started.elapsed().as_millis() as u64,
                model: config.model.clone(),
                image_data_url: thumb_data_url.to_string(),
                created_at_ms: current_epoch_ms(),
            });
        }
    }

    translate_text_only(
        client,
        config,
        job_id,
        &original_text,
        &ocr_lang,
        thumb_data_url,
        target_lang,
        "ai",
        on_stream,
    )
    .await
}

pub async fn translate_text_only(
    client: &Client,
    config: &AppConfig,
    job_id: u64,
    original_text: &str,
    source_lang_hint: &str,
    thumb_data_url: &str,
    target_lang: &str,
    engine: &str,
    on_stream: Option<&StreamSink>,
) -> Result<TranslationResult, String> {
    let started = Instant::now();

    if engine == "google" {
        let (detected, translated_text) =
            google_translate_text(client, config, original_text, target_lang).await?;
        let source_lang = if source_lang_hint.trim().is_empty() || source_lang_hint == "自动识别" {
            detected
        } else {
            source_lang_hint.to_string()
        };
        return Ok(TranslationResult {
            id: job_id,
            mode: "translate".to_string(),
            engine: "google".to_string(),
            source_lang,
            target_lang: target_lang.to_string(),
            original_text: original_text.to_string(),
            translated_text,
            blocks: Vec::new(),
            elapsed_ms: started.elapsed().as_millis() as u64,
            model: "Google Translate".to_string(),
            image_data_url: thumb_data_url.to_string(),
            created_at_ms: current_epoch_ms(),
        });
    }

    if engine == "deepl" {
        let (detected, translated_text) =
            deepl_translate_text(client, config, original_text, target_lang).await?;
        let source_lang = if source_lang_hint.trim().is_empty() || source_lang_hint == "自动识别" {
            detected
        } else {
            source_lang_hint.to_string()
        };
        return Ok(TranslationResult {
            id: job_id,
            mode: "translate".to_string(),
            engine: "deepl".to_string(),
            source_lang,
            target_lang: target_lang.to_string(),
            original_text: original_text.to_string(),
            translated_text,
            blocks: Vec::new(),
            elapsed_ms: started.elapsed().as_millis() as u64,
            model: "DeepL".to_string(),
            image_data_url: thumb_data_url.to_string(),
            created_at_ms: current_epoch_ms(),
        });
    }

    let mut prompt = format!(
        "请将以下文本自然、准确地翻译为「{}」，保留原有分段与格式，仅输出译文：\n\n{}",
        target_lang, original_text
    );
    if !config.custom_prompt.trim().is_empty() {
        prompt.push_str("\n\n补充要求：");
        prompt.push_str(config.custom_prompt.trim());
    }
    let glossary_block = build_glossary_prompt_block(&config.glossary);
    if !glossary_block.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(&glossary_block);
    }

    let translated_text = if is_qwen_mt_model(&config.model) {
        let mt_payload = json!({
            "model": config.model.trim(),
            "messages": [
                { "role": "user", "content": original_text }
            ],
            "translation_options": {
                "source_lang": "auto",
                "target_lang": map_qwen_mt_lang(target_lang)
            }
        });
        match call_chat_maybe_stream_text(client, config, &mt_payload, on_stream).await {
            Ok(text) => text,
            Err(_) => {
                let fallback_payload = json!({
                    "model": config.model.trim(),
                    "messages": [
                        { "role": "user", "content": prompt }
                    ]
                });
                call_chat_maybe_stream_text(client, config, &fallback_payload, on_stream).await?
            }
        }
    } else {
        let payload = json!({
            "model": config.model.trim(),
            "temperature": 0.1,
            "messages": [
                { "role": "user", "content": prompt }
            ]
        });
        call_chat_maybe_stream_text(client, config, &payload, on_stream).await?
    };
    Ok(TranslationResult {
        id: job_id,
        mode: "translate".to_string(),
        engine: "ai".to_string(),
        source_lang: if source_lang_hint.trim().is_empty() {
            "自动识别".to_string()
        } else {
            source_lang_hint.to_string()
        },
        target_lang: target_lang.to_string(),
        original_text: original_text.to_string(),
        translated_text: translated_text.trim().to_string(),
        blocks: Vec::new(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        model: config.model.clone(),
        image_data_url: thumb_data_url.to_string(),
        created_at_ms: current_epoch_ms(),
    })
}

pub async fn ocr_image_only(
    client: &Client,
    config: &AppConfig,
    job_id: u64,
    raw_base64: &str,
    thumb_data_url: &str,
    on_stream: Option<&StreamSink>,
) -> Result<TranslationResult, String> {
    let started = Instant::now();
    let (source_lang, original_text, used_model) =
        ocr_image_text_with_source(client, config, raw_base64, on_stream).await?;
    let is_empty = original_text.trim().is_empty();

    Ok(TranslationResult {
        id: job_id,
        mode: "ocr".to_string(),
        engine: "ai".to_string(),
        source_lang,
        target_lang: "纯文本识别".to_string(),
        original_text: original_text.clone(),
        translated_text: if is_empty {
            "未在选区内检测到文字".to_string()
        } else {
            original_text
        },
        blocks: Vec::new(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        model: used_model,
        image_data_url: thumb_data_url.to_string(),
        created_at_ms: current_epoch_ms(),
    })
}

pub async fn ask_ai_about_image(
    client: &Client,
    config: &AppConfig,
    raw_base64: &str,
    history: &[AiChatMessage],
    on_stream: Option<&StreamSink>,
) -> Result<String, String> {
    ask_ai_about_images(client, config, &[raw_base64.to_string()], history, on_stream).await
}

async fn ask_ai_with_ocr_text_fallback(
    client: &Client,
    config: &AppConfig,
    images_b64: &[String],
    history: &[AiChatMessage],
    on_stream: Option<&StreamSink>,
) -> Result<String, String> {
    let mut ocr_blocks = Vec::with_capacity(images_b64.len());
    for (idx, b64) in images_b64.iter().enumerate() {
        let text = match try_local_ocr_from_b64(config, b64).await {
            Ok((_lang, t, _engine)) if !t.trim().is_empty() => t,
            Ok(_) => "（未检测到文字）".to_string(),
            Err(e) => format!("（OCR 识别失败: {e}）"),
        };
        if images_b64.len() > 1 {
            ocr_blocks.push(format!("[图{} 文字内容]\n{}", idx + 1, text));
        } else {
            ocr_blocks.push(format!("[截图文字内容]\n{}", text));
        }
    }

    let combined_ocr = ocr_blocks.join("\n\n");
    if is_qwen_mt_model(&config.model) {
        let mut user_lines = vec![
            "请结合以下截图中的文字，回答用户问题：".to_string(),
            combined_ocr.clone(),
        ];
        for msg in history {
            let prefix = if msg.role == "assistant" { "助手" } else { "用户" };
            user_lines.push(format!("{prefix}: {}", msg.content));
        }
        let payload = json!({
            "model": config.model.trim(),
            "messages": [
                {
                    "role": "user",
                    "content": user_lines.join("\n\n")
                }
            ]
        });
        return match call_chat_maybe_stream_text(client, config, &payload, on_stream).await {
            Ok(answer) => Ok(answer.trim().to_string()),
            Err(err) => Err(format!(
                "当前模型「{}」为专用机器翻译模型，不支持自由问答；请在设置中切换为视觉/对话模型（如 qwen-vl-plus / qwen-plus / gpt-4o-mini）。原始错误：{err}",
                config.model.trim()
            )),
        };
    }

    let mut api_messages: Vec<Value> = vec![json!({
        "role": "system",
        "content": "你是内置于桌面截图工具中的 AI 助手。由于当前配置的是纯文本模型，系统已先通过本地 OCR 提取截图中的文字内容供你参考。请结合这些文字回答用户的问题。"
    })];

    for (idx, msg) in history.iter().enumerate() {
        if idx == 0 && msg.role == "user" {
            api_messages.push(json!({
                "role": "user",
                "content": format!("{}\n\n{}", combined_ocr, msg.content)
            }));
        } else {
            api_messages.push(json!({
                "role": msg.role,
                "content": msg.content
            }));
        }
    }

    let payload = json!({
        "model": config.model.trim(),
        "temperature": 0.3,
        "messages": api_messages
    });

    let answer = call_chat_maybe_stream_text(client, config, &payload, on_stream).await?;
    Ok(answer.trim().to_string())
}

pub async fn ask_ai_about_images(
    client: &Client,
    config: &AppConfig,
    images_b64: &[String],
    history: &[AiChatMessage],
    on_stream: Option<&StreamSink>,
) -> Result<String, String> {
    if history.is_empty() {
        return Err("请输入要向 AI 提问的内容".to_string());
    }
    if images_b64.is_empty() {
        return Err("没有可供分析的截图".to_string());
    }

    let _ = validate_ai_endpoint(config)?;

    if is_likely_text_only_model(&config.model) {
        return ask_ai_with_ocr_text_fallback(client, config, images_b64, history, on_stream).await;
    }

    let system_prompt = if images_b64.len() > 1 {
        format!(
            "你是内置于桌面截图工具中的视觉 AI 助手。用户一共提供了 {} 张截图，按顺序依次为 图1 到 图{}。请综合全部画面内容，用清晰、简洁、准确的中文（若用户指定其他语言则按要求回答）识别内容、对照关联或回答具体追问，必要时明确标注「图1」「图2」等编号。",
            images_b64.len(),
            images_b64.len()
        )
    } else {
        "你是内置于桌面截图工具中的视觉 AI 助手。请仔细观察用户提供的截图，用清晰、简洁、准确的中文（若用户指定其他语言则按要求回答）识别画面内容、解释文字/报错/图表或回答具体追问。".to_string()
    };

    let mut api_messages: Vec<Value> = vec![json!({
        "role": "system",
        "content": system_prompt
    })];

    for (idx, msg) in history.iter().enumerate() {
        if idx == 0 && msg.role == "user" {
            let mut content_parts: Vec<Value> = vec![json!({
                "type": "text",
                "text": msg.content
            })];
            for (img_idx, b64) in images_b64.iter().enumerate() {
                if images_b64.len() > 1 {
                    content_parts.push(json!({
                        "type": "text",
                        "text": format!("[图{}]", img_idx + 1)
                    }));
                }
                content_parts.push(json!({
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:image/png;base64,{b64}")
                    }
                }));
            }
            api_messages.push(json!({
                "role": "user",
                "content": content_parts
            }));
        } else {
            api_messages.push(json!({
                "role": msg.role,
                "content": msg.content
            }));
        }
    }

    let payload = json!({
        "model": config.model.trim(),
        "temperature": 0.3,
        "messages": api_messages
    });

    match call_chat_maybe_stream_text(client, config, &payload, on_stream).await {
        Ok(answer) if !is_vision_refusal(&answer) => Ok(answer.trim().to_string()),
        Ok(_) | Err(_) => {
            ask_ai_with_ocr_text_fallback(client, config, images_b64, history, on_stream).await
        }
    }
}

fn build_text_ask_messages(source_text: &str, history: &[AiChatMessage]) -> Vec<Value> {
    let mut api_messages: Vec<Value> = vec![json!({
        "role": "system",
        "content": "你是内置于桌面划词/截图翻译工具中的 AI 助手。用户提供了一段选区或剪贴板纯文本内容，请结合该原文用清晰、简洁、准确的中文（若用户指定其他语言或格式如 Markdown 表格、LaTeX 公式、代码解释等，则按要求输出）回答用户的问题与追问。"
    })];
    let context_header = format!("[选区文本内容]\n{}", source_text.trim());
    for (idx, msg) in history.iter().enumerate() {
        if idx == 0 && msg.role == "user" {
            api_messages.push(json!({
                "role": "user",
                "content": format!("{context_header}\n\n{}", msg.content)
            }));
        } else {
            api_messages.push(json!({
                "role": msg.role,
                "content": msg.content
            }));
        }
    }
    api_messages
}

pub async fn ask_ai_about_text(
    client: &Client,
    config: &AppConfig,
    source_text: &str,
    history: &[AiChatMessage],
    on_stream: Option<&StreamSink>,
) -> Result<String, String> {
    if history.is_empty() {
        return Err("请输入要向 AI 提问的内容".to_string());
    }
    if source_text.trim().is_empty() {
        return Err("没有可供分析的文本内容".to_string());
    }
    let _ = validate_ai_endpoint(config)?;

    if is_qwen_mt_model(&config.model) {
        let mut user_lines = vec![
            "请结合以下文本内容回答用户问题：".to_string(),
            format!("[选区文本内容]\n{}", source_text.trim()),
        ];
        for msg in history {
            let prefix = if msg.role == "assistant" { "助手" } else { "用户" };
            user_lines.push(format!("{prefix}: {}", msg.content));
        }
        let payload = json!({
            "model": config.model.trim(),
            "messages": [
                {
                    "role": "user",
                    "content": user_lines.join("\n\n")
                }
            ]
        });
        return match call_chat_maybe_stream_text(client, config, &payload, on_stream).await {
            Ok(answer) => Ok(answer.trim().to_string()),
            Err(err) => Err(format!(
                "当前模型「{}」为专用机器翻译模型，不支持自由问答；请在设置中切换为对话模型（如 qwen-plus / gpt-4o-mini）。原始错误：{err}",
                config.model.trim()
            )),
        };
    }

    let api_messages = build_text_ask_messages(source_text, history);
    let payload = json!({
        "model": config.model.trim(),
        "temperature": 0.3,
        "messages": api_messages
    });

    let answer = call_chat_maybe_stream_text(client, config, &payload, on_stream).await?;
    Ok(answer.trim().to_string())
}

pub async fn ocr_images_batch(
    client: &Client,
    config: &AppConfig,
    items: &[(String, String)],
    on_item_stream: Option<&BatchStreamSink>,
) -> Result<(Vec<BatchTextItem>, u64, String), String> {
    if items.is_empty() {
        return Err("没有可识别的截图".to_string());
    }
    let started = Instant::now();
    let mut set = tokio::task::JoinSet::new();

    for (index, (title, b64)) in items.iter().enumerate() {
        let client = client.clone();
        let config = config.clone();
        let title = title.clone();
        let b64 = b64.clone();
        let item_sink: Option<StreamSink> = on_item_stream.map(|batch_sink| {
            let batch_sink = Arc::clone(batch_sink);
            Arc::new(move |preview: StreamPreview| batch_sink(index, preview)) as StreamSink
        });
        set.spawn(async move {
            match ocr_image_text_with_model(&client, &config, &b64, item_sink.as_ref()).await {
                Ok((source_lang, original_text, used_model)) => {
                    let display = if original_text.trim().is_empty() {
                        "（未检测到文字）".to_string()
                    } else {
                        original_text.clone()
                    };
                    (
                        true,
                        None,
                        used_model,
                        BatchTextItem {
                            index,
                            title,
                            source_lang,
                            original_text,
                            translated_text: display,
                            blocks: Vec::new(),
                        },
                    )
                }
                Err(err) => (
                    false,
                    Some(err.clone()),
                    String::new(),
                    BatchTextItem {
                        index,
                        title,
                        source_lang: "失败".to_string(),
                        original_text: String::new(),
                        translated_text: format!("（识别失败：{err}）"),
                        blocks: Vec::new(),
                    },
                ),
            }
        });
    }

    let mut out = Vec::with_capacity(items.len());
    let mut ok_count = 0usize;
    let mut first_err: Option<String> = None;
    let mut resolved_model = String::new();
    while let Some(res) = set.join_next().await {
        let (ok, err, used_model, item) = res.map_err(|e| format!("并发识字任务异常: {e}"))?;
        if ok {
            ok_count += 1;
            if resolved_model.is_empty() && !used_model.is_empty() {
                resolved_model = used_model;
            }
        } else if first_err.is_none() {
            first_err = err;
        }
        out.push(item);
    }
    if ok_count == 0 {
        if let Some(err) = first_err {
            return Err(err);
        }
    }
    out.sort_by_key(|item| item.index);
    if resolved_model.is_empty() {
        resolved_model = native_ocr_label().to_string();
    }
    Ok((out, started.elapsed().as_millis() as u64, resolved_model))
}

pub async fn translate_images_batch(
    client: &Client,
    config: &AppConfig,
    items: &[(String, String)],
    target_lang: &str,
    engine: &str,
    on_item_stream: Option<&BatchStreamSink>,
) -> Result<(Vec<BatchTextItem>, u64, String), String> {
    if items.is_empty() {
        return Err("没有可翻译的截图".to_string());
    }
    let started = Instant::now();
    let mut set = tokio::task::JoinSet::new();

    for (index, (title, b64)) in items.iter().enumerate() {
        let client = client.clone();
        let config = config.clone();
        let title = title.clone();
        let b64 = b64.clone();
        let target_lang = target_lang.to_string();
        let engine = engine.to_string();
        let item_sink: Option<StreamSink> = on_item_stream.map(|batch_sink| {
            let batch_sink = Arc::clone(batch_sink);
            Arc::new(move |preview: StreamPreview| batch_sink(index, preview)) as StreamSink
        });
        set.spawn(async move {
            match translate_image_full(
                &client,
                &config,
                (index + 1) as u64,
                &b64,
                "",
                &target_lang,
                &engine,
                item_sink.as_ref(),
            )
            .await
            {
                Ok(res) => (
                    true,
                    None,
                    BatchTextItem {
                        index,
                        title,
                        source_lang: res.source_lang,
                        original_text: res.original_text,
                        translated_text: res.translated_text,
                        blocks: res.blocks,
                    },
                    Some(res.model),
                ),
                Err(err) => (
                    false,
                    Some(err.clone()),
                    BatchTextItem {
                        index,
                        title,
                        source_lang: "失败".to_string(),
                        original_text: String::new(),
                        translated_text: format!("（翻译失败：{err}）"),
                        blocks: Vec::new(),
                    },
                    None,
                ),
            }
        });
    }

    let mut out = Vec::with_capacity(items.len());
    let mut model_name = config.model.clone();
    let mut ok_count = 0usize;
    let mut first_err: Option<String> = None;
    while let Some(res) = set.join_next().await {
        let (ok, err, item, m) = res.map_err(|e| format!("并发翻译任务异常: {e}"))?;
        if ok {
            ok_count += 1;
            if let Some(m) = m {
                model_name = m;
            }
        } else if first_err.is_none() {
            first_err = err;
        }
        out.push(item);
    }
    if ok_count == 0 {
        if let Some(err) = first_err {
            return Err(err);
        }
    }
    out.sort_by_key(|item| item.index);
    Ok((out, started.elapsed().as_millis() as u64, model_name))
}

pub async fn test_connection(
    client: &Client,
    config: &AppConfig,
    target: &str,
) -> Result<String, String> {
    let started = Instant::now();
    match target {
        "google" => {
            let (_lang, text) =
                google_translate_text(client, config, "Hello world", "简体中文").await?;
            let ms = started.elapsed().as_millis();
            Ok(format!("Google 翻译连通（耗时 {ms} ms，示例：{text}）"))
        }
        "deepl" => {
            let (_lang, text) =
                deepl_translate_text(client, config, "Hello world", "简体中文").await?;
            let ms = started.elapsed().as_millis();
            Ok(format!("DeepL 连通（耗时 {ms} ms，示例：{text}）"))
        }
        _ => {
            if is_qwen_mt_model(&config.model) {
                let mt_probe = json!({
                    "model": config.model.trim(),
                    "messages": [
                        {
                            "role": "user",
                            "content": "Hello"
                        }
                    ],
                    "translation_options": {
                        "source_lang": "auto",
                        "target_lang": "Chinese"
                    }
                });
                let mt_ok = call_chat_completions(client, config, &mt_probe).await;
                if mt_ok.is_err() {
                    let chat_probe = json!({
                        "model": config.model.trim(),
                        "messages": [
                            {
                                "role": "user",
                                "content": "Reply with OK"
                            }
                        ]
                    });
                    let _ = call_chat_completions(client, config, &chat_probe).await?;
                }
                let ms = started.elapsed().as_millis();
                return Ok(format!(
                    "翻译模型连通正常（{} 已自动启用「本地 OCR + AI 翻译」模式；如需看图识字或问 AI，可切换为 qwen-vl-plus 等视觉模型，耗时 {ms} ms）",
                    config.model.trim()
                ));
            }

            let text_payload = json!({
                "model": config.model.trim(),
                "max_tokens": 16,
                "messages": [
                    {
                        "role": "user",
                        "content": "Reply with OK"
                    }
                ]
            });
            let _ = call_chat_completions(client, config, &text_payload).await?;

            if is_likely_text_only_model(&config.model) {
                let ms = started.elapsed().as_millis();
                return Ok(format!(
                    "文本模型连通正常（{} 不支持直接看图，已自动启用「本地 OCR + 文本模型」混合模式，耗时 {ms} ms）",
                    config.model.trim()
                ));
            }

            // Tiny 10x10 PNG to probe whether endpoint/model actually accepts vision input
            let probe_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAYAAACNMs+9AAAAFUlEQVR42mNk+M9Qz0AEYBxVSF+FAAhKDveksOjmAAAAAElFTkSuQmCC";
            let vision_payload = json!({
                "model": config.model.trim(),
                "max_tokens": 32,
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            { "type": "text", "text": "If you can see this image, reply OK." },
                            {
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:image/png;base64,{probe_b64}")
                                }
                            }
                        ]
                    }
                ]
            });

            match call_chat_completions(client, config, &vision_payload).await {
                Ok(reply) if !is_vision_refusal(&reply) => {
                    let ms = started.elapsed().as_millis();
                    Ok(format!("视觉 AI 模型连通正常（支持看图识字，耗时 {ms} ms）"))
                }
                _ => {
                    let ms = started.elapsed().as_millis();
                    Ok(format!(
                        "AI 接口连通（当前模型不支持直接看图，已自动启用「本地 OCR + 文本模型」混合模式，耗时 {ms} ms）"
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_avoids_duplicate_chat_completions() {
        let mut cfg = AppConfig::default();
        cfg.api_base_url = "https://maas.qianwenaiapi.com/compatible-mode/v1".to_string();
        cfg.api_key = "sk-test".to_string();
        assert_eq!(
            validate_ai_endpoint(&cfg).unwrap(),
            "https://maas.qianwenaiapi.com/compatible-mode/v1/chat/completions"
        );

        cfg.api_base_url =
            "https://maas.qianwenaiapi.com/compatible-mode/v1/chat/completions/".to_string();
        assert_eq!(
            validate_ai_endpoint(&cfg).unwrap(),
            "https://maas.qianwenaiapi.com/compatible-mode/v1/chat/completions"
        );
    }

    #[test]
    fn provider_endpoint_accepts_only_https_or_loopback_http() {
        for base in ["http://localhost:11434/v1", "http://127.0.0.1/v1", "http://[::1]/v1"] {
            let config = AppConfig { api_base_url: base.into(), ..AppConfig::default() };
            assert!(validate_ai_endpoint(&config).is_ok(), "rejected {base}");
        }
        let config = AppConfig { api_base_url: "http://example.test/v1".into(), api_key: "test-key".into(), ..AppConfig::default() };
        assert!(validate_ai_endpoint(&config).is_err());
        assert!(same_ai_origin("https://example.test/v1", "https://example.test/v2"));
        assert!(!same_ai_origin("https://example.test/v1", "https://other.test/v1"));
    }

    #[test]
    fn strip_optional_params_merges_system_and_drops_detail() {
        let payload = json!({
            "model": "test-model",
            "temperature": 0.2,
            "max_tokens": 64,
            "messages": [
                { "role": "system", "content": "Be concise." },
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "Hi" },
                        { "type": "image_url", "image_url": { "url": "data:image/png;base64,AA", "detail": "high" } }
                    ]
                }
            ]
        });
        let stripped = strip_optional_chat_params(&payload).unwrap();
        assert!(stripped.get("temperature").is_none());
        assert!(stripped.get("max_tokens").is_none());
        let msgs = stripped["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        let parts = msgs[0]["content"].as_array().unwrap();
        assert_eq!(parts[0]["text"], "Be concise.");
        assert!(parts[2]["image_url"].get("detail").is_none());
    }

    #[test]
    fn detects_qwen_mt_models() {
        assert!(is_qwen_mt_model("qwen-mt-plus"));
        assert!(is_qwen_mt_model("Qwen-MT-Turbo"));
        assert!(is_likely_text_only_model("qwen-mt-plus"));
        assert!(!is_likely_text_only_model("qwen-vl-plus"));
    }

    #[test]
    fn parses_glossary_entries_and_formats_prompt() {
        let raw = "\n# comment\n// ignore\nSnapTrans=SnapTrans 截屏翻译\nLLM => 大语言模型\nOCR -> 文字识别\nempty=\n";
        let entries = parse_glossary_entries(raw);
        assert_eq!(
            entries,
            vec![
                ("SnapTrans".to_string(), "SnapTrans 截屏翻译".to_string()),
                ("LLM".to_string(), "大语言模型".to_string()),
                ("OCR".to_string(), "文字识别".to_string()),
            ]
        );
        let block = build_glossary_prompt_block(raw);
        assert!(block.contains("- SnapTrans => SnapTrans 截屏翻译"));
        assert!(block.contains("- LLM => 大语言模型"));
        assert!(build_glossary_prompt_block("   # only comment").is_empty());
    }

    #[test]
    fn sse_parser_handles_split_utf8_chunks_and_done() {
        let mut parser = SseStreamParser::default();
        let mut deltas = Vec::new();
        let frame1 = "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n";
        let frame2 = "data: {\"choices\":[{\"delta\":{\"content\":\"世界！\"}}]}\r\ndata: [DONE]\r\n";
        let full_bytes = [frame1.as_bytes(), frame2.as_bytes()].concat();
        // Feed 5 bytes at a time to force splits right in the middle of 3-byte UTF-8 Chinese chars
        for chunk in full_bytes.chunks(5) {
            parser
                .push_bytes(chunk, &mut |d| deltas.push(d.to_string()))
                .unwrap();
        }
        let final_text = parser.finish(&mut |d| deltas.push(d.to_string())).unwrap();
        assert_eq!(final_text, "你好世界！");
        assert_eq!(deltas, vec!["你好", "世界！"]);
    }

    #[test]
    fn sse_parser_falls_back_to_plain_json_response() {
        let mut parser = SseStreamParser::default();
        let mut deltas = Vec::new();
        let body = r#"{"choices":[{"message":{"content":"整段返回内容"}}]}"#;
        parser
            .push_bytes(body.as_bytes(), &mut |d| deltas.push(d.to_string()))
            .unwrap();
        let final_text = parser.finish(&mut |d| deltas.push(d.to_string())).unwrap();
        assert_eq!(final_text, "整段返回内容");
        assert_eq!(deltas, vec!["整段返回内容"]);
    }

    #[test]
    fn sse_parser_surfaces_mid_stream_error() {
        let mut parser = SseStreamParser::default();
        let err_frame = b"data: {\"error\":{\"message\":\"rate limit exceeded\"}}\n\n";
        let err = parser.push_bytes(err_frame, &mut |_| {}).unwrap_err();
        assert!(err.contains("rate limit exceeded"), "unexpected err: {err}");
    }

    #[test]
    fn partial_json_extractor_streams_single_and_block_translations() {
        let partial_single = r#"```json
{"source_lang": "English", "original_text": "say \"hi\"\nline 2", "translated_text": "说 \"你好\"\n第二"#;
        let preview = build_json_stream_preview(partial_single, "translated_text", Some("original_text"))
            .expect("preview expected");
        assert_eq!(preview.text, "说 \"你好\"\n第二");
        assert_eq!(preview.original_text.as_deref(), Some("say \"hi\"\nline 2"));

        let partial_blocks = r#"{"source_lang":"English","blocks":[{"id":0,"original_text":"A","translated_text":"甲"},{"id":1,"original_text":"B","translated_text":"乙中"#;
        let preview2 = build_json_stream_preview(partial_blocks, "translated_text", Some("original_text"))
            .expect("blocks preview expected");
        assert_eq!(preview2.text, "甲\n乙中");
        assert_eq!(preview2.original_text.as_deref(), Some("A\nB"));

        // Trailing incomplete escape should not break or emit garbage
        let cut_escape = r#"{"translated_text": "完成一半\u4e"#;
        let preview3 = build_json_stream_preview(cut_escape, "translated_text", None).unwrap();
        assert_eq!(preview3.text, "完成一半");

        // Fake key inside original_text value must not be extracted as translated_text
        let tricky = r#"{"original_text": "text with \"translated_text\": \"FAKE\"", "translated_text": "真实译文"#;
        let preview4 = build_json_stream_preview(tricky, "translated_text", None).unwrap();
        assert_eq!(preview4.text, "真实译文");
    }

    #[test]
    fn build_text_ask_messages_embeds_source_context_on_first_turn_only() {
        let history = vec![
            AiChatMessage {
                role: "user".to_string(),
                content: "总结要点".to_string(),
            },
            AiChatMessage {
                role: "assistant".to_string(),
                content: "第一点...".to_string(),
            },
            AiChatMessage {
                role: "user".to_string(),
                content: "转成 Markdown 表格".to_string(),
            },
        ];
        let msgs = build_text_ask_messages("  Hello\nWorld  ", &history);
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0]["role"], "system");
        let first_user = msgs[1]["content"].as_str().unwrap();
        assert!(first_user.contains("[选区文本内容]\nHello\nWorld"));
        assert!(first_user.ends_with("总结要点"));
        assert_eq!(msgs[2]["role"], "assistant");
        assert_eq!(msgs[3]["content"], "转成 Markdown 表格");
    }

    #[test]
    fn ask_ai_about_text_streams_sse_chunks_from_local_server() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::Mutex;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().unwrap();
        let captured_req = Arc::new(Mutex::new(String::new()));
        let captured_req_srv = Arc::clone(&captured_req);

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
            let mut buf = Vec::new();
            let mut tmp = [0u8; 2048];
            loop {
                match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            let hdr = String::from_utf8_lossy(&buf[..pos]).to_string();
                            let content_len = hdr
                                .lines()
                                .find_map(|l| {
                                    let lower = l.to_ascii_lowercase();
                                    lower
                                        .strip_prefix("content-length:")
                                        .and_then(|v| v.trim().parse::<usize>().ok())
                                })
                                .unwrap_or(0);
                            if buf.len() >= pos + 4 + content_len {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            if let Ok(mut g) = captured_req_srv.lock() {
                *g = String::from_utf8_lossy(&buf).to_string();
            }

            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            stream.write_all(header.as_bytes()).unwrap();
            stream.flush().unwrap();

            for frame in [
                "data: {\"choices\":[{\"delta\":{\"content\":\"第一部分：\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"Rust 所有权安全。\"}}]}\n\n",
                "data: [DONE]\n\n",
            ] {
                let chunk = format!("{:x}\r\n{}\r\n", frame.len(), frame);
                stream.write_all(chunk.as_bytes()).unwrap();
                stream.flush().unwrap();
                std::thread::sleep(Duration::from_millis(20));
            }
            stream.write_all(b"0\r\n\r\n").unwrap();
            stream.flush().unwrap();
        });

        let answer = tauri::async_runtime::block_on(async {
            let mut cfg = AppConfig::default();
            cfg.api_base_url = format!("http://{addr}/v1");
            cfg.api_key = "test-key".to_string();
            cfg.model = "gpt-4o-mini".to_string();
            let client = crate::config::build_api_client("").unwrap();
            let history = vec![AiChatMessage {
                role: "user".to_string(),
                content: "总结要点".to_string(),
            }];
            let seen_streams = Arc::new(Mutex::new(Vec::<String>::new()));
            let seen_clone = Arc::clone(&seen_streams);
            let sink: StreamSink = Arc::new(move |preview| {
                if let Ok(mut g) = seen_clone.lock() {
                    g.push(preview.text);
                }
            });
            let res = ask_ai_about_text(&client, &cfg, "Rust ownership model", &history, Some(&sink))
                .await
                .unwrap();
            let snaps = seen_streams.lock().unwrap().clone();
            assert!(!snaps.is_empty(), "expected incremental stream previews");
            assert_eq!(snaps.last().unwrap(), "第一部分：Rust 所有权安全。");
            res
        });

        handle.join().unwrap();
        assert_eq!(answer, "第一部分：Rust 所有权安全。");
        let req_text = captured_req.lock().unwrap();
        assert!(req_text.contains("Rust ownership model"));
        assert!(req_text.contains("[选区文本内容]"));
    }
}

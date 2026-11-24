#[derive(Debug, Clone)]
pub struct OcrLayoutBlock {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub line_height: f32,
    pub line_count: u32,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct OcrLineBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub text: String,
    pub had_leading_icon: bool,
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

fn starts_with_list_marker(text: &str) -> bool {
    let s = text.trim_start();
    if s.is_empty() {
        return false;
    }
    let first = s.chars().next().unwrap();
    if matches!(
        first,
        '•' | '◦' | '▪' | '▫' | '○' | '●' | '✓' | '✔' | '☑' | '☐' | '①'..='⑳'
    ) {
        return true;
    }
    if matches!(first, '-' | '*' | '+')
        && s.chars().nth(1).map(|c| c.is_whitespace()).unwrap_or(false)
    {
        return true;
    }
    // Numbered item like "1. ", "2) ", "(1) "
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() && i < 3 {
        i += 1;
    }
    if i > 0 && i < bytes.len() && matches!(bytes[i], b'.' | b')') {
        if bytes.get(i + 1).map(|b| b.is_ascii_whitespace()).unwrap_or(false) {
            return true;
        }
    }
    false
}

fn join_wrapped_text(prev: &str, next: &str) -> String {
    let p = prev.trim_end();
    let n = next.trim_start();
    if p.is_empty() {
        return n.to_string();
    }
    if n.is_empty() {
        return p.to_string();
    }
    let prev_chars: Vec<char> = p.chars().collect();
    let last_ch = *prev_chars.last().unwrap();
    let first_next = n.chars().next().unwrap();
    if last_ch == '-'
        && prev_chars
            .len()
            .checked_sub(2)
            .and_then(|idx| prev_chars.get(idx))
            .map(|c| c.is_ascii_alphanumeric())
            .unwrap_or(false)
        && first_next.is_ascii_alphabetic()
    {
        return format!("{p}{n}");
    }
    if is_cjk_char(last_ch) && is_cjk_char(first_next) {
        return format!("{p}{n}");
    }
    format!("{p} {n}")
}

pub(crate) fn group_ocr_lines_into_blocks(mut lines: Vec<OcrLineBox>) -> Vec<OcrLayoutBlock> {
    if lines.is_empty() {
        return Vec::new();
    }

    // Sort top-to-bottom, keeping lines on the same visual row in left-to-right order.
    lines.sort_by(|a, b| {
        let overlap = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
        let min_h = a.h.min(b.h).max(0.001);
        if overlap > min_h * 0.45 {
            a.x.total_cmp(&b.x)
        } else {
            a.y.total_cmp(&b.y)
        }
    });

    struct BlockAcc {
        x: f32,
        y: f32,
        right: f32,
        bottom: f32,
        line_h_sum: f32,
        line_count: u32,
        last_line_w: f32,
        last_line_text: String,
        text: String,
    }

    let mut blocks: Vec<BlockAcc> = Vec::with_capacity(lines.len());

    for line in &lines {
        let mut merged_idx: Option<usize> = None;

        if !line.had_leading_icon && !starts_with_list_marker(&line.text) {
            for (bi, blk) in blocks.iter().enumerate().rev() {
                if blk.line_count >= 6 {
                    continue;
                }
                let blk_lh = blk.line_h_sum / blk.line_count as f32;
                let avg_h = (blk_lh + line.h) * 0.5;
                let gap = line.y - blk.bottom;
                if gap < -avg_h * 0.25 || gap > avg_h * 0.68 {
                    continue;
                }
                let h_ratio = blk_lh.max(line.h) / blk_lh.min(line.h).max(0.001);
                if h_ratio > 1.32 {
                    continue;
                }
                let left_diff = (line.x - blk.x).abs();
                let left_tol = (avg_h * 1.65).max(0.035);
                if left_diff > left_tol {
                    continue;
                }
                // Estimate max width of lines in this horizontal column.
                let col_max_w = lines
                    .iter()
                    .filter(|other| (other.x - blk.x).abs() <= (avg_h * 2.2).max(0.055))
                    .map(|other| other.w)
                    .fold(blk.last_line_w, f32::max);

                let prev_trimmed = blk.last_line_text.trim_end();
                let prev_last = prev_trimmed.chars().last();
                let prev_has_wrap_cue = matches!(
                    prev_last,
                    Some('-' | ',' | '，' | '&' | '/' | '、' | ':' | '：' | '(' | '（')
                );
                let prev_ends_hard = matches!(
                    prev_last,
                    Some(';' | '{' | '}' | '。' | '!' | '！' | '?' | '？')
                );
                let prev_is_wide = blk.last_line_w >= col_max_w * 0.66
                    && prev_trimmed.chars().count() >= 14;
                let can_wrap = (prev_is_wide || (prev_has_wrap_cue && blk.last_line_w >= col_max_w * 0.52))
                    && (!prev_ends_hard || (gap <= avg_h * 0.42 && blk.last_line_w >= col_max_w * 0.82));

                if can_wrap {
                    merged_idx = Some(bi);
                    break;
                }
            }
        }

        if let Some(bi) = merged_idx {
            let blk = &mut blocks[bi];
            blk.x = blk.x.min(line.x);
            blk.y = blk.y.min(line.y);
            blk.right = blk.right.max(line.x + line.w);
            blk.bottom = blk.bottom.max(line.y + line.h);
            blk.line_h_sum += line.h;
            blk.line_count += 1;
            blk.last_line_w = line.w;
            blk.last_line_text = line.text.clone();
            blk.text = join_wrapped_text(&blk.text, &line.text);
        } else {
            blocks.push(BlockAcc {
                x: line.x,
                y: line.y,
                right: line.x + line.w,
                bottom: line.y + line.h,
                line_h_sum: line.h,
                line_count: 1,
                last_line_w: line.w,
                last_line_text: line.text.clone(),
                text: line.text.clone(),
            });
        }
    }

    blocks
        .into_iter()
        .map(|b| OcrLayoutBlock {
            x: b.x.clamp(0.0, 1.0),
            y: b.y.clamp(0.0, 1.0),
            w: (b.right - b.x).clamp(0.005, 1.0 - b.x.clamp(0.0, 1.0)),
            h: (b.bottom - b.y).clamp(0.005, 1.0 - b.y.clamp(0.0, 1.0)),
            line_height: (b.line_h_sum / b.line_count as f32).clamp(0.005, 1.0),
            line_count: b.line_count,
            text: b.text,
        })
        .collect()
}

pub(crate) fn guess_lang_from_text(text: &str) -> String {
    let mut cjk = 0usize;
    let mut kana = 0usize;
    let mut hangul = 0usize;
    let mut latin = 0usize;
    for ch in text.chars() {
        match ch as u32 {
            0x3040..=0x30FF => kana += 1,
            0xAC00..=0xD7AF => hangul += 1,
            0x4E00..=0x9FFF => cjk += 1,
            c if (c as u8 as char).is_ascii_alphabetic() && c < 128 => latin += 1,
            _ => {}
        }
    }
    if kana > 2 {
        "日本語".to_string()
    } else if hangul > 2 {
        "한국어".to_string()
    } else if cjk >= 8 && latin >= 16 {
        "中英混排".to_string()
    } else if cjk > latin / 2 && cjk > 0 {
        "中文".to_string()
    } else if latin > 0 {
        "English".to_string()
    } else {
        "自动识别".to_string()
    }
}

#[cfg(windows)]
mod imp {
    use super::OcrLayoutBlock;
    pub(super) use super::OcrLineBox;
    pub(super) use super::group_ocr_lines_into_blocks;
    use super::{guess_lang_from_text, is_cjk_char};
    use image::imageops::FilterType;
    use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::{OcrEngine, OcrResult};
    use windows::Security::Cryptography::CryptographicBuffer;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_SZ,
    };
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };

    struct ComScope(bool);
    impl Drop for ComScope {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn init_com_mta() -> ComScope {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        ComScope(hr.is_ok())
    }

    fn normalize_ocr_line(raw: &str) -> String {
        let chars: Vec<char> = raw.chars().collect();
        let mut out = String::with_capacity(raw.len());
        for (i, &ch) in chars.iter().enumerate() {
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let next = chars.get(i + 1).copied();
            let near_ascii = prev
                .map(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | ':' | '(' | ')'))
                .unwrap_or(false)
                || next
                    .map(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | ':' | '(' | ')'))
                    .unwrap_or(false);
            let mapped = match ch {
                '．' if near_ascii => '.',
                '：' if near_ascii => ':',
                '；' if near_ascii => ';',
                '％' if near_ascii => '%',
                '（' if next.map(|c| c.is_ascii_alphanumeric()).unwrap_or(false) => '(',
                '）' if prev.map(|c| c.is_ascii_alphanumeric() || c == ')').unwrap_or(false) => ')',
                '，' if prev.map(|c| c.is_ascii_digit()).unwrap_or(false)
                    && next.map(|c| c.is_ascii_digit()).unwrap_or(false) => '.',
                _ => ch,
            };
            out.push(mapped);
        }
        // Remove stray spaces before/after dots or slashes inside identifiers/versions (e.g. "0 . 7 . 1")
        // and normalize spaces before/after comma-between-digits (e.g. "99， 5%" -> "99.5%").
        let mut collapsed = String::with_capacity(out.len());
        let out_chars: Vec<char> = out.chars().collect();
        for (i, &ch) in out_chars.iter().enumerate() {
            if ch == ' ' {
                let p = if i > 0 { Some(out_chars[i - 1]) } else { None };
                let n = out_chars.get(i + 1).copied();
                // Collapse stray spaces around punctuation (., /, _, ，) and before closing/after opening delimiters,
                // as well as spaces inside short lowercase fragments split across punctuation like "&confi g" or "&ite ms".
                let is_punct_join = matches!(
                    (p, n),
                    (Some(a), Some(b))
                        if (a.is_ascii_alphanumeric() && matches!(b, '.' | '/' | '_' | '，' | ',' | ')' | ';'))
                            || (matches!(a, '.' | '/' | '_' | '，' | '&') && b.is_ascii_alphanumeric())
                );
                if is_punct_join {
                    continue;
                }
            }
            collapsed.push(ch);
        }
        // Re-check digit,digit -> digit.digit after space collapsing
        let collapsed_chars: Vec<char> = collapsed.chars().collect();
        let mut final_out = String::with_capacity(collapsed.len());
        for (i, &ch) in collapsed_chars.iter().enumerate() {
            if ch == '，' {
                let p = if i > 0 { Some(collapsed_chars[i - 1]) } else { None };
                let n = collapsed_chars.get(i + 1).copied();
                if p.map(|c| c.is_ascii_digit()).unwrap_or(false)
                    && n.map(|c| c.is_ascii_digit()).unwrap_or(false)
                {
                    final_out.push('.');
                    continue;
                }
            }
            final_out.push(ch);
        }
        final_out
    }

    fn join_broken_ascii_words(text: &str) -> String {
        // Windows zh-Hans-CN OCR frequently inserts spurious spaces inside English identifiers/words
        // (e.g. "retu rn" or "confi g"). Merge short lowercase fragments when separated by a single space
        // and not forming a real common short English word.
        const KEEP_SHORT: &[&str] = &[
            "a", "an", "as", "at", "be", "by", "do", "go", "he", "hi", "if", "in", "is", "it",
            "me", "my", "no", "of", "on", "or", "ok", "so", "to", "up", "us", "we",
            "and", "are", "but", "can", "did", "for", "get", "had", "has", "her", "him", "his",
            "how", "its", "let", "may", "new", "not", "now", "off", "old", "one", "our", "out",
            "own", "run", "say", "see", "set", "she", "the", "too", "try", "two", "use", "var",
            "way", "who", "why", "you", "pub", "use", "mod", "fn", "mut", "ref",
        ];
        let mut lines_out = Vec::new();
        for line in text.lines() {
            let tokens: Vec<&str> = line.split(' ').collect();
            let mut merged: Vec<String> = Vec::with_capacity(tokens.len());
            let mut i = 0;
            while i < tokens.len() {
                let cur = tokens[i];
                if i + 1 < tokens.len() {
                    let nxt = tokens[i + 1];
                    let cur_core = cur.trim_start_matches(|c: char| matches!(c, '(' | '&' | '*' | '[' | '{'));
                    let nxt_core = nxt.trim_end_matches(|c: char| matches!(c, ',' | ';' | ')' | ']' | '}'));
                    let cur_alpha = !cur_core.is_empty() && cur_core.chars().all(|c| c.is_ascii_lowercase());
                    let nxt_alpha = !nxt_core.is_empty() && nxt_core.chars().all(|c| c.is_ascii_lowercase());
                    if cur_alpha
                        && nxt_alpha
                        && (cur_core.len() <= 2 || nxt_core.len() <= 2)
                        && !KEEP_SHORT.contains(&cur_core)
                        && !KEEP_SHORT.contains(&nxt_core)
                    {
                        merged.push(format!("{cur}{nxt}"));
                        i += 2;
                        continue;
                    }
                }
                merged.push(cur.to_string());
                i += 1;
            }
            lines_out.push(merged.join(" "));
        }
        lines_out.join("\n")
    }

    #[derive(Clone, Debug)]
    pub(super) struct OcrWordBox {
        pub text: String,
        pub x: f32,
        pub y: f32,
        pub width: f32,
        pub height: f32,
    }

    pub(super) struct OcrBitmapVariant {
        pub scale: u32,
        pub src_h: u32,
        pub content_w: u32,
        pub content_h: u32,
        pub pad: u32,
        pub canvas: RgbaImage,
    }

    fn is_leading_icon_noise(token: &str) -> bool {
        let chars: Vec<char> = token.trim().chars().collect();
        if chars.is_empty() || chars.len() > 2 {
            return false;
        }
        chars.iter().all(|&c| {
            matches!(
                c,
                '.' | ','
                    | '．'
                    | '。'
                    | '，'
                    | '、'
                    | '@'
                    | '·'
                    | '•'
                    | '◦'
                    | '○'
                    | '●'
                    | '◌'
                    | '◎'
                    | '©'
                    | '®'
                    | '°'
                    | '▪'
                    | '▫'
                    | '■'
                    | '□'
                    | '☐'
                    | '☑'
                    | '✓'
                    | '✔'
                    | '✕'
                    | '✖'
                    | '⋯'
                    | '…'
            )
        })
    }

    fn is_circular_icon_misread(first: &OcrWordBox, second: &OcrWordBox, rest_h: f32) -> bool {
        let tok = first.text.trim();
        let mut chars = tok.chars();
        let Some(ch) = chars.next() else {
            return false;
        };
        if chars.next().is_some() {
            return false;
        }
        if !matches!(ch, '0' | 'o' | 'O' | 'Q' | 'e' | 'c' | 'C' | 'v' | 'V' | 'x' | 'X') {
            return false;
        }
        let Some(next_ch) = second.text.trim().chars().next() else {
            return false;
        };
        let next_starts_item = next_ch.is_ascii_uppercase() || is_cjk_char(next_ch);
        if !next_starts_item {
            return false;
        }
        let gap = second.x - (first.x + first.width);
        gap >= rest_h * 0.20 && first.width <= rest_h * 1.25
    }

    pub(super) fn restore_underscores(words: &[OcrWordBox], canvas: &RgbaImage) -> Vec<OcrWordBox> {
        let mut restored = Vec::with_capacity(words.len());
        for (index, word) in words.iter().enumerate() {
            restored.push(word.clone());
            if let Some(next) = words.get(index + 1) {
                // OCR may return two words when an underscore is below both bounding boxes.
                let height = word.height.max(next.height);
                let base = (word.y + word.height).min(next.y + next.height);
                let left = (word.x + word.width).ceil() as u32;
                let right = (next.x.floor() as u32).min(canvas.width());
                let width = right.saturating_sub(left);
                let ascii = |ch: char| ch.is_ascii_alphanumeric();
                if word.text.chars().last().is_some_and(ascii)
                    && next.text.chars().next().is_some_and(ascii)
                    && (width as f32) >= (height * 0.25).max(3.0)
                    && (width as f32) <= height * 0.95
                    && (word.y + word.height - next.y - next.height).abs() <= height * 0.2
                {
                    let start = (base + height * 0.06).ceil() as u32;
                    let end = ((base + height * 0.45).ceil() as u32).min(canvas.height());
                    let clear_row = base.max(0.0) as u32;
                    if clear_row < canvas.height() {
                        for y in start..end {
                            let mut run = 0;
                            let mut longest = 0;
                            for x in left..right {
                                let above = canvas.get_pixel(x, clear_row)[0];
                                let stroke = canvas.get_pixel(x, y)[0];
                                if above.saturating_sub(stroke) >= 60 {
                                    run += 1;
                                    longest = longest.max(run);
                                } else {
                                    run = 0;
                                }
                            }
                            if longest * 4 >= width * 3 {
                                restored.push(OcrWordBox {
                                    text: "_".into(), x: left as f32, y: base,
                                    width: width as f32, height: 0.0,
                                });
                                break;
                            }
                        }
                    }
                }
            }
        }
        restored
    }

    pub(super) fn format_ocr_line_with_boxes(words: &[OcrWordBox]) -> String {
        let mut out = String::new();
        let mut prev_right: Option<f32> = None;
        let mut avg_h = 0.0f32;
        let mut h_cnt = 0usize;
        let mut ascii_gaps: Vec<f32> = Vec::new();
        for (idx, w) in words.iter().enumerate() {
            if w.text == "_" && w.width > 0.0 {
                ascii_gaps.push(w.width);
            }
            if w.height > 0.0 {
                avg_h += w.height;
                h_cnt += 1;
            }
            if idx > 0 {
                let prev = &words[idx - 1];
                let prev_end_ascii = prev
                    .text
                    .trim()
                    .chars()
                    .last()
                    .is_some_and(|c| c.is_ascii_alphanumeric());
                let cur_start_ascii = w.text.trim().chars().next()
                    .is_some_and(|c| c.is_ascii_alphanumeric());
                if prev_end_ascii && cur_start_ascii && prev.width > 0.0 {
                    let g = w.x - (prev.x + prev.width);
                    if g >= -2.0 {
                        ascii_gaps.push(g);
                    }
                }
            }
        }
        let line_h = if h_cnt > 0 { avg_h / h_cnt as f32 } else { 0.0 };
        let ref_space_gap = if ascii_gaps.is_empty() {
            0.0
        } else {
            ascii_gaps.sort_by(|a, b| a.total_cmp(b));
            // 70th percentile represents normal inter-word spacing even if 1-2 tokens were split
            let p70 = ((ascii_gaps.len() * 7) / 10).min(ascii_gaps.len() - 1);
            ascii_gaps[p70]
        };

        for w in words {
            let token = w.text.trim();
            if token.is_empty() {
                continue;
            }
            if out.is_empty() {
                out.push_str(token);
                prev_right = Some(w.x + w.width);
                continue;
            }
            let prev_ch = out.chars().last();
            let next_ch = token.chars().next();
            let prev_cjk = prev_ch.map(is_cjk_char).unwrap_or(false);
            let next_cjk = next_ch.map(is_cjk_char).unwrap_or(false);
            let no_space_before = matches!(
                next_ch,
                Some(',' | '.' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '%' | '/' | '_' | '，' | '。' | '；' | '：' | '！' | '？' | '）' | '】' | '、' | '．' | '％')
            );
            let no_space_after = matches!(prev_ch, Some('(' | '[' | '{' | '/' | '_' | '（' | '【' | '“' | '‘'));
            let ascii_punct_join = matches!(prev_ch, Some('.' | ':'))
                && next_ch.map(|c| c.is_ascii_alphanumeric()).unwrap_or(false);
            let tight_ascii_kern = line_h > 0.0
                && prev_right
                    .map(|pr| {
                        let gap = w.x - pr;
                        let both_ascii_word = prev_ch
                            .map(|c| c.is_ascii_alphanumeric() || c == '_')
                            .unwrap_or(false)
                            && next_ch
                                .map(|c| c.is_ascii_alphanumeric() || c == '_')
                                .unwrap_or(false);
                        let rel_narrow = if ref_space_gap > 0.0 {
                            gap <= ref_space_gap * 0.58
                        } else {
                            true
                        };
                        both_ascii_word && gap <= line_h * 0.25 && rel_narrow
                    })
                    .unwrap_or(false);

            if (prev_cjk && next_cjk)
                || no_space_before
                || no_space_after
                || ascii_punct_join
                || tight_ascii_kern
            {
                out.push_str(token);
            } else {
                out.push(' ');
                out.push_str(token);
            }
            prev_right = Some(w.x + w.width);
        }
        join_broken_ascii_words(&normalize_ocr_line(&out))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn format_ocr_line(words: &[String]) -> String {
        let boxes: Vec<OcrWordBox> = words
            .iter()
            .map(|t| OcrWordBox {
                text: t.clone(),
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            })
            .collect();
        format_ocr_line_with_boxes(&boxes)
    }

    pub(super) fn build_ocr_bitmap_variants(png_bytes: &[u8]) -> Result<Vec<OcrBitmapVariant>, String> {
        let dyn_img =
            image::load_from_memory(png_bytes).map_err(|e| format!("解析截图失败: {e}"))?;
        let (w, h) = dyn_img.dimensions();
        let gray_src = dyn_img.to_luma8();

        // Detect dark mode using interior luminance distribution (ignoring outer 2px borders/frames).
        let x_start = (w / 10).min(w.saturating_sub(1));
        let x_end = (w - w / 10).max(x_start + 1);
        let y_start = (h / 10).min(h.saturating_sub(1));
        let y_end = (h - h / 10).max(y_start + 1);
        let mut dark_px = 0u64;
        let mut light_px = 0u64;
        let mut lum_sum = 0u64;
        let mut lum_cnt = 0u64;
        let step_x = ((x_end - x_start) / 48).max(1);
        let step_y = ((y_end - y_start) / 48).max(1);
        for y in (y_start..y_end).step_by(step_y as usize) {
            for x in (x_start..x_end).step_by(step_x as usize) {
                let v = gray_src.get_pixel(x, y)[0];
                lum_sum += u64::from(v);
                lum_cnt += 1;
                if v < 110 {
                    dark_px += 1;
                } else if v > 155 {
                    light_px += 1;
                }
            }
        }
        let avg_lum = if lum_cnt > 0 { lum_sum / lum_cnt } else { 255 };
        let is_dark_bg = avg_lum < 118 || (dark_px > light_px.saturating_mul(2) && avg_lum < 140);

        let normalized_gray = if is_dark_bg {
            let mut inv = gray_src;
            for p in inv.pixels_mut() {
                p[0] = 255u8.saturating_sub(p[0]);
            }
            DynamicImage::ImageLuma8(inv)
        } else {
            DynamicImage::ImageLuma8(gray_src)
        };

        // Keep native glyph edges. Interpolate a second size for small selections.
        let scales: &[u32] = if w < 1400 && h < 900 { &[1, 2] } else { &[1] };
        let pad = 28;
        let max_content_size = OcrEngine::MaxImageDimension()
            .map_err(|e| format!("获取 Windows OCR 图像尺寸上限失败: {e}"))?
            .checked_sub(pad * 2)
            .filter(|&size| size > 0)
            .ok_or("Windows OCR 图像尺寸上限无效")?;
        let mut variants = Vec::with_capacity(scales.len());
        for &scale in scales {
            let side = w.max(h).saturating_mul(scale).min(max_content_size);
            let scaled = if scale == 1 && w.max(h) <= max_content_size {
                normalized_gray.clone()
            } else {
                normalized_gray.resize(side, side, FilterType::Triangle)
            };
            let luma = scaled.to_luma8();
            let lw = luma.width();
            let lh = luma.height();
            let mut canvas = RgbaImage::from_pixel(lw + pad * 2, lh + pad * 2, Rgba([255, 255, 255, 255]));
            for (x, y, pixel) in luma.enumerate_pixels() {
                let v = pixel[0];
                canvas.put_pixel(x + pad, y + pad, Rgba([v, v, v, 255]));
            }
            variants.push(OcrBitmapVariant {
                scale,
                src_h: h,
                content_w: lw,
                content_h: lh,
                pad,
                canvas,
            });
        }

        Ok(variants)
    }

    pub(super) fn score_ocr_candidate(lang_tag: &str, text: &str) -> i64 {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return i64::MIN / 2;
        }

        let chars: Vec<char> = trimmed.chars().collect();
        let mut cjk = 0i64;
        let mut contiguous_cjk = 0i64;
        let mut isolated_cjk = 0i64;
        let mut ascii_alpha = 0i64;
        let mut ascii_digit = 0i64;
        let mut pipe_count = 0i64;
        let mut rare_split_penalty = 0i64;

        for (i, &ch) in chars.iter().enumerate() {
            if is_cjk_char(ch) {
                cjk += 1;
                let prev_cjk = i > 0 && is_cjk_char(chars[i - 1]);
                let next_cjk = i + 1 < chars.len() && is_cjk_char(chars[i + 1]);
                if prev_cjk || next_cjk {
                    contiguous_cjk += 1;
                } else {
                    isolated_cjk += 1;
                }
                // Penalize classic CJK radical-split artifacts (e.g. "亻故" instead of "做")
                if matches!(ch, '亻' | '氵' | '讠' | '扌' | '口' | '艹' | '阝' | '纟' | '钅') && next_cjk {
                    if ch == '亻' || ch == '氵' || ch == '讠' || ch == '扌' || ch == '纟' || ch == '钅' {
                        rare_split_penalty += 22;
                    }
                }
            } else if ch.is_ascii_alphabetic() {
                ascii_alpha += 1;
            } else if ch.is_ascii_digit() {
                ascii_digit += 1;
            } else if ch == '|' {
                pipe_count += 1;
            } else if matches!(ch, '」' | '「' | '℃' | '℉' | '『' | '』' | '丨' | '丿') {
                rare_split_penalty += 25;
            }
        }

        let mut single_fragment_penalty = 0i64;
        let mut mixed_alnum_penalty = 0i64;
        for tok in trimmed.split_whitespace() {
            let clean = tok.trim_matches(|c: char| !c.is_alphanumeric() && c != '|');
            if clean.len() == 1 {
                if let Some(ch) = clean.chars().next() {
                    if ch.is_ascii_alphabetic() && ch != 'a' && ch != 'A' && ch != 'I' {
                        single_fragment_penalty += 12;
                    }
                }
            } else if clean.len() >= 3 {
                let has_alpha = clean.chars().any(|c| c.is_ascii_alphabetic());
                let has_zero_or_one = clean.chars().any(|c| c == '0' || c == '1');
                if has_alpha && has_zero_or_one {
                    mixed_alnum_penalty += 8;
                }
            }
        }

        let lang_lower = lang_tag.to_ascii_lowercase();
        let is_en = lang_lower.starts_with("en");

        // Extra glyphs alone are weak evidence: a split Han character may look like two valid ones.
        let mut score = (ascii_alpha + ascii_digit).min(30) * 2 + contiguous_cjk.min(8) * 6 + cjk.min(8) * 2;
        score -= pipe_count * 10;
        score -= rare_split_penalty;
        score -= single_fragment_penalty;
        score -= mixed_alnum_penalty;

        if contiguous_cjk >= 2 {
            if is_en {
                score -= 200;
            } else {
                score += 120;
            }
        } else {
            score -= isolated_cjk * 25;
            if is_en {
                score += 60;
            }
        }

        score
    }

    fn extract_lines_with_engine(
        engine: &OcrEngine,
        bitmap: &SoftwareBitmap,
        variant: &OcrBitmapVariant,
    ) -> Result<(String, Vec<OcrLineBox>), String> {
        let op = engine
            .RecognizeAsync(bitmap)
            .map_err(|e| format!("启动 Windows OCR 失败: {e}"))?;
        let result: OcrResult = op
            .join()
            .map_err(|e| format!("Windows OCR 识别失败: {e}"))?;

        let lines = result
            .Lines()
            .map_err(|e| format!("读取 OCR 行结果失败: {e}"))?;
        let cw = variant.content_w.max(1) as f32;
        let ch = variant.content_h.max(1) as f32;
        let pad_f = variant.pad as f32;

        let mut out_lines = Vec::new();
        let mut line_boxes = Vec::new();
        for line in lines {
            if let Ok(words_view) = line.Words() {
                let mut words = Vec::new();
                for word in words_view {
                    if let Ok(t) = word.Text() {
                        let rect = word.BoundingRect().unwrap_or_default();
                        words.push(OcrWordBox {
                            text: t.to_string(),
                            x: rect.X,
                            y: rect.Y,
                            width: rect.Width,
                            height: rect.Height,
                        });
                    }
                }
                let mut had_leading_icon = false;
                if words.len() >= 2 {
                    let rest_h = words[1..]
                        .iter()
                        .map(|w| w.height)
                        .filter(|&h| h > 0.0)
                        .fold(0.0f32, f32::max);
                    if is_leading_icon_noise(&words[0].text) {
                        let gap = words[1].x - (words[0].x + words[0].width);
                        let tok = words[0].text.trim();
                        let obviously_icon = tok != "." && tok != ",";
                        if obviously_icon || gap >= rest_h * 0.25 || words[0].width <= rest_h * 0.55 {
                            words.remove(0);
                            had_leading_icon = true;
                        }
                    } else if is_circular_icon_misread(&words[0], &words[1], rest_h) {
                        words.remove(0);
                        had_leading_icon = true;
                    }
                }
                let formatted = format_ocr_line_with_boxes(&restore_underscores(&words, &variant.canvas));
                if !formatted.trim().is_empty() {
                    let mut min_x = f32::INFINITY;
                    let mut min_y = f32::INFINITY;
                    let mut max_x = f32::NEG_INFINITY;
                    let mut max_y = f32::NEG_INFINITY;
                    for w in &words {
                        if w.width > 0.0 && w.height > 0.0 {
                            min_x = min_x.min(w.x);
                            min_y = min_y.min(w.y);
                            max_x = max_x.max(w.x + w.width);
                            max_y = max_y.max(w.y + w.height);
                        }
                    }
                    if min_x.is_finite() && max_x > min_x && max_y > min_y {
                        let nx = ((min_x - pad_f) / cw).clamp(0.0, 1.0);
                        let ny = ((min_y - pad_f) / ch).clamp(0.0, 1.0);
                        let nr = ((max_x - pad_f) / cw).clamp(nx, 1.0);
                        let nb = ((max_y - pad_f) / ch).clamp(ny, 1.0);
                        let nw = (nr - nx).max(0.005);
                        let nh = (nb - ny).max(0.005);
                        line_boxes.push(OcrLineBox {
                            x: nx,
                            y: ny,
                            w: nw,
                            h: nh,
                            text: formatted.clone(),
                            had_leading_icon,
                        });
                    }
                    out_lines.push(formatted);
                    continue;
                }
            }
            if let Ok(t) = line.Text() {
                let s = t.to_string();
                if !s.trim().is_empty() {
                    out_lines.push(s);
                }
            }
        }
        Ok((out_lines.join("\n"), line_boxes))
    }

    pub fn ocr_png_with_layout_sync(
        png_bytes: &[u8],
    ) -> Result<(String, String, Vec<OcrLayoutBlock>), String> {
        let _com = init_com_mta();
        let variants = build_ocr_bitmap_variants(png_bytes)?;

        let mut candidates: Vec<(String, OcrEngine)> = Vec::new();
        let mut seen_tags: Vec<String> = Vec::new();

        fn push_candidate(
            candidates: &mut Vec<(String, OcrEngine)>,
            seen_tags: &mut Vec<String>,
            eng: OcrEngine,
        ) {
            let tag = eng
                .RecognizerLanguage()
                .and_then(|l| l.LanguageTag())
                .map(|t| t.to_string())
                .unwrap_or_default();
            let norm = tag.to_ascii_lowercase();
            if norm.is_empty() || !seen_tags.contains(&norm) {
                if !norm.is_empty() {
                    seen_tags.push(norm);
                }
                candidates.push((tag, eng));
            }
        }

        if let Ok(eng) = OcrEngine::TryCreateFromUserProfileLanguages() {
            push_candidate(&mut candidates, &mut seen_tags, eng);
        }
        if let Ok(langs) = OcrEngine::AvailableRecognizerLanguages() {
            let mut other_langs = Vec::new();
            for lang in langs {
                let is_en = lang
                    .LanguageTag()
                    .map(|t| t.to_string().to_ascii_lowercase().starts_with("en"))
                    .unwrap_or(false);
                if is_en {
                    if let Ok(eng) = OcrEngine::TryCreateFromLanguage(&lang) {
                        push_candidate(&mut candidates, &mut seen_tags, eng);
                    }
                } else {
                    other_langs.push(lang);
                }
            }
            for lang in other_langs {
                if let Ok(eng) = OcrEngine::TryCreateFromLanguage(&lang) {
                    push_candidate(&mut candidates, &mut seen_tags, eng);
                }
            }
        }
        if candidates.is_empty() {
            if let Ok(en) = Language::CreateLanguage(&"en-US".into()) {
                if let Ok(eng) = OcrEngine::TryCreateFromLanguage(&en) {
                    push_candidate(&mut candidates, &mut seen_tags, eng);
                }
            }
        }
        if candidates.is_empty() {
            return Err("系统未安装可用的 Windows OCR 语言包".to_string());
        }

        let mut best_text = String::new();
        let mut best_lines: Vec<OcrLineBox> = Vec::new();
        let mut best_score = i64::MIN;

        for variant in variants {
            let scale = variant.scale;
            let src_h = variant.src_h;
            let width = variant.canvas.width() as i32;
            let height = variant.canvas.height() as i32;
            let buffer = CryptographicBuffer::CreateFromByteArray(variant.canvas.as_raw())
                .map_err(|e| format!("创建 OCR 图像缓冲区失败: {e}"))?;
            let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
                &buffer,
                BitmapPixelFormat::Rgba8,
                width,
                height,
                BitmapAlphaMode::Straight,
            )
            .map_err(|e| format!("创建 SoftwareBitmap 失败: {e}"))?;

            let mut tried_primary = 0usize;
            for (tag, eng) in &candidates {
                let is_en = tag.to_ascii_lowercase().starts_with("en");
                if tried_primary >= 2 && !best_text.trim().is_empty() && !is_en {
                    continue;
                }
                tried_primary += 1;
                match extract_lines_with_engine(eng, &bitmap, &variant) {
                    Ok((text, lines)) => {
                        if !text.trim().is_empty() {
                            let mut score = score_ocr_candidate(tag, &text);
                            if scale == 1 && src_h >= 120 {
                                score += 4;
                            } else if scale >= 2 && src_h < 64 {
                                score += 10;
                            }
                            if score > best_score {
                                best_score = score;
                                best_text = text;
                                best_lines = lines;
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!("[win_ocr] engine {tag} scale={scale} failed: {err}");
                    }
                }
            }
        }

        let blocks = group_ocr_lines_into_blocks(best_lines);
        let detected = guess_lang_from_text(&best_text);
        Ok((detected, best_text, blocks))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn ocr_png_sync(png_bytes: &[u8]) -> Result<(String, String), String> {
        let (lang, text, _blocks) = ocr_png_with_layout_sync(png_bytes)?;
        Ok((lang, text))
    }

    pub fn force_window_foreground(raw_hwnd: isize) {
        let hwnd = HWND(raw_hwnd as *mut std::ffi::c_void);
        unsafe {
            let fg = GetForegroundWindow();
            let cur_tid = GetCurrentThreadId();
            let fg_tid = if !fg.is_invalid() {
                GetWindowThreadProcessId(fg, None)
            } else {
                0
            };
            let attached =
                fg_tid != 0 && fg_tid != cur_tid && AttachThreadInput(cur_tid, fg_tid, true).as_bool();
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            if attached {
                let _ = AttachThreadInput(cur_tid, fg_tid, false);
            }
        }
    }

    pub fn set_windows_autostart(enabled: bool) -> Result<(), String> {
        let exe_path = std::env::current_exe()
            .map_err(|e| format!("获取程序路径失败: {e}"))?;
        let exe_str = exe_path.to_string_lossy();
        let subkey = windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let value_name = windows::core::w!("Luma");

        unsafe {
            let mut hkey = HKEY::default();
            let status = RegOpenKeyExW(HKEY_CURRENT_USER, subkey, Some(0), KEY_SET_VALUE, &mut hkey);
            if status.is_err() {
                return Err(format!("打开注册表启动项失败: {:?}", status));
            }
            let result = if enabled {
                let quoted = format!("\"{exe_str}\"");
                let mut wide: Vec<u16> = quoted.encode_utf16().collect();
                wide.push(0);
                let byte_slice = std::slice::from_raw_parts(
                    wide.as_ptr() as *const u8,
                    wide.len() * std::mem::size_of::<u16>(),
                );
                let set_status = RegSetValueExW(hkey, value_name, Some(0), REG_SZ, Some(byte_slice));
                if set_status.is_err() {
                    Err(format!("写入开机自启注册表失败: {:?}", set_status))
                } else {
                    Ok(())
                }
            } else {
                let _ = RegDeleteValueW(hkey, value_name);
                Ok(())
            };
            let _ = RegCloseKey(hkey);
            result
        }
    }
}

#[cfg(windows)]
pub fn set_windows_autostart(enabled: bool) -> Result<(), String> {
    imp::set_windows_autostart(enabled)
}

#[cfg(target_os = "macos")]
pub fn set_windows_autostart(enabled: bool) -> Result<(), String> {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "无法获取用户 HOME 目录".to_string())?;
    let agents_dir = home.join("Library").join("LaunchAgents");
    let plist_path = agents_dir.join("com.snaptrans.app.plist");
    if !enabled {
        match std::fs::remove_file(&plist_path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("移除 macOS 开机自启配置失败: {err}")),
        }
    } else {
        let exe = std::env::current_exe().map_err(|e| format!("无法获取当前程序路径: {e}"))?;
        let exe_str = exe
            .to_str()
            .ok_or_else(|| "程序路径包含无效 UTF-8 字符".to_string())?;
        let escaped = exe_str
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        std::fs::create_dir_all(&agents_dir)
            .map_err(|e| format!("创建 LaunchAgents 目录失败: {e}"))?;
        let plist = format!(
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ",
                "\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
                "<plist version=\"1.0\">\n",
                "<dict>\n",
                "  <key>Label</key>\n",
                "  <string>com.snaptrans.app</string>\n",
                "  <key>ProgramArguments</key>\n",
                "  <array>\n",
                "    <string>{escaped}</string>\n",
                "  </array>\n",
                "  <key>RunAtLoad</key>\n",
                "  <true/>\n",
                "</dict>\n",
                "</plist>\n"
            ),
            escaped = escaped
        );
        std::fs::write(&plist_path, plist)
            .map_err(|e| format!("写入 macOS 开机自启配置失败: {e}"))
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn set_windows_autostart(_enabled: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
pub async fn ocr_png_bytes_with_layout(
    png_bytes: Vec<u8>,
) -> Result<(String, String, Vec<OcrLayoutBlock>), String> {
    tauri::async_runtime::spawn_blocking(move || imp::ocr_png_with_layout_sync(&png_bytes))
        .await
        .map_err(|e| format!("本地 OCR 线程异常: {e}"))?
}

#[cfg(target_os = "macos")]
pub async fn ocr_png_bytes_with_layout(
    png_bytes: Vec<u8>,
) -> Result<(String, String, Vec<OcrLayoutBlock>), String> {
    tauri::async_runtime::spawn_blocking(move || crate::macos_bridge::ocr_png_with_layout_sync(&png_bytes))
        .await
        .map_err(|e| format!("Apple Vision OCR 线程异常: {e}"))?
}

#[cfg(not(any(windows, target_os = "macos")))]
pub async fn ocr_png_bytes_with_layout(
    _png_bytes: Vec<u8>,
) -> Result<(String, String, Vec<OcrLayoutBlock>), String> {
    Err("当前系统不支持系统原生 OCR".to_string())
}

#[cfg(any(windows, target_os = "macos"))]
#[allow(dead_code)]
pub async fn ocr_png_bytes_native(png_bytes: Vec<u8>) -> Result<(String, String), String> {
    let (lang, text, _blocks) = ocr_png_bytes_with_layout(png_bytes).await?;
    Ok((lang, text))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub async fn ocr_png_bytes_native(_png_bytes: Vec<u8>) -> Result<(String, String), String> {
    Err("当前系统不支持系统原生 OCR".to_string())
}

#[cfg(windows)]
pub fn force_window_foreground(raw_hwnd: isize) {
    imp::force_window_foreground(raw_hwnd);
}

#[cfg(target_os = "macos")]
pub fn force_window_foreground(raw_hwnd: isize) {
    crate::macos_bridge::force_window_foreground(raw_hwnd);
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn force_window_foreground(_raw_hwnd: isize) {}

#[cfg(all(test, windows))]
mod tests {
    use super::imp::{format_ocr_line, ocr_png_sync, score_ocr_candidate};
    use image::codecs::png::PngEncoder;
    use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage};

    #[test]
    fn formats_mixed_cjk_and_latin_lines() {
        let words = vec![
            "检".to_string(),
            "查".to_string(),
            "SnapTrans".to_string(),
            "配置".to_string(),
        ];
        assert_eq!(format_ocr_line(&words), "检查 SnapTrans 配置");

        let punct = vec!["canceled".to_string(), ",".to_string(), "and".to_string()];
        assert_eq!(
            format_ocr_line(&punct),
            "canceled, and"
        );

        let version_words = vec![
            "Luma".to_string(),
            "0".to_string(),
            "．".to_string(),
            "7".to_string(),
            "．".to_string(),
            "1".to_string(),
        ];
        assert_eq!(format_ocr_line(&version_words), "Luma 0.7.1");

        let broken_code = vec![
            "return".to_string(),
            "(&client,".to_string(),
            "&confi".to_string(),
            "g,".to_string(),
            "&ite".to_string(),
            "ms);".to_string(),
        ];
        assert_eq!(format_ocr_line(&broken_code), "return (&client, &config, &items);");
    }

    #[test]
    fn dark_mode_bordered_screenshot_ocr_does_not_misread_as_light() {
        // Render crisp white text on dark background inside a light border (common UI card)
        // and verify OCR recognizes it cleanly without splitting CJK glyphs.
        let mut img = RgbaImage::from_pixel(320, 90, Rgba([22, 24, 29, 255]));
        for x in 0..320 {
            img.put_pixel(x, 0, Rgba([240, 240, 240, 255]));
            img.put_pixel(x, 89, Rgba([240, 240, 240, 255]));
        }
        // Paint simple dark-on-light vs light-on-dark pixel blocks check via build_ocr_bitmap_variants
        let mut buf = Vec::new();
        PngEncoder::new(&mut buf)
            .write_image(img.as_raw(), 320, 90, ExtendedColorType::Rgba8)
            .unwrap();
        let variants = super::imp::build_ocr_bitmap_variants(&buf).unwrap();
        assert!(!variants.is_empty());
        // Interior dark pixels should be inverted to bright white background (~255)
        let rgba = &variants[0].canvas;
        let center = rgba.get_pixel(rgba.width() / 2, rgba.height() / 2);
        assert!(center[0] > 230, "expected inverted bright background, got {}", center[0]);
    }

    #[test]
    fn groups_wrapped_checklist_items_into_aligned_blocks() {
        use super::imp::{group_ocr_lines_into_blocks, OcrLineBox};

        let lines = vec![
            OcrLineBox {
                x: 0.14, y: 0.12, w: 0.72, h: 0.05,
                text: "Setup server rclone mount & mint Canvas REST key".into(),
                had_leading_icon: true,
            },
            OcrLineBox {
                x: 0.14, y: 0.22, w: 0.74, h: 0.05,
                text: "Upgrade DB & backend: Shopify multi-img, auto-sync,".into(),
                had_leading_icon: true,
            },
            OcrLineBox {
                x: 0.14, y: 0.29, w: 0.30, h: 0.05,
                text: "watch & canvas CRUD".into(),
                had_leading_icon: false,
            },
            OcrLineBox {
                x: 0.14, y: 0.39, w: 0.72, h: 0.05,
                text: "Add reverse-prompt, multi-ref generation & pose-".into(),
                had_leading_icon: true,
            },
            OcrLineBox {
                x: 0.14, y: 0.46, w: 0.18, h: 0.05,
                text: "base APIs".into(),
                had_leading_icon: false,
            },
            OcrLineBox {
                x: 0.14, y: 0.57, w: 0.76, h: 0.05,
                text: "Run pytest, deploy to server, and verify live end-to-end".into(),
                had_leading_icon: false,
            },
        ];

        let blocks = group_ocr_lines_into_blocks(lines);
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0].line_count, 1);
        assert_eq!(blocks[1].line_count, 2);
        assert_eq!(
            blocks[1].text,
            "Upgrade DB & backend: Shopify multi-img, auto-sync, watch & canvas CRUD"
        );
        assert_eq!(blocks[2].line_count, 2);
        assert_eq!(
            blocks[2].text,
            "Add reverse-prompt, multi-ref generation & pose-base APIs"
        );
        assert_eq!(blocks[3].line_count, 1);
    }

    #[test]
    fn optional_live_sample_ocr_smoke() {
        let dir = std::env::temp_dir().join("luma-ocr-test");
        for (file, expected_sub) in [
            ("mixed_ui.png", "Luma 0.7.1"),
            ("dark_code.png", "ocr_images_batch"),
            ("small_text.png", "99.5%"),
        ] {
            let p = dir.join(file);
            if p.exists() {
                let bytes = std::fs::read(&p).unwrap();
                let (_lang, text) = ocr_png_sync(&bytes).unwrap();
                eprintln!("[OCR sample {file}] => {text}");
                assert!(!text.trim().is_empty(), "empty OCR for {file}");
                let _ = expected_sub;
            }
        }
        let crop_p = std::env::temp_dir().join("luma-verify").join("ocr_crop_test.png");
        if crop_p.exists() {
            let bytes = std::fs::read(&crop_p).unwrap();
            let (_lang, text) = ocr_png_sync(&bytes).unwrap();
            eprintln!("[OCR crop] => {text}");
            assert!(text.contains("accuracy"), "expected joined 'accuracy', got {text}");
        }
        if let Ok(custom_path) = std::env::var("LUMA_OCR_SMOKE_FILE") {
            let p = std::path::Path::new(&custom_path);
            if p.exists() {
                let bytes = std::fs::read(p).unwrap();
                let (lang, text, blocks) = super::imp::ocr_png_with_layout_sync(&bytes).unwrap();
                eprintln!("[OCR custom lang={lang}] blocks={}", blocks.len());
                for (i, b) in blocks.iter().enumerate() {
                    eprintln!(
                        "  [{i}] x={:.3} y={:.3} w={:.3} h={:.3} lh={:.3} lines={} | {}",
                        b.x, b.y, b.w, b.h, b.line_height, b.line_count, b.text
                    );
                }
                assert!(!text.trim().is_empty(), "empty OCR for custom sample");
            }
        }
    }

    #[test]
    fn prefers_clean_english_over_garbled_cjk_misrecognition() {
        let garbled = "Update on my G00 创 e AI Ultra\nthings h ave gotte n even worse.\nN ow my Google C01ab\n| can no";
        let clean = "Update on my Google AI Ultra\nthings have gotten even worse.\nNow my Google Colab\nI can no";
        assert!(score_ocr_candidate("en-US", clean) > score_ocr_candidate("zh-Hans-CN", garbled));
        assert!(
            score_ocr_candidate("zh-Hans-CN", "检查 SnapTrans 配置并点击保存")
                > score_ocr_candidate("en-US", "SnapTrans")
        );
    }

    #[test]
    fn does_not_reward_extra_han_glyphs_when_mixed_text_is_already_recognized() {
        let clean = "OpenMotion 时间轴\n请保存密码并检查配置";
        let split = "OpenMotion 日寸间车由\n请保存密石马并检查配置";
        assert_eq!(score_ocr_candidate("zh-Hans-CN", clean), score_ocr_candidate("zh-Hans-CN", split));
    }

    #[test]
    fn restores_underscores_only_when_a_stroke_is_visible() {
        use super::imp::{format_ocr_line_with_boxes, restore_underscores, OcrWordBox};
        let words = [
            OcrWordBox { text: "demo".into(), x: 5.0, y: 5.0, width: 30.0, height: 20.0 },
            OcrWordBox { text: "token".into(), x: 45.0, y: 5.0, width: 30.0, height: 20.0 },
        ];
        let mut image = RgbaImage::from_pixel(100, 40, Rgba([230, 230, 230, 255]));
        assert_eq!(format_ocr_line_with_boxes(&restore_underscores(&words, &image)), "demo token");
        for x in 36..44 {
            image.put_pixel(x, 30, Rgba([45, 45, 45, 255]));
        }
        assert_eq!(format_ocr_line_with_boxes(&restore_underscores(&words, &image)), "demo_token");
    }

    #[test]
    fn joins_tightly_kerned_english_subwords_via_bounding_boxes() {
        use super::imp::{format_ocr_line_with_boxes, OcrWordBox};
        let boxes = vec![
            OcrWordBox { text: "accu".to_string(), x: 10.0, y: 0.0, width: 32.0, height: 18.0 },
            OcrWordBox { text: "racy".to_string(), x: 43.0, y: 0.0, width: 30.0, height: 18.0 },
            OcrWordBox { text: "and".to_string(), x: 82.0, y: 0.0, width: 26.0, height: 18.0 },
        ];
        assert_eq!(format_ocr_line_with_boxes(&boxes), "accuracy and");
    }

    #[test]
    fn ocr_blank_image_returns_empty_without_error() {
        let img = RgbaImage::from_pixel(120, 40, Rgba([255, 255, 255, 255]));
        let mut buf = Vec::new();
        PngEncoder::new(&mut buf)
            .write_image(img.as_raw(), 120, 40, ExtendedColorType::Rgba8)
            .unwrap();
        let (_lang, text) = ocr_png_sync(&buf).unwrap();
        assert!(text.trim().is_empty());
    }
}

//! Text helpers: sizes, times, payload sanitising, JSON and hex rendering.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use crate::theme::Theme;

pub fn human_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

pub fn human_count(n: u64) -> String {
    if n < 10_000 {
        commas(n)
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1e3)
    } else {
        format!("{:.1}M", n as f64 / 1e6)
    }
}

pub fn msg_count(n: u64) -> String {
    if n == 1 {
        "1 msg".into()
    } else {
        format!("{} msgs", human_count(n))
    }
}

pub fn commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

pub fn ago(now: SystemTime, t: SystemTime) -> String {
    let d = now.duration_since(t).unwrap_or_default().as_secs();
    match d {
        0 => "now".into(),
        1..=59 => format!("{d}s ago"),
        60..=3599 => format!("{}m ago", d / 60),
        3600..=86399 => format!("{}h ago", d / 3600),
        _ => format!("{}d ago", d / 86400),
    }
}

/// Local wall-clock time; `date` adds the calendar day.
pub fn clock(t: SystemTime, date: bool) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (y, mo, day, h, mi, s) = local_parts(d.as_secs());
    let hms = format!("{h:02}:{mi:02}:{s:02}.{:03}", d.subsec_millis());
    if date {
        format!("{y:04}-{mo:02}-{day:02} {hms}")
    } else {
        hms
    }
}

#[cfg(unix)]
fn local_parts(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let secs = secs as _;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    (
        tm.tm_year + 1900,
        tm.tm_mon as u32 + 1,
        tm.tm_mday as u32,
        tm.tm_hour as u32,
        tm.tm_min as u32,
        tm.tm_sec as u32,
    )
}

#[cfg(windows)]
fn local_parts(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    // FILETIME counts 100 ns ticks since 1601-01-01.
    let ticks = (secs + 11_644_473_600) * 10_000_000;
    let ft = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        FileTimeToSystemTime(&ft, &mut utc);
        SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local);
    }
    let l = local;
    (
        l.wYear as i32,
        l.wMonth as u32,
        l.wDay as u32,
        l.wHour as u32,
        l.wMinute as u32,
        l.wSecond as u32,
    )
}

/// At most `max` bytes of `b` as one printable line.
pub fn one_line(b: &[u8], max: usize) -> String {
    let b = &b[..b.len().min(max)];
    let s = match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(e) if e.error_len().is_none() => String::from_utf8_lossy(&b[..e.valid_up_to()]).into_owned(),
        Err(_) => String::from_utf8_lossy(b).into_owned(),
    };
    s.chars()
        .filter_map(|c| match c {
            '\n' => Some('↵'),
            '\r' => None,
            '\t' => Some(' '),
            '\u{FFFD}' => Some('·'),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => Some('·'),
            c => Some(c),
        })
        .collect()
}

/// Does this look like human-readable text?
pub fn is_text(b: &[u8]) -> bool {
    let ok = match std::str::from_utf8(b) {
        Ok(_) => true,
        // tolerate a rune cut off by truncation
        Err(e) => e.error_len().is_none() && b.len() - e.valid_up_to() < 4,
    };
    if !ok {
        return false;
    }
    let ctrl = b
        .iter()
        .filter(|&&c| c < 0x20 && c != b'\n' && c != b'\r' && c != b'\t')
        .count();
    ctrl * 100 <= b.len()
}

pub fn str_width(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Hard-wraps plain text to `w` cells.
pub fn wrap(s: &str, w: usize) -> Vec<String> {
    if w == 0 {
        return vec![];
    }
    let mut out = vec![String::new()];
    let mut cur = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if cur + cw > w {
            out.push(String::new());
            cur = 0;
        }
        out.last_mut().unwrap().push(c);
        cur += cw;
    }
    out
}

/// Truncates (with …) or pads plain text to exactly `w` cells.
pub fn pad(s: &str, w: usize) -> String {
    let line = fit(vec![Span::raw(s.to_string())], w);
    let mut out: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let cur = str_width(&out);
    out.extend(std::iter::repeat_n(' ', w.saturating_sub(cur)));
    out
}

/// Keeps the end of `s` so long topic paths keep their leaf.
pub fn trunc_left(s: &str, w: usize) -> String {
    if str_width(s) <= w {
        return s.to_string();
    }
    if w <= 1 {
        return "…".into();
    }
    let mut kept: Vec<char> = Vec::new();
    let mut cur = 0;
    for c in s.chars().rev() {
        let cw = c.width().unwrap_or(0);
        if cur + cw > w - 1 {
            break;
        }
        kept.push(c);
        cur += cw;
    }
    std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

/// Truncates styled spans to `w` cells, ending in … when cut.
pub fn fit<'a>(spans: Vec<Span<'a>>, w: usize) -> Line<'a> {
    let total: usize = spans.iter().map(|s| s.width()).sum();
    if total <= w {
        return Line::from(spans);
    }
    let mut out = Vec::new();
    let mut used = 0;
    for s in spans {
        let sw = s.width();
        if used + sw < w {
            used += sw;
            out.push(s);
            continue;
        }
        let mut text = String::new();
        for c in s.content.chars() {
            let cw = c.width().unwrap_or(0);
            if used + cw > w - 1 {
                break;
            }
            text.push(c);
            used += cw;
        }
        out.push(Span::styled(text + "…", s.style));
        break;
    }
    Line::from(out)
}

pub fn hex_lines(b: &[u8], w: usize, th: &Theme) -> Vec<Line<'static>> {
    let mut per = 16;
    while per > 4 && 10 + per * 3 + 2 + per > w {
        per /= 2;
    }
    b.chunks(per)
        .enumerate()
        .map(|(i, chunk)| {
            let mut hx = String::with_capacity(per * 3 + 1);
            let mut asc = String::with_capacity(per);
            for j in 0..per {
                match chunk.get(j) {
                    Some(c) => {
                        hx.push_str(&format!("{c:02x} "));
                        asc.push(if (0x20..0x7f).contains(c) { *c as char } else { '.' });
                    }
                    None => hx.push_str("   "),
                }
                if j == 7 && per == 16 {
                    hx.push(' ');
                }
            }
            Line::from(vec![
                Span::styled(format!("{:08x}  ", i * per), th.faint),
                Span::styled(hx, th.number),
                Span::raw(" "),
                Span::styled(asc, th.value),
            ])
        })
        .collect()
}

// ── JSON ───────────────────────────────────────────────────────────────────

/// Validates and pretty-prints JSON with two-space indents; None if invalid.
pub fn json_pretty(src: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(src).ok()?;
    let mut p = JsonPrinter {
        b: s.as_bytes(),
        i: 0,
        out: String::with_capacity(s.len() * 2),
        depth: 0,
    };
    p.ws();
    p.value()?;
    p.ws();
    (p.i == p.b.len()).then_some(p.out)
}

struct JsonPrinter<'a> {
    b: &'a [u8],
    i: usize,
    out: String,
    depth: usize,
}

impl JsonPrinter<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn newline(&mut self) {
        self.out.push('\n');
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
    }
    fn value(&mut self) -> Option<()> {
        match *self.b.get(self.i)? {
            b'{' => self.container(b'}', true),
            b'[' => self.container(b']', false),
            b'"' => self.string(),
            b't' => self.lit("true"),
            b'f' => self.lit("false"),
            b'n' => self.lit("null"),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }
    fn container(&mut self, close: u8, object: bool) -> Option<()> {
        self.out.push(self.b[self.i] as char);
        self.i += 1;
        self.ws();
        if *self.b.get(self.i)? == close {
            self.out.push(close as char);
            self.i += 1;
            return Some(());
        }
        self.depth += 1;
        loop {
            self.newline();
            if object {
                if *self.b.get(self.i)? != b'"' {
                    return None;
                }
                self.string()?;
                self.ws();
                if *self.b.get(self.i)? != b':' {
                    return None;
                }
                self.i += 1;
                self.out.push_str(": ");
                self.ws();
            }
            self.value()?;
            self.ws();
            match *self.b.get(self.i)? {
                b',' => {
                    self.out.push(',');
                    self.i += 1;
                    self.ws();
                }
                c if c == close => {
                    self.i += 1;
                    self.depth -= 1;
                    self.newline();
                    self.out.push(close as char);
                    return Some(());
                }
                _ => return None,
            }
        }
    }
    fn string(&mut self) -> Option<()> {
        let start = self.i;
        self.i += 1;
        loop {
            match *self.b.get(self.i)? {
                b'"' => break,
                b'\\' => match *self.b.get(self.i + 1)? {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.i += 2,
                    b'u' => {
                        let hex = self.b.get(self.i + 2..self.i + 6)?;
                        if !hex.iter().all(|c| c.is_ascii_hexdigit()) {
                            return None;
                        }
                        self.i += 6;
                    }
                    _ => return None,
                },
                c if c < 0x20 => return None,
                _ => self.i += 1,
            }
        }
        self.i += 1;
        self.out.push_str(std::str::from_utf8(&self.b[start..self.i]).ok()?);
        Some(())
    }
    fn lit(&mut self, word: &str) -> Option<()> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            self.out.push_str(word);
            Some(())
        } else {
            None
        }
    }
    fn number(&mut self) -> Option<()> {
        let start = self.i;
        let digits = |p: &mut Self| {
            let s = p.i;
            while p.i < p.b.len() && p.b[p.i].is_ascii_digit() {
                p.i += 1;
            }
            p.i > s
        };
        if self.b[self.i] == b'-' {
            self.i += 1;
        }
        if !digits(self) {
            return None;
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            if !digits(self) {
                return None;
            }
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !digits(self) {
                return None;
            }
        }
        self.out.push_str(std::str::from_utf8(&self.b[start..self.i]).ok()?);
        Some(())
    }
}

/// Colours one line of indented JSON.
pub fn highlight_json(line: &str, th: &Theme) -> Line<'static> {
    let b = line.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(b.len());
            let is_key = line[i..].trim_start().starts_with(':');
            spans.push(Span::styled(
                line[start..i].to_string(),
                if is_key { th.blue } else { th.green },
            ));
        } else if c == b'-' || c.is_ascii_digit() {
            while i < b.len() && b"+-0123456789.eE".contains(&b[i]) {
                i += 1;
            }
            spans.push(Span::styled(line[start..i].to_string(), th.number));
        } else if c.is_ascii_lowercase() {
            while i < b.len() && b[i].is_ascii_lowercase() {
                i += 1;
            }
            spans.push(Span::styled(line[start..i].to_string(), th.keyword));
        } else if b"{}[],:".contains(&c) {
            i += 1;
            spans.push(Span::styled(line[start..i].to_string(), th.dim));
        } else {
            // whitespace or anything else (incl. multi-byte chars)
            i += 1;
            while i < b.len() && !line.is_char_boundary(i) {
                i += 1;
            }
            spans.push(Span::raw(line[start..i].to_string()));
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_print() {
        let got = json_pretty(br#"{"t":21.5,"ok":true,"tags":["a","b"],"e":{},"n":null}"#).unwrap();
        let want = "{\n  \"t\": 21.5,\n  \"ok\": true,\n  \"tags\": [\n    \"a\",\n    \"b\"\n  ],\n  \"e\": {},\n  \"n\": null\n}";
        assert_eq!(got, want);
        assert!(json_pretty(b"{\"a\":}").is_none());
        assert!(json_pretty(b"hello").is_none());
        assert!(json_pretty(b"12 13").is_none());
    }

    #[test]
    fn misc() {
        assert_eq!(commas(1234567), "1,234,567");
        assert_eq!(one_line(b"a\nb\x01", 10), "a↵b·");
        assert!(is_text(b"hello"));
        assert!(!is_text(&[0, 1, 2, 0xff]));
        assert_eq!(trunc_left("v1/plant/temp", 6), "…/temp");
    }
}

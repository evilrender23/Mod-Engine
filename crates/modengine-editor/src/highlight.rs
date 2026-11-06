//! ModScript syntax highlighting for the code editors.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};
use std::collections::HashSet;
use std::sync::OnceLock;

const KEYWORDS: &[&str] = &[
    "var", "if", "else", "while", "do", "until", "for", "repeat", "switch", "case", "default", "break", "continue", "return",
    "exit", "with", "function", "and", "or", "not", "mod", "div", "object", "on", "in", "super",
];
const VALUES: &[&str] = &["true", "false", "null", "none", "self", "other", "all", "global"];

fn builtins() -> &'static HashSet<String> {
    static B: OnceLock<HashSet<String>> = OnceLock::new();
    B.get_or_init(|| {
        let mut s: HashSet<String> = modengine::natives::names().into_iter().collect();
        // network_type_data, buffer_u8, async_load...
        s.extend(modengine::natives::constant_names().into_iter().filter(|n| n.contains('_')));
        for (n, _) in modengine::instance::BUILTIN_VARS {
            s.insert(format!("@{n}"));
        }
        s
    })
}

pub fn highlight(src: &str, font: FontId, dark: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    let pal = crate::theme::code_palette(dark);
    let p = &pal;
    let b = src.as_bytes();
    let mut i = 0;
    let fmt = |c: Color32| TextFormat { font_id: font.clone(), color: c, ..Default::default() };
    let push = |job: &mut LayoutJob, s: &str, c: Color32| job.append(s, 0.0, fmt(c));
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            push(&mut job, &src[start..i], p.comment);
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            push(&mut job, &src[start..i], p.comment);
        } else if c == b'"' || c == b'\'' {
            i += 1;
            while i < b.len() && b[i] != c && b[i] != b'\n' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(b.len());
            push(&mut job, &src[start..i], p.string);
        } else if c == b'#' && i + 7 <= b.len() && b[i + 1..i + 7].iter().all(|x| x.is_ascii_hexdigit()) {
            i += 7;
            let col = u32::from_str_radix(&src[start + 1..i], 16).unwrap_or(0);
            let swatch = Color32::from_rgb((col >> 16) as u8, (col >> 8) as u8, col as u8);
            job.append(&src[start..i], 0.0, TextFormat { font_id: font.clone(), color: swatch, underline: (1.0, swatch).into(), ..Default::default() });
        } else if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') {
                i += 1;
            }
            push(&mut job, &src[start..i], p.number);
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let w = &src[start..i];
            let next = src[i..].trim_start().chars().next();
            let col = if KEYWORDS.contains(&w) {
                p.keyword
            } else if VALUES.contains(&w) {
                p.value
            } else if next == Some('(') && builtins().contains(w) {
                p.builtin
            } else if builtins().contains(&format!("@{w}")) {
                p.var
            } else {
                p.text
            };
            push(&mut job, w, col);
        } else {
            let ch_len = src[i..].chars().next().map(|ch| ch.len_utf8()).unwrap_or(1);
            i += ch_len;
            push(&mut job, &src[start..i], p.text);
        }
    }
    job
}

/// A monospace, syntax-highlighted multi-line text editor.
pub fn code_editor(ui: &mut eframe::egui::Ui, text: &mut String, id: impl std::hash::Hash + std::fmt::Debug, rows: usize) -> eframe::egui::Response {
    use eframe::egui;
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = highlight(buf.as_str(), FontId::monospace(13.5), ui.visuals().dark_mode);
        job.wrap.max_width = wrap;
        ui.ctx().fonts_mut(|f| f.layout_job(job))
    };
    ui.add(
        egui::TextEdit::multiline(text)
            .id_salt(id)
            .code_editor()
            .desired_rows(rows)
            .desired_width(f32::INFINITY)
            .lock_focus(true)
            .layouter(&mut layouter),
    )
}

/// Compile check for a whole file: `(line, message)` of the first error.
pub fn compile_error(file: &str, src: &str) -> Option<(u32, String)> {
    modscript::parse_file(file, src).err().map(|e| (e.line, e.msg))
}

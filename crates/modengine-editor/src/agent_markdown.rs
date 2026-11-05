//! Markdown of the agent's replies: headings, paragraphs with **bold**, *italic*, `code` and
//! [links](https://…), bulleted and numbered lists, quotes, rules and fenced code blocks (with
//! ModScript highlighting and a copy button).

use eframe::egui::{self, RichText};

/// A block of a reply.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(u8, String),
    Paragraph(String),
    /// `marker` is "•" or "1." etc.; `depth` from indentation.
    Item { marker: String, depth: u8, text: String },
    Quote(String),
    Rule,
    Code { lang: String, code: String },
}

/// Splits `text` into blocks.
pub fn blocks(text: &str) -> Vec<Block> {
    let mut out = vec![];
    let mut para = String::new();
    let mut code: Option<(String, String, String)> = None; // (fence, lang, code)
    let flush = |para: &mut String, out: &mut Vec<Block>| {
        if !para.trim().is_empty() {
            out.push(Block::Paragraph(para.trim().to_string()));
        }
        para.clear();
    };
    for line in text.lines() {
        if let Some((fence, lang, body)) = &mut code {
            if line.trim_start().starts_with(fence.as_str()) && line.trim().trim_start_matches('`').is_empty() {
                out.push(Block::Code { lang: std::mem::take(lang), code: body.trim_end_matches('\n').to_string() });
                code = None;
            } else {
                body.push_str(line);
                body.push('\n');
            }
            continue;
        }
        let t = line.trim_start();
        let indent = line.len() - t.len();
        if t.starts_with("```") {
            flush(&mut para, &mut out);
            let fence: String = t.chars().take_while(|c| *c == '`').collect();
            code = Some((fence.clone(), t[fence.len()..].trim().to_lowercase(), String::new()));
            continue;
        }
        if t.is_empty() {
            flush(&mut para, &mut out);
            continue;
        }
        let hashes = t.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            flush(&mut para, &mut out);
            out.push(Block::Heading(hashes as u8, t[hashes..].trim().trim_end_matches('#').trim().to_string()));
            continue;
        }
        if matches!(t, "---" | "***" | "___") || (t.len() >= 3 && t.chars().all(|c| c == '-')) {
            flush(&mut para, &mut out);
            out.push(Block::Rule);
            continue;
        }
        let depth = (indent / 2).min(4) as u8;
        if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")) {
            flush(&mut para, &mut out);
            let (marker, rest) = match rest.strip_prefix("[ ] ").map(|r| ("☐", r)).or_else(|| rest.strip_prefix("[x] ").map(|r| ("☑", r))) {
                Some((m, r)) => (m, r),
                None => ("•", rest),
            };
            out.push(Block::Item { marker: marker.into(), depth, text: rest.trim().to_string() });
            continue;
        }
        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && digits < 4 && (t[digits..].starts_with(". ") || t[digits..].starts_with(") ")) {
            flush(&mut para, &mut out);
            out.push(Block::Item { marker: format!("{}.", &t[..digits]), depth, text: t[digits + 2..].trim().to_string() });
            continue;
        }
        if let Some(q) = t.strip_prefix('>') {
            flush(&mut para, &mut out);
            match out.last_mut() {
                Some(Block::Quote(prev)) => {
                    prev.push('\n');
                    prev.push_str(q.trim());
                }
                _ => out.push(Block::Quote(q.trim().to_string())),
            }
            continue;
        }
        // A line after a list item, indented, continues it.
        if indent >= 2 && para.is_empty() {
            if let Some(Block::Item { text, .. }) = out.last_mut() {
                text.push(' ');
                text.push_str(t);
                continue;
            }
        }
        if !para.is_empty() {
            para.push(if line.ends_with("  ") { '\n' } else { ' ' });
        }
        para.push_str(t.trim_end());
    }
    if let Some((_, lang, body)) = code {
        out.push(Block::Code { lang, code: body.trim_end_matches('\n').to_string() });
    }
    flush(&mut para, &mut out);
    out
}

/// A piece of a paragraph.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    /// A link's target.
    pub url: Option<String>,
}

/// The inline formatting of a paragraph.
pub fn spans(text: &str) -> Vec<Span> {
    let mut out: Vec<Span> = vec![];
    let (mut bold, mut italic) = (false, false);
    let mut cur = String::new();
    let push = |out: &mut Vec<Span>, cur: &mut String, bold: bool, italic: bool| {
        if !cur.is_empty() {
            out.push(Span { text: std::mem::take(cur), bold, italic, code: false, url: None });
        }
    };
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let at = |i: usize| chars.get(i).copied();
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && at(i + 1).is_some_and(|n| "*_`[]\\".contains(n)) {
            cur.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '`' {
            if let Some(end) = (i + 1..chars.len()).find(|j| chars[*j] == '`') {
                push(&mut out, &mut cur, bold, italic);
                out.push(Span { text: chars[i + 1..end].iter().collect(), bold, italic, code: true, url: None });
                i = end + 1;
                continue;
            }
        }
        if c == '[' {
            if let Some(close) = (i + 1..chars.len()).find(|j| chars[*j] == ']') {
                if at(close + 1) == Some('(') {
                    if let Some(end) = (close + 2..chars.len()).find(|j| chars[*j] == ')') {
                        push(&mut out, &mut cur, bold, italic);
                        let label: String = chars[i + 1..close].iter().collect();
                        let url: String = chars[close + 2..end].iter().collect();
                        out.push(Span { text: label, bold, italic, code: false, url: Some(url) });
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        if (c == '*' || c == '_') && at(i + 1) == Some(c) {
            push(&mut out, &mut cur, bold, italic);
            bold = !bold;
            i += 2;
            continue;
        }
        // A single * or _ toggles italics when it hugs a word (not "a * b", not snake_case).
        if c == '*' || c == '_' {
            let prev = if i == 0 { None } else { at(i - 1) };
            let next = at(i + 1);
            let opens = !italic && next.is_some_and(|n| !n.is_whitespace()) && prev.is_none_or(|p| !p.is_alphanumeric());
            let closes = italic && prev.is_some_and(|p| !p.is_whitespace()) && next.is_none_or(|n| !n.is_alphanumeric());
            if opens || closes {
                push(&mut out, &mut cur, bold, italic);
                italic = !italic;
                i += 1;
                continue;
            }
        }
        // Bare links.
        if (c == 'h') && (text_at(&chars, i, "https://") || text_at(&chars, i, "http://")) && (i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(') {
            let end = (i..chars.len()).find(|j| chars[*j].is_whitespace()).unwrap_or(chars.len());
            let mut url: String = chars[i..end].iter().collect();
            while url.ends_with(['.', ',', ')', ';', ':']) {
                url.pop();
            }
            push(&mut out, &mut cur, bold, italic);
            i += url.chars().count();
            out.push(Span { text: url.clone(), bold, italic, code: false, url: Some(url) });
            continue;
        }
        cur.push(c);
        i += 1;
    }
    push(&mut out, &mut cur, bold, italic);
    out
}

fn text_at(chars: &[char], i: usize, s: &str) -> bool {
    s.chars().enumerate().all(|(k, c)| chars.get(i + k) == Some(&c))
}

fn rich(s: &Span, size: Option<f32>, strong: bool) -> RichText {
    let mut r = RichText::new(&s.text);
    if let Some(sz) = size {
        r = r.size(sz);
    }
    if s.bold || strong {
        r = r.strong();
    }
    if s.italic {
        r = r.italics();
    }
    if s.code {
        r = r.code();
    }
    r
}

/// Draws one paragraph's spans, wrapping.
fn inline(ui: &mut egui::Ui, text: &str, size: Option<f32>, strong: bool) {
    let sp = spans(text);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for s in &sp {
            match &s.url {
                Some(url) => {
                    ui.hyperlink_to(rich(s, size, strong), url).on_hover_text(url);
                }
                None => {
                    ui.label(rich(s, size, strong));
                }
            }
        }
    });
}

/// Draws a reply.
pub fn show(ui: &mut egui::Ui, text: &str) {
    let base = ui.style().text_styles.get(&egui::TextStyle::Body).map(|f| f.size).unwrap_or(14.0);
    for (i, b) in blocks(text).iter().enumerate() {
        if i > 0 {
            ui.add_space(2.0);
        }
        match b {
            Block::Heading(level, t) => {
                let size = match level {
                    1 => base + 5.0,
                    2 => base + 3.0,
                    3 => base + 1.0,
                    _ => base,
                };
                ui.add_space(2.0);
                inline(ui, t, Some(size), true);
            }
            Block::Paragraph(t) => {
                for line in t.split('\n') {
                    inline(ui, line, None, false);
                }
            }
            Block::Item { marker, depth, text } => {
                ui.horizontal(|ui| {
                    ui.add_space(4.0 + *depth as f32 * 14.0);
                    ui.label(RichText::new(marker).weak());
                    ui.vertical(|ui| inline(ui, text, None, false));
                });
            }
            Block::Quote(t) => {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(3.0, 16.0 * t.lines().count().max(1) as f32), egui::Sense::hover());
                    ui.painter().rect_filled(r, 1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
                    ui.vertical(|ui| {
                        for line in t.lines() {
                            inline(ui, line, None, false);
                        }
                    });
                });
            }
            Block::Rule => {
                ui.separator();
            }
            Block::Code { lang, code } => code_block(ui, lang, code),
        }
    }
}

/// A fenced code block: highlighted when it is ModScript, with a copy button.
pub fn code_block(ui: &mut egui::Ui, lang: &str, code: &str) {
    egui::Frame::new().fill(ui.visuals().extreme_bg_color).corner_radius(crate::theme::RADIUS).inner_margin(6).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(if lang.is_empty() { "código" } else { lang }).weak().small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("⧉ Copiar").small()).frame(false)).on_hover_text("Copiar el código").clicked() {
                    ui.ctx().copy_text(code.to_string());
                }
            });
        });
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let modscript = matches!(lang, "ms" | "modscript" | "gml" | "gamemaker" | "js" | "javascript");
        egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).show(ui, |ui| {
            if modscript {
                let job = crate::highlight::highlight(code, font, ui.visuals().dark_mode);
                ui.add(egui::Label::new(job).extend());
            } else {
                ui.add(egui::Label::new(RichText::new(code).monospace()).extend());
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_blocks() {
        let b = blocks("# Hecho\nUna línea\nsigue.\n\n- uno\n  continúa\n- [x] dos\n2. tres\n> cita\n---\n```ms\nx = 1\n```\nfin");
        assert_eq!(
            b,
            vec![
                Block::Heading(1, "Hecho".into()),
                Block::Paragraph("Una línea sigue.".into()),
                Block::Item { marker: "•".into(), depth: 0, text: "uno continúa".into() },
                Block::Item { marker: "☑".into(), depth: 0, text: "dos".into() },
                Block::Item { marker: "2.".into(), depth: 0, text: "tres".into() },
                Block::Quote("cita".into()),
                Block::Rule,
                Block::Code { lang: "ms".into(), code: "x = 1".into() },
                Block::Paragraph("fin".into()),
            ]
        );
        // An unclosed fence still shows its code.
        assert_eq!(blocks("```\nabc"), vec![Block::Code { lang: String::new(), code: "abc".into() }]);
    }

    #[test]
    fn inline_formatting() {
        let s = spans("Sube **jump_speed** a `12`, *mucho* y lee [la guía](https://x.org/a) o https://y.org/b.");
        let parts: Vec<(&str, bool, bool, bool, Option<&str>)> = s.iter().map(|s| (s.text.as_str(), s.bold, s.italic, s.code, s.url.as_deref())).collect();
        assert_eq!(
            parts,
            vec![
                ("Sube ", false, false, false, None),
                ("jump_speed", true, false, false, None),
                (" a ", false, false, false, None),
                ("12", false, false, true, None),
                (", ", false, false, false, None),
                ("mucho", false, true, false, None),
                (" y lee ", false, false, false, None),
                ("la guía", false, false, false, Some("https://x.org/a")),
                (" o ", false, false, false, None),
                ("https://y.org/b", false, false, false, Some("https://y.org/b")),
                (".", false, false, false, None),
            ]
        );
        // snake_case and lone asterisks stay text.
        assert_eq!(spans("a * b y on_step_end").iter().map(|s| s.text.as_str()).collect::<String>(), "a * b y on_step_end");
        assert!(spans("a * b y on_step_end").iter().all(|s| !s.italic));
    }
}

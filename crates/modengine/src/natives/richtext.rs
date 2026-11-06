//! Text drawn glyph by glyph: `draw_text_glyphs` (a callback moves / tints every
//! character) and `draw_text_rich` (inline tags: `[c=#f00]`, `[wave]`, `[shake]`, `[b]`...).
//! Glyph positions come from the same measuring the backend uses, so they match
//! `draw_text` with TTF, sprite and built-in fonts alike.

use super::{err, Reg, A};
use crate::draw::{parse_color, DrawCmd, Rgba};
use crate::engine::Engine;
use modscript::{call_value, intern, CallCx, RtResult, Value};

/// One character to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    pub ch: char,
    /// Pen position (top-left of the character cell) before effects.
    pub x: f64,
    pub y: f64,
    pub line: usize,
    pub style: Style,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub color: u32,
    pub alpha: f64,
    pub scale: f64,
    pub wave: f64,
    pub shake: f64,
    pub rainbow: bool,
    pub bold: bool,
}

/// What a tag changes, so `[/]` / `[/c]` can undo it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tag {
    Color(u32),
    Alpha(f64),
    Scale(f64),
    Wave(f64),
    Shake(f64),
    Rainbow,
    Bold,
}

impl Tag {
    fn kind(&self) -> &'static str {
        match self {
            Tag::Color(_) => "c",
            Tag::Alpha(_) => "a",
            Tag::Scale(_) => "s",
            Tag::Wave(_) => "wave",
            Tag::Shake(_) => "shake",
            Tag::Rainbow => "rainbow",
            Tag::Bold => "b",
        }
    }
}

fn parse_tag(body: &str) -> Option<Tag> {
    let (name, arg) = match body.split_once('=') {
        Some((n, a)) => (n.trim(), Some(a.trim())),
        None => (body.trim(), None),
    };
    let num = |d: f64| arg.map(|a| a.parse::<f64>().ok()).unwrap_or(Some(d));
    Some(match name {
        "c" | "color" | "colour" => Tag::Color(parse_color(arg?)?),
        "a" | "alpha" => Tag::Alpha(num(1.0)?.clamp(0.0, 1.0)),
        "s" | "scale" => Tag::Scale(num(1.0)?.max(0.0)),
        "wave" => Tag::Wave(num(2.0)?),
        "shake" => Tag::Shake(num(1.0)?),
        "rainbow" => Tag::Rainbow,
        "b" | "bold" => Tag::Bold,
        _ => return None,
    })
}

/// Rich text → (character, style) pairs. Unknown tags stay as text; `[[` is a literal `[`.
pub fn parse_rich(text: &str, base: &Style) -> Vec<(char, Style)> {
    let mut out = vec![];
    let mut stack: Vec<Tag> = vec![];
    let style = |stack: &[Tag]| {
        let mut s = base.clone();
        for t in stack {
            match *t {
                Tag::Color(c) => s.color = c,
                Tag::Alpha(a) => s.alpha = base.alpha * a,
                Tag::Scale(k) => s.scale = base.scale * k,
                Tag::Wave(a) => s.wave = a,
                Tag::Shake(a) => s.shake = a,
                Tag::Rainbow => s.rainbow = true,
                Tag::Bold => s.bold = true,
            }
        }
        s
    };
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' {
            if chars.get(i + 1) == Some(&'[') {
                out.push(('[', style(&stack)));
                i += 2;
                continue;
            }
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == ']' || c == '[') {
                let end = i + 1 + end;
                if chars[end] == ']' {
                    let body: String = chars[i + 1..end].iter().collect();
                    if let Some(close) = body.strip_prefix('/') {
                        let close = match close.trim() {
                            "color" | "colour" => "c",
                            "alpha" => "a",
                            "scale" => "s",
                            "bold" => "b",
                            other => other,
                        };
                        let k = if close.is_empty() { stack.len().checked_sub(1) } else { stack.iter().rposition(|t| t.kind() == close) };
                        if let Some(k) = k {
                            stack.remove(k);
                            i = end + 1;
                            continue;
                        }
                    } else if let Some(t) = parse_tag(&body) {
                        stack.push(t);
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        out.push((c, style(&stack)));
        i += 1;
    }
    out
}

/// The text without its tags.
pub fn strip_rich(text: &str) -> String {
    let base = Style { color: 0, alpha: 1.0, scale: 1.0, wave: 0.0, shake: 0.0, rainbow: false, bold: false };
    parse_rich(text, &base).into_iter().map(|(c, _)| c).collect()
}

fn width(e: &Engine, s: &str) -> f64 {
    (e.measure)(e.draw.font, s, 1.0).0 as f64
}

fn line_height(e: &Engine, sep: f64) -> f64 {
    if sep >= 0.0 {
        sep
    } else {
        (e.measure)(e.draw.font, "A", 1.0).1 as f64
    }
}

/// Breaks styled characters into lines (at `\n`, and at spaces to fit `max_w` when > 0).
fn wrap_styled(e: &Engine, chars: Vec<(char, Style)>, max_w: f64) -> Vec<Vec<(char, Style)>> {
    let mut lines = vec![];
    for para in chars.split(|(c, _)| *c == '\n') {
        if max_w <= 0.0 {
            lines.push(para.to_vec());
            continue;
        }
        let mut line: Vec<(char, Style)> = vec![];
        for word in para.split(|(c, _)| *c == ' ') {
            let plain = |v: &[(char, Style)]| v.iter().map(|(c, _)| *c).collect::<String>();
            let mut candidate = line.clone();
            if !candidate.is_empty() {
                candidate.push((' ', word.first().map(|w| w.1.clone()).unwrap_or_else(|| line[line.len() - 1].1.clone())));
            }
            candidate.extend_from_slice(word);
            if line.is_empty() || width(e, &plain(&candidate)) <= max_w {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_vec();
            }
        }
        lines.push(line);
    }
    lines
}

/// Lays out styled lines at (x, y) honouring `set_align`.
fn layout(e: &Engine, lines: Vec<Vec<(char, Style)>>, x: f64, y: f64, lh: f64) -> (Vec<Glyph>, f64, f64) {
    let total = lh * lines.len() as f64;
    let top = y - match e.draw.valign {
        1 => total / 2.0,
        2 => total,
        _ => 0.0,
    };
    let mut out = vec![];
    let mut max_w = 0.0f64;
    for (li, line) in lines.into_iter().enumerate() {
        let text: String = line.iter().map(|(c, _)| *c).collect();
        // scaled characters push the rest of the line
        let mut extra = 0.0;
        let mut xs = Vec::with_capacity(line.len());
        let mut prefix = String::new();
        for (c, st) in &line {
            prefix.push(*c);
            let one = width(e, &c.to_string());
            let pen = width(e, &prefix) - one + extra;
            xs.push(pen);
            extra += one * (st.scale - 1.0);
        }
        let w = width(e, &text) + extra;
        max_w = max_w.max(w);
        let lx = x - match e.draw.halign {
            1 => w / 2.0,
            2 => w,
            _ => 0.0,
        };
        for ((c, st), px) in line.into_iter().zip(xs) {
            out.push(Glyph { ch: c, x: lx + px, y: top + li as f64 * lh, line: li, style: st });
        }
    }
    (out, max_w, total)
}

/// Deterministic jitter in [-1, 1] for `[shake]` (changes every 2 steps).
fn jitter(seed: u64) -> f64 {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    super::rand01(&mut s) * 2.0 - 1.0
}

fn hsv(h: f64) -> u32 {
    let h = h.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    let to = |v: f64| (v * 255.0).round() as u32;
    (to(r) << 16) | (to(g) << 8) | to(b)
}

/// Records one glyph (twice, 1 px apart, when bold).
fn emit(e: &mut Engine, ch: char, x: f64, y: f64, scale: f64, color: u32, alpha: f64, bold: bool) {
    if ch == ' ' || alpha <= 0.0 || scale <= 0.0 {
        return;
    }
    let (font, c) = (e.draw.font, Rgba::from_rgb(color, alpha));
    let text = ch.to_string();
    // scale around the character's centre
    let (w, h) = (e.measure)(font, &text, 1.0);
    let (ox, oy) = (w as f64 * (1.0 - scale) / 2.0, h as f64 * (1.0 - scale) / 2.0);
    for k in 0..if bold { 2 } else { 1 } {
        e.cmds.push(DrawCmd::Text { x: (x + ox + k as f64) as f32, y: (y + oy) as f32, text: text.clone(), font, scale: scale as f32, color: c, halign: 0, valign: 0 });
    }
}

fn base_style(e: &Engine) -> Style {
    Style { color: e.draw.color, alpha: e.draw.alpha, scale: 1.0, wave: 0.0, shake: 0.0, rainbow: false, bold: false }
}

fn size_value(w: f64, h: f64, lines: usize) -> Value {
    Value::new_struct([(intern("w"), Value::Num(w)), (intern("h"), Value::Num(h)), (intern("lines"), Value::Num(lines as f64))])
}

fn num_field(v: &Value, k: &str) -> Option<f64> {
    match v {
        Value::Struct(s) => s.borrow().fields.get(&intern(k)).and_then(|v| v.as_num()),
        _ => None,
    }
}

fn draw_glyphs(e: &mut Engine, cx: &CallCx, a: &[Value]) -> RtResult<Value> {
    // draw_text_glyphs(x, y, text, fn(i, ch, x, y, line) -> { dx, dy, color, alpha, scale, visible, char } | null)
    let p = A(a);
    let (x, y) = (p.num(0)?, p.num(1)?);
    let text = p.text(2)?;
    let f = p.val(3)?.clone();
    if !matches!(f, Value::Func(_) | Value::Native(_)) {
        return err(format!("draw_text_glyphs: argument #4 must be a function, got {}", f.type_name()));
    }
    let base = base_style(e);
    let lines: Vec<Vec<(char, Style)>> = text.split('\n').map(|l| l.chars().map(|c| (c, base.clone())).collect()).collect();
    let lh = line_height(e, -1.0);
    let n_lines = lines.len();
    let (glyphs, w, h) = layout(e, lines, x, y, lh);
    for (i, g) in glyphs.into_iter().enumerate() {
        let r = call_value(e, &f, cx, vec![Value::Num(i as f64), Value::string(g.ch.to_string()), Value::Num(g.x), Value::Num(g.y), Value::Num(g.line as f64)])?;
        if matches!(&r, Value::Bool(false)) || matches!(&r, Value::Struct(s) if s.borrow().fields.get(&intern("visible")).is_some_and(|v| !v.truthy())) {
            continue;
        }
        let dx = num_field(&r, "dx").unwrap_or(0.0);
        let dy = num_field(&r, "dy").unwrap_or(0.0);
        let color = num_field(&r, "color").or_else(|| num_field(&r, "colour")).map(|c| c.max(0.0) as u32 & 0xFF_FFFF).unwrap_or(base.color);
        let alpha = num_field(&r, "alpha").unwrap_or(base.alpha);
        let scale = num_field(&r, "scale").unwrap_or(1.0);
        let ch = match &r {
            Value::Struct(s) => s.borrow().fields.get(&intern("char")).and_then(|v| v.to_string().chars().next()).unwrap_or(g.ch),
            _ => g.ch,
        };
        emit(e, ch, g.x + dx, g.y + dy, scale, color, alpha, false);
    }
    Ok(size_value(w, h, n_lines))
}

fn draw_rich(e: &mut Engine, a: &[Value]) -> RtResult<Value> {
    // draw_text_rich(x, y, text, [w=-1], [sep=-1]) -> { w, h, lines }
    let p = A(a);
    let (x, y) = (p.num(0)?, p.num(1)?);
    let chars = parse_rich(&p.text(2)?, &base_style(e));
    let lines = wrap_styled(e, chars, p.num_or(3, -1.0)?);
    let n = lines.len();
    let lh = line_height(e, p.num_or(4, -1.0)?);
    let (glyphs, w, h) = layout(e, lines, x, y, lh);
    let t = e.steps;
    for (i, g) in glyphs.into_iter().enumerate() {
        let st = &g.style;
        let mut gy = g.y;
        let mut gx = g.x;
        if st.wave != 0.0 {
            gy += (t as f64 * 0.15 + i as f64 * 0.6).sin() * st.wave;
        }
        if st.shake != 0.0 {
            let s = (t / 2) * 7919 + i as u64 * 104_729;
            gx += jitter(s) * st.shake;
            gy += jitter(s ^ 0xABCDEF) * st.shake;
        }
        let color = if st.rainbow { hsv(t as f64 * 4.0 + i as f64 * 24.0) } else { st.color };
        emit(e, g.ch, gx, gy, st.scale, color, st.alpha, st.bold);
    }
    Ok(size_value(w, h, n))
}

pub(super) fn register(r: &mut Reg) {
    r.f("draw_text_glyphs", draw_glyphs);
    r.f("draw_text_rich", |e, _, a| draw_rich(e, a));
    r.f("text_rich_strip", |_, _, a| Ok(Value::string(strip_rich(&A(a).text(0)?))));
    r.f("text_rich_size", |e, _, a| {
        // text_rich_size(text, [w=-1], [sep=-1]) -> { w, h, lines } without drawing
        let p = A(a);
        let chars = parse_rich(&p.text(0)?, &base_style(e));
        let lines = wrap_styled(e, chars, p.num_or(1, -1.0)?);
        let n = lines.len();
        let lh = line_height(e, p.num_or(2, -1.0)?);
        let (_, w, h) = layout(e, lines, 0.0, 0.0, lh);
        Ok(size_value(w, h, n))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Style {
        Style { color: 0xFFFFFF, alpha: 1.0, scale: 1.0, wave: 0.0, shake: 0.0, rainbow: false, bold: false }
    }

    #[test]
    fn tags_nest_and_close() {
        let v = parse_rich("a[c=#ff0000]b[wave]c[/]d[/c]e", &base());
        let s: String = v.iter().map(|(c, _)| *c).collect();
        assert_eq!(s, "abcde");
        assert_eq!(v[1].1.color, 0xFF0000);
        assert_eq!((v[2].1.color, v[2].1.wave), (0xFF0000, 2.0));
        assert_eq!((v[3].1.color, v[3].1.wave), (0xFF0000, 0.0));
        assert_eq!(v[4].1.color, 0xFFFFFF);
    }

    #[test]
    fn unknown_tags_and_escapes_stay_text() {
        assert_eq!(strip_rich("[x] [[b] [b]ok[/b] [c=red]r[/color]"), "[x] [b] ok r");
        assert_eq!(strip_rich("100% [unclosed"), "100% [unclosed");
        let v = parse_rich("[b][s=2]x", &base());
        assert!(v[0].1.bold && v[0].1.scale == 2.0);
    }
}

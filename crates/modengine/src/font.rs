//! Font assets (`fonts/`), GameMaker style: a TTF/OTF face (or a sprite) baked once into a
//! glyph atlas at a fixed size, with bold/italic, antialiasing on/off and character ranges.
//!
//! ```toml
//! # fonts/fnt_title.toml
//! file = "fnt_title.ttf"   # TTF/OTF next to this file; omit it to use the built-in font
//! size = 24
//! bold = true              # synthesized when the face isn't bold already
//! italic = false           # synthesized (slanted) when the face isn't italic
//! antialias = false        # crisp pixel-art glyphs
//! ranges = [[32, 127], [161, 255]]   # characters baked into the atlas
//! spacing = 0              # extra pixels between characters
//!
//! # ...or a sprite font: one sprite frame per character, in this order
//! # sprite = "spr_digits"
//! # chars = "0123456789"
//! # proportional = true    # trim each glyph to its pixels (otherwise fixed width)
//! ```
//!
//! The atlas is plain RGBA (white glyphs, coverage in alpha), shared by the game backend
//! (one texture) and the editor preview, and its metrics drive `text_width`/`text_height`
//! everywhere, also in headless runs.

use crate::project::Sprite;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Default character range (printable ASCII), like GameMaker's.
pub const DEFAULT_RANGES: [[u32; 2]; 1] = [[32, 127]];

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct FontToml {
    pub file: Option<String>,
    pub size: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub antialias: Option<bool>,
    pub ranges: Vec<[u32; 2]>,
    pub spacing: f32,
    pub sprite: Option<String>,
    pub chars: Option<String>,
    pub proportional: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    /// Rectangle in the atlas.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Offset of the bitmap from the pen position (y down, relative to the line top).
    pub ox: f32,
    pub oy: f32,
    pub advance: f32,
}

/// A baked font: one RGBA page plus glyph metrics.
#[derive(Clone, Debug)]
pub struct GlyphAtlas {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub glyphs: HashMap<char, Glyph>,
    /// Distance between baselines.
    pub line_height: f32,
    pub ascent: f32,
    pub spacing: f32,
    kern: HashMap<(char, char), f32>,
}

pub struct FontAsset {
    pub name: String,
    /// TTF/OTF bytes (empty for the built-in font and sprite fonts).
    pub data: Vec<u8>,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub antialias: bool,
    pub ranges: Vec<[u32; 2]>,
    /// Font file name (`fonts/<file>`) or sprite name, for `font_get_fontname`.
    pub source: String,
    /// None: the backend's built-in font at `size`.
    pub atlas: Option<Arc<GlyphAtlas>>,
    /// Why the face couldn't be baked (the game then uses the built-in font, as before).
    pub error: Option<String>,
}

impl FontAsset {
    pub fn first(&self) -> u32 {
        self.ranges.iter().map(|r| r[0]).min().unwrap_or(32)
    }
    pub fn last(&self) -> u32 {
        self.ranges.iter().map(|r| r[1]).max().unwrap_or(127)
    }
}

/// Loads `fonts/<name>.toml|ttf|otf`. `sprite` finds sprites for sprite fonts.
pub fn load_font<'s>(name: &str, path: &Path, is_toml: bool, sprite: &dyn Fn(&str) -> Option<&'s Sprite>) -> Result<FontAsset, String> {
    let where_ = path.display().to_string();
    let t: FontToml = if is_toml {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{where_}: {e}"))?;
        toml::from_str(&text).map_err(|e| format!("{where_}: {e}"))?
    } else {
        FontToml { file: Some(path.file_name().unwrap_or_default().to_string_lossy().to_string()), ..FontToml::default() }
    };
    let size = t.size.unwrap_or(16.0).clamp(1.0, 512.0);
    let antialias = t.antialias.unwrap_or(true);
    let ranges = if t.ranges.is_empty() { DEFAULT_RANGES.to_vec() } else { t.ranges.clone() };
    for r in &ranges {
        if r[0] > r[1] || r[1] > 0x10FFFF {
            return Err(format!("{where_}: bad character range [{}, {}]", r[0], r[1]));
        }
    }
    if ranges.iter().map(|r| (r[1] - r[0] + 1) as u64).sum::<u64>() > 20_000 {
        return Err(format!("{where_}: too many characters in `ranges` (max 20000)"));
    }
    let mut f = FontAsset { name: name.to_string(), data: vec![], size, bold: t.bold, italic: t.italic, antialias, ranges, source: String::new(), atlas: None, error: None };
    if let Some(s) = &t.sprite {
        let spr = sprite(s).ok_or_else(|| format!("{where_}: sprite '{s}' does not exist"))?;
        let chars = t.chars.clone().unwrap_or_else(|| (32u8..127).map(|c| c as char).collect());
        f.source = s.clone();
        f.size = spr.fh as f32;
        f.atlas = Some(Arc::new(bake_sprite(spr, &chars, t.proportional.unwrap_or(true), t.spacing)));
        return Ok(f);
    }
    if let Some(file) = &t.file {
        let fp = path.parent().unwrap_or(Path::new(".")).join(file);
        f.data = std::fs::read(&fp).map_err(|e| format!("{}: {e}", fp.display()))?;
        f.source = file.clone();
        // a broken face doesn't stop the game from loading: it falls back to the built-in font
        match bake_ttf(&f.data, &f.ranges, size, t.bold, t.italic, antialias, t.spacing) {
            Ok(atlas) => f.atlas = Some(Arc::new(atlas)),
            Err(e) => f.error = Some(format!("{where_}: {e}")),
        }
    }
    Ok(f)
}

/// Rasterizes the characters of `ranges` from TTF/OTF bytes.
pub fn bake_ttf(data: &[u8], ranges: &[[u32; 2]], size: f32, bold: bool, italic: bool, antialias: bool, spacing: f32) -> Result<GlyphAtlas, String> {
    let font = fontdue::Font::from_bytes(data, fontdue::FontSettings { scale: size, ..Default::default() }).map_err(|e| format!("not a valid TTF/OTF font: {e}"))?;
    let lm = font.horizontal_line_metrics(size);
    let ascent = lm.map(|l| l.ascent).unwrap_or(size * 0.8).ceil();
    let line_height = lm.map(|l| l.new_line_size).unwrap_or(size * 1.2).ceil();
    // Synthesized styles, only when the face itself isn't already that style.
    let face = font.name().unwrap_or("").to_lowercase();
    let fake_bold = bold && !face.contains("bold");
    let fake_italic = italic && !(face.contains("italic") || face.contains("oblique"));
    let embolden = if fake_bold { (size / 14.0).round().max(1.0) as usize } else { 0 };
    let mut bitmaps: Vec<(char, usize, usize, Vec<u8>, f32, f32, f32)> = vec![];
    for r in ranges {
        for cp in r[0]..=r[1] {
            let Some(ch) = char::from_u32(cp) else { continue };
            if !font.has_glyph(ch) && ch != ' ' {
                continue;
            }
            let (m, mut bmp) = font.rasterize(ch, size);
            let (mut w, h) = (m.width, m.height);
            let ox = m.xmin as f32;
            let oy = ascent - (m.ymin as f32 + h as f32);
            let mut adv = m.advance_width;
            if fake_italic && w > 0 && h > 0 {
                // shear: rows near the top move right
                let slant = 0.2;
                let extra = (h as f32 * slant).ceil() as usize;
                let mut out = vec![0u8; (w + extra) * h];
                for y in 0..h {
                    // the baseline is at row (ascent - oy); keep it in place
                    let above = (ascent - oy) - y as f32;
                    let shift = (above * slant).round().max(0.0) as usize;
                    for x in 0..w {
                        if let Some(o) = out.get_mut(y * (w + extra) + x + shift.min(extra)) {
                            *o = bmp[y * w + x];
                        }
                    }
                }
                bmp = out;
                w += extra;
            }
            if embolden > 0 && w > 0 && h > 0 {
                let nw = w + embolden;
                let mut out = vec![0u8; nw * h];
                for y in 0..h {
                    for x in 0..w {
                        let v = bmp[y * w + x];
                        for k in 0..=embolden {
                            let o = &mut out[y * nw + x + k];
                            *o = (*o).max(v);
                        }
                    }
                }
                bmp = out;
                w = nw;
                adv += embolden as f32;
            }
            if !antialias {
                for v in &mut bmp {
                    *v = if *v >= 128 { 255 } else { 0 };
                }
            }
            bitmaps.push((ch, w, h, bmp, ox, oy, adv));
        }
    }
    let mut kern = HashMap::new();
    let chars: Vec<char> = bitmaps.iter().map(|b| b.0).collect();
    if chars.len() <= 400 {
        for &a in &chars {
            for &b in &chars {
                if let Some(k) = font.horizontal_kern(a, b, size).filter(|k| k.abs() >= 0.5) {
                    kern.insert((a, b), k.round());
                }
            }
        }
    }
    let mut atlas = pack(bitmaps.into_iter().map(|(c, w, h, bmp, ox, oy, adv)| (c, w, h, bmp.into_iter().map(|a| [255, 255, 255, a]).collect(), ox, oy, adv)).collect(), line_height, ascent, spacing);
    atlas.kern = kern;
    Ok(atlas)
}

/// A sprite font: frame `i` of the sprite is character `chars[i]`.
pub fn bake_sprite(spr: &Sprite, chars: &str, proportional: bool, spacing: f32) -> GlyphAtlas {
    let mut items = vec![];
    for (i, ch) in chars.chars().enumerate() {
        let Some(fr) = spr.frames.get(i) else { break };
        let px = |x: u32, y: u32| -> [u8; 4] {
            let k = (((fr[1] + y) * spr.sheet_w + fr[0] + x) * 4) as usize;
            [spr.rgba[k], spr.rgba[k + 1], spr.rgba[k + 2], spr.rgba[k + 3]]
        };
        let (mut x0, mut x1) = (0, fr[2]);
        if proportional {
            let used: Vec<u32> = (0..fr[2]).filter(|&x| (0..fr[3]).any(|y| px(x, y)[3] > 0)).collect();
            match (used.first(), used.last()) {
                (Some(&a), Some(&b)) => (x0, x1) = (a, b + 1),
                // an empty frame (a space) keeps half the width
                _ => (x0, x1) = (0, (fr[2] / 2).max(1)),
            }
        }
        let w = (x1 - x0) as usize;
        let h = fr[3] as usize;
        let mut rgba = Vec::with_capacity(w * h);
        for y in 0..fr[3] {
            for x in x0..x1 {
                rgba.push(px(x, y));
            }
        }
        let adv = w as f32 + if proportional { 1.0 } else { 0.0 };
        items.push((ch, w, h, rgba, 0.0, 0.0, adv));
    }
    pack(items, spr.fh as f32, spr.fh as f32, spacing)
}

type Item = (char, usize, usize, Vec<[u8; 4]>, f32, f32, f32);

/// Shelf-packs glyph bitmaps into one page (1 px of padding).
fn pack(mut items: Vec<Item>, line_height: f32, ascent: f32, spacing: f32) -> GlyphAtlas {
    items.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    let area: usize = items.iter().map(|i| (i.1 + 1) * (i.2 + 1)).sum();
    let mut width = 64u32;
    while (width as usize * width as usize) < area * 2 && width < 4096 {
        width *= 2;
    }
    let (mut x, mut y, mut shelf) = (1u32, 1u32, 0u32);
    let mut glyphs = HashMap::new();
    let mut placed = vec![];
    for (ch, w, h, rgba, ox, oy, adv) in items {
        let (w32, h32) = (w as u32, h as u32);
        if x + w32 + 1 > width {
            x = 1;
            y += shelf + 1;
            shelf = 0;
        }
        glyphs.insert(ch, Glyph { x, y, w: w32, h: h32, ox, oy, advance: adv });
        placed.push((x, y, w, h, rgba));
        x += w32 + 1;
        shelf = shelf.max(h32);
    }
    let height = (y + shelf + 1).max(1).next_power_of_two();
    let mut page = vec![0u8; (width * height * 4) as usize];
    for (px, py, w, h, rgba) in placed {
        for row in 0..h {
            for col in 0..w {
                let k = (((py as usize + row) * width as usize) + px as usize + col) * 4;
                page[k..k + 4].copy_from_slice(&rgba[row * w + col]);
            }
        }
    }
    GlyphAtlas { w: width, h: height, rgba: page, glyphs, line_height, ascent, spacing, kern: HashMap::new() }
}

/// One glyph quad to draw: atlas rectangle and its top-left position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub glyph: Glyph,
    pub x: f32,
    pub y: f32,
}

impl GlyphAtlas {
    /// Width of one line (no line breaks) in pixels at scale 1.
    pub fn line_width(&self, line: &str) -> f32 {
        let mut w = 0.0f32;
        let mut prev: Option<char> = None;
        let mut any = false;
        for ch in line.chars() {
            let Some(g) = self.glyphs.get(&ch) else { continue };
            if let Some(p) = prev {
                w += self.kern.get(&(p, ch)).copied().unwrap_or(0.0) + self.spacing;
            }
            w += g.advance;
            prev = Some(ch);
            any = true;
        }
        if any {
            w
        } else {
            0.0
        }
    }

    /// Size of a (possibly multi-line) text at `scale`.
    pub fn measure(&self, text: &str, scale: f32) -> (f32, f32) {
        let lines: Vec<&str> = text.split('\n').collect();
        let w = lines.iter().map(|l| self.line_width(l)).fold(0.0, f32::max);
        (w * scale, self.line_height * lines.len() as f32 * scale)
    }

    /// Glyph quads of one line whose top-left corner is (0, 0), at scale 1.
    pub fn layout_line(&self, line: &str) -> Vec<Quad> {
        let mut out = vec![];
        let mut pen = 0.0f32;
        let mut prev: Option<char> = None;
        for ch in line.chars() {
            let Some(g) = self.glyphs.get(&ch) else { continue };
            if let Some(p) = prev {
                pen += self.kern.get(&(p, ch)).copied().unwrap_or(0.0) + self.spacing;
            }
            if g.w > 0 && g.h > 0 {
                out.push(Quad { glyph: *g, x: (pen + g.ox).round(), y: g.oy.round() });
            }
            pen += g.advance;
            prev = Some(ch);
        }
        out
    }
}

/// Text measuring without a window: baked fonts give exact sizes, the built-in font is
/// approximated (8×16 px per character at size 16).
pub fn headless_measure(fonts: &[FontAsset]) -> Box<dyn Fn(Option<u32>, &str, f32) -> (f32, f32)> {
    let atlases: Vec<(Option<Arc<GlyphAtlas>>, f32)> = fonts.iter().map(|f| (f.atlas.clone(), f.size)).collect();
    Box::new(move |font, text, scale| {
        let approx = |size: f32| {
            let lines = text.split('\n').count() as f32;
            let longest = text.split('\n').map(|l| l.chars().count()).max().unwrap_or(0) as f32;
            (longest * size / 2.0 * scale, lines * size * scale)
        };
        match font.and_then(|i| atlases.get(i as usize)) {
            Some((Some(a), _)) => a.measure(text, scale),
            Some((None, size)) => approx(*size),
            None => approx(16.0),
        }
    })
}

/// Breaks `text` into lines no wider than `max_w` (words longer than that are split).
pub fn wrap(text: &str, max_w: f32, width_of: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut out = vec![];
    for para in text.split('\n') {
        if max_w <= 0.0 {
            out.push(para.to_string());
            continue;
        }
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if width_of(&candidate) <= max_w || line.is_empty() && width_of(word) <= max_w {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            // a word too long for a whole line: split it by characters
            let mut piece = String::new();
            for ch in word.chars() {
                piece.push(ch);
                if width_of(&piece) > max_w && piece.chars().count() > 1 {
                    piece.pop();
                    out.push(std::mem::take(&mut piece));
                    piece.push(ch);
                }
            }
            line = piece;
        }
        out.push(line);
    }
    out
}

/// TTF/OTF files of the system font folders (for the editor's font picker).
pub fn system_fonts() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = vec!["/usr/share/fonts".into(), "/usr/local/share/fonts".into(), "/Library/Fonts".into(), "/System/Library/Fonts".into(), "C:\\Windows\\Fonts".into()];
    if let Some(h) = std::env::var_os("HOME") {
        let h = std::path::PathBuf::from(h);
        dirs.push(h.join(".local/share/fonts"));
        dirs.push(h.join(".fonts"));
        dirs.push(h.join("Library/Fonts"));
    }
    fn walk(d: &Path, out: &mut Vec<std::path::PathBuf>, depth: u32) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && depth < 5 {
                walk(&p, out, depth + 1);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ttf") || x.eq_ignore_ascii_case("otf")) {
                out.push(p);
            }
        }
    }
    let mut out = vec![];
    for d in dirs {
        walk(&d, &mut out, 0);
    }
    out.sort_by_key(|p| p.file_name().map(|f| f.to_string_lossy().to_lowercase()));
    out.dedup();
    out
}

/// TOML text of a font asset as the editor writes it.
pub fn write_toml(t: &FontToml) -> String {
    let q = |s: &str| toml::Value::String(s.to_string()).to_string();
    let mut s = String::new();
    if let Some(sp) = &t.sprite {
        s += &format!("sprite = {}\n", q(sp));
        if let Some(c) = &t.chars {
            s += &format!("chars = {}\n", q(c));
        }
        if let Some(p) = t.proportional {
            s += &format!("proportional = {p}\n");
        }
    } else if let Some(f) = &t.file {
        s += &format!("file = {}\n", q(f));
    }
    if t.sprite.is_none() {
        s += &format!("size = {}\n", modscript::fmt_num(t.size.unwrap_or(16.0) as f64));
        if t.bold {
            s += "bold = true\n";
        }
        if t.italic {
            s += "italic = true\n";
        }
        if t.antialias == Some(false) {
            s += "antialias = false\n";
        }
        if !t.ranges.is_empty() && t.ranges != DEFAULT_RANGES {
            let r: Vec<String> = t.ranges.iter().map(|r| format!("[{}, {}]", r[0], r[1])).collect();
            s += &format!("ranges = [{}]\n", r.join(", "));
        }
    }
    if t.spacing != 0.0 {
        s += &format!("spacing = {}\n", modscript::fmt_num(t.spacing as f64));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn any_system_font() -> Option<Vec<u8>> {
        system_fonts().into_iter().find_map(|p| {
            let d = std::fs::read(&p).ok()?;
            // a regular face, so bold is synthesized
            let f = fontdue::Font::from_bytes(d.as_slice(), fontdue::FontSettings::default()).ok()?;
            let face = f.name().unwrap_or("").to_lowercase();
            if !(f.has_glyph('A') && f.has_glyph('W') && f.has_glyph('ñ')) || ["bold", "italic", "oblique"].iter().any(|s| face.contains(s)) {
                return None;
            }
            Some(d)
        })
    }

    #[test]
    fn ttf_atlas_metrics_and_styles() {
        let Some(data) = any_system_font() else { return };
        let a = bake_ttf(&data, &DEFAULT_RANGES, 16.0, false, false, true, 0.0).unwrap();
        assert!(a.glyphs.contains_key(&'A') && a.glyphs.contains_key(&' '));
        assert!(!a.glyphs.contains_key(&'ñ'), "outside the range");
        let (w, h) = a.measure("WWW", 1.0);
        assert!(w > 20.0 && h >= 16.0, "{w} {h}");
        assert_eq!(a.measure("WWW\nW", 2.0).1, a.line_height * 4.0);
        assert_eq!(a.line_width("Wñ"), a.line_width("W"), "missing glyphs are skipped");
        // bigger, bold and spaced fonts are wider; no antialias means only 0/255 alpha
        let big = bake_ttf(&data, &DEFAULT_RANGES, 32.0, false, false, true, 0.0).unwrap();
        assert!(big.line_width("Hola") > a.line_width("Hola") * 1.6);
        let bold = bake_ttf(&data, &DEFAULT_RANGES, 16.0, true, false, true, 0.0).unwrap();
        assert!(bold.line_width("Hola") > a.line_width("Hola"));
        let spaced = bake_ttf(&data, &DEFAULT_RANGES, 16.0, false, false, true, 2.0).unwrap();
        assert_eq!(spaced.line_width("Hola"), a.line_width("Hola") + 6.0);
        let crisp = bake_ttf(&data, &DEFAULT_RANGES, 16.0, false, false, false, 0.0).unwrap();
        assert!(crisp.rgba.chunks(4).all(|p| p[3] == 0 || p[3] == 255));
        assert!(a.rgba.chunks(4).any(|p| p[3] > 0 && p[3] < 255));
        let latin = bake_ttf(&data, &[[32, 127], [160, 255]], 16.0, false, false, true, 0.0).unwrap();
        assert!(latin.glyphs.contains_key(&'ñ'));
        let quads = a.layout_line("Hi");
        assert_eq!(quads.len(), 2);
        assert!(quads[1].x > quads[0].x);
    }

    #[test]
    fn sprite_font_and_wrap() {
        // 3 frames of 4x5: "A" full, "B" two columns, " " empty
        let (fw, fh) = (4u32, 5u32);
        let mut rgba = vec![0u8; (fw * 3 * fh * 4) as usize];
        for y in 0..fh {
            for x in 0..fw {
                rgba[((y * fw * 3 + x) * 4 + 3) as usize] = 255;
            }
            for x in 1..3 {
                rgba[((y * fw * 3 + fw + x) * 4 + 3) as usize] = 255;
            }
        }
        let spr = Sprite { name: "s".into(), fw, fh, frames: vec![[0, 0, 4, 5], [4, 0, 4, 5], [8, 0, 4, 5]], sheet_w: 12, sheet_h: 5, rgba, origin: (0.0, 0.0), bbox: [0.0, 0.0, 4.0, 5.0], fps: 1.0 };
        let a = bake_sprite(&spr, "AB ", true, 0.0);
        assert_eq!(a.glyphs[&'A'].w, 4);
        assert_eq!(a.glyphs[&'B'].w, 2, "proportional glyphs are trimmed");
        assert_eq!(a.line_width("AB"), 5.0 + 3.0);
        let fixed = bake_sprite(&spr, "AB ", false, 1.0);
        assert_eq!(fixed.line_width("AB"), 4.0 + 1.0 + 4.0);
        assert_eq!(fixed.measure("A", 1.0).1, 5.0);
        let w = |s: &str| s.chars().count() as f32 * 8.0;
        assert_eq!(wrap("uno dos tres", 60.0, &w), ["uno dos", "tres"]);
        assert_eq!(wrap("abcdefghij", 32.0, &w), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("a\nb", 0.0, &w), ["a", "b"]);
    }

    #[test]
    fn toml_roundtrip() {
        let t = FontToml { file: Some("f.ttf".into()), size: Some(12.0), bold: true, antialias: Some(false), ranges: vec![[32, 127], [160, 255]], spacing: 1.0, ..Default::default() };
        let back: FontToml = toml::from_str(&write_toml(&t)).unwrap();
        assert_eq!(back, t);
        let s = FontToml { sprite: Some("spr".into()), chars: Some("0123".into()), proportional: Some(false), ..Default::default() };
        let back: FontToml = toml::from_str(&write_toml(&s)).unwrap();
        assert_eq!(back.sprite, s.sprite);
        assert_eq!(back.chars, s.chars);
    }
}

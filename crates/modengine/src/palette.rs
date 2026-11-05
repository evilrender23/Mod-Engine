//! Colour palettes: the built-in Pear36 default, the file formats Lospec offers
//! (JSON, .hex, GIMP .gpl, JASC .pal, PNG strips) and the stable character mapping
//! that pixel-art sprites use (`[palette]` in game.toml).
//!
//! Character mapping: colour `i` of a palette gets the `i`-th character of
//! [`PALETTE_CHARS`] (`0`-`9`, then `A`-`Z`, then `a`-`z`, then symbols), so Pear36
//! is `0`..`9` + `A`..`Z`. `'.'` and `' '` stay transparent and `'` is never used
//! (it would end a `'''` block).

use std::collections::{BTreeMap, BTreeSet};

/// Characters given to palette colours, in order (index `i` -> `i`-th char).
pub const PALETTE_CHARS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#@$%&*+=-~!?<>^/|:;";

/// Pear36 by PineappleOnPizza (https://lospec.com/palette-list/pear36).
pub const PEAR36: [u32; 36] = [
    0x5e315b, 0x8c3f5d, 0xba6156, 0xf2a65e, 0xffe478, 0xcfff70, 0x8fde5d, 0x3ca370, 0x3d6e70, 0x323e4f, 0x322947, 0x473b78, //
    0x4b5bab, 0x4da6ff, 0x66ffe3, 0xffffeb, 0xc2c2d1, 0x7e7e8f, 0x606070, 0x43434f, 0x272736, 0x3e2347, 0x57294b, 0x964253, //
    0xe36956, 0xffb570, 0xff9166, 0xeb564b, 0xb0305c, 0x73275c, 0x422445, 0x5a265e, 0x80366b, 0xbd4882, 0xff6b97, 0xffb5b5,
];

/// Most colours a palette may have (one character each).
pub fn max_colors() -> usize {
    PALETTE_CHARS.chars().count()
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Palette {
    pub name: String,
    pub author: String,
    /// `0xRRGGBB`, in the palette's order.
    pub colors: Vec<u32>,
}

impl Palette {
    pub fn pear36() -> Palette {
        Palette { name: "Pear36".into(), author: "PineappleOnPizza".into(), colors: PEAR36.to_vec() }
    }

    /// `"0" = "#5e315b"`, ... with the standard characters (extra colours beyond
    /// [`max_colors`] are dropped).
    pub fn char_map(&self) -> BTreeMap<String, String> {
        self.colors.iter().zip(PALETTE_CHARS.chars()).map(|(c, ch)| (ch.to_string(), hex(*c))).collect()
    }

    /// Index of the colour closest to `c` (perceptually weighted RGB distance).
    pub fn nearest(&self, c: u32) -> Option<usize> {
        (0..self.colors.len()).min_by_key(|&i| distance(self.colors[i], c))
    }

    /// Our own JSON (same shape as Lospec's: colours without `#`).
    pub fn to_json(&self) -> String {
        let colors: Vec<String> = self.colors.iter().map(|c| format!("{c:06x}")).collect();
        serde_json::json!({ "name": self.name, "author": self.author, "colors": colors }).to_string()
    }

    /// Parses any supported format; `file_name` picks it by extension and is the
    /// fallback palette name.
    pub fn parse_any(file_name: &str, bytes: &[u8]) -> Result<Palette, String> {
        let lower = file_name.to_ascii_lowercase();
        let stem = std::path::Path::new(file_name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let stem = stem.trim_end_matches("-1x").to_string();
        if lower.ends_with(".png") || bytes.starts_with(b"\x89PNG") {
            return parse_png(bytes, &stem);
        }
        let text = String::from_utf8_lossy(bytes);
        let t = text.trim_start_matches('\u{feff}').trim_start();
        let mut p = if t.starts_with('{') {
            parse_json(t)?
        } else if t.starts_with("GIMP Palette") {
            parse_gpl(t)?
        } else if t.starts_with("JASC-PAL") {
            parse_pal(t)?
        } else {
            parse_hex_list(t)?
        };
        if p.name.is_empty() {
            p.name = stem;
        }
        Ok(p)
    }
}

pub fn hex(c: u32) -> String {
    format!("#{c:06x}")
}

/// Squared "redmean" distance: cheap and much closer to perception than plain RGB.
pub fn distance(a: u32, b: u32) -> u64 {
    let ch = |c: u32, s: u32| ((c >> s) & 255) as i64;
    let (r1, g1, b1, r2, g2, b2) = (ch(a, 16), ch(a, 8), ch(a, 0), ch(b, 16), ch(b, 8), ch(b, 0));
    let rm = (r1 + r2) / 2;
    let (dr, dg, db) = (r1 - r2, g1 - g2, b1 - b2);
    (((512 + rm) * dr * dr >> 8) + 4 * dg * dg + ((767 - rm) * db * db >> 8)) as u64
}

fn hex6(s: &str) -> Option<u32> {
    let t = s.trim().trim_start_matches('#');
    let t = if t.len() == 8 && t.to_ascii_lowercase().starts_with("ff") { &t[2..] } else { t }; // AARRGGBB
    (t.len() == 6 && t.chars().all(|c| c.is_ascii_hexdigit())).then(|| u32::from_str_radix(t, 16).ok()).flatten()
}

fn check(p: Palette) -> Result<Palette, String> {
    if p.colors.is_empty() {
        return Err("la paleta no tiene colores".into());
    }
    Ok(p)
}

/// Lospec's `<slug>.json`: `{"name": .., "author": .., "colors": ["5e315b", ..]}`.
pub fn parse_json(text: &str) -> Result<Palette, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("JSON no válido: {e}"))?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let arr = v.get("colors").and_then(|c| c.as_array()).ok_or("falta la lista \"colors\"")?;
    let mut colors = vec![];
    for c in arr {
        let s = c.as_str().ok_or("los colores deben ser texto")?;
        colors.push(hex6(s).ok_or_else(|| format!("color no válido: {s}"))?);
    }
    check(Palette { name: s("name"), author: s("author"), colors })
}

/// `.hex`: one `rrggbb` per line.
pub fn parse_hex_list(text: &str) -> Result<Palette, String> {
    let mut colors = vec![];
    for l in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(';') && !l.starts_with("//")) {
        colors.push(hex6(l).ok_or_else(|| format!("línea no válida en .hex: {l}"))?);
    }
    check(Palette { colors, ..Default::default() })
}

fn rgb3(l: &str) -> Option<u32> {
    let mut it = l.split_whitespace().map(|n| n.parse::<u32>().ok().filter(|v| *v <= 255));
    let (r, g, b) = (it.next()??, it.next()??, it.next()??);
    Some(r << 16 | g << 8 | b)
}

/// GIMP `.gpl`: header, `Name:`/`Columns:`/`#` lines, then `R G B [name]`.
pub fn parse_gpl(text: &str) -> Result<Palette, String> {
    let mut p = Palette::default();
    for l in text.lines().skip(1).map(str::trim) {
        if let Some(n) = l.strip_prefix("Name:") {
            p.name = n.trim().to_string();
        } else if let Some(a) = l.strip_prefix("#").and_then(|c| c.trim().strip_prefix("Author:")) {
            p.author = a.trim().to_string();
        } else if l.is_empty() || l.starts_with('#') || l.starts_with("Columns:") {
        } else {
            p.colors.push(rgb3(l).ok_or_else(|| format!("línea no válida en .gpl: {l}"))?);
        }
    }
    check(p)
}

/// JASC `.pal`: `JASC-PAL`, `0100`, count, then `R G B` lines.
pub fn parse_pal(text: &str) -> Result<Palette, String> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() < 3 {
        return Err("fichero .pal incompleto".into());
    }
    let n: usize = lines[2].parse().map_err(|_| format!("número de colores no válido: {}", lines[2]))?;
    let mut colors = vec![];
    for l in lines.iter().skip(3).take(n) {
        colors.push(rgb3(l).ok_or_else(|| format!("línea no válida en .pal: {l}"))?);
    }
    if colors.len() != n {
        return Err(format!("el .pal anuncia {n} colores pero tiene {}", colors.len()));
    }
    check(Palette { colors, ..Default::default() })
}

/// A PNG strip/swatch image: every distinct opaque colour in reading order.
pub fn parse_png(bytes: &[u8], name: &str) -> Result<Palette, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("PNG no válido: {e}"))?.to_rgba8();
    let mut seen = BTreeSet::new();
    let mut colors = vec![];
    for px in img.pixels() {
        let [r, g, b, a] = px.0;
        let c = (r as u32) << 16 | (g as u32) << 8 | b as u32;
        if a >= 128 && seen.insert(c) {
            colors.push(c);
            if colors.len() > 256 {
                return Err("la imagen tiene más de 256 colores: no parece una paleta".into());
            }
        }
    }
    check(Palette { name: name.into(), colors, ..Default::default() })
}

/// `pear36`, `https://lospec.com/palette-list/pear36`, `.../pear36.json` -> `pear36`.
pub fn lospec_slug(input: &str) -> Option<String> {
    let t = input.trim().trim_end_matches('/');
    let t = t.split(['?', '#']).next().unwrap_or(t);
    let last = t.rsplit('/').next().unwrap_or(t);
    let last = [".json", ".hex", ".gpl", ".pal", "-1x.png", "-8x.png", "-32x.png", ".png"].iter().fold(last, |s, e| s.strip_suffix(e).unwrap_or(s));
    let slug = last.to_ascii_lowercase();
    (!slug.is_empty() && slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then_some(slug)
}

pub fn lospec_json_url(slug: &str) -> String {
    format!("https://lospec.com/palette-list/{slug}.json")
}

/// Adds `pal` to an existing `[palette]` without touching its entries: colours already
/// present keep their character; the others get their standard character when it is
/// free, else the first free one. Returns the new map and how many colours didn't fit.
pub fn merge_chars(existing: &BTreeMap<String, String>, pal: &Palette) -> (BTreeMap<String, String>, usize) {
    let mut out = existing.clone();
    let mut by_color: BTreeMap<u32, String> = BTreeMap::new();
    for (k, v) in existing {
        if let Some(c) = crate::draw::parse_color(v) {
            by_color.entry(c).or_insert(k.clone());
        }
    }
    // first every colour whose standard character is free, then the rest on characters
    // no colour of this palette wants (so one clash doesn't shift all the others)
    let mut left = vec![];
    for (i, (c, ch)) in pal.colors.iter().zip(PALETTE_CHARS.chars().map(Some).chain(std::iter::repeat(None))).enumerate() {
        if by_color.contains_key(c) {
            continue;
        }
        match ch.map(|c| c.to_string()).filter(|k| !out.contains_key(k)) {
            Some(k) => {
                out.insert(k.clone(), hex(*c));
                by_color.insert(*c, k);
            }
            None => left.push(i),
        }
    }
    let mut skipped = 0;
    let wanted = pal.colors.len();
    for i in left {
        let c = pal.colors[i];
        if by_color.contains_key(&c) {
            continue;
        }
        let free = |skip: usize| PALETTE_CHARS.chars().skip(skip).map(|c| c.to_string()).find(|k| !out.contains_key(k));
        match free(wanted).or_else(|| free(0)) {
            Some(k) => {
                out.insert(k.clone(), hex(c));
                by_color.insert(c, k);
            }
            None => skipped += 1,
        }
    }
    (out, skipped)
}

/// For replacing a project palette: every character in `old` (global + sprite-local
/// colours) -> the character of the nearest colour of `pal` in its standard mapping.
pub fn remap_table(old: &BTreeMap<char, u32>, pal: &Palette) -> BTreeMap<char, char> {
    let chars: Vec<char> = PALETTE_CHARS.chars().collect();
    old.iter().filter_map(|(k, c)| pal.nearest(*c).and_then(|i| chars.get(i)).map(|n| (*k, *n))).collect()
}

/// Applies a character table to pixel rows (`.`/space and unknown chars are kept).
pub fn remap_rows(frame: &str, table: &BTreeMap<char, char>) -> String {
    frame.chars().map(|c| table.get(&c).copied().unwrap_or(c)).collect()
}

/// The palette's colours in index order: `[palette]` entries sorted by character
/// (for the standard mapping that is the original palette order).
pub fn ordered(map: &BTreeMap<String, String>) -> Vec<(char, u32)> {
    let mut v: Vec<(char, u32)> = map.iter().filter_map(|(k, c)| Some((k.chars().next()?, crate::draw::parse_color(c)?))).collect();
    let rank = |ch: char| PALETTE_CHARS.find(ch).unwrap_or(usize::MAX);
    v.sort_by_key(|(ch, _)| (rank(*ch), *ch));
    v
}

/// `[palette]` lines for a new game.toml with this palette (with a credit comment).
pub fn game_toml_section(pal: &Palette) -> String {
    let mut s = format!("# Paleta: {} de {} (lospec.com). '.' es transparente.\n[palette]\n", pal.name, pal.author);
    for (k, v) in ordered(&pal.char_map()) {
        s += &format!("\"{k}\" = \"{}\"\n", hex(v));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pear36_is_complete_and_mapped_0_to_z() {
        let p = Palette::pear36();
        assert_eq!(p.colors.len(), 36);
        let m = p.char_map();
        assert_eq!(m.len(), 36);
        assert_eq!(m["0"], "#5e315b");
        assert_eq!(m["9"], "#323e4f");
        assert_eq!(m["A"], "#322947");
        assert_eq!(m["F"], "#ffffeb");
        assert_eq!(m["Z"], "#ffb5b5");
        assert!(!PALETTE_CHARS.contains(['.', ' ', '\'', '"', '\\']));
        let uniq: BTreeSet<char> = PALETTE_CHARS.chars().collect();
        assert_eq!(uniq.len(), max_colors(), "characters are distinct");
        // the order is stable: sorted keys give back the palette order
        assert_eq!(ordered(&m).iter().map(|(_, c)| *c).collect::<Vec<_>>(), PEAR36.to_vec());
    }

    #[test]
    fn formats() {
        let j = parse_any("x.json", br##"{"name":"Tiny","author":"Me","colors":["ff0000","00FF00","#0000ff"]}"##).unwrap();
        assert_eq!((j.name.as_str(), j.author.as_str(), j.colors.clone()), ("Tiny", "Me", vec![0xff0000, 0x00ff00, 0x0000ff]));
        let h = parse_any("tiny.hex", b"ff0000\r\n00ff00\n\n0000ff\n").unwrap();
        assert_eq!((h.name.as_str(), h.colors.clone()), ("tiny", vec![0xff0000, 0x00ff00, 0x0000ff]));
        let g = parse_any("t.gpl", b"GIMP Palette\nName: Tiny GPL\nColumns: 3\n#\n# Author: Bob\n255   0   0\tRed\n  0 255   0\tGreen\n0 0 255 Index 3\n").unwrap();
        assert_eq!((g.name.as_str(), g.author.as_str(), g.colors.clone()), ("Tiny GPL", "Bob", vec![0xff0000, 0x00ff00, 0x0000ff]));
        let p = parse_any("t.pal", b"JASC-PAL\r\n0100\r\n3\r\n255 0 0\r\n0 255 0\r\n0 0 255\r\n").unwrap();
        assert_eq!((p.name.as_str(), p.colors.clone()), ("t", vec![0xff0000, 0x00ff00, 0x0000ff]));
        assert!(parse_any("bad.pal", b"JASC-PAL\n0100\n4\n1 2 3\n").is_err());
        assert!(parse_any("bad.hex", b"zzzzzz\n").is_err());
        assert!(parse_any("empty.json", br#"{"colors":[]}"#).is_err());
        // PNG strip (what Lospec's -1x.png is): duplicates and transparent pixels ignored
        let mut img = image::RgbaImage::new(4, 2);
        for (i, c) in [[255, 0, 0, 255], [0, 255, 0, 255], [255, 0, 0, 255], [0, 0, 0, 0]].iter().enumerate() {
            img.put_pixel(i as u32, 0, image::Rgba(*c));
            img.put_pixel(i as u32, 1, image::Rgba([0, 0, 255, 255]));
        }
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let s = parse_any("strip-1x.png", &png).unwrap();
        assert_eq!((s.name.as_str(), s.colors.clone()), ("strip", vec![0xff0000, 0x00ff00, 0x0000ff]));
        // round trip through our JSON
        assert_eq!(parse_any("a.json", Palette::pear36().to_json().as_bytes()).unwrap(), Palette::pear36());
    }

    fn parse_any(n: &str, b: &[u8]) -> Result<Palette, String> {
        Palette::parse_any(n, b)
    }

    #[test]
    fn slugs() {
        for (i, o) in [
            ("pear36", Some("pear36")),
            ("https://lospec.com/palette-list/pear36", Some("pear36")),
            ("https://lospec.com/palette-list/endesga-32/", Some("endesga-32")),
            ("lospec.com/palette-list/Pear36.json", Some("pear36")),
            ("https://lospec.com/palette-list/pear36-1x.png?x=1", Some("pear36")),
            ("  sweetie-16 ", Some("sweetie-16")),
            ("", None),
            ("bad slug!", None),
        ] {
            assert_eq!(lospec_slug(i).as_deref(), o, "{i}");
        }
        assert_eq!(lospec_json_url("pear36"), "https://lospec.com/palette-list/pear36.json");
    }

    #[test]
    fn merge_keeps_existing_chars() {
        let mut old = BTreeMap::new();
        old.insert("k".to_string(), "#1a1c2c".to_string());
        old.insert("0".to_string(), "#ff00ff".to_string()); // clashes with Pear36 '0'
        old.insert("w".to_string(), "#ffffeb".to_string()); // same colour as Pear36 'F'
        let (m, skipped) = merge_chars(&old, &Palette::pear36());
        assert_eq!(skipped, 0);
        assert_eq!(m["k"], "#1a1c2c");
        assert_eq!(m["0"], "#ff00ff", "existing entries are untouched");
        assert_eq!(m["w"], "#ffffeb");
        assert!(!m.contains_key("F"), "a colour already present is not duplicated");
        assert_eq!(m["1"], "#8c3f5d", "free standard chars are used");
        assert_eq!(m["a"], "#5e315b", "the clashing colour got the first char after Pear36's");
        assert_eq!(m["Z"], "#ffb5b5", "and nothing else moved");
        assert_eq!(m.len(), 3 + 35);
        // merging twice changes nothing
        assert_eq!(merge_chars(&m, &Palette::pear36()).0, m);
    }

    #[test]
    fn nearest_remap() {
        let p = Palette::pear36();
        assert_eq!(p.nearest(0xffffff), Some(15)); // ffffeb
        assert_eq!(p.nearest(0x000000), Some(20)); // 272736
        assert_eq!(p.nearest(0xff0000), Some(27)); // eb564b
        let old: BTreeMap<char, u32> = [('k', 0x1a1c2c), ('w', 0xf4f4f4), ('y', 0xffcd75)].into();
        let t = remap_table(&old, &p);
        assert_eq!(t[&'k'], 'K');
        assert_eq!(t[&'w'], 'F');
        assert_eq!(t[&'y'], '4'); // ffe478
        assert_eq!(remap_rows("..kw\nyy..", &t), "..KF\n44..");
    }

    #[test]
    fn toml_section_loads() {
        let s = format!("[game]\nname = \"t\"\n\n{}", game_toml_section(&Palette::pear36()));
        let g: crate::project::GameToml = toml::from_str(&s).unwrap();
        assert_eq!(g.palette, Palette::pear36().char_map());
    }
}

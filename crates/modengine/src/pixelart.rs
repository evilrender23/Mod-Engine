//! Compact pixel art for sprite TOML files, and baking big text sprites into PNGs.
//!
//! `pixels_rle` is the run-length form of `pixels` (same palette characters):
//!
//! ```toml
//! pixels_rle = ['''
//! .*4R*8.*4
//! .*2R*12.*2 *3
//! ''']
//! ```
//!
//! * `C*N` is N pixels of character `C` (`R*8` = `RRRRRRRR`); a character alone is one
//!   pixel, so plain rows are valid RLE too.
//! * A row ending in a space and `*N` repeats N times (` *3` = this row and two copies).
//! * `.` is transparent. Spaces are not pixels: they only separate a run from a digit
//!   colour after it (`R*8 3` = 8 × `R` and one `3`) and the row from its repeat.
//! * The `*` character itself is written `**N` (always with a count).
//!
//! `modengine sprite-bake` turns text sprites into `name.png` + a short `name.toml` (or into
//! `pixels_rle` with `--rle`), keeping origin, mask, fps and the other settings.

use crate::draw::parse_color;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Rows of one frame of `pixels_rle`.
pub fn decode_rle(frame: &str) -> Result<Vec<Vec<char>>, String> {
    let mut rows = Vec::new();
    for (ln, line) in frame.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // optional row repeat: "<runs> *N"
        let (body, times) = match line.rsplit_once(char::is_whitespace) {
            Some((b, rep)) if rep.starts_with('*') && rep.len() > 1 && rep[1..].bytes().all(|c| c.is_ascii_digit()) => {
                (b.trim_end(), rep[1..].parse::<usize>().map_err(|_| format!("row {}: bad repeat '{rep}'", ln + 1))?)
            }
            _ => (line, 1),
        };
        let row = decode_row(body).map_err(|e| format!("row {}: {e}", ln + 1))?;
        if times == 0 || times > 100_000 {
            return Err(format!("row {}: repeat must be 1..100000", ln + 1));
        }
        for _ in 0..times {
            rows.push(row.clone());
        }
    }
    Ok(rows)
}

fn decode_row(s: &str) -> Result<Vec<char>, String> {
    let cs: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        i += 1;
        if c.is_whitespace() {
            continue;
        }
        // "C*digits" is a run
        if i + 1 < cs.len() && cs[i] == '*' && cs[i + 1].is_ascii_digit() {
            let mut j = i + 1;
            while j < cs.len() && cs[j].is_ascii_digit() {
                j += 1;
            }
            let n: usize = cs[i + 1..j].iter().collect::<String>().parse().map_err(|_| "run too long".to_string())?;
            if n > 1_000_000 {
                return Err(format!("run of {n} pixels is too long"));
            }
            out.extend(std::iter::repeat_n(c, n));
            i = j;
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

/// Run-length text of one frame (inverse of [`decode_rle`]).
pub fn encode_rle(rows: &[Vec<char>]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let mut n = 1;
        while i + n < rows.len() && rows[i + n] == rows[i] {
            n += 1;
        }
        let mut line = encode_row(&rows[i]);
        if line.is_empty() {
            line.push('.');
        }
        if n > 1 {
            line += &format!(" *{n}");
        }
        lines.push(line);
        i += n;
    }
    lines.join("\n")
}

fn encode_row(row: &[char]) -> String {
    let mut s = String::new();
    let mut i = 0;
    while i < row.len() {
        let c = row[i];
        let mut n = 1;
        while i + n < row.len() && row[i + n] == c {
            n += 1;
        }
        // a run costs 2 + digits characters; '*' always carries a count so it is never
        // mistaken for the start of one
        if c == '*' || n >= 4 {
            s += &format!("{c}*{n}");
            // a digit colour right after the count would read as more digits
            if row.get(i + n).is_some_and(|d| d.is_ascii_digit()) {
                s.push(' ');
            }
        } else {
            s.extend(std::iter::repeat_n(c, n));
        }
        i += n;
    }
    s
}

/// The frames of a sprite TOML's `pixels` / `pixels_rle` as rows of characters.
pub fn frame_grids(pixels: &[String], pixels_rle: &[String]) -> Result<Vec<Vec<Vec<char>>>, String> {
    if !pixels.is_empty() && !pixels_rle.is_empty() {
        return Err("use `pixels` or `pixels_rle`, not both".into());
    }
    if !pixels_rle.is_empty() {
        return pixels_rle.iter().enumerate().map(|(i, f)| decode_rle(f).map_err(|e| format!("pixels_rle frame {i}: {e}"))).collect();
    }
    Ok(pixels.iter().map(|f| f.lines().map(|l| l.trim().chars().collect::<Vec<_>>()).filter(|l| !l.is_empty()).collect()).collect())
}

/// Palette of a text sprite: the game's `[palette]` overridden by the sprite's own.
pub fn palette(global: &BTreeMap<String, String>, local: &BTreeMap<String, String>) -> Result<HashMap<char, [u8; 4]>, String> {
    let mut pal = HashMap::new();
    for (k, v) in global.iter().chain(local.iter()) {
        let ch = k.chars().next().ok_or("empty palette key")?;
        let c = parse_color(v).ok_or_else(|| format!("bad palette colour '{v}'"))?;
        pal.insert(ch, [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
    }
    Ok(pal)
}

/// A horizontal strip of the frames: (frame w, frame h, frame count, RGBA of the strip).
pub fn rasterize(grids: &[Vec<Vec<char>>], pal: &HashMap<char, [u8; 4]>) -> Result<(u32, u32, u32, Vec<u8>), String> {
    let fw = grids.iter().flat_map(|g| g.iter().map(|r| r.len())).max().unwrap_or(0) as u32;
    let fh = grids.iter().map(|g| g.len()).max().unwrap_or(0) as u32;
    if fw == 0 || fh == 0 {
        return Err("pixel art is empty".into());
    }
    let n = grids.len() as u32;
    let sheet_w = fw * n;
    let mut rgba = vec![0u8; (sheet_w * fh * 4) as usize];
    for (fi, g) in grids.iter().enumerate() {
        let ox = fi as u32 * fw;
        for (y, row) in g.iter().enumerate() {
            for (x, ch) in row.iter().enumerate() {
                if *ch == '.' || *ch == ' ' {
                    continue;
                }
                let px = pal.get(ch).ok_or_else(|| format!("character '{ch}' is not in the palette"))?;
                let i = (((y as u32) * sheet_w + ox + x as u32) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(px);
            }
        }
    }
    Ok((fw, fh, n, rgba))
}

// ------------------------------------------------------------------ baking

/// The `[palette]` of a project's game.toml.
pub fn project_palette(project: &Path) -> Result<BTreeMap<String, String>, String> {
    let game = project.join("game.toml");
    let text = std::fs::read_to_string(&game).map_err(|e| format!("{}: {e}", game.display()))?;
    let g: crate::project::GameToml = toml::from_str(&text).map_err(|e| format!("{}: {e}", game.display()))?;
    Ok(g.palette)
}

/// PNG of an RGBA image, compressed hard (pixel art has long runs of one colour).
pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::ImageEncoder;
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Best, FilterType::Adaptive).write_image(rgba, w, h, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    Ok(png)
}

/// What `sprite-bake` does with a text sprite.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BakeTo {
    /// `name.png` + `image = "name.png"` in the TOML.
    Png,
    /// `pixels_rle` in the TOML.
    Rle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Baked {
    pub name: String,
    pub toml: PathBuf,
    pub png: Option<PathBuf>,
    /// Bytes before (TOML) and after (TOML + PNG).
    pub before: u64,
    pub after: u64,
}

/// Converts the text sprite `sprites/<name>.toml` (`pixels` or `pixels_rle`). Every other
/// key (origin, mask, fps, rotation...) is kept; `dry_run` only measures.
pub fn bake_sprite(toml_path: &Path, global: &BTreeMap<String, String>, to: BakeTo, dry_run: bool) -> Result<Option<Baked>, String> {
    let where_ = toml_path.display().to_string();
    let text = std::fs::read_to_string(toml_path).map_err(|e| format!("{where_}: {e}"))?;
    let mut t: toml::Table = toml::from_str(&text).map_err(|e| format!("{where_}: {e}"))?;
    let strings = |v: Option<&toml::Value>| -> Vec<String> { v.and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default() };
    let (pixels, rle) = (strings(t.get("pixels")), strings(t.get("pixels_rle")));
    if pixels.is_empty() && rle.is_empty() {
        return Ok(None);
    }
    let grids = frame_grids(&pixels, &rle).map_err(|e| format!("{where_}: {e}"))?;
    let local: BTreeMap<String, String> = t.get("palette").and_then(|v| v.as_table()).map(|p| p.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()).unwrap_or_default();
    let name = toml_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let before = text.len() as u64;
    let (new_text, png) = match to {
        BakeTo::Png => {
            let pal = palette(global, &local).map_err(|e| format!("{where_}: {e}"))?;
            let (fw, fh, n, rgba) = rasterize(&grids, &pal).map_err(|e| format!("{where_}: {e}"))?;
            let png = encode_png(fw * n, fh, &rgba)?;
            t.remove("pixels");
            t.remove("pixels_rle");
            t.remove("palette");
            let file = format!("{name}.png");
            let mut out = toml::Table::new();
            out.insert("image".into(), toml::Value::String(file.clone()));
            if n > 1 {
                out.insert("frames".into(), toml::Value::Integer(n as i64));
            }
            // rows that were shorter than the widest one are padded: the frame size is kept
            for (k, v) in t {
                out.insert(k, v);
            }
            (toml::to_string(&out).map_err(|e| e.to_string())?, Some((toml_path.with_file_name(file), png)))
        }
        BakeTo::Rle => {
            let w = grids.iter().flat_map(|g| g.iter().map(|r| r.len())).max().unwrap_or(0);
            let frames: Vec<String> = grids
                .iter()
                .map(|g| {
                    // pad short rows so the frame keeps its size
                    let rows: Vec<Vec<char>> = g.iter().map(|r| r.iter().map(|c| if *c == ' ' { '.' } else { *c }).chain(std::iter::repeat('.')).take(w).collect()).collect();
                    format!("'''\n{}\n'''", encode_rle(&rows))
                })
                .collect();
            t.remove("pixels");
            t.remove("pixels_rle");
            // plain keys, then the pixels, then tables like [palette] (keys written after a
            // table header would belong to it)
            let (tables, plain): (toml::Table, toml::Table) = t.into_iter().partition(|(_, v)| v.is_table());
            let mut s = toml::to_string(&plain).map_err(|e| e.to_string())?;
            s += &format!("pixels_rle = [{}]\n", frames.join(", "));
            if !tables.is_empty() {
                s += "\n";
                s += &toml::to_string(&tables).map_err(|e| e.to_string())?;
            }
            (s, None)
        }
    };
    let after = new_text.len() as u64 + png.as_ref().map(|p| p.1.len() as u64).unwrap_or(0);
    if !dry_run {
        if let Some((p, bytes)) = &png {
            std::fs::write(p, bytes).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        std::fs::write(toml_path, &new_text).map_err(|e| format!("{where_}: {e}"))?;
    }
    Ok(Some(Baked { name, toml: toml_path.to_path_buf(), png: png.map(|p| p.0), before, after }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(s: &[&str]) -> Vec<Vec<char>> {
        s.iter().map(|r| r.chars().collect()).collect()
    }

    #[test]
    fn rle_roundtrip_and_digits_as_colours() {
        let g = rows(&["000VVVVVVUUU", "000VVVVVVUUU", "000VVVVVVUUU", "*.**3*****x", "....1111222", "5", "VVVVVV3333V*"]);
        let enc = encode_rle(&g);
        assert!(enc.starts_with("000V*6UUU *3\n"), "{enc}");
        assert_eq!(decode_rle(&enc).unwrap(), g);
        // hand-written: digits are palette characters, counts follow '*'
        assert_eq!(decode_rle("0*3 1\n12*2 *2").unwrap(), rows(&["0001", "122", "122"]));
        assert_eq!(decode_rle("**2V").unwrap(), rows(&["**V"]));
        assert_eq!(decode_rle("a b").unwrap(), rows(&["ab"]), "spaces are not pixels");
        assert_eq!(decode_rle("R*3 3*2").unwrap(), rows(&["RRR33"]));
        assert!(enc.contains("V*6 3*4"), "{enc}");
    }

    #[test]
    fn rle_is_much_smaller_on_flat_art() {
        let g: Vec<Vec<char>> = (0..48).map(|y| (0..96).map(|x| if (x / 24 + y / 24) % 2 == 0 { 'A' } else { 'B' }).collect()).collect();
        let plain: usize = g.iter().map(|r| r.len() + 1).sum();
        let enc = encode_rle(&g);
        assert!(enc.len() * 20 < plain, "{} vs {plain}", enc.len());
        assert_eq!(decode_rle(&enc).unwrap(), g);
    }

    #[test]
    fn bake_keeps_settings_and_pixels() {
        let dir = std::env::temp_dir().join(format!("modengine_bake_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = "origin = \"center\"\nfps = 4\nmask = [1, 1, 2, 2]\nrotation = \"rotsprite\"\npalette = { r = \"#ff0000\" }\npixels = ['''\n.rr.\nr00r\n''', '''\nrrrr\n.00.\n''']\n";
        let global: BTreeMap<String, String> = [("0".to_string(), "#0000ff".to_string())].into();
        let p = dir.join("s.toml");
        std::fs::write(&p, src).unwrap();
        let before = crate::project::load_sprite("s", &p, true, &global).unwrap();
        // RLE first, then PNG: both load to the same sprite
        let r = bake_sprite(&p, &global, BakeTo::Rle, false).unwrap().unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("pixels_rle"));
        assert!(r.png.is_none());
        let mid = crate::project::load_sprite("s", &p, true, &global).unwrap();
        let b = bake_sprite(&p, &global, BakeTo::Png, false).unwrap().unwrap();
        assert!(b.png.as_ref().unwrap().is_file());
        let after = crate::project::load_sprite("s", &p, true, &global).unwrap();
        for s in [&mid, &after] {
            assert_eq!((s.fw, s.fh, s.frames.len()), (before.fw, before.fh, before.frames.len()));
            assert_eq!((s.origin, s.bbox, s.fps), (before.origin, before.bbox, before.fps));
            assert!(s.rgba == before.rgba, "pixels changed");
            assert_eq!(s.rotation, before.rotation);
        }
        // a PNG sprite is not a text sprite any more
        assert_eq!(bake_sprite(&p, &global, BakeTo::Png, true).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

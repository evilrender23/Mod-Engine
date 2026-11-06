//! `modengine pixel-check`: the verification steps of docs/pixel-art.md as code.
//!
//! For text sprites (`pixels` / `pixels_rle`) the TOML is parsed here, not through the
//! loader, so broken art (rows of the wrong width, characters missing from the palette)
//! is reported with its position instead of a single load error. PNG sprites go through
//! the loader and get the colour, alpha, frame and anchor checks.
//!
//! Positions are 0-based: `frame 0, fila 3, col 5` is the pixel x=5, y=3 of the first
//! frame. Previews ([`write_previews`]) are PNGs scaled with nearest neighbour: every
//! frame side by side, over light and dark backgrounds, the loop and onion skin of an
//! animation, and a 3×3 repetition for tiles.

use crate::palette::Palette;
use crate::project::{asset_files, load_sprite, parse_origin};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Result of one check, from best to worst.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Status {
    Ok,
    Info,
    /// Not run (no palette to compare with, not an animation...).
    Skipped,
    Warn,
    Error,
}

impl Status {
    pub fn key(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Info => "info",
            Status::Skipped => "omitida",
            Status::Warn => "aviso",
            Status::Error => "error",
        }
    }
}

/// A pixel or row: frame, row (y) and column (x), all 0-based.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct At {
    pub frame: usize,
    pub row: usize,
    pub col: usize,
}

impl std::fmt::Display for At {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frame {}, fila {}, col {}", self.frame, self.row, self.col)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    /// Stable id for scripts and agents (`row_width`, `symbols`...).
    pub id: &'static str,
    /// Spanish name shown in the text output.
    pub title: &'static str,
    pub status: Status,
    pub message: String,
    /// Some of the places involved (at most [`MAX_AT`]).
    pub at: Vec<At>,
    /// How many places in total.
    pub count: usize,
}

/// Positions listed per finding.
pub const MAX_AT: usize = 12;

fn check(id: &'static str, title: &'static str, status: Status, message: impl Into<String>) -> Check {
    Check { id, title, status, message: message.into(), at: vec![], count: 0 }
}

/// Which colours count as allowed for the closed-palette check.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum PaletteChoice {
    /// game.toml's `[palette]`, when it has one (deviations are warnings: a sprite may add
    /// local colours).
    #[default]
    Project,
    /// Skip the check.
    Off,
    /// A palette given explicitly (deviations are errors).
    Colors { name: String, colors: Vec<u32> },
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub palette: PaletteChoice,
    /// PNG sprites: partially transparent pixels are errors instead of warnings.
    pub binary_alpha: bool,
    /// The frames are repeatable tiles: check the seams and write the 3×3 preview.
    pub tile: bool,
    /// The exact cell size asked for (`--size 16x16`).
    pub size: Option<(u32, u32)>,
}

/// One sprite's frames, all of the same cell size.
pub struct Art {
    pub fw: u32,
    pub fh: u32,
    /// RGBA of each frame (`fw * fh * 4`).
    pub frames: Vec<Vec<u8>>,
    pub origin: (f64, f64),
    /// `origin = "bottom"` or an origin on the last row: things standing on the ground.
    pub feet_origin: bool,
}

impl Art {
    fn px(&self, f: usize, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.fw + x) * 4) as usize;
        let p = &self.frames[f][i..i + 4];
        [p[0], p[1], p[2], p[3]]
    }
}

pub struct Report {
    pub name: String,
    pub source: PathBuf,
    /// "texto" (pixels / pixels_rle) or "png".
    pub kind: &'static str,
    pub art: Option<Art>,
    pub checks: Vec<Check>,
    /// Opaque colours used, most used first: (0xRRGGBB, pixels).
    pub colors: Vec<(u32, usize)>,
    pub previews: Vec<PathBuf>,
    /// What only a person (or the agent looking at the previews) can judge.
    pub pending: Vec<String>,
}

impl Report {
    pub fn count(&self, s: Status) -> usize {
        self.checks.iter().filter(|c| c.status == s).count()
    }
    pub fn errors(&self) -> usize {
        self.count(Status::Error)
    }
    pub fn warnings(&self) -> usize {
        self.count(Status::Warn)
    }
    pub fn get(&self, id: &str) -> Option<&Check> {
        self.checks.iter().find(|c| c.id == id)
    }

    pub fn to_json(&self) -> Value {
        let checks: Vec<Value> = self
            .checks
            .iter()
            .map(|c| {
                let mut v = json!({ "id": c.id, "title": c.title, "status": c.status.key(), "message": c.message });
                if !c.at.is_empty() {
                    v["at"] = json!(c.at.iter().map(|a| json!({ "frame": a.frame, "row": a.row, "col": a.col })).collect::<Vec<_>>());
                    v["count"] = json!(c.count);
                }
                v
            })
            .collect();
        let mut v = json!({
            "name": self.name,
            "source": self.source.display().to_string(),
            "kind": self.kind,
            "ok": self.errors() == 0,
            "errors": self.errors(),
            "warnings": self.warnings(),
            "checks": checks,
            "colors": self.colors.iter().map(|(c, n)| json!({ "color": hex(*c), "pixels": n })).collect::<Vec<_>>(),
            "previews": self.previews.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "pending": self.pending,
        });
        if let Some(a) = &self.art {
            v["frames"] = json!(a.frames.len());
            v["cell"] = json!([a.fw, a.fh]);
            v["origin"] = json!([a.origin.0, a.origin.1]);
        }
        v
    }

    pub fn to_text(&self) -> String {
        let mut s = match &self.art {
            Some(a) => format!(
                "{} ({}, {}): {} frame{} de {}×{}, origen [{}, {}]\n",
                self.name,
                self.source.display(),
                self.kind,
                a.frames.len(),
                if a.frames.len() == 1 { "" } else { "s" },
                a.fw,
                a.fh,
                a.origin.0,
                a.origin.1
            ),
            None => format!("{} ({}, {})\n", self.name, self.source.display(), self.kind),
        };
        for c in &self.checks {
            s += &format!("  {:<9} {:<18} {}\n", c.status.key(), c.title, c.message);
            if !c.at.is_empty() {
                let shown: Vec<String> = c.at.iter().map(|a| a.to_string()).collect();
                let more = if c.count > c.at.len() { format!(" … ({} en total)", c.count) } else { String::new() };
                s += &format!("  {:<9} {:<18} en: {}{more}\n", "", "", shown.join("; "));
            }
        }
        for p in &self.pending {
            s += &format!("  {:<9} {p}\n", "pendiente");
        }
        if !self.previews.is_empty() {
            s += "  vistas previas:\n";
            for p in &self.previews {
                s += &format!("    {}\n", p.display());
            }
        }
        s
    }
}

pub fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xff_ffff)
}

fn rgb(p: [u8; 4]) -> u32 {
    (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32
}

// ------------------------------------------------------------------ finding sprites

/// Every sprite of the project: (name, file), the .toml when there is one.
pub fn all_sprites(project: &Path) -> Vec<(String, PathBuf)> {
    asset_files(&project.join("sprites"), &["png"]).into_iter().map(|(n, p, _)| (n, p)).collect()
}

/// What the user typed (`coin`, `sprites/coin.toml`, `art/key.png`) as (name, file).
pub fn resolve(project: &Path, arg: &str) -> Result<(String, PathBuf), String> {
    let p = Path::new(arg);
    let lower = arg.to_ascii_lowercase();
    if lower.ends_with(".toml") || lower.ends_with(".png") {
        let cands = [p.to_path_buf(), project.join(p)];
        let Some(f) = cands.into_iter().find(|c| c.is_file()) else { return Err(format!("no existe {arg}")) };
        let name = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        // a PNG described by a TOML of the same name is checked through the TOML
        let toml = f.with_extension("toml");
        if lower.ends_with(".png") && toml.is_file() {
            return Ok((name, toml));
        }
        return Ok((name, f));
    }
    let dir = project.join("sprites");
    for c in [dir.join(format!("{arg}.toml")), dir.join(format!("{arg}.png"))] {
        if c.is_file() {
            return Ok((arg.to_string(), c));
        }
    }
    Err(format!("no hay ningún sprite «{arg}» (ni sprites/{arg}.toml ni sprites/{arg}.png)"))
}

// ------------------------------------------------------------------ checking

/// Checks one sprite file (`.toml` with `pixels`/`pixels_rle`/`image`, or a `.png`).
/// `global` is game.toml's `[palette]`.
pub fn check_sprite(name: &str, path: &Path, global: &BTreeMap<String, String>, opts: &Options) -> Report {
    let mut r = Report { name: name.to_string(), source: path.to_path_buf(), kind: "png", art: None, checks: vec![], colors: vec![], previews: vec![], pending: vec![] };
    let is_toml = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("toml"));
    let table: Option<toml::Table> = if is_toml {
        let parsed = std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| toml::from_str::<toml::Table>(&t).map_err(|e| e.to_string()));
        match parsed {
            Ok(t) => Some(t),
            Err(e) => {
                r.checks.push(check("toml", "TOML", Status::Error, format!("no se puede leer: {e}")));
                return r;
            }
        }
    } else {
        None
    };
    let strings = |k: &str| -> Vec<String> { table.as_ref().and_then(|t| t.get(k)).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default() };
    let (pixels, rle) = (strings("pixels"), strings("pixels_rle"));
    let art = if !pixels.is_empty() || !rle.is_empty() {
        r.kind = "texto";
        text_art(table.as_ref().unwrap(), &pixels, &rle, global, opts, &mut r.checks)
    } else {
        png_art(name, path, is_toml, table.as_ref(), global, opts, &mut r.checks)
    };
    if let Some(c) = name_check(name) {
        r.checks.insert(0, c);
    }
    let Some(art) = art else { return r };
    common_checks(&art, r.kind == "texto", global, opts, &mut r);
    r.pending.push("revisión visual a tamaño nativo y ampliado: ¿se lee el rasgo principal? (abre las vistas previas)".into());
    r.pending.push("sobre fondo claro y oscuro: huecos o halos en el contorno".into());
    if art.frames.len() > 1 {
        r.pending.push("animación: poses clave, movimiento y paso del último frame al primero (vista _bucle)".into());
    }
    if opts.tile {
        r.pending.push("tile: uniones, bandas o dibujos que aparecen al repetir (vista _3x3)".into());
    }
    r.art = Some(art);
    r
}

/// Asset names share the namespace of the built-in constants and functions: a sprite named
/// `key` hides `key.escape` from every script and the project stops loading.
pub fn name_check(name: &str) -> Option<Check> {
    if crate::natives::constant_names().iter().any(|c| c == name) {
        return Some(check("name", "nombre", Status::Error, format!("«{name}» es una constante integrada ({name}.…): el sprite la taparía en todos los scripts; usa otro nombre (spr_{name}, …)")));
    }
    if crate::natives::names().iter().any(|f| f == name) {
        return Some(check("name", "nombre", Status::Error, format!("«{name}» es una función integrada: el sprite la taparía; usa otro nombre (spr_{name}, …)")));
    }
    None
}

/// The most frequent value (ties: the first one seen).
fn mode(v: &[usize]) -> usize {
    let mut counts: Vec<(usize, usize)> = vec![];
    for &x in v {
        match counts.iter_mut().find(|(k, _)| *k == x) {
            Some(c) => c.1 += 1,
            None => counts.push((x, 1)),
        }
    }
    let best = counts.iter().map(|c| c.1).max().unwrap_or(0);
    counts.iter().find(|c| c.1 == best).map(|c| c.0).unwrap_or(0)
}

/// Magenta: characters missing from the palette, in the previews.
const MISSING: [u8; 4] = [255, 0, 255, 255];

fn text_art(t: &toml::Table, pixels: &[String], rle: &[String], global: &BTreeMap<String, String>, opts: &Options, checks: &mut Vec<Check>) -> Option<Art> {
    let grids = match crate::pixelart::frame_grids(pixels, rle) {
        Ok(g) => g,
        Err(e) => {
            checks.push(check("toml", "TOML", Status::Error, e));
            return None;
        }
    };
    if grids.iter().all(|g| g.is_empty()) {
        checks.push(check("frames", "frames", Status::Error, "el dibujo está vacío (ningún frame tiene filas)"));
        return None;
    }
    // --- the palette: game.toml's, overridden by the sprite's own
    let local: BTreeMap<String, String> = t.get("palette").and_then(|v| v.as_table()).map(|p| p.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()).unwrap_or_default();
    let mut pal: HashMap<char, u32> = HashMap::new();
    let mut global_pal: HashMap<char, u32> = HashMap::new();
    let mut entry_notes: Vec<String> = vec![];
    let mut bad_colours: Vec<String> = vec![];
    for (is_local, map) in [(false, global), (true, &local)] {
        for (k, v) in map {
            let Some(ch) = k.chars().next() else {
                bad_colours.push("clave vacía en la paleta".into());
                continue;
            };
            if k.chars().count() > 1 {
                entry_notes.push(format!("la clave «{k}» tiene varios caracteres: solo cuenta '{ch}'"));
            }
            if ch == '.' || ch == ' ' {
                entry_notes.push(format!("'{ch}' siempre es transparente: su color de la paleta no se usa"));
            }
            match crate::draw::parse_color(v) {
                Some(c) => {
                    if is_local {
                        if let Some(&g) = global_pal.get(&ch) {
                            if g != c {
                                entry_notes.push(format!("'{ch}' de la paleta local ({}) tapa al '{ch}' del proyecto ({})", hex(c), hex(g)));
                            }
                        } else if let Some((gc, _)) = global_pal.iter().find(|(_, &g)| g == c) {
                            entry_notes.push(format!("'{ch}' = {} ya está en la paleta del proyecto como '{gc}'", hex(c)));
                        }
                    } else {
                        global_pal.insert(ch, c);
                    }
                    pal.insert(ch, c);
                }
                None => bad_colours.push(format!("'{ch}' = «{v}» no es un color")),
            }
        }
    }
    // --- sizes: every frame as tall as the others, every row as wide as the others
    let heights: Vec<usize> = grids.iter().map(|g| g.len()).collect();
    let widths: Vec<usize> = grids.iter().flat_map(|g| g.iter().map(|r| r.len())).collect();
    let (w, h) = (mode(&widths), mode(&heights));
    let mut c = check("row_width", "ancho de filas", Status::Ok, format!("todas las filas miden {w} px"));
    for (f, g) in grids.iter().enumerate() {
        for (y, row) in g.iter().enumerate() {
            if row.len() != w {
                c.count += 1;
                if c.at.len() < MAX_AT {
                    c.at.push(At { frame: f, row: y, col: row.len().min(w) });
                }
            }
        }
    }
    if c.count > 0 {
        let first = c.at[0];
        let len = grids[first.frame][first.row].len();
        c.status = Status::Error;
        c.message = format!("{} fila(s) no miden {w} px (la primera: frame {}, fila {}, mide {len})", c.count, first.frame, first.row);
    }
    checks.push(c);
    let mut c = check("frame_size", "tamaño de frames", Status::Ok, format!("{} frame(s) de {w}×{h}", grids.len()));
    let wrong: Vec<String> = heights.iter().enumerate().filter(|(_, &fh)| fh != h).map(|(f, fh)| format!("frame {f}: {fh} filas")).collect();
    if !wrong.is_empty() {
        c.status = Status::Error;
        c.message = format!("los frames no miden lo mismo (se esperaban {h} filas): {}", wrong.join(", "));
    }
    checks.push(c);
    if opts.size.is_some_and(|(sw, sh)| (sw as usize, sh as usize) != (w, h)) {
        let (sw, sh) = opts.size.unwrap();
        checks.push(check("size", "tamaño pedido", Status::Error, format!("el dibujo mide {w}×{h} y se pidió {sw}×{sh}")));
    } else if let Some((sw, sh)) = opts.size {
        checks.push(check("size", "tamaño pedido", Status::Ok, format!("{sw}×{sh}, como se pidió")));
    }
    // --- symbols
    let mut unknown: BTreeMap<char, (usize, Vec<At>)> = BTreeMap::new();
    let mut used: BTreeMap<char, usize> = BTreeMap::new();
    let mut spaces = 0usize;
    for (f, g) in grids.iter().enumerate() {
        for (y, row) in g.iter().enumerate() {
            for (x, ch) in row.iter().enumerate() {
                match ch {
                    '.' => {}
                    ' ' => spaces += 1,
                    ch if pal.contains_key(ch) => *used.entry(*ch).or_default() += 1,
                    ch => {
                        let e = unknown.entry(*ch).or_default();
                        e.0 += 1;
                        if e.1.len() < 4 {
                            e.1.push(At { frame: f, row: y, col: x });
                        }
                    }
                }
            }
        }
    }
    let mut c = check("symbols", "símbolos", Status::Ok, format!("{} carácter(es) usados, todos en la paleta", used.len()));
    if !unknown.is_empty() {
        c.status = Status::Error;
        c.message = format!(
            "no están en la paleta: {}",
            unknown.iter().map(|(ch, (n, _))| format!("'{ch}' ({n} vez/veces)")).collect::<Vec<_>>().join(", ")
        );
        c.count = unknown.values().map(|u| u.0).sum();
        c.at = unknown.values().flat_map(|u| u.1.clone()).take(MAX_AT).collect();
    } else if spaces > 0 {
        c.status = Status::Warn;
        c.message = format!("{spaces} espacio(s) dentro de las filas: el transparente es '.' (los espacios al principio o al final de una fila se pierden)");
    }
    checks.push(c);
    // --- palette entries
    let unused: Vec<String> = local.keys().filter_map(|k| k.chars().next()).filter(|ch| !used.contains_key(ch) && *ch != '.').map(|ch| format!("'{ch}'")).collect();
    if !unused.is_empty() {
        entry_notes.push(format!("la paleta local define sin usar: {}", unused.join(", ")));
    }
    let mut c = check("palette_entries", "entradas de paleta", Status::Ok, if local.is_empty() { "usa solo la paleta del proyecto".to_string() } else { format!("{} color(es) locales", local.len()) });
    if !bad_colours.is_empty() {
        c.status = Status::Error;
        c.message = bad_colours.join("; ");
        if !entry_notes.is_empty() {
            c.message += &format!("; {}", entry_notes.join("; "));
        }
    } else if !entry_notes.is_empty() {
        c.status = Status::Warn;
        c.message = entry_notes.join("; ");
    }
    checks.push(c);
    // --- raster (as the loader does: short rows padded, cell = the widest row / tallest frame)
    let fw = grids.iter().flat_map(|g| g.iter().map(|r| r.len())).max().unwrap_or(0) as u32;
    let fh = grids.iter().map(|g| g.len()).max().unwrap_or(0) as u32;
    if fw == 0 || fh == 0 {
        return None;
    }
    let frames: Vec<Vec<u8>> = grids
        .iter()
        .map(|g| {
            let mut rgba = vec![0u8; (fw * fh * 4) as usize];
            for (y, row) in g.iter().enumerate() {
                for (x, ch) in row.iter().enumerate() {
                    if *ch == '.' || *ch == ' ' {
                        continue;
                    }
                    let px = match pal.get(ch) {
                        Some(&c) => [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255],
                        None => MISSING,
                    };
                    let i = ((y as u32 * fw + x as u32) * 4) as usize;
                    rgba[i..i + 4].copy_from_slice(&px);
                }
            }
            rgba
        })
        .collect();
    let (origin, feet) = origin_of(t.get("origin"), fw, fh, checks);
    mask_check(t.get("mask"), fw, fh, checks);
    Some(Art { fw, fh, frames, origin, feet_origin: feet })
}

fn origin_of(v: Option<&toml::Value>, fw: u32, fh: u32, checks: &mut Vec<Check>) -> ((f64, f64), bool) {
    let origin = match parse_origin(v, fw, fh) {
        Ok(o) => o,
        Err(e) => {
            checks.push(check("origin", "origen", Status::Error, e));
            (0.0, 0.0)
        }
    };
    let feet = matches!(v, Some(toml::Value::String(s)) if s == "bottom") || (origin.1 >= fh as f64 - 1.0 && fh > 1);
    if !checks.iter().any(|c| c.id == "origin") {
        let inside = (0.0..=fw as f64).contains(&origin.0) && (0.0..=fh as f64).contains(&origin.1);
        checks.push(if inside {
            check("origin", "origen", Status::Ok, format!("[{}, {}] dentro de la celda de {fw}×{fh}", origin.0, origin.1))
        } else {
            check("origin", "origen", Status::Warn, format!("[{}, {}] está fuera de la celda de {fw}×{fh}", origin.0, origin.1))
        });
    }
    (origin, feet)
}

fn mask_check(v: Option<&toml::Value>, fw: u32, fh: u32, checks: &mut Vec<Check>) {
    let Some(v) = v else { return };
    let n: Vec<i64> = v.as_array().map(|a| a.iter().filter_map(|x| x.as_integer()).collect()).unwrap_or_default();
    let [l, t, r, b] = n[..] else {
        checks.push(check("mask", "máscara", Status::Error, "mask debe ser [izq, arriba, der, abajo]"));
        return;
    };
    let ok = l >= 0 && t >= 0 && l <= r && t <= b && r < fw as i64 && b < fh as i64;
    checks.push(if ok {
        check("mask", "máscara", Status::Ok, format!("[{l}, {t}, {r}, {b}] dentro de la celda"))
    } else {
        check("mask", "máscara", Status::Warn, format!("[{l}, {t}, {r}, {b}] se sale de la celda de {fw}×{fh} o está al revés"))
    });
}

#[allow(clippy::too_many_arguments)]
fn png_art(name: &str, path: &Path, is_toml: bool, t: Option<&toml::Table>, global: &BTreeMap<String, String>, opts: &Options, checks: &mut Vec<Check>) -> Option<Art> {
    let s = match load_sprite(name, path, is_toml, global) {
        Ok(s) => s,
        Err(e) => {
            checks.push(check("load", "carga", Status::Error, e));
            return None;
        }
    };
    let frames: Vec<Vec<u8>> = s
        .frames
        .iter()
        .map(|f| {
            let mut out = Vec::with_capacity((f[2] * f[3] * 4) as usize);
            for y in 0..f[3] {
                let i = (((f[1] + y) * s.sheet_w + f[0]) * 4) as usize;
                out.extend_from_slice(&s.rgba[i..i + (f[2] * 4) as usize]);
            }
            out
        })
        .collect();
    // frames / frame_size against the image
    let mut c = check("frames", "frames", Status::Ok, format!("{} frame(s) de {}×{} en una imagen de {}×{}", frames.len(), s.fw, s.fh, s.sheet_w, s.sheet_h));
    let (rest_x, rest_y) = (s.sheet_w % s.fw, s.sheet_h % s.fh);
    if rest_x != 0 || rest_y != 0 {
        c.status = Status::Warn;
        c.message += &format!(": sobran {rest_x} px a la derecha y {rest_y} abajo (frames o frame_size no encajan con la imagen)");
    }
    checks.push(c);
    if let Some((sw, sh)) = opts.size {
        checks.push(if (sw, sh) == (s.fw, s.fh) {
            check("size", "tamaño pedido", Status::Ok, format!("{sw}×{sh}, como se pidió"))
        } else {
            check("size", "tamaño pedido", Status::Error, format!("los frames miden {}×{} y se pidió {sw}×{sh}", s.fw, s.fh))
        });
    }
    let (origin, feet) = origin_of(t.and_then(|t| t.get("origin")), s.fw, s.fh, checks);
    mask_check(t.and_then(|t| t.get("mask")), s.fw, s.fh, checks);
    Some(Art { fw: s.fw, fh: s.fh, frames, origin, feet_origin: feet })
}

/// The colours a palette choice allows; `None` with the reason when the check is skipped.
fn allowed(global: &BTreeMap<String, String>, choice: &PaletteChoice) -> Result<(String, Vec<u32>, Status), String> {
    match choice {
        PaletteChoice::Off => Err("omitida (--palette ninguna)".into()),
        PaletteChoice::Colors { name, colors } => Ok((name.clone(), colors.clone(), Status::Error)),
        PaletteChoice::Project => {
            let colors: Vec<u32> = global.values().filter_map(|v| crate::draw::parse_color(v)).collect();
            if colors.is_empty() {
                Err("omitida: game.toml no tiene [palette] (usa --palette <colores|archivo>)".into())
            } else {
                Ok(("la paleta del proyecto".into(), colors, Status::Warn))
            }
        }
    }
}

fn common_checks(a: &Art, text: bool, global: &BTreeMap<String, String>, opts: &Options, r: &mut Report) {
    let n = a.frames.len();
    // colours used (opaque pixels; magenta of missing characters left out)
    let mut used: HashMap<u32, (usize, At)> = HashMap::new();
    let mut partial = (0usize, vec![]);
    for f in 0..n {
        for y in 0..a.fh {
            for x in 0..a.fw {
                let p = a.px(f, x, y);
                if p[3] == 0 || (text && p == MISSING) {
                    continue;
                }
                if p[3] < 255 {
                    partial.0 += 1;
                    if partial.1.len() < MAX_AT {
                        partial.1.push(At { frame: f, row: y as usize, col: x as usize });
                    }
                }
                used.entry(rgb(p)).or_insert((0, At { frame: f, row: y as usize, col: x as usize })).0 += 1;
            }
        }
    }
    let mut colors: Vec<(u32, usize)> = used.iter().map(|(c, (k, _))| (*c, *k)).collect();
    colors.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    r.colors = colors.clone();
    // closed palette
    r.checks.push(match allowed(global, &opts.palette) {
        Err(why) => check("closed_palette", "paleta cerrada", Status::Skipped, why),
        Ok((pname, allowed, sev)) => {
            let outside: Vec<&(u32, usize)> = colors.iter().filter(|(c, _)| !allowed.contains(c)).collect();
            if outside.is_empty() {
                check("closed_palette", "paleta cerrada", Status::Ok, format!("los {} colores visibles están en {pname}", colors.len()))
            } else {
                let mut c = check(
                    "closed_palette",
                    "paleta cerrada",
                    sev,
                    format!("{} color(es) fuera de {pname}: {}", outside.len(), outside.iter().map(|(c, k)| format!("{} ({k} px)", hex(*c))).collect::<Vec<_>>().join(", ")),
                );
                c.count = outside.iter().map(|o| o.1).sum();
                c.at = outside.iter().take(MAX_AT).map(|(col, _)| used[col].1).collect();
                c
            }
        }
    });
    // alpha
    r.checks.push(if text {
        check("alpha", "alfa binaria", Status::Ok, "solo 0 y 255 (sprite de texto)")
    } else if partial.0 == 0 {
        check("alpha", "alfa binaria", Status::Ok, "el canal alfa solo tiene 0 y 255")
    } else {
        let mut c = check(
            "alpha",
            "alfa binaria",
            if opts.binary_alpha { Status::Error } else { Status::Warn },
            format!("{} píxel(es) semitransparentes (alfa entre 1 y 254)", partial.0),
        );
        c.count = partial.0;
        c.at = partial.1;
        c
    });
    // empty frames
    let empty: Vec<usize> = (0..n).filter(|&f| a.frames[f].chunks(4).all(|p| p[3] == 0)).collect();
    r.checks.push(if empty.is_empty() {
        check("empty_frames", "frames vacíos", Status::Ok, "ningún frame está vacío")
    } else {
        check("empty_frames", "frames vacíos", Status::Warn, format!("frame(s) sin ningún píxel visible: {}", empty.iter().map(|f| f.to_string()).collect::<Vec<_>>().join(", ")))
    });
    // anchor: the content's bounding box per frame, the feet line for sprites standing on their origin
    let boxes: Vec<Option<[u32; 4]>> = (0..n).map(|f| bbox(a, f)).collect();
    let bottoms: Vec<(usize, u32)> = boxes.iter().enumerate().filter_map(|(f, b)| b.map(|b| (f, b[3]))).collect();
    let oy = a.origin.1;
    r.checks.push(if bottoms.is_empty() {
        check("anchor", "anclaje", Status::Skipped, "no hay píxeles visibles")
    } else if a.feet_origin {
        let distinct: Vec<u32> = {
            let mut d: Vec<u32> = bottoms.iter().map(|b| b.1).collect();
            d.sort();
            d.dedup();
            d
        };
        let gap = oy - (bottoms[0].1 as f64 + 1.0);
        if distinct.len() > 1 {
            let mut c = check(
                "anchor",
                "anclaje",
                Status::Warn,
                format!(
                    "la línea de apoyo (fila más baja con píxeles) cambia entre frames con el origen abajo: {}",
                    bottoms.iter().map(|(f, b)| format!("frame {f} y={b}")).collect::<Vec<_>>().join(", ")
                ),
            );
            c.count = bottoms.iter().filter(|b| b.1 != bottoms[0].1).count();
            c.at = bottoms.iter().filter(|b| b.1 != bottoms[0].1).take(MAX_AT).map(|(f, b)| At { frame: *f, row: *b as usize, col: 0 }).collect();
            c
        } else if gap.abs() >= 1.0 {
            check("anchor", "anclaje", Status::Warn, format!("todos los frames apoyan en y={}, pero el origen está en y={oy}: el sprite {} {} px", bottoms[0].1, if gap > 0.0 { "flota" } else { "se hunde" }, gap.abs()))
        } else {
            check("anchor", "anclaje", Status::Ok, format!("todos los frames apoyan en y={} (origen y={oy})", bottoms[0].1))
        }
    } else {
        let lines: Vec<String> = boxes.iter().enumerate().filter_map(|(f, b)| b.map(|b| format!("frame {f}: x {}..{}, y {}..{}", b[0], b[2], b[1], b[3]))).collect();
        let moving = boxes.iter().flatten().any(|b| Some(*b) != boxes.iter().flatten().next().copied());
        check("anchor", "anclaje", Status::Info, format!("contenido {}: {}", if moving { "que se mueve en la celda" } else { "fijo en la celda" }, lines.join("; ")))
    });
    // isolated pixels: motes (no opaque neighbour at all) and colour orphans
    let (mut motes, mut orphans, mut at) = (0usize, 0usize, vec![]);
    for f in 0..n {
        for y in 0..a.fh {
            for x in 0..a.fw {
                let p = a.px(f, x, y);
                if p[3] == 0 {
                    continue;
                }
                let (mut opaque_n, mut same_n) = (false, false);
                for (dx, dy) in [(-1i32, -1i32), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= a.fw as i32 || ny >= a.fh as i32 {
                        continue;
                    }
                    let q = a.px(f, nx as u32, ny as u32);
                    if q[3] > 0 {
                        opaque_n = true;
                        if q == p && (dx == 0 || dy == 0) {
                            same_n = true;
                        }
                    }
                }
                if !opaque_n {
                    motes += 1;
                    if at.len() < MAX_AT {
                        at.push(At { frame: f, row: y as usize, col: x as usize });
                    }
                } else if !same_n {
                    orphans += 1;
                }
            }
        }
    }
    let mut c = check(
        "isolated_pixels",
        "píxeles sueltos",
        Status::Info,
        format!("{motes} mota(s) sin vecinos opacos y {orphans} píxel(es) sin vecino del mismo color: cada píxel suelto debe tener una función (pupila, brillo, hebilla)"),
    );
    c.count = motes;
    c.at = at;
    r.checks.push(c);
    if opts.tile {
        r.checks.push(tile_seams(a));
    }
}

/// [left, top, right, bottom] of the visible pixels of frame `f` (inclusive).
fn bbox(a: &Art, f: usize) -> Option<[u32; 4]> {
    let (mut l, mut t, mut r, mut b) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..a.fh {
        for x in 0..a.fw {
            if a.px(f, x, y)[3] > 0 {
                l = l.min(x);
                t = t.min(y);
                r = r.max(x);
                b = b.max(y);
            }
        }
    }
    (l != u32::MAX).then_some([l, t, r, b])
}

/// Repeatable tiles: edges that differ a lot read as seams, holes on the edges as gaps.
fn tile_seams(a: &Art) -> Check {
    let mut parts = vec![];
    let mut worst = Status::Ok;
    let mut at = vec![];
    for f in 0..a.frames.len() {
        let lr = (0..a.fh).filter(|&y| a.px(f, 0, y) != a.px(f, a.fw - 1, y)).count();
        let tb = (0..a.fw).filter(|&x| a.px(f, x, 0) != a.px(f, x, a.fh - 1)).count();
        let mut holes = 0;
        for y in 0..a.fh {
            for x in 0..a.fw {
                if (x == 0 || y == 0 || x == a.fw - 1 || y == a.fh - 1) && a.px(f, x, y)[3] < 255 {
                    holes += 1;
                    if at.len() < MAX_AT {
                        at.push(At { frame: f, row: y as usize, col: x as usize });
                    }
                }
            }
        }
        let bad = lr * 2 > a.fh as usize || tb * 2 > a.fw as usize || holes > 0;
        if bad {
            worst = Status::Warn;
        }
        parts.push(format!(
            "frame {f}: izq/der difieren en {lr} de {} px, arriba/abajo en {tb} de {} px{}",
            a.fh,
            a.fw,
            if holes > 0 { format!(", {holes} px transparentes en el borde") } else { String::new() }
        ));
    }
    let mut c = check("tile_seams", "uniones del tile", worst, parts.join("; "));
    c.count = at.len();
    c.at = at;
    c
}

// ------------------------------------------------------------------ previews

/// Default preview scale: frames at least 128 px on their long side.
pub fn default_scale(fw: u32, fh: u32) -> u32 {
    let long = fw.max(fh).max(1);
    128u32.div_ceil(long).clamp(1, 32)
}

struct Canvas {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

impl Canvas {
    fn new(w: u32, h: u32, bg: [u8; 3]) -> Canvas {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            rgba.extend_from_slice(&[bg[0], bg[1], bg[2], 255]);
        }
        Canvas { w, h, rgba }
    }
    fn blend(&mut self, x: u32, y: u32, p: [u8; 4], alpha: f32) {
        if x >= self.w || y >= self.h {
            return;
        }
        let a = p[3] as f32 / 255.0 * alpha;
        if a <= 0.0 {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        for k in 0..3 {
            self.rgba[i + k] = (self.rgba[i + k] as f32 * (1.0 - a) + p[k] as f32 * a).round() as u8;
        }
    }
    fn fill(&mut self, x: u32, y: u32, w: u32, h: u32, c: [u8; 3]) {
        for yy in y..(y + h).min(self.h) {
            for xx in x..(x + w).min(self.w) {
                let i = ((yy * self.w + xx) * 4) as usize;
                self.rgba[i..i + 3].copy_from_slice(&c);
            }
        }
    }
    fn checker(&mut self, x: u32, y: u32, w: u32, h: u32, cell: u32) {
        for yy in 0..h {
            for xx in 0..w {
                let c = if ((xx / cell) + (yy / cell)) % 2 == 0 { [204, 204, 204] } else { [153, 153, 153] };
                self.fill(x + xx, y + yy, 1, 1, c);
            }
        }
    }
    /// Frame `f` of `a` at (x, y), `s` times bigger, optionally tinted (onion skin).
    fn frame(&mut self, a: &Art, f: usize, x: u32, y: u32, s: u32, tint: Option<[u8; 3]>, alpha: f32) {
        for fy in 0..a.fh {
            for fx in 0..a.fw {
                let mut p = a.px(f, fx, fy);
                if p[3] == 0 {
                    continue;
                }
                if let Some(t) = tint {
                    p = [t[0], t[1], t[2], p[3]];
                }
                for dy in 0..s {
                    for dx in 0..s {
                        self.blend(x + fx * s + dx, y + fy * s + dy, p, alpha);
                    }
                }
            }
        }
    }
    fn save(&self, path: &Path) -> Result<(), String> {
        let png = crate::pixelart::encode_png(self.w, self.h, &self.rgba)?;
        std::fs::write(path, png).map_err(|e| format!("{}: {e}", path.display()))
    }
}

const LIGHT: [u8; 3] = [238, 238, 232];
const DARK: [u8; 3] = [24, 22, 34];
const GAP_BG: [u8; 3] = [60, 60, 70];

/// File-name-safe sprite name.
fn file_name(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

/// Writes the previews of `r` into `dir` and records their paths in `r.previews`:
/// `<name>_x<scale>.png` (every frame side by side over a checkerboard), `<name>_fondos.png`
/// (native size and scaled, on a light and on a dark background), `<name>_bucle.png` (with
/// several frames: last frame next to the first, then onion skin of each frame over the
/// one before) and `<name>_3x3.png` (with `tile`: each frame repeated 3×3).
pub fn write_previews(r: &mut Report, dir: &Path, scale: Option<u32>, tile: bool) -> Result<(), String> {
    let Some(a) = &r.art else { return Ok(()) };
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let s = scale.unwrap_or_else(|| default_scale(a.fw, a.fh)).max(1);
    let n = a.frames.len() as u32;
    let (cw, ch) = (a.fw * s, a.fh * s);
    let base = file_name(&r.name);
    let mut out = vec![];

    // all frames, scaled, over a checkerboard
    let mut c = Canvas::new(cw * n, ch, [0, 0, 0]);
    c.checker(0, 0, cw * n, ch, s.max(2));
    for f in 0..n {
        c.frame(a, f as usize, f * cw, 0, s, None, 1.0);
    }
    let p = dir.join(format!("{base}_x{s}.png"));
    c.save(&p)?;
    out.push(p);

    // light and dark backgrounds: native strip, then the scaled strip
    let pad = 8;
    let row_h = ch + 2 * pad;
    let w = pad + a.fw * n + pad + cw * n + pad;
    let mut c = Canvas::new(w, row_h * 2, LIGHT);
    c.fill(0, row_h, w, row_h, DARK);
    for (row, _) in [LIGHT, DARK].iter().enumerate() {
        let y0 = row as u32 * row_h + pad;
        for f in 0..n {
            c.frame(a, f as usize, pad + f * a.fw, y0, 1, None, 1.0);
            c.frame(a, f as usize, pad + a.fw * n + pad + f * cw, y0, s, None, 1.0);
        }
    }
    let p = dir.join(format!("{base}_fondos.png"));
    c.save(&p)?;
    out.push(p);

    // the loop: last + first, then onion skins (previous frame red, this one blue)
    if n > 1 {
        let gap = pad;
        let w = cw * 2 + gap + n * (cw + gap);
        let mut c = Canvas::new(w, ch, GAP_BG);
        c.fill(0, 0, cw * 2, ch, LIGHT);
        c.frame(a, n as usize - 1, 0, 0, s, None, 1.0);
        c.frame(a, 0, cw, 0, s, None, 1.0);
        let oy = (a.origin.1 * s as f64).round() as u32;
        let ox = (a.origin.0 * s as f64).round() as u32;
        for f in 0..n {
            let x0 = cw * 2 + gap + f * (cw + gap);
            c.fill(x0, 0, cw, ch, LIGHT);
            let prev = ((f + n - 1) % n) as usize;
            c.frame(a, prev, x0, 0, s, Some([220, 50, 50]), 0.5);
            c.frame(a, f as usize, x0, 0, s, Some([40, 80, 220]), 0.5);
            // origin guides
            if oy <= ch {
                c.fill(x0, oy.min(ch - 1), cw, 1, [40, 160, 70]);
            }
            if ox <= cw {
                c.fill(x0 + ox.min(cw - 1), 0, 1, ch, [40, 160, 70]);
            }
        }
        let p = dir.join(format!("{base}_bucle.png"));
        c.save(&p)?;
        out.push(p);
    }

    // tiles: each frame repeated 3×3
    if tile {
        let gap = pad;
        let w = n * cw * 3 + (n - 1) * gap;
        let mut c = Canvas::new(w, ch * 3, GAP_BG);
        for f in 0..n {
            let x0 = f * (cw * 3 + gap);
            c.checker(x0, 0, cw * 3, ch * 3, s.max(2));
            for ty in 0..3 {
                for tx in 0..3 {
                    c.frame(a, f as usize, x0 + tx * cw, ty * ch, s, None, 1.0);
                }
            }
        }
        let p = dir.join(format!("{base}_3x3.png"));
        c.save(&p)?;
        out.push(p);
    }
    r.previews = out;
    Ok(())
}

// ------------------------------------------------------------------ palettes

/// `--palette` values: `proyecto`, `ninguna`, `pear36`, colours (`#1a1c2c,#f4f4f4`) or a
/// palette file (.hex, .gpl, .pal, .json, PNG strip, as Lospec offers them).
pub fn parse_palette_choice(v: &str, saved_dir: Option<&Path>) -> Result<PaletteChoice, String> {
    let lower = v.trim().to_lowercase();
    match lower.as_str() {
        "proyecto" | "project" | "" => return Ok(PaletteChoice::Project),
        "ninguna" | "none" | "off" | "no" => return Ok(PaletteChoice::Off),
        "pear36" => return Ok(PaletteChoice::Colors { name: "Pear36".into(), colors: crate::palette::PEAR36.to_vec() }),
        _ => {}
    }
    let file = Path::new(v);
    let saved = saved_dir.map(|d| d.join(format!("{lower}.json")));
    if let Some(f) = [Some(file.to_path_buf()), saved].into_iter().flatten().find(|f| f.is_file()) {
        let bytes = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        let p = Palette::parse_any(&f.to_string_lossy(), &bytes)?;
        let name = if p.name.is_empty() { f.display().to_string() } else { p.name.clone() };
        return Ok(PaletteChoice::Colors { name: format!("la paleta {name}"), colors: p.colors });
    }
    let colors: Result<Vec<u32>, String> = v.split([',', ' ']).filter(|s| !s.trim().is_empty()).map(|s| crate::draw::parse_color(s).ok_or_else(|| format!("«{s}» no es un color"))).collect();
    let colors = colors.map_err(|e| format!("--palette: {e} (usa proyecto, ninguna, pear36, #rrggbb,#rrggbb… o un archivo de paleta)"))?;
    if colors.is_empty() {
        return Err("--palette: no hay colores".into());
    }
    Ok(PaletteChoice::Colors { name: format!("los {} colores de --palette", colors.len()), colors })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("modengine_pixelcheck_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn global() -> BTreeMap<String, String> {
        [("k", "#1a1c2c"), ("y", "#ffcd75"), ("o", "#ef7d57")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    fn text(dir: &Path, name: &str, src: &str, opts: &Options) -> Report {
        let p = dir.join(format!("{name}.toml"));
        std::fs::write(&p, src).unwrap();
        check_sprite(name, &p, &global(), opts)
    }

    #[test]
    fn row_width_reports_frame_and_row() {
        let d = tmp("rows");
        let r = text(&d, "a", "pixels = ['''\n.kk.\nkyyk\nkyyyk\n.kk.\n''']\n", &Options::default());
        let c = r.get("row_width").unwrap();
        assert_eq!(c.status, Status::Error);
        assert_eq!(c.at, [At { frame: 0, row: 2, col: 4 }]);
        assert!(c.message.contains("frame 0, fila 2, mide 5"), "{}", c.message);
        // a frame with a missing row
        let r = text(&d, "b", "pixels = ['''\n.kk.\nkyyk\n''', '''\n.kk.\n''']\n", &Options::default());
        assert_eq!(r.get("frame_size").unwrap().status, Status::Error);
        assert!(r.get("frame_size").unwrap().message.contains("frame 1: 1 filas"));
        // the loader itself pads the row and loads it: only pixel-check sees it
        assert!(crate::project::load_sprite("a", &d.join("a.toml"), true, &global()).is_ok());
        let ok = text(&d, "c", "pixels = ['''\n.kk.\nkyyk\n''']\n", &Options { size: Some((4, 2)), ..Default::default() });
        assert_eq!(ok.errors(), 0, "{}", ok.to_text());
        assert_eq!(ok.get("size").unwrap().status, Status::Ok);
        let wrong = text(&d, "c", "pixels = ['''\n.kk.\nkyyk\n''']\n", &Options { size: Some((16, 16)), ..Default::default() });
        assert_eq!(wrong.get("size").unwrap().status, Status::Error);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unknown_symbols_and_palette_entries() {
        let d = tmp("sym");
        let r = text(&d, "a", "palette = { z = \"#ffcd75\", q = \"#123456\", kk = \"#1a1c2c\" }\npixels = ['''\nkZk\nkyk\nkZz\n''']\n", &Options::default());
        let c = r.get("symbols").unwrap();
        assert_eq!(c.status, Status::Error);
        assert_eq!(c.count, 2);
        assert_eq!(c.at[0], At { frame: 0, row: 0, col: 1 });
        assert!(c.message.contains("'Z'"));
        let e = r.get("palette_entries").unwrap();
        assert_eq!(e.status, Status::Warn);
        assert!(e.message.contains("ya está en la paleta del proyecto como 'y'") && e.message.contains("sin usar: 'q'") && e.message.contains("varios caracteres"), "{}", e.message);
        // spaces are transparent but fragile
        let r = text(&d, "b", "pixels = ['''\nk k\nkyk\n''']\n", &Options::default());
        assert_eq!(r.get("symbols").unwrap().status, Status::Warn);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn closed_palette_in_text_and_png() {
        let d = tmp("pal");
        // a local colour outside the project palette: a warning with the project palette...
        let r = text(&d, "a", "palette = { q = \"#123456\" }\npixels = ['''\nkq\nyk\n''']\n", &Options::default());
        assert_eq!(r.get("closed_palette").unwrap().status, Status::Warn);
        // ...an error with an explicit one
        let pc = parse_palette_choice("#1a1c2c,#ffcd75", None).unwrap();
        let r = text(&d, "a", "palette = { q = \"#123456\" }\npixels = ['''\nkq\nyk\n''']\n", &Options { palette: pc, ..Default::default() });
        let c = r.get("closed_palette").unwrap();
        assert_eq!(c.status, Status::Error);
        assert!(c.message.contains("#123456 (1 px)"));
        assert_eq!(c.at, [At { frame: 0, row: 0, col: 1 }]);
        // a PNG with an off-palette pixel
        let mut img = image::RgbaImage::from_pixel(4, 4, image::Rgba([0x1a, 0x1c, 0x2c, 255]));
        img.put_pixel(2, 3, image::Rgba([1, 2, 3, 255]));
        img.save(d.join("p.png")).unwrap();
        let r = check_sprite("p", &d.join("p.png"), &global(), &Options::default());
        let c = r.get("closed_palette").unwrap();
        assert_eq!((c.status, c.at.clone()), (Status::Warn, vec![At { frame: 0, row: 3, col: 2 }]), "{}", r.to_text());
        // no project palette: skipped
        let r = check_sprite("p", &d.join("p.png"), &BTreeMap::new(), &Options::default());
        assert_eq!(r.get("closed_palette").unwrap().status, Status::Skipped);
        assert_eq!(parse_palette_choice("ninguna", None).unwrap(), PaletteChoice::Off);
        assert!(parse_palette_choice("rojo-raro", None).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn partial_alpha_in_png() {
        let d = tmp("alpha");
        let mut img = image::RgbaImage::from_pixel(3, 3, image::Rgba([0, 0, 0, 0]));
        img.put_pixel(1, 1, image::Rgba([255, 205, 117, 255]));
        img.put_pixel(2, 1, image::Rgba([255, 205, 117, 128]));
        img.save(d.join("h.png")).unwrap();
        let r = check_sprite("h", &d.join("h.png"), &global(), &Options::default());
        let c = r.get("alpha").unwrap();
        assert_eq!((c.status, c.count), (Status::Warn, 1));
        assert_eq!(c.at, [At { frame: 0, row: 1, col: 2 }]);
        let r = check_sprite("h", &d.join("h.png"), &global(), &Options { binary_alpha: true, ..Default::default() });
        assert_eq!(r.get("alpha").unwrap().status, Status::Error);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn anchor_drift_and_empty_frames() {
        let d = tmp("anchor");
        // origin at the bottom: frame 1 lifts its feet one pixel
        let r = text(&d, "w", "origin = \"bottom\"\npixels = ['''\n.k.\n.k.\nk.k\n''', '''\n.k.\nk.k\n...\n''', '''\n...\n...\n...\n''']\n", &Options::default());
        let c = r.get("anchor").unwrap();
        assert_eq!(c.status, Status::Warn, "{}", r.to_text());
        assert_eq!(c.at, [At { frame: 1, row: 1, col: 0 }]);
        assert!(r.get("empty_frames").unwrap().message.contains(": 2"));
        assert_eq!(r.get("empty_frames").unwrap().status, Status::Warn);
        // steady feet
        let r = text(&d, "w", "origin = \"bottom\"\npixels = ['''\n.k.\nk.k\n''', '''\nk.k\n.k.\n''']\n", &Options::default());
        assert_eq!(r.get("anchor").unwrap().status, Status::Ok, "{}", r.to_text());
        // floating above the origin
        let r = text(&d, "w", "origin = [1, 3]\npixels = ['''\n.k.\nk.k\n...\n''']\n", &Options::default());
        let c = r.get("anchor").unwrap();
        assert!(c.status == Status::Warn && c.message.contains("flota 1 px"), "{}", c.message);
        // origin outside the cell
        let r = text(&d, "w", "origin = [9, 1]\npixels = ['''\nkk\n''']\n", &Options::default());
        assert_eq!(r.get("origin").unwrap().status, Status::Warn);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tile_seams_and_isolated_pixels() {
        let d = tmp("tile");
        let opts = Options { tile: true, ..Default::default() };
        let good = text(&d, "t", "pixels = ['''\nkyyk\nyooy\nyooy\nkyyk\n''']\n", &opts);
        assert_eq!(good.get("tile_seams").unwrap().status, Status::Ok, "{}", good.to_text());
        let bad = text(&d, "t", "pixels = ['''\nkkkk\nkooy\nkooy\nyyy.\n''']\n", &opts);
        let c = bad.get("tile_seams").unwrap();
        assert_eq!(c.status, Status::Warn);
        assert!(c.message.contains("1 px transparentes") && c.message.contains("arriba/abajo en 4 de 4"), "{}", c.message);
        let r = text(&d, "m", "pixels = ['''\nk...\n....\n..yo\n''']\n", &Options::default());
        let c = r.get("isolated_pixels").unwrap();
        assert_eq!((c.status, c.count), (Status::Info, 1));
        assert_eq!(c.at, [At { frame: 0, row: 0, col: 0 }]);
        assert!(c.message.contains("debe tener una función"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn previews_have_the_expected_sizes() {
        let d = tmp("prev");
        let mut r = text(&d, "anim", "origin = \"bottom\"\npixels = ['''\nkyyk\nk..k\n''', '''\n.yy.\nk..k\n''', '''\nkZZk\nk..k\n''']\n", &Options { tile: true, ..Default::default() });
        let out = d.join("out");
        write_previews(&mut r, &out, None, true).unwrap();
        let s = default_scale(4, 2);
        assert_eq!(s, 32);
        let dims = |p: &str| image::open(out.join(p)).unwrap().to_rgba8().dimensions();
        assert_eq!(dims(&format!("anim_x{s}.png")), (4 * s * 3, 2 * s));
        assert_eq!(dims("anim_fondos.png"), (8 + 12 + 8 + 4 * s * 3 + 8, (2 * s + 16) * 2));
        assert_eq!(dims("anim_bucle.png"), (4 * s * 2 + 8 + 3 * (4 * s + 8), 2 * s));
        assert_eq!(dims("anim_3x3.png"), (3 * (12 * s) + 2 * 8, 6 * s));
        assert_eq!(r.previews.len(), 4);
        // missing characters show as magenta, and dark/light halves really differ
        let img = image::open(out.join(format!("anim_x{s}.png"))).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(2 * 4 * s + s + 1, 1).0, [255, 0, 255, 255]);
        let f = image::open(out.join("anim_fondos.png")).unwrap().to_rgba8();
        assert_eq!(f.get_pixel(0, 0).0[..3], LIGHT);
        assert_eq!(f.get_pixel(0, f.height() - 1).0[..3], DARK);
        // a single frame without --tile: two previews
        let mut one = text(&d, "one", "pixels = ['''\nkk\n''']\n", &Options::default());
        write_previews(&mut one, &out, Some(3), false).unwrap();
        assert_eq!(one.previews.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect::<Vec<_>>(), ["one_x3.png", "one_fondos.png"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn names_of_built_ins_are_errors() {
        let d = tmp("name");
        let src = "pixels = ['''\nky\n''']\n";
        let r = text(&d, "key", src, &Options::default());
        let c = r.get("name").unwrap();
        assert_eq!(c.status, Status::Error);
        assert!(c.message.contains("constante integrada"), "{}", c.message);
        assert_eq!(text(&d, "draw_sprite", src, &Options::default()).get("name").unwrap().status, Status::Error);
        assert!(text(&d, "llave", src, &Options::default()).get("name").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn resolves_names_and_paths() {
        let d = tmp("res");
        std::fs::create_dir_all(d.join("sprites")).unwrap();
        std::fs::write(d.join("sprites/a.toml"), "pixels=['''\nk\n''']").unwrap();
        image::RgbaImage::new(2, 2).save(d.join("sprites/b.png")).unwrap();
        assert_eq!(resolve(&d, "a").unwrap().1, d.join("sprites/a.toml"));
        assert_eq!(resolve(&d, "b").unwrap().1, d.join("sprites/b.png"));
        assert_eq!(resolve(&d, "sprites/a.toml").unwrap().0, "a");
        assert!(resolve(&d, "nada").is_err());
        assert_eq!(all_sprites(&d).len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }
}

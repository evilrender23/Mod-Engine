//! Importing art from anywhere on disk into a project: PNG images become sprites (`sprites/`)
//! or tile sets (`tilesets/` + the sheet as a sprite), WAV/OGG files sounds (`sounds/`) and
//! TTF/OTF files fonts (`fonts/`).
//!
//! Importing is copying the file into the project as `<name>.<ext>` and, only for a sprite with
//! options that differ from the engine's defaults (strip or grid of frames, origin, fps),
//! writing the companion `<name>.toml`. A tile set also gets `tilesets/<name>.toml`. Nothing
//! here changes what the engine understands.
//!
//! The editor's import dialog (`modengine-editor/src/import.rs`) and `modengine import` (the
//! command line, also what the editor's coding agent runs) both go through [`import`], so they
//! write the same files. [`plan`] + [`execute`] are the command line's batch version: folders
//! are walked, names cleaned up and made unique, frames guessed from the file names; and
//! [`inspect`] describes an image (size, likely frame layouts, colours) for whoever decides.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Files above this size get a warning (they are still imported).
pub const BIG_FILE: u64 = 20 * 1024 * 1024;

// ------------------------------------------------------------------ kinds and names

/// What a file is, by its extension.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub enum Kind {
    Sprite,
    Sound,
    Font,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Sprite, Kind::Sound, Kind::Font];

    /// By extension, case-insensitive: the formats the engine loads.
    pub fn of(path: &Path) -> Option<Kind> {
        let ext = path.extension()?.to_string_lossy().to_lowercase();
        Kind::ALL.into_iter().find(|k| k.exts().contains(&ext.as_str()))
    }

    pub fn exts(self) -> &'static [&'static str] {
        match self {
            Kind::Sprite => &["png"],
            Kind::Sound => &["wav", "ogg"],
            Kind::Font => &["ttf", "otf"],
        }
    }

    /// Project folder the files go to.
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Sprite => "sprites",
            Kind::Sound => "sounds",
            Kind::Font => "fonts",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Sprite => "sprite",
            Kind::Sound => "sonido",
            Kind::Font => "fuente",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::Sprite => "▣",
            Kind::Sound => "♪",
            Kind::Font => "A",
        }
    }

    /// File browser filter label.
    pub fn plural(self) -> &'static str {
        match self {
            Kind::Sprite => "Imágenes (PNG)",
            Kind::Sound => "Sonidos (WAV, OGG)",
            Kind::Font => "Fuentes (TTF, OTF)",
        }
    }
}

/// A valid asset/object name: letters, digits and `_`, not starting with a digit.
pub fn valid_name(s: &str) -> bool {
    let mut ch = s.chars();
    matches!(ch.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') && ch.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// An asset name from a file name: ASCII letters, digits and `_`, lower case (`PlayerRun` →
/// `player_run`, `Moneda dorada (2)` → `moneda_dorada_2`), never starting with a digit.
pub fn sanitize(stem: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in stem.chars() {
        let c = fold_accent(c);
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && prev_lower && !out.ends_with('_') {
                out.push('_');
            }
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
            out.push(c.to_ascii_lowercase());
        } else {
            prev_lower = false;
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
        }
    }
    let mut out = out.trim_end_matches('_').to_string();
    if out.is_empty() {
        out = "asset".into();
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn fold_accent(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
        'Á' | 'À' | 'Ä' | 'Â' | 'Ã' | 'Å' => 'A',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'É' | 'È' | 'Ë' | 'Ê' => 'E',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'Í' | 'Ì' | 'Ï' | 'Î' => 'I',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
        'Ó' | 'Ò' | 'Ö' | 'Ô' | 'Õ' => 'O',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'Ú' | 'Ù' | 'Ü' | 'Û' => 'U',
        'ñ' => 'n',
        'Ñ' => 'N',
        'ç' => 'c',
        'Ç' => 'C',
        c => c,
    }
}

/// GameMaker-style strips: `hero_run_strip4` → (`hero_run`, 4 frames).
pub fn strip_suffix(stem: &str) -> Option<(&str, u32)> {
    let i = stem.to_lowercase().rfind("_strip").or_else(|| stem.to_lowercase().rfind("-strip"))?;
    let n: u32 = stem[i + 6..].trim().parse().ok()?;
    (n > 0 && i > 0).then(|| (&stem[..i], n))
}

/// A trailing `WxH` (`hero_32x32`, `tiles-16x16`): (`hero`, 32, 32).
pub fn size_suffix(stem: &str) -> Option<(&str, u32, u32)> {
    let cut = stem.rfind(|c: char| c == '_' || c == '-' || c == ' ' || c == '.')?;
    let (w, h) = stem[cut + 1..].to_lowercase().split_once('x').map(|(a, b)| (a.to_string(), b.to_string()))?;
    let (w, h): (u32, u32) = (w.parse().ok()?, h.parse().ok()?);
    (w > 0 && h > 0 && cut > 0).then(|| (&stem[..cut], w, h))
}

/// A trailing `_N` (`coin_8`): (`coin`, 8).
pub fn count_suffix(stem: &str) -> Option<(&str, u32)> {
    let cut = stem.rfind(|c: char| c == '_' || c == '-' || c == ' ')?;
    let n: u32 = stem[cut + 1..].parse().ok()?;
    (n > 1 && cut > 0).then(|| (&stem[..cut], n))
}

/// `base`, or `base_2`, `base_3`... the first one not taken.
pub fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base}_{n}")).find(|n| !taken(n)).unwrap()
}

/// `~`, `~/x`, quotes and `file://` URIs (what file managers paste) become a plain path.
pub fn expand(s: &str) -> PathBuf {
    let s = s.trim().trim_matches(|c| c == '"' || c == '\'');
    let s = match s.strip_prefix("file://") {
        Some(rest) => percent_decode(rest),
        None => s.to_string(),
    };
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if s == "~" {
        home()
    } else if let Some(rest) = s.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(s)
    }
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn human_size(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else if n >= 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

/// A quoted TOML string.
fn q(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

// ------------------------------------------------------------------ sprite options

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    TopLeft,
    Top,
    Center,
    Left,
    Bottom,
    /// A pixel of the frame.
    At(i32, i32),
}

impl Origin {
    /// The named ones (what the editor's menu offers).
    pub const ALL: [Origin; 5] = [Origin::TopLeft, Origin::Center, Origin::Bottom, Origin::Top, Origin::Left];

    /// The engine's name (`origin = "..."`); `custom` for a pixel.
    pub fn key(self) -> &'static str {
        match self {
            Origin::TopLeft => "top_left",
            Origin::Top => "top",
            Origin::Center => "center",
            Origin::Left => "left",
            Origin::Bottom => "bottom",
            Origin::At(..) => "custom",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Origin::TopLeft => "arriba a la izquierda",
            Origin::Top => "arriba al centro",
            Origin::Center => "centro",
            Origin::Left => "izquierda al centro",
            Origin::Bottom => "abajo al centro",
            Origin::At(..) => "un píxel concreto",
        }
    }

    /// `top-left`, `top_left`, `center`, `bottom-center`/`bottom`, `top`, `left` or `x,y`.
    pub fn parse(s: &str) -> Option<Origin> {
        let k = s.trim().to_lowercase().replace(['-', ' '], "_");
        Some(match k.as_str() {
            "top_left" | "topleft" | "tl" => Origin::TopLeft,
            "top" | "top_center" => Origin::Top,
            "center" | "centre" | "middle" => Origin::Center,
            "left" | "left_center" => Origin::Left,
            "bottom" | "bottom_center" | "feet" => Origin::Bottom,
            _ => {
                let (x, y) = s.split_once(',')?;
                Origin::At(x.trim().parse().ok()?, y.trim().parse().ok()?)
            }
        })
    }

    /// Where it is inside a `w`×`h` frame (as the engine computes it).
    pub fn point(self, w: f32, h: f32) -> (f32, f32) {
        match self {
            Origin::TopLeft => (0.0, 0.0),
            Origin::Top => (w / 2.0, 0.0),
            Origin::Center => (w / 2.0, h / 2.0),
            Origin::Left => (0.0, h / 2.0),
            Origin::Bottom => (w / 2.0, h),
            Origin::At(x, y) => (x as f32, y as f32),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Frames {
    Single,
    /// Horizontal strip of N frames.
    Strip(u32),
    /// Grid of cells of this size (rows left to right).
    Grid(u32, u32),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpriteOpts {
    pub frames: Frames,
    pub origin: Origin,
    pub fps: f64,
}

pub const DEFAULT_FPS: f64 = 10.0;

impl Default for SpriteOpts {
    fn default() -> Self {
        SpriteOpts { frames: Frames::Single, origin: Origin::TopLeft, fps: DEFAULT_FPS }
    }
}

impl SpriteOpts {
    /// What a loose PNG already is: no TOML needed.
    pub fn is_default(&self) -> bool {
        matches!(self.frames, Frames::Single | Frames::Strip(1)) && self.origin == Origin::TopLeft && self.fps == DEFAULT_FPS
    }

    /// `(frames, frame width, frame height)` of a `w`×`h` image, as the engine slices it.
    pub fn slice(&self, w: u32, h: u32) -> Result<(u32, u32, u32), String> {
        match self.frames {
            Frames::Single => Ok((1, w, h)),
            Frames::Strip(n) => {
                if n == 0 {
                    return Err("el número de frames tiene que ser al menos 1".into());
                }
                if n > w {
                    return Err(format!("la imagen mide {w} px de ancho: no caben {n} frames"));
                }
                Ok((n, w / n, h))
            }
            Frames::Grid(fw, fh) => {
                if fw == 0 || fh == 0 {
                    return Err("el tamaño del frame tiene que ser positivo".into());
                }
                if fw > w || fh > h {
                    return Err(format!("un frame de {fw}×{fh} no cabe en una imagen de {w}×{h}"));
                }
                Ok(((w / fw) * (h / fh), fw, fh))
            }
        }
    }

    /// Pixels the slicing leaves out (the engine ignores them).
    pub fn leftover(&self, w: u32, h: u32) -> Option<String> {
        match self.frames {
            Frames::Strip(n) if n > 0 && w % n != 0 => Some(format!("{w} px no se divide entre {n}: sobran {} px a la derecha", w % n)),
            Frames::Grid(fw, fh) if fw > 0 && fh > 0 && (w % fw != 0 || h % fh != 0) => {
                Some(format!("{w}×{h} no es múltiplo de {fw}×{fh}: los bordes sobrantes se ignoran"))
            }
            _ => None,
        }
    }
}

/// The sprite's `.toml` for `image_file`, or `None` when the defaults are enough (then the
/// loose PNG is the sprite).
pub fn sprite_toml(image_file: &str, o: &SpriteOpts) -> Option<String> {
    if o.is_default() {
        return None;
    }
    let mut s = format!("image = {}\n", q(image_file));
    match o.frames {
        Frames::Single | Frames::Strip(1) => {}
        Frames::Strip(n) => s += &format!("frames = {n}\n"),
        Frames::Grid(w, h) => s += &format!("frame_size = [{w}, {h}]\n"),
    }
    match o.origin {
        Origin::TopLeft => {}
        Origin::At(x, y) => s += &format!("origin = [{x}, {y}]\n"),
        named => s += &format!("origin = {}\n", q(named.key())),
    }
    if o.fps != DEFAULT_FPS {
        s += &format!("fps = {}\n", modscript::fmt_num(o.fps));
    }
    Some(s)
}

// ------------------------------------------------------------------ tile sets

/// How a tile set image is cut.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TileOpts {
    pub tile: [u32; 2],
    /// Pixels before the first tile.
    pub offset: [u32; 2],
    /// Pixels between tiles.
    pub sep: [u32; 2],
}

impl TileOpts {
    pub fn new(w: u32, h: u32) -> TileOpts {
        TileOpts { tile: [w, h], offset: [0, 0], sep: [0, 0] }
    }

    /// `(columns, rows)` of tiles in a `w`×`h` image (as `tileset::slice` cuts it).
    pub fn grid(&self, w: u32, h: u32) -> Result<(u32, u32), String> {
        let [tw, th] = self.tile;
        if tw == 0 || th == 0 {
            return Err("el tamaño del tile tiene que ser positivo".into());
        }
        let (cols, rows, _) = crate::tileset::slice(w, h, tw, th, self.offset, self.sep);
        if cols == 0 || rows == 0 {
            return Err(format!("la imagen ({w}×{h}) es más pequeña que un tile de {tw}×{th}"));
        }
        Ok((cols, rows))
    }
}

/// `tilesets/<name>.toml` cutting the sprite `sprite`.
pub fn tileset_toml(sprite: &str, t: &TileOpts) -> String {
    let mut s = format!("sprite = {}\ntile_width = {}\ntile_height = {}\n", q(sprite), t.tile[0], t.tile[1]);
    if t.offset != [0, 0] {
        s += &format!("offset = [{}, {}]\n", t.offset[0], t.offset[1]);
    }
    if t.sep != [0, 0] {
        s += &format!("separation = [{}, {}]\n", t.sep[0], t.sep[1]);
    }
    s
}

/// The sprite holding a tile set's sheet: `ts_cave` → `spr_cave`, `dungeon` → `spr_dungeon`.
pub fn tileset_sprite_name(tileset: &str) -> String {
    format!("spr_{}", tileset.strip_prefix("ts_").unwrap_or(tileset))
}

/// Does the file name say "tile set"? (`dungeon_tiles`, `Tileset 16x16`, `ts_cave`, `terrain`.)
pub fn looks_like_tileset(stem: &str) -> bool {
    let s = sanitize(stem);
    s.starts_with("ts_")
        || s.split('_').any(|t| matches!(t, "tileset" | "tilesets" | "tiles" | "tilemap" | "tilesheet" | "terrain" | "autotile"))
        || s.contains("tileset")
}

// ------------------------------------------------------------------ importing

/// One imported asset.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub kind: Kind,
    pub name: String,
    /// Files written, relative to the project (`sprites/hero.png`, `sprites/hero.toml`).
    pub files: Vec<String>,
    /// An asset with this name was replaced (its old files are in `.trash`).
    pub replaced: bool,
    /// The file it was copied from (the store looks up its attribution by it).
    pub src: PathBuf,
}

/// The files of asset `name` in `folder`: `name.*`, plus the file its `.toml` points to.
pub fn asset_files_named(folder: &Path, name: &str) -> Vec<PathBuf> {
    let mut out = vec![];
    let Ok(rd) = fs::read_dir(folder) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_file() && p.file_stem().is_some_and(|s| s == name) {
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")) {
                if let Ok(t) = fs::read_to_string(&p).unwrap_or_default().parse::<toml::Table>() {
                    for key in ["image", "file"] {
                        if let Some(f) = t.get(key).and_then(|v| v.as_str()) {
                            let f = folder.join(f);
                            if f.is_file() && !out.contains(&f) {
                                out.push(f);
                            }
                        }
                    }
                }
            }
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// Moves a project file into `<project>/.trash/` (same as deleting from the editor's browser).
pub fn trash(project: &Path, path: &Path) -> Result<(), String> {
    let dir = project.join(".trash");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = path.file_name().ok_or("bad path")?;
    let mut target = dir.join(name);
    let mut n = 1;
    while target.exists() {
        target = dir.join(format!("{}.{n}", name.to_string_lossy()));
        n += 1;
    }
    fs::rename(path, &target).map_err(|e| format!("{}: {e}", path.display()))
}

/// Copies `src` into the project as asset `name` of `kind` and, for a sprite with
/// non-default options, writes its `.toml`. With `overwrite`, an asset that already has this
/// name in that folder is moved to `.trash` first; without it that is an error.
pub fn import(project: &Path, kind: Kind, src: &Path, name: &str, opts: &SpriteOpts, overwrite: bool) -> Result<Imported, String> {
    if !valid_name(name) {
        return Err(format!("'{name}' no es un nombre válido (letras, números y _, sin empezar por número)"));
    }
    if Kind::of(src) != Some(kind) {
        return Err(format!("{}: no es un fichero de {}", src.display(), kind.label()));
    }
    if !src.is_file() {
        return Err(format!("no existe el fichero {}", src.display()));
    }
    let folder = project.join(kind.folder());
    fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let old = asset_files_named(&folder, name);
    let replaced = !old.is_empty();
    if replaced {
        if !overwrite {
            return Err(format!("ya existe un asset '{name}' en {}/", kind.folder()));
        }
        let src_real = src.canonicalize().unwrap_or(src.to_path_buf());
        if old.iter().any(|f| f.canonicalize().is_ok_and(|f| f == src_real)) {
            return Err(format!("{} ya es el asset '{name}' del proyecto", src.display()));
        }
    }
    // Read before touching the project, so a failure leaves it as it was.
    let bytes = fs::read(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    write_asset(project, kind, &bytes, &ext, name, opts, &old, src)
}

/// Writes asset `name` (its file and, for a sprite, its `.toml`), after moving `old` to `.trash`.
#[allow(clippy::too_many_arguments)]
fn write_asset(project: &Path, kind: Kind, bytes: &[u8], ext: &str, name: &str, opts: &SpriteOpts, old: &[PathBuf], src: &Path) -> Result<Imported, String> {
    let folder = project.join(kind.folder());
    let replaced = !old.is_empty();
    for f in old {
        trash(project, f)?;
    }
    let file = format!("{name}.{ext}");
    let dst = folder.join(&file);
    fs::write(&dst, bytes).map_err(|e| format!("{}: {e}", dst.display()))?;
    let mut files = vec![format!("{}/{file}", kind.folder())];
    if kind == Kind::Sprite {
        if let Some(text) = sprite_toml(&file, opts) {
            let t = folder.join(format!("{name}.toml"));
            fs::write(&t, text).map_err(|e| format!("{}: {e}", t.display()))?;
            files.push(format!("{}/{name}.toml", kind.folder()));
        }
    }
    Ok(Imported { kind, name: name.to_string(), files, replaced, src: src.to_path_buf() })
}

/// Imports the cells of one row of a sheet as the sprite `name`: `crop` (`[x, y, w, h]`) is
/// cut out of `src` and saved as `sprites/<name>.png`, a strip of `opts` frames.
pub fn import_crop(project: &Path, src: &Path, crop: [u32; 4], name: &str, opts: &SpriteOpts, overwrite: bool) -> Result<Imported, String> {
    if !valid_name(name) {
        return Err(format!("'{name}' no es un nombre válido (letras, números y _, sin empezar por número)"));
    }
    let img = image::open(src).map_err(|e| format!("{}: no se puede leer la imagen: {e}", src.display()))?.to_rgba8();
    let [x, y, w, h] = crop;
    if x + w > img.width() || y + h > img.height() || w == 0 || h == 0 {
        return Err(format!("{}: el recorte {x},{y} {w}×{h} se sale de la imagen", src.display()));
    }
    let part = image::imageops::crop_imm(&img, x, y, w, h).to_image();
    let mut bytes = vec![];
    part.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let folder = project.join("sprites");
    fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let old = asset_files_named(&folder, name);
    if !old.is_empty() && !overwrite {
        return Err(format!("ya existe un asset '{name}' en sprites/"));
    }
    write_asset(project, Kind::Sprite, &bytes, "png", name, opts, &old, src)
}

/// Rows of frames of a `fw`×`fh` grid in `src` that have something drawn: `(row, frames)`,
/// where `frames` counts up to the last cell with visible pixels.
pub fn sheet_rows(src: &Path, fw: u32, fh: u32) -> Result<Vec<(u32, u32)>, String> {
    let img = image::open(src).map_err(|e| format!("{}: no se puede leer la imagen: {e}", src.display()))?.to_rgba8();
    let (w, h) = img.dimensions();
    if fw == 0 || fh == 0 || fw > w || fh > h {
        return Err(format!("un frame de {fw}×{fh} no cabe en una imagen de {w}×{h}"));
    }
    let mut out = vec![];
    for r in 0..h / fh {
        let filled = |c: u32| (c * fw..(c + 1) * fw).any(|x| (r * fh..(r + 1) * fh).any(|y| img.get_pixel(x, y)[3] > 0));
        if let Some(last) = (0..w / fw).rev().find(|&c| filled(c)) {
            out.push((r, last + 1));
        }
    }
    Ok(out)
}

/// Imports `src` as the tile set `name`: the sheet becomes the sprite `sprite` (a loose PNG)
/// and `tilesets/<name>.toml` cuts it. `overwrite` replaces both when they exist.
pub fn import_tileset(project: &Path, src: &Path, name: &str, sprite: &str, t: &TileOpts, overwrite: bool) -> Result<Imported, String> {
    if !valid_name(name) {
        return Err(format!("'{name}' no es un nombre válido (letras, números y _, sin empezar por número)"));
    }
    if name == sprite {
        return Err(format!("el tile set y su sprite no pueden llamarse igual ('{name}')"));
    }
    let (w, h) = image_size(src)?;
    t.grid(w, h)?;
    let folder = project.join("tilesets");
    let old = asset_files_named(&folder, name);
    if !old.is_empty() && !overwrite {
        return Err(format!("ya existe un tile set '{name}' en tilesets/"));
    }
    let spr = import(project, Kind::Sprite, src, sprite, &SpriteOpts::default(), overwrite)?;
    fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    for f in &old {
        trash(project, f)?;
    }
    let path = folder.join(format!("{name}.toml"));
    fs::write(&path, tileset_toml(sprite, t)).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut files = spr.files;
    files.push(format!("tilesets/{name}.toml"));
    Ok(Imported { kind: Kind::Sprite, name: name.to_string(), files, replaced: spr.replaced || !old.is_empty(), src: src.to_path_buf() })
}

/// Width and height of an image file (decoding only the header when possible).
pub fn image_size(path: &Path) -> Result<(u32, u32), String> {
    image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| format!("{}: {e}", path.display()))?
        .into_dimensions()
        .map_err(|e| format!("{}: no se puede leer la imagen: {e}", path.display()))
}

/// Does the file look like what its extension says? (Images are checked by decoding them.)
pub fn header_problem(kind: Kind, path: &Path) -> Option<String> {
    use std::io::Read;
    let mut head = [0u8; 12];
    let n = fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let head = &head[..n];
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let ok = match (kind, ext.as_str()) {
        (Kind::Sound, "wav") => head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WAVE"),
        (Kind::Sound, "ogg") => head.starts_with(b"OggS"),
        (Kind::Font, _) => head.starts_with(&[0, 1, 0, 0]) || head.starts_with(b"OTTO") || head.starts_with(b"true") || head.starts_with(b"ttcf"),
        _ => true,
    };
    (!ok).then(|| format!("no parece un fichero .{ext} válido"))
}

// ------------------------------------------------------------------ names in use

/// What already has a name in a project. All assets, objects and rooms share one namespace.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    Asset(Kind),
    Tileset,
    /// An object, a room, a path, a shader...
    Other,
}

/// Every name in use in the project folder, read from disk (no image is decoded): asset
/// files by folder and `object Name` blocks of the scripts.
pub fn disk_owners(project: &Path) -> BTreeMap<String, Owner> {
    let mut m = BTreeMap::new();
    let stems = |dir: &str, exts: &[&str]| -> Vec<String> {
        let mut v = vec![];
        if let Ok(rd) = fs::read_dir(project.join(dir)) {
            for e in rd.flatten() {
                let p = e.path();
                let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                if p.is_file() && (exts.is_empty() || exts.contains(&ext.as_str())) {
                    if let Some(s) = p.file_stem() {
                        v.push(s.to_string_lossy().into_owned());
                    }
                }
            }
        }
        v
    };
    for dir in ["rooms", "notes", "paths", "timelines", "curves", "sequences", "particles"] {
        for n in stems(dir, &["toml", "md"]) {
            m.insert(n, Owner::Other);
        }
    }
    for n in stems("shaders", &["fsh", "vsh", "toml"]) {
        m.insert(n, Owner::Other);
    }
    for n in stems("tilesets", &["toml"]) {
        m.insert(n, Owner::Tileset);
    }
    for k in Kind::ALL {
        let mut exts = k.exts().to_vec();
        exts.push("toml");
        for n in stems(k.folder(), &exts) {
            m.insert(n, Owner::Asset(k));
        }
    }
    let mut scripts = vec![];
    crate::project::collect_scripts(project, project, &mut scripts).ok();
    for (_, text) in scripts {
        for l in text.lines() {
            if let Some(rest) = l.trim_start().strip_prefix("object ") {
                let n: String = rest.trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if !n.is_empty() {
                    m.entry(n).or_insert(Owner::Other);
                }
            }
        }
    }
    m
}

// ------------------------------------------------------------------ batch import (CLI)

/// What a planned import makes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum What {
    Sprite,
    Tileset,
    Sound,
    Font,
}

impl What {
    pub fn key(self) -> &'static str {
        match self {
            What::Sprite => "sprite",
            What::Tileset => "tileset",
            What::Sound => "sound",
            What::Font => "font",
        }
    }

    pub fn parse(s: &str) -> Option<What> {
        Some(match s.trim().to_lowercase().as_str() {
            "sprite" | "sprites" | "image" => What::Sprite,
            "tileset" | "tilesets" | "tile_set" | "tile-set" | "tiles" => What::Tileset,
            "sound" | "sounds" | "audio" | "sonido" => What::Sound,
            "font" | "fonts" | "fuente" => What::Font,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            What::Sprite => "sprite",
            What::Tileset => "tile set",
            What::Sound => "sonido",
            What::Font => "fuente",
        }
    }

    fn file_kind(self) -> Kind {
        match self {
            What::Sprite | What::Tileset => Kind::Sprite,
            What::Sound => Kind::Sound,
            What::Font => Kind::Font,
        }
    }
}

/// Options of `modengine import`; `None` means "decide per file".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    pub kind: Option<What>,
    /// Only with one file.
    pub name: Option<String>,
    pub frames: Option<Frames>,
    pub origin: Option<Origin>,
    pub fps: Option<f64>,
    pub tile: Option<[u32; 2]>,
    pub offset: [u32; 2],
    pub sep: [u32; 2],
    pub overwrite: bool,
    /// Names start with the subfolder they were found in (`enemies/slime.png` → `enemies_slime`).
    pub prefix_dirs: bool,
    /// Put in front of every name (`spr_`).
    pub name_prefix: String,
    /// Each row of a grid sheet becomes its own sprite (`<name>_<row name>`).
    pub split_rows: bool,
    /// Names of the rows for `split_rows` (`idle`, `walk`...); missing ones are `row<N>`.
    pub row_names: Vec<String>,
    /// Only this part `[x, y, w, h]` of the image (one file): a sprite out of an atlas.
    pub crop: Option<[u32; 4]>,
}

/// One file to import.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub src: PathBuf,
    pub what: What,
    pub name: String,
    /// Tile sets: the sprite with the sheet.
    pub sprite: Option<String>,
    pub opts: SpriteOpts,
    pub tile: Option<TileOpts>,
    /// Image size.
    pub size: Option<(u32, u32)>,
    /// Replaces an existing asset of this name (`--overwrite`).
    pub replace: bool,
    /// Why the frames/tiles are what they are (`nombre: _strip4`).
    pub why: String,
    /// A sprite imported as one frame that looks like a sheet: the options that would cut it.
    pub hint: String,
    /// One row of a sheet (`--split-rows`): `[x, y, w, h]` cut out of the image.
    pub crop: Option<[u32; 4]>,
    pub warnings: Vec<String>,
}

impl Item {
    /// The project files importing it writes.
    pub fn files(&self) -> Vec<String> {
        let ext = self.src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        match self.what {
            What::Sprite => {
                let ext = if self.crop.is_some() { "png".to_string() } else { ext };
                let mut v = vec![format!("sprites/{}.{ext}", self.name)];
                if sprite_toml("x", &self.opts).is_some() {
                    v.push(format!("sprites/{}.toml", self.name));
                }
                v
            }
            What::Tileset => vec![format!("sprites/{}.{ext}", self.sprite.as_deref().unwrap_or("")), format!("tilesets/{}.toml", self.name)],
            What::Sound => vec![format!("sounds/{}.{ext}", self.name)],
            What::Font => vec![format!("fonts/{}.{ext}", self.name)],
        }
    }

    /// `(frames, frame w, frame h)` of a sprite.
    pub fn slice(&self) -> Option<(u32, u32, u32)> {
        let (w, h) = match self.crop {
            Some([_, _, w, h]) => (w, h),
            None => self.size?,
        };
        match self.what {
            What::Sprite => self.opts.slice(w, h).ok(),
            What::Tileset => self.tile.and_then(|t| t.grid(w, h).ok().map(|(c, r)| (c * r, t.tile[0], t.tile[1]))),
            _ => None,
        }
    }
}

/// What `modengine import` is going to do.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub project: PathBuf,
    pub items: Vec<Item>,
    /// Files left out and why.
    pub skipped: Vec<(PathBuf, String)>,
    /// Licence / readme / credits files found next to the art.
    pub license_files: Vec<PathBuf>,
    /// Nothing is imported while there is one.
    pub errors: Vec<String>,
}

/// Junk that archives and OSes leave next to art.
pub fn is_junk(name: &str) -> bool {
    let l = name.to_lowercase();
    l.starts_with('.') || l == "thumbs.db" || l == "desktop.ini" || l == "__macosx" || l.ends_with('~')
}

/// `LICENSE`, `license.txt`, `README.md`, `Credits.txt`, `COPYING`, `attribution.txt`...
pub fn is_license_file(name: &str) -> bool {
    let l = name.to_lowercase();
    let stem = l.split('.').next().unwrap_or("");
    let text_ext = !l.contains('.') || [".txt", ".md", ".html", ".htm", ".rtf"].iter().any(|e| l.ends_with(e));
    text_ext && ["license", "licence", "readme", "credits", "credit", "copying", "attribution", "authors", "legal"].iter().any(|k| stem.contains(k))
}

/// Every file under `dir` (sorted, junk skipped), with the subfolders it is in.
fn walk(dir: &Path, rel: &[String], out: &mut Vec<(PathBuf, Vec<String>)>, depth: u32) {
    if depth > 16 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if is_junk(&name) {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            let mut r = rel.to_vec();
            r.push(name);
            walk(&p, &r, out, depth + 1);
        } else if p.is_file() {
            out.push((p, rel.to_vec()));
        }
    }
}

/// Frames for a sprite from its file name (`_strip4`, `_32x32`, `_4` with square frames).
/// Returns the name without the suffix, the frames and why.
pub fn frames_from_name(stem: &str, w: u32, h: u32) -> (String, Frames, String) {
    if let Some((base, n)) = strip_suffix(stem) {
        if n <= w {
            return (base.to_string(), Frames::Strip(n), format!("nombre: _strip{n}"));
        }
    }
    if let Some((base, fw, fh)) = size_suffix(stem) {
        if fw <= w && fh <= h && w % fw == 0 && h % fh == 0 && (fw, fh) != (w, h) {
            let frames = if fh == h { Frames::Strip(w / fw) } else { Frames::Grid(fw, fh) };
            return (base.to_string(), frames, format!("nombre: _{fw}x{fh}"));
        }
    }
    if let Some((base, n)) = count_suffix(stem) {
        if w % n == 0 && w / n == h {
            return (base.to_string(), Frames::Strip(n), format!("nombre: _{n} con frames cuadrados"));
        }
    }
    (stem.to_string(), Frames::Single, String::new())
}

/// Tile size for a tile set without `--tile`: from the name (`_16x16`), else 16, 32 or 8 px
/// when they divide the image.
pub fn tile_from_name(stem: &str, w: u32, h: u32) -> Option<([u32; 2], String)> {
    if let Some((_, tw, th)) = size_suffix(stem) {
        if tw <= w && th <= h {
            return Some(([tw, th], format!("nombre: _{tw}x{th}")));
        }
    }
    [16, 32, 8, 24, 48, 64].into_iter().find(|t| w % t == 0 && h % t == 0 && w >= *t && h >= *t).map(|t| ([t, t], format!("{t}×{t} divide la imagen")))
}

/// Works out what importing `inputs` (files and folders) into `project` with `o` does,
/// without touching anything.
pub fn plan(project: &Path, inputs: &[PathBuf], o: &Options) -> Plan {
    let mut plan = Plan { project: project.to_path_buf(), ..Default::default() };
    if !project.join("game.toml").is_file() {
        plan.errors.push(format!("{} no es un proyecto de Mod Engine (falta game.toml)", project.display()));
        return plan;
    }
    // (file, subfolders, given directly)
    let mut files: Vec<(PathBuf, Vec<String>, bool)> = vec![];
    for i in inputs {
        if i.is_dir() {
            let mut v = vec![];
            walk(i, &[], &mut v, 0);
            files.extend(v.into_iter().map(|(p, r)| (p, r, false)));
        } else if i.is_file() {
            files.push((i.clone(), vec![], true));
        } else {
            plan.errors.push(format!("no existe: {}", i.display()));
        }
    }
    if o.crop.is_some() && (files.iter().filter(|f| Kind::of(&f.0).is_some()).count() != 1 || inputs.len() != 1) {
        plan.errors.push("--crop solo vale para importar un fichero".into());
    }
    if o.name.is_some() && (files.iter().filter(|f| Kind::of(&f.0).is_some()).count() != 1 || inputs.len() != 1) {
        plan.errors.push("--name solo vale para importar un fichero".into());
    }
    let project_real = project.canonicalize().unwrap_or(project.to_path_buf());
    let owners = disk_owners(project);
    let mut planned: BTreeSet<String> = BTreeSet::new();
    for (src, dirs, direct) in files {
        let file_name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if src.canonicalize().is_ok_and(|p| p.starts_with(&project_real)) && !direct {
            plan.skipped.push((src, "ya está dentro del proyecto".into()));
            continue;
        }
        let Some(kind) = Kind::of(&src) else {
            if is_license_file(&file_name) {
                plan.license_files.push(src);
            } else {
                let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_else(|| "sin extensión".into());
                let msg = format!("formato no soportado ({ext}): usa PNG, WAV, OGG, TTF u OTF");
                if direct {
                    plan.errors.push(format!("{}: {msg}", src.display()));
                } else {
                    plan.skipped.push((src, msg));
                }
            }
            continue;
        };
        let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut what = match kind {
            Kind::Sound => What::Sound,
            Kind::Font => What::Font,
            Kind::Sprite if o.tile.is_some() || looks_like_tileset(&stem) => What::Tileset,
            Kind::Sprite => What::Sprite,
        };
        if let Some(k) = o.kind {
            if k.file_kind() != kind {
                let msg = format!("no es un fichero de {} (--kind {})", k.label(), k.key());
                if direct {
                    plan.errors.push(format!("{}: {msg}", src.display()));
                } else {
                    plan.skipped.push((src, msg));
                }
                continue;
            }
            what = k;
        }
        let mut item = Item { src: src.clone(), what, name: String::new(), sprite: None, opts: SpriteOpts::default(), tile: None, size: None, replace: false, why: String::new(), hint: String::new(), crop: None, warnings: vec![] };
        let bytes = fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
        if bytes > BIG_FILE {
            item.warnings.push(format!("fichero grande ({}): el juego tardará más en cargar", human_size(bytes)));
        }
        let mut base = stem.clone();
        match what {
            What::Sound | What::Font => {
                if let Some(e) = header_problem(kind, &src) {
                    plan.errors.push(format!("{}: {e}", src.display()));
                    continue;
                }
            }
            What::Sprite | What::Tileset => {
                let (w, h) = match image::open(&src) {
                    Ok(img) => (img.width(), img.height()),
                    Err(e) => {
                        plan.errors.push(format!("{}: no se puede leer la imagen: {e}", src.display()));
                        continue;
                    }
                };
                item.size = Some((w, h));
                let (w, h) = match o.crop {
                    Some([x, y, cw, ch]) if what == What::Sprite => {
                        if cw == 0 || ch == 0 || x + cw > w || y + ch > h {
                            plan.errors.push(format!("{}: --crop {x},{y},{cw},{ch} se sale de la imagen ({w}×{h})", src.display()));
                            continue;
                        }
                        item.crop = Some([x, y, cw, ch]);
                        (cw, ch)
                    }
                    Some(_) => {
                        plan.errors.push("--crop solo vale para sprites".into());
                        continue;
                    }
                    None => (w, h),
                };
                if what == What::Sprite {
                    let (b, frames, why) = frames_from_name(&stem, w, h);
                    base = b;
                    item.opts = SpriteOpts { frames: o.frames.unwrap_or(frames), origin: o.origin.unwrap_or(Origin::TopLeft), fps: o.fps.unwrap_or(DEFAULT_FPS) };
                    item.why = if o.frames.is_some() { "--frames/--grid".into() } else { why };
                    match item.opts.slice(w, h) {
                        Ok(_) => {}
                        Err(e) => {
                            plan.errors.push(format!("{}: {e}", src.display()));
                            continue;
                        }
                    }
                    if let Some(l) = item.opts.leftover(w, h) {
                        item.warnings.push(l);
                    }
                    if item.opts.frames == Frames::Single && item.crop.is_none() {
                        if let Some(l) = inspect(&src).ok().and_then(|i| i.layouts.into_iter().find(|l| l.kind != "single" && l.confidence >= 0.6)) {
                            item.hint = format!("{} ({}): {}", l.args, l.kind, l.reason);
                        }
                    }
                } else {
                    if let Some((b, _, _)) = size_suffix(&stem) {
                        base = b.to_string();
                    }
                    let (tile, why) = match o.tile {
                        Some(t) => (t, "--tile".to_string()),
                        None => match tile_from_name(&stem, w, h) {
                            Some(t) => t,
                            None => {
                                plan.errors.push(format!("{}: no sé el tamaño de los tiles de una imagen de {w}×{h}: usa --tile WxH", src.display()));
                                continue;
                            }
                        },
                    };
                    let t = TileOpts { tile, offset: o.offset, sep: o.sep };
                    if let Err(e) = t.grid(w, h) {
                        plan.errors.push(format!("{}: {e}", src.display()));
                        continue;
                    }
                    item.tile = Some(t);
                    item.why = why;
                }
            }
        }
        // the name: given, or the file's (with its folders when asked), cleaned up
        let explicit = o.name.clone();
        let mut name = match &explicit {
            Some(n) => n.clone(),
            None => {
                let mut parts: Vec<String> = if o.prefix_dirs { dirs.iter().map(|d| sanitize(d)).collect() } else { vec![] };
                parts.push(sanitize(&base));
                sanitize(&format!("{}{}", o.name_prefix, parts.join("_")))
            }
        };
        if what == What::Tileset && explicit.is_none() && !name.starts_with("ts_") {
            name = format!("ts_{}", name.trim_start_matches("tileset_").trim_start_matches("spr_"));
        }
        // --split-rows: every row of the grid is a sprite of its own
        let mut variants = vec![(item, name)];
        if o.split_rows && what == What::Sprite {
            let (mut item, base) = variants.pop().unwrap_or_else(|| unreachable!());
            match item.opts.frames {
                Frames::Grid(_, _) if item.crop.is_some() => plan.errors.push("--split-rows y --crop no van juntos".into()),
                Frames::Grid(fw, fh) => match sheet_rows(&src, fw, fh) {
                    Ok(rows) => {
                        for (r, n) in rows {
                            let row = o.row_names.get(r as usize).map(|s| s.trim().to_string()).unwrap_or_default();
                            if row == "-" {
                                continue;
                            }
                            let row = if row.is_empty() { format!("row{}", r + 1) } else { sanitize(&row) };
                            item.crop = Some([0, r * fh, n * fw, fh]);
                            item.opts.frames = if n > 1 { Frames::Strip(n) } else { Frames::Single };
                            item.why = format!("fila {} de la rejilla de {fw}×{fh}", r + 1);
                            item.hint.clear();
                            variants.push((item.clone(), format!("{base}_{row}")));
                        }
                    }
                    Err(e) => plan.errors.push(format!("{}: {e}", src.display())),
                },
                _ => plan.errors.push(format!("{}: --split-rows necesita una rejilla de frames (--grid WxH)", src.display())),
            }
        }
        for (mut item, mut name) in variants {
        if !valid_name(&name) {
            plan.errors.push(format!("'{name}' no es un nombre válido (letras, números y _, sin empezar por número)"));
            continue;
        }
        let same_kind = |n: &str, w: What| match (owners.get(n), w) {
            (Some(Owner::Tileset), What::Tileset) => true,
            (Some(Owner::Asset(k)), w) if w != What::Tileset => *k == w.file_kind(),
            _ => false,
        };
        let free = |n: &str, w: What| !planned.contains(n) && (!owners.contains_key(n) || (o.overwrite && same_kind(n, w)));
        if explicit.is_some() {
            if !free(&name, what) {
                let msg = if planned.contains(&name) {
                    format!("dos ficheros se llamarían '{name}'")
                } else if same_kind(&name, what) {
                    format!("ya existe un {} '{name}' (usa --overwrite para sustituirlo)", what.label())
                } else {
                    format!("el nombre '{name}' ya lo usa otro asset, objeto o room (todos comparten nombres)")
                };
                plan.errors.push(msg);
                continue;
            }
        } else {
            name = unique(&name, |n| !free(n, what));
        }
        item.replace = owners.contains_key(&name);
        if what == What::Tileset {
            let mut spr = tileset_sprite_name(&name);
            if !free(&spr, What::Sprite) || spr == name {
                spr = unique(&spr, |n| n == name || !free(n, What::Sprite));
            }
            item.replace |= owners.contains_key(&spr);
            planned.insert(spr.clone());
            item.sprite = Some(spr);
        }
        planned.insert(name.clone());
        item.name = name;
        plan.items.push(item);
        }
    }
    if plan.items.is_empty() && plan.errors.is_empty() {
        plan.errors.push("no hay nada que importar (PNG, WAV, OGG, TTF u OTF)".into());
    }
    plan
}

/// What [`execute`] did.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// `(planned item, files written, replaced)`.
    pub done: Vec<(Item, Vec<String>, bool)>,
    pub errors: Vec<String>,
    /// CREDITS.md was written.
    pub credits: Option<PathBuf>,
}

/// Imports the plan (nothing when it has errors); items marked `replace` send the asset they
/// replace to `.trash`. With `credit`, the files are credited in
/// CREDITS.md under the folder they came from.
pub fn execute(plan: &Plan, credit: Option<&str>) -> Report {
    let mut r = Report::default();
    if !plan.errors.is_empty() {
        r.errors = plan.errors.clone();
        return r;
    }
    for it in &plan.items {
        let res = match it.what {
            What::Tileset => import_tileset(&plan.project, &it.src, &it.name, it.sprite.as_deref().unwrap_or(""), &it.tile.unwrap_or(TileOpts::new(16, 16)), it.replace),
            What::Sprite if it.crop.is_some() => import_crop(&plan.project, &it.src, it.crop.unwrap_or_default(), &it.name, &it.opts, it.replace),
            w => import(&plan.project, w.file_kind(), &it.src, &it.name, &it.opts, it.replace),
        };
        match res {
            Ok(imp) => r.done.push((it.clone(), imp.files, imp.replaced)),
            Err(e) => r.errors.push(format!("{}: {e}", it.src.display())),
        }
    }
    if let Some(text) = credit.filter(|t| !t.trim().is_empty()) {
        // grouped by the folder each file came from
        let mut groups: BTreeMap<PathBuf, Vec<(String, String)>> = BTreeMap::new();
        for (it, files, _) in &r.done {
            let dir = it.src.parent().map(Path::to_path_buf).unwrap_or_default();
            let src = it.src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            groups.entry(dir).or_default().extend(files.iter().map(|f| (f.clone(), src.clone())));
        }
        for (dir, files) in groups {
            let title = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string());
            let c = crate::credits::LocalCredit { slug: sanitize(&title), title, text: text.to_string() };
            match crate::credits::write_local(&plan.project, &c, &files) {
                Ok(Some(p)) => r.credits = Some(p),
                Ok(None) => {}
                Err(e) => r.errors.push(e),
            }
        }
    }
    r
}

// ------------------------------------------------------------------ looking at an image

/// A way to cut an image into frames.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// `single`, `strip`, `grid` or `tileset`.
    pub kind: &'static str,
    pub frames: u32,
    pub frame_w: u32,
    pub frame_h: u32,
    /// How sure: 0..1.
    pub confidence: f32,
    pub reason: String,
    /// The `modengine import` options for it.
    pub args: String,
}

/// What [`inspect`] finds out about an image.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageInfo {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// Some pixel is (partly) transparent.
    pub transparency: bool,
    /// Different colours among the visible pixels (capped at 4097: "more than 4096").
    pub colors: usize,
    /// Content box `[x, y, w, h]` of the visible pixels.
    pub content: [u32; 4],
    /// Looks like a tile set by its name.
    pub tileset_name: bool,
    /// Most likely first.
    pub layouts: Vec<Layout>,
    /// Separate drawings `[x, y, w, h]`, row by row (for atlases: pick one with `--crop`).
    pub blobs: Vec<[u32; 4]>,
}

/// Runs of `true` in `v`: `(start, end)` exclusive.
fn runs(v: &[bool]) -> Vec<(u32, u32)> {
    let mut out = vec![];
    let mut start = None;
    for (i, &b) in v.iter().enumerate() {
        match (b, start) {
            (true, None) => start = Some(i as u32),
            (false, Some(s)) => {
                out.push((s, i as u32));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, v.len() as u32));
    }
    out
}

/// The cell size along one axis that puts every run of content in a cell of its own, in
/// each band (row of frames) separately: the largest such size that makes two cells or more.
/// Cells may stay empty (an animation shorter than its row), and a quarter of the bands may
/// disagree (frames that touch, effects that overflow their cell).
fn cell_size(len: u32, bands: &[Vec<(u32, u32)>]) -> Option<u32> {
    let used: Vec<&Vec<(u32, u32)>> = bands.iter().filter(|b| !b.is_empty()).collect();
    if !used.iter().any(|b| b.len() >= 2) {
        return None;
    }
    let need = (used.len() * 3).div_ceil(4).max(1);
    (2..len).rev().filter(|c| len % c == 0).find(|&c| {
        let fits = |runs: &&&Vec<(u32, u32)>| {
            let mut seen = BTreeSet::new();
            runs.iter().all(|&(a, b)| a / c == (b - 1) / c && seen.insert(a / c))
        };
        let ok: Vec<&&Vec<(u32, u32)>> = used.iter().filter(|r| fits(r)).collect();
        ok.len() >= need && ok.iter().any(|r| r.len() >= 2)
    })
}

/// Size, colours and the likely ways to cut an image into frames (transparent gutters between
/// sprites, square frames along the width, the file name, tile sizes).
pub fn inspect(path: &Path) -> Result<ImageInfo, String> {
    let img = image::open(path).map_err(|e| format!("{}: no se puede leer la imagen: {e}", path.display()))?.to_rgba8();
    let (w, h) = img.dimensions();
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut cols = vec![false; w as usize];
    let mut rows = vec![false; h as usize];
    let mut transparency = false;
    let mut colors: BTreeSet<[u8; 4]> = BTreeSet::new();
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] < 255 {
            transparency = true;
        }
        if p[3] > 0 {
            cols[x as usize] = true;
            rows[y as usize] = true;
            if colors.len() <= 4096 {
                colors.insert(p.0);
            }
        }
    }
    let (cr, rr) = (runs(&cols), runs(&rows));
    // columns with something visible between rows y0..y1
    let cols_in = |y0: u32, y1: u32| -> Vec<(u32, u32)> {
        let v: Vec<bool> = (0..w).map(|x| (y0..y1).any(|y| img.get_pixel(x, y)[3] > 0)).collect();
        runs(&v)
    };
    let content = match (cr.first(), cr.last(), rr.first(), rr.last()) {
        (Some(c0), Some(c1), Some(r0), Some(r1)) => [c0.0, r0.0, c1.1 - c0.0, r1.1 - r0.0],
        _ => [0, 0, 0, 0],
    };
    let mut layouts: Vec<Layout> = vec![];
    let mut push = |l: Layout| {
        if !layouts.iter().any(|o| o.kind == l.kind && o.frame_w == l.frame_w && o.frame_h == l.frame_h) {
            layouts.push(l);
        }
    };
    let strip = |n: u32, fw: u32, confidence: f32, reason: String| Layout { kind: "strip", frames: n, frame_w: fw, frame_h: h, confidence, reason, args: format!("--frames {n}") };
    let grid = |fw: u32, fh: u32, confidence: f32, reason: String| Layout {
        kind: if fh == h { "strip" } else { "grid" },
        frames: (w / fw) * (h / fh),
        frame_w: fw,
        frame_h: fh,
        confidence,
        reason,
        args: if fh == h { format!("--frames {}", w / fw) } else { format!("--grid {fw}x{fh}") },
    };
    let tileset_name = looks_like_tileset(&stem);
    // 1. the file name says
    let (_, by_name, why) = frames_from_name(&stem, w, h);
    match by_name {
        Frames::Strip(n) if n > 1 => push(strip(n, w / n, 0.95, why)),
        Frames::Grid(fw, fh) => push(grid(fw, fh, 0.9, why)),
        _ => {}
    }
    // 2. transparent gutters split the sheet into a uniform grid
    let ch = cell_size(h, &[rr.clone()]);
    let bands: Vec<Vec<(u32, u32)>> = match ch {
        Some(c) => (0..h / c).map(|r| cols_in(r * c, (r + 1) * c)).collect(),
        None => vec![cr.clone()],
    };
    let cw = cell_size(w, &bands);
    if cw.is_some() || ch.is_some() {
        let fw = cw.unwrap_or(w);
        let fh = ch.unwrap_or(h);
        let (c, r) = (w / fw, h / fh);
        push(grid(fw, fh, 0.85, format!("huecos transparentes: {c}×{r} celdas de {fw}×{fh} px, cada dibujo en su celda")));
    }
    // 3. square frames side by side (or stacked)
    if w > h && h > 0 && w % h == 0 {
        push(strip(w / h, h, if transparency { 0.6 } else { 0.4 }, format!("{} frames cuadrados de {h}×{h} a lo ancho", w / h)));
    } else if h > w && w > 0 && h % w == 0 {
        push(grid(w, w, 0.45, format!("{} frames cuadrados de {w}×{w} en vertical", h / w)));
    }
    // 4. a tile set: opaque sheets, or the name says so
    for t in [16u32, 32, 8] {
        if w % t == 0 && h % t == 0 && (w / t) * (h / t) >= 4 && (tileset_name || !transparency || (w / t >= 4 && h / t >= 4)) {
            let conf = if tileset_name { 0.8 } else if !transparency { 0.5 } else { 0.3 };
            push(Layout {
                kind: "tileset",
                frames: (w / t) * (h / t),
                frame_w: t,
                frame_h: t,
                confidence: conf,
                reason: format!("rejilla de tiles de {t}×{t} ({}×{}){}", w / t, h / t, if tileset_name { ", y el nombre dice tile set" } else { "" }),
                args: format!("--kind tileset --tile {t}x{t}"),
            });
            break;
        }
    }
    // every drawing separated by transparent space, band by band (at most 256)
    let mut blobs = vec![];
    for &(y0, y1) in rr.iter() {
        for (x0, x1) in cols_in(y0, y1) {
            let ys: Vec<u32> = (y0..y1).filter(|&y| (x0..x1).any(|x| img.get_pixel(x, y)[3] > 0)).collect();
            if let (Some(a), Some(b)) = (ys.first(), ys.last()) {
                blobs.push([x0, *a, x1 - x0, b + 1 - a]);
            }
            if blobs.len() >= 256 {
                break;
            }
        }
    }
    if !transparency {
        blobs.clear();
    }
    let alone = layouts.is_empty();
    layouts.push(Layout { kind: "single", frames: 1, frame_w: w, frame_h: h, confidence: if alone { 0.7 } else { 0.2 }, reason: "la imagen entera es un frame".into(), args: String::new() });
    layouts.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    Ok(ImageInfo { path: path.to_path_buf(), width: w, height: h, transparency, colors: colors.len(), content, tileset_name, layouts, blobs })
}

/// Images under `path` (a file or a folder, recursively), junk skipped.
pub fn images_in(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let mut v = vec![];
    walk(path, &[], &mut v, 0);
    v.into_iter().map(|(p, _)| p).filter(|p| Kind::of(p) == Some(Kind::Sprite)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_suffixes() {
        assert_eq!(strip_suffix("hero_run_strip4"), Some(("hero_run", 4)));
        assert_eq!(strip_suffix("Hero-Strip12"), Some(("Hero", 12)));
        assert_eq!(strip_suffix("strip4"), None);
        assert_eq!(size_suffix("tiles_16x16"), Some(("tiles", 16, 16)));
        assert_eq!(size_suffix("Hero 32X48"), Some(("Hero", 32, 48)));
        assert_eq!(size_suffix("x16"), None);
        assert_eq!(count_suffix("coin_8"), Some(("coin", 8)));
        assert_eq!(count_suffix("coin_1"), None);
        assert_eq!(frames_from_name("coin_8", 64, 8).1, Frames::Strip(8));
        assert_eq!(frames_from_name("tree_2", 30, 40), ("tree_2".into(), Frames::Single, String::new()), "not square frames: a name");
        assert_eq!(frames_from_name("bat_16x16", 64, 32).1, Frames::Grid(16, 16));
        assert_eq!(frames_from_name("bat_16x16", 64, 16).1, Frames::Strip(4));
        assert!(looks_like_tileset("Dungeon Tiles") && looks_like_tileset("ts_cave") && looks_like_tileset("tileset16") && !looks_like_tileset("tilted_hero"));
        assert!(is_license_file("LICENSE") && is_license_file("License.txt") && is_license_file("README.md") && !is_license_file("readme.png"));
        assert!(is_junk(".DS_Store") && is_junk("Thumbs.db") && is_junk("__MACOSX") && !is_junk("hero.png"));
        assert_eq!(Origin::parse("bottom-center"), Some(Origin::Bottom));
        assert_eq!(Origin::parse("top-left"), Some(Origin::TopLeft));
        assert_eq!(Origin::parse("8, 15"), Some(Origin::At(8, 15)));
        assert_eq!(Origin::parse("nope"), None);
        let at = SpriteOpts { origin: Origin::At(8, 15), ..Default::default() };
        assert_eq!(sprite_toml("a.png", &at).unwrap(), "image = \"a.png\"\norigin = [8, 15]\n");
    }

    #[test]
    fn gutters_find_cells() {
        // four 10-px blobs in 16-px cells
        assert_eq!(cell_size(64, &[vec![(3, 13), (19, 29), (35, 45), (51, 61)]]), Some(16));
        // one blob: no grid
        assert_eq!(cell_size(64, &[vec![(3, 60)]]), None);
        // blobs that don't fit a uniform grid
        assert_eq!(cell_size(64, &[vec![(0, 30), (31, 33)]]), None);
        // rows of different lengths: the cells come from the longest
        assert_eq!(cell_size(96, &[vec![(2, 30), (34, 62), (66, 94)], vec![(2, 30)]]), Some(32));
    }
}

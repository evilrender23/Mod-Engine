//! Reading a project folder from disk.
//!
//! ```text
//! mygame/
//!   game.toml            name, resolution, room order, palette
//!   sprites/*.toml|png   images (PNG strips/grids or inline pixel art)
//!   sounds/*.toml|wav|ogg  audio files or synthesized effects
//!   fonts/*.toml|ttf
//!   rooms/*.toml         levels: size, layers, instances, ASCII maps
//!   **/*.ms              ModScript: objects, functions, init code
//! ```

use crate::draw::parse_color;
use crate::synth::{render_wav, SynthDef};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------ game.toml

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct GameToml {
    pub game: GameSection,
    pub display: DisplaySection,
    /// Default characters for inline pixel-art sprites: `"r" = "#ff0000"`.
    pub palette: BTreeMap<String, String>,
    /// Options for `modengine export` (Archivo → Exportar…).
    pub export: ExportSection,
    /// Android package options (APK export, touch controls).
    pub android: AndroidSection,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct GameSection {
    pub name: String,
    pub start_room: Option<String>,
    /// Room order used by next_room()/prev_room(). Defaults to alphabetical.
    pub rooms: Vec<String>,
    /// Version shown by exported games (`1.0.0`).
    pub version: String,
    /// PNG inside the project used as the window/executable/app icon.
    pub icon: Option<String>,
}

impl Default for GameSection {
    fn default() -> Self {
        GameSection { name: "Mod Engine game".into(), start_room: None, rooms: vec![], version: "1.0.0".into(), icon: None }
    }
}

/// `[export]`: how `modengine export` packages the game.
#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ExportSection {
    /// File name of the executable (default: the game name).
    pub exe_name: Option<String>,
    /// Output folder (relative to the project; default `export/<target>`).
    pub out_dir: Option<String>,
    /// Last target exported from the editor: "linux", "windows" or "android".
    pub target: Option<String>,
    /// Windows: keep a console window (useful to see `show_debug_message` output).
    pub windows_console: bool,
}

/// `[android]`: APK options.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct AndroidSection {
    /// Application id, e.g. `com.example.mygame` (default: `com.modengine.<name>`).
    pub package: Option<String>,
    /// Increase for every release uploaded to a store.
    pub version_code: u32,
    /// "landscape", "portrait" or "sensor".
    pub orientation: String,
    /// On-screen direction pad: "none", "arrows" (left/right) or "dpad" (4 directions).
    pub virtual_pad: String,
    /// On-screen buttons, bottom right, each pressing a key: `["space", "z"]`.
    pub virtual_buttons: Vec<String>,
    /// CPU architectures to include: "arm64-v8a", "armeabi-v7a", "x86_64".
    pub abis: Vec<String>,
}

impl Default for AndroidSection {
    fn default() -> Self {
        AndroidSection {
            package: None,
            version_code: 1,
            orientation: "landscape".into(),
            virtual_pad: "none".into(),
            virtual_buttons: vec![],
            abis: vec!["arm64-v8a".into()],
        }
    }
}

impl GameToml {
    /// The Android application id (configured or derived from the game name).
    pub fn android_package(&self) -> String {
        match self.android.package.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => p.to_string(),
            None => {
                let mut id: String = crate::vfs::safe_id(&self.game.name).chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if id.is_empty() || id.starts_with(|c: char| c.is_ascii_digit() || c == '_') {
                    id = format!("game{id}");
                }
                format!("com.modengine.{id}")
            }
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct DisplaySection {
    /// Game resolution (the camera size).
    pub width: u32,
    pub height: u32,
    /// Initial window size multiplier.
    pub scale: u32,
    /// Only scale the game screen by whole numbers (crisp pixels).
    pub pixel_perfect: bool,
    pub fullscreen: bool,
    /// Logic steps per second (rooms may override with `speed`).
    pub step_rate: f64,
    /// Colour of the bars around the game screen.
    pub letterbox: String,
    /// Default text size in pixels.
    pub font_size: f32,
    /// Smooth (linear) filtering instead of nearest-neighbour.
    pub smooth: bool,
}

impl Default for DisplaySection {
    fn default() -> Self {
        DisplaySection {
            width: 320,
            height: 180,
            scale: 3,
            pixel_perfect: true,
            fullscreen: false,
            step_rate: 60.0,
            letterbox: "#000000".into(),
            font_size: 16.0,
            smooth: false,
        }
    }
}

// ------------------------------------------------------------------ assets

pub struct Sprite {
    pub name: String,
    /// Size of one frame.
    pub fw: u32,
    pub fh: u32,
    /// Frame rectangles (x, y, w, h) inside the sheet.
    pub frames: Vec<[u32; 4]>,
    pub sheet_w: u32,
    pub sheet_h: u32,
    pub rgba: Vec<u8>,
    pub origin: (f64, f64),
    /// Collision box in frame pixels: left, top, right (exclusive), bottom (exclusive).
    pub bbox: [f64; 4],
    /// Animation frames per second (at anim_speed = 1).
    pub fps: f64,
}

pub struct SoundAsset {
    pub name: String,
    pub data: Vec<u8>,
    pub volume: f64,
}

pub use crate::font::FontAsset;
pub use crate::tileset::TileSetAsset;

#[derive(Deserialize, Default)]
#[serde(default)]
struct SpriteToml {
    image: Option<String>,
    frames: Option<u32>,
    frame_size: Option<[u32; 2]>,
    origin: Option<toml::Value>,
    fps: Option<f64>,
    mask: Option<[i32; 4]>,
    palette: BTreeMap<String, String>,
    pixels: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SoundToml {
    file: Option<String>,
    synth: Option<SynthDef>,
    volume: Option<f64>,
}

// ------------------------------------------------------------------ rooms (raw)

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct RoomToml {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub background: Option<String>,
    pub speed: Option<f64>,
    pub persistent: bool,
    /// ModScript run once after the room's instances are created.
    pub code: Option<String>,
    pub camera: CameraToml,
    #[serde(rename = "layer")]
    pub layers: Vec<LayerToml>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct CameraToml {
    pub follow: Option<String>,
    /// 1 = snap to the target, 0.1 = smooth.
    pub smooth: f64,
    pub x: f64,
    pub y: f64,
}

impl Default for CameraToml {
    fn default() -> Self {
        CameraToml { follow: None, smooth: 1.0, x: 0.0, y: 0.0 }
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct LayerToml {
    pub name: String,
    /// "instances", "tiles" or "background"
    #[serde(rename = "type")]
    pub kind: String,
    pub depth: f64,
    pub visible: bool,
    pub alpha: f64,
    // background
    pub sprite: Option<String>,
    pub color: Option<String>,
    pub tile_x: bool,
    pub tile_y: bool,
    pub stretch: bool,
    pub hspeed: f64,
    pub vspeed: f64,
    pub parallax: Option<toml::Value>,
    pub x: f64,
    pub y: f64,
    // tiles / instance maps
    pub tileset: Option<String>,
    pub cell: Option<[u32; 2]>,
    pub legend: BTreeMap<String, toml::Value>,
    pub map: Option<String>,
    /// Tile layers: rows of tile data separated by spaces (`-1` or `.` = empty), an
    /// alternative to `legend` + `map` for big tile sets.
    pub data: Option<String>,
    // instances
    pub instances: Vec<InstToml>,
}

impl Default for LayerToml {
    fn default() -> Self {
        LayerToml {
            name: String::new(),
            kind: "instances".into(),
            depth: 0.0,
            visible: true,
            alpha: 1.0,
            sprite: None,
            color: None,
            tile_x: false,
            tile_y: false,
            stretch: false,
            hspeed: 0.0,
            vspeed: 0.0,
            parallax: None,
            x: 0.0,
            y: 0.0,
            tileset: None,
            cell: None,
            legend: BTreeMap::new(),
            map: None,
            data: None,
            instances: vec![],
        }
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct InstToml {
    pub object: String,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub vars: toml::Table,
}

// ------------------------------------------------------------------ project

pub struct Project {
    pub dir: PathBuf,
    pub config: GameToml,
    pub sprites: Vec<Sprite>,
    pub sounds: Vec<SoundAsset>,
    pub fonts: Vec<FontAsset>,
    pub tilesets: Vec<TileSetAsset>,
    pub rooms: Vec<(String, PathBuf, RoomToml)>,
    pub scripts: Vec<(String, String)>,
}

fn read_text(p: &Path) -> Result<String, String> {
    crate::vfs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn read_toml<T: for<'de> Deserialize<'de>>(p: &Path) -> Result<T, String> {
    let text = read_text(p)?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))
}

fn list_dir(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = match crate::vfs::read_dir(dir) {
        Ok(rd) => rd.into_iter().filter(|(_, is_dir)| !is_dir).map(|(p, _)| p).collect(),
        Err(_) => vec![],
    };
    v.sort();
    v
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

fn ext(p: &Path) -> String {
    p.extension().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Loose files without a .toml (e.g. `sprites/hero.png`) are assets too,
/// unless a .toml with the same name describes them.
pub fn asset_files(dir: &Path, exts: &[&str]) -> Vec<(String, PathBuf, bool)> {
    let files = list_dir(dir);
    let tomls: Vec<String> = files.iter().filter(|p| ext(p) == "toml").map(|p| stem(p)).collect();
    let mut out = Vec::new();
    for p in &files {
        let e = ext(p);
        if e == "toml" {
            out.push((stem(p), p.clone(), true));
        } else if exts.contains(&e.as_str()) && !tomls.contains(&stem(p)) {
            out.push((stem(p), p.clone(), false));
        }
    }
    out
}

impl Project {
    pub fn load(dir: &Path) -> Result<Project, String> {
        let game_path = dir.join("game.toml");
        if !crate::vfs::is_file(&game_path) {
            return Err(format!("{} is not a Mod Engine project (missing game.toml)", dir.display()));
        }
        let config: GameToml = read_toml(&game_path)?;

        let mut sprites = Vec::new();
        for (name, path, is_toml) in asset_files(&dir.join("sprites"), &["png"]) {
            sprites.push(load_sprite(&name, &path, is_toml, &config.palette)?);
        }

        let mut sounds = Vec::new();
        for (name, path, is_toml) in asset_files(&dir.join("sounds"), &["wav", "ogg"]) {
            if is_toml {
                let t: SoundToml = read_toml(&path)?;
                let data = match (&t.file, &t.synth) {
                    (Some(f), _) => {
                        let fp = path.parent().unwrap().join(f);
                        crate::vfs::read(&fp).map_err(|e| format!("{}: {e}", fp.display()))?
                    }
                    (None, Some(s)) => render_wav(s),
                    (None, None) => return Err(format!("{}: needs `file = \"...\"` or a [synth] table", path.display())),
                };
                sounds.push(SoundAsset { name, data, volume: t.volume.unwrap_or(1.0) });
            } else {
                let data = crate::vfs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                sounds.push(SoundAsset { name, data, volume: 1.0 });
            }
        }

        let mut fonts = Vec::new();
        let mut tilesets = Vec::new();
        {
            let find_sprite = |n: &str| sprites.iter().find(|s| s.name == n);
            for (name, path, is_toml) in asset_files(&dir.join("fonts"), &["ttf", "otf"]) {
                fonts.push(crate::font::load_font(&name, &path, is_toml, &find_sprite)?);
            }
            for (name, path, _) in asset_files(&dir.join("tilesets"), &[]) {
                tilesets.push(crate::tileset::load_tileset(&name, &path, &find_sprite)?);
            }
        }

        let mut rooms = Vec::new();
        for p in list_dir(&dir.join("rooms")) {
            if ext(&p) == "toml" {
                rooms.push((stem(&p), p.clone(), read_toml::<RoomToml>(&p)?));
            }
        }
        if rooms.is_empty() {
            return Err("the project has no rooms (add rooms/<name>.toml)".into());
        }

        let mut scripts = Vec::new();
        collect_scripts(dir, dir, &mut scripts)?;
        scripts.sort();
        // extension libraries load first; disabled ones not at all
        crate::extension::order_scripts(&mut scripts, &crate::extension::load_all(dir)?);

        Ok(Project { dir: dir.to_path_buf(), config, sprites, sounds, fonts, tilesets, rooms, scripts })
    }
}

pub fn collect_scripts(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> Result<(), String> {
    let Ok(rd) = crate::vfs::read_dir(dir) else { return Ok(()) };
    for (p, is_dir) in rd {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.starts_with('.') || name == "target" || name == "saves" {
            continue;
        }
        if is_dir {
            collect_scripts(root, &p, out)?;
        } else if ext(&p) == "ms" {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            out.push((rel, read_text(&p)?));
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ sprites

pub fn load_sprite(name: &str, path: &Path, is_toml: bool, global_palette: &BTreeMap<String, String>) -> Result<Sprite, String> {
    let t: SpriteToml = if is_toml { read_toml(path)? } else { SpriteToml::default() };
    let where_ = path.display().to_string();

    let (fw, fh, frames, sheet_w, sheet_h, rgba) = if !t.pixels.is_empty() {
        pixel_frames(&t, global_palette).map_err(|e| format!("{where_}: {e}"))?
    } else {
        let img_path = if is_toml {
            let f = t.image.as_ref().ok_or_else(|| format!("{where_}: needs `image = \"file.png\"` or `pixels = [...]`"))?;
            path.parent().unwrap().join(f)
        } else {
            path.to_path_buf()
        };
        let bytes = crate::vfs::read(&img_path).map_err(|e| format!("{}: {e}", img_path.display()))?;
        let img = image::load_from_memory(&bytes).map_err(|e| format!("{}: {e}", img_path.display()))?.to_rgba8();
        let (w, h) = img.dimensions();
        let mut frames = Vec::new();
        let (fw, fh) = if let Some([fw, fh]) = t.frame_size {
            if fw == 0 || fh == 0 {
                return Err(format!("{where_}: frame_size must be positive"));
            }
            for row in 0..h / fh {
                for col in 0..w / fw {
                    frames.push([col * fw, row * fh, fw, fh]);
                }
            }
            (fw, fh)
        } else {
            let n = t.frames.unwrap_or(1).max(1);
            let fw = w / n;
            for i in 0..n {
                frames.push([i * fw, 0, fw, h]);
            }
            (fw, h)
        };
        if frames.is_empty() {
            return Err(format!("{where_}: image is smaller than one frame"));
        }
        (fw, fh, frames, w, h, img.into_raw())
    };

    let origin = parse_origin(t.origin.as_ref(), fw, fh).map_err(|e| format!("{where_}: {e}"))?;
    let bbox = match t.mask {
        Some([l, tp, r, b]) => [l as f64, tp as f64, r as f64 + 1.0, b as f64 + 1.0],
        None => alpha_bbox(&rgba, sheet_w, &frames, fw, fh),
    };
    Ok(Sprite {
        name: name.to_string(),
        fw,
        fh,
        frames,
        sheet_w,
        sheet_h,
        rgba,
        origin,
        bbox,
        fps: t.fps.unwrap_or(10.0),
    })
}

type Frames = (u32, u32, Vec<[u32; 4]>, u32, u32, Vec<u8>);

fn pixel_frames(t: &SpriteToml, global: &BTreeMap<String, String>) -> Result<Frames, String> {
    let mut pal: HashMap<char, [u8; 4]> = HashMap::new();
    for (k, v) in global.iter().chain(t.palette.iter()) {
        let ch = k.chars().next().ok_or("empty palette key")?;
        let c = parse_color(v).ok_or_else(|| format!("bad palette colour '{v}'"))?;
        pal.insert(ch, [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
    }
    let grids: Vec<Vec<Vec<char>>> = t
        .pixels
        .iter()
        .map(|f| f.lines().map(|l| l.trim().chars().collect::<Vec<_>>()).filter(|l| !l.is_empty()).collect())
        .collect();
    let fw = grids.iter().flat_map(|g| g.iter().map(|r| r.len())).max().unwrap_or(0) as u32;
    let fh = grids.iter().map(|g| g.len()).max().unwrap_or(0) as u32;
    if fw == 0 || fh == 0 {
        return Err("pixel art is empty".into());
    }
    let n = grids.len() as u32;
    let sheet_w = fw * n;
    let mut rgba = vec![0u8; (sheet_w * fh * 4) as usize];
    let mut frames = Vec::new();
    for (fi, g) in grids.iter().enumerate() {
        let ox = fi as u32 * fw;
        frames.push([ox, 0, fw, fh]);
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
    Ok((fw, fh, frames, sheet_w, fh, rgba))
}

fn parse_origin(v: Option<&toml::Value>, fw: u32, fh: u32) -> Result<(f64, f64), String> {
    let (w, h) = (fw as f64, fh as f64);
    Ok(match v {
        None => (0.0, 0.0),
        Some(toml::Value::Array(a)) if a.len() == 2 => {
            let n = |v: &toml::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64));
            match (n(&a[0]), n(&a[1])) {
                (Some(x), Some(y)) => (x, y),
                _ => return Err("origin must be [x, y]".into()),
            }
        }
        Some(toml::Value::String(s)) => match s.as_str() {
            "top_left" => (0.0, 0.0),
            "top" => (w / 2.0, 0.0),
            "center" | "middle" => (w / 2.0, h / 2.0),
            "bottom" => (w / 2.0, h),
            "left" => (0.0, h / 2.0),
            other => return Err(format!("unknown origin '{other}' (use [x, y], top_left, top, center, left or bottom)")),
        },
        Some(_) => return Err("origin must be [x, y] or a name like \"center\"".into()),
    })
}

fn alpha_bbox(rgba: &[u8], sheet_w: u32, frames: &[[u32; 4]], fw: u32, fh: u32) -> [f64; 4] {
    let (mut l, mut t, mut r, mut b) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for f in frames {
        for y in 0..f[3] {
            for x in 0..f[2] {
                let i = (((f[1] + y) * sheet_w + f[0] + x) * 4 + 3) as usize;
                if rgba.get(i).copied().unwrap_or(0) > 0 {
                    l = l.min(x);
                    t = t.min(y);
                    r = r.max(x + 1);
                    b = b.max(y + 1);
                }
            }
        }
    }
    if l == u32::MAX {
        [0.0, 0.0, fw as f64, fh as f64]
    } else {
        [l as f64, t as f64, r as f64, b as f64]
    }
}

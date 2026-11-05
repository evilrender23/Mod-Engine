//! Tile set assets (`tilesets/`), GameMaker style: an image cut into a grid of tiles with
//! offset and separation, plus animated tiles and 16-tile auto tiles. Tile layers of rooms
//! (`tileset = "<name>"`) paint with them; scripts use the `tilemap_*` / `tile_*` functions.
//!
//! ```toml
//! # tilesets/ts_cave.toml
//! sprite = "spr_cave"        # a sprite of the project (its whole sheet)...
//! # image = "cave.png"       # ...or a PNG next to this file
//! tile_width = 16
//! tile_height = 16
//! offset = [0, 0]            # pixels before the first tile
//! separation = [0, 0]        # pixels between tiles
//! fps = 6                    # speed of the animated tiles
//!
//! [[animation]]              # tile 12 shows 12, 13, 14, 15, 12...
//! name = "agua"
//! frames = [12, 13, 14, 15]
//!
//! [[autotile]]               # 16 tiles indexed by the neighbours: up=1 right=2 down=4 left=8
//! name = "hierba"
//! tiles = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
//! ```
//!
//! Tile data (what `tilemap_get` returns) is the tile index plus transform bits, like
//! GameMaker's: [`MIRROR`], [`FLIP`], [`ROTATE`]; `-1` is an empty cell.

use crate::project::Sprite;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Bits of tile data.
pub const INDEX_MASK: i32 = 0x7_FFFF;
pub const MIRROR: i32 = 1 << 28;
pub const FLIP: i32 = 1 << 29;
pub const ROTATE: i32 = 1 << 30;
/// An empty cell.
pub const EMPTY: i32 = -1;

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct TileSetToml {
    pub sprite: Option<String>,
    pub image: Option<String>,
    pub tile_width: Option<u32>,
    pub tile_height: Option<u32>,
    pub offset: [u32; 2],
    pub separation: [u32; 2],
    pub fps: Option<f64>,
    #[serde(rename = "animation")]
    pub animations: Vec<TileAnimToml>,
    #[serde(rename = "autotile")]
    pub autotiles: Vec<AutoTileToml>,
}

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct TileAnimToml {
    pub name: String,
    pub frames: Vec<u32>,
}

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct AutoTileToml {
    pub name: String,
    /// 16 tile indices (-1 = none) by neighbour mask: up 1, right 2, down 4, left 8.
    pub tiles: Vec<i32>,
}

pub struct TileSetAsset {
    pub name: String,
    /// The tiles as frames of one sprite (frame `i` = tile `i`), origin at the top left.
    pub sprite: Sprite,
    pub tile_w: u32,
    pub tile_h: u32,
    pub columns: u32,
    pub rows: u32,
    pub fps: f64,
    pub anims: Vec<TileAnimToml>,
    pub autotiles: Vec<AutoTileToml>,
    /// Tile index → frames, for the first frame of every animation.
    pub anim_of: HashMap<u32, usize>,
    /// Image the tiles were cut from (sprite name or file), for the editor.
    pub source: String,
    /// Index of [`TileSetAsset::sprite`] in the engine's sprite list (set by the engine).
    pub sprite_index: u32,
}

impl TileSetAsset {
    /// Moves the tile sprite out (the engine keeps it in its sprite list).
    pub fn take_sprite(&mut self) -> Sprite {
        let s = &mut self.sprite;
        Sprite {
            name: s.name.clone(),
            fw: s.fw,
            fh: s.fh,
            frames: s.frames.clone(),
            sheet_w: s.sheet_w,
            sheet_h: s.sheet_h,
            rgba: std::mem::take(&mut s.rgba),
            origin: s.origin,
            bbox: s.bbox,
            fps: s.fps,
        }
    }

    pub fn count(&self) -> u32 {
        self.sprite.frames.len() as u32
    }

    /// Frame to show for tile `index` after `t` seconds (animated tiles cycle).
    pub fn frame_at(&self, index: u32, t: f64) -> u32 {
        match self.anim_of.get(&index).map(|&a| &self.anims[a].frames) {
            Some(f) if !f.is_empty() && self.fps > 0.0 => f[((t * self.fps).floor().max(0.0) as usize) % f.len()],
            _ => index,
        }
    }
}

/// Tile rectangles `[x, y, w, h]` of a `sheet_w`×`sheet_h` image, row by row.
pub fn slice(sheet_w: u32, sheet_h: u32, tw: u32, th: u32, offset: [u32; 2], sep: [u32; 2]) -> (u32, u32, Vec<[u32; 4]>) {
    if tw == 0 || th == 0 {
        return (0, 0, vec![]);
    }
    let fit = |size: u32, off: u32, t: u32, s: u32| if size < off + t { 0 } else { (size - off - t) / (t + s) + 1 };
    let cols = fit(sheet_w, offset[0], tw, sep[0]);
    let rows = fit(sheet_h, offset[1], th, sep[1]);
    let mut frames = vec![];
    for r in 0..rows {
        for c in 0..cols {
            frames.push([offset[0] + c * (tw + sep[0]), offset[1] + r * (th + sep[1]), tw, th]);
        }
    }
    (cols, rows, frames)
}

/// Loads `tilesets/<name>.toml`. `sprite` finds the project's sprites.
pub fn load_tileset<'s>(name: &str, path: &Path, sprite: &dyn Fn(&str) -> Option<&'s Sprite>) -> Result<TileSetAsset, String> {
    let where_ = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|e| format!("{where_}: {e}"))?;
    let t: TileSetToml = toml::from_str(&text).map_err(|e| format!("{where_}: {e}"))?;
    from_toml(name, &t, path.parent().unwrap_or(Path::new(".")), sprite).map_err(|e| format!("{where_}: {e}"))
}

pub fn from_toml<'s>(name: &str, t: &TileSetToml, folder: &Path, sprite: &dyn Fn(&str) -> Option<&'s Sprite>) -> Result<TileSetAsset, String> {
    let (sheet_w, sheet_h, rgba, source, default_size) = match (&t.sprite, &t.image) {
        (Some(s), _) => {
            let spr = sprite(s).ok_or_else(|| format!("sprite '{s}' does not exist"))?;
            (spr.sheet_w, spr.sheet_h, spr.rgba.clone(), s.clone(), if spr.frames.len() > 1 { [spr.fw, spr.fh] } else { [16, 16] })
        }
        (None, Some(f)) => {
            let p = folder.join(f);
            let bytes = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            let img = image::load_from_memory(&bytes).map_err(|e| format!("{}: {e}", p.display()))?.to_rgba8();
            let (w, h) = img.dimensions();
            (w, h, img.into_raw(), f.clone(), [16, 16])
        }
        (None, None) => return Err("needs `sprite = \"name\"` or `image = \"file.png\"`".into()),
    };
    let tw = t.tile_width.unwrap_or(default_size[0]);
    let th = t.tile_height.unwrap_or(default_size[1]);
    if tw == 0 || th == 0 {
        return Err("tile_width and tile_height must be positive".into());
    }
    let (columns, rows, frames) = slice(sheet_w, sheet_h, tw, th, t.offset, t.separation);
    if frames.is_empty() {
        return Err(format!("the image ({sheet_w}x{sheet_h}) is smaller than one {tw}x{th} tile"));
    }
    let n = frames.len() as u32;
    let mut anim_of = HashMap::new();
    for (i, a) in t.animations.iter().enumerate() {
        if let Some(bad) = a.frames.iter().find(|f| **f >= n) {
            return Err(format!("animation '{}': tile {bad} does not exist (the set has {n})", a.name));
        }
        if let Some(first) = a.frames.first() {
            anim_of.insert(*first, i);
        }
    }
    for a in &t.autotiles {
        if a.tiles.len() != 16 {
            return Err(format!("autotile '{}' needs 16 tiles, has {}", a.name, a.tiles.len()));
        }
        if let Some(bad) = a.tiles.iter().find(|x| **x >= n as i32) {
            return Err(format!("autotile '{}': tile {bad} does not exist (the set has {n})", a.name));
        }
    }
    let sprite = Sprite { name: format!("tileset:{name}"), fw: tw, fh: th, frames, sheet_w, sheet_h, rgba, origin: (0.0, 0.0), bbox: [0.0, 0.0, tw as f64, th as f64], fps: 0.0 };
    Ok(TileSetAsset {
        name: name.to_string(),
        sprite,
        tile_w: tw,
        tile_h: th,
        columns,
        rows,
        fps: t.fps.unwrap_or(4.0),
        anims: t.animations.clone(),
        autotiles: t.autotiles.clone(),
        anim_of,
        source,
        sprite_index: 0,
    })
}

/// TOML text of a tile set as the editor writes it.
pub fn write_toml(t: &TileSetToml) -> String {
    let q = |s: &str| toml::Value::String(s.to_string()).to_string();
    let mut s = String::new();
    if let Some(sp) = &t.sprite {
        s += &format!("sprite = {}\n", q(sp));
    } else if let Some(i) = &t.image {
        s += &format!("image = {}\n", q(i));
    }
    s += &format!("tile_width = {}\ntile_height = {}\n", t.tile_width.unwrap_or(16), t.tile_height.unwrap_or(16));
    if t.offset != [0, 0] {
        s += &format!("offset = [{}, {}]\n", t.offset[0], t.offset[1]);
    }
    if t.separation != [0, 0] {
        s += &format!("separation = [{}, {}]\n", t.separation[0], t.separation[1]);
    }
    if let Some(f) = t.fps {
        s += &format!("fps = {}\n", modscript::fmt_num(f));
    }
    for a in &t.animations {
        let f: Vec<String> = a.frames.iter().map(|x| x.to_string()).collect();
        s += &format!("\n[[animation]]\nname = {}\nframes = [{}]\n", q(&a.name), f.join(", "));
    }
    for a in &t.autotiles {
        let f: Vec<String> = a.tiles.iter().map(|x| x.to_string()).collect();
        s += &format!("\n[[autotile]]\nname = {}\ntiles = [{}]\n", q(&a.name), f.join(", "));
    }
    s
}

// ------------------------------------------------------------------ tile data

/// Tile index of tile data (-1 when empty).
pub fn index(d: i32) -> i32 {
    if d < 0 {
        EMPTY
    } else {
        d & INDEX_MASK
    }
}

/// Where a texel of the tile image shows: the image coordinate (0..1) seen at tile position
/// (u, v) once mirrored, flipped and then rotated 90° clockwise as the data says.
pub fn sample(d: i32, u: f32, v: f32) -> (f32, f32) {
    // rotation is applied last: undo it first
    let (mut x, mut y) = if d >= 0 && d & ROTATE != 0 { (v, 1.0 - u) } else { (u, v) };
    if d >= 0 && d & MIRROR != 0 {
        x = 1.0 - x;
    }
    if d >= 0 && d & FLIP != 0 {
        y = 1.0 - y;
    }
    (x, y)
}

/// Sprite draw parameters (x, y, scale_x, scale_y, angle in degrees counter-clockwise) that
/// put a tile with transform bits `d` exactly on the cell at (x0, y0) of size w×h, for a
/// sprite with origin (0, 0) drawn like the backend does (flip in place, then rotate about
/// the position).
pub fn draw_params(d: i32, x0: f64, y0: f64, w: f64, h: f64) -> (f64, f64, f64, f64, f64) {
    let sx = if d >= 0 && d & MIRROR != 0 { -1.0 } else { 1.0 };
    let sy = if d >= 0 && d & FLIP != 0 { -1.0 } else { 1.0 };
    let rot = d >= 0 && d & ROTATE != 0;
    // rectangle relative to the position (0, 0) before rotating
    let (l, t) = (if sx < 0.0 { -w } else { 0.0 }, if sy < 0.0 { -h } else { 0.0 });
    // 90° clockwise on screen (y down): (dx, dy) -> (-dy, dx)
    let (minx, miny) = if rot { (-(t + h), l) } else { (l, t) };
    (x0 - minx, y0 - miny, sx, sy, if rot { -90.0 } else { 0.0 })
}

/// Neighbour mask of an auto tile cell: up 1, right 2, down 4, left 8.
pub fn autotile_mask(up: bool, right: bool, down: bool, left: bool) -> usize {
    up as usize | (right as usize) << 1 | (down as usize) << 2 | (left as usize) << 3
}

fn cell(grid: &[Vec<i32>], c: i32, r: i32) -> i32 {
    if c < 0 || r < 0 {
        return EMPTY;
    }
    grid.get(r as usize).and_then(|row| row.get(c as usize)).copied().unwrap_or(EMPTY)
}

fn put(grid: &mut Vec<Vec<i32>>, c: i32, r: i32, v: i32) {
    if c < 0 || r < 0 {
        return;
    }
    let (c, r) = (c as usize, r as usize);
    if grid.len() <= r {
        grid.resize(r + 1, vec![]);
    }
    if grid[r].len() <= c {
        grid[r].resize(c + 1, EMPTY);
    }
    grid[r][c] = v;
}

/// Paints (`on`) or erases one cell of an auto tile and re-picks the tiles of it and its
/// four neighbours from `tiles` (16 entries by neighbour mask). A cell belongs to the auto
/// tile when its tile index is one of `tiles`.
pub fn autotile_apply(grid: &mut Vec<Vec<i32>>, tiles: &[i32], c: i32, r: i32, on: bool) {
    if tiles.len() != 16 || c < 0 || r < 0 {
        return;
    }
    let member = |g: &[Vec<i32>], c: i32, r: i32| {
        let i = index(cell(g, c, r));
        i >= 0 && tiles.contains(&i)
    };
    let full = tiles[15];
    put(grid, c, r, if on { full.max(0) } else { EMPTY });
    for (dc, dr) in [(0, 0), (0, -1), (1, 0), (0, 1), (-1, 0)] {
        let (x, y) = (c + dc, r + dr);
        if !member(grid, x, y) {
            continue;
        }
        let m = autotile_mask(member(grid, x, y - 1), member(grid, x + 1, y), member(grid, x, y + 1), member(grid, x - 1, y));
        let t = if tiles[m] >= 0 { tiles[m] } else { full };
        put(grid, x, y, t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slicing_with_offset_and_separation() {
        let (c, r, f) = slice(64, 32, 16, 16, [0, 0], [0, 0]);
        assert_eq!((c, r, f.len()), (4, 2, 8));
        assert_eq!(f[5], [16, 16, 16, 16]);
        // 1 px border and 2 px gaps: 1 + 16 + 2 + 16 + 2 + 16 = 53
        let (c, r, f) = slice(53, 17, 16, 16, [1, 1], [2, 2]);
        assert_eq!((c, r), (3, 1));
        assert_eq!(f[2], [37, 1, 16, 16]);
        assert_eq!(slice(8, 8, 16, 16, [0, 0], [0, 0]).2.len(), 0);
    }

    /// The backend's model: flip inside the destination rectangle, rotate it about (x, y).
    fn backend_corner(p: (f64, f64, f64, f64, f64), w: f64, h: f64, u: f64, v: f64) -> (f64, f64) {
        let (x, y, sx, sy, angle) = p;
        let left = (x).min(x + w * sx);
        let top = (y).min(y + h * sy);
        // texel (u, v) of the image lands here before rotation
        let px = if sx < 0.0 { left + (1.0 - u) * w } else { left + u * w };
        let py = if sy < 0.0 { top + (1.0 - v) * h } else { top + v * h };
        let r = (-angle).to_radians();
        let (dx, dy) = (px - x, py - y);
        (x + dx * r.cos() - dy * r.sin(), y + dx * r.sin() + dy * r.cos())
    }

    #[test]
    fn transforms_land_on_the_cell_and_match_the_editor() {
        let (w, h) = (16.0, 16.0);
        for d in [0, MIRROR, FLIP, ROTATE, MIRROR | ROTATE, FLIP | ROTATE, MIRROR | FLIP | ROTATE, MIRROR | FLIP] {
            let p = draw_params(d | 3, 32.0, 48.0, w, h);
            // every image texel goes where `sample` (the editor's UV mapping) says it shows
            for (u, v) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.25, 0.75)] {
                let (sx, sy) = backend_corner(p, w, h, u, v);
                let (cu, cv) = ((sx - 32.0) / w, (sy - 48.0) / h);
                assert!((-1e-6..=1.0 + 1e-6).contains(&cu) && (-1e-6..=1.0 + 1e-6).contains(&cv), "data {d:x}: off the cell ({cu}, {cv})");
                let (iu, iv) = sample(d | 3, cu as f32, cv as f32);
                assert!((iu as f64 - u).abs() < 1e-4 && (iv as f64 - v).abs() < 1e-4, "data {d:x}: texel ({u},{v}) shows at ({cu},{cv}) but the editor samples ({iu},{iv})");
            }
        }
        assert_eq!(index(MIRROR | ROTATE | 7), 7);
        assert_eq!(index(-1), -1);
        assert_eq!(autotile_mask(true, false, true, true), 13);
    }

    #[test]
    fn autotiles_follow_their_neighbours() {
        // tile for mask m is 100 + m
        let tiles: Vec<i32> = (0..16).map(|m| 100 + m).collect();
        let mut g: Vec<Vec<i32>> = vec![];
        autotile_apply(&mut g, &tiles, 1, 1, true);
        assert_eq!(cell(&g, 1, 1), 100, "alone: no neighbours");
        autotile_apply(&mut g, &tiles, 2, 1, true);
        assert_eq!(cell(&g, 1, 1), 102, "right neighbour");
        assert_eq!(cell(&g, 2, 1), 108, "left neighbour");
        autotile_apply(&mut g, &tiles, 1, 2, true);
        assert_eq!(cell(&g, 1, 1), 106, "right + down");
        assert_eq!(cell(&g, 1, 2), 101, "up");
        autotile_apply(&mut g, &tiles, 2, 1, false);
        assert_eq!(cell(&g, 2, 1), EMPTY);
        assert_eq!(cell(&g, 1, 1), 104, "only down left");
    }

    #[test]
    fn loads_from_png_with_animations() {
        let dir = std::env::temp_dir().join(format!("modengine_ts_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        image::RgbaImage::from_pixel(32, 16, image::Rgba([1, 2, 3, 255])).save(dir.join("t.png")).unwrap();
        let t = TileSetToml { image: Some("t.png".into()), tile_width: Some(8), tile_height: Some(8), fps: Some(2.0), animations: vec![TileAnimToml { name: "a".into(), frames: vec![1, 2, 3] }], ..Default::default() };
        std::fs::write(dir.join("ts.toml"), write_toml(&t)).unwrap();
        let ts = load_tileset("ts", &dir.join("ts.toml"), &|_| None).unwrap();
        assert_eq!((ts.columns, ts.rows, ts.count()), (4, 2, 8));
        assert_eq!(ts.frame_at(1, 0.0), 1);
        assert_eq!(ts.frame_at(1, 0.5), 2);
        assert_eq!(ts.frame_at(1, 1.6), 1);
        assert_eq!(ts.frame_at(5, 3.0), 5);
        let back: TileSetToml = toml::from_str(&write_toml(&t)).unwrap();
        assert_eq!(back, t);
        let bad = TileSetToml { autotiles: vec![AutoTileToml { name: "x".into(), tiles: vec![0; 3] }], ..t };
        std::fs::write(dir.join("bad.toml"), write_toml(&bad)).unwrap();
        assert!(load_tileset("bad", &dir.join("bad.toml"), &|_| None).err().unwrap().contains("16 tiles"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

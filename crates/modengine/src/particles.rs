//! Particle systems (GameMaker style): particle *types* describe how one particle looks and
//! moves, *emitters* create particles inside a region, and a *system* owns emitters and live
//! particles. Systems come from `particles/<name>.toml` assets or are built in code with the
//! `part_*` functions.
//!
//! The simulation here is renderer-agnostic and shared by the engine and the editor's live
//! preview: [`PartWorld::step`] advances it and [`PartWorld::draw_list`] says what to draw.
//!
//! ```toml
//! # particles/fire.toml
//! depth = -10
//!
//! [[type]]
//! name = "flame"
//! shape = "flare"            # or sprite = "spr_spark"
//! size = [0.2, 0.4]
//! size_incr = -0.004
//! speed = [0.5, 1.2]
//! direction = [80, 100]
//! colors = ["#ffe066", "#ff7a00", "#5a1000"]
//! alphas = [1.0, 0.8, 0.0]
//! additive = true
//! life = [30, 50]
//! death_type = "smoke"
//! death_count = 1
//!
//! [[emitter]]
//! name = "base"
//! type = "flame"
//! shape = "ellipse"
//! region = [-12, -4, 12, 4]
//! mode = "stream"
//! count = 3
//! ```

use crate::draw::parse_color;
use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ================================================================== built-in shapes

/// Built-in particle shapes, in GameMaker's order (`pt_shape_pixel` = 0 …).
pub const SHAPE_NAMES: [&str; 14] =
    ["pixel", "disk", "square", "line", "star", "circle", "ring", "sphere", "flare", "spark", "explosion", "cloud", "smoke", "snow"];

/// Side of the built-in shape textures: a particle of size 1 is this many pixels wide.
pub const SHAPE_SIZE: u32 = 64;

pub fn shape_index(name: &str) -> Option<usize> {
    SHAPE_NAMES.iter().position(|s| *s == name.trim().to_lowercase())
}

fn hash2(x: i32, y: i32, seed: u32) -> f64 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xFFFF) as f64 / 65535.0
}

/// Smooth value noise in [0, 1].
fn noise(x: f64, y: f64, seed: u32) -> f64 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (s(fx), s(fy));
    let a = hash2(xi, yi, seed) + (hash2(xi + 1, yi, seed) - hash2(xi, yi, seed)) * sx;
    let b = hash2(xi, yi + 1, seed) + (hash2(xi + 1, yi + 1, seed) - hash2(xi, yi + 1, seed)) * sx;
    a + (b - a) * sy
}

fn fbm(x: f64, y: f64, seed: u32) -> f64 {
    let mut v = 0.0;
    let mut amp = 0.5;
    let mut f = 1.0;
    for o in 0..4 {
        v += amp * noise(x * f, y * f, seed + o);
        amp *= 0.5;
        f *= 2.0;
    }
    v / 0.9375
}

fn seg_dist(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
}

/// Opacity of built-in shape `k` at (u, v) in [-1, 1]².
pub fn shape_alpha(k: usize, u: f64, v: f64) -> f64 {
    let r = (u * u + v * v).sqrt();
    let aa = |d: f64| (d * 24.0).clamp(0.0, 1.0); // soft 1-2 px edge for d = distance inside
    let a = match k {
        0 => aa(0.07 - u.abs().max(v.abs())),
        1 => aa(0.95 - r),
        2 => aa(0.95 - u.abs().max(v.abs())),
        3 => aa(0.08 - v.abs()) * aa(0.97 - u.abs()),
        4 => {
            let th = v.atan2(u) + std::f64::consts::FRAC_PI_2;
            let seg = std::f64::consts::TAU / 5.0;
            let t = ((th.rem_euclid(seg) / seg) - 0.5).abs() * 2.0; // 1 at the tips, 0 between
            aa(0.38 + 0.57 * t - r)
        }
        5 => aa(0.06 - (r - 0.86).abs()),
        6 => (-((r - 0.68) / 0.13).powi(2)).exp() * aa(1.0 - r),
        7 => (1.0 - r * r).max(0.0).powf(0.9),
        8 => {
            let glow = (1.0 - r).max(0.0).powi(3);
            let rays = ((1.0 - u.abs() * 9.0).max(0.0) * (1.0 - v.abs())).max((1.0 - v.abs() * 9.0).max(0.0) * (1.0 - u.abs()));
            (glow * 1.2 + rays * 0.6).min(1.0)
        }
        9 => {
            let ray = |a: f64, b: f64| (1.0 - a.abs() * 16.0).max(0.0) * (1.0 - b.abs()).max(0.0).powi(2);
            let d = std::f64::consts::FRAC_1_SQRT_2;
            let (du, dv) = ((u + v) * d, (u - v) * d);
            (ray(u, v).max(ray(v, u)) + 0.5 * ray(du, dv).max(ray(dv, du)) + (1.0 - r * 4.0).max(0.0)).min(1.0)
        }
        10 => {
            let th = v.atan2(u);
            let edge = 0.55 + 0.35 * noise(th.cos() * 2.5 + 3.0, th.sin() * 2.5 + 3.0, 7);
            let body = aa(edge - r);
            body * (0.55 + 0.45 * fbm(u * 3.0 + 9.0, v * 3.0 + 9.0, 11))
        }
        11 => {
            let n = fbm(u * 2.2 + 5.0, v * 2.2 + 5.0, 3);
            ((n - 0.25) * 2.2).clamp(0.0, 1.0) * (1.0 - r).max(0.0).powf(0.7)
        }
        12 => {
            let n = fbm(u * 2.8 + 1.0, v * 2.8 + 1.0, 5);
            (n * (1.0 - r).max(0.0).powf(1.3) * 1.4).min(1.0)
        }
        13 => {
            let mut d = f64::MAX;
            for i in 0..6 {
                let a = i as f64 * std::f64::consts::PI / 3.0;
                let (c, s) = (a.cos(), a.sin());
                d = d.min(seg_dist(u, v, 0.0, 0.0, c * 0.92, s * 0.92));
                for (t, l) in [(0.45, 0.25), (0.7, 0.18)] {
                    let (bx, by) = (c * t, s * t);
                    for side in [-1.0, 1.0] {
                        let a2 = a + side * 0.75;
                        d = d.min(seg_dist(u, v, bx, by, bx + a2.cos() * l, by + a2.sin() * l));
                    }
                }
            }
            aa(0.055 - d)
        }
        _ => 0.0,
    };
    a.clamp(0.0, 1.0)
}

/// RGBA pixels (white, shape in the alpha channel) of built-in shape `k`, `SHAPE_SIZE`² big.
pub fn shape_image(k: usize) -> Vec<u8> {
    let n = SHAPE_SIZE;
    let mut out = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            // 3×3 supersampling keeps the small shapes smooth
            let mut a = 0.0;
            for sy in 0..3 {
                for sx in 0..3 {
                    let u = (x as f64 + (sx as f64 + 0.5) / 3.0) / n as f64 * 2.0 - 1.0;
                    let v = (y as f64 + (sy as f64 + 0.5) / 3.0) / n as f64 * 2.0 - 1.0;
                    a += shape_alpha(k, u, v);
                }
            }
            out.extend_from_slice(&[255, 255, 255, (a / 9.0 * 255.0).round() as u8]);
        }
    }
    out
}

// ================================================================== asset file format

/// A number or a `[min, max]` pair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range2(pub [f64; 2]);

impl Range2 {
    pub fn one(v: f64) -> Self {
        Range2([v, v])
    }
}

impl Serialize for Range2 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.0[0] == self.0[1] {
            s.serialize_f64(self.0[0])
        } else {
            self.0.serialize(s)
        }
    }
}

impl<'de> Deserialize<'de> for Range2 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum R {
            N(f64),
            I(i64),
            A(Vec<f64>),
        }
        Ok(match R::deserialize(d)? {
            R::N(v) => Range2([v, v]),
            R::I(v) => Range2([v as f64, v as f64]),
            R::A(a) if a.len() == 2 => Range2([a[0], a[1]]),
            R::A(a) if a.len() == 1 => Range2([a[0], a[0]]),
            R::A(_) => return Err(de::Error::custom("expected a number or [min, max]")),
        })
    }
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}
fn is_false(v: &bool) -> bool {
    !*v
}
fn is_one2(v: &[f64; 2]) -> bool {
    *v == [1.0, 1.0]
}

/// One `[[type]]` of a particle system file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct PartTypeToml {
    pub name: String,
    /// Built-in shape (see [`SHAPE_NAMES`]); ignored when `sprite` is set.
    pub shape: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sprite: Option<String>,
    /// Play the sprite's animation…
    #[serde(skip_serializing_if = "is_false")]
    pub animate: bool,
    /// …stretched over the particle's life.
    #[serde(skip_serializing_if = "is_false")]
    pub stretch: bool,
    /// Start on a random frame.
    #[serde(skip_serializing_if = "is_false")]
    pub random_frame: bool,
    pub size: Range2,
    #[serde(skip_serializing_if = "is_zero")]
    pub size_incr: f64,
    #[serde(skip_serializing_if = "is_zero")]
    pub size_wiggle: f64,
    #[serde(skip_serializing_if = "is_one2")]
    pub scale: [f64; 2],
    pub speed: Range2,
    #[serde(skip_serializing_if = "is_zero")]
    pub speed_incr: f64,
    #[serde(skip_serializing_if = "is_zero")]
    pub speed_wiggle: f64,
    pub direction: Range2,
    #[serde(skip_serializing_if = "is_zero")]
    pub direction_incr: f64,
    #[serde(skip_serializing_if = "is_zero")]
    pub direction_wiggle: f64,
    #[serde(skip_serializing_if = "is_zero")]
    pub gravity: f64,
    pub gravity_dir: f64,
    pub orientation: Range2,
    #[serde(skip_serializing_if = "is_zero")]
    pub orientation_incr: f64,
    #[serde(skip_serializing_if = "is_zero")]
    pub orientation_wiggle: f64,
    /// Add the direction of motion to the orientation.
    #[serde(skip_serializing_if = "is_false")]
    pub orientation_relative: bool,
    /// 1 to 3 colours: constant, start→end or start→middle→end over the particle's life.
    pub colors: Vec<String>,
    /// "fade" (over life, default) or "mix" (a random colour between the first two, fixed).
    pub color_mode: String,
    /// 1 to 3 alpha values over life.
    pub alphas: Vec<f64>,
    #[serde(skip_serializing_if = "is_false")]
    pub additive: bool,
    /// Life in steps.
    pub life: Range2,
    /// Type created every step at each particle (count < 0: one with a 1/-count chance).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_type: Option<String>,
    #[serde(skip_serializing_if = "is_zero_i")]
    pub step_count: i32,
    /// Type created where each particle dies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub death_type: Option<String>,
    #[serde(skip_serializing_if = "is_zero_i")]
    pub death_count: i32,
}

fn is_zero_i(v: &i32) -> bool {
    *v == 0
}

impl Default for PartTypeToml {
    fn default() -> Self {
        PartTypeToml {
            name: "particle".into(),
            shape: "disk".into(),
            sprite: None,
            animate: false,
            stretch: false,
            random_frame: false,
            size: Range2([0.1, 0.2]),
            size_incr: 0.0,
            size_wiggle: 0.0,
            scale: [1.0, 1.0],
            speed: Range2([1.0, 2.0]),
            speed_incr: 0.0,
            speed_wiggle: 0.0,
            direction: Range2([0.0, 360.0]),
            direction_incr: 0.0,
            direction_wiggle: 0.0,
            gravity: 0.0,
            gravity_dir: 270.0,
            orientation: Range2([0.0, 0.0]),
            orientation_incr: 0.0,
            orientation_wiggle: 0.0,
            orientation_relative: false,
            colors: vec!["#ffffff".into()],
            color_mode: "fade".into(),
            alphas: vec![1.0, 0.0],
            additive: false,
            life: Range2([30.0, 60.0]),
            step_type: None,
            step_count: 0,
            death_type: None,
            death_count: 0,
        }
    }
}

/// One `[[emitter]]` of a particle system file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct EmitterToml {
    pub name: String,
    /// Name of the `[[type]]` it emits.
    #[serde(rename = "type")]
    pub ty: String,
    /// "rectangle", "ellipse", "diamond" or "line".
    pub shape: String,
    /// "linear", "gaussian" or "invgaussian".
    pub distribution: String,
    /// x1, y1, x2, y2 relative to the system's position.
    pub region: [f64; 4],
    /// "stream" (every step) or "burst" (once, or every `interval` steps).
    pub mode: String,
    /// Particles per step (stream) or per burst. Negative stream counts emit one particle with a
    /// 1/-count chance each step.
    pub count: i32,
    pub enabled: bool,
    /// Steps to wait before the first emission ([min, max]).
    #[serde(skip_serializing_if = "is_zero2")]
    pub delay: Range2,
    /// Bursts repeat every `interval` steps ([min, max]); 0 = only once.
    #[serde(skip_serializing_if = "is_zero2")]
    pub interval: Range2,
}

fn is_zero2(v: &Range2) -> bool {
    v.0 == [0.0, 0.0]
}

impl Default for EmitterToml {
    fn default() -> Self {
        EmitterToml {
            name: "emitter".into(),
            ty: "particle".into(),
            shape: "rectangle".into(),
            distribution: "linear".into(),
            region: [-16.0, -16.0, 16.0, 16.0],
            mode: "stream".into(),
            count: 2,
            enabled: true,
            delay: Range2([0.0, 0.0]),
            interval: Range2([0.0, 0.0]),
        }
    }
}

/// A whole `particles/<name>.toml` file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct PartSysToml {
    /// Drawing depth (like instances: higher = further back).
    pub depth: f64,
    /// Draw old particles on top of new ones.
    #[serde(skip_serializing_if = "is_false")]
    pub new_below: bool,
    #[serde(rename = "type")]
    pub types: Vec<PartTypeToml>,
    #[serde(rename = "emitter")]
    pub emitters: Vec<EmitterToml>,
}

impl Default for PartSysToml {
    fn default() -> Self {
        PartSysToml { depth: 0.0, new_below: false, types: vec![], emitters: vec![] }
    }
}

impl PartSysToml {
    pub fn parse(text: &str) -> Result<PartSysToml, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// Problems that would stop the engine from loading the file (unknown names and shapes).
    pub fn validate(&self, sprite_exists: &dyn Fn(&str) -> bool) -> Vec<String> {
        let mut out = vec![];
        let names: Vec<&str> = self.types.iter().map(|t| t.name.as_str()).collect();
        for (i, t) in self.types.iter().enumerate() {
            if names[..i].contains(&t.name.as_str()) {
                out.push(format!("type '{}' is defined twice", t.name));
            }
            match &t.sprite {
                Some(s) if !sprite_exists(s) => out.push(format!("type '{}': sprite '{s}' does not exist", t.name)),
                None if shape_index(&t.shape).is_none() => {
                    out.push(format!("type '{}': unknown shape '{}' (use {})", t.name, t.shape, SHAPE_NAMES.join(", ")))
                }
                _ => {}
            }
            for c in &t.colors {
                if parse_color(c).is_none() {
                    out.push(format!("type '{}': bad colour '{c}'", t.name));
                }
            }
            if !matches!(t.color_mode.as_str(), "" | "fade" | "mix") {
                out.push(format!("type '{}': color_mode must be \"fade\" or \"mix\"", t.name));
            }
            for (what, n) in [("step_type", &t.step_type), ("death_type", &t.death_type)] {
                if let Some(n) = n.as_deref().filter(|n| !n.is_empty()) {
                    if !names.contains(&n) {
                        out.push(format!("type '{}': {what} '{n}' does not exist", t.name));
                    }
                }
            }
        }
        for e in &self.emitters {
            if !names.contains(&e.ty.as_str()) {
                out.push(format!("emitter '{}': type '{}' does not exist", e.name, e.ty));
            }
            if RegionShape::parse(&e.shape).is_none() {
                out.push(format!("emitter '{}': unknown shape '{}' (rectangle, ellipse, diamond or line)", e.name, e.shape));
            }
            if Distr::parse(&e.distribution).is_none() {
                out.push(format!("emitter '{}': unknown distribution '{}' (linear, gaussian or invgaussian)", e.name, e.distribution));
            }
            if !matches!(e.mode.as_str(), "stream" | "burst") {
                out.push(format!("emitter '{}': mode must be \"stream\" or \"burst\"", e.name));
            }
        }
        out
    }
}

/// `particles/*.toml` of a project: (name, path, parsed file).
pub fn load_assets(dir: &Path) -> Result<Vec<(String, PathBuf, PartSysToml)>, String> {
    let mut out = vec![];
    for (name, path, is_toml) in crate::project::asset_files(&dir.join("particles"), &[]) {
        if !is_toml {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let t = PartSysToml::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        out.push((name, path, t));
    }
    Ok(out)
}

// ================================================================== runtime

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tex {
    Shape(u8),
    /// A sprite index (the engine's or, in the editor, the project's).
    Sprite(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RegionShape {
    Rectangle,
    Ellipse,
    Diamond,
    Line,
}

impl RegionShape {
    pub const ALL: [&'static str; 4] = ["rectangle", "ellipse", "diamond", "line"];
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "rectangle" | "rect" | "" => RegionShape::Rectangle,
            "ellipse" => RegionShape::Ellipse,
            "diamond" => RegionShape::Diamond,
            "line" => RegionShape::Line,
            _ => return None,
        })
    }
    pub fn from_num(n: i64) -> Self {
        [RegionShape::Rectangle, RegionShape::Ellipse, RegionShape::Diamond, RegionShape::Line][n.clamp(0, 3) as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Distr {
    Linear,
    Gaussian,
    InvGaussian,
}

impl Distr {
    pub const ALL: [&'static str; 3] = ["linear", "gaussian", "invgaussian"];
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "linear" | "" => Distr::Linear,
            "gaussian" => Distr::Gaussian,
            "invgaussian" => Distr::InvGaussian,
            _ => return None,
        })
    }
    pub fn from_num(n: i64) -> Self {
        [Distr::Linear, Distr::Gaussian, Distr::InvGaussian][n.clamp(0, 2) as usize]
    }
}

/// A particle type ready to simulate (`part_type_*` change these fields).
#[derive(Clone, Debug)]
pub struct PartType {
    pub tex: Tex,
    pub animate: bool,
    pub stretch: bool,
    pub random_frame: bool,
    pub size: [f64; 2],
    pub size_incr: f64,
    pub size_wiggle: f64,
    pub scale: [f64; 2],
    pub speed: [f64; 2],
    pub speed_incr: f64,
    pub speed_wiggle: f64,
    pub dir: [f64; 2],
    pub dir_incr: f64,
    pub dir_wiggle: f64,
    pub grav: f64,
    pub grav_dir: f64,
    pub orient: [f64; 2],
    pub orient_incr: f64,
    pub orient_wiggle: f64,
    pub orient_rel: bool,
    /// 1–3 colours (0xRRGGBB).
    pub colors: Vec<u32>,
    /// Random fixed colour between colors[0] and colors[1].
    pub color_mix: bool,
    /// Random fixed colour from RGB ranges [rmin, rmax, gmin, gmax, bmin, bmax] (0–255).
    pub color_rgb: Option<[f64; 6]>,
    /// Random fixed colour from HSV ranges (0–255 each, like GameMaker).
    pub color_hsv: Option<[f64; 6]>,
    pub alphas: Vec<f64>,
    pub additive: bool,
    pub life: [f64; 2],
    pub step: (i32, Option<usize>),
    pub death: (i32, Option<usize>),
}

impl Default for PartType {
    fn default() -> Self {
        // GameMaker's defaults for part_type_create()
        PartType {
            tex: Tex::Shape(0),
            animate: false,
            stretch: false,
            random_frame: false,
            size: [1.0, 1.0],
            size_incr: 0.0,
            size_wiggle: 0.0,
            scale: [1.0, 1.0],
            speed: [0.0, 0.0],
            speed_incr: 0.0,
            speed_wiggle: 0.0,
            dir: [0.0, 0.0],
            dir_incr: 0.0,
            dir_wiggle: 0.0,
            grav: 0.0,
            grav_dir: 270.0,
            orient: [0.0, 0.0],
            orient_incr: 0.0,
            orient_wiggle: 0.0,
            orient_rel: false,
            colors: vec![0xFFFFFF],
            color_mix: false,
            color_rgb: None,
            color_hsv: None,
            alphas: vec![1.0],
            additive: false,
            life: [100.0, 100.0],
            step: (0, None),
            death: (0, None),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Particle {
    pub ty: usize,
    pub x: f64,
    pub y: f64,
    pub speed: f64,
    pub dir: f64,
    pub size: f64,
    pub orient: f64,
    pub age: f64,
    pub life: f64,
    /// Fixed colour for mix/rgb/hsv types.
    pub color: Option<u32>,
    pub frame: f64,
    /// Random phase used by the wiggles.
    pub seed: f64,
}

#[derive(Clone, Debug)]
pub struct Emitter {
    pub name: String,
    pub ty: Option<usize>,
    pub shape: RegionShape,
    pub distr: Distr,
    /// xmin, xmax, ymin, ymax (GameMaker's argument order).
    pub region: [f64; 4],
    pub stream: i32,
    pub burst: i32,
    /// Burst interval in steps ([min, max]; 0 = no repeat).
    pub interval: [f64; 2],
    pub enabled: bool,
    /// Steps left before it emits (delay, then interval countdown).
    pub wait: f64,
    /// A burst is still due (asset bursts fire once after their delay).
    pub burst_due: bool,
}

impl Emitter {
    pub fn new() -> Self {
        Emitter {
            name: String::new(),
            ty: None,
            shape: RegionShape::Rectangle,
            distr: Distr::Linear,
            region: [0.0; 4],
            stream: 0,
            burst: 0,
            interval: [0.0, 0.0],
            enabled: true,
            wait: 0.0,
            burst_due: false,
        }
    }
}

impl Default for Emitter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct System {
    pub x: f64,
    pub y: f64,
    pub depth: f64,
    pub angle: f64,
    pub tint: u32,
    pub alpha: f64,
    pub auto_update: bool,
    pub auto_draw: bool,
    pub new_below: bool,
    pub emitters: Vec<Option<Emitter>>,
    pub parts: Vec<Particle>,
    /// Types created for this system from its asset (destroyed with it).
    pub own_types: Vec<usize>,
    /// The asset it was created from.
    pub asset: Option<u32>,
}

impl System {
    pub fn new() -> Self {
        System {
            x: 0.0,
            y: 0.0,
            depth: 0.0,
            angle: 0.0,
            tint: 0xFFFFFF,
            alpha: 1.0,
            auto_update: true,
            auto_draw: true,
            new_below: false,
            emitters: vec![],
            parts: vec![],
            own_types: vec![],
            asset: None,
        }
    }
}

impl Default for System {
    fn default() -> Self {
        Self::new()
    }
}

/// One textured quad to draw for a particle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartDraw {
    pub tex: Tex,
    pub frame: u32,
    pub x: f64,
    pub y: f64,
    /// Scale relative to the texture (shapes are `SHAPE_SIZE` px; sprites their own size).
    pub sx: f64,
    pub sy: f64,
    /// Degrees, counter-clockwise.
    pub angle: f64,
    pub color: u32,
    pub alpha: f64,
    pub additive: bool,
}

/// Hard cap of live particles per system (keeps a runaway stream from freezing the game).
pub const MAX_PARTICLES: usize = 20_000;

/// Every particle type and system of a running game (or of an editor preview).
#[derive(Clone, Debug)]
pub struct PartWorld {
    /// Loaded `particles/*.toml` files, by asset index.
    pub assets: Vec<(String, PartSysToml)>,
    pub types: Vec<Option<PartType>>,
    pub systems: Vec<Option<System>>,
    pub rng: u64,
}

impl Default for PartWorld {
    fn default() -> Self {
        PartWorld { assets: vec![], types: vec![], systems: vec![], rng: 0x2545_F491_4F6C_DD1D }
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn lerp_rgb(a: u32, b: u32, t: f64) -> u32 {
    let ch = |c: u32, s: u32| ((c >> s) & 255) as f64;
    let mut out = 0;
    for s in [16, 8, 0] {
        out |= (lerp(ch(a, s), ch(b, s), t).round().clamp(0.0, 255.0) as u32) << s;
    }
    out
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> u32 {
    // h, s, v in 0..255 like GameMaker
    let (h, s, v) = ((h / 255.0 * 6.0).rem_euclid(6.0), s / 255.0, v / 255.0);
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let q = |f: f64| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    q(r) << 16 | q(g) << 8 | q(b)
}

/// Value of a 1–3 point curve at t in [0, 1].
fn curve(v: &[f64], t: f64) -> f64 {
    match v.len() {
        0 => 1.0,
        1 => v[0],
        2 => lerp(v[0], v[1], t),
        _ => {
            if t < 0.5 {
                lerp(v[0], v[1], t * 2.0)
            } else {
                lerp(v[1], v[2], t * 2.0 - 1.0)
            }
        }
    }
}

fn color_curve(c: &[u32], t: f64) -> u32 {
    match c.len() {
        0 => 0xFFFFFF,
        1 => c[0],
        2 => lerp_rgb(c[0], c[1], t),
        _ => {
            if t < 0.5 {
                lerp_rgb(c[0], c[1], t * 2.0)
            } else {
                lerp_rgb(c[1], c[2], t * 2.0 - 1.0)
            }
        }
    }
}

impl PartWorld {
    pub fn new(assets: Vec<(String, PartSysToml)>) -> Self {
        PartWorld { assets, ..Default::default() }
    }

    pub fn rand(&mut self) -> f64 {
        crate::natives::rand01(&mut self.rng)
    }

    fn range(&mut self, r: [f64; 2]) -> f64 {
        if r[0] == r[1] {
            r[0]
        } else {
            lerp(r[0], r[1], self.rand())
        }
    }

    /// Gaussian-ish sample in [0, 1] (sum of uniforms, mean 0.5).
    fn distr01(&mut self, d: Distr) -> f64 {
        match d {
            Distr::Linear => self.rand(),
            Distr::Gaussian => (self.rand() + self.rand() + self.rand() + self.rand()) / 4.0,
            Distr::InvGaussian => {
                let g = (self.rand() + self.rand() + self.rand() + self.rand()) / 4.0;
                if g < 0.5 {
                    0.5 - g
                } else {
                    1.5 - g
                }
            }
        }
    }

    // ------------------------------------------------------------ types

    pub fn type_create(&mut self, t: PartType) -> usize {
        if let Some(i) = self.types.iter().position(|t| t.is_none()) {
            self.types[i] = Some(t);
            i
        } else {
            self.types.push(Some(t));
            self.types.len() - 1
        }
    }

    pub fn type_get(&mut self, i: usize) -> Option<&mut PartType> {
        self.types.get_mut(i).and_then(|t| t.as_mut())
    }

    pub fn type_destroy(&mut self, i: usize) {
        if let Some(t) = self.types.get_mut(i) {
            *t = None;
        }
        for s in self.systems.iter_mut().flatten() {
            s.parts.retain(|p| p.ty != i);
        }
    }

    /// A runtime type from its file description. `sprite` resolves sprite names.
    pub fn type_from_toml(t: &PartTypeToml, sprite: &dyn Fn(&str) -> Option<u32>) -> Result<PartType, String> {
        let tex = match &t.sprite {
            Some(s) if !s.is_empty() => Tex::Sprite(sprite(s).ok_or_else(|| format!("type '{}': sprite '{s}' does not exist", t.name))?),
            _ => Tex::Shape(shape_index(&t.shape).ok_or_else(|| format!("type '{}': unknown shape '{}'", t.name, t.shape))? as u8),
        };
        let colors = t
            .colors
            .iter()
            .take(3)
            .map(|c| parse_color(c).ok_or_else(|| format!("type '{}': bad colour '{c}'", t.name)))
            .collect::<Result<Vec<u32>, String>>()?;
        Ok(PartType {
            tex,
            animate: t.animate,
            stretch: t.stretch,
            random_frame: t.random_frame,
            size: t.size.0,
            size_incr: t.size_incr,
            size_wiggle: t.size_wiggle,
            scale: t.scale,
            speed: t.speed.0,
            speed_incr: t.speed_incr,
            speed_wiggle: t.speed_wiggle,
            dir: t.direction.0,
            dir_incr: t.direction_incr,
            dir_wiggle: t.direction_wiggle,
            grav: t.gravity,
            grav_dir: t.gravity_dir,
            orient: t.orientation.0,
            orient_incr: t.orientation_incr,
            orient_wiggle: t.orientation_wiggle,
            orient_rel: t.orientation_relative,
            colors: if colors.is_empty() { vec![0xFFFFFF] } else { colors },
            color_mix: t.color_mode == "mix",
            color_rgb: None,
            color_hsv: None,
            alphas: if t.alphas.is_empty() { vec![1.0] } else { t.alphas.iter().take(3).copied().collect() },
            additive: t.additive,
            life: t.life.0,
            step: (t.step_count, None),
            death: (t.death_count, None),
        })
    }

    // ------------------------------------------------------------ systems

    pub fn system_create(&mut self, s: System) -> usize {
        if let Some(i) = self.systems.iter().position(|s| s.is_none()) {
            self.systems[i] = Some(s);
            i
        } else {
            self.systems.push(Some(s));
            self.systems.len() - 1
        }
    }

    pub fn sys(&mut self, i: usize) -> Option<&mut System> {
        self.systems.get_mut(i).and_then(|s| s.as_mut())
    }

    pub fn system_destroy(&mut self, i: usize) {
        if let Some(Some(s)) = self.systems.get_mut(i).map(|s| s.take()) {
            for t in s.own_types {
                self.type_destroy(t);
            }
        }
    }

    pub fn clear(&mut self) {
        self.types.clear();
        self.systems.clear();
    }

    /// Instantiates asset `a` as a new system at (x, y). Returns its id.
    pub fn system_from_asset(&mut self, a: usize, x: f64, y: f64, sprite: &dyn Fn(&str) -> Option<u32>) -> Result<usize, String> {
        let file = self.assets.get(a).map(|(_, f)| f.clone()).ok_or("no such particle system")?;
        let mut ids: HashMap<String, usize> = HashMap::new();
        let mut own = vec![];
        for t in &file.types {
            let pt = Self::type_from_toml(t, sprite)?;
            let id = self.type_create(pt);
            ids.insert(t.name.clone(), id);
            own.push(id);
        }
        // step / death types refer to other types of the same file
        for t in &file.types {
            let id = ids[&t.name];
            let step = t.step_type.as_ref().and_then(|n| ids.get(n)).copied();
            let death = t.death_type.as_ref().and_then(|n| ids.get(n)).copied();
            if let Some(pt) = self.type_get(id) {
                pt.step.1 = step;
                pt.death.1 = death;
            }
        }
        let mut sys = System { x, y, depth: file.depth, new_below: file.new_below, own_types: own, asset: Some(a as u32), ..System::new() };
        for e in &file.emitters {
            let [x1, y1, x2, y2] = e.region;
            let burst = e.mode == "burst";
            let mut em = Emitter {
                name: e.name.clone(),
                ty: ids.get(&e.ty).copied(),
                shape: RegionShape::parse(&e.shape).unwrap_or(RegionShape::Rectangle),
                distr: Distr::parse(&e.distribution).unwrap_or(Distr::Linear),
                region: [x1.min(x2), x1.max(x2), y1.min(y2), y1.max(y2)],
                stream: if burst { 0 } else { e.count },
                burst: if burst { e.count } else { 0 },
                interval: e.interval.0,
                enabled: e.enabled,
                wait: 0.0,
                burst_due: burst,
            };
            if e.shape == "line" {
                // a line keeps its direction (x1,y1 → x2,y2)
                em.region = [x1, x2, y1, y2];
            }
            em.wait = self.range(e.delay.0).max(0.0);
            sys.emitters.push(Some(em));
        }
        let id = self.system_create(sys);
        // bursts without delay fire right away (GameMaker emits them when the system is created)
        self.fire_due_bursts(id);
        Ok(id)
    }

    fn fire_due_bursts(&mut self, sys: usize) {
        let Some(s) = self.sys(sys) else { return };
        let mut todo = vec![];
        for (k, e) in s.emitters.iter_mut().enumerate() {
            if let Some(e) = e {
                if e.enabled && e.burst_due && e.wait <= 0.0 {
                    todo.push(k);
                }
            }
        }
        for k in todo {
            let (n, ty) = {
                let e = self.sys(sys).and_then(|s| s.emitters[k].as_mut()).unwrap();
                e.burst_due = false;
                (e.burst, e.ty)
            };
            if let Some(ty) = ty {
                self.emitter_emit(sys, k, ty, n);
            }
            let iv = self.sys(sys).and_then(|s| s.emitters[k].as_ref()).map(|e| e.interval).unwrap_or([0.0, 0.0]);
            if iv[1] > 0.0 {
                let w = self.range(iv).max(1.0);
                if let Some(e) = self.sys(sys).and_then(|s| s.emitters[k].as_mut()) {
                    e.wait = w;
                    e.burst_due = true;
                }
            }
        }
    }

    // ------------------------------------------------------------ creating particles

    /// Creates `n` particles of type `ty` at (x, y) in system coordinates (`n < 0`: one particle
    /// with a 1/-n chance). `color` overrides the type's colour.
    pub fn create(&mut self, sys: usize, x: f64, y: f64, ty: usize, n: i32, color: Option<u32>) {
        let n = if n < 0 {
            if self.rand() * (-n as f64) < 1.0 {
                1
            } else {
                0
            }
        } else {
            n
        };
        let Some(t) = self.types.get(ty).and_then(|t| t.clone()) else { return };
        for _ in 0..n {
            let len = self.sys(sys).map(|s| s.parts.len()).unwrap_or(MAX_PARTICLES);
            if len >= MAX_PARTICLES {
                return;
            }
            let life = self.range(t.life).max(1.0);
            let fixed = if let Some(c) = color {
                Some(c)
            } else if let Some(r) = t.color_rgb {
                let (a, b, c) = (self.range([r[0], r[1]]), self.range([r[2], r[3]]), self.range([r[4], r[5]]));
                Some((a.round().clamp(0.0, 255.0) as u32) << 16 | (b.round().clamp(0.0, 255.0) as u32) << 8 | c.round().clamp(0.0, 255.0) as u32)
            } else if let Some(h) = t.color_hsv {
                let (a, b, c) = (self.range([h[0], h[1]]), self.range([h[2], h[3]]), self.range([h[4], h[5]]));
                Some(hsv_to_rgb(a, b, c))
            } else if t.color_mix && t.colors.len() >= 2 {
                let k = self.rand();
                Some(lerp_rgb(t.colors[0], t.colors[1], k))
            } else {
                None
            };
            let p = Particle {
                ty,
                x,
                y,
                speed: self.range(t.speed),
                dir: self.range(t.dir),
                size: self.range(t.size),
                orient: self.range(t.orient),
                age: 0.0,
                life,
                color: fixed,
                frame: if t.random_frame { (self.rand() * 1000.0).floor() } else { 0.0 },
                seed: self.rand() * std::f64::consts::TAU,
            };
            if let Some(s) = self.sys(sys) {
                s.parts.push(p);
            }
        }
    }

    /// A random point inside an emitter's region.
    pub fn region_point(&mut self, e: &Emitter) -> (f64, f64) {
        let [x1, x2, y1, y2] = e.region;
        let (cx, cy, rx, ry) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0, (x2 - x1) / 2.0, (y2 - y1) / 2.0);
        match e.shape {
            RegionShape::Line => {
                let t = self.distr01(e.distr);
                (lerp(x1, x2, t), lerp(y1, y2, t))
            }
            RegionShape::Rectangle => {
                let (a, b) = (self.distr01(e.distr), self.distr01(e.distr));
                (lerp(x1, x2, a), lerp(y1, y2, b))
            }
            RegionShape::Ellipse | RegionShape::Diamond => {
                for _ in 0..32 {
                    let (a, b) = (self.distr01(e.distr) * 2.0 - 1.0, self.distr01(e.distr) * 2.0 - 1.0);
                    let inside = match e.shape {
                        RegionShape::Ellipse => a * a + b * b <= 1.0,
                        _ => a.abs() + b.abs() <= 1.0,
                    };
                    if inside {
                        return (cx + a * rx, cy + b * ry);
                    }
                }
                (cx, cy)
            }
        }
    }

    pub fn emitter_emit(&mut self, sys: usize, em: usize, ty: usize, n: i32) {
        let Some(e) = self.sys(sys).and_then(|s| s.emitters.get(em).cloned().flatten()) else { return };
        let count = if n < 0 {
            if self.rand() * (-n as f64) < 1.0 {
                1
            } else {
                0
            }
        } else {
            n
        };
        for _ in 0..count {
            let (x, y) = self.region_point(&e);
            self.create(sys, x, y, ty, 1, None);
        }
    }

    // ------------------------------------------------------------ simulation

    /// Advances every system with automatic update on.
    pub fn step(&mut self) {
        for i in 0..self.systems.len() {
            if self.systems[i].as_ref().is_some_and(|s| s.auto_update) {
                self.system_update(i);
            }
        }
    }

    /// One step of system `sys`: emitters, then particle motion, ageing and spawning.
    pub fn system_update(&mut self, sys: usize) {
        // particles first, so new ones start at age 0 and live exactly `life` steps
        let Some(s) = self.systems.get_mut(sys).and_then(|s| s.as_mut()) else { return };
        let mut parts = std::mem::take(&mut s.parts);
        let mut spawns: Vec<(f64, f64, usize, i32)> = vec![];
        parts.retain_mut(|p| {
            let Some(Some(t)) = self.types.get(p.ty) else { return false };
            p.age += 1.0;
            if p.age >= p.life {
                if let (n, Some(d)) = t.death {
                    if n != 0 {
                        spawns.push((p.x, p.y, d, n));
                    }
                }
                return false;
            }
            p.speed = (p.speed + t.speed_incr).max(0.0);
            p.dir += t.dir_incr;
            p.size = (p.size + t.size_incr).max(0.0);
            p.orient += t.orient_incr;
            let r = p.dir.to_radians();
            let (mut hs, mut vs) = (p.speed * r.cos(), -p.speed * r.sin());
            if t.grav != 0.0 {
                let g = t.grav_dir.to_radians();
                hs += t.grav * g.cos() * p.age.min(1.0);
                vs -= t.grav * g.sin() * p.age.min(1.0);
                p.speed = (hs * hs + vs * vs).sqrt();
                if p.speed > 0.0 {
                    p.dir = (-vs).atan2(hs).to_degrees();
                }
            }
            // wiggles oscillate around the base value (they don't accumulate)
            let w = (p.age * 0.35 + p.seed).sin();
            let ws = p.speed + t.speed_wiggle * w;
            let wd = (p.dir + t.dir_wiggle * w).to_radians();
            if t.speed_wiggle != 0.0 || t.dir_wiggle != 0.0 {
                hs = ws * wd.cos();
                vs = -ws * wd.sin();
            }
            p.x += hs;
            p.y += vs;
            if let (n, Some(st)) = t.step {
                if n != 0 {
                    spawns.push((p.x, p.y, st, n));
                }
            }
            true
        });
        if let Some(s) = self.systems.get_mut(sys).and_then(|s| s.as_mut()) {
            s.parts = parts;
        }
        for (x, y, t, n) in spawns {
            self.create(sys, x, y, t, n, None);
        }
        // emitters
        let Some(s) = self.sys(sys) else { return };
        let mut streams = vec![];
        for (k, e) in s.emitters.iter_mut().enumerate() {
            let Some(e) = e else { continue };
            if !e.enabled {
                continue;
            }
            if e.wait > 0.0 {
                e.wait -= 1.0;
                continue;
            }
            if e.stream != 0 && !e.burst_due {
                if let Some(t) = e.ty {
                    streams.push((k, t, e.stream));
                }
            }
        }
        for (k, t, n) in streams {
            self.emitter_emit(sys, k, t, n);
            // a stream with an interval emits every `interval` steps instead of every step
            let iv = self.sys(sys).and_then(|s| s.emitters[k].as_ref()).map(|e| e.interval).unwrap_or([0.0, 0.0]);
            if iv[1] > 0.0 {
                let w = (self.range(iv) - 1.0).max(0.0);
                if let Some(e) = self.sys(sys).and_then(|s| s.emitters[k].as_mut()) {
                    e.wait = w;
                }
            }
        }
        self.fire_due_bursts(sys);

    }

    /// What to draw for system `sys`, in drawing order, in room coordinates.
    /// `frames(sprite)` gives the number of frames of a sprite.
    pub fn draw_list(&self, sys: usize, frames: &dyn Fn(u32) -> u32) -> Vec<PartDraw> {
        let Some(Some(s)) = self.systems.get(sys) else { return vec![] };
        let (sa, ca) = s.angle.to_radians().sin_cos();
        let mut out = Vec::with_capacity(s.parts.len());
        let mut push = |p: &Particle| {
            let Some(Some(t)) = self.types.get(p.ty) else { return };
            let f = (p.age / p.life).clamp(0.0, 1.0);
            let w = (p.age * 0.35 + p.seed).sin();
            let size = (p.size + t.size_wiggle * w).max(0.0);
            let mut angle = p.orient + t.orient_wiggle * w;
            if t.orient_rel {
                angle += p.dir;
            }
            let base = p.color.unwrap_or_else(|| color_curve(&t.colors, f));
            let color = if s.tint == 0xFFFFFF { base } else { mul_rgb(base, s.tint) };
            let alpha = curve(&t.alphas, f) * s.alpha;
            let (tex, frame) = match t.tex {
                Tex::Shape(k) => (Tex::Shape(k), 0),
                Tex::Sprite(spr) => {
                    let n = frames(spr).max(1);
                    let fr = if t.stretch {
                        (f * n as f64).floor() as i64
                    } else if t.animate {
                        p.frame as i64 + (p.age * 0.25).floor() as i64
                    } else {
                        p.frame as i64
                    };
                    (Tex::Sprite(spr), fr.rem_euclid(n as i64) as u32)
                }
            };
            // the system's position and angle transform everything it draws
            let (x, y) = (p.x * ca + p.y * sa, -p.x * sa + p.y * ca);
            out.push(PartDraw {
                tex,
                frame,
                x: x + s.x,
                y: y + s.y,
                sx: size * t.scale[0],
                sy: size * t.scale[1],
                angle: angle + s.angle,
                color,
                alpha,
                additive: t.additive,
            });
        };
        if s.new_below {
            s.parts.iter().rev().for_each(&mut push);
        } else {
            s.parts.iter().for_each(&mut push);
        }
        out
    }

    pub fn count(&self, sys: usize) -> usize {
        self.systems.get(sys).and_then(|s| s.as_ref()).map(|s| s.parts.len()).unwrap_or(0)
    }
}

fn mul_rgb(a: u32, b: u32) -> u32 {
    let ch = |c: u32, s: u32| ((c >> s) & 255) as u32;
    let mut out = 0;
    for s in [16, 8, 0] {
        out |= (ch(a, s) * ch(b, s) / 255) << s;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fire() -> PartSysToml {
        PartSysToml::parse(
            r##"
depth = -5
[[type]]
name = "flame"
shape = "flare"
size = [0.2, 0.4]
speed = [1, 2]
direction = 90
colors = ["#ffff00", "#ff0000"]
alphas = [1, 0]
life = 10
death_type = "smoke"
death_count = 1

[[type]]
name = "smoke"
shape = "smoke"
life = [5, 5]
speed = 0

[[emitter]]
name = "base"
type = "flame"
shape = "ellipse"
region = [-10, -2, 10, 2]
count = 3

[[emitter]]
name = "pop"
type = "smoke"
mode = "burst"
count = 7
"##,
        )
        .unwrap()
    }

    #[test]
    fn parses_ranges_and_round_trips() {
        let f = fire();
        assert_eq!(f.types[0].direction, Range2([90.0, 90.0]));
        assert_eq!(f.types[0].speed, Range2([1.0, 2.0]));
        assert_eq!(f.emitters[1].mode, "burst");
        assert!(f.validate(&|_| false).is_empty(), "{:?}", f.validate(&|_| false));
        let again = PartSysToml::parse(&f.to_toml()).unwrap();
        assert_eq!(again, f);
    }

    #[test]
    fn validation_finds_bad_names() {
        let mut f = fire();
        f.emitters[0].ty = "nope".into();
        f.types[0].shape = "blob".into();
        let errs = f.validate(&|_| false);
        assert_eq!(errs.len(), 2, "{errs:?}");
    }

    #[test]
    fn simulation_streams_bursts_moves_and_dies() {
        let mut w = PartWorld::new(vec![("fire".into(), fire())]);
        let s = w.system_from_asset(0, 100.0, 50.0, &|_| None).unwrap();
        assert_eq!(w.count(s), 7, "the burst fires when the system is created");
        w.system_update(s);
        // 3 streamed flames + 7 smoke puffs
        assert_eq!(w.count(s), 10);
        let flame = w.systems[s].as_ref().unwrap().parts.iter().find(|p| p.age == 0.0).unwrap().clone();
        assert!(flame.y.abs() <= 2.0 + 1e-9 && flame.x.abs() <= 10.0 + 1e-9, "inside the ellipse region");
        for _ in 0..5 {
            w.system_update(s);
        }
        // flames move up (direction 90 = up = -y)
        let sys = w.systems[s].as_ref().unwrap();
        let moving: Vec<&Particle> = sys.parts.iter().filter(|p| p.speed > 0.0).collect();
        assert!(!moving.is_empty() && moving.iter().all(|p| p.y < 2.0));
        // after many steps the count is stable: 3 new flames per step living 10 steps, each
        // leaving one smoke puff living 5 steps
        for _ in 0..40 {
            w.system_update(s);
        }
        let n = w.count(s);
        assert!((40..=50).contains(&n), "steady state, got {n}");
        let d = w.draw_list(s, &|_| 1);
        assert_eq!(d.len(), n);
        assert!(d.iter().all(|p| p.x > 80.0 && p.x < 120.0), "drawn around the system position");
        w.system_destroy(s);
        assert!(w.types.iter().all(|t| t.is_none()), "asset types die with the system");
    }

    #[test]
    fn colour_and_alpha_follow_life() {
        let mut w = PartWorld::default();
        let t = w.type_create(PartType { colors: vec![0xFF0000, 0x0000FF], alphas: vec![1.0, 0.0], life: [10.0, 10.0], ..Default::default() });
        let s = w.system_create(System::new());
        w.create(s, 0.0, 0.0, t, 1, None);
        for _ in 0..5 {
            w.system_update(s);
        }
        let d = w.draw_list(s, &|_| 1)[0];
        assert_eq!(d.color, 0x800080);
        assert!((d.alpha - 0.5).abs() < 1e-9);
    }

    #[test]
    fn shapes_have_content() {
        for k in 0..SHAPE_NAMES.len() {
            let img = shape_image(k);
            let opaque = img.chunks(4).filter(|p| p[3] > 128).count();
            assert!(opaque > 0, "shape {} is empty", SHAPE_NAMES[k]);
        }
    }
}

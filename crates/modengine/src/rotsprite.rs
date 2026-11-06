//! Rotating pixel art without the jaggies of nearest-neighbour rotation: RotSprite
//! (Scale2x three times → 8×, rotate with nearest sampling, scale back down picking the most
//! common colour of each pixel). Backends compute one rotated image per (sprite frame, angle
//! step) and cache it; see `rotation` in sprite TOML files and `[display]` of game.toml.
//!
//! Angles are degrees counter-clockwise on screen, as `image_angle`.

use serde::Deserialize;

/// How a sprite looks when drawn rotated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RotMode {
    /// The GPU rotates the texture with nearest-neighbour sampling (fast, jagged edges).
    #[default]
    Nearest,
    /// RotSprite: clean pixel-art rotation, computed once per angle step and cached.
    Rotsprite,
    /// Linear filtering for this sprite (blurry but smooth), rotated by the GPU.
    Smooth,
}

impl RotMode {
    pub fn parse(s: &str) -> Result<RotMode, String> {
        match s.trim().to_lowercase().as_str() {
            "nearest" | "" => Ok(RotMode::Nearest),
            "rotsprite" => Ok(RotMode::Rotsprite),
            "smooth" | "linear" => Ok(RotMode::Smooth),
            other => Err(format!("unknown rotation '{other}' (use \"nearest\", \"rotsprite\" or \"smooth\")")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RotMode::Nearest => "nearest",
            RotMode::Rotsprite => "rotsprite",
            RotMode::Smooth => "smooth",
        }
    }
}

/// Rotation settings of a sprite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rotation {
    pub mode: RotMode,
    /// Snap angles to this many steps per turn (0 = free). RotSprite caches one image per
    /// step; with 0 it uses 1° steps.
    pub steps: u32,
}

impl Rotation {
    /// The angle actually drawn for `angle` (snapped to `steps`).
    pub fn snap(&self, angle: f32) -> f32 {
        if self.steps == 0 {
            return angle;
        }
        let step = 360.0 / self.steps as f32;
        (angle / step).round() * step
    }

    /// Cache bucket (0..buckets) of an angle for RotSprite.
    pub fn bucket(&self, angle: f32) -> (u32, u32) {
        let n = if self.steps == 0 { 360 } else { self.steps.min(3600) };
        let a = angle.rem_euclid(360.0);
        let b = ((a / 360.0 * n as f32).round() as u32) % n;
        (b, n)
    }
}

/// How to draw a rotated, scaled and maybe mirrored sprite from an unscaled rotated image:
/// the image angle and whether to mirror the result. The backend's model is "mirror in
/// place, then rotate about the origin": R(a)·M_x = M_x·R(-a), and mirroring both axes is a
/// half turn.
pub fn plan(sx: f32, sy: f32, angle: f32) -> (f32, bool, bool) {
    match (sx < 0.0, sy < 0.0) {
        (false, false) => (angle, false, false),
        (true, false) => (-angle, true, false),
        (false, true) => (-angle, false, true),
        (true, true) => (angle + 180.0, false, false),
    }
}

/// An RGBA image.
#[derive(Clone, Debug, PartialEq)]
pub struct Img {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u32>,
}

impl Img {
    /// Frame `[x, y, w, h]` of an RGBA sheet.
    pub fn from_sheet(rgba: &[u8], sheet_w: u32, fr: [u32; 4]) -> Img {
        let mut px = Vec::with_capacity((fr[2] * fr[3]) as usize);
        for y in 0..fr[3] {
            for x in 0..fr[2] {
                let i = (((fr[1] + y) * sheet_w + fr[0] + x) * 4) as usize;
                px.push(rgba.get(i..i + 4).map(|p| u32::from_le_bytes([p[0], p[1], p[2], p[3]])).unwrap_or(0));
            }
        }
        Img { w: fr[2], h: fr[3], px }
    }

    pub fn to_rgba(&self) -> Vec<u8> {
        self.px.iter().flat_map(|p| p.to_le_bytes()).collect()
    }

    fn at(&self, x: i64, y: i64) -> u32 {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return 0;
        }
        self.px[(y as u32 * self.w + x as u32) as usize]
    }
}

/// Fully transparent pixels compare equal whatever their RGB.
fn same(a: u32, b: u32) -> bool {
    a == b || (a >> 24 == 0 && b >> 24 == 0)
}

/// Scale2x (EPX): doubles the image keeping edges sharp and smoothing diagonal steps.
pub fn scale2x(src: &Img) -> Img {
    let (w, h) = (src.w, src.h);
    let mut px = vec![0u32; (w * 2 * h * 2) as usize];
    let ow = w * 2;
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let p = src.at(x, y);
            let a = if y > 0 { src.at(x, y - 1) } else { p };
            let b = if x + 1 < w as i64 { src.at(x + 1, y) } else { p };
            let c = if x > 0 { src.at(x - 1, y) } else { p };
            let d = if y + 1 < h as i64 { src.at(x, y + 1) } else { p };
            let (mut e0, mut e1, mut e2, mut e3) = (p, p, p, p);
            if !same(c, b) && !same(a, d) {
                if same(a, c) {
                    e0 = a;
                }
                if same(a, b) {
                    e1 = b;
                }
                if same(d, c) {
                    e2 = c;
                }
                if same(d, b) {
                    e3 = d;
                }
            }
            let (ox, oy) = (x as u32 * 2, y as u32 * 2);
            px[(oy * ow + ox) as usize] = e0;
            px[(oy * ow + ox + 1) as usize] = e1;
            px[((oy + 1) * ow + ox) as usize] = e2;
            px[((oy + 1) * ow + ox + 1) as usize] = e3;
        }
    }
    Img { w: ow, h: h * 2, px }
}

/// The 8× Scale2x image RotSprite samples from (compute once per frame).
pub fn upscale8(src: &Img) -> Img {
    scale2x(&scale2x(&scale2x(src)))
}

/// A rotated frame: the image and where the origin (pivot) lands inside it.
#[derive(Clone, Debug, PartialEq)]
pub struct Rotated {
    pub img: Img,
    pub ox: f32,
    pub oy: f32,
}

/// Placement of a frame of size w×h rotated by `angle` about (px, py): output size and the
/// pivot inside it. The pivot keeps its sub-pixel offset, so angle 0 maps pixels 1:1.
fn bounds(w: u32, h: u32, px: f32, py: f32, angle: f32) -> (u32, u32, f32, f32) {
    let r = -angle.to_radians();
    let (c, s) = (r.cos(), r.sin());
    let corners = [(-px, -py), (w as f32 - px, -py), (-px, h as f32 - py), (w as f32 - px, h as f32 - py)];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in corners {
        let (rx, ry) = (x * c - y * s, x * s + y * c);
        x0 = x0.min(rx);
        y0 = y0.min(ry);
        x1 = x1.max(rx);
        y1 = y1.max(ry);
    }
    let eps = 1e-3;
    let (fx, fy) = (px - px.floor(), py - py.floor());
    let kx = (-x0 - fx - eps).ceil().max(0.0);
    let ky = (-y0 - fy - eps).ceil().max(0.0);
    let (ox, oy) = (fx + kx, fy + ky);
    let ow = (ox + x1 - eps).ceil().max(1.0) as u32;
    let oh = (oy + y1 - eps).ceil().max(1.0) as u32;
    (ow, oh, ox, oy)
}

/// RotSprite of one frame `src` (its [`upscale8`] is `big`) about the origin (px, py).
pub fn rotate(src: &Img, big: &Img, px: f32, py: f32, angle: f32) -> Rotated {
    let (ow, oh, ox, oy) = bounds(src.w, src.h, px, py, angle);
    // output offset q (from the pivot) comes from source offset R(angle)·q
    let r = angle.to_radians();
    let (c, s) = (r.cos(), r.sin());
    // 4×4 samples of the 8× image per output pixel, one per 2×2 block of it, so every
    // Scale2x quadrant weighs the same; the most common colour wins, ties go to the colour
    // plain nearest rotation would pick
    const SUB: [f32; 4] = [-0.375, -0.125, 0.125, 0.375];
    let mut out = vec![0u32; (ow * oh) as usize];
    let mut votes: [(u32, u32); 16] = [(0, 0); 16];
    for y in 0..oh {
        for x in 0..ow {
            let (qx, qy) = (x as f32 + 0.5 - ox, y as f32 + 0.5 - oy);
            let (cx, cy) = (qx * c - qy * s + px, qx * s + qy * c + py);
            let near = src.at(cx.floor() as i64, cy.floor() as i64);
            let near = if near >> 24 == 0 { 0 } else { near };
            let mut n = 0usize;
            for dy in SUB {
                for dx in SUB {
                    let (sx, sy) = (cx + dx * c - dy * s, cy + dx * s + dy * c);
                    let col = big.at((sx * 8.0).floor() as i64, (sy * 8.0).floor() as i64);
                    let col = if col >> 24 == 0 { 0 } else { col };
                    match votes[..n].iter_mut().find(|v| v.0 == col) {
                        Some(v) => v.1 += 2,
                        None => {
                            votes[n] = (col, 2);
                            n += 1;
                        }
                    }
                }
            }
            if let Some(v) = votes[..n].iter_mut().find(|v| v.0 == near) {
                v.1 += 1;
            }
            out[(y * ow + x) as usize] = votes[..n].iter().max_by_key(|v| v.1).map(|v| v.0).unwrap_or(0);
        }
    }
    Rotated { img: Img { w: ow, h: oh, px: out }, ox, oy }
}

/// Plain nearest-neighbour rotation into the same layout as [`rotate`] (for comparisons).
pub fn rotate_nearest(src: &Img, px: f32, py: f32, angle: f32) -> Rotated {
    let (ow, oh, ox, oy) = bounds(src.w, src.h, px, py, angle);
    let r = angle.to_radians();
    let (c, s) = (r.cos(), r.sin());
    let mut out = vec![0u32; (ow * oh) as usize];
    for y in 0..oh {
        for x in 0..ow {
            let (qx, qy) = (x as f32 + 0.5 - ox, y as f32 + 0.5 - oy);
            let (sx, sy) = (qx * c - qy * s + px, qx * s + qy * c + py);
            out[(y * ow + x) as usize] = src.at(sx.floor() as i64, sy.floor() as i64);
        }
    }
    Rotated { img: Img { w: ow, h: oh, px: out }, ox, oy }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u32 = 0xFF0000FF;
    const B: u32 = 0xFFFF0000;

    fn img(rows: &[&str]) -> Img {
        let h = rows.len() as u32;
        let w = rows[0].len() as u32;
        let px = rows.iter().flat_map(|r| r.chars().map(|c| if c == 'r' { R } else if c == 'b' { B } else { 0 })).collect();
        Img { w, h, px }
    }

    #[test]
    fn zero_angle_is_identity_and_quarter_turns_are_exact() {
        let a = img(&["rr..", "rbb.", "..b.", "...b", "r..."]);
        let big = upscale8(&a);
        assert_eq!((big.w, big.h), (32, 40));
        for (px, py) in [(0.0, 0.0), (2.0, 2.5), (1.5, 1.5)] {
            let r = rotate(&a, &big, px, py, 0.0);
            assert_eq!((r.img.w, r.img.h), (a.w, a.h), "origin {px},{py}");
            assert_eq!((r.ox, r.oy), (px, py));
            assert!(r.img.px == a.px, "angle 0 changed the pixels");
        }
        // 90° counter-clockwise on screen: the top row becomes the left column, bottom-up
        let r = rotate(&a, &big, 0.0, 0.0, 90.0);
        assert_eq!((r.img.w, r.img.h), (5, 4));
        let n = rotate_nearest(&a, 0.0, 0.0, 90.0);
        assert!(r.img.px == n.img.px, "quarter turns are lossless");
        assert_eq!(r.img.at(0, 3), R, "top-left pixel goes to the bottom-left");
        assert_eq!((r.ox, r.oy), (0.0, 4.0));
    }

    #[test]
    fn scale2x_smooths_diagonals_only() {
        let d = img(&["r.", ".r"]);
        let s = scale2x(&d);
        // the diagonal becomes a connected line: the empty pixels next to it gain the
        // inner corners, the pixels on it lose their outer ones
        assert_eq!(s.at(2, 1), R);
        assert_eq!(s.at(1, 2), R);
        assert_eq!(s.at(1, 1), 0);
        assert_eq!(s.at(0, 0), R);
        let flat = img(&["rr", "rr"]);
        assert!(scale2x(&flat).px.iter().all(|p| *p == R));
    }

    #[test]
    fn rotsprite_keeps_colours_and_has_fewer_holes_than_nearest() {
        // a thick ring: nearest rotation at odd angles leaves isolated pixels and holes
        let n = 24;
        let rows: Vec<String> = (0..n)
            .map(|y| {
                (0..n)
                    .map(|x| {
                        let d = ((x as f32 - 11.5).powi(2) + (y as f32 - 11.5).powi(2)).sqrt();
                        if (7.0..10.5).contains(&d) {
                            if x < 12 {
                                'r'
                            } else {
                                'b'
                            }
                        } else {
                            '.'
                        }
                    })
                    .collect()
            })
            .collect();
        let refs: Vec<&str> = rows.iter().map(|s| s.as_str()).collect();
        let a = img(&refs);
        let big = upscale8(&a);
        for ang in [17.0f32, 33.0, 45.0, 71.0] {
            let r = rotate(&a, &big, 12.0, 12.0, ang);
            assert!(r.img.px.iter().all(|p| *p == 0 || *p == R || *p == B), "only palette colours");
            let count = |i: &Img| i.px.iter().filter(|p| **p != 0).count() as f32;
            let ratio = count(&r.img) / count(&a);
            assert!((0.85..1.15).contains(&ratio), "area kept at {ang}°: {ratio}");
            // isolated opaque pixels (no 4-neighbour of the same colour) are the jaggies
            let lonely = |i: &Img| {
                let mut k = 0;
                for y in 0..i.h as i64 {
                    for x in 0..i.w as i64 {
                        let p = i.at(x, y);
                        if p != 0 && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|(dx, dy)| i.at(x + dx, y + dy) != p) {
                            k += 1;
                        }
                    }
                }
                k
            };
            let nr = rotate_nearest(&a, 12.0, 12.0, ang);
            assert!(lonely(&r.img) <= lonely(&nr.img), "{ang}°: rotsprite {} vs nearest {}", lonely(&r.img), lonely(&nr.img));
        }
    }

    #[test]
    fn plans_and_buckets() {
        assert_eq!(plan(2.0, 2.0, 30.0), (30.0, false, false));
        assert_eq!(plan(-2.0, 2.0, 30.0), (-30.0, true, false));
        assert_eq!(plan(-1.0, -1.0, 30.0), (210.0, false, false));
        let r = Rotation { mode: RotMode::Rotsprite, steps: 8 };
        assert_eq!(r.snap(50.0), 45.0);
        assert_eq!(r.bucket(-45.0), (7, 8));
        assert_eq!(Rotation::default().bucket(359.7), (0, 360));
        assert_eq!(RotMode::parse("RotSprite").unwrap(), RotMode::Rotsprite);
        assert!(RotMode::parse("bilinear").is_err());
    }
}

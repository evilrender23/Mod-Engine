//! First-person grid views ("blobber" dungeons: Eye of the Beholder, Legend of Grimrock),
//! textured primitives (`draw_sprite_pos`, `draw_primitive_*`) and the pixelate
//! post-process. Engine side only: state, world geometry, projection, picking and the
//! movement helpers, all pure and testable. The GPU part lives in `modengine-cli/src/fpview.rs`.
//!
//! World conventions of a view:
//! - cell (x, y) of the map covers `[x, x+1] × [y, y+1]` on the ground plane; world X goes
//!   east, world Z goes south (map rows), world Y goes up; walls are 1 unit tall.
//! - the camera at cell coordinates (x, y) stands at the *centre* of that cell
//!   (`x + 0.5, y + 0.5`), so a game tweens `x` from 3 to 4 to step one cell east.
//! - angles are GameMaker's: degrees, 0 = east, 90 = north (up on the map), counter-clockwise.
//!   Grid directions are `dir` 0 = east, 1 = north, 2 = west, 3 = south (`angle = dir * 90`).

use crate::draw::Rgba;
use crate::project::Sprite;
use std::collections::HashMap;
use std::rc::Rc;

// ------------------------------------------------------------------ grid directions

/// Map offsets of the four grid directions (0 east, 1 north, 2 west, 3 south).
pub const DIRS: [(i64, i64); 4] = [(1, 0), (0, -1), (-1, 0), (0, 1)];

/// Any integer direction wrapped to 0..4.
pub fn norm_dir(d: i64) -> usize {
    d.rem_euclid(4) as usize
}

pub fn dir_dx(d: i64) -> i64 {
    DIRS[norm_dir(d)].0
}

pub fn dir_dy(d: i64) -> i64 {
    DIRS[norm_dir(d)].1
}

/// The grid direction closest to an angle in degrees.
pub fn angle_to_dir(angle: f64) -> usize {
    ((angle / 90.0).round() as i64).rem_euclid(4) as usize
}

// ------------------------------------------------------------------ view state

/// What covers a floor or ceiling cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    /// Nothing (the background / fog colour shows through).
    None,
    Color(u32),
    Tex { spr: u32, frame: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct WallType {
    /// Sprite and frame (None: a flat colour).
    pub tex: Option<(u32, u32)>,
    /// Tint (0xRRGGBB) multiplied with the texture.
    pub color: u32,
    /// Blocks movement (`fpview_can_move`).
    pub solid: bool,
}

impl Default for WallType {
    fn default() -> Self {
        WallType { tex: None, color: 0x808080, solid: true }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Billboard {
    /// Cell coordinates (the centre of the cell at integer values).
    pub x: f32,
    pub y: f32,
    pub spr: u32,
    pub frame: u32,
    /// Height in cells (width follows the sprite's aspect).
    pub scale: f32,
    /// Height of the bottom above the floor, in cells.
    pub yoff: f32,
    pub color: u32,
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cam {
    pub x: f32,
    pub y: f32,
    /// Degrees, 0 = east, 90 = north.
    pub angle: f32,
    /// Eye height (0 floor, 1 ceiling).
    pub height: f32,
    /// Degrees up (positive) or down.
    pub pitch: f32,
}

/// One world quad: corners top-left, top-right, bottom-right, bottom-left (as seen from the
/// front); `uv` = (u0, v0, u1, v1) in the texture (normalised sheet coordinates).
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub p: [[f32; 3]; 4],
    pub uv: [f32; 4],
    /// Sprite whose sheet textures it (None: white, i.e. the plain colour).
    pub tex: Option<u32>,
    pub color: [u8; 4],
}

impl Face {
    fn center(&self) -> (f32, f32) {
        ((self.p[0][0] + self.p[2][0]) * 0.5, (self.p[0][2] + self.p[2][2]) * 0.5)
    }
}

pub struct FpView {
    /// Internal resolution of the 3D image.
    pub w: u32,
    pub h: u32,
    pub mw: usize,
    pub mh: usize,
    /// Row-major cells: 0 (or negative) empty, n > 0 = wall type n.
    pub cells: Vec<i32>,
    pub walls: HashMap<i32, WallType>,
    pub floor: Fill,
    pub ceiling: Fill,
    pub cell_floor: HashMap<(i64, i64), Fill>,
    pub cell_ceiling: HashMap<(i64, i64), Fill>,
    /// Thin walls / doors on cell edges, canonical key `(x, y, 0)` = east edge of (x, y),
    /// `(x, y, 3)` = south edge (see [`edge_key`]).
    pub edges: HashMap<(i64, i64, u8), i32>,
    /// Fog colour and distances (cells) where it starts and becomes total.
    pub fog: (u32, f32, f32),
    /// Horizontal field of view in degrees.
    pub fov: f32,
    pub cam: Cam,
    pub billboards: Vec<Billboard>,
    /// Faces further than this (cells) are not drawn. 0 = the fog end (or 24 without fog).
    pub max_dist: f32,
    /// Brightness of north/south wall faces (east/west faces are 1): cheap depth cue.
    pub side_shade: f32,
    /// Where the last `fpview_draw` put the image (x, y, w, h), for `fpview_pick`.
    pub last_rect: [f32; 4],
    geo: Option<Rc<Vec<Face>>>,
}

pub const NEAR: f32 = 0.02;

impl FpView {
    pub fn new(w: u32, h: u32) -> FpView {
        FpView {
            w: w.max(1),
            h: h.max(1),
            mw: 0,
            mh: 0,
            cells: vec![],
            walls: HashMap::new(),
            floor: Fill::Color(0x3a3a3a),
            ceiling: Fill::Color(0x1a1a1a),
            cell_floor: HashMap::new(),
            cell_ceiling: HashMap::new(),
            edges: HashMap::new(),
            fog: (0x000000, 2.0, 9.0),
            fov: 75.0,
            cam: Cam { x: 0.0, y: 0.0, angle: 0.0, height: 0.5, pitch: 0.0 },
            billboards: vec![],
            max_dist: 0.0,
            side_shade: 0.8,
            last_rect: [0.0, 0.0, w as f32, h as f32],
            geo: None,
        }
    }

    /// World geometry changed: rebuild on the next draw.
    pub fn dirty(&mut self) {
        self.geo = None;
    }

    pub fn set_map(&mut self, w: usize, h: usize, cells: Vec<i32>) {
        self.mw = w;
        self.mh = h;
        self.cells = cells;
        self.dirty();
    }

    pub fn inside(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.mw && (y as usize) < self.mh
    }

    /// Cell value (0 outside the map).
    pub fn cell(&self, x: i64, y: i64) -> i32 {
        if self.inside(x, y) {
            self.cells[y as usize * self.mw + x as usize]
        } else {
            0
        }
    }

    pub fn set_cell(&mut self, x: i64, y: i64, v: i32) -> bool {
        if !self.inside(x, y) {
            return false;
        }
        let i = y as usize * self.mw + x as usize;
        if self.cells[i] != v {
            self.cells[i] = v;
            self.dirty();
        }
        true
    }

    fn wall(&self, t: i32) -> WallType {
        self.walls.get(&t).cloned().unwrap_or_default()
    }

    /// Is (x, y) a wall that blocks movement? Outside the map counts as solid.
    pub fn solid(&self, x: i64, y: i64) -> bool {
        if !self.inside(x, y) {
            return true;
        }
        let v = self.cell(x, y);
        v > 0 && self.wall(v).solid
    }

    pub fn edge(&self, x: i64, y: i64, dir: i64) -> i32 {
        self.edges.get(&edge_key(x, y, dir)).copied().unwrap_or(0)
    }

    pub fn set_edge(&mut self, x: i64, y: i64, dir: i64, t: i32) {
        let k = edge_key(x, y, dir);
        if t <= 0 {
            self.edges.remove(&k);
        } else {
            self.edges.insert(k, t);
        }
        self.dirty();
    }

    /// Can one step from (x, y) in `dir` (walls of the target cell and edges in between)?
    pub fn can_move(&self, x: i64, y: i64, dir: i64) -> bool {
        let (nx, ny) = (x + dir_dx(dir), y + dir_dy(dir));
        if self.solid(nx, ny) {
            return false;
        }
        let e = self.edge(x, y, dir);
        !(e > 0 && self.wall(e).solid)
    }

    pub fn draw_distance(&self) -> f32 {
        if self.max_dist > 0.0 {
            self.max_dist
        } else if self.fog.2 > 0.0 {
            self.fog.2 + 0.5
        } else {
            24.0
        }
    }

    /// Every wall, edge, floor and ceiling quad of the map (cached until the map changes).
    pub fn geometry(&mut self, sprites: &[Sprite]) -> Rc<Vec<Face>> {
        if let Some(g) = &self.geo {
            return g.clone();
        }
        let g = Rc::new(build_geometry(self, sprites));
        self.geo = Some(g.clone());
        g
    }

    /// Camera basis: eye position and the right / up / forward unit vectors (world space).
    pub fn basis(&self) -> ([f32; 3], [f32; 3], [f32; 3], [f32; 3]) {
        let c = &self.cam;
        let (a, p) = (c.angle.to_radians(), c.pitch.clamp(-89.0, 89.0).to_radians());
        let eye = [c.x + 0.5, c.height, c.y + 0.5];
        // angle 0 = +X (east), 90 = -Z (north)
        let fwd = [a.cos() * p.cos(), p.sin(), -a.sin() * p.cos()];
        let right = norm([-fwd[2], 0.0, fwd[0]]);
        let up = cross(right, fwd);
        (eye, right, up, fwd)
    }

    /// tan of half the horizontal and vertical fields of view.
    pub fn tan_half(&self) -> (f32, f32) {
        let th = (self.fov.clamp(10.0, 170.0).to_radians() * 0.5).tan();
        (th, th * self.h as f32 / self.w as f32)
    }

    /// View-projection matrix, column-major (OpenGL). `flip_y` for render targets, whose
    /// first row is the top of the image (see canvas.rs).
    pub fn matrix(&self, flip_y: bool) -> [f32; 16] {
        let (eye, r, u, f) = self.basis();
        let (tx, ty) = self.tan_half();
        let far = self.draw_distance() + 2.0;
        let near = NEAR;
        let sy = if flip_y { -1.0 / ty } else { 1.0 / ty };
        let sx = 1.0 / tx;
        let a = (far + near) / (near - far);
        let b = 2.0 * far * near / (near - far);
        // view rows: right, up, -forward
        let view = [
            [r[0], r[1], r[2], -dot(r, eye)],
            [u[0], u[1], u[2], -dot(u, eye)],
            [-f[0], -f[1], -f[2], dot(f, eye)],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let proj = [[sx, 0.0, 0.0, 0.0], [0.0, sy, 0.0, 0.0], [0.0, 0.0, a, b], [0.0, 0.0, -1.0, 0.0]];
        let mut out = [0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                out[col * 4 + row] = (0..4).map(|k| proj[row][k] * view[k][col]).sum();
            }
        }
        out
    }

    /// World point → pixel of the view image (x right, y down) and its distance along the
    /// view direction; None behind the camera.
    pub fn project_world(&self, p: [f32; 3]) -> Option<(f32, f32, f32)> {
        let (eye, r, u, f) = self.basis();
        let d = sub(p, eye);
        let z = dot(d, f);
        if z <= NEAR {
            return None;
        }
        let (tx, ty) = self.tan_half();
        let nx = dot(d, r) / (z * tx);
        let ny = dot(d, u) / (z * ty);
        Some(((nx + 1.0) * 0.5 * self.w as f32, (1.0 - ny) * 0.5 * self.h as f32, z))
    }

    /// Ray through pixel (px, py) of the view image: origin and unit direction.
    pub fn ray(&self, px: f32, py: f32) -> ([f32; 3], [f32; 3]) {
        let (eye, r, u, f) = self.basis();
        let (tx, ty) = self.tan_half();
        let nx = px / self.w as f32 * 2.0 - 1.0;
        let ny = 1.0 - py / self.h as f32 * 2.0;
        let d = add(add(scale(r, nx * tx), scale(u, ny * ty)), f);
        (eye, norm(d))
    }

    /// The quads of the billboards, sorted far to near, camera-facing.
    pub fn billboard_faces(&self, sprites: &[Sprite]) -> Vec<(usize, Face, f32)> {
        let (eye, r, _, f) = self.basis();
        let right = norm([r[0], 0.0, r[2]]);
        let fflat = norm([f[0], 0.0, f[2]]);
        let maxd = self.draw_distance();
        let mut out = Vec::with_capacity(self.billboards.len());
        for (i, b) in self.billboards.iter().enumerate() {
            let Some(s) = sprites.get(b.spr as usize) else { continue };
            let base = [b.x + 0.5, b.yoff, b.y + 0.5];
            let depth = dot(sub(base, eye), fflat);
            if depth <= NEAR || depth > maxd {
                continue;
            }
            let h = b.scale.max(0.0);
            let w = h * s.fw as f32 / s.fh.max(1) as f32;
            let half = scale(right, w * 0.5);
            let (l, rr) = (sub(base, half), add(base, half));
            let top = |p: [f32; 3]| [p[0], p[1] + h, p[2]];
            let (uv, tex) = frame_uv(s, b.frame);
            let c = Rgba::from_rgb(b.color, b.alpha as f64);
            out.push((i, Face { p: [top(l), top(rr), rr, l], uv, tex: tex.then_some(b.spr), color: rgba8(c) }, depth));
        }
        out.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    /// What the frame of this camera needs on the GPU: the visible faces (grouped by
    /// texture), the billboards (far to near), matrix and fog.
    pub fn frame(&mut self, sprites: &[Sprite]) -> FpFrame {
        let geo = self.geometry(sprites);
        let (eye, _, _, f) = self.basis();
        let maxd = self.draw_distance();
        let (tx, ty) = self.tan_half();
        // conservative horizontal cone (pitch and the vertical fov widen it)
        let half = (tx.atan() + ty.atan() * 0.5 + self.cam.pitch.abs().to_radians()).min(std::f32::consts::PI);
        let fl = (f[0] * f[0] + f[2] * f[2]).sqrt().max(1e-4);
        let (fx, fz) = (f[0] / fl, f[2] / fl);
        let mut faces: Vec<Face> = geo
            .iter()
            .filter(|q| {
                let (cx, cz) = q.center();
                let (dx, dz) = (cx - eye[0], cz - eye[2]);
                let d = (dx * dx + dz * dz).sqrt();
                if d > maxd + 0.75 {
                    return false;
                }
                if d < 1.5 || half >= std::f32::consts::FRAC_PI_2 * 1.9 {
                    return true;
                }
                let cosang = (dx * fx + dz * fz) / d;
                cosang >= (half + (0.75 / d).min(1.0).asin()).min(std::f32::consts::PI).cos()
            })
            .cloned()
            .collect();
        faces.sort_by_key(|q| q.tex.map(|t| t as i64).unwrap_or(-1));
        let sprites_q = self.billboard_faces(sprites).into_iter().map(|(_, q, _)| q).collect();
        let fc = Rgba::from_rgb(self.fog.0, 1.0);
        FpFrame {
            w: self.w,
            h: self.h,
            mvp: self.matrix(true),
            eye,
            fog: [fc.r, fc.g, fc.b, 1.0],
            fog_range: [self.fog.1, self.fog.2.max(self.fog.1 + 0.001)],
            faces,
            sprites: sprites_q,
        }
    }

    /// The first thing under pixel (px, py) of the view image.
    pub fn pick(&mut self, sprites: &[Sprite], px: f32, py: f32) -> Option<Pick> {
        let (o, d) = self.ray(px, py);
        let maxd = self.draw_distance();
        // floor / ceiling planes limit the ray
        let mut t_plane = f32::INFINITY;
        let mut plane = None;
        if d[1] < -1e-6 {
            t_plane = (0.0 - o[1]) / d[1];
            plane = Some(false);
        } else if d[1] > 1e-6 {
            t_plane = (1.0 - o[1]) / d[1];
            plane = Some(true);
        }
        let flat = (d[0] * d[0] + d[2] * d[2]).sqrt();
        let limit = t_plane.min(if flat > 1e-6 { (maxd + 1.0) / flat } else { f32::INFINITY });
        let mut best: Option<Pick> = self.dda_hit(o, d, limit);
        if best.is_none() && t_plane <= limit && t_plane.is_finite() {
            let p = add(o, scale(d, t_plane));
            let (cx, cy) = (p[0].floor() as i64, p[2].floor() as i64);
            let fill = match plane {
                Some(true) => *self.cell_ceiling.get(&(cx, cy)).unwrap_or(&self.ceiling),
                _ => *self.cell_floor.get(&(cx, cy)).unwrap_or(&self.floor),
            };
            if self.inside(cx, cy) && fill != Fill::None {
                let kind = if plane == Some(true) { PickKind::Ceiling } else { PickKind::Floor };
                best = Some(Pick { kind, cell_x: cx, cell_y: cy, dir: 0, value: self.cell(cx, cy), index: 0, dist: t_plane, x: p[0], y: p[2] });
            }
        }
        // billboards in front of that (only opaque texels count)
        let (_, r, _, f) = self.basis();
        let right = norm([r[0], 0.0, r[2]]);
        let n = norm([f[0], 0.0, f[2]]);
        let mut bb: Option<(f32, usize)> = None;
        for (i, b) in self.billboards.iter().enumerate() {
            let Some(s) = sprites.get(b.spr as usize) else { continue };
            let base = [b.x + 0.5, b.yoff, b.y + 0.5];
            let den = dot(d, n);
            if den.abs() < 1e-6 {
                continue;
            }
            let t = dot(sub(base, o), n) / den;
            if t <= 0.0 || best.as_ref().is_some_and(|p| t >= p.dist) || bb.is_some_and(|(bt, _)| t >= bt) {
                continue;
            }
            let q = add(o, scale(d, t));
            let h = b.scale.max(1e-6);
            let w = h * s.fw as f32 / s.fh.max(1) as f32;
            let u = dot(sub(q, base), right) / w + 0.5;
            let v = 1.0 - (q[1] - b.yoff) / h;
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            if texel_alpha(s, b.frame, u, v) > 16 {
                bb = Some((t, i));
            }
        }
        if let Some((t, i)) = bb {
            let b = &self.billboards[i];
            best = Some(Pick { kind: PickKind::Billboard, cell_x: (b.x + 0.5).floor() as i64, cell_y: (b.y + 0.5).floor() as i64, dir: 0, value: 0, index: i, dist: t, x: b.x + 0.5, y: b.y + 0.5 });
        }
        best
    }

    /// Walks the grid along the ray until a wall face or an edge, up to parameter `limit`.
    fn dda_hit(&self, o: [f32; 3], d: [f32; 3], limit: f32) -> Option<Pick> {
        let (mut cx, mut cy) = (o[0].floor() as i64, o[2].floor() as i64);
        let step_x: i64 = if d[0] > 0.0 { 1 } else { -1 };
        let step_y: i64 = if d[2] > 0.0 { 1 } else { -1 };
        let inv = |v: f32| if v.abs() < 1e-9 { f32::INFINITY } else { 1.0 / v.abs() };
        let (dtx, dty) = (inv(d[0]), inv(d[2]));
        let mut tx = if d[0] > 0.0 { (cx as f32 + 1.0 - o[0]) * dtx } else { (o[0] - cx as f32) * dtx };
        let mut ty = if d[2] > 0.0 { (cy as f32 + 1.0 - o[2]) * dty } else { (o[2] - cy as f32) * dty };
        for _ in 0..4096 {
            let (t, dir) = if tx < ty { (tx, if step_x > 0 { 0 } else { 2 }) } else { (ty, if step_y > 0 { 3 } else { 1 }) };
            if t > limit || !t.is_finite() {
                return None;
            }
            let p = add(o, scale(d, t));
            let e = self.edge(cx, cy, dir);
            if e > 0 {
                return Some(Pick { kind: PickKind::Edge, cell_x: cx, cell_y: cy, dir: dir as u8, value: e, index: 0, dist: t, x: p[0], y: p[2] });
            }
            if tx < ty {
                cx += step_x;
                tx += dtx;
            } else {
                cy += step_y;
                ty += dty;
            }
            if !self.inside(cx, cy) && (cx < -1 || cy < -1 || cx > self.mw as i64 || cy > self.mh as i64) {
                return None;
            }
            let v = self.cell(cx, cy);
            if v > 0 {
                // the face we hit looks back at us
                let face = (dir + 2) % 4;
                return Some(Pick { kind: PickKind::Wall, cell_x: cx, cell_y: cy, dir: face as u8, value: v, index: 0, dist: t, x: p[0], y: p[2] });
            }
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PickKind {
    Wall,
    Edge,
    Floor,
    Ceiling,
    Billboard,
}

impl PickKind {
    pub fn name(self) -> &'static str {
        match self {
            PickKind::Wall => "wall",
            PickKind::Edge => "edge",
            PickKind::Floor => "floor",
            PickKind::Ceiling => "ceiling",
            PickKind::Billboard => "billboard",
        }
    }
}

/// Result of [`FpView::pick`].
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub kind: PickKind,
    pub cell_x: i64,
    pub cell_y: i64,
    /// Wall: the side of the wall cell that was hit; edge: the side of `cell` it is on.
    pub dir: u8,
    /// Wall / edge type (cell value for floors).
    pub value: i32,
    /// Billboard index.
    pub index: usize,
    /// Distance along the ray.
    pub dist: f32,
    /// World point hit on the ground plane (cell units, not centred).
    pub x: f32,
    pub y: f32,
}

/// Canonical key of the edge on side `dir` of cell (x, y).
pub fn edge_key(x: i64, y: i64, dir: i64) -> (i64, i64, u8) {
    match norm_dir(dir) {
        0 => (x, y, 0),
        2 => (x - 1, y, 0),
        3 => (x, y, 3),
        _ => (x, y - 1, 3),
    }
}

/// Snapshot of one view for the GPU (a `DrawCmd::FpView`).
#[derive(Clone, Debug)]
pub struct FpFrame {
    pub w: u32,
    pub h: u32,
    /// Column-major view-projection (y flipped for render targets).
    pub mvp: [f32; 16],
    pub eye: [f32; 3],
    pub fog: [f32; 4],
    pub fog_range: [f32; 2],
    /// Opaque quads (walls, edges, floors, ceilings), grouped by texture.
    pub faces: Vec<Face>,
    /// Billboards, far to near.
    pub sprites: Vec<Face>,
}

fn rgba8(c: Rgba) -> [u8; 4] {
    [(c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8, (c.a * 255.0).round() as u8]
}

fn shade_rgb(c: u32, k: f32) -> [u8; 4] {
    let ch = |s: u32| ((((c >> s) & 255) as f32) * k).round().clamp(0.0, 255.0) as u8;
    [ch(16), ch(8), ch(0), 255]
}

/// Normalised sheet rectangle of a frame (u0, v0, u1, v1) and whether the sprite exists.
pub fn frame_uv(s: &Sprite, frame: u32) -> ([f32; 4], bool) {
    let n = s.frames.len().max(1);
    let Some(fr) = s.frames.get(frame as usize % n) else { return ([0.0, 0.0, 1.0, 1.0], false) };
    let (sw, sh) = (s.sheet_w.max(1) as f32, s.sheet_h.max(1) as f32);
    ([fr[0] as f32 / sw, fr[1] as f32 / sh, (fr[0] + fr[2]) as f32 / sw, (fr[1] + fr[3]) as f32 / sh], true)
}

/// Alpha of the texel at (u, v) ∈ [0,1)² of a frame.
pub fn texel_alpha(s: &Sprite, frame: u32, u: f32, v: f32) -> u8 {
    let n = s.frames.len().max(1);
    let Some(fr) = s.frames.get(frame as usize % n) else { return 255 };
    let x = fr[0] + ((u * fr[2] as f32) as u32).min(fr[2].saturating_sub(1));
    let y = fr[1] + ((v * fr[3] as f32) as u32).min(fr[3].saturating_sub(1));
    s.rgba.get(((y * s.sheet_w + x) * 4 + 3) as usize).copied().unwrap_or(255)
}

fn fill_face(sprites: &[Sprite], fill: Fill, p: [[f32; 3]; 4]) -> Option<Face> {
    match fill {
        Fill::None => None,
        Fill::Color(c) => Some(Face { p, uv: [0.0, 0.0, 1.0, 1.0], tex: None, color: shade_rgb(c, 1.0) }),
        Fill::Tex { spr, frame } => {
            let s = sprites.get(spr as usize)?;
            let (uv, _) = frame_uv(s, frame);
            Some(Face { p, uv, tex: Some(spr), color: [255; 4] })
        }
    }
}

fn wall_face(v: &FpView, sprites: &[Sprite], t: i32, p: [[f32; 3]; 4], shade: f32) -> Face {
    let w = v.wall(t);
    match w.tex.and_then(|(spr, f)| sprites.get(spr as usize).map(|s| (spr, frame_uv(s, f).0))) {
        Some((spr, uv)) => Face { p, uv, tex: Some(spr), color: shade_rgb(w.color, shade) },
        None => Face { p, uv: [0.0, 0.0, 1.0, 1.0], tex: None, color: shade_rgb(w.color, shade) },
    }
}

/// Quad of the face of the cell side `dir` of the empty cell (x, y), as seen from inside.
fn side_quad(x: i64, y: i64, dir: usize) -> [[f32; 3]; 4] {
    let (cx, cz) = (x as f32 + 0.5, y as f32 + 0.5);
    let (dx, dz) = (DIRS[dir].0 as f32, DIRS[dir].1 as f32);
    // right of someone looking that way
    let (rx, rz) = (-dz, dx);
    let (mx, mz) = (cx + dx * 0.5, cz + dz * 0.5);
    let l = [mx - rx * 0.5, mz - rz * 0.5];
    let r = [mx + rx * 0.5, mz + rz * 0.5];
    [[l[0], 1.0, l[1]], [r[0], 1.0, r[1]], [r[0], 0.0, r[1]], [l[0], 0.0, l[1]]]
}

fn build_geometry(v: &FpView, sprites: &[Sprite]) -> Vec<Face> {
    let mut out = Vec::new();
    for y in 0..v.mh as i64 {
        for x in 0..v.mw as i64 {
            if v.cell(x, y) > 0 {
                continue;
            }
            for dir in 0..4 {
                let (nx, ny) = (x + DIRS[dir].0, y + DIRS[dir].1);
                let n = v.cell(nx, ny);
                if n > 0 && v.inside(nx, ny) {
                    let shade = if dir % 2 == 0 { 1.0 } else { v.side_shade };
                    out.push(wall_face(v, sprites, n, side_quad(x, y, dir), shade));
                }
            }
            let (x0, z0, x1, z1) = (x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0);
            let floor = *v.cell_floor.get(&(x, y)).unwrap_or(&v.floor);
            // floor seen from above: top edge = north
            if let Some(f) = fill_face(sprites, floor, [[x0, 0.0, z0], [x1, 0.0, z0], [x1, 0.0, z1], [x0, 0.0, z1]]) {
                out.push(f);
            }
            let ceil = *v.cell_ceiling.get(&(x, y)).unwrap_or(&v.ceiling);
            if let Some(f) = fill_face(sprites, ceil, [[x0, 1.0, z1], [x1, 1.0, z1], [x1, 1.0, z0], [x0, 1.0, z0]]) {
                out.push(f);
            }
        }
    }
    let mut edges: Vec<_> = v.edges.iter().collect();
    edges.sort();
    for (&(x, y, d), &t) in edges {
        let shade = if d == 0 { 1.0 } else { v.side_shade };
        out.push(wall_face(v, sprites, t, side_quad(x, y, d as usize), shade));
    }
    out
}

// ------------------------------------------------------------------ small vector maths

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    if l < 1e-9 {
        a
    } else {
        scale(a, 1.0 / l)
    }
}

// ------------------------------------------------------------------ primitives

/// GameMaker's primitive kinds (`pr_*`, same values).
pub const PR_NAMES: [(&str, u8); 6] =
    [("pr_pointlist", 1), ("pr_linelist", 2), ("pr_linestrip", 3), ("pr_trianglelist", 4), ("pr_trianglestrip", 5), ("pr_trianglefan", 6)];

/// One 2D vertex of a `DrawCmd::Mesh`: position, normalised sheet UV and colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MVert {
    pub x: f32,
    pub y: f32,
    pub u: f32,
    pub v: f32,
    pub color: [u8; 4],
}

/// A primitive being built between `draw_primitive_begin` and `draw_primitive_end`.
#[derive(Clone, Debug)]
pub struct PrimBuild {
    pub kind: u8,
    /// Sprite, frame and the frame's sheet rectangle (u0, v0, u1, v1).
    pub tex: Option<(u32, [f32; 4])>,
    pub verts: Vec<MVert>,
}

/// Turns a primitive of `kind` into a triangle list (lines become 1-pixel quads, points
/// 1×1 squares, like `draw_line` / `draw_point`).
pub fn triangulate(kind: u8, v: &[MVert]) -> Vec<MVert> {
    let mut out = Vec::new();
    match kind {
        4 => out.extend(v.chunks_exact(3).flatten()),
        5 => {
            for i in 2..v.len() {
                // keep the winding consistent (not needed without culling, but tidy)
                if i % 2 == 0 {
                    out.extend([v[i - 2], v[i - 1], v[i]]);
                } else {
                    out.extend([v[i - 1], v[i - 2], v[i]]);
                }
            }
        }
        6 => {
            for i in 2..v.len() {
                out.extend([v[0], v[i - 1], v[i]]);
            }
        }
        2 => v.chunks_exact(2).for_each(|s| line_quad(&mut out, s[0], s[1])),
        3 => v.windows(2).for_each(|s| line_quad(&mut out, s[0], s[1])),
        1 => {
            for p in v {
                let (x, y) = (p.x.floor(), p.y.floor());
                let q = |x, y| MVert { x, y, ..*p };
                out.extend([q(x, y), q(x + 1.0, y), q(x + 1.0, y + 1.0), q(x, y), q(x + 1.0, y + 1.0), q(x, y + 1.0)]);
            }
        }
        _ => {}
    }
    out
}

fn line_quad(out: &mut Vec<MVert>, a: MVert, b: MVert) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l = (dx * dx + dy * dy).sqrt();
    if l < 1e-6 {
        return;
    }
    let (nx, ny) = (-dy / l * 0.5, dx / l * 0.5);
    let o = |p: MVert, s: f32| MVert { x: p.x + nx * s, y: p.y + ny * s, ..p };
    out.extend([o(a, 1.0), o(b, 1.0), o(b, -1.0), o(a, 1.0), o(b, -1.0), o(a, -1.0)]);
}

/// Projective map of the unit square onto the quad p0 (0,0), p1 (1,0), p2 (1,1), p3 (0,1).
pub fn square_to_quad(p: [[f32; 2]; 4]) -> impl Fn(f32, f32) -> (f32, f32) {
    let ([x0, y0], [x1, y1], [x2, y2], [x3, y3]) = (p[0], p[1], p[2], p[3]);
    let (sx, sy) = (x0 - x1 + x2 - x3, y0 - y1 + y2 - y3);
    let (a, b, c, d, e, f, g, h);
    if sx.abs() < 1e-6 && sy.abs() < 1e-6 {
        (a, b, c, d, e, f, g, h) = (x1 - x0, x3 - x0, x0, y1 - y0, y3 - y0, y0, 0.0, 0.0);
    } else {
        let (dx1, dx2, dy1, dy2) = (x1 - x2, x3 - x2, y1 - y2, y3 - y2);
        let den = dx1 * dy2 - dx2 * dy1;
        let den = if den.abs() < 1e-9 { 1e-9 } else { den };
        g = (sx * dy2 - dx2 * sy) / den;
        h = (dx1 * sy - sx * dy1) / den;
        (a, b, c) = (x1 - x0 + g * x1, x3 - x0 + h * x3, x0);
        (d, e, f) = (y1 - y0 + g * y1, y3 - y0 + h * y3, y0);
    }
    move |u, v| {
        let w = g * u + h * v + 1.0;
        let w = if w.abs() < 1e-9 { 1e-9 } else { w };
        ((a * u + b * v + c) / w, (d * u + e * v + f) / w)
    }
}

/// Triangles of `draw_sprite_pos`: two (affine, like GameMaker) or an 8×8 grid following the
/// projective map (perspective-correct texture).
pub fn sprite_pos_tris(p: [[f32; 2]; 4], uv: [f32; 4], color: [u8; 4], perspective: bool) -> Vec<MVert> {
    let vert = |x: f32, y: f32, s: f32, t: f32| MVert { x, y, u: uv[0] + (uv[2] - uv[0]) * s, v: uv[1] + (uv[3] - uv[1]) * t, color };
    if !perspective {
        let c = [vert(p[0][0], p[0][1], 0.0, 0.0), vert(p[1][0], p[1][1], 1.0, 0.0), vert(p[2][0], p[2][1], 1.0, 1.0), vert(p[3][0], p[3][1], 0.0, 1.0)];
        return vec![c[0], c[1], c[2], c[0], c[2], c[3]];
    }
    const N: usize = 8;
    let map = square_to_quad(p);
    let mut grid = Vec::with_capacity((N + 1) * (N + 1));
    for j in 0..=N {
        for i in 0..=N {
            let (s, t) = (i as f32 / N as f32, j as f32 / N as f32);
            let (x, y) = map(s, t);
            grid.push(vert(x, y, s, t));
        }
    }
    let mut out = Vec::with_capacity(N * N * 6);
    for j in 0..N {
        for i in 0..N {
            let k = j * (N + 1) + i;
            let (a, b, c, d) = (grid[k], grid[k + 1], grid[k + N + 2], grid[k + N + 1]);
            out.extend([a, b, c, a, c, d]);
        }
    }
    out
}

// ------------------------------------------------------------------ post-process

/// `postfx_pixelate` settings.
#[derive(Clone, Debug, PartialEq)]
pub struct PostFx {
    pub pixels_wide: u32,
    /// Colour levels per channel (0 = no posterize).
    pub levels: u32,
    /// Ordered dither strength 0..1.
    pub dither: f32,
    /// Colours to snap to (empty = none). At most [`MAX_PALETTE`].
    pub palette: Rc<Vec<u32>>,
    /// Applied after the GUI too (default: only the world, before `draw_gui`).
    pub gui: bool,
}

pub const MAX_PALETTE: usize = 64;

/// Coarsest image a full `postfx_transition(1)` goes to.
pub const TRANSITION_MIN_WIDTH: f32 = 6.0;

/// Effective width in pixels for transition `t` (log interpolation from `base`).
pub fn transition_width(base: f32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let base = base.max(TRANSITION_MIN_WIDTH);
    (base * (TRANSITION_MIN_WIDTH / base).powf(t)).round().max(1.0) as u32
}

// ------------------------------------------------------------------ engine state

#[derive(Default)]
pub struct FpState {
    pub views: HashMap<u32, FpView>,
    pub next: u32,
    pub prim: Option<PrimBuild>,
    pub postfx: Option<PostFx>,
    /// `postfx_transition` amount (0 = off).
    pub transition: f32,
}

impl std::fmt::Debug for FpState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FpState({} views)", self.views.len())
    }
}

/// Pushes the post-process command at its place of the frame: before `draw_gui`
/// (`gui = false`) or after it (`gui = true`). Called by `Engine::render`.
pub fn postfx_cmd(e: &mut crate::Engine, gui: bool) {
    let t = e.fp.transition;
    let cmd = match &e.fp.postfx {
        Some(p) if p.gui == gui => {
            let w = if t > 0.0 { transition_width(p.pixels_wide as f32, t) } else { p.pixels_wide };
            Some(crate::draw::DrawCmd::PostFx { pixels_wide: w, levels: p.levels, dither: p.dither, palette: p.palette.clone() })
        }
        // a transition alone pixelates the whole frame
        None if gui && t > 0.0 => {
            Some(crate::draw::DrawCmd::PostFx { pixels_wide: transition_width(e.cam.w as f32, t), levels: 0, dither: 0.0, palette: Rc::new(vec![]) })
        }
        _ => None,
    };
    if let Some(c) = cmd {
        e.cmds.push(c);
    }
}

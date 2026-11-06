//! Line of sight and raycasts (`raycast`, `raycast_all`, `line_of_sight`), GameMaker's
//! collision queries (`place_meeting`, `instance_place`, `collision_line`,
//! `collision_line_list`...) and collision layers (`collision_layer` / `collision_mask`).
//!
//! What a ray can hit (the `target` argument): an object, an instance, `all`, the name of a
//! tile layer (any non-empty tile blocks), an `mp_grid` id (its blocked cells), or an array
//! mixing them.

use super::{err, Reg, A};
use crate::engine::{Engine, LayerKind};
use crate::navgrid::{dda, Cells, GridHit};
use modscript::{intern, AssetKind, CallCx, RtResult, Scope, Value};

impl Engine {
    /// [`Engine::hits_rect`] counting only instances on a layer of `mask` (`collision_layer & mask != 0`).
    pub fn hits_rect_mask(&self, rect: [f64; 4], target: &Value, except: Option<u32>, first: bool, mask: u32) -> Vec<u32> {
        let mut out = Vec::new();
        for (j, o) in self.insts.iter().enumerate() {
            if !o.alive || Some(o.id) == except || o.col_layer & mask == 0 || !self.matches(o, target) {
                continue;
            }
            if let Some(ob) = self.bbox(j) {
                if ob[0] < rect[2] && ob[2] > rect[0] && ob[1] < rect[3] && ob[3] > rect[1] {
                    out.push(o.id);
                    if first {
                        break;
                    }
                }
            }
        }
        out
    }
}

/// One thing a ray hit.
#[derive(Clone, Debug)]
pub struct RayHit {
    /// The instance hit (None for tiles and grid cells).
    pub inst: Option<u32>,
    pub x: f64,
    pub y: f64,
    pub dist: f64,
    pub nx: f64,
    pub ny: f64,
    /// For tiles / grid cells: the layer name or grid id, and the cell.
    pub wall: Option<(Value, usize, usize)>,
}

impl RayHit {
    fn to_value(&self, len: f64) -> Value {
        let n = Value::Num;
        let mut f = vec![
            (intern("inst"), self.inst.map(Value::Inst).unwrap_or(Value::Null)),
            (intern("x"), n(self.x)),
            (intern("y"), n(self.y)),
            (intern("distance"), n(self.dist)),
            (intern("fraction"), n(if len > 0.0 { self.dist / len } else { 0.0 })),
            (intern("nx"), n(self.nx)),
            (intern("ny"), n(self.ny)),
        ];
        if let Some((src, cx, cy)) = &self.wall {
            f.push((intern("source"), src.clone()));
            f.push((intern("cell_x"), n(*cx as f64)));
            f.push((intern("cell_y"), n(*cy as f64)));
        }
        Value::new_struct(f)
    }
}

/// Where segment p1→p2 enters box `b` (slab method): distance along the segment and the normal
/// of the face it crosses; (0, (0, 0)) when it starts inside.
pub fn segment_box(x1: f64, y1: f64, x2: f64, y2: f64, b: [f64; 4]) -> Option<(f64, f64, f64)> {
    let (dx, dy) = (x2 - x1, y2 - y1);
    let len = dx.hypot(dy);
    if x1 >= b[0] && x1 < b[2] && y1 >= b[1] && y1 < b[3] {
        return Some((0.0, 0.0, 0.0));
    }
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    let (mut nx, mut ny) = (0.0, 0.0);
    for (d, p, lo, hi, axis) in [(dx, x1, b[0], b[2], 0), (dy, y1, b[1], b[3], 1)] {
        if d.abs() < 1e-12 {
            if p < lo || p >= hi {
                return None;
            }
            continue;
        }
        let (mut a, mut c) = ((lo - p) / d, (hi - p) / d);
        let mut normal = -1.0;
        if a > c {
            std::mem::swap(&mut a, &mut c);
            normal = 1.0;
        }
        if a > t0 {
            t0 = a;
            (nx, ny) = if axis == 0 { (normal, 0.0) } else { (0.0, normal) };
        }
        t1 = t1.min(c);
        if t0 > t1 {
            return None;
        }
    }
    Some((t0 * len, nx, ny))
}

/// Cells of tile layer `li` (non-empty tiles block).
fn tile_cells(e: &Engine, li: usize) -> Option<(Cells, &Vec<Vec<i32>>)> {
    match &e.layers[li].kind {
        LayerKind::Tiles { cw, ch, grid, ox, oy, .. } => {
            let w = grid.iter().map(|r| r.len()).max().unwrap_or(0);
            Some((Cells { left: *ox, top: *oy, cw: *cw as f64, ch: *ch as f64, w, h: grid.len() }, grid))
        }
        _ => None,
    }
}

fn tile_layer(e: &Engine, name: &str) -> RtResult<usize> {
    match e.layers.iter().position(|l| l.name == name) {
        Some(li) if matches!(e.layers[li].kind, LayerKind::Tiles { .. }) => Ok(li),
        Some(_) => err(format!("layer '{name}' is not a tile layer")),
        None => err(format!("there is no tile layer '{name}' in this room")),
    }
}

fn tile_blocked(grid: &[Vec<i32>], cx: i64, cy: i64) -> bool {
    grid.get(cy as usize).and_then(|r| r.get(cx as usize)).is_some_and(|d| crate::tileset::index(*d) >= 0)
}

fn wall_hit(h: GridHit, src: Value) -> RayHit {
    RayHit { inst: None, x: h.x, y: h.y, dist: h.dist, nx: h.nx, ny: h.ny, wall: Some((src, h.cx, h.cy)) }
}

/// Everything the segment hits among `target`, sorted by distance. Instances: all of them
/// (excluding `except`, only layers in `mask`); tiles / grids: the first blocked cell of each.
/// With `stop_at_walls`, instances behind the nearest wall are dropped.
pub fn cast(e: &Engine, x1: f64, y1: f64, x2: f64, y2: f64, target: &Value, except: Option<u32>, mask: u32, stop_at_walls: bool) -> RtResult<Vec<RayHit>> {
    let mut hits = vec![];
    collect(e, x1, y1, x2, y2, target, except, mask, &mut hits)?;
    hits.sort_by(|a, b| a.dist.partial_cmp(&b.dist).unwrap_or(std::cmp::Ordering::Equal).then(a.inst.cmp(&b.inst)));
    if stop_at_walls {
        if let Some(k) = hits.iter().position(|h| h.wall.is_some()) {
            hits.truncate(k + 1);
        }
    }
    Ok(hits)
}

#[allow(clippy::too_many_arguments)]
fn collect(e: &Engine, x1: f64, y1: f64, x2: f64, y2: f64, target: &Value, except: Option<u32>, mask: u32, out: &mut Vec<RayHit>) -> RtResult<()> {
    match target {
        Value::Array(a) => {
            for t in a.borrow().iter() {
                collect(e, x1, y1, x2, y2, t, except, mask, out)?;
            }
        }
        Value::Str(name) => {
            let li = tile_layer(e, name)?;
            if let Some((cells, grid)) = tile_cells(e, li) {
                if let Some(h) = dda(&cells, &|cx, cy| tile_blocked(grid, cx, cy), x1, y1, x2, y2) {
                    out.push(wall_hit(h, target.clone()));
                }
            }
        }
        Value::Num(n) => {
            let g = super::navgrid::grid_ref(e, *n)?;
            if let Some(h) = g.raycast(x1, y1, x2, y2) {
                out.push(wall_hit(h, target.clone()));
            }
        }
        Value::Asset(AssetKind::Object, _) | Value::Inst(_) | Value::All => {
            let rect = [x1.min(x2) - 0.001, y1.min(y2) - 0.001, x1.max(x2) + 0.001, y1.max(y2) + 0.001];
            for id in e.hits_rect_mask(rect, target, except, false, mask) {
                let Some(b) = e.idx(id).and_then(|i| e.bbox(i)) else { continue };
                if let Some((d, nx, ny)) = segment_box(x1, y1, x2, y2, b) {
                    let len = (x2 - x1).hypot(y2 - y1);
                    let t = if len > 0.0 { d / len } else { 0.0 };
                    out.push(RayHit { inst: Some(id), x: x1 + (x2 - x1) * t, y: y1 + (y2 - y1) * t, dist: d, nx, ny, wall: None });
                }
            }
        }
        other => return err(format!("a ray can hit objects, instances, all, a tile layer name or an mp_grid; got {}", other.type_name())),
    }
    Ok(())
}

/// The calling instance (excluded from its own rays and queries) and its collision mask.
fn caller(e: &Engine, cx: &CallCx) -> (Option<u32>, u32) {
    match cx.self_ {
        Scope::Inst(id) => (Some(id), e.idx(id).map(|i| e.insts[i].col_mask).unwrap_or(u32::MAX)),
        _ => (None, u32::MAX),
    }
}

/// Mask argument `i` (default: the caller's `collision_mask`; -1 = every layer).
fn mask_arg(e: &Engine, cx: &CallCx, a: &A, i: usize) -> RtResult<u32> {
    match a.opt(i) {
        None => Ok(caller(e, cx).1),
        Some(_) => {
            let n = a.num(i)?;
            Ok(if n < 0.0 { u32::MAX } else { (n as u64 & 0xFFFF_FFFF) as u32 })
        }
    }
}

fn seg(a: &A) -> RtResult<(f64, f64, f64, f64)> {
    Ok((a.num(0)?, a.num(1)?, a.num(2)?, a.num(3)?))
}

/// (x, y, dir, length) → segment.
fn seg_dir(a: &A) -> RtResult<(f64, f64, f64, f64)> {
    let (x, y, d, l) = (a.num(0)?, a.num(1)?, a.num(2)?.to_radians(), a.num(3)?);
    Ok((x, y, x + d.cos() * l, y - d.sin() * l))
}

fn inst_list(ids: Vec<u32>) -> Value {
    Value::array(ids.into_iter().map(Value::Inst).collect())
}

/// Instances (matching `target`, layers in `mask`) a shape touches, using `test(bbox)`.
fn shape_hits(e: &Engine, rect: [f64; 4], target: &Value, except: Option<u32>, mask: u32, test: &dyn Fn([f64; 4]) -> bool) -> Vec<u32> {
    e.hits_rect_mask(rect, target, except, false, mask).into_iter().filter(|id| e.idx(*id).and_then(|i| e.bbox(i)).is_some_and(test)).collect()
}

/// GameMaker's `notme` argument (default true): exclude the caller.
fn except(e: &Engine, cx: &CallCx, a: &A, i: usize) -> Option<u32> {
    if a.bool_or(i, true) {
        caller(e, cx).0
    } else {
        None
    }
}

fn circle_box(x: f64, y: f64, r: f64) -> impl Fn([f64; 4]) -> bool {
    move |b| (x.clamp(b[0], b[2]) - x).hypot(y.clamp(b[1], b[3]) - y) <= r
}

/// Instances under the caller's box at (x, y), honouring its `collision_mask`.
fn place(e: &Engine, cx: &CallCx, a: &A, first: bool) -> RtResult<Vec<u32>> {
    let i = super::me(e, cx)?;
    Ok(e.hits_at(i, a.num(0)?, a.num(1)?, &a.target(2)?, first))
}

pub(super) fn register(r: &mut super::Reg) {
    reg(r)
}

fn reg(r: &mut Reg) {
    // ---------------------------------------------------------------- rays
    r.f("raycast", |e, cx, a| {
        // raycast(x1, y1, x2, y2, target, [mask]) -> { inst, x, y, distance, fraction, nx, ny } or null
        let p = A(a);
        let (x1, y1, x2, y2) = seg(&p)?;
        let hits = cast(e, x1, y1, x2, y2, p.val(4)?, caller(e, cx).0, mask_arg(e, cx, &p, 5)?, true)?;
        Ok(hits.first().map(|h| h.to_value((x2 - x1).hypot(y2 - y1))).unwrap_or(Value::Null))
    });
    r.f("raycast_dir", |e, cx, a| {
        // raycast_dir(x, y, dir, length, target, [mask])
        let p = A(a);
        let (x1, y1, x2, y2) = seg_dir(&p)?;
        let hits = cast(e, x1, y1, x2, y2, p.val(4)?, caller(e, cx).0, mask_arg(e, cx, &p, 5)?, true)?;
        Ok(hits.first().map(|h| h.to_value(p.num(3).unwrap_or(0.0))).unwrap_or(Value::Null))
    });
    r.f("raycast_all", |e, cx, a| {
        // raycast_all(x1, y1, x2, y2, target, [mask], [stop_at_walls=true]): every hit, nearest first
        let p = A(a);
        let (x1, y1, x2, y2) = seg(&p)?;
        let hits = cast(e, x1, y1, x2, y2, p.val(4)?, caller(e, cx).0, mask_arg(e, cx, &p, 5)?, p.bool_or(6, true))?;
        let len = (x2 - x1).hypot(y2 - y1);
        Ok(Value::array(hits.iter().map(|h| h.to_value(len)).collect()))
    });
    r.f("line_of_sight", |e, cx, a| {
        // line_of_sight(x1, y1, x2, y2, blockers, [mask]): true if nothing in `blockers` is in the way
        let p = A(a);
        let (x1, y1, x2, y2) = seg(&p)?;
        Ok(Value::Bool(cast(e, x1, y1, x2, y2, p.val(4)?, caller(e, cx).0, mask_arg(e, cx, &p, 5)?, true)?.is_empty()))
    });

    // ---------------------------------------------------------------- GameMaker queries
    r.f("place_meeting", |e, cx, a| Ok(Value::Bool(!place(e, cx, &A(a), true)?.is_empty())));
    r.f("instance_place", |e, cx, a| Ok(place(e, cx, &A(a), true)?.first().map(|id| Value::Inst(*id)).unwrap_or(Value::Null)));
    r.f("instance_place_list", |e, cx, a| Ok(inst_list(place(e, cx, &A(a), false)?)));
    r.f("place_free", |e, cx, a| {
        let i = super::me(e, cx)?;
        Ok(Value::Bool(!e.solid_at(i, A(a).num(0)?, A(a).num(1)?)))
    });
    r.f("place_empty", |e, cx, a| {
        // place_empty(x, y, [obj=all]): nothing at all (solid or not) under the caller's box
        let p = A(a);
        let i = super::me(e, cx)?;
        let t = p.opt(2).cloned().unwrap_or(Value::All);
        Ok(Value::Bool(e.hits_at(i, p.num(0)?, p.num(1)?, &t, true).is_empty()))
    });
    fn point(e: &Engine, cx: &CallCx, a: &A, first: bool, notme: usize) -> RtResult<Vec<u32>> {
        let (x, y) = (a.num(0)?, a.num(1)?);
        let t = a.target(2)?;
        let ex = except(e, cx, a, notme);
        let mut v = shape_hits(e, [x - 0.001, y - 0.001, x + 0.001, y + 0.001], &t, ex, caller(e, cx).1, &|b| x >= b[0] && x < b[2] && y >= b[1] && y < b[3]);
        if first {
            v.truncate(1);
        }
        Ok(v)
    }
    // position_*: no notme argument (GameMaker counts the caller too)
    r.f("position_meeting", |e, cx, a| {
        let args: Vec<Value> = a.iter().take(3).cloned().chain([Value::Bool(false)]).collect();
        Ok(Value::Bool(!point(e, cx, &A(&args), true, 3)?.is_empty()))
    });
    r.f("instance_position", |e, cx, a| {
        let args: Vec<Value> = a.iter().take(3).cloned().chain([Value::Bool(false)]).collect();
        Ok(point(e, cx, &A(&args), true, 3)?.first().map(|id| Value::Inst(*id)).unwrap_or(Value::Null))
    });
    r.f("instance_position_list", |e, cx, a| {
        let args: Vec<Value> = a.iter().take(3).cloned().chain([Value::Bool(false)]).collect();
        Ok(inst_list(point(e, cx, &A(&args), false, 3)?))
    });
    // collision_*(…, obj, [prec], [notme=true]): `prec` is accepted for compatibility (boxes only)
    r.f("collision_point", |e, cx, a| Ok(point(e, cx, &A(a), true, 4)?.first().map(|id| Value::Inst(*id)).unwrap_or(Value::Null)));
    r.f("collision_point_list", |e, cx, a| Ok(inst_list(point(e, cx, &A(a), false, 4)?)));
    fn rect(e: &Engine, cx: &CallCx, a: &A) -> RtResult<Vec<u32>> {
        let (x1, y1, x2, y2) = seg(a)?;
        let r = [x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)];
        Ok(shape_hits(e, r, &a.target(4)?, except(e, cx, a, 6), caller(e, cx).1, &|_| true))
    }
    r.f("collision_rectangle", |e, cx, a| Ok(rect(e, cx, &A(a))?.first().map(|id| Value::Inst(*id)).unwrap_or(Value::Null)));
    r.f("collision_rectangle_list", |e, cx, a| Ok(inst_list(rect(e, cx, &A(a))?)));
    fn circle(e: &Engine, cx: &CallCx, a: &A) -> RtResult<Vec<u32>> {
        let (x, y, rad) = (a.num(0)?, a.num(1)?, a.num(2)?);
        Ok(shape_hits(e, [x - rad, y - rad, x + rad, y + rad], &a.target(3)?, except(e, cx, a, 5), caller(e, cx).1, &circle_box(x, y, rad)))
    }
    r.f("collision_circle", |e, cx, a| Ok(circle(e, cx, &A(a))?.first().map(|id| Value::Inst(*id)).unwrap_or(Value::Null)));
    r.f("collision_circle_list", |e, cx, a| Ok(inst_list(circle(e, cx, &A(a))?)));
    fn line(e: &Engine, cx: &CallCx, a: &A) -> RtResult<Vec<u32>> {
        let (x1, y1, x2, y2) = seg(a)?;
        let t = a.target(4)?;
        Ok(cast(e, x1, y1, x2, y2, &t, except(e, cx, a, 6), caller(e, cx).1, false)?.into_iter().filter_map(|h| h.inst).collect())
    }
    // collision_line: the nearest instance on the segment (GameMaker returns any of them)
    r.f("collision_line", |e, cx, a| Ok(line(e, cx, &A(a))?.first().map(|id| Value::Inst(*id)).unwrap_or(Value::Null)));
    // collision_line_list(x1, y1, x2, y2, obj, [prec], [notme=true], [ordered=true]): array, nearest first
    r.f("collision_line_list", |e, cx, a| {
        let p = A(a);
        let mut ids = line(e, cx, &p)?;
        if !p.bool_or(7, true) {
            ids.sort();
        }
        Ok(inst_list(ids))
    });

    // ---------------------------------------------------------------- tiles as walls
    // (an instance's own box against tiles is `tile_meeting`, in tilemap.rs)
    r.f("tile_rect_hits", |e, _, a| {
        // tile_rect_hits(x1, y1, x2, y2, layer): any non-empty tile in the rectangle?
        let p = A(a);
        let (x1, y1, x2, y2) = seg(&p)?;
        let li = tile_layer(e, &p.text(4)?)?;
        Ok(Value::Bool(tiles_in_box(e, li, [x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)])))
    });
}

/// Any non-empty tile of layer `li` overlapping box `b` (right/bottom edges exclusive)?
fn tiles_in_box(e: &Engine, li: usize, b: [f64; 4]) -> bool {
    let Some((c, grid)) = tile_cells(e, li) else { return false };
    let eps = 1e-6;
    let (ax, ay) = (((b[0] - c.left) / c.cw).floor() as i64, ((b[1] - c.top) / c.ch).floor() as i64);
    let (bx, by) = (((b[2] - eps - c.left) / c.cw).floor() as i64, ((b[3] - eps - c.top) / c.ch).floor() as i64);
    (ay.max(0)..=by).any(|cy| (ax.max(0)..=bx).any(|cx| tile_blocked(grid, cx, cy)))
}

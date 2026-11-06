//! First-person grid views (`fpview_*`), grid direction helpers (`grid_dir_*`), textured
//! primitives (`draw_sprite_pos`, `draw_primitive_*`, `draw_vertex_*`), the minimap
//! (`draw_minimap`, `fpview_draw_minimap`) and the pixelate post-process (`postfx_*`).
//! State and maths live in `crate::fpview`; the GPU side in modengine-cli/src/fpview.rs.

use super::{err, Reg, A};
use crate::draw::{DrawCmd, Rgba};
use crate::engine::Engine;
use crate::fpview::{self as fp, Fill, FpView, MVert, PostFx, PrimBuild, WallType};
use modscript::{intern, AssetKind, RtResult, Sym, Value};
use std::collections::HashMap;
use std::rc::Rc;

pub(crate) fn constants(c: &mut HashMap<Sym, Value>) {
    for (n, v) in fp::PR_NAMES {
        c.insert(intern(n), Value::Num(v as f64));
    }
}

fn frame_of(e: &Engine, spr: u32, f: f64) -> u32 {
    let n = e.sprites[spr as usize].frames.len() as i64;
    (f.floor() as i64).rem_euclid(n.max(1)) as u32
}

fn rgb(v: f64) -> u32 {
    (v.max(0.0) as u32) & 0xFF_FFFF
}

fn rgba8(c: u32, alpha: f64) -> [u8; 4] {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    [(c >> 16 & 255) as u8, (c >> 8 & 255) as u8, (c & 255) as u8, a]
}

fn view_id(p: &A, i: usize) -> RtResult<u32> {
    let v = p.num(i)?;
    if v < 0.0 || v.fract() != 0.0 {
        return err(format!("argument #{} must be a view id (fpview_create), got {v}", i + 1));
    }
    Ok(v as u32)
}

fn view<'a>(e: &'a mut Engine, p: &A, i: usize, f: &str) -> RtResult<&'a mut FpView> {
    let id = view_id(p, i)?;
    e.fp.views.get_mut(&id).ok_or_else(|| modscript::RtError::new(format!("{f}: the view {id} does not exist (fpview_create)")))
}

/// A map from a 2D array (`map[y][x]`) or a ds_grid id: (width, height, row-major cells).
pub(crate) fn map_arg(e: &Engine, v: &Value, f: &str) -> RtResult<(usize, usize, Vec<i32>)> {
    match v {
        Value::Array(rows) => {
            let rows = rows.borrow();
            let h = rows.len();
            let mut w = 0;
            let mut grid: Vec<Vec<i32>> = Vec::with_capacity(h);
            for (y, r) in rows.iter().enumerate() {
                let Value::Array(r) = r else {
                    return err(format!("{f}: the map must be an array of rows (map[y][x]); row {y} is a {}", r.type_name()));
                };
                let r: Vec<i32> = r.borrow().iter().map(|c| c.as_num().unwrap_or(0.0) as i32).collect();
                w = w.max(r.len());
                grid.push(r);
            }
            let mut cells = vec![0; w * h];
            for (y, r) in grid.iter().enumerate() {
                cells[y * w..y * w + r.len()].copy_from_slice(r);
            }
            Ok((w, h, cells))
        }
        Value::Num(_) => grid_cells(e, v, f),
        other => err(format!("{f}: the map must be a 2D array (map[y][x]) or a ds_grid, got {}", other.type_name())),
    }
}

/// A `ds_grid` (numbers; anything else counts as 0) as a map.
fn grid_cells(e: &Engine, v: &Value, f: &str) -> RtResult<(usize, usize, Vec<i32>)> {
    let id = v.as_num().unwrap_or(-1.0);
    let Some(g) = e.ds_grids.get(id) else { return err(format!("{f}: {v} is not a ds_grid (or use a 2D array map[y][x])")) };
    let mut cells = Vec::with_capacity(g.w * g.h);
    for y in 0..g.h as i64 {
        for x in 0..g.w as i64 {
            cells.push(g.num(x, y, 0.0) as i32);
        }
    }
    Ok((g.w, g.h, cells))
}

/// Floor / ceiling: a sprite (with frame), a colour, or undefined / false = nothing.
fn fill_arg(e: &Engine, p: &A, i: usize, sub: usize) -> RtResult<Fill> {
    match p.0.get(i) {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(Fill::None),
        Some(Value::Asset(AssetKind::Sprite, s)) => Ok(Fill::Tex { spr: *s, frame: frame_of(e, *s, p.num_or(sub, 0.0)?) }),
        Some(Value::Num(c)) if *c >= 0.0 => Ok(Fill::Color(rgb(*c))),
        Some(other) => err(format!("argument #{} must be a sprite, a colour or undefined, got {}", i + 1, other.type_name())),
    }
}

fn opt_sprite(p: &A, i: usize) -> RtResult<Option<u32>> {
    match p.0.get(i) {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(None),
        _ => p.asset(i, AssetKind::Sprite).map(Some),
    }
}

fn s(fields: Vec<(&str, Value)>) -> Value {
    Value::new_struct(fields.into_iter().map(|(k, v)| (intern(k), v)))
}

fn n(v: impl Into<f64>) -> Value {
    Value::Num(v.into())
}

pub(super) fn register(r: &mut Reg) {
    // ---------------------------------------------------------------- grid directions
    r.f("grid_dir_dx", |_, _, a| Ok(n(fp::dir_dx(A(a).int(0)?) as f64)));
    r.f("grid_dir_dy", |_, _, a| Ok(n(fp::dir_dy(A(a).int(0)?) as f64)));
    r.f("grid_dir_angle", |_, _, a| Ok(n(fp::norm_dir(A(a).int(0)?) as f64 * 90.0)));
    r.f("angle_to_grid_dir", |_, _, a| Ok(n(fp::angle_to_dir(A(a).num(0)?) as f64)));

    // ---------------------------------------------------------------- views
    r.f("fpview_create", |e, _, a| {
        let p = A(a);
        let (w, h) = (p.num_or(0, 320.0)?, p.num_or(1, 180.0)?);
        if !(1.0..=4096.0).contains(&w) || !(1.0..=4096.0).contains(&h) {
            return err(format!("fpview_create: size {w}x{h} out of range (1..4096)"));
        }
        e.fp.next += 1;
        let id = e.fp.next;
        e.fp.views.insert(id, FpView::new(w as u32, h as u32));
        Ok(n(id))
    });
    r.f("fpview_destroy", |e, _, a| {
        let id = view_id(&A(a), 0)?;
        Ok(Value::Bool(e.fp.views.remove(&id).is_some()))
    });
    r.f("fpview_exists", |e, _, a| Ok(Value::Bool(A(a).num(0).ok().is_some_and(|v| v >= 0.0 && e.fp.views.contains_key(&(v as u32))))));
    r.f("fpview_set_map", |e, _, a| {
        let p = A(a);
        let (w, h, cells) = map_arg(e, p.val(1)?, "fpview_set_map")?;
        view(e, &p, 0, "fpview_set_map")?.set_map(w, h, cells);
        Ok(Value::Null)
    });
    r.f("fpview_get_cell", |e, _, a| {
        let p = A(a);
        let (x, y) = (p.int(1)?, p.int(2)?);
        Ok(n(view(e, &p, 0, "fpview_get_cell")?.cell(x, y)))
    });
    r.f("fpview_set_cell", |e, _, a| {
        let p = A(a);
        let (x, y, v) = (p.int(1)?, p.int(2)?, p.int(3)? as i32);
        Ok(Value::Bool(view(e, &p, 0, "fpview_set_cell")?.set_cell(x, y, v)))
    });
    r.f("fpview_set_wall", |e, _, a| {
        // fpview_set_wall(view, type, sprite_or_undefined, [subimg], [color], [solid])
        let p = A(a);
        let t = p.int(1)? as i32;
        if t <= 0 {
            return err("fpview_set_wall: wall types are numbers > 0 (0 is an empty cell)");
        }
        let tex = match opt_sprite(&p, 2)? {
            Some(s) => Some((s, frame_of(e, s, p.num_or(3, 0.0)?))),
            None => None,
        };
        let default_col = if tex.is_some() { 0xFFFFFF } else { 0x808080 };
        let wt = WallType { tex, color: rgb(p.num_or(4, default_col as f64)?), solid: p.bool_or(5, true) };
        let v = view(e, &p, 0, "fpview_set_wall")?;
        v.walls.insert(t, wt);
        v.dirty();
        Ok(Value::Null)
    });
    r.f("fpview_set_floor", |e, _, a| {
        let p = A(a);
        let f = fill_arg(e, &p, 1, 2)?;
        let v = view(e, &p, 0, "fpview_set_floor")?;
        v.floor = f;
        v.dirty();
        Ok(Value::Null)
    });
    r.f("fpview_set_ceiling", |e, _, a| {
        let p = A(a);
        let f = fill_arg(e, &p, 1, 2)?;
        let v = view(e, &p, 0, "fpview_set_ceiling")?;
        v.ceiling = f;
        v.dirty();
        Ok(Value::Null)
    });
    fn cell_fill(e: &mut Engine, a: &[Value], ceiling: bool, f: &str) -> RtResult<Value> {
        // fpview_set_cell_floor(view, x, y, sprite_or_color_or_undefined, [subimg]); a missing
        // 4th argument removes the override
        let p = A(a);
        let (x, y) = (p.int(1)?, p.int(2)?);
        let fill = if p.len() > 3 { Some(fill_arg(e, &p, 3, 4)?) } else { None };
        let v = view(e, &p, 0, f)?;
        let m = if ceiling { &mut v.cell_ceiling } else { &mut v.cell_floor };
        match fill {
            Some(fl) => m.insert((x, y), fl),
            None => m.remove(&(x, y)),
        };
        v.dirty();
        Ok(Value::Null)
    }
    r.f("fpview_set_cell_floor", |e, _, a| cell_fill(e, a, false, "fpview_set_cell_floor"));
    r.f("fpview_set_cell_ceiling", |e, _, a| cell_fill(e, a, true, "fpview_set_cell_ceiling"));
    r.f("fpview_set_edge", |e, _, a| {
        // fpview_set_edge(view, x, y, dir, type): thin wall / door on side `dir` of (x, y); 0 removes
        let p = A(a);
        let (x, y, d, t) = (p.int(1)?, p.int(2)?, p.int(3)?, p.int(4)? as i32);
        view(e, &p, 0, "fpview_set_edge")?.set_edge(x, y, d, t);
        Ok(Value::Null)
    });
    r.f("fpview_get_edge", |e, _, a| {
        let p = A(a);
        let (x, y, d) = (p.int(1)?, p.int(2)?, p.int(3)?);
        Ok(n(view(e, &p, 0, "fpview_get_edge")?.edge(x, y, d)))
    });
    r.f("fpview_set_fog", |e, _, a| {
        let p = A(a);
        let (c, s0, s1) = (rgb(p.num(1)?), p.num(2)? as f32, p.num(3)? as f32);
        let v = view(e, &p, 0, "fpview_set_fog")?;
        v.fog = (c, s0.max(0.0), s1.max(s0.max(0.0) + 0.001));
        Ok(Value::Null)
    });
    r.f("fpview_set_fov", |e, _, a| {
        let p = A(a);
        let f = p.num(1)?.clamp(10.0, 170.0) as f32;
        view(e, &p, 0, "fpview_set_fov")?.fov = f;
        Ok(Value::Null)
    });
    r.f("fpview_set_draw_distance", |e, _, a| {
        let p = A(a);
        let d = p.num(1)?.max(0.0) as f32;
        view(e, &p, 0, "fpview_set_draw_distance")?.max_dist = d;
        Ok(Value::Null)
    });
    r.f("fpview_set_shading", |e, _, a| {
        let p = A(a);
        let k = p.num(1)?.clamp(0.0, 1.0) as f32;
        let v = view(e, &p, 0, "fpview_set_shading")?;
        v.side_shade = k;
        v.dirty();
        Ok(Value::Null)
    });
    r.f("fpview_set_camera", |e, _, a| {
        // fpview_set_camera(view, x, y, angle, [height = 0.5], [pitch = 0])
        let p = A(a);
        let cam = fp::Cam {
            x: p.num(1)? as f32,
            y: p.num(2)? as f32,
            angle: p.num(3)? as f32,
            height: p.num_or(4, 0.5)?.clamp(0.01, 0.99) as f32,
            pitch: p.num_or(5, 0.0)?.clamp(-89.0, 89.0) as f32,
        };
        view(e, &p, 0, "fpview_set_camera")?.cam = cam;
        Ok(Value::Null)
    });
    r.f("fpview_get_camera", |e, _, a| {
        let p = A(a);
        let c = view(e, &p, 0, "fpview_get_camera")?.cam;
        Ok(s(vec![("x", n(c.x)), ("y", n(c.y)), ("angle", n(c.angle)), ("height", n(c.height)), ("pitch", n(c.pitch))]))
    });
    r.f("fpview_add_billboard", |e, _, a| {
        // fpview_add_billboard(view, x, y, sprite, [subimg], [scale = 0.6], [yoffset = 0], [color], [alpha])
        let p = A(a);
        let spr = p.asset(3, AssetKind::Sprite)?;
        let b = fp::Billboard {
            x: p.num(1)? as f32,
            y: p.num(2)? as f32,
            spr,
            frame: frame_of(e, spr, p.num_or(4, 0.0)?),
            scale: p.num_or(5, 0.6)?.max(0.0) as f32,
            yoff: p.num_or(6, 0.0)? as f32,
            color: rgb(p.num_or(7, 0xFFFFFF as f64)?),
            alpha: p.num_or(8, 1.0)?.clamp(0.0, 1.0) as f32,
        };
        let v = view(e, &p, 0, "fpview_add_billboard")?;
        v.billboards.push(b);
        Ok(n((v.billboards.len() - 1) as f64))
    });
    r.f("fpview_clear_billboards", |e, _, a| {
        view(e, &A(a), 0, "fpview_clear_billboards")?.billboards.clear();
        Ok(Value::Null)
    });
    r.f("fpview_draw", |e, _, a| {
        // fpview_draw(view, x, y, [w], [h]) in the current coordinates / target
        let p = A(a);
        let id = view_id(&p, 0)?;
        let (x, y) = (p.num(1)? as f32, p.num(2)? as f32);
        let Some(mut v) = e.fp.views.remove(&id) else { return err(format!("fpview_draw: the view {id} does not exist (fpview_create)")) };
        let (w, h) = (p.num_or(3, v.w as f64)? as f32, p.num_or(4, v.h as f64)? as f32);
        v.last_rect = [x, y, w, h];
        let frame = v.frame(&e.sprites);
        e.fp.views.insert(id, v);
        e.cmds.push(DrawCmd::FpView { frame: Rc::new(frame), x, y, w, h });
        Ok(Value::Null)
    });
    r.f("fpview_pick", |e, _, a| {
        // fpview_pick(view, x, y) with (x, y) in the coordinates of the last fpview_draw
        let p = A(a);
        let id = view_id(&p, 0)?;
        let (sx, sy) = (p.num(1)? as f32, p.num(2)? as f32);
        let Some(mut v) = e.fp.views.remove(&id) else { return err(format!("fpview_pick: the view {id} does not exist (fpview_create)")) };
        let [rx, ry, rw, rh] = v.last_rect;
        let (px, py) = ((sx - rx) / rw.max(1e-6) * v.w as f32, (sy - ry) / rh.max(1e-6) * v.h as f32);
        let inside = (0.0..v.w as f32).contains(&px) && (0.0..v.h as f32).contains(&py);
        let hit = if inside { v.pick(&e.sprites, px, py) } else { None };
        e.fp.views.insert(id, v);
        Ok(match hit {
            None => Value::Null,
            Some(h) => s(vec![
                ("kind", Value::str(h.kind.name())),
                ("index", if h.kind == fp::PickKind::Billboard { n(h.index as f64) } else { n(-1.0) }),
                ("cell_x", n(h.cell_x as f64)),
                ("cell_y", n(h.cell_y as f64)),
                ("dir", n(h.dir)),
                ("value", n(h.value)),
                ("distance", n(h.dist)),
                ("x", n(h.x)),
                ("y", n(h.y)),
            ]),
        })
    });
    r.f("fpview_project", |e, _, a| {
        // fpview_project(view, x, y, [height = 0.5]) -> { x, y, depth } in the coordinates of
        // the last fpview_draw, or undefined behind the camera
        let p = A(a);
        let (x, y, hgt) = (p.num(1)? as f32, p.num(2)? as f32, p.num_or(3, 0.5)? as f32);
        let v = view(e, &p, 0, "fpview_project")?;
        let [rx, ry, rw, rh] = v.last_rect;
        Ok(match v.project_world([x + 0.5, hgt, y + 0.5]) {
            None => Value::Null,
            Some((px, py, d)) => s(vec![("x", n(rx + px / v.w as f32 * rw)), ("y", n(ry + py / v.h as f32 * rh)), ("depth", n(d))]),
        })
    });
    r.f("fpview_can_move", |e, _, a| {
        let p = A(a);
        let (x, y, d) = (p.int(1)?, p.int(2)?, p.int(3)?);
        Ok(Value::Bool(view(e, &p, 0, "fpview_can_move")?.can_move(x, y, d)))
    });

    // ---------------------------------------------------------------- primitives
    r.f("draw_sprite_pos", |e, _, a| {
        // draw_sprite_pos(sprite, subimg, x1, y1, x2, y2, x3, y3, x4, y4, alpha, [perspective])
        let p = A(a);
        let spr = p.asset(0, AssetKind::Sprite)?;
        let frame = frame_of(e, spr, p.num(1)?);
        let mut q = [[0f32; 2]; 4];
        for (k, c) in q.iter_mut().enumerate() {
            *c = [p.num(2 + k * 2)? as f32, p.num(3 + k * 2)? as f32];
        }
        let alpha = p.num_or(10, 1.0)?;
        let (uv, _) = fp::frame_uv(&e.sprites[spr as usize], frame);
        let verts = fp::sprite_pos_tris(q, uv, rgba8(0xFFFFFF, alpha), p.bool_or(11, false));
        e.cmds.push(DrawCmd::Mesh { tex: Some(spr), verts });
        Ok(Value::Null)
    });
    fn begin(e: &mut Engine, a: &[Value], textured: bool) -> RtResult<Value> {
        let p = A(a);
        let kind = p.int(0)?;
        if !(1..=6).contains(&kind) {
            return err(format!("draw_primitive_begin: unknown kind {kind} (pr_pointlist, pr_linelist, pr_linestrip, pr_trianglelist, pr_trianglestrip, pr_trianglefan)"));
        }
        let tex = if textured {
            match opt_sprite(&p, 1)? {
                Some(spr) => {
                    let frame = frame_of(e, spr, p.num_or(2, 0.0)?);
                    Some((spr, fp::frame_uv(&e.sprites[spr as usize], frame).0))
                }
                None => None,
            }
        } else {
            None
        };
        if e.fp.prim.is_some() {
            return err("draw_primitive_begin: the previous primitive was not finished (draw_primitive_end)");
        }
        e.fp.prim = Some(PrimBuild { kind: kind as u8, tex, verts: vec![] });
        Ok(Value::Null)
    }
    r.f("draw_primitive_begin", |e, _, a| begin(e, a, false));
    r.f("draw_primitive_begin_texture", |e, _, a| begin(e, a, true));
    fn vertex(e: &mut Engine, a: &[Value], uv: bool, colored: bool) -> RtResult<Value> {
        let p = A(a);
        let (x, y) = (p.num(0)? as f32, p.num(1)? as f32);
        let (u, v) = if uv { (p.num(2)? as f32, p.num(3)? as f32) } else { (0.0, 0.0) };
        let k = if uv { 4 } else { 2 };
        let (c, al) = if colored { (rgb(p.num(k)?), p.num_or(k + 1, 1.0)?) } else { (e.draw.color, e.draw.alpha) };
        let Some(pb) = e.fp.prim.as_mut() else { return err("draw_vertex: call draw_primitive_begin first") };
        if pb.verts.len() >= 1 << 20 {
            return err("draw_vertex: too many vertices in one primitive (max 1048576)");
        }
        let (u, v) = match pb.tex {
            Some((_, r)) => (r[0] + (r[2] - r[0]) * u, r[1] + (r[3] - r[1]) * v),
            None => (0.0, 0.0),
        };
        pb.verts.push(MVert { x, y, u, v, color: rgba8(c, al) });
        Ok(Value::Null)
    }
    r.f("draw_vertex", |e, _, a| vertex(e, a, false, false));
    r.f("draw_vertex_color", |e, _, a| vertex(e, a, false, true));
    r.f("draw_vertex_colour", |e, _, a| vertex(e, a, false, true));
    r.f("draw_vertex_texture", |e, _, a| vertex(e, a, true, false));
    r.f("draw_vertex_texture_color", |e, _, a| vertex(e, a, true, true));
    r.f("draw_vertex_texture_colour", |e, _, a| vertex(e, a, true, true));
    r.f("draw_primitive_end", |e, _, _| {
        let Some(pb) = e.fp.prim.take() else { return err("draw_primitive_end without draw_primitive_begin") };
        let verts = fp::triangulate(pb.kind, &pb.verts);
        if !verts.is_empty() {
            e.cmds.push(DrawCmd::Mesh { tex: pb.tex.map(|t| t.0), verts });
        }
        Ok(Value::Null)
    });

    // ---------------------------------------------------------------- minimap
    r.f("draw_minimap", |e, _, a| {
        // draw_minimap(map, x, y, cell_px, [options])
        let p = A(a);
        let (w, h, cells) = map_arg(e, p.val(0)?, "draw_minimap")?;
        let opts = minimap_opts(e, &p, 4)?;
        minimap(e, w, h, &cells, None, p.num(1)? as f32, p.num(2)? as f32, p.num(3)? as f32, &opts);
        Ok(Value::Null)
    });
    r.f("fpview_draw_minimap", |e, _, a| {
        // fpview_draw_minimap(view, x, y, cell_px, [options]): the view's map, edges and camera
        let p = A(a);
        let mut opts = minimap_opts(e, &p, 4)?;
        let v = view(e, &p, 0, "fpview_draw_minimap")?;
        let (w, h, cells) = (v.mw, v.mh, v.cells.clone());
        let edges: Vec<(i64, i64, u8)> = v.edges.keys().copied().collect();
        if opts.player.is_none() {
            opts.player = Some((v.cam.x, v.cam.y, v.cam.angle));
        }
        minimap(e, w, h, &cells, Some(&edges), p.num(1)? as f32, p.num(2)? as f32, p.num(3)? as f32, &opts);
        Ok(Value::Null)
    });

    // ---------------------------------------------------------------- post-process
    r.f("postfx_pixelate", |e, _, a| {
        // postfx_pixelate(pixels_wide, [posterize_levels = 0], [dither = 0], [palette], [include_gui = false])
        let p = A(a);
        let w = p.num(0)?;
        if !(1.0..=8192.0).contains(&w) {
            return err(format!("postfx_pixelate: pixels_wide must be 1..8192, got {w}"));
        }
        let levels = p.num_or(1, 0.0)?.clamp(0.0, 256.0) as u32;
        let dither = p.num_or(2, 0.0)?.clamp(0.0, 4.0) as f32;
        let palette = palette_arg(e, &p, 3)?;
        e.fp.postfx = Some(PostFx { pixels_wide: w as u32, levels: if levels == 1 { 2 } else { levels }, dither, palette: Rc::new(palette), gui: p.bool_or(4, false) });
        Ok(Value::Null)
    });
    r.f("postfx_off", |e, _, _| {
        e.fp.postfx = None;
        Ok(Value::Null)
    });
    r.f("postfx_transition", |e, _, a| {
        e.fp.transition = A(a).num(0)?.clamp(0.0, 1.0) as f32;
        Ok(Value::Null)
    });
    r.f("postfx_get", |e, _, _| {
        Ok(match &e.fp.postfx {
            None => Value::Null,
            Some(p) => s(vec![
                ("pixels_wide", n(p.pixels_wide)),
                ("levels", n(p.levels)),
                ("dither", n(p.dither)),
                ("palette_size", n(p.palette.len() as f64)),
                ("include_gui", Value::Bool(p.gui)),
                ("transition", n(e.fp.transition)),
            ]),
        })
    });
}

/// Palette for `postfx_pixelate`: `true` = game.toml's, an array of colours, or a sprite
/// (its distinct opaque colours, e.g. an imported Lospec PNG).
fn palette_arg(e: &Engine, p: &A, i: usize) -> RtResult<Vec<u32>> {
    let mut out: Vec<u32> = match p.0.get(i) {
        None | Some(Value::Null) | Some(Value::Bool(false)) => vec![],
        Some(Value::Bool(true)) => crate::palette::ordered(&e.config.palette).into_iter().map(|(_, c)| c).collect(),
        Some(Value::Array(arr)) => arr.borrow().iter().filter_map(|v| v.as_num()).map(rgb).collect(),
        Some(Value::Asset(AssetKind::Sprite, s)) => {
            let sp = &e.sprites[*s as usize];
            let mut v = vec![];
            for px in sp.rgba.chunks_exact(4) {
                if px[3] >= 128 {
                    let c = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
                    if !v.contains(&c) {
                        v.push(c);
                        if v.len() > fp::MAX_PALETTE {
                            break;
                        }
                    }
                }
            }
            v
        }
        Some(other) => return err(format!("postfx_pixelate: the palette must be true, an array of colours or a sprite, got {}", other.type_name())),
    };
    if out.len() > fp::MAX_PALETTE {
        return err(format!("postfx_pixelate: at most {} palette colours", fp::MAX_PALETTE));
    }
    out.dedup();
    Ok(out)
}

// ------------------------------------------------------------------ minimap

pub(crate) struct MinimapOpts {
    visited: Option<(usize, usize, Vec<i32>)>,
    /// Cell x, y and angle in degrees.
    player: Option<(f32, f32, f32)>,
    /// Window size in pixels (clipped, scrolls to keep the player centred).
    window: Option<(f32, f32)>,
    wall: u32,
    floor: u32,
    unseen: Option<u32>,
    edge: u32,
    player_col: u32,
    background: Option<u32>,
    alpha: f64,
    markers: Vec<(f32, f32, u32)>,
}

fn minimap_opts(e: &Engine, p: &A, i: usize) -> RtResult<MinimapOpts> {
    let mut o = MinimapOpts {
        visited: None,
        player: None,
        window: None,
        wall: 0x8b8b8b,
        floor: 0x3b3b52,
        unseen: None,
        edge: 0xe0c070,
        player_col: 0xffd34d,
        background: None,
        alpha: e.draw.alpha,
        markers: vec![],
    };
    let Some(v) = p.opt(i) else { return Ok(o) };
    let Value::Struct(st) = v else { return err(format!("argument #{} of the minimap must be a struct of options", i + 1)) };
    let st = st.borrow();
    let get = |k: &str| st.fields.get(&intern(k)).filter(|v| !v.is_null());
    let num = |k: &str| get(k).and_then(|v| v.as_num());
    if let Some(v) = get("visited") {
        o.visited = Some(map_arg(e, v, "draw_minimap visited")?);
    }
    if let (Some(x), Some(y)) = (num("player_x"), num("player_y")) {
        let ang = num("angle").or_else(|| num("dir").map(|d| fp::norm_dir(d.floor() as i64) as f64 * 90.0)).unwrap_or(0.0);
        o.player = Some((x as f32, y as f32, ang as f32));
    }
    if let (Some(w), Some(h)) = (num("width"), num("height")) {
        o.window = Some((w.max(0.0) as f32, h.max(0.0) as f32));
    }
    let col = |k: &str, d: u32| num(k).map(rgb).unwrap_or(d);
    o.wall = col("wall", o.wall);
    o.floor = col("floor", o.floor);
    o.edge = col("edge", o.edge);
    o.player_col = col("player", o.player_col);
    o.unseen = num("unseen").map(rgb);
    o.background = num("background").map(rgb);
    if let Some(al) = num("alpha") {
        o.alpha = al.clamp(0.0, 1.0);
    }
    if let Some(Value::Array(ms)) = get("markers") {
        for m in ms.borrow().iter() {
            if let Value::Struct(m) = m {
                let m = m.borrow();
                let g = |k: &str| m.fields.get(&intern(k)).and_then(|v| v.as_num());
                if let (Some(x), Some(y)) = (g("x"), g("y")) {
                    o.markers.push((x as f32, y as f32, g("color").map(rgb).unwrap_or(0xff4040)));
                }
            }
        }
    }
    Ok(o)
}

#[allow(clippy::too_many_arguments)]
fn minimap(e: &mut Engine, w: usize, h: usize, cells: &[i32], edges: Option<&[(i64, i64, u8)]>, x: f32, y: f32, cp: f32, o: &MinimapOpts) {
    let cp = cp.max(0.5);
    let col = |c: u32| Rgba::from_rgb(c, o.alpha);
    // window: clip and scroll so the player stays centred
    let (ox, oy, clip) = match (o.window, o.player) {
        (Some((ww, wh)), Some((px, py, _))) => (x + ww * 0.5 - (px + 0.5) * cp, y + wh * 0.5 - (py + 0.5) * cp, Some([x, y, ww, wh])),
        (Some((ww, wh)), None) => (x, y, Some([x, y, ww, wh])),
        _ => (x, y, None),
    };
    let prev_clip = e.ui.clip.last().copied();
    if let Some(c) = clip {
        let c = match prev_clip {
            Some(p) => {
                let (x0, y0) = (c[0].max(p[0]), c[1].max(p[1]));
                let (x1, y1) = ((c[0] + c[2]).min(p[0] + p[2]), (c[1] + c[3]).min(p[1] + p[3]));
                [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
            }
            None => c,
        };
        if let Some(bg) = o.background {
            e.cmds.push(DrawCmd::Rect { x: c[0], y: c[1], w: c[2], h: c[3], color: col(bg), outline: false });
        }
        e.cmds.push(DrawCmd::Scissor(Some(c)));
    } else if let Some(bg) = o.background {
        e.cmds.push(DrawCmd::Rect { x, y, w: w as f32 * cp, h: h as f32 * cp, color: col(bg), outline: false });
    }
    // only the cells that can be on screen
    let (cx0, cy0, cx1, cy1) = match clip {
        Some(c) => (
            (((c[0] - ox) / cp).floor() as i64).max(0),
            (((c[1] - oy) / cp).floor() as i64).max(0),
            (((c[0] + c[2] - ox) / cp).ceil() as i64).min(w as i64),
            (((c[1] + c[3] - oy) / cp).ceil() as i64).min(h as i64),
        ),
        None => (0, 0, w as i64, h as i64),
    };
    let seen = |cx: i64, cy: i64| -> bool {
        match &o.visited {
            None => true,
            Some((vw, vh, v)) => cx >= 0 && cy >= 0 && (cx as usize) < *vw && (cy as usize) < *vh && v[cy as usize * vw + cx as usize] != 0,
        }
    };
    let near_seen = |cx: i64, cy: i64| (-1..=1).any(|dy| (-1..=1).any(|dx| seen(cx + dx, cy + dy)));
    for cy in cy0..cy1 {
        for cx in cx0..cx1 {
            let v = cells[cy as usize * w + cx as usize];
            // walls show once a neighbouring cell was visited
            let shown = if v > 0 { near_seen(cx, cy) } else { seen(cx, cy) };
            let c = match (shown, v > 0) {
                (true, true) => Some(o.wall),
                (true, false) => Some(o.floor),
                (false, _) => o.unseen,
            };
            if let Some(c) = c {
                e.cmds.push(DrawCmd::Rect { x: ox + cx as f32 * cp, y: oy + cy as f32 * cp, w: cp, h: cp, color: col(c), outline: false });
            }
        }
    }
    for &(ex, ey, d) in edges.unwrap_or(&[]) {
        if !seen(ex, ey) && !seen(ex + fp::dir_dx(d as i64), ey + fp::dir_dy(d as i64)) {
            continue;
        }
        let (x0, y0) = (ox + ex as f32 * cp, oy + ey as f32 * cp);
        let (x1, y1, x2, y2) = if d == 0 { (x0 + cp, y0, x0 + cp, y0 + cp) } else { (x0, y0 + cp, x0 + cp, y0 + cp) };
        e.cmds.push(DrawCmd::Line { x1, y1, x2, y2, width: (cp / 4.0).max(1.0), color: col(o.edge) });
    }
    for &(mx, my, c) in &o.markers {
        let r = (cp * 0.3).max(1.0);
        e.cmds.push(DrawCmd::Rect { x: ox + (mx + 0.5) * cp - r, y: oy + (my + 0.5) * cp - r, w: r * 2.0, h: r * 2.0, color: col(c), outline: false });
    }
    if let Some((px, py, ang)) = o.player {
        let (cx, cy) = (ox + (px + 0.5) * cp, oy + (py + 0.5) * cp);
        let a = ang.to_radians();
        let (fx, fy) = (a.cos(), -a.sin());
        let r = (cp * 0.45).max(2.0);
        let tip = (cx + fx * r, cy + fy * r);
        let back = (cx - fx * r * 0.7, cy - fy * r * 0.7);
        let (sx, sy) = (-fy * r * 0.7, fx * r * 0.7);
        let pts = [tip.0, tip.1, back.0 + sx, back.1 + sy, back.0 - sx, back.1 - sy];
        e.cmds.push(DrawCmd::Tri { pts, color: col(o.player_col), outline: false });
    }
    if clip.is_some() {
        e.cmds.push(DrawCmd::Scissor(prev_clip));
    }
}

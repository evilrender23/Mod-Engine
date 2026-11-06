//! Navigation grids with GameMaker's names (`mp_grid_create`, `mp_grid_add_rectangle`,
//! `mp_grid_path`...) plus point-array paths, BFS distance maps and flow fields
//! (`mp_grid_path_points`, `mp_grid_distance_map`, `mp_flow_*`).

use super::{err, Reg, A};
use crate::draw::{DrawCmd, Rgba};
use crate::engine::{Engine, LayerKind};
use crate::navgrid::{Flow, MpGrid, INF};
use crate::path::{PathPoint, PathToml};
use modscript::{intern, AssetKind, RtResult, Value};

pub(crate) fn grid_ref(e: &Engine, id: f64) -> RtResult<&MpGrid> {
    match e.nav.grids.get(id.max(0.0) as usize).and_then(|g| g.as_ref()) {
        Some(g) if id >= 0.0 => Ok(g),
        _ => err(format!("{id} is not an mp_grid (destroyed?)")),
    }
}

fn grid_id(e: &Engine, a: &A, i: usize) -> RtResult<usize> {
    let id = a.num(i)?;
    grid_ref(e, id)?;
    Ok(id as usize)
}

fn grid_mut<'a>(e: &'a mut Engine, a: &A, i: usize) -> RtResult<&'a mut MpGrid> {
    let id = grid_id(e, a, i)?;
    Ok(e.nav.grids[id].as_mut().expect("checked"))
}

fn flow_id(e: &Engine, a: &A, i: usize) -> RtResult<usize> {
    let id = a.num(i)?;
    match e.nav.flows.get(id.max(0.0) as usize).and_then(|f| f.as_ref()) {
        Some(_) if id >= 0.0 => Ok(id as usize),
        _ => err(format!("{id} is not a flow field (destroyed?)")),
    }
}

/// The cell under a pixel position, if it is inside the grid.
fn cell_in(g: &MpGrid, x: f64, y: f64) -> Option<(usize, usize)> {
    let (cx, cy) = g.cell_of(x, y);
    g.in_bounds(cx, cy).then_some((cx as usize, cy as usize))
}

/// Pixel path: start point, centres of the intermediate cells, goal point.
fn path_points(g: &MpGrid, x1: f64, y1: f64, x2: f64, y2: f64, diag: bool) -> Option<Vec<(f64, f64)>> {
    let (a, b) = (cell_in(g, x1, y1)?, cell_in(g, x2, y2)?);
    let cells = g.astar(a, b, diag)?;
    let mut pts = vec![(x1, y1)];
    for &(cx, cy) in cells.iter().skip(1).take(cells.len().saturating_sub(2)) {
        pts.push(g.center(cx, cy));
    }
    pts.push((x2, y2));
    Some(pts)
}

/// Goal cells from (x, y) or an array of `[x, y]` points (argument `i`).
fn goals(g: &MpGrid, a: &A, i: usize) -> RtResult<Vec<(usize, usize)>> {
    if let Value::Array(arr) = a.val(i)? {
        let mut out = vec![];
        for p in arr.borrow().iter() {
            let xy = match p {
                Value::Array(q) => {
                    let q = q.borrow();
                    (q.first().and_then(|v| v.as_num()), q.get(1).and_then(|v| v.as_num()))
                }
                Value::Struct(s) => {
                    let s = s.borrow();
                    (s.fields.get(&intern("x")).and_then(|v| v.as_num()), s.fields.get(&intern("y")).and_then(|v| v.as_num()))
                }
                _ => (None, None),
            };
            match xy {
                (Some(x), Some(y)) => out.extend(cell_in(g, x, y)),
                _ => return err("goals must be [x, y] pairs or { x, y } structs"),
            }
        }
        return Ok(out);
    }
    Ok(cell_in(g, a.num(i)?, a.num(i + 1)?).into_iter().collect())
}

fn dist_value(d: f64) -> Value {
    Value::Num(if d == INF { -1.0 } else { d })
}

fn compute_flow(e: &Engine, grid: usize, goals: Vec<(usize, usize)>, diag: bool, max: f64) -> Flow {
    let g = e.nav.grids[grid].as_ref().expect("checked");
    Flow { grid, w: g.w, h: g.h, dist: g.distances(&goals, diag, max), diag, goals }
}

/// The grid of a flow field, which must still exist and have the same size.
fn flow_grid(e: &Engine, f: usize) -> RtResult<(&Flow, &MpGrid)> {
    let fl = e.nav.flows[f].as_ref().expect("checked");
    match e.nav.grids.get(fl.grid).and_then(|g| g.as_ref()) {
        Some(g) if g.w == fl.w && g.h == fl.h => Ok((fl, g)),
        _ => err("the flow field's grid was destroyed"),
    }
}

pub(super) fn register(r: &mut Reg) {
    // ---------------------------------------------------------------- grids
    r.f("mp_grid_create", |e, _, a| {
        // mp_grid_create(left, top, hcells, vcells, cellwidth, cellheight) -> id
        let p = A(a);
        let (w, h) = (p.num(2)?.max(0.0) as usize, p.num(3)?.max(0.0) as usize);
        if w == 0 || h == 0 || w * h > 4_000_000 {
            return err(format!("mp_grid_create: {w}×{h} cells is not a valid size"));
        }
        let (cw, ch) = (p.num(4)?, p.num(5)?);
        if cw <= 0.0 || ch <= 0.0 {
            return err("mp_grid_create: cells must be bigger than 0 px");
        }
        let g = MpGrid::new(p.num(0)?, p.num(1)?, w, h, cw, ch);
        let id = match e.nav.grids.iter().position(|g| g.is_none()) {
            Some(k) => {
                e.nav.grids[k] = Some(g);
                k
            }
            None => {
                e.nav.grids.push(Some(g));
                e.nav.grids.len() - 1
            }
        };
        Ok(Value::Num(id as f64))
    });
    r.f("mp_grid_destroy", |e, _, a| {
        let id = grid_id(e, &A(a), 0)?;
        e.nav.grids[id] = None;
        Ok(Value::Null)
    });
    r.f("mp_grid_exists", |e, _, a| Ok(Value::Bool(A(a).num(0).ok().is_some_and(|n| grid_ref(e, n).is_ok()))));
    r.f("mp_grid_clear_all", |e, _, a| {
        grid_mut(e, &A(a), 0)?.cells.iter_mut().for_each(|c| *c = false);
        Ok(Value::Null)
    });
    r.f("mp_grid_add_cell", |e, _, a| {
        let p = A(a);
        let (cx, cy) = (p.int(1)?, p.int(2)?);
        let g = grid_mut(e, &p, 0)?;
        let ok = g.in_bounds(cx, cy);
        g.set(cx, cy, true);
        Ok(Value::Bool(ok))
    });
    r.f("mp_grid_clear_cell", |e, _, a| {
        let p = A(a);
        let (cx, cy) = (p.int(1)?, p.int(2)?);
        let g = grid_mut(e, &p, 0)?;
        let ok = g.in_bounds(cx, cy);
        g.set(cx, cy, false);
        Ok(Value::Bool(ok))
    });
    r.f("mp_grid_get_cell", |e, _, a| {
        // -1 = blocked (or outside), 0 = free (GameMaker's values)
        let p = A(a);
        let g = grid_ref(e, p.num(0)?)?;
        Ok(Value::Num(if g.blocked(p.int(1)?, p.int(2)?) { -1.0 } else { 0.0 }))
    });
    r.f("mp_grid_cell_at", |e, _, a| {
        // mp_grid_cell_at(grid, x, y): the same for a pixel position
        let p = A(a);
        let g = grid_ref(e, p.num(0)?)?;
        let (cx, cy) = g.cell_of(p.num(1)?, p.num(2)?);
        Ok(Value::Num(if g.blocked(cx, cy) { -1.0 } else { 0.0 }))
    });
    r.f("mp_grid_cell_of", |e, _, a| {
        // mp_grid_cell_of(grid, x, y) -> [cx, cy] (may be outside the grid)
        let p = A(a);
        let (cx, cy) = grid_ref(e, p.num(0)?)?.cell_of(p.num(1)?, p.num(2)?);
        Ok(Value::array(vec![Value::Num(cx as f64), Value::Num(cy as f64)]))
    });
    r.f("mp_grid_cell_center", |e, _, a| {
        // mp_grid_cell_center(grid, cx, cy) -> [x, y]
        let p = A(a);
        let g = grid_ref(e, p.num(0)?)?;
        let (cx, cy) = (p.num(1)?.floor(), p.num(2)?.floor());
        Ok(Value::array(vec![Value::Num(g.left + (cx + 0.5) * g.cw), Value::Num(g.top + (cy + 0.5) * g.ch)]))
    });
    for (name, v) in [("mp_grid_add_rectangle", true), ("mp_grid_clear_rectangle", false)] {
        let f: super::NativeFn = if v {
            |e, _, a| rect_fill(e, a, true)
        } else {
            |e, _, a| rect_fill(e, a, false)
        };
        r.f(name, f);
    }
    fn rect_fill(e: &mut Engine, a: &[modscript::Value], v: bool) -> RtResult<Value> {
        let p = A(a);
        let (x1, y1, x2, y2) = (p.num(1)?, p.num(2)?, p.num(3)?, p.num(4)?);
        grid_mut(e, &p, 0)?.fill_rect(x1, y1, x2, y2, v);
        Ok(Value::Null)
    }
    r.f("mp_grid_add_instances", |e, _, a| {
        // mp_grid_add_instances(grid, obj, [prec]): cells their collision boxes overlap
        let p = A(a);
        let id = grid_id(e, &p, 0)?;
        let boxes: Vec<[f64; 4]> = e.resolve(&p.target(1)?).into_iter().filter_map(|i| e.idx(i).and_then(|k| e.bbox(k))).collect();
        let g = e.nav.grids[id].as_mut().expect("checked");
        for b in boxes {
            g.fill_box(b);
        }
        Ok(Value::Null)
    });
    r.f("mp_grid_add_tilemap", |e, _, a| {
        // mp_grid_add_tilemap(grid, layer, [tiles]): cells whose centre is on a non-empty tile
        // of that layer (or on one of the tile indices in the `tiles` array)
        let p = A(a);
        let id = grid_id(e, &p, 0)?;
        let name = p.text(1)?;
        let only: Option<Vec<i32>> = match p.opt(2) {
            Some(Value::Array(t)) => Some(t.borrow().iter().filter_map(|v| v.as_num()).map(|n| n as i32).collect()),
            Some(other) => return err(format!("mp_grid_add_tilemap: tiles must be an array of tile indices, got {}", other.type_name())),
            None => None,
        };
        let Some(li) = e.layers.iter().position(|l| l.name == name && matches!(l.kind, LayerKind::Tiles { .. })) else {
            return err(format!("there is no tile layer '{name}' in this room"));
        };
        let LayerKind::Tiles { cw, ch, grid, ox, oy, .. } = &e.layers[li].kind else { unreachable!() };
        let (cw, ch, ox, oy) = (*cw as f64, *ch as f64, *ox, *oy);
        let tiles = grid.clone();
        let g = e.nav.grids[id].as_mut().expect("checked");
        let mut n = 0;
        for cy in 0..g.h {
            for cx in 0..g.w {
                let (x, y) = g.center(cx, cy);
                let (tc, tr) = (((x - ox) / cw).floor(), ((y - oy) / ch).floor());
                if tc < 0.0 || tr < 0.0 {
                    continue;
                }
                let Some(&d) = tiles.get(tr as usize).and_then(|r| r.get(tc as usize)) else { continue };
                let idx = crate::tileset::index(d);
                if idx >= 0 && only.as_ref().is_none_or(|o| o.contains(&idx)) {
                    g.cells[cy * g.w + cx] = true;
                    n += 1;
                }
            }
        }
        Ok(Value::Num(n as f64))
    });
    r.f("mp_grid_rect_free", |e, _, a| {
        // mp_grid_rect_free(grid, x1, y1, x2, y2): no blocked cell under the rectangle
        let p = A(a);
        let g = grid_ref(e, p.num(0)?)?;
        let (x1, y1, x2, y2) = (p.num(1)?, p.num(2)?, p.num(3)?, p.num(4)?);
        let (ax, ay) = g.cell_of(x1.min(x2), y1.min(y2));
        let (bx, by) = g.cell_of(x1.max(x2) - 1e-6, y1.max(y2) - 1e-6);
        Ok(Value::Bool((ay..=by).all(|cy| (ax..=bx).all(|cx| !g.blocked(cx, cy)))))
    });
    r.f("mp_grid_place_free", |e, cx, a| {
        // mp_grid_place_free(grid, x, y): would the caller's box at (x, y) be on free cells only?
        let p = A(a);
        let i = super::me(e, cx)?;
        let g = grid_ref(e, p.num(0)?)?;
        let Some(b) = e.bbox_at(i, p.num(1)?, p.num(2)?) else { return Ok(Value::Bool(true)) };
        let (ax, ay) = g.cell_of(b[0], b[1]);
        let (bx, by) = g.cell_of(b[2] - 1e-6, b[3] - 1e-6);
        Ok(Value::Bool((ay..=by).all(|cy| (ax..=bx).all(|cx| !g.blocked(cx, cy)))))
    });

    // ---------------------------------------------------------------- paths
    r.f("mp_grid_path", |e, _, a| {
        // mp_grid_path(grid, path, xstart, ystart, xgoal, ygoal, allowdiag) -> found?
        let p = A(a);
        let g = grid_ref(e, p.num(0)?)?;
        let path = p.asset(1, AssetKind::Path)? as usize;
        if !e.paths.get(path).is_some_and(|d| d.alive) {
            return err("the path was deleted");
        }
        let Some(pts) = path_points(g, p.num(2)?, p.num(3)?, p.num(4)?, p.num(5)?, p.bool_or(6, false)) else { return Ok(Value::Bool(false)) };
        let data = &mut e.paths[path].data;
        let speed = 100.0;
        *data = PathToml { points: pts.into_iter().map(|(x, y)| PathPoint { x, y, speed }).collect(), closed: false, smooth: false, ..data.clone() };
        e.paths[path].rebuild();
        Ok(Value::Bool(true))
    });
    r.f("mp_grid_path_points", |e, _, a| {
        // mp_grid_path_points(grid, x1, y1, x2, y2, [diag=true]) -> [[x, y], ...] or [] if unreachable
        let p = A(a);
        let g = grid_ref(e, p.num(0)?)?;
        let pts = path_points(g, p.num(1)?, p.num(2)?, p.num(3)?, p.num(4)?, p.bool_or(5, true)).unwrap_or_default();
        Ok(Value::array(pts.into_iter().map(|(x, y)| Value::array(vec![Value::Num(x), Value::Num(y)])).collect()))
    });
    r.f("mp_grid_distance_map", |e, _, a| {
        // mp_grid_distance_map(grid, x, y, [diag=false], [max]) -> rows of distances in cells (-1 = unreachable)
        let p = A(a);
        let g = grid_ref(e, p.num(0)?)?;
        let goals = goals(g, &p, 1)?;
        let k = if matches!(p.val(1)?, Value::Array(_)) { 2 } else { 3 };
        let d = g.distances(&goals, p.bool_or(k, false), p.num_or(k + 1, INF)?);
        let rows = (0..g.h).map(|cy| Value::array((0..g.w).map(|cx| dist_value(d[cy * g.w + cx])).collect())).collect();
        Ok(Value::array(rows))
    });
    r.f("mp_grid_draw", |e, _, a| {
        // mp_grid_draw(grid, [free_color], [blocked_color]): free cells green, blocked red (alpha = draw alpha)
        let p = A(a);
        let g = grid_ref(e, p.num(0)?)?.clone();
        let alpha = e.draw.alpha;
        let free = p.opt(1).and_then(|v| v.as_num()).map(|c| Rgba::from_rgb(c as u32, alpha)).unwrap_or(Rgba::from_rgb(0x43A047, alpha * 0.35));
        let blocked = p.opt(2).and_then(|v| v.as_num()).map(|c| Rgba::from_rgb(c as u32, alpha)).unwrap_or(Rgba::from_rgb(0xE53935, alpha * 0.6));
        for cy in 0..g.h {
            for cx in 0..g.w {
                let color = if g.cells[cy * g.w + cx] { blocked } else { free };
                let (x, y) = (g.left + cx as f64 * g.cw, g.top + cy as f64 * g.ch);
                e.cmds.push(DrawCmd::Rect { x: x as f32, y: y as f32, w: g.cw as f32, h: g.ch as f32, color, outline: false });
            }
        }
        Ok(Value::Null)
    });
    r.f("mp_grid_raycast", |e, _, a| {
        // mp_grid_raycast(grid, x1, y1, x2, y2) -> { x, y, distance, nx, ny, cell_x, cell_y } or null
        let p = A(a);
        let g = grid_ref(e, p.num(0)?)?;
        Ok(match g.raycast(p.num(1)?, p.num(2)?, p.num(3)?, p.num(4)?) {
            Some(h) => Value::new_struct([
                (intern("x"), Value::Num(h.x)),
                (intern("y"), Value::Num(h.y)),
                (intern("distance"), Value::Num(h.dist)),
                (intern("nx"), Value::Num(h.nx)),
                (intern("ny"), Value::Num(h.ny)),
                (intern("cell_x"), Value::Num(h.cx as f64)),
                (intern("cell_y"), Value::Num(h.cy as f64)),
            ]),
            None => Value::Null,
        })
    });

    // ---------------------------------------------------------------- flow fields
    r.f("mp_flow_create", |e, _, a| {
        // mp_flow_create(grid, x, y, [diag=true], [max]) or mp_flow_create(grid, [[x, y], ...], [diag], [max])
        let p = A(a);
        let grid = grid_id(e, &p, 0)?;
        let goals = goals(e.nav.grids[grid].as_ref().expect("checked"), &p, 1)?;
        let k = if matches!(p.val(1)?, Value::Array(_)) { 2 } else { 3 };
        let flow = compute_flow(e, grid, goals, p.bool_or(k, true), p.num_or(k + 1, INF)?);
        let id = match e.nav.flows.iter().position(|f| f.is_none()) {
            Some(i) => {
                e.nav.flows[i] = Some(flow);
                i
            }
            None => {
                e.nav.flows.push(Some(flow));
                e.nav.flows.len() - 1
            }
        };
        Ok(Value::Num(id as f64))
    });
    r.f("mp_flow_update", |e, _, a| {
        // mp_flow_update(flow, [x, y]): recompute after the grid changed (optionally with a new goal)
        let p = A(a);
        let f = flow_id(e, &p, 0)?;
        let (grid, diag) = {
            let fl = e.nav.flows[f].as_ref().expect("checked");
            (fl.grid, fl.diag)
        };
        let g = match e.nav.grids.get(grid).and_then(|g| g.as_ref()) {
            Some(g) => g,
            None => return err("the flow field's grid was destroyed"),
        };
        let goals = if p.opt(1).is_some() { goals(g, &p, 1)? } else { e.nav.flows[f].as_ref().expect("checked").goals.clone() };
        let flow = compute_flow(e, grid, goals, diag, INF);
        e.nav.flows[f] = Some(flow);
        Ok(Value::Null)
    });
    r.f("mp_flow_destroy", |e, _, a| {
        let f = flow_id(e, &A(a), 0)?;
        e.nav.flows[f] = None;
        Ok(Value::Null)
    });
    r.f("mp_flow_distance", |e, _, a| {
        // distance to the nearest goal in cells (-1 = unreachable or outside the grid)
        let p = A(a);
        let f = flow_id(e, &p, 0)?;
        let (fl, g) = flow_grid(e, f)?;
        let (cx, cy) = g.cell_of(p.num(1)?, p.num(2)?);
        Ok(dist_value(fl.at(cx, cy)))
    });
    r.f("mp_flow_dir", |e, _, a| {
        // mp_flow_dir(flow, x, y): direction in degrees towards the next cell (-1 at a goal / unreachable)
        let p = A(a);
        let f = flow_id(e, &p, 0)?;
        let (fl, g) = flow_grid(e, f)?;
        let (x, y) = (p.num(1)?, p.num(2)?);
        let (cx, cy) = g.cell_of(x, y);
        Ok(Value::Num(match fl.next(g, cx, cy) {
            Some((nx, ny)) => {
                let (tx, ty) = g.center(nx, ny);
                (-(ty - y)).atan2(tx - x).to_degrees().rem_euclid(360.0) + 0.0
            }
            None => -1.0,
        }))
    });
    r.f("mp_flow_next", |e, _, a| {
        // mp_flow_next(flow, x, y) -> [x, y] centre of the next cell, or null
        let p = A(a);
        let f = flow_id(e, &p, 0)?;
        let (fl, g) = flow_grid(e, f)?;
        let (cx, cy) = g.cell_of(p.num(1)?, p.num(2)?);
        Ok(match fl.next(g, cx, cy) {
            Some((nx, ny)) => {
                let (x, y) = g.center(nx, ny);
                Value::array(vec![Value::Num(x), Value::Num(y)])
            }
            None => Value::Null,
        })
    });
}

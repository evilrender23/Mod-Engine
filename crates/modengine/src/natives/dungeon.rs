//! `dungeon_*`: the procedural dungeon generator of `dungeon.rs` for scripts, plus the
//! standalone `delaunay(points)` and `mst(points, [edges])`.

use super::{err, rand01, Reg, A};
use crate::draw::{DrawCmd, Rgba};
use crate::dsgrid::DsGrid;
use crate::dungeon::{self as dg, Canvas, Config, Dungeon, DungeonSlot, DrawOpts, ParseCtx};
use crate::engine::{Engine, LayerKind};
use modscript::{intern, RtError, RtResult, Value};
use serde_json::{Map, Value as J};

/// A template from a room asset: `{ room: "rm_x", layer: "walls", markers: { obj_enemy: "E" } }`.
/// Cells with a tile on `layer` (default: the first tile layer) are walls, the others floor;
/// instances of the objects in `markers` put that letter on their cell.
pub fn room_rows(e: &Engine, o: &Map<String, J>) -> Result<Vec<String>, String> {
    let name = o.get("room").and_then(|r| r.as_str()).ok_or("room debe ser el nombre de una room")?;
    let room = e.rooms.iter().find(|r| r.name == name).ok_or(format!("no hay ninguna room '{name}'"))?;
    let want = o.get("layer").and_then(|l| l.as_str());
    let layer = room
        .layers
        .iter()
        .find(|l| matches!(l.kind, LayerKind::Tiles { .. }) && want.is_none_or(|w| l.name == w))
        .ok_or(match want {
            Some(w) => format!("la room '{name}' no tiene la capa de tiles '{w}'"),
            None => format!("la room '{name}' no tiene capas de tiles (las paredes)"),
        })?;
    let LayerKind::Tiles { cw, ch, grid, ox, oy, .. } = &layer.kind else { unreachable!() };
    let (cw, ch) = ((*cw).max(1) as f64, (*ch).max(1) as f64);
    let (cols, rows) = (((room.w - ox) / cw).ceil().max(1.0) as usize, ((room.h - oy) / ch).ceil().max(1.0) as usize);
    let mut g: Vec<Vec<char>> = (0..rows)
        .map(|r| (0..cols).map(|c| if grid.get(r).and_then(|row| row.get(c)).is_some_and(|&d| crate::tileset::index(d) >= 0) { '#' } else { '.' }).collect())
        .collect();
    if let Some(J::Object(m)) = o.get("markers") {
        for s in &room.spawns {
            let obj = &e.objects[s.object as usize].name;
            if let Some(ch_) = m.get(obj).and_then(|v| v.as_str()).and_then(|v| v.chars().next()) {
                let (c, r) = (((s.x - ox) / cw).floor(), ((s.y - oy) / ch).floor());
                if c >= 0.0 && r >= 0.0 && (r as usize) < rows && (c as usize) < cols {
                    g[r as usize][c as usize] = ch_;
                }
            }
        }
    }
    Ok(g.into_iter().map(|r| r.into_iter().collect()).collect())
}

/// Reads a config with the project's folder (for `file = ...`) and rooms (for `room = ...`).
pub fn parse_config(e: &Engine, j: &J, warnings: &mut Vec<String>) -> Result<Config, String> {
    let loader = |o: &Map<String, J>| room_rows(e, o);
    let ctx = ParseCtx { base: Some(e.dir.clone()), room_loader: Some(&loader) };
    Config::parse(j, &ctx, warnings)
}

/// Text of a config file: TOML or JSON (by extension, else by trying both).
pub fn parse_config_text(text: &str, json_first: bool) -> Result<J, String> {
    let t = || toml::from_str::<J>(text).map_err(|e| format!("TOML no válido: {e}"));
    let j = || serde_json::from_str::<J>(text).map_err(|e| format!("JSON no válido: {e}"));
    if json_first {
        j().or_else(|_| t())
    } else {
        t().or_else(|e1| j().map_err(|_| e1))
    }
}

fn slot<'a>(e: &'a Engine, p: &A, i: usize) -> RtResult<&'a DungeonSlot> {
    let id = p.num(i)?;
    if id >= 0.0 {
        if let Some(Some(s)) = e.dungeons.list.get(id as usize) {
            return Ok(s);
        }
    }
    err(format!("{} no es una mazmorra de dungeon_create() (¿destruida?)", Value::Num(id)))
}

fn generated<'a>(e: &'a Engine, p: &A, i: usize) -> RtResult<&'a Dungeon> {
    slot(e, p, i)?.result.as_ref().ok_or_else(|| RtError::new("la mazmorra aún no se ha generado: llama antes a dungeon_generate(id, semilla)"))
}

fn json_val(j: &J) -> Value {
    super::grid::value_from_json(j)
}

fn xy(x: usize, y: usize) -> Value {
    Value::array(vec![Value::Num(x as f64), Value::Num(y as f64)])
}

/// Points from `[[x, y], ...]` or `[{x, y}, ...]`.
fn points(v: &Value, f: &str) -> RtResult<Vec<(f64, f64)>> {
    let Value::Array(a) = v else { return err(format!("{f}: se esperaba un array de puntos [[x, y], ...]")) };
    a.borrow()
        .iter()
        .map(|p| match p {
            Value::Array(q) => {
                let q = q.borrow();
                match (q.first().and_then(|v| v.as_num()), q.get(1).and_then(|v| v.as_num())) {
                    (Some(x), Some(y)) => Ok((x, y)),
                    _ => err(format!("{f}: cada punto debe ser [x, y]")),
                }
            }
            Value::Struct(s) => {
                let s = s.borrow();
                match (s.fields.get(&intern("x")).and_then(|v| v.as_num()), s.fields.get(&intern("y")).and_then(|v| v.as_num())) {
                    (Some(x), Some(y)) => Ok((x, y)),
                    _ => err(format!("{f}: cada punto debe ser {{ x, y }}")),
                }
            }
            _ => err(format!("{f}: cada punto debe ser [x, y] o {{ x, y }}")),
        })
        .collect()
}

fn edges_value(e: &[(usize, usize)]) -> Value {
    Value::array(e.iter().map(|&(a, b)| Value::array(vec![Value::Num(a as f64), Value::Num(b as f64)])).collect())
}

/// Draws into the engine's command list.
struct CmdCanvas<'a> {
    cmds: &'a mut Vec<DrawCmd>,
    alpha: f64,
}

impl Canvas for CmdCanvas<'_> {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, rgb: u32, alpha: f32) {
        self.cmds.push(DrawCmd::Rect { x, y, w, h, color: Rgba::from_rgb(rgb, alpha as f64 * self.alpha), outline: false });
    }
    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, rgb: u32, alpha: f32) {
        self.cmds.push(DrawCmd::Line { x1, y1, x2, y2, width, color: Rgba::from_rgb(rgb, alpha as f64 * self.alpha) });
    }
}

fn draw_opts(v: Option<&Value>) -> RtResult<DrawOpts> {
    let mut o = DrawOpts::default();
    let Some(v) = v else { return Ok(o) };
    let Value::Struct(s) = v else { return err("dungeon_debug_draw: las opciones deben ser un struct { mst: true, delaunay: false, ... }") };
    for (k, v) in s.borrow().fields.iter() {
        let b = v.truthy();
        match &*k.name() {
            "cells" => o.cells = b,
            "rooms" => o.rooms = b,
            "delaunay" => o.delaunay = b,
            "mst" => o.mst = b,
            "extra" => o.extra = b,
            "main_path" => o.main_path = b,
            "distance" => o.distance = b,
            "events" => o.events = b,
            other => return err(format!("dungeon_debug_draw: opción desconocida '{other}' (cells, rooms, delaunay, mst, extra, main_path, distance, events)")),
        }
    }
    Ok(o)
}

/// Refreshes the dungeon's own ds_grid with its cells.
fn sync_grid(e: &mut Engine, id: usize) -> usize {
    let s = e.dungeons.list[id].as_ref().expect("checked");
    let d = s.result.as_ref().expect("generated");
    let g = DsGrid::from_nums(d.w, d.h, &d.cells.iter().map(|&c| c as f64).collect::<Vec<_>>());
    match s.grid {
        Some(gid) if e.ds_grids.list.get(gid).is_some_and(|g| g.is_some()) => {
            e.ds_grids.list[gid] = Some(g);
            gid
        }
        _ => {
            let gid = e.ds_grids.add(g);
            e.dungeons.list[id].as_mut().expect("checked").grid = Some(gid);
            gid
        }
    }
}

fn place(d: &Dungeon, p: (usize, usize), room: usize) -> Value {
    Value::new_struct([(intern("x"), Value::Num(p.0 as f64)), (intern("y"), Value::Num(p.1 as f64)), (intern("room"), Value::Num(room as f64)), (intern("distance"), Value::Num(d.distance(p.0 as i64, p.1 as i64) as f64))])
}

pub(super) fn register(r: &mut Reg) {
    r.f("dungeon_create", |e, _, a| {
        // dungeon_create(config_struct | "file.toml") -> id
        let p = A(a);
        let j = match p.val(0)? {
            Value::Str(path) => {
                let full = e.dir.join(path.as_ref());
                let text = std::fs::read_to_string(&full).map_err(|er| RtError::new(format!("dungeon_create: no se pudo leer '{path}': {er}")))?;
                parse_config_text(&text, path.ends_with(".json")).map_err(|m| RtError::new(format!("dungeon_create: {path}: {m}")))?
            }
            v @ Value::Struct(_) => super::core::to_json(v)?,
            other => return err(format!("dungeon_create: se esperaba un struct de configuración o la ruta de un .toml/.json, no {}", other.type_name())),
        };
        let mut warnings = vec![];
        let config = parse_config(e, &j, &mut warnings).map_err(|m| RtError::new(format!("dungeon_create: {m}")))?;
        for w in &warnings {
            let msg = format!("dungeon_create: {w}");
            if e.seen_warnings.insert(msg.clone()) {
                eprintln!("aviso: {msg}");
            }
        }
        Ok(Value::Num(e.dungeons.add(DungeonSlot { config, result: None, config_warnings: warnings, grid: None }) as f64))
    });
    r.f("dungeon_generate", |e, _, a| {
        // dungeon_generate(id, [seed], [rng]) -> true. seed: number or text; without seed it is
        // drawn from `rng` (an rng_create stream) or from the game's random numbers (--seed).
        let p = A(a);
        let id = p.num(0)?;
        slot(e, &p, 0)?;
        let seed = match p.opt(1) {
            Some(Value::Num(n)) => *n as i64 as u64,
            Some(Value::Str(s)) => dg::seed_from_text(s),
            Some(other) => return err(format!("dungeon_generate: la semilla debe ser un número o un texto, no {}", other.type_name())),
            None => {
                let st = match p.opt(2) {
                    Some(_) => super::grid::stream(e, p.num(2)?)?,
                    None => &mut e.rng,
                };
                (rand01(st) * (1u64 << 53) as f64) as u64
            }
        };
        let cfg = e.dungeons.list[id as usize].as_ref().expect("checked").config.clone();
        let d = dg::generate(&cfg, seed).map_err(|m| RtError::new(format!("dungeon_generate: {m}")))?;
        for w in &d.warnings {
            let msg = format!("dungeon_generate: {w}");
            if e.seen_warnings.insert(msg.clone()) {
                eprintln!("aviso: {msg}");
            }
        }
        e.dungeons.list[id as usize].as_mut().expect("checked").result = Some(d);
        sync_grid(e, id as usize);
        Ok(Value::Bool(true))
    });
    r.f("dungeon_destroy", |e, _, a| {
        let p = A(a);
        let id = p.num(0)? as usize;
        let g = slot(e, &p, 0)?.grid;
        if let Some(g) = g {
            if let Some(s) = e.ds_grids.list.get_mut(g) {
                *s = None;
            }
        }
        e.dungeons.list[id] = None;
        Ok(Value::Null)
    });
    r.f("dungeon_exists", |e, _, a| Ok(Value::Bool(slot(e, &A(a), 0).is_ok())));
    r.f("dungeon_grid", |e, _, a| {
        // the dungeon's ds_grid (same id every time, refreshed by dungeon_generate; freed by
        // dungeon_destroy). 0 floor, 1 wall, 2 door, 3 corridor.
        let p = A(a);
        generated(e, &p, 0)?;
        let id = p.num(0)? as usize;
        let s = e.dungeons.list[id].as_ref().expect("checked");
        let gid = match s.grid {
            Some(g) if e.ds_grids.list.get(g).is_some_and(|x| x.is_some()) => g,
            _ => sync_grid(e, id),
        };
        Ok(Value::Num(gid as f64))
    });
    r.f("dungeon_cell", |e, _, a| {
        let p = A(a);
        Ok(Value::Num(generated(e, &p, 0)?.cell(p.int(1)?, p.int(2)?) as f64))
    });
    r.f("dungeon_width", |e, _, a| Ok(Value::Num(slot(e, &A(a), 0)?.config.width as f64)));
    r.f("dungeon_height", |e, _, a| Ok(Value::Num(slot(e, &A(a), 0)?.config.height as f64)));
    r.f("dungeon_rooms", |e, _, a| {
        let d = generated(e, &A(a), 0)?;
        Ok(Value::array((0..d.rooms.len()).map(|i| json_val(&d.room_json(i))).collect()))
    });
    r.f("dungeon_room_at", |e, _, a| {
        // room index of a floor cell, -1 for corridors / walls / outside
        let p = A(a);
        let d = generated(e, &p, 0)?;
        let (x, y) = (p.int(1)?, p.int(2)?);
        Ok(Value::Num(if x >= 0 && y >= 0 && (x as usize) < d.w && (y as usize) < d.h { d.room_of[y as usize * d.w + x as usize] as f64 } else { -1.0 }))
    });
    r.f("dungeon_events", |e, _, a| {
        // dungeon_events(id, [type]) -> [{ type, x, y, room, distance, t, on_main_path, rule, data }]
        let p = A(a);
        let d = generated(e, &p, 0)?;
        let only = p.opt(1).map(|v| v.to_string());
        Ok(Value::array(d.events.iter().filter(|ev| only.as_ref().is_none_or(|t| &ev.kind == t)).map(|ev| json_val(&d.event_json(ev))).collect()))
    });
    r.f("dungeon_start", |e, _, a| {
        let d = generated(e, &A(a), 0)?;
        Ok(place(d, d.start, d.start_room))
    });
    r.f("dungeon_exit", |e, _, a| {
        let d = generated(e, &A(a), 0)?;
        Ok(place(d, d.exit, d.exit_room))
    });
    r.f("dungeon_main_path", |e, _, a| {
        let d = generated(e, &A(a), 0)?;
        Ok(Value::array(d.main_path.iter().map(|&(x, y)| xy(x, y)).collect()))
    });
    r.f("dungeon_on_main_path", |e, _, a| {
        let p = A(a);
        let d = generated(e, &p, 0)?;
        let (x, y) = (p.int(1)?, p.int(2)?);
        Ok(Value::Bool(x >= 0 && y >= 0 && (x as usize) < d.w && (y as usize) < d.h && d.on_path[y as usize * d.w + x as usize]))
    });
    r.f("dungeon_distance", |e, _, a| {
        // walking distance in cells from the start (-1: wall / unreachable)
        let p = A(a);
        Ok(Value::Num(generated(e, &p, 0)?.distance(p.int(1)?, p.int(2)?) as f64))
    });
    r.f("dungeon_distance_grid", |e, _, a| {
        // a NEW ds_grid with the distance from the start of every cell (destroy it yourself)
        let d = generated(e, &A(a), 0)?;
        let g = DsGrid::from_nums(d.w, d.h, &d.dist.iter().map(|&v| v as f64).collect::<Vec<_>>());
        Ok(Value::Num(e.ds_grids.add(g) as f64))
    });
    r.f("dungeon_dead_ends", |e, _, a| {
        let d = generated(e, &A(a), 0)?;
        Ok(Value::array(d.dead_ends.iter().map(|&(x, y)| xy(x, y)).collect()))
    });
    r.f("dungeon_edges", |e, _, a| {
        // { delaunay, mst, extra }: room index pairs of each graph layer
        let d = generated(e, &A(a), 0)?;
        Ok(Value::new_struct([(intern("delaunay"), edges_value(&d.delaunay)), (intern("mst"), edges_value(&d.mst)), (intern("extra"), edges_value(&d.extra))]))
    });
    r.f("dungeon_info", |e, _, a| {
        // { seed, width, height, hash, rooms, loops, events, main_path_length, attempts, warnings }
        let p = A(a);
        let s = slot(e, &p, 0)?;
        let Some(d) = s.result.as_ref() else { return err("la mazmorra aún no se ha generado: llama antes a dungeon_generate(id, semilla)") };
        let mut warnings: Vec<Value> = s.config_warnings.iter().map(|w| Value::str(w)).collect();
        warnings.extend(d.warnings.iter().map(|w| Value::str(w)));
        Ok(Value::new_struct([
            (intern("seed"), Value::Num(d.seed as f64)),
            (intern("width"), Value::Num(d.w as f64)),
            (intern("height"), Value::Num(d.h as f64)),
            (intern("hash"), Value::string(format!("{:016x}", d.hash()))),
            (intern("rooms"), Value::Num(d.rooms.len() as f64)),
            (intern("loops"), Value::Num(d.loops() as f64)),
            (intern("events"), Value::Num(d.events.len() as f64)),
            (intern("main_path_length"), Value::Num(d.main_path.len() as f64)),
            (intern("attempts"), Value::Num(d.attempts as f64)),
            (intern("warnings"), Value::array(warnings)),
        ]))
    });
    r.f("dungeon_ascii", |e, _, a| Ok(Value::string(generated(e, &A(a), 0)?.ascii())));
    r.f("dungeon_debug_draw", |e, _, a| {
        // dungeon_debug_draw(id, x, y, cell_px, [{ cells, rooms, delaunay, mst, extra, main_path, distance, events }])
        let p = A(a);
        let (x, y, cs) = (p.num(1)? as f32, p.num(2)? as f32, p.num_or(3, 4.0)? as f32);
        let o = draw_opts(p.opt(4))?;
        let id = p.num(0)? as usize;
        generated(e, &p, 0)?;
        let alpha = e.draw.alpha;
        let d = e.dungeons.list[id].as_ref().and_then(|s| s.result.as_ref()).expect("checked");
        let mut c = CmdCanvas { cmds: &mut e.cmds, alpha };
        dg::draw(d, &mut c, x, y, cs, &o);
        Ok(Value::Null)
    });
    r.f("delaunay", |_, _, a| {
        // delaunay([[x, y], ...]) -> { triangles: [[i, j, k], ...], edges: [[i, j], ...] }
        let pts = points(A(a).val(0)?, "delaunay")?;
        let tris = dg::delaunay(&pts);
        let edges = dg::delaunay_edges(&pts);
        Ok(Value::new_struct([
            (intern("triangles"), Value::array(tris.iter().map(|t| Value::array(t.iter().map(|&i| Value::Num(i as f64)).collect())).collect())),
            (intern("edges"), edges_value(&edges)),
        ]))
    });
    r.f("mst", |_, _, a| {
        // mst(points, [edges]) -> [[i, j], ...]: minimum spanning tree by length; without edges
        // over the Delaunay graph (the same tree as over every pair)
        let p = A(a);
        let pts = points(p.val(0)?, "mst")?;
        let edges: Vec<(usize, usize)> = match p.opt(1) {
            Some(Value::Array(e)) => e
                .borrow()
                .iter()
                .map(|x| match x {
                    Value::Array(q) => {
                        let q = q.borrow();
                        match (q.first().and_then(|v| v.as_num()), q.get(1).and_then(|v| v.as_num())) {
                            (Some(a), Some(b)) if a >= 0.0 && b >= 0.0 => Ok((a as usize, b as usize)),
                            _ => err("mst: cada arista debe ser [i, j]"),
                        }
                    }
                    _ => err("mst: cada arista debe ser [i, j]"),
                })
                .collect::<RtResult<_>>()?,
            Some(_) => return err("mst: las aristas deben ser un array [[i, j], ...]"),
            None => dg::delaunay_edges(&pts),
        };
        Ok(edges_value(&dg::mst(pts.len(), &edges, |i, j| dg::dist(pts[i], pts[j]))))
    });
}

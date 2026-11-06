//! Tile maps and tile sets, GameMaker style: `layer_tilemap_get_id`, `tilemap_get/set`,
//! `tilemap_get_at_pixel`, `tile_get_index`, mirror/flip/rotate bits, `draw_tilemap`...
//!
//! A tile map is a tile layer of the room. Functions take its name (`"Tiles"`), the id that
//! `layer_tilemap_get_id` returns, or `null` for the first tile layer.

use super::{err, me, Reg, A};
use crate::draw::{DrawCmd, Rgba};
use crate::engine::{Engine, Layer, LayerKind};
use crate::tileset::{self, EMPTY, FLIP, INDEX_MASK, MIRROR, ROTATE};
use modscript::{intern, AssetKind, RtResult, Value};

/// Parses a layer's `data`: one line per row of cells, tile data separated by spaces (`.` or
/// `-1` = empty). Every line is a row (an empty line is an empty row); trailing ones are dropped.
pub fn parse_data(data: &str) -> Result<Vec<Vec<i32>>, String> {
    let lines: Vec<&str> = data.lines().collect();
    let end = lines.iter().rposition(|l| !l.trim().is_empty()).map(|e| e + 1).unwrap_or(0);
    lines[..end]
        .iter()
        .map(|l| {
            l.split_whitespace()
                .map(|w| match w {
                    "." => Ok(EMPTY),
                    _ => w.parse::<i64>().map(|v| if v < 0 { EMPTY } else { v as i32 }).map_err(|_| format!("bad tile '{w}' in `data`")),
                })
                .collect()
        })
        .collect()
}

/// The inverse of [`parse_data`] (trailing empty cells and rows dropped).
pub fn write_data(grid: &[Vec<i32>]) -> String {
    let mut rows: Vec<String> = grid
        .iter()
        .map(|row| {
            let n = row.iter().rposition(|t| *t >= 0).map(|i| i + 1).unwrap_or(0);
            if n == 0 {
                return ".".to_string();
            }
            row[..n].iter().map(|t| if *t < 0 { ".".to_string() } else { t.to_string() }).collect::<Vec<_>>().join(" ")
        })
        .collect();
    while rows.last().is_some_and(|r| r == ".") {
        rows.pop();
    }
    rows.join("\n")
}

/// Draws the visible part of tile layer `li` (animated tiles and transforms included).
pub fn draw_tiles(e: &mut Engine, li: usize, ox: f64, oy: f64, view: (f64, f64, f64, f64), col: Rgba) {
    let (vx, vy, vw, vh) = view;
    let t = e.steps as f64 / e.step_rate().max(1.0);
    let mut out = vec![];
    if let LayerKind::Tiles { tileset, set, cw, ch, grid, scale, .. } = &e.layers[li].kind {
        let nframes = e.sprites[*tileset as usize].frames.len() as i32;
        let ts = set.map(|s| &e.tilesets[s as usize]);
        let (cw, ch) = (*cw as f64, *ch as f64);
        // tiles of a scaled layer are drawn bigger (no baked copy of the image)
        let (tw, th) = (e.sprites[*tileset as usize].fw as f64 * scale.0, e.sprites[*tileset as usize].fh as f64 * scale.1);
        let c0 = ((vx - ox) / cw).floor().max(0.0) as usize;
        let r0 = ((vy - oy) / ch).floor().max(0.0) as usize;
        let c1 = ((vx + vw - ox) / cw).ceil().max(0.0) as usize;
        let r1 = ((vy + vh - oy) / ch).ceil().max(0.0) as usize;
        for (r, row) in grid.iter().enumerate().take(r1 + 1).skip(r0) {
            for (c, &d) in row.iter().enumerate().take(c1 + 1).skip(c0) {
                let idx = tileset::index(d);
                if idx < 0 || idx >= nframes {
                    continue;
                }
                let frame = ts.map(|ts| ts.frame_at(idx as u32, t)).unwrap_or(idx as u32);
                let (x, y, sx, sy, angle) = tileset::draw_params(d, ox + c as f64 * cw, oy + r as f64 * ch, tw, th);
                let (sx, sy) = (sx * scale.0, sy * scale.1);
                out.push(DrawCmd::Sprite { spr: *tileset, frame, x: x as f32, y: y as f32, sx: sx as f32, sy: sy as f32, angle: angle as f32, color: col });
            }
        }
    }
    e.cmds.extend(out);
}

/// Index of the tile layer named by argument `i` (name, id, or null = the first one).
fn tm(e: &Engine, a: &A, i: usize) -> RtResult<usize> {
    let li = match a.opt(i) {
        None => e.layers.iter().position(|l| matches!(l.kind, LayerKind::Tiles { .. })),
        Some(Value::Num(n)) => Some(*n as usize).filter(|k| *n >= 0.0 && *k < e.layers.len()),
        Some(v) => {
            let name = v.to_string();
            e.layers.iter().position(|l| l.name == name)
        }
    };
    match li {
        Some(li) if matches!(e.layers[li].kind, LayerKind::Tiles { .. }) => Ok(li),
        Some(_) => err(format!("layer {} is not a tile layer", a.0.get(i).map(|v| v.to_string()).unwrap_or_default())),
        None => err(format!("no tile layer {}", a.0.get(i).map(|v| v.to_string()).unwrap_or_else(|| "in this room".into()))),
    }
}

struct Map<'a> {
    cw: f64,
    ch: f64,
    ox: f64,
    oy: f64,
    grid: &'a mut Vec<Vec<i32>>,
}

fn map(e: &mut Engine, li: usize) -> Map<'_> {
    match &mut e.layers[li].kind {
        LayerKind::Tiles { cw, ch, grid, ox, oy, .. } => Map { cw: *cw as f64, ch: *ch as f64, ox: *ox, oy: *oy, grid },
        _ => unreachable!("checked by tm()"),
    }
}

fn get(grid: &[Vec<i32>], c: i64, r: i64) -> i32 {
    if c < 0 || r < 0 {
        return EMPTY;
    }
    grid.get(r as usize).and_then(|row| row.get(c as usize)).copied().unwrap_or(EMPTY)
}

fn set(grid: &mut Vec<Vec<i32>>, c: i64, r: i64, v: i32) -> bool {
    if c < 0 || r < 0 || c > 100_000 || r > 100_000 {
        return false;
    }
    let (c, r) = (c as usize, r as usize);
    if grid.len() <= r {
        grid.resize(r + 1, vec![]);
    }
    if grid[r].len() <= c {
        grid[r].resize(c + 1, EMPTY);
    }
    grid[r][c] = v;
    true
}

fn data(a: &A, i: usize) -> RtResult<i32> {
    let v = a.num(i)?;
    Ok(if v < 0.0 { EMPTY } else { v as i64 as i32 })
}

fn bit(d: i32, b: i32) -> Value {
    Value::Bool(d >= 0 && d & b != 0)
}

fn with_bit(d: i32, b: i32, on: bool) -> Value {
    if d < 0 {
        return Value::Num(EMPTY as f64);
    }
    Value::Num((if on { d | b } else { d & !b }) as f64)
}

/// Tile set (asset) or sprite (older projects) drawn by a tile layer.
fn tileset_value(e: &Engine, li: usize) -> Value {
    match &e.layers[li].kind {
        LayerKind::Tiles { set: Some(s), .. } => Value::Asset(AssetKind::TileSet, *s),
        LayerKind::Tiles { tileset, .. } => Value::Asset(AssetKind::Sprite, *tileset),
        _ => Value::Null,
    }
}

/// (sprite, tile set) of a tile set or sprite argument.
fn tileset_arg(e: &Engine, a: &A, i: usize) -> RtResult<(u32, Option<u32>)> {
    match a.val(i)? {
        Value::Asset(AssetKind::TileSet, t) => Ok((e.tilesets[*t as usize].sprite_index, Some(*t))),
        Value::Asset(AssetKind::Sprite, s) => Ok((*s, None)),
        other => err(format!("argument #{} must be a tile set, got {}", i + 1, other.type_name())),
    }
}

pub(crate) fn register(r: &mut Reg) {
    // ---------------------------------------------------------------- tile maps (layers)
    r.f("layer_tilemap_get_id", |e, _, a| {
        let name = A(a).text(0)?;
        Ok(Value::Num(e.layers.iter().position(|l| l.name == name && matches!(l.kind, LayerKind::Tiles { .. })).map(|i| i as f64).unwrap_or(-1.0)))
    });
    r.f("layer_tilemap_exists", |e, _, a| {
        let name = A(a).text(0)?;
        Ok(Value::Bool(e.layers.iter().any(|l| l.name == name && matches!(l.kind, LayerKind::Tiles { .. }))))
    });
    r.f("layer_tilemap_create", |e, _, a| {
        // layer_tilemap_create(name, x, y, tileset, width_cells, height_cells, [depth])
        let p = A(a);
        let name = p.text(0)?;
        if e.layers.iter().any(|l| l.name == name) {
            return err(format!("there is already a layer named '{name}'"));
        }
        let (tileset, set) = tileset_arg(e, &p, 3)?;
        let spr = &e.sprites[tileset as usize];
        let (w, h) = (p.num(4)?.max(0.0) as usize, p.num(5)?.max(0.0) as usize);
        let kind = LayerKind::Tiles { tileset, set, cw: spr.fw, ch: spr.fh, grid: vec![vec![EMPTY; w]; h], ox: p.num(1)?, oy: p.num(2)?, scale: (1.0, 1.0) };
        e.layers.push(Layer { name, depth: p.num_or(6, 0.0)?, visible: true, alpha: 1.0, kind });
        Ok(Value::Num((e.layers.len() - 1) as f64))
    });
    r.f("tilemap_get", |e, _, a| {
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let (c, rr) = (p.int(1)?, p.int(2)?);
        Ok(Value::Num(get(map(e, li).grid, c, rr) as f64))
    });
    r.f("tilemap_set", |e, _, a| {
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let (d, c, rr) = (data(&p, 1)?, p.int(2)?, p.int(3)?);
        Ok(Value::Bool(set(map(e, li).grid, c, rr, d)))
    });
    r.f("tilemap_get_at_pixel", |e, _, a| {
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let (x, y) = (p.num(1)?, p.num(2)?);
        let m = map(e, li);
        let (c, rr) = (((x - m.ox) / m.cw).floor() as i64, ((y - m.oy) / m.ch).floor() as i64);
        Ok(Value::Num(get(m.grid, c, rr) as f64))
    });
    r.f("tilemap_set_at_pixel", |e, _, a| {
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let (d, x, y) = (data(&p, 1)?, p.num(2)?, p.num(3)?);
        let m = map(e, li);
        let (c, rr) = (((x - m.ox) / m.cw).floor() as i64, ((y - m.oy) / m.ch).floor() as i64);
        Ok(Value::Bool(set(m.grid, c, rr, d)))
    });
    r.f("tilemap_get_cell_x_at_pixel", |e, _, a| {
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let x = p.num(1)?;
        let m = map(e, li);
        Ok(Value::Num(((x - m.ox) / m.cw).floor()))
    });
    r.f("tilemap_get_cell_y_at_pixel", |e, _, a| {
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let y = p.num(2)?;
        let m = map(e, li);
        Ok(Value::Num(((y - m.oy) / m.ch).floor()))
    });
    r.f("tilemap_get_width", |e, _, a| {
        let li = tm(e, &A(a), 0)?;
        Ok(Value::Num(map(e, li).grid.iter().map(|r| r.len()).max().unwrap_or(0) as f64))
    });
    r.f("tilemap_get_height", |e, _, a| {
        let li = tm(e, &A(a), 0)?;
        Ok(Value::Num(map(e, li).grid.len() as f64))
    });
    r.f("tilemap_set_width", |e, _, a| {
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let w = p.num(1)?.clamp(0.0, 100_000.0) as usize;
        for row in map(e, li).grid.iter_mut() {
            row.resize(w, EMPTY);
        }
        Ok(Value::Null)
    });
    r.f("tilemap_set_height", |e, _, a| {
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let h = p.num(1)?.clamp(0.0, 100_000.0) as usize;
        let m = map(e, li);
        let w = m.grid.iter().map(|r| r.len()).max().unwrap_or(0);
        m.grid.resize(h, vec![EMPTY; w]);
        Ok(Value::Null)
    });
    r.f("tilemap_get_tile_width", |e, _, a| {
        let li = tm(e, &A(a), 0)?;
        Ok(Value::Num(map(e, li).cw))
    });
    r.f("tilemap_get_tile_height", |e, _, a| {
        let li = tm(e, &A(a), 0)?;
        Ok(Value::Num(map(e, li).ch))
    });
    r.f("tilemap_get_x", |e, _, a| {
        let li = tm(e, &A(a), 0)?;
        Ok(Value::Num(map(e, li).ox))
    });
    r.f("tilemap_get_y", |e, _, a| {
        let li = tm(e, &A(a), 0)?;
        Ok(Value::Num(map(e, li).oy))
    });
    r.f("tilemap_x", |e, _, a| {
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let v = p.num(1)?;
        if let LayerKind::Tiles { ox, .. } = &mut e.layers[li].kind {
            *ox = v;
        }
        Ok(Value::Null)
    });
    r.f("tilemap_y", |e, _, a| {
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let v = p.num(1)?;
        if let LayerKind::Tiles { oy, .. } = &mut e.layers[li].kind {
            *oy = v;
        }
        Ok(Value::Null)
    });
    r.f("tilemap_set_scale", |e, _, a| {
        // tilemap_set_scale(tilemap, scale_x, [scale_y = scale_x]): bigger tiles and cells
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let sx = p.num(1)?;
        let sy = p.num_or(2, sx)?;
        if !(sx > 0.0 && sy > 0.0 && sx.is_finite() && sy.is_finite()) {
            return err(format!("tilemap_set_scale: the scale must be positive, got {sx}, {sy}"));
        }
        if let LayerKind::Tiles { cw, ch, scale, .. } = &mut e.layers[li].kind {
            // the unscaled cell stays the same: 16 px cells at scale 3 are 48 px
            let (bw, bh) = (*cw as f64 / scale.0, *ch as f64 / scale.1);
            *cw = (bw * sx).round().max(1.0) as u32;
            *ch = (bh * sy).round().max(1.0) as u32;
            *scale = (sx, sy);
        }
        Ok(Value::Null)
    });
    r.f("tilemap_get_scale_x", |e, _, a| {
        let li = tm(e, &A(a), 0)?;
        Ok(Value::Num(match &e.layers[li].kind {
            LayerKind::Tiles { scale, .. } => scale.0,
            _ => 1.0,
        }))
    });
    r.f("tilemap_get_scale_y", |e, _, a| {
        let li = tm(e, &A(a), 0)?;
        Ok(Value::Num(match &e.layers[li].kind {
            LayerKind::Tiles { scale, .. } => scale.1,
            _ => 1.0,
        }))
    });
    r.f("tilemap_get_tileset", |e, _, a| {
        let li = tm(e, &A(a), 0)?;
        Ok(tileset_value(e, li))
    });
    r.f("tilemap_tileset", |e, _, a| {
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let (new_sprite, new_set) = tileset_arg(e, &p, 1)?;
        if let LayerKind::Tiles { tileset, set, .. } = &mut e.layers[li].kind {
            *tileset = new_sprite;
            *set = new_set;
        }
        Ok(Value::Null)
    });
    r.f("tilemap_clear", |e, _, a| {
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let d = if p.opt(1).is_some() { data(&p, 1)? } else { EMPTY };
        for row in map(e, li).grid.iter_mut() {
            row.iter_mut().for_each(|t| *t = d);
        }
        Ok(Value::Null)
    });
    r.f("tilemap_autotile", |e, _, a| {
        // tilemap_autotile(tilemap, autotile name or number, cell_x, cell_y, [on = true])
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let Some(s) = (match &e.layers[li].kind {
            LayerKind::Tiles { set, .. } => *set,
            _ => None,
        }) else {
            return err("the layer's tiles come from a sprite, not a tile set with auto tiles");
        };
        let ts = &e.tilesets[s as usize];
        let which = p.val(1)?;
        let auto = match which {
            Value::Num(n) => ts.autotiles.get(*n as usize),
            v => ts.autotiles.iter().find(|t| t.name == v.to_string()),
        };
        let Some(auto) = auto else { return err(format!("tile set '{}' has no auto tile {which}", ts.name)) };
        let tiles = auto.tiles.clone();
        let (c, rr, on) = (p.int(2)?, p.int(3)?, p.bool_or(4, true));
        tileset::autotile_apply(map(e, li).grid, &tiles, c as i32, rr as i32, on);
        Ok(Value::Null)
    });
    r.f("tile_meeting", |e, cx, a| {
        // tile_meeting(x, y, [tilemap]): would the calling instance touch a tile at (x, y)?
        let p = A(a);
        let i = me(e, cx)?;
        let li = tm(e, &p, 2)?;
        let Some(b) = e.bbox_at(i, p.num(0)?, p.num(1)?) else { return Ok(Value::Bool(false)) };
        let m = map(e, li);
        let c0 = ((b[0] - m.ox) / m.cw).floor() as i64;
        let c1 = ((b[2] - m.ox) / m.cw).ceil() as i64 - 1;
        let r0 = ((b[1] - m.oy) / m.ch).floor() as i64;
        let r1 = ((b[3] - m.oy) / m.ch).ceil() as i64 - 1;
        for rr in r0..=r1 {
            for c in c0..=c1 {
                if get(m.grid, c, rr) >= 0 {
                    return Ok(Value::Bool(true));
                }
            }
        }
        Ok(Value::Bool(false))
    });

    // ---------------------------------------------------------------- tile data
    r.f("tile_get_index", |_, _, a| Ok(Value::Num(tileset::index(data(&A(a), 0)?) as f64)));
    r.f("tile_set_index", |_, _, a| {
        let p = A(a);
        let d = data(&p, 0)?;
        let i = (p.num(1)?.max(0.0) as i32) & INDEX_MASK;
        Ok(Value::Num((if d < 0 { i } else { (d & !INDEX_MASK) | i }) as f64))
    });
    r.f("tile_get_empty", |_, _, a| Ok(Value::Bool(data(&A(a), 0)? < 0)));
    r.f("tile_set_empty", |_, _, _| Ok(Value::Num(EMPTY as f64)));
    r.f("tile_get_mirror", |_, _, a| Ok(bit(data(&A(a), 0)?, MIRROR)));
    r.f("tile_get_flip", |_, _, a| Ok(bit(data(&A(a), 0)?, FLIP)));
    r.f("tile_get_rotate", |_, _, a| Ok(bit(data(&A(a), 0)?, ROTATE)));
    r.f("tile_set_mirror", |_, _, a| Ok(with_bit(data(&A(a), 0)?, MIRROR, A(a).bool_or(1, true))));
    r.f("tile_set_flip", |_, _, a| Ok(with_bit(data(&A(a), 0)?, FLIP, A(a).bool_or(1, true))));
    r.f("tile_set_rotate", |_, _, a| Ok(with_bit(data(&A(a), 0)?, ROTATE, A(a).bool_or(1, true))));

    // ---------------------------------------------------------------- tile sets
    r.f("tileset_get_info", |e, _, a| {
        let t = A(a).asset(0, AssetKind::TileSet)?;
        let ts = &e.tilesets[t as usize];
        let anims = ts
            .anims
            .iter()
            .map(|an| Value::new_struct([(intern("name"), Value::str(&an.name)), (intern("frames"), Value::array(an.frames.iter().map(|f| Value::Num(*f as f64)).collect()))]))
            .collect();
        let autos = ts.autotiles.iter().map(|au| Value::new_struct([(intern("name"), Value::str(&au.name)), (intern("tiles"), Value::array(au.tiles.iter().map(|f| Value::Num(*f as f64)).collect()))])).collect();
        Ok(Value::new_struct([
            (intern("name"), Value::str(&ts.name)),
            (intern("tile_width"), Value::Num(ts.tile_w as f64)),
            (intern("tile_height"), Value::Num(ts.tile_h as f64)),
            (intern("tile_count"), Value::Num(ts.count() as f64)),
            (intern("columns"), Value::Num(ts.columns as f64)),
            (intern("rows"), Value::Num(ts.rows as f64)),
            (intern("fps"), Value::Num(ts.fps)),
            (intern("animations"), Value::array(anims)),
            (intern("autotiles"), Value::array(autos)),
        ]))
    });
    r.f("draw_tile", |e, _, a| {
        // draw_tile(tileset, tile_data, frame, x, y): frame of an animated tile (-1 = current)
        let p = A(a);
        let (spr, set) = tileset_arg(e, &p, 0)?;
        let d = data(&p, 1)?;
        let idx = tileset::index(d);
        let s = &e.sprites[spr as usize];
        if idx < 0 || idx as usize >= s.frames.len() {
            return Ok(Value::Null);
        }
        let (tw, th) = (s.fw as f64, s.fh as f64);
        let fr = p.num_or(2, -1.0)?;
        let frame = match set.map(|t| &e.tilesets[t as usize]) {
            Some(ts) if fr < 0.0 => ts.frame_at(idx as u32, e.steps as f64 / e.step_rate().max(1.0)),
            Some(ts) => match ts.anim_of.get(&(idx as u32)).map(|&k| &ts.anims[k].frames) {
                Some(f) if !f.is_empty() => f[(fr as usize) % f.len()],
                _ => idx as u32,
            },
            None => idx as u32,
        };
        let (x, y, sx, sy, angle) = tileset::draw_params(d, p.num(3)?, p.num(4)?, tw, th);
        let color = Rgba::from_rgb(0xFFFFFF, e.draw.alpha);
        e.cmds.push(DrawCmd::Sprite { spr, frame, x: x as f32, y: y as f32, sx: sx as f32, sy: sy as f32, angle: angle as f32, color });
        Ok(Value::Null)
    });
    r.f("draw_tilemap", |e, _, a| {
        // draw_tilemap(tilemap, x, y): draws a tile layer somewhere else (e.g. a minimap)
        let p = A(a);
        let li = tm(e, &p, 0)?;
        let (x, y) = (p.num(1)?, p.num(2)?);
        let color = Rgba::from_rgb(0xFFFFFF, e.draw.alpha);
        let (cw, ch, rows, cols) = match &e.layers[li].kind {
            LayerKind::Tiles { cw, ch, grid, .. } => (*cw as f64, *ch as f64, grid.len() as f64, grid.iter().map(|r| r.len()).max().unwrap_or(0) as f64),
            _ => return Ok(Value::Null),
        };
        draw_tiles(e, li, x, y, (x, y, cols * cw, rows * ch), color);
        Ok(Value::Null)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_roundtrip() {
        let g = parse_data("\n 0 1 . 3\n\n-1 268435461\n\n").unwrap();
        assert_eq!(g, vec![vec![], vec![0, 1, -1, 3], vec![], vec![-1, 268_435_461]], "leading empty rows count");
        assert_eq!(write_data(&g), ".\n0 1 . 3\n.\n. 268435461");
        assert_eq!(parse_data(&write_data(&g)).unwrap(), vec![vec![-1], vec![0, 1, -1, 3], vec![-1], vec![-1, 268_435_461]]);
        assert!(parse_data("1 x").is_err());
    }
}

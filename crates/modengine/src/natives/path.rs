//! Paths: following them (path_start...), reading them and building them at runtime.

use super::{me, Reg, A};
use crate::draw::DrawCmd;
use crate::engine::Engine;
use crate::path::{EndAction, PathDef, PathPoint, PathToml};
use modscript::{AssetKind, RtError, RtResult, Value};

fn err<T>(msg: impl Into<String>) -> RtResult<T> {
    Err(RtError::new(msg))
}

fn path_arg(e: &Engine, a: &A, i: usize) -> RtResult<usize> {
    let p = a.asset(i, AssetKind::Path)? as usize;
    match e.paths.get(p) {
        Some(d) if d.alive => Ok(p),
        _ => err("the path was deleted"),
    }
}

fn point_arg(e: &Engine, p: usize, a: &A, i: usize) -> RtResult<usize> {
    let n = a.int(i)?;
    let len = e.paths[p].data.points.len();
    if n < 0 || n as usize >= len {
        return err(format!("point {n} does not exist (the path has {len})"));
    }
    Ok(n as usize)
}

/// Center of the path's points (used by mirror, flip, rotate and rescale).
fn center(t: &PathToml) -> (f64, f64) {
    let Some(f) = t.points.first() else { return (0.0, 0.0) };
    let b = t.points.iter().fold((f.x, f.y, f.x, f.y), |b, p| (b.0.min(p.x), b.1.min(p.y), b.2.max(p.x), b.3.max(p.y)));
    ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0)
}

/// Applies `f` to a path's data and rebuilds its polyline.
fn edit(e: &mut Engine, p: usize, f: impl FnOnce(&mut PathToml)) -> RtResult<Value> {
    f(&mut e.paths[p].data);
    e.paths[p].rebuild();
    Ok(Value::Null)
}

fn new_path(e: &mut Engine, data: PathToml) -> Value {
    let i = e.paths.len();
    e.paths.push(PathDef::new(&format!("__path{i}"), data));
    Value::Asset(AssetKind::Path, i as u32)
}

pub(super) fn register(r: &mut Reg) {
    // ---------------------------------------------------------------- following
    r.f("path_start", |e, cx, a| {
        let p = A(a);
        let i = me(e, cx)?;
        let pi = path_arg(e, &p, 0)?;
        let speed = p.num(1)?;
        let end = EndAction::from_num(p.num_or(2, 0.0)?);
        let absolute = p.bool_or(3, false);
        let path = &e.paths[pi];
        crate::path::start(&mut e.insts[i], pi as u32, path, speed, end, absolute);
        Ok(Value::Null)
    });
    r.f("path_end", |e, cx, _| {
        let i = me(e, cx)?;
        e.insts[i].path.index = None;
        Ok(Value::Null)
    });

    // ---------------------------------------------------------------- reading
    r.f("path_exists", |e, _, a| Ok(Value::Bool(matches!(A(a).val(0)?, Value::Asset(AssetKind::Path, p) if e.paths.get(*p as usize).is_some_and(|d| d.alive)))));
    r.f("path_get_name", |e, _, a| {
        let p = path_arg(e, &A(a), 0)?;
        Ok(Value::str(&e.paths[p].name))
    });
    r.f("path_get_length", |e, _, a| Ok(Value::Num(e.paths[path_arg(e, &A(a), 0)?].length())));
    r.f("path_get_x", |e, _, a| Ok(Value::Num(e.paths[path_arg(e, &A(a), 0)?].at(A(a).num(1)?).0)));
    r.f("path_get_y", |e, _, a| Ok(Value::Num(e.paths[path_arg(e, &A(a), 0)?].at(A(a).num(1)?).1)));
    r.f("path_get_speed", |e, _, a| Ok(Value::Num(e.paths[path_arg(e, &A(a), 0)?].at(A(a).num(1)?).2)));
    r.f("path_get_number", |e, _, a| Ok(Value::Num(e.paths[path_arg(e, &A(a), 0)?].data.points.len() as f64)));
    r.f("path_get_point_x", |e, _, a| {
        let p = path_arg(e, &A(a), 0)?;
        Ok(Value::Num(e.paths[p].data.points[point_arg(e, p, &A(a), 1)?].x))
    });
    r.f("path_get_point_y", |e, _, a| {
        let p = path_arg(e, &A(a), 0)?;
        Ok(Value::Num(e.paths[p].data.points[point_arg(e, p, &A(a), 1)?].y))
    });
    r.f("path_get_point_speed", |e, _, a| {
        let p = path_arg(e, &A(a), 0)?;
        Ok(Value::Num(e.paths[p].data.points[point_arg(e, p, &A(a), 1)?].speed))
    });
    r.f("path_get_closed", |e, _, a| Ok(Value::Bool(e.paths[path_arg(e, &A(a), 0)?].data.closed)));
    r.f("path_get_kind", |e, _, a| Ok(Value::Num(if e.paths[path_arg(e, &A(a), 0)?].data.smooth { 1.0 } else { 0.0 })));
    r.f("path_get_precision", |e, _, a| Ok(Value::Num(e.paths[path_arg(e, &A(a), 0)?].data.precision as f64)));

    // ---------------------------------------------------------------- building
    r.f("path_add", |e, _, _| Ok(new_path(e, PathToml::default())));
    r.f("path_duplicate", |e, _, a| {
        let p = path_arg(e, &A(a), 0)?;
        let data = e.paths[p].data.clone();
        Ok(new_path(e, data))
    });
    r.f("path_delete", |e, _, a| {
        let p = path_arg(e, &A(a), 0)?;
        e.paths[p].alive = false;
        for inst in e.insts.iter_mut().filter(|i| i.path.index == Some(p as u32)) {
            inst.path.index = None;
        }
        Ok(Value::Null)
    });
    r.f("path_assign", |e, _, a| {
        let (t, s) = (path_arg(e, &A(a), 0)?, path_arg(e, &A(a), 1)?);
        let data = e.paths[s].data.clone();
        edit(e, t, |d| *d = data)
    });
    r.f("path_append", |e, _, a| {
        let (t, s) = (path_arg(e, &A(a), 0)?, path_arg(e, &A(a), 1)?);
        let pts = e.paths[s].data.points.clone();
        edit(e, t, |d| d.points.extend(pts))
    });
    r.f("path_add_point", |e, _, a| {
        let p = A(a);
        let pi = path_arg(e, &p, 0)?;
        let pt = PathPoint { x: p.num(1)?, y: p.num(2)?, speed: p.num_or(3, 100.0)? };
        edit(e, pi, |d| d.points.push(pt))
    });
    r.f("path_insert_point", |e, _, a| {
        let p = A(a);
        let pi = path_arg(e, &p, 0)?;
        let n = (p.int(1)?.max(0) as usize).min(e.paths[pi].data.points.len());
        let pt = PathPoint { x: p.num(2)?, y: p.num(3)?, speed: p.num_or(4, 100.0)? };
        edit(e, pi, |d| d.points.insert(n, pt))
    });
    r.f("path_change_point", |e, _, a| {
        let p = A(a);
        let pi = path_arg(e, &p, 0)?;
        let n = point_arg(e, pi, &p, 1)?;
        let speed = p.num_or(4, e.paths[pi].data.points[n].speed)?;
        let pt = PathPoint { x: p.num(2)?, y: p.num(3)?, speed };
        edit(e, pi, |d| d.points[n] = pt)
    });
    r.f("path_delete_point", |e, _, a| {
        let pi = path_arg(e, &A(a), 0)?;
        let n = point_arg(e, pi, &A(a), 1)?;
        edit(e, pi, |d| {
            d.points.remove(n);
        })
    });
    r.f("path_clear_points", |e, _, a| {
        let pi = path_arg(e, &A(a), 0)?;
        edit(e, pi, |d| d.points.clear())
    });
    r.f("path_set_closed", |e, _, a| {
        let pi = path_arg(e, &A(a), 0)?;
        let v = A(a).bool_or(1, true);
        edit(e, pi, |d| d.closed = v)
    });
    r.f("path_set_kind", |e, _, a| {
        let pi = path_arg(e, &A(a), 0)?;
        let v = A(a).num(1)? != 0.0;
        edit(e, pi, |d| d.smooth = v)
    });
    r.f("path_set_precision", |e, _, a| {
        let pi = path_arg(e, &A(a), 0)?;
        let v = A(a).int(1)?.clamp(1, 8) as u32;
        edit(e, pi, |d| d.precision = v)
    });
    r.f("path_reverse", |e, _, a| {
        let pi = path_arg(e, &A(a), 0)?;
        edit(e, pi, |d| d.points.reverse())
    });
    r.f("path_shift", |e, _, a| {
        let pi = path_arg(e, &A(a), 0)?;
        let (dx, dy) = (A(a).num(1)?, A(a).num(2)?);
        edit(e, pi, |d| {
            for p in &mut d.points {
                p.x += dx;
                p.y += dy;
            }
        })
    });
    r.f("path_mirror", |e, _, a| {
        let pi = path_arg(e, &A(a), 0)?;
        edit(e, pi, |d| {
            let (cx, _) = center(d);
            for p in &mut d.points {
                p.x = 2.0 * cx - p.x;
            }
        })
    });
    r.f("path_flip", |e, _, a| {
        let pi = path_arg(e, &A(a), 0)?;
        edit(e, pi, |d| {
            let (_, cy) = center(d);
            for p in &mut d.points {
                p.y = 2.0 * cy - p.y;
            }
        })
    });
    r.f("path_rescale", |e, _, a| {
        let pi = path_arg(e, &A(a), 0)?;
        let (sx, sy) = (A(a).num(1)?, A(a).num(2)?);
        edit(e, pi, |d| {
            let (cx, cy) = center(d);
            for p in &mut d.points {
                p.x = cx + (p.x - cx) * sx;
                p.y = cy + (p.y - cy) * sy;
            }
        })
    });
    r.f("path_rotate", |e, _, a| {
        let pi = path_arg(e, &A(a), 0)?;
        let ang = A(a).num(1)?.to_radians();
        edit(e, pi, |d| {
            let (cx, cy) = center(d);
            let (c, s) = (ang.cos(), ang.sin());
            for p in &mut d.points {
                let (dx, dy) = (p.x - cx, p.y - cy);
                p.x = cx + dx * c + dy * s;
                p.y = cy - dx * s + dy * c;
            }
        })
    });

    // ---------------------------------------------------------------- drawing
    r.f("draw_path", |e, _, a| {
        let p = A(a);
        let pi = path_arg(e, &p, 0)?;
        let (x, y) = (p.num(1)?, p.num(2)?);
        let absolute = p.bool_or(3, false);
        let path = &e.paths[pi];
        let (x0, y0, _) = path.at(0.0);
        let (ox, oy) = if absolute { (0.0, 0.0) } else { (x - x0, y - y0) };
        let color = e.draw.rgba();
        let cmds: Vec<DrawCmd> = path
            .poly
            .windows(2)
            .map(|w| DrawCmd::Line { x1: (w[0].0 + ox) as f32, y1: (w[0].1 + oy) as f32, x2: (w[1].0 + ox) as f32, y2: (w[1].1 + oy) as f32, width: 1.0, color })
            .collect();
        e.cmds.extend(cmds);
        Ok(Value::Null)
    });
}

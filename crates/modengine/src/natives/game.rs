//! Instances, collisions, motion, rooms, camera, input, audio, events and saving.

use super::core::{from_json, to_json};
use super::{me, Reg, A};
use crate::engine::{AudioCmd, Engine, LayerKind};
use crate::object::Ev;
use modscript::{AssetKind, RtError, RtResult, Value};

fn err<T>(msg: impl Into<String>) -> RtResult<T> {
    Err(RtError::new(msg))
}

fn inst_or_null(id: Option<u32>) -> RtResult<Value> {
    Ok(id.map(Value::Inst).unwrap_or(Value::Null))
}

fn key_arg(a: &A, i: usize) -> RtResult<usize> {
    let k = a.num(i)?;
    if k < 0.0 || k as usize >= crate::input::KEY_NAMES.len() {
        return err(format!("{k} is not a key (use key.left, key.a...)"));
    }
    Ok(k as usize)
}

fn mouse_arg(a: &A, i: usize) -> RtResult<usize> {
    let b = a.num_or(i, 0.0)?;
    if !(0.0..3.0).contains(&b) {
        return err("mouse button must be mouse.left, mouse.right or mouse.middle");
    }
    Ok(b as usize)
}

/// Moves the caller step by step in `dir` until touching `target` (or a solid). Returns distance moved.
fn move_contact(e: &mut Engine, i: usize, dir: f64, max: f64, target: Option<&Value>) -> f64 {
    let (dx, dy) = (dir.to_radians().cos(), -dir.to_radians().sin());
    let mut moved = 0.0;
    let blocked = |e: &Engine, x: f64, y: f64| match target {
        Some(t) => !e.hits_at(i, x, y, t, true).is_empty(),
        None => e.solid_at(i, x, y),
    };
    while moved < max {
        let step = (max - moved).min(1.0);
        let (nx, ny) = (e.insts[i].x + dx * step, e.insts[i].y + dy * step);
        if blocked(e, nx, ny) {
            break;
        }
        e.insts[i].x = nx;
        e.insts[i].y = ny;
        moved += step;
    }
    moved
}

pub(super) fn register(r: &mut Reg) {
    // ---------------------------------------------------------------- instances
    r.f("spawn", |e, cx, a| {
        let p = A(a);
        let obj = p.asset(0, AssetKind::Object)?;
        let (x, y) = (p.num(1)?, p.num(2)?);
        let depth = p.num_or(3, 0.0)?;
        let id = e.create_instance(obj, x, y, depth);
        let vars = match p.opt(4) {
            Some(Value::Struct(s)) => s.borrow().fields.iter().map(|(k, v)| (*k, v.clone())).collect(),
            Some(other) => return err(format!("spawn(): argument #5 must be a struct of variables, got {}", other.type_name())),
            None => vec![],
        };
        e.init_instance(id, &vars, None, cx.self_.clone());
        Ok(Value::Inst(id))
    });
    r.f("destroy", |e, cx, a| {
        let targets = match A(a).opt(0) {
            Some(v) => e.resolve(v),
            None => vec![cx.self_.inst().ok_or_else(|| RtError::new("destroy() without arguments must be called by an instance"))?],
        };
        let run = A(a).bool_or(1, true);
        for id in targets {
            e.destroy(id, run);
        }
        Ok(Value::Null)
    });
    r.f("exists", |e, _, a| Ok(Value::Bool(match A(a).val(0)? {
        Value::Null => false,
        v => !e.resolve(v).is_empty(),
    })));
    r.f("count", |e, _, a| Ok(Value::Num(e.resolve(&A(a).target(0)?).len() as f64)));
    r.f("find", |e, _, a| {
        let list = e.resolve(&A(a).target(0)?);
        inst_or_null(list.get(A(a).num_or(1, 0.0)? as usize).copied())
    });
    r.f("instances", |e, _, a| Ok(Value::array(e.resolve(&A(a).target(0)?).into_iter().map(Value::Inst).collect())));
    for (name, far) in [("nearest", false), ("furthest", true)] {
        let f: super::NativeFn = if far {
            |e, cx, a| pick(e, cx, a, true)
        } else {
            |e, cx, a| pick(e, cx, a, false)
        };
        r.f(name, f);
    }
    r.f("inst_at", |e, _, a| {
        let (x, y) = (A(a).num(0)?, A(a).num(1)?);
        let t = A(a).target(2)?;
        inst_or_null(e.hits_rect([x, y, x + 0.001, y + 0.001], &t, None, true).first().copied())
    });
    r.f("is_a", |e, _, a| {
        let obj = A(a).asset(1, AssetKind::Object)?;
        Ok(Value::Bool(match A(a).val(0)? {
            Value::Inst(id) => e.idx(*id).map(|i| e.is_a(e.insts[i].object, obj)).unwrap_or(false),
            Value::Asset(AssetKind::Object, o) => e.is_a(*o, obj),
            _ => false,
        }))
    });
    r.f("asset_name", |e, _, a| match A(a).val(0)? {
        Value::Asset(k, i) => Ok(Value::string(e.asset_name(*k, *i))),
        Value::Inst(id) => match e.idx(*id) {
            Some(i) => Ok(Value::string(e.asset_name(AssetKind::Object, e.insts[i].object))),
            None => Ok(Value::Null),
        },
        other => err(format!("asset_name() of {}", other.type_name())),
    });

    // ---------------------------------------------------------------- collisions
    r.f("collides_at", |e, cx, a| {
        let i = me(e, cx)?;
        Ok(Value::Bool(!e.hits_at(i, A(a).num(0)?, A(a).num(1)?, &A(a).target(2)?, true).is_empty()))
    });
    r.f("collider_at", |e, cx, a| {
        let i = me(e, cx)?;
        inst_or_null(e.hits_at(i, A(a).num(0)?, A(a).num(1)?, &A(a).target(2)?, true).first().copied())
    });
    r.f("colliders_at", |e, cx, a| {
        let i = me(e, cx)?;
        let hits = e.hits_at(i, A(a).num(0)?, A(a).num(1)?, &A(a).target(2)?, false);
        Ok(Value::array(hits.into_iter().map(Value::Inst).collect()))
    });
    r.f("free_at", |e, cx, a| {
        let i = me(e, cx)?;
        Ok(Value::Bool(!e.solid_at(i, A(a).num(0)?, A(a).num(1)?)))
    });
    r.f("rect_hits", |e, _, a| {
        let p = A(a);
        let (x1, y1, x2, y2) = (p.num(0)?, p.num(1)?, p.num(2)?, p.num(3)?);
        let rect = [x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)];
        inst_or_null(e.hits_rect(rect, &p.target(4)?, None, true).first().copied())
    });
    r.f("circle_hits", |e, _, a| {
        let p = A(a);
        let (cx_, cy, rad) = (p.num(0)?, p.num(1)?, p.num(2)?);
        let t = p.target(3)?;
        let cand = e.hits_rect([cx_ - rad, cy - rad, cx_ + rad, cy + rad], &t, None, false);
        for id in cand {
            if let Some(b) = e.idx(id).and_then(|i| e.bbox(i)) {
                let (nx, ny) = (cx_.clamp(b[0], b[2]), cy.clamp(b[1], b[3]));
                if (nx - cx_).hypot(ny - cy) <= rad {
                    return Ok(Value::Inst(id));
                }
            }
        }
        Ok(Value::Null)
    });
    r.f("line_hits", |e, _, a| {
        let p = A(a);
        let (x1, y1, x2, y2) = (p.num(0)?, p.num(1)?, p.num(2)?, p.num(3)?);
        let t = p.target(4)?;
        let cand = e.hits_rect([x1.min(x2), y1.min(y2), x1.max(x2) + 0.001, y1.max(y2) + 0.001], &t, None, false);
        for id in cand {
            if let Some(b) = e.idx(id).and_then(|i| e.bbox(i)) {
                if segment_hits_box(x1, y1, x2, y2, b) {
                    return Ok(Value::Inst(id));
                }
            }
        }
        Ok(Value::Null)
    });
    r.f("move_contact", |e, cx, a| {
        let i = me(e, cx)?;
        let t = A(a).opt(2).cloned();
        let d = move_contact(e, i, A(a).num(0)?, A(a).num_or(1, 1000.0)?, t.as_ref());
        Ok(Value::Num(d))
    });

    // ---------------------------------------------------------------- motion
    r.f("motion_set", |e, cx, a| {
        let i = me(e, cx)?;
        let inst = &mut e.insts[i];
        inst.dir = A(a).num(0)?.rem_euclid(360.0);
        inst.set_speed(A(a).num(1)?);
        Ok(Value::Null)
    });
    r.f("motion_add", |e, cx, a| {
        let i = me(e, cx)?;
        let (d, s) = (A(a).num(0)?.to_radians(), A(a).num(1)?);
        let inst = &mut e.insts[i];
        let (h, v) = (inst.hspeed + s * d.cos(), inst.vspeed - s * d.sin());
        inst.set_hv(h, v);
        Ok(Value::Null)
    });
    r.f("move_towards", |e, cx, a| {
        let i = me(e, cx)?;
        let (tx, ty, s) = (A(a).num(0)?, A(a).num(1)?, A(a).num(2)?);
        let inst = &mut e.insts[i];
        inst.dir = (-(ty - inst.y)).atan2(tx - inst.x).to_degrees().rem_euclid(360.0);
        inst.set_speed(s);
        Ok(Value::Null)
    });
    r.f("step_towards", |e, cx, a| {
        let i = me(e, cx)?;
        let (tx, ty, s) = (A(a).num(0)?, A(a).num(1)?, A(a).num(2)?);
        let inst = &mut e.insts[i];
        let (dx, dy) = (tx - inst.x, ty - inst.y);
        let d = dx.hypot(dy);
        if d <= s || d == 0.0 {
            inst.x = tx;
            inst.y = ty;
            return Ok(Value::Bool(true));
        }
        inst.x += dx / d * s;
        inst.y += dy / d * s;
        Ok(Value::Bool(false))
    });
    r.f("move_wrap", |e, cx, a| {
        let i = me(e, cx)?;
        let (hor, ver, margin) = (A(a).bool_or(0, true), A(a).bool_or(1, true), A(a).num_or(2, 0.0)?);
        let (w, h) = (e.room_w, e.room_h);
        let inst = &mut e.insts[i];
        if hor {
            if inst.x < -margin {
                inst.x += w + margin * 2.0;
            } else if inst.x > w + margin {
                inst.x -= w + margin * 2.0;
            }
        }
        if ver {
            if inst.y < -margin {
                inst.y += h + margin * 2.0;
            } else if inst.y > h + margin {
                inst.y -= h + margin * 2.0;
            }
        }
        Ok(Value::Null)
    });
    r.f("distance_to", |e, cx, a| {
        let i = me(e, cx)?;
        let ids = e.resolve(&A(a).target(0)?);
        let (x, y) = (e.insts[i].x, e.insts[i].y);
        let me_id = e.insts[i].id;
        let d = ids
            .iter()
            .filter(|id| **id != me_id)
            .filter_map(|id| e.idx(*id))
            .map(|j| (e.insts[j].x - x).hypot(e.insts[j].y - y))
            .fold(f64::INFINITY, f64::min);
        Ok(Value::Num(d))
    });

    // ---------------------------------------------------------------- events
    r.f("inherited", |e, cx, _| {
        e.run_inherited(cx)?;
        Ok(Value::Null)
    });
    r.f("event_user", |e, cx, a| {
        let n = A(a).num(0)?;
        if !(0.0..16.0).contains(&n) {
            return err("user events go from 0 to 15");
        }
        let target = match A(a).opt(1) {
            Some(t) => e.resolve(t),
            None => vec![cx.self_.inst().ok_or_else(|| RtError::new("event_user() needs an instance"))?],
        };
        for id in target {
            e.run_event(id, Ev::User(n as u8), cx.other.clone());
        }
        Ok(Value::Null)
    });

    // ---------------------------------------------------------------- rooms
    r.f("goto_room", |e, _, a| {
        e.pending_room = Some(A(a).asset(0, AssetKind::Room)? as usize);
        Ok(Value::Null)
    });
    r.f("next_room", |e, _, _| {
        let pos = e.room_order.iter().position(|r| *r == e.room).unwrap_or(0);
        match e.room_order.get(pos + 1) {
            Some(r) => {
                e.pending_room = Some(*r);
                Ok(Value::Bool(true))
            }
            None => Ok(Value::Bool(false)),
        }
    });
    r.f("prev_room", |e, _, _| {
        let pos = e.room_order.iter().position(|r| *r == e.room).unwrap_or(0);
        if pos == 0 {
            return Ok(Value::Bool(false));
        }
        e.pending_room = Some(e.room_order[pos - 1]);
        Ok(Value::Bool(true))
    });
    r.f("restart_room", |e, _, _| {
        e.pending_room = Some(e.room);
        Ok(Value::Null)
    });
    r.f("restart_game", |e, _, _| {
        e.pending_restart = true;
        Ok(Value::Null)
    });
    r.f("room_name", |e, _, _| Ok(Value::string(e.rooms[e.room].name.clone())));
    r.f("quit", |e, _, _| {
        e.quit = true;
        Ok(Value::Null)
    });

    // ---------------------------------------------------------------- camera
    r.f("camera_set", |e, _, a| {
        e.cam.follow = None;
        e.cam.x = A(a).num(0)?;
        e.cam.y = A(a).num(1)?;
        Ok(Value::Null)
    });
    r.f("camera_follow", |e, _, a| {
        e.cam.follow = A(a).opt(0).cloned();
        e.cam.smooth = A(a).num_or(1, e.cam.smooth)?.clamp(0.01, 1.0);
        Ok(Value::Null)
    });
    r.f("camera_shake", |e, _, a| {
        e.cam.shake = A(a).num(0)?;
        e.cam.shake_steps = A(a).num_or(1, 10.0)?.max(0.0) as u32;
        Ok(Value::Null)
    });
    r.f("camera_x", |e, _, _| Ok(Value::Num(e.cam.x)));
    r.f("camera_y", |e, _, _| Ok(Value::Num(e.cam.y)));
    r.f("camera_w", |e, _, _| Ok(Value::Num(e.cam.w)));
    r.f("camera_h", |e, _, _| Ok(Value::Num(e.cam.h)));

    // ---------------------------------------------------------------- input
    r.f("key_down", |e, _, a| Ok(Value::Bool(e.input.down[key_arg(&A(a), 0)?])));
    r.f("key_pressed", |e, _, a| Ok(Value::Bool(e.input.pressed[key_arg(&A(a), 0)?])));
    r.f("key_released", |e, _, a| Ok(Value::Bool(e.input.released[key_arg(&A(a), 0)?])));
    r.f("key_any", |e, _, _| Ok(Value::Bool(e.input.any_down())));
    r.f("key_any_pressed", |e, _, _| Ok(Value::Bool(e.input.any_pressed())));
    r.f("input_axis", |e, _, a| {
        // input_axis(negative_keys..., positive_keys...) — pass arrays to allow several keys
        let side = |v: &Value| -> RtResult<bool> {
            let list = match v {
                Value::Array(arr) => arr.borrow().clone(),
                other => vec![other.clone()],
            };
            for k in list {
                let k = k.as_num().ok_or_else(|| RtError::new("input_axis() needs keys"))? as usize;
                if e.input.down.get(k).copied().unwrap_or(false) {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        let neg = side(A(a).val(0)?)?;
        let pos = side(A(a).val(1)?)?;
        Ok(Value::Num(pos as i32 as f64 - neg as i32 as f64))
    });
    r.f("mouse_down", |e, _, a| Ok(Value::Bool(e.input.mouse_down[mouse_arg(&A(a), 0)?])));
    r.f("mouse_pressed", |e, _, a| Ok(Value::Bool(e.input.mouse_pressed[mouse_arg(&A(a), 0)?])));
    r.f("mouse_released", |e, _, a| Ok(Value::Bool(e.input.mouse_released[mouse_arg(&A(a), 0)?])));
    r.f("mouse_wheel", |e, _, _| Ok(Value::Num(e.input.wheel)));
    r.f("gui_mouse_x", |e, _, _| Ok(Value::Num(e.input.mouse_x)));
    r.f("gui_mouse_y", |e, _, _| Ok(Value::Num(e.input.mouse_y)));

    // ---------------------------------------------------------------- audio
    r.f("play_sound", |e, _, a| {
        let snd = A(a).asset(0, AssetKind::Sound)?;
        let looped = A(a).bool_or(1, false);
        let volume = A(a).num_or(2, 1.0)? * e.sounds[snd as usize].volume;
        e.audio.push(AudioCmd::Play { snd, looped, volume });
        Ok(Value::Null)
    });
    r.f("stop_sound", |e, _, a| {
        e.audio.push(AudioCmd::Stop(A(a).asset(0, AssetKind::Sound)?));
        Ok(Value::Null)
    });
    r.f("stop_all_sounds", |e, _, _| {
        e.audio.push(AudioCmd::StopAll);
        Ok(Value::Null)
    });
    r.f("sound_volume", |e, _, a| {
        let snd = A(a).asset(0, AssetKind::Sound)?;
        let v = A(a).num(1)? * e.sounds[snd as usize].volume;
        e.audio.push(AudioCmd::Volume(snd, v));
        Ok(Value::Null)
    });

    // ---------------------------------------------------------------- tiles & layers
    r.f("tile_at", |e, _, a| {
        let (x, y) = (A(a).num(0)?, A(a).num(1)?);
        let layer = match A(a).opt(2) {
            Some(_) => Some(A(a).text(2)?),
            None => None,
        };
        let Some(li) = e.layer_index(layer.as_deref()) else { return Ok(Value::Num(-1.0)) };
        if let LayerKind::Tiles { cw, ch, grid, ox, oy, .. } = &e.layers[li].kind {
            let (c, r) = (((x - ox) / *cw as f64).floor(), ((y - oy) / *ch as f64).floor());
            if c >= 0.0 && r >= 0.0 {
                if let Some(t) = grid.get(r as usize).and_then(|row| row.get(c as usize)) {
                    return Ok(Value::Num(*t as f64));
                }
            }
        }
        Ok(Value::Num(-1.0))
    });
    r.f("tile_set", |e, _, a| {
        let (x, y, t) = (A(a).num(0)?, A(a).num(1)?, A(a).num(2)? as i32);
        let layer = match A(a).opt(3) {
            Some(_) => Some(A(a).text(3)?),
            None => None,
        };
        let Some(li) = e.layer_index(layer.as_deref()) else { return err("no tile layer found") };
        if let LayerKind::Tiles { cw, ch, grid, ox, oy, .. } = &mut e.layers[li].kind {
            let (c, r) = (((x - *ox) / *cw as f64).floor(), ((y - *oy) / *ch as f64).floor());
            if c < 0.0 || r < 0.0 {
                return Ok(Value::Bool(false));
            }
            let (c, r) = (c as usize, r as usize);
            if grid.len() <= r {
                grid.resize(r + 1, vec![]);
            }
            if grid[r].len() <= c {
                grid[r].resize(c + 1, -1);
            }
            grid[r][c] = t;
            return Ok(Value::Bool(true));
        }
        err("layer is not a tile layer")
    });
    r.f("layer_visible", |e, _, a| {
        let name = A(a).text(0)?;
        let vis = A(a).bool_or(1, true);
        match e.layers.iter_mut().find(|l| l.name == name) {
            Some(l) => {
                l.visible = vis;
                Ok(Value::Null)
            }
            None => err(format!("no layer named '{name}'")),
        }
    });

    // ---------------------------------------------------------------- game state & time
    r.f("steps", |e, _, _| Ok(Value::Num(e.steps as f64)));
    r.f("time", |e, _, _| Ok(Value::Num(e.steps as f64 / e.step_rate())));
    r.f("fps", |e, _, _| Ok(Value::Num(e.fps)));
    r.f("step_rate", |e, _, _| Ok(Value::Num(e.step_rate())));
    r.f("set_step_rate", |e, _, a| {
        e.room_speed = A(a).num(0)?.clamp(1.0, 1000.0);
        Ok(Value::Null)
    });
    r.f("debug_mode", |e, _, a| {
        e.debug = A(a).bool_or(0, true);
        Ok(Value::Null)
    });

    // ---------------------------------------------------------------- saving
    r.f("save_data", |e, _, a| {
        let name = safe_name(&A(a).text(0)?)?;
        let json = to_json(A(a).val(1)?)?;
        let dir = e.dir.join("saves");
        let ok = std::fs::create_dir_all(&dir).is_ok()
            && std::fs::write(dir.join(format!("{name}.json")), serde_json::to_string_pretty(&json).unwrap_or_default()).is_ok();
        Ok(Value::Bool(ok))
    });
    r.f("load_data", |e, _, a| {
        let name = safe_name(&A(a).text(0)?)?;
        let path = e.dir.join("saves").join(format!("{name}.json"));
        let fallback = a.get(1).cloned().unwrap_or(Value::Null);
        match std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
            Some(j) => Ok(from_json(&j)),
            None => Ok(fallback),
        }
    });
}

fn safe_name(s: &str) -> RtResult<String> {
    if s.is_empty() || !s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return err(format!("save name '{s}' may only contain letters, digits, '_' and '-'"));
    }
    Ok(s.to_string())
}

fn pick(e: &mut Engine, cx: &modscript::CallCx, a: &[Value], far: bool) -> RtResult<Value> {
    let p = A(a);
    let t = p.target(0)?;
    let me_id = cx.self_.inst();
    let (x, y) = if p.len() >= 3 {
        (p.num(1)?, p.num(2)?)
    } else {
        match me_id.and_then(|id| e.idx(id)) {
            Some(i) => (e.insts[i].x, e.insts[i].y),
            None => return err("pass x, y when not called from an instance"),
        }
    };
    let mut best: Option<(f64, u32)> = None;
    for id in e.resolve(&t) {
        if Some(id) == me_id {
            continue;
        }
        let Some(i) = e.idx(id) else { continue };
        let d = (e.insts[i].x - x).hypot(e.insts[i].y - y);
        let better = match best {
            None => true,
            Some((bd, _)) => if far { d > bd } else { d < bd },
        };
        if better {
            best = Some((d, id));
        }
    }
    inst_or_null(best.map(|b| b.1))
}

fn segment_hits_box(x1: f64, y1: f64, x2: f64, y2: f64, b: [f64; 4]) -> bool {
    // Liang–Barsky clipping
    let (dx, dy) = (x2 - x1, y2 - y1);
    let mut t0: f64 = 0.0;
    let mut t1: f64 = 1.0;
    for (p, q) in [(-dx, x1 - b[0]), (dx, b[2] - x1), (-dy, y1 - b[1]), (dy, b[3] - y1)] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}


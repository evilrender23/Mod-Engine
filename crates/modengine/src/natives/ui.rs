//! Text input variables (`keyboard_string`, `keyboard_lastchar`), the mouse cursor
//! (`window_set_cursor`, `cursor_sprite`), clipping (`gpu_set_scissor` and a push/pop
//! stack) and rounded rectangles (`draw_roundrect*`).

use super::{err, Reg, A};
use crate::draw::{DrawCmd, Rgba};
use crate::engine::Engine;
use crate::ui_state::{intersect, ExtVar, CURSORS};
use modscript::{intern, AssetKind, RtError, RtResult, Sym, Value};
use std::collections::HashMap;

/// Reads one of the [`ExtVar`]s (`i` = the instance, for the collision layers).
pub(crate) fn get_ext(e: &Engine, i: Option<usize>, v: ExtVar) -> Value {
    match v {
        ExtVar::CollisionLayer => Value::Num(i.map(|i| e.insts[i].col_layer as f64).unwrap_or(0.0)),
        ExtVar::CollisionMask => Value::Num(i.map(|i| e.insts[i].col_mask as f64).unwrap_or(0.0)),
        ExtVar::KeyboardString => Value::string(e.keyboard_string.clone()),
        ExtVar::KeyboardLastchar => Value::string(e.ui.lastchar.clone()),
        ExtVar::CursorSprite => e.ui.cursor_sprite.map(|s| Value::Asset(AssetKind::Sprite, s)).unwrap_or(Value::Null),
    }
}

pub(crate) fn set_ext(e: &mut Engine, i: Option<usize>, x: ExtVar, name: Sym, v: Value) -> RtResult<()> {
    let bits = |v: &Value| -> RtResult<u32> {
        match v.as_num() {
            Some(n) if n >= 0.0 => Ok((n as u64 & 0xFFFF_FFFF) as u32),
            Some(n) if n == -1.0 => Ok(u32::MAX),
            _ => Err(RtError::new(format!("'{name}' must be a bit mask (1, 2, 4... or -1 = all), got {v}"))),
        }
    };
    match x {
        ExtVar::CollisionLayer | ExtVar::CollisionMask => {
            let Some(i) = i else { return err(format!("'{name}' belongs to an instance")) };
            let b = bits(&v)?;
            if x == ExtVar::CollisionLayer {
                e.insts[i].col_layer = b;
            } else {
                e.insts[i].col_mask = b;
            }
        }
        ExtVar::KeyboardString => e.keyboard_string = v.to_string().chars().take(super::net::KEYBOARD_MAX).collect(),
        ExtVar::KeyboardLastchar => e.ui.lastchar = v.to_string(),
        ExtVar::CursorSprite => {
            e.ui.cursor_sprite = match v {
                Value::Null => None,
                Value::Asset(AssetKind::Sprite, s) => Some(s),
                // GameMaker uses -1 for "no sprite"
                Value::Num(n) if n < 0.0 => None,
                other => return err(format!("'{name}' must be a sprite or null, got {}", other.type_name())),
            }
        }
    }
    Ok(())
}

/// Draws `cursor_sprite` at the mouse, on top of the GUI (called at the end of `render`).
pub(crate) fn draw_cursor(e: &mut Engine) {
    let Some(spr) = e.ui.cursor_sprite else { return };
    let Some(s) = e.sprites.get(spr as usize) else { return };
    let n = s.frames.len().max(1) as f64;
    let rate = e.step_rate().max(1.0);
    let frame = ((e.steps as f64 * s.fps / rate).floor() % n) as u32;
    if !e.ui.clip.is_empty() {
        e.ui.clip.clear();
        e.cmds.push(DrawCmd::Scissor(None));
    }
    e.cmds.push(DrawCmd::Sprite { spr, frame, x: e.input.mouse_x as f32, y: e.input.mouse_y as f32, sx: 1.0, sy: 1.0, angle: 0.0, color: Rgba::from_rgb(0xFFFFFF, 1.0) });
}

pub(crate) fn add_constants(c: &mut HashMap<Sym, Value>) {
    for (n, v) in CURSORS {
        c.insert(intern(n), Value::Num(*v as f64));
    }
}

/// `[x, y, w, h]` from (x, y, w, h) numbers or a struct `{ x, y, w, h }` (gpu_get_scissor's).
fn rect_arg(a: &A) -> RtResult<[f32; 4]> {
    if let Some(Value::Struct(s)) = a.opt(0) {
        let s = s.borrow();
        let g = |k: &str| s.fields.get(&intern(k)).and_then(|v| v.as_num()).ok_or_else(|| RtError::new(format!("the scissor struct needs a number '{k}'")));
        return Ok([g("x")? as f32, g("y")? as f32, g("w")?.max(0.0) as f32, g("h")?.max(0.0) as f32]);
    }
    Ok([a.num(0)? as f32, a.num(1)? as f32, a.num(2)?.max(0.0) as f32, a.num(3)?.max(0.0) as f32])
}

fn rect_value(r: [f32; 4]) -> Value {
    Value::new_struct([
        (intern("x"), Value::Num(r[0] as f64)),
        (intern("y"), Value::Num(r[1] as f64)),
        (intern("w"), Value::Num(r[2] as f64)),
        (intern("h"), Value::Num(r[3] as f64)),
    ])
}

fn emit_clip(e: &mut Engine) {
    let top = e.ui.clip.last().copied();
    e.cmds.push(DrawCmd::Scissor(top));
}

/// Outline of a rounded rectangle as a polygon (clockwise on screen).
pub fn roundrect_points(x1: f64, y1: f64, x2: f64, y2: f64, rx: f64, ry: f64) -> Vec<(f64, f64)> {
    let (l, t, r, b) = (x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2));
    let rx = rx.clamp(0.0, (r - l) / 2.0);
    let ry = ry.clamp(0.0, (b - t) / 2.0);
    if rx <= 0.0 || ry <= 0.0 {
        return vec![(l, t), (r, t), (r, b), (l, b)];
    }
    let n = ((rx.max(ry) / 1.5).ceil() as usize).clamp(2, 16);
    let mut pts = Vec::with_capacity(4 * (n + 1));
    // corner centres and their start angles (screen y down: 0 = right, 90 = down)
    let corners = [(r - rx, t + ry, 270.0), (r - rx, b - ry, 0.0), (l + rx, b - ry, 90.0), (l + rx, t + ry, 180.0)];
    for (cx, cy, a0) in corners {
        for k in 0..=n {
            let a = (a0 + 90.0 * k as f64 / n as f64).to_radians();
            pts.push((cx + rx * a.cos(), cy + ry * a.sin()));
        }
    }
    pts
}

/// Records a rounded rectangle: a triangle fan when filled, line segments as an outline.
#[allow(clippy::too_many_arguments)]
fn roundrect(e: &mut Engine, x1: f64, y1: f64, x2: f64, y2: f64, rx: f64, ry: f64, color: Rgba, outline: bool, width: f64) {
    if outline {
        // 1 px lines sit on pixel centres, inside the rectangle (like draw_rect's outline)
        let h = width / 2.0;
        let pts = roundrect_points(x1.min(x2) + h, y1.min(y2) + h, x1.max(x2) - h, y1.max(y2) - h, (rx - h).max(0.0), (ry - h).max(0.0));
        for k in 0..pts.len() {
            let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
            e.cmds.push(DrawCmd::Line { x1: a.0 as f32, y1: a.1 as f32, x2: b.0 as f32, y2: b.1 as f32, width: width as f32, color });
        }
        return;
    }
    let pts = roundrect_points(x1, y1, x2, y2, rx, ry);
    let (cx, cy) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
    for k in 0..pts.len() {
        let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
        if (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9 {
            continue;
        }
        let pts = [cx as f32, cy as f32, a.0 as f32, a.1 as f32, b.0 as f32, b.1 as f32];
        e.cmds.push(DrawCmd::Tri { pts, color, outline: false });
    }
}

fn to_rgb(v: f64) -> u32 {
    (v.max(0.0) as u32) & 0xFF_FFFF
}

pub(super) fn register(r: &mut Reg) {
    // ---------------------------------------------------------------- typed text
    r.f("keyboard_lastchar", |e, _, _| Ok(Value::string(e.ui.lastchar.clone())));

    // ---------------------------------------------------------------- cursor
    r.f("window_set_cursor", |e, _, a| {
        let c = A(a).num(0)? as i32;
        if !CURSORS.iter().any(|(_, v)| *v == c) {
            return err(format!("{c} is not a cursor (use cr_default, cr_none, cr_cross, cr_handpoint...)"));
        }
        e.ui.cursor = c;
        Ok(Value::Null)
    });
    r.f("window_get_cursor", |e, _, _| Ok(Value::Num(e.ui.cursor as f64)));
    r.f("cursor_set_sprite", |e, _, a| {
        e.ui.cursor_sprite = A(a).opt_asset(0, AssetKind::Sprite)?;
        Ok(Value::Null)
    });

    // ---------------------------------------------------------------- scissor
    r.f("gpu_set_scissor", |e, _, a| {
        let rect = rect_arg(&A(a))?;
        e.ui.clip = vec![rect];
        emit_clip(e);
        Ok(Value::Null)
    });
    r.f("gpu_get_scissor", |e, _, _| Ok(e.ui.clip.last().map(|r| rect_value(*r)).unwrap_or(Value::Null)));
    r.f("gpu_reset_scissor", |e, _, _| {
        e.ui.clip.clear();
        emit_clip(e);
        Ok(Value::Null)
    });
    r.f("gpu_push_scissor", |e, _, a| {
        // nested clipping: the new rectangle is cut to the current one
        let rect = rect_arg(&A(a))?;
        let rect = match e.ui.clip.last() {
            Some(top) => intersect(*top, rect),
            None => rect,
        };
        e.ui.clip.push(rect);
        emit_clip(e);
        Ok(Value::Null)
    });
    r.f("gpu_pop_scissor", |e, _, _| {
        if e.ui.clip.pop().is_none() {
            return err("gpu_pop_scissor() without a gpu_push_scissor()");
        }
        emit_clip(e);
        Ok(Value::Null)
    });

    // ---------------------------------------------------------------- rounded rectangles
    r.f("draw_roundrect", |e, _, a| {
        // draw_roundrect(x1, y1, x2, y2, [outline], [radius=6])
        let p = A(a);
        let rad = p.num_or(5, 6.0)?;
        let color = e.draw.rgba();
        roundrect(e, p.num(0)?, p.num(1)?, p.num(2)?, p.num(3)?, rad, rad, color, p.bool_or(4, false), 1.0);
        Ok(Value::Null)
    });
    r.f("draw_roundrect_ext", |e, _, a| {
        // draw_roundrect_ext(x1, y1, x2, y2, radx, rady, [outline], [thickness])
        let p = A(a);
        let color = e.draw.rgba();
        roundrect(e, p.num(0)?, p.num(1)?, p.num(2)?, p.num(3)?, p.num(4)?, p.num(5)?, color, p.bool_or(6, false), p.num_or(7, 1.0)?.max(0.1));
        Ok(Value::Null)
    });
    fn colour(e: &mut Engine, a: &[Value], ext: bool) -> RtResult<Value> {
        // _colour(x1, y1, x2, y2, fill, border, [outline])  _colour_ext(x1, y1, x2, y2, radx, rady, fill, border, [outline])
        let p = A(a);
        let (rx, ry, k) = if ext { (p.num(4)?, p.num(5)?, 6) } else { (6.0, 6.0, 4) };
        let (x1, y1, x2, y2) = (p.num(0)?, p.num(1)?, p.num(2)?, p.num(3)?);
        let alpha = e.draw.alpha;
        let fill = Rgba::from_rgb(to_rgb(p.num(k)?), alpha);
        let border = Rgba::from_rgb(to_rgb(p.num(k + 1)?), alpha);
        if !p.bool_or(k + 2, false) {
            roundrect(e, x1, y1, x2, y2, rx, ry, fill, false, 1.0);
        }
        if border != fill || p.bool_or(k + 2, false) {
            roundrect(e, x1, y1, x2, y2, rx, ry, border, true, 1.0);
        }
        Ok(Value::Null)
    }
    r.f("draw_roundrect_colour", |e, _, a| colour(e, a, false));
    r.f("draw_roundrect_color", |e, _, a| colour(e, a, false));
    r.f("draw_roundrect_colour_ext", |e, _, a| colour(e, a, true));
    r.f("draw_roundrect_color_ext", |e, _, a| colour(e, a, true));
}

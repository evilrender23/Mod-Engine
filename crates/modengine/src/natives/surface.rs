//! Surface functions (GameMaker's `surface_*`, `draw_surface*`, `application_surface_*`)
//! and `draw_clear`. They record commands; the backend owns the GPU render targets.

use super::{Reg, A};
use crate::draw::{DrawCmd, Rgba};
use crate::engine::Engine;
use crate::surface::{APP, MAX_SIZE};
use modscript::{RtError, RtResult, Value};

fn err<T>(msg: impl Into<String>) -> RtResult<T> {
    Err(RtError::new(msg))
}

fn app_size(e: &Engine) -> (u32, u32) {
    (e.config.display.width, e.config.display.height)
}

/// A surface argument: (id, width, height). application_surface is valid.
fn surface(e: &Engine, a: &[Value], i: usize) -> RtResult<(u32, u32, u32)> {
    let id = A(a).int(i)?;
    match e.surfaces.size(id, app_size(e)) {
        Some((w, h)) => Ok((id as u32, w, h)),
        None => err(format!("la surface {id} no existe (créala con surface_create y comprueba surface_exists)")),
    }
}

fn dims(a: &[Value], i: usize) -> RtResult<(u32, u32)> {
    let (w, h) = (A(a).num(i)?, A(a).num(i + 1)?);
    if !(1.0..=MAX_SIZE as f64).contains(&w) || !(1.0..=MAX_SIZE as f64).contains(&h) {
        return err(format!("tamaño de surface no válido: {w}×{h} (de 1 a {MAX_SIZE})"));
    }
    Ok((w as u32, h as u32))
}

fn rgb(v: f64) -> u32 {
    (v.max(0.0) as u32) & 0xFF_FFFF
}

/// Pushes a `DrawCmd::Surface`. `part` = (left, top, w, h) in surface pixels.
#[allow(clippy::too_many_arguments)]
fn draw(e: &mut Engine, id: u32, part: Option<[f32; 4]>, x: f64, y: f64, w: f64, h: f64, angle: f64, color: Rgba) {
    e.cmds.push(DrawCmd::Surface { id, src: part, x: x as f32, y: y as f32, w: w as f32, h: h as f32, angle: angle as f32, color });
}

pub(super) fn register(r: &mut Reg) {
    r.f("surface_create", |e, _, a| {
        let (w, h) = dims(a, 0)?;
        Ok(Value::Num(e.surfaces.create(w, h) as f64))
    });
    r.f("surface_free", |e, _, a| {
        let id = A(a).int(0)?;
        if id == APP as i64 {
            return err("application_surface no se puede liberar");
        }
        if let Ok(i) = u32::try_from(id) {
            if e.surfaces.stack.contains(&i) {
                return err("no se puede liberar una surface mientras es el destino del dibujo (surface_reset_target antes)");
            }
            e.surfaces.live.remove(&i);
        }
        Ok(Value::Null)
    });
    r.f("surface_exists", |e, _, a| {
        let id = A(a).num(0).unwrap_or(-1.0);
        Ok(Value::Bool(id.fract() == 0.0 && e.surfaces.size(id as i64, app_size(e)).is_some()))
    });
    r.f("surface_get_width", |e, _, a| Ok(Value::Num(surface(e, a, 0)?.1 as f64)));
    r.f("surface_get_height", |e, _, a| Ok(Value::Num(surface(e, a, 0)?.2 as f64)));
    r.f("surface_resize", |e, _, a| {
        let (id, _, _) = surface(e, a, 0)?;
        if id == APP {
            return err("application_surface tiene el tamaño de [display] en game.toml");
        }
        let (w, h) = dims(a, 1)?;
        e.surfaces.live.insert(id, (w, h));
        Ok(Value::Null)
    });
    r.f("surface_set_target", |e, _, a| {
        let (id, w, h) = surface(e, a, 0)?;
        if id == APP {
            return err("application_surface ya es el destino normal del dibujo: usa surface_reset_target()");
        }
        e.surfaces.stack.push(id);
        e.cmds.push(DrawCmd::Target(Some((id, w, h))));
        Ok(Value::Bool(true))
    });
    r.f("surface_reset_target", |e, _, _| {
        if e.surfaces.stack.pop().is_none() {
            return err("surface_reset_target() sin surface_set_target()");
        }
        let back = e.surfaces.stack.last().and_then(|&id| e.surfaces.live.get(&id).map(|&(w, h)| (id, w, h)));
        e.cmds.push(DrawCmd::Target(back));
        Ok(Value::Bool(true))
    });
    r.f("surface_get_target", |e, _, _| Ok(Value::Num(e.surfaces.stack.last().map(|&i| i as f64).unwrap_or(-1.0))));
    r.f("draw_surface", |e, _, a| {
        let (id, w, h) = surface(e, a, 0)?;
        let p = A(a);
        let color = Rgba::from_rgb(0xFFFFFF, e.draw.alpha);
        draw(e, id, None, p.num(1)?, p.num(2)?, w as f64, h as f64, 0.0, color);
        Ok(Value::Null)
    });
    r.f("draw_surface_ext", |e, _, a| {
        let (id, w, h) = surface(e, a, 0)?;
        let p = A(a);
        let color = Rgba::from_rgb(rgb(p.num_or(6, 0xFFFFFF as f64)?), p.num_or(7, 1.0)?);
        let (sx, sy) = (p.num_or(3, 1.0)?, p.num_or(4, 1.0)?);
        draw(e, id, None, p.num(1)?, p.num(2)?, w as f64 * sx, h as f64 * sy, p.num_or(5, 0.0)?, color);
        Ok(Value::Null)
    });
    r.f("draw_surface_stretched", |e, _, a| {
        let (id, _, _) = surface(e, a, 0)?;
        let p = A(a);
        let color = Rgba::from_rgb(0xFFFFFF, e.draw.alpha);
        draw(e, id, None, p.num(1)?, p.num(2)?, p.num(3)?, p.num(4)?, 0.0, color);
        Ok(Value::Null)
    });
    r.f("draw_surface_stretched_ext", |e, _, a| {
        let (id, _, _) = surface(e, a, 0)?;
        let p = A(a);
        let color = Rgba::from_rgb(rgb(p.num_or(5, 0xFFFFFF as f64)?), p.num_or(6, 1.0)?);
        draw(e, id, None, p.num(1)?, p.num(2)?, p.num(3)?, p.num(4)?, 0.0, color);
        Ok(Value::Null)
    });
    // draw_surface_part(surf, left, top, w, h, x, y)
    r.f("draw_surface_part", |e, _, a| {
        let (id, _, _) = surface(e, a, 0)?;
        let p = A(a);
        let (l, t, w, h) = (p.num(1)?, p.num(2)?, p.num(3)?, p.num(4)?);
        let color = Rgba::from_rgb(0xFFFFFF, e.draw.alpha);
        draw(e, id, Some([l as f32, t as f32, w as f32, h as f32]), p.num(5)?, p.num(6)?, w, h, 0.0, color);
        Ok(Value::Null)
    });
    // draw_surface_part_ext(surf, left, top, w, h, x, y, xscale, yscale, colour, alpha)
    r.f("draw_surface_part_ext", |e, _, a| {
        let (id, _, _) = surface(e, a, 0)?;
        let p = A(a);
        let (l, t, w, h) = (p.num(1)?, p.num(2)?, p.num(3)?, p.num(4)?);
        let (sx, sy) = (p.num_or(7, 1.0)?, p.num_or(8, 1.0)?);
        let color = Rgba::from_rgb(rgb(p.num_or(9, 0xFFFFFF as f64)?), p.num_or(10, 1.0)?);
        draw(e, id, Some([l as f32, t as f32, w as f32, h as f32]), p.num(5)?, p.num(6)?, w * sx, h * sy, 0.0, color);
        Ok(Value::Null)
    });
    r.f("application_surface_draw_enable", |e, _, a| {
        e.surfaces.app_draw = A(a).bool_or(0, true);
        Ok(Value::Null)
    });
    r.f("application_surface_is_enabled", |_, _, _| Ok(Value::Bool(true)));
    r.f("application_surface_is_draw_enabled", |e, _, _| Ok(Value::Bool(e.surfaces.app_draw)));
    // whatever is being drawn to (frame or surface), alpha included
    r.f("draw_clear", |e, _, a| {
        let c = rgb(A(a).num(0)?);
        e.cmds.push(DrawCmd::Clear(Rgba::from_rgb(c, 1.0)));
        Ok(Value::Null)
    });
    r.f("draw_clear_alpha", |e, _, a| {
        let c = rgb(A(a).num(0)?);
        e.cmds.push(DrawCmd::Clear(Rgba::from_rgb(c, A(a).num_or(1, 1.0)?)));
        Ok(Value::Null)
    });
}

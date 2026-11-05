//! Drawing functions. They record commands into the engine's draw list.

use super::{me, Reg, A};
use crate::draw::{DrawCmd, Rgba};
use crate::engine::Engine;
use modscript::{AssetKind, RtError, RtResult, Value};

fn err<T>(msg: impl Into<String>) -> RtResult<T> {
    Err(RtError::new(msg))
}

fn col(e: &Engine) -> Rgba {
    e.draw.rgba()
}

fn frame_of(e: &Engine, spr: u32, f: f64) -> u32 {
    let n = e.sprites[spr as usize].frames.len() as i64;
    (f.floor() as i64).rem_euclid(n.max(1)) as u32
}

fn f(v: f64) -> f32 {
    v as f32
}

fn to_rgb(v: f64) -> u32 {
    (v.max(0.0) as u32) & 0xFF_FFFF
}

pub(super) fn register(r: &mut Reg) {
    r.f("draw_self", |e, cx, _| {
        let i = me(e, cx)?;
        e.draw_inst(i);
        Ok(Value::Null)
    });
    r.f("draw_sprite", |e, _, a| {
        let p = A(a);
        let spr = p.asset(0, AssetKind::Sprite)?;
        let frame = frame_of(e, spr, p.num(1)?);
        let color = Rgba::from_rgb(0xFFFFFF, e.draw.alpha);
        e.cmds.push(DrawCmd::Sprite { spr, frame, x: f(p.num(2)?), y: f(p.num(3)?), sx: 1.0, sy: 1.0, angle: 0.0, color });
        Ok(Value::Null)
    });
    r.f("draw_sprite_ext", |e, _, a| {
        let p = A(a);
        let spr = p.asset(0, AssetKind::Sprite)?;
        let frame = frame_of(e, spr, p.num(1)?);
        let color = Rgba::from_rgb(to_rgb(p.num_or(7, 0xFFFFFF as f64)?), p.num_or(8, 1.0)?);
        e.cmds.push(DrawCmd::Sprite {
            spr,
            frame,
            x: f(p.num(2)?),
            y: f(p.num(3)?),
            sx: f(p.num_or(4, 1.0)?),
            sy: f(p.num_or(5, 1.0)?),
            angle: f(p.num_or(6, 0.0)?),
            color,
        });
        Ok(Value::Null)
    });
    r.f("draw_sprite_stretched", |e, _, a| {
        let p = A(a);
        let spr = p.asset(0, AssetKind::Sprite)?;
        let frame = frame_of(e, spr, p.num(1)?);
        let color = Rgba::from_rgb(0xFFFFFF, e.draw.alpha);
        e.cmds.push(DrawCmd::Stretched { spr, frame, x: f(p.num(2)?), y: f(p.num(3)?), w: f(p.num(4)?), h: f(p.num(5)?), color });
        Ok(Value::Null)
    });
    r.f("draw_rect", |e, _, a| {
        let p = A(a);
        let (x1, y1, x2, y2) = (p.num(0)?, p.num(1)?, p.num(2)?, p.num(3)?);
        let color = col(e);
        e.cmds.push(DrawCmd::Rect {
            x: f(x1.min(x2)),
            y: f(y1.min(y2)),
            w: f((x2 - x1).abs()),
            h: f((y2 - y1).abs()),
            color,
            outline: p.bool_or(4, false),
        });
        Ok(Value::Null)
    });
    r.f("draw_circle", |e, _, a| {
        let p = A(a);
        let color = col(e);
        e.cmds.push(DrawCmd::Circle { x: f(p.num(0)?), y: f(p.num(1)?), r: f(p.num(2)?), color, outline: p.bool_or(3, false) });
        Ok(Value::Null)
    });
    r.f("draw_line", |e, _, a| {
        let p = A(a);
        let color = col(e);
        e.cmds.push(DrawCmd::Line {
            x1: f(p.num(0)?),
            y1: f(p.num(1)?),
            x2: f(p.num(2)?),
            y2: f(p.num(3)?),
            width: f(p.num_or(4, 1.0)?),
            color,
        });
        Ok(Value::Null)
    });
    r.f("draw_tri", |e, _, a| {
        let p = A(a);
        let mut pts = [0f32; 6];
        for (k, v) in pts.iter_mut().enumerate() {
            *v = f(p.num(k)?);
        }
        let color = col(e);
        e.cmds.push(DrawCmd::Tri { pts, color, outline: p.bool_or(6, false) });
        Ok(Value::Null)
    });
    r.f("draw_point", |e, _, a| {
        let p = A(a);
        let color = col(e);
        e.cmds.push(DrawCmd::Rect { x: f(p.num(0)?.floor()), y: f(p.num(1)?.floor()), w: 1.0, h: 1.0, color, outline: false });
        Ok(Value::Null)
    });
    r.f("draw_text", |e, _, a| {
        let p = A(a);
        let text = p.text(2)?;
        let (font, color, halign, valign) = (e.draw.font, col(e), e.draw.halign, e.draw.valign);
        e.cmds.push(DrawCmd::Text { x: f(p.num(0)?), y: f(p.num(1)?), text, font, scale: 1.0, color, halign, valign });
        Ok(Value::Null)
    });
    r.f("draw_text_ext", |e, _, a| {
        // draw_text_ext(x, y, text, scale, [color], [alpha])
        let p = A(a);
        let text = p.text(2)?;
        let scale = f(p.num_or(3, 1.0)?);
        let color = Rgba::from_rgb(to_rgb(p.num_or(4, e.draw.color as f64)?), p.num_or(5, e.draw.alpha)?);
        let (font, halign, valign) = (e.draw.font, e.draw.halign, e.draw.valign);
        e.cmds.push(DrawCmd::Text { x: f(p.num(0)?), y: f(p.num(1)?), text, font, scale, color, halign, valign });
        Ok(Value::Null)
    });
    r.f("draw_bar", |e, _, a| {
        // draw_bar(x1, y1, x2, y2, fraction 0..1, back_color, fill_color)
        let p = A(a);
        let (x1, y1, x2, y2) = (p.num(0)?, p.num(1)?, p.num(2)?, p.num(3)?);
        let t = p.num(4)?.clamp(0.0, 1.0);
        let alpha = e.draw.alpha;
        let back = Rgba::from_rgb(to_rgb(p.num_or(5, 0x202020 as f64)?), alpha);
        let fill = Rgba::from_rgb(to_rgb(p.num_or(6, 0x43A047 as f64)?), alpha);
        e.cmds.push(DrawCmd::Rect { x: f(x1), y: f(y1), w: f(x2 - x1), h: f(y2 - y1), color: back, outline: false });
        e.cmds.push(DrawCmd::Rect { x: f(x1), y: f(y1), w: f((x2 - x1) * t), h: f(y2 - y1), color: fill, outline: false });
        Ok(Value::Null)
    });
    r.f("set_color", |e, _, a| {
        e.draw.color = to_rgb(A(a).num(0)?);
        Ok(Value::Null)
    });
    r.f("get_color", |e, _, _| Ok(Value::Num(e.draw.color as f64)));
    r.f("set_alpha", |e, _, a| {
        e.draw.alpha = A(a).num(0)?.clamp(0.0, 1.0);
        Ok(Value::Null)
    });
    r.f("get_alpha", |e, _, _| Ok(Value::Num(e.draw.alpha)));
    r.f("set_font", |e, _, a| {
        e.draw.font = A(a).opt_asset(0, AssetKind::Font)?;
        Ok(Value::Null)
    });
    r.f("set_align", |e, _, a| {
        let h = A(a).num(0)?;
        let v = A(a).num_or(1, e.draw.valign as f64)?;
        if !(0.0..=2.0).contains(&h) || !(0.0..=2.0).contains(&v) {
            return err("use align.left/center/right and align.top/middle/bottom");
        }
        e.draw.halign = h as u8;
        e.draw.valign = v as u8;
        Ok(Value::Null)
    });
    r.f("text_width", |e, _, a| {
        let t = A(a).text(0)?;
        Ok(Value::Num((e.measure)(e.draw.font, &t, 1.0).0 as f64))
    });
    r.f("text_height", |e, _, a| {
        let t = A(a).text(0)?;
        Ok(Value::Num((e.measure)(e.draw.font, &t, 1.0).1 as f64))
    });

    // ---------------------------------------------------------------- colours
    r.f("rgb", |_, _, a| {
        let p = A(a);
        let c = |v: f64| v.clamp(0.0, 255.0) as u32;
        Ok(Value::Num(((c(p.num(0)?) << 16) | (c(p.num(1)?) << 8) | c(p.num(2)?)) as f64))
    });
    r.f("hsv", |_, _, a| {
        // hue 0..360, saturation 0..1, value 0..1
        let p = A(a);
        let (h, s, v) = (p.num(0)?.rem_euclid(360.0), p.num(1)?.clamp(0.0, 1.0), p.num(2)?.clamp(0.0, 1.0));
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = v - c;
        let (r1, g1, b1) = match (h / 60.0) as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let to = |u: f64| ((u + m) * 255.0).round() as u32;
        Ok(Value::Num(((to(r1) << 16) | (to(g1) << 8) | to(b1)) as f64))
    });
    r.f("color_mix", |_, _, a| {
        let p = A(a);
        let (c1, c2, t) = (to_rgb(p.num(0)?), to_rgb(p.num(1)?), p.num(2)?.clamp(0.0, 1.0));
        let ch = |c: u32, s: u32| ((c >> s) & 255) as f64;
        let mix = |s: u32| ((ch(c1, s) + (ch(c2, s) - ch(c1, s)) * t).round() as u32) << s;
        Ok(Value::Num((mix(16) | mix(8) | mix(0)) as f64))
    });
    r.f("color_r", |_, _, a| Ok(Value::Num(((to_rgb(A(a).num(0)?) >> 16) & 255) as f64)));
    r.f("color_g", |_, _, a| Ok(Value::Num(((to_rgb(A(a).num(0)?) >> 8) & 255) as f64)));
    r.f("color_b", |_, _, a| Ok(Value::Num((to_rgb(A(a).num(0)?) & 255) as f64)));
}

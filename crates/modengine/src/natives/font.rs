//! Fonts and text measuring, with GameMaker's names next to Mod Engine's own
//! (`draw_set_font` = `set_font`, `string_width` = `text_width`...).

use super::{err, Reg, A};
use crate::draw::DrawCmd;
use crate::engine::Engine;
use modscript::{intern, AssetKind, RtResult, Value};

/// Line separation for wrapped text: `sep` < 0 means the font's own line height.
fn line_height(e: &Engine, sep: f64) -> f64 {
    if sep >= 0.0 {
        return sep;
    }
    (e.measure)(e.draw.font, "A", 1.0).1 as f64
}

/// `text` broken into lines no wider than `w` pixels (`w` < 0: only at line breaks).
fn wrapped(e: &Engine, text: &str, w: f64) -> Vec<String> {
    let font = e.draw.font;
    crate::font::wrap(text, w as f32, &|s: &str| (e.measure)(font, s, 1.0).0)
}

fn font_arg(e: &Engine, a: &A, i: usize) -> RtResult<usize> {
    let f = a.asset(i, AssetKind::Font)? as usize;
    if f >= e.fonts.len() {
        return err("font does not exist");
    }
    Ok(f)
}

fn font_or_current(e: &Engine, a: &A, i: usize) -> RtResult<Option<u32>> {
    match a.opt(i) {
        Some(_) => Ok(Some(font_arg(e, a, i)? as u32)),
        None => Ok(e.draw.font),
    }
}

pub(crate) fn register(r: &mut Reg) {
    r.f("draw_set_font", |e, _, a| {
        e.draw.font = A(a).opt_asset(0, AssetKind::Font)?;
        Ok(Value::Null)
    });
    fn current(e: &mut Engine, _: &modscript::CallCx, _: &[Value]) -> RtResult<Value> {
        Ok(e.draw.font.map(|f| Value::Asset(AssetKind::Font, f)).unwrap_or(Value::Null))
    }
    r.f("draw_get_font", current);
    r.f("get_font", current);

    // measuring with the current font
    r.f("string_width", |e, _, a| {
        let t = A(a).text(0)?;
        Ok(Value::Num((e.measure)(e.draw.font, &t, 1.0).0 as f64))
    });
    r.f("string_height", |e, _, a| {
        let t = A(a).text(0)?;
        Ok(Value::Num((e.measure)(e.draw.font, &t, 1.0).1 as f64))
    });
    fn width_ext(e: &mut Engine, _: &modscript::CallCx, a: &[Value]) -> RtResult<Value> {
        // (text, sep, w): width of the text wrapped at w pixels
        let p = A(a);
        let lines = wrapped(e, &p.text(0)?, p.num_or(2, -1.0)?);
        Ok(Value::Num(lines.iter().map(|l| (e.measure)(e.draw.font, l, 1.0).0 as f64).fold(0.0, f64::max)))
    }
    fn height_ext(e: &mut Engine, _: &modscript::CallCx, a: &[Value]) -> RtResult<Value> {
        let p = A(a);
        let lines = wrapped(e, &p.text(0)?, p.num_or(2, -1.0)?);
        Ok(Value::Num(lines.len() as f64 * line_height(e, p.num_or(1, -1.0)?)))
    }
    r.f("string_width_ext", width_ext);
    r.f("text_width_ext", width_ext);
    r.f("string_height_ext", height_ext);
    r.f("text_height_ext", height_ext);
    r.f("text_wrap", |e, _, a| {
        // text_wrap(text, w): the lines the text breaks into, as an array of strings
        let p = A(a);
        let lines = wrapped(e, &p.text(0)?, p.num(1)?);
        Ok(Value::array(lines.into_iter().map(Value::string).collect()))
    });
    r.f("draw_text_wrap", |e, _, a| {
        // draw_text_wrap(x, y, text, sep, w): wrapped at w pixels, sep pixels between lines
        let p = A(a);
        let (x, y) = (p.num(0)?, p.num(1)?);
        let lines = wrapped(e, &p.text(2)?, p.num_or(4, -1.0)?);
        let lh = line_height(e, p.num_or(3, -1.0)?);
        let total = lh * lines.len() as f64;
        let top = y - match e.draw.valign {
            1 => total / 2.0,
            2 => total,
            _ => 0.0,
        };
        let (font, color, halign) = (e.draw.font, e.draw.rgba(), e.draw.halign);
        for (k, l) in lines.into_iter().enumerate() {
            e.cmds.push(DrawCmd::Text { x: x as f32, y: (top + k as f64 * lh) as f32, text: l, font, scale: 1.0, color, halign, valign: 0 });
        }
        Ok(Value::Null)
    });

    // font properties
    r.f("font_get_name", |e, _, a| {
        let f = font_arg(e, &A(a), 0)?;
        Ok(Value::str(&e.fonts[f].name))
    });
    r.f("font_get_fontname", |e, _, a| {
        let f = font_arg(e, &A(a), 0)?;
        Ok(Value::str(&e.fonts[f].source))
    });
    r.f("font_get_size", |e, _, a| {
        let p = A(a);
        match font_or_current(e, &p, 0)? {
            Some(f) => Ok(Value::Num(e.fonts[f as usize].size as f64)),
            None => Ok(Value::Num(e.config.display.font_size as f64)),
        }
    });
    r.f("font_get_bold", |e, _, a| {
        let f = font_arg(e, &A(a), 0)?;
        Ok(Value::Bool(e.fonts[f].bold))
    });
    r.f("font_get_italic", |e, _, a| {
        let f = font_arg(e, &A(a), 0)?;
        Ok(Value::Bool(e.fonts[f].italic))
    });
    r.f("font_get_first", |e, _, a| {
        let f = font_arg(e, &A(a), 0)?;
        Ok(Value::Num(e.fonts[f].first() as f64))
    });
    r.f("font_get_last", |e, _, a| {
        let f = font_arg(e, &A(a), 0)?;
        Ok(Value::Num(e.fonts[f].last() as f64))
    });
    r.f("font_has_char", |e, _, a| {
        // font_has_char(font, "ñ"): is the character baked into the font?
        let p = A(a);
        let f = font_arg(e, &p, 0)?;
        let ch = p.text(1)?.chars().next();
        Ok(Value::Bool(match (&e.fonts[f].atlas, ch) {
            (Some(at), Some(c)) => at.glyphs.contains_key(&c),
            (None, Some(c)) => e.fonts[f].ranges.iter().any(|r| (r[0]..=r[1]).contains(&(c as u32))),
            _ => false,
        }))
    });
    r.f("font_get_info", |e, _, a| {
        let f = font_arg(e, &A(a), 0)?;
        let fo = &e.fonts[f];
        let (line, ascent, glyphs) = match &fo.atlas {
            Some(at) => (at.line_height as f64, at.ascent as f64, at.glyphs.len() as f64),
            None => (fo.size as f64, fo.size as f64 * 0.8, 0.0),
        };
        let ranges = fo.ranges.iter().map(|r| Value::array(vec![Value::Num(r[0] as f64), Value::Num(r[1] as f64)])).collect();
        Ok(Value::new_struct([
            (intern("name"), Value::str(&fo.name)),
            (intern("fontname"), Value::str(&fo.source)),
            (intern("size"), Value::Num(fo.size as f64)),
            (intern("bold"), Value::Bool(fo.bold)),
            (intern("italic"), Value::Bool(fo.italic)),
            (intern("antialias"), Value::Bool(fo.antialias)),
            (intern("ranges"), Value::array(ranges)),
            (intern("line_height"), Value::Num(line)),
            (intern("ascent"), Value::Num(ascent)),
            (intern("glyphs"), Value::Num(glyphs)),
        ]))
    });
}

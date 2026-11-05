//! The game palette (`[palette]` in game.toml) from scripts.
//!
//! Colours are indexed in character order (`0`-`9`, `A`-`Z`, `a`-`z`, symbols), which
//! for a palette imported with the editor is the original palette order.

use super::{Reg, A};
use crate::engine::Engine;
use crate::palette::{ordered, Palette};
use modscript::{RtError, RtResult, Value};

fn entries(e: &Engine) -> Vec<(char, u32)> {
    ordered(&e.config.palette)
}

/// Index or one-character string -> position in [`entries`].
fn lookup(e: &Engine, a: &A, i: usize) -> RtResult<Option<(char, u32)>> {
    let list = entries(e);
    Ok(match a.val(i)? {
        Value::Str(s) => {
            let ch = s.chars().next();
            list.into_iter().find(|(c, _)| Some(*c) == ch)
        }
        _ => {
            let n = a.num(i)?;
            if n < 0.0 {
                None
            } else {
                list.get(n.floor() as usize).copied()
            }
        }
    })
}

pub(super) fn register(r: &mut Reg) {
    // palette_size() -> number of colours in game.toml's [palette]
    r.f("palette_size", |e, _, _| Ok(Value::Num(entries(e).len() as f64)));
    // palette_color(index or "char") -> colour (like color.red); errors when missing
    r.f("palette_color", |e, _, a| {
        let p = A(a);
        match lookup(e, &p, 0)? {
            Some((_, c)) => Ok(Value::Num(c as f64)),
            None => Err(RtError::new(format!("palette_color: {} is not in the palette ({} colours)", p.text(0)?, entries(e).len()))),
        }
    });
    // palette_char(index) -> the pixel-art character of that colour ("" when out of range)
    r.f("palette_char", |e, _, a| {
        let p = A(a);
        Ok(Value::Str(lookup(e, &p, 0)?.map(|(c, _)| c.to_string()).unwrap_or_default().into()))
    });
    // palette_nearest(color) -> index of the closest palette colour (-1 if the palette is empty)
    r.f("palette_nearest", |e, _, a| {
        let c = A(a).num(0)?.max(0.0) as u32 & 0xFFFFFF;
        let pal = Palette { colors: entries(e).into_iter().map(|(_, c)| c).collect(), ..Default::default() };
        Ok(Value::Num(pal.nearest(c).map(|i| i as f64).unwrap_or(-1.0)))
    });
}

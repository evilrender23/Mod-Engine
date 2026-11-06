//! Game feel helpers: `flash`, `sprite_outline`, `sprite_overwrite`, `pulse`,
//! `popup_text`, `hitstop`, `juice_set_reduced`, plus layered music
//! (`audio_play_layers` / `audio_layers_set`). See `feel.rs`.

use super::{err, Reg, A};
use crate::audio::{self, PlayOpts};
use crate::engine::Engine;
use crate::feel::{self, Popup};
use modscript::{intern, AssetKind, RtResult, Value};

/// Instances named by argument `i` (an instance, an object or `all`).
fn insts(e: &Engine, a: &A, i: usize) -> RtResult<Vec<u32>> {
    let t = a.target(i)?;
    Ok(e.resolve(&t))
}

/// A colour argument; null / a negative number turn the effect off.
fn color_opt(a: &A, i: usize, default: Option<u32>) -> RtResult<Option<u32>> {
    match a.0.get(i) {
        None => Ok(default),
        Some(Value::Null) | Some(Value::Bool(false)) => Ok(None),
        Some(v) => match v.as_num() {
            Some(n) if n < 0.0 => Ok(None),
            Some(n) => Ok(Some((n as u32) & 0xFF_FFFF)),
            None => err(format!("argument #{} must be a colour (#rrggbb) or null, got {}", i + 1, v.type_name())),
        },
    }
}

fn popup_style(e: &Engine, x: f64, y: f64, text: String, style: Option<&Value>) -> RtResult<Popup> {
    let mut p = Popup { text, x, y, gui: false, color: 0xFFFFFF, outline: Some(0x000000), font: e.draw.font, rise: 24.0, dur: 0.8, t: 0.0, scale: 1.0, pop: 1.6, active: true };
    let Some(v) = style else { return Ok(p) };
    let Value::Struct(s) = v else { return err("popup_text(x, y, text, [style]): style must be a struct { color, font, rise, duration, scale, scale_pop, outline, gui }") };
    for (k, v) in s.borrow().fields.iter() {
        let num = || v.as_num().ok_or_else(|| modscript::RtError::new(format!("popup style '{k}' must be a number")));
        match &*k.name() {
            "color" | "colour" => p.color = num()? as u32 & 0xFF_FFFF,
            "outline" => {
                p.outline = match v {
                    Value::Null | Value::Bool(false) => None,
                    Value::Bool(true) => Some(0),
                    _ => Some(num()? as u32 & 0xFF_FFFF),
                }
            }
            "font" => {
                p.font = match v {
                    Value::Asset(AssetKind::Font, f) => Some(*f),
                    Value::Null => None,
                    _ => return err("popup style 'font' must be a font asset"),
                }
            }
            "rise" => p.rise = num()?,
            "duration" => p.dur = num()?.max(0.01),
            "scale" => p.scale = num()?,
            "scale_pop" | "pop" => p.pop = num()?,
            "gui" => p.gui = v.truthy(),
            other => return err(format!("unknown popup style '{other}' (color, font, rise, duration, scale, scale_pop, outline, gui)")),
        }
    }
    Ok(p)
}

pub(super) fn register(r: &mut Reg) {
    // flash(inst, [color], [seconds], [amount])
    r.f("flash", |e, _, a| {
        let p = A(a);
        let ids = insts(e, &p, 0)?;
        let c = color_opt(&p, 1, Some(0xFFFFFF))?.unwrap_or(0xFFFFFF);
        let secs = p.num_or(2, 0.1)?.max(0.0);
        let amount = p.num_or(3, 1.0)?.clamp(0.0, 1.0);
        for id in ids {
            e.feel.fx.entry(id).or_default().flash = if secs > 0.0 { Some((c, amount, secs, secs)) } else { None };
        }
        Ok(Value::Null)
    });
    // sprite_outline(inst, color | null, [thickness], [alpha])
    r.f("sprite_outline", |e, _, a| {
        let p = A(a);
        let ids = insts(e, &p, 0)?;
        let c = color_opt(&p, 1, None)?;
        let th = p.num_or(2, 1.0)?.round().clamp(1.0, 8.0) as u32;
        let alpha = p.num_or(3, 1.0)?.clamp(0.0, 1.0);
        for id in ids {
            e.feel.fx.entry(id).or_default().outline = c.map(|c| (c, th, alpha));
        }
        Ok(Value::Null)
    });
    // sprite_overwrite(inst, color | null, [amount])
    r.f("sprite_overwrite", |e, _, a| {
        let p = A(a);
        let ids = insts(e, &p, 0)?;
        let c = color_opt(&p, 1, None)?;
        let amount = p.num_or(2, 1.0)?.clamp(0.0, 1.0);
        for id in ids {
            e.feel.fx.entry(id).or_default().overwrite = c.map(|c| (c, amount));
        }
        Ok(Value::Null)
    });
    r.f("sprite_fx_clear", |e, _, a| {
        let p = A(a);
        for id in insts(e, &p, 0)? {
            if let Some(fx) = e.feel.fx.get_mut(&id) {
                // a running pulse keeps going (it owns the scale)
                *fx = feel::InstFx { pulse: fx.pulse, ..Default::default() };
            }
        }
        Ok(Value::Null)
    });
    // sprite_fx(inst) -> { flash, outline, overwrite } current amounts (debugging / tests)
    r.f("sprite_fx", |e, _, a| {
        let p = A(a);
        let id = match p.val(0)? {
            Value::Inst(id) => *id,
            o => return err(format!("sprite_fx(inst): expected an instance, not {}", o.type_name())),
        };
        let fx = e.feel.fx.get(&id).copied().unwrap_or_default();
        let (mc, ma) = fx.mix(e.feel.reduced).unwrap_or((0, 0.0));
        Ok(Value::new_struct([
            (intern("overwrite_color"), Value::Num(mc as f64)),
            (intern("overwrite_amount"), Value::Num(ma)),
            (intern("outline_color"), fx.outline.map(|o| Value::Num(o.0 as f64)).unwrap_or(Value::Null)),
            (intern("outline_thickness"), Value::Num(fx.outline.map(|o| o.1 as f64).unwrap_or(0.0))),
            (intern("flashing"), Value::Bool(fx.flash.is_some())),
        ]))
    });
    // pulse(inst, [scale], [seconds])
    r.f("pulse", |e, _, a| {
        let p = A(a);
        let ids = insts(e, &p, 0)?;
        let (s, secs) = (p.num_or(1, 1.2)?, p.num_or(2, 0.15)?);
        let mut last = Value::Null;
        for id in ids {
            last = Value::Num(feel::pulse(e, id, s, secs)? as f64);
        }
        Ok(last)
    });
    // popup_text(x, y, text, [style])
    r.f("popup_text", |e, _, a| {
        let p = A(a);
        let pop = popup_style(e, p.num(0)?, p.num(1)?, p.text(2)?, p.opt(3))?;
        Ok(Value::Num(feel::add_popup(e, pop) as f64))
    });
    r.f("popup_count", |e, _, _| Ok(Value::Num(e.feel.active_popups() as f64)));
    r.f("popup_clear", |e, _, _| {
        e.feel.popups.iter_mut().for_each(|p| p.active = false);
        Ok(Value::Null)
    });
    // hitstop(seconds): freeze the gameplay step (a longer one already running wins)
    r.f("hitstop", |e, _, a| {
        let secs = A(a).num(0)?.max(0.0);
        let steps = (secs * e.step_rate()).round() as u64;
        e.feel.hitstop = e.feel.hitstop.max(steps);
        Ok(Value::Null)
    });
    r.f("hitstop_active", |e, _, _| Ok(Value::Bool(e.feel.hitstop > 0)));
    r.f("hitstop_cancel", |e, _, _| {
        e.feel.hitstop = 0;
        Ok(Value::Null)
    });
    r.f("juice_set_reduced", |e, _, a| {
        e.feel.reduced = A(a).bool_or(0, true);
        Ok(Value::Null)
    });
    r.f("juice_reduced", |e, _, _| Ok(Value::Bool(e.feel.reduced)));

    // ------------------------------------------------------------ layered music
    // audio_play_layers([snd, ...], [gains], [loop=true]) -> [voice, ...] started on the same sample
    r.f("audio_play_layers", |e, _, a| {
        let p = A(a);
        let snds: Vec<u32> = p
            .array(0)?
            .borrow()
            .iter()
            .map(|v| match v {
                Value::Asset(AssetKind::Sound, s) => Ok(*s),
                o => err(format!("audio_play_layers(): every layer must be a sound, not {}", o.type_name())),
            })
            .collect::<RtResult<_>>()?;
        let gains: Vec<f64> = match p.opt(1) {
            Some(Value::Array(g)) => g.borrow().iter().map(|v| v.as_num().unwrap_or(1.0)).collect(),
            _ => vec![],
        };
        let looped = p.bool_or(2, true);
        for &s in &snds {
            audio::ensure_decoded(e, s);
        }
        // one lock: every layer starts in the same mixer block, at the same position
        let ids: Vec<u32> = {
            let mut m = audio::lock(e);
            snds.iter().enumerate().map(|(i, s)| m.play(*s, PlayOpts { looped, gain: gains.get(i).copied().unwrap_or(1.0) as f32, priority: 100.0, ..Default::default() })).collect()
        };
        for (i, s) in snds.iter().enumerate() {
            e.audio.push(crate::engine::AudioCmd::Play { snd: *s, looped, volume: gains.get(i).copied().unwrap_or(1.0) });
        }
        Ok(Value::array(ids.into_iter().map(|i| Value::Num(i as f64)).collect()))
    });
    // audio_layers_set(voices, gains, [ms]): fade every layer to its gain (crossfades)
    r.f("audio_layers_set", |e, _, a| {
        let p = A(a);
        let voices: Vec<u32> = p.array(0)?.borrow().iter().filter_map(|v| v.as_num()).map(|n| n.max(0.0) as u32).collect();
        let gains: Vec<f64> = p.array(1)?.borrow().iter().map(|v| v.as_num().unwrap_or(0.0)).collect();
        let ms = p.num_or(2, 0.0)?;
        let mut m = audio::lock(e);
        for (i, id) in voices.iter().enumerate() {
            if let (Some(v), Some(g)) = (m.voice_mut(*id), gains.get(i)) {
                v.gain.set(*g as f32, ms);
            }
        }
        Ok(Value::Null)
    });
}

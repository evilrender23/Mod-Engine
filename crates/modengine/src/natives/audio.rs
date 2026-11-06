//! Audio: play_sound & co. plus GameMaker's audio_* (voices, pitch, fades, groups,
//! intro→loop, emitters and the listener). The mixing is done in `crate::audio`.

use super::{err, A, Reg};
use crate::audio::{self, Emitter, Falloff, PlayOpts, Spatial};
use crate::engine::{AudioCmd, Engine};
use modscript::{intern, AssetKind, RtResult, Sym, Value};
use std::collections::HashMap;

/// A sound asset or a playing voice (the number play_sound returned).
enum Target {
    Sound(u32),
    Voice(u32),
}

fn target(a: &A, i: usize) -> RtResult<Target> {
    match a.val(i)? {
        Value::Asset(AssetKind::Sound, s) => Ok(Target::Sound(*s)),
        Value::Num(n) => Ok(Target::Voice(n.max(0.0) as u32)),
        o => err(format!("argument #{} must be a sound or a voice (the number play_sound returns), got {}", i + 1, o.type_name())),
    }
}

fn play(e: &mut Engine, snd: u32, o: PlayOpts) -> Value {
    audio::ensure_decoded(e, snd);
    let (looped, volume) = (o.looped, o.gain as f64);
    let id = audio::lock(e).play(snd, o);
    e.audio.push(AudioCmd::Play { snd, looped, volume });
    Value::Num(id as f64)
}

fn struct_num(s: &HashMap<Sym, Value>, k: &str) -> Option<f64> {
    s.get(&intern(k)).and_then(|v| v.as_num())
}

/// play_sound(snd, {loop, volume, pitch, pan, offset, priority, fade, x, y, ref, max, factor, emitter})
fn opts_from_struct(v: &Value) -> RtResult<PlayOpts> {
    let Value::Struct(s) = v else { unreachable!() };
    let f: HashMap<Sym, Value> = s.borrow().fields.iter().map(|(k, v)| (*k, v.clone())).collect();
    for k in f.keys() {
        let n = k.name();
        if !["loop", "volume", "gain", "pitch", "pan", "offset", "priority", "fade", "x", "y", "ref", "max", "factor", "emitter"].contains(&&*n) {
            return err(format!("play_sound(): opción desconocida '{n}' (loop, volume, pitch, pan, offset, priority, fade, x, y, ref, max, factor, emitter)"));
        }
    }
    let mut o = PlayOpts {
        looped: f.get(&intern("loop")).is_some_and(|v| v.truthy()),
        gain: struct_num(&f, "volume").or(struct_num(&f, "gain")).unwrap_or(1.0) as f32,
        pitch: struct_num(&f, "pitch").unwrap_or(1.0) as f32,
        pan: struct_num(&f, "pan").unwrap_or(0.0) as f32,
        offset: struct_num(&f, "offset").unwrap_or(0.0),
        priority: struct_num(&f, "priority").unwrap_or(0.0),
        fade_in: struct_num(&f, "fade").unwrap_or(0.0),
        spatial: Spatial::None,
    };
    if let Some(em) = struct_num(&f, "emitter") {
        o.spatial = Spatial::Emitter(em.max(0.0) as u32);
    } else if let (Some(x), Some(y)) = (struct_num(&f, "x"), struct_num(&f, "y")) {
        let d = Falloff::default();
        let falloff = Falloff {
            reference: struct_num(&f, "ref").unwrap_or(d.reference),
            max: struct_num(&f, "max").unwrap_or(d.max),
            factor: struct_num(&f, "factor").unwrap_or(d.factor),
        };
        o.spatial = Spatial::At { x, y, falloff };
    }
    Ok(o)
}

fn emitter_idx(e: &mut Engine, a: &A, i: usize) -> RtResult<usize> {
    let n = a.num(i)?;
    let m = audio::lock(e);
    match m.emitters.get(n.max(0.0) as usize) {
        Some(Some(_)) if n >= 0.0 => Ok(n as usize),
        _ => err(format!("{} no es un emisor de audio_emitter_create() (¿ya liberado?)", Value::Num(n))),
    }
}

fn group_name(a: &A, i: usize) -> RtResult<String> {
    match a.val(i)? {
        Value::Str(s) => Ok(s.to_string()),
        o => err(format!("argument #{} must be the name of an audio group (text), got {}", i + 1, o.type_name())),
    }
}

/// Common tail of GameMaker's play functions: [gain], [offset], [pitch] from index `i`.
fn gm_tail(a: &A, i: usize, o: &mut PlayOpts) -> RtResult<()> {
    o.gain = a.num_or(i, 1.0)? as f32;
    o.offset = a.num_or(i + 1, 0.0)?;
    o.pitch = a.num_or(i + 2, 1.0)? as f32;
    Ok(())
}

pub(crate) fn register(r: &mut Reg) {
    // ---------------------------------------------------------------- Mod Engine names
    r.f("play_sound", |e, _, a| {
        let p = A(a);
        let snd = p.asset(0, AssetKind::Sound)?;
        let o = match p.opt(1) {
            Some(v @ Value::Struct(_)) => opts_from_struct(v)?,
            _ => PlayOpts { looped: p.bool_or(1, false), gain: p.num_or(2, 1.0)? as f32, ..Default::default() },
        };
        Ok(play(e, snd, o))
    });
    r.f("stop_sound", |e, _, a| {
        let p = A(a);
        let ms = p.num_or(1, 0.0)?;
        match target(&p, 0)? {
            Target::Sound(s) => {
                audio::lock(e).stop_sound(s, ms);
                e.audio.push(AudioCmd::Stop(s));
            }
            Target::Voice(v) => audio::lock(e).stop(v, ms),
        }
        Ok(Value::Null)
    });
    r.f("stop_all_sounds", |e, _, _| {
        audio::stop_all(e);
        e.audio.push(AudioCmd::StopAll);
        Ok(Value::Null)
    });
    // sound_volume(snd | voice, v, [ms]): volume of the voices playing now
    r.f("sound_volume", |e, _, a| {
        let p = A(a);
        let (v, ms) = (p.num(1)? as f32, p.num_or(2, 0.0)?);
        let t = target(&p, 0)?;
        let mut m = audio::lock(e);
        for voice in m.voices.iter_mut().filter(|x| match t {
            Target::Sound(s) => x.snd == s,
            Target::Voice(id) => x.id == id,
        }) {
            voice.gain.set(v, ms);
        }
        drop(m);
        if let Target::Sound(s) = t {
            e.audio.push(AudioCmd::Volume(s, v as f64));
        }
        Ok(Value::Null)
    });

    // ---------------------------------------------------------------- GameMaker names
    r.f("audio_play_sound", |e, _, a| {
        let p = A(a);
        let snd = p.asset(0, AssetKind::Sound)?;
        let mut o = PlayOpts { priority: p.num_or(1, 0.0)?, looped: p.bool_or(2, false), ..Default::default() };
        gm_tail(&p, 3, &mut o)?;
        Ok(play(e, snd, o))
    });
    r.f("audio_play_sound_at", |e, _, a| {
        // (snd, x, y, z, falloff_ref, falloff_max, falloff_factor, loop, priority, [gain, offset, pitch])
        let p = A(a);
        let snd = p.asset(0, AssetKind::Sound)?;
        let falloff = Falloff { reference: p.num(4)?, max: p.num(5)?, factor: p.num(6)? };
        let mut o = PlayOpts {
            spatial: Spatial::At { x: p.num(1)?, y: p.num(2)?, falloff },
            looped: p.bool_or(7, false),
            priority: p.num_or(8, 0.0)?,
            ..Default::default()
        };
        gm_tail(&p, 9, &mut o)?;
        Ok(play(e, snd, o))
    });
    r.f("audio_play_sound_on", |e, _, a| {
        // (emitter, snd, loop, priority, [gain, offset, pitch])
        let p = A(a);
        let em = emitter_idx(e, &p, 0)?;
        let snd = p.asset(1, AssetKind::Sound)?;
        let mut o = PlayOpts { spatial: Spatial::Emitter(em as u32), looped: p.bool_or(2, false), priority: p.num_or(3, 0.0)?, ..Default::default() };
        gm_tail(&p, 4, &mut o)?;
        Ok(play(e, snd, o))
    });
    // ModScript extension: audio_play_sound_intro_loop(snd_intro, snd_loop, [gain]) plays the
    // first sound once and the second in a loop right when it ends (no gap)
    r.f("audio_play_sound_intro_loop", |e, _, a| {
        let p = A(a);
        let (intro, lp) = (p.asset(0, AssetKind::Sound)?, p.asset(1, AssetKind::Sound)?);
        let gain = p.num_or(2, 1.0)? as f32;
        audio::ensure_decoded(e, lp);
        let id = play(e, intro, PlayOpts { gain, ..Default::default() });
        if let Value::Num(v) = id {
            if let Some(voice) = audio::lock(e).voice_mut(v as u32) {
                voice.then = Some(lp);
            }
        }
        Ok(id)
    });
    r.f("audio_stop_sound", |e, _, a| {
        let p = A(a);
        let ms = p.num_or(1, 0.0)?;
        match target(&p, 0)? {
            Target::Sound(s) => audio::lock(e).stop_sound(s, ms),
            Target::Voice(v) => audio::lock(e).stop(v, ms),
        }
        Ok(Value::Null)
    });
    r.f("audio_stop_all", |e, _, _| {
        audio::stop_all(e);
        Ok(Value::Null)
    });
    for (name, pause) in [("audio_pause_sound", true), ("audio_resume_sound", false)] {
        let f: super::NativeFn = if pause {
            |e, _, a| set_paused(e, &A(a), true)
        } else {
            |e, _, a| set_paused(e, &A(a), false)
        };
        r.f(name, f);
    }
    r.f("audio_pause_all", |e, _, _| {
        audio::lock(e).voices.iter_mut().for_each(|v| v.paused = true);
        Ok(Value::Null)
    });
    r.f("audio_resume_all", |e, _, _| {
        audio::lock(e).voices.iter_mut().for_each(|v| v.paused = false);
        Ok(Value::Null)
    });
    r.f("audio_is_playing", |e, _, a| {
        let t = target(&A(a), 0)?;
        let m = audio::lock(e);
        Ok(Value::Bool(match t {
            Target::Sound(s) => m.voices.iter().any(|v| v.snd == s),
            Target::Voice(id) => m.voice(id).is_some(),
        }))
    });
    r.f("audio_is_paused", |e, _, a| {
        let t = target(&A(a), 0)?;
        let m = audio::lock(e);
        Ok(Value::Bool(match t {
            Target::Sound(s) => m.voices.iter().any(|v| v.snd == s && v.paused),
            Target::Voice(id) => m.voice(id).is_some_and(|v| v.paused),
        }))
    });
    // audio_sound_gain(snd | voice, gain, time_ms): on a sound, all its voices (now and later)
    r.f("audio_sound_gain", |e, _, a| {
        let p = A(a);
        let (g, ms) = (p.num(1)? as f32, p.num_or(2, 0.0)?);
        let t = target(&p, 0)?;
        let mut m = audio::lock(e);
        match t {
            Target::Sound(s) => {
                if let Some(slot) = m.slots.get_mut(s as usize) {
                    slot.gain.set(g, ms);
                }
            }
            Target::Voice(id) => {
                if let Some(v) = m.voice_mut(id) {
                    v.gain.set(g, ms);
                }
            }
        }
        Ok(Value::Null)
    });
    r.f("audio_sound_get_gain", |e, _, a| {
        let t = target(&A(a), 0)?;
        let m = audio::lock(e);
        Ok(Value::Num(match t {
            Target::Sound(s) => m.slots.get(s as usize).map_or(0.0, |s| s.gain.cur),
            Target::Voice(id) => m.voice(id).map_or(0.0, |v| v.gain.cur),
        } as f64))
    });
    r.f("audio_sound_pitch", |e, _, a| {
        let p = A(a);
        let pitch = p.num(1)?.max(0.0) as f32;
        let t = target(&p, 0)?;
        let mut m = audio::lock(e);
        for v in m.voices.iter_mut().filter(|v| match t {
            Target::Sound(s) => v.snd == s,
            Target::Voice(id) => v.id == id,
        }) {
            v.pitch = pitch;
        }
        Ok(Value::Null)
    });
    r.f("audio_sound_get_pitch", |e, _, a| {
        let p = A(a);
        let id = match target(&p, 0)? {
            Target::Voice(id) => id,
            Target::Sound(_) => return Ok(Value::Num(1.0)),
        };
        Ok(Value::Num(audio::lock(e).voice(id).map_or(1.0, |v| v.pitch) as f64))
    });
    // extension: pan of a voice (-1 left .. 1 right)
    r.f("audio_sound_pan", |e, _, a| {
        let p = A(a);
        let pan = p.num(1)?.clamp(-1.0, 1.0) as f32;
        if let Target::Voice(id) = target(&p, 0)? {
            if let Some(v) = audio::lock(e).voice_mut(id) {
                v.pan = pan;
            }
        }
        Ok(Value::Null)
    });
    r.f("audio_sound_get_track_position", |e, _, a| {
        let Target::Voice(id) = target(&A(a), 0)? else { return Ok(Value::Num(0.0)) };
        Ok(Value::Num(audio::lock(e).position(id).unwrap_or(0.0)))
    });
    r.f("audio_sound_set_track_position", |e, _, a| {
        let p = A(a);
        let secs = p.num(1)?.max(0.0);
        let Target::Voice(id) = target(&p, 0)? else { return Ok(Value::Null) };
        if let Some(v) = audio::lock(e).voice_mut(id) {
            v.pos = secs * audio::RATE;
        }
        Ok(Value::Null)
    });
    r.f("audio_sound_length", |e, _, a| {
        let t = target(&A(a), 0)?;
        let snd = match t {
            Target::Sound(s) => s,
            Target::Voice(id) => match audio::lock(e).voice(id) {
                Some(v) => v.snd,
                None => return Ok(Value::Num(0.0)),
            },
        };
        audio::ensure_decoded(e, snd);
        Ok(Value::Num(audio::lock(e).length(snd)))
    });
    r.f("audio_sound_loop", |e, _, a| {
        let p = A(a);
        let on = p.bool_or(1, true);
        if let Target::Voice(id) = target(&p, 0)? {
            if let Some(v) = audio::lock(e).voice_mut(id) {
                v.looped = on;
            }
        }
        Ok(Value::Null)
    });
    r.f("audio_sound_get_loop", |e, _, a| {
        let Target::Voice(id) = target(&A(a), 0)? else { return Ok(Value::Bool(false)) };
        Ok(Value::Bool(audio::lock(e).voice(id).is_some_and(|v| v.looped)))
    });
    for (name, start) in [("audio_sound_loop_start", true), ("audio_sound_loop_end", false)] {
        let f: super::NativeFn = if start { |e, _, a| loop_point(e, &A(a), true) } else { |e, _, a| loop_point(e, &A(a), false) };
        r.f(name, f);
    }
    r.f("audio_master_gain", |e, _, a| {
        let p = A(a);
        let (g, ms) = (p.num(0)? as f32, p.num_or(1, 0.0)?);
        audio::lock(e).master.set(g, ms);
        Ok(Value::Null)
    });
    r.f("audio_get_master_gain", |e, _, _| Ok(Value::Num(audio::lock(e).master.cur as f64)));
    r.f("audio_group_set_gain", |e, _, a| {
        let p = A(a);
        let name = group_name(&p, 0)?;
        let (g, ms) = (p.num(1)? as f32, p.num_or(2, 0.0)?);
        let mut m = audio::lock(e);
        let i = m.group(&name);
        m.groups[i].gain.set(g, ms);
        Ok(Value::Null)
    });
    r.f("audio_group_get_gain", |e, _, a| {
        let name = group_name(&A(a), 0)?;
        let m = audio::lock(e);
        Ok(Value::Num(m.groups.iter().find(|g| g.name == name).map_or(1.0, |g| g.gain.cur) as f64))
    });
    r.f("audio_group_stop_all", |e, _, a| {
        let p = A(a);
        let name = group_name(&p, 0)?;
        let ms = p.num_or(1, 0.0)?;
        let mut m = audio::lock(e);
        let i = m.group(&name);
        m.stop_group(i, ms);
        Ok(Value::Null)
    });
    r.f("audio_sound_get_audio_group", |e, _, a| {
        let snd = A(a).asset(0, AssetKind::Sound)?;
        let m = audio::lock(e);
        let g = m.slots.get(snd as usize).map_or(0, |s| s.group);
        Ok(Value::string(m.groups[g].name.clone()))
    });
    r.f("audio_listener_position", |e, _, a| {
        let p = A(a);
        let (x, y) = (p.num(0)?, p.num(1)?);
        e.audio_mix.listener_set = true;
        audio::lock(e).listener = (x, y);
        Ok(Value::Null)
    });
    // extension: the listener follows the centre of the view again (the default)
    r.f("audio_listener_follow_view", |e, _, _| {
        e.audio_mix.listener_set = false;
        Ok(Value::Null)
    });
    r.f("audio_emitter_create", |e, _, _| {
        let mut m = audio::lock(e);
        let em = Some(Emitter { x: 0.0, y: 0.0, gain: 1.0, pitch: 1.0, falloff: Falloff::default() });
        let i = match m.emitters.iter().position(|x| x.is_none()) {
            Some(i) => {
                m.emitters[i] = em;
                i
            }
            None => {
                m.emitters.push(em);
                m.emitters.len() - 1
            }
        };
        Ok(Value::Num(i as f64))
    });
    r.f("audio_emitter_exists", |e, _, a| {
        let n = A(a).num(0)?;
        Ok(Value::Bool(n >= 0.0 && matches!(audio::lock(e).emitters.get(n as usize), Some(Some(_)))))
    });
    r.f("audio_emitter_free", |e, _, a| {
        let i = emitter_idx(e, &A(a), 0)?;
        audio::lock(e).emitters[i] = None;
        Ok(Value::Null)
    });
    r.f("audio_emitter_position", |e, _, a| {
        let p = A(a);
        let i = emitter_idx(e, &p, 0)?;
        let (x, y) = (p.num(1)?, p.num(2)?);
        if let Some(em) = audio::lock(e).emitters[i].as_mut() {
            em.x = x;
            em.y = y;
        }
        Ok(Value::Null)
    });
    r.f("audio_emitter_falloff", |e, _, a| {
        let p = A(a);
        let i = emitter_idx(e, &p, 0)?;
        let f = Falloff { reference: p.num(1)?, max: p.num(2)?, factor: p.num(3)? };
        if let Some(em) = audio::lock(e).emitters[i].as_mut() {
            em.falloff = f;
        }
        Ok(Value::Null)
    });
    r.f("audio_emitter_gain", |e, _, a| {
        let p = A(a);
        let i = emitter_idx(e, &p, 0)?;
        let g = p.num(1)? as f32;
        if let Some(em) = audio::lock(e).emitters[i].as_mut() {
            em.gain = g;
        }
        Ok(Value::Null)
    });
    r.f("audio_emitter_pitch", |e, _, a| {
        let p = A(a);
        let i = emitter_idx(e, &p, 0)?;
        let g = p.num(1)?.max(0.0) as f32;
        if let Some(em) = audio::lock(e).emitters[i].as_mut() {
            em.pitch = g;
        }
        Ok(Value::Null)
    });
    // test / debug helper: the voices playing now as structs
    r.f("audio_voices", |e, _, _| {
        let m = audio::lock(e);
        let list: Vec<Value> = m
            .voices
            .iter()
            .map(|v| {
                Value::new_struct([
                    (intern("id"), Value::Num(v.id as f64)),
                    (intern("sound"), Value::Asset(AssetKind::Sound, v.snd)),
                    (intern("gain"), Value::Num(v.gain.cur as f64)),
                    (intern("pitch"), Value::Num(v.pitch as f64)),
                    (intern("pan"), Value::Num(v.pan as f64)),
                    (intern("position"), Value::Num(v.pos / audio::RATE)),
                    (intern("loop"), Value::Bool(v.looped)),
                    (intern("paused"), Value::Bool(v.paused)),
                ])
            })
            .collect();
        Ok(Value::array(list))
    });
}

fn set_paused(e: &mut Engine, a: &A, pause: bool) -> RtResult<Value> {
    let t = target(a, 0)?;
    for v in audio::lock(e).voices.iter_mut().filter(|v| match t {
        Target::Sound(s) => v.snd == s,
        Target::Voice(id) => v.id == id,
    }) {
        v.paused = pause;
    }
    Ok(Value::Null)
}

fn loop_point(e: &mut Engine, a: &A, start: bool) -> RtResult<Value> {
    let snd = a.asset(0, AssetKind::Sound)?;
    let secs = a.num(1)?.max(0.0);
    audio::ensure_decoded(e, snd);
    let mut m = audio::lock(e);
    if let Some(s) = m.slots.get_mut(snd as usize) {
        let frames = s.frames();
        let f = ((secs * audio::RATE).round() as usize).min(frames);
        if start {
            s.meta_loop.0 = Some(secs);
            s.loop_frames.0 = f;
        } else {
            s.meta_loop.1 = Some(secs);
            s.loop_frames.1 = f;
        }
        if s.loop_frames.1 <= s.loop_frames.0 {
            s.loop_frames = (0, frames);
        }
    }
    Ok(Value::Null)
}

pub(crate) fn constants(c: &mut HashMap<Sym, Value>) {
    c.insert(intern("audiogroup_default"), Value::string("default".to_string()));
}

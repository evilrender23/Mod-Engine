//! Sequences: playing them in the room (layer_sequence_*) and reading them.

use super::{Reg, A};
use crate::engine::Engine;
use crate::sequence::{self, SeqInst};
use modscript::{intern, AssetKind, RtError, RtResult, Value};

fn el(e: &Engine, a: &A, i: usize) -> RtResult<u32> {
    let id = a.num(i)? as u32;
    if e.seqs.get(id).is_none() {
        return Err(RtError::new(format!("sequence element {id} does not exist")));
    }
    Ok(id)
}

/// The element is the last argument: `layer_sequence_exists(layer, el)` or `(el)`.
fn el_last(e: &Engine, a: &A) -> RtResult<u32> {
    el(e, a, a.len().saturating_sub(1))
}

fn set(e: &mut Engine, a: &[Value], f: impl FnOnce(&mut SeqInst, f64)) -> RtResult<Value> {
    let id = el(e, &A(a), 0)?;
    let v = A(a).num(1)?;
    if let Some(s) = e.seqs.get_mut(id) {
        f(s, v);
    }
    sequence::sync_objects(e, id);
    Ok(Value::Null)
}

fn get(e: &Engine, a: &[Value], f: impl FnOnce(&SeqInst) -> f64) -> RtResult<Value> {
    let id = el(e, &A(a), 0)?;
    Ok(Value::Num(e.seqs.get(id).map(f).unwrap_or(0.0)))
}

/// Depth of a layer given by name (or a number, used as the depth itself).
fn layer_depth(e: &Engine, v: &Value) -> RtResult<(f64, Option<String>)> {
    match v {
        Value::Num(d) => Ok((*d, None)),
        Value::Str(n) => match e.layers.iter().find(|l| *l.name == **n) {
            Some(l) => Ok((l.depth, Some(l.name.clone()))),
            None => {
                let names: Vec<&str> = e.layers.iter().map(|l| l.name.as_str()).collect();
                Err(RtError::new(format!("layer \"{n}\" does not exist in this room (layers: {})", names.join(", "))))
            }
        },
        other => Err(RtError::new(format!("layer must be a layer name or a depth, got {}", other.type_name()))),
    }
}

pub(super) fn register(r: &mut Reg) {
    r.f("layer_sequence_create", |e, _, a| {
        let p = A(a);
        let (depth, layer) = layer_depth(e, p.val(0)?)?;
        let seq = p.asset(3, AssetKind::Sequence)?;
        Ok(Value::Num(sequence::create(e, seq, p.num(1)?, p.num(2)?, depth, layer) as f64))
    });
    // shortcut: sequence_create(seq, x, y, [depth])
    r.f("sequence_create", |e, _, a| {
        let p = A(a);
        let seq = p.asset(0, AssetKind::Sequence)?;
        Ok(Value::Num(sequence::create(e, seq, p.num(1)?, p.num(2)?, p.num_or(3, 0.0)?, None) as f64))
    });
    r.f("layer_sequence_destroy", |e, _, a| {
        let id = el(e, &A(a), 0)?;
        sequence::destroy(e, id);
        Ok(Value::Null)
    });
    r.f("layer_sequence_exists", |e, _, a| Ok(Value::Bool(el_last(e, &A(a)).is_ok())));
    r.f("layer_sequence_x", |e, _, a| set(e, a, |s, v| s.x = v));
    r.f("layer_sequence_y", |e, _, a| set(e, a, |s, v| s.y = v));
    r.f("layer_sequence_angle", |e, _, a| set(e, a, |s, v| s.angle = v));
    r.f("layer_sequence_xscale", |e, _, a| set(e, a, |s, v| s.xscale = v));
    r.f("layer_sequence_yscale", |e, _, a| set(e, a, |s, v| s.yscale = v));
    r.f("layer_sequence_speedscale", |e, _, a| set(e, a, |s, v| s.speed = v));
    r.f("layer_sequence_headdir", |e, _, a| set(e, a, |s, v| s.dir = if v < 0.0 { -1.0 } else { 1.0 }));
    r.f("layer_sequence_headpos", |e, _, a| {
        let id = el(e, &A(a), 0)?;
        let len = e.sequences[e.seqs.get(id).map(|s| s.seq).unwrap_or(0) as usize].data.length;
        set(e, a, |s, v| {
            s.head = v.clamp(0.0, len);
            s.finished = false;
        })
    });
    r.f("layer_sequence_play", |e, _, a| {
        let id = el(e, &A(a), 0)?;
        let len = e.seqs.get(id).map(|s| e.sequences[s.seq as usize].data.length).unwrap_or(0.0);
        if let Some(s) = e.seqs.get_mut(id) {
            s.paused = false;
            if s.finished {
                s.finished = false;
                s.head = if s.dir < 0.0 { len } else { 0.0 };
            }
        }
        sequence::sync_objects(e, id);
        Ok(Value::Null)
    });
    r.f("layer_sequence_pause", |e, _, a| {
        let id = el(e, &A(a), 0)?;
        if let Some(s) = e.seqs.get_mut(id) {
            s.paused = true;
        }
        Ok(Value::Null)
    });
    r.f("layer_sequence_is_paused", |e, _, a| Ok(Value::Bool(e.seqs.get(el(e, &A(a), 0)?).is_some_and(|s| s.paused))));
    r.f("layer_sequence_is_finished", |e, _, a| Ok(Value::Bool(e.seqs.get(el(e, &A(a), 0)?).is_some_and(|s| s.finished))));
    r.f("layer_sequence_get_x", |e, _, a| get(e, a, |s| s.x));
    r.f("layer_sequence_get_y", |e, _, a| get(e, a, |s| s.y));
    r.f("layer_sequence_get_angle", |e, _, a| get(e, a, |s| s.angle));
    r.f("layer_sequence_get_xscale", |e, _, a| get(e, a, |s| s.xscale));
    r.f("layer_sequence_get_yscale", |e, _, a| get(e, a, |s| s.yscale));
    r.f("layer_sequence_get_headpos", |e, _, a| get(e, a, |s| s.head));
    r.f("layer_sequence_get_headdir", |e, _, a| get(e, a, |s| s.dir));
    r.f("layer_sequence_get_speedscale", |e, _, a| get(e, a, |s| s.speed));
    r.f("layer_sequence_get_length", |e, _, a| {
        let id = el(e, &A(a), 0)?;
        Ok(Value::Num(e.seqs.get(id).map(|s| e.sequences[s.seq as usize].data.length).unwrap_or(0.0)))
    });
    r.f("layer_sequence_get_sequence", |e, _, a| {
        let id = el(e, &A(a), 0)?;
        Ok(e.seqs.get(id).map(|s| Value::Asset(AssetKind::Sequence, s.seq)).unwrap_or(Value::Null))
    });
    r.f("layer_sequence_get_track_instance", |e, _, a| {
        let id = el(e, &A(a), 0)?;
        let name = A(a).str(1)?;
        let Some(s) = e.seqs.get(id) else { return Ok(Value::Null) };
        let def = &e.sequences[s.seq as usize];
        let ti = def.data.tracks.iter().position(|t| *t.name == *name).ok_or_else(|| RtError::new(format!("sequence '{}' has no track \"{name}\"", def.name)))?;
        Ok(s.track_insts[ti].filter(|&i| e.alive(i)).map(Value::Inst).unwrap_or(Value::Null))
    });
    r.f("sequence_exists", |e, _, a| Ok(Value::Bool(matches!(A(a).val(0)?, Value::Asset(AssetKind::Sequence, s) if (*s as usize) < e.sequences.len()))));
    r.f("sequence_get", |e, _, a| {
        let s = A(a).asset(0, AssetKind::Sequence)? as usize;
        let d = &e.sequences.get(s).ok_or_else(|| RtError::new("the sequence does not exist"))?;
        let tracks = d
            .data
            .tracks
            .iter()
            .map(|t| {
                Value::new_struct([
                    (intern("name"), Value::str(&t.name)),
                    (intern("type"), Value::str(t.kind.name())),
                    (intern("asset"), Value::str(&t.asset)),
                    (intern("start"), Value::Num(t.start)),
                    (intern("end"), Value::Num(t.end_or(d.data.length))),
                    (intern("keys"), Value::Num(t.keys.len() as f64)),
                ])
            })
            .collect();
        Ok(Value::new_struct([
            (intern("name"), Value::str(&d.name)),
            (intern("length"), Value::Num(d.data.length)),
            (intern("playback_speed"), Value::Num(d.data.fps)),
            (intern("playback"), Value::str(d.data.playback.name())),
            (intern("xorigin"), Value::Num(d.data.origin[0])),
            (intern("yorigin"), Value::Num(d.data.origin[1])),
            (intern("tracks"), Value::array(tracks)),
            (intern("moments"), Value::Num(d.moments.len() as f64)),
        ]))
    });
}

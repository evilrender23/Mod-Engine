//! Animation curves as GameMaker exposes them: structs with channels and points.
//!
//! `animcurve_get(curve)` → `{ name, channels: [channel...] }`
//! channel → `{ name, type: "linear"|"smooth"|"bezier", iterations, points: [{ posx, value }] }`

use super::{Reg, A};
use crate::animcurve::{AnimCurve, Channel, CurvePoint, CurveType};
use crate::engine::Engine;
use modscript::{intern, AssetKind, RtError, RtResult, StructRef, Value};

fn curve_arg(e: &Engine, a: &A, i: usize) -> RtResult<usize> {
    let c = a.asset(i, AssetKind::AnimCurve)? as usize;
    e.curves.get(c).map(|_| c).ok_or_else(|| RtError::new("the animation curve does not exist"))
}

fn point_struct(p: &CurvePoint) -> Value {
    let mut f = vec![(intern("posx"), Value::Num(p.x)), (intern("value"), Value::Num(p.y))];
    if let (Some(i), Some(o)) = (p.h_in, p.h_out) {
        f.push((intern("handles"), Value::array(vec![Value::Num(i.0), Value::Num(i.1), Value::Num(o.0), Value::Num(o.1)])));
    }
    Value::new_struct(f)
}

pub fn channel_struct(c: &Channel) -> Value {
    Value::new_struct([
        (intern("name"), Value::str(&c.name)),
        (intern("type"), Value::str(c.kind.name())),
        (intern("iterations"), Value::Num(c.iterations as f64)),
        (intern("points"), Value::array(c.points.iter().map(point_struct).collect())),
    ])
}

fn curve_struct(c: &AnimCurve) -> Value {
    Value::new_struct([(intern("name"), Value::str(&c.name)), (intern("channels"), Value::array(c.channels.iter().map(channel_struct).collect()))])
}

fn field(s: &StructRef, name: &str) -> Value {
    s.borrow().fields.get(&intern(name)).cloned().unwrap_or(Value::Null)
}

/// Reads a channel struct (as returned by animcurve_get_channel or built by hand).
fn channel_from(s: &StructRef) -> RtResult<Channel> {
    let kind = match field(s, "type") {
        Value::Str(t) => CurveType::parse(&t).ok_or_else(|| RtError::new(format!("unknown curve type \"{t}\" (linear, smooth or bezier)")))?,
        Value::Num(n) => match n as i64 {
            1 => CurveType::Smooth,
            2 => CurveType::Bezier,
            _ => CurveType::Linear,
        },
        _ => CurveType::Linear,
    };
    let mut pts = vec![];
    if let Value::Array(arr) = field(s, "points") {
        for v in arr.borrow().iter() {
            let Value::Struct(p) = v else { return Err(RtError::new("channel points must be structs { posx, value }")) };
            let num = |n: &str| field(p, n).as_num().unwrap_or(0.0);
            let mut pt = CurvePoint::new(num("posx"), num("value"));
            if let Value::Array(h) = field(p, "handles") {
                let h: Vec<f64> = h.borrow().iter().map(|v| v.as_num().unwrap_or(0.0)).collect();
                if h.len() == 4 {
                    pt.h_in = Some((h[0], h[1]));
                    pt.h_out = Some((h[2], h[3]));
                }
            }
            pts.push(pt);
        }
    }
    let name = field(s, "name").to_string();
    Ok(Channel::new(&name, kind, pts))
}

/// Channel of an asset curve by name or index.
fn channel_of<'a>(e: &'a Engine, c: usize, which: &Value) -> RtResult<&'a Channel> {
    let curve = &e.curves[c];
    match which {
        Value::Str(n) => curve.channel(n).ok_or_else(|| RtError::new(format!("curve '{}' has no channel \"{n}\"", curve.name))),
        Value::Num(n) => curve.channels.get(*n as usize).ok_or_else(|| RtError::new(format!("curve '{}' has no channel {n}", curve.name))),
        other => Err(RtError::new(format!("channel must be a name or an index, got {}", other.type_name()))),
    }
}

pub(super) fn register(r: &mut Reg) {
    r.f("animcurve_get", |e, _, a| Ok(curve_struct(&e.curves[curve_arg(e, &A(a), 0)?])));
    r.f("animcurve_exists", |e, _, a| Ok(Value::Bool(matches!(A(a).val(0)?, Value::Asset(AssetKind::AnimCurve, c) if (*c as usize) < e.curves.len()))));
    r.f("animcurve_get_channel", |e, _, a| {
        let c = curve_arg(e, &A(a), 0)?;
        Ok(channel_struct(channel_of(e, c, A(a).val(1)?)?))
    });
    r.f("animcurve_get_channel_index", |e, _, a| {
        let c = curve_arg(e, &A(a), 0)?;
        let n = A(a).str(1)?;
        Ok(Value::Num(e.curves[c].channels.iter().position(|ch| *ch.name == *n).map(|i| i as f64).unwrap_or(-1.0)))
    });
    r.f("animcurve_channel_evaluate", |_, _, a| {
        let ch = channel_from(&A(a).strukt(0)?)?;
        Ok(Value::Num(ch.eval(A(a).num(1)?)))
    });
    // Faster shortcut without building structs: animcurve_evaluate(curve, "y", x)
    r.f("animcurve_evaluate", |e, _, a| {
        let c = curve_arg(e, &A(a), 0)?;
        let x = A(a).num(2)?;
        Ok(Value::Num(channel_of(e, c, A(a).val(1)?)?.eval(x)))
    });
    r.f("animcurve_create", |_, _, _| Ok(Value::new_struct([(intern("name"), Value::str("")), (intern("channels"), Value::array(vec![]))])));
    r.f("animcurve_channel_new", |_, _, _| Ok(channel_struct(&Channel::new("", CurveType::Linear, vec![]))));
    r.f("animcurve_point_new", |_, _, _| Ok(point_struct(&CurvePoint::new(0.0, 0.0))));
    r.f("animcurve_destroy", |_, _, _| Ok(Value::Null));
}

//! Tweens and easing: `ease`, `tween`, `tween_to`, `tween_from`, `tween_chain`,
//! `tween_cancel*`, `tween_done`, `tween_on_complete`, yoyo / loops / unscaled and the
//! global tween speed. See `feel.rs`.

use super::{err, Reg, A};
use crate::feel::{self, Ease, Target, TweenSpec};
use modscript::{intern, AssetKind, CallCx, RtResult, Value};

fn target(a: &A, i: usize, f: &str) -> RtResult<Target> {
    match a.val(i)? {
        Value::Inst(id) => Ok(Target::Inst(*id)),
        Value::Struct(s) => Ok(Target::Struct(s.clone())),
        other => err(format!("{f}(): the target must be an instance (self, id) or a struct, not {}", other.type_name())),
    }
}

/// An easing argument: a name, an animation curve or null (default `quad_out`).
pub(crate) fn ease_arg(a: &A, i: usize) -> RtResult<Ease> {
    match a.opt(i) {
        None => Ok(Ease::Named("quad_out".into())),
        Some(Value::Str(s)) => {
            if feel::ease(s, 0.5).is_none() {
                return err(format!("unknown easing '{s}' (linear, smooth, quad_in, cubic_out, back_inout, elastic_out, bounce_out... or an animation curve)"));
            }
            Ok(Ease::Named(s.to_string()))
        }
        Some(Value::Asset(AssetKind::AnimCurve, c)) => Ok(Ease::Curve(*c)),
        Some(other) => err(format!("the easing must be a name like \"quad_out\" or an animation curve, not {}", other.type_name())),
    }
}

fn fields(a: &A, i: usize) -> RtResult<Vec<(modscript::Sym, f64)>> {
    let s = a.strukt(i)?;
    let s = s.borrow();
    let mut out = vec![];
    for (k, v) in s.fields.iter() {
        match v.as_num() {
            Some(n) => out.push((*k, n)),
            None => return err(format!("'{k}' must be a number, got {}", v.type_name())),
        }
    }
    Ok(out)
}

struct Opts {
    delay: f64,
    yoyo: bool,
    loops: i64,
    unscaled: bool,
    on_complete: Option<Value>,
}

fn opts(a: &A, i: usize) -> RtResult<Opts> {
    let mut o = Opts { delay: 0.0, yoyo: false, loops: 0, unscaled: false, on_complete: None };
    let Some(v) = a.opt(i) else { return Ok(o) };
    let Value::Struct(s) = v else { return err(format!("argument #{} must be an options struct {{ delay, yoyo, loops, unscaled, on_complete }}", i + 1)) };
    for (k, v) in s.borrow().fields.iter() {
        let num = || v.as_num().ok_or_else(|| modscript::RtError::new(format!("option '{k}' must be a number")));
        match &*k.name() {
            "delay" => o.delay = num()?,
            "yoyo" => o.yoyo = v.truthy(),
            "loops" | "loop" => {
                o.loops = match v {
                    Value::Bool(true) => -1,
                    Value::Bool(false) => 0,
                    _ => num()?.floor() as i64,
                }
            }
            "unscaled" => o.unscaled = v.truthy(),
            "on_complete" => o.on_complete = Some(v.clone()),
            other => return err(format!("unknown tween option '{other}' (delay, yoyo, loops, unscaled, on_complete)")),
        }
    }
    Ok(o)
}

fn spec(target: Target, values: Vec<(modscript::Sym, f64)>, from: bool, dur: f64, ease: Ease, o: Opts, cx: &CallCx) -> TweenSpec {
    TweenSpec {
        target,
        values,
        from,
        dur,
        ease,
        delay: o.delay,
        yoyo: o.yoyo,
        loops: o.loops,
        unscaled: o.unscaled,
        on_complete: o.on_complete.map(|f| (f, cx.clone())),
    }
}

fn id_arg(a: &A, i: usize) -> RtResult<u32> {
    Ok(a.num(i)?.max(0.0) as u32)
}

fn with_tween(e: &mut crate::engine::Engine, a: &A, f: impl FnOnce(&mut feel::Tween)) -> RtResult<Value> {
    let id = id_arg(a, 0)?;
    match e.feel.tweens.get_mut(id) {
        Some(t) => {
            f(t);
            Ok(Value::Bool(true))
        }
        None => Ok(Value::Bool(false)),
    }
}

pub(super) fn register(r: &mut Reg) {
    r.f("ease", |_, _, a| {
        let p = A(a);
        let t = p.num(1)?;
        match p.val(0)? {
            Value::Str(s) => match feel::ease(s, t) {
                Some(v) => Ok(Value::Num(v)),
                None => err(format!("unknown easing '{s}'")),
            },
            other => err(format!("ease(name, t): name must be a string like \"quad_out\", not {}", other.type_name())),
        }
    });
    r.f("ease_names", |_, _, _| Ok(Value::array(feel::ease_names().into_iter().map(Value::string).collect())));
    // tween(target, field, to, seconds, [ease], [delay])
    r.f("tween", |e, cx, a| {
        let p = A(a);
        let t = target(&p, 0, "tween")?;
        let field = intern(&p.str(1)?);
        let o = Opts { delay: p.num_or(5, 0.0)?, yoyo: false, loops: 0, unscaled: false, on_complete: None };
        let id = feel::create(e, spec(t, vec![(field, p.num(2)?)], false, p.num(3)?, ease_arg(&p, 4)?, o, cx))?;
        Ok(Value::Num(id as f64))
    });
    // tween_to(target, {field: to, ...}, seconds, [ease], [options])
    r.f("tween_to", |e, cx, a| {
        let p = A(a);
        let t = target(&p, 0, "tween_to")?;
        let id = feel::create(e, spec(t, fields(&p, 1)?, false, p.num(2)?, ease_arg(&p, 3)?, opts(&p, 4)?, cx))?;
        Ok(Value::Num(id as f64))
    });
    // tween_from(target, {field: start, ...}, seconds, [ease], [options]): from there to now
    r.f("tween_from", |e, cx, a| {
        let p = A(a);
        let t = target(&p, 0, "tween_from")?;
        let id = feel::create(e, spec(t, fields(&p, 1)?, true, p.num(2)?, ease_arg(&p, 3)?, opts(&p, 4)?, cx))?;
        Ok(Value::Num(id as f64))
    });
    r.f("tween_done", |e, _, a| Ok(Value::Bool(e.feel.tweens.get(id_arg(&A(a), 0)?).is_none())));
    r.f("tween_exists", |e, _, a| Ok(Value::Bool(e.feel.tweens.get(id_arg(&A(a), 0)?).is_some())));
    r.f("tween_remaining", |e, _, a| {
        let id = id_arg(&A(a), 0)?;
        Ok(Value::Num(e.feel.tweens.get(id).map(|t| t.remaining()).unwrap_or(0.0)))
    });
    // tween_cancel(id, [jump_to_end])
    r.f("tween_cancel", |e, _, a| {
        let p = A(a);
        let id = id_arg(&p, 0)?;
        if e.feel.tweens.get(id).is_none() {
            return Ok(Value::Bool(false));
        }
        if p.bool_or(1, false) {
            feel::finish_values(e, id);
        }
        e.feel.tweens.list.retain(|t| t.id != id);
        Ok(Value::Bool(true))
    });
    r.f("tween_cancel_all", |e, _, a| {
        let p = A(a);
        let before = e.feel.tweens.list.len();
        match p.opt(0) {
            None => e.feel.tweens.list.clear(),
            Some(_) => {
                let t = target(&p, 0, "tween_cancel_all")?;
                e.feel.tweens.list.retain(|x| !x.target.same(&t));
            }
        }
        Ok(Value::Num((before - e.feel.tweens.list.len()) as f64))
    });
    r.f("tween_count", |e, _, a| {
        let p = A(a);
        Ok(Value::Num(match p.opt(0) {
            None => e.feel.tweens.list.len(),
            Some(_) => {
                let t = target(&p, 0, "tween_count")?;
                e.feel.tweens.list.iter().filter(|x| x.target.same(&t)).count()
            }
        } as f64))
    });
    r.f("tween_on_complete", |e, cx, a| {
        let p = A(a);
        let f = match p.val(1)? {
            f @ (Value::Func(_) | Value::Native(_)) => f.clone(),
            o => return err(format!("tween_on_complete(id, fn): fn must be a function, not {}", o.type_name())),
        };
        let cx = cx.clone();
        with_tween(e, &p, move |t| t.on_complete = Some((f, cx)))
    });
    // tween_chain(a, b, c...) or tween_chain([a, b, c]): each waits for the previous one
    r.f("tween_chain", |e, _, a| {
        let ids: Vec<u32> = match a.first() {
            Some(Value::Array(arr)) => arr.borrow().iter().filter_map(|v| v.as_num()).map(|n| n.max(0.0) as u32).collect(),
            _ => a.iter().filter_map(|v| v.as_num()).map(|n| n.max(0.0) as u32).collect(),
        };
        for w in ids.windows(2) {
            if let Some(t) = e.feel.tweens.get_mut(w[1]) {
                t.after = Some(w[0]);
            }
        }
        Ok(ids.last().map(|i| Value::Num(*i as f64)).unwrap_or(Value::Null))
    });
    r.f("tween_set_yoyo", |e, _, a| {
        let p = A(a);
        let v = p.bool_or(1, true);
        with_tween(e, &p, |t| t.yoyo = v)
    });
    r.f("tween_set_loops", |e, _, a| {
        let p = A(a);
        let n = p.num(1)?.floor().max(-1.0) as i64;
        with_tween(e, &p, |t| t.loops = n)
    });
    r.f("tween_set_unscaled", |e, _, a| {
        let p = A(a);
        let v = p.bool_or(1, true);
        with_tween(e, &p, |t| t.unscaled = v)
    });
    r.f("tween_pause", |e, _, a| with_tween(e, &A(a), |t| t.paused = true));
    r.f("tween_resume", |e, _, a| with_tween(e, &A(a), |t| t.paused = false));
    r.f("tween_set_speed", |e, _, a| {
        let s = A(a).num(0)?;
        if !(s >= 0.0) {
            return err("tween_set_speed(mult): the multiplier must be 0 or more");
        }
        e.feel.tweens.speed = s;
        Ok(Value::Null)
    });
    r.f("tween_get_speed", |e, _, _| Ok(Value::Num(e.feel.tweens.speed())));
}

//! Timelines: running them on instances and building them at runtime.

use super::{me, Reg, A};
use crate::engine::Engine;
use crate::timeline::{Action, TimelineDef};
use modscript::{AssetKind, RtError, RtResult, Value};

fn tl_arg(e: &Engine, a: &A, i: usize) -> RtResult<usize> {
    let t = a.asset(i, AssetKind::Timeline)? as usize;
    match e.timelines.get(t) {
        Some(d) if d.alive => Ok(t),
        _ => Err(RtError::new("the timeline was deleted")),
    }
}

pub(super) fn register(r: &mut Reg) {
    r.f("timeline_start", |e, cx, a| {
        let p = A(a);
        let i = me(e, cx)?;
        let t = tl_arg(e, &p, 0)? as u32;
        let (speed, looping) = (p.num_or(1, 1.0)?, p.bool_or(2, false));
        let run = &mut e.insts[i].timeline;
        run.index = Some(t);
        run.position = 0.0;
        run.speed = speed;
        run.looping = looping;
        run.running = true;
        Ok(Value::Null)
    });
    r.f("timeline_stop", |e, cx, _| {
        let i = me(e, cx)?;
        let run = &mut e.insts[i].timeline;
        run.running = false;
        run.position = 0.0;
        Ok(Value::Null)
    });
    r.f("timeline_add", |e, _, _| {
        let i = e.timelines.len();
        e.timelines.push(TimelineDef::empty(&format!("__timeline{i}")));
        Ok(Value::Asset(AssetKind::Timeline, i as u32))
    });
    r.f("timeline_delete", |e, _, a| {
        let t = tl_arg(e, &A(a), 0)?;
        e.timelines[t].alive = false;
        e.timelines[t].moments.clear();
        Ok(Value::Null)
    });
    r.f("timeline_exists", |e, _, a| {
        Ok(Value::Bool(matches!(A(a).val(0)?, Value::Asset(AssetKind::Timeline, t) if e.timelines.get(*t as usize).is_some_and(|d| d.alive))))
    });
    r.f("timeline_get_name", |e, _, a| Ok(Value::str(&e.timelines[tl_arg(e, &A(a), 0)?].name)));
    r.f("timeline_moment_add_script", |e, _, a| {
        let p = A(a);
        let t = tl_arg(e, &p, 0)?;
        let step = p.num(1)?.floor().max(0.0);
        let f = p.val(2)?.clone();
        if !matches!(f, Value::Func(_) | Value::Native(_)) {
            return Err(RtError::new(format!("argument #3 must be a function, got {}", f.type_name())));
        }
        e.timelines[t].add(step, Action::Func(f));
        Ok(Value::Null)
    });
    r.f("timeline_moment_clear", |e, _, a| {
        let t = tl_arg(e, &A(a), 0)?;
        let step = A(a).num(1)?.floor();
        e.timelines[t].moments.retain(|m| m.step != step);
        Ok(Value::Null)
    });
    r.f("timeline_clear", |e, _, a| {
        let t = tl_arg(e, &A(a), 0)?;
        e.timelines[t].moments.clear();
        Ok(Value::Null)
    });
    r.f("timeline_size", |e, _, a| Ok(Value::Num(e.timelines[tl_arg(e, &A(a), 0)?].moments.len() as f64)));
    r.f("timeline_max_moment", |e, _, a| Ok(Value::Num(e.timelines[tl_arg(e, &A(a), 0)?].max_moment())));
}

//! coroutine(), wait()/yield() outside coroutines, call_later() / call_cancel().

use super::{err, A, Reg};
use crate::coroutine as co;
use modscript::{RtError, Value};

fn handle(a: &A, i: usize) -> Result<u32, RtError> {
    Ok(a.num(i)?.max(0.0) as u32)
}

pub(crate) fn register(r: &mut Reg) {
    r.f("coroutine", |e, cx, a| {
        let f = A(a).val(0)?.clone();
        let id = co::start(e, &f, cx, a[1..].to_vec())?;
        Ok(Value::Num(id as f64))
    });
    r.f("coroutine_running", |e, _, a| Ok(Value::Bool(e.coros.running(handle(&A(a), 0)?))));
    r.f("coroutine_stop", |e, _, a| {
        let id = handle(&A(a), 0)?;
        Ok(Value::Bool(co::stop(e, id)))
    });
    r.f("coroutine_stop_all", |e, _, a| {
        let owner = match A(a).opt(0) {
            None => None,
            Some(Value::Inst(id)) => Some(*id),
            Some(o) => return err(format!("coroutine_stop_all([instancia]): se esperaba una instancia, no {}", o.type_name())),
        };
        co::stop_all(e, owner);
        Ok(Value::Null)
    });
    r.f("coroutine_pause", |e, _, a| {
        let id = handle(&A(a), 0)?;
        Ok(Value::Bool(co::set_paused(e, id, true)))
    });
    r.f("coroutine_resume", |e, _, a| {
        let id = handle(&A(a), 0)?;
        Ok(Value::Bool(co::set_paused(e, id, false)))
    });
    r.f("coroutine_count", |e, _, a| {
        let owner = match A(a).opt(0) {
            None => None,
            Some(Value::Inst(id)) => Some(*id),
            Some(Value::All) => None,
            Some(o) => return err(format!("coroutine_count([instancia]): se esperaba una instancia, no {}", o.type_name())),
        };
        Ok(Value::Num(e.coros.count(owner) as f64))
    });
    for name in modscript::coro::SUSPEND_NAMES {
        r.f(name, |_, _, _| {
            err("wait()/yield()/wait_seconds()/wait_until() solo funcionan como instrucción del cuerpo de una corrutina: coroutine(function() { ... wait(10) ... })")
        });
    }
    // GameMaker: call_later(period, units, callback, [loop])
    r.f("call_later", |e, cx, a| {
        let p = A(a);
        let period = p.num(0)?;
        let units = p.num(1)?;
        let f = match p.val(2)? {
            f @ (Value::Func(_) | Value::Native(_)) => f.clone(),
            o => return err(format!("call_later(): el argumento #3 debe ser una función, no {}", o.type_name())),
        };
        let steps = if units == UNITS_SECONDS { period * e.step_rate() } else { period };
        let id = co::call_later(e, steps.round().max(1.0) as u64, f, cx.clone(), p.bool_or(3, false));
        Ok(Value::Num(id as f64))
    });
    r.f("call_cancel", |e, _, a| {
        let id = handle(&A(a), 0)?;
        Ok(Value::Bool(co::stop(e, id)))
    });
}

/// `time_source_units_seconds` (GameMaker); `time_source_units_frames` is 1.
pub(crate) const UNITS_SECONDS: f64 = 0.0;

pub(crate) fn constants(c: &mut std::collections::HashMap<modscript::Sym, Value>) {
    c.insert(modscript::intern("time_source_units_seconds"), Value::Num(UNITS_SECONDS));
    c.insert(modscript::intern("time_source_units_frames"), Value::Num(1.0));
}

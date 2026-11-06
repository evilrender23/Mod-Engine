//! actionq_create / actionq_push / actionq_set_speed ...: presentation queues for the
//! model/view split (see `crate::actionq`).

use super::{err, A, Reg};
use crate::actionq::{self as aq, Item};
use crate::engine::Engine;
use modscript::{RtResult, Value};

fn handle(a: &A, i: usize) -> RtResult<usize> {
    let n = a.num(i)?;
    if n < 0.0 || n.fract() != 0.0 {
        return err(format!("{} no es una cola de actionq_create()", Value::Num(n)));
    }
    Ok(n as usize)
}

fn queue<'a>(e: &'a mut Engine, a: &A, i: usize) -> RtResult<(usize, &'a mut aq::Queue)> {
    let h = handle(a, i)?;
    match e.actionqs.get_mut(h) {
        Some(q) => Ok((h, q)),
        None => err(format!("{h} no es una cola de actionq_create() (¿ya destruida?)")),
    }
}

fn func(v: &Value, what: &str) -> RtResult<Value> {
    match v {
        Value::Func(_) | Value::Native(_) => Ok(v.clone()),
        o => err(format!("{what}: se esperaba una función, no {}", o.type_name())),
    }
}

pub(crate) fn register(r: &mut Reg) {
    // actionq_create([auto = true]): auto queues play by themselves every step
    r.f("actionq_create", |e, _, a| Ok(Value::Num(e.actionqs.create(A(a).bool_or(0, true)) as f64)));
    r.f("actionq_destroy", |e, _, a| {
        let h = handle(&A(a), 0)?;
        if let Some(q) = e.actionqs.list.get_mut(h) {
            *q = None;
        }
        Ok(Value::Null)
    });
    r.f("actionq_exists", |e, _, a| {
        let n = A(a).num(0)?;
        Ok(Value::Bool(n >= 0.0 && e.actionqs.get_mut(n as usize).is_some()))
    });
    // actionq_push(q, fn, args...): plays fn(args...) as a coroutine after the previous ones
    r.f("actionq_push", |e, cx, a| {
        let p = A(a);
        let f = func(p.val(1)?, "actionq_push(cola, función, args...)")?;
        let args = a.get(2..).map(|s| s.to_vec()).unwrap_or_default();
        queue(e, &p, 0)?.1.items.push_back(Item::Call(f, args, cx.clone()));
        Ok(Value::Null)
    });
    // actionq_push_next(q, fn, args...): plays right after the current action (reactions)
    r.f("actionq_push_next", |e, cx, a| {
        let p = A(a);
        let f = func(p.val(1)?, "actionq_push_next(cola, función, args...)")?;
        let args = a.get(2..).map(|s| s.to_vec()).unwrap_or_default();
        queue(e, &p, 0)?.1.push_next(Item::Call(f, args, cx.clone()));
        Ok(Value::Null)
    });
    // actionq_push_wait(q, seconds)
    r.f("actionq_push_wait", |e, _, a| {
        let p = A(a);
        let steps = (p.num(1)? * e.step_rate()).round().max(0.0) as u64;
        queue(e, &p, 0)?.1.items.push_back(Item::Wait(steps));
        Ok(Value::Null)
    });
    // actionq_push_parallel(q, [fn, fn, [fn, arg...]]): all start together; the queue
    // goes on when the last one ends
    r.f("actionq_push_parallel", |e, cx, a| {
        let p = A(a);
        let list = p.array(1)?;
        let mut jobs = vec![];
        for v in list.borrow().iter() {
            match v {
                Value::Array(x) => {
                    let x = x.borrow();
                    let f = func(x.first().unwrap_or(&Value::Null), "actionq_push_parallel: cada [función, args...]")?;
                    jobs.push((f, x[1..].to_vec()));
                }
                f => jobs.push((func(f, "actionq_push_parallel(cola, [funciones])")?, vec![])),
            }
        }
        queue(e, &p, 0)?.1.items.push_back(Item::Parallel(jobs, cx.clone()));
        Ok(Value::Null)
    });
    // actionq_update(q): one step at the queue's speed (for queues with auto = false)
    r.f("actionq_update", |e, _, a| {
        let (h, _) = queue(e, &A(a), 0)?;
        aq::update(e, h);
        Ok(Value::Null)
    });
    // actionq_finish(q): plays everything queued at once (a "skip" button)
    r.f("actionq_finish", |e, _, a| {
        let (h, _) = queue(e, &A(a), 0)?;
        aq::finish(e, h);
        Ok(Value::Null)
    });
    r.f("actionq_busy", |e, _, a| Ok(Value::Bool(queue(e, &A(a), 0)?.1.busy())));
    r.f("actionq_size", |e, _, a| Ok(Value::Num(queue(e, &A(a), 0)?.1.size() as f64)));
    r.f("actionq_clear", |e, _, a| {
        queue(e, &A(a), 0)?.1.clear();
        Ok(Value::Null)
    });
    // actionq_set_speed(q, mult): 1 normal, 2/4 faster, infinity instant, 0 paused
    r.f("actionq_set_speed", |e, _, a| {
        let p = A(a);
        let s = p.num(1)?;
        if s.is_nan() || s < 0.0 {
            return err("la velocidad debe ser 0 (pausa), un multiplicador positivo o infinity (instantáneo)");
        }
        queue(e, &p, 0)?.1.speed = s;
        Ok(Value::Null)
    });
    r.f("actionq_get_speed", |e, _, a| Ok(Value::Num(queue(e, &A(a), 0)?.1.speed)));
}

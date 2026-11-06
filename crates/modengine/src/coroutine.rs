//! Coroutines (`coroutine(fn)`, `wait(n)`, `yield()`) and delayed calls (`call_later`).
//!
//! A coroutine runs right away until its first `wait`/`yield`, and is then resumed once
//! per step, right after the `step` event of all instances, in creation order. One that
//! belongs to an instance (it was created from that instance's code) stops when the
//! instance is destroyed or leaves the room; one created from global code lives until it
//! ends, is stopped or the game restarts.

use crate::engine::Engine;
use modscript::{call_value, CallCx, Coroutine, Resume, RtError, Value};

pub(crate) enum Wait {
    /// Steps still to go (1 = resume on the next step).
    Steps(u64),
    /// A function checked once per step.
    Until(Value),
}

pub(crate) struct Entry {
    pub id: u32,
    pub owner: Option<u32>,
    /// None while it is running (taken out to run it).
    pub co: Option<Coroutine>,
    pub wait: Wait,
    pub paused: bool,
    pub stopped: bool,
}

pub(crate) struct Timer {
    pub id: u32,
    pub owner: Option<u32>,
    pub cx: CallCx,
    pub period: u64,
    pub left: u64,
    pub f: Value,
    pub looped: bool,
}

#[derive(Default)]
pub struct Coroutines {
    pub(crate) list: Vec<Entry>,
    pub(crate) timers: Vec<Timer>,
    next: u32,
}

impl Coroutines {
    pub(crate) fn new_id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }
    pub fn clear(&mut self) {
        self.list.clear();
        self.timers.clear();
    }
    pub fn running(&self, id: u32) -> bool {
        self.list.iter().any(|c| c.id == id && !c.stopped)
    }
    /// Live coroutines (all of them, or those of one instance).
    pub fn count(&self, owner: Option<u32>) -> usize {
        self.list.iter().filter(|c| !c.stopped && (owner.is_none() || c.owner == owner)).count()
    }
}

/// Starts `f(args...)` as a coroutine and runs it up to its first suspension.
pub(crate) fn start(e: &mut Engine, f: &Value, cx: &CallCx, args: Vec<Value>) -> Result<u32, RtError> {
    let co = Coroutine::new(e, f, cx, args)?;
    let owner = co.cx().self_.inst();
    let id = e.coros.new_id();
    e.coros.list.push(Entry { id, owner, co: Some(co), wait: Wait::Steps(0), paused: false, stopped: false });
    run(e, id)?;
    Ok(id)
}

/// Resumes coroutine `id` until it suspends again (or ends / fails).
fn run(e: &mut Engine, id: u32) -> Result<(), RtError> {
    let Some(i) = e.coros.list.iter().position(|c| c.id == id) else { return Ok(()) };
    let Some(mut co) = e.coros.list[i].co.take() else { return Ok(()) };
    let rate = e.step_rate();
    let mut result = Ok(());
    let wait = loop {
        match co.resume(e) {
            Ok(Resume::Done(_)) => break None,
            Ok(Resume::Suspended(name, args)) => {
                let n = |i: usize| args.get(i).and_then(|v| v.as_num());
                let w = match &*name.name() {
                    "yield" => Wait::Steps(1),
                    "wait" => match n(0) {
                        Some(s) => Wait::Steps(s.round().max(0.0) as u64),
                        None => {
                            result = Err(RtError::new("wait(pasos) necesita un número de pasos"));
                            break None;
                        }
                    },
                    "wait_seconds" => match n(0) {
                        Some(s) => Wait::Steps((s * rate).round().max(0.0) as u64),
                        None => {
                            result = Err(RtError::new("wait_seconds(segundos) necesita un número"));
                            break None;
                        }
                    },
                    _ => match args.first() {
                        Some(f @ (Value::Func(_) | Value::Native(_))) => Wait::Until(f.clone()),
                        _ => {
                            result = Err(RtError::new("wait_until(función) necesita una función que devuelva true para seguir"));
                            break None;
                        }
                    },
                };
                // wait(0) goes on at once
                if matches!(w, Wait::Steps(0)) {
                    continue;
                }
                break Some(w);
            }
            Err(err) => {
                result = Err(err);
                break None;
            }
        }
    };
    // the list may have changed while it ran (other coroutines started or stopped)
    if let Some(i) = e.coros.list.iter().position(|c| c.id == id) {
        match wait {
            Some(w) if !e.coros.list[i].stopped => {
                e.coros.list[i].wait = w;
                e.coros.list[i].co = Some(co);
            }
            _ => {
                e.coros.list.remove(i);
            }
        }
    }
    result
}

fn owner_gone(e: &Engine, owner: Option<u32>) -> bool {
    owner.is_some_and(|o| !e.alive(o))
}

/// Once per step, after the `step` event.
pub(crate) fn step(e: &mut Engine) {
    if e.coros.list.is_empty() && e.coros.timers.is_empty() {
        return;
    }
    let ids: Vec<u32> = e.coros.list.iter().map(|c| c.id).collect();
    for id in ids {
        let Some(i) = e.coros.list.iter().position(|c| c.id == id) else { continue };
        let c = &e.coros.list[i];
        if c.stopped || c.co.is_none() {
            continue;
        }
        if owner_gone(e, c.owner) {
            e.coros.list.remove(i);
            continue;
        }
        let c = &mut e.coros.list[i];
        if c.paused {
            continue;
        }
        let go = match &mut c.wait {
            Wait::Steps(n) => {
                *n = n.saturating_sub(1);
                *n == 0
            }
            Wait::Until(f) => {
                let f = f.clone();
                let cx = c.co.as_ref().map(|co| co.cx().clone()).unwrap_or_else(CallCx::global);
                match call_value(e, &f, &cx, vec![]) {
                    Ok(v) => v.truthy(),
                    Err(err) => {
                        e.report(err);
                        if let Some(i) = e.coros.list.iter().position(|c| c.id == id) {
                            e.coros.list.remove(i);
                        }
                        continue;
                    }
                }
            }
        };
        if go {
            if let Err(err) = run(e, id) {
                e.report(err);
            }
        }
    }
    e.coros.list.retain(|c| !c.stopped);
    timers(e);
}

fn timers(e: &mut Engine) {
    let ids: Vec<u32> = e.coros.timers.iter().map(|t| t.id).collect();
    for id in ids {
        let Some(i) = e.coros.timers.iter().position(|t| t.id == id) else { continue };
        if owner_gone(e, e.coros.timers[i].owner) {
            e.coros.timers.remove(i);
            continue;
        }
        let t = &mut e.coros.timers[i];
        t.left = t.left.saturating_sub(1);
        if t.left > 0 {
            continue;
        }
        let (f, cx, looped, period) = (t.f.clone(), t.cx.clone(), t.looped, t.period);
        if looped {
            t.left = period.max(1);
        } else {
            e.coros.timers.remove(i);
        }
        if let Err(err) = call_value(e, &f, &cx, vec![]) {
            e.report(err);
        }
    }
}

/// `call_later(period, units, fn, [loop])`.
pub(crate) fn call_later(e: &mut Engine, steps: u64, f: Value, cx: CallCx, looped: bool) -> u32 {
    let id = e.coros.new_id();
    let owner = match &f {
        Value::Func(c) => c.this.as_ref().and_then(|s| s.inst()).or(cx.self_.inst()),
        _ => cx.self_.inst(),
    };
    let period = steps.max(1);
    e.coros.timers.push(Timer { id, owner, cx, period, left: period, f, looped });
    id
}

pub(crate) fn stop(e: &mut Engine, id: u32) -> bool {
    let mut found = false;
    for c in e.coros.list.iter_mut().filter(|c| c.id == id) {
        c.stopped = true;
        found = true;
    }
    let before = e.coros.timers.len();
    e.coros.timers.retain(|t| t.id != id);
    found || e.coros.timers.len() != before
}

/// Stops every coroutine (and delayed call) of an instance, or all of them.
pub(crate) fn stop_all(e: &mut Engine, owner: Option<u32>) {
    for c in e.coros.list.iter_mut().filter(|c| owner.is_none() || c.owner == owner) {
        c.stopped = true;
    }
    e.coros.timers.retain(|t| owner.is_some() && t.owner != owner);
}

pub(crate) fn set_paused(e: &mut Engine, id: u32, p: bool) -> bool {
    match e.coros.list.iter_mut().find(|c| c.id == id && !c.stopped) {
        Some(c) => {
            c.paused = p;
            true
        }
        None => false,
    }
}

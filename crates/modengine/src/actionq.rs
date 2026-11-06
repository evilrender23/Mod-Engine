//! Action queues (`actionq_*`): the model/view split of turn-based games.
//!
//! Game logic resolves a whole move at once (damage, deaths, draws) and pushes
//! *presentation* actions: functions that animate, wait and play sounds. The queue runs
//! them one after another (or a group in parallel), each one as a coroutine that may use
//! `wait(steps)`, `wait_seconds(s)`, `yield()` and `wait_until(fn)`.
//!
//! Speed: every game step a queue runs `speed` sub-steps (fractions accumulate), so
//! ×2 plays the same actions in half the time and `infinity` (instant) runs everything
//! at once, still in order; 0 pauses it. Waits count sub-steps, so the waits and
//! `wait(1)` animation loops inside actions all scale with it.

use crate::engine::Engine;
use modscript::{call_value, CallCx, Coroutine, Resume, RtError, Value};
use std::collections::VecDeque;

pub(crate) enum Item {
    Call(Value, Vec<Value>, CallCx),
    /// Sub-steps to wait.
    Wait(u64),
    Parallel(Vec<(Value, Vec<Value>)>, CallCx),
}

enum JobWait {
    Steps(u64),
    Until(Value),
}

struct Job {
    id: u32,
    owner: Option<u32>,
    /// None while it runs.
    co: Option<Coroutine>,
    wait: JobWait,
}

pub(crate) struct Queue {
    pub items: VecDeque<Item>,
    jobs: Vec<Job>,
    /// Sub-steps left of a `actionq_push_wait`.
    wait_left: u64,
    pub speed: f64,
    acc: f64,
    /// Updated by the engine every step (otherwise only by `actionq_update`).
    pub auto: bool,
    /// Items put in front by `actionq_push_next` since the last one started (so several
    /// keep their order).
    front: usize,
}

impl Queue {
    pub fn new(auto: bool) -> Queue {
        Queue { items: VecDeque::new(), jobs: vec![], wait_left: 0, speed: 1.0, acc: 0.0, auto, front: 0 }
    }
    /// Queues an action right after the one playing (before the rest).
    pub fn push_next(&mut self, it: Item) {
        let at = self.front.min(self.items.len());
        self.items.insert(at, it);
        self.front += 1;
    }
    pub fn busy(&self) -> bool {
        self.wait_left > 0 || !self.jobs.is_empty() || !self.items.is_empty()
    }
    /// Pending actions, counting the one playing.
    pub fn size(&self) -> usize {
        self.items.len() + usize::from(self.wait_left > 0 || !self.jobs.is_empty())
    }
    pub fn clear(&mut self) {
        self.items.clear();
        self.jobs.clear();
        self.wait_left = 0;
        self.acc = 0.0;
        self.front = 0;
    }
}

#[derive(Default)]
pub struct ActionQueues {
    pub(crate) list: Vec<Option<Queue>>,
    next_job: u32,
}

impl ActionQueues {
    pub fn clear(&mut self) {
        self.list.clear();
    }
    pub(crate) fn get_mut(&mut self, h: usize) -> Option<&mut Queue> {
        self.list.get_mut(h).and_then(|q| q.as_mut())
    }
    pub(crate) fn create(&mut self, auto: bool) -> usize {
        match self.list.iter().position(|q| q.is_none()) {
            Some(i) => {
                self.list[i] = Some(Queue::new(auto));
                i
            }
            None => {
                self.list.push(Some(Queue::new(auto)));
                self.list.len() - 1
            }
        }
    }
}

/// Sub-steps run at most per update (`infinity` speed included) before giving up:
/// something waits forever (a `while (true) { wait(1) }` action).
const MAX_SUBSTEPS: u32 = 200_000;

fn owner_gone(e: &Engine, owner: Option<u32>) -> bool {
    owner.is_some_and(|o| !e.alive(o))
}

/// Starts an action: a script function becomes a coroutine run up to its first wait;
/// a built-in is just called.
fn start(e: &mut Engine, h: usize, f: Value, args: Vec<Value>, cx: &CallCx) {
    if owner_gone(e, cx.self_.inst()) {
        return;
    }
    if !matches!(f, Value::Func(_)) {
        if let Err(err) = call_value(e, &f, cx, args) {
            e.report(err);
        }
        return;
    }
    let co = match Coroutine::new(e, &f, cx, args) {
        Ok(co) => co,
        Err(err) => {
            e.report(err);
            return;
        }
    };
    let owner = co.cx().self_.inst();
    e.actionqs.next_job += 1;
    let id = e.actionqs.next_job;
    let Some(q) = e.actionqs.get_mut(h) else { return };
    q.jobs.push(Job { id, owner, co: Some(co), wait: JobWait::Steps(0) });
    resume(e, h, id);
}

/// Runs job `id` until it waits again or ends.
fn resume(e: &mut Engine, h: usize, id: u32) {
    let Some(mut co) = e.actionqs.get_mut(h).and_then(|q| q.jobs.iter_mut().find(|j| j.id == id)).and_then(|j| j.co.take()) else { return };
    let rate = e.step_rate();
    let wait = loop {
        match co.resume(e) {
            Ok(Resume::Done(_)) => break None,
            Ok(Resume::Suspended(name, args)) => {
                let n = |i: usize| args.get(i).and_then(|v| v.as_num());
                let w = match &*name.name() {
                    "yield" => JobWait::Steps(1),
                    "wait" => JobWait::Steps(n(0).unwrap_or(1.0).round().max(0.0) as u64),
                    "wait_seconds" => JobWait::Steps((n(0).unwrap_or(0.0) * rate).round().max(0.0) as u64),
                    _ => match args.first() {
                        Some(f @ (Value::Func(_) | Value::Native(_))) => JobWait::Until(f.clone()),
                        _ => {
                            e.report(RtError::new("wait_until(función) necesita una función que devuelva true para seguir"));
                            break None;
                        }
                    },
                };
                if matches!(w, JobWait::Steps(0)) {
                    continue;
                }
                break Some(w);
            }
            Err(err) => {
                e.report(err);
                break None;
            }
        }
    };
    // the queue may have been cleared or destroyed while the action ran
    if let Some(q) = e.actionqs.get_mut(h) {
        if let Some(i) = q.jobs.iter().position(|j| j.id == id) {
            match wait {
                Some(w) => {
                    q.jobs[i].wait = w;
                    q.jobs[i].co = Some(co);
                }
                None => {
                    q.jobs.remove(i);
                }
            }
        }
    }
}

/// One sub-step of queue `h`. Returns whether anything happened (instant mode stops
/// when only `wait_until` conditions are pending and none is true).
fn substep(e: &mut Engine, h: usize) -> bool {
    let mut progressed = false;
    {
        let Some(q) = e.actionqs.get_mut(h) else { return false };
        if q.wait_left > 0 {
            q.wait_left -= 1;
            progressed = true;
        }
    }
    let ids: Vec<(u32, Option<u32>)> = match e.actionqs.get_mut(h) {
        Some(q) => q.jobs.iter().map(|j| (j.id, j.owner)).collect(),
        None => return false,
    };
    for (id, owner) in ids {
        if owner_gone(e, owner) {
            if let Some(q) = e.actionqs.get_mut(h) {
                q.jobs.retain(|j| j.id != id);
            }
            progressed = true;
            continue;
        }
        let go = {
            let Some(j) = e.actionqs.get_mut(h).and_then(|q| q.jobs.iter_mut().find(|j| j.id == id)) else { continue };
            match &mut j.wait {
                JobWait::Steps(n) => {
                    *n = n.saturating_sub(1);
                    progressed = true;
                    *n == 0
                }
                JobWait::Until(f) => {
                    let f = f.clone();
                    let cx = j.co.as_ref().map(|c| c.cx().clone()).unwrap_or_else(CallCx::global);
                    match call_value(e, &f, &cx, vec![]) {
                        Ok(v) => v.truthy(),
                        Err(err) => {
                            e.report(err);
                            if let Some(q) = e.actionqs.get_mut(h) {
                                q.jobs.retain(|j| j.id != id);
                            }
                            progressed = true;
                            continue;
                        }
                    }
                }
            }
        };
        if go {
            progressed = true;
            resume(e, h, id);
        }
    }
    // nothing playing: start the next actions (instant ones finish right away)
    loop {
        let item = {
            let Some(q) = e.actionqs.get_mut(h) else { break };
            if q.wait_left > 0 || !q.jobs.is_empty() {
                break;
            }
            q.front = 0;
            match q.items.pop_front() {
                Some(it) => it,
                None => break,
            }
        };
        progressed = true;
        match item {
            Item::Wait(n) => {
                if let Some(q) = e.actionqs.get_mut(h) {
                    q.wait_left = n;
                }
            }
            Item::Call(f, args, cx) => start(e, h, f, args, &cx),
            Item::Parallel(list, cx) => {
                for (f, args) in list {
                    start(e, h, f, args, &cx);
                }
            }
        }
    }
    progressed
}

/// Plays everything still queued right now, in order (skip button, `infinity` speed).
pub(crate) fn finish(e: &mut Engine, h: usize) {
    let mut n = 0;
    while e.actionqs.get_mut(h).is_some_and(|q| q.busy()) {
        if !substep(e, h) {
            break; // only false wait_until conditions left: try again next step
        }
        n += 1;
        if n >= MAX_SUBSTEPS {
            e.report(RtError::new(format!("actionq: la cola {h} sigue ocupada tras {MAX_SUBSTEPS} sub-pasos en modo instantáneo (¿una acción que espera para siempre?)")));
            break;
        }
    }
}

/// Advances queue `h` by one game step at its speed.
pub(crate) fn update(e: &mut Engine, h: usize) {
    let Some(q) = e.actionqs.get_mut(h) else { return };
    if q.speed.is_infinite() {
        finish(e, h);
        return;
    }
    q.acc += q.speed.max(0.0);
    let n = (q.acc.floor() as u32).min(MAX_SUBSTEPS);
    q.acc -= q.acc.floor();
    for _ in 0..n {
        substep(e, h);
    }
}

/// Once per step, after the coroutines: queues created with auto = true.
pub(crate) fn step(e: &mut Engine) {
    if e.actionqs.list.is_empty() {
        return;
    }
    for h in 0..e.actionqs.list.len() {
        if e.actionqs.list[h].as_ref().is_some_and(|q| q.auto) {
            update(e, h);
        }
    }
}

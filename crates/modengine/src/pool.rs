//! Instance pools: `instance_pool_create(obj, n)`, `pool_spawn`, `pool_release`.
//!
//! A released instance leaves the room like a destroyed one (without its `destroy`
//! event) and its storage is kept; `pool_spawn` reuses it for a new instance with a NEW
//! id, running the property initializers, the `vars` and `create` again — exactly what
//! `spawn` does, so objects need no changes and old references to a released bullet
//! never see it come back to life.

use crate::engine::Engine;
use modscript::{ArrayRef, Sym, Value};
use std::collections::HashMap;

/// Storage of a released instance.
pub(crate) struct Recycled {
    pub vars: HashMap<Sym, Value>,
    pub alarms: ArrayRef,
}

#[derive(Default)]
pub struct Pools {
    /// Free storage per object.
    pub(crate) free: HashMap<u32, Vec<Recycled>>,
    /// Maximum kept per object (instance_pool_create's n; default 256).
    pub(crate) cap: HashMap<u32, usize>,
    pub(crate) stats: PoolStats,
}

/// Counters for tests and tuning (`pool_stats()`).
#[derive(Default, Clone, Copy)]
pub struct PoolStats {
    pub reused: u64,
    pub created: u64,
    pub released: u64,
}

pub const DEFAULT_CAP: usize = 256;

impl Pools {
    pub fn clear(&mut self) {
        self.free.clear();
        self.cap.clear();
        self.stats = PoolStats::default();
    }
    pub fn free_count(&self, obj: u32) -> usize {
        self.free.get(&obj).map_or(0, |v| v.len())
    }
}

fn fresh_alarms() -> ArrayRef {
    match Value::array(vec![Value::Num(-1.0); crate::instance::ALARM_COUNT]) {
        Value::Array(a) => a,
        _ => unreachable!(),
    }
}

/// Pre-allocates `n` instances' storage for `obj`.
pub(crate) fn create(e: &mut Engine, obj: u32, n: usize) {
    e.pools.cap.insert(obj, n.max(1));
    let free = e.pools.free.entry(obj).or_default();
    while free.len() < n {
        free.push(Recycled { vars: HashMap::with_capacity(16), alarms: fresh_alarms() });
    }
}

/// Like spawn(): a new instance of `obj`, reusing pooled storage if there is some.
pub(crate) fn spawn(e: &mut Engine, obj: u32, x: f64, y: f64, depth: f64, vars: &[(Sym, Value)], creator: modscript::Scope) -> u32 {
    let id = e.create_instance(obj, x, y, depth);
    if let Some(r) = e.pools.free.get_mut(&obj).and_then(|v| v.pop()) {
        if let Some(i) = e.idx(id) {
            let inst = &mut e.insts[i];
            {
                let mut a = r.alarms.borrow_mut();
                a.iter_mut().for_each(|v| *v = Value::Num(-1.0));
                a.resize(crate::instance::ALARM_COUNT, Value::Num(-1.0));
            }
            inst.alarms = r.alarms;
            let mut vars = r.vars;
            vars.clear();
            inst.vars = vars;
        }
        e.pools.stats.reused += 1;
    } else {
        e.pools.stats.created += 1;
    }
    e.init_instance(id, vars, None, creator);
    id
}

/// Takes `id` out of the room (no destroy event unless `run_destroy`) and keeps its storage.
pub(crate) fn release(e: &mut Engine, id: u32, run_destroy: bool) -> bool {
    let Some(i) = e.idx(id) else { return false };
    if !e.insts[i].alive {
        return false;
    }
    let obj = e.insts[i].object;
    e.destroy(id, run_destroy);
    let Some(i) = e.idx(id) else { return true };
    let cap = e.pools.cap.get(&obj).copied().unwrap_or(DEFAULT_CAP);
    if e.pools.free_count(obj) < cap {
        let inst = &mut e.insts[i];
        // the dead instance keeps an empty map / fresh alarms until it is swept at the end of the step
        let vars = std::mem::take(&mut inst.vars);
        let alarms = std::mem::replace(&mut inst.alarms, fresh_alarms());
        e.pools.free.entry(obj).or_default().push(Recycled { vars, alarms });
    }
    e.pools.stats.released += 1;
    true
}

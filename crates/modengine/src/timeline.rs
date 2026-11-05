//! Timelines (`timelines/<name>.toml`): ModScript code that runs at given steps
//! ("moments") of an instance's timeline, like GameMaker time lines.
//!
//! ```toml
//! [[moment]]
//! step = 0
//! code = '''
//! hspeed = 2
//! '''
//!
//! [[moment]]
//! step = 60
//! code = "hspeed = -2"
//! ```

use crate::engine::Engine;
use crate::instance::Instance;
use modscript::{call_value, parse_stmts, run_stmts, AssetKind, CallCx, RtError, RtResult, Scope, Stmt, Value};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MomentToml {
    pub step: u32,
    #[serde(default)]
    pub code: String,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct TimelineToml {
    #[serde(default, rename = "moment")]
    pub moments: Vec<MomentToml>,
}

impl TimelineToml {
    /// Moments sorted by step.
    pub fn sorted(&mut self) {
        self.moments.sort_by_key(|m| m.step);
    }
}

#[derive(Clone)]
pub enum Action {
    Code(Rc<Vec<Stmt>>),
    /// A function added with `timeline_moment_add_script`.
    Func(Value),
}

#[derive(Clone)]
pub struct Moment {
    pub step: f64,
    pub actions: Vec<Action>,
}

#[derive(Clone)]
pub struct TimelineDef {
    pub name: String,
    pub file: Rc<str>,
    /// Sorted by step; one entry per step.
    pub moments: Vec<Moment>,
    /// false after `timeline_delete`.
    pub alive: bool,
}

impl TimelineDef {
    pub fn empty(name: &str) -> Self {
        TimelineDef { name: name.into(), file: Rc::from(name), moments: vec![], alive: true }
    }

    pub fn from_toml(name: &str, file: &str, t: &TimelineToml) -> Result<TimelineDef, String> {
        let mut tl = TimelineDef { name: name.into(), file: Rc::from(file), moments: vec![], alive: true };
        for m in &t.moments {
            let label = format!("{file} (moment {})", m.step);
            let stmts = parse_stmts(&label, &m.code).map_err(|e| format!("{file}: moment {}: line {}: {}", m.step, e.line, e.msg))?;
            tl.add(m.step as f64, Action::Code(Rc::new(stmts)));
        }
        Ok(tl)
    }

    pub fn load(name: &str, path: &Path, rel: &str) -> Result<TimelineDef, String> {
        let text = crate::vfs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let t: TimelineToml = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        TimelineDef::from_toml(name, rel, &t)
    }

    pub fn add(&mut self, step: f64, a: Action) {
        match self.moments.iter().position(|m| m.step >= step) {
            Some(i) if self.moments[i].step == step => self.moments[i].actions.push(a),
            Some(i) => self.moments.insert(i, Moment { step, actions: vec![a] }),
            None => self.moments.push(Moment { step, actions: vec![a] }),
        }
    }

    pub fn max_moment(&self) -> f64 {
        self.moments.last().map(|m| m.step).unwrap_or(0.0)
    }
}

pub fn write_toml(t: &TimelineToml) -> String {
    let mut s = String::new();
    for m in &t.moments {
        s.push_str(&format!("[[moment]]\nstep = {}\n", m.step));
        let code = m.code.trim_end();
        if code.contains('\n') || code.contains('"') || code.contains('\\') {
            if code.contains("'''") || code.ends_with('\'') {
                s.push_str(&format!("code = {}\n\n", toml::Value::String(code.to_string())));
            } else {
                // the newline after the opening ''' is not part of the string
                s.push_str(&format!("code = '''\n{code}'''\n\n"));
            }
        } else {
            s.push_str(&format!("code = {}\n\n", toml::Value::String(code.to_string())));
        }
    }
    s
}

// ------------------------------------------------------------------ per-instance state

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineRun {
    pub index: Option<u32>,
    pub position: f64,
    pub speed: f64,
    pub running: bool,
    pub looping: bool,
}

impl Default for TimelineRun {
    fn default() -> Self {
        TimelineRun { index: None, position: 0.0, speed: 1.0, running: false, looping: false }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TlVar {
    Index,
    Position,
    Speed,
    Running,
    Loop,
}

pub const VARS: &[(&str, TlVar)] = &[
    ("timeline_index", TlVar::Index),
    ("timeline_position", TlVar::Position),
    ("timeline_speed", TlVar::Speed),
    ("timeline_running", TlVar::Running),
    ("timeline_loop", TlVar::Loop),
];

pub fn get_var(inst: &Instance, v: TlVar) -> Value {
    let t = &inst.timeline;
    match v {
        TlVar::Index => t.index.map(|i| Value::Asset(AssetKind::Timeline, i)).unwrap_or(Value::Num(-1.0)),
        TlVar::Position => Value::Num(t.position),
        TlVar::Speed => Value::Num(t.speed),
        TlVar::Running => Value::Bool(t.running),
        TlVar::Loop => Value::Bool(t.looping),
    }
}

pub fn set_var(inst: &mut Instance, v: TlVar, val: Value) -> RtResult<()> {
    let num = |val: &Value, n: &str| val.as_num().ok_or_else(|| RtError::new(format!("'{n}' must be a number, got {}", val.type_name())));
    let t = &mut inst.timeline;
    match v {
        TlVar::Index => {
            t.index = match val {
                Value::Asset(AssetKind::Timeline, i) => Some(i),
                Value::Null => None,
                Value::Num(n) if n < 0.0 => None,
                other => return Err(RtError::new(format!("'timeline_index' must be a timeline (or -1), got {}", other.type_name()))),
            }
        }
        TlVar::Position => t.position = num(&val, "timeline_position")?,
        TlVar::Speed => t.speed = num(&val, "timeline_speed")?,
        TlVar::Running => t.running = val.truthy(),
        TlVar::Loop => t.looping = val.truthy(),
    }
    Ok(())
}

/// Advances the timelines of `ids` one step, running the moments they pass in each
/// instance's context. Runs after the begin step event, like in GameMaker.
pub fn step(e: &mut Engine, ids: &[u32]) {
    for &id in ids {
        let Some(i) = e.idx(id) else { continue };
        let inst = &e.insts[i];
        if !inst.alive || !inst.timeline.running {
            continue;
        }
        let Some(ti) = inst.timeline.index else { continue };
        let Some(tl) = e.timelines.get(ti as usize).filter(|t| t.alive) else { continue };
        let (old, speed) = (inst.timeline.position, inst.timeline.speed);
        let new = old + speed;
        // moments in [old, new) going forward, (new, old] going backwards
        let mut due: Vec<Moment> = tl.moments.iter().filter(|m| if speed >= 0.0 { m.step >= old && m.step < new } else { m.step <= old && m.step > new }).cloned().collect();
        if speed < 0.0 {
            due.reverse();
        }
        let (file, max) = (tl.file.clone(), tl.max_moment());
        let mut pos = new;
        if e.insts[i].timeline.looping && !tl.moments.is_empty() {
            let period = max + 1.0;
            if pos > max {
                pos -= period * ((pos - max) / period).ceil();
            } else if pos < 0.0 {
                pos += period * (-pos / period).ceil();
            }
        }
        e.insts[i].timeline.position = pos;
        for m in due {
            for a in &m.actions {
                if !e.alive(id) {
                    break;
                }
                // moment code may change the timeline itself: stop when it does
                let r = match a {
                    Action::Code(stmts) => run_stmts(e, stmts, &file, CallCx { self_: Scope::Inst(id), other: Scope::Global }),
                    Action::Func(f) => call_value(e, f, &CallCx { self_: Scope::Inst(id), other: Scope::Global }, vec![]).map(|_| ()),
                };
                if let Err(err) = r {
                    e.report(err);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_roundtrip_and_sorting() {
        let mut t = TimelineToml {
            moments: vec![
                MomentToml { step: 30, code: "x += 1\ny = \"a\"".into() },
                MomentToml { step: 0, code: "speed = 2".into() },
            ],
        };
        t.sorted();
        assert_eq!(t.moments[0].step, 0);
        let back: TimelineToml = toml::from_str(&write_toml(&t)).unwrap();
        assert_eq!(back, t);
        let tl = TimelineDef::from_toml("t", "timelines/t.toml", &t).unwrap();
        assert_eq!(tl.moments.len(), 2);
        assert_eq!(tl.max_moment(), 30.0);
        let bad = TimelineToml { moments: vec![MomentToml { step: 3, code: "x = (".into() }] };
        let err = TimelineDef::from_toml("t", "timelines/t.toml", &bad).err().unwrap();
        assert!(err.contains("moment 3"), "{err}");
    }
}

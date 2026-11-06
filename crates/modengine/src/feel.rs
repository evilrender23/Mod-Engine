//! Game feel: tweens (`tween_to`, easing, chains, yoyo), per-instance sprite effects
//! (`flash`, `sprite_outline`, `sprite_overwrite`, `pulse`), pooled floating text
//! (`popup_text`), hit-stop (`hitstop`) and the "reduced effects" accessibility switch.
//!
//! Everything advances inside [`Engine::tick`] with the fixed step (1 / step rate seconds
//! per step), so the same input gives the same frames. Tweens run before the instances'
//! events of the step; a hit-stop freezes the whole gameplay step (events, motion,
//! coroutines, flashes and normal tweens) while unscaled tweens, popups, audio and the
//! camera shake keep going.
//!
//! Sprite effects are drawn by the default sprite shader of the GPU backend
//! ([`DrawCmd::SpriteFx`]): the overwrite colour is mixed into the sprite's pixels (alpha
//! kept), and an outline is the same sprite drawn around itself fully overwritten, so it
//! all stays in the same batch as the other sprites.

use crate::draw::{DrawCmd, Rgba};
use crate::engine::Engine;
use modscript::{call_value, intern, CallCx, Host, RtError, StructRef, Sym, Value};
use std::collections::HashMap;

// ================================================================== tweens

/// What a tween animates: an instance's variables or a struct's fields.
#[derive(Clone)]
pub enum Target {
    Inst(u32),
    Struct(StructRef),
}

impl Target {
    pub fn same(&self, o: &Target) -> bool {
        match (self, o) {
            (Target::Inst(a), Target::Inst(b)) => a == b,
            (Target::Struct(a), Target::Struct(b)) => std::rc::Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// Easing of a tween: a named function (`quad_out`, `back_inout`...) or an animation curve.
#[derive(Clone, Debug, PartialEq)]
pub enum Ease {
    Named(String),
    Curve(u32),
}

impl Ease {
    pub fn apply(&self, e: &Engine, t: f64) -> f64 {
        match self {
            Ease::Named(n) => ease(n, t).unwrap_or(t),
            Ease::Curve(c) => e.curves.get(*c as usize).and_then(|c| c.channels.first()).map(|ch| ch.eval(t.clamp(0.0, 1.0))).unwrap_or(t),
        }
    }
}

/// Easing by name: `linear`, `smooth`, and `<family>_<in|out|inout>` for the families
/// quad, cubic, quart, quint, sine, expo, circ, back, elastic and bounce (also
/// `ease_in`/`ease_out`/`ease_in_out` = cubic). `t` is clamped to 0..1; back and elastic
/// overshoot outside 0..1 on purpose.
pub fn ease(name: &str, t: f64) -> Option<f64> {
    match name {
        // the family names alone mean their "out" version (the usual feel for UI)
        "quad" | "cubic" | "quart" | "quint" | "sine" | "expo" | "circ" | "back" | "elastic" => crate::animcurve::ease(&format!("{name}_out"), t),
        _ => crate::animcurve::ease(name, t),
    }
}

/// Every easing name `ease()` knows (docs, editor, tests).
pub fn ease_names() -> Vec<String> {
    let mut v = vec!["linear".to_string(), "smooth".to_string()];
    for f in ["quad", "cubic", "quart", "quint", "sine", "expo", "circ", "back", "elastic", "bounce"] {
        for m in ["in", "out", "inout"] {
            v.push(format!("{f}_{m}"));
        }
    }
    v
}

struct Field {
    name: Sym,
    from: f64,
    to: f64,
    /// Interpolated per RGB channel (`tint`, `*color`, `*colour`).
    color: bool,
}

pub struct Tween {
    pub id: u32,
    pub target: Target,
    fields: Vec<Field>,
    /// `from` values are read from the target when the tween starts (tween_to).
    resolve_from: bool,
    started: bool,
    pub dur: f64,
    pub delay: f64,
    /// Seconds since it was allowed to run (delay included).
    elapsed: f64,
    pub ease: Ease,
    pub yoyo: bool,
    /// Extra repetitions: 0 = play once, -1 = forever.
    pub loops: i64,
    pub unscaled: bool,
    pub paused: bool,
    /// Waits for this tween to finish first (`tween_chain`).
    pub after: Option<u32>,
    pub on_complete: Option<(Value, CallCx)>,
}

fn is_color_field(name: &str) -> bool {
    name == "tint" || name.ends_with("color") || name.ends_with("colour")
}

fn lerp_color(a: f64, b: f64, k: f64) -> f64 {
    let (a, b) = (a.max(0.0) as u32, b.max(0.0) as u32);
    let ch = |c: u32, s: u32| ((c >> s) & 255) as f64;
    let mix = |s: u32| ((ch(a, s) + (ch(b, s) - ch(a, s)) * k).round().clamp(0.0, 255.0) as u32) << s;
    (mix(16) | mix(8) | mix(0)) as f64
}

impl Tween {
    /// Total length in seconds (delay excluded); infinite for endless loops.
    pub fn length(&self) -> f64 {
        let cycle = self.dur * if self.yoyo { 2.0 } else { 1.0 };
        if self.loops < 0 {
            f64::INFINITY
        } else {
            cycle * (self.loops + 1) as f64
        }
    }
    /// Seconds still to go (delay included).
    pub fn remaining(&self) -> f64 {
        (self.delay + self.length() - self.elapsed).max(0.0)
    }
    /// Eased progress 0..1 at `local` seconds after the delay, and whether it ended.
    fn progress(&self, local: f64) -> (f64, bool) {
        if self.dur <= 0.0 {
            return (if self.yoyo { 0.0 } else { 1.0 }, true);
        }
        let len = self.length();
        if local >= len - 1e-9 {
            return (if self.yoyo { 0.0 } else { 1.0 }, true);
        }
        let cycle = self.dur * if self.yoyo { 2.0 } else { 1.0 };
        let phase = local - (local / cycle).floor() * cycle;
        let p = if self.yoyo && phase > self.dur { 1.0 - (phase - self.dur) / self.dur } else { phase / self.dur };
        (p.clamp(0.0, 1.0), false)
    }
}

pub struct Tweens {
    pub list: Vec<Tween>,
    next: u32,
    /// Presentation speed of tweens that are not unscaled (`tween_set_speed`).
    pub speed: f64,
}

impl Default for Tweens {
    fn default() -> Self {
        Tweens { list: vec![], next: 0, speed: 1.0 }
    }
}

impl Tweens {
    // the list is always sorted by id: new ids grow and removing keeps the order
    pub fn get(&self, id: u32) -> Option<&Tween> {
        self.list.binary_search_by_key(&id, |t| t.id).ok().map(|i| &self.list[i])
    }
    pub fn get_mut(&mut self, id: u32) -> Option<&mut Tween> {
        self.list.binary_search_by_key(&id, |t| t.id).ok().map(|i| &mut self.list[i])
    }
    pub fn speed(&self) -> f64 {
        self.speed.max(0.0)
    }
}

/// Reads a number (field `name` of the target).
pub fn read_num(e: &mut Engine, t: &Target, name: Sym) -> Result<f64, RtError> {
    let v = match t {
        Target::Inst(id) => e.inst_get(*id, name)?,
        Target::Struct(s) => s.borrow().fields.get(&name).cloned(),
    };
    match v {
        Some(v) => v.as_num().ok_or_else(|| RtError::new(format!("tween: '{name}' must be a number, it is {}", v.type_name()))),
        None => Err(RtError::new(format!("tween: the target has no variable '{name}'"))),
    }
}

pub fn write_num(e: &mut Engine, t: &Target, name: Sym, v: f64) -> Result<(), RtError> {
    match t {
        Target::Inst(id) => e.inst_set(*id, name, Value::Num(v)),
        Target::Struct(s) => {
            s.borrow_mut().fields.insert(name, Value::Num(v));
            Ok(())
        }
    }
}

/// Options of a new tween.
pub struct TweenSpec {
    pub target: Target,
    /// (field, value): the end values (tween_to) or the start values (tween_from).
    pub values: Vec<(Sym, f64)>,
    pub from: bool,
    pub dur: f64,
    pub ease: Ease,
    pub delay: f64,
    pub yoyo: bool,
    pub loops: i64,
    pub unscaled: bool,
    pub on_complete: Option<(Value, CallCx)>,
}

/// Creates a tween; `tween_from` sets the start values right away.
pub fn create(e: &mut Engine, s: TweenSpec) -> Result<u32, RtError> {
    let mut fields = vec![];
    for (name, v) in &s.values {
        let color = is_color_field(&name.name());
        if s.from {
            let now = read_num(e, &s.target, *name)?;
            write_num(e, &s.target, *name, *v)?;
            fields.push(Field { name: *name, from: *v, to: now, color });
        } else {
            if let Target::Inst(id) = &s.target {
                // fail now (not on the first step) for a variable that doesn't exist
                read_num(e, &Target::Inst(*id), *name)?;
            }
            fields.push(Field { name: *name, from: 0.0, to: *v, color });
        }
    }
    let tw = &mut e.feel.tweens;
    tw.next += 1;
    let id = tw.next;
    tw.list.push(Tween {
        id,
        target: s.target,
        fields,
        resolve_from: !s.from,
        started: false,
        dur: s.dur.max(0.0),
        delay: s.delay.max(0.0),
        elapsed: 0.0,
        ease: s.ease,
        yoyo: s.yoyo,
        loops: s.loops.max(-1),
        unscaled: s.unscaled,
        paused: false,
        after: None,
        on_complete: s.on_complete,
    });
    Ok(id)
}

fn target_alive(e: &Engine, t: &Target) -> bool {
    match t {
        Target::Inst(id) => e.alive(*id),
        Target::Struct(_) => true,
    }
}

/// Moves a tween to its end (`to`, or `from` for yoyo) without callbacks.
pub fn finish_values(e: &mut Engine, id: u32) {
    let Some(t) = e.feel.tweens.get(id) else { return };
    if !t.started && t.resolve_from {
        // never started: the end values are simply the targets
        let (target, vals): (Target, Vec<(Sym, f64)>) = (t.target.clone(), t.fields.iter().map(|f| (f.name, f.to)).collect());
        if !t.yoyo {
            for (n, v) in vals {
                let _ = write_num(e, &target, n, v);
            }
        }
        return;
    }
    let target = t.target.clone();
    let vals: Vec<(Sym, f64)> = t.fields.iter().map(|f| (f.name, if t.yoyo { f.from } else { f.to })).collect();
    for (n, v) in vals {
        let _ = write_num(e, &target, n, v);
    }
}

fn step_tweens(e: &mut Engine, frozen: bool) {
    if e.feel.tweens.list.is_empty() {
        return;
    }
    let rate = e.step_rate();
    let speed = e.feel.tweens.speed();
    let ids: Vec<u32> = e.feel.tweens.list.iter().map(|t| t.id).collect();
    for &id in &ids {
        let Some(t) = e.feel.tweens.get(id) else { continue };
        if !target_alive(e, &t.target) {
            e.feel.tweens.list.retain(|t| t.id != id);
            continue;
        }
        if let Some(a) = t.after {
            // a chained tween starts on the step after the previous one ended
            if e.feel.tweens.get(a).is_some() || ids.binary_search(&a).is_ok() {
                if e.feel.tweens.get(a).is_none() {
                    e.feel.tweens.get_mut(id).unwrap().after = None;
                }
                continue;
            }
        }
        if t.paused || (frozen && !t.unscaled) {
            continue;
        }
        let dt = if t.unscaled { 1.0 / rate } else { speed / rate };
        let t = e.feel.tweens.get_mut(id).unwrap();
        t.after = None;
        t.elapsed += dt;
        if t.elapsed < t.delay - 1e-9 {
            continue;
        }
        let local = (t.elapsed - t.delay).max(0.0);
        if !t.started {
            t.started = true;
            if t.resolve_from {
                let target = t.target.clone();
                let names: Vec<Sym> = t.fields.iter().map(|f| f.name).collect();
                let mut froms = vec![];
                for n in names {
                    match read_num(e, &target, n) {
                        Ok(v) => froms.push(v),
                        Err(err) => {
                            e.report(err);
                            e.feel.tweens.list.retain(|t| t.id != id);
                            froms.clear();
                            break;
                        }
                    }
                }
                let Some(t) = e.feel.tweens.get_mut(id) else { continue };
                for (f, v) in t.fields.iter_mut().zip(froms) {
                    f.from = v;
                }
            }
        }
        let Some(t) = e.feel.tweens.get(id) else { continue };
        let (p, done) = t.progress(local);
        let k = t.ease.apply(e, p);
        let target = t.target.clone();
        let vals: Vec<(Sym, f64)> = t.fields.iter().map(|f| (f.name, if f.color { lerp_color(f.from, f.to, k) } else { f.from + (f.to - f.from) * k })).collect();
        for (n, v) in vals {
            if let Err(err) = write_num(e, &target, n, v) {
                e.report(err);
                e.feel.tweens.list.retain(|t| t.id != id);
                break;
            }
        }
        if done {
            let cb = e.feel.tweens.get_mut(id).and_then(|t| t.on_complete.take());
            e.feel.tweens.list.retain(|t| t.id != id);
            if let Some((f, cx)) = cb {
                if let Err(err) = call_value(e, &f, &cx, vec![Value::Num(id as f64)]) {
                    e.report(err);
                }
            }
        }
    }
}

// ================================================================== sprite effects

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InstFx {
    /// (colour, amount at start, seconds left, total seconds)
    pub flash: Option<(u32, f64, f64, f64)>,
    /// (colour, thickness in pixels, alpha)
    pub outline: Option<(u32, u32, f64)>,
    /// (colour, amount 0..1)
    pub overwrite: Option<(u32, f64)>,
    /// (tween, base scale x, base scale y) of a running `pulse`.
    pub pulse: Option<(u32, f64, f64)>,
}

impl InstFx {
    fn idle(&self) -> bool {
        self.flash.is_none() && self.outline.is_none() && self.overwrite.is_none() && self.pulse.is_none()
    }
    /// Current overwrite colour and amount (the flash wins while it is stronger).
    pub fn mix(&self, reduced: bool) -> Option<(u32, f64)> {
        let flash = self.flash.map(|(c, a, left, total)| {
            let k = if total > 0.0 { (left / total).clamp(0.0, 1.0) } else { 0.0 };
            let a = a * k;
            (c, if reduced { a.min(REDUCED_FLASH) } else { a })
        });
        match (flash, self.overwrite) {
            (Some(f), Some(o)) => Some(if f.1 >= o.1 { f } else { o }),
            (f, o) => f.or(o).filter(|m| m.1 > 0.0),
        }
    }
}

/// Strongest flash allowed with `juice_set_reduced(true)`.
pub const REDUCED_FLASH: f64 = 0.35;
/// Camera shake kept with `juice_set_reduced(true)`.
pub const REDUCED_SHAKE: f64 = 0.25;

// ================================================================== popups

#[derive(Clone, Debug)]
pub struct Popup {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub gui: bool,
    pub color: u32,
    pub outline: Option<u32>,
    pub font: Option<u32>,
    pub rise: f64,
    pub dur: f64,
    pub t: f64,
    pub scale: f64,
    pub pop: f64,
    pub active: bool,
}

/// Popups kept at most (the oldest is reused when all are busy).
pub const POPUP_POOL: usize = 256;

// ================================================================== state

#[derive(Default)]
pub struct Feel {
    pub tweens: Tweens,
    pub fx: HashMap<u32, InstFx>,
    pub popups: Vec<Popup>,
    /// Popups created since the start (pool tests and stats).
    pub popups_made: u64,
    /// Steps of hit-stop still to go.
    pub hitstop: u64,
    pub reduced: bool,
    pub focus: crate::uifocus::Focus,
    room: Option<usize>,
}

impl Feel {
    /// Restarting the game forgets everything but the accessibility switch.
    pub fn clear(&mut self) {
        let reduced = self.reduced;
        *self = Feel::default();
        self.reduced = reduced;
    }
    pub fn active_popups(&self) -> usize {
        self.popups.iter().filter(|p| p.active).count()
    }
}

/// Adds a popup, reusing a finished slot (or the oldest one when the pool is full).
pub fn add_popup(e: &mut Engine, mut p: Popup) -> usize {
    p.active = true;
    p.t = 0.0;
    if e.feel.reduced {
        p.pop = 1.0;
    }
    e.feel.popups_made += 1;
    let pool = &mut e.feel.popups;
    if let Some(i) = pool.iter().position(|q| !q.active) {
        pool[i] = p;
        return i;
    }
    if pool.len() < POPUP_POOL {
        pool.push(p);
        return pool.len() - 1;
    }
    // full: replace the one closest to its end
    let i = pool.iter().enumerate().max_by(|a, b| (a.1.t / a.1.dur.max(1e-9)).partial_cmp(&(b.1.t / b.1.dur.max(1e-9))).unwrap_or(std::cmp::Ordering::Equal)).map(|x| x.0).unwrap_or(0);
    pool[i] = p;
    i
}

/// Once per step, at the very start of [`Engine::tick`]. Returns true during a hit-stop:
/// the rest of the gameplay step is skipped.
pub fn step(e: &mut Engine) -> bool {
    if e.feel.room != Some(e.room) {
        // popups and focus groups belong to the room they were made in
        if e.feel.room.is_some() {
            e.feel.popups.iter_mut().for_each(|p| p.active = false);
            e.feel.focus.clear();
        }
        e.feel.room = Some(e.room);
    }
    let frozen = e.feel.hitstop > 0;
    if frozen {
        e.feel.hitstop -= 1;
    }
    let dt = 1.0 / e.step_rate();
    step_tweens(e, frozen);
    for p in e.feel.popups.iter_mut().filter(|p| p.active) {
        p.t += dt;
        if p.t >= p.dur {
            p.active = false;
        }
    }
    if !e.feel.fx.is_empty() {
        let alive: Vec<u32> = e.feel.fx.keys().copied().filter(|id| !e.alive(*id)).collect();
        for id in alive {
            e.feel.fx.remove(&id);
        }
        let tweens = &e.feel.tweens;
        for fx in e.feel.fx.values_mut() {
            if !frozen {
                if let Some(f) = &mut fx.flash {
                    f.2 -= dt;
                    if f.2 <= 1e-9 {
                        fx.flash = None;
                    }
                }
            }
            if let Some((tw, _, _)) = fx.pulse {
                if tweens.get(tw).is_none() {
                    fx.pulse = None;
                }
            }
        }
        e.feel.fx.retain(|_, f| !f.idle());
    }
    frozen
}

/// Multiplier of the camera shake (reduced effects).
pub fn shake_scale(e: &Engine) -> f64 {
    if e.feel.reduced {
        REDUCED_SHAKE
    } else {
        1.0
    }
}

fn rgba(c: u32, a: f64) -> [f32; 4] {
    let c = Rgba::from_rgb(c, a);
    [c.r, c.g, c.b, c.a]
}

/// Draws instance `i` with its sprite effects. False when it has none (the caller draws
/// it the plain way).
pub fn draw_inst_fx(e: &mut Engine, i: usize) -> bool {
    if e.feel.fx.is_empty() {
        return false;
    }
    let inst = &e.insts[i];
    let Some(fx) = e.feel.fx.get(&inst.id).copied() else { return false };
    let Some(s) = inst.sprite else { return false };
    let n = e.sprites[s as usize].frames.len() as i64;
    let frame = (inst.frame.floor() as i64).rem_euclid(n.max(1)) as u32;
    let (x, y, sx, sy, angle, tint, alpha) = (inst.x as f32, inst.y as f32, inst.scale_x as f32, inst.scale_y as f32, inst.angle as f32, inst.tint, inst.alpha);
    let mix = fx.mix(e.feel.reduced);
    if fx.outline.is_none() && mix.is_none() {
        return false;
    }
    if let Some((oc, thick, oa)) = fx.outline {
        e.cmds.push(DrawCmd::SpriteFx(Some(rgba(oc, 1.0))));
        let t = thick.clamp(1, 8) as i32;
        let color = Rgba::from_rgb(0xFFFFFF, alpha * oa);
        for dy in -t..=t {
            for dx in -t..=t {
                if (dx, dy) == (0, 0) || dx.abs() + dy.abs() > t {
                    continue;
                }
                e.cmds.push(DrawCmd::Sprite { spr: s, frame, x: x + dx as f32, y: y + dy as f32, sx, sy, angle, color });
            }
        }
    }
    e.cmds.push(DrawCmd::SpriteFx(mix.map(|(c, a)| rgba(c, a))));
    e.cmds.push(DrawCmd::Sprite { spr: s, frame, x, y, sx, sy, angle, color: Rgba::from_rgb(tint, alpha) });
    e.cmds.push(DrawCmd::SpriteFx(None));
    true
}

fn draw_popups(e: &mut Engine, gui: bool) {
    if e.feel.popups.is_empty() {
        return;
    }
    let list: Vec<Popup> = e.feel.popups.iter().filter(|p| p.active && p.gui == gui).cloned().collect();
    for p in list {
        let k = (p.t / p.dur.max(1e-9)).clamp(0.0, 1.0);
        let y = p.y - p.rise * ease("quad_out", k).unwrap_or(k);
        // grows to `pop` at once and settles back during the first fifth
        let pk = (k / 0.2).min(1.0);
        let scale = p.scale * (p.pop + (1.0 - p.pop) * ease("quad_out", pk).unwrap_or(pk));
        // fades out during the last third
        let alpha = if k > 0.66 { 1.0 - (k - 0.66) / 0.34 } else { 1.0 };
        let (x, y, sc) = (p.x as f32, y as f32, scale as f32);
        if let Some(oc) = p.outline {
            let col = Rgba::from_rgb(oc, alpha);
            for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                // one pixel around, whatever the scale (a crisp outline, not a smudge)
                e.cmds.push(DrawCmd::Text { x: x + dx, y: y + dy, text: p.text.clone(), font: p.font, scale: sc, color: col, halign: 1, valign: 1 });
            }
        }
        e.cmds.push(DrawCmd::Text { x, y, text: p.text, font: p.font, scale: sc, color: Rgba::from_rgb(p.color, alpha), halign: 1, valign: 1 });
    }
}

/// World-space popups, on top of every instance (end of the room part of `render`).
pub fn draw_world(e: &mut Engine) {
    draw_popups(e, false);
}

/// GUI popups, on top of the Draw GUI events.
pub fn draw_gui(e: &mut Engine) {
    draw_popups(e, true);
}

/// `pulse(inst, scale, seconds)`: scale up and back (restarting a running pulse).
pub fn pulse(e: &mut Engine, id: u32, scale: f64, secs: f64) -> Result<u32, RtError> {
    let Some(i) = e.idx(id) else { return Err(RtError::new(format!("instance {id} does not exist"))) };
    let prev = e.feel.fx.get(&id).and_then(|f| f.pulse);
    let (bx, by) = match prev {
        Some((tw, bx, by)) => {
            e.feel.tweens.list.retain(|t| t.id != tw);
            (bx, by)
        }
        None => (e.insts[i].scale_x, e.insts[i].scale_y),
    };
    e.insts[i].scale_x = bx;
    e.insts[i].scale_y = by;
    let tw = create(
        e,
        TweenSpec {
            target: Target::Inst(id),
            values: vec![(intern("scale_x"), bx * scale), (intern("scale_y"), by * scale)],
            from: false,
            dur: (secs / 2.0).max(0.0),
            ease: Ease::Named("quad_out".into()),
            delay: 0.0,
            yoyo: true,
            loops: 0,
            unscaled: false,
            on_complete: None,
        },
    )?;
    e.feel.fx.entry(id).or_default().pulse = Some((tw, bx, by));
    Ok(tw)
}

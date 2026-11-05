//! Sequences (`sequences/<name>.toml`): small animations made of tracks of sprites,
//! objects and sounds with keyframes, played in a room like GameMaker sequences.
//!
//! ```toml
//! length = 60          # frames
//! fps = 30             # frames per second of the sequence (independent of the room speed)
//! playback = "loop"    # oneshot | loop | pingpong
//! origin = [0, 0]      # point of the sequence placed at (x, y)
//!
//! [[track]]
//! name = "coin"
//! type = "sprite"      # sprite | object | sound
//! asset = "coin"
//! start = 0            # frame range where the track exists (end defaults to length)
//! [[track.key]]
//! frame = 0
//! x = 0
//! y = 0
//! ease = "ease_out"    # easing towards the next key: linear, smooth, step, ease_in,
//!                      # ease_out, ease_in_out, bounce or the name of an animation curve
//! [[track.key]]
//! frame = 30
//! y = -40
//! angle = 180
//! scale_x = 2
//! alpha = 0.5
//! image = 2            # sprite frame (when no key sets it, the sprite animates at its fps)
//!
//! [[moment]]
//! frame = 59
//! code = "show_debug(\"end of loop\")"
//! ```

use crate::animcurve::{ease, AnimCurve};
use crate::draw::{DrawCmd, Rgba};
use crate::engine::{AudioCmd, Engine};
use modscript::{intern, parse_stmts, run_stmts, AssetKind, CallCx, Host, Stmt, Value};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::rc::Rc;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Playback {
    #[default]
    Oneshot,
    Loop,
    Pingpong,
}

impl Playback {
    pub fn name(self) -> &'static str {
        match self {
            Playback::Oneshot => "oneshot",
            Playback::Loop => "loop",
            Playback::Pingpong => "pingpong",
        }
    }
    pub const ALL: [Playback; 3] = [Playback::Oneshot, Playback::Loop, Playback::Pingpong];
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    #[default]
    Sprite,
    Object,
    Sound,
}

impl TrackKind {
    pub fn name(self) -> &'static str {
        match self {
            TrackKind::Sprite => "sprite",
            TrackKind::Object => "object",
            TrackKind::Sound => "sound",
        }
    }
    pub const ALL: [TrackKind; 3] = [TrackKind::Sprite, TrackKind::Object, TrackKind::Sound];
}

/// A keyframe: only the properties it sets are keyed.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Key {
    pub frame: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ease: String,
}

/// The keyable properties, in dopesheet order.
pub const PROPS: [&str; 7] = ["x", "y", "angle", "scale_x", "scale_y", "alpha", "image"];

impl Key {
    pub fn get(&self, prop: usize) -> Option<f64> {
        [self.x, self.y, self.angle, self.scale_x, self.scale_y, self.alpha, self.image][prop]
    }
    pub fn slot(&mut self, prop: usize) -> &mut Option<f64> {
        match prop {
            0 => &mut self.x,
            1 => &mut self.y,
            2 => &mut self.angle,
            3 => &mut self.scale_x,
            4 => &mut self.scale_y,
            5 => &mut self.alpha,
            _ => &mut self.image,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct TrackToml {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: TrackKind,
    #[serde(default)]
    pub asset: String,
    #[serde(default)]
    pub start: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    #[serde(default, rename = "key")]
    pub keys: Vec<Key>,
}

impl TrackToml {
    pub fn sort(&mut self) {
        self.keys.sort_by(|a, b| a.frame.partial_cmp(&b.frame).unwrap_or(std::cmp::Ordering::Equal));
    }
    pub fn end_or(&self, length: f64) -> f64 {
        self.end.unwrap_or(length)
    }
    pub fn active(&self, frame: f64, length: f64) -> bool {
        // a track that lasts until the end stays visible on the last frame of a finished sequence
        let end = self.end_or(length);
        frame >= self.start && (frame < end || (end >= length && frame <= length))
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct SeqMomentToml {
    pub frame: f64,
    #[serde(default)]
    pub code: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SequenceToml {
    #[serde(default = "d60")]
    pub length: f64,
    #[serde(default = "d30")]
    pub fps: f64,
    #[serde(default)]
    pub playback: Playback,
    #[serde(default)]
    pub origin: [f64; 2],
    /// Size of the editor's preview canvas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<[u32; 2]>,
    #[serde(default, rename = "track")]
    pub tracks: Vec<TrackToml>,
    #[serde(default, rename = "moment")]
    pub moments: Vec<SeqMomentToml>,
}

fn d60() -> f64 {
    60.0
}
fn d30() -> f64 {
    30.0
}

impl Default for SequenceToml {
    fn default() -> Self {
        SequenceToml { length: 60.0, fps: 30.0, playback: Playback::Oneshot, origin: [0.0, 0.0], canvas: None, tracks: vec![], moments: vec![] }
    }
}

/// Interpolated properties of a track at one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Props {
    pub x: f64,
    pub y: f64,
    pub angle: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub alpha: f64,
    /// Keyed sprite frame, or None to animate at the sprite's own speed.
    pub image: Option<f64>,
}

impl Default for Props {
    fn default() -> Self {
        Props { x: 0.0, y: 0.0, angle: 0.0, scale_x: 1.0, scale_y: 1.0, alpha: 1.0, image: None }
    }
}

const DEFAULTS: [f64; 7] = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0];

/// Value of property `prop` of a track at `frame`. `curve` resolves easing names that are
/// not built in (animation curves): `(name, t) -> eased t`.
pub fn sample_prop(keys: &[Key], prop: usize, frame: f64, curve: &dyn Fn(&str, f64) -> Option<f64>) -> Option<f64> {
    let mut prev: Option<&Key> = None;
    for k in keys.iter().filter(|k| k.get(prop).is_some()) {
        if k.frame > frame {
            let Some(a) = prev else { return k.get(prop) };
            let (va, vb) = (a.get(prop)?, k.get(prop)?);
            let span = k.frame - a.frame;
            if span <= 0.0 {
                return Some(vb);
            }
            let t = (frame - a.frame) / span;
            let e = ease(&a.ease, t).or_else(|| curve(&a.ease, t)).unwrap_or(t);
            // image indexes step between whole frames
            let v = va + (vb - va) * e;
            return Some(if prop == 6 { v.floor() } else { v });
        }
        prev = Some(k);
    }
    prev.and_then(|k| k.get(prop))
}

pub fn sample(track: &TrackToml, frame: f64, curve: &dyn Fn(&str, f64) -> Option<f64>) -> Props {
    let mut v = DEFAULTS;
    for (p, slot) in v.iter_mut().enumerate().take(6) {
        if let Some(x) = sample_prop(&track.keys, p, frame, curve) {
            *slot = x;
        }
    }
    Props { x: v[0], y: v[1], angle: v[2], scale_x: v[3], scale_y: v[4], alpha: v[5], image: sample_prop(&track.keys, 6, frame, curve) }
}

/// Places a track's local point in the room for a sequence at (x, y) with angle and scale.
pub fn transform(local: (f64, f64), origin: [f64; 2], at: (f64, f64), angle: f64, scale: (f64, f64)) -> (f64, f64) {
    let (lx, ly) = ((local.0 - origin[0]) * scale.0, (local.1 - origin[1]) * scale.1);
    let r = angle.to_radians();
    let (c, s) = (r.cos(), r.sin());
    (at.0 + lx * c + ly * s, at.1 - lx * s + ly * c)
}

/// Easing through an animation curve named `curve` or `curve.channel`.
pub fn curve_ease(curves: &[AnimCurve], name: &str, t: f64) -> Option<f64> {
    let (cn, ch) = name.split_once('.').unwrap_or((name, ""));
    let c = curves.iter().find(|c| c.name == cn)?;
    let chan = if ch.is_empty() { c.channels.first() } else { c.channel(ch) }?;
    Some(chan.eval(t))
}

pub fn write_toml(t: &SequenceToml) -> Result<String, String> {
    toml::to_string(t).map_err(|e| e.to_string())
}

// ------------------------------------------------------------------ loaded asset

#[derive(Clone)]
pub struct SequenceDef {
    pub name: String,
    pub data: SequenceToml,
    pub file: Rc<str>,
    /// Asset of each track (sprite / object / sound index).
    pub assets: Vec<Option<u32>>,
    pub moments: Vec<(f64, Rc<Vec<Stmt>>)>,
}

impl SequenceDef {
    pub fn load(name: &str, path: &Path, rel: &str) -> Result<(String, SequenceToml), String> {
        let text = crate::vfs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut t: SequenceToml = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if t.length <= 0.0 || t.fps <= 0.0 {
            return Err(format!("{rel}: length and fps must be positive"));
        }
        for tr in &mut t.tracks {
            tr.sort();
        }
        Ok((name.to_string(), t))
    }

    /// Resolves asset names (after every asset is registered) and parses the moments.
    pub fn resolve(e: &mut Engine, name: &str, rel: &str, t: SequenceToml) -> Result<SequenceDef, String> {
        let mut assets = Vec::new();
        for tr in &t.tracks {
            let want = match tr.kind {
                TrackKind::Sprite => AssetKind::Sprite,
                TrackKind::Object => AssetKind::Object,
                TrackKind::Sound => AssetKind::Sound,
            };
            match e.constant(intern(&tr.asset)) {
                Some(Value::Asset(k, i)) if k == want => assets.push(Some(i)),
                _ if tr.asset.is_empty() => assets.push(None),
                _ => return Err(format!("{rel}: track '{}': {} '{}' does not exist", tr.name, want.name(), tr.asset)),
            }
        }
        let mut moments = Vec::new();
        for m in &t.moments {
            let label = format!("{rel} (moment {})", m.frame);
            let stmts = parse_stmts(&label, &m.code).map_err(|e| format!("{rel}: moment {}: line {}: {}", m.frame, e.line, e.msg))?;
            moments.push((m.frame, Rc::new(stmts)));
        }
        moments.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(SequenceDef { name: name.into(), data: t, file: Rc::from(rel), assets, moments })
    }
}

// ------------------------------------------------------------------ playing in a room

#[derive(Clone, Debug)]
pub struct SeqInst {
    pub id: u32,
    pub seq: u32,
    pub x: f64,
    pub y: f64,
    pub angle: f64,
    pub xscale: f64,
    pub yscale: f64,
    pub depth: f64,
    pub layer: Option<String>,
    pub head: f64,
    /// 1 forwards, -1 backwards.
    pub dir: f64,
    pub speed: f64,
    pub paused: bool,
    pub finished: bool,
    /// Instance created by each object track while it is active.
    pub track_insts: Vec<Option<u32>>,
}

#[derive(Default)]
pub struct SeqRuntime {
    pub list: Vec<SeqInst>,
    pub next_id: u32,
}

impl SeqRuntime {
    pub fn get(&self, id: u32) -> Option<&SeqInst> {
        self.list.iter().find(|s| s.id == id)
    }
    pub fn get_mut(&mut self, id: u32) -> Option<&mut SeqInst> {
        self.list.iter_mut().find(|s| s.id == id)
    }
}

/// Creates a playing sequence element. Returns its id.
pub fn create(e: &mut Engine, seq: u32, x: f64, y: f64, depth: f64, layer: Option<String>) -> u32 {
    e.seqs.next_id += 1;
    let id = e.seqs.next_id;
    let n = e.sequences[seq as usize].data.tracks.len();
    e.seqs.list.push(SeqInst { id, seq, x, y, angle: 0.0, xscale: 1.0, yscale: 1.0, depth, layer, head: 0.0, dir: 1.0, speed: 1.0, paused: false, finished: false, track_insts: vec![None; n] });
    sync_objects(e, id);
    id
}

/// Removes a sequence element and the instances its object tracks created.
pub fn destroy(e: &mut Engine, id: u32) {
    if let Some(i) = e.seqs.list.iter().position(|s| s.id == id) {
        let s = e.seqs.list.remove(i);
        for inst in s.track_insts.into_iter().flatten() {
            e.destroy(inst, true);
        }
    }
}

fn easer(e: &Engine) -> impl Fn(&str, f64) -> Option<f64> + '_ {
    move |name, t| curve_ease(&e.curves, name, t)
}

/// Creates, moves or removes the instances of the object tracks of sequence element `id`.
pub fn sync_objects(e: &mut Engine, id: u32) {
    let Some(s) = e.seqs.get(id).cloned() else { return };
    let def = e.sequences[s.seq as usize].clone();
    let len = def.data.length;
    for (ti, tr) in def.data.tracks.iter().enumerate() {
        if tr.kind != TrackKind::Object {
            continue;
        }
        let Some(obj) = def.assets[ti] else { continue };
        let active = tr.active(s.head, len);
        let current = s.track_insts[ti].filter(|&iid| e.alive(iid));
        if !active {
            if let Some(iid) = current {
                e.destroy(iid, true);
            }
            if let Some(si) = e.seqs.get_mut(id) {
                si.track_insts[ti] = None;
            }
            continue;
        }
        let p = sample(tr, s.head, &easer(e));
        let (x, y) = transform((p.x, p.y), def.data.origin, (s.x, s.y), s.angle, (s.xscale, s.yscale));
        let iid = match current {
            Some(iid) => iid,
            None => {
                let iid = e.create_instance(obj, x, y, s.depth);
                e.init_instance(iid, &[], None, modscript::Scope::Global);
                if let Some(si) = e.seqs.get_mut(id) {
                    si.track_insts[ti] = Some(iid);
                }
                iid
            }
        };
        if let Some(i) = e.idx(iid) {
            let inst = &mut e.insts[i];
            inst.x = x;
            inst.y = y;
            inst.angle = p.angle + s.angle;
            inst.scale_x = p.scale_x * s.xscale;
            inst.scale_y = p.scale_y * s.yscale;
            inst.alpha = p.alpha;
            if let Some(f) = p.image {
                inst.frame = f;
                inst.anim_speed = 0.0;
            }
        }
    }
}

/// Advances every playing sequence one step: moments, sounds, end of playback, objects.
pub fn step(e: &mut Engine) {
    let rate = e.step_rate();
    let ids: Vec<u32> = e.seqs.list.iter().map(|s| s.id).collect();
    for id in ids {
        let Some(s) = e.seqs.get(id).cloned() else { continue };
        if s.paused || s.finished {
            continue;
        }
        let def = e.sequences[s.seq as usize].clone();
        let len = def.data.length;
        let delta = def.data.fps / rate * s.speed * s.dir;
        if delta == 0.0 {
            continue;
        }
        let mut head = s.head + delta;
        let mut dir = s.dir;
        let mut finished = false;
        // frame ranges crossed this step (two when wrapping around)
        let mut spans: Vec<(f64, f64)> = vec![];
        if delta > 0.0 {
            if head >= len {
                match def.data.playback {
                    Playback::Oneshot => {
                        spans.push((s.head, len + 1e-9));
                        head = len;
                        finished = true;
                    }
                    Playback::Loop => {
                        spans.push((s.head, len));
                        head = (head - len).rem_euclid(len.max(1e-9));
                        spans.push((0.0, head));
                    }
                    Playback::Pingpong => {
                        spans.push((s.head, len + 1e-9));
                        head = (2.0 * len - head).max(0.0);
                        dir = -dir;
                    }
                }
            } else {
                spans.push((s.head, head));
            }
        } else if head <= 0.0 {
            match def.data.playback {
                Playback::Oneshot => {
                    spans.push((0.0, s.head));
                    head = 0.0;
                    finished = true;
                }
                Playback::Loop => {
                    spans.push((0.0, s.head));
                    head = head.rem_euclid(len.max(1e-9));
                    spans.push((head, len));
                }
                Playback::Pingpong => {
                    spans.push((0.0, s.head));
                    head = (-head).min(len);
                    dir = -dir;
                }
            }
        } else {
            spans.push((head, s.head));
        }
        if let Some(si) = e.seqs.get_mut(id) {
            si.head = head;
            si.dir = dir;
            si.finished = finished;
        }
        let crossed = |f: f64| spans.iter().any(|&(a, b)| f >= a && f < b);
        for (ti, tr) in def.data.tracks.iter().enumerate() {
            if tr.kind == TrackKind::Sound && crossed(tr.start) {
                if let Some(snd) = def.assets[ti] {
                    let volume = tr.keys.first().and_then(|k| k.alpha).unwrap_or(1.0);
                    e.audio.push(AudioCmd::Play { snd, looped: false, volume });
                }
            }
        }
        for (f, code) in &def.moments {
            if crossed(*f) {
                if let Err(err) = run_stmts(e, code, &def.file, CallCx::global()) {
                    e.report(err);
                }
            }
        }
        sync_objects(e, id);
    }
}

/// Draw list commands for the sprite tracks of sequence element `id`.
pub fn draw(e: &mut Engine, id: u32) {
    let Some(s) = e.seqs.get(id).cloned() else { return };
    let def = &e.sequences[s.seq as usize];
    let len = def.data.length;
    let mut cmds = vec![];
    for (ti, tr) in def.data.tracks.iter().enumerate() {
        if tr.kind != TrackKind::Sprite || !tr.active(s.head, len) {
            continue;
        }
        let Some(spr) = def.assets[ti] else { continue };
        let p = sample(tr, s.head, &easer(e));
        let (x, y) = transform((p.x, p.y), def.data.origin, (s.x, s.y), s.angle, (s.xscale, s.yscale));
        let sp = &e.sprites[spr as usize];
        let n = sp.frames.len().max(1) as f64;
        let frame = match p.image {
            Some(f) => f,
            None => ((s.head - tr.start) / def.data.fps * sp.fps).floor(),
        };
        cmds.push(DrawCmd::Sprite {
            spr,
            frame: frame.rem_euclid(n) as u32,
            x: x as f32,
            y: y as f32,
            sx: (p.scale_x * s.xscale) as f32,
            sy: (p.scale_y * s.yscale) as f32,
            angle: (p.angle + s.angle) as f32,
            color: Rgba::from_rgb(0xFFFFFF, p.alpha),
        });
    }
    e.cmds.extend(cmds);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(frame: f64, x: Option<f64>, ease: &str) -> Key {
        Key { frame, x, ease: ease.into(), ..Default::default() }
    }

    #[test]
    fn interpolation_per_property() {
        let tr = TrackToml { keys: vec![key(0.0, Some(0.0), ""), Key { frame: 10.0, y: Some(5.0), ..Default::default() }, key(20.0, Some(100.0), "")], ..Default::default() };
        let none = |_: &str, _: f64| None;
        let p = sample(&tr, 10.0, &none);
        assert_eq!(p.x, 50.0);
        assert_eq!(p.y, 5.0, "a single key holds its value");
        assert_eq!(sample(&tr, 30.0, &none).x, 100.0);
        assert_eq!(p.scale_x, 1.0);
        let stepped = TrackToml { keys: vec![key(0.0, Some(0.0), "step"), key(10.0, Some(10.0), "")], ..Default::default() };
        assert_eq!(sample(&stepped, 9.0, &none).x, 0.0);
        let curved = TrackToml { keys: vec![key(0.0, Some(0.0), "mycurve"), key(10.0, Some(10.0), "")], ..Default::default() };
        let sq = |n: &str, t: f64| if n == "mycurve" { Some(t * t) } else { None };
        assert!((sample(&curved, 5.0, &sq).x - 2.5).abs() < 1e-9);
    }

    #[test]
    fn transform_rotates_counter_clockwise() {
        let (x, y) = transform((10.0, 0.0), [0.0, 0.0], (100.0, 100.0), 90.0, (1.0, 1.0));
        assert!((x - 100.0).abs() < 1e-9 && (y - 90.0).abs() < 1e-9);
    }

    #[test]
    fn toml_roundtrip() {
        let mut t = SequenceToml::default();
        t.playback = Playback::Pingpong;
        t.tracks.push(TrackToml { name: "a".into(), kind: TrackKind::Object, asset: "Coin".into(), start: 0.0, end: Some(30.0), keys: vec![key(0.0, Some(1.0), "smooth")] });
        t.moments.push(SeqMomentToml { frame: 3.0, code: "x = 1".into() });
        let back: SequenceToml = toml::from_str(&write_toml(&t).unwrap()).unwrap();
        assert_eq!(back, t);
    }
}

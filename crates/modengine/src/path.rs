//! Paths (`paths/<name>.toml`): a list of points instances can follow with
//! `path_start`, straight or smooth, open or closed — like GameMaker paths.
//!
//! ```toml
//! closed = false
//! smooth = true      # curved through the corners (GameMaker's "smooth curve")
//! precision = 4      # 1..8: subdivisions of each curved corner (2^precision)
//! room = "level1"    # only used by the editor as a backdrop
//! [[point]]
//! x = 32
//! y = 64
//! speed = 100        # % of the path speed at this point (optional, default 100)
//! ```

use crate::instance::Instance;
use modscript::{RtError, RtResult, Value};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathPoint {
    pub x: f64,
    pub y: f64,
    #[serde(default = "hundred")]
    pub speed: f64,
}

fn hundred() -> f64 {
    100.0
}

fn four() -> u32 {
    4
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathToml {
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub smooth: bool,
    #[serde(default = "four")]
    pub precision: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    #[serde(default, rename = "point")]
    pub points: Vec<PathPoint>,
}

impl Default for PathToml {
    fn default() -> Self {
        PathToml { closed: false, smooth: false, precision: 4, room: None, points: vec![] }
    }
}

/// A path plus its cached polyline (recomputed whenever it changes).
#[derive(Clone, Debug)]
pub struct PathDef {
    pub name: String,
    pub data: PathToml,
    /// Polyline actually followed: (x, y, speed%).
    pub poly: Vec<(f64, f64, f64)>,
    /// Cumulative length at each polyline point.
    pub cum: Vec<f64>,
    /// false after `path_delete`.
    pub alive: bool,
}

impl PathDef {
    pub fn new(name: &str, data: PathToml) -> Self {
        let mut p = PathDef { name: name.into(), data, poly: vec![], cum: vec![], alive: true };
        p.rebuild();
        p
    }

    pub fn load(name: &str, path: &Path) -> Result<PathDef, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let t: PathToml = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(PathDef::new(name, t))
    }

    /// Recomputes the polyline after editing `data`.
    pub fn rebuild(&mut self) {
        self.data.precision = self.data.precision.clamp(1, 8);
        self.poly = polyline(&self.data);
        self.cum = Vec::with_capacity(self.poly.len());
        let mut acc = 0.0;
        for (i, p) in self.poly.iter().enumerate() {
            if i > 0 {
                let q = self.poly[i - 1];
                acc += (p.0 - q.0).hypot(p.1 - q.1);
            }
            self.cum.push(acc);
        }
    }

    pub fn length(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// Point (x, y, speed%) at `pos` in 0..1 along the path's length.
    pub fn at(&self, pos: f64) -> (f64, f64, f64) {
        match self.poly.len() {
            0 => return (0.0, 0.0, 100.0),
            1 => return self.poly[0],
            _ => {}
        }
        let len = self.length();
        if len <= 0.0 {
            return self.poly[0];
        }
        let d = pos.clamp(0.0, 1.0) * len;
        let i = match self.cum.binary_search_by(|c| c.partial_cmp(&d).unwrap_or(std::cmp::Ordering::Less)) {
            Ok(i) => return self.poly[i],
            Err(i) => i.clamp(1, self.poly.len() - 1),
        };
        let (a, b) = (self.poly[i - 1], self.poly[i]);
        let seg = self.cum[i] - self.cum[i - 1];
        let t = if seg > 0.0 { (d - self.cum[i - 1]) / seg } else { 0.0 };
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
    }

    /// Bounding box of the polyline: (min_x, min_y, max_x, max_y).
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let first = self.poly.first()?;
        Some(self.poly.iter().fold((first.0, first.1, first.0, first.1), |b, p| (b.0.min(p.0), b.1.min(p.1), b.2.max(p.0), b.3.max(p.1))))
    }
}

fn lerp3(a: (f64, f64, f64), b: (f64, f64, f64), t: f64) -> (f64, f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

/// GameMaker's path shapes: straight segments, or a quadratic B-spline whose control
/// points are the path points (the curve passes through the midpoints of the edges and
/// through the end points of open paths).
pub fn polyline(t: &PathToml) -> Vec<(f64, f64, f64)> {
    let pts: Vec<(f64, f64, f64)> = t.points.iter().map(|p| (p.x, p.y, p.speed)).collect();
    let n = pts.len();
    if n < 2 {
        return pts;
    }
    if !t.smooth || n < 3 {
        let mut v = pts.clone();
        if t.closed {
            v.push(pts[0]);
        }
        return v;
    }
    let steps = 1usize << t.precision.clamp(1, 8);
    let mut out = Vec::new();
    let quad = |out: &mut Vec<(f64, f64, f64)>, a: (f64, f64, f64), c: (f64, f64, f64), b: (f64, f64, f64), first: bool| {
        for s in (if first { 0 } else { 1 })..=steps {
            let t = s as f64 / steps as f64;
            let p = lerp3(lerp3(a, c, t), lerp3(c, b, t), t);
            out.push(p);
        }
    };
    let mid = |a: (f64, f64, f64), b: (f64, f64, f64)| lerp3(a, b, 0.5);
    if t.closed {
        for i in 0..n {
            let (p0, p1, p2) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            quad(&mut out, mid(p0, p1), p1, mid(p1, p2), i == 0);
        }
    } else {
        out.push(pts[0]);
        // first half-edge straight to the first midpoint, then curved corners
        for i in 1..n - 1 {
            let (p0, p1, p2) = (pts[i - 1], pts[i], pts[i + 1]);
            let a = if i == 1 { p0 } else { mid(p0, p1) };
            let b = if i == n - 2 { p2 } else { mid(p1, p2) };
            quad(&mut out, a, p1, b, false);
        }
    }
    out
}

pub fn write_toml(t: &PathToml) -> String {
    let f = crate::animcurve::fmt;
    let mut s = format!("closed = {}\nsmooth = {}\nprecision = {}\n", t.closed, t.smooth, t.precision);
    if let Some(r) = &t.room {
        s.push_str(&format!("room = {}\n", toml::Value::String(r.clone())));
    }
    for p in &t.points {
        s.push_str(&format!("\n[[point]]\nx = {}\ny = {}\n", f(p.x), f(p.y)));
        if p.speed != 100.0 {
            s.push_str(&format!("speed = {}\n", f(p.speed)));
        }
    }
    s
}

// ------------------------------------------------------------------ following a path

/// What to do when an instance reaches the end of its path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EndAction {
    #[default]
    Stop,
    Restart,
    Continue,
    Reverse,
}

impl EndAction {
    pub fn from_num(n: f64) -> EndAction {
        match n as i64 {
            1 => EndAction::Restart,
            2 => EndAction::Continue,
            3 => EndAction::Reverse,
            _ => EndAction::Stop,
        }
    }
    pub fn num(self) -> f64 {
        match self {
            EndAction::Stop => 0.0,
            EndAction::Restart => 1.0,
            EndAction::Continue => 2.0,
            EndAction::Reverse => 3.0,
        }
    }
}

/// Path state of one instance (`path_index`, `path_position`...).
#[derive(Clone, Debug, PartialEq)]
pub struct PathFollow {
    pub index: Option<u32>,
    pub position: f64,
    pub position_prev: f64,
    pub speed: f64,
    pub scale: f64,
    /// Rotation of the path in degrees (counter-clockwise).
    pub orientation: f64,
    pub end_action: EndAction,
    /// Where the path's first point is placed in the room.
    pub anchor: (f64, f64),
}

impl Default for PathFollow {
    fn default() -> Self {
        PathFollow { index: None, position: 0.0, position_prev: 0.0, speed: 0.0, scale: 1.0, orientation: 0.0, end_action: EndAction::Stop, anchor: (0.0, 0.0) }
    }
}

impl PathFollow {
    /// Room position of the point at `pos` of `p` with this instance's anchor, scale and orientation.
    pub fn place(&self, p: &PathDef, pos: f64) -> (f64, f64) {
        let (x, y, _) = p.at(pos);
        let (x0, y0, _) = p.at(0.0);
        let (dx, dy) = ((x - x0) * self.scale, (y - y0) * self.scale);
        let r = self.orientation.to_radians();
        let (c, s) = (r.cos(), r.sin());
        (self.anchor.0 + dx * c + dy * s, self.anchor.1 - dx * s + dy * c)
    }
}

/// Built-in path variables of an instance.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PathVar {
    Index,
    Position,
    PositionPrevious,
    Speed,
    Scale,
    Orientation,
    EndAction,
}

pub const VARS: &[(&str, PathVar)] = &[
    ("path_index", PathVar::Index),
    ("path_position", PathVar::Position),
    ("path_positionprevious", PathVar::PositionPrevious),
    ("path_speed", PathVar::Speed),
    ("path_scale", PathVar::Scale),
    ("path_orientation", PathVar::Orientation),
    ("path_endaction", PathVar::EndAction),
];

pub fn get_var(inst: &Instance, v: PathVar) -> Value {
    let p = &inst.path;
    match v {
        PathVar::Index => p.index.map(|i| Value::Asset(modscript::AssetKind::Path, i)).unwrap_or(Value::Num(-1.0)),
        PathVar::Position => Value::Num(p.position),
        PathVar::PositionPrevious => Value::Num(p.position_prev),
        PathVar::Speed => Value::Num(p.speed),
        PathVar::Scale => Value::Num(p.scale),
        PathVar::Orientation => Value::Num(p.orientation),
        PathVar::EndAction => Value::Num(p.end_action.num()),
    }
}

pub fn set_var(inst: &mut Instance, v: PathVar, val: Value) -> RtResult<()> {
    let num = |val: &Value, n: &str| val.as_num().ok_or_else(|| RtError::new(format!("'{n}' must be a number, got {}", val.type_name())));
    let p = &mut inst.path;
    match v {
        PathVar::Index => return Err(RtError::new("'path_index' is read-only: use path_start(path, speed, end_action, absolute) or path_end()")),
        PathVar::Position => p.position = num(&val, "path_position")?.clamp(0.0, 1.0),
        PathVar::PositionPrevious => p.position_prev = num(&val, "path_positionprevious")?,
        PathVar::Speed => p.speed = num(&val, "path_speed")?,
        PathVar::Scale => p.scale = num(&val, "path_scale")?,
        PathVar::Orientation => p.orientation = num(&val, "path_orientation")?,
        PathVar::EndAction => p.end_action = EndAction::from_num(num(&val, "path_endaction")?),
    }
    Ok(())
}

/// Advances one instance along its path. Returns true when the path ended (stop action).
pub fn advance(inst: &mut Instance, p: &PathDef) -> bool {
    let len = p.length() * inst.path.scale.abs();
    let st = &mut inst.path;
    st.position_prev = st.position;
    let mut ended = false;
    if len > 0.0 {
        let (_, _, ps) = p.at(st.position);
        st.position += st.speed * (ps / 100.0) / len;
        if st.position >= 1.0 || st.position <= 0.0 {
            let forward = st.position >= 1.0;
            let over = if forward { st.position - 1.0 } else { -st.position };
            match st.end_action {
                EndAction::Stop => {
                    st.position = st.position.clamp(0.0, 1.0);
                    ended = true;
                }
                EndAction::Restart => st.position = if forward { over.min(1.0) } else { 1.0 - over.min(1.0) },
                EndAction::Continue => {
                    // the next lap starts where this one ended
                    let end = st.place(p, if forward { 1.0 } else { 0.0 });
                    let start = st.place(p, if forward { 0.0 } else { 1.0 });
                    st.anchor.0 += end.0 - start.0;
                    st.anchor.1 += end.1 - start.1;
                    st.position = if forward { over.min(1.0) } else { 1.0 - over.min(1.0) };
                }
                EndAction::Reverse => {
                    st.speed = -st.speed;
                    st.position = if forward { 1.0 - over.min(1.0) } else { over.min(1.0) };
                }
            }
        }
    } else {
        ended = st.end_action == EndAction::Stop;
    }
    let (x, y) = st.place(p, st.position);
    let (dx, dy) = (x - inst.x, y - inst.y);
    if dx != 0.0 || dy != 0.0 {
        inst.dir = (-dy).atan2(dx).to_degrees().rem_euclid(360.0);
    }
    inst.x = x;
    inst.y = y;
    if ended {
        inst.path.index = None;
    }
    ended
}

/// Starts instance `inst` on path `pi` (GameMaker's path_start).
pub fn start(inst: &mut Instance, pi: u32, p: &PathDef, speed: f64, end_action: EndAction, absolute: bool) {
    let (x0, y0, _) = p.at(0.0);
    let st = &mut inst.path;
    st.index = Some(pi);
    st.speed = speed;
    st.end_action = end_action;
    st.position = if speed < 0.0 { 1.0 } else { 0.0 };
    st.position_prev = st.position;
    st.anchor = if absolute { (x0, y0) } else { (inst.x, inst.y) };
    if !absolute && speed < 0.0 {
        // a relative path walked backwards starts at its end point, where the instance is
        let (x1, y1) = st.place(p, 1.0);
        st.anchor = (2.0 * inst.x - x1, 2.0 * inst.y - y1);
    }
    inst.hspeed = 0.0;
    inst.vspeed = 0.0;
    if absolute {
        let (x, y) = inst.path.place(p, inst.path.position);
        inst.x = x;
        inst.y = y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(smooth: bool, closed: bool) -> PathDef {
        let pts = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
        PathDef::new("sq", PathToml { closed, smooth, precision: 4, room: None, points: pts.iter().map(|&(x, y)| PathPoint { x, y, speed: 100.0 }).collect() })
    }

    #[test]
    fn straight_lengths_and_positions() {
        let open = square(false, false);
        assert_eq!(open.length(), 300.0);
        assert_eq!(open.at(0.5), (100.0, 50.0, 100.0));
        let closed = square(false, true);
        assert_eq!(closed.length(), 400.0);
        assert_eq!(closed.at(1.0), (0.0, 0.0, 100.0));
    }

    #[test]
    fn smooth_paths_cut_corners_but_keep_open_ends() {
        let s = square(true, false);
        assert_eq!(s.at(0.0), (0.0, 0.0, 100.0));
        let end = s.at(1.0);
        assert!((end.0 - 0.0).abs() < 1e-9 && (end.1 - 100.0).abs() < 1e-9);
        assert!(s.length() < 300.0 && s.length() > 200.0);
        let c = square(true, true);
        // closed smooth square: a rounded shape that never touches the corners
        assert!(c.poly.iter().all(|p| !(p.0 == 100.0 && p.1 == 0.0)));
    }

    #[test]
    fn follow_with_end_actions() {
        let p = square(false, false);
        let mut inst = Instance::new(1, 0, 10.0, 10.0, 0.0);
        start(&mut inst, 0, &p, 75.0, EndAction::Stop, false);
        for _ in 0..3 {
            assert!(!advance(&mut inst, &p));
        }
        assert!((inst.x - 85.0).abs() < 1e-9 && (inst.y - 110.0).abs() < 1e-9, "{} {}", inst.x, inst.y);
        assert!(advance(&mut inst, &p));
        assert_eq!(inst.path.index, None);
        assert!((inst.x - 10.0).abs() < 1e-9 && (inst.y - 110.0).abs() < 1e-9);

        let mut r = Instance::new(2, 0, 0.0, 0.0, 0.0);
        start(&mut r, 0, &p, 100.0, EndAction::Reverse, true);
        for _ in 0..4 {
            advance(&mut r, &p);
        }
        assert!(r.path.speed < 0.0 && r.path.index.is_some());

        let mut c = Instance::new(3, 0, 0.0, 0.0, 0.0);
        start(&mut c, 0, &p, 150.0, EndAction::Continue, true);
        advance(&mut c, &p);
        advance(&mut c, &p);
        // after one lap the path restarts from where it ended: (0,100) is the new origin
        assert!((c.path.anchor.1 - 100.0).abs() < 1e-9);
    }

    #[test]
    fn toml_roundtrip() {
        let p = square(true, true);
        let back: PathToml = toml::from_str(&write_toml(&p.data)).unwrap();
        assert_eq!(back, p.data);
    }
}

//! Animation curves (`curves/<name>.toml`): named channels of points `(x, y)` with
//! x in 0..1, evaluated with linear, smooth (Catmull-Rom) or bezier interpolation.
//!
//! ```toml
//! [[channel]]
//! name = "y"
//! type = "smooth"            # linear | smooth | bezier
//! points = [[0, 0], [0.5, 1], [1, 0]]
//! # bezier points may carry handles: [x, y, in_dx, in_dy, out_dx, out_dy]
//! ```

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CurveType {
    #[default]
    Linear,
    /// Catmull-Rom through every point (GameMaker's "smooth").
    Smooth,
    /// Cubic bezier with a handle on each side of every point.
    Bezier,
}

impl CurveType {
    pub fn name(self) -> &'static str {
        match self {
            CurveType::Linear => "linear",
            CurveType::Smooth => "smooth",
            CurveType::Bezier => "bezier",
        }
    }
    pub fn parse(s: &str) -> Option<CurveType> {
        Some(match s {
            "linear" => CurveType::Linear,
            "smooth" | "catmullrom" | "catmull_rom" => CurveType::Smooth,
            "bezier" => CurveType::Bezier,
            _ => return None,
        })
    }
    pub const ALL: [CurveType; 3] = [CurveType::Linear, CurveType::Smooth, CurveType::Bezier];
}

/// One point of a channel. Handles are offsets from the point (bezier only); `None` means
/// "automatic" (flat, a third of the way to the neighbour).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CurvePoint {
    pub x: f64,
    pub y: f64,
    pub h_in: Option<(f64, f64)>,
    pub h_out: Option<(f64, f64)>,
}

impl CurvePoint {
    pub fn new(x: f64, y: f64) -> Self {
        CurvePoint { x, y, h_in: None, h_out: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelToml {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: CurveType,
    /// Samples per segment used when drawing (GameMaker's "iterations"); evaluation is exact.
    #[serde(default = "default_iterations")]
    pub iterations: u32,
    #[serde(default)]
    pub points: Vec<Vec<f64>>,
}

fn default_iterations() -> u32 {
    16
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct CurveToml {
    #[serde(default, rename = "channel")]
    pub channels: Vec<ChannelToml>,
}

/// A ready-to-evaluate channel.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub name: String,
    pub kind: CurveType,
    pub iterations: u32,
    /// Sorted by x.
    pub points: Vec<CurvePoint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimCurve {
    pub name: String,
    pub channels: Vec<Channel>,
}

impl Channel {
    pub fn new(name: &str, kind: CurveType, points: Vec<CurvePoint>) -> Self {
        let mut c = Channel { name: name.into(), kind, iterations: 16, points };
        c.sort();
        c
    }

    pub fn sort(&mut self) {
        self.points.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    }

    pub fn from_toml(t: &ChannelToml) -> Result<Channel, String> {
        let mut pts = Vec::new();
        for p in &t.points {
            let pt = match p.as_slice() {
                [x, y] => CurvePoint::new(*x, *y),
                [x, y, ix, iy, ox, oy] => CurvePoint { x: *x, y: *y, h_in: Some((*ix, *iy)), h_out: Some((*ox, *oy)) },
                _ => return Err(format!("channel '{}': points are [x, y] or [x, y, in_dx, in_dy, out_dx, out_dy]", t.name)),
            };
            if !pt.x.is_finite() || !pt.y.is_finite() {
                return Err(format!("channel '{}': point values must be numbers", t.name));
            }
            pts.push(pt);
        }
        let mut c = Channel { name: t.name.clone(), kind: t.kind, iterations: t.iterations.max(1), points: pts };
        c.sort();
        Ok(c)
    }

    pub fn to_toml(&self) -> ChannelToml {
        ChannelToml {
            name: self.name.clone(),
            kind: self.kind,
            iterations: self.iterations,
            points: self
                .points
                .iter()
                .map(|p| match (p.h_in, p.h_out) {
                    (None, None) => vec![p.x, p.y],
                    (i, o) => {
                        let (i, o) = (i.unwrap_or((0.0, 0.0)), o.unwrap_or((0.0, 0.0)));
                        vec![p.x, p.y, i.0, i.1, o.0, o.1]
                    }
                })
                .collect(),
        }
    }

    /// Handle offsets of point `i` (explicit or automatic), clamped so the curve stays a
    /// function of x.
    pub fn handles(&self, i: usize) -> ((f64, f64), (f64, f64)) {
        let p = &self.points[i];
        let left = if i > 0 { p.x - self.points[i - 1].x } else { 0.0 };
        let right = if i + 1 < self.points.len() { self.points[i + 1].x - p.x } else { 0.0 };
        let hin = p.h_in.unwrap_or((-left / 3.0, 0.0));
        let hout = p.h_out.unwrap_or((right / 3.0, 0.0));
        ((hin.0.clamp(-left, 0.0), hin.1), (hout.0.clamp(0.0, right), hout.1))
    }

    /// Value of the channel at `x` (clamped to the first/last point outside their range).
    pub fn eval(&self, x: f64) -> f64 {
        let pts = &self.points;
        match pts.len() {
            0 => return 0.0,
            1 => return pts[0].y,
            _ => {}
        }
        if x <= pts[0].x {
            return pts[0].y;
        }
        let n = pts.len();
        if x >= pts[n - 1].x {
            return pts[n - 1].y;
        }
        let i = pts.windows(2).position(|w| x >= w[0].x && x <= w[1].x).unwrap_or(n - 2);
        let (a, b) = (&pts[i], &pts[i + 1]);
        let span = b.x - a.x;
        if span <= 1e-12 {
            return b.y;
        }
        let t = (x - a.x) / span;
        match self.kind {
            CurveType::Linear => a.y + (b.y - a.y) * t,
            CurveType::Smooth => {
                let p0 = if i > 0 { pts[i - 1].y } else { a.y };
                let p3 = if i + 2 < n { pts[i + 2].y } else { b.y };
                catmull_rom(p0, a.y, b.y, p3, t)
            }
            CurveType::Bezier => {
                let (_, ho) = self.handles(i);
                let (hi, _) = self.handles(i + 1);
                let (x0, y0) = (a.x, a.y);
                let (x1, y1) = (a.x + ho.0, a.y + ho.1);
                let (x2, y2) = (b.x + hi.0, b.y + hi.1);
                let (x3, y3) = (b.x, b.y);
                // x(s) is monotonic because the handles are clamped: bisection is exact enough
                let (mut lo, mut hi_) = (0.0f64, 1.0f64);
                for _ in 0..48 {
                    let mid = (lo + hi_) / 2.0;
                    if cubic(x0, x1, x2, x3, mid) < x {
                        lo = mid;
                    } else {
                        hi_ = mid;
                    }
                }
                cubic(y0, y1, y2, y3, (lo + hi_) / 2.0)
            }
        }
    }
}

fn cubic(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
}

pub fn catmull_rom(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1) + (-p0 + p2) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

impl AnimCurve {
    pub fn from_toml(name: &str, t: &CurveToml) -> Result<AnimCurve, String> {
        let channels = t.channels.iter().map(Channel::from_toml).collect::<Result<Vec<_>, _>>()?;
        Ok(AnimCurve { name: name.into(), channels })
    }

    pub fn to_toml(&self) -> CurveToml {
        CurveToml { channels: self.channels.iter().map(|c| c.to_toml()).collect() }
    }

    pub fn load(name: &str, path: &Path) -> Result<AnimCurve, String> {
        let text = crate::vfs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let t: CurveToml = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        AnimCurve::from_toml(name, &t).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn channel(&self, name: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.name == name)
    }
}

pub fn write_toml(c: &CurveToml) -> String {
    let mut s = String::new();
    for ch in &c.channels {
        s.push_str("[[channel]]\n");
        s.push_str(&format!("name = {}\n", toml::Value::String(ch.name.clone())));
        s.push_str(&format!("type = \"{}\"\n", ch.kind.name()));
        if ch.iterations != 16 {
            s.push_str(&format!("iterations = {}\n", ch.iterations));
        }
        let pts: Vec<String> = ch.points.iter().map(|p| format!("[{}]", p.iter().map(|v| fmt(*v)).collect::<Vec<_>>().join(", "))).collect();
        s.push_str(&format!("points = [{}]\n\n", pts.join(", ")));
    }
    s
}

pub(crate) fn fmt(v: f64) -> String {
    let r = (v * 10000.0).round() / 10000.0;
    if r == r.trunc() && r.abs() < 1e15 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

// ------------------------------------------------------------------ presets

/// Ready-made shapes for the editor (and `animcurve_channel_new` callers): name, type, points.
pub fn preset(name: &str) -> Option<(CurveType, Vec<CurvePoint>)> {
    let p = CurvePoint::new;
    let sampled = |f: &dyn Fn(f64) -> f64, n: usize| -> Vec<CurvePoint> { (0..=n).map(|i| i as f64 / n as f64).map(|x| p(x, f(x))).collect() };
    Some(match name {
        "linear" => (CurveType::Linear, vec![p(0.0, 0.0), p(1.0, 1.0)]),
        "ease_in" => (CurveType::Bezier, vec![CurvePoint { x: 0.0, y: 0.0, h_in: None, h_out: Some((0.42, 0.0)) }, CurvePoint { x: 1.0, y: 1.0, h_in: Some((0.0, 0.0)), h_out: None }]),
        "ease_out" => (CurveType::Bezier, vec![CurvePoint { x: 0.0, y: 0.0, h_in: None, h_out: Some((0.0, 0.0)) }, CurvePoint { x: 1.0, y: 1.0, h_in: Some((-0.42, 0.0)), h_out: None }]),
        "ease_in_out" => (CurveType::Bezier, vec![CurvePoint { x: 0.0, y: 0.0, h_in: None, h_out: Some((0.42, 0.0)) }, CurvePoint { x: 1.0, y: 1.0, h_in: Some((-0.42, 0.0)), h_out: None }]),
        "smooth_step" => (CurveType::Smooth, vec![p(0.0, 0.0), p(0.5, 0.5), p(1.0, 1.0)]),
        "bounce" => (CurveType::Linear, sampled(&bounce_out, 40)),
        "elastic" => (CurveType::Smooth, sampled(&|x| if x <= 0.0 || x >= 1.0 { x.clamp(0.0, 1.0) } else { 2f64.powf(-10.0 * x) * ((x * 10.0 - 0.75) * (2.0 * std::f64::consts::PI / 3.0)).sin() + 1.0 }, 32)),
        "back" => (CurveType::Smooth, sampled(&|x| { let c1 = 1.70158; let c3 = c1 + 1.0; c3 * x * x * x - c1 * x * x }, 12)),
        "sine" => (CurveType::Smooth, sampled(&|x| (x * 2.0 * std::f64::consts::PI).sin(), 16)),
        "step" => (CurveType::Linear, vec![p(0.0, 0.0), p(0.4999, 0.0), p(0.5, 1.0), p(1.0, 1.0)]),
        "constant" => (CurveType::Linear, vec![p(0.0, 1.0), p(1.0, 1.0)]),
        // GameMaker's material-style curves
        "fast_out_slow_in" => (CurveType::Bezier, vec![CurvePoint { x: 0.0, y: 0.0, h_in: None, h_out: Some((0.4, 0.0)) }, CurvePoint { x: 1.0, y: 1.0, h_in: Some((-0.8, 0.0)), h_out: None }]),
        "slow_middle" => (CurveType::Bezier, vec![CurvePoint { x: 0.0, y: 0.0, h_in: None, h_out: Some((0.0, 0.6)) }, CurvePoint { x: 1.0, y: 1.0, h_in: Some((0.0, -0.6)), h_out: None }]),
        // the Penner family: cubic_in, quart_out, expo_inout, circ_in, back_out, elastic_inout, bounce_in...
        other => {
            other.rsplit_once('_')?;
            ease(other, 0.5)?;
            let n = if other.starts_with("elastic") || other.starts_with("bounce") { 48 } else { 24 };
            (CurveType::Linear, sampled(&|x| ease(other, x).unwrap_or(x), n))
        }
    })
}

/// "In" shape of the Penner easing families (out and in-out are derived from it).
fn penner_in(family: &str, t: f64) -> Option<f64> {
    use std::f64::consts::PI;
    Some(match family {
        "quad" => t * t,
        "cubic" => t * t * t,
        "quart" => t.powi(4),
        "quint" => t.powi(5),
        "sine" => 1.0 - (t * PI / 2.0).cos(),
        "expo" => {
            if t <= 0.0 {
                0.0
            } else {
                2f64.powf(10.0 * t - 10.0)
            }
        }
        "circ" => 1.0 - (1.0 - t * t).max(0.0).sqrt(),
        "back" => {
            let c1 = 1.70158;
            (c1 + 1.0) * t * t * t - c1 * t * t
        }
        "elastic" => {
            if t <= 0.0 || t >= 1.0 {
                t
            } else {
                -(2f64.powf(10.0 * t - 10.0)) * ((t * 10.0 - 10.75) * (2.0 * PI / 3.0)).sin()
            }
        }
        "bounce" => 1.0 - bounce_out(1.0 - t),
        _ => return None,
    })
}

pub const PRESETS: &[(&str, &str)] = &[
    ("linear", "Lineal"),
    ("ease_in", "Ease in"),
    ("ease_out", "Ease out"),
    ("ease_in_out", "Ease in-out"),
    ("smooth_step", "Suave"),
    ("bounce", "Rebote"),
    ("elastic", "Elástico"),
    ("back", "Retroceso"),
    ("sine", "Seno (onda)"),
    ("step", "Escalón"),
    ("constant", "Constante"),
];

/// The rest of GameMaker's curve library: `<family>_<in|out|inout>` plus two material curves.
pub const MORE_PRESETS: &[&str] = &[
    "cubic_in", "cubic_out", "cubic_inout", "quart_in", "quart_out", "quart_inout", "expo_in", "expo_out", "expo_inout", "circ_in", "circ_out",
    "circ_inout", "back_in", "back_out", "back_inout", "elastic_in", "elastic_out", "elastic_inout", "bounce_in", "bounce_out", "bounce_inout",
    "quad_in", "quad_out", "quad_inout", "quint_in", "quint_out", "quint_inout", "sine_in", "sine_out", "sine_inout", "fast_out_slow_in", "slow_middle",
];

fn bounce_out(x: f64) -> f64 {
    let (n1, d1) = (7.5625, 2.75);
    if x < 1.0 / d1 {
        n1 * x * x
    } else if x < 2.0 / d1 {
        let x = x - 1.5 / d1;
        n1 * x * x + 0.75
    } else if x < 2.5 / d1 {
        let x = x - 2.25 / d1;
        n1 * x * x + 0.9375
    } else {
        let x = x - 2.625 / d1;
        n1 * x * x + 0.984375
    }
}

/// Easing by name used by sequence keyframes: `linear`, `smooth`, `step`, `ease_in`...
/// Returns None for unknown names (sequences then look for an animation curve).
pub fn ease(name: &str, t: f64) -> Option<f64> {
    let t = t.clamp(0.0, 1.0);
    Some(match name {
        "" | "linear" => t,
        "smooth" => t * t * (3.0 - 2.0 * t),
        "step" => 0.0,
        "ease_in" => t * t * t,
        "ease_out" => 1.0 - (1.0 - t).powi(3),
        "ease_in_out" => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        "bounce" => bounce_out(t),
        other => {
            // cubic_in, expo_out, back_inout...
            let (family, mode) = other.rsplit_once('_')?;
            match mode {
                "in" => penner_in(family, t)?,
                "out" => 1.0 - penner_in(family, 1.0 - t)?,
                "inout" | "in_out" => {
                    if t < 0.5 {
                        penner_in(family, 2.0 * t)? / 2.0
                    } else {
                        1.0 - penner_in(family, 2.0 - 2.0 * t)? / 2.0
                    }
                }
                _ => return None,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_smooth_and_bezier() {
        let pts = vec![CurvePoint::new(0.0, 0.0), CurvePoint::new(0.5, 1.0), CurvePoint::new(1.0, 0.0)];
        let l = Channel::new("a", CurveType::Linear, pts.clone());
        assert!((l.eval(0.25) - 0.5).abs() < 1e-9);
        assert_eq!(l.eval(-1.0), 0.0);
        assert_eq!(l.eval(2.0), 0.0);
        let s = Channel::new("a", CurveType::Smooth, pts.clone());
        assert!((s.eval(0.5) - 1.0).abs() < 1e-9);
        assert!(s.eval(0.25) > 0.5, "catmull-rom bulges above the straight line");
        let (_, pts) = preset("ease_in_out").unwrap();
        let b = Channel::new("e", CurveType::Bezier, pts);
        assert!((b.eval(0.5) - 0.5).abs() < 1e-6);
        assert!(b.eval(0.1) < 0.1 && b.eval(0.9) > 0.9);
        assert!((b.eval(1.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn toml_roundtrip() {
        let src = "[[channel]]\nname = \"x\"\ntype = \"bezier\"\npoints = [[0, 0, 0, 0, 0.3, 0], [1, 1]]\n\n[[channel]]\nname = \"y\"\ntype = \"smooth\"\npoints = [[0, 1], [1, 0]]\n";
        let t: CurveToml = toml::from_str(src).unwrap();
        let c = AnimCurve::from_toml("c", &t).unwrap();
        assert_eq!(c.channels.len(), 2);
        assert_eq!(c.channels[0].points[0].h_out, Some((0.3, 0.0)));
        let again: CurveToml = toml::from_str(&write_toml(&c.to_toml())).unwrap();
        assert_eq!(AnimCurve::from_toml("c", &again).unwrap(), c);
        for k in PRESETS.iter().map(|p| p.0).chain(MORE_PRESETS.iter().copied()) {
            let (ty, pts) = preset(k).unwrap_or_else(|| panic!("{k}"));
            let ch = Channel::new(k, ty, pts);
            assert!(ch.eval(0.0).abs() < 1e-6 || k == "constant" || k == "sine", "{k} starts at 0");
            assert!((ch.eval(1.0) - 1.0).abs() < 1e-6 || k == "sine", "{k} ends at 1");
        }
    }
}

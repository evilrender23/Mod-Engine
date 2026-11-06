//! Procedural dungeons from room templates (`dungeon_*`, `modengine dungeon`).
//!
//! The generator is a fixed pipeline of layers, each configurable from a config
//! (ModScript struct, TOML or JSON, see docs/mazmorras.md):
//!
//! 1. **rooms**: pick room shapes (ASCII templates or plain rectangles) and scatter them
//!    around the centre of the map;
//! 2. **separation**: push overlapping rooms apart (springs) and snap them to cells; rooms
//!    that still collide are moved to free spots or dropped;
//! 3. **graph**: Delaunay triangulation of the room centres → minimum spanning tree →
//!    a share of the unused edges added back as loops;
//! 4. **corridors**: A* over the cells between rooms (turn cost, noise, reuse of
//!    corridors) from a door spot of one room to a door spot of the other;
//! 5. **analysis**: start / exit, distance from the start, main path, room graph depth,
//!    dead ends;
//! 6. **events**: rules (`{type: "enemy", count: [4, 7], where: "room"}`) turned into
//!    events with a cell and a room.
//!
//! Everything comes from one seed: the same config and seed give the same dungeon.
//! Nothing here touches the engine; `natives/dungeon.rs` wraps it for scripts and the
//! CLI renders it to PNG with [`render_rgba`].

use serde_json::{json, Map, Value as J};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

// ------------------------------------------------------------------ cells

/// Room floor (walkable).
pub const FLOOR: u8 = 0;
/// Solid wall (not walkable).
pub const WALL: u8 = 1;
/// Door between a room and a corridor (walkable).
pub const DOOR: u8 = 2;
/// Corridor floor (walkable).
pub const CORRIDOR: u8 = 3;

pub fn walkable(c: u8) -> bool {
    c != WALL
}

// ------------------------------------------------------------------ random numbers

/// xorshift64* seeded with splitmix64 (the same family as `rng_create`).
#[derive(Clone, Debug)]
pub struct Rng(u64);

pub fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) | 1
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(mix(seed))
    }
    /// In [0, 1).
    pub fn f(&mut self) -> f64 {
        crate::natives::rand01(&mut self.0)
    }
    /// In [lo, hi] (inclusive).
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        let (lo, hi) = (lo.min(hi), lo.max(hi));
        (lo + (self.f() * (hi - lo + 1) as f64) as i64).min(hi)
    }
    pub fn index(&mut self, n: usize) -> usize {
        ((self.f() * n as f64) as usize).min(n.saturating_sub(1))
    }
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.index(i + 1);
            v.swap(i, j);
        }
    }
    /// Index chosen by weight (None if every weight is 0).
    pub fn weighted(&mut self, w: &[f64]) -> Option<usize> {
        let total: f64 = w.iter().map(|x| x.max(0.0)).sum();
        if total <= 0.0 {
            return None;
        }
        let mut r = self.f() * total;
        for (i, x) in w.iter().enumerate() {
            let x = x.max(0.0);
            if r < x {
                return Some(i);
            }
            r -= x;
        }
        w.iter().rposition(|x| *x > 0.0)
    }
}

/// A seed from a number or a text (FNV-1a), like `rng_create`.
pub fn seed_from_text(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

// ------------------------------------------------------------------ geometry helpers

/// Delaunay triangulation (Bowyer–Watson). Returns triangles as point indices
/// (counter-clockwise not guaranteed). Duplicate points are ignored; collinear sets
/// give no triangles (use [`delaunay_edges`], which falls back to a chain).
pub fn delaunay(points: &[(f64, f64)]) -> Vec<[usize; 3]> {
    let n = points.len();
    if n < 3 {
        return vec![];
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in points {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let d = (x1 - x0).max(y1 - y0).max(1.0);
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let mut pts: Vec<(f64, f64)> = points.to_vec();
    pts.push((mx - 40.0 * d, my - 20.0 * d));
    pts.push((mx, my + 40.0 * d));
    pts.push((mx + 40.0 * d, my - 20.0 * d));
    struct Tri {
        v: [usize; 3],
        c: (f64, f64, f64),
    }
    let circ = |v: [usize; 3], pts: &[(f64, f64)]| -> (f64, f64, f64) {
        let (ax, ay) = pts[v[0]];
        let (bx, by) = pts[v[1]];
        let (cx, cy) = pts[v[2]];
        let dd = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
        if dd.abs() < 1e-12 {
            return (0.0, 0.0, f64::INFINITY);
        }
        let (a2, b2, c2) = (ax * ax + ay * ay, bx * bx + by * by, cx * cx + cy * cy);
        let ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / dd;
        let uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / dd;
        (ux, uy, (ax - ux).powi(2) + (ay - uy).powi(2))
    };
    let s = [n, n + 1, n + 2];
    let mut tris = vec![Tri { v: s, c: circ(s, &pts) }];
    let mut seen: Vec<(f64, f64)> = Vec::with_capacity(n);
    for i in 0..n {
        let (px, py) = pts[i];
        if seen.iter().any(|&(x, y)| (x - px).abs() < 1e-9 && (y - py).abs() < 1e-9) {
            continue;
        }
        seen.push((px, py));
        let mut edges: Vec<(usize, usize)> = vec![];
        tris.retain(|t| {
            let (cx, cy, r2) = t.c;
            let d2 = (px - cx).powi(2) + (py - cy).powi(2);
            let inside = r2.is_infinite() || d2 < r2 - 1e-9 * r2.max(1.0);
            if inside {
                for k in 0..3 {
                    let (a, b) = (t.v[k], t.v[(k + 1) % 3]);
                    edges.push((a.min(b), a.max(b)));
                }
            }
            !inside
        });
        edges.sort_unstable();
        let mut k = 0;
        while k < edges.len() {
            if k + 1 < edges.len() && edges[k] == edges[k + 1] {
                let e = edges[k];
                while k < edges.len() && edges[k] == e {
                    k += 1;
                }
                continue;
            }
            let (a, b) = edges[k];
            let v = [a, b, i];
            tris.push(Tri { v, c: circ(v, &pts) });
            k += 1;
        }
    }
    tris.into_iter()
        .filter(|t| t.v.iter().all(|&v| v < n))
        .filter(|t| {
            let (a, b, c) = (pts[t.v[0]], pts[t.v[1]], pts[t.v[2]]);
            ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)).abs() > 1e-9
        })
        .map(|t| t.v)
        .collect()
}

/// Unique edges `(a, b)` with a < b of a set of triangles.
pub fn triangle_edges(tris: &[[usize; 3]]) -> Vec<(usize, usize)> {
    let mut e: Vec<(usize, usize)> = tris.iter().flat_map(|t| (0..3).map(move |k| (t[k].min(t[(k + 1) % 3]), t[k].max(t[(k + 1) % 3])))).collect();
    e.sort_unstable();
    e.dedup();
    e
}

/// Delaunay edges of the points, always connecting every point: collinear or duplicate
/// points are joined to their nearest component (so a spanning tree always exists).
pub fn delaunay_edges(points: &[(f64, f64)]) -> Vec<(usize, usize)> {
    let mut e = triangle_edges(&delaunay(points));
    let n = points.len();
    if n >= 2 {
        // connect leftover components with the shortest edges between them
        let mut uf = UnionFind::new(n);
        for &(a, b) in &e {
            uf.union(a, b);
        }
        let mut all: Vec<(f64, usize, usize)> = vec![];
        for a in 0..n {
            for b in a + 1..n {
                if uf.find(a) != uf.find(b) {
                    all.push((dist(points[a], points[b]), a, b));
                }
            }
        }
        all.sort_by(|x, y| x.partial_cmp(y).unwrap_or(Ordering::Equal));
        for (_, a, b) in all {
            if uf.union(a, b) {
                e.push((a, b));
            }
        }
        e.sort_unstable();
        e.dedup();
    }
    e
}

pub fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

pub struct UnionFind(Vec<usize>);

impl UnionFind {
    pub fn new(n: usize) -> Self {
        UnionFind((0..n).collect())
    }
    pub fn find(&mut self, mut a: usize) -> usize {
        while self.0[a] != a {
            self.0[a] = self.0[self.0[a]];
            a = self.0[a];
        }
        a
    }
    /// Joins the sets of a and b; false if they were already joined.
    pub fn union(&mut self, a: usize, b: usize) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        self.0[ra.max(rb)] = ra.min(rb);
        true
    }
}

/// Minimum spanning tree / forest (Kruskal) of `edges` weighted by `weight`.
/// Ties are broken by the edge's indices, so the result is deterministic.
pub fn mst(n: usize, edges: &[(usize, usize)], weight: impl Fn(usize, usize) -> f64) -> Vec<(usize, usize)> {
    let mut e: Vec<(f64, usize, usize)> = edges.iter().filter(|(a, b)| *a < n && *b < n && a != b).map(|&(a, b)| (weight(a, b), a.min(b), a.max(b))).collect();
    e.sort_by(|x, y| x.partial_cmp(y).unwrap_or(Ordering::Equal));
    let mut uf = UnionFind::new(n);
    e.into_iter().filter(|&(_, a, b)| uf.union(a, b)).map(|(_, a, b)| (a, b)).collect()
}

// ------------------------------------------------------------------ config

#[derive(Clone, Debug)]
pub struct Template {
    pub name: String,
    /// Rows of characters: '#' wall, '.' floor, ' ' outside, 'D'/'+' door spot, other
    /// letters/digits = markers on floor cells.
    pub rows: Vec<Vec<char>>,
    pub weight: f64,
    pub tags: Vec<String>,
    /// Placed at least `min` times (if the room count allows) and at most `max`.
    pub min: usize,
    pub max: Option<usize>,
    pub rotate: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Where {
    Room,
    Corridor,
    DeadEnd,
    DeadEndCell,
    StartRoom,
    ExitRoom,
    MainPath,
    Marker,
    Any,
}

impl Where {
    fn parse(s: &str) -> Option<Where> {
        Some(match s {
            "room" | "sala" => Where::Room,
            "corridor" | "pasillo" => Where::Corridor,
            "dead_end" | "dead_end_room" => Where::DeadEnd,
            "dead_end_cell" => Where::DeadEndCell,
            "start_room" => Where::StartRoom,
            "exit_room" => Where::ExitRoom,
            "main_path" => Where::MainPath,
            "marker" => Where::Marker,
            "any" => Where::Any,
            _ => return None,
        })
    }
    fn room_based(&self) -> bool {
        matches!(self, Where::Room | Where::DeadEnd | Where::StartRoom | Where::ExitRoom)
    }
}

#[derive(Clone, Debug)]
pub struct Rule {
    /// Event types and their weights (one is chosen per event).
    pub types: Vec<(String, f64)>,
    pub count: (usize, usize),
    pub r#where: Where,
    pub marker: Option<char>,
    pub on_main_path: Option<bool>,
    pub min_distance: Option<i32>,
    pub max_distance: Option<i32>,
    /// Minimum Chebyshev distance (cells) to any other event.
    pub min_spacing: i32,
    pub per_room: usize,
    pub chance: f64,
    pub exclude_start: bool,
    pub room_tags: Vec<String>,
    /// "random" (default), "far" or "near": order in which rooms/cells are tried.
    pub prefer: String,
    pub data: J,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub width: usize,
    pub height: usize,
    pub rooms: (usize, usize),
    pub room_min: (usize, usize),
    pub room_max: (usize, usize),
    pub templates: Vec<Template>,
    pub rect_weight: f64,
    pub spacing: usize,
    pub spread: f64,
    pub extra_edges: f64,
    /// Loops only between rooms at most this many times the tree's average edge length
    /// apart (0 = no limit).
    pub extra_max_length: f64,
    pub turn_cost: f64,
    pub noise: f64,
    pub reuse_cost: f64,
    pub corridor_width: usize,
    pub doors: bool,
    /// "auto" (markers, else farthest pair), "farthest", "random".
    pub start: String,
    pub events: Vec<Rule>,
    pub attempts: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            width: 64,
            height: 64,
            rooms: (8, 12),
            room_min: (4, 4),
            room_max: (9, 8),
            templates: vec![],
            rect_weight: 1.0,
            spacing: 2,
            spread: 0.6,
            extra_edges: 0.15,
            extra_max_length: 2.0,
            turn_cost: 1.0,
            noise: 0.0,
            reuse_cost: 0.5,
            corridor_width: 1,
            doors: true,
            start: "auto".into(),
            events: vec![],
            attempts: 30,
        }
    }
}

/// How `{ room: "rm_x" }` templates are read (the engine provides it, see natives/dungeon.rs).
pub type RoomLoader<'a> = &'a dyn Fn(&Map<String, J>) -> Result<Vec<String>, String>;

pub struct ParseCtx<'a> {
    /// Folder for `file = "..."` templates (the project folder).
    pub base: Option<std::path::PathBuf>,
    pub room_loader: Option<RoomLoader<'a>>,
}

fn num(v: &J, what: &str) -> Result<f64, String> {
    v.as_f64().ok_or_else(|| format!("{what}: se esperaba un número, no {v}"))
}

fn range(v: &J, what: &str) -> Result<(usize, usize), String> {
    match v {
        J::Array(a) if a.len() == 2 => {
            let (x, y) = (num(&a[0], what)?.max(0.0) as usize, num(&a[1], what)?.max(0.0) as usize);
            Ok((x.min(y), x.max(y)))
        }
        J::Array(_) => Err(format!("{what}: se esperaba [mín, máx]")),
        v => {
            let n = num(v, what)?.max(0.0) as usize;
            Ok((n, n))
        }
    }
}

fn pair(v: &J, what: &str) -> Result<(usize, usize), String> {
    match v {
        J::Array(a) if a.len() == 2 => Ok((num(&a[0], what)?.max(1.0) as usize, num(&a[1], what)?.max(1.0) as usize)),
        J::Object(o) => Ok((num(o.get("w").unwrap_or(&J::Null), what)?.max(1.0) as usize, num(o.get("h").unwrap_or(&J::Null), what)?.max(1.0) as usize)),
        v => {
            let n = num(v, what)?.max(1.0) as usize;
            Ok((n, n))
        }
    }
}

fn strs(v: &J) -> Vec<String> {
    match v {
        J::Array(a) => a.iter().filter_map(|s| s.as_str().map(String::from)).collect(),
        J::String(s) => vec![s.clone()],
        _ => vec![],
    }
}

/// Rows from an array of strings or one multi-line string (blank first/last lines removed).
fn rows_of(v: &J, what: &str) -> Result<Vec<String>, String> {
    let mut rows: Vec<String> = match v {
        J::Array(a) => a.iter().map(|r| r.as_str().map(String::from).ok_or_else(|| format!("{what}: cada fila debe ser un texto"))).collect::<Result<_, _>>()?,
        J::String(s) => s.lines().map(String::from).collect(),
        _ => return Err(format!("{what}: rows debe ser una lista de textos o un texto de varias líneas")),
    };
    while rows.first().is_some_and(|r| r.trim().is_empty()) {
        rows.remove(0);
    }
    while rows.last().is_some_and(|r| r.trim().is_empty()) {
        rows.pop();
    }
    Ok(rows)
}

impl Config {
    /// Parses a config (struct / TOML / JSON as JSON). Unknown keys go to `warnings`.
    pub fn parse(j: &J, ctx: &ParseCtx, warnings: &mut Vec<String>) -> Result<Config, String> {
        let mut c = Config::default();
        let Some(o) = j.as_object() else { return Err("la configuración de la mazmorra debe ser un struct/tabla".into()) };
        let mut rect_weight_set = false;
        for (k, v) in o {
            match k.as_str() {
                "width" => c.width = num(v, "width")?.max(8.0) as usize,
                "height" => c.height = num(v, "height")?.max(8.0) as usize,
                "size" => {
                    let (w, h) = pair(v, "size")?;
                    c.width = w.max(8);
                    c.height = h.max(8);
                }
                "rooms" => c.rooms = range(v, "rooms")?,
                "room_size" | "room_min" | "room_max" => {
                    if k == "room_size" {
                        let o = v.as_object().ok_or("room_size: se esperaba { min: [w, h], max: [w, h] }")?;
                        if let Some(m) = o.get("min") {
                            c.room_min = pair(m, "room_size.min")?;
                        }
                        if let Some(m) = o.get("max") {
                            c.room_max = pair(m, "room_size.max")?;
                        }
                    } else if k == "room_min" {
                        c.room_min = pair(v, k)?;
                    } else {
                        c.room_max = pair(v, k)?;
                    }
                }
                "templates" => {
                    let list = v.as_array().ok_or("templates: se esperaba una lista de plantillas")?;
                    for (i, t) in list.iter().enumerate() {
                        c.templates.push(Self::template(t, i, ctx, warnings)?);
                    }
                }
                "rect_weight" => {
                    c.rect_weight = num(v, k)?.max(0.0);
                    rect_weight_set = true;
                }
                "spacing" => c.spacing = num(v, k)?.max(0.0) as usize,
                "spread" => c.spread = num(v, k)?.clamp(0.0, 1.0),
                "extra_edges" | "loops" => c.extra_edges = num(v, k)?.clamp(0.0, 1.0),
                "extra_max_length" => c.extra_max_length = num(v, k)?.max(0.0),
                "corridor" => match v {
                    J::String(s) => c.corridor_preset(s)?,
                    J::Object(o) => {
                        for (ck, cv) in o {
                            match ck.as_str() {
                                "mode" => c.corridor_preset(cv.as_str().unwrap_or(""))?,
                                "turn_cost" => c.turn_cost = num(cv, "corridor.turn_cost")?.max(0.0),
                                "noise" => c.noise = num(cv, "corridor.noise")?.max(0.0),
                                "reuse_cost" => c.reuse_cost = num(cv, "corridor.reuse_cost")?.clamp(0.05, 10.0),
                                "width" => c.corridor_width = num(cv, "corridor.width")?.clamp(1.0, 4.0) as usize,
                                other => warnings.push(format!("corridor: clave desconocida '{other}'")),
                            }
                        }
                    }
                    _ => return Err("corridor: se esperaba \"astar\", \"l\", \"wiggly\" o un struct".into()),
                },
                "corridor_width" => c.corridor_width = num(v, k)?.clamp(1.0, 4.0) as usize,
                "doors" => c.doors = v.as_bool().ok_or("doors: se esperaba true o false")?,
                "start" => {
                    let s = v.as_str().unwrap_or("");
                    if !matches!(s, "auto" | "farthest" | "random") {
                        return Err(format!("start: '{s}' no vale (auto, farthest, random)"));
                    }
                    c.start = s.into();
                }
                "events" => {
                    let list = v.as_array().ok_or("events: se esperaba una lista de reglas")?;
                    for (i, r) in list.iter().enumerate() {
                        c.events.push(Self::rule(r, i, warnings)?);
                    }
                }
                "attempts" => c.attempts = num(v, k)?.clamp(1.0, 500.0) as usize,
                "name" | "description" | "id" => {}
                other => warnings.push(format!("clave desconocida '{other}' en la configuración de la mazmorra")),
            }
        }
        if !c.templates.is_empty() && !rect_weight_set {
            c.rect_weight = 0.0;
        }
        if c.templates.is_empty() && c.rect_weight <= 0.0 {
            c.rect_weight = 1.0;
        }
        c.room_max = (c.room_max.0.max(c.room_min.0), c.room_max.1.max(c.room_min.1));
        if c.rooms.1 == 0 {
            return Err("rooms: hace falta al menos una sala".into());
        }
        Ok(c)
    }

    fn corridor_preset(&mut self, s: &str) -> Result<(), String> {
        match s {
            "astar" | "" => {}
            "l" | "straight" => self.turn_cost = 8.0,
            "wiggly" => {
                self.turn_cost = 0.2;
                self.noise = 3.0;
            }
            other => return Err(format!("corridor: modo '{other}' desconocido (astar, l, wiggly)")),
        }
        Ok(())
    }

    fn template(t: &J, i: usize, ctx: &ParseCtx, warnings: &mut Vec<String>) -> Result<Template, String> {
        let what = format!("templates[{i}]");
        let (o, rows) = match t {
            J::Array(_) | J::String(_) => (Map::new(), rows_of(t, &what)?),
            J::Object(o) => {
                let rows = if let Some(r) = o.get("rows") {
                    rows_of(r, &what)?
                } else if let Some(f) = o.get("file").and_then(|f| f.as_str()) {
                    let p = match &ctx.base {
                        Some(b) => b.join(f),
                        None => f.into(),
                    };
                    let text = std::fs::read_to_string(&p).map_err(|e| format!("{what}: no se pudo leer '{}': {e}", p.display()))?;
                    rows_of(&J::String(text), &what)?
                } else if o.contains_key("room") {
                    match ctx.room_loader {
                        Some(load) => load(o).map_err(|m| format!("{what}: {m}"))?,
                        None => return Err(format!("{what}: las plantillas desde una room necesitan un proyecto")),
                    }
                } else {
                    return Err(format!("{what}: falta rows, file o room"));
                };
                (o.clone(), rows)
            }
            _ => return Err(format!("{what}: se esperaba un struct con rows")),
        };
        let mut tp = Template { name: format!("plantilla{i}"), rows: rows.iter().map(|r| r.chars().collect()).collect(), weight: 1.0, tags: vec![], min: 0, max: None, rotate: false };
        for (k, v) in &o {
            match k.as_str() {
                "rows" | "file" | "room" | "layer" | "markers" | "walls" => {}
                "name" => tp.name = v.as_str().unwrap_or("").to_string(),
                "weight" => tp.weight = num(v, &format!("{what}.weight"))?.max(0.0),
                "tags" => tp.tags = strs(v),
                "min" => tp.min = num(v, &format!("{what}.min"))?.max(0.0) as usize,
                "max" => tp.max = Some(num(v, &format!("{what}.max"))?.max(0.0) as usize),
                "rotate" => tp.rotate = v.as_bool().unwrap_or(false),
                other => warnings.push(format!("{what}: clave desconocida '{other}'")),
            }
        }
        if !tp.rows.iter().flatten().any(|&ch| is_floor_char(ch)) {
            return Err(format!("{what} ('{}'): la plantilla no tiene suelo ('.' o marcadores)", tp.name));
        }
        Ok(tp)
    }

    fn rule(r: &J, i: usize, warnings: &mut Vec<String>) -> Result<Rule, String> {
        let what = format!("events[{i}]");
        let o = r.as_object().ok_or(format!("{what}: se esperaba un struct {{ type, count, where }}"))?;
        let mut rule = Rule {
            types: vec![],
            count: (1, 1),
            r#where: Where::Room,
            marker: None,
            on_main_path: None,
            min_distance: None,
            max_distance: None,
            min_spacing: 1,
            per_room: 0,
            chance: 1.0,
            exclude_start: true,
            room_tags: vec![],
            prefer: "random".into(),
            data: J::Null,
        };
        let mut per_room_set = false;
        for (k, v) in o {
            match k.as_str() {
                "type" | "types" => {
                    rule.types = match v {
                        J::String(s) => vec![(s.clone(), 1.0)],
                        J::Array(a) => a
                            .iter()
                            .map(|t| match t {
                                J::String(s) => Ok((s.clone(), 1.0)),
                                J::Object(t) => Ok((
                                    t.get("type").or(t.get("name")).and_then(|s| s.as_str()).ok_or(format!("{what}.type: cada opción necesita type"))?.to_string(),
                                    t.get("weight").and_then(|w| w.as_f64()).unwrap_or(1.0),
                                )),
                                _ => Err(format!("{what}.type: opción no válida {t}")),
                            })
                            .collect::<Result<_, _>>()?,
                        _ => return Err(format!("{what}.type: se esperaba un texto o una lista")),
                    }
                }
                "count" => rule.count = range(v, &format!("{what}.count"))?,
                "where" => {
                    let s = v.as_str().unwrap_or("");
                    rule.r#where = Where::parse(s).ok_or(format!("{what}.where: '{s}' no vale (room, corridor, dead_end, dead_end_cell, start_room, exit_room, main_path, marker, any)"))?;
                }
                "marker" => rule.marker = v.as_str().and_then(|s| s.chars().next()),
                "on_main_path" => rule.on_main_path = v.as_bool(),
                "not_on_main_path" => rule.on_main_path = v.as_bool().map(|b| !b).filter(|b| !b).or(rule.on_main_path),
                "min_distance_from_start" | "min_distance" => rule.min_distance = Some(num(v, k)? as i32),
                "max_distance_from_start" | "max_distance" => rule.max_distance = Some(num(v, k)? as i32),
                "min_spacing" | "spacing" => rule.min_spacing = num(v, k)?.max(0.0) as i32,
                "per_room" => {
                    rule.per_room = num(v, k)?.max(0.0) as usize;
                    per_room_set = true;
                }
                "chance" => rule.chance = num(v, k)?.clamp(0.0, 1.0),
                "exclude_start_room" | "exclude_start" => rule.exclude_start = v.as_bool().unwrap_or(true),
                "room_tags" | "tags" => rule.room_tags = strs(v),
                "prefer" => {
                    let s = v.as_str().unwrap_or("random");
                    if !matches!(s, "random" | "far" | "near") {
                        return Err(format!("{what}.prefer: '{s}' no vale (random, far, near)"));
                    }
                    rule.prefer = s.into();
                }
                "data" => rule.data = v.clone(),
                "weight" => {
                    if let Some((_, w)) = rule.types.first_mut() {
                        *w = num(v, k)?;
                    }
                }
                other => warnings.push(format!("{what}: clave desconocida '{other}'")),
            }
        }
        if rule.types.is_empty() {
            return Err(format!("{what}: falta type"));
        }
        if !per_room_set {
            rule.per_room = if rule.r#where.room_based() { 1 } else { 0 };
        }
        if rule.r#where == Where::StartRoom {
            rule.exclude_start = false;
        }
        if rule.r#where == Where::Marker && rule.marker.is_none() {
            return Err(format!("{what}: where = \"marker\" necesita marker (una letra de las plantillas)"));
        }
        Ok(rule)
    }
}

fn is_floor_char(ch: char) -> bool {
    ch == '.' || (ch.is_alphanumeric() && ch != 'D')
}

// ------------------------------------------------------------------ shapes

#[derive(Clone, Copy, PartialEq, Debug)]
enum Sc {
    Void,
    Floor,
    Wall,
    DoorSpot,
}

#[derive(Clone, Debug)]
struct Shape {
    w: usize,
    h: usize,
    cells: Vec<Sc>,
    markers: Vec<(usize, usize, char)>,
    template: Option<usize>,
}

impl Shape {
    fn at(&self, x: i64, y: i64) -> Sc {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            Sc::Void
        } else {
            self.cells[y as usize * self.w + x as usize]
        }
    }

    /// From template rows: floors, walls ('#' plus a ring around floors), door spots.
    fn from_rows(rows: &[Vec<char>], template: Option<usize>) -> Shape {
        let th = rows.len();
        let tw = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let (w, h) = (tw + 2, th + 2);
        let mut cells = vec![Sc::Void; w * h];
        let mut markers = vec![];
        for (y, r) in rows.iter().enumerate() {
            for (x, &ch) in r.iter().enumerate() {
                let i = (y + 1) * w + x + 1;
                cells[i] = match ch {
                    '#' => Sc::Wall,
                    'D' | '+' => Sc::DoorSpot,
                    c if is_floor_char(c) => {
                        if c != '.' {
                            markers.push((x + 1, y + 1, c));
                        }
                        Sc::Floor
                    }
                    _ => Sc::Void,
                };
            }
        }
        // walls around the floor
        for y in 0..h {
            for x in 0..w {
                if cells[y * w + x] != Sc::Void {
                    continue;
                }
                let near = (-1..=1).any(|dy: i64| {
                    (-1..=1).any(|dx: i64| {
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h && cells[ny as usize * w + nx as usize] == Sc::Floor
                    })
                });
                if near {
                    cells[y * w + x] = Sc::Wall;
                }
            }
        }
        let mut s = Shape { w, h, cells, markers, template };
        s.crop();
        s
    }

    fn rect(fw: usize, fh: usize) -> Shape {
        let rows: Vec<Vec<char>> = (0..fh).map(|_| vec!['.'; fw]).collect();
        Shape::from_rows(&rows, None)
    }

    fn crop(&mut self) {
        let used: Vec<(usize, usize)> = (0..self.h).flat_map(|y| (0..self.w).map(move |x| (x, y))).filter(|&(x, y)| self.cells[y * self.w + x] != Sc::Void).collect();
        let x0 = used.iter().map(|p| p.0).min().unwrap_or(0);
        let y0 = used.iter().map(|p| p.1).min().unwrap_or(0);
        let x1 = used.iter().map(|p| p.0).max().unwrap_or(0);
        let y1 = used.iter().map(|p| p.1).max().unwrap_or(0);
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut cells = vec![Sc::Void; w * h];
        for y in 0..h {
            for x in 0..w {
                cells[y * w + x] = self.cells[(y + y0) * self.w + x + x0];
            }
        }
        self.markers.iter_mut().for_each(|m| {
            m.0 -= x0;
            m.1 -= y0
        });
        self.w = w;
        self.h = h;
        self.cells = cells;
    }

    /// Rotated 90° clockwise `turns` times, mirrored first if `flip`.
    fn transformed(&self, turns: usize, flip: bool) -> Shape {
        let mut s = self.clone();
        if flip {
            let mut cells = s.cells.clone();
            for y in 0..s.h {
                for x in 0..s.w {
                    cells[y * s.w + x] = s.cells[y * s.w + (s.w - 1 - x)];
                }
            }
            s.cells = cells;
            s.markers.iter_mut().for_each(|m| m.0 = s.w - 1 - m.0);
        }
        for _ in 0..turns % 4 {
            let (w, h) = (s.h, s.w);
            let mut cells = vec![Sc::Void; w * h];
            for y in 0..h {
                for x in 0..w {
                    // new (x, y) <- old (y, oldh - 1 - x)
                    cells[y * w + x] = s.cells[(s.h - 1 - x) * s.w + y];
                }
            }
            s.markers.iter_mut().for_each(|m| {
                let (ox, oy) = (m.0, m.1);
                m.0 = s.h - 1 - oy;
                m.1 = ox;
            });
            s.w = w;
            s.h = h;
            s.cells = cells;
        }
        s
    }

    /// Wall cells that can become doors: explicit 'D' spots, else walls with floor on
    /// one side and the outside on the opposite side.
    fn door_spots(&self) -> Vec<(usize, usize)> {
        let explicit: Vec<(usize, usize)> = (0..self.h).flat_map(|y| (0..self.w).map(move |x| (x, y))).filter(|&(x, y)| self.cells[y * self.w + x] == Sc::DoorSpot).collect();
        if !explicit.is_empty() {
            return explicit;
        }
        let mut v = vec![];
        for y in 0..self.h {
            for x in 0..self.w {
                if self.cells[y * self.w + x] != Sc::Wall {
                    continue;
                }
                let (xi, yi) = (x as i64, y as i64);
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    if self.at(xi + dx, yi + dy) == Sc::Floor && self.at(xi - dx, yi - dy) == Sc::Void {
                        v.push((x, y));
                        break;
                    }
                }
            }
        }
        v
    }
}

// ------------------------------------------------------------------ results

#[derive(Clone, Debug)]
pub struct Room {
    /// Footprint (walls included) in cells.
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub template: String,
    pub tags: Vec<String>,
    pub floor: Vec<(usize, usize)>,
    pub markers: Vec<(usize, usize, char)>,
    pub door_spots: Vec<(usize, usize)>,
    pub doors: Vec<(usize, usize)>,
    /// Floor cell nearest to the middle of the room.
    pub center: (usize, usize),
    pub neighbors: Vec<usize>,
    /// Hops in the room graph from the start room.
    pub depth: i32,
    /// Order along the main path (-1 = off the path).
    pub path_index: i32,
    /// Hops in the room graph to the nearest room on the main path.
    pub dist_from_main: i32,
    /// Walking distance (cells) from the start to `center`.
    pub distance: i32,
    pub on_main_path: bool,
    pub is_start: bool,
    pub is_exit: bool,
    pub dead_end: bool,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub kind: String,
    pub x: usize,
    pub y: usize,
    pub room: Option<usize>,
    pub distance: i32,
    pub on_main_path: bool,
    pub rule: usize,
    pub data: J,
}

#[derive(Clone, Debug, Default)]
pub struct Dungeon {
    pub seed: u64,
    pub w: usize,
    pub h: usize,
    /// FLOOR, WALL, DOOR, CORRIDOR per cell (row-major).
    pub cells: Vec<u8>,
    /// Room of each floor cell (-1: corridor, wall, door).
    pub room_of: Vec<i32>,
    pub rooms: Vec<Room>,
    pub delaunay: Vec<(usize, usize)>,
    pub mst: Vec<(usize, usize)>,
    pub extra: Vec<(usize, usize)>,
    /// Unused Delaunay edges short enough to become loops (`extra` takes a share of them).
    pub loop_candidates: usize,
    pub corridors: Vec<Vec<(usize, usize)>>,
    pub start: (usize, usize),
    pub exit: (usize, usize),
    pub start_room: usize,
    pub exit_room: usize,
    /// Walking distance from the start (-1 = unreachable / wall).
    pub dist: Vec<i32>,
    pub main_path: Vec<(usize, usize)>,
    pub on_path: Vec<bool>,
    pub dead_ends: Vec<(usize, usize)>,
    pub events: Vec<Event>,
    pub attempts: usize,
    pub warnings: Vec<String>,
}

impl Dungeon {
    pub fn cell(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            WALL
        } else {
            self.cells[y as usize * self.w + x as usize]
        }
    }

    pub fn distance(&self, x: i64, y: i64) -> i32 {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            -1
        } else {
            self.dist[y as usize * self.w + x as usize]
        }
    }

    /// Room graph edges actually connected (MST + extra).
    pub fn edges(&self) -> Vec<(usize, usize)> {
        let mut e = self.mst.clone();
        e.extend(self.extra.iter().copied());
        e
    }

    /// FNV-1a of the cells (determinism checks).
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &c in &self.cells {
            h ^= c as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        for e in &self.events {
            for b in e.kind.bytes().chain((e.x as u32).to_le_bytes()).chain((e.y as u32).to_le_bytes()) {
                h ^= b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }

    /// The map as text: '#' wall, '.' floor, ',' corridor, '+' door, 'S'/'X' start/exit,
    /// first letter of each event type in lowercase... (uppercase for the boss-like ones
    /// is not attempted: one character per cell).
    pub fn ascii(&self) -> String {
        let mut g: Vec<Vec<char>> = (0..self.h)
            .map(|y| {
                (0..self.w)
                    .map(|x| match self.cells[y * self.w + x] {
                        FLOOR => '.',
                        DOOR => '+',
                        CORRIDOR => ',',
                        _ => '#',
                    })
                    .collect()
            })
            .collect();
        for e in &self.events {
            g[e.y][e.x] = e.kind.chars().next().unwrap_or('?').to_ascii_lowercase();
        }
        g[self.start.1][self.start.0] = 'S';
        g[self.exit.1][self.exit.0] = 'X';
        g.into_iter().map(|r| r.into_iter().collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    pub fn room_json(&self, i: usize) -> J {
        let r = &self.rooms[i];
        let pts = |v: &[(usize, usize)]| J::Array(v.iter().map(|&(x, y)| json!([x, y])).collect());
        json!({
            "index": i,
            "x": r.x, "y": r.y, "w": r.w, "h": r.h,
            "cx": r.center.0, "cy": r.center.1,
            "template": r.template,
            "tags": r.tags,
            "neighbors": r.neighbors,
            "depth": r.depth,
            "path_index": r.path_index,
            "dist_from_main": r.dist_from_main,
            "distance": r.distance,
            "on_main_path": r.on_main_path,
            "is_start": r.is_start,
            "is_exit": r.is_exit,
            "dead_end": r.dead_end,
            "doors": pts(&r.doors),
            "markers": r.markers.iter().map(|&(x, y, c)| json!({"x": x, "y": y, "marker": c.to_string()})).collect::<Vec<_>>(),
            "floor_cells": r.floor.len(),
        })
    }

    pub fn event_json(&self, e: &Event) -> J {
        let max = self.dist.iter().copied().max().unwrap_or(1).max(1);
        json!({
            "type": e.kind, "x": e.x, "y": e.y,
            "room": e.room.map(|r| r as i64).unwrap_or(-1),
            "distance": e.distance,
            "t": e.distance.max(0) as f64 / max as f64,
            "on_main_path": e.on_main_path,
            "rule": e.rule,
            "data": e.data,
        })
    }

    pub fn loops(&self) -> usize {
        self.extra.len()
    }

    pub fn to_json(&self) -> J {
        json!({
            "seed": self.seed,
            "width": self.w,
            "height": self.h,
            "hash": format!("{:016x}", self.hash()),
            "attempts": self.attempts,
            "start": [self.start.0, self.start.1],
            "exit": [self.exit.0, self.exit.1],
            "start_room": self.start_room,
            "exit_room": self.exit_room,
            "rooms": (0..self.rooms.len()).map(|i| self.room_json(i)).collect::<Vec<_>>(),
            "delaunay": self.delaunay.iter().map(|e| json!([e.0, e.1])).collect::<Vec<_>>(),
            "mst": self.mst.iter().map(|e| json!([e.0, e.1])).collect::<Vec<_>>(),
            "extra": self.extra.iter().map(|e| json!([e.0, e.1])).collect::<Vec<_>>(),
            "loop_candidates": self.loop_candidates,
            "main_path_length": self.main_path.len(),
            "dead_ends": self.dead_ends.iter().map(|e| json!([e.0, e.1])).collect::<Vec<_>>(),
            "events": self.events.iter().map(|e| self.event_json(e)).collect::<Vec<_>>(),
            "warnings": self.warnings,
        })
    }
}

// ------------------------------------------------------------------ engine state

/// A `dungeon_create` generator: its config and the last generated dungeon.
pub struct DungeonSlot {
    pub config: Config,
    pub result: Option<Dungeon>,
    /// Warnings from reading the config.
    pub config_warnings: Vec<String>,
    /// The ds_grid `dungeon_grid` returns (refreshed by each `dungeon_generate`).
    pub grid: Option<usize>,
}

#[derive(Default)]
pub struct Dungeons {
    pub list: Vec<Option<DungeonSlot>>,
}

impl Dungeons {
    pub fn add(&mut self, d: DungeonSlot) -> usize {
        match self.list.iter().position(|s| s.is_none()) {
            Some(i) => {
                self.list[i] = Some(d);
                i
            }
            None => {
                self.list.push(Some(d));
                self.list.len() - 1
            }
        }
    }

    /// The last dungeon generated by generator `id` (public accessor for other natives).
    pub fn get(&self, id: f64) -> Option<&Dungeon> {
        if id < 0.0 || id.fract() != 0.0 {
            return None;
        }
        self.list.get(id as usize).and_then(|s| s.as_ref()).and_then(|s| s.result.as_ref())
    }
}

// ------------------------------------------------------------------ generation

struct Placed {
    shape: Shape,
    x: i64,
    y: i64,
}

fn overlaps(a: &Placed, b: &Placed, pad: i64) -> (i64, i64) {
    let ox = (a.x + a.shape.w as i64 + pad).min(b.x + b.shape.w as i64 + pad) - a.x.max(b.x);
    let oy = (a.y + a.shape.h as i64 + pad).min(b.y + b.shape.h as i64 + pad) - a.y.max(b.y);
    (ox, oy)
}

fn collide(a: &Placed, b: &Placed, pad: i64) -> bool {
    let (ox, oy) = overlaps(a, b, pad);
    ox > 0 && oy > 0
}

/// Generates a dungeon. Errors only for impossible configs; partial results (fewer
/// rooms or events than asked) come with `warnings`.
pub fn generate(cfg: &Config, seed: u64) -> Result<Dungeon, String> {
    let mut best: Option<Dungeon> = None;
    for attempt in 0..cfg.attempts.max(1) {
        let mut rng = Rng::new(seed ^ (attempt as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let d = generate_once(cfg, seed, &mut rng)?;
        let ok = d.rooms.len() >= cfg.rooms.0.max(1).min(cfg.rooms.1);
        let better = best.as_ref().is_none_or(|b| d.rooms.len() > b.rooms.len());
        if ok {
            let mut d = d;
            d.attempts = attempt + 1;
            return Ok(d);
        }
        if better {
            let mut d = d;
            d.attempts = attempt + 1;
            best = Some(d);
        }
    }
    let mut d = best.expect("at least one attempt");
    d.warnings.push(format!("solo caben {} salas de las {} pedidas como mínimo en {}×{} (prueba un mapa más grande, menos salas o salas más pequeñas)", d.rooms.len(), cfg.rooms.0, cfg.width, cfg.height));
    if d.rooms.is_empty() {
        return Err(format!("no cabe ninguna sala en {}×{}", cfg.width, cfg.height));
    }
    Ok(d)
}

fn pick_shapes(cfg: &Config, rng: &mut Rng, n: usize) -> Vec<Shape> {
    let mut used = vec![0usize; cfg.templates.len()];
    let mut out = vec![];
    let make = |t: usize, rng: &mut Rng| -> Shape {
        let tp = &cfg.templates[t];
        let s = Shape::from_rows(&tp.rows, Some(t));
        if tp.rotate {
            s.transformed(rng.index(4), rng.f() < 0.5)
        } else {
            s
        }
    };
    // required templates first
    for (t, tp) in cfg.templates.iter().enumerate() {
        for _ in 0..tp.min {
            if out.len() < n {
                out.push(make(t, rng));
                used[t] += 1;
            }
        }
    }
    while out.len() < n {
        let mut w: Vec<f64> = cfg.templates.iter().enumerate().map(|(t, tp)| if tp.max.is_some_and(|m| used[t] >= m) { 0.0 } else { tp.weight }).collect();
        w.push(cfg.rect_weight);
        let Some(k) = rng.weighted(&w) else { break };
        if k == cfg.templates.len() {
            let fw = rng.range(cfg.room_min.0 as i64, cfg.room_max.0 as i64) as usize;
            let fh = rng.range(cfg.room_min.1 as i64, cfg.room_max.1 as i64) as usize;
            out.push(Shape::rect(fw.max(1), fh.max(1)));
        } else {
            out.push(make(k, rng));
            used[k] += 1;
        }
    }
    out
}

fn generate_once(cfg: &Config, seed: u64, rng: &mut Rng) -> Result<Dungeon, String> {
    let (w, h) = (cfg.width, cfg.height);
    let n = rng.range(cfg.rooms.0 as i64, cfg.rooms.1 as i64) as usize;
    let shapes = pick_shapes(cfg, rng, n);
    let pad = cfg.spacing as i64;
    let border = 1i64;
    // ---- scatter around the centre (springs push them apart next)
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let mut placed: Vec<Placed> = shapes
        .into_iter()
        .map(|s| {
            let a = rng.f() * std::f64::consts::TAU;
            let r = rng.f().sqrt() * cfg.spread;
            let (px, py) = (cx + a.cos() * r * w as f64 / 2.0, cy + a.sin() * r * h as f64 / 2.0);
            let (x, y) = ((px - s.w as f64 / 2.0).round() as i64, (py - s.h as f64 / 2.0).round() as i64);
            Placed { shape: s, x, y }
        })
        .collect();
    let clamp = |p: &mut Placed| {
        p.x = p.x.clamp(border, (w as i64 - border - p.shape.w as i64).max(border));
        p.y = p.y.clamp(border, (h as i64 - border - p.shape.h as i64).max(border));
    };
    // ---- separation: push every colliding pair apart along the axis of least overlap
    for _ in 0..400 {
        let mut moved = false;
        for i in 0..placed.len() {
            for j in i + 1..placed.len() {
                let (ox, oy) = overlaps(&placed[i], &placed[j], pad);
                if ox <= 0 || oy <= 0 {
                    continue;
                }
                moved = true;
                let (ai, aj) = ((placed[i].x * 2 + placed[i].shape.w as i64, placed[i].y * 2 + placed[i].shape.h as i64), (placed[j].x * 2 + placed[j].shape.w as i64, placed[j].y * 2 + placed[j].shape.h as i64));
                if ox < oy || (ox == oy && rng.f() < 0.5) {
                    let s = if ai.0 < aj.0 || (ai.0 == aj.0 && rng.f() < 0.5) { -1 } else { 1 };
                    let (a, b) = ((ox + 1) / 2, ox - (ox + 1) / 2);
                    placed[i].x += s * a;
                    placed[j].x -= s * b.max(if a == 0 { 1 } else { 0 });
                } else {
                    let s = if ai.1 < aj.1 || (ai.1 == aj.1 && rng.f() < 0.5) { -1 } else { 1 };
                    let (a, b) = ((oy + 1) / 2, oy - (oy + 1) / 2);
                    placed[i].y += s * a;
                    placed[j].y -= s * b.max(if a == 0 { 1 } else { 0 });
                }
                clamp(&mut placed[i]);
                clamp(&mut placed[j]);
            }
        }
        if !moved {
            break;
        }
    }
    // ---- keep the rooms that fit, central ones first; retry the rest in free spots
    for p in placed.iter_mut() {
        clamp(p);
    }
    let fits = |p: &Placed| p.x >= border && p.y >= border && p.x + p.shape.w as i64 <= w as i64 - border && p.y + p.shape.h as i64 <= h as i64 - border;
    let mut order: Vec<usize> = (0..placed.len()).collect();
    let centre_d = |p: &Placed| ((p.x as f64 + p.shape.w as f64 / 2.0 - cx).powi(2) + (p.y as f64 + p.shape.h as f64 / 2.0 - cy).powi(2)) as i64;
    order.sort_by_key(|&i| (centre_d(&placed[i]), i));
    let mut kept: Vec<Placed> = vec![];
    let mut dropped: Vec<Placed> = vec![];
    let mut taken: Vec<Option<Placed>> = placed.into_iter().map(Some).collect();
    for i in order {
        let p = taken[i].take().expect("once");
        if fits(&p) && !kept.iter().any(|k| collide(k, &p, pad)) {
            kept.push(p);
        } else {
            dropped.push(p);
        }
    }
    for mut p in dropped {
        for _ in 0..60 {
            let mx = (w as i64 - border - p.shape.w as i64).max(border);
            let my = (h as i64 - border - p.shape.h as i64).max(border);
            p.x = rng.range(border, mx);
            p.y = rng.range(border, my);
            if fits(&p) && !kept.iter().any(|k| collide(k, &p, pad)) {
                kept.push(p);
                break;
            }
        }
    }
    // stable order: top to bottom, left to right
    kept.sort_by_key(|p| (p.y, p.x));
    build(cfg, seed, rng, kept)
}

fn build(cfg: &Config, seed: u64, rng: &mut Rng, placed: Vec<Placed>) -> Result<Dungeon, String> {
    let (w, h) = (cfg.width, cfg.height);
    let mut d = Dungeon { seed, w, h, cells: vec![WALL; w * h], room_of: vec![-1; w * h], ..Default::default() };
    // footprint owner of every cell (walls included)
    let mut foot = vec![-1i32; w * h];
    for (ri, p) in placed.iter().enumerate() {
        let s = &p.shape;
        let mut room = Room {
            x: p.x as usize,
            y: p.y as usize,
            w: s.w,
            h: s.h,
            template: s.template.map(|t| cfg.templates[t].name.clone()).unwrap_or_else(|| "rect".into()),
            tags: s.template.map(|t| cfg.templates[t].tags.clone()).unwrap_or_default(),
            floor: vec![],
            markers: s.markers.iter().map(|&(x, y, c)| (x + p.x as usize, y + p.y as usize, c)).collect(),
            door_spots: s.door_spots().into_iter().map(|(x, y)| (x + p.x as usize, y + p.y as usize)).collect(),
            doors: vec![],
            center: (0, 0),
            neighbors: vec![],
            depth: -1,
            path_index: -1,
            dist_from_main: -1,
            distance: -1,
            on_main_path: false,
            is_start: false,
            is_exit: false,
            dead_end: false,
        };
        for sy in 0..s.h {
            for sx in 0..s.w {
                let c = s.cells[sy * s.w + sx];
                if c == Sc::Void {
                    continue;
                }
                let (x, y) = (p.x as usize + sx, p.y as usize + sy);
                foot[y * w + x] = ri as i32;
                if c == Sc::Floor {
                    d.cells[y * w + x] = FLOOR;
                    d.room_of[y * w + x] = ri as i32;
                    room.floor.push((x, y));
                }
            }
        }
        let (mx, my) = (p.x as f64 + s.w as f64 / 2.0 - 0.5, p.y as f64 + s.h as f64 / 2.0 - 0.5);
        room.center = *room.floor.iter().min_by(|a, b| {
            let da = (a.0 as f64 - mx).powi(2) + (a.1 as f64 - my).powi(2);
            let db = (b.0 as f64 - mx).powi(2) + (b.1 as f64 - my).powi(2);
            da.partial_cmp(&db).unwrap_or(Ordering::Equal)
        }).expect("templates have floor");
        d.rooms.push(room);
    }
    let n = d.rooms.len();
    // ---- graph: Delaunay -> MST -> extra edges
    let centres: Vec<(f64, f64)> = d.rooms.iter().map(|r| (r.center.0 as f64, r.center.1 as f64)).collect();
    d.delaunay = delaunay_edges(&centres);
    d.mst = mst(n, &d.delaunay, |a, b| dist(centres[a], centres[b]));
    // loop candidates: unused Delaunay edges not much longer than the tree's average edge
    // (long hull edges would make corridors across the whole map)
    let avg = d.mst.iter().map(|&(a, b)| dist(centres[a], centres[b])).sum::<f64>() / d.mst.len().max(1) as f64;
    let max_len = if cfg.extra_max_length > 0.0 { avg * cfg.extra_max_length } else { f64::INFINITY };
    let mut unused: Vec<(usize, usize)> = d.delaunay.iter().copied().filter(|e| !d.mst.contains(e) && dist(centres[e.0], centres[e.1]) <= max_len).collect();
    d.loop_candidates = unused.len();
    rng.shuffle(&mut unused);
    let k = (cfg.extra_edges * unused.len() as f64).round() as usize;
    d.extra = unused.into_iter().take(k).collect();
    d.extra.sort_unstable();
    for &(a, b) in d.mst.iter().chain(d.extra.iter()) {
        d.rooms[a].neighbors.push(b);
        d.rooms[b].neighbors.push(a);
    }
    for r in d.rooms.iter_mut() {
        r.neighbors.sort_unstable();
        r.neighbors.dedup();
    }
    // ---- corridors
    let noise: Vec<f64> = (0..w * h).map(|_| if cfg.noise > 0.0 { rng.f() * cfg.noise } else { 0.0 }).collect();
    let edges = d.edges();
    for (a, b) in edges {
        let path = corridor(&d, &foot, &noise, cfg, a, b, false).or_else(|| {
            d.warnings.push(format!("el pasillo {a}–{b} tuvo que atravesar otras salas"));
            corridor(&d, &foot, &noise, cfg, a, b, true)
        });
        let Some(path) = path else {
            d.warnings.push(format!("no se pudo unir la sala {a} con la {b}"));
            continue;
        };
        carve(&mut d, &foot, cfg, &path, a, b);
        d.corridors.push(path);
    }
    analyse(&mut d, cfg, rng);
    place_events(&mut d, cfg, rng);
    Ok(d)
}

#[derive(PartialEq)]
struct Node(f64, usize, u8);
impl Eq for Node {}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal).then_with(|| o.1.cmp(&self.1)).then_with(|| o.2.cmp(&self.2))
    }
}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

const DIRS: [(i64, i64); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// A* between a door spot of room a and one of room b. `force` lets the path cross
/// other rooms (expensive) when nothing else works.
fn corridor(d: &Dungeon, foot: &[i32], noise: &[f64], cfg: &Config, a: usize, b: usize, force: bool) -> Option<Vec<(usize, usize)>> {
    let (w, h) = (d.w, d.h);
    let goal_c = d.rooms[b].center;
    let is_goal = |i: usize| d.rooms[b].door_spots.iter().any(|&(x, y)| y * w + x == i);
    let min_cost = cfg.reuse_cost.min(1.0);
    let heur = |i: usize| ((i % w).abs_diff(goal_c.0) + (i / w).abs_diff(goal_c.1)) as f64 * min_cost * 0.5;
    let states = w * h * 5;
    let mut g = vec![f64::INFINITY; states];
    let mut prev = vec![usize::MAX; states];
    let mut heap = BinaryHeap::new();
    for &(x, y) in &d.rooms[a].door_spots {
        let i = y * w + x;
        // reuse doors the room already has
        let c = if d.cells[i] == DOOR || d.cells[i] == FLOOR { 0.0 } else { 2.0 };
        let s = i * 5 + 4;
        if c < g[s] {
            g[s] = c;
            heap.push(Node(c + heur(i), s, 0));
        }
    }
    while let Some(Node(_, s, _)) = heap.pop() {
        let (i, dir) = (s / 5, s % 5);
        let gc = g[s];
        if is_goal(i) && dir != 4 {
            let mut path = vec![];
            let mut cur = s;
            while cur != usize::MAX {
                path.push(((cur / 5) % w, (cur / 5) / w));
                cur = prev[cur];
            }
            path.reverse();
            return Some(path);
        }
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        for (k, (dx, dy)) in DIRS.iter().enumerate() {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 1 || ny < 1 || nx >= w as i64 - 1 || ny >= h as i64 - 1 {
                continue;
            }
            let ni = ny as usize * w + nx as usize;
            let owner = foot[ni];
            let mut cost = if owner < 0 {
                if d.cells[ni] == CORRIDOR {
                    cfg.reuse_cost
                } else {
                    1.0 + noise[ni]
                }
            } else if owner as usize == b && is_goal(ni) {
                // enter b only straight through a door spot
                if d.cells[ni] == DOOR || d.cells[ni] == FLOOR {
                    0.0
                } else {
                    2.0
                }
            } else if force && owner as usize != a && owner as usize != b {
                40.0
            } else {
                continue;
            };
            if dir != 4 && dir != k {
                cost += cfg.turn_cost;
            }
            let ns = ni * 5 + k;
            let nc = gc + cost;
            if nc < g[ns] {
                g[ns] = nc;
                prev[ns] = s;
                heap.push(Node(nc + heur(ni), ns, k as u8));
            }
        }
    }
    None
}

fn carve(d: &mut Dungeon, foot: &[i32], cfg: &Config, path: &[(usize, usize)], a: usize, b: usize) {
    let w = d.w;
    let door = if cfg.doors { DOOR } else { FLOOR };
    let last = path.len() - 1;
    for (k, &(x, y)) in path.iter().enumerate() {
        let i = y * w + x;
        if k == 0 || k == last {
            let r = if k == 0 { a } else { b };
            if d.cells[i] == WALL {
                d.cells[i] = door;
            }
            if !d.rooms[r].doors.contains(&(x, y)) {
                d.rooms[r].doors.push((x, y));
            }
            continue;
        }
        if foot[i] >= 0 {
            // forced through another room: open its walls
            if d.cells[i] == WALL {
                d.cells[i] = door;
            }
            continue;
        }
        if d.cells[i] == WALL {
            d.cells[i] = CORRIDOR;
        }
        for ex in 0..cfg.corridor_width {
            for ey in 0..cfg.corridor_width {
                let (cx, cy) = (x + ex, y + ey);
                if cx >= 1 && cy >= 1 && cx < d.w - 1 && cy < d.h - 1 && foot[cy * w + cx] < 0 && d.cells[cy * w + cx] == WALL {
                    d.cells[cy * w + cx] = CORRIDOR;
                }
            }
        }
    }
}

/// BFS distances over walkable cells from `sources` (-1 = unreachable).
pub fn bfs(cells: &[u8], w: usize, h: usize, sources: &[(usize, usize)]) -> Vec<i32> {
    let mut dist = vec![-1i32; w * h];
    let mut q = std::collections::VecDeque::new();
    for &(x, y) in sources {
        let i = y * w + x;
        if walkable(cells[i]) && dist[i] < 0 {
            dist[i] = 0;
            q.push_back(i);
        }
    }
    while let Some(i) = q.pop_front() {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        for (dx, dy) in DIRS {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                continue;
            }
            let ni = ny as usize * w + nx as usize;
            if dist[ni] < 0 && walkable(cells[ni]) {
                dist[ni] = dist[i] + 1;
                q.push_back(ni);
            }
        }
    }
    dist
}

fn graph_bfs(rooms: &[Room], sources: &[usize]) -> Vec<i32> {
    let mut d = vec![-1; rooms.len()];
    let mut q = std::collections::VecDeque::new();
    for &s in sources {
        d[s] = 0;
        q.push_back(s);
    }
    while let Some(r) = q.pop_front() {
        for &nb in &rooms[r].neighbors {
            if d[nb] < 0 {
                d[nb] = d[r] + 1;
                q.push_back(nb);
            }
        }
    }
    d
}

fn analyse(d: &mut Dungeon, cfg: &Config, rng: &mut Rng) {
    let (w, h) = (d.w, d.h);
    let n = d.rooms.len();
    let marker_room = |d: &Dungeon, m: char| d.rooms.iter().position(|r| r.markers.iter().any(|mk| mk.2 == m));
    let farthest_from = |d: &Dungeon, r: usize| -> usize {
        let dist = bfs(&d.cells, w, h, &[d.rooms[r].center]);
        (0..n).filter(|&k| k != r).max_by_key(|&k| (dist[d.rooms[k].center.1 * w + d.rooms[k].center.0], std::cmp::Reverse(k))).unwrap_or(r)
    };
    let markers_ok = cfg.start == "auto";
    let start_room = match (markers_ok.then(|| marker_room(d, 'S')).flatten(), cfg.start.as_str()) {
        (Some(r), _) => r,
        (None, "random") => rng.index(n),
        _ => {
            // double sweep: the farthest room from room 0, then the farthest from that one
            let a = farthest_from(d, 0);
            if markers_ok && marker_room(d, 'X').is_some() {
                farthest_from(d, marker_room(d, 'X').expect("checked"))
            } else {
                a
            }
        }
    };
    let exit_room = match markers_ok.then(|| marker_room(d, 'X')).flatten() {
        Some(r) if r != start_room || n == 1 => r,
        _ => farthest_from(d, start_room),
    };
    let marker_cell = |r: &Room, m: char| r.markers.iter().find(|mk| mk.2 == m).map(|mk| (mk.0, mk.1));
    d.start_room = start_room;
    d.exit_room = exit_room;
    d.start = marker_cell(&d.rooms[start_room], 'S').unwrap_or(d.rooms[start_room].center);
    d.exit = marker_cell(&d.rooms[exit_room], 'X').unwrap_or(d.rooms[exit_room].center);
    if d.exit == d.start {
        // a single room: the floor cell farthest from the start
        let dist = bfs(&d.cells, w, h, &[d.start]);
        if let Some(&c) = d.rooms[exit_room].floor.iter().max_by_key(|c| dist[c.1 * w + c.0]) {
            d.exit = c;
        }
    }
    d.rooms[start_room].is_start = true;
    d.rooms[exit_room].is_exit = true;
    d.dist = bfs(&d.cells, w, h, &[d.start]);
    // main path: walk back from the exit along decreasing distances
    d.on_path = vec![false; w * h];
    let mut cur = d.exit;
    if d.dist[cur.1 * w + cur.0] >= 0 {
        let mut path = vec![cur];
        while cur != d.start {
            let cd = d.dist[cur.1 * w + cur.0];
            let next = DIRS.iter().map(|(dx, dy)| (cur.0 as i64 + dx, cur.1 as i64 + dy)).find(|&(x, y)| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && d.dist[y as usize * w + x as usize] == cd - 1);
            let Some((x, y)) = next else { break };
            cur = (x as usize, y as usize);
            path.push(cur);
        }
        path.reverse();
        d.main_path = path;
    } else {
        d.warnings.push("la salida no es alcanzable desde el inicio".into());
    }
    for &(x, y) in &d.main_path {
        d.on_path[y * w + x] = true;
    }
    // rooms along the path, in order
    let mut idx = 0;
    for &(x, y) in &d.main_path.clone() {
        let r = d.room_of[y * w + x];
        if r >= 0 && d.rooms[r as usize].path_index < 0 {
            d.rooms[r as usize].path_index = idx;
            d.rooms[r as usize].on_main_path = true;
            idx += 1;
        }
    }
    let depth = graph_bfs(&d.rooms, &[start_room]);
    let on: Vec<usize> = (0..n).filter(|&r| d.rooms[r].on_main_path).collect();
    let from_main = graph_bfs(&d.rooms, &on);
    for r in 0..n {
        let c = d.rooms[r].center;
        d.rooms[r].depth = depth[r];
        d.rooms[r].dist_from_main = from_main[r];
        d.rooms[r].distance = d.dist[c.1 * w + c.0];
    }
    // dead-end rooms: leaves of the room graph, or tips of side branches (off the main path,
    // no neighbour farther from it) so loops do not hide them
    for r in 0..n {
        let room = &d.rooms[r];
        let tip = !room.on_main_path && room.dist_from_main > 0 && room.neighbors.iter().all(|&nb| d.rooms[nb].dist_from_main <= room.dist_from_main);
        d.rooms[r].dead_end = (room.neighbors.len() == 1 || tip) && r != start_room && r != exit_room;
    }
    // dead-end cells: walkable with exactly one walkable neighbour
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            if !walkable(d.cells[y * w + x]) {
                continue;
            }
            let k = DIRS.iter().filter(|(dx, dy)| walkable(d.cell(x as i64 + dx, y as i64 + dy))).count();
            if k == 1 {
                d.dead_ends.push((x, y));
            }
        }
    }
}

fn place_events(d: &mut Dungeon, cfg: &Config, rng: &mut Rng) {
    let w = d.w;
    let mut taken = vec![false; d.w * d.h];
    taken[d.start.1 * w + d.start.0] = true;
    taken[d.exit.1 * w + d.exit.0] = true;
    for (ri, rule) in cfg.events.iter().enumerate() {
        if rule.chance < 1.0 && rng.f() >= rule.chance {
            continue;
        }
        let want = rng.range(rule.count.0 as i64, rule.count.1 as i64) as usize;
        if want == 0 {
            continue;
        }
        // candidate rooms (room-based rules) or cells
        let room_ok = |d: &Dungeon, r: usize| -> bool {
            let room = &d.rooms[r];
            if rule.exclude_start && r == d.start_room {
                return false;
            }
            if let Some(m) = rule.on_main_path {
                if room.on_main_path != m {
                    return false;
                }
            }
            if !rule.room_tags.is_empty() && !rule.room_tags.iter().any(|t| room.tags.contains(t)) {
                return false;
            }
            true
        };
        let cell_ok = |d: &Dungeon, taken: &[bool], events: &[Event], x: usize, y: usize, room_rule: bool| -> bool {
            let i = y * w + x;
            if taken[i] || d.cells[i] == DOOR || d.cells[i] == WALL {
                return false;
            }
            let dd = d.dist[i];
            if dd < 0 || rule.min_distance.is_some_and(|m| dd < m) || rule.max_distance.is_some_and(|m| dd > m) {
                return false;
            }
            if !room_rule {
                if let Some(m) = rule.on_main_path {
                    if d.on_path[i] != m {
                        return false;
                    }
                }
                if rule.exclude_start && d.room_of[i] >= 0 && d.room_of[i] as usize == d.start_room {
                    return false;
                }
                if !rule.room_tags.is_empty() {
                    let r = d.room_of[i];
                    if r < 0 || !rule.room_tags.iter().any(|t| d.rooms[r as usize].tags.contains(t)) {
                        return false;
                    }
                }
            }
            if rule.min_spacing > 0 && events.iter().any(|e| (e.x as i64 - x as i64).abs().max((e.y as i64 - y as i64).abs()) < rule.min_spacing as i64) {
                return false;
            }
            true
        };
        let order = |rng: &mut Rng, v: &mut Vec<(usize, i32)>| {
            rng.shuffle(v);
            match rule.prefer.as_str() {
                "far" => v.sort_by_key(|x| std::cmp::Reverse(x.1)),
                "near" => v.sort_by_key(|x| x.1),
                _ => {}
            }
        };
        let mut placed = 0;
        let mut new_events: Vec<Event> = vec![];
        if rule.r#where.room_based() {
            let mut rooms: Vec<(usize, i32)> = match rule.r#where {
                Where::StartRoom => vec![(d.start_room, 0)],
                Where::ExitRoom => vec![(d.exit_room, 0)],
                Where::DeadEnd => {
                    let v: Vec<(usize, i32)> = (0..d.rooms.len()).filter(|&r| d.rooms[r].dead_end).map(|r| (r, d.rooms[r].distance)).collect();
                    v
                }
                _ => (0..d.rooms.len()).map(|r| (r, d.rooms[r].distance)).collect(),
            };
            rooms.retain(|&(r, _)| rule.r#where == Where::StartRoom || room_ok(d, r));
            order(rng, &mut rooms);
            let per_room = if rule.per_room == 0 { usize::MAX } else { rule.per_room };
            let mut counts = vec![0usize; d.rooms.len()];
            // several passes so per_room > 1 spreads over rooms first
            'outer: for _ in 0..per_room.min(64) {
                let mut progress = false;
                for &(r, _) in &rooms {
                    if placed >= want {
                        break 'outer;
                    }
                    if counts[r] >= per_room {
                        continue;
                    }
                    let room = &d.rooms[r];
                    // marker cells first, then the centre, then any floor cell
                    let mut cands: Vec<(usize, usize)> = vec![];
                    if let Some(m) = rule.marker {
                        let mut mk: Vec<(usize, usize)> = room.markers.iter().filter(|x| x.2 == m).map(|x| (x.0, x.1)).collect();
                        rng.shuffle(&mut mk);
                        cands.extend(mk);
                    }
                    // cells with other markers last: they are kept for the rules that ask for them
                    let marked = |c: &(usize, usize)| room.markers.iter().any(|m| (m.0, m.1) == *c);
                    let mut fl = room.floor.clone();
                    rng.shuffle(&mut fl);
                    if !marked(&room.center) {
                        cands.push(room.center);
                    }
                    cands.extend(fl.iter().filter(|c| !marked(c)));
                    cands.extend(fl.iter().filter(|c| marked(c)));
                    let all_events: Vec<Event> = d.events.iter().chain(new_events.iter()).cloned().collect();
                    if let Some(&(x, y)) = cands.iter().find(|&&(x, y)| cell_ok(d, &taken, &all_events, x, y, true)) {
                        taken[y * w + x] = true;
                        new_events.push(Event { kind: String::new(), x, y, room: Some(r), distance: d.dist[y * w + x], on_main_path: d.on_path[y * w + x], rule: ri, data: rule.data.clone() });
                        counts[r] += 1;
                        placed += 1;
                        progress = true;
                    }
                }
                if !progress {
                    break;
                }
            }
        } else {
            let mut cells: Vec<(usize, i32)> = match rule.r#where {
                Where::Corridor => (0..d.cells.len()).filter(|&i| d.cells[i] == CORRIDOR).map(|i| (i, d.dist[i])).collect(),
                Where::DeadEndCell => d.dead_ends.iter().map(|&(x, y)| (y * w + x, d.dist[y * w + x])).collect(),
                Where::MainPath => d.main_path.iter().map(|&(x, y)| (y * w + x, d.dist[y * w + x])).collect(),
                Where::Marker => {
                    let m = rule.marker.expect("checked");
                    d.rooms.iter().flat_map(|r| r.markers.iter()).filter(|x| x.2 == m).map(|x| (x.1 * w + x.0, d.dist[x.1 * w + x.0])).collect()
                }
                _ => (0..d.cells.len()).filter(|&i| walkable(d.cells[i])).map(|i| (i, d.dist[i])).collect(),
            };
            cells.sort_unstable();
            order(rng, &mut cells);
            let per_room = if rule.per_room == 0 { usize::MAX } else { rule.per_room };
            let mut counts = vec![0usize; d.rooms.len()];
            for (i, _) in cells {
                if placed >= want {
                    break;
                }
                let (x, y) = (i % w, i / w);
                let r = d.room_of[i];
                if r >= 0 && counts[r as usize] >= per_room {
                    continue;
                }
                let all_events: Vec<Event> = d.events.iter().chain(new_events.iter()).cloned().collect();
                if cell_ok(d, &taken, &all_events, x, y, false) {
                    taken[i] = true;
                    new_events.push(Event { kind: String::new(), x, y, room: (r >= 0).then_some(r as usize), distance: d.dist[i], on_main_path: d.on_path[i], rule: ri, data: rule.data.clone() });
                    if r >= 0 {
                        counts[r as usize] += 1;
                    }
                    placed += 1;
                }
            }
        }
        let weights: Vec<f64> = rule.types.iter().map(|t| t.1).collect();
        for mut e in new_events {
            let k = rng.weighted(&weights).unwrap_or(0);
            e.kind = rule.types[k].0.clone();
            d.events.push(e);
        }
        if placed < want {
            d.warnings.push(format!("regla {ri} ({}): colocados {placed} de {want}", rule.types[0].0));
        }
    }
}

// ------------------------------------------------------------------ debug drawing

/// What `dungeon_debug_draw` / the CLI draw.
#[derive(Clone, Debug)]
pub struct DrawOpts {
    pub cells: bool,
    pub rooms: bool,
    pub delaunay: bool,
    pub mst: bool,
    pub extra: bool,
    pub main_path: bool,
    pub distance: bool,
    pub events: bool,
}

impl Default for DrawOpts {
    fn default() -> Self {
        DrawOpts { cells: true, rooms: true, delaunay: false, mst: true, extra: true, main_path: true, distance: false, events: true }
    }
}

/// Something rectangles and lines can be drawn on (a PNG, the engine's draw list).
pub trait Canvas {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, rgb: u32, alpha: f32);
    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, rgb: u32, alpha: f32);
}

pub const COLOR_WALL: u32 = 0x1B1B24;
pub const COLOR_FLOOR: u32 = 0x6D6D7A;
pub const COLOR_CORRIDOR: u32 = 0x9A9077;
pub const COLOR_DOOR: u32 = 0xC0843E;
pub const COLOR_MST: u32 = 0xFFD54F;
pub const COLOR_EXTRA: u32 = 0xF06292;
pub const COLOR_DELAUNAY: u32 = 0x4FC3F7;
pub const COLOR_PATH: u32 = 0xE53935;
pub const COLOR_START: u32 = 0x43A047;
pub const COLOR_EXIT: u32 = 0x2979FF;

/// A colour per event type: well-known names get fixed colours, others a hashed one.
pub fn event_color(kind: &str) -> u32 {
    match kind {
        "enemy" | "enemigo" => 0xFF5252,
        "chest" | "cofre" | "treasure" => 0xFFCA28,
        "boss" | "jefe" => 0xD500F9,
        "miniboss" | "elite" => 0xFF9100,
        "shop" | "tienda" => 0x00E676,
        "heal" | "fountain" | "well" => 0x40C4FF,
        _ => {
            const P: [u32; 8] = [0x26C6DA, 0x9CCC65, 0xFFEE58, 0xEC407A, 0x7E57C2, 0x8D6E63, 0x29B6F6, 0xD4E157];
            P[(seed_from_text(kind) % P.len() as u64) as usize]
        }
    }
}

/// Draws the dungeon at (x, y) with `cs` pixels per cell. Runs of equal cells are merged
/// into one rectangle, so a 64×64 map is a few hundred rectangles.
pub fn draw(d: &Dungeon, c: &mut dyn Canvas, x: f32, y: f32, cs: f32, o: &DrawOpts) {
    let (w, h) = (d.w, d.h);
    let maxd = d.dist.iter().copied().max().unwrap_or(1).max(1) as f32;
    if o.cells {
        for cy in 0..h {
            let mut cx = 0;
            while cx < w {
                let v = d.cells[cy * w + cx];
                let mut end = cx + 1;
                while end < w && d.cells[cy * w + end] == v {
                    end += 1;
                }
                let col = match v {
                    FLOOR => COLOR_FLOOR,
                    DOOR => COLOR_DOOR,
                    CORRIDOR => COLOR_CORRIDOR,
                    _ => COLOR_WALL,
                };
                c.rect(x + cx as f32 * cs, y + cy as f32 * cs, (end - cx) as f32 * cs, cs, col, 1.0);
                cx = end;
            }
        }
    }
    if o.distance {
        for cy in 0..h {
            for cx in 0..w {
                let dd = d.dist[cy * w + cx];
                if dd < 0 {
                    continue;
                }
                let t = dd as f32 / maxd;
                let r = (t * 255.0) as u32;
                let b = ((1.0 - t) * 255.0) as u32;
                c.rect(x + cx as f32 * cs, y + cy as f32 * cs, cs, cs, (r << 16) | (40 << 8) | b, 0.45);
            }
        }
    }
    if o.main_path {
        let m = if cs < 6.0 { cs * 0.25 } else { (cs * 0.3).max(1.0).min(cs * 0.5) };
        for &(px, py) in &d.main_path {
            c.rect(x + px as f32 * cs + m, y + py as f32 * cs + m, cs - 2.0 * m, cs - 2.0 * m, COLOR_PATH, 0.85);
        }
    }
    let centre = |r: usize| (x + (d.rooms[r].center.0 as f32 + 0.5) * cs, y + (d.rooms[r].center.1 as f32 + 0.5) * cs);
    if o.rooms {
        for r in &d.rooms {
            let (rx, ry, rw, rh) = (x + r.x as f32 * cs, y + r.y as f32 * cs, r.w as f32 * cs, r.h as f32 * cs);
            let t = (cs * 0.15).max(1.0);
            let col = if r.is_start { COLOR_START } else if r.is_exit { COLOR_EXIT } else { 0xFFFFFF };
            c.rect(rx, ry, rw, t, col, 0.5);
            c.rect(rx, ry + rh - t, rw, t, col, 0.5);
            c.rect(rx, ry, t, rh, col, 0.5);
            c.rect(rx + rw - t, ry, t, rh, col, 0.5);
        }
    }
    let lw = (cs * 0.25).max(1.0);
    if o.delaunay {
        for &(a, b) in &d.delaunay {
            let ((x1, y1), (x2, y2)) = (centre(a), centre(b));
            c.line(x1, y1, x2, y2, (lw * 0.5).max(1.0), COLOR_DELAUNAY, 0.5);
        }
    }
    if o.mst {
        for &(a, b) in &d.mst {
            let ((x1, y1), (x2, y2)) = (centre(a), centre(b));
            c.line(x1, y1, x2, y2, lw, COLOR_MST, 0.9);
        }
    }
    if o.extra {
        for &(a, b) in &d.extra {
            let ((x1, y1), (x2, y2)) = (centre(a), centre(b));
            c.line(x1, y1, x2, y2, lw, COLOR_EXTRA, 0.9);
        }
    }
    // small cells (minimaps): whole cells without outlines, or they vanish
    let small = cs < 6.0;
    if o.events {
        let m = (cs * 0.15).max(0.0);
        for e in &d.events {
            let (ex, ey) = (x + e.x as f32 * cs, y + e.y as f32 * cs);
            if small {
                c.rect(ex, ey, cs, cs, event_color(&e.kind), 1.0);
            } else {
                c.rect(ex + m, ey + m, cs - 2.0 * m, cs - 2.0 * m, 0x000000, 1.0);
                c.rect(ex + m + 1.0, ey + m + 1.0, cs - 2.0 * m - 2.0, cs - 2.0 * m - 2.0, event_color(&e.kind), 1.0);
            }
        }
    }
    for (p, col) in [(d.start, COLOR_START), (d.exit, COLOR_EXIT)] {
        let (px, py) = (x + p.0 as f32 * cs, y + p.1 as f32 * cs);
        if small {
            c.rect(px, py, cs, cs, col, 1.0);
        } else {
            c.rect(px, py, cs, cs, 0xFFFFFF, 1.0);
            c.rect(px + 1.0, py + 1.0, cs - 2.0, cs - 2.0, col, 1.0);
        }
    }
}

/// A software canvas: RGBA pixels (for PNG previews).
pub struct Raster {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl Raster {
    pub fn new(w: u32, h: u32, rgb: u32) -> Raster {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            px.extend_from_slice(&[(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255]);
        }
        Raster { w, h, px }
    }
    fn blend(&mut self, x: i64, y: i64, rgb: u32, a: f32) {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        for (k, sh) in [16, 8, 0].iter().enumerate() {
            let s = ((rgb >> sh) & 0xFF) as f32;
            self.px[i + k] = (self.px[i + k] as f32 * (1.0 - a) + s * a).round() as u8;
        }
    }
}

impl Canvas for Raster {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, rgb: u32, alpha: f32) {
        let (x0, y0, x1, y1) = (x.round() as i64, y.round() as i64, (x + w).round() as i64, (y + h).round() as i64);
        for py in y0..y1 {
            for px in x0..x1 {
                self.blend(px, py, rgb, alpha);
            }
        }
    }
    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, rgb: u32, alpha: f32) {
        let len = ((x2 - x1).powi(2) + (y2 - y1).powi(2)).sqrt().max(1.0);
        let steps = (len * 2.0) as i64;
        let r = (width / 2.0).max(0.5);
        let mut last = (i64::MIN, i64::MIN);
        let mut seen = std::collections::HashSet::new();
        for s in 0..=steps {
            let t = s as f32 / steps.max(1) as f32;
            let (cx, cy) = (x1 + (x2 - x1) * t, y1 + (y2 - y1) * t);
            let c = (cx.round() as i64, cy.round() as i64);
            if c == last {
                continue;
            }
            last = c;
            let ri = r.ceil() as i64;
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    let (px, py) = (c.0 + dx, c.1 + dy);
                    if ((px as f32 - cx).powi(2) + (py as f32 - cy).powi(2)).sqrt() <= r && seen.insert((px, py)) {
                        self.blend(px, py, rgb, alpha);
                    }
                }
            }
        }
    }
}

/// The dungeon as an RGBA image (`cs` pixels per cell).
pub fn render_rgba(d: &Dungeon, cs: u32, o: &DrawOpts) -> Raster {
    let mut r = Raster::new(d.w as u32 * cs, d.h as u32 * cs, COLOR_WALL);
    draw(d, &mut r, 0.0, 0.0, cs as f32, o);
    r
}

pub fn write_png(r: &Raster, path: &std::path::Path) -> Result<(), String> {
    image::RgbaImage::from_raw(r.w, r.h, r.px.clone()).ok_or("imagen vacía")?.save(path).map_err(|e| format!("no se pudo escribir {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_rotation_keeps_markers_on_floor() {
        let rows: Vec<Vec<char>> = ["#####", "#.E.#", "#...D", "#####"].iter().map(|r| r.chars().collect()).collect();
        let s = Shape::from_rows(&rows, None);
        for t in 0..4 {
            for f in [false, true] {
                let r = s.transformed(t, f);
                assert_eq!(r.markers.len(), 1);
                let (x, y, c) = r.markers[0];
                assert_eq!(c, 'E');
                assert_eq!(r.at(x as i64, y as i64), Sc::Floor);
                assert_eq!(r.door_spots().len(), 1);
            }
        }
    }

    #[test]
    fn rect_room_has_ring_and_door_spots() {
        let s = Shape::rect(3, 2);
        assert_eq!((s.w, s.h), (5, 4));
        // 3 + 3 + 2 + 2 wall cells on the sides (no corners)
        assert_eq!(s.door_spots().len(), 10);
    }
}

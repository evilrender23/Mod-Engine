//! Navigation grids (`mp_grid_*`): blocked cells, A* paths, BFS distance maps and flow
//! fields, plus a DDA raycast through the cells. Pure data, no engine access, so it is
//! easy to test and reuse.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

/// A grid of `w` × `h` cells of `cw` × `ch` pixels whose top-left corner is (`left`, `top`).
#[derive(Clone, Debug)]
pub struct MpGrid {
    pub left: f64,
    pub top: f64,
    pub w: usize,
    pub h: usize,
    pub cw: f64,
    pub ch: f64,
    /// true = blocked.
    pub cells: Vec<bool>,
}

/// Result of a flow-field / BFS computation: distance (in cells, diagonals cost √2) from the
/// goals to every cell, `INF` where unreachable.
#[derive(Clone, Debug)]
pub struct Flow {
    pub grid: usize,
    pub w: usize,
    pub h: usize,
    pub dist: Vec<f64>,
    pub diag: bool,
    pub goals: Vec<(usize, usize)>,
}

pub const INF: f64 = f64::INFINITY;

const DIRS4: [(i64, i64); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
const DIRS8: [(i64, i64); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)];

/// Hit of a ray through the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridHit {
    pub x: f64,
    pub y: f64,
    pub dist: f64,
    pub nx: f64,
    pub ny: f64,
    pub cx: usize,
    pub cy: usize,
}

impl MpGrid {
    pub fn new(left: f64, top: f64, w: usize, h: usize, cw: f64, ch: f64) -> Self {
        MpGrid { left, top, w, h, cw: cw.max(1e-6), ch: ch.max(1e-6), cells: vec![false; w * h] }
    }

    pub fn in_bounds(&self, cx: i64, cy: i64) -> bool {
        cx >= 0 && cy >= 0 && (cx as usize) < self.w && (cy as usize) < self.h
    }

    /// Is the cell blocked? Cells outside the grid count as blocked.
    pub fn blocked(&self, cx: i64, cy: i64) -> bool {
        !self.in_bounds(cx, cy) || self.cells[cy as usize * self.w + cx as usize]
    }

    pub fn set(&mut self, cx: i64, cy: i64, v: bool) {
        if self.in_bounds(cx, cy) {
            let w = self.w;
            self.cells[cy as usize * w + cx as usize] = v;
        }
    }

    /// Cell containing a pixel position (may be outside the grid).
    pub fn cell_of(&self, x: f64, y: f64) -> (i64, i64) {
        (((x - self.left) / self.cw).floor() as i64, ((y - self.top) / self.ch).floor() as i64)
    }

    pub fn center(&self, cx: usize, cy: usize) -> (f64, f64) {
        (self.left + (cx as f64 + 0.5) * self.cw, self.top + (cy as f64 + 0.5) * self.ch)
    }

    /// Marks (or clears) every cell overlapping the rectangle (pixels, inclusive corners like GameMaker).
    pub fn fill_rect(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, v: bool) {
        let (ax, ay) = self.cell_of(x1.min(x2), y1.min(y2));
        let (bx, by) = self.cell_of(x1.max(x2), y1.max(y2));
        for cy in ay.max(0)..=by.min(self.h as i64 - 1) {
            for cx in ax.max(0)..=bx.min(self.w as i64 - 1) {
                self.set(cx, cy, v);
            }
        }
    }

    /// Marks cells whose area overlaps the box `[x1, y1, x2, y2)` (right/bottom edges exclusive),
    /// the way instances block cells.
    pub fn fill_box(&mut self, b: [f64; 4]) {
        let eps = 1e-6;
        let (ax, ay) = self.cell_of(b[0], b[1]);
        let (bx, by) = self.cell_of(b[2] - eps, b[3] - eps);
        for cy in ay.max(0)..=by.min(self.h as i64 - 1) {
            for cx in ax.max(0)..=bx.min(self.w as i64 - 1) {
                self.set(cx, cy, true);
            }
        }
    }

    /// Neighbours of a cell; with `diag`, diagonal moves don't cut blocked corners.
    fn neighbours(&self, cx: usize, cy: usize, diag: bool) -> impl Iterator<Item = (usize, usize, f64)> + '_ {
        let dirs: &[(i64, i64)] = if diag { &DIRS8 } else { &DIRS4 };
        dirs.iter().filter_map(move |&(dx, dy)| {
            let (nx, ny) = (cx as i64 + dx, cy as i64 + dy);
            if self.blocked(nx, ny) {
                return None;
            }
            if dx != 0 && dy != 0 && (self.blocked(cx as i64 + dx, cy as i64) || self.blocked(cx as i64, cy as i64 + dy)) {
                return None;
            }
            Some((nx as usize, ny as usize, if dx != 0 && dy != 0 { std::f64::consts::SQRT_2 } else { 1.0 }))
        })
    }

    /// A* from cell `a` to cell `b`. Returns the cells of the path (both ends included).
    pub fn astar(&self, a: (usize, usize), b: (usize, usize), diag: bool) -> Option<Vec<(usize, usize)>> {
        if self.blocked(a.0 as i64, a.1 as i64) || self.blocked(b.0 as i64, b.1 as i64) {
            return None;
        }
        let n = self.w * self.h;
        let idx = |c: (usize, usize)| c.1 * self.w + c.0;
        let heur = |c: (usize, usize)| {
            let (dx, dy) = ((c.0 as f64 - b.0 as f64).abs(), (c.1 as f64 - b.1 as f64).abs());
            if diag {
                dx.max(dy) + (std::f64::consts::SQRT_2 - 1.0) * dx.min(dy)
            } else {
                dx + dy
            }
        };
        let mut g = vec![INF; n];
        let mut from = vec![usize::MAX; n];
        let mut closed = vec![false; n];
        let mut open = BinaryHeap::new();
        g[idx(a)] = 0.0;
        let mut seq = 0u64;
        open.push(Node { f: heur(a), h: heur(a), seq, cell: idx(a) });
        while let Some(Node { cell, .. }) = open.pop() {
            if closed[cell] {
                continue;
            }
            closed[cell] = true;
            if cell == idx(b) {
                let mut out = vec![];
                let mut c = cell;
                while c != usize::MAX {
                    out.push((c % self.w, c / self.w));
                    c = from[c];
                }
                out.reverse();
                return Some(out);
            }
            let (cx, cy) = (cell % self.w, cell / self.w);
            for (nx, ny, cost) in self.neighbours(cx, cy, diag) {
                let ni = ny * self.w + nx;
                let ng = g[cell] + cost;
                if ng + 1e-9 < g[ni] {
                    g[ni] = ng;
                    from[ni] = cell;
                    seq += 1;
                    let h = heur((nx, ny));
                    open.push(Node { f: ng + h, h, seq, cell: ni });
                }
            }
        }
        None
    }

    /// Distance (Dijkstra; plain BFS when `diag` is false) from the goal cells to every cell.
    /// `max` stops the search beyond that distance (cells further away stay unreachable).
    pub fn distances(&self, goals: &[(usize, usize)], diag: bool, max: f64) -> Vec<f64> {
        let n = self.w * self.h;
        let mut dist = vec![INF; n];
        if !diag {
            let mut q = VecDeque::new();
            for &(gx, gy) in goals {
                if !self.blocked(gx as i64, gy as i64) {
                    dist[gy * self.w + gx] = 0.0;
                    q.push_back((gx, gy));
                }
            }
            while let Some((cx, cy)) = q.pop_front() {
                let d = dist[cy * self.w + cx] + 1.0;
                if d > max {
                    continue;
                }
                for (nx, ny, _) in self.neighbours(cx, cy, false) {
                    let ni = ny * self.w + nx;
                    if dist[ni] == INF {
                        dist[ni] = d;
                        q.push_back((nx, ny));
                    }
                }
            }
            return dist;
        }
        let mut heap = BinaryHeap::new();
        let mut seq = 0;
        for &(gx, gy) in goals {
            if !self.blocked(gx as i64, gy as i64) {
                dist[gy * self.w + gx] = 0.0;
                heap.push(Node { f: 0.0, h: 0.0, seq, cell: gy * self.w + gx });
                seq += 1;
            }
        }
        while let Some(Node { f, cell, .. }) = heap.pop() {
            if f > dist[cell] {
                continue;
            }
            let (cx, cy) = (cell % self.w, cell / self.w);
            for (nx, ny, cost) in self.neighbours(cx, cy, true) {
                let ni = ny * self.w + nx;
                let nd = f + cost;
                if nd <= max && nd + 1e-9 < dist[ni] {
                    dist[ni] = nd;
                    seq += 1;
                    heap.push(Node { f: nd, h: 0.0, seq, cell: ni });
                }
            }
        }
        dist
    }

    /// First blocked cell on the segment (see [`dda`]).
    pub fn raycast(&self, x1: f64, y1: f64, x2: f64, y2: f64) -> Option<GridHit> {
        let c = Cells { left: self.left, top: self.top, cw: self.cw, ch: self.ch, w: self.w, h: self.h };
        dda(&c, &|cx, cy| self.cells[cy as usize * self.w + cx as usize], x1, y1, x2, y2)
    }
}

/// Geometry of a grid of cells (an mp_grid or a tile layer).
#[derive(Clone, Copy, Debug)]
pub struct Cells {
    pub left: f64,
    pub top: f64,
    pub cw: f64,
    pub ch: f64,
    pub w: usize,
    pub h: usize,
}

/// Steps a ray from (x1, y1) towards (x2, y2) through the cells (DDA) and returns the first
/// blocked cell it enters, with the hit point and the normal of the face it crossed.
/// `blocked` is only asked about cells inside the grid; cells outside never block.
/// A ray that starts inside a blocked cell hits at its start with normal (0, 0).
pub fn dda(c: &Cells, blocked: &dyn Fn(i64, i64) -> bool, x1: f64, y1: f64, x2: f64, y2: f64) -> Option<GridHit> {
    let inside = |cx: i64, cy: i64| cx >= 0 && cy >= 0 && (cx as usize) < c.w && (cy as usize) < c.h;
    let (dx, dy) = (x2 - x1, y2 - y1);
    let len = dx.hypot(dy);
    let mut cx = ((x1 - c.left) / c.cw).floor() as i64;
    let mut cy = ((y1 - c.top) / c.ch).floor() as i64;
    if inside(cx, cy) && blocked(cx, cy) {
        return Some(GridHit { x: x1, y: y1, dist: 0.0, nx: 0.0, ny: 0.0, cx: cx as usize, cy: cy as usize });
    }
    if len == 0.0 {
        return None;
    }
    let (ux, uy) = (dx / len, dy / len);
    let step_x: i64 = if ux > 0.0 { 1 } else { -1 };
    let step_y: i64 = if uy > 0.0 { 1 } else { -1 };
    let next_edge = |cell: i64, step: i64, origin: f64, size: f64| origin + (cell + if step > 0 { 1 } else { 0 }) as f64 * size;
    let mut t_max_x = if ux != 0.0 { (next_edge(cx, step_x, c.left, c.cw) - x1) / ux } else { INF };
    let mut t_max_y = if uy != 0.0 { (next_edge(cy, step_y, c.top, c.ch) - y1) / uy } else { INF };
    let t_dx = if ux != 0.0 { c.cw / ux.abs() } else { INF };
    let t_dy = if uy != 0.0 { c.ch / uy.abs() } else { INF };
    loop {
        let (t, nx, ny) = if t_max_x < t_max_y {
            cx += step_x;
            let t = t_max_x;
            t_max_x += t_dx;
            (t, -step_x as f64, 0.0)
        } else {
            cy += step_y;
            let t = t_max_y;
            t_max_y += t_dy;
            (t, 0.0, -step_y as f64)
        };
        if t > len || !t.is_finite() {
            return None;
        }
        if inside(cx, cy) && blocked(cx, cy) {
            return Some(GridHit { x: x1 + ux * t, y: y1 + uy * t, dist: t, nx, ny, cx: cx as usize, cy: cy as usize });
        }
    }
}

impl Flow {
    pub fn at(&self, cx: i64, cy: i64) -> f64 {
        if cx < 0 || cy < 0 || cx as usize >= self.w || cy as usize >= self.h {
            return INF;
        }
        self.dist[cy as usize * self.w + cx as usize]
    }

    /// The neighbour cell that gets closest to the goal (None at a goal or when unreachable).
    pub fn next(&self, g: &MpGrid, cx: i64, cy: i64) -> Option<(usize, usize)> {
        let here = self.at(cx, cy);
        if here == INF || here == 0.0 {
            return None;
        }
        let mut best: Option<(usize, usize, f64)> = None;
        for (nx, ny, cost) in g.neighbours(cx as usize, cy as usize, self.diag) {
            let d = self.dist[ny * self.w + nx] + cost * 1e-6;
            if d < here && best.is_none_or(|b| d < b.2) {
                best = Some((nx, ny, d));
            }
        }
        best.map(|b| (b.0, b.1))
    }
}

#[derive(PartialEq)]
struct Node {
    f: f64,
    h: f64,
    seq: u64,
    cell: usize,
}

impl Eq for Node {}

impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        // min-heap on f, then on h (prefer nodes nearer the goal), then FIFO for determinism
        o.f.partial_cmp(&self.f)
            .unwrap_or(Ordering::Equal)
            .then(o.h.partial_cmp(&self.h).unwrap_or(Ordering::Equal))
            .then(o.seq.cmp(&self.seq))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// All grids and flow fields of the running game (ids are indices; deleted = None).
#[derive(Default)]
pub struct NavState {
    pub grids: Vec<Option<MpGrid>>,
    pub flows: Vec<Option<Flow>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str]) -> MpGrid {
        let mut g = MpGrid::new(0.0, 0.0, rows[0].len(), rows.len(), 10.0, 10.0);
        for (y, r) in rows.iter().enumerate() {
            for (x, c) in r.chars().enumerate() {
                g.set(x as i64, y as i64, c == '#');
            }
        }
        g
    }

    #[test]
    fn astar_goes_around_walls() {
        let g = grid(&["....", ".##.", ".#..", "...."]);
        let p = g.astar((0, 0), (2, 2), false).unwrap();
        assert_eq!(p.first(), Some(&(0, 0)));
        assert_eq!(p.last(), Some(&(2, 2)));
        assert_eq!(p.len(), 7, "{p:?}"); // 6 orthogonal moves around the wall
        assert!(p.iter().all(|&(x, y)| !g.blocked(x as i64, y as i64)));
        let open = grid(&["....", "....", "....", "...."]);
        assert_eq!(open.astar((0, 0), (3, 3), true).unwrap().len(), 4, "straight diagonal");
        assert_eq!(open.astar((0, 0), (3, 3), false).unwrap().len(), 7);
    }

    #[test]
    fn no_path_and_no_corner_cutting() {
        let g = grid(&[".#", "#."]);
        assert!(g.astar((0, 0), (1, 1), true).is_none(), "can't squeeze between two corners");
        assert!(g.astar((0, 0), (1, 0), false).is_none(), "goal blocked");
    }

    #[test]
    fn bfs_and_flow() {
        let g = grid(&["....", "###.", "...."]);
        let d = g.distances(&[(0, 2)], false, INF);
        assert_eq!(d[0], 8.0); // (0,0) → around the right end → (0,2)
        assert_eq!(d[4], INF); // wall
        let f = Flow { grid: 0, w: g.w, h: g.h, dist: d, diag: false, goals: vec![(0, 2)] };
        assert_eq!(f.next(&g, 0, 0), Some((1, 0)));
        assert_eq!(f.next(&g, 0, 2), None);
        let limited = g.distances(&[(0, 2)], false, 2.0);
        assert_eq!(limited[2 * 4 + 2], 2.0);
        assert_eq!(limited[0], INF);
    }

    #[test]
    fn grid_raycast_hits_face_with_normal() {
        let g = grid(&["....", "..#.", "...."]);
        let h = g.raycast(5.0, 15.0, 39.0, 15.0).unwrap();
        assert_eq!((h.x, h.y, h.nx, h.ny, h.cx, h.cy), (20.0, 15.0, -1.0, 0.0, 2, 1));
        assert_eq!(h.dist, 15.0);
        assert!(g.raycast(5.0, 5.0, 39.0, 5.0).is_none());
        let v = g.raycast(25.0, 29.0, 25.0, 0.0).unwrap();
        assert_eq!((v.y, v.ny), (20.0, 1.0));
    }
}

//! 2D grids of values with GameMaker's `ds_grid_*` semantics (see `natives/grid.rs`).
//!
//! A grid is `w`×`h` cells of any ModScript value (numbers, text, arrays...), stored row
//! by row. Grids live in [`DsGrids`] (`Engine::ds_grids`) and scripts use their index as
//! the id. Other natives (the first-person view, minimaps, the dungeon generator) read
//! them through [`DsGrids::get`] / [`DsGrid::num`] without going through ModScript.

use modscript::Value;

#[derive(Clone, Debug, Default)]
pub struct DsGrid {
    pub w: usize,
    pub h: usize,
    /// Row-major cells: `cells[y * w + x]`.
    pub cells: Vec<Value>,
}

impl std::fmt::Debug for DsGrids {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DsGrids({} slots)", self.list.len())
    }
}

impl DsGrid {
    /// A grid filled with 0 (GameMaker's default).
    pub fn new(w: usize, h: usize) -> DsGrid {
        DsGrid { w, h, cells: vec![Value::Num(0.0); w * h] }
    }

    /// A grid of numbers, `vals[y * w + x]`.
    pub fn from_nums(w: usize, h: usize, vals: &[f64]) -> DsGrid {
        DsGrid { w, h, cells: vals.iter().map(|&n| Value::Num(n)).collect() }
    }

    pub fn in_bounds(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h
    }

    /// The value at (x, y), or None outside the grid.
    pub fn get(&self, x: i64, y: i64) -> Option<&Value> {
        self.in_bounds(x, y).then(|| &self.cells[y as usize * self.w + x as usize])
    }

    /// The cell as a number (text, null and outside cells count as `default`).
    pub fn num(&self, x: i64, y: i64, default: f64) -> f64 {
        self.get(x, y).and_then(|v| v.as_num()).unwrap_or(default)
    }

    /// Every cell as an integer, row-major (non-numbers become `default`): what map
    /// renderers such as the first-person view want.
    pub fn to_i32(&self, default: i32) -> Vec<i32> {
        self.cells.iter().map(|v| v.as_num().map(|n| n as i32).unwrap_or(default)).collect()
    }

    /// Sets (x, y); false when outside the grid.
    pub fn set(&mut self, x: i64, y: i64, v: Value) -> bool {
        if !self.in_bounds(x, y) {
            return false;
        }
        let w = self.w;
        self.cells[y as usize * w + x as usize] = v;
        true
    }

    /// Changes the size keeping the cells that still fit (new cells are 0).
    pub fn resize(&mut self, w: usize, h: usize) {
        let mut cells = vec![Value::Num(0.0); w * h];
        for y in 0..h.min(self.h) {
            for x in 0..w.min(self.w) {
                cells[y * w + x] = std::mem::take(&mut self.cells[y * self.w + x]);
            }
        }
        *self = DsGrid { w, h, cells };
    }

    /// The cells of a region clamped to the grid: (x1, y1, x2, y2) inclusive, any order.
    pub fn region(&self, x1: i64, y1: i64, x2: i64, y2: i64) -> Option<(usize, usize, usize, usize)> {
        if self.w == 0 || self.h == 0 {
            return None;
        }
        let (ax, bx) = (x1.min(x2), x1.max(x2));
        let (ay, by) = (y1.min(y2), y1.max(y2));
        if bx < 0 || by < 0 || ax >= self.w as i64 || ay >= self.h as i64 {
            return None;
        }
        let c = |v: i64, n: usize| v.clamp(0, n as i64 - 1) as usize;
        Some((c(ax, self.w), c(ay, self.h), c(bx, self.w), c(by, self.h)))
    }

    /// Serialized as JSON text: `{"ds_grid":1,"w":W,"h":H,"rle":[[count, value], ...]}`
    /// (runs of equal values, so maps full of walls stay small).
    pub fn to_json(&self) -> Result<serde_json::Value, String> {
        let mut rle: Vec<serde_json::Value> = vec![];
        let mut run: Option<(serde_json::Value, u64)> = None;
        for v in &self.cells {
            let j = crate::natives::grid::value_to_json(v)?;
            match &mut run {
                Some((cur, n)) if *cur == j => *n += 1,
                _ => {
                    if let Some((cur, n)) = run.take() {
                        rle.push(serde_json::json!([n, cur]));
                    }
                    run = Some((j, 1));
                }
            }
        }
        if let Some((cur, n)) = run {
            rle.push(serde_json::json!([n, cur]));
        }
        Ok(serde_json::json!({ "ds_grid": 1, "w": self.w, "h": self.h, "rle": rle }))
    }

    /// The inverse of [`DsGrid::to_json`].
    pub fn from_json(j: &serde_json::Value) -> Result<DsGrid, String> {
        let bad = || "no es un ds_grid escrito con ds_grid_write()".to_string();
        let w = j.get("w").and_then(|v| v.as_u64()).ok_or_else(bad)? as usize;
        let h = j.get("h").and_then(|v| v.as_u64()).ok_or_else(bad)? as usize;
        if w.saturating_mul(h) > 16_000_000 {
            return Err(format!("ds_grid de {w}×{h} demasiado grande"));
        }
        let mut cells = Vec::with_capacity(w * h);
        for run in j.get("rle").and_then(|v| v.as_array()).ok_or_else(bad)? {
            let n = run.get(0).and_then(|v| v.as_u64()).ok_or_else(bad)? as usize;
            let v = crate::natives::grid::value_from_json(run.get(1).ok_or_else(bad)?);
            if cells.len() + n > w * h {
                return Err(bad());
            }
            cells.extend(std::iter::repeat_n(v, n));
        }
        if cells.len() != w * h {
            return Err(bad());
        }
        Ok(DsGrid { w, h, cells })
    }
}

/// Every grid created by `ds_grid_create` (None = destroyed slot, reused).
#[derive(Default)]
pub struct DsGrids {
    pub list: Vec<Option<DsGrid>>,
}

impl DsGrids {
    /// Stores a grid and returns its id.
    pub fn add(&mut self, g: DsGrid) -> usize {
        match self.list.iter().position(|s| s.is_none()) {
            Some(i) => {
                self.list[i] = Some(g);
                i
            }
            None => {
                self.list.push(Some(g));
                self.list.len() - 1
            }
        }
    }

    /// The grid with script id `id` (public accessor for other natives, e.g. fpview).
    pub fn get(&self, id: f64) -> Option<&DsGrid> {
        if id < 0.0 || id.fract() != 0.0 {
            return None;
        }
        self.list.get(id as usize).and_then(|g| g.as_ref())
    }

    pub fn get_mut(&mut self, id: f64) -> Option<&mut DsGrid> {
        if id < 0.0 || id.fract() != 0.0 {
            return None;
        }
        self.list.get_mut(id as usize).and_then(|g| g.as_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip_with_runs() {
        let mut g = DsGrid::new(5, 3);
        g.set(1, 1, Value::str("a"));
        g.set(4, 2, Value::Num(2.5));
        let j = g.to_json().unwrap();
        assert!(j["rle"].as_array().unwrap().len() <= 5);
        let back = DsGrid::from_json(&j).unwrap();
        assert_eq!((back.w, back.h), (5, 3));
        assert_eq!(back.get(1, 1).unwrap().to_string(), "a");
        assert_eq!(back.num(4, 2, 0.0), 2.5);
        assert_eq!(back.num(0, 0, -1.0), 0.0);
    }

    #[test]
    fn resize_keeps_cells() {
        let mut g = DsGrid::new(3, 3);
        g.set(2, 2, Value::Num(9.0));
        g.set(0, 1, Value::Num(4.0));
        g.resize(2, 4);
        assert_eq!(g.num(0, 1, 0.0), 4.0);
        assert!(g.get(2, 2).is_none());
        assert_eq!(g.num(1, 3, 1.0), 0.0);
    }
}

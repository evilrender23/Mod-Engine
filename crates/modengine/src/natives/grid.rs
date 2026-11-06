//! `ds_grid_*`: GameMaker's 2D grids (see `dsgrid.rs`), their serialization
//! (`ds_grid_write` / `ds_grid_read`), arrays of rows and `mp_grid_add_grid` to turn a
//! map into a navigation grid. `grid[# x, y]` in ModScript calls these functions.

use super::{err, rand01, Reg, A};
use crate::dsgrid::DsGrid;
use crate::engine::Engine;
use modscript::ast::BinOp;
use modscript::interp::binary;
use modscript::{RtError, RtResult, Value};

/// ModScript value -> JSON (same rules as `to_json` / save_data).
pub(crate) fn value_to_json(v: &Value) -> Result<serde_json::Value, String> {
    super::core::to_json(v).map_err(|e| e.to_string())
}

pub(crate) fn value_from_json(j: &serde_json::Value) -> Value {
    super::core::from_json(j)
}

fn grid<'a>(e: &'a Engine, a: &A, i: usize) -> RtResult<&'a DsGrid> {
    let id = a.num(i)?;
    e.ds_grids.get(id).ok_or_else(|| RtError::new(format!("{} no es un ds_grid (¿destruido?)", Value::Num(id))))
}

fn grid_mut<'a>(e: &'a mut Engine, a: &A, i: usize) -> RtResult<&'a mut DsGrid> {
    let id = a.num(i)?;
    e.ds_grids.get_mut(id).ok_or_else(|| RtError::new(format!("{} no es un ds_grid (¿destruido?)", Value::Num(id))))
}

/// The state of an `rng_create()` stream.
pub(crate) fn stream(e: &mut Engine, s: f64) -> RtResult<&mut u64> {
    let bad = || RtError::new(format!("{} no es un generador de rng_create() (¿destruido?)", Value::Num(s)));
    if s < 0.0 {
        return Err(bad());
    }
    e.rngs.state_mut(s as usize).ok_or_else(bad)
}

fn size(a: &A, i: usize, what: &str) -> RtResult<(usize, usize)> {
    let (w, h) = (a.num(i)?, a.num(i + 1)?);
    if w < 0.0 || h < 0.0 || w * h > 16_000_000.0 {
        return err(format!("{what}: {w}×{h} no es un tamaño de rejilla válido"));
    }
    Ok((w as usize, h as usize))
}

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => x == y,
        _ => binary(BinOp::Eq, a, b).map(|v| v.truthy()).unwrap_or(false),
    }
}

/// Applies `op` to every cell of the region (x1, y1, x2, y2) of argument 0's grid.
fn region_op(e: &mut Engine, a: &A, v: Value, op: Option<BinOp>) -> RtResult<Value> {
    let g = grid_mut(e, a, 0)?;
    let Some((x1, y1, x2, y2)) = g.region(a.int(1)?, a.int(2)?, a.int(3)?, a.int(4)?) else { return Ok(Value::Null) };
    for y in y1..=y2 {
        for x in x1..=x2 {
            let c = &mut g.cells[y * g.w + x];
            *c = match op {
                None => v.clone(),
                Some(op) => binary(op, c, &v)?,
            };
        }
    }
    Ok(Value::Null)
}

fn cell_op(e: &mut Engine, a: &A, op: BinOp) -> RtResult<Value> {
    let (x, y, v) = (a.int(1)?, a.int(2)?, a.val(3)?.clone());
    let g = grid_mut(e, a, 0)?;
    if !g.in_bounds(x, y) {
        return Ok(Value::Null);
    }
    let c = &mut g.cells[y as usize * g.w + x as usize];
    *c = binary(op, c, &v)?;
    Ok(Value::Null)
}

/// Numbers in a region (non-numbers are skipped).
fn region_nums(e: &Engine, a: &A) -> RtResult<Vec<f64>> {
    let g = grid(e, a, 0)?;
    let Some((x1, y1, x2, y2)) = g.region(a.int(1)?, a.int(2)?, a.int(3)?, a.int(4)?) else { return Ok(vec![]) };
    Ok((y1..=y2).flat_map(|y| (x1..=x2).map(move |x| (x, y))).filter_map(|(x, y)| g.cells[y * g.w + x].as_num()).collect())
}

/// First cell (row by row) of the region holding `v`.
fn find(e: &Engine, a: &A) -> RtResult<Option<(usize, usize)>> {
    let g = grid(e, a, 0)?;
    let v = a.val(5)?;
    let Some((x1, y1, x2, y2)) = g.region(a.int(1)?, a.int(2)?, a.int(3)?, a.int(4)?) else { return Ok(None) };
    for y in y1..=y2 {
        for x in x1..=x2 {
            if same(&g.cells[y * g.w + x], v) {
                return Ok(Some((x, y)));
            }
        }
    }
    Ok(None)
}

/// Rows of values (`[[a, b], [c, d]]`) -> grid.
fn grid_from_rows(rows: &Value) -> RtResult<DsGrid> {
    let Value::Array(rows) = rows else { return err("ds_grid_from_array: se esperaba un array de filas [[...], [...]]") };
    let rows = rows.borrow();
    let h = rows.len();
    let mut w = 0;
    for r in rows.iter() {
        match r {
            Value::Array(r) => w = w.max(r.borrow().len()),
            other => return err(format!("ds_grid_from_array: cada fila debe ser un array, no {}", other.type_name())),
        }
    }
    let mut g = DsGrid::new(w, h);
    for (y, r) in rows.iter().enumerate() {
        if let Value::Array(r) = r {
            for (x, v) in r.borrow().iter().enumerate() {
                g.cells[y * w + x] = v.clone();
            }
        }
    }
    Ok(g)
}

pub(super) fn register(r: &mut Reg) {
    r.f("ds_grid_create", |e, _, a| {
        let (w, h) = size(&A(a), 0, "ds_grid_create")?;
        Ok(Value::Num(e.ds_grids.add(DsGrid::new(w, h)) as f64))
    });
    r.f("ds_grid_destroy", |e, _, a| {
        let p = A(a);
        grid(e, &p, 0)?;
        e.ds_grids.list[p.num(0)? as usize] = None;
        Ok(Value::Null)
    });
    r.f("ds_grid_exists", |e, _, a| Ok(Value::Bool(A(a).num(0).ok().is_some_and(|id| e.ds_grids.get(id).is_some()))));
    r.f("ds_grid_width", |e, _, a| Ok(Value::Num(grid(e, &A(a), 0)?.w as f64)));
    r.f("ds_grid_height", |e, _, a| Ok(Value::Num(grid(e, &A(a), 0)?.h as f64)));
    r.f("ds_grid_get", |e, _, a| {
        // outside the grid: null (GameMaker's undefined)
        let p = A(a);
        Ok(grid(e, &p, 0)?.get(p.int(1)?, p.int(2)?).cloned().unwrap_or(Value::Null))
    });
    r.f("ds_grid_set", |e, _, a| {
        let p = A(a);
        let (x, y, v) = (p.int(1)?, p.int(2)?, p.val(3)?.clone());
        grid_mut(e, &p, 0)?.set(x, y, v);
        Ok(Value::Null)
    });
    r.f("ds_grid_add", |e, _, a| cell_op(e, &A(a), BinOp::Add));
    r.f("ds_grid_multiply", |e, _, a| cell_op(e, &A(a), BinOp::Mul));
    r.f("ds_grid_clear", |e, _, a| {
        let p = A(a);
        let v = p.val(1)?.clone();
        grid_mut(e, &p, 0)?.cells.iter_mut().for_each(|c| *c = v.clone());
        Ok(Value::Null)
    });
    r.f("ds_grid_set_region", |e, _, a| {
        let p = A(a);
        region_op(e, &p, p.val(5)?.clone(), None)
    });
    r.f("ds_grid_add_region", |e, _, a| {
        let p = A(a);
        region_op(e, &p, p.val(5)?.clone(), Some(BinOp::Add))
    });
    r.f("ds_grid_multiply_region", |e, _, a| {
        let p = A(a);
        region_op(e, &p, p.val(5)?.clone(), Some(BinOp::Mul))
    });
    r.f("ds_grid_set_disk", |e, _, a| {
        // ds_grid_set_disk(grid, xm, ym, r, value): cells whose centre is within r of (xm, ym)
        let p = A(a);
        let (xm, ym, rad, v) = (p.num(1)?, p.num(2)?, p.num(3)?, p.val(4)?.clone());
        let g = grid_mut(e, &p, 0)?;
        for y in (ym - rad).floor().max(0.0) as i64..=((ym + rad).ceil() as i64).min(g.h as i64 - 1) {
            for x in (xm - rad).floor().max(0.0) as i64..=((xm + rad).ceil() as i64).min(g.w as i64 - 1) {
                if (x as f64 - xm).powi(2) + (y as f64 - ym).powi(2) <= rad * rad {
                    g.set(x, y, v.clone());
                }
            }
        }
        Ok(Value::Null)
    });
    r.f("ds_grid_resize", |e, _, a| {
        let p = A(a);
        let (w, h) = size(&p, 1, "ds_grid_resize")?;
        grid_mut(e, &p, 0)?.resize(w, h);
        Ok(Value::Null)
    });
    r.f("ds_grid_copy", |e, _, a| {
        // ds_grid_copy(dest, source): dest becomes a copy (same size)
        let p = A(a);
        let src = grid(e, &p, 1)?.clone();
        *grid_mut(e, &p, 0)? = src;
        Ok(Value::Null)
    });
    r.f("ds_grid_set_grid_region", |e, _, a| {
        // ds_grid_set_grid_region(dest, source, x1, y1, x2, y2, xpos, ypos)
        let p = A(a);
        let src = grid(e, &p, 1)?;
        let Some((x1, y1, x2, y2)) = src.region(p.int(2)?, p.int(3)?, p.int(4)?, p.int(5)?) else { return Ok(Value::Null) };
        let part: Vec<(i64, i64, Value)> = (y1..=y2).flat_map(|y| (x1..=x2).map(move |x| (x, y))).map(|(x, y)| ((x - x1) as i64, (y - y1) as i64, src.cells[y * src.w + x].clone())).collect();
        let (dx, dy) = (p.int(6)?, p.int(7)?);
        let dst = grid_mut(e, &p, 0)?;
        for (x, y, v) in part {
            dst.set(dx + x, dy + y, v);
        }
        Ok(Value::Null)
    });
    r.f("ds_grid_get_max", |e, _, a| Ok(Value::Num(region_nums(e, &A(a))?.into_iter().fold(f64::NAN, f64::max)).nan_to_null()));
    r.f("ds_grid_get_min", |e, _, a| Ok(Value::Num(region_nums(e, &A(a))?.into_iter().fold(f64::NAN, f64::min)).nan_to_null()));
    r.f("ds_grid_get_sum", |e, _, a| Ok(Value::Num(region_nums(e, &A(a))?.into_iter().sum())));
    r.f("ds_grid_get_mean", |e, _, a| {
        let v = region_nums(e, &A(a))?;
        Ok(if v.is_empty() { Value::Num(0.0) } else { Value::Num(v.iter().sum::<f64>() / v.len() as f64) })
    });
    r.f("ds_grid_value_exists", |e, _, a| Ok(Value::Bool(find(e, &A(a))?.is_some())));
    r.f("ds_grid_value_x", |e, _, a| Ok(Value::Num(find(e, &A(a))?.map(|c| c.0 as f64).unwrap_or(-1.0))));
    r.f("ds_grid_value_y", |e, _, a| Ok(Value::Num(find(e, &A(a))?.map(|c| c.1 as f64).unwrap_or(-1.0))));
    r.f("ds_grid_count", |e, _, a| {
        // ds_grid_count(grid, value) -> how many cells hold value
        let p = A(a);
        let v = p.val(1)?;
        Ok(Value::Num(grid(e, &p, 0)?.cells.iter().filter(|c| same(c, v)).count() as f64))
    });
    r.f("ds_grid_shuffle", |e, _, a| {
        // ds_grid_shuffle(grid, [rng]): default stream, or an rng_create() stream
        let p = A(a);
        grid(e, &p, 0)?;
        let mut st = match p.opt(1) {
            Some(_) => {
                *stream(e, p.num(1)?)?
            }
            None => e.rng,
        };
        let g = grid_mut(e, &p, 0)?;
        for i in (1..g.cells.len()).rev() {
            let j = ((rand01(&mut st) * (i + 1) as f64) as usize).min(i);
            g.cells.swap(i, j);
        }
        match p.opt(1) {
            Some(_) => *stream(e, p.num(1)?)? = st,
            None => e.rng = st,
        }
        Ok(Value::Null)
    });
    r.f("ds_grid_sort", |e, _, a| {
        // ds_grid_sort(grid, column, ascending): reorders the rows by that column
        let p = A(a);
        let (col, asc) = (p.int(1)?, p.bool_or(2, true));
        let g = grid_mut(e, &p, 0)?;
        if col < 0 || col as usize >= g.w {
            return Ok(Value::Null);
        }
        let w = g.w;
        let mut rows: Vec<Vec<Value>> = g.cells.chunks(w.max(1)).map(|c| c.to_vec()).collect();
        let key = |r: &Vec<Value>| r[col as usize].clone();
        rows.sort_by(|ra, rb| {
            let (ka, kb) = (key(ra), key(rb));
            let o = match (&ka, &kb) {
                (Value::Num(x), Value::Num(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
                (Value::Num(_), _) => std::cmp::Ordering::Less,
                (_, Value::Num(_)) => std::cmp::Ordering::Greater,
                _ => ka.to_string().cmp(&kb.to_string()),
            };
            if asc {
                o
            } else {
                o.reverse()
            }
        });
        g.cells = rows.into_iter().flatten().collect();
        Ok(Value::Null)
    });
    r.f("ds_grid_to_array", |e, _, a| {
        // rows: result[y][x]
        let g = grid(e, &A(a), 0)?;
        Ok(Value::array((0..g.h).map(|y| Value::array(g.cells[y * g.w..(y + 1) * g.w].to_vec())).collect()))
    });
    r.f("ds_grid_from_array", |e, _, a| {
        // ds_grid_from_array(rows, [grid]) -> grid id (fills `grid`, resized, or a new one)
        let p = A(a);
        let g = grid_from_rows(p.val(0)?)?;
        match p.opt(1) {
            Some(_) => {
                *grid_mut(e, &p, 1)? = g;
                Ok(Value::Num(p.num(1)?))
            }
            None => Ok(Value::Num(e.ds_grids.add(g) as f64)),
        }
    });
    r.f("ds_grid_write", |e, _, a| {
        let j = grid(e, &A(a), 0)?.to_json().map_err(|m| RtError::new(format!("ds_grid_write: {m}")))?;
        Ok(Value::string(j.to_string()))
    });
    r.f("ds_grid_read", |e, _, a| {
        // ds_grid_read(grid, text): the grid takes the size and cells written by ds_grid_write
        let p = A(a);
        let t = p.str(1)?;
        let j: serde_json::Value = serde_json::from_str(&t).map_err(|_| RtError::new("ds_grid_read: el texto no es de ds_grid_write()"))?;
        let g = DsGrid::from_json(&j).map_err(|m| RtError::new(format!("ds_grid_read: {m}")))?;
        *grid_mut(e, &p, 0)? = g;
        Ok(Value::Null)
    });
    r.f("mp_grid_add_grid", |e, _, a| {
        // mp_grid_add_grid(mp_grid, ds_grid, [values=[1]], [x, y]) -> cells blocked: every cell
        // of the ds_grid holding one of `values` blocks the mp_grid cell (offset by x, y cells)
        let p = A(a);
        let blocked: Vec<Value> = match p.opt(2) {
            Some(Value::Array(v)) => v.borrow().clone(),
            Some(v) => vec![v.clone()],
            None => vec![Value::Num(1.0)],
        };
        let (ox, oy) = (p.num_or(3, 0.0)? as i64, p.num_or(4, 0.0)? as i64);
        let g = grid(e, &p, 1)?.clone();
        super::navgrid::grid_ref(e, p.num(0)?)?;
        let id = p.num(0)? as usize;
        let mp = e.nav.grids[id].as_mut().expect("checked");
        let mut n = 0;
        for y in 0..g.h {
            for x in 0..g.w {
                let (cx, cy) = (x as i64 + ox, y as i64 + oy);
                if mp.in_bounds(cx, cy) && blocked.iter().any(|b| same(&g.cells[y * g.w + x], b)) {
                    mp.set(cx, cy, true);
                    n += 1;
                }
            }
        }
        Ok(Value::Num(n as f64))
    });
}

trait NanNull {
    fn nan_to_null(self) -> Value;
}

impl NanNull for Value {
    fn nan_to_null(self) -> Value {
        match self {
            Value::Num(n) if n.is_nan() => Value::Num(0.0),
            v => v,
        }
    }
}

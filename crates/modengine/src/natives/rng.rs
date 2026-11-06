//! Separate random number streams: `rng_create(seed)` and `rng_*`.
//!
//! The default stream (`rand`, `irand`, `choose`, `shuffle`, `seed()`) is shared by the
//! whole game, so any extra call (a particle, a sound variation) shifts every later
//! number. A stream is an independent generator with its own state: use one for level
//! generation and another (or the default) for gameplay.

use super::{err, rand01, A, Reg};
use crate::engine::Engine;
use modscript::{RtResult, Value};

#[derive(Default)]
pub struct Streams {
    list: Vec<Option<u64>>,
}

/// splitmix64: spreads any seed (0, 1, 2...) over the whole state space.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) | 1
}

/// A seed from a number or a text (`rng_create("mazmorra")`).
fn seed_of(v: &Value) -> RtResult<u64> {
    match v {
        Value::Num(n) => Ok(mix(n.round() as i64 as u64)),
        Value::Str(s) => {
            // FNV-1a
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in s.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            Ok(mix(h))
        }
        o => err(format!("la semilla debe ser un número o un texto, no {}", o.type_name())),
    }
}

fn stream<'a>(e: &'a mut Engine, a: &A) -> RtResult<&'a mut u64> {
    let i = a.num(0)?;
    match e.rngs.list.get_mut(i.max(0.0) as usize) {
        Some(Some(s)) if i >= 0.0 => Ok(s),
        _ => err(format!("{} no es un generador de rng_create() (¿ya destruido?)", Value::Num(i))),
    }
}

fn r01(e: &mut Engine, a: &A) -> RtResult<f64> {
    Ok(rand01(stream(e, a)?))
}

fn irange(e: &mut Engine, a: &A, lo: i64, hi: i64) -> RtResult<Value> {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    let r = r01(e, a)?;
    Ok(Value::Num((lo + (r * (hi - lo + 1) as f64) as i64).min(hi) as f64))
}

pub(crate) fn register(r: &mut Reg) {
    r.f("rng_create", |e, _, a| {
        let state = match A(a).opt(0) {
            Some(v) => seed_of(v)?,
            // without a seed: drawn from the default stream, so `seed(n)` / --seed fix it too
            None => mix((rand01(&mut e.rng) * (1u64 << 53) as f64) as u64),
        };
        let i = match e.rngs.list.iter().position(|s| s.is_none()) {
            Some(i) => {
                e.rngs.list[i] = Some(state);
                i
            }
            None => {
                e.rngs.list.push(Some(state));
                e.rngs.list.len() - 1
            }
        };
        Ok(Value::Num(i as f64))
    });
    r.f("rng_destroy", |e, _, a| {
        let i = A(a).num(0)?.max(0.0) as usize;
        if let Some(s) = e.rngs.list.get_mut(i) {
            *s = None;
        }
        Ok(Value::Null)
    });
    r.f("rng_exists", |e, _, a| {
        let i = A(a).num(0)?;
        Ok(Value::Bool(i >= 0.0 && matches!(e.rngs.list.get(i as usize), Some(Some(_)))))
    });
    r.f("rng_seed", |e, _, a| {
        let s = seed_of(A(a).val(1)?)?;
        *stream(e, &A(a))? = s;
        Ok(Value::Null)
    });
    // the state as text (an exact u64 does not fit in a number): save it and restore it
    // with rng_set_state to continue the same sequence after loading a game
    r.f("rng_get_state", |e, _, a| Ok(Value::string(format!("{:016x}", *stream(e, &A(a))?))));
    r.f("rng_set_state", |e, _, a| {
        let t = A(a).text(1)?;
        let v = u64::from_str_radix(t.trim(), 16).map_err(|_| modscript::RtError::new(format!("rng_set_state: '{t}' no es un estado de rng_get_state()")))?;
        *stream(e, &A(a))? = if v == 0 { 1 } else { v };
        Ok(Value::Null)
    });
    // GameMaker-style names with the stream first
    r.f("rng_random", |e, _, a| {
        let max = A(a).num_or(1, 1.0)?;
        Ok(Value::Num(r01(e, &A(a))? * max))
    });
    r.f("rng_random_range", |e, _, a| {
        let (lo, hi) = (A(a).num(1)?, A(a).num(2)?);
        Ok(Value::Num(lo + r01(e, &A(a))? * (hi - lo)))
    });
    r.f("rng_irandom", |e, _, a| {
        let n = A(a).int(1)?;
        irange(e, &A(a), 0, n)
    });
    r.f("rng_irandom_range", |e, _, a| {
        let (lo, hi) = (A(a).int(1)?, A(a).int(2)?);
        irange(e, &A(a), lo, hi)
    });
    r.f("rng_chance", |e, _, a| {
        let p = A(a).num(1)?;
        Ok(Value::Bool(r01(e, &A(a))? < p))
    });
    r.f("rng_choose", |e, _, a| {
        // rng_choose(r, a, b, c) or rng_choose(r, array)
        let items: Vec<Value> = match (a.len(), a.get(1)) {
            (2, Some(Value::Array(arr))) => arr.borrow().clone(),
            _ => a.get(1..).map(|s| s.to_vec()).unwrap_or_default(),
        };
        let r = r01(e, &A(a))?;
        if items.is_empty() {
            return Ok(Value::Null);
        }
        Ok(items[((r * items.len() as f64) as usize).min(items.len() - 1)].clone())
    });
    r.f("rng_shuffle", |e, _, a| {
        let arr = A(a).array(1)?;
        let st = stream(e, &A(a))?;
        let mut v = arr.borrow_mut();
        for i in (1..v.len()).rev() {
            let j = (rand01(st) * (i + 1) as f64) as usize;
            v.swap(i, j.min(i));
        }
        Ok(Value::Null)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_are_spread_and_never_zero() {
        assert_ne!(mix(0), 0);
        assert_ne!(mix(1), mix(2));
        let (mut a, mut b) = (mix(1), mix(2));
        let (x, y) = (rand01(&mut a), rand01(&mut b));
        assert!((x - y).abs() > 1e-6);
    }
}

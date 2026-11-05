//! Math, strings, arrays, structs, types and debugging helpers.

use super::{rand01, Reg, A};

use modscript::{call_value, intern, RtError, RtResult, Value};

fn err<T>(msg: impl Into<String>) -> RtResult<T> {
    Err(RtError::new(msg))
}

fn n(v: f64) -> RtResult<Value> {
    Ok(Value::Num(v))
}

pub(super) fn register(r: &mut Reg) {
    // ---------------------------------------------------------------- math
    r.f("abs", |_, _, a| n(A(a).num(0)?.abs()));
    r.f("sign", |_, _, a| {
        let v = A(a).num(0)?;
        n(if v > 0.0 { 1.0 } else if v < 0.0 { -1.0 } else { 0.0 })
    });
    r.f("round", |_, _, a| n(A(a).num(0)?.round()));
    r.f("floor", |_, _, a| n(A(a).num(0)?.floor()));
    r.f("ceil", |_, _, a| n(A(a).num(0)?.ceil()));
    r.f("frac", |_, _, a| n(A(a).num(0)?.fract()));
    r.f("sqrt", |_, _, a| n(A(a).num(0)?.sqrt()));
    r.f("sqr", |_, _, a| {
        let v = A(a).num(0)?;
        n(v * v)
    });
    r.f("pow", |_, _, a| n(A(a).num(0)?.powf(A(a).num(1)?)));
    r.f("exp", |_, _, a| n(A(a).num(0)?.exp()));
    r.f("ln", |_, _, a| n(A(a).num(0)?.ln()));
    r.f("log2", |_, _, a| n(A(a).num(0)?.log2()));
    r.f("log10", |_, _, a| n(A(a).num(0)?.log10()));
    r.f("sin", |_, _, a| n(A(a).num(0)?.sin()));
    r.f("cos", |_, _, a| n(A(a).num(0)?.cos()));
    r.f("tan", |_, _, a| n(A(a).num(0)?.tan()));
    r.f("asin", |_, _, a| n(A(a).num(0)?.asin()));
    r.f("acos", |_, _, a| n(A(a).num(0)?.acos()));
    r.f("atan", |_, _, a| n(A(a).num(0)?.atan()));
    r.f("atan2", |_, _, a| n(A(a).num(0)?.atan2(A(a).num(1)?)));
    r.f("dsin", |_, _, a| n(A(a).num(0)?.to_radians().sin()));
    r.f("dcos", |_, _, a| n(A(a).num(0)?.to_radians().cos()));
    r.f("deg", |_, _, a| n(A(a).num(0)?.to_degrees()));
    r.f("rad", |_, _, a| n(A(a).num(0)?.to_radians()));
    r.f("min", |_, _, a| {
        let mut m = f64::INFINITY;
        for i in 0..a.len() {
            m = m.min(A(a).num(i)?);
        }
        n(m)
    });
    r.f("max", |_, _, a| {
        let mut m = f64::NEG_INFINITY;
        for i in 0..a.len() {
            m = m.max(A(a).num(i)?);
        }
        n(m)
    });
    r.f("clamp", |_, _, a| {
        let (v, lo, hi) = (A(a).num(0)?, A(a).num(1)?, A(a).num(2)?);
        n(v.max(lo).min(hi))
    });
    r.f("lerp", |_, _, a| {
        let (x, y, t) = (A(a).num(0)?, A(a).num(1)?, A(a).num(2)?);
        n(x + (y - x) * t)
    });
    r.f("approach", |_, _, a| {
        let (v, target, amount) = (A(a).num(0)?, A(a).num(1)?, A(a).num(2)?.abs());
        n(if v < target { (v + amount).min(target) } else { (v - amount).max(target) })
    });
    r.f("wrap", |_, _, a| {
        let (v, lo, hi) = (A(a).num(0)?, A(a).num(1)?, A(a).num(2)?);
        if hi <= lo {
            return n(lo);
        }
        n(lo + (v - lo).rem_euclid(hi - lo))
    });
    r.f("dist", |_, _, a| {
        let (x1, y1, x2, y2) = (A(a).num(0)?, A(a).num(1)?, A(a).num(2)?, A(a).num(3)?);
        n((x2 - x1).hypot(y2 - y1))
    });
    r.f("angle_to", |_, _, a| {
        let (x1, y1, x2, y2) = (A(a).num(0)?, A(a).num(1)?, A(a).num(2)?, A(a).num(3)?);
        n((-(y2 - y1)).atan2(x2 - x1).to_degrees().rem_euclid(360.0))
    });
    r.f("len_x", |_, _, a| n(A(a).num(0)? * A(a).num(1)?.to_radians().cos()));
    r.f("len_y", |_, _, a| n(-A(a).num(0)? * A(a).num(1)?.to_radians().sin()));
    r.f("angle_diff", |_, _, a| {
        let d = (A(a).num(1)? - A(a).num(0)?).rem_euclid(360.0);
        n(if d > 180.0 { d - 360.0 } else { d })
    });

    // ---------------------------------------------------------------- random
    r.f("rand", |e, _, a| {
        let max = A(a).num_or(0, 1.0)?;
        n(rand01(&mut e.rng) * max)
    });
    r.f("rand_range", |e, _, a| {
        let (lo, hi) = (A(a).num(0)?, A(a).num(1)?);
        n(lo + rand01(&mut e.rng) * (hi - lo))
    });
    r.f("irand", |e, _, a| {
        let (lo, hi) = (A(a).int(0)?, A(a).int(1)?);
        let (lo, hi) = (lo.min(hi), lo.max(hi));
        n((lo + (rand01(&mut e.rng) * (hi - lo + 1) as f64) as i64).min(hi) as f64)
    });
    r.f("chance", |e, _, a| Ok(Value::Bool(rand01(&mut e.rng) < A(a).num(0)?)));
    r.f("choose", |e, _, a| {
        if a.is_empty() {
            return Ok(Value::Null);
        }
        let i = ((rand01(&mut e.rng) * a.len() as f64) as usize).min(a.len() - 1);
        Ok(a[i].clone())
    });
    r.f("seed", |e, _, a| {
        e.rng = (A(a).num(0)? as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        Ok(Value::Null)
    });
    r.f("randomize", |e, _, _| {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        e.rng = t | 1;
        Ok(Value::Null)
    });

    // ---------------------------------------------------------------- types
    r.f("type_of", |_, _, a| Ok(Value::str(A(a).val(0)?.type_name())));
    r.f("is_num", |_, _, a| Ok(Value::Bool(matches!(A(a).val(0)?, Value::Num(_)))));
    r.f("is_str", |_, _, a| Ok(Value::Bool(matches!(A(a).val(0)?, Value::Str(_)))));
    r.f("is_array", |_, _, a| Ok(Value::Bool(matches!(A(a).val(0)?, Value::Array(_)))));
    r.f("is_struct", |_, _, a| Ok(Value::Bool(matches!(A(a).val(0)?, Value::Struct(_)))));
    r.f("is_func", |_, _, a| Ok(Value::Bool(matches!(A(a).val(0)?, Value::Func(_) | Value::Native(_)))));
    r.f("is_inst", |_, _, a| Ok(Value::Bool(matches!(A(a).val(0)?, Value::Inst(_)))));
    r.f("is_null", |_, _, a| Ok(Value::Bool(a.first().map(|v| v.is_null()).unwrap_or(true))));

    // ---------------------------------------------------------------- strings
    r.f("str", |_, _, a| Ok(Value::string(A(a).text(0)?)));
    r.f("num", |_, _, a| match A(a).val(0)? {
        Value::Num(v) => n(*v),
        Value::Bool(b) => n(if *b { 1.0 } else { 0.0 }),
        Value::Str(s) => match s.trim().parse::<f64>() {
            Ok(v) => n(v),
            Err(_) => Ok(Value::Null),
        },
        other => err(format!("cannot convert {} to a number", other.type_name())),
    });
    r.f("len", |_, _, a| match A(a).val(0)? {
        Value::Str(s) => n(s.chars().count() as f64),
        Value::Array(v) => n(v.borrow().len() as f64),
        Value::Struct(s) => n(s.borrow().fields.len() as f64),
        other => err(format!("len() of {}", other.type_name())),
    });
    r.f("upper", |_, _, a| Ok(Value::string(A(a).str(0)?.to_uppercase())));
    r.f("lower", |_, _, a| Ok(Value::string(A(a).str(0)?.to_lowercase())));
    r.f("trim", |_, _, a| Ok(Value::str(A(a).str(0)?.trim())));
    r.f("substr", |_, _, a| {
        let s = A(a).str(0)?;
        let start = A(a).num(1)?.max(0.0) as usize;
        let count = A(a).num_or(2, f64::INFINITY)?.max(0.0);
        let out: String = s.chars().skip(start).take(if count.is_infinite() { usize::MAX } else { count as usize }).collect();
        Ok(Value::string(out))
    });
    r.f("replace", |_, _, a| Ok(Value::string(A(a).str(0)?.replace(&*A(a).str(1)?, &A(a).text(2)?))));
    r.f("split", |_, _, a| {
        let s = A(a).str(0)?;
        let sep = A(a).str(1)?;
        let parts: Vec<Value> = if sep.is_empty() { s.chars().map(|c| Value::string(c.to_string())).collect() } else { s.split(&*sep).map(Value::str).collect() };
        Ok(Value::array(parts))
    });
    r.f("join", |_, _, a| {
        let arr = A(a).array(0)?;
        let sep = match A(a).opt(1) {
            Some(_) => A(a).text(1)?,
            None => String::new(),
        };
        let s = arr.borrow().iter().map(|v| v.to_string()).collect::<Vec<_>>().join(&sep);
        Ok(Value::string(s))
    });
    r.f("starts_with", |_, _, a| Ok(Value::Bool(A(a).str(0)?.starts_with(&*A(a).str(1)?))));
    r.f("ends_with", |_, _, a| Ok(Value::Bool(A(a).str(0)?.ends_with(&*A(a).str(1)?))));
    r.f("chr", |_, _, a| Ok(Value::string(char::from_u32(A(a).num(0)? as u32).map(|c| c.to_string()).unwrap_or_default())));
    r.f("ord", |_, _, a| n(A(a).str(0)?.chars().next().map(|c| c as u32 as f64).unwrap_or(-1.0)));
    r.f("format", |_, _, a| {
        let fmt = A(a).str(0)?;
        let mut out = String::new();
        let mut next = 1;
        let mut parts = fmt.split("{}");
        if let Some(first) = parts.next() {
            out.push_str(first);
        }
        for p in parts {
            out.push_str(&a.get(next).map(|v| v.to_string()).unwrap_or_default());
            next += 1;
            out.push_str(p);
        }
        Ok(Value::string(out))
    });
    r.f("pad", |_, _, a| {
        // pad(value, width, [fill]) — right-aligns, handy for scores: pad(score, 6, "0")
        let s = A(a).text(0)?;
        let w = A(a).num(1)?.max(0.0) as usize;
        let fill = match A(a).opt(2) {
            Some(_) => A(a).text(2)?.chars().next().unwrap_or(' '),
            None => ' ',
        };
        let len = s.chars().count();
        Ok(Value::string(if len >= w { s } else { std::iter::repeat_n(fill, w - len).chain(s.chars()).collect() }))
    });

    // ---------------------------------------------------------------- arrays
    r.f("array", |_, _, a| {
        let size = A(a).num_or(0, 0.0)?.max(0.0) as usize;
        let fill = a.get(1).cloned().unwrap_or(Value::Null);
        Ok(Value::array(vec![fill; size]))
    });
    r.f("range", |_, _, a| {
        let (lo, hi) = if a.len() >= 2 { (A(a).int(0)?, A(a).int(1)?) } else { (0, A(a).int(0)?) };
        if hi - lo > 10_000_000 {
            return err("range too large");
        }
        Ok(Value::array((lo..hi).map(|i| Value::Num(i as f64)).collect()))
    });
    r.f("push", |_, _, a| {
        let arr = A(a).array(0)?;
        arr.borrow_mut().extend(a[1..].iter().cloned());
        let len = arr.borrow().len();
        n(len as f64)
    });
    r.f("pop", |_, _, a| Ok(A(a).array(0)?.borrow_mut().pop().unwrap_or(Value::Null)));
    r.f("insert", |_, _, a| {
        let arr = A(a).array(0)?;
        let i = (A(a).num(1)?.max(0.0) as usize).min(arr.borrow().len());
        arr.borrow_mut().insert(i, A(a).val(2)?.clone());
        Ok(Value::Null)
    });
    r.f("remove", |_, _, a| {
        let arr = A(a).array(0)?;
        let i = A(a).num(1)?.max(0.0) as usize;
        let mut v = arr.borrow_mut();
        if i < v.len() {
            Ok(v.remove(i))
        } else {
            Ok(Value::Null)
        }
    });
    r.f("clear", |_, _, a| {
        match A(a).val(0)? {
            Value::Array(arr) => arr.borrow_mut().clear(),
            Value::Struct(s) => s.borrow_mut().fields.clear(),
            other => return err(format!("clear() of {}", other.type_name())),
        }
        Ok(Value::Null)
    });
    r.f("contains", |_, _, a| match A(a).val(0)? {
        Value::Str(s) => Ok(Value::Bool(s.contains(&*A(a).text(1)?))),
        Value::Array(arr) => {
            let needle = A(a).val(1)?;
            Ok(Value::Bool(arr.borrow().iter().any(|v| v.loose_eq(needle))))
        }
        other => err(format!("contains() of {}", other.type_name())),
    });
    r.f("index_of", |_, _, a| match A(a).val(0)? {
        Value::Str(s) => {
            let needle = A(a).text(1)?;
            n(s.find(&needle).map(|b| s[..b].chars().count() as f64).unwrap_or(-1.0))
        }
        Value::Array(arr) => {
            let needle = A(a).val(1)?;
            n(arr.borrow().iter().position(|v| v.loose_eq(needle)).map(|i| i as f64).unwrap_or(-1.0))
        }
        other => err(format!("index_of() of {}", other.type_name())),
    });
    r.f("first", |_, _, a| Ok(A(a).array(0)?.borrow().first().cloned().unwrap_or(Value::Null)));
    r.f("last", |_, _, a| Ok(A(a).array(0)?.borrow().last().cloned().unwrap_or(Value::Null)));
    r.f("reverse", |_, _, a| {
        A(a).array(0)?.borrow_mut().reverse();
        Ok(Value::Null)
    });
    r.f("slice", |_, _, a| {
        let arr = A(a).array(0)?;
        let v = arr.borrow();
        let start = (A(a).num_or(1, 0.0)?.max(0.0) as usize).min(v.len());
        let end = (A(a).num_or(2, v.len() as f64)?.max(0.0) as usize).clamp(start, v.len());
        Ok(Value::array(v[start..end].to_vec()))
    });
    r.f("copy", |_, _, a| {
        Ok(match A(a).val(0)? {
            Value::Array(arr) => Value::array(arr.borrow().clone()),
            Value::Struct(s) => Value::new_struct(s.borrow().fields.iter().map(|(k, v)| (*k, v.clone()))),
            other => other.clone(),
        })
    });
    r.f("sum", |_, _, a| {
        let mut t = 0.0;
        for v in A(a).array(0)?.borrow().iter() {
            t += v.as_num().ok_or_else(|| RtError::new("sum() needs an array of numbers"))?;
        }
        n(t)
    });
    r.f("shuffle", |e, _, a| {
        let arr = A(a).array(0)?;
        let mut v = arr.borrow_mut();
        for i in (1..v.len()).rev() {
            let j = (rand01(&mut e.rng) * (i + 1) as f64) as usize;
            v.swap(i, j.min(i));
        }
        Ok(Value::Null)
    });
    r.f("sort", |e, cx, a| {
        let arr = A(a).array(0)?;
        let mut items = arr.borrow().clone();
        match A(a).opt(1) {
            None => {
                let mut bad = false;
                items.sort_by(|x, y| match (x, y) {
                    (Value::Num(p), Value::Num(q)) => p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal),
                    (Value::Str(p), Value::Str(q)) => p.cmp(q),
                    _ => {
                        bad = true;
                        std::cmp::Ordering::Equal
                    }
                });
                if bad {
                    return err("sort() without a compare function needs all numbers or all strings");
                }
            }
            Some(f) => {
                // insertion sort so the compare function may run script code safely
                let f = f.clone();
                for i in 1..items.len() {
                    let mut j = i;
                    while j > 0 {
                        let c = call_value(e, &f, cx, vec![items[j - 1].clone(), items[j].clone()])?;
                        if c.as_num().unwrap_or(0.0) > 0.0 {
                            items.swap(j - 1, j);
                            j -= 1;
                        } else {
                            break;
                        }
                    }
                }
            }
        }
        *arr.borrow_mut() = items;
        Ok(Value::Null)
    });
    r.f("map", |e, cx, a| {
        let items = A(a).array(0)?.borrow().clone();
        let f = A(a).val(1)?.clone();
        let mut out = Vec::with_capacity(items.len());
        for (i, v) in items.into_iter().enumerate() {
            out.push(call_value(e, &f, cx, vec![v, Value::Num(i as f64)])?);
        }
        Ok(Value::array(out))
    });
    r.f("filter", |e, cx, a| {
        let items = A(a).array(0)?.borrow().clone();
        let f = A(a).val(1)?.clone();
        let mut out = Vec::new();
        for (i, v) in items.into_iter().enumerate() {
            if call_value(e, &f, cx, vec![v.clone(), Value::Num(i as f64)])?.truthy() {
                out.push(v);
            }
        }
        Ok(Value::array(out))
    });

    // ---------------------------------------------------------------- structs
    r.f("keys", |_, _, a| Ok(Value::array(A(a).strukt(0)?.borrow().fields.keys().map(|k| Value::str(&k.name())).collect())));
    r.f("has", |_, _, a| Ok(Value::Bool(A(a).strukt(0)?.borrow().fields.contains_key(&intern(&A(a).text(1)?)))));
    r.f("get", |_, _, a| {
        let s = A(a).strukt(0)?;
        let v = s.borrow().fields.get(&intern(&A(a).text(1)?)).cloned();
        Ok(v.unwrap_or_else(|| a.get(2).cloned().unwrap_or(Value::Null)))
    });
    r.f("set", |_, _, a| {
        A(a).strukt(0)?.borrow_mut().fields.insert(intern(&A(a).text(1)?), A(a).val(2)?.clone());
        Ok(Value::Null)
    });
    r.f("remove_key", |_, _, a| Ok(A(a).strukt(0)?.borrow_mut().fields.shift_remove(&intern(&A(a).text(1)?)).unwrap_or(Value::Null)));

    // ---------------------------------------------------------------- debug & misc
    r.f("print", |_, _, a| {
        println!("{}", a.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" "));
        Ok(Value::Null)
    });
    r.f("error", |_, _, a| err(A(a).text(0)?));
    r.f("call", |e, cx, a| {
        let f = A(a).val(0)?.clone();
        call_value(e, &f, cx, a[1..].to_vec())
    });
    r.f("to_json", |_, _, a| Ok(Value::string(serde_json::to_string(&to_json(A(a).val(0)?)?).unwrap_or_default())));
    r.f("from_json", |_, _, a| {
        let j: serde_json::Value = serde_json::from_str(&A(a).str(0)?).map_err(|e| RtError::new(format!("bad JSON: {e}")))?;
        Ok(from_json(&j))
    });
}

pub(crate) fn to_json(v: &Value) -> RtResult<serde_json::Value> {
    Ok(match v {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Num(x) if x.fract() == 0.0 && x.abs() < 9e15 => serde_json::Value::Number((*x as i64).into()),
        Value::Num(x) => serde_json::Number::from_f64(*x).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
        Value::Str(s) => serde_json::Value::String(s.to_string()),
        Value::Array(a) => serde_json::Value::Array(a.borrow().iter().map(to_json).collect::<RtResult<_>>()?),
        Value::Struct(s) => {
            let mut m = serde_json::Map::new();
            for (k, v) in s.borrow().fields.iter() {
                m.insert(k.name().to_string(), to_json(v)?);
            }
            serde_json::Value::Object(m)
        }
        other => return err(format!("cannot save a {} as data", other.type_name())),
    })
}

pub(crate) fn from_json(j: &serde_json::Value) -> Value {
    match j {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(x) => Value::Num(x.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::str(s),
        serde_json::Value::Array(a) => Value::array(a.iter().map(from_json).collect()),
        serde_json::Value::Object(m) => Value::new_struct(m.iter().map(|(k, v)| (intern(k), from_json(v)))),
    }
}


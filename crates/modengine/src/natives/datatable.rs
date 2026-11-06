//! data_get / data_ids / data_find ...: read-only access to the data tables of `data/`
//! (see `crate::datatable`), plus `data_dispatch` for lists of composable effects.

use super::{err, A, Reg};
use crate::datatable::{deep_copy, Table};
use crate::engine::Engine;
use modscript::{call_value, intern, RtResult, Value};

fn table<'a>(e: &'a Engine, name: &str) -> RtResult<&'a Table> {
    e.data.table(name).ok_or_else(|| {
        let names: Vec<&str> = e.data.tables.iter().map(|t| t.name.as_str()).collect();
        let hint = crate::datatable::suggest_pub(name, names.iter().copied());
        let list = if names.is_empty() { "el proyecto no tiene tablas: crea data/<tabla>.toml".to_string() } else { format!("tablas: {}", names.join(", ")) };
        modscript::RtError::new(format!("no hay ninguna tabla '{name}'{hint} ({list})"))
    })
}

fn record<'a>(t: &'a Table, id: &str) -> RtResult<&'a Value> {
    match t.index.get(id) {
        Some(i) => Ok(&t.values[*i]),
        None => err(format!("'{id}' no es un id de la tabla '{}'{} ({})", t.name, crate::datatable::suggest_pub(id, t.ids.iter().map(|s| s.as_str())), t.file)),
    }
}

/// The record id given as text or number.
fn id_arg(a: &A, i: usize) -> RtResult<String> {
    match a.val(i)? {
        Value::Str(s) => Ok(s.to_string()),
        Value::Num(n) => Ok(modscript::fmt_num(*n)),
        o => err(format!("argument #{} must be a record id (string), got {}", i + 1, o.type_name())),
    }
}

fn field_of(v: &Value, field: &str) -> Value {
    match v {
        Value::Struct(s) => s.borrow().fields.get(&intern(field)).cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

pub(crate) fn register(r: &mut Reg) {
    // data_get(table, id, [field]): a copy of the record (or of one field)
    r.f("data_get", |e, _, a| {
        let p = A(a);
        let t = table(e, &p.text(0)?)?;
        let rec = record(t, &id_arg(&p, 1)?)?;
        match p.opt(2) {
            Some(f) => {
                let f = f.to_string();
                let v = field_of(rec, &f);
                if v.is_null() && !matches!(rec, Value::Struct(s) if s.borrow().fields.contains_key(&intern(&f))) {
                    return err(format!("el registro '{}' de '{}' no tiene el campo '{f}'", id_arg(&p, 1)?, t.name));
                }
                Ok(deep_copy(&v))
            }
            None => Ok(deep_copy(rec)),
        }
    });
    r.f("data_exists", |e, _, a| {
        let p = A(a);
        let Some(t) = e.data.table(&p.text(0)?) else { return Ok(Value::Bool(false)) };
        Ok(Value::Bool(match p.opt(1) {
            Some(_) => t.index.contains_key(&id_arg(&p, 1)?),
            None => true,
        }))
    });
    r.f("data_ids", |e, _, a| Ok(Value::array(table(e, &A(a).text(0)?)?.ids.iter().map(|s| Value::str(s)).collect())));
    r.f("data_all", |e, _, a| Ok(Value::array(table(e, &A(a).text(0)?)?.values.iter().map(deep_copy).collect())));
    r.f("data_count", |e, _, a| Ok(Value::Num(table(e, &A(a).text(0)?)?.values.len() as f64)));
    r.f("data_tables", |e, _, _| Ok(Value::array(e.data.tables.iter().map(|t| Value::str(&t.name)).collect())));
    // data_find(table, field, value) or data_find(table, fn(record) -> bool): copies of the matches
    r.f("data_find", |e, cx, a| {
        let p = A(a);
        let t = table(e, &p.text(0)?)?;
        let values: Vec<Value> = t.values.clone();
        match p.val(1)? {
            f @ (Value::Func(_) | Value::Native(_)) => {
                let f = f.clone();
                let mut out = vec![];
                for v in values {
                    let c = deep_copy(&v);
                    if call_value(e, &f, cx, vec![c.clone()])?.truthy() {
                        out.push(c);
                    }
                }
                Ok(Value::array(out))
            }
            field => {
                let field = field.to_string();
                let want = p.val(2)?.clone();
                Ok(Value::array(values.iter().filter(|v| field_of(v, &field).loose_eq(&want)).map(deep_copy).collect()))
            }
        }
    });
    r.f("data_reload", |e, _, _| match crate::datatable::reload(e) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(msg) => {
            e.report(modscript::RtError::new(format!("data_reload(): se siguen usando los datos anteriores:\n{msg}")));
            Ok(Value::Bool(false))
        }
    });
    r.f("data_version", |e, _, _| Ok(Value::Num(e.data.version as f64)));
    // data_dispatch(effect, handlers, args...): handlers[effect.type](effect, args...)
    // An array of effects runs each one in order and returns the array of results.
    r.f("data_dispatch", |e, cx, a| {
        let p = A(a);
        let handlers = p.strukt(1)?;
        let extra: Vec<Value> = a.get(2..).map(|s| s.to_vec()).unwrap_or_default();
        let one = |e: &mut Engine, eff: &Value| -> RtResult<Value> {
            let kind = match field_of(eff, "type") {
                Value::Str(s) => s,
                Value::Null => return err(format!("el efecto {eff:?} no tiene el campo type")),
                o => return err(format!("el campo type del efecto debe ser un texto, no {}", o.type_name())),
            };
            let h = {
                let hb = handlers.borrow();
                hb.fields.get(&intern(&kind)).or_else(|| hb.fields.get(&intern("fallback"))).cloned()
            };
            let Some(h) = h else {
                let names: Vec<String> = handlers.borrow().fields.keys().map(|k| k.name().to_string()).collect();
                return err(format!("no hay handler para el efecto '{kind}'{} (handlers: {}; añade uno o un handler 'fallback')", crate::datatable::suggest_pub(&kind, names.iter().map(|s| s.as_str())), names.join(", ")));
            };
            let mut args = vec![eff.clone()];
            args.extend(extra.iter().cloned());
            call_value(e, &h, cx, args)
        };
        match p.val(0)? {
            Value::Array(list) => {
                let items: Vec<Value> = list.borrow().clone();
                let mut out = vec![];
                for it in &items {
                    out.push(one(e, it)?);
                }
                Ok(Value::array(out))
            }
            v => one(e, &v.clone()),
        }
    });
}

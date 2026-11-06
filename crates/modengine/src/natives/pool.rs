//! instance_pool_create / pool_spawn / pool_release (see `crate::pool`).

use super::{err, A, Reg};
use crate::pool;
use modscript::{intern, AssetKind, RtError, Value};

pub(crate) fn register(r: &mut Reg) {
    r.f("instance_pool_create", |e, _, a| {
        let p = A(a);
        let obj = p.asset(0, AssetKind::Object)?;
        let n = p.num(1)?.clamp(0.0, 100_000.0) as usize;
        pool::create(e, obj, n);
        Ok(Value::Null)
    });
    // pool_spawn(obj, x, y, [depth], [vars]): same arguments as spawn()
    r.f("pool_spawn", |e, cx, a| {
        let p = A(a);
        let obj = p.asset(0, AssetKind::Object)?;
        let (x, y) = (p.num(1)?, p.num(2)?);
        let depth = p.num_or(3, 0.0)?;
        let vars: Vec<_> = match p.opt(4) {
            Some(Value::Struct(s)) => s.borrow().fields.iter().map(|(k, v)| (*k, v.clone())).collect(),
            Some(o) => return err(format!("pool_spawn(): argument #5 must be a struct of variables, got {}", o.type_name())),
            None => vec![],
        };
        let id = pool::spawn(e, obj, x, y, depth, &vars, cx.self_.clone());
        Ok(Value::Inst(id))
    });
    // pool_release([inst], [run_destroy=false])
    r.f("pool_release", |e, cx, a| {
        let p = A(a);
        let id = match p.opt(0) {
            Some(Value::Inst(id)) => *id,
            Some(o) => return err(format!("pool_release(): se esperaba una instancia, no {}", o.type_name())),
            None => cx.self_.inst().ok_or_else(|| RtError::new("pool_release() sin argumentos debe llamarlo una instancia"))?,
        };
        Ok(Value::Bool(pool::release(e, id, p.bool_or(1, false))))
    });
    r.f("pool_free_count", |e, _, a| {
        let obj = A(a).asset(0, AssetKind::Object)?;
        Ok(Value::Num(e.pools.free_count(obj) as f64))
    });
    r.f("pool_stats", |e, _, _| {
        let s = e.pools.stats;
        Ok(Value::new_struct([
            (intern("reused"), Value::Num(s.reused as f64)),
            (intern("created"), Value::Num(s.created as f64)),
            (intern("released"), Value::Num(s.released as f64)),
        ]))
    });
}

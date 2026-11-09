//! Prefabs at run time: `prefab_spawn` and the group of a prefab's instances (prefab.rs).

use super::{Reg, A};
use crate::engine::Engine;
use modscript::{AssetKind, CallCx, RtError, RtResult, Value};

/// Instance given as argument `i`, or the caller.
fn inst_arg(e: &Engine, cx: &CallCx, a: &A, i: usize, f: &str) -> RtResult<Option<usize>> {
    match a.opt(i) {
        Some(Value::Inst(id)) => Ok(e.idx(*id)),
        Some(other) => Err(RtError::new(format!("{f}(): argument #{} must be an instance, got {}", i + 1, other.type_name()))),
        None => match cx.self_.inst() {
            Some(id) => Ok(e.idx(id)),
            None => Err(RtError::new(format!("{f}() without arguments must be called by an instance"))),
        },
    }
}

/// Live members of the group instance `i` belongs to (empty when it is not in one).
fn members(e: &Engine, i: usize) -> Vec<Value> {
    let (_, k_group) = crate::prefab::group_syms();
    match e.insts[i].vars.get(&k_group) {
        Some(Value::Array(a)) => a.borrow().iter().filter(|v| matches!(v, Value::Inst(id) if e.alive(*id))).cloned().collect(),
        _ => vec![],
    }
}

pub(super) fn register(r: &mut Reg) {
    r.f("prefab_spawn", |e, cx, a| {
        let p = A(a);
        let pi = p.asset(0, AssetKind::Prefab)?;
        let (x, y) = (p.num(1)?, p.num(2)?);
        let depth = p.num_or(3, 0.0)?;
        let over: Vec<(modscript::Sym, Value)> = match p.opt(4) {
            Some(Value::Struct(s)) => s.borrow().fields.iter().map(|(k, v)| (*k, v.clone())).collect(),
            Some(other) => return Err(RtError::new(format!("prefab_spawn(): argument #5 must be a struct of variables, got {}", other.type_name()))),
            None => vec![],
        };
        let list = crate::prefab::spawns(e, pi, x, y, depth, 0, &|_| Ok(over.clone())).map_err(RtError::new)?;
        let ids = crate::prefab::spawn_batch(e, &list, cx.self_.clone());
        Ok(Value::array(ids.into_iter().map(Value::Inst).collect()))
    });
    r.f("prefab_exists", |e, _, a| Ok(Value::Bool(matches!(A(a).val(0)?, Value::Asset(AssetKind::Prefab, i) if (*i as usize) < e.prefabs.len()))));
    r.f("prefab_count", |e, _, a| {
        let pi = A(a).asset(0, AssetKind::Prefab)?;
        Ok(Value::Num(e.prefabs[pi as usize].items.len() as f64))
    });
    r.f("prefab_objects", |e, _, a| {
        let pi = A(a).asset(0, AssetKind::Prefab)?;
        Ok(Value::array(e.prefabs[pi as usize].items.iter().map(|it| Value::Asset(AssetKind::Object, it.object)).collect()))
    });
    r.f("prefab_description", |e, _, a| {
        let pi = A(a).asset(0, AssetKind::Prefab)?;
        Ok(Value::string(e.prefabs[pi as usize].description.clone()))
    });
    r.f("prefab_of", |e, cx, a| {
        let Some(i) = inst_arg(e, cx, &A(a), 0, "prefab_of")? else { return Ok(Value::Null) };
        let (k_pf, _) = crate::prefab::group_syms();
        Ok(e.insts[i].vars.get(&k_pf).cloned().unwrap_or(Value::Null))
    });
    r.f("prefab_members", |e, cx, a| {
        let Some(i) = inst_arg(e, cx, &A(a), 0, "prefab_members")? else { return Ok(Value::array(vec![])) };
        Ok(Value::array(members(e, i)))
    });
    r.f("prefab_destroy", |e, cx, a| {
        let Some(i) = inst_arg(e, cx, &A(a), 0, "prefab_destroy")? else { return Ok(Value::Num(0.0)) };
        let run = A(a).bool_or(1, true);
        let mut list = members(e, i);
        if list.is_empty() {
            list.push(Value::Inst(e.insts[i].id));
        }
        let n = list.len();
        for v in list {
            if let Value::Inst(id) = v {
                e.destroy(id, run);
            }
        }
        Ok(Value::Num(n as f64))
    });
}

//! Prefabs (`prefabs/<name>.toml`): a reusable group of instances placed together, like the
//! prefabs of Unity or Godot's packed scenes. A torch with its light, a turret with two
//! guards, a platform with three coins on top...
//!
//! ```toml
//! description = "Torreta con dos escoltas"
//!
//! [[instance]]
//! object = "Turret"
//! x = 0
//! y = 0
//! vars = { fire_rate = 30 }
//!
//! [[instance]]
//! object = "pf_lamp"      # another prefab: nested, expanded at (0, -32)
//! x = 0
//! y = -32
//! ```
//!
//! Positions are relative to the prefab's origin. Rooms place a prefab wherever they place an
//! object (map legends, `instances = [...]`); `vars` given there apply to every instance of
//! the group. At run time `prefab_spawn()` creates one, and every instance of a group gets
//! `prefab` (the prefab asset) and `prefab_group` (all the instances created with it, set
//! before any `create` event runs, so members can find each other).

use crate::engine::{Engine, Spawn};
use crate::project::InstToml;
use modscript::{intern, AssetKind, Sym, Value};
use serde::Deserialize;
use std::path::PathBuf;

/// Nested prefabs deeper than this are an error (a cycle is reported before that).
pub const MAX_DEPTH: usize = 16;

/// A prefab file as written.
#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct PrefabToml {
    pub description: String,
    /// Grid of the prefab editor (only used by the editor).
    pub grid: Option<[u32; 2]>,
    #[serde(rename = "instance")]
    pub instances: Vec<InstToml>,
}

/// One plain object instance of a flattened prefab, relative to the prefab's origin.
#[derive(Clone, Debug, PartialEq)]
pub struct FlatItem {
    pub object: String,
    pub x: f64,
    pub y: f64,
    pub vars: toml::Table,
}

/// Expands prefab `name` into plain object instances: nested prefabs (an instance whose
/// `object` names a prefab) are expanded at their position, and the `vars` of the nesting
/// instance override the ones inside. `get` finds a prefab by name (None: an object).
pub fn flatten<'a>(name: &str, get: &dyn Fn(&str) -> Option<&'a PrefabToml>) -> Result<Vec<FlatItem>, String> {
    let mut out = vec![];
    let mut stack = vec![];
    walk(name, 0.0, 0.0, &toml::Table::new(), get, &mut stack, &mut out)?;
    Ok(out)
}

fn walk<'a>(name: &str, x: f64, y: f64, over: &toml::Table, get: &dyn Fn(&str) -> Option<&'a PrefabToml>, stack: &mut Vec<String>, out: &mut Vec<FlatItem>) -> Result<(), String> {
    if stack.iter().any(|s| s == name) {
        let chain: Vec<&str> = stack.iter().map(|s| s.as_str()).chain([name]).collect();
        return Err(format!("el prefab '{}' se contiene a sí mismo: {}", stack[0], chain.join(" → ")));
    }
    if stack.len() >= MAX_DEPTH {
        return Err(format!("prefabs anidados a más de {MAX_DEPTH} niveles en '{}'", stack[0]));
    }
    let p = get(name).ok_or_else(|| format!("el prefab '{name}' no existe"))?;
    stack.push(name.to_string());
    for it in &p.instances {
        let mut vars = it.vars.clone();
        for (k, v) in over {
            vars.insert(k.clone(), v.clone());
        }
        let (ix, iy) = (x + it.x, y + it.y);
        if get(&it.object).is_some() {
            walk(&it.object, ix, iy, &vars, get, stack, out)?;
        } else {
            out.push(FlatItem { object: it.object.clone(), x: ix, y: iy, vars });
        }
    }
    stack.pop();
    Ok(())
}

/// Writes a prefab file (the editor's format: comments are not kept).
pub fn write_toml(p: &PrefabToml) -> String {
    let num = |f: f64| if f.fract() == 0.0 && f.abs() < 1e15 { format!("{}", f as i64) } else { format!("{f}") };
    let q = |s: &str| toml::Value::String(s.to_string()).to_string();
    let mut s = String::new();
    if !p.description.is_empty() {
        s += &format!("description = {}\n", q(&p.description));
    }
    if let Some([w, h]) = p.grid {
        s += &format!("grid = [{w}, {h}]\n");
    }
    for i in &p.instances {
        s += &format!("\n[[instance]]\nobject = {}\nx = {}\ny = {}\n", q(&i.object), num(i.x), num(i.y));
        if !i.vars.is_empty() {
            let v: Vec<String> = i.vars.iter().map(|(k, v)| format!("{} = {v}", toml_key(k))).collect();
            s += &format!("vars = {{ {} }}\n", v.join(", "));
        }
    }
    s
}

fn toml_key(k: &str) -> String {
    if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        k.to_string()
    } else {
        toml::Value::String(k.to_string()).to_string()
    }
}

// ------------------------------------------------------------------ engine side

/// One object instance of a resolved prefab.
#[derive(Clone, Debug)]
pub struct Item {
    pub object: u32,
    pub dx: f64,
    pub dy: f64,
    pub vars: Vec<(Sym, Value)>,
}

/// A prefab ready to be spawned: its nested prefabs expanded and its objects resolved.
#[derive(Clone, Debug)]
pub struct PrefabDef {
    pub name: String,
    pub description: String,
    pub items: Vec<Item>,
}

/// Resolves every prefab of the project (after objects exist, before rooms).
pub fn resolve(e: &mut Engine, raw: &[(String, PathBuf, PrefabToml)], errors: &mut Vec<String>) {
    let get = |n: &str| raw.iter().find(|r| r.0 == n).map(|r| &r.2);
    for (name, _, t) in raw {
        let file = format!("prefabs/{name}.toml");
        let mut items = vec![];
        match flatten(name, &get) {
            Ok(flat) => {
                for f in flat {
                    let res = e.object_by_name(&f.object).and_then(|o| {
                        let vars = e.inst_vars(o, &f.vars).map_err(|m| format!("{} en {}, {}: {m}", f.object, f.x, f.y))?;
                        Ok(Item { object: o, dx: f.x, dy: f.y, vars })
                    });
                    match res {
                        Ok(it) => items.push(it),
                        Err(m) => errors.push(format!("{file}: {m}")),
                    }
                }
            }
            Err(m) => errors.push(format!("{file}: {m}")),
        }
        // pushed even when broken so asset indices stay aligned (the load fails anyway)
        e.prefabs.push(PrefabDef { name: name.clone(), description: t.description.clone(), items });
    }
}

/// The spawns of prefab `pi` placed at (x, y): `over` (room `vars`, already converted for
/// each object) replaces the prefab's own values. `serial` tells groups apart.
pub fn spawns(e: &Engine, pi: u32, x: f64, y: f64, depth: f64, serial: u32, over: &dyn Fn(u32) -> Result<Vec<(Sym, Value)>, String>) -> Result<Vec<Spawn>, String> {
    let def = &e.prefabs[pi as usize];
    let mut out = Vec::with_capacity(def.items.len());
    for it in &def.items {
        let mut vars = it.vars.clone();
        for (k, v) in over(it.object)? {
            match vars.iter_mut().find(|(s, _)| *s == k) {
                Some(slot) => slot.1 = v,
                None => vars.push((k, v)),
            }
        }
        out.push(Spawn { object: it.object, x: x + it.dx, y: y + it.dy, depth, vars, cell: None, prefab: Some((pi, serial)) });
    }
    Ok(out)
}

/// Instance variable names set on every member of a prefab group.
pub fn group_syms() -> (Sym, Sym) {
    (intern("prefab"), intern("prefab_group"))
}

/// Creates the instances of `list` (one prefab group, or a single plain spawn) and then
/// initializes them in order, so every member exists when the `create` events run.
pub fn spawn_batch(e: &mut Engine, list: &[Spawn], creator: modscript::Scope) -> Vec<u32> {
    let ids: Vec<u32> = list.iter().map(|s| e.create_instance(s.object, s.x, s.y, s.depth)).collect();
    if let Some((pi, _)) = list.first().and_then(|s| s.prefab) {
        let (k_pf, k_group) = group_syms();
        let group = Value::array(ids.iter().map(|&i| Value::Inst(i)).collect());
        for &id in &ids {
            if let Some(i) = e.idx(id) {
                e.insts[i].vars.insert(k_pf, Value::Asset(AssetKind::Prefab, pi));
                e.insts[i].vars.insert(k_group, group.clone());
            }
        }
    }
    for (s, &id) in list.iter().zip(&ids) {
        e.init_instance(id, &s.vars, s.cell, creator.clone());
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pf(text: &str) -> PrefabToml {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn nested_prefabs_flatten_with_offsets_and_overrides() {
        let lamp = pf("[[instance]]\nobject = \"Lamp\"\nx = 0\ny = -8\nvars = { power = 1, color = 2 }\n");
        let post = pf("[[instance]]\nobject = \"Post\"\nx = 0\ny = 0\n[[instance]]\nobject = \"lamp\"\nx = 4\ny = -16\nvars = { power = 5 }\n");
        let get = |n: &str| match n {
            "lamp" => Some(&lamp),
            "post" => Some(&post),
            _ => None,
        };
        let flat = flatten("post", &get).unwrap();
        assert_eq!(flat.len(), 2);
        assert_eq!((flat[0].object.as_str(), flat[0].x, flat[0].y), ("Post", 0.0, 0.0));
        assert_eq!((flat[1].object.as_str(), flat[1].x, flat[1].y), ("Lamp", 4.0, -24.0));
        assert_eq!(flat[1].vars.get("power").and_then(|v| v.as_integer()), Some(5), "the nesting instance wins");
        assert_eq!(flat[1].vars.get("color").and_then(|v| v.as_integer()), Some(2));
    }

    #[test]
    fn cycles_are_reported_with_the_chain() {
        let a = pf("[[instance]]\nobject = \"b\"\nx = 0\ny = 0\n");
        let b = pf("[[instance]]\nobject = \"a\"\nx = 0\ny = 0\n");
        let get = |n: &str| match n {
            "a" => Some(&a),
            "b" => Some(&b),
            _ => None,
        };
        let err = flatten("a", &get).unwrap_err();
        assert!(err.contains("a → b → a"), "{err}");
    }

    #[test]
    fn write_and_read_back() {
        let p = pf("description = \"Torre\"\ngrid = [8, 8]\n[[instance]]\nobject = \"T\"\nx = 1.5\ny = -2\nvars = { hp = 3, name = \"x\" }\n");
        let text = write_toml(&p);
        assert_eq!(pf(&text), p, "{text}");
    }
}

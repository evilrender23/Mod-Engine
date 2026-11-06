//! save_data / load_data: atomic JSON saves with a backup copy, and defaults merged in.
//!
//! save_data writes `<name>.json.tmp`, flushes it to disk, copies the previous good
//! file to `<name>.json.bak` and only then renames the new file over `<name>.json`
//! (a rename is atomic): a crash or a full disk never leaves a half-written save.
//! load_data falls back to the `.bak` copy when the main file is missing or corrupt,
//! and fills keys missing from the saved structs with the given defaults.

use super::core::{from_json, to_json};
use super::{err, A, Reg};
use modscript::{RtResult, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn safe_name(s: &str) -> RtResult<String> {
    if s.is_empty() || !s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return err(format!("save name '{s}' may only contain letters, digits, '_' and '-'"));
    }
    Ok(s.to_string())
}

fn paths(dir: &Path, name: &str) -> (PathBuf, PathBuf, PathBuf) {
    (dir.join(format!("{name}.json")), dir.join(format!("{name}.json.bak")), dir.join(format!("{name}.json.tmp")))
}

fn parse(path: &Path) -> Result<serde_json::Value, String> {
    let t = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&t).map_err(|e| e.to_string())
}

/// Writes `text` to `<name>.json` atomically, keeping the previous version as `.bak`.
pub fn write_atomic(dir: &Path, name: &str, text: &str) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let (main, bak, tmp) = paths(dir, name);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    // keep the last good version (a corrupt main file never replaces a good backup)
    if parse(&main).is_ok() {
        fs::copy(&main, &bak)?;
        if let Ok(f) = fs::File::open(&bak) {
            let _ = f.sync_all();
        }
    }
    fs::rename(&tmp, &main)?;
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        // make the rename itself durable
        let _ = d.sync_all();
    }
    Ok(())
}

/// Reads `<name>.json`, or its `.bak` copy if the main file is damaged.
/// None = no save at all; the warnings are for the console.
pub fn read_with_backup(dir: &Path, name: &str, warnings: &mut Vec<String>) -> Option<serde_json::Value> {
    let (main, bak, _) = paths(dir, name);
    match parse(&main) {
        Ok(j) => return Some(j),
        Err(e) if main.exists() => warnings.push(format!("la partida '{}' está dañada ({e})", main.display())),
        Err(_) => {}
    }
    match parse(&bak) {
        Ok(j) => {
            if main.exists() || bak.exists() {
                warnings.push(format!("se carga la copia de seguridad '{}'", bak.display()));
            }
            Some(j)
        }
        Err(e) => {
            if bak.exists() {
                warnings.push(format!("la copia '{}' también está dañada ({e})", bak.display()));
            }
            None
        }
    }
}

/// Fills what `loaded` lacks from `defaults`: keys missing in structs (recursively).
/// Values present in `loaded` always win, even with a different type.
pub fn merge_defaults(loaded: Value, defaults: &Value) -> Value {
    match (&loaded, defaults) {
        (Value::Struct(l), Value::Struct(d)) => {
            let missing: Vec<(modscript::Sym, Value)> = {
                let lb = l.borrow();
                d.borrow().fields.iter().filter(|(k, _)| !lb.fields.contains_key(*k)).map(|(k, v)| (*k, deep_copy(v))).collect()
            };
            let present: Vec<(modscript::Sym, Value)> = {
                let lb = l.borrow();
                let db = d.borrow();
                lb.fields.iter().filter_map(|(k, v)| db.fields.get(k).map(|dv| (*k, merge_defaults(v.clone(), dv)))).collect()
            };
            {
                let mut lb = l.borrow_mut();
                for (k, v) in present.into_iter().chain(missing) {
                    lb.fields.insert(k, v);
                }
            }
            loaded
        }
        _ => loaded,
    }
}

/// Copy of arrays/structs so that the defaults are never shared with the loaded data.
fn deep_copy(v: &Value) -> Value {
    match v {
        Value::Array(a) => Value::array(a.borrow().iter().map(deep_copy).collect()),
        Value::Struct(s) => Value::new_struct(s.borrow().fields.iter().map(|(k, v)| (*k, deep_copy(v)))),
        o => o.clone(),
    }
}

pub(crate) fn register(r: &mut Reg) {
    r.f("save_data", |e, _, a| {
        let name = safe_name(&A(a).text(0)?)?;
        let json = to_json(A(a).val(1)?)?;
        let text = serde_json::to_string_pretty(&json).unwrap_or_default();
        match write_atomic(&e.save_dir, &name, &text) {
            Ok(()) => Ok(Value::Bool(true)),
            Err(err) => {
                eprintln!("[aviso] save_data('{name}'): no se pudo guardar en {}: {err}", e.save_dir.display());
                Ok(Value::Bool(false))
            }
        }
    });
    r.f("load_data", |e, _, a| {
        let name = safe_name(&A(a).text(0)?)?;
        let defaults = a.get(1).cloned().unwrap_or(Value::Null);
        let mut warnings = Vec::new();
        let loaded = read_with_backup(&e.save_dir, &name, &mut warnings);
        for w in warnings {
            eprintln!("[aviso] load_data('{name}'): {w}");
        }
        match loaded {
            Some(j) => Ok(merge_defaults(from_json(&j), &defaults)),
            None => Ok(deep_copy(&defaults)),
        }
    });
    r.f("save_exists", |e, _, a| {
        let name = safe_name(&A(a).text(0)?)?;
        let (main, bak, _) = paths(&e.save_dir, &name);
        Ok(Value::Bool(main.exists() || bak.exists()))
    });
    r.f("save_delete", |e, _, a| {
        let name = safe_name(&A(a).text(0)?)?;
        let (main, bak, tmp) = paths(&e.save_dir, &name);
        let mut any = false;
        for p in [main, bak, tmp] {
            any |= fs::remove_file(p).is_ok();
        }
        Ok(Value::Bool(any))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use modscript::intern;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("modengine-save-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn atomic_write_keeps_a_backup_of_the_previous_version() {
        let d = tmp("bak");
        write_atomic(&d, "p", "{\"v\": 1}").unwrap();
        assert!(!d.join("p.json.bak").exists());
        write_atomic(&d, "p", "{\"v\": 2}").unwrap();
        assert_eq!(fs::read_to_string(d.join("p.json.bak")).unwrap(), "{\"v\": 1}");
        assert_eq!(fs::read_to_string(d.join("p.json")).unwrap(), "{\"v\": 2}");
        assert!(!d.join("p.json.tmp").exists());
        // a corrupt main file falls back to the backup, with a warning
        fs::write(d.join("p.json"), "{\"v\": 3").unwrap();
        let mut w = vec![];
        let j = read_with_backup(&d, "p", &mut w).unwrap();
        assert_eq!(j["v"], 1);
        assert!(w[0].contains("dañada"), "{w:?}");
        // saving over a corrupt file does not overwrite the good backup
        write_atomic(&d, "p", "{\"v\": 4}").unwrap();
        assert_eq!(fs::read_to_string(d.join("p.json.bak")).unwrap(), "{\"v\": 1}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn defaults_fill_missing_keys_recursively() {
        let k = intern;
        let loaded = Value::new_struct([(k("a"), Value::Num(5.0)), (k("sub"), Value::new_struct([(k("x"), Value::Num(1.0))]))]);
        let defaults = Value::new_struct([
            (k("a"), Value::Num(0.0)),
            (k("b"), Value::Num(2.0)),
            (k("sub"), Value::new_struct([(k("x"), Value::Num(0.0)), (k("y"), Value::Num(9.0))])),
            (k("list"), Value::array(vec![Value::Num(1.0)])),
        ]);
        let m = merge_defaults(loaded, &defaults);
        let Value::Struct(s) = &m else { panic!() };
        let s = s.borrow();
        assert_eq!(s.fields[&k("a")].as_num(), Some(5.0));
        assert_eq!(s.fields[&k("b")].as_num(), Some(2.0));
        let Value::Struct(sub) = &s.fields[&k("sub")] else { panic!() };
        assert_eq!(sub.borrow().fields[&k("x")].as_num(), Some(1.0));
        assert_eq!(sub.borrow().fields[&k("y")].as_num(), Some(9.0));
        // copied, not shared with the defaults
        let (Value::Array(a1), Value::Struct(d)) = (&s.fields[&k("list")], &defaults) else { panic!() };
        let Value::Array(a2) = &d.borrow().fields[&k("list")] else { panic!() };
        assert!(!std::rc::Rc::ptr_eq(a1, a2));
    }
}

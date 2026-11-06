//! Localization functions (`loc`, `loc_set_language`...). See `loc.rs`.

use super::{err, Reg, A};
use crate::engine::Engine;
use modscript::{RtError, Value};

/// Reads the tables the first time a `loc*` function runs.
pub(crate) fn ensure(e: &mut Engine) {
    if e.loc.loaded {
        return;
    }
    e.loc = crate::loc::load(&e.dir);
    for msg in e.loc.errors.clone() {
        e.report(RtError::new(msg));
    }
}

fn args(v: Option<&Value>) -> Result<Vec<(String, String, Option<f64>)>, RtError> {
    match v {
        None => Ok(vec![]),
        Some(Value::Struct(s)) => Ok(s.borrow().fields.iter().map(|(k, v)| (k.name().to_string(), v.to_string(), v.as_num())).collect()),
        Some(o) => Err(RtError::new(format!("loc(key, [args]): args must be a struct like {{ name: \"Ana\", n: 3 }}, not {}", o.type_name()))),
    }
}

/// Language of the system (`LANG`, `LC_ALL`...): "es" from "es_ES.UTF-8".
pub fn system_language(env: &dyn Fn(&str) -> Option<String>) -> String {
    for k in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        if let Some(v) = env(k).filter(|v| !v.is_empty() && v != "C" && v != "POSIX") {
            let code: String = v.split(['_', '.', ':', '-', '@']).next().unwrap_or("").to_ascii_lowercase();
            if !code.is_empty() {
                return code;
            }
        }
    }
    "en".into()
}

pub(super) fn register(r: &mut Reg) {
    r.f("loc", |e, _, a| {
        let p = A(a);
        let key = p.text(0)?;
        let args = args(p.opt(1))?;
        ensure(e);
        Ok(Value::string(e.loc.text(&key, &args)))
    });
    r.f("loc_exists", |e, _, a| {
        let key = A(a).text(0)?;
        ensure(e);
        Ok(Value::Bool(e.loc.exists(&key)))
    });
    r.f("loc_set_language", |e, _, a| {
        let code = A(a).text(0)?;
        ensure(e);
        if code != crate::loc::PSEUDO && !e.loc.tables.contains_key(&code) {
            return Ok(Value::Bool(false));
        }
        e.loc.current = code;
        Ok(Value::Bool(true))
    });
    r.f("loc_language", |e, _, _| {
        ensure(e);
        Ok(Value::string(e.loc.current.clone()))
    });
    r.f("loc_languages", |e, _, _| {
        ensure(e);
        Ok(Value::array(e.loc.languages().into_iter().map(Value::string).collect()))
    });
    r.f("loc_default_language", |e, _, _| {
        ensure(e);
        Ok(Value::string(e.loc.default.clone()))
    });
    r.f("loc_fallback_language", |e, _, _| {
        ensure(e);
        Ok(e.loc.fallback.clone().map(Value::string).unwrap_or(Value::Null))
    });
    r.f("loc_set_pseudo", |e, _, a| {
        ensure(e);
        e.loc.pseudo = A(a).bool_or(0, true);
        Ok(Value::Null)
    });
    r.f("loc_pseudo", |e, _, _| {
        ensure(e);
        Ok(Value::Bool(e.loc.pseudo || e.loc.current == crate::loc::PSEUDO))
    });
    r.f("loc_missing", |e, _, _| {
        ensure(e);
        let mut v: Vec<String> = e.loc.missing.iter().cloned().collect();
        v.sort();
        Ok(Value::array(v.into_iter().map(Value::string).collect()))
    });
    r.f("loc_reload", |e, _, _| {
        let (cur, pseudo) = (e.loc.current.clone(), e.loc.pseudo);
        e.loc.loaded = false;
        ensure(e);
        if cur == crate::loc::PSEUDO || e.loc.tables.contains_key(&cur) {
            e.loc.current = cur;
        }
        e.loc.pseudo = pseudo;
        Ok(Value::Null)
    });
    r.f("loc_system_language", |_, _, _| Ok(Value::string(system_language(&|k| std::env::var(k).ok()))));
    // loc_plural(n): CLDR category ("one", "few", "other"...) of n in the current language
    r.f("loc_plural", |e, _, a| {
        let n = A(a).num(0)?;
        ensure(e);
        let lang = if e.loc.current == crate::loc::PSEUDO { e.loc.default.clone() } else { e.loc.current.clone() };
        Ok(Value::str(crate::loc::plural_category(&lang, n)))
    });
    r.f("loc_keys", |e, _, a| {
        ensure(e);
        let lang = match A(a).opt(0) {
            Some(v) => v.to_string(),
            None => e.loc.default.clone(),
        };
        match e.loc.tables.get(&lang) {
            Some(t) => Ok(Value::array(t.keys().map(|k| Value::string(k.clone())).collect())),
            None => err(format!("loc_keys(): there is no lang/{lang}.toml")),
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn system_language_from_env() {
        let env = |v: &'static str| move |k: &str| if k == "LANG" { Some(v.to_string()) } else { None };
        assert_eq!(super::system_language(&env("es_ES.UTF-8")), "es");
        assert_eq!(super::system_language(&env("C")), "en");
        assert_eq!(super::system_language(&env("pt_BR")), "pt");
    }
}

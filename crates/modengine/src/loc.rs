//! Localization: string tables in `lang/<code>.toml` (`key = "text"`, nested tables give
//! dotted keys), `{name}` placeholders, plural forms, a fallback language and a pseudo
//! language that makes untranslated or clipped text easy to spot.
//!
//! ```toml
//! # lang/es.toml
//! title = "Mazmorra"
//! [menu]
//! play = "Jugar"                       # key "menu.play"
//! greet = "Hola, {name}"
//! [coins]                              # plural: chosen with the argument n (or count)
//! one = "{n} moneda"
//! other = "{n} monedas"
//! ```
//!
//! game.toml: `[localization] default = "es"  fallback = "en"`. Without it the default is
//! the first language found (alphabetically) and there is no fallback.
//!
//! The tables are read the first time the game uses a `loc*` function, from the project
//! folder or the exported pack.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

/// One entry of a table: plain text or plural forms (`zero`, `one`, `two`, `few`, `many`, `other`).
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Text(String),
    Plural(BTreeMap<String, String>),
}

pub const PLURAL_KEYS: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];

/// Code of the built-in pseudo language.
pub const PSEUDO: &str = "pseudo";

pub type Table = BTreeMap<String, Entry>;

#[derive(Default)]
pub struct Loc {
    pub loaded: bool,
    /// language code → table
    pub tables: BTreeMap<String, Table>,
    pub default: String,
    pub fallback: Option<String>,
    pub current: String,
    /// Pseudo-localize whatever language is current.
    pub pseudo: bool,
    /// Errors found while reading the tables (reported once).
    pub errors: Vec<String>,
    /// Keys asked for that no table has.
    pub missing: HashSet<String>,
}

fn is_plural_table(t: &toml::Table) -> bool {
    t.contains_key("other") && t.keys().all(|k| PLURAL_KEYS.contains(&k.as_str())) && t.values().all(|v| v.is_str())
}

fn flatten(prefix: &str, t: &toml::Table, out: &mut Table, errors: &mut Vec<String>, file: &str) {
    for (k, v) in t {
        let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
        match v {
            toml::Value::String(s) => {
                out.insert(key, Entry::Text(s.clone()));
            }
            toml::Value::Table(sub) if is_plural_table(sub) => {
                out.insert(key, Entry::Plural(sub.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect()));
            }
            toml::Value::Table(sub) => flatten(&key, sub, out, errors, file),
            other => errors.push(format!("{file}: '{key}' debe ser texto, no {}", other.type_str())),
        }
    }
}

/// Parses one table file.
pub fn parse_table(src: &str, file: &str) -> Result<(Table, Vec<String>), String> {
    let t: toml::Table = toml::from_str(src).map_err(|e| format!("{file}: {e}"))?;
    let mut out = Table::new();
    let mut errors = vec![];
    flatten("", &t, &mut out, &mut errors, file);
    Ok((out, errors))
}

/// Reads `lang/*.toml` and the `[localization]` section of game.toml (folder or pack).
pub fn load(dir: &Path) -> Loc {
    let mut l = Loc { loaded: true, ..Default::default() };
    let lang = dir.join("lang");
    if let Ok(entries) = crate::vfs::read_dir(&lang) {
        let mut files: Vec<_> = entries.into_iter().filter(|(p, is_dir)| !is_dir && p.extension().is_some_and(|x| x == "toml")).map(|(p, _)| p).collect();
        files.sort();
        for p in files {
            let code = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let file = format!("lang/{code}.toml");
            match crate::vfs::read_to_string(&p).map_err(|e| format!("{file}: {e}")).and_then(|s| parse_table(&s, &file)) {
                Ok((t, errs)) => {
                    l.errors.extend(errs);
                    l.tables.insert(code, t);
                }
                Err(e) => l.errors.push(e),
            }
        }
    }
    let cfg = crate::vfs::read_to_string(&dir.join("game.toml")).ok().and_then(|s| toml::from_str::<toml::Table>(&s).ok());
    let sec = cfg.as_ref().and_then(|c| c.get("localization")).and_then(|v| v.as_table());
    let get = |k: &str| sec.and_then(|s| s.get(k)).and_then(|v| v.as_str()).map(|s| s.to_string());
    l.default = get("default").or_else(|| l.tables.keys().next().cloned()).unwrap_or_default();
    l.fallback = get("fallback").filter(|f| *f != l.default);
    l.current = l.default.clone();
    l
}

impl Loc {
    /// Languages with a table, plus `pseudo` when there is any.
    pub fn languages(&self) -> Vec<String> {
        let mut v: Vec<String> = self.tables.keys().cloned().collect();
        if !v.is_empty() {
            v.push(PSEUDO.into());
        }
        v
    }

    fn lookup(&self, lang: &str, key: &str) -> Option<&Entry> {
        self.tables.get(lang).and_then(|t| t.get(key))
    }

    /// The entry for `key` in the current language, then the fallback, then the default.
    pub fn entry(&self, key: &str) -> Option<&Entry> {
        let base = if self.current == PSEUDO { self.default.as_str() } else { self.current.as_str() };
        self.lookup(base, key).or_else(|| self.fallback.as_deref().and_then(|f| self.lookup(f, key))).or_else(|| self.lookup(&self.default, key))
    }

    pub fn exists(&self, key: &str) -> bool {
        self.entry(key).is_some()
    }

    fn pseudo_on(&self) -> bool {
        self.pseudo || self.current == PSEUDO
    }

    /// Language whose plural rules apply.
    fn rules_lang(&self) -> &str {
        if self.current == PSEUDO {
            &self.default
        } else {
            &self.current
        }
    }

    /// Text of `key` with `{name}` replaced by `args` (a missing key gives the key itself).
    pub fn text(&mut self, key: &str, args: &[(String, String, Option<f64>)]) -> String {
        let n = args.iter().find(|a| a.0 == "n" || a.0 == "count").and_then(|a| a.2);
        let template = match self.entry(key) {
            Some(Entry::Text(s)) => s.clone(),
            Some(Entry::Plural(forms)) => {
                let cat = plural_category(self.rules_lang(), n.unwrap_or(0.0));
                let pick = if n == Some(0.0) && forms.contains_key("zero") { "zero" } else { cat };
                forms.get(pick).or_else(|| forms.get("other")).cloned().unwrap_or_default()
            }
            None => {
                self.missing.insert(key.to_string());
                return key.to_string();
            }
        };
        let template = if self.pseudo_on() { pseudo(&template) } else { template };
        format_args(&template, args)
    }
}

/// CLDR plural category of `n` for a language code (the common rules; `other` otherwise).
pub fn plural_category(lang: &str, n: f64) -> &'static str {
    let base = lang.split(['-', '_']).next().unwrap_or(lang).to_ascii_lowercase();
    let int = n.fract() == 0.0;
    let i = n.abs().trunc() as u64;
    match base.as_str() {
        "ja" | "ko" | "zh" | "th" | "vi" | "id" | "tr" => "other",
        "fr" | "pt" if base == "fr" || lang.contains("BR") || lang.contains("br") => {
            if i <= 1 {
                "one"
            } else {
                "other"
            }
        }
        "ru" | "uk" | "be" => {
            if !int {
                return "other";
            }
            let (m10, m100) = (i % 10, i % 100);
            if m10 == 1 && m100 != 11 {
                "one"
            } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                "few"
            } else {
                "many"
            }
        }
        "pl" => {
            if !int {
                return "other";
            }
            let (m10, m100) = (i % 10, i % 100);
            if i == 1 {
                "one"
            } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                "few"
            } else {
                "many"
            }
        }
        _ => {
            if int && i == 1 {
                "one"
            } else {
                "other"
            }
        }
    }
}

/// Replaces `{name}` with the argument's text; `{{` and `}}` are literal braces; unknown
/// placeholders stay as they are.
pub fn format_args(t: &str, args: &[(String, String, Option<f64>)]) -> String {
    let mut out = String::with_capacity(t.len() + 16);
    let mut it = t.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        match c {
            '{' if it.peek().map(|x| x.1) == Some('{') => {
                it.next();
                out.push('{');
            }
            '}' if it.peek().map(|x| x.1) == Some('}') => {
                it.next();
                out.push('}');
            }
            '{' => match t[i + 1..].find('}') {
                Some(end) => {
                    let name = &t[i + 1..i + 1 + end];
                    match args.iter().find(|a| a.0 == name) {
                        Some(a) => out.push_str(&a.1),
                        None => {
                            out.push('{');
                            out.push_str(name);
                            out.push('}');
                        }
                    }
                    for _ in 0..name.chars().count() + 1 {
                        it.next();
                    }
                }
                None => out.push('{'),
            },
            c => out.push(c),
        }
    }
    out
}

fn accent(c: char) -> char {
    match c {
        'a' => 'á',
        'e' => 'é',
        'i' => 'í',
        'o' => 'ö',
        'u' => 'ü',
        'n' => 'ñ',
        'c' => 'ç',
        'y' => 'ý',
        'A' => 'Å',
        'E' => 'É',
        'I' => 'Î',
        'O' => 'Ø',
        'U' => 'Û',
        'N' => 'Ñ',
        'C' => 'Ç',
        'Y' => 'Ý',
        c => c,
    }
}

/// Pseudo-localization: accented letters, about 30% longer and wrapped in `[ ]`.
/// `{placeholders}` and rich-text `[tags]` are kept as they are.
pub fn pseudo(t: &str) -> String {
    let mut out = String::from("[");
    let mut letters = 0usize;
    let mut depth_brace = false;
    let mut depth_tag = false;
    for c in t.chars() {
        match c {
            '{' => depth_brace = true,
            '}' => depth_brace = false,
            '[' => depth_tag = true,
            ']' => depth_tag = false,
            _ => {}
        }
        if depth_brace || depth_tag || c == '}' || c == ']' {
            out.push(c);
            continue;
        }
        if c.is_alphabetic() {
            letters += 1;
        }
        out.push(accent(c));
    }
    let extra = (letters as f64 * 0.3).ceil() as usize;
    if extra > 0 {
        out.push(' ');
        for _ in 0..extra {
            out.push('~');
        }
    }
    out.push(']');
    out
}

/// Escapes text for a TOML basic string.
fn toml_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04X}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn toml_key(k: &str) -> String {
    if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        k.to_string()
    } else {
        toml_str(k)
    }
}

/// Writes a table back as TOML: plain keys first, then one `[section]` per first part of
/// dotted keys, plurals as inline tables. Reading it back gives the same table.
pub fn write_table(t: &Table) -> String {
    let mut root: Vec<(&str, &Entry)> = vec![];
    let mut sections: BTreeMap<&str, Vec<(&str, &Entry)>> = BTreeMap::new();
    for (k, v) in t {
        match k.split_once('.') {
            Some((s, rest)) => sections.entry(s).or_default().push((rest, v)),
            None => root.push((k, v)),
        }
    }
    let entry = |k: &str, v: &Entry| -> String {
        let key = k.split('.').map(toml_key).collect::<Vec<_>>().join(".");
        match v {
            Entry::Text(s) => format!("{key} = {}\n", toml_str(s)),
            Entry::Plural(f) => format!("{key} = {{ {} }}\n", f.iter().map(|(c, s)| format!("{c} = {}", toml_str(s))).collect::<Vec<_>>().join(", ")),
        }
    };
    let mut out = String::new();
    for (k, v) in &root {
        // a root key that is also a section name can't be written as `k = ...` before it
        if sections.contains_key(k) {
            continue;
        }
        out += &entry(k, v);
    }
    for (s, items) in &sections {
        out += &format!("\n[{}]\n", toml_key(s));
        for (k, v) in items {
            out += &entry(k, v);
        }
    }
    out
}

/// A `loc("key")` call with a literal key found in the code.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyUse {
    pub key: String,
    pub file: String,
    pub line: u32,
}

/// Every `loc("literal")` / `loc_exists("literal")` in the loaded code.
pub fn key_uses(e: &crate::engine::Engine) -> Vec<KeyUse> {
    walk_code(e).0
}

/// The `loc("literal")` calls of the code, and every string literal in it (keys kept in
/// arrays or structs, like `names = ["card.fire", ...]`, count as used).
fn walk_code(e: &crate::engine::Engine) -> (Vec<KeyUse>, HashSet<String>) {
    use modscript::ast::{Expr, Stmt, StmtKind};
    struct W {
        out: Vec<KeyUse>,
        lits: HashSet<String>,
        file: String,
        line: u32,
    }
    impl W {
        fn expr(&mut self, x: &Expr) {
            match x {
                Expr::Str(s) => {
                    self.lits.insert(s.to_string());
                }
                Expr::Call(f, args) => {
                    if let (Expr::Ident(n), Some(Expr::Str(s))) = (&**f, args.first()) {
                        let name = n.name();
                        if &*name == "loc" || &*name == "loc_exists" {
                            self.out.push(KeyUse { key: s.to_string(), file: self.file.clone(), line: self.line });
                        }
                    }
                    self.expr(f);
                    args.iter().for_each(|a| self.expr(a));
                }
                Expr::Array(v) => v.iter().for_each(|a| self.expr(a)),
                Expr::Struct(v) => v.iter().for_each(|(_, a)| self.expr(a)),
                Expr::Unary(_, a) | Expr::Field(a, _) => self.expr(a),
                Expr::Binary(_, a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Coalesce(a, b) | Expr::Index(a, b) => {
                    self.expr(a);
                    self.expr(b);
                }
                Expr::Ternary(a, b, c) => {
                    self.expr(a);
                    self.expr(b);
                    self.expr(c);
                }
                Expr::Func(d) => {
                    let saved = self.line;
                    self.block(&d.body);
                    self.line = saved;
                }
                _ => {}
            }
        }
        fn stmt(&mut self, s: &Stmt) {
            self.line = s.line;
            match &s.kind {
                StmtKind::Expr(x) | StmtKind::Return(Some(x)) => self.expr(x),
                StmtKind::Var(v) => v.iter().filter_map(|(_, x)| x.as_ref()).for_each(|x| self.expr(x)),
                StmtKind::Assign(a, _, b) => {
                    self.expr(a);
                    self.expr(b);
                }
                StmtKind::Incr(a, _) => self.expr(a),
                StmtKind::If(c, a, b) => {
                    self.expr(c);
                    self.stmt(a);
                    if let Some(b) = b {
                        self.stmt(b);
                    }
                }
                StmtKind::While(c, b) | StmtKind::Repeat(c, b) | StmtKind::With(c, b) => {
                    self.expr(c);
                    self.stmt(b);
                }
                StmtKind::DoUntil(b, c) => {
                    self.stmt(b);
                    self.expr(c);
                }
                StmtKind::For(a, c, n, b) => {
                    if let Some(a) = a {
                        self.stmt(a);
                    }
                    if let Some(c) = c {
                        self.expr(c);
                    }
                    if let Some(n) = n {
                        self.stmt(n);
                    }
                    self.stmt(b);
                }
                StmtKind::ForIn(_, _, c, b) => {
                    self.expr(c);
                    self.stmt(b);
                }
                StmtKind::Switch(c, cases) => {
                    self.expr(c);
                    for case in cases {
                        if let Some(v) = &case.value {
                            self.expr(v);
                        }
                        self.block(&case.body);
                    }
                }
                StmtKind::Block(b) => self.block(b),
                StmtKind::FuncDecl(_, d) => self.block(&d.body),
                _ => {}
            }
        }
        fn block(&mut self, b: &[Stmt]) {
            for s in b {
                self.stmt(s);
            }
        }
    }
    let mut w = W { out: vec![], lits: HashSet::new(), file: String::new(), line: 0 };
    for o in &e.objects {
        w.file = o.file.to_string();
        w.block(&o.props);
        for d in o.own_events.values() {
            w.block(&d.body);
        }
    }
    for d in e.functions.values() {
        w.file = d.file.to_string();
        w.block(&d.body);
    }
    for (file, code) in &e.init_code {
        w.file = file.to_string();
        w.block(code);
    }
    for r in &e.rooms {
        if let Some(code) = &r.code {
            w.file = r.file.to_string();
            w.block(code);
        }
    }
    let mut out = w.out;
    out.sort_by(|a, b| (&a.file, a.line, &a.key).cmp(&(&b.file, b.line, &b.key)));
    out.dedup();
    (out, w.lits)
}

/// Every text inside a data record (nested lists and structs too).
fn data_strings(v: &modscript::Value, out: &mut HashSet<String>) {
    match v {
        modscript::Value::Str(s) => {
            out.insert(s.to_string());
        }
        modscript::Value::Array(a) => a.borrow().iter().for_each(|x| data_strings(x, out)),
        modscript::Value::Struct(st) => st.borrow().fields.iter().for_each(|(_, x)| data_strings(x, out)),
        _ => {}
    }
}

/// `modengine check` findings: keys used in code that a language lacks, keys no code
/// uses (only literal `loc("key")` calls count), and errors in the tables.
pub fn check(e: &crate::engine::Engine) -> Vec<crate::lint::Finding> {
    use crate::lint::{Finding, Severity};
    let l = load(&e.dir);
    let mut out = vec![];
    for err in &l.errors {
        out.push(Finding { severity: Severity::Error, file: "lang".into(), line: 0, msg: err.clone() });
    }
    if l.tables.is_empty() {
        return out;
    }
    let (uses, mut lits) = walk_code(e);
    // text in the data tables counts too: records usually carry their own key (`name = "card.slash"`)
    for t in &e.data.tables {
        for v in &t.values {
            data_strings(v, &mut lits);
        }
    }
    let mut reported: HashSet<(String, String)> = HashSet::new();
    for u in &uses {
        let missing: Vec<&str> = l.tables.iter().filter(|(_, t)| !t.contains_key(&u.key)).map(|(c, _)| c.as_str()).collect();
        if !missing.is_empty() && reported.insert((u.key.clone(), u.file.clone())) {
            out.push(Finding {
                severity: if missing.len() == l.tables.len() { Severity::Error } else { Severity::Warning },
                file: u.file.clone(),
                line: u.line,
                msg: format!("texto sin traducir: la clave '{}' falta en {}", u.key, missing.iter().map(|c| format!("lang/{c}.toml")).collect::<Vec<_>>().join(", ")),
            });
        }
    }
    let used: HashSet<&str> = uses.iter().map(|u| u.key.as_str()).collect();
    let mut unused: HashMap<&str, Vec<&str>> = HashMap::new();
    for (code, t) in &l.tables {
        for k in t.keys() {
            if !used.contains(k.as_str()) && !lits.contains(k.as_str()) {
                unused.entry(code.as_str()).or_default().push(k.as_str());
            }
        }
    }
    let mut codes: Vec<&&str> = unused.keys().collect();
    codes.sort();
    for c in codes {
        let keys = &unused[*c];
        out.push(Finding {
            severity: Severity::Warning,
            file: format!("lang/{c}.toml"),
            line: 0,
            msg: format!(
                "{} clave(s) que no aparecen en el código (ni en loc(\"clave\") ni como texto): {}{}",
                keys.len(),
                keys.iter().take(12).copied().collect::<Vec<_>>().join(", "),
                if keys.len() > 12 { ", …" } else { "" }
            ),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(k: &str, v: &str) -> (String, String, Option<f64>) {
        (k.into(), v.into(), v.parse().ok())
    }

    #[test]
    fn placeholders_and_escapes() {
        assert_eq!(format_args("Hola, {name}! {{x}} {missing}", &[a("name", "Ana")]), "Hola, Ana! {x} {missing}");
        assert_eq!(format_args("{a}{b}", &[a("a", "1"), a("b", "ñ")]), "1ñ");
        assert_eq!(format_args("sin cierre {", &[]), "sin cierre {");
    }

    #[test]
    fn plural_rules() {
        assert_eq!(plural_category("es", 1.0), "one");
        assert_eq!(plural_category("en", 2.0), "other");
        assert_eq!(plural_category("fr", 0.0), "one");
        assert_eq!(plural_category("ru", 22.0), "few");
        assert_eq!(plural_category("ru", 25.0), "many");
        assert_eq!(plural_category("ru", 21.0), "one");
        assert_eq!(plural_category("pl", 12.0), "many");
        assert_eq!(plural_category("ja", 1.0), "other");
    }

    #[test]
    fn pseudo_keeps_placeholders_and_grows() {
        let p = pseudo("Play {n} cards [c=#f00]now[/c]");
        assert!(p.starts_with('[') && p.ends_with(']'));
        assert!(p.contains("{n}") && p.contains("[c=#f00]") && p.contains("[/c]"), "{p}");
        assert!(p.contains('á') && p.chars().count() as f64 >= "Play {n} cards [c=#f00]now[/c]".len() as f64 * 1.2, "{p}");
    }

    #[test]
    fn tables_roundtrip() {
        let src = "title = \"T\\\"x\"\n[menu]\nplay = \"Jugar\"\n[coins]\none = \"{n} moneda\"\nother = \"{n} monedas\"\n[a.b]\nc = \"deep\"\n";
        let (t, errs) = parse_table(src, "es.toml").unwrap();
        assert!(errs.is_empty());
        assert_eq!(t.get("menu.play"), Some(&Entry::Text("Jugar".into())));
        assert!(matches!(t.get("coins"), Some(Entry::Plural(_))));
        assert_eq!(t.get("a.b.c"), Some(&Entry::Text("deep".into())));
        let again = parse_table(&write_table(&t), "x").unwrap().0;
        assert_eq!(again, t);
    }
}

//! Structural view of `object Name : Parent { ... }` blocks inside a `.ms` file.
//!
//! The object editor works on this model and writes it back in a canonical
//! layout. Everything outside the object (other objects, functions, comments)
//! is left byte-for-byte untouched, and event/method bodies are kept verbatim
//! (only re-indented).

use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    /// `name = value`, or `name: ty = value` when `ty` is set.
    Prop { name: String, ty: Option<String>, value: String },
    Method { name: String, params: String, body: String },
    Event { name: String, arg: Option<String>, body: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Comment lines written just before the item.
    pub comments: Vec<String>,
    pub kind: ItemKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectSrc {
    pub name: String,
    pub parent: Option<String>,
    pub items: Vec<Item>,
}

/// An object found in a file. `model` is `None` when the body uses something
/// the structural editor does not understand (it can still be edited as text).
#[derive(Clone, Debug)]
pub struct Found {
    pub name: String,
    pub parent: Option<String>,
    pub span: Range<usize>,
    pub model: Option<ObjectSrc>,
}

impl ObjectSrc {
    pub fn prop(&self, name: &str) -> Option<&str> {
        self.items.iter().find_map(|i| match &i.kind {
            ItemKind::Prop { name: n, value, .. } if n == name => Some(value.as_str()),
            _ => None,
        })
    }

    /// Sets (or with `None` removes) a `name = value` line. New props go after the last prop.
    pub fn set_prop(&mut self, name: &str, value: Option<String>) {
        let pos = self.items.iter().position(|i| matches!(&i.kind, ItemKind::Prop { name: n, .. } if n == name));
        match (pos, value) {
            (Some(p), Some(v)) => {
                if let ItemKind::Prop { value, .. } = &mut self.items[p].kind {
                    *value = v;
                }
            }
            (Some(p), None) => {
                self.items.remove(p);
            }
            (None, Some(v)) => {
                let at = self.items.iter().rposition(|i| matches!(i.kind, ItemKind::Prop { .. })).map(|p| p + 1).unwrap_or(0);
                self.items.insert(at, Item { comments: vec![], kind: ItemKind::Prop { name: name.to_string(), ty: None, value: v } });
            }
            (None, None) => {}
        }
    }

    pub fn event_index(&self, name: &str, arg: Option<&str>) -> Option<usize> {
        self.items.iter().position(|i| matches!(&i.kind, ItemKind::Event { name: n, arg: a, .. } if n == name && a.as_deref() == arg))
    }
}

// ------------------------------------------------------------------ scanning

struct Sc<'a> {
    s: &'a [u8],
    i: usize,
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}
fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

impl<'a> Sc<'a> {
    fn at(&self, k: usize) -> u8 {
        *self.s.get(k).unwrap_or(&0)
    }
    fn eof(&self) -> bool {
        self.i >= self.s.len()
    }
    fn text(&self, r: Range<usize>) -> &'a str {
        std::str::from_utf8(&self.s[r]).unwrap_or("")
    }

    /// Skips a string literal starting at `i` (which must be a quote). Returns the index after it.
    fn skip_string(&self, mut i: usize) -> usize {
        let q = self.at(i);
        i += 1;
        while i < self.s.len() {
            match self.s[i] {
                b'\\' => i += 2,
                c if c == q => return i + 1,
                _ => i += 1,
            }
        }
        i
    }

    /// If a comment starts at `i`, returns its end.
    fn comment_end(&self, i: usize) -> Option<usize> {
        if self.at(i) == b'/' && self.at(i + 1) == b'/' {
            let mut j = i;
            while j < self.s.len() && self.s[j] != b'\n' {
                j += 1;
            }
            Some(j)
        } else if self.at(i) == b'/' && self.at(i + 1) == b'*' {
            let mut j = i + 2;
            while j < self.s.len() && !(self.s[j] == b'*' && self.at(j + 1) == b'/') {
                j += 1;
            }
            Some((j + 2).min(self.s.len()))
        } else {
            None
        }
    }

    fn skip_ws(&mut self) {
        while !self.eof() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    /// Skips whitespace and comments, returning the comment texts.
    fn skip_trivia(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            match self.comment_end(self.i) {
                Some(e) => {
                    out.push(self.text(self.i..e).trim_end().to_string());
                    self.i = e;
                }
                None => return out,
            }
        }
    }

    fn ident(&mut self) -> Option<&'a str> {
        if !is_ident_start(self.at(self.i)) {
            return None;
        }
        let st = self.i;
        while is_ident(self.at(self.i)) {
            self.i += 1;
        }
        Some(self.text(st..self.i))
    }

    fn peek_word(&self, w: &str) -> bool {
        let b = w.as_bytes();
        self.s.get(self.i..self.i + b.len()) == Some(b) && !is_ident(self.at(self.i + b.len())) && (self.i == 0 || !is_ident(self.at(self.i - 1)))
    }

    /// `open` is the index of an opening bracket; returns the index of its partner.
    fn matching(&self, open: usize) -> Option<usize> {
        let mut depth = 0i32;
        let mut i = open;
        while i < self.s.len() {
            if let Some(e) = self.comment_end(i) {
                i = e;
                continue;
            }
            match self.s[i] {
                b'"' | b'\'' => {
                    i = self.skip_string(i);
                    continue;
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// Reads a `{ ... }` block at the cursor and returns its inner text.
    fn block(&mut self) -> Option<String> {
        self.skip_ws();
        if self.at(self.i) != b'{' {
            return None;
        }
        let end = self.matching(self.i)?;
        let inner = self.text(self.i + 1..end).to_string();
        self.i = end + 1;
        Some(inner)
    }

    /// Reads `( ... )` at the cursor and returns the inner text.
    fn parens(&mut self) -> Option<String> {
        self.skip_ws();
        if self.at(self.i) != b'(' {
            return None;
        }
        let end = self.matching(self.i)?;
        let inner = self.text(self.i + 1..end).trim().to_string();
        self.i = end + 1;
        Some(inner)
    }

    /// Reads a type after `name:` up to the `=` (parentheses and strings may contain anything).
    fn type_text(&mut self) -> Option<String> {
        let st = self.i;
        let mut depth = 0;
        while !self.eof() {
            match self.s[self.i] {
                b'"' | b'\'' => {
                    self.i = self.skip_string(self.i);
                    continue;
                }
                b'(' => depth += 1,
                b')' => depth -= 1,
                b'=' if depth == 0 => break,
                b'\n' | b'{' | b'}' if depth == 0 => return None,
                _ => {}
            }
            self.i += 1;
        }
        let t = self.text(st..self.i).trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    }

    /// Reads an expression up to the end of the line (or `;`), following brackets over newlines.
    fn rest_of_value(&mut self) -> String {
        let st = self.i;
        while !self.eof() {
            let c = self.s[self.i];
            if c == b'\n' || c == b';' {
                break;
            }
            if c == b'"' || c == b'\'' {
                self.i = self.skip_string(self.i);
                continue;
            }
            if c == b'(' || c == b'[' || c == b'{' {
                match self.matching(self.i) {
                    Some(e) => {
                        self.i = e + 1;
                        continue;
                    }
                    None => {
                        self.i = self.s.len();
                        break;
                    }
                }
            }
            if c == b'}' {
                break;
            }
            self.i += 1;
        }
        let v = self.text(st..self.i).trim().to_string();
        if self.at(self.i) == b';' {
            self.i += 1;
        }
        v
    }
}

fn parse_body(sc: &mut Sc, end: usize) -> Option<Vec<Item>> {
    let mut items = Vec::new();
    loop {
        let comments = sc.skip_trivia();
        if sc.i >= end {
            return Some(items);
        }
        if sc.at(sc.i) == b';' {
            sc.i += 1;
            continue;
        }
        if sc.peek_word("on") {
            sc.i += 2;
            sc.skip_ws();
            let name = sc.ident()?.to_string();
            let save = sc.i;
            sc.skip_ws();
            let arg = if sc.at(sc.i) == b'(' {
                let a = sc.parens()?;
                if a.is_empty() {
                    None
                } else {
                    Some(a)
                }
            } else {
                sc.i = save;
                None
            };
            let body = dedent(&sc.block()?);
            items.push(Item { comments, kind: ItemKind::Event { name, arg, body } });
        } else if sc.peek_word("function") {
            sc.i += 8;
            sc.skip_ws();
            let name = sc.ident()?.to_string();
            let params = sc.parens()?;
            let body = dedent(&sc.block()?);
            items.push(Item { comments, kind: ItemKind::Method { name, params, body } });
        } else if is_ident_start(sc.at(sc.i)) {
            let name = sc.ident()?.to_string();
            sc.skip_ws();
            let ty = if sc.at(sc.i) == b':' {
                sc.i += 1;
                Some(sc.type_text()?)
            } else {
                None
            };
            if sc.at(sc.i) != b'=' || sc.at(sc.i + 1) == b'=' {
                return None;
            }
            sc.i += 1;
            while sc.at(sc.i) == b' ' || sc.at(sc.i) == b'\t' {
                sc.i += 1;
            }
            let value = sc.rest_of_value();
            items.push(Item { comments, kind: ItemKind::Prop { name, ty, value } });
        } else {
            return None;
        }
        if sc.i > end {
            return None;
        }
    }
}

/// Finds every top-level `object` declaration in `src`.
pub fn find_objects(src: &str) -> Vec<Found> {
    let mut sc = Sc { s: src.as_bytes(), i: 0 };
    let mut out = Vec::new();
    let mut depth = 0i32;
    while !sc.eof() {
        if let Some(e) = sc.comment_end(sc.i) {
            sc.i = e;
            continue;
        }
        let c = sc.s[sc.i];
        if c == b'"' || c == b'\'' {
            sc.i = sc.skip_string(sc.i);
            continue;
        }
        if depth == 0 && sc.peek_word("object") {
            let start = sc.i;
            sc.i += 6;
            sc.skip_ws();
            let Some(name) = sc.ident() else { continue };
            let name = name.to_string();
            sc.skip_ws();
            let mut parent = None;
            if sc.at(sc.i) == b':' {
                sc.i += 1;
                sc.skip_ws();
                parent = sc.ident().map(String::from);
                sc.skip_ws();
            }
            if sc.at(sc.i) != b'{' {
                continue;
            }
            let Some(close) = sc.matching(sc.i) else { break };
            let mut inner = Sc { s: sc.s, i: sc.i + 1 };
            let model = parse_body(&mut inner, close).map(|items| ObjectSrc { name: name.clone(), parent: parent.clone(), items });
            out.push(Found { name, parent, span: start..close + 1, model });
            sc.i = close + 1;
            continue;
        }
        match c {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if is_ident(c) {
            while is_ident(sc.at(sc.i)) {
                sc.i += 1;
            }
        } else {
            sc.i += 1;
        }
    }
    out
}

// ------------------------------------------------------------------ writing

pub fn dedent(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let first = lines.iter().position(|l| !l.trim().is_empty());
    let Some(first) = first else { return String::new() };
    let last = lines.iter().rposition(|l| !l.trim().is_empty()).unwrap();
    let slice = &lines[first..=last];
    let indent = slice.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    slice
        .iter()
        .map(|l| if l.trim().is_empty() { "" } else if l.len() >= indent { &l[indent..] } else { l.trim_start() })
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

fn indent(body: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    body.lines().map(|l| if l.trim().is_empty() { String::new() } else { format!("{pad}{l}") }).collect::<Vec<_>>().join("\n")
}

pub fn write_object(o: &ObjectSrc) -> String {
    let mut s = format!("object {}", o.name);
    if let Some(p) = &o.parent {
        s.push_str(&format!(" : {p}"));
    }
    s.push_str(" {\n");
    let mut prev_prop: Option<bool> = None;
    for item in &o.items {
        let is_prop = matches!(item.kind, ItemKind::Prop { .. });
        if let Some(pp) = prev_prop {
            if !(pp && is_prop) {
                s.push('\n');
            }
        }
        prev_prop = Some(is_prop);
        for c in &item.comments {
            s.push_str(&indent(c, 4));
            s.push('\n');
        }
        match &item.kind {
            ItemKind::Prop { name, ty: None, value } => s.push_str(&format!("    {name} = {value}\n")),
            ItemKind::Prop { name, ty: Some(t), value } => s.push_str(&format!("    {name}: {t} = {value}\n")),
            ItemKind::Method { name, params, body } => {
                s.push_str(&format!("    function {name}({params}) {{\n"));
                if !body.trim().is_empty() {
                    s.push_str(&indent(body, 8));
                    s.push('\n');
                }
                s.push_str("    }\n");
            }
            ItemKind::Event { name, arg, body } => {
                match arg {
                    Some(a) => s.push_str(&format!("    on {name}({a}) {{\n")),
                    None => s.push_str(&format!("    on {name} {{\n")),
                }
                if !body.trim().is_empty() {
                    s.push_str(&indent(body, 8));
                    s.push('\n');
                }
                s.push_str("    }\n");
            }
        }
    }
    s.push('}');
    s
}

/// Replaces the object called `name` in `src` with `new_text` (or appends it).
pub fn splice(src: &str, name: &str, new_text: &str) -> String {
    match find_objects(src).into_iter().find(|f| f.name == name) {
        Some(f) => format!("{}{}{}", &src[..f.span.start], new_text, &src[f.span.end..]),
        None => {
            let mut s = src.trim_end().to_string();
            if !s.is_empty() {
                s.push_str("\n\n");
            }
            s.push_str(new_text);
            s.push('\n');
            s
        }
    }
}

/// Removes the object called `name` from `src`.
pub fn remove(src: &str, name: &str) -> String {
    match find_objects(src).into_iter().find(|f| f.name == name) {
        Some(f) => {
            let before = src[..f.span.start].trim_end();
            let after = src[f.span.end..].trim_start();
            let mut s = before.to_string();
            if !s.is_empty() && !after.is_empty() {
                s.push_str("\n\n");
            }
            s.push_str(after);
            if !s.is_empty() && !s.ends_with('\n') {
                s.push('\n');
            }
            s
        }
        None => src.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"// header comment stays
function helper() { return 1 }

object Player : Actor {
    sprite = player_idle
    mask = player_idle      // same box
    hits = { a: 1,
             b: 2 }
    mode: list("a = b", "c") = "c"

    // a method
    function die() {
        if dead return
        dead = true
    }

    on step {
        var s = "}"   // braces in strings are fine
        x += 1
    }
    on collision(Slime) { other.squash() }
    on key_pressed(key.space) {
    }
}

object Weird { alarm[0] = 5 }
"#;

    #[test]
    fn finds_and_parses() {
        let f = find_objects(SRC);
        assert_eq!(f.len(), 2);
        let p = f[0].model.as_ref().unwrap();
        assert_eq!(p.name, "Player");
        assert_eq!(p.parent.as_deref(), Some("Actor"));
        assert_eq!(p.prop("sprite"), Some("player_idle"));
        assert_eq!(p.prop("mask"), Some("player_idle      // same box"));
        assert_eq!(p.prop("hits"), Some("{ a: 1,\n             b: 2 }"));
        assert_eq!(p.items.len(), 8);
        assert!(matches!(&p.items[3].kind, ItemKind::Prop { ty: Some(t), value, .. } if t == "list(\"a = b\", \"c\")" && value == "\"c\""));
        assert_eq!(p.items[4].comments, vec!["// a method"]);
        match &p.items[5].kind {
            ItemKind::Event { name, body, .. } => {
                assert_eq!(name, "step");
                assert_eq!(body, "var s = \"}\"   // braces in strings are fine\nx += 1");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(p.event_index("collision", Some("Slime")), Some(6));
        assert!(f[1].model.is_none(), "unsupported body must not be structurally editable");
    }

    #[test]
    fn roundtrip_keeps_meaning_and_surroundings() {
        let f = find_objects(SRC);
        let mut m = f[0].model.clone().unwrap();
        m.set_prop("depth", Some("-10".into()));
        m.set_prop("mask", None);
        let text = splice(SRC, "Player", &write_object(&m));
        assert!(text.starts_with("// header comment stays\nfunction helper()"));
        assert!(text.contains("object Weird { alarm[0] = 5 }"));
        let again = find_objects(&text)[0].model.clone().unwrap();
        assert_eq!(again, m);
        modscript::parse_file("t.ms", &text).expect("rewritten file must still compile");
    }

    #[test]
    fn example_objects_roundtrip() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/platformer/objects");
        for e in std::fs::read_dir(dir).unwrap() {
            let src = std::fs::read_to_string(e.unwrap().path()).unwrap();
            for f in find_objects(&src) {
                let m = f.model.unwrap_or_else(|| panic!("{} should be editable", f.name));
                let text = splice(&src, &m.name, &write_object(&m));
                let back = find_objects(&text).into_iter().find(|x| x.name == m.name).unwrap().model.unwrap();
                assert_eq!(back, m);
                modscript::parse_file("t.ms", &text).unwrap();
            }
        }
    }

    #[test]
    fn remove_object() {
        let t = remove(SRC, "Player");
        assert!(!t.contains("object Player"));
        assert!(t.contains("function helper()") && t.contains("object Weird"));
    }
}

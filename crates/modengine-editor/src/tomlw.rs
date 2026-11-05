//! Small helpers to write readable TOML by hand (the editor controls the layout
//! so maps and pixel art stay as multi-line blocks).

use modscript::fmt_num;

/// A quoted TOML string.
pub fn q(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// A key, quoted only when needed.
pub fn key(k: &str) -> String {
    if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        k.to_string()
    } else {
        q(k)
    }
}

pub fn num(f: f64) -> String {
    fmt_num(f)
}

/// A multi-line literal block (`'''` ... `'''`), falling back to an escaped string.
pub fn block(s: &str) -> String {
    if s.contains("'''") {
        q(s)
    } else {
        format!("'''\n{}\n'''", s.trim_end_matches('\n'))
    }
}

pub fn color_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let c = modengine::draw::parse_color(s)?;
    Some([(c >> 16) as u8, (c >> 8) as u8, c as u8])
}

/// A valid asset/object name: letters, digits and `_`, not starting with a digit.
pub fn valid_name(s: &str) -> bool {
    let mut ch = s.chars();
    matches!(ch.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') && ch.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

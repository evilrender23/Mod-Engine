//! The project's `CREDITS.md`: who made the third-party assets a game uses. The editor's store
//! adds OpenGameArt assets (`modengine-editor/src/store_credits.rs`); `modengine import
//! --credit "…"` adds art imported from a folder with the text the user or the agent gives
//! (usually what the pack's LICENSE/README says). Both share the header, the dates and the
//! layout: one `## ` section per source, a line per file, and a machine-readable marker at the
//! end of the section so later imports from the same source land in it.

use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "CREDITS.md";

pub const HEADER: &str = "# Créditos\n\nAssets de terceros usados en este juego. El editor de Mod Engine añade aquí lo que importas desde la Tienda (OpenGameArt.org); puedes editarlo, pero no borres a nadie cuya licencia pida crédito (CC-BY, OGA-BY, CC-BY-SA, GPL).\n";

/// `YYYY-MM-DD` (UTC) of a Unix time.
pub fn date(unix: u64) -> String {
    // days → civil date (Howard Hinnant's algorithm)
    let z = (unix / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn today() -> String {
    date(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

/// Art imported from a folder or file on disk, credited with free text.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LocalCredit {
    /// Groups imports from the same source: `<!-- import:<slug> -->`.
    pub slug: String,
    /// Section title: the folder (or file) it came from.
    pub title: String,
    /// What the pack says: author, licence, link.
    pub text: String,
}

/// `text` (the current CREDITS.md, maybe empty) with `files` (`(project file, source file)`)
/// of `c` added, under its section when there is one already. Files already listed are not
/// repeated.
pub fn add_local(text: &str, c: &LocalCredit, files: &[(String, String)], date: &str) -> String {
    let mut text = if text.trim().is_empty() { HEADER.to_string() } else { text.to_string() };
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let line = |(f, src): &(String, String)| format!("- Fichero: `{f}` (de «{src}», importado el {date})");
    let mark = format!("<!-- import:{} -->", c.slug);
    if let Some(at) = text.find(&mark) {
        let section_start = text[..at].rfind("\n## ").unwrap_or(0);
        let section = &text[section_start..at];
        let new: String = files.iter().filter(|(f, _)| !section.contains(&format!("`{f}`"))).map(|f| line(f) + "\n").collect();
        text.insert_str(at, &new);
        return text;
    }
    let mut s = format!("\n## {}\n\n", c.title);
    for (i, l) in c.text.trim().lines().filter(|l| !l.trim().is_empty()).enumerate() {
        s += &if i == 0 { format!("- Créditos / licencia: {}\n", l.trim()) } else { format!("  {}\n", l.trim()) };
    }
    for f in files {
        s += &(line(f) + "\n");
    }
    s += &format!("{mark}\n");
    text + &s
}

/// Adds the credit to `<project>/CREDITS.md`; returns the file when it changed.
pub fn write_local(project: &Path, c: &LocalCredit, files: &[(String, String)]) -> Result<Option<PathBuf>, String> {
    let path = project.join(FILE);
    let old = fs::read_to_string(&path).unwrap_or_default();
    let new = add_local(&old, c, files, &today());
    if new == old {
        return Ok(None);
    }
    fs::write(&path, new).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_credits_group_by_source() {
        let c = LocalCredit { slug: "kenney_pack".into(), title: "kenney_pack".into(), text: "Kenney (www.kenney.nl)\nLicencia: CC0".into() };
        let t = add_local("", &c, &[("sprites/hero.png".into(), "hero.png".into())], "2026-11-05");
        assert!(t.starts_with("# Créditos\n"));
        assert!(t.contains("## kenney_pack\n\n- Créditos / licencia: Kenney (www.kenney.nl)\n  Licencia: CC0\n- Fichero: `sprites/hero.png` (de «hero.png», importado el 2026-11-05)\n<!-- import:kenney_pack -->\n"), "{t}");
        let t2 = add_local(&t, &c, &[("sprites/hero.png".into(), "hero.png".into()), ("sounds/jump.wav".into(), "jump.wav".into())], "2026-11-06");
        assert_eq!(t2.matches("## kenney_pack").count(), 1);
        assert_eq!(t2.matches("`sprites/hero.png`").count(), 1);
        assert!(t2.find("`sounds/jump.wav`").unwrap() < t2.find("<!-- import:kenney_pack -->").unwrap());
        assert_eq!(date(1_793_836_800), "2026-11-05");
    }
}

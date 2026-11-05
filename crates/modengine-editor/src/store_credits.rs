//! Attribution of assets imported from the store: every import adds the asset to the
//! project's `CREDITS.md` (title, author, licence(s), source URL, files, date), grouped by
//! asset and with a machine-readable marker (`<!-- oga:<slug> -->`), and a sprite that has a
//! `.toml` also gets a comment saying where it came from. CREDITS.md is the record that
//! counts: the comment is a convenience.

use std::fs;
use std::path::Path;

pub use modengine::credits::FILE;

/// Where an imported file came from.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Credit {
    pub slug: String,
    pub title: String,
    pub author: String,
    pub author_url: Option<String>,
    pub collaborators: Vec<String>,
    /// Licence names as shown on OGA (`CC-BY 3.0`), with their URL when known.
    pub licenses: Vec<(String, Option<String>)>,
    pub url: String,
    /// The author's own attribution text, if any.
    pub notice: String,
    /// What was downloaded: `slime.png`, or `Slime.zip → Slime/slime_idle.png`.
    pub source_file: String,
}

impl Credit {
    pub fn from_asset(a: &crate::store_oga::Asset, source_file: &str) -> Credit {
        Credit {
            slug: a.slug.clone(),
            title: a.title.clone(),
            author: a.author.clone(),
            author_url: a.author_url.clone(),
            collaborators: a.collaborators.clone(),
            licenses: a.licenses.iter().map(|l| (l.name(), l.url().map(String::from))).collect(),
            url: a.url(),
            notice: a.notice.clone(),
            source_file: source_file.to_string(),
        }
    }

    /// One line, for logs and TOML comments.
    pub fn one_line(&self) -> String {
        let lic = if self.licenses.is_empty() { "licencia desconocida".to_string() } else { self.licenses.iter().map(|l| l.0.clone()).collect::<Vec<_>>().join(" / ") };
        format!("«{}» de {} · {} · {}", self.title, self.author_or_unknown(), lic, self.url)
    }

    fn author_or_unknown(&self) -> &str {
        if self.author.is_empty() { "autor desconocido" } else { &self.author }
    }
}

pub use modengine::credits::today;
#[cfg(test)]
use modengine::credits::date;

use modengine::credits::HEADER;

fn marker(slug: &str) -> String {
    format!("<!-- oga:{slug} -->")
}

/// `text` (the current CREDITS.md, maybe empty) with `files` of `c` added: under the asset's
/// section when it already has one, else in a new section at the end. Files already listed
/// are not repeated.
pub fn add_credit(text: &str, c: &Credit, files: &[String], date: &str) -> String {
    let mut text = if text.trim().is_empty() { HEADER.to_string() } else { text.to_string() };
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let file_lines: Vec<String> = files.iter().map(|f| format!("- Fichero: `{f}` (de «{}», importado el {date})", c.source_file)).collect();
    let mark = marker(&c.slug);
    if let Some(at) = text.find(&mark) {
        // the section ends at the marker: new files go just before it
        let section_start = text[..at].rfind("\n## ").unwrap_or(0);
        let section = &text[section_start..at];
        let new: Vec<&String> = files.iter().zip(&file_lines).filter(|(f, _)| !section.contains(&format!("`{f}`"))).map(|(_, l)| l).collect();
        if new.is_empty() {
            return text;
        }
        let insert = new.iter().map(|l| format!("{l}\n")).collect::<String>();
        text.insert_str(at, &insert);
        return text;
    }
    let mut s = format!("\n## {} — {}\n\n", c.title, c.author_or_unknown());
    match &c.author_url {
        Some(u) => s += &format!("- Autor: [{}]({u})\n", c.author_or_unknown()),
        None => s += &format!("- Autor: {}\n", c.author_or_unknown()),
    }
    if !c.collaborators.is_empty() {
        s += &format!("- Colaboradores: {}\n", c.collaborators.join(", "));
    }
    if c.licenses.is_empty() {
        s += "- Licencia: **desconocida** (revisa la página original)\n";
    } else {
        let ls: Vec<String> = c.licenses.iter().map(|(n, u)| match u {
            Some(u) => format!("[{n}]({u})"),
            None => n.clone(),
        }).collect();
        let label = if c.licenses.len() > 1 { "Licencias (a elegir)" } else { "Licencia" };
        s += &format!("- {label}: {}\n", ls.join(", "));
    }
    s += &format!("- Fuente: <{}>\n", c.url);
    if !c.notice.trim().is_empty() {
        s += &format!("- Aviso del autor: \"{}\"\n", c.notice.trim().replace('\n', " "));
    }
    for l in &file_lines {
        s += &format!("{l}\n");
    }
    s += &format!("{mark}\n");
    text + &s
}

/// Adds the credit to `<project>/CREDITS.md`; a sprite `.toml` among `files` also gets a
/// comment on top. Returns the path written.
pub fn write(project: &Path, c: &Credit, files: &[String]) -> Result<std::path::PathBuf, String> {
    let path = project.join(FILE);
    let old = fs::read_to_string(&path).unwrap_or_default();
    let new = add_credit(&old, c, files, &today());
    if new != old {
        fs::write(&path, new).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    for f in files.iter().filter(|f| f.ends_with(".toml")) {
        let p = project.join(f);
        if let Ok(t) = fs::read_to_string(&p) {
            if !t.contains(&c.url) {
                let note = format!("# Origen: {}\n# Créditos completos en CREDITS.md\n", c.one_line());
                let _ = fs::write(&p, note + &t);
            }
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credit() -> Credit {
        Credit {
            slug: "animated-slime".into(),
            title: "Animated Slime".into(),
            author: "Calciumtrice".into(),
            author_url: Some("https://opengameart.org/users/calciumtrice".into()),
            collaborators: vec![],
            licenses: vec![("CC-BY 3.0".into(), Some("https://creativecommons.org/licenses/by/3.0/".into()))],
            url: "https://opengameart.org/content/animated-slime".into(),
            notice: "Animated Slime by Calciumtrice, usable under Creative Commons Attribution 3.0 license.".into(),
            source_file: "slime spritesheet calciumtrice.png".into(),
        }
    }

    #[test]
    fn dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(1_793_836_800), "2026-11-05");
    }

    #[test]
    fn credits_md_grows_by_asset() {
        let c = credit();
        let t = add_credit("", &c, &["sprites/slime.png".into(), "sprites/slime.toml".into()], "2026-11-05");
        assert!(t.starts_with("# Créditos\n"));
        assert!(t.contains("## Animated Slime — Calciumtrice\n"));
        assert!(t.contains("- Autor: [Calciumtrice](https://opengameart.org/users/calciumtrice)\n"));
        assert!(t.contains("- Licencia: [CC-BY 3.0](https://creativecommons.org/licenses/by/3.0/)\n"));
        assert!(t.contains("- Fuente: <https://opengameart.org/content/animated-slime>\n"));
        assert!(t.contains("- Aviso del autor: \"Animated Slime by Calciumtrice"));
        assert!(t.contains("- Fichero: `sprites/slime.png` (de «slime spritesheet calciumtrice.png», importado el 2026-11-05)\n"));
        assert!(t.trim_end().ends_with("<!-- oga:animated-slime -->"));
        // the same asset again: one section, new file added, old one not repeated
        let t2 = add_credit(&t, &c, &["sprites/slime.png".into(), "sprites/slime_2.png".into()], "2026-11-06");
        assert_eq!(t2.matches("## Animated Slime").count(), 1);
        assert_eq!(t2.matches("`sprites/slime.png`").count(), 1);
        assert!(t2.contains("`sprites/slime_2.png`"));
        assert!(t2.find("`sprites/slime_2.png`").unwrap() < t2.find("<!-- oga:animated-slime -->").unwrap());
        assert_eq!(add_credit(&t2, &c, &["sprites/slime.png".into()], "x"), t2, "nothing new: unchanged");
        // another asset, unknown licence, user text kept
        let mut other = credit();
        other.slug = "x".into();
        other.title = "X".into();
        other.licenses.clear();
        other.author.clear();
        let t3 = add_credit(&format!("{t2}\nMúsica: mía.\n"), &other, &["sounds/x.ogg".into()], "2026-11-07");
        assert!(t3.contains("Música: mía.") && t3.contains("## X — autor desconocido") && t3.contains("**desconocida**"));
    }

    #[test]
    fn writes_into_the_project() {
        let dir = crate::store_net::tests::tmp("credits");
        std::fs::create_dir_all(dir.join("sprites")).unwrap();
        std::fs::write(dir.join("sprites/slime.toml"), "image = \"slime.png\"\nframes = 4\n").unwrap();
        let c = credit();
        write(&dir, &c, &["sprites/slime.png".into(), "sprites/slime.toml".into()]).unwrap();
        write(&dir, &c, &["sprites/slime.png".into(), "sprites/slime.toml".into()]).unwrap();
        let md = std::fs::read_to_string(dir.join("CREDITS.md")).unwrap();
        assert_eq!(md.matches("## Animated Slime").count(), 1);
        let toml_text = std::fs::read_to_string(dir.join("sprites/slime.toml")).unwrap();
        assert_eq!(toml_text.matches("# Origen:").count(), 1, "{toml_text}");
        let t: toml::Table = toml_text.parse().unwrap();
        assert_eq!(t["frames"].as_integer(), Some(4), "still valid TOML");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
